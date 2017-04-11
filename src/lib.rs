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
use std::cell::{UnsafeCell, RefCell};

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

// {{{ Miofib
lazy_static! {
    static ref MIOFIB: Miofib = {
        Miofib::new()
    };
}

struct Miofib {
    spawn_tx_i: AtomicUsize,
    loop_tx: Vec<LoopTx>,
    loop_join: Vec<thread::JoinHandle<()>>,
    log: Logger,
    polls: Vec<mio::Poll>,
}

impl Drop for Miofib {
    fn drop(&mut self) {
        self.loop_join
            .drain(..)
            .map(|join| { let _ = join.join(); })
            .count();
    }
}


impl Miofib {
    fn new() -> Self {
        let log = miofib_logger();
        info!(log, "Creating miofib instance");
        let (txs, mut joins_and_polls): (_, Vec<(_, _)>) = (0..1)
            .map(|i| {
                     let (tx, rx) = channel();
                     let (mut loop_, mio_loop) = Loop::new(i, rx, &log);
                     let join = thread::spawn(move || loop_.run());
                     (tx, (join, mio_loop))
                 })
            .unzip();

        let (joins, polls) = joins_and_polls.drain(..).unzip();

        Miofib {
            spawn_tx_i: AtomicUsize::new(0),
            loop_tx: txs,
            loop_join: joins,
            polls: polls,
            log: log,
        }
    }

    fn poll(&self, i: usize) -> &mio::Poll {
        &self.polls[i]
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

// {{{ Fiber
thread_local! {
    // TODO: UnsafeCell is most probably redundant with RefCell here
    pub static TL_CUR_TRANSFER: UnsafeCell<RefCell<Option<context::Transfer>>> = UnsafeCell::new(RefCell::new(None));
}

fn save_transfer(t: context::Transfer) {
    TL_CUR_TRANSFER.with(|cur_t| {
        let cur_transfer : &mut RefCell<Option<context::Transfer>> = unsafe { &mut *cur_t.get() } ;
        let mut cur_transfer = cur_transfer.borrow_mut();
        debug_assert!(cur_transfer.is_none());

        *cur_transfer = Some(t);
    })
}

fn pop_transfer() -> context::Transfer {
    TL_CUR_TRANSFER.with(|cur_t| {
        let cur_transfer : &mut RefCell<Option<context::Transfer>> = unsafe { &mut *cur_t.get() } ;
        let mut cur_transfer = cur_transfer.borrow_mut();

        cur_transfer.take().expect("pop_transfer")
    })
}

struct Fiber {
    cur_context: Option<context::Context>,
    stack: stack::ProtectedFixedSizeStack,
    log: Logger,
}

impl Fiber {
    fn new<F, T>(f: F, log: &Logger) -> Self
        where F: Send + 'static + FnOnce() -> T,
              T: Send + 'static
    {
        trace!(log, "spawning fiber");

        // Workaround for Box<FnOnce> not working
        let mut f = UnsafeCell::new(Some(boxfnonce::SendBoxFnOnce::from(move || f())));

        extern "C" fn context_function(t: context::Transfer) -> ! {

            let f = {
                let cell: &mut UnsafeCell<Option<boxfnonce::SendBoxFnOnce<(), ()>>> =
                    unsafe { mem::transmute(t.data) };
                unsafe { (*cell.get()).take() }.unwrap()
            };

            let t = t.context.resume(0);

            save_transfer(t);

            f.call();

            pop_transfer().context.resume(1);

            unreachable!();
        }

        let stack = stack::ProtectedFixedSizeStack::default();

        let context = context::Context::new(&stack, context_function);
        let t = context.resume(&mut f as *mut _ as usize);
        debug_assert!(unsafe { (&*f.get()) }.is_none());

        trace!(log, "fiber created");
        Fiber {
            cur_context: Some(t.context),
            stack: stack,
            log: log.clone(),
        }
    }

    fn resume(&mut self) {
        let t = self.cur_context.take().unwrap().resume(0);
        self.cur_context = Some(t.context);
    }
}

unsafe impl Send for Fiber {}
// }}}

// {{{ LoopChannel
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
        self.ctrl.set_readiness(mio::Ready::readable()).unwrap();
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
    id: usize,
    rx: LoopRx,
    fibers: Slab<Fiber>,
    log: Logger,
}

impl Loop {
    fn new(id: usize, rx: LoopRx, log: &Logger) -> (Self, mio::Poll) {

        let log = log.new(o!("loop-id" => id));

        trace!(log, "creating loop");
        let poll = mio::Poll::new().unwrap();

        poll.register(&rx.rx_registration,
                      mio::Token(0),
                      mio::Ready::readable(),
                      mio::PollOpt::edge())
            .unwrap();

        (Loop {
             id: id,
             rx: rx,
             log: log,
             fibers: Slab::with_capacity(1024),
         },
         poll)
    }

    fn run(&mut self) {
        let mut events = mio::Events::with_capacity(1024);

        loop {
            trace!(self.log, "poll");
            let event_num = MIOFIB.poll(self.id).poll(&mut events, None).unwrap();
            trace!(self.log, "events"; "no" => event_num);

            for event in &events {
                if event.token() == mio::Token(0) && event.readiness().is_readable() {
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
                        Ok(fib_i) => {
                            trace!(self.log, "resuming"; "fiber-id" => fib_i);
                            self.fibers[fib_i].resume();
                        }
                        Err(_fiber) => panic!("Run out of slab"),
                    }
                }

            }
        }
    }
}
// }}}

// {{{ Evented
thread_local! {
    // TODO: UnsafeCell is most probably redundant with RefCell here
    pub static TL_FIBER_ID: UnsafeCell<usize> =
        UnsafeCell::new(-1isize as usize);
    pub static TL_LOOP_ID: UnsafeCell<usize> =
        UnsafeCell::new(-1isize as usize);
}



pub trait Evented {
    fn notify_on(&mut self, interest: mio::Ready);
}

//impl<T> Evented for T where T: mio::Evented {}

struct AsyncIO<T>
    where T: mio::Evented
{
    io: T,
    registered_on: Option<(usize, usize, mio::Ready)>,
}

impl<T> Evented for AsyncIO<T>
    where T: mio::Evented
{
    // Handle out-of loop condition (cur_loop == -1?)
    fn notify_on(&mut self, interest: mio::Ready) {
        let cur_fiber = TL_FIBER_ID.with(|id| unsafe { *id.get() });
        let cur_loop = TL_LOOP_ID.with(|id| unsafe { *id.get() });
        match self.registered_on {
            Some((my_loop, my_fiber, my_readiness)) => {
                if cur_loop == my_loop {
                    if (cur_fiber, interest) != (my_fiber, my_readiness) {
                        MIOFIB
                            .poll(cur_loop)
                            .reregister(&self.io,
                                        mio::Token(my_fiber),
                                        interest,
                                        mio::PollOpt::oneshot())
                            .unwrap();
                        self.registered_on = Some((cur_loop, cur_fiber, interest));
                    }
                } else {
                    MIOFIB.poll(my_loop).deregister(&self.io).unwrap();
                    MIOFIB
                        .poll(cur_loop)
                        .register(&self.io,
                                  mio::Token(cur_fiber),
                                  interest,
                                  mio::PollOpt::oneshot())
                        .unwrap();
                    self.registered_on = Some((cur_loop, cur_fiber, interest));
                }
            }
            None => {
                MIOFIB
                    .poll(cur_loop)
                    .register(&self.io,
                              mio::Token(cur_fiber),
                              interest,
                              mio::PollOpt::oneshot())
                    .unwrap();
                self.registered_on = Some((cur_loop, cur_fiber, interest));
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

pub fn yield_now() {
    let t = pop_transfer().context.resume(0);
    save_transfer(t);
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
              });

        thread::spawn(|| thread::sleep_ms(3000)).join().unwrap();
        spawn(|| {
                  printerrln!("It works2");
              });


        thread::spawn(|| thread::sleep_ms(3000)).join().unwrap();
    }
}
// }}}

// vim: foldmethod=marker foldmarker={{{,}}}
