extern crate mioco;

use mioco::sync::mpsc;
use std::thread;

#[test]
fn tx_rx_outside_mioco() {
    let (tx, rx) = mpsc::channel::<i32>();

    thread::spawn(move || for i in 0..10 {
                      let _ = tx.send(i);
                  });

    thread::sleep_ms(1000);

    for i in 0..10 {
        assert_eq!(i, rx.try_recv().unwrap());
    }
}

#[test]
fn tx_outside_rx_inside_mioco() {
    let (tx, rx) = mpsc::channel::<i32>();
    for i in 0..10 {
        let _ = tx.send(i);
    }

    mioco::spawn(move || {
        for i in 0..10 {
            assert_eq!(i, rx.try_recv().unwrap());
        }
    });

    thread::sleep_ms(1000);
}

#[test]
fn tx_inside_rx_inside_mioco() {
    let (tx, rx) = mpsc::channel::<i32>();
    
    mioco::spawn(move ||{
        for i in 0..10 {
            let _ = tx.send(i);
        }
    });

    mioco::spawn(move || {
        for i in 0..10 {
            assert_eq!(i, rx.try_recv().unwrap());
        }
    });

    thread::sleep_ms(1000);
}
