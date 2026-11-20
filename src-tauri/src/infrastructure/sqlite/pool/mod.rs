//! The connections kept open to one database file, each handed to one holder at a time.
//!
//! Opening a connection is not cheap here: the key is applied and the file's settings are set
//! before it answers anything. So a connection given back is kept for whoever asks next, and a new
//! one is opened only while every kept one is in use and the pool is under its ceiling; past the
//! ceiling, the one asking waits for a connection to come back, for as long as the pool was told
//! to wait. The first connection is opened when the pool is built, so a file that cannot be opened
//! says so at once rather than at the first question.
//!
//! What the pool keeps is kept from growing for good: a connection given back while others have
//! sat unused for longer than the pool was told closes the longest unused of them, all but the last.
//! A burst of reading leaves no crowd of open files behind it.

use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rusqlite::Connection;

#[cfg(test)]
mod tests;

/// How the pool opens one more connection to its file, ready to answer.
type Open = dyn Fn() -> rusqlite::Result<Connection> + Send + Sync;

/// Why no connection was handed out.
#[derive(Debug, thiserror::Error)]
pub enum PoolError {
    /// Every connection was in use, and none came back while the one asking waited.
    #[error("no connection came free within {0:?}")]
    TimedOut(Duration),
    /// A new connection could not be opened.
    #[error("a connection could not be opened: {0}")]
    Open(#[from] rusqlite::Error),
}

/// How far the pool grows, how long the one asking waits, and how long an unused connection stays.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The most connections open at once, in use or kept.
    pub ceiling: usize,
    /// How long one asking waits for a connection to come back, once the ceiling is reached.
    pub wait: Duration,
    /// How long a kept connection may go unused before it is closed, all but the last.
    pub unused_for: Duration,
}

/// The connections to one file. A clone hands out the same connections.
#[derive(Clone)]
pub struct Pool {
    shared: Arc<Shared>,
}

struct Shared {
    open: Box<Open>,
    limits: Limits,
    state: Mutex<State>,
    came_back: Condvar,
}

struct State {
    /// The connections nobody holds, the longest unused first.
    kept: Vec<Kept>,
    /// Every connection that exists, held or kept, and every one being opened.
    open: usize,
}

struct Kept {
    connection: Connection,
    since: Instant,
}

impl Pool {
    /// Builds the pool over a way of opening its file, and opens the first connection.
    pub fn new(
        limits: Limits,
        open: impl Fn() -> rusqlite::Result<Connection> + Send + Sync + 'static,
    ) -> Result<Self, PoolError> {
        let first = open()?;

        Ok(Self {
            shared: Arc::new(Shared {
                open: Box::new(open),
                limits: Limits {
                    ceiling: limits.ceiling.max(1),
                    ..limits
                },
                state: Mutex::new(State {
                    kept: vec![Kept {
                        connection: first,
                        since: Instant::now(),
                    }],
                    open: 1,
                }),
                came_back: Condvar::new(),
            }),
        })
    }

    /// A connection for the one asking alone, until the handle is dropped: the one given back
    /// last, a new one while the pool is under its ceiling, or else the first to come back while
    /// it waits.
    pub fn get(&self) -> Result<Pooled, PoolError> {
        let shared = &self.shared;
        let until = Instant::now() + shared.limits.wait;
        let mut state = shared.lock();

        loop {
            if let Some(kept) = state.kept.pop() {
                return Ok(self.holding(kept.connection));
            }

            if state.open < shared.limits.ceiling {
                state.open += 1;
                drop(state);
                // Opened outside the lock: it takes long enough that everybody else asking would
                // wait behind it for nothing.
                return match (shared.open)() {
                    Ok(connection) => Ok(self.holding(connection)),
                    Err(error) => {
                        shared.lock().open -= 1;
                        shared.came_back.notify_one();
                        Err(PoolError::Open(error))
                    }
                };
            }

            let now = Instant::now();
            if now >= until {
                return Err(PoolError::TimedOut(shared.limits.wait));
            }
            state = shared
                .came_back
                .wait_timeout(state, until - now)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// How many connections exist, held or kept.
    #[cfg(test)]
    pub fn open_connections(&self) -> usize {
        self.shared.lock().open
    }

    fn holding(&self, connection: Connection) -> Pooled {
        Pooled {
            connection: Some(connection),
            shared: Arc::clone(&self.shared),
        }
    }
}

/// What the pool is set to and how many connections it holds, and nothing of what they hold.
impl std::fmt::Debug for Pool {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.shared.lock();
        formatter
            .debug_struct("Pool")
            .field("limits", &self.shared.limits)
            .field("open", &state.open)
            .field("kept", &state.kept.len())
            .finish()
    }
}

impl Shared {
    /// The state, even after a holder panicked while it was taken: nothing here is left half
    /// changed by a panic, since nothing that can panic runs while it is taken.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes a connection back: kept for the next to ask, or closed when it was given back inside
    /// a transaction, which whoever got it next would otherwise find themselves in.
    fn take_back(&self, connection: Connection) {
        let closing = {
            let mut state = self.lock();
            let now = Instant::now();
            let mut closing = Vec::new();

            if connection.is_autocommit() {
                state.kept.push(Kept {
                    connection,
                    since: now,
                });
            } else {
                closing.push(connection);
            }

            while state.kept.len() > 1
                && now.duration_since(state.kept[0].since) >= self.limits.unused_for
            {
                closing.push(state.kept.remove(0).connection);
            }

            state.open -= closing.len();
            closing
        };

        // Closed outside the lock, and each slot it frees is somebody's to take.
        let freed = closing.len();
        drop(closing);
        if freed > 0 {
            self.came_back.notify_all();
        } else {
            self.came_back.notify_one();
        }
    }
}

/// One connection, the holder's alone until this is dropped, when it goes back to its pool.
pub struct Pooled {
    connection: Option<Connection>,
    shared: Arc<Shared>,
}

impl Deref for Pooled {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        match &self.connection {
            Some(connection) => connection,
            None => unreachable!("a pooled connection is held until it is dropped"),
        }
    }
}

impl DerefMut for Pooled {
    fn deref_mut(&mut self) -> &mut Connection {
        match &mut self.connection {
            Some(connection) => connection,
            None => unreachable!("a pooled connection is held until it is dropped"),
        }
    }
}

impl Drop for Pooled {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            self.shared.take_back(connection);
        }
    }
}
