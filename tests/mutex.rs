extern crate mioco;

use mioco::sync::Mutex;
use std::thread;

use std::sync::Arc;


#[test]
fn mioco_mutex_basic() {
    let counter = Arc::new(Mutex::new(0usize));

    const FIBERS_NUM :usize = 64;
    const ITER_NUM :usize = 16 * 1024;

    let mut joins = Vec::new();
    for _ in 0..FIBERS_NUM {

        let join = mioco::spawn({
            let counter = counter.clone();
            move || {

            for _ in 0..ITER_NUM {
                let mut guard = counter.lock().unwrap();
                *guard += 1
            }
        }});

        joins.push(join);
    }

    for join in joins.drain(..) {
        join.join().unwrap();
    }

    assert_eq!(*counter.lock().unwrap(), FIBERS_NUM * ITER_NUM);
}


#[test]
fn mioco_and_threads_mutex_basic() {
    let counter = Arc::new(Mutex::new(0usize));

    const FIBERS_NUM :usize = 64;
    const THREADS_NUM : usize = 64;
    const ITER_NUM : usize = 16 * 1024;

    let mut fiber_joins = Vec::new();
    for _ in 0..FIBERS_NUM {

        let join = mioco::spawn({
            let counter = counter.clone();
            move || {

            for _ in 0..ITER_NUM {
                let mut guard = counter.lock().unwrap();
                *guard += 1
            }
        }});

        fiber_joins.push(join);
    }

    let mut thread_joins = Vec::new();
    for _ in 0..THREADS_NUM {

        let join = thread::spawn({
            let counter = counter.clone();
            move || {

            for _ in 0..ITER_NUM {
                let mut guard = counter.lock().unwrap();
                *guard += 1
            }
        }});

        thread_joins.push(join);
    }

    for join in fiber_joins.drain(..) {
        join.join().unwrap();
    }

    for join in thread_joins.drain(..) {
        join.join().unwrap();
    }


    assert_eq!(*counter.lock().unwrap(), (FIBERS_NUM + THREADS_NUM) * ITER_NUM);
}
