use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, AtomicBool, Ordering};
use {MIOCO, LoopMsg, FiberId, co_switch_out, TL_LOOP_ID, TL_FIBER_ID};
use std;

struct Shared {
    loop_id: AtomicUsize,
    fiber_id: AtomicUsize,
    is_set: AtomicBool,
}

#[derive(Clone)]
pub struct Sender {
    shared: Arc<Shared>,
}

impl Sender {
    // Notify the `Receiver`
    //
    // This does not block
    pub fn notify(&self) {
        // TODO: Optimize/fix ordering?
        if !self.shared.is_set.swap(true, Ordering::SeqCst) {
            let fiber_id = self.shared.fiber_id.load(Ordering::Relaxed);
            let loop_id = self.shared.loop_id.load(Ordering::Relaxed);
            if loop_id != std::usize::MAX {
                MIOCO.loop_tx[loop_id].send(LoopMsg::Wake(FiberId(fiber_id)));
            }
        }
    }
}


pub struct Receiver {
    shared: Arc<Shared>,
}

impl Receiver {

    pub fn reset(&self) {
        self.shared.is_set.store(false, Ordering::SeqCst)
    }

    pub fn wait(&self) {
        loop {
            let cur_fiber = TL_FIBER_ID.with(|id| id.get());
            let cur_loop = TL_LOOP_ID.with(|id| id.get());
            self.shared.fiber_id.store(cur_fiber.0, Ordering::Relaxed);
            self.shared.loop_id.store(cur_loop.0, Ordering::Relaxed);
            // TODO: Optimize/fix ordering?
            if self.shared.is_set.load(Ordering::SeqCst) {
                return;
            }
            co_switch_out();
        }
    }

    /// Try waiting
    ///
    /// returns `true` if was notified
    pub fn try_wait(&self) -> bool {
        // TODO: Optimize/fix ordering?
        self.shared.is_set.swap(false, Ordering::SeqCst)
    }
}

pub fn notify_channel() -> (Sender, Receiver) {
    let shared = Arc::new(Shared {
                              fiber_id: AtomicUsize::new(std::usize::MAX),
                              loop_id: AtomicUsize::new(std::usize::MAX),
                              is_set: AtomicBool::new(false),
                          });

    (Sender { shared: shared.clone() }, Receiver { shared: shared })
}
