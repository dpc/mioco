//! Broadcast allows waking up multiple receivers at once
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicUsize, Ordering};
use {MIOCO, co_switch_out, get_cur_fullid, FullId};
use std::collections::HashSet;
use std::mem;
use std::cell::RefCell;

struct Shared {
    wake_list: Mutex<HashSet<FullId>>,
    round: AtomicUsize,
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
        let mut wake_list = self.shared.wake_list.lock().unwrap();

        let _prev_round = self.shared.round.fetch_add(1, Ordering::SeqCst);


        if wake_list.is_empty() {
            return;
        }

        let replaced_list = mem::replace(&mut *wake_list, HashSet::new());
        drop(wake_list);

        for &id in &replaced_list {

            MIOCO.wake(id)
        }
    }
}


pub struct Receiver {
    shared: Arc<Shared>,
    prev_round: RefCell<usize>,
}

impl Receiver {
    pub fn reset(&self) {
        *self.prev_round.borrow_mut() = self.shared.round.load(Ordering::SeqCst)
    }

    pub fn wait(&self) {

        let id = get_cur_fullid();
        self.shared.wake_list.lock().unwrap().insert(id);

        loop {

            // TODO: Optimize/fix ordering?
            let cur_round = self.shared.round.load(Ordering::SeqCst);

            let mut self_prev_round = self.prev_round.borrow_mut();
            if *self_prev_round != cur_round {
                *self_prev_round = cur_round;

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
        let mut self_prev_round = self.prev_round.borrow_mut();
        let prev_round = *self_prev_round;
        *self_prev_round = self.shared.round.load(Ordering::SeqCst);
        prev_round != *self_prev_round
    }
}

pub fn notify_channel() -> (Sender, Receiver) {
    let shared = Arc::new(Shared {
                              wake_list: Mutex::new(HashSet::new()),
                              round: AtomicUsize::new(0),
                          });

    (Sender { shared: shared.clone() },
     Receiver {
         shared: shared,
         prev_round: RefCell::new(0),
     })
}
