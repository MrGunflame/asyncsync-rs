#![feature(test)]

extern crate test;

use std::{rc::Rc, sync::Arc};

use test::Bencher;
use tokio::{runtime::Runtime, sync::mpsc, task::LocalSet};

#[bench]
fn bench_notify_1000(b: &mut Bencher) {
    use asyncsync::Notify;

    b.iter(|| {
        runtime().block_on(async {
            let notify = Arc::new(Notify::new());
            let (tx, mut rx) = mpsc::channel(1000);

            for _ in 0..1000 {
                let notify = notify.clone();
                let tx = tx.clone();

                tokio::task::spawn(async move {
                    notify.notified().await;
                    let _ = tx.send(()).await;
                });
            }

            for _ in 0..1000 {
                notify.notify_one();
                let _ = rx.recv().await;
            }
        });
    });
}

#[bench]
fn bench_notify_local_1000(b: &mut Bencher) {
    use asyncsync::local::Notify;

    b.iter(|| {
        let tasks = LocalSet::new();

        let notify = Rc::new(Notify::new());
        let (tx, mut rx) = mpsc::channel(1000);

        for _ in 0..1000 {
            let notify = notify.clone();
            let tx = tx.clone();

            tasks.spawn_local(async move {
                notify.notified().await;
                let _ = tx.send(()).await;
            });
        }

        tasks.spawn_local(async move {
            for _ in 0..1000 {
                notify.notify_one();
                let _ = rx.recv().await;
            }
        });

        runtime_local().block_on(tasks);
    });
}

#[bench]
fn bench_tokio_notify_1000(b: &mut Bencher) {
    use tokio::sync::Notify;

    b.iter(|| {
        runtime().block_on(async {
            let notify = Arc::new(Notify::new());
            let (tx, mut rx) = mpsc::channel(1000);

            for _ in 0..1000 {
                let notify = notify.clone();
                let tx = tx.clone();

                tokio::task::spawn(async move {
                    notify.notified().await;
                    let _ = tx.send(()).await;
                });
            }

            for _ in 0..1000 {
                notify.notify_one();
                let _ = rx.recv().await;
            }
        });
    });
}

fn runtime() -> Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn runtime_local() -> Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
