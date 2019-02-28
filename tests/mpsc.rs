extern crate mioco;

use mioco::sync::mpsc;
use std::thread;

// NOTE: All the tests are using single MIOCO instance

#[test]
fn tx_rx_outside_mioco() {
    let (tx, rx) = mpsc::channel::<usize>();

    thread::spawn(move || for i in 0..10 {
                      let _ = tx.send(i);
                  });

    for i in 0..10 {
        assert_eq!(i, rx.recv().unwrap());
    }
}

#[test]
fn tx_outside_rx_inside_mioco() {
    let (tx, rx) = mpsc::channel::<usize>();
    let (sync_tx, sync_rx) = mpsc::channel();

    for i in 0..10 {
        let _ = tx.send(i);
    }

    mioco::spawn(move || {
        for i in 0..10 {
            assert_eq!(i, rx.recv().unwrap());
        }
        let _ = sync_tx.send(true);
    });

    assert_eq!(true, sync_rx.recv().unwrap());
}

#[test]
fn tx_inside_rx_inside_mioco() {
    let (tx, rx) = mpsc::channel::<usize>();
    let (sync_tx, sync_rx) = mpsc::channel();
    
    mioco::spawn(move ||{
        for i in 0..10 {
            let _ = tx.send(i);
        }
    });

    mioco::spawn(move || {
        for i in 0..10 {
            assert_eq!(i, rx.recv().unwrap());
        }
        let _ = sync_tx.send(true);
    });

    assert_eq!(true, sync_rx.recv().unwrap());
}

#[test]
fn sync_tx_rx_outside_mioco() {
    let (tx, rx) = mpsc::sync_channel::<usize>(5);
    let (sync_tx, sync_rx) = mpsc::channel();

    thread::spawn(move || for i in 0..10 {
                      let _ = tx.send(i);
                  });

    thread::sleep(std::time::Duration::from_micros(1_000));

    mioco::spawn(move || {
        for i in 0..10 {
            assert_eq!(i, rx.recv().unwrap());
        }
        let _ = sync_tx.send(true);
    });

    assert_eq!(true, sync_rx.recv().unwrap());
}

#[test]
fn sync_tx_outside_rx_inside_mioco() {
    let (tx, rx) = mpsc::sync_channel::<usize>(5);
    let (sync_tx, sync_rx) = mpsc::channel();

    thread::spawn(move || {
        for i in 0..10 {
            let _ = tx.send(i);
        }
    });

    // allow the queue to get full
    thread::sleep(std::time::Duration::from_micros(1_000));

    mioco::spawn(move || {
        for i in 0..10 {
            assert_eq!(i, rx.recv().unwrap());
        }
        let _ = sync_tx.send(true);
    });

    assert_eq!(true, sync_rx.recv().unwrap());
}

#[test]
fn sync_tx_inside_rx_inside_mioco() {
    let (tx, rx) = mpsc::sync_channel::<usize>(5);
    let (sync_tx, sync_rx) = mpsc::channel();

    mioco::spawn(move || {
        for i in 0..10 {
            let _ = tx.send(i);
        }
    });

    // allow the queue to get full
    thread::sleep(std::time::Duration::from_micros(1_000));

    mioco::spawn(move || {
        for i in 0..10 {
            assert_eq!(i, rx.recv().unwrap());
        }
        let _ = sync_tx.send(true);
    });

    assert_eq!(true, sync_rx.recv().unwrap());
}
