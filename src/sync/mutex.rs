use std;
use super::broadcast;

pub struct Mutex<T> {
    element: std::sync::Mutex<T>,
    broadcast_rx: broadcast::Receiver,
    broadcast_tx: broadcast::Sender,
}

impl<T> Mutex<T> {

    pub fn new(t : T) -> Self {
        let (btx, brx) = broadcast::channel();
        Mutex {
            element: std::sync::Mutex::new(t),
            broadcast_tx: btx,
            broadcast_rx: brx,
        }
    }
    pub fn lock(&self) -> LockResult<MutexGuard<T>> {

    }
}


