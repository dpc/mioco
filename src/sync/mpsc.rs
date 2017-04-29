use super::super::{in_coroutine};

use std::sync::mpsc;

use super::notify;
use super::broadcast;

/// Channel receiving end
///
/// Create with `channel()`
pub struct Receiver<T>{
    rx: Option<mpsc::Receiver<T>>,
    notify_rx: notify::Receiver,
}

impl<T> Receiver<T>
    where T: 'static
{
    /// Receive `T` sent using corresponding `Sender::send()`.
    ///
    /// Will block coroutine if no elements are available.
    pub fn recv(&self) -> Result<T, mpsc::RecvError> {
        if in_coroutine() {
            loop {
                self.notify_rx.reset();

                let recv = self.try_recv();

                match recv {
                    Err(mpsc::TryRecvError::Empty) =>
                        self.notify_rx.wait(),
                    Err(mpsc::TryRecvError::Disconnected) => return Err(mpsc::RecvError),
                    Ok(t) => {
                        return Ok(t)
                    },
                }
            }
        } else {
            let recv = self.rx.as_ref().unwrap().recv();

                match recv {
                    Err(e) => Err(e),
                    Ok(t) => {
                        return Ok(t)
                    },
                }
        }
    }

    /// Try reading data from the queue.
    ///
    /// This will not block.
    pub fn try_recv(&self) -> Result<T, mpsc::TryRecvError> {
        let recv = self.rx.as_ref().unwrap().try_recv();
        match recv {
            Err(e) => Err(e),
            Ok(t) => {
                return Ok(t)
            },
        }
    }
}

/// Channel sending end
///
/// Use this inside mioco coroutines or outside of mioco itself to send data
/// asynchronously to the receiving end.
///
/// Create with `channel()`
pub struct Sender<T> {
    tx: Option<mpsc::Sender<T>>,
    notif_tx: notify::Sender,
}


impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Sender {
            tx: self.tx.clone(),
            notif_tx: self.notif_tx.clone(),
        }
    }
}


impl<T> Sender<T> {
    /// Deliver `T` to the other end of the channel.
    ///
    /// Channel behaves like a queue.
    ///
    /// This is non-blocking operation.
    pub fn send(&self, t: T) -> Result<(), mpsc::SendError<T>> {
        try!(self.tx.as_ref().unwrap().send(t));
        self.notif_tx.notify();
        Ok(())
    }
}

/// Channel receiving end
///
/// Create with `channel()`
pub struct SyncReceiver<T>{
    rx: Option<mpsc::Receiver<T>>,
    broadcast_tx: broadcast::Sender,
    broadcast_rx: broadcast::Receiver,
}

impl<T> SyncReceiver<T>
    where T: 'static
{
    /// Receive `T` sent using corresponding `Sender::send()`.
    ///
    /// Will block coroutine if no elements are available.
    pub fn recv(&self) -> Result<T, mpsc::RecvError> {
        if in_coroutine() {
            loop {
                self.broadcast_rx.reset();

                let recv = self.try_recv();

                match recv {
                    Err(mpsc::TryRecvError::Empty) => self.broadcast_rx.wait(),
                    Err(mpsc::TryRecvError::Disconnected) => return Err(mpsc::RecvError),
                    Ok(t) => {
                        self.broadcast_tx.notify();
                        return Ok(t)
                    },
                }
            }
        } else {
            let recv = self.rx.as_ref().unwrap().recv();

                match recv {
                    Err(e) => Err(e),
                    Ok(t) => {
                        return Ok(t)
                    },
                }
        }
    }

    /// Try reading data from the queue.
    ///
    /// This will not block.
    pub fn try_recv(&self) -> Result<T, mpsc::TryRecvError> {
        let recv = self.rx.as_ref().unwrap().try_recv();
        match recv {
            Err(e) => Err(e),
            Ok(t) => {
                return Ok(t)
            },
        }
    }
}

/// Channel sync sending end
///
/// Use this inside mioco coroutines or outside of mioco itself to send data
/// asynchronously to the receiving end.
///
/// Create with `sync_channel()`
pub struct SyncSender<T> {
    tx: Option<mpsc::SyncSender<T>>,
    broadcast_rx: broadcast::Receiver,
    broadcast_tx: broadcast::Sender,
}

impl<T> SyncSender<T> {
    pub fn send(&self, mut t: T) -> Result<(), mpsc::SendError<T>> {
        if in_coroutine() {
            loop {
                t = match self.try_send(t) {
                    Ok(t) => return Ok(t),
                    Err(mpsc::TrySendError::Full(t)) => {
                        self.broadcast_rx.wait();
                        t
                    },
                    Err(mpsc::TrySendError::Disconnected(v)) => return Err(mpsc::SendError(v))
                };
            }
        } else {
            // previous send is notified. so 'rx' will be
            // notified and there will be space for this
            // send at some point
            self.tx.as_ref().unwrap().send(t)?;
        }
        self.broadcast_tx.notify();
        Ok(())
    }

    /// Deliver `T` to the other end of the channel.
    ///
    /// Channel behaves like a queue.
    ///
    /// This is non-blocking operation.
    pub fn try_send(&self, t: T) -> Result<(), mpsc::TrySendError<T>> {
        self.tx.as_ref().unwrap().try_send(t)?;
        self.broadcast_tx.notify();
        Ok(())
    }
}

/// Create a channel
///
/// Channel can be used to deliver data via MPSC queue.
///
/// Channel is modeled after `std::sync::mpsc::channel()`, only
/// supporting mioco-aware sending and receiving.
///
/// When receiving end is outside of coroutine, channel will behave just
/// like `std::sync::mpsc::channel()`.
pub fn channel<T>() -> (Sender<T>, Receiver<T>) {

    let (n_tx, n_rx) = notify::channel();
    let (tx, rx) = mpsc::channel();
    (Sender {
        tx: Some(tx),
        notif_tx: n_tx,
    },
        Receiver {
            rx: Some(rx),
            notify_rx: n_rx,
        })
}

impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        {
            self.tx.take();
        }
        self.notif_tx.notify()
    }
}

pub fn sync_channel<T>(size: usize) -> (SyncSender<T>, SyncReceiver<T>) {

    let (btx, brx) = broadcast::channel();
    let (tx, rx) = mpsc::sync_channel(size);
    (   SyncSender {
            tx: Some(tx),
            broadcast_tx: btx.clone(),
            broadcast_rx: brx.clone(),
        },
        SyncReceiver {
            rx: Some(rx),
            broadcast_tx: btx,
            broadcast_rx: brx,
        }
    )
}

impl<T> Drop for SyncSender<T> {
    fn drop(&mut self) {
        {
            self.tx.take();
        }
        self.broadcast_tx.notify()
    }
}
