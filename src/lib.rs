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
                     let loop_ = Loop::new(rx);
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
        let mut f = UnsafeCell::new(Some(
            boxfnonce::SendBoxFnOnce::from(
                move |mut t: context::Transfer| {
                    f()
                })
        ));

        extern "C" fn context_function(mut t: context::Transfer) -> ! {

            let f = {
                let cell : &mut UnsafeCell<Option<boxfnonce::SendBoxFnOnce<(context::Transfer,),()>>> = unsafe { mem::transmute(t.data) };
                unsafe { (*cell.get()).take()}.unwrap()
            };

            let t = t.context.resume(0);

            f.call(t);

            unreachable!();
        }

        let stack = stack::ProtectedFixedSizeStack::default();

        let context = context::Context::new(&stack, context_function);
        let t = context.resume(&mut f as *mut _ as usize);

        let i = self.spawn_tx_i.fetch_add(1, Ordering::Relaxed);
        let i = i % self.loop_tx.len();

        self.loop_tx[i].send(LoopMsg::Spawn(t.context));

    }
}
// }}}

// {{{ Channel
enum LoopMsg {
    Nop,
    Spawn(context::Context),
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
    reg: mio::Registration,
}

impl LoopRx {
    fn new(rx: mpsc::Receiver<LoopMsg>, reg: mio::Registration) -> Self {
        LoopRx { rx: rx, reg: reg }
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
}

impl Loop {
    fn new(rx: LoopRx) -> Self {
        Loop {
            poll: mio::Poll::new().unwrap(),
            rx: rx,
        }
    }
    fn run(&self) {}
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
    #[test]
    fn it_works() {}
}

// vim: foldmethod=marker foldmarker={{{,}}}
