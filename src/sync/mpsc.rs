use super::super::{in_coroutine};

use std::sync::mpsc;

use super::notify;

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

    let (n_tx, n_rx) = notify::notify_channel();
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
