use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::Connection;

use super::{Limits, Pool, PoolError};

const AN_HOUR: Duration = Duration::from_secs(60 * 60);

fn limits(ceiling: usize, wait: Duration) -> Limits {
    Limits {
        ceiling,
        wait,
        unused_for: AN_HOUR,
    }
}

/// A pool over databases in memory, counting how many it has opened.
fn counted(limits: Limits) -> (Pool, Arc<AtomicUsize>) {
    let opened = Arc::new(AtomicUsize::new(0));
    let counting = Arc::clone(&opened);
    let pool = Pool::new(limits, move || {
        counting.fetch_add(1, Ordering::SeqCst);
        Connection::open_in_memory()
    })
    .expect("the pool opens");
    (pool, opened)
}

#[test]
fn building_the_pool_opens_one_connection_and_no_more() {
    let (pool, opened) = counted(limits(12, Duration::from_millis(50)));

    assert_eq!(opened.load(Ordering::SeqCst), 1);
    assert_eq!(pool.open_connections(), 1);
}

#[test]
fn a_file_that_cannot_be_opened_is_refused_when_the_pool_is_built() {
    let refused = Pool::new(limits(4, Duration::from_millis(50)), || {
        Connection::open("/a folder that is not there/file.sqlite3")
    });

    assert!(matches!(refused, Err(PoolError::Open(_))));
}

#[test]
fn a_connection_given_back_is_the_one_handed_out_next() {
    let (pool, opened) = counted(limits(4, Duration::from_millis(50)));
    {
        let first = pool.get().expect("a connection");
        // A table of this connection's own: another connection to memory would not have it.
        first
            .execute_batch("CREATE TEMP TABLE mine (a INTEGER);")
            .expect("the table is made");
    }

    let next = pool.get().expect("a connection");

    next.execute_batch("SELECT a FROM mine;")
        .expect("the same connection came back");
    assert_eq!(opened.load(Ordering::SeqCst), 1);
}

#[test]
fn a_connection_is_opened_only_while_every_kept_one_is_held() {
    let (pool, opened) = counted(limits(4, Duration::from_millis(50)));

    let first = pool.get().expect("the kept connection");
    let second = pool.get().expect("one more");

    assert_eq!(opened.load(Ordering::SeqCst), 2);
    assert_eq!(pool.open_connections(), 2);
    drop((first, second));
}

#[test]
fn past_the_ceiling_the_one_asking_waits_and_then_is_refused() {
    let wait = Duration::from_millis(80);
    let (pool, opened) = counted(limits(2, wait));
    let held = (pool.get().expect("one"), pool.get().expect("two"));

    let asked = Instant::now();
    let refused = pool.get();

    assert!(matches!(refused, Err(PoolError::TimedOut(_))));
    assert!(
        asked.elapsed() >= wait,
        "it gave up before its wait was over"
    );
    assert_eq!(opened.load(Ordering::SeqCst), 2);
    drop(held);
}

#[test]
fn one_waiting_takes_the_connection_the_moment_it_comes_back() {
    let (pool, _) = counted(limits(1, Duration::from_secs(5)));
    let held = pool.get().expect("the only connection");

    let waiting = {
        let pool = pool.clone();
        thread::spawn(move || {
            let asked = Instant::now();
            let got = pool.get().map(|_| ());
            (got, asked.elapsed())
        })
    };
    thread::sleep(Duration::from_millis(50));
    drop(held);
    let (got, took) = waiting.join().expect("the waiting thread ends");

    assert!(got.is_ok(), "the connection given back was not handed on");
    assert!(took < Duration::from_secs(2), "it waited {took:?}");
}

#[test]
fn a_connection_that_cannot_be_opened_frees_its_place() {
    let opened = Arc::new(AtomicUsize::new(0));
    let counting = Arc::clone(&opened);
    // The first and the third open, the second does not.
    let pool = Pool::new(limits(2, Duration::from_millis(50)), move || match counting
        .fetch_add(1, Ordering::SeqCst)
    {
        1 => Err(rusqlite::Error::InvalidQuery),
        _ => Connection::open_in_memory(),
    })
    .expect("the pool opens");
    let held = pool.get().expect("the kept connection");

    let refused = pool.get();
    assert!(matches!(refused, Err(PoolError::Open(_))));
    assert_eq!(pool.open_connections(), 1);

    let next = pool.get().expect("its place was free again");
    assert_eq!(pool.open_connections(), 2);
    drop((held, next));
}

#[test]
fn a_connection_given_back_inside_a_transaction_is_closed_rather_than_kept() {
    let (pool, opened) = counted(limits(4, Duration::from_millis(50)));
    {
        let left = pool.get().expect("a connection");
        left.execute_batch("BEGIN;").expect("a transaction begins");
    }

    assert_eq!(pool.open_connections(), 0);
    let next = pool.get().expect("a new connection");
    assert!(
        next.is_autocommit(),
        "it was handed out inside somebody's transaction"
    );
    assert_eq!(opened.load(Ordering::SeqCst), 2);
}

#[test]
fn connections_left_unused_are_closed_all_but_the_last() {
    let (pool, _) = counted(Limits {
        ceiling: 4,
        wait: Duration::from_millis(50),
        unused_for: Duration::ZERO,
    });
    let held = (
        pool.get().expect("one"),
        pool.get().expect("two"),
        pool.get().expect("three"),
    );
    assert_eq!(pool.open_connections(), 3);

    drop(held);

    assert_eq!(pool.open_connections(), 1);
}

#[test]
fn connections_used_lately_are_kept() {
    let (pool, _) = counted(limits(4, Duration::from_millis(50)));
    let held = (
        pool.get().expect("one"),
        pool.get().expect("two"),
        pool.get().expect("three"),
    );

    drop(held);

    assert_eq!(pool.open_connections(), 3);
}

#[test]
fn a_holder_that_panics_gives_its_connection_back() {
    let (pool, opened) = counted(limits(1, Duration::from_millis(50)));

    let panicked = {
        let pool = pool.clone();
        thread::spawn(move || {
            let _held = pool.get().expect("the only connection");
            panic!("the holder fails");
        })
        .join()
    };

    assert!(panicked.is_err());
    pool.get().expect("the connection came back");
    assert_eq!(opened.load(Ordering::SeqCst), 1);
}

#[test]
fn many_asking_at_once_share_the_ceiling_and_every_one_is_answered() {
    let (pool, opened) = counted(limits(4, Duration::from_secs(5)));
    let asking: Vec<_> = (0..8)
        .map(|_| {
            let pool = pool.clone();
            thread::spawn(move || {
                for _ in 0..50 {
                    let connection = pool.get().expect("a connection within the wait");
                    connection
                        .execute_batch("SELECT 1;")
                        .expect("the connection answers");
                }
            })
        })
        .collect();

    for one in asking {
        one.join().expect("every thread is answered");
    }

    assert!(opened.load(Ordering::SeqCst) <= 4);
    assert!(pool.open_connections() <= 4);
}
