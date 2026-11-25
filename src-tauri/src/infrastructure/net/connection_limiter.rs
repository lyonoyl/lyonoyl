//! Pauses connections to a mail server that stopped answering.
//!
//! When a connection to a server times out, is turned away, or has its handshake cut off, and the
//! server has answered no other connection since that one set out, the server is paused: every new
//! connection to it is refused here, sending nothing, until the pause ends. Then one probe
//! connection is let through, and connections that come while it is on its way wait for its answer
//! rather than knock themselves. A probe the server does not answer doubles the pause, up to
//! [`LONGEST_PAUSE`]; the first connection the server takes ends the pause for every mailbox on it.
//! Connections already open carry on. A connection a person is waiting for does not wait for the
//! pause to end: it goes as the probe, unless one is already on its way.
//!
//! Without the pause, dozens of mailboxes on one server, three tries for every failed connection,
//! and every watch reconnecting thirty seconds after it fails are about sixty connection attempts a
//! minute at a server that has stopped answering, which is how a provider's firewall keeps a
//! machine shut out for hours.
//!
//! Every mail connection to another machine goes through here, whichever of `connect`,
//! `connect_retrying` or `connect_retrying_for` opens it, as `limiter_for` in this module's parent
//! decides.

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, SystemTime};

use super::TransportError;
use crate::ports::journal::{Field, Journal, Note, Value};

/// How long a server that stopped answering is paused, the first time.
pub const FIRST_PAUSE: Duration = Duration::from_secs(30);

/// The longest a pause grows to, however long the server stays silent.
///
/// A server that comes back is found by the first connection asked for once this long has passed
/// since the last probe failed, and its mail follows.
pub const LONGEST_PAUSE: Duration = Duration::from_secs(120);

/// How long a probe is waited for, by the connections behind it and before another may be let
/// through.
///
/// Longer than a connection and its handshake can take, each at its deadline, so it only runs out
/// for a probe dropped half way, which never reports how it ended.
pub const PROBE_AT_MOST: Duration = Duration::from_secs(60);

/// How many servers are remembered before those with nothing to remember are forgotten.
const SERVERS_AT_MOST: usize = 256;

/// A server, told apart by every address its name resolved to, with the port.
///
/// Addresses rather than the name: a firewall counts what arrives at its addresses, and one
/// provider's server answers to as many names as its customers have domains.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Server(Vec<SocketAddr>);

impl Server {
    pub fn at(addresses: &[SocketAddr]) -> Self {
        let mut addresses = addresses.to_vec();
        addresses.sort_unstable();
        addresses.dedup();
        Self(addresses)
    }
}

/// Whether a person is waiting for a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiting {
    /// Nobody: a round, a watch, a push. Refused while its server is paused.
    Nobody,
    /// A person: a refresh, a message to open, a connection to test. It goes as the probe of a
    /// paused server without waiting for the pause to end, so a server that has come back is found
    /// at once, and one at a time, so walking through unread mail is not a flood either.
    Person,
}

/// What the limiter decides about a connection about to be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Open it. It carries the moment it was decided, which says whether the server answered
    /// anybody after it set out.
    Open(SystemTime),
    /// Open it, as the probe of a paused server. It carries the moment it was let through, which
    /// tells its outcome apart from a later probe's.
    Probe(SystemTime),
    /// A probe is on its way: wait for its answer, then decide again.
    Wait,
    /// Do not open it: the server is paused, and this is not the probe.
    Refuse,
}

/// How a connection attempt ended, as far as its server is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The server took the connection: it is open, or its certificate was refused here, which is
    /// a server that answered.
    Answered,
    /// The server did not answer, turned the connection away, or cut the handshake off.
    FailedThere,
    /// Nothing reached the server: no network or route here, or a refusal of this application's
    /// own before anything was sent.
    FailedHere,
}

/// The outcome of an attempt that ended in `opened`.
pub fn outcome_of<T>(opened: &Result<T, TransportError>) -> Outcome {
    match opened {
        Ok(_) | Err(TransportError::Tls) => Outcome::Answered,
        Err(TransportError::Timeout | TransportError::CutOff) => Outcome::FailedThere,
        Err(
            TransportError::NoRoute
            | TransportError::Unreachable
            | TransportError::PlaintextRefused
            | TransportError::UnsupportedEncryption,
        ) => Outcome::FailedHere,
    }
}

/// What the limiter knows of one server.
#[derive(Debug, Default, Clone, Copy)]
struct Known {
    /// Set while it is paused.
    pause: Option<Pause>,
    /// When it last took a connection, kept for [`LONGEST_PAUSE`]: a connection that set out
    /// before then and failed after it is not a server that stopped answering.
    answered: Option<SystemTime>,
}

impl Known {
    /// Whether there is anything left to remember at `now`.
    fn worth_keeping(&self, now: SystemTime) -> bool {
        self.pause.is_some()
            || self.answered.is_some_and(|answered| {
                now.duration_since(answered)
                    .is_ok_and(|ago| ago < LONGEST_PAUSE)
            })
    }
}

/// A paused server.
///
/// On the wall clock: the steady clock stops while the machine sleeps, so a pause measured on it
/// carries on after a night's sleep for whatever was left of it, and the round the waking starts is
/// refused.
#[derive(Debug, Clone, Copy)]
struct Pause {
    /// When the server stopped answering.
    since: SystemTime,
    /// When this pause began: the first failure, or the probe that failed last.
    began: SystemTime,
    /// How long this pause lasts.
    length: Duration,
    /// When the probe now on its way was let through, if one is.
    probe: Option<SystemTime>,
}

impl Pause {
    /// Whether this pause holds at `now`.
    ///
    /// A clock set back to before the pause began is outside it, so the pause ends rather than
    /// stretching by however far the clock went back.
    fn holds_at(&self, now: SystemTime) -> bool {
        self.began <= now && now < self.began + self.length
    }

    /// Whether a probe let through is still on its way at `now`, by the same rule.
    fn probing_at(&self, now: SystemTime) -> bool {
        self.probe
            .is_some_and(|let_through| let_through <= now && now < let_through + PROBE_AT_MOST)
    }
}

/// What happened to a server, which the journal writes down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The server is paused for this long: its first silence, or a probe it did not answer.
    Paused { length: Duration },
    /// The server answered again, this long after it stopped answering.
    Resumed { after: Duration },
}

/// The mail servers this process connects to, as far as there is anything to remember of them: a
/// pause, or a moment ago they answered.
#[derive(Debug, Default)]
pub struct ConnectionLimiter {
    servers: Mutex<HashMap<Server, Known>>,
    /// Woken whenever a probe says how it ended, so the connections waiting behind it decide again.
    probed: tokio::sync::Notify,
    /// How many connections to each server were refused, for a test to count.
    #[cfg(test)]
    refused: Mutex<HashMap<Server, usize>>,
}

impl ConnectionLimiter {
    /// Opens a connection to `server`, or refuses it while the server is paused and this is not
    /// the probe.
    ///
    /// `opening` is not started at all when the connection is refused, so a refusal sends nothing.
    pub async fn open<T>(
        &self,
        server: &Server,
        waiting: Waiting,
        opening: impl Future<Output = Result<T, TransportError>>,
    ) -> Result<T, TransportError> {
        let decision = match self.decide(server, waiting, SystemTime::now()) {
            Decision::Wait => self.after_the_probe(server, waiting).await,
            decided => decided,
        };
        if decision == Decision::Refuse {
            #[cfg(test)]
            {
                *locked(&self.refused).entry(server.clone()).or_default() += 1;
            }
            return Err(TransportError::Unreachable);
        }

        let opened = opening.await;
        self.record(server, decision, outcome_of(&opened), SystemTime::now());
        opened
    }

    /// Waits for the probe on its way to say how it ended, and decides again: open where the
    /// server answered it, refuse where it did not, or probe in its place where it learned nothing.
    /// A probe that never says is waited for as long as one can take.
    async fn after_the_probe(&self, server: &Server, waiting: Waiting) -> Decision {
        let deadline = tokio::time::Instant::now() + PROBE_AT_MOST;
        loop {
            // Listening before looking, so a probe that ends between the two is not missed.
            let probed = self.probed.notified();
            tokio::pin!(probed);
            probed.as_mut().enable();

            match self.decide(server, waiting, SystemTime::now()) {
                Decision::Wait => {}
                decided => return decided,
            }
            if tokio::time::timeout_at(deadline, probed).await.is_err() {
                return match self.decide(server, waiting, SystemTime::now()) {
                    Decision::Wait => Decision::Refuse,
                    decided => decided,
                };
            }
        }
    }

    /// Decides about one connection to `server`, at `now`.
    pub fn decide(&self, server: &Server, waiting: Waiting, now: SystemTime) -> Decision {
        let mut servers = locked(&self.servers);
        let Some(pause) = servers
            .get_mut(server)
            .and_then(|known| known.pause.as_mut())
        else {
            return Decision::Open(now);
        };
        if pause.probing_at(now) {
            return Decision::Wait;
        }
        if waiting == Waiting::Nobody && pause.holds_at(now) {
            return Decision::Refuse;
        }

        pause.probe = Some(now);
        Decision::Probe(now)
    }

    /// Records how a connection that was given `decision` ended, at `now`, and answers with what it
    /// changed, which the journal has written down by then.
    ///
    /// A failure pauses the server for [`FIRST_PAUSE`], unless the server answered another
    /// connection after this one set out; each probe the server does not answer doubles the pause,
    /// up to [`LONGEST_PAUSE`]. A connection opened before the pause that fails after it changes
    /// nothing: it is the same silence, heard late. Any connection the server takes ends the pause.
    pub fn record(
        &self,
        server: &Server,
        decision: Decision,
        outcome: Outcome,
        now: SystemTime,
    ) -> Option<Change> {
        let change = {
            let mut servers = locked(&self.servers);
            if servers.len() >= SERVERS_AT_MOST {
                servers.retain(|_, known| known.worth_keeping(now));
            }
            let mut known = servers.get(server).copied().unwrap_or_default();
            let change = Self::apply(&mut known, decision, outcome, now);
            match known.worth_keeping(now) {
                true => servers.insert(server.clone(), known),
                false => servers.remove(server),
            };
            change
        };
        if matches!(decision, Decision::Probe(_)) || change.is_some() {
            self.probed.notify_waiters();
        }

        if let Some(change) = change {
            let (note, event, field) = line_for(change);
            let journal = crate::infrastructure::journal::journal();
            if journal.keeps(note) {
                journal.write_down(note, event, &[field]);
            }
        }
        change
    }

    fn apply(
        known: &mut Known,
        decision: Decision,
        outcome: Outcome,
        now: SystemTime,
    ) -> Option<Change> {
        // Only the probe now on its way can lengthen the pause, or hand probing to the next
        // connection: one that outlived its time and was replaced says nothing about the probe
        // after it.
        let is_current_probe = |pause: &Pause| match decision {
            Decision::Probe(let_through) => pause.probe == Some(let_through),
            Decision::Open(_) | Decision::Wait | Decision::Refuse => false,
        };
        // Whether the server answered somebody after this connection set out, which makes its
        // failure the failure of one connection rather than a server gone quiet.
        let answered_since = |decided: SystemTime| known.answered.is_some_and(|at| at >= decided);

        match (outcome, known.pause) {
            (Outcome::Answered, pause) => {
                known.answered = Some(now);
                known.pause = None;
                pause.map(|pause| Change::Resumed {
                    after: now.duration_since(pause.since).unwrap_or_default(),
                })
            }
            (Outcome::FailedThere, None) => match decision {
                Decision::Open(decided) | Decision::Probe(decided) if !answered_since(decided) => {
                    known.pause = Some(Pause {
                        since: now,
                        began: now,
                        length: FIRST_PAUSE,
                        probe: None,
                    });
                    Some(Change::Paused {
                        length: FIRST_PAUSE,
                    })
                }
                _ => None,
            },
            (Outcome::FailedThere, Some(pause)) if is_current_probe(&pause) => {
                let length = (pause.length * 2).min(LONGEST_PAUSE);
                known.pause = Some(Pause {
                    began: now,
                    length,
                    probe: None,
                    ..pause
                });
                Some(Change::Paused { length })
            }
            // The probe learned nothing about the server, so the next connection may probe
            // instead, at once.
            (Outcome::FailedHere, Some(pause)) if is_current_probe(&pause) => {
                known.pause = Some(Pause {
                    probe: None,
                    ..pause
                });
                None
            }
            (Outcome::FailedThere | Outcome::FailedHere, _) => None,
        }
    }

    /// How many servers are paused.
    #[cfg(test)]
    pub(crate) fn paused_servers(&self) -> usize {
        locked(&self.servers)
            .values()
            .filter(|known| known.pause.is_some())
            .count()
    }

    /// How many connections to `server` this limiter has refused.
    #[cfg(test)]
    pub(crate) fn refusals_of(&self, server: &Server) -> usize {
        locked(&self.refused).get(server).copied().unwrap_or(0)
    }
}

/// The journal line a change is written down as.
///
/// A pause at the level every build writes, as a pass that stopped is, so the run in which nothing
/// arrived says why; the server answering again beside it, at the level of a milestone.
pub(super) fn line_for(change: Change) -> (Note, &'static str, Field) {
    match change {
        Change::Paused { length } => (
            Note::Trouble,
            "server.paused",
            ("for", Value::Millis(length.as_millis() as u64)),
        ),
        Change::Resumed { after } => (
            Note::Worth,
            "server.resumed",
            ("after", Value::Millis(after.as_millis() as u64)),
        ),
    }
}

/// The limiter every mail connection to another machine goes through: reading, watching, acting
/// on and filing mail over IMAP. Sending opens its own connections and is not limited.
pub fn connection_limiter() -> &'static ConnectionLimiter {
    static LIMITER: OnceLock<ConnectionLimiter> = OnceLock::new();
    LIMITER.get_or_init(ConnectionLimiter::default)
}

/// What a lock holds, whatever a thread that panicked holding it left behind: a few moments and
/// counts, where a decision read from half an update is a probe a moment early.
fn locked<T>(held: &Mutex<T>) -> MutexGuard<'_, T> {
    held.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
