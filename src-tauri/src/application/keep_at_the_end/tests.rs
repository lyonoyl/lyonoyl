use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;
use std::time::Duration;

use super::*;
use crate::ports::drafts::Kept;
use lyonoyl_domain::outgoing::{OutgoingFile, LONGEST_BODY};

/// One draft as the drawer holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    account_id: String,
    draft: Draft,
    /// The files it carries, by the name each is kept as.
    files: Vec<String>,
}

/// A drafts folder in memory that keeps the way the profile does: under a number it holds, a keep
/// replaces that draft; under none, or under one thrown away, it writes a draft of its own.
#[derive(Default)]
struct Drawer {
    rows: Mutex<BTreeMap<i64, Row>>,
    /// How many keeps it was asked for, written or not.
    asked: Mutex<usize>,
    /// Holding as much as an account's drafts may.
    full: bool,
    /// A profile that will not be written.
    broken: bool,
    /// Keeps, and will not let go of a draft thrown away.
    clings: bool,
    /// Breaks in the middle of a keep or a throwing away, as a fault in an adapter would.
    panics: bool,
    /// How long a keep or a throwing away takes to write, for a race that has to overlap to prove
    /// anything.
    takes: Duration,
    /// Stops the next keep as it begins, says so, and waits to be let go on.
    halfway: Mutex<Option<(Sender<()>, Receiver<()>)>>,
}

impl Drawer {
    /// Holding one draft, about Tuesday, under `id`.
    fn holding(id: i64, body: &str) -> Self {
        let drawer = Self::default();
        drawer.rows.lock().expect("test lock").insert(id, row(body));
        drawer
    }

    fn rows(&self) -> Vec<(i64, Row)> {
        self.rows
            .lock()
            .expect("test lock")
            .iter()
            .map(|(id, row)| (*id, row.clone()))
            .collect()
    }

    fn asked(&self) -> usize {
        *self.asked.lock().expect("test lock")
    }

    /// Stops the next keep as it begins. Answers what says it has begun, and what lets it go on.
    fn stop_halfway(&self) -> (Receiver<()>, Sender<()>) {
        let (began, has_begun) = channel();
        let (go_on, waiting) = channel();
        *self.halfway.lock().expect("test lock") = Some((began, waiting));
        (has_begun, go_on)
    }
}

impl Drafts for &Drawer {
    fn keep(
        &self,
        account_id: &str,
        under: Option<i64>,
        draft: &Draft,
        files: &[DraftFile],
    ) -> Result<i64, DraftError> {
        *self.asked.lock().expect("test lock") += 1;
        let stopped = self.halfway.lock().expect("test lock").take();
        if let Some((began, waiting)) = stopped {
            began
                .send(())
                .expect("the test waits for the keep to begin");
            waiting.recv().expect("the test lets the keep go on");
        }
        thread::sleep(self.takes);
        if self.panics {
            panic!("a keep that breaks halfway");
        }
        if self.full {
            return Err(DraftError::NoRoom);
        }
        if self.broken {
            return Err(DraftError::Unavailable);
        }

        let mut rows = self.rows.lock().expect("test lock");
        let id = match under {
            Some(id) if rows.contains_key(&id) => id,
            _ => rows.keys().last().map_or(1, |last| last + 1),
        };
        rows.insert(
            id,
            Row {
                account_id: account_id.to_owned(),
                draft: draft.clone(),
                files: files.iter().map(|one| one.kept_as.clone()).collect(),
            },
        );
        Ok(id)
    }

    fn files(&self, _id: i64) -> Result<Vec<DraftFile>, DraftError> {
        unreachable!("nothing here reads a draft's files")
    }

    fn came_back(
        &self,
        _account_id: &str,
        _from_outbox: i64,
        _draft: &Draft,
    ) -> Result<Option<i64>, DraftError> {
        unreachable!("nothing comes back from the outbox here")
    }

    fn latest(&self, _account_id: &str) -> Result<Option<Kept>, DraftError> {
        unreachable!("nothing here reads the newest draft")
    }

    fn all(&self, _account_id: &str) -> Result<Vec<Kept>, DraftError> {
        unreachable!("nothing here lists the drafts")
    }

    fn discard(&self, id: i64) -> Result<(), DraftError> {
        thread::sleep(self.takes);
        if self.panics {
            panic!("a throwing away that breaks halfway");
        }
        if self.clings {
            return Err(DraftError::Unavailable);
        }

        self.rows.lock().expect("test lock").remove(&id);
        Ok(())
    }
}

/// An outbox in memory: the messages it was handed, by what they say, in order.
#[derive(Default)]
struct Outbox {
    held: Mutex<Vec<String>>,
}

impl Outbox {
    /// Writes down the message saying `body`, and answers the name it will carry.
    fn take(&self, body: &str) -> Result<String, UseCaseError> {
        self.held.lock().expect("test lock").push(body.to_owned());
        Ok("<tuesday@example.com>".to_owned())
    }

    fn held(&self) -> Vec<String> {
        self.held.lock().expect("test lock").clone()
    }
}

/// A message to Kappa about Tuesday, saying `body`.
fn typed(body: &str) -> Draft {
    Draft {
        recipients: "kappa@example.net".to_owned(),
        subject: "About Tuesday".to_owned(),
        body: body.to_owned(),
        ..Draft::default()
    }
}

/// That message as the drawer holds it, in the account it is written from, carrying nothing.
fn row(body: &str) -> Row {
    Row {
        account_id: "acc-1".to_owned(),
        draft: typed(body),
        files: Vec::new(),
    }
}

/// What the composer `writing` has on screen, kept `under` a number or under none.
fn noted(writing: &str, under: Option<i64>, body: &str) -> Note {
    Note {
        writing: writing.to_owned(),
        account_id: "acc-1".to_owned(),
        under,
        draft: typed(body),
        files: Vec::new(),
    }
}

/// A file of `size` bytes, kept as `kept_as`.
fn a_file(kept_as: &str, size: usize) -> DraftFile {
    DraftFile {
        kept_as: kept_as.to_owned(),
        file: Arc::new(OutgoingFile {
            name: format!("{kept_as}.pdf"),
            content_type: "application/pdf".to_owned(),
            bytes: vec![0; size],
        }),
    }
}

/// The first keep of the composer `writing`, under no number, saying `body`.
fn first_keep(
    being: &BeingWritten,
    drawer: &Drawer,
    writing: &str,
    body: &str,
) -> Result<i64, UseCaseError> {
    being.keep(drawer, writing, "acc-1", None, typed(body), &[])
}

/// Long enough for an end that does not wait for a keep caught halfway to have written.
const LONG_ENOUGH: Duration = Duration::from_millis(50);

#[test]
fn a_note_is_written_down_as_a_draft_when_the_application_ends() {
    // Typed a moment ago and kept by nothing yet: the end is the only keep it gets.
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    being
        .note(Note {
            files: vec![a_file("t1", 10)],
            ..noted("w1", None, "Shall we")
        })
        .expect("a message being written is held");

    being.at_the_end(|| Some(&drawer));

    assert_eq!(
        drawer.rows(),
        vec![(
            1,
            Row {
                files: vec!["t1".to_owned()],
                ..row("Shall we")
            }
        )]
    );
}

#[test]
fn a_note_under_the_number_it_is_kept_under_replaces_that_draft_and_writes_no_second() {
    let being = BeingWritten::default();
    let drawer = Drawer::holding(4, "Shall");
    being
        .note(noted("w1", Some(4), "Shall we"))
        .expect("a message being written is held");

    being.at_the_end(|| Some(&drawer));

    assert_eq!(drawer.rows(), vec![(4, row("Shall we"))]);
}

#[test]
fn nothing_noted_writes_nothing_and_asks_for_no_profile() {
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    let asked = Cell::new(false);

    being.at_the_end(|| {
        asked.set(true);
        Some(&drawer)
    });

    assert!(drawer.rows().is_empty());
    assert!(
        !asked.get(),
        "the profile was asked for with nothing to write"
    );
}

#[test]
fn a_note_let_go_of_writes_nothing() {
    // What sending, leaving by the way out that keeps, and a composer taken off the screen all
    // do: the message stopped being written, and a draft written now would be a second copy of a
    // message on its way, or of one already kept.
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    being
        .note(noted("w1", None, "Shall we"))
        .expect("a message being written is held");

    being.forget("w1");
    being.at_the_end(|| Some(&drawer));

    assert!(drawer.rows().is_empty());
}

#[test]
fn another_composer_letting_go_leaves_the_note_of_the_one_on_screen() {
    // A composer taken off the screen can be finishing a way out it started, and what it lets go
    // of then is its own.
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    being
        .note(noted("w2", None, "Shall we"))
        .expect("a message being written is held");

    being.forget("w1");
    being.at_the_end(|| Some(&drawer));

    assert_eq!(drawer.rows(), vec![(1, row("Shall we"))]);
}

#[test]
fn a_note_under_a_draft_thrown_away_is_not_written_back() {
    // Written by the end, it would find the draft gone and come back as a draft of its own.
    let being = BeingWritten::default();
    let drawer = Drawer::holding(4, "Shall");
    being
        .note(noted("w1", Some(4), "Shall we"))
        .expect("a message being written is held");

    being.discard(&drawer, 4).expect("thrown away");
    being.at_the_end(|| Some(&drawer));

    assert!(drawer.rows().is_empty(), "{:?}", drawer.rows());
}

#[test]
fn a_draft_thrown_away_while_the_end_writes_it_stays_thrown_away() {
    // The end caught writing the note under the draft's number, and the draft thrown away at that
    // moment. Thrown away first, the end would find the number gone and write the message down
    // as a draft of its own.
    let being = BeingWritten::default();
    let drawer = Arc::new(Drawer::holding(4, "Shall"));
    being
        .note(noted("w1", Some(4), "Shall we"))
        .expect("a message being written is held");
    let (began, go_on) = drawer.stop_halfway();

    let ending = {
        let (being, drawer) = (being.clone(), Arc::clone(&drawer));
        thread::spawn(move || being.at_the_end(|| Some(&*drawer)))
    };
    began.recv().expect("the end began writing");
    let throwing_away = {
        let (being, drawer) = (being.clone(), Arc::clone(&drawer));
        thread::spawn(move || being.discard(&*drawer, 4))
    };
    thread::sleep(LONG_ENOUGH);
    go_on.send(()).expect("the end is waiting");

    ending.join().expect("the end");
    throwing_away
        .join()
        .expect("the throwing away")
        .expect("thrown away");
    assert!(drawer.rows().is_empty(), "{:?}", drawer.rows());
}

#[test]
fn a_first_keep_the_end_catches_halfway_lands_first_and_the_end_writes_under_its_number() {
    // The keep was asked for before the application ended and is writing when it does. The end
    // waits for it and writes the newest note under the number it got: one draft, holding what
    // was on screen last.
    let being = BeingWritten::default();
    let drawer = Arc::new(Drawer::default());
    being
        .note(noted("w1", None, "Shall"))
        .expect("a message being written is held");
    let (began, go_on) = drawer.stop_halfway();

    let keeping = {
        let (being, drawer) = (being.clone(), Arc::clone(&drawer));
        thread::spawn(move || first_keep(&being, &drawer, "w1", "Shall"))
    };
    began.recv().expect("the keep began writing");
    being
        .note(noted("w1", None, "Shall we meet"))
        .expect("typed while the keep was on its way");
    let ending = {
        let (being, drawer) = (being.clone(), Arc::clone(&drawer));
        thread::spawn(move || being.at_the_end(|| Some(&*drawer)))
    };
    thread::sleep(LONG_ENOUGH);
    go_on.send(()).expect("the keep is waiting");

    let id = keeping.join().expect("the keep").expect("kept");
    ending.join().expect("the end");
    assert_eq!(drawer.rows(), vec![(id, row("Shall we meet"))]);
}

#[test]
fn a_first_keep_that_had_not_begun_when_the_end_came_writes_nothing_over_it() {
    // Asked for before the application ended, and its turn came after: what it holds is older
    // than what the end wrote, and written, it would be a second draft of the same message.
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    being
        .note(noted("w1", None, "Shall we"))
        .expect("a message being written is held");

    being.at_the_end(|| Some(&drawer));
    let answered = first_keep(&being, &drawer, "w1", "Shall").expect("answered");

    assert_eq!(drawer.rows(), vec![(answered, row("Shall we"))]);
}

#[test]
fn a_first_keep_and_the_end_leave_one_draft_however_they_fall() {
    // Both orders above, and every interleaving between them, many times over.
    const ROUNDS: u64 = 200;

    let drafts_left: Vec<usize> = (0..ROUNDS)
        .map(|round| {
            let being = BeingWritten::default();
            let drawer = Arc::new(Drawer {
                takes: Duration::from_micros(200),
                ..Drawer::default()
            });
            being
                .note(noted("w1", None, "Shall we"))
                .expect("a message being written is held");
            let together = Arc::new(Barrier::new(2));

            let keeping = {
                let (being, drawer, together) =
                    (being.clone(), Arc::clone(&drawer), Arc::clone(&together));
                thread::spawn(move || {
                    together.wait();
                    first_keep(&being, &drawer, "w1", "Shall we")
                })
            };
            let ending = {
                let (being, drawer, together) =
                    (being.clone(), Arc::clone(&drawer), Arc::clone(&together));
                thread::spawn(move || {
                    together.wait();
                    thread::sleep(Duration::from_micros(round % 5 * 100));
                    being.at_the_end(|| Some(&*drawer));
                })
            };

            keeping.join().expect("the keep").expect("kept");
            ending.join().expect("the end");
            drawer.rows().len()
        })
        .collect();

    assert!(
        drafts_left.iter().all(|left| *left == 1),
        "a message ended as more than one draft, or none: {drafts_left:?}"
    );
}

#[test]
fn a_keep_the_end_catches_halfway_is_written_over_by_the_newer_note() {
    // Kept under its number a moment ago, and typed into since. Written before that keep, the end
    // would be written over by it, and the last of the typing lost.
    let being = BeingWritten::default();
    let drawer = Arc::new(Drawer::holding(4, "Shall"));
    being
        .note(noted("w1", Some(4), "Shall we meet"))
        .expect("a message being written is held");
    let (began, go_on) = drawer.stop_halfway();

    let keeping = {
        let (being, drawer) = (being.clone(), Arc::clone(&drawer));
        thread::spawn(move || being.keep(&*drawer, "w1", "acc-1", Some(4), typed("Shall we"), &[]))
    };
    began.recv().expect("the keep began writing");
    let ending = {
        let (being, drawer) = (being.clone(), Arc::clone(&drawer));
        thread::spawn(move || being.at_the_end(|| Some(&*drawer)))
    };
    thread::sleep(LONG_ENOUGH);
    go_on.send(()).expect("the keep is waiting");

    keeping.join().expect("the keep").expect("kept");
    ending.join().expect("the end");
    assert_eq!(drawer.rows(), vec![(4, row("Shall we meet"))]);
}

#[test]
fn a_keep_whose_turn_comes_after_the_end_writes_nothing_over_it() {
    let being = BeingWritten::default();
    let drawer = Drawer::holding(4, "Shall");
    being
        .note(noted("w1", Some(4), "Shall we meet"))
        .expect("a message being written is held");

    being.at_the_end(|| Some(&drawer));
    let answered = being
        .keep(&drawer, "w1", "acc-1", Some(4), typed("Shall we"), &[])
        .expect("answered");

    assert_eq!(answered, 4);
    assert_eq!(drawer.rows(), vec![(4, row("Shall we meet"))]);
}

#[test]
fn a_keep_of_another_message_after_the_end_is_written_as_it_would_have_been() {
    // A composer taken off the screen with its keep still on its way: the end wrote another
    // composer's note, and nothing newer of this message is anywhere.
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    being
        .note(noted("w2", None, "Shall we"))
        .expect("a message being written is held");

    being.at_the_end(|| Some(&drawer));
    let kept = first_keep(&being, &drawer, "w1", "Another thought").expect("kept");

    assert_eq!(
        drawer.rows(),
        vec![(1, row("Shall we")), (kept, row("Another thought"))]
    );
}

#[test]
fn a_note_sent_before_the_window_heard_of_its_first_keep_is_written_under_that_keep() {
    // The keep has landed and its answer is on its way to the window, which goes on noting under
    // no number until it arrives.
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    being
        .note(noted("w1", None, "Shall"))
        .expect("a message being written is held");
    let kept = first_keep(&being, &drawer, "w1", "Shall").expect("kept");

    being
        .note(noted("w1", None, "Shall we"))
        .expect("typed before the answer arrived");
    being.at_the_end(|| Some(&drawer));

    assert_eq!(drawer.rows(), vec![(kept, row("Shall we"))]);
}

#[test]
fn a_first_keep_of_another_composer_gives_this_note_no_number() {
    // A composer taken off the screen with its first keep on its way, and another opened in its
    // place. Given that number, the end would write the new message over the old one's draft.
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    being
        .note(noted("w2", None, "A second message"))
        .expect("a message being written is held");

    let first = first_keep(&being, &drawer, "w1", "A first message").expect("kept");
    being.at_the_end(|| Some(&drawer));

    assert_eq!(
        drawer.rows(),
        vec![
            (first, row("A first message")),
            (first + 1, row("A second message"))
        ]
    );
}

#[test]
fn the_first_keep_of_a_composer_gone_gives_the_next_one_no_number() {
    // Kept and taken off the screen, and another opened in its place, which notes under no number
    // until its own first keep. Given the number of the one before, the end would write the new
    // message over that one's draft.
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    being
        .note(noted("w1", None, "A first message"))
        .expect("a message being written is held");
    let first = first_keep(&being, &drawer, "w1", "A first message").expect("kept");

    being.forget("w1");
    being
        .note(noted("w2", None, "A second message"))
        .expect("a message being written is held");
    being.at_the_end(|| Some(&drawer));

    assert_eq!(
        drawer.rows(),
        vec![
            (first, row("A first message")),
            (first + 1, row("A second message"))
        ]
    );
}

#[test]
fn a_first_keep_that_lands_with_its_note_let_go_of_still_numbers_the_notes_after_it() {
    // Everything taken off the screen while the keep was on its way lets go of the note, so it
    // lands with none held. Typed into again before the window hears of the keep, the composer
    // notes under no number, and the end would write the message beside the draft the keep made.
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    being
        .note(noted("w1", None, "Shall"))
        .expect("a message being written is held");

    being.forget("w1");
    let kept = first_keep(&being, &drawer, "w1", "Shall").expect("kept");
    being
        .note(noted("w1", None, "Shall we"))
        .expect("typed before the answer arrived");
    being.at_the_end(|| Some(&drawer));

    assert_eq!(drawer.rows(), vec![(kept, row("Shall we"))]);
}

#[test]
fn a_late_first_keep_of_a_composer_gone_leaves_the_number_of_the_one_on_screen() {
    // A composer taken off the screen with its first keep on its way, and that keep landing after
    // the first keep of the one in its place, before the window hears of it. Given the late
    // keep's number, the note would be written over the other message's draft.
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    being
        .note(noted("w2", None, "A second"))
        .expect("a message being written is held");
    let second = first_keep(&being, &drawer, "w2", "A second").expect("kept");

    let first = first_keep(&being, &drawer, "w1", "A first message").expect("kept");
    being
        .note(noted("w2", None, "A second message"))
        .expect("typed before the answer arrived");
    being.at_the_end(|| Some(&drawer));

    assert_eq!(
        drawer.rows(),
        vec![
            (second, row("A second message")),
            (first, row("A first message"))
        ]
    );
}

#[test]
fn a_message_handed_to_the_outbox_takes_its_note_with_it() {
    // On its way, the message is no longer being written, and a draft of it written by the end
    // would be a second copy.
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    let outbox = Outbox::default();
    being
        .note(noted("w1", None, "Shall we"))
        .expect("a message being written is held");

    let handed = being.hand_over(&drawer, "w1", None, || outbox.take("Shall we"));
    being.at_the_end(|| Some(&drawer));

    assert!(handed.is_ok(), "{handed:?}");
    assert_eq!(outbox.held(), vec!["Shall we".to_owned()]);
    assert!(drawer.rows().is_empty(), "{:?}", drawer.rows());
}

#[test]
fn a_message_handed_to_the_outbox_takes_the_draft_it_was_kept_as_with_it() {
    // Kept a moment before Send was pressed, and changed since. Left behind, the draft would be an
    // older copy of a message on its way, for somebody to send again.
    let being = BeingWritten::default();
    let drawer = Drawer::holding(4, "Shall");
    let outbox = Outbox::default();
    being
        .note(noted("w1", Some(4), "Shall we"))
        .expect("a message being written is held");

    let handed = being.hand_over(&drawer, "w1", Some(4), || outbox.take("Shall we"));
    being.at_the_end(|| Some(&drawer));

    assert!(handed.is_ok(), "{handed:?}");
    assert_eq!(outbox.held(), vec!["Shall we".to_owned()]);
    assert!(drawer.rows().is_empty(), "{:?}", drawer.rows());
}

#[test]
fn an_end_that_comes_as_a_message_is_handed_over_finds_the_draft_it_was_kept_as_gone() {
    // Send was pressed over a message kept a moment before, and the application ends while the
    // outbox writes it. Thrown away once the window heard the message had gone, the draft would
    // still be there as the end finished, and the process would go with both in place.
    let being = BeingWritten::default();
    let drawer = Arc::new(Drawer {
        takes: LONG_ENOUGH,
        ..Drawer::holding(4, "Shall")
    });
    let outbox = Arc::new(Outbox::default());
    being
        .note(noted("w1", Some(4), "Shall we"))
        .expect("a message being written is held");
    let (began, has_begun) = channel();
    let (go_on, waiting) = channel::<()>();

    let handing_over = {
        let (being, drawer, outbox) = (being.clone(), Arc::clone(&drawer), Arc::clone(&outbox));
        thread::spawn(move || {
            being.hand_over(&*drawer, "w1", Some(4), || {
                began
                    .send(())
                    .expect("the test waits for the outbox to begin");
                waiting.recv().expect("the test lets the outbox go on");
                outbox.take("Shall we")
            })
        })
    };
    has_begun.recv().expect("the outbox began writing");
    let ending = {
        let (being, drawer, outbox) = (being.clone(), Arc::clone(&drawer), Arc::clone(&outbox));
        thread::spawn(move || {
            being.at_the_end(|| Some(&*drawer));
            // What is there as the end finishes, which is what is there once the process has gone.
            (outbox.held(), drawer.rows())
        })
    };
    thread::sleep(LONG_ENOUGH);
    go_on.send(()).expect("the outbox is waiting");

    let (held, rows) = ending.join().expect("the end");
    let handed = handing_over.join().expect("the hand-over");
    assert!(handed.is_ok(), "{handed:?}");
    assert_eq!(held, vec!["Shall we".to_owned()]);
    assert!(rows.is_empty(), "{rows:?}");
}

#[test]
fn a_draft_the_profile_will_not_let_go_of_leaves_the_message_handed_over() {
    // The message is in the outbox. Told it was not sent, the window would say so, and sending it
    // again would put it there twice: the draft that stays is worth less than that.
    let being = BeingWritten::default();
    let drawer = Drawer {
        clings: true,
        ..Drawer::holding(4, "Shall")
    };
    let outbox = Outbox::default();
    being
        .note(noted("w1", Some(4), "Shall we"))
        .expect("a message being written is held");

    let handed = being.hand_over(&drawer, "w1", Some(4), || outbox.take("Shall we"));
    being.at_the_end(|| Some(&drawer));

    assert!(handed.is_ok(), "{handed:?}");
    assert_eq!(outbox.held(), vec!["Shall we".to_owned()]);
    assert_eq!(drawer.rows(), vec![(4, row("Shall"))]);
}

#[test]
fn a_draft_that_breaks_as_it_is_thrown_away_leaves_the_message_handed_over() {
    // A fault in the adapter once the outbox has the message. Carried out of the hand-over, it
    // would reach the window as a message not sent, which sending again would put there twice.
    let being = BeingWritten::default();
    let drawer = Drawer {
        panics: true,
        ..Drawer::holding(4, "Shall")
    };
    let outbox = Outbox::default();
    being
        .note(noted("w1", Some(4), "Shall we"))
        .expect("a message being written is held");

    let handed = being.hand_over(&drawer, "w1", Some(4), || outbox.take("Shall we"));

    assert!(handed.is_ok(), "{handed:?}");
    assert_eq!(outbox.held(), vec!["Shall we".to_owned()]);
    assert_eq!(drawer.rows(), vec![(4, row("Shall"))]);
}

#[test]
fn the_end_waits_for_a_message_on_its_way_to_the_outbox_and_writes_no_draft_of_it() {
    // Sent a moment after it was typed, with no keep made, and the outbox is writing it as the
    // application ends. Let go of before the outbox had it, the note would leave the end nothing to
    // write, and the message would be in neither place once the process had gone; written by the
    // end meanwhile, it would be a draft of a message on its way as well.
    let being = BeingWritten::default();
    let drawer = Arc::new(Drawer::default());
    let outbox = Arc::new(Outbox::default());
    being
        .note(noted("w1", None, "Shall we"))
        .expect("a message being written is held");
    let (began, has_begun) = channel();
    let (go_on, waiting) = channel::<()>();

    let handing_over = {
        let (being, drawer, outbox) = (being.clone(), Arc::clone(&drawer), Arc::clone(&outbox));
        thread::spawn(move || {
            being.hand_over(&*drawer, "w1", None, || {
                began
                    .send(())
                    .expect("the test waits for the outbox to begin");
                waiting.recv().expect("the test lets the outbox go on");
                outbox.take("Shall we")
            })
        })
    };
    has_begun.recv().expect("the outbox began writing");
    let ending = {
        let (being, drawer) = (being.clone(), Arc::clone(&drawer));
        thread::spawn(move || being.at_the_end(|| Some(&*drawer)))
    };
    thread::sleep(LONG_ENOUGH);
    go_on.send(()).expect("the outbox is waiting");

    let handed = handing_over.join().expect("the hand-over");
    ending.join().expect("the end");
    assert!(handed.is_ok(), "{handed:?}");
    assert_eq!(outbox.held(), vec!["Shall we".to_owned()]);
    assert!(drawer.rows().is_empty(), "{:?}", drawer.rows());
}

#[test]
fn a_message_the_end_wrote_down_first_is_not_handed_to_the_outbox_as_well() {
    // Send was pressed as the application ended, and the end came first: the message is a draft,
    // the one it was kept as, and in the outbox as well it would be a second copy.
    let being = BeingWritten::default();
    let drawer = Drawer::holding(4, "Shall");
    let outbox = Outbox::default();
    being
        .note(noted("w1", Some(4), "Shall we"))
        .expect("a message being written is held");

    being.at_the_end(|| Some(&drawer));
    let handed = being.hand_over(&drawer, "w1", Some(4), || outbox.take("Shall we"));

    assert!(matches!(handed, Err(UseCaseError::Cancelled)), "{handed:?}");
    assert!(outbox.held().is_empty(), "{:?}", outbox.held());
    assert_eq!(drawer.rows(), vec![(4, row("Shall we"))]);
}

#[test]
fn a_message_the_outbox_refuses_keeps_its_note_and_its_draft() {
    // Refused — nobody to send it to, say — it stays on screen, still being written, and the end
    // writes it down over the draft it was kept as.
    let being = BeingWritten::default();
    let drawer = Drawer::holding(4, "Shall");
    being
        .note(noted("w1", Some(4), "Shall we"))
        .expect("a message being written is held");

    let refused = being.hand_over(&drawer, "w1", Some(4), || {
        Err::<String, _>(UseCaseError::MessageHasNoRecipients)
    });
    being.at_the_end(|| Some(&drawer));

    assert!(
        matches!(refused, Err(UseCaseError::MessageHasNoRecipients)),
        "{refused:?}"
    );
    assert_eq!(drawer.rows(), vec![(4, row("Shall we"))]);
}

#[test]
fn a_message_of_another_composer_after_the_end_is_handed_over_as_it_would_have_been() {
    // What the end wrote is another message: this one is nowhere else, and refused, it would be
    // lost.
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    let outbox = Outbox::default();
    being
        .note(noted("w2", None, "Shall we"))
        .expect("a message being written is held");

    being.at_the_end(|| Some(&drawer));
    let handed = being.hand_over(&drawer, "w1", None, || outbox.take("Another thought"));

    assert!(handed.is_ok(), "{handed:?}");
    assert_eq!(outbox.held(), vec!["Another thought".to_owned()]);
    assert_eq!(drawer.rows(), vec![(1, row("Shall we"))]);
}

#[test]
fn a_note_past_what_a_keep_would_write_is_refused_and_the_one_before_stays() {
    // Held, it would be memory parked in the core for a message no keep writes down. The one
    // before is what the last keep that could be written would have written.
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    being
        .note(noted("w1", None, "Shall we"))
        .expect("a message being written is held");

    let refused = being.note(noted("w1", None, &"b".repeat(LONGEST_BODY + 1)));
    being.at_the_end(|| Some(&drawer));

    assert!(
        matches!(refused, Err(UseCaseError::MessageIsTooLarge)),
        "{refused:?}"
    );
    assert_eq!(drawer.rows(), vec![(1, row("Shall we"))]);
}

#[test]
fn a_note_is_held_naming_its_account_and_composer_as_long_as_this_build_makes_and_no_longer() {
    let being = BeingWritten::default();
    let at_the_limit = Note {
        writing: "w".repeat(LONGEST_NAME),
        account_id: "a".repeat(LONGEST_NAME),
        ..noted("w1", None, "Shall we")
    };

    let held = being.note(at_the_limit.clone());
    let a_longer_composer = being.note(Note {
        writing: "w".repeat(LONGEST_NAME + 1),
        ..at_the_limit.clone()
    });
    let a_longer_account = being.note(Note {
        account_id: "a".repeat(LONGEST_NAME + 1),
        ..at_the_limit
    });

    assert!(held.is_ok(), "{held:?}");
    assert!(
        matches!(a_longer_composer, Err(UseCaseError::MessageIsTooLarge)),
        "{a_longer_composer:?}"
    );
    assert!(
        matches!(a_longer_account, Err(UseCaseError::MessageIsTooLarge)),
        "{a_longer_account:?}"
    );
}

#[test]
fn refused_for_room_the_end_writes_nothing_and_goes_on() {
    // An account whose drafts hold as much as they may: the keep is refused at the end as it is
    // while the application runs. Nothing is written, nothing breaks, and nothing pretends it was.
    let being = BeingWritten::default();
    let drawer = Drawer {
        full: true,
        ..Drawer::default()
    };
    being
        .note(noted("w1", None, "Shall we"))
        .expect("a message being written is held");

    being.at_the_end(|| Some(&drawer));
    let after = first_keep(&being, &drawer, "w1", "Shall we");

    assert!(drawer.rows().is_empty());
    assert!(
        matches!(after, Err(UseCaseError::NoRoomForDrafts)),
        "a keep was answered as if the end had written: {after:?}"
    );
}

#[test]
fn a_profile_that_cannot_be_written_at_the_end_writes_nothing_and_goes_on() {
    let being = BeingWritten::default();
    let drawer = Drawer {
        broken: true,
        ..Drawer::default()
    };
    being
        .note(noted("w1", None, "Shall we"))
        .expect("a message being written is held");

    being.at_the_end(|| Some(&drawer));
    let after = first_keep(&being, &drawer, "w1", "Shall we");

    assert!(drawer.rows().is_empty());
    assert!(
        matches!(after, Err(UseCaseError::StorageUnavailable)),
        "a keep was answered as if the end had written: {after:?}"
    );
}

#[test]
fn a_profile_not_open_at_the_end_writes_nothing_and_goes_on() {
    // Storage that did not open, or waits for a master password: there is nowhere to write, and
    // the end that wrote nothing answers no keep in its place.
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    being
        .note(noted("w1", None, "Shall we"))
        .expect("a message being written is held");

    being.at_the_end(|| None::<&Drawer>);
    let after = first_keep(&being, &drawer, "w1", "Shall we").expect("kept");

    assert_eq!(drawer.rows(), vec![(after, row("Shall we"))]);
}

#[test]
fn drafts_that_break_at_the_end_do_not_take_the_end_with_them() {
    // The run callback is where the end happens, and a panic there would stop what comes after it
    // there as well: where the window was, and what the journal holds.
    let being = BeingWritten::default();
    let breaking = Drawer {
        panics: true,
        ..Drawer::default()
    };
    being
        .note(noted("w1", None, "Shall we"))
        .expect("a message being written is held");

    being.at_the_end(|| Some(&breaking));
    let drawer = Drawer::default();
    let afterwards = first_keep(&being, &drawer, "w2", "Another thought");

    assert!(afterwards.is_ok(), "{afterwards:?}");
    assert_eq!(breaking.asked(), 1);
}

#[test]
fn the_end_writes_once_however_often_it_is_told() {
    // Closing the window tells the application it is ending twice; so does a restart.
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    being
        .note(noted("w1", None, "Shall we"))
        .expect("a message being written is held");

    being.at_the_end(|| Some(&drawer));
    being
        .note(noted("w1", None, "Shall we meet"))
        .expect("a message being written is held");
    being.at_the_end(|| Some(&drawer));

    assert_eq!(drawer.rows(), vec![(1, row("Shall we"))]);
    assert_eq!(drawer.asked(), 1);
}

#[test]
fn nothing_is_written_once_the_files_are_set_aside() {
    // The application restarts after Start fresh, and the message it would keep on the way out
    // would go into a profile kept as it was for somebody to look at.
    let being = BeingWritten::default();
    let drawer = Drawer::default();
    let asked = Cell::new(false);
    being
        .note(noted("w1", None, "Shall we"))
        .expect("a message being written is held");

    being.set_aside();
    being.at_the_end(|| {
        asked.set(true);
        Some(&drawer)
    });

    assert!(drawer.rows().is_empty());
    assert!(
        !asked.get(),
        "the profile was asked for after it was set aside"
    );
}
