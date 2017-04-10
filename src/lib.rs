extern crate mio;
#[macro_use]
extern crate lazy_static;
extern crate context;
extern crate boxfnonce;

use std::mem;
use std::cell::UnsafeCell;

use std::sync::{mpsc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};

use context::stack;

// {{{ Misc
macro_rules! printerrln {
    ($($arg:tt)*) => ({
        use std::io::prelude::*;
        if let Err(e) = writeln!(&mut ::std::io::stderr(), "{}",
            format_args!($($arg)*)) {
            panic!(concat!(
                    "Failed to write to stderr.\n",
                    "Original error output: {}\n",
                    "Secondary error writing to stderr: {}"),
                    format_args!($($arg)*), e);
        }
    })
}
// }}}

struct Fiber {
    cur_context : context::Context,
    stack: stack::ProtectedFixedSizeStack,
}

impl Fiber {

    fn new<F, T>(f: F) -> Self
        where F: Send + 'static + FnOnce() -> T,
              T: Send + 'static {
        let mut f = UnsafeCell::new(Some(
            boxfnonce::SendBoxFnOnce::from(
                move |mut t: context::Transfer| {
                    f()
                })
        ));

        extern "C" fn context_function(mut t: context::Transfer) -> ! {
            printerrln!("Here");

            let f = {
                let cell : &mut UnsafeCell<Option<boxfnonce::SendBoxFnOnce<(context::Transfer,),()>>> = unsafe { mem::transmute(t.data) };
                unsafe { (*cell.get()).take()}.unwrap()
            };

            printerrln!("Here2");
            let t = t.context.resume(0);

            f.call(t);
            printerrln!("Here3");

            unreachable!();
        }

        let stack = stack::ProtectedFixedSizeStack::default();

        let context = context::Context::new(&stack, context_function);
        let t = context.resume(&mut f as *mut _ as usize);
        debug_assert!(unsafe { (&*f.get()) }.is_none());

        Fiber {
            cur_context: t.context,
            stack: stack
        }
    }
}

unsafe impl Send for Fiber {}

// {{{ Miofib
lazy_static! {
    static ref MIOFIB: Miofib = {
        Miofib::new()
    };
}

struct Miofib {
    spawn_tx_i : AtomicUsize,
    loop_tx: Vec<LoopTx>,
    loop_join: Vec<std::thread::JoinHandle<()>>,
}

impl Miofib {
    fn new() -> Self {
        let (txs, joins) = (0..4)
            .map(|_| {
                     let (tx, rx) = channel();
                     let mut loop_ = Loop::new(rx);
                     let join = std::thread::spawn(move || loop_.run());
                     (tx, join)
                 })
            .unzip();

        Miofib {
            spawn_tx_i : AtomicUsize::new(0),
            loop_tx: txs,
            loop_join: joins,
        }
    }

    fn spawn<F, T>(&self, f: F)
        where F: Send + 'static + FnOnce() -> T,
              T: Send + 'static
    {
        let fiber = Fiber::new(f);

        let i = self.spawn_tx_i.fetch_add(1, Ordering::Relaxed);
        let i = i % self.loop_tx.len();

        self.loop_tx[i].send(LoopMsg::Spawn(fiber));
    }
}
// }}}

// {{{ Channel
enum LoopMsg {
    Spawn(Fiber),
}

struct LoopTx {
    // TODO: Optimize use mpsc with Sync Sender
    tx: Mutex<mpsc::Sender<LoopMsg>>,
    ctrl: mio::SetReadiness,
}

impl LoopTx {
    fn new(tx: mpsc::Sender<LoopMsg>, ctrl: mio::SetReadiness) -> Self {
        LoopTx {
            tx: Mutex::new(tx),
            ctrl: ctrl,
        }
    }
    fn send(&self, msg : LoopMsg) {
        self.tx.lock().unwrap().send(msg).unwrap();
        self.ctrl.set_readiness(mio::Ready::readable());
    }
}

struct LoopRx {
    // TODO: Optimize use mpsc with Sync Sender
    rx: mpsc::Receiver<LoopMsg>,
    rx_registration: mio::Registration,
}

impl LoopRx {
    fn new(rx: mpsc::Receiver<LoopMsg>, reg: mio::Registration) -> Self {
        LoopRx { rx: rx, rx_registration: reg }
    }
}

fn channel() -> (LoopTx, LoopRx) {

    let (reg, ctrl) = mio::Registration::new2();
    let (tx, rx) = mpsc::channel();

    (LoopTx::new(tx, ctrl), LoopRx::new(rx, reg))
}

// }}}

// {{{ Loop
/// Event loop on a given thread
struct Loop {
    poll: mio::Poll,
    rx: LoopRx,
    events : mio::Events,
}

impl Loop {
    fn new(rx: LoopRx) -> Self {

        let poll = mio::Poll::new().unwrap();
        let mut events = mio::Events::with_capacity(1024);

        poll.register(&rx.rx_registration, mio::Token(0),
        mio::Ready::readable(), mio::PollOpt::level());

        Loop {
            poll: poll,
            rx: rx,
            events: events,
        }
    }

    fn run(&mut self) {
        loop {
            let events = self.poll.poll(&mut self.events, None).unwrap();
        }
    }
}
// }}}

//TODO: pub fn spawn<F, T>(f: F) -> Receiver<T>
pub fn spawn<F, T>(f: F)
    where F: FnOnce() -> T,
          F: Send + 'static,
          T: Send + 'static
{
    MIOFIB.spawn(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        spawn(|| {
            printerrln!("It works");
            assert_eq!(1, 2);

        })
    }
}

// vim: foldmethod=marker foldmarker={{{,}}}
