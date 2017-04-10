extern crate mio;
#[macro_use]
extern crate lazy_static;
extern crate context;
extern crate boxfnonce;
#[macro_use]
extern crate slog;
extern crate slog_term;
extern crate slog_async;
extern crate slab;

use slog::{Logger, Drain};

use std::{mem, thread};
use std::cell::UnsafeCell;

use std::sync::{mpsc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};

use context::stack;
use slab::Slab;

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
fn miofib_logger() -> Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain).build().fuse();

    slog::Logger::root(drain, o!("miofib" => env!("CARGO_PKG_VERSION") ))
}
// }}}

// {{{ Fiber
struct Fiber {
    cur_context: context::Context,
    stack: stack::ProtectedFixedSizeStack,
    log : Logger,
}

impl Fiber {
    fn new<F, T>(f: F, log : &Logger) -> Self
        where F: Send + 'static + FnOnce() -> T,
              T: Send + 'static
    {
        trace!(log, "spawning fiber");

        let mut f = UnsafeCell::new(Some(
            boxfnonce::SendBoxFnOnce::from(
                move |mut t: context::Transfer| {
                    f()
                })
        ));

        extern "C" fn context_function(mut t: context::Transfer) -> ! {

            let f = {
                let cell : &mut UnsafeCell<Option<boxfnonce::SendBoxFnOnce<(context::Transfer,),()>>> = unsafe { mem::transmute(t.data) };
                unsafe { (*cell.get()).take() }.unwrap()
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

        trace!(log, "fiber created");
        Fiber {
            cur_context: t.context,
            stack: stack,
            log: log.clone(),
        }
    }

    fn resume(&mut self) {
    }
}

unsafe impl Send for Fiber {}
// }}}

// {{{ Miofib
lazy_static! {
    static ref MIOFIB: Miofib = {
        Miofib::new()
    };
}

struct Miofib {
    spawn_tx_i: AtomicUsize,
    loop_tx: Vec<LoopTx>,
    loop_join: Vec<std::thread::JoinHandle<()>>,
    log : Logger,
}



impl Miofib {
    fn new() -> Self {
        let log = miofib_logger();
        info!(log, "Creating miofib instance");
        let (txs, joins) = (0..1)
            .map(|i| {
                     let (tx, rx) = channel();
                     let mut loop_ = Loop::new(i, rx, &log);
                     let join = std::thread::spawn(move || loop_.run());
                     (tx, join)
                 })
            .unzip();

        Miofib {
            spawn_tx_i: AtomicUsize::new(0),
            loop_tx: txs,
            loop_join: joins,
            log: log,
        }
    }

    fn spawn<F, T>(&self, f: F)
        where F: Send + 'static + FnOnce() -> T,
              T: Send + 'static
    {
        let fiber = Fiber::new(f, &self.log);

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
    fn send(&self, msg: LoopMsg) {
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
        LoopRx {
            rx: rx,
            rx_registration: reg,
        }
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
    fibers : Slab<Fiber>,
    log: Logger,
}

impl Loop {
    fn new(loop_id: usize, rx: LoopRx, log: &Logger) -> Self {

        let log = log.new(o!("loop_id" => loop_id));

        trace!(log, "creating loop");
        let poll = mio::Poll::new().unwrap();

        poll.register(&rx.rx_registration,
                      mio::Token(0),
                      mio::Ready::readable(),
                      mio::PollOpt::edge());

        Loop {
            poll: poll,
            rx: rx,
            log: log,
            fibers: Slab::with_capacity(1024),
        }
    }

    fn run(&mut self) {
        let mut events = mio::Events::with_capacity(1024);

        loop {
            trace!(self.log, "poll");
            let event_num = self.poll.poll(&mut events, None).unwrap();
            trace!(self.log, "events"; "no" => event_num);


            for event in &events {
                if event.token() == mio::Token(0) &&
                    event.readiness().is_readable() {
                    trace!(self.log, "rx queue ready");
                    self.poll_queue();
                } else {
                    trace!(self.log, "different event"; "token" => event.token().0);
                }
            }
        }
    }

    fn poll_queue(&mut self) {
        while let Ok(msg) = self.rx.rx.try_recv() {
            match msg {
                LoopMsg::Spawn(fiber) => {
                    match self.fibers.insert(fiber) {
                        Ok( fib_i) => {
                            self.fibers[fib_i].resume();
                        }
                        Err(_fiber) => panic!("Run out of slab"),
                    }
                },

            }
        }
    }
}
// }}}

// {{{ API
//TODO: pub fn spawn<F, T>(f: F) -> Receiver<T>
pub fn spawn<F, T>(f: F)
    where F: FnOnce() -> T,
          F: Send + 'static,
          T: Send + 'static
{
    MIOFIB.spawn(f)
}
// }}}

// {{{ Tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        spawn(|| {
                  printerrln!("It works");
                  assert_eq!(1, 2);

              });

        thread::spawn(|| thread::sleep_ms(3000)).join().unwrap();
        spawn(|| {
                  printerrln!("It works");
                  assert_eq!(1, 2);

              });


        thread::spawn(|| thread::sleep_ms(3000)).join().unwrap();
    }
}
// }}}

// vim: foldmethod=marker foldmarker={{{,}}}
