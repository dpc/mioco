use super::broadcast;
use {in_coroutine};

use std::sync as ssync;
use std::ops;

pub struct Mutex<T> {
    inner: ssync::Mutex<T>,
    broadcast_rx: ssync::Mutex<broadcast::Receiver>,
    broadcast_tx: broadcast::Sender,
}

pub struct MutexGuard<'a, T>
where T : ?Sized + 'a {
    inner: Option<ssync::MutexGuard<'a, T>>,
    broadcast_tx: broadcast::Sender,
}

impl<'a, T> Drop for MutexGuard<'a, T>
where T : ?Sized + 'a {
    fn drop(&mut self) {
        self.inner.take();
        self.broadcast_tx.notify()
    }
}

impl<'a, T> ops::Deref for MutexGuard<'a, T>
where T : ?Sized + 'a {
    type Target = T;

    fn deref(&self) -> &T{
        self.inner.as_ref().unwrap()
    }
}

impl<'a, T> ops::DerefMut for MutexGuard<'a, T>
where T : ?Sized + 'a {

    fn deref_mut(&mut self) -> &mut T{
        self.inner.as_mut().unwrap()
    }
}

impl<T> Mutex<T> {

    pub fn new(t : T) -> Self {
        let (btx, brx) = broadcast::channel();
        Mutex {
            inner: ssync::Mutex::new(t),
            broadcast_tx: btx,
            broadcast_rx: ssync::Mutex::new(brx),
        }
    }

    pub fn lock(&self) -> ssync::LockResult<MutexGuard<T>> {
        if !in_coroutine() {
            match self.inner.lock() {
                Ok(guard) =>  Ok(MutexGuard {
                    inner: Some(guard),
                    broadcast_tx: self.broadcast_tx.clone()
                }),
                Err(poison_error) => Err(ssync::PoisonError::new(
                        MutexGuard {
                            inner: Some(poison_error.into_inner()),
                            broadcast_tx: self.broadcast_tx.clone()
                        }
                        )
                                                     )
            }
        } else {
            // TODO: Optimistically defer locking and cloning
            // to after first `try_lock` error?
            let broadcast_rx = self.broadcast_rx.lock().unwrap().clone();
            loop {
                broadcast_rx.reset();

                match self.inner.try_lock() {
                    Ok(guard) => return Ok(MutexGuard {
                        inner: Some(guard),
                        broadcast_tx: self.broadcast_tx.clone()
                    }),
                    Err(ssync::TryLockError::Poisoned(poison_err)) => return Err(ssync::PoisonError::new(MutexGuard {
                        inner: Some(poison_err.into_inner()),
                        broadcast_tx: self.broadcast_tx.clone()
                    })),
                    Err(ssync::TryLockError::WouldBlock) => {
                        broadcast_rx.wait()
                    }
                }
            }
        }
    }
}


