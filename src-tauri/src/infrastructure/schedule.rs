//! Reading every account on a timer, so mail arrives without anyone asking for it.
//!
//! The rules that keep a hundred accounts from becoming a hundred simultaneous connections live
//! here: how many may be talking to their servers at once, and the promise that one account has one
//! conversation at a time — not two rounds, not a round and a person pressing refresh, and not
//! either of them and a change on its way to the server. Who waits for whom is [`turn`].
//!
//! Time and tasks are the reason this is infrastructure. What it decides is small enough to read in
//! one sitting; what it depends on is not.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Semaphore;

use crate::ports::account_sync::{
    AccountChanged, AccountSync, AccountSyncError, AccountWatch, Watched, Watermark,
};
use crate::ports::journal::{Journal, Note, Value};

/// How many accounts may be talking to their servers at once.
///
/// Not one — a single slow server would then hold up every other account. Not all of them either:
/// a hundred mailboxes opening a hundred connections in the same moment is what
/// [`docs/performance/response-budgets.md`] refuses, and on battery it is the whole round spent on handshakes.
///
/// Wide enough that a person's own mailboxes do not queue behind each other. At four, ten accounts
/// were read three deep and a round lasted as long as three of them end to end — which is what it
/// looked like, because that is what it was. Past the number of mailboxes anybody actually has,
/// widening this buys nothing and costs a connection each.
pub const AT_ONCE: usize = 16;

/// How long a round waits on one account before it lets another start.
///
/// Not a cancellation, and it cannot be one: a pass is blocking work on a blocking thread, and
/// nothing here can interrupt it. What this does is stop a round *waiting* — the place in the
/// queue is given up, another account begins, and the slow one carries on to its own end. The
/// claim it holds is what keeps a second pass off it in the meantime.
///
/// It exists because the only network deadline measures silence. A server that answers with one
/// byte every half minute is never silent, so it holds a place for ever; four of them hold every
/// place there is, and the other ninety-six accounts are never read at all. Generous on purpose:
/// this is a backstop against that, not a way of sharing time out.
const A_PASS_AT_MOST: Duration = Duration::from_secs(600);

/// How long between rounds.
const BETWEEN_ROUNDS: Duration = Duration::from_secs(120);

/// How many accounts may hold a watching connection at once.
///
/// Every account, up to the hundred this application is built for. A watch is a connection held
/// open and one of the runtime's blocking threads held with it, and the runtime keeps five hundred
/// and twelve of those: a hundred watches leave four hundred for every pass and every database
/// write. An account past the hundredth is not left out — the rounds read it like any other, and
/// it hears about new mail within two minutes rather than at once.
pub const WATCHING_AT_MOST: usize = 100;

/// How many of `AT_ONCE` a round may hold at one moment.
///
/// The rest is what a person gets. The total limit is about the machine and the servers, and a
/// round that takes all of it is a round a person waits behind: the round has until the next one
/// and the person is sitting in front of the answer. Half, rounded up, so a limit of one is still
/// a round that runs.
const A_ROUND_AT_ONCE: usize = AT_ONCE - AT_ONCE / 2;

/// A round that may take every place leaves a person pressing refresh nothing to take, which is
/// the whole of what the share is for. Answered when this compiles rather than when a test runs.
const _: () = assert!(A_ROUND_AT_ONCE < AT_ONCE);

/// How long a watcher waits after a watch failed before opening another one.
///
/// A server that is down would otherwise be reconnected to in a loop, as fast as the machine can.
/// Short of a round, because what ends most watches that fail is a connection the network dropped
/// — after a sleep, or a router that forgot it — and the mail arriving meanwhile is heard only
/// once the next watch is open.
const AFTER_A_FAILED_WATCH: Duration = Duration::from_secs(30);

/// How often the wait between rounds looks at the wall clock.
const GLANCE: Duration = Duration::from_secs(5);

/// How much longer by the wall clock than it was meant to a glance may take before it is a glance
/// the machine slept through.
const ASLEEP: Duration = Duration::from_secs(10);

/// How long after a push lets go of an account its watcher takes news about it for that push coming
/// back.
///
/// A server tells every watch on a mailbox about a change to it, and that includes the change this
/// device's own push has just made: a message marked read in the inbox comes back to the inbox's
/// watch as news, and read as news it is a whole pass over the account for what the index already
/// holds. The development server tells its watch half a second after it answers the push. Five
/// seconds is ten times that, which leaves room for a server slower to tell its watchers and for
/// the round trips a watch takes to end on a slow link.
///
/// Only news that something already there changed is taken for an echo. Mail arriving never is: a
/// person going through their inbox pushes every few seconds, and a window that swallowed arrivals
/// would keep new mail off the screen for as long as they kept reading. A change from another
/// device to what was already there, landing inside the window, is read by the next round, which
/// reads every account, watched or not, within [`BETWEEN_ROUNDS`].
const A_PUSH_ECHOES_FOR: Duration = Duration::from_secs(5);

/// Why an account is being read, which decides who waits for whom and who is told about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// A person asked: the refresh button, or the first read of an account just added. The only
    /// reading the foot of the window speaks about, because it is the only reading somebody is
    /// waiting on.
    Person,
    /// Nobody asked: a round on the timer, or a server saying something already there changed.
    Background,
    /// A server said mail arrived. Nobody asked either, and the reading is the same, but whatever
    /// holds the account may have read the inbox before the mail came.
    Arrival,
    /// What a person just did, on its way to the server.
    Push,
}

/// Whether one who asks may take an account, has to wait for it, or leaves it alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Turn {
    Take,
    Wait,
    Leave,
}

/// The rule, in one place.
///
/// A push waits for anything: what a person did is never dropped, only told a moment later. A
/// pass nobody asked for waits for a push, because it would otherwise read the account before the
/// change was on the server, and leaves an account anything else is reading — the one that holds
/// it reads the account too, and a change to what was already there that it missed is read by the
/// next round. Mail arriving is not left to that: a pass that read the inbox a second before the
/// server said so does not have it, and the next round is two minutes away, so it waits for
/// whatever holds the account and reads it after. A person waits for anything but another person,
/// who is already doing what they asked.
fn turn(asking: Why, holding: Option<Why>) -> Turn {
    match (asking, holding) {
        (_, None) => Turn::Take,
        (Why::Push, Some(_)) => Turn::Wait,
        (Why::Arrival, Some(_)) => Turn::Wait,
        (Why::Background, Some(Why::Push)) => Turn::Wait,
        (Why::Background, Some(_)) => Turn::Leave,
        (Why::Person, Some(Why::Person)) => Turn::Leave,
        (Why::Person, Some(_)) => Turn::Wait,
    }
}

/// The accounts being read or pushed to right now.
///
/// Shared with whatever else can start one, so a person pressing refresh, a round coming round and
/// a change on its way to the server never open two conversations with the same account at once.
#[derive(Clone, Default)]
pub struct InFlight {
    reading: Arc<Mutex<Reading>>,
    /// Woken whenever a claim ends, so whoever is waiting for that account looks again.
    freed: Arc<tokio::sync::Notify>,
    /// Where a pass a person asked for says how far it has got, so a claim ending clears that too.
    how_far: Option<crate::infrastructure::progress::HowFarAlong>,
}

impl InFlight {
    /// The same, told where to clear a pass's progress from when the pass ends.
    pub fn reporting_to(how_far: crate::infrastructure::progress::HowFarAlong) -> Self {
        Self {
            how_far: Some(how_far),
            ..Self::default()
        }
    }
}

/// What is held, and how much a person has asked for since the last moment nobody was waiting.
///
/// `asked` is every account a person has asked about in this burst, not how many are held now. A
/// line saying "three of nine" needs both, and the total is not a thing anybody could count from a
/// database: what a person asked for is known only while it is being done.
///
/// Named rather than counted, and that is what bounds it: an account asked about twice in one
/// burst is one mailbox to read, not two.
///
/// `pushed` is when a push last let go of each account, kept only while it can still matter to a
/// watcher: for [`A_PUSH_ECHOES_FOR`].
#[derive(Default)]
struct Reading {
    accounts: HashMap<String, Why>,
    asked: HashSet<String>,
    pushed: HashMap<String, tokio::time::Instant>,
}

/// How far a burst of reading has got: how many are done, out of how many were asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HowFar {
    pub done: u32,
    pub asked: u32,
}

impl InFlight {
    /// Claims an account now, or answers `None` when something else holds it.
    pub fn claim(&self, account_id: &str, why: Why) -> Option<Claim> {
        self.try_to_claim(account_id, why).ok()
    }

    /// Claims an account, waiting for it where the rule above says to wait, or answers `None`
    /// where it says to leave it alone.
    pub async fn claim_waiting(&self, account_id: &str, why: Why) -> Option<Claim> {
        loop {
            // Listening before looking, so a claim that ends between the two is not missed.
            let freed = self.freed.notified();
            tokio::pin!(freed);
            freed.as_mut().enable();

            match self.try_to_claim(account_id, why) {
                Ok(claim) => return Some(claim),
                Err(Turn::Wait) => freed.await,
                Err(_) => return None,
            }
        }
    }

    fn try_to_claim(&self, account_id: &str, why: Why) -> Result<Claim, Turn> {
        let mut held = self.reading.lock().expect("the lock is never poisoned");

        match turn(why, held.accounts.get(account_id).copied()) {
            Turn::Take => {}
            other => return Err(other),
        }

        held.accounts.insert(account_id.to_owned(), why);
        if why == Why::Person {
            held.asked.insert(account_id.to_owned());
        }

        Ok(Claim {
            reading: Arc::clone(&self.reading),
            freed: Arc::clone(&self.freed),
            account_id: account_id.to_owned(),
            why,
            how_far: self.how_far.clone(),
        })
    }

    /// Whether news a watcher has just heard about this account is most likely this device's own
    /// push coming back: a push holds the account, or let go of it less than [`A_PUSH_ECHOES_FOR`]
    /// ago.
    ///
    /// Asked by a watcher alone, and only of news that something already there changed: mail
    /// arriving is never an echo. A round and a refresh read whenever they come, and a round is
    /// what reads a change from elsewhere that a watcher took for an echo.
    fn hears_its_own_push(&self, account_id: &str) -> bool {
        let held = self.reading.lock().expect("the lock is never poisoned");

        held.accounts.get(account_id) == Some(&Why::Push)
            || held
                .pushed
                .get(account_id)
                .is_some_and(|ended| ended.elapsed() < A_PUSH_ECHOES_FOR)
    }

    /// How many accounts are held, for whatever reason.
    pub fn count(&self) -> usize {
        self.reading
            .lock()
            .expect("the lock is never poisoned")
            .accounts
            .len()
    }

    /// How far the reading a person asked for has got.
    ///
    /// Nothing a person asked for is nothing to say: a round on the timer and a change on its way
    /// to the server are not counted, because nobody is waiting on them. `asked` goes back to
    /// nought when the last one finishes, so a line drawn from this disappears rather than sitting
    /// at nine of nine.
    pub fn how_far(&self) -> HowFar {
        let held = self.reading.lock().expect("the lock is never poisoned");
        let still = held
            .accounts
            .values()
            .filter(|why| **why == Why::Person)
            .count();

        HowFar {
            done: u32::try_from(held.asked.len() - still).unwrap_or(u32::MAX),
            asked: u32::try_from(held.asked.len()).unwrap_or(u32::MAX),
        }
    }
}

/// Held for as long as one account is being read or pushed to. Releases it however the work ends,
/// including by panic: a claim that outlived its work would silence that account until the app
/// restarts, and whoever was waiting for it would wait for ever.
pub struct Claim {
    reading: Arc<Mutex<Reading>>,
    freed: Arc<tokio::sync::Notify>,
    account_id: String,
    why: Why,
    /// Cleared with the claim, so a pass that ended any way at all leaves nothing behind.
    how_far: Option<crate::infrastructure::progress::HowFarAlong>,
}

impl Drop for Claim {
    fn drop(&mut self) {
        // The claim first, and the progress after it. Anything that can fail between taking the
        // claim and giving it back is an account nothing will read again until the application
        // restarts, which is what this type exists to prevent.
        let ended = {
            let mut held = self.reading.lock().expect("the lock is never poisoned");
            held.accounts.remove(&self.account_id);

            // A push and the read back after it both end here. What is older than the window
            // answers nothing any more, and dropping it is what keeps this from holding every
            // account ever pushed to.
            if self.why == Why::Push {
                held.pushed
                    .retain(|_, ended| ended.elapsed() < A_PUSH_ECHOES_FOR);
                held.pushed
                    .insert(self.account_id.clone(), tokio::time::Instant::now());
            }

            // The last one a person asked for ends the burst. Without this the next refresh
            // would begin against a total left over from the one before it, and the line would
            // say four of nine before anything had been read at all.
            let nobody_waits = !held.accounts.values().any(|why| *why == Why::Person);
            if nobody_waits {
                held.asked.clear();
            }
            nobody_waits
        };

        self.freed.notify_waiters();

        if self.why != Why::Person {
            return;
        }
        if let Some(how_far) = &self.how_far {
            if ended {
                how_far.nothing_is_being_read();
            } else {
                how_far.finished(&self.account_id);
            }
        }
    }
}

/// Reads one account, if nothing else is reading it, and answers with what changed.
///
/// Waits for a change on its way to the server, which would otherwise be read before the server
/// had it, and leaves an account anything else is already reading.
async fn read_one<S: AccountSync>(
    sync: Arc<S>,
    in_flight: InFlight,
    runtime: tokio::runtime::Handle,
    account_id: String,
) -> Option<(String, AccountChanged)> {
    let claim = in_flight
        .claim_waiting(&account_id, Why::Background)
        .await?;

    // The pass itself is blocking: SQLite and a synchronous protocol adapter. It belongs off the
    // runtime's worker threads, or four accounts would stall everything else.
    let outcome = runtime
        .spawn_blocking(move || {
            let changed = sync.sync(&account_id);
            drop(claim);
            (account_id, changed)
        })
        .await
        .ok()?;

    match outcome {
        (account_id, Ok(changed)) if changed.is_anything() => Some((account_id, changed)),
        _ => None,
    }
}

/// Reads every account, `AT_ONCE` of them at a time, and answers with what changed.
pub async fn one_round<S: AccountSync>(
    sync: Arc<S>,
    in_flight: InFlight,
    runtime: tokio::runtime::Handle,
    at_once: Arc<Semaphore>,
) -> Vec<(String, AccountChanged)> {
    one_round_waiting_at_most(sync, in_flight, runtime, at_once, A_PASS_AT_MOST).await
}

/// The same round, told how long to wait on one account.
///
/// The bound is an argument only so a test can drive the real thing with a real clock: pausing
/// time does not work here, because the pass is a blocking task and the runtime is never idle
/// while one is outstanding.
async fn one_round_waiting_at_most<S: AccountSync>(
    sync: Arc<S>,
    in_flight: InFlight,
    runtime: tokio::runtime::Handle,
    // Shared with the window's own way of asking, because the limit is about how many mailboxes
    // may be talking to a server at once and not about who asked. Two of them, one each, meant a
    // round and a person pressing refresh could be thirty-two conversations between them.
    permits: Arc<Semaphore>,
    at_most: Duration,
) -> Vec<(String, AccountChanged)> {
    let Ok(accounts) = sync.accounts() else {
        return Vec::new();
    };

    // What the whole round costs, beside what each pass in it costs. The two are different
    // questions: a round of ten accounts that are each quick is still slow if they queue, and the
    // per-pass line cannot say so because every pass in it looks fine.
    let began = std::time::Instant::now();
    let asked = accounts.len();

    // A round's own share of the limit above, made here because one round runs at a time. Taken
    // before the shared permit and never the other way round, so the two waits cannot cross.
    let ours = Arc::new(Semaphore::new(A_ROUND_AT_ONCE));

    let mut running = Vec::new();

    for account_id in accounts {
        let ours = Arc::clone(&ours);
        let permits = Arc::clone(&permits);
        let sync = Arc::clone(&sync);
        let in_flight = in_flight.clone();
        let runtime = runtime.clone();

        running.push(tokio::spawn(async move {
            let _share = ours.acquire().await.ok()?;
            let _permit = permits.acquire().await.ok()?;
            tokio::time::timeout(at_most, read_one(sync, in_flight, runtime, account_id))
                .await
                .ok()?
        }));
    }

    let mut changed = Vec::new();
    for task in running {
        if let Ok(Some(one)) = task.await {
            changed.push(one);
        }
    }

    let journal = crate::infrastructure::journal::journal();
    if journal.keeps(Note::Detail) {
        journal.write_down(
            Note::Detail,
            "round.finished",
            &[
                ("took", Value::Millis(began.elapsed().as_millis() as u64)),
                ("accounts", Value::Count(asked as u64)),
                ("atOnce", Value::Count(AT_ONCE as u64)),
                ("changed", Value::Count(changed.len() as u64)),
            ],
        );
    }

    changed
}

/// Runs rounds until the application stops, announcing what each one changed.
///
/// The first comes as the application starts and one every [`BETWEEN_ROUNDS`] after it, and
/// nothing moves them: what a person does reaches the server through
/// [`crate::infrastructure::pushes`], which never waits for a round and never starts one.
pub fn start<S: AccountSync>(
    sync: Arc<S>,
    in_flight: InFlight,
    runtime: tokio::runtime::Handle,
    // What every way of reading shares, because the limit is about how many mailboxes may be
    // talking to a server at once and not about who asked: a round, a person pressing refresh, a
    // push, and a watcher whose server has just spoken. A round takes at most half of it
    // (`A_ROUND_AT_ONCE`) so that a person is never behind the whole of one.
    at_once: Arc<Semaphore>,
    announce: impl Fn(&str, AccountChanged) + Send + 'static,
) {
    let spawner = runtime.clone();
    spawner.spawn(async move {
        loop {
            // Before the wait: every account is read as the application starts, and the foot of
            // the window shows it as it shows any round. The window reads on its own only an
            // account it has no folders for yet.
            let changed = one_round(
                Arc::clone(&sync),
                in_flight.clone(),
                runtime.clone(),
                Arc::clone(&at_once),
            )
            .await;

            for (account_id, changed) in changed {
                announce(&account_id, changed);
            }

            until_the_next_round(BETWEEN_ROUNDS).await;
        }
    });
}

/// Waits out the time between rounds, and ends at once when the machine wakes from sleep.
///
/// The clock a wait is measured on stops while the machine sleeps, so a wait measured on it alone
/// carries on after a wake for whatever was left of it: up to two minutes in which the mail that
/// arrived overnight is not read. Waited in glances, each held against the wall clock — a glance
/// that took much longer by the wall than it was meant to is a glance the machine slept through.
async fn until_the_next_round(wait: Duration) {
    let mut left = wait;

    while !left.is_zero() {
        let glance = left.min(GLANCE);
        let before = std::time::SystemTime::now();
        tokio::time::sleep(glance).await;

        // A clock set back meanwhile reads as no time at all, which is a glance like any other.
        let by_the_wall = std::time::SystemTime::now()
            .duration_since(before)
            .unwrap_or(glance);
        if slept_through(glance, by_the_wall) {
            return;
        }

        left -= glance;
    }
}

/// Whether a glance meant to last `glance` that took `by_the_wall` was slept through.
fn slept_through(glance: Duration, by_the_wall: Duration) -> bool {
    by_the_wall > glance + ASLEEP
}

/// What a watcher does with the answer it got.
///
/// The decision is separated from the loop deliberately: the loop runs for the life of the
/// application, and a rule tested through it can only be tested by a test that never returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NextStep {
    /// Mail arrived: read the account, then watch again.
    ReadThenWatch,
    /// Something already there changed: read the account unless it is this device's own push
    /// coming back, then watch again.
    ReadUnlessEchoThenWatch,
    /// Nothing happened while the watch lasted, which is how most of them end.
    WatchAgain,
    /// The watch failed. Wait before opening another one, or a server that is down would be
    /// reconnected to as fast as the machine can.
    WaitThenWatch,
    /// Nothing left to watch: this server cannot push, or the account is gone.
    Stop,
}

/// The rule itself.
fn step(answer: Result<Watched, AccountSyncError>) -> NextStep {
    match answer {
        Ok(Watched::Arrived) => NextStep::ReadThenWatch,
        Ok(Watched::Changed) => NextStep::ReadUnlessEchoThenWatch,
        Ok(Watched::Quiet) => NextStep::WatchAgain,
        // Nothing is wrong with such a server; the timer covers it from here.
        Ok(Watched::CannotPush) => NextStep::Stop,
        Err(_) => NextStep::WaitThenWatch,
    }
}

/// How a round, a watch or a push tells the window that an account changed.
pub type Announce = Arc<dyn Fn(&str, AccountChanged) + Send + Sync>;

/// Why a watcher ended, which is what decides whether the account is given another one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ended {
    /// The account is no longer on the device, or could not be listed at all. The slot goes back,
    /// and the account is watched again if it returns.
    Released,
    /// This server cannot push. Opening another watch on it every round would be the reconnect
    /// loop a watcher exists to avoid.
    CannotPush,
}

/// Watches one account for as long as its server can be watched.
///
/// Ends by itself in the two cases where watching is pointless: a server that cannot push, and an
/// account that is no longer on the device.
async fn watching_one<S: AccountSync, W: AccountWatch>(
    account_id: String,
    sync: Arc<S>,
    watch: Arc<W>,
    in_flight: InFlight,
    runtime: tokio::runtime::Handle,
    at_once: Arc<Semaphore>,
    announce: Announce,
) -> Ended {
    // Where the last watch ended, handed to the next one. Between the two nothing listens — the
    // news is being read, or the next connection opened — and the next watch reports what arrived
    // meanwhile the moment it looks, rather than counting it as already there.
    let mut seen: Option<Watermark> = None;

    loop {
        // An account that was removed while its watch was open has nothing left to watch.
        match sync.accounts() {
            Ok(accounts) if accounts.iter().any(|known| known == &account_id) => {}
            Ok(_) => return Ended::Released,
            Err(_) => return Ended::Released,
        }

        let watching = {
            let watch = Arc::clone(&watch);
            let account_id = account_id.clone();
            let since = seen.clone();
            runtime
                .spawn_blocking(move || watch.watch(&account_id, since.as_ref()))
                .await
        };

        // A watcher whose task itself failed is treated as a failed watch: something is wrong, and
        // the wait is what keeps it from becoming a loop. A watch that failed heard nothing, so
        // the next one takes up where the last one that ended did.
        let answer = watching
            .unwrap_or(Err(AccountSyncError::Failed))
            .map(|ended| {
                seen = ended.seen.or(seen.take());
                ended.watched
            });

        match step(answer) {
            // What this device has just pushed, coming back: the index holds it already. Judged
            // when the news arrives and not after a wait for the push, which would read it anyway.
            NextStep::ReadUnlessEchoThenWatch if in_flight.hears_its_own_push(&account_id) => {}
            NextStep::ReadThenWatch => {
                if let Some(changed) = read_now(
                    &account_id,
                    Why::Arrival,
                    &sync,
                    &in_flight,
                    &runtime,
                    &at_once,
                )
                .await
                {
                    announce(&account_id, changed);
                }
            }
            NextStep::ReadUnlessEchoThenWatch => {
                if let Some(changed) = read_now(
                    &account_id,
                    Why::Background,
                    &sync,
                    &in_flight,
                    &runtime,
                    &at_once,
                )
                .await
                {
                    announce(&account_id, changed);
                }
            }
            NextStep::WatchAgain => {}
            NextStep::WaitThenWatch => tokio::time::sleep(AFTER_A_FAILED_WATCH).await,
            NextStep::Stop => return Ended::CannotPush,
        }
    }
}

/// One pass, claimed the same way a round claims it, so a watch and a timer never read the same
/// account at once.
///
/// Behind the same limit as everything else that reads. Eight watchers each reading the moment
/// their server says something is a mailbox talking to a server eight times over, and a limit that
/// did not cover them would be a number describing some of the connections rather than all of
/// them.
///
/// `why` is [`Why::Arrival`] for mail arriving and [`Why::Background`] for anything else, and
/// [`turn`] says what each waits for.
async fn read_now<S: AccountSync>(
    account_id: &str,
    why: Why,
    sync: &Arc<S>,
    in_flight: &InFlight,
    runtime: &tokio::runtime::Handle,
    at_once: &Arc<Semaphore>,
) -> Option<AccountChanged> {
    let _permit = at_once.acquire().await.ok()?;
    // A push that took the account after the news arrived is waited out rather than the news
    // dropped: the server spoke before that push reached it, so what it said is not the push
    // coming back, and it has to be read.
    let claim = in_flight.claim_waiting(account_id, why).await?;
    let sync = Arc::clone(sync);
    let account_id = account_id.to_owned();

    let changed = runtime
        .spawn_blocking(move || {
            let changed = sync.sync(&account_id);
            drop(claim);
            changed
        })
        .await
        .ok()?
        .ok()?;

    changed.is_anything().then_some(changed)
}

/// Which accounts are being watched, and which are not worth watching.
///
/// One lock over both sets: every decision reads them together, and a slot taken between the two
/// reads is exactly the race the cap exists to prevent.
#[derive(Default)]
struct Slots {
    running: HashSet<String>,
    cannot_push: HashSet<String>,
}

#[derive(Clone, Default)]
struct Watchers(Arc<Mutex<Slots>>);

impl Watchers {
    /// Takes a slot for the account, or answers `false` when it must not be given one: it already
    /// has a watcher, its server cannot push, or every slot is taken.
    fn claim(&self, account_id: &str) -> bool {
        let mut slots = self.0.lock().expect("the lock is never poisoned");

        if slots.running.len() >= WATCHING_AT_MOST || slots.cannot_push.contains(account_id) {
            return false;
        }

        slots.running.insert(account_id.to_owned())
    }

    /// Gives the slot back, and remembers a server that cannot push so it is not asked again.
    fn release(&self, account_id: &str, ended: Ended) {
        let mut slots = self.0.lock().expect("the lock is never poisoned");

        slots.running.remove(account_id);
        if ended == Ended::CannotPush {
            slots.cannot_push.insert(account_id.to_owned());
        }
    }
}

/// Keeps a watcher running for as many accounts as may be watched at once.
///
/// Started once, and from then on it decides for itself: an account added after the application
/// started is watched from the next round, and a slot given back — by an account that was removed,
/// or by a server that turned out not to be watchable — goes to one that was waiting for it.
/// Without that the first account on a new device would go unwatched until the next launch, and on
/// a new device that is the only account there is.
pub fn watch_accounts<S: AccountSync, W: AccountWatch>(
    sync: Arc<S>,
    watch: Arc<W>,
    in_flight: InFlight,
    runtime: tokio::runtime::Handle,
    // The same one the rounds and the window take from. What a watcher does when its server speaks
    // is a pass like any other.
    at_once: Arc<Semaphore>,
    announce: impl Fn(&str, AccountChanged) + Send + Sync + 'static,
) {
    let announce: Announce = Arc::new(announce);
    let watchers = Watchers::default();
    let spawner = runtime.clone();

    spawner.spawn(async move {
        loop {
            // Before the wait, unlike a round: the accounts already on the device are watched from
            // the moment the application starts.
            fill_the_free_slots(
                &sync, &watch, &in_flight, &runtime, &at_once, &announce, &watchers,
            );

            until_the_next_round(BETWEEN_ROUNDS).await;
        }
    });
}

/// Gives a watcher to every account that should have one and does not.
fn fill_the_free_slots<S: AccountSync, W: AccountWatch>(
    sync: &Arc<S>,
    watch: &Arc<W>,
    in_flight: &InFlight,
    runtime: &tokio::runtime::Handle,
    at_once: &Arc<Semaphore>,
    announce: &Announce,
    watchers: &Watchers,
) {
    // A device that cannot be asked what it holds is asked again next round.
    let Ok(accounts) = sync.accounts() else {
        return;
    };

    for account_id in accounts {
        if !watchers.claim(&account_id) {
            continue;
        }

        // The watcher runs as a task of its own so that its slot is given back however it ends —
        // a panic in it must cost one round, not the ability to watch that account until restart.
        let watching = runtime.spawn(watching_one(
            account_id.clone(),
            Arc::clone(sync),
            Arc::clone(watch),
            in_flight.clone(),
            runtime.clone(),
            Arc::clone(at_once),
            Arc::clone(announce),
        ));
        let watchers = watchers.clone();

        runtime.spawn(async move {
            let ended = watching.await.unwrap_or(Ended::Released);
            watchers.release(&account_id, ended);
        });
    }
}

/// Work whose last request is the only one that matters: at most one running, and one behind it.
///
/// What a burst of clicking asks of anything that reads everything again — the number on the icon
/// walks every account's inbox. Started once per click, thirty clicks a second is thirty of those
/// queued on the threads a push and a pass need, each answering a question the next one replaces.
/// One behind the running one is enough for the last click to be counted.
pub struct OneBehind(Mutex<(bool, bool)>);

impl OneBehind {
    pub const fn new() -> Self {
        Self(Mutex::new((false, false)))
    }

    /// Whether the caller starts the work now. When it is already running, it is run once more
    /// when it ends, and the caller does nothing.
    pub fn asked(&self) -> bool {
        let mut state = self.0.lock().expect("the lock is never poisoned");
        match *state {
            (true, _) => {
                state.1 = true;
                false
            }
            (false, _) => {
                *state = (true, false);
                true
            }
        }
    }

    /// Said by the one running it, when a run ends: whether to run it again.
    pub fn finished(&self) -> bool {
        let mut state = self.0.lock().expect("the lock is never poisoned");
        let again = state.1;
        *state = (again, false);
        again
    }
}

impl Default for OneBehind {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    use super::*;

    /// A glance that took about as long by the wall clock as it was meant to was not slept
    /// through, and one that took far longer was; a clock set back meanwhile is not a sleep.
    #[test]
    fn a_glance_the_machine_slept_through_is_told_apart() {
        let glance = Duration::from_secs(5);

        for awake in [0, 5, 6, 15] {
            assert!(
                !slept_through(glance, Duration::from_secs(awake)),
                "{awake} seconds by the wall was taken for a sleep"
            );
        }
        for asleep in [16, 120, 8 * 60 * 60] {
            assert!(
                slept_through(glance, Duration::from_secs(asleep)),
                "{asleep} seconds by the wall was not taken for a sleep"
            );
        }
    }
    use crate::ports::account_sync::WatchEnded;

    #[test]
    fn a_burst_of_asking_is_one_run_and_one_behind_it() {
        let drawing = OneBehind::new();

        assert!(drawing.asked(), "nothing running: run it");
        for _ in 0..30 {
            assert!(!drawing.asked(), "started again while it was running");
        }

        assert!(drawing.finished(), "the last of the burst was not counted");
        assert!(!drawing.finished(), "run more than once for the burst");
        assert!(drawing.asked(), "stuck after the burst");
    }

    /// A watcher that keeps going past any script is a defect, and a test must not wait for it.
    ///
    /// The account list is what ends such a loop here: after this many answers the fixture says the
    /// device has no accounts, the watcher stops, and the count below fails the test with a name
    /// instead of hanging. A timeout could not do it — a loop with nothing to wait for never lets
    /// the paused clock move.
    const MORE_THAN_ANY_SCRIPT: usize = 4 * WATCHING_AT_MOST;

    /// Runs one watcher to its end.
    async fn watched(
        account_id: &str,
        sync: &Arc<CountingSync>,
        watch: &Arc<ScriptedWatch>,
        in_flight: InFlight,
        announce: Announce,
    ) {
        watching_one(
            account_id.to_owned(),
            Arc::clone(sync),
            Arc::clone(watch),
            in_flight,
            tokio::runtime::Handle::current(),
            Arc::new(Semaphore::new(AT_ONCE)),
            announce,
        )
        .await;
    }

    /// A watcher that answers from a script and counts how many times it was asked.
    struct ScriptedWatch {
        answers: Mutex<Vec<Result<WatchEnded, AccountSyncError>>>,
        asked: AtomicUsize,
        /// Where each watch was told the last one ended, in the order they were opened.
        since: Mutex<Vec<Option<Watermark>>>,
    }

    impl ScriptedWatch {
        fn saying(answers: Vec<Result<Watched, AccountSyncError>>) -> Arc<Self> {
            Self::ending(
                answers
                    .into_iter()
                    .map(|answer| answer.map(WatchEnded::from))
                    .collect(),
            )
        }

        fn ending(answers: Vec<Result<WatchEnded, AccountSyncError>>) -> Arc<Self> {
            Arc::new(Self {
                // Taken from the end, so the script reads in order.
                answers: Mutex::new(answers.into_iter().rev().collect()),
                asked: AtomicUsize::new(0),
                since: Mutex::new(Vec::new()),
            })
        }
    }

    impl AccountWatch for ScriptedWatch {
        fn watch(
            &self,
            _account_id: &str,
            since: Option<&Watermark>,
        ) -> Result<WatchEnded, AccountSyncError> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            self.since
                .lock()
                .expect("the lock is never poisoned")
                .push(since.cloned());
            self.answers
                .lock()
                .expect("the lock is never poisoned")
                .pop()
                // The script ran out: stop, rather than loop for ever in a test.
                .unwrap_or(Ok(Watched::CannotPush.into()))
        }
    }

    #[test]
    fn what_a_watcher_does_with_each_answer() {
        // The rule, apart from the loop that runs it: a watcher lives as long as the application,
        // and a rule tested through it could only be tested by a test that never returns.
        assert_eq!(step(Ok(Watched::Arrived)), NextStep::ReadThenWatch);
        assert_eq!(
            step(Ok(Watched::Changed)),
            NextStep::ReadUnlessEchoThenWatch
        );
        assert_eq!(step(Ok(Watched::Quiet)), NextStep::WatchAgain);
        assert_eq!(step(Ok(Watched::CannotPush)), NextStep::Stop);
        assert_eq!(step(Err(AccountSyncError::Failed)), NextStep::WaitThenWatch);
    }

    #[tokio::test(start_paused = true)]
    async fn a_server_that_says_mail_arrived_is_read_at_once() {
        let sync = Arc::new(CountingSync::with(1));
        let watch = ScriptedWatch::saying(vec![
            Ok(Watched::Arrived),
            Ok(Watched::Quiet),
            Ok(Watched::Changed),
            Ok(Watched::CannotPush),
        ]);
        let announced = Arc::new(AtomicUsize::new(0));

        let heard = Arc::clone(&announced);
        watched(
            "acc-0",
            &sync,
            &watch,
            InFlight::default(),
            Arc::new(move |_, _| {
                heard.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .await;

        assert_eq!(sync.passes.load(Ordering::SeqCst), 2, "one pass per report");
        assert_eq!(announced.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn the_next_watch_takes_up_where_the_last_one_ended() {
        // Between two watches nothing listens: the news is read, or a watch taken for an echo is
        // opened again. What arrived meanwhile is news only to a watch told where the last one
        // ended; one that starts from the mailbox as it finds it counts it as already there, and
        // it waits for the next round.
        let mark = |at: &str| Some(Watermark::new(at));
        let sync = Arc::new(CountingSync::with(1));
        let watch = ScriptedWatch::ending(vec![
            Ok(WatchEnded {
                watched: Watched::Quiet,
                seen: mark("first"),
            }),
            Ok(WatchEnded {
                watched: Watched::Arrived,
                seen: mark("second"),
            }),
            Err(AccountSyncError::Failed),
            Ok(Watched::Quiet.into()),
            Ok(Watched::CannotPush.into()),
        ]);

        watched(
            "acc-0",
            &sync,
            &watch,
            InFlight::default(),
            Arc::new(|_, _| {}),
        )
        .await;

        assert_eq!(
            *watch.since.lock().expect("test lock"),
            vec![
                None,
                mark("first"),
                mark("second"),
                // A watch that failed heard nothing, and one that says nothing about where it got
                // to moved nothing: both leave the last place standing.
                mark("second"),
                mark("second"),
            ]
        );
    }

    #[test]
    fn mail_arriving_waits_for_whatever_holds_the_account() {
        for holding in [Why::Person, Why::Background, Why::Arrival, Why::Push] {
            assert_eq!(
                turn(Why::Arrival, Some(holding)),
                Turn::Wait,
                "held by {holding:?}"
            );
        }
        assert_eq!(turn(Why::Arrival, None), Turn::Take);

        // What it holds is a pass begun after the mail came. A round has nothing to add to it, and
        // a person or a push waits for it as for any other pass.
        assert_eq!(turn(Why::Background, Some(Why::Arrival)), Turn::Leave);
        assert_eq!(turn(Why::Person, Some(Why::Arrival)), Turn::Wait);
        assert_eq!(turn(Why::Push, Some(Why::Arrival)), Turn::Wait);
    }

    /// How many passes a watcher runs on `heard` while a claim of `held_by` holds its account, and
    /// how many once that claim has let go.
    async fn passes_while_held(heard: Watched, held_by: Why) -> (usize, usize) {
        let sync = Arc::new(CountingSync::with(1));
        let watch = ScriptedWatch::saying(vec![Ok(heard), Ok(Watched::CannotPush)]);
        let in_flight = InFlight::default();
        let holding = in_flight
            .claim("acc-0", held_by)
            .expect("nothing holds it yet");

        let watching = tokio::spawn(watching_one(
            "acc-0".to_owned(),
            Arc::clone(&sync),
            Arc::clone(&watch),
            in_flight.clone(),
            tokio::runtime::Handle::current(),
            Arc::new(Semaphore::new(AT_ONCE)),
            Arc::new(|_: &str, _: AccountChanged| {}),
        ));
        settle().await;
        let while_held = sync.passes.load(Ordering::SeqCst);

        drop(holding);
        watching
            .await
            .expect("the watcher ends when its script does");

        (while_held, sync.passes.load(Ordering::SeqCst))
    }

    #[tokio::test(start_paused = true)]
    async fn mail_arriving_while_a_pass_holds_the_account_is_read_once_it_lets_go() {
        // A round or a refresh reads the inbox first and everything else after, and mail the
        // server announces in between is not in what it read. Left to it, the mail would wait for
        // the next round — two minutes, which is the wait a watch exists to take away.
        assert_eq!(
            passes_while_held(Watched::Arrived, Why::Background).await,
            (0, 1),
            "a round"
        );
        assert_eq!(
            passes_while_held(Watched::Arrived, Why::Person).await,
            (0, 1),
            "a refresh"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_server_that_cannot_push_is_asked_once_and_left_alone() {
        // Otherwise the watcher would reconnect to it in a loop, for ever, for nothing.
        let sync = Arc::new(CountingSync::with(1));
        let watch = ScriptedWatch::saying(vec![Ok(Watched::CannotPush)]);

        watched(
            "acc-0",
            &sync,
            &watch,
            InFlight::default(),
            Arc::new(|_, _| {}),
        )
        .await;

        assert_eq!(watch.asked.load(Ordering::SeqCst), 1);
        assert_eq!(sync.passes.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn a_watch_that_failed_is_not_opened_again_immediately() {
        // A server that is down would otherwise be reconnected to as fast as the machine can.
        let sync = Arc::new(CountingSync::with(1));
        let watch =
            ScriptedWatch::saying(vec![Err(AccountSyncError::Failed), Ok(Watched::CannotPush)]);

        let started = tokio::time::Instant::now();
        watched(
            "acc-0",
            &sync,
            &watch,
            InFlight::default(),
            Arc::new(|_, _| {}),
        )
        .await;

        assert!(
            started.elapsed() >= AFTER_A_FAILED_WATCH,
            "the next watch opened after {:?}",
            started.elapsed()
        );
        assert_eq!(watch.asked.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn an_account_that_was_removed_stops_being_watched() {
        let sync = Arc::new(CountingSync::with(1));
        let watch = ScriptedWatch::saying(vec![Ok(Watched::Changed)]);

        watched(
            "acc-does-not-exist",
            &sync,
            &watch,
            InFlight::default(),
            Arc::new(|_, _| {}),
        )
        .await;

        assert_eq!(
            watch.asked.load(Ordering::SeqCst),
            0,
            "an account that is not on the device is not watched"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_round_leaves_places_for_a_person_who_is_waiting() {
        // The limit is shared, and a semaphore hands places out in the order they were asked for.
        // A round asks for every account the moment it begins, so without a share of its own it
        // puts a person pressing refresh behind every mailbox in the round — which on a hundred
        // accounts is a wait with no bound a person could guess at.
        let taken = Arc::new(Semaphore::new(AT_ONCE));
        let sync = Arc::new(HoldingSync::default());

        let round = tokio::spawn(one_round(
            Arc::clone(&sync),
            InFlight::default(),
            tokio::runtime::Handle::current(),
            Arc::clone(&taken),
        ));

        // Long enough for the round to have taken everything it is ever going to take.
        tokio::time::sleep(Duration::from_millis(250)).await;

        // Read now and judged below. A pass is blocking and cannot be cancelled, so a runtime with
        // one outstanding does not shut down — asserting before letting go turns a failure into a
        // test that hangs instead of one that says what is wrong.
        let holding = sync.reading.load(Ordering::SeqCst);
        let free = taken.available_permits();

        sync.let_go.store(true, Ordering::SeqCst);
        round
            .await
            .expect("the round finishes once the passes are let go");

        assert_eq!(
            holding, A_ROUND_AT_ONCE,
            "a round held more than its share of the limit"
        );
        assert_eq!(
            free,
            AT_ONCE - A_ROUND_AT_ONCE,
            "what a person pressing refresh would have found free"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_round_on_the_timer_is_not_somebody_reading() {
        // Nobody asked for it, so nobody is told about it. Counted as reading, every round would
        // put "Checking mail" and a bar at the foot of the window every two minutes, over a person
        // who had only opened a message.
        let in_flight = InFlight::default();
        let sync = Arc::new(HoldingSync::default());

        let round = tokio::spawn(one_round(
            Arc::clone(&sync),
            in_flight.clone(),
            tokio::runtime::Handle::current(),
            Arc::new(Semaphore::new(AT_ONCE)),
        ));

        tokio::time::sleep(Duration::from_millis(250)).await;

        // Read while the passes are held, and judged after they are let go, for the reason given
        // in the test above.
        let holding = sync.reading.load(Ordering::SeqCst);
        let shown = in_flight.how_far();

        sync.let_go.store(true, Ordering::SeqCst);
        round
            .await
            .expect("the round finishes once the passes are let go");

        assert!(holding > 0, "the round was reading, or this proves nothing");
        assert_eq!(shown, HowFar::default(), "and it said so to nobody");
    }

    #[tokio::test]
    async fn a_watcher_reading_takes_a_place_like_everything_else() {
        // A watcher reads the moment its server speaks, and there are eight of them. Outside the
        // limit, that is eight mailboxes talking to servers on top of the sixteen the number
        // allows — a limit describing some of the connections rather than all of them.
        let taken = Arc::new(Semaphore::new(1));
        let held = Arc::clone(&taken)
            .try_acquire_owned()
            .expect("the only place there is");

        let sync = Arc::new(CountingSync::with(1));
        let watch = ScriptedWatch::saying(vec![Ok(Watched::Changed), Ok(Watched::CannotPush)]);

        let watching = tokio::spawn(watching_one(
            "acc-0".to_owned(),
            Arc::clone(&sync),
            watch,
            InFlight::default(),
            tokio::runtime::Handle::current(),
            Arc::clone(&taken),
            Arc::new(|_: &str, _: AccountChanged| {}),
        ));

        tokio::time::sleep(Duration::from_millis(200)).await;
        let read_while_held = sync.passes.load(Ordering::SeqCst);

        drop(held);
        let _ = watching.await;

        assert_eq!(
            read_while_held, 0,
            "a watcher read a mailbox past the limit somebody else was holding"
        );
        assert_eq!(
            sync.passes.load(Ordering::SeqCst),
            1,
            "and it read once the place was free"
        );
    }

    /// A pass that does not return until the test lets it, so the test can see what a round is
    /// holding rather than what it held a moment ago. Let go at the end: a blocking task cannot be
    /// cancelled, and a runtime shutting down waits for every one of them.
    #[derive(Default)]
    struct HoldingSync {
        reading: AtomicUsize,
        let_go: AtomicBool,
    }

    impl AccountSync for HoldingSync {
        fn accounts(&self) -> Result<Vec<String>, AccountSyncError> {
            Ok((0..AT_ONCE * 2).map(|n| format!("acc-{n}")).collect())
        }

        fn sync(&self, _account_id: &str) -> Result<AccountChanged, AccountSyncError> {
            self.reading.fetch_add(1, Ordering::SeqCst);
            while !self.let_go.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }

            Ok(AccountChanged::default())
        }
    }

    #[tokio::test]
    async fn a_round_waits_behind_the_same_limit_the_window_does() {
        // The limit is about how many mailboxes may be talking to a server at once, not about who
        // asked. Given one each, a round and a person pressing refresh were twice it between them
        // — which is the thing the number exists to stop.
        let taken = Arc::new(Semaphore::new(AT_ONCE));
        let held: Vec<_> = (0..AT_ONCE)
            .map(|_| {
                Arc::clone(&taken)
                    .try_acquire_owned()
                    .expect("the window has them all")
            })
            .collect();

        let sync = Arc::new(CountingSync::with(3));
        let round = tokio::spawn(one_round(
            Arc::clone(&sync),
            InFlight::default(),
            tokio::runtime::Handle::current(),
            Arc::clone(&taken),
        ));

        // Nothing is read while the window holds every place.
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            sync.highest.load(Ordering::SeqCst),
            0,
            "a round read a mailbox past the limit the window was holding"
        );

        drop(held);
        let changed = round.await.expect("the round finishes");
        assert_eq!(changed.len(), 3, "and reads them once there is room");
    }

    #[test]
    fn how_far_counts_what_is_done_out_of_what_was_asked_for() {
        // The line beside the gear is drawn from this. `asked` is the most that were ever in hand
        // at once since the last quiet moment — what a round was asked to do — rather than a
        // number counted from a database, which is not a thing anybody could count afterwards.
        let in_flight = InFlight::default();
        assert_eq!(in_flight.how_far(), HowFar { done: 0, asked: 0 });

        let first = in_flight.claim("acc-1", Why::Person).expect("free");
        let second = in_flight.claim("acc-2", Why::Person).expect("free");
        let third = in_flight.claim("acc-3", Why::Person).expect("free");
        assert_eq!(in_flight.how_far(), HowFar { done: 0, asked: 3 });

        drop(second);
        assert_eq!(in_flight.how_far(), HowFar { done: 1, asked: 3 });

        drop(first);
        assert_eq!(in_flight.how_far(), HowFar { done: 2, asked: 3 });

        // The last one out ends the burst. Left standing, the next round would begin against a
        // total from the one before it and the line would say two of three before anything had
        // been read at all.
        drop(third);
        assert_eq!(in_flight.how_far(), HowFar { done: 0, asked: 0 });

        let again = in_flight.claim("acc-1", Why::Person).expect("free");
        assert_eq!(in_flight.how_far(), HowFar { done: 0, asked: 1 });
        drop(again);
    }

    #[test]
    fn a_total_that_has_been_reached_does_not_come_back_down() {
        // What was asked for is the most that were ever in hand, not how many are in hand now.
        // Counted the second way, one starting after two had finished would take the total down
        // with it — a bar that was three-quarters full sliding back to a third, mid-round, for no
        // reason a person could see.
        let in_flight = InFlight::default();
        let first = in_flight.claim("acc-1", Why::Person).expect("free");
        let second = in_flight.claim("acc-2", Why::Person).expect("free");
        let third = in_flight.claim("acc-3", Why::Person).expect("free");
        assert_eq!(in_flight.how_far().asked, 3);

        drop(first);
        drop(second);

        // A fourth begins while the third is still going: two are done, four were asked for.
        let fourth = in_flight.claim("acc-4", Why::Person).expect("free");
        assert_eq!(in_flight.how_far(), HowFar { done: 2, asked: 4 });

        drop(third);
        drop(fourth);
    }

    #[test]
    fn a_pass_that_outlives_its_round_does_not_grow_the_total_for_ever() {
        // The 600-second backstop drops the waiting, not the pass: a blocking read cannot be
        // cancelled, so the claim it holds outlives the round that took it. Every round after that
        // claims every other account again, and counted by claims the total grows by one per
        // account per round until the stuck pass ends — a line reading "nine of thirty-four".
        let in_flight = InFlight::default();
        let stuck = in_flight.claim("acc-stuck", Why::Person).expect("free");

        for _round in 0..4 {
            let again = in_flight
                .claim("acc-1", Why::Person)
                .expect("free between rounds");
            drop(again);
        }

        assert_eq!(
            in_flight.how_far(),
            HowFar { done: 1, asked: 2 },
            "two mailboxes were asked for, however many rounds went by"
        );

        drop(stuck);
        assert_eq!(
            in_flight.how_far(),
            HowFar::default(),
            "the burst ends when the last pass does"
        );
    }

    #[test]
    fn a_claim_ending_clears_what_its_pass_said_about_itself() {
        // Otherwise a pass that stopped halfway holds its own share of the bar for ever, and the
        // next round begins against it.
        use crate::ports::progress::Progress;

        let how_far = crate::infrastructure::progress::HowFarAlong::default();
        let in_flight = InFlight::reporting_to(how_far.clone());

        let one = in_flight.claim("acc-1", Why::Person).expect("free");
        let two = in_flight.claim("acc-2", Why::Person).expect("free");
        how_far.reading("acc-1", 4);
        how_far.reading("acc-2", 4);
        how_far.folder_read("acc-1");

        drop(one);
        let half = how_far.along(1, 2);
        assert_eq!(
            half.through, PARTS_OF_A_MAILBOX,
            "a mailbox that finished is a whole mailbox of the bar"
        );

        drop(two);
        assert_eq!(
            how_far.along(0, 0),
            crate::infrastructure::progress::Along::default(),
            "the last claim out leaves nothing behind"
        );
    }

    /// What one mailbox is worth in [`crate::infrastructure::progress`], written here so this test
    /// fails rather than quietly passing if that ever stops being a round number.
    const PARTS_OF_A_MAILBOX: u32 = 1000;

    #[tokio::test(start_paused = true)]
    async fn an_account_being_read_by_a_round_is_not_read_again_by_its_watcher() {
        // The claim is the same one the rounds take: two connections to one server, at the same
        // moment, is what it exists to prevent.
        let sync = Arc::new(CountingSync::with(1));
        let watch = ScriptedWatch::saying(vec![Ok(Watched::Changed), Ok(Watched::CannotPush)]);

        let in_flight = InFlight::default();
        let held = in_flight
            .claim("acc-0", Why::Background)
            .expect("nothing holds it yet");

        watched(
            "acc-0",
            &sync,
            &watch,
            in_flight.clone(),
            Arc::new(|_, _| {}),
        )
        .await;

        assert_eq!(sync.passes.load(Ordering::SeqCst), 0);
        drop(held);
    }

    /// How many passes a watcher runs on news that comes `after` a claim of `held_by` on
    /// `pushed_to` ended. The clock is paused, so `after` is exactly how long it was.
    async fn passes_on_news(pushed_to: &str, held_by: Why, after: Duration) -> usize {
        passes_on(Watched::Changed, pushed_to, held_by, after).await
    }

    /// The same, for news of what the watch `heard`.
    async fn passes_on(heard: Watched, pushed_to: &str, held_by: Why, after: Duration) -> usize {
        let sync = Arc::new(CountingSync::with(1));
        let watch = ScriptedWatch::saying(vec![Ok(heard), Ok(Watched::CannotPush)]);
        let in_flight = InFlight::default();
        drop(
            in_flight
                .claim(pushed_to, held_by)
                .expect("nothing holds it yet"),
        );
        tokio::time::advance(after).await;

        watched("acc-0", &sync, &watch, in_flight, Arc::new(|_, _| {})).await;

        assert_eq!(
            watch.asked.load(Ordering::SeqCst),
            2,
            "the account is watched again either way"
        );
        sync.passes.load(Ordering::SeqCst)
    }

    #[tokio::test(start_paused = true)]
    async fn news_right_after_a_push_is_that_push_coming_back() {
        // A server tells every watch on the inbox about a change to it, the change this device has
        // just pushed included. Read as news, every message marked read is a whole pass over the
        // account for what the index already holds.
        assert_eq!(
            passes_on_news("acc-0", Why::Push, Duration::ZERO).await,
            0,
            "the moment the push let go"
        );
        assert_eq!(
            passes_on_news(
                "acc-0",
                Why::Push,
                A_PUSH_ECHOES_FOR - Duration::from_millis(1)
            )
            .await,
            0,
            "at the last moment of the window"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn mail_arriving_right_after_a_push_is_read_at_once() {
        // A person going through their inbox pushes every few seconds. Taken for an echo, mail
        // arriving in between would wait for the next round -- two minutes -- for as long as they
        // kept reading, and a move into the inbox from here is read back by the push anyway.
        assert_eq!(
            passes_on(Watched::Arrived, "acc-0", Why::Push, Duration::ZERO).await,
            1,
            "the moment the push let go"
        );
        assert_eq!(
            passes_on(
                Watched::Arrived,
                "acc-0",
                Why::Push,
                A_PUSH_ECHOES_FOR - Duration::from_millis(1)
            )
            .await,
            1,
            "at the last moment of the window"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn news_once_a_push_is_past_is_read_as_before() {
        // What another device changes arrives the moment its server says so, however recently this
        // one pushed.
        assert_eq!(
            passes_on_news("acc-0", Why::Push, A_PUSH_ECHOES_FOR).await,
            1,
            "where the window ends"
        );
        assert_eq!(
            passes_on_news("acc-0", Why::Push, BETWEEN_ROUNDS).await,
            1,
            "long after it"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn only_a_push_to_the_same_account_quiets_its_watcher() {
        assert_eq!(
            passes_on_news("acc-1", Why::Push, Duration::ZERO).await,
            1,
            "a push to another account"
        );
        assert_eq!(
            passes_on_news("acc-0", Why::Background, Duration::ZERO).await,
            1,
            "a round that has just read it"
        );
        assert_eq!(
            passes_on_news("acc-0", Why::Person, Duration::ZERO).await,
            1,
            "a refresh that has just read it"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn news_while_a_push_holds_the_account_is_not_read() {
        // The server answers the push and tells the watch in the same breath, so what the push did
        // can come back before the push has let go. Waited out and then read, it is the same pass
        // for nothing, only later.
        let sync = Arc::new(CountingSync::with(1));
        let watch = ScriptedWatch::saying(vec![Ok(Watched::Changed), Ok(Watched::CannotPush)]);
        let in_flight = InFlight::default();
        let pushing = in_flight
            .claim("acc-0", Why::Push)
            .expect("nothing holds it yet");

        let watching = tokio::spawn(watching_one(
            "acc-0".to_owned(),
            Arc::clone(&sync),
            Arc::clone(&watch),
            in_flight.clone(),
            tokio::runtime::Handle::current(),
            Arc::new(Semaphore::new(AT_ONCE)),
            Arc::new(|_: &str, _: AccountChanged| {}),
        ));
        settle().await;

        // Let go only now, and the watcher judged after it: a watcher that waited for the push
        // reads the account the moment this happens.
        drop(pushing);
        watching
            .await
            .expect("the watcher ends when its script does");

        assert_eq!(sync.passes.load(Ordering::SeqCst), 0);
        assert_eq!(
            watch.asked.load(Ordering::SeqCst),
            2,
            "and the account is watched again"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn mail_arriving_while_a_push_holds_the_account_is_read_once_it_lets_go() {
        // The push may take a whole send or a whole read back. Mail arriving meanwhile is not what
        // the push did, so it is waited out and read, not dropped.
        let sync = Arc::new(CountingSync::with(1));
        let watch = ScriptedWatch::saying(vec![Ok(Watched::Arrived), Ok(Watched::CannotPush)]);
        let in_flight = InFlight::default();
        let pushing = in_flight
            .claim("acc-0", Why::Push)
            .expect("nothing holds it yet");

        let watching = tokio::spawn(watching_one(
            "acc-0".to_owned(),
            Arc::clone(&sync),
            Arc::clone(&watch),
            in_flight.clone(),
            tokio::runtime::Handle::current(),
            Arc::new(Semaphore::new(AT_ONCE)),
            Arc::new(|_: &str, _: AccountChanged| {}),
        ));
        settle().await;
        assert_eq!(
            sync.passes.load(Ordering::SeqCst),
            0,
            "read while the push still held the account"
        );

        drop(pushing);
        watching
            .await
            .expect("the watcher ends when its script does");

        assert_eq!(sync.passes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_refresh_right_after_a_push_reads_at_once() {
        // The quiet after a push is the watcher's alone. A person pressing refresh asked for a read,
        // and a refresh that waited out a window nobody can see is a button that does nothing.
        let in_flight = InFlight::default();
        drop(
            in_flight
                .claim("acc-0", Why::Push)
                .expect("nothing holds it yet"),
        );

        let refresh = tokio::time::timeout(
            Duration::from_secs(1),
            in_flight.claim_waiting("acc-0", Why::Person),
        )
        .await;

        assert!(
            matches!(refresh, Ok(Some(_))),
            "a refresh right after a push was not given the account"
        );
    }

    #[tokio::test]
    async fn a_round_right_after_a_push_reads_the_account() {
        // Nor is it the timer's. A round is what reads news the watcher let pass, and a round that
        // let it pass too would leave it to the one after.
        let sync = Arc::new(CountingSync::with(1));
        let in_flight = InFlight::default();
        drop(
            in_flight
                .claim("acc-0", Why::Push)
                .expect("nothing holds it yet"),
        );

        let changed = one_round(
            Arc::clone(&sync),
            in_flight,
            tokio::runtime::Handle::current(),
            Arc::new(Semaphore::new(AT_ONCE)),
        )
        .await;

        assert_eq!(changed.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn what_is_kept_about_pushes_is_only_what_can_still_matter() {
        // An entry per account ever pushed to, kept for the life of the application, is a map that
        // only grows. Past the window an entry answers nothing, so it goes.
        let in_flight = InFlight::default();
        for number in 0..100 {
            drop(
                in_flight
                    .claim(&format!("acc-{number}"), Why::Push)
                    .expect("free"),
            );
        }
        tokio::time::advance(A_PUSH_ECHOES_FOR).await;
        drop(in_flight.claim("acc-last", Why::Push).expect("free"));

        let kept = in_flight.reading.lock().expect("test lock").pushed.len();
        assert_eq!(kept, 1, "only the push still inside its window");
    }

    /// Every account a person has is watched, not the first few: an account left to the rounds
    /// hears about its mail minutes after a watched one does.
    #[tokio::test(start_paused = true)]
    async fn every_account_a_person_has_is_watched() {
        let sync = Arc::new(CountingSync::with(30));
        let watch = ScriptedWatch::saying(Vec::new());

        watch_accounts(
            Arc::clone(&sync),
            Arc::clone(&watch),
            InFlight::default(),
            tokio::runtime::Handle::current(),
            Arc::new(Semaphore::new(AT_ONCE)),
            |_, _| {},
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;

        assert_eq!(watch.asked.load(Ordering::SeqCst), 30);
    }

    #[tokio::test(start_paused = true)]
    async fn no_more_accounts_are_watched_than_the_cap_allows() {
        let sync = Arc::new(CountingSync::with(WATCHING_AT_MOST + 20));
        let watch = ScriptedWatch::saying(Vec::new());

        watch_accounts(
            Arc::clone(&sync),
            Arc::clone(&watch),
            InFlight::default(),
            tokio::runtime::Handle::current(),
            Arc::new(Semaphore::new(AT_ONCE)),
            |_, _| {},
        );
        // Let every watcher that was started ask its one question and stop.
        tokio::time::sleep(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;

        assert_eq!(watch.asked.load(Ordering::SeqCst), WATCHING_AT_MOST);
    }

    /// Lets what was just started run, without letting the clock reach the next round.
    async fn settle() {
        // The watch itself runs on a blocking thread; the sleep and the yield are what let its
        // answer arrive before the assertion reads it.
        tokio::time::sleep(Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
    }

    /// Lets the supervisor reach its next round, and what it started run.
    async fn after_a_round() {
        tokio::time::sleep(BETWEEN_ROUNDS).await;
        settle().await;
    }

    /// A device whose account list changes while the application runs.
    struct ChangingAccounts {
        held: Mutex<Vec<String>>,
    }

    impl ChangingAccounts {
        fn holding(accounts: &[String]) -> Arc<Self> {
            Arc::new(Self {
                held: Mutex::new(accounts.to_vec()),
            })
        }

        fn numbered(count: usize) -> Vec<String> {
            (0..count).map(|number| format!("acc-{number}")).collect()
        }

        fn add(&self, account_id: &str) {
            self.held
                .lock()
                .expect("test lock")
                .push(account_id.to_owned());
        }
    }

    impl AccountSync for ChangingAccounts {
        fn sync(&self, _account_id: &str) -> Result<AccountChanged, AccountSyncError> {
            Ok(AccountChanged::default())
        }

        fn accounts(&self) -> Result<Vec<String>, AccountSyncError> {
            Ok(self.held.lock().expect("test lock").clone())
        }
    }

    /// A watch that answers every account the same way and records which ones it was asked about.
    struct RecordingWatch {
        answer: Watched,
        asked: Mutex<Vec<String>>,
    }

    impl RecordingWatch {
        fn answering(answer: Watched) -> Arc<Self> {
            Arc::new(Self {
                answer,
                asked: Mutex::new(Vec::new()),
            })
        }

        fn asked(&self) -> Vec<String> {
            self.asked.lock().expect("test lock").clone()
        }
    }

    impl AccountWatch for RecordingWatch {
        fn watch(
            &self,
            account_id: &str,
            _since: Option<&Watermark>,
        ) -> Result<WatchEnded, AccountSyncError> {
            self.asked
                .lock()
                .expect("test lock")
                .push(account_id.to_owned());

            Ok(self.answer.into())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_account_added_after_the_application_started_is_watched() {
        // The account most likely to be added is the first one, on a device that had none. Starting
        // the watchers once, at launch, would leave exactly that account to the timer.
        let sync = ChangingAccounts::holding(&[]);
        let watch = RecordingWatch::answering(Watched::CannotPush);

        watch_accounts(
            Arc::clone(&sync),
            Arc::clone(&watch),
            InFlight::default(),
            tokio::runtime::Handle::current(),
            Arc::new(Semaphore::new(AT_ONCE)),
            |_, _| {},
        );
        settle().await;
        assert!(watch.asked().is_empty(), "there was nothing to watch yet");

        sync.add("acc-new");
        after_a_round().await;

        assert_eq!(watch.asked(), vec!["acc-new".to_owned()]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_server_that_cannot_push_is_not_given_another_watcher() {
        // The watcher itself stops; what this is about is the supervisor not starting another one
        // every round, which would be the same reconnect loop with a slower clock.
        let sync = ChangingAccounts::holding(&["acc-0".to_owned()]);
        let watch = RecordingWatch::answering(Watched::CannotPush);

        watch_accounts(
            Arc::clone(&sync),
            Arc::clone(&watch),
            InFlight::default(),
            tokio::runtime::Handle::current(),
            Arc::new(Semaphore::new(AT_ONCE)),
            |_, _| {},
        );
        settle().await;
        assert_eq!(watch.asked().len(), 1);

        after_a_round().await;
        after_a_round().await;

        assert_eq!(watch.asked().len(), 1, "asked once, then left to the timer");
    }

    #[tokio::test(start_paused = true)]
    async fn a_watcher_brought_down_by_a_panic_gives_its_slot_back() {
        // The panic printed while this test runs is the point of it: `panic = "unwind"` means one
        // account's watcher can die without the process going with it, and a slot it never gave
        // back would be an account left unwatched until the next start.
        let sync = Arc::new(CountingSync::with(1));
        let watch = RecordingWatch::answering(Watched::Changed);

        watch_accounts(
            Arc::clone(&sync),
            Arc::clone(&watch),
            InFlight::default(),
            tokio::runtime::Handle::current(),
            Arc::new(Semaphore::new(AT_ONCE)),
            |_, _| panic!("the window is gone"),
        );
        settle().await;
        assert_eq!(watch.asked().len(), 1);

        after_a_round().await;

        assert_eq!(
            watch.asked().len(),
            2,
            "the account was never watched again after its watcher died"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_slot_that_was_given_back_goes_to_an_account_that_was_waiting() {
        let sync = ChangingAccounts::holding(&ChangingAccounts::numbered(WATCHING_AT_MOST + 1));
        let watch = RecordingWatch::answering(Watched::CannotPush);

        watch_accounts(
            Arc::clone(&sync),
            Arc::clone(&watch),
            InFlight::default(),
            tokio::runtime::Handle::current(),
            Arc::new(Semaphore::new(AT_ONCE)),
            |_, _| {},
        );
        settle().await;
        assert_eq!(
            watch.asked().len(),
            WATCHING_AT_MOST,
            "not one more than the cap at a time"
        );

        after_a_round().await;

        assert_eq!(
            watch.asked().len(),
            WATCHING_AT_MOST + 1,
            "the account left over got the slot the others gave back"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_round_reads_the_accounts_as_the_application_starts_and_again_after_the_wait() {
        let sync = Arc::new(CountingSync::with(0));

        start(
            Arc::clone(&sync),
            InFlight::default(),
            tokio::runtime::Handle::current(),
            Arc::new(Semaphore::new(AT_ONCE)),
            |_, _| {},
        );
        settle().await;
        assert_eq!(
            sync.listings.load(Ordering::SeqCst),
            1,
            "the accounts were not read as the application started"
        );

        after_a_round().await;
        assert_eq!(
            sync.listings.load(Ordering::SeqCst),
            2,
            "no round came after the wait"
        );
    }

    /// Counts how many passes overlap, which is the number the cap is about.
    struct CountingSync {
        accounts: Vec<String>,
        running: AtomicUsize,
        highest: AtomicUsize,
        passes: AtomicUsize,
        listings: AtomicUsize,
        changed: AccountChanged,
        fails: bool,
    }

    impl CountingSync {
        fn with(count: usize) -> Self {
            Self {
                accounts: (0..count).map(|number| format!("acc-{number}")).collect(),
                running: AtomicUsize::new(0),
                highest: AtomicUsize::new(0),
                passes: AtomicUsize::new(0),
                listings: AtomicUsize::new(0),
                changed: AccountChanged {
                    arrived: 1,
                    ..AccountChanged::default()
                },
                fails: false,
            }
        }
    }

    impl AccountSync for CountingSync {
        fn sync(&self, _account_id: &str) -> Result<AccountChanged, AccountSyncError> {
            let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.highest.fetch_max(now, Ordering::SeqCst);
            self.passes.fetch_add(1, Ordering::SeqCst);

            std::thread::sleep(Duration::from_millis(20));

            self.running.fetch_sub(1, Ordering::SeqCst);

            if self.fails {
                return Err(AccountSyncError::Failed);
            }
            Ok(self.changed)
        }

        fn accounts(&self) -> Result<Vec<String>, AccountSyncError> {
            // A watcher asks before every watch. Past any script the device is said to be empty,
            // which stops a loop that should have stopped itself and fails the test that counts.
            if self.listings.fetch_add(1, Ordering::SeqCst) > MORE_THAN_ANY_SCRIPT {
                return Ok(Vec::new());
            }
            Ok(self.accounts.clone())
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a runtime")
    }

    #[test]
    fn a_round_waits_for_a_change_on_its_way_and_then_reads_the_account() {
        // Read while the change is still on its way, the account would be read without it -- a
        // pass that finds the message where the person just moved it from.
        let runtime = runtime();
        let sync = Arc::new(CountingSync::with(3));
        let in_flight = InFlight::default();
        let pushing = in_flight
            .claim("acc-1", Why::Push)
            .expect("nothing holds it yet");

        let round = runtime.spawn(one_round(
            Arc::clone(&sync),
            in_flight.clone(),
            runtime.handle().clone(),
            Arc::new(Semaphore::new(AT_ONCE)),
        ));

        std::thread::sleep(Duration::from_millis(300));
        let while_pushing = sync.passes.load(Ordering::SeqCst);
        drop(pushing);
        runtime
            .block_on(round)
            .expect("the round finishes once the push is over");

        assert_eq!(while_pushing, 2, "the other two were not held up");
        assert_eq!(
            sync.passes.load(Ordering::SeqCst),
            3,
            "and the pushed one was read"
        );
    }

    #[test]
    fn a_person_waits_for_background_work_and_is_turned_away_by_another_person() {
        // Refresh pressed during a round is not answered "being read right now": the round is
        // silent, and a refresh turned away by it would be a button that does nothing. It waits
        // for the round instead, and only a refresh already underway answers for it.
        let runtime = runtime();
        let in_flight = InFlight::default();

        for holder in [Why::Background, Why::Push] {
            let held = in_flight.claim("acc-1", holder).expect("free");
            let asking = {
                let in_flight = in_flight.clone();
                runtime.spawn(async move { in_flight.claim_waiting("acc-1", Why::Person).await })
            };
            std::thread::sleep(Duration::from_millis(100));
            assert!(
                !asking.is_finished(),
                "a person did not wait for {holder:?}"
            );
            drop(held);
            let claim = runtime
                .block_on(asking)
                .expect("the wait ends")
                .expect("and the account is theirs");
            drop(claim);
        }

        let person = in_flight.claim("acc-1", Why::Person).expect("free");
        assert!(
            runtime
                .block_on(in_flight.claim_waiting("acc-1", Why::Person))
                .is_none(),
            "a second refresh is the first one"
        );
        drop(person);
    }

    #[test]
    fn a_wait_ends_when_the_claim_is_given_back_by_a_panic() {
        let runtime = runtime();
        let in_flight = InFlight::default();
        let held = in_flight.claim("acc-1", Why::Background).expect("free");

        let asking = {
            let in_flight = in_flight.clone();
            runtime.spawn(async move { in_flight.claim_waiting("acc-1", Why::Push).await })
        };
        let _ = std::thread::spawn(move || {
            let _held = held;
            panic!("a pass that fell over");
        })
        .join();

        assert!(runtime.block_on(asking).expect("the wait ends").is_some());
    }

    #[test]
    fn nobody_waiting_is_counted_as_reading() {
        // The foot of the window speaks for what a person asked for. A round, a watcher and a
        // push are all real work on the same accounts, and none of them is anybody waiting.
        let in_flight = InFlight::default();
        let round = in_flight.claim("acc-1", Why::Background).expect("free");
        let push = in_flight.claim("acc-2", Why::Push).expect("free");
        assert_eq!(in_flight.how_far(), HowFar::default());

        let person = in_flight.claim("acc-3", Why::Person).expect("free");
        assert_eq!(in_flight.how_far(), HowFar { done: 0, asked: 1 });

        drop(round);
        drop(push);
        assert_eq!(
            in_flight.how_far(),
            HowFar { done: 0, asked: 1 },
            "their ending finishes nothing a person asked for"
        );
        drop(person);
        assert_eq!(in_flight.how_far(), HowFar::default());
    }

    #[test]
    fn a_round_reads_every_account() {
        let runtime = runtime();
        let sync = Arc::new(CountingSync::with(9));

        let changed = runtime.block_on(one_round(
            Arc::clone(&sync),
            InFlight::default(),
            runtime.handle().clone(),
            Arc::new(Semaphore::new(AT_ONCE)),
        ));

        assert_eq!(sync.passes.load(Ordering::SeqCst), 9);
        assert_eq!(changed.len(), 9);
    }

    /// Accounts that answer, and accounts that never do.
    ///
    /// A server that hands over one byte every half minute is never silent, so the only network
    /// deadline there is never fires on it. What it does is hold its place for as long as it
    /// pleases.
    struct StuckSync {
        accounts: Vec<String>,
        stuck: usize,
        holds_for: Duration,
        read: Mutex<Vec<String>>,
    }

    impl AccountSync for StuckSync {
        fn sync(&self, account_id: &str) -> Result<AccountChanged, AccountSyncError> {
            let number: usize = account_id
                .trim_start_matches("acc-")
                .parse()
                .expect("generated");

            if number < self.stuck {
                // Not asleep and not failing: working, for longer than anybody waits.
                std::thread::sleep(self.holds_for);
            }

            self.read
                .lock()
                .expect("the lock is never poisoned")
                .push(account_id.to_owned());

            Ok(AccountChanged {
                arrived: 1,
                ..AccountChanged::default()
            })
        }

        fn accounts(&self) -> Result<Vec<String>, AccountSyncError> {
            Ok(self.accounts.clone())
        }
    }

    #[test]
    fn accounts_that_never_answer_do_not_keep_the_others_from_being_read() {
        // Four is every place there is. Without a bound on how long a round waits for one account,
        // four servers that answer slowly enough are four accounts that are read and ninety-six
        // that are not — and the only network deadline there is measures silence, which a server
        // dribbling a byte a minute never falls into.
        //
        // What this asserts is the waiting, not the reading: given long enough everybody is read
        // either way. The bound is wide because it is a wall clock on somebody else's machine.
        const NEVER: Duration = Duration::from_secs(4);

        let sync = Arc::new(StuckSync {
            accounts: (0..AT_ONCE + 3)
                .map(|number| format!("acc-{number}"))
                .collect(),
            stuck: AT_ONCE,
            holds_for: NEVER,
            read: Mutex::new(Vec::new()),
        });

        let runtime = runtime();
        let started = std::time::Instant::now();
        runtime.block_on(one_round_waiting_at_most(
            Arc::clone(&sync),
            InFlight::default(),
            runtime.handle().clone(),
            Arc::new(Semaphore::new(AT_ONCE)),
            Duration::from_millis(50),
        ));
        let took = started.elapsed();

        let read = sync
            .read
            .lock()
            .expect("the lock is never poisoned")
            .clone();
        for number in AT_ONCE..AT_ONCE + 3 {
            let expected = format!("acc-{number}");
            assert!(
                read.contains(&expected),
                "{expected} was never read: {read:?}"
            );
        }

        assert!(
            took < NEVER / 2,
            "the round waited {took:?} on accounts that were never going to answer"
        );
    }

    #[test]
    fn a_hundred_accounts_do_not_become_a_hundred_connections() {
        let runtime = runtime();
        let sync = Arc::new(CountingSync::with(100));

        runtime.block_on(one_round(
            Arc::clone(&sync),
            InFlight::default(),
            runtime.handle().clone(),
            Arc::new(Semaphore::new(AT_ONCE)),
        ));

        assert_eq!(sync.passes.load(Ordering::SeqCst), 100);
        assert!(
            sync.highest.load(Ordering::SeqCst) <= AT_ONCE,
            "{} accounts were read at once, the cap is {AT_ONCE}",
            sync.highest.load(Ordering::SeqCst)
        );
    }

    /// A watch that holds its thread the way a real one does, and counts how many overlap.
    #[derive(Default)]
    struct BusyWatch {
        running: AtomicUsize,
        highest: AtomicUsize,
        asked: AtomicUsize,
    }

    impl AccountWatch for BusyWatch {
        fn watch(
            &self,
            _account_id: &str,
            _since: Option<&Watermark>,
        ) -> Result<WatchEnded, AccountSyncError> {
            let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.highest.fetch_max(now, Ordering::SeqCst);
            self.asked.fetch_add(1, Ordering::SeqCst);

            std::thread::sleep(Duration::from_millis(20));

            self.running.fetch_sub(1, Ordering::SeqCst);

            // One watch per account: the supervisor then leaves it to the timer, and the test ends.
            Ok(Watched::CannotPush.into())
        }
    }

    #[test]
    fn a_hundred_accounts_watching_do_not_hold_up_a_round() {
        // A watch holds a connection and a blocking thread for as long as it lasts, and both are
        // shared with every pass and every write. What this looks for is the shape of that going
        // wrong: not a slow round, a round that never finishes.
        let runtime = runtime();
        let sync = Arc::new(CountingSync::with(100));
        let watch = Arc::new(BusyWatch::default());
        let in_flight = InFlight::default();
        let started = std::time::Instant::now();

        runtime.block_on(async {
            watch_accounts(
                Arc::clone(&sync),
                Arc::clone(&watch),
                in_flight.clone(),
                tokio::runtime::Handle::current(),
                Arc::new(Semaphore::new(AT_ONCE)),
                |_, _| {},
            );
            // Let the watchers take their slots before the round starts, so the two overlap.
            tokio::time::sleep(Duration::from_millis(50)).await;

            one_round(
                Arc::clone(&sync),
                in_flight,
                tokio::runtime::Handle::current(),
                Arc::new(Semaphore::new(AT_ONCE)),
            )
            .await;
        });
        let elapsed = started.elapsed();

        assert_eq!(
            watch.asked.load(Ordering::SeqCst),
            WATCHING_AT_MOST,
            "a hundred accounts were offered more watchers than the cap allows"
        );
        assert!(
            watch.highest.load(Ordering::SeqCst) <= WATCHING_AT_MOST,
            "{} watches were open at once, the cap is {WATCHING_AT_MOST}",
            watch.highest.load(Ordering::SeqCst)
        );
        assert_eq!(
            sync.passes.load(Ordering::SeqCst),
            100,
            "the round did not read every account while the watchers held theirs"
        );
        assert!(
            elapsed < Duration::from_secs(30),
            "a hundred watched accounts took {elapsed:?}"
        );
    }

    #[test]
    fn an_account_someone_else_is_reading_is_left_alone() {
        // Otherwise pressing refresh while a round comes round opens two connections to one server.
        let runtime = runtime();
        let sync = Arc::new(CountingSync::with(3));
        let in_flight = InFlight::default();
        let held = in_flight
            .claim("acc-1", Why::Background)
            .expect("nothing holds it yet");

        runtime.block_on(one_round(
            Arc::clone(&sync),
            in_flight.clone(),
            runtime.handle().clone(),
            Arc::new(Semaphore::new(AT_ONCE)),
        ));

        assert_eq!(
            sync.passes.load(Ordering::SeqCst),
            2,
            "the claimed account must be skipped"
        );
        drop(held);
        assert_eq!(in_flight.count(), 0, "and every claim is given back");
    }

    #[test]
    fn a_claim_is_given_back_however_the_pass_ends() {
        let runtime = runtime();
        let sync = Arc::new(CountingSync {
            fails: true,
            ..CountingSync::with(4)
        });
        let in_flight = InFlight::default();

        let changed = runtime.block_on(one_round(
            Arc::clone(&sync),
            in_flight.clone(),
            runtime.handle().clone(),
            Arc::new(Semaphore::new(AT_ONCE)),
        ));

        assert!(changed.is_empty(), "a failed pass changes nothing");
        assert_eq!(
            in_flight.count(),
            0,
            "a claim left behind would silence that account until the app restarts"
        );
    }

    #[test]
    fn a_quiet_mailbox_is_not_announced() {
        // Announcing a pass that changed nothing redraws the list under the reader for nothing.
        let runtime = runtime();
        let sync = Arc::new(CountingSync {
            changed: AccountChanged::default(),
            ..CountingSync::with(5)
        });

        let changed = runtime.block_on(one_round(
            Arc::clone(&sync),
            InFlight::default(),
            runtime.handle().clone(),
            Arc::new(Semaphore::new(AT_ONCE)),
        ));

        assert_eq!(
            sync.passes.load(Ordering::SeqCst),
            5,
            "every account is read"
        );
        assert!(changed.is_empty(), "and none of them is worth announcing");
    }

    #[test]
    fn one_account_failing_does_not_stop_the_others() {
        let runtime = runtime();
        let sync = Arc::new(CountingSync::with(6));

        let changed = runtime.block_on(one_round(
            Arc::clone(&sync),
            InFlight::default(),
            runtime.handle().clone(),
            Arc::new(Semaphore::new(AT_ONCE)),
        ));

        assert_eq!(changed.len(), 6);
    }

    #[test]
    fn something_worth_telling_is_told_apart_from_nothing() {
        assert!(!AccountChanged::default().is_anything());
        assert!(AccountChanged {
            arrived: 1,
            ..AccountChanged::default()
        }
        .is_anything());
        assert!(AccountChanged {
            removed: 1,
            ..AccountChanged::default()
        }
        .is_anything());
        assert!(AccountChanged {
            updated: 1,
            ..AccountChanged::default()
        }
        .is_anything());
        // A round that brought no mail still put what fifty messages say into the index, so a
        // search open on the screen is answering with fewer matches than there now are.
        assert!(AccountChanged {
            bodies_read: 1,
            ..AccountChanged::default()
        }
        .is_anything());
    }
}
