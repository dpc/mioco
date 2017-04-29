extern crate mioco;

use mioco::sync::mpsc;
use std::thread;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

// NOTE: All the tests are using single MIOCO instance and
// the number of messages that reciver receives varies
// across machines. so the tests here aren't very scalable.
// on the other hand, the sleep times are generous and
// receiver should ideally receive all the messages. anything
// otherwise might help identify performance issues.

#[test]
fn tx_rx_outside_mioco() {
    let (tx, rx) = mpsc::channel::<usize>();

    thread::spawn(move || for i in 0..10 {
                      let _ = tx.send(i);
                  });

    thread::sleep_ms(1000);

    for i in 0..10 {
        assert_eq!(i, rx.recv().unwrap());
    }
}

#[test]
fn tx_outside_rx_inside_mioco() {
    let (tx, rx) = mpsc::channel::<usize>();
    // use count to check if all the messages are
    // received at receiver end
    let count = Arc::new(AtomicUsize::new(0));

    for i in 0..10 {
        let _ = tx.send(i);
    }

    let count_clone = count.clone();
    mioco::spawn(move || {
        for i in 0..10 {
            assert_eq!(i, rx.recv().unwrap());
            count_clone.store(i, Ordering::SeqCst);
        }
    });

    thread::sleep_ms(3000);
    assert_eq!(9, count.load(Ordering::SeqCst));
}

#[test]
fn tx_inside_rx_inside_mioco() {
    let (tx, rx) = mpsc::channel::<usize>();
    let count = Arc::new(AtomicUsize::new(0));
    
    mioco::spawn(move ||{
        for i in 0..10 {
            let _ = tx.send(i);
        }
    });

    let count_clone = count.clone();
    mioco::spawn(move || {
        for i in 0..10 {
            assert_eq!(i, rx.recv().unwrap());
            count_clone.store(i, Ordering::SeqCst);
        }
    });

    thread::sleep_ms(3000);
    assert_eq!(9, count.load(Ordering::SeqCst));
}

#[test]
fn sync_tx_rx_outside_mioco() {
    let (tx, rx) = mpsc::sync_channel::<usize>(5);
    let count = Arc::new(AtomicUsize::new(0));

    thread::spawn(move || for i in 0..10 {
                      let _ = tx.send(i);
                  });

    thread::sleep_ms(1000);

    let count_clone = count.clone();
    for i in 0..10 {
        assert_eq!(i, rx.recv().unwrap());
        count_clone.store(i, Ordering::SeqCst);
    }

    thread::sleep_ms(3000);
    assert_eq!(9, count.load(Ordering::SeqCst));
}

#[test]
fn sync_tx_outside_rx_inside_mioco() {
    let (tx, rx) = mpsc::sync_channel::<usize>(5);
    let count = Arc::new(AtomicUsize::new(0));

    thread::spawn(move || {
        for i in 0..10 {
            let _ = tx.send(i);
        }
    });

    let count_clone = count.clone();
    mioco::spawn(move || {
        for i in 0..10 {
            assert_eq!(i, rx.recv().unwrap());
            count_clone.store(i, Ordering::SeqCst);
        }
    });

    thread::sleep_ms(1000);
    assert_eq!(9, count.load(Ordering::SeqCst));
}

#[test]
fn sync_tx_inside_rx_inside_mioco() {
    let (tx, rx) = mpsc::sync_channel::<usize>(5);
    let count = Arc::new(AtomicUsize::new(0));

    mioco::spawn(move || {
        for i in 0..10 {
            let _ = tx.send(i);
        }
    });

    thread::sleep_ms(1000);

    let count_clone = count.clone();
    mioco::spawn(move || {
        for i in 0..10 {
            assert_eq!(i, rx.recv().unwrap());
            count_clone.store(i, Ordering::SeqCst);
        }
    });

    thread::sleep_ms(3000);
    assert_eq!(9, count.load(Ordering::SeqCst));
}
