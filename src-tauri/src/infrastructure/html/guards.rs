//! What bounds a parser fed a file somebody else wrote.
//!
//! Not what it is allowed to produce — that is the sanitizer — but what it may cost: how many
//! documents are being read at one moment, how long any of them may take, and what happens when one
//! of them does the three things a parser does to a process it cannot be trusted with. Nothing here
//! knows what HTML is.

use super::LARGEST_MESSAGE;

/// How long the whole of it may take.
const AT_MOST: std::time::Duration = std::time::Duration::from_secs(5);

/// How many bodies may be being rendered at the same moment.
///
/// The time limit below bounds the *wait*, not the work: a thread that has run out of time is left
/// to finish because it cannot be killed, and the cost of the work is superlinear in the size of
/// the document — measured on this sanitizer, 250 KB takes four seconds and 2 MB is still holding a
/// core after seventy. So the thing that actually bounds what one machine will spend is how many of
/// those may exist at once, and nothing else does.
///
/// Four, because a person reading their mail never has two: a body is opened, it renders in
/// milliseconds, and the one before it is finished. Four is room for a retry landing on top of a
/// redraw. What it refuses is a caller asking in a loop, which is the only way to reach it.
const AT_ONCE: usize = 4;

/// How long a body will wait for one of those places before giving up.
///
/// Short on purpose. A render takes milliseconds, so anything asked for while the machine is not
/// under attack is served immediately and this number is never reached. Under attack it is what
/// stops the callers that are queueing from being a second unbounded resource: each of them holds a
/// thread of the blocking pool while it waits, and holding one for a second is affordable where
/// holding one for the render's own five is not.
const FOR_A_PLACE: std::time::Duration = std::time::Duration::from_secs(1);

/// How long a body waits when somebody is sitting in front of the answer.
///
/// Longer, because the alternative is not "a little slower" but "we could not look" — and until
/// this was told apart from "there is nothing to look at", the person was shown the second when
/// the first had happened. A wait that is seen is a wait; a wait that ends in the wrong sentence
/// is a fault.
const WHILE_SOMEBODY_WAITS: std::time::Duration = std::time::Duration::from_secs(5);

/// Who is asking for a place, which decides how long they wait and how much room they leave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Asking {
    /// A person is in front of the answer: opening a message, or quoting one to answer it.
    SomebodyIsWaiting,
    /// A round that reads mail for the index. It has the rest of the day, and nothing on screen
    /// changes when it is a moment later.
    NobodyIsWaiting,
}

/// How many bodies are being rendered at once, and who is waiting for a place.
///
/// A type rather than a pair of statics so that a test can have one of its own: places taken from
/// the one the process shares would be places the rest of the suite could not have, and a test that
/// makes the machine look busy must not make it look busy to everybody else.
struct Places {
    at_once: usize,
    running: std::sync::Mutex<usize>,
    freed: std::sync::Condvar,
}

impl Places {
    const fn new(at_once: usize) -> Self {
        Self {
            at_once,
            running: std::sync::Mutex::new(0),
            freed: std::sync::Condvar::new(),
        }
    }

    /// A place to render in, waiting a little while there is none.
    ///
    /// Waits rather than refuses. The limit exists to bound what is running at one moment —
    /// threads, each with a stack of its own, and the cores they sit on — not to refuse work: a
    /// person who opened a fourth message while three were still going wants it rendered, not an
    /// error, and a render takes milliseconds. What the limit stops is a caller asking in a loop,
    /// and that caller meets it as a wait that ends in nothing rather than as a machine with fifty
    /// threads on it.
    fn take(&'static self, asking: Asking) -> Option<Rendering> {
        // How many places this caller may find taken and still get one. Work nobody is waiting for
        // leaves room behind it: a person who opened a message is sitting in front of the answer,
        // and an indexing round has the rest of the day. Without this the round takes all four —
        // it runs on every account at once — and the reader is the one who waits.
        let ceiling = match asking {
            Asking::SomebodyIsWaiting => self.at_once,
            Asking::NobodyIsWaiting => self.at_once - self.at_once / 2,
        };
        let patience = match asking {
            Asking::SomebodyIsWaiting => WHILE_SOMEBODY_WAITS,
            Asking::NobodyIsWaiting => FOR_A_PLACE,
        };

        let held = self.running.lock().unwrap_or_else(|held| held.into_inner());

        let (mut held, waited) = self
            .freed
            .wait_timeout_while(held, patience, |running| *running >= ceiling)
            .unwrap_or_else(|held| held.into_inner());

        if waited.timed_out() && *held >= ceiling {
            return None;
        }

        *held += 1;
        Some(Rendering { of: self })
    }
}

/// The places this process renders in.
static RENDERING: Places = Places::new(AT_ONCE);

/// One render's place. Gives it back when the thread that holds it ends, however it ends.
struct Rendering {
    of: &'static Places,
}

impl Drop for Rendering {
    fn drop(&mut self) {
        let mut running = self
            .of
            .running
            .lock()
            .unwrap_or_else(|held| held.into_inner());
        *running = running.saturating_sub(1);
        // One waiter, because one place came back.
        self.of.freed.notify_one();
    }
}

/// A place to render in, from the ones this process shares.
fn a_place_to_render(asking: Asking) -> Option<Rendering> {
    RENDERING.take(asking)
}

/// The stack the work runs on.
///
/// Bigger than a thread's default because the parser recurses over what it is given, and the
/// thread is its own so that a stack this size is not held by everything else in the process.
const STACK: usize = 16 * 1024 * 1024;

/// What came of asking for a message to be read behind the guards.
///
/// Three answers rather than two, because "I could not look" and "there is nothing to look at" are
/// different sentences and were the same one. Told apart nowhere, the first was shown to a person
/// as the second — and, in the pass that reads mail for the index, it was written down as the
/// second, which took a message out of every search for good.
pub(super) enum Guarded<T> {
    /// It ran, and this is what it said.
    Answered(T),
    /// It was not run, and running it later answers the same: there is nothing there, or there is
    /// more of it than this build reads.
    Refused,
    /// There was nowhere to run it. Nothing is known about this message; asking again when the
    /// machine is quieter is a different question rather than the same one repeated.
    Busy,
}

/// Runs `work` on a thread of its own, with a stack of its own, a time limit and a catch for a
/// panic: the three ways a parser fed a hostile file takes the process with it. Nothing comes back
/// when any of them trips, and `panic = "unwind"` is what makes that possible.
pub(super) fn behind_the_guards<T: Send + 'static>(
    raw: &[u8],
    named: &str,
    asking: Asking,
    work: impl FnOnce(&[u8]) -> T + Send + 'static,
) -> Guarded<T> {
    if raw.is_empty() || raw.len() > LARGEST_MESSAGE {
        return Guarded::Refused;
    }

    // Refused before a thread exists rather than after one is running, because a running one
    // cannot be stopped.
    let Some(place) = a_place_to_render(asking) else {
        return Guarded::Busy;
    };

    let message = raw.to_vec();
    let (answer, wait) = std::sync::mpsc::channel();

    let worker = std::thread::Builder::new()
        .name(named.to_owned())
        .stack_size(STACK)
        .spawn(move || {
            // Moved in, so the place is given back when this thread ends — not when whoever was
            // waiting for it stopped waiting.
            let _place = place;

            let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&message)));

            // The receiver is gone once the wait is over; what was made goes with the message.
            if let Ok(done) = done {
                let _ = answer.send(done);
            }
        });

    if worker.is_err() {
        // No thread to run it on. Nothing was learned about the message, which is the same
        // sentence as having found nowhere to run it.
        return Guarded::Busy;
    }

    // Work that takes longer than this is work nobody is waiting for any more. The thread is left
    // to finish on its own — it cannot be killed — and it keeps its place until it does, so what
    // bounds the machine is `AT_ONCE` rather than this wait.
    //
    // Giving up here is refusal rather than busyness: this message was looked at, for as long as
    // this build looks at anything, and it did not finish. Asking again would take as long again.
    match wait.recv_timeout(AT_MOST) {
        Ok(done) => Guarded::Answered(done),
        Err(_) => Guarded::Refused,
    }
}

#[cfg(test)]
mod how_many_bodies_at_once {
    use super::super::*;
    use super::{Asking, Places, Rendering, FOR_A_PLACE};

    // Places of each test's own. Not one set shared between them, and not the set the process
    // renders in: a test that makes the machine look busy must not make it look busy to the tests
    // running beside it — including the other one in this module, which is how this was found.
    static SATURATED: Places = Places::new(4);
    static GIVEN_BACK: Places = Places::new(4);
    static YIELDING: Places = Places::new(4);

    #[test]
    fn as_many_as_the_build_allows_and_not_one_more() {
        let places: Vec<Rendering> = (0..2)
            .map(|_| {
                SATURATED
                    .take(Asking::NobodyIsWaiting)
                    .expect("a place while there is room")
            })
            .collect();

        // The one that would make it three. It waits for a place, finds none in the time it is
        // willing to wait, and gives up — which is what a caller asking in a loop meets, and the
        // reason the render's own time limit was never a bound: the ones before it are still
        // running whether or not anybody is still waiting for them.
        let asked = std::time::Instant::now();
        assert!(SATURATED.take(Asking::NobodyIsWaiting).is_none());
        assert!(
            asked.elapsed() >= FOR_A_PLACE,
            "it did not wait for a place"
        );

        drop(places);
    }

    /// A person opening a message does not queue behind a round that reads mail for the index.
    ///
    /// The round runs on four accounts at once and reads fifty messages in each, so on equal terms
    /// it holds every place there is, for as long as it wants them. The person is in front of the
    /// answer; the round has the rest of the day.
    #[test]
    fn somebody_waiting_is_not_kept_out_by_work_nobody_is_waiting_for() {
        let held: Vec<Rendering> =
            std::iter::repeat_with(|| YIELDING.take(Asking::NobodyIsWaiting))
                .take_while(Option::is_some)
                .flatten()
                .collect();

        assert!(
            held.len() < 4,
            "the round took every place there is: {} of 4",
            held.len()
        );

        let reading = YIELDING.take(Asking::SomebodyIsWaiting);
        assert!(
            reading.is_some(),
            "a person opening a message waited on an indexing round"
        );

        drop(reading);
        drop(held);
    }

    #[test]
    fn a_place_comes_back_when_the_work_that_held_it_ends() {
        let places: Vec<Rendering> = (0..2)
            .map(|_| {
                GIVEN_BACK
                    .take(Asking::NobodyIsWaiting)
                    .expect("a place while there is room")
            })
            .collect();

        drop(places);

        // Given back by the thread ending, however it ended — a panic caught, a body rendered, or
        // a wait somebody else gave up on.
        assert!(GIVEN_BACK.take(Asking::NobodyIsWaiting).is_some());
    }

    #[test]
    fn a_body_gives_its_place_back_when_it_is_rendered() {
        let shown = body_of(
            b"Subject: x\r\nContent-Type: text/plain\r\n\r\nhello",
            Viewing {
                form: BodyForm::Sent,
                pictures: Pictures::Blocked,
            },
        );

        assert_eq!(shown.kind, BodyKind::PlainText);
    }
}
