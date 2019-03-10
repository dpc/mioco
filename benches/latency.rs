//! Latency benchmarks
#![feature(test)]
extern crate mioco;
extern crate test;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use test::Bencher;

struct SendBencher(*mut Bencher);

// Don't judge me. -- dpc
unsafe impl Send for SendBencher {}

//
// WARNING!
//
// Some pieces here are very fragile and will short-circuit on
// any spurious wakeup, eg. when previous task leaves wakeup
// call that was not received by `wait`.
//

#[bench]
fn mpsc_pingpong(b: &mut Bencher) {
    let (tx1, rx1) = mioco::sync::mpsc::channel();
    let (tx2, rx2) = mioco::sync::mpsc::channel();

    let b = SendBencher(b as *mut Bencher);

    let join1 = mioco::spawn(move || {
        let mut prev = 0;
        loop {
            tx2.send(prev + 1).unwrap();
            match rx1.recv() {
                Ok(x) => prev = x,
                Err(_) => break,
            }
        }
    });

    let join2 = mioco::spawn(move || {
        let b = unsafe { &mut *b.0 };
        b.iter(|| {
            let x = rx2.recv().unwrap();
            tx1.send(x + 1).unwrap();
        });
    });

    join1.join().unwrap();
    join2.join().unwrap();
}

#[bench]
fn notify_pingpong(b: &mut Bencher) {
    let (tx1, rx1) = mioco::sync::notify::channel();
    let (tx2, rx2) = mioco::sync::notify::channel();

    let b = SendBencher(b as *mut Bencher);

    let finished = Arc::new(AtomicBool::new(false));
    let join1 = mioco::spawn({
        let finished = finished.clone();
        move || {
            while !finished.load(Ordering::SeqCst) {
                rx1.reset();
                tx2.notify();
                rx1.wait();
            }
        }
    });

    let join2 = mioco::spawn(move || {
        let b = unsafe { &mut *b.0 };
        b.iter(|| {
            rx2.wait();
            rx2.reset();
            tx1.notify();
        });
        rx2.wait();
        finished.store(true, Ordering::SeqCst);
        tx1.notify();
    });

    join1.join().unwrap();
    join2.join().unwrap();
}

#[bench]
fn broadcast_pingpong(b: &mut Bencher) {
    let (tx1, rx1) = mioco::sync::broadcast::channel();
    let (tx2, rx2) = mioco::sync::broadcast::channel();

    let b = SendBencher(b as *mut Bencher);

    let finished = Arc::new(AtomicBool::new(false));

    let join1 = mioco::spawn({
        let finished = finished.clone();
        move || {
            while !finished.load(Ordering::SeqCst) {
                rx1.reset();
                tx2.notify();
                rx1.wait();
            }
        }
    });

    let join2 = mioco::spawn(move || {
        let b = unsafe { &mut *b.0 };
        b.iter(|| {
            rx2.wait();
            rx2.reset();
            tx1.notify();
        });
        rx2.wait();
        finished.store(true, Ordering::SeqCst);
        tx1.notify();
    });

    join1.join().unwrap();
    join2.join().unwrap();
}
