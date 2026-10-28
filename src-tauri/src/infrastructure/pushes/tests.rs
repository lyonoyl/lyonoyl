use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::*;
use crate::ports::account_sync::{AccountChanged, AccountSync, AccountSyncError};

/// Short, so a test waits a little and a real clock can drive the real thing.
const GATHER: Duration = Duration::from_millis(40);

/// Long enough for anything a test is waiting on to have happened several times over.
const SETTLED: Duration = Duration::from_millis(400);

/// A server that counts the pushes it is given, and holds one open while the test says so.
#[derive(Default)]
struct Counting {
    pushes: AtomicUsize,
    holding: AtomicBool,
    running: AtomicBool,
    /// Whether each push says there is more behind it, until this many have run.
    more_until: AtomicUsize,
    /// Whether each push moved a message, which only a read brings into the index.
    moved: AtomicBool,
    panics: AtomicBool,
    read_backs: AtomicUsize,
    /// How many read-backs let go of something that was waiting for them.
    letting_go: AtomicUsize,
    /// Whether each push sent a message, whose copy is then filed.
    sends: AtomicBool,
    filings: AtomicUsize,
    /// How many read-backs there had been when the copies were filed.
    read_backs_at_filing: AtomicUsize,
}

impl AccountPush for Counting {
    fn push(&self, _account_id: &str) -> Result<Pushed, AccountSyncError> {
        self.running.store(true, Ordering::SeqCst);
        while self.holding.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let done = self.pushes.fetch_add(1, Ordering::SeqCst) + 1;
        self.running.store(false, Ordering::SeqCst);

        if self.panics.swap(false, Ordering::SeqCst) {
            panic!("a push that stopped halfway");
        }

        Ok(Pushed {
            moved: self.moved.load(Ordering::SeqCst),
            more: done < self.more_until.load(Ordering::SeqCst),
            sent: self.sends.load(Ordering::SeqCst),
        })
    }

    fn file_copies(&self, _account_id: &str) -> Result<bool, AccountSyncError> {
        self.read_backs_at_filing
            .store(self.read_backs.load(Ordering::SeqCst), Ordering::SeqCst);
        self.filings.fetch_add(1, Ordering::SeqCst);
        Ok(true)
    }

    fn read_back(&self, _account_id: &str) -> Result<ReadBack, AccountSyncError> {
        self.read_backs.fetch_add(1, Ordering::SeqCst);
        let let_go = self
            .letting_go
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok();

        Ok(ReadBack {
            changed: AccountChanged {
                removed: 1,
                ..AccountChanged::default()
            },
            let_go: u32::from(let_go),
        })
    }
}

impl Counting {
    fn pushes(&self) -> usize {
        self.pushes.load(Ordering::SeqCst)
    }

    fn read_backs(&self) -> usize {
        self.read_backs.load(Ordering::SeqCst)
    }
}

/// Pushes on a real runtime whose copies settle for as long as given, announcing nothing.
fn pushing_settling(push: &Arc<Counting>, copy_settles: Duration) -> Pushes {
    Pushes::gathering_for(
        Arc::clone(push) as Arc<dyn AccountPush>,
        InFlight::default(),
        tokio::runtime::Handle::current(),
        Arc::new(Semaphore::new(crate::infrastructure::schedule::AT_ONCE)),
        Arc::new(|_: &str, _: AccountChanged| {}),
        GATHER,
        copy_settles,
    )
}

/// Pushes on a real runtime, with what they announce written down.
fn pushing(push: &Arc<Counting>, in_flight: InFlight) -> (Pushes, Arc<Mutex<Vec<String>>>) {
    let announced = Arc::new(Mutex::new(Vec::new()));
    let told = Arc::clone(&announced);

    (
        Pushes::gathering_for(
            Arc::clone(push) as Arc<dyn AccountPush>,
            in_flight,
            tokio::runtime::Handle::current(),
            Arc::new(Semaphore::new(crate::infrastructure::schedule::AT_ONCE)),
            Arc::new(move |account_id: &str, _changed: AccountChanged| {
                told.lock().expect("test lock").push(account_id.to_owned());
            }),
            GATHER,
            GATHER,
        ),
        announced,
    )
}

async fn until(what: impl Fn() -> bool) {
    let began = std::time::Instant::now();
    while !what() && began.elapsed() < SETTLED * 5 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_that_sent_a_message_files_its_copy_and_then_reads_the_account_back() {
    // A message that went out is in Sent seconds later: its copy filed once its server has had its
    // moment, and the account read back after that rather than a round later.
    let push = Arc::new(Counting::default());
    push.sends.store(true, Ordering::SeqCst);
    let (pushes, announced) = pushing(&push, InFlight::default());

    pushes.please_push("acc-1");
    until(|| push.read_backs() == 1).await;

    assert_eq!(push.filings.load(Ordering::SeqCst), 1, "no copy was filed");
    assert_eq!(
        push.read_backs_at_filing.load(Ordering::SeqCst),
        0,
        "the account was read back before the copy was filed"
    );
    assert_eq!(push.read_backs(), 1);
    assert_eq!(*announced.lock().expect("test lock"), vec!["acc-1"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_written_while_a_copy_settles_goes_without_waiting_for_the_copy() {
    // Two messages a moment apart: the second is handed over as soon as it is written, and the
    // copies are filed once nothing more is asked for and their servers have had their moment.
    let push = Arc::new(Counting::default());
    push.sends.store(true, Ordering::SeqCst);
    let pushes = pushing_settling(&push, SETTLED * 2);

    pushes.please_push("acc-1");
    until(|| push.pushes() == 1).await;
    tokio::time::sleep(GATHER * 2).await;
    let asked = std::time::Instant::now();
    pushes.please_push("acc-1");
    until(|| push.pushes() == 2).await;
    let waited = asked.elapsed();

    assert_eq!(push.pushes(), 2);
    assert!(
        waited < SETTLED,
        "the second message waited for the first one's copy: {waited:?}"
    );
    assert_eq!(
        push.filings.load(Ordering::SeqCst),
        0,
        "a copy was filed before its server had had its moment"
    );

    until(|| push.read_backs() == 1).await;
    assert_eq!(push.filings.load(Ordering::SeqCst), 1);
    assert_eq!(push.read_backs(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_that_sent_nothing_files_nothing_and_reads_nothing() {
    let push = Arc::new(Counting::default());
    let (pushes, _) = pushing(&push, InFlight::default());

    pushes.please_push("acc-1");
    tokio::time::sleep(SETTLED).await;

    assert_eq!(push.pushes(), 1);
    assert_eq!(push.filings.load(Ordering::SeqCst), 0);
    assert_eq!(push.read_backs(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_burst_of_changes_to_one_account_is_one_push() {
    // Walking a folder with the arrow keys marks a message read at every step. A push each would
    // be a conversation with the server per keystroke.
    let push = Arc::new(Counting::default());
    let (pushes, _) = pushing(&push, InFlight::default());

    for _ in 0..5 {
        pushes.please_push("acc-1");
    }
    tokio::time::sleep(SETTLED).await;

    assert_eq!(push.pushes(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn what_is_asked_while_a_push_is_underway_is_carried_by_exactly_one_more() {
    let push = Arc::new(Counting::default());
    push.holding.store(true, Ordering::SeqCst);
    let (pushes, _) = pushing(&push, InFlight::default());

    pushes.please_push("acc-1");
    until(|| push.running.load(Ordering::SeqCst)).await;
    for _ in 0..3 {
        pushes.please_push("acc-1");
    }
    push.holding.store(false, Ordering::SeqCst);
    tokio::time::sleep(SETTLED).await;

    assert_eq!(
        push.pushes(),
        2,
        "one underway, and one for everything since"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn two_accounts_are_pushed_apart() {
    let push = Arc::new(Counting::default());
    let (pushes, _) = pushing(&push, InFlight::default());

    pushes.please_push("acc-1");
    pushes.please_push("acc-2");
    tokio::time::sleep(SETTLED).await;

    assert_eq!(push.pushes(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_waits_for_a_pass_reading_the_account_and_is_not_dropped() {
    // A round reading the account when the change is written: the push waits its turn. Dropped
    // instead, the change would wait for the next round to reach the server.
    let push = Arc::new(Counting::default());
    let in_flight = InFlight::default();
    let (pushes, _) = pushing(&push, in_flight.clone());

    let reading = in_flight
        .claim("acc-1", Why::Background)
        .expect("nothing holds it yet");
    pushes.please_push("acc-1");
    tokio::time::sleep(SETTLED).await;
    let while_read = push.pushes();

    drop(reading);
    until(|| push.pushes() == 1).await;

    assert_eq!(while_read, 0, "it talked to the server while a pass was");
    assert_eq!(push.pushes(), 1, "and it went once the pass was over");
}

/// Passes over a handful of other accounts, each held open until the test lets it go.
#[derive(Default)]
struct HeldOpen {
    begun: AtomicUsize,
    let_go: AtomicBool,
}

impl AccountSync for HeldOpen {
    fn sync(&self, _account_id: &str) -> Result<AccountChanged, AccountSyncError> {
        self.begun.fetch_add(1, Ordering::SeqCst);
        while !self.let_go.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(AccountChanged::default())
    }

    fn accounts(&self) -> Result<Vec<String>, AccountSyncError> {
        Ok((1..=4).map(|at| format!("round-{at}")).collect())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_is_not_kept_waiting_by_a_round_over_other_accounts() {
    // A round is underway, every pass of it still talking to its server. What a person did to
    // another account goes now: it is not queued behind the round, and not behind its timer.
    let push = Arc::new(Counting::default());
    let in_flight = InFlight::default();
    let at_once = Arc::new(Semaphore::new(crate::infrastructure::schedule::AT_ONCE));
    let pushes = Pushes::gathering_for(
        Arc::clone(&push) as Arc<dyn AccountPush>,
        in_flight.clone(),
        tokio::runtime::Handle::current(),
        Arc::clone(&at_once),
        Arc::new(|_: &str, _: AccountChanged| {}),
        GATHER,
        GATHER,
    );

    let passes = Arc::new(HeldOpen::default());
    let round = tokio::spawn(crate::infrastructure::schedule::one_round(
        Arc::clone(&passes),
        in_flight.clone(),
        tokio::runtime::Handle::current(),
        Arc::clone(&at_once),
    ));
    until(|| passes.begun.load(Ordering::SeqCst) == 4).await;

    pushes.please_push("acc-9");
    until(|| push.pushes() == 1).await;
    let round_still_running = !round.is_finished();

    passes.let_go.store(true, Ordering::SeqCst);
    round.await.expect("the round ends");

    assert_eq!(push.pushes(), 1, "the push waited for the round");
    assert!(
        round_still_running,
        "the round was over before the push went"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_that_carried_as_much_as_it_may_is_followed_by_another() {
    let push = Arc::new(Counting::default());
    push.more_until.store(3, Ordering::SeqCst);
    let (pushes, _) = pushing(&push, InFlight::default());

    pushes.please_push("acc-1");
    tokio::time::sleep(SETTLED).await;

    assert_eq!(push.pushes(), 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_that_moved_nothing_reads_nothing_and_announces_nothing() {
    // A flag the server confirmed is already where every view reads it. Reading the account for
    // it would keep a server busy for nothing, and announcing it would redraw the list under the
    // reader.
    let push = Arc::new(Counting::default());
    let (pushes, announced) = pushing(&push, InFlight::default());

    pushes.please_push("acc-1");
    tokio::time::sleep(SETTLED).await;
    assert_eq!(push.read_backs(), 0);
    assert!(announced.lock().expect("test lock").is_empty());

    push.moved.store(true, Ordering::SeqCst);
    pushes.please_push("acc-1");
    tokio::time::sleep(SETTLED).await;
    assert_eq!(push.read_backs(), 1);
    assert_eq!(
        *announced.lock().expect("test lock"),
        vec!["acc-1".to_owned()]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_burst_that_moved_mail_is_read_back_once_when_it_is_over() {
    // A trash emptied twenty at a time is many pushes, and each of them moved something. Read back
    // after every one, that is the whole account read once per twenty messages.
    let push = Arc::new(Counting::default());
    push.moved.store(true, Ordering::SeqCst);
    push.more_until.store(4, Ordering::SeqCst);
    let (pushes, announced) = pushing(&push, InFlight::default());

    pushes.please_push("acc-1");
    tokio::time::sleep(SETTLED).await;

    assert_eq!(push.pushes(), 4);
    assert_eq!(push.read_backs(), 1);
    assert_eq!(announced.lock().expect("test lock").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn what_a_read_back_lets_go_of_is_pushed_straight_after_it() {
    // Archive and undo before the archiving was read back: the way back waits for the read, and
    // goes as soon as it is done rather than at the next round.
    let push = Arc::new(Counting::default());
    push.moved.store(true, Ordering::SeqCst);
    push.letting_go.store(1, Ordering::SeqCst);
    let (pushes, _) = pushing(&push, InFlight::default());

    pushes.please_push("acc-1");
    tokio::time::sleep(SETTLED).await;

    assert_eq!(push.pushes(), 2, "the way back waited for the next round");
    assert_eq!(push.read_backs(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_is_not_shown_as_anybody_reading() {
    let push = Arc::new(Counting::default());
    push.holding.store(true, Ordering::SeqCst);
    let in_flight = InFlight::default();
    let (pushes, _) = pushing(&push, in_flight.clone());

    pushes.please_push("acc-1");
    until(|| push.running.load(Ordering::SeqCst)).await;
    let shown = in_flight.how_far();
    push.holding.store(false, Ordering::SeqCst);

    assert_eq!(shown, crate::infrastructure::schedule::HowFar::default());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_push_that_fell_over_does_not_leave_the_account_unpushable() {
    let push = Arc::new(Counting::default());
    push.panics.store(true, Ordering::SeqCst);
    let in_flight = InFlight::default();
    let (pushes, _) = pushing(&push, in_flight.clone());

    pushes.please_push("acc-1");
    until(|| push.pushes() == 1).await;
    tokio::time::sleep(SETTLED).await;

    pushes.please_push("acc-1");
    until(|| push.pushes() == 2).await;

    assert_eq!(push.pushes(), 2);
    assert_eq!(in_flight.count(), 0, "and its claim was given back");
}
