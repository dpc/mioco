extern crate mio;
#[macro_use]
extern crate lazy_static;
extern crate context;
#[macro_use]
extern crate slog;
extern crate slog_term;
extern crate slog_async;
extern crate slab;
extern crate num_cpus;

use slog::{Logger, Drain};

use std::{mem, thread, io, panic};
use std::cell::{Cell, UnsafeCell, RefCell};

use std::sync::{mpsc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};

use context::stack;
use slab::Slab;

mod thunk;
use thunk::Thunk;

pub mod net;
pub mod sync;

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

fn mioco_logger() -> Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
//        let drain = slog_async::Async::new(drain).build().fuse();

    let drain = std::sync::Mutex::new(drain).fuse();
    slog::Logger::root(drain, o!("mioco" => env!("CARGO_PKG_VERSION") ))
}



// }}}

// {{{ Mioco
lazy_static! {
    static ref MIOCO: Mioco = {
        Mioco::new()
    };
}

struct Mioco {
    spawn_tx_i: AtomicUsize,
    loop_tx: Vec<LoopTx>,
    loop_join: Vec<thread::JoinHandle<()>>,
    log: Logger,
    polls: Vec<mio::Poll>,
    num_threads: usize,
}

impl Drop for Mioco {
    fn drop(&mut self) {
        self.loop_join
            .drain(..)
            .map(|join| { let _ = join.join(); })
            .count();
    }
}


impl Mioco {
    fn new() -> Self {
        let log = mioco_logger();
        let num_threads = num_cpus::get();
        debug!(log, "Creating mioco instance");
        let (txs, mut joins_and_polls): (_, Vec<(_, _)>) = (0..num_threads)
            .map(|i| {
                     let (tx, rx) = channel();
                     let (mut loop_, mio_loop) = Loop::new(LoopId(i), rx, &log);
                     let join = thread::spawn(move || loop_.run());
                     (tx, (join, mio_loop))
                 })
            .unzip();

        let (joins, polls) = joins_and_polls.drain(..).unzip();

        Mioco {
            spawn_tx_i: AtomicUsize::new(0),
            loop_tx: txs,
            loop_join: joins,
            polls: polls,
            log: log,
            num_threads: num_threads,
        }
    }

    fn poll(&self, id: LoopId) -> &mio::Poll {
        &self.polls[id.0]
    }

    fn spawn<F, T>(&self, f: F) -> JoinHandle<T>
        where F: Send + 'static + FnOnce() -> T,
              T: Send + 'static
    {
        let (tx, rx) = mpsc::channel();
        let fiber = Fiber::new(f, tx, &self.log);

        let i = self.spawn_tx_i.fetch_add(1, Ordering::Relaxed);
        let i = i % self.loop_tx.len();

        self.loop_tx[i].send(LoopMsg::Spawn(fiber));

        JoinHandle {
            rx: rx,
        }
    }

    fn wake(&self, id : FullId) {
        if id.loop_.0 != std::usize::MAX {
            MIOCO.loop_tx[id.loop_.0].send(LoopMsg::Wake(id.fiber));
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
struct FullId {
    loop_: LoopId,
    fiber: FiberId,
}

fn get_cur_fullid() -> FullId {

    let cur_loop= TL_LOOP_ID.with(|id| id.get());
    let cur_fiber = TL_FIBER_ID.with(|id| id.get());
    FullId {
        loop_: cur_loop,
        fiber: cur_fiber,
    }
}

// }}}

// {{{ Fiber
const TL_FIBER_ID_NONE : FiberId = FiberId(-1isize as usize);

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
struct FiberId(usize);

impl slog::Value for FiberId {
    fn serialize(&self,
                 record: &slog::Record,
                 key: slog::Key,
                 serializer: &mut slog::Serializer)
                 -> slog::Result {
                     self.0.serialize(record, key, serializer)
                 }
}

thread_local! {
    static TL_CUR_TRANSFER: RefCell<Option<context::Transfer>> = RefCell::new(None);
}

fn save_transfer(t: context::Transfer) {
    TL_CUR_TRANSFER.with(|cur_t| {
        let mut cur_transfer = cur_t.borrow_mut();
        debug_assert!(cur_transfer.is_none());

        *cur_transfer = Some(t);
    });
}

fn pop_transfer() -> context::Transfer {
    TL_CUR_TRANSFER.with(|cur_t| {
        let mut cur_transfer = cur_t.borrow_mut();

        cur_transfer.take().expect("pop_transfer")
     })
}

fn co_switch_out() {
    let t = pop_transfer().context.resume(0);
    save_transfer(t);
}

thread_local! {
    static TL_FIBER_ID: Cell<FiberId> =
        Cell::new(TL_FIBER_ID_NONE);
}


struct Fiber {
    cur_context: Option<context::Context>,
    _stack: stack::ProtectedFixedSizeStack,
    finished: bool,
}

extern "C" fn context_function(t: context::Transfer) -> ! {
    {
        let f: Thunk<'static> = {
            let cell : &RefCell<Option<Thunk<'static>>> = unsafe { mem::transmute(t.data) };
            cell.borrow_mut().take().unwrap()
        };

        let t = t.context.resume(0);

        save_transfer(t);

        let _res = f.invoke(());
    }

    loop {
        save_transfer(pop_transfer().context.resume(1));
    }
}

impl Fiber {
    fn new<F, T>(f: F, exit_sender : mpsc::Sender<std::thread::Result<T>>, log: &Logger) -> Self
        where F:  Send + 'static + FnOnce() -> T,
              T: Send + 'static
    {
        trace!(log, "spawning fiber");

        // Workaround for Box<FnOnce> not working
        let f : RefCell<Option<Thunk<'static>>> =
            RefCell::new(Some(Thunk::new(move || {

            let res = panic::catch_unwind(panic::AssertUnwindSafe(move || {
                f()
            }));

            match res {
                Ok(res) => {
                    let _ = exit_sender.send(Ok(res));
                }
                Err(cause) => {
                    let _ = exit_sender.send(Err(cause));
                }
            }
            })));

        let stack = stack::ProtectedFixedSizeStack::default();

        let context = context::Context::new(&stack, context_function);
        let t = context.resume(&f as *const _ as usize);
        debug_assert!(f.borrow().is_none());

        trace!(log, "fiber created");
        Fiber {
            cur_context: Some(t.context),
            _stack: stack,
            finished: false,
        }
    }

    fn resume(&mut self, loop_id: LoopId, fiber_id: FiberId) {
        TL_LOOP_ID.with(|id| id.set(loop_id));
        TL_FIBER_ID.with(|id| id.set(fiber_id));
        let t = self.cur_context.take().unwrap().resume(0);
        self.cur_context = Some(t.context);
        TL_LOOP_ID.with(|id| id.set(TL_LOOP_ID_NONE));
        TL_FIBER_ID.with(|id| id.set(TL_FIBER_ID_NONE));

        if t.data == 1 {
            self.finished = true;
        }
    }

    fn is_finished(&self) -> bool {
        self.finished
    }
}

// }}}

// {{{ LoopChannel
enum LoopMsg {
    Spawn(Fiber),
    Wake(FiberId),
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
const TL_LOOP_ID_NONE : LoopId = LoopId(-1isize as usize);

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
struct LoopId(usize);

impl slog::Value for LoopId {
    fn serialize(&self,
                 record: &slog::Record,
                 key: slog::Key,
                 serializer: &mut slog::Serializer)
                 -> slog::Result {
                     self.0.serialize(record, key, serializer)
                 }
}

thread_local! {
    static TL_LOOP_ID: Cell<LoopId> =
        Cell::new(TL_LOOP_ID_NONE);
    static TL_LOOP_LOG: UnsafeCell<Logger> =
        UnsafeCell::new(Logger::root(slog::Discard, o!()));
}


/// Event loop on a given thread
struct Loop {
    id: LoopId,
    rx: LoopRx,
    fibers: Slab<Fiber>,
    log: Logger,
}

const QUEUE_TOKEN: usize = std::usize::MAX - 1;

impl Loop {
    fn new(id: LoopId, rx: LoopRx, log: &Logger) -> (Self, mio::Poll) {

        let log = log.new(o!("loop-id" => id));

        trace!(log, "creating loop");
        let poll = mio::Poll::new().unwrap();

        poll.register(&rx.rx_registration,
                      mio::Token(QUEUE_TOKEN),
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

        TL_LOOP_LOG.with(|log| unsafe { *log.get() = self.log.clone() });
        loop {
            trace!(self.log, "poll");
            let event_num = MIOCO.poll(self.id).poll(&mut events, None).unwrap();
            trace!(self.log, "events"; "no" => event_num);

            for event in &events {
                let token = event.token().0;
                trace!(self.log, "received token"; "token" => token);
                if token == QUEUE_TOKEN && event.readiness().is_readable() {
                    self.poll_queue();
                } else {
                    if self.fibers.contains(token) {
                        self.resume_fib(FiberId(token))
                    }
                }
            }
        }
    }

    fn poll_queue(&mut self) {
        loop {
            match self.rx.rx.try_recv() {
                Ok(msg) => match msg {
                    LoopMsg::Spawn(fiber) => {
                        match self.fibers.insert(fiber) {
                            Ok(fib_i) => {
                                trace!(self.log, "fiber spawned"; "fiber-id" => fib_i);
                                self.resume_fib(FiberId(fib_i));
                            }
                            Err(_fiber) => panic!("Ran out of slab"),
                        }
                    },
                    LoopMsg::Wake(fiber_id) => {
                        if self.fibers.contains(fiber_id.0) {
                            self.resume_fib(fiber_id);
                        }
                    }
                },
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(e) => {
                    error!(self.log, "queue recv failed"; "err" => %e);
                    panic!("queue recv failed");
                }
            }
        }
    }

    fn resume_fib(&mut self, fib_i: FiberId) {
        trace!(self.log, "fiber resuming"; "fiber-id" => fib_i);
        self.fibers[fib_i.0].resume(self.id, fib_i);
        trace!(self.log, "fiber suspended"; "fiber-id" => fib_i);
        if self.fibers[fib_i.0].is_finished() {
            trace!(self.log, "fiber finished"; "fiber-id" => fib_i);
            self.fibers.remove(fib_i.0);
        }
    }
}
// }}}

// {{{ Evented
// TODO: Is this trait even needed? What's the point? Select maybe?
trait Evented {
    fn notify_on(&self, interest: mio::Ready);
    fn block_on(&self, interest: mio::Ready) {
        self.notify_on(interest);
        co_switch_out();
    }
}

pub struct AsyncIO<T>
    where T: mio::Evented
{
    io: T,
    registered_on: RefCell<Option<(LoopId, FiberId, mio::Ready)>>,
}

impl<T> AsyncIO<T>
    where T: mio::Evented
{
    pub fn new(t: T) -> Self {
        AsyncIO {
            io: t,
            registered_on: RefCell::new(None),
        }
    }
}

impl<T> Evented for AsyncIO<T>
    where T: mio::Evented
{
    // TODO: Handle out-of loop condition (cur_loop == -1?)
    fn notify_on(&self, interest: mio::Ready) {
        let cur_fiber = TL_FIBER_ID.with(|id| id.get());
        let cur_loop = TL_LOOP_ID.with(|id| id.get());
        let log: &Logger = TL_LOOP_LOG.with(|log| unsafe { &*log.get() as &Logger });
        let registered_on = *self.registered_on.borrow();
        trace!(log, "notify on"; "fiber-id" => cur_fiber, "interest" =>
               ?interest);
        match registered_on {
            Some((my_loop, my_fiber, my_readiness)) => {
                if cur_loop == my_loop {
                    if (cur_fiber, interest) != (my_fiber, my_readiness) {
                        trace!(log, "reregister"; "fiber-id" => cur_fiber, "interest" => ?interest);
                        MIOCO
                            .poll(cur_loop)
                            .reregister(&self.io,
                                        mio::Token(my_fiber.0),
                                        interest,
                                        mio::PollOpt::edge())
                            .unwrap();
                        *self.registered_on.borrow_mut() = Some((cur_loop, cur_fiber, interest));
                    }
                } else {
                    trace!(log, "deregister"; "fiber-id" => cur_fiber,
                           "interest" => ?interest,
                           "old-fiber-id" => my_fiber,
                           "old-loop" => my_loop
                           );
                    MIOCO.poll(my_loop).deregister(&self.io).unwrap();
                    trace!(log, "register"; "fiber-id" => cur_fiber, "interest" => ?interest);
                    MIOCO
                        .poll(cur_loop)
                        .register(&self.io,
                                  mio::Token(cur_fiber.0),
                                  interest,
                                  mio::PollOpt::edge())
                        .unwrap();
                    *self.registered_on.borrow_mut() = Some((cur_loop, cur_fiber, interest));
                }
            }
            None => {
                trace!(log, "register"; "fiber-id" => cur_fiber, "interest" => ?interest);
                MIOCO
                    .poll(cur_loop)
                    .register(&self.io,
                              mio::Token(cur_fiber.0),
                              interest,
                              mio::PollOpt::edge())
                    .unwrap();
                *self.registered_on.borrow_mut() = Some((cur_loop, cur_fiber, interest));
            }
        }

    }
}

impl<MT> io::Read for AsyncIO<MT>
    where MT: mio::Evented + io::Read + 'static
{
    /// Block on read.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let in_coroutine = in_coroutine();

        loop {
            let res = self.io.read(buf);

            if !in_coroutine {
                return res;
            }

            match res {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.block_on(mio::Ready::readable())
                }
                res => {
                    return res;
                }
            }
        }
    }
}

impl<MT> io::Write for AsyncIO<MT>
    where MT: mio::Evented + 'static + io::Write
{
    /// Block on write.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let in_coroutine = in_coroutine();


        loop {
            let res = self.io.write(buf);

            if !in_coroutine {
                return res;
            }

            match res {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.block_on(mio::Ready::writable())
                }
                res => {
                    return res;
                }
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        let in_coroutine = in_coroutine();

        loop {
            let res = self.io.flush();

            if !in_coroutine {
                return res;
            }

            match res {
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    self.block_on(mio::Ready::writable())
                }
                res => {
                    return res;
                }
            }
        }
    }
}



// }}}

// {{{ API
/// Spawn a mioco coroutine that executes the given function.
///
/// If called inside an existing mioco instance - spawn and run a new coroutine
/// in it.
///
/// If called outside of existing mioco instance - spawn a new mioco instance
/// in a separate thread or use existing mioco instance to run new mioco
/// coroutine. The API intention is to guarantee:
///
/// * This function does not block
/// * The coroutine will execute in a mioco instance
///
/// the details on reusing existing mioco instances might change.
///
/// Any panics in the given function are caught
/// and result in an `Err` that is available in the `JoinHandle`.
pub fn spawn<F, T>(f: F) -> JoinHandle<T>
    where F: FnOnce() -> T,
          F: Send + 'static,
          T: Send + 'static
{
    MIOCO.spawn(f)
}

/// Yield execution of the current coroutine.
///
/// Coroutine can yield execution without blocking on anything
/// particular to allow scheduler to run other coroutines before
/// resuming execution of the current one.
pub fn yield_now() {
    if in_coroutine() {
        let cur_fiber = TL_FIBER_ID.with(|id| id.get());
        let cur_loop = TL_LOOP_ID.with(|id| id.get());
        MIOCO.loop_tx[cur_loop.0].send(LoopMsg::Wake(cur_fiber));
        co_switch_out();
    } else {
        std::thread::yield_now();
    }
}

/// Check if running inside a mioco coroutine.
///
/// Returns true when executing inside a mioco coroutine, false otherwise.
pub fn in_coroutine() -> bool {
    TL_LOOP_ID.with(|id| id.get() != TL_LOOP_ID_NONE)
}

/// Join handle for fiber result
///
/// Modeled after `std::thread::JoinHandle`.
pub struct JoinHandle<T> {
    rx : mpsc::Receiver<std::thread::Result<T>>,
}

impl<T> JoinHandle<T> {
    pub fn join(&self) -> thread::Result<T> {
        self.rx.recv().expect("Fiber should sent a result")
    }
}

/// Get number of threads of current mioco instance.
///
/// Get number of threads of the Mioco instance that the current coroutine
/// is running in.
///
/// This is useful eg. for load balancing: spawning as many coroutines as
/// there is handling threads that can run them.
pub fn thread_num() -> usize {
    MIOCO.num_threads
}
// }}}

// {{{ Tests
#[cfg(test)]
mod tests {
}
// }}}

// vim: foldmethod=marker foldmarker={{{,}}}
