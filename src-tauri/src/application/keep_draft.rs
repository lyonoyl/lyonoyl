//! Writing down what is in the window, within bounds that stop it filling a disk.
//!
//! Nothing in a draft is parsed: half an address is allowed, and so is no subject at all. What is
//! bounded is how much of it there is. A draft goes into the profile, the one database that is
//! never dropped and never rebuilt, so a window that could hand over a field of any size would be
//! a way to fill a disk that nothing clears.

use crate::application::error::UseCaseError;
use crate::ports::drafts::{Draft, DraftFile, Drafts};
use lyonoyl_domain::message::LONGEST_MESSAGE_ID;
use lyonoyl_domain::outgoing::{fit_in_one_message, LONGEST_BODY};
use lyonoyl_domain::thread::{both_ends, NAMES_CONSULTED};

/// How long one name in a conversation may be as the window holds it: an identifier, and the angle
/// brackets around it.
const LONGEST_NAME: usize = LONGEST_MESSAGE_ID + "<>".len();

pub struct KeepDraft<D: Drafts> {
    drafts: D,
}

impl<D: Drafts> KeepDraft<D> {
    pub fn new(drafts: D) -> Self {
        Self { drafts }
    }

    /// Writes it down, replacing what was kept under `under`, and answers the number it is kept
    /// under.
    ///
    /// Refused whole when it holds more text than a message may carry — in its body, in its
    /// subject, or in its three lines of addresses together — or names the message it answers with
    /// more than an identifier holds: a draft like that could never be sent.
    ///
    /// The subject and the addresses are held to that and to nothing tighter, though one header
    /// line holds far less and a message goes to far fewer people. An answer's arrive filled in
    /// from the message being answered — everybody it named, when it answers everybody — and
    /// nothing bounds either on its way in: refused here, nothing of that answer is ever kept, and
    /// the window does not close until somebody shortens a subject they never typed or a list of
    /// people they never chose. Sending is what refuses a subject too long for its header, and more
    /// people than a message may be sent to.
    ///
    /// The conversation it belongs to is not typed at all. It arrives with the message being
    /// answered, out of somebody else's header, so it is cut to the names a reply carries rather
    /// than refused, and a name too long for any reply to carry is left out.
    ///
    /// The files it carries are everything attached to it now, and together they may weigh what a
    /// message may and no more: past that it is refused whole, because files no message could carry
    /// make a draft that can never be sent, written into the database that nothing clears. The text
    /// is held to its own bound and not to the room the files leave it. Refused for that, a keep
    /// would lose what somebody typed beside a photograph rather than tell them anything, and
    /// sending is what refuses a message too heavy to go.
    ///
    /// And an account's drafts are bounded together, at figures no person's own use reaches
    /// (`MOST_DRAFTS`, `HEAVIEST_DRAFTS`): a draft of its own past how many there may be, or a keep
    /// that makes one heavier past what they may weigh together, is refused whole with
    /// `NoRoomForDrafts`. The drafts check that in the change that writes the draft, because two
    /// keeps checked here and written there could both take the last of the room.
    pub fn execute(
        &self,
        account_id: &str,
        under: Option<i64>,
        draft: Draft,
        files: &[DraftFile],
    ) -> Result<i64, UseCaseError> {
        let draft = bounded(draft, files)?;

        Ok(self.drafts.keep(account_id, under, &draft, files)?)
    }
}

/// What a keep writes of `draft` carrying `files`, or the refusal it answers with, and nothing
/// written either way.
///
/// The bounds `KeepDraft::execute` describes, asked on their own: a note of what is on screen is
/// held to them before it is held at all (`keep_at_the_end`), so the core keeps in memory no more
/// than it would write down.
pub fn bounded(draft: Draft, files: &[DraftFile]) -> Result<Draft, UseCaseError> {
    // Saturating: an overflow would wrap round to a small number and pass.
    let addresses = [&draft.recipients, &draft.copies, &draft.blind]
        .iter()
        .map(|line| line.len())
        .fold(0, usize::saturating_add);

    if addresses > LONGEST_BODY
        || draft.subject.len() > LONGEST_BODY
        || draft.body.len() > LONGEST_BODY
        || !fit_in_one_message(files.iter().map(|one| &*one.file))
        || draft
            .answering
            .as_ref()
            .is_some_and(|name| name.len() > LONGEST_NAME)
    {
        return Err(UseCaseError::MessageIsTooLarge);
    }

    let carried: Vec<&String> = draft
        .chain
        .iter()
        .filter(|name| name.len() <= LONGEST_NAME)
        .collect();
    let chain = both_ends(&carried, NAMES_CONSULTED)
        .map(|name| (*name).clone())
        .collect();

    Ok(Draft { chain, ..draft })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::drafts::{DraftError, Kept};
    use lyonoyl_domain::outgoing::{
        EmailAddress, Outgoing, Subject, HEAVIEST_MESSAGE, LONGEST_ADDRESS, LONGEST_SUBJECT,
        MOST_RECIPIENTS,
    };
    use std::sync::Mutex;

    /// Somewhere that keeps what it is handed, and remembers it.
    #[derive(Default)]
    struct Drawer {
        kept: Mutex<Vec<Draft>>,
        /// The files each keep was handed, by the name each is kept as.
        files: Mutex<Vec<Vec<String>>>,
        broken: bool,
        /// Holding as much as an account's drafts may.
        full: bool,
    }

    impl Drafts for &Drawer {
        fn keep(
            &self,
            _account_id: &str,
            _under: Option<i64>,
            draft: &Draft,
            files: &[DraftFile],
        ) -> Result<i64, DraftError> {
            if self.broken {
                return Err(DraftError::Unavailable);
            }
            if self.full {
                return Err(DraftError::NoRoom);
            }
            self.files
                .lock()
                .expect("test lock")
                .push(files.iter().map(|one| one.kept_as.clone()).collect());
            let mut kept = self.kept.lock().expect("test lock");
            kept.push(draft.clone());
            Ok(kept.len() as i64)
        }

        fn files(&self, _id: i64) -> Result<Vec<DraftFile>, DraftError> {
            unreachable!("keeping a draft reads none of its files")
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
            Ok(None)
        }

        fn all(&self, _account_id: &str) -> Result<Vec<Kept>, DraftError> {
            Ok(Vec::new())
        }

        fn discard(&self, _id: i64) -> Result<(), DraftError> {
            Ok(())
        }
    }

    /// Half a message, as somebody leaves one: an address with no domain yet, and no subject.
    fn half_written() -> Draft {
        Draft {
            recipients: "kappa@".to_owned(),
            body: "Shall we".to_owned(),
            ..Draft::default()
        }
    }

    fn kept(drawer: &Drawer, draft: Draft) -> Result<i64, UseCaseError> {
        KeepDraft::new(drawer).execute("acc-1", None, draft, &[])
    }

    /// A file of `size` bytes, kept as `kept_as`.
    fn a_file(kept_as: &str, size: usize) -> DraftFile {
        DraftFile {
            kept_as: kept_as.to_owned(),
            file: std::sync::Arc::new(lyonoyl_domain::outgoing::OutgoingFile {
                name: format!("{kept_as}.bin"),
                content_type: "application/octet-stream".to_owned(),
                bytes: vec![0; size],
            }),
        }
    }

    fn kept_carrying(
        drawer: &Drawer,
        draft: Draft,
        files: &[DraftFile],
    ) -> Result<i64, UseCaseError> {
        KeepDraft::new(drawer).execute("acc-1", None, draft, files)
    }

    /// Kept with `fill` making one field `limit` long, and refused one past it with nothing
    /// written.
    fn kept_to_the_limit_and_refused_past_it(fill: fn(&mut Draft, usize), limit: usize) {
        let drawer = Drawer::default();

        let mut at_the_limit = Draft::default();
        fill(&mut at_the_limit, limit);
        assert!(kept(&drawer, at_the_limit).is_ok(), "refused at its limit");

        let mut past_it = Draft::default();
        fill(&mut past_it, limit + 1);
        assert!(
            matches!(kept(&drawer, past_it), Err(UseCaseError::MessageIsTooLarge)),
            "kept one past its limit"
        );
        assert_eq!(
            drawer.kept.lock().expect("test lock").len(),
            1,
            "what was refused was written all the same"
        );
    }

    #[test]
    fn half_a_message_is_kept_exactly_as_it_was_typed() {
        let drawer = Drawer::default();

        kept(&drawer, half_written()).expect("half a message is a draft");

        assert_eq!(
            *drawer.kept.lock().expect("test lock"),
            vec![half_written()]
        );
    }

    #[test]
    fn an_answer_is_kept_with_a_subject_too_long_to_send() {
        // Filled in from the message being answered, which nothing bounded on its way in: longer
        // than one header line holds, and still somebody's answer to keep. Sending refuses it.
        let answer = Draft {
            subject: format!("Re: {}", "s".repeat(LONGEST_SUBJECT)),
            ..half_written()
        };
        assert!(
            Subject::parse(&answer.subject).is_err(),
            "a subject a message could carry proves nothing here"
        );
        let drawer = Drawer::default();

        kept(&drawer, answer.clone()).expect("an answer with a long subject is kept");

        assert_eq!(*drawer.kept.lock().expect("test lock"), vec![answer]);
    }

    #[test]
    fn a_subject_is_kept_up_to_the_longest_text_a_message_may_carry() {
        kept_to_the_limit_and_refused_past_it(
            |draft, length| draft.subject = "s".repeat(length),
            LONGEST_BODY,
        );
    }

    #[test]
    fn a_body_is_kept_up_to_the_longest_a_message_may_carry() {
        kept_to_the_limit_and_refused_past_it(
            |draft, length| draft.body = "b".repeat(length),
            LONGEST_BODY,
        );
    }

    #[test]
    fn addresses_are_kept_up_to_the_longest_text_a_message_may_carry() {
        kept_to_the_limit_and_refused_past_it(
            |draft, length| draft.recipients = "r".repeat(length),
            LONGEST_BODY,
        );
    }

    #[test]
    fn an_answer_to_everybody_is_kept_with_more_people_than_a_message_may_be_sent_to() {
        // Everybody the answered message named, filled in from it, and nothing bounded them on
        // their way in: more people than one message may go to, in more text than all of those
        // would take at the longest an address may be. Still somebody's answer to keep. Sending
        // refuses it.
        let everybody: Vec<String> = (0..MOST_RECIPIENTS * 16)
            .map(|at| format!("kappa{at}@example.net"))
            .collect();
        let (named, copied) = everybody.split_at(everybody.len() / 2);
        let answer = Draft {
            recipients: named.join(", "),
            copies: copied.join(", "),
            ..half_written()
        };
        assert!(
            answer.recipients.len() + answer.copies.len()
                > MOST_RECIPIENTS * (LONGEST_ADDRESS + ", ".len()),
            "addresses all of a message's recipients could be written in prove nothing here"
        );
        let each = |line: &str| -> Vec<EmailAddress> {
            line.split(", ")
                .map(|one| EmailAddress::parse(one).expect("an address"))
                .collect()
        };
        let as_sent = Outgoing {
            files: Vec::new(),
            from: EmailAddress::parse("zeta@example.com").expect("an address"),
            to: each(&answer.recipients),
            cc: each(&answer.copies),
            bcc: Vec::new(),
            subject: Subject::parse("Re: All of us").expect("a subject"),
            body: answer.body.clone(),
            thread: Default::default(),
        };
        assert!(
            !as_sent.is_within_bounds(),
            "a message that could be sent proves nothing here"
        );
        let drawer = Drawer::default();

        kept(&drawer, answer.clone()).expect("an answer to everybody is kept");

        assert_eq!(*drawer.kept.lock().expect("test lock"), vec![answer]);
    }

    #[test]
    fn the_three_lines_of_addresses_are_bounded_together() {
        // What the three lines hold is counted together, as a message's recipients are: three
        // lines each under the bound are over it together.
        let third = LONGEST_BODY / 3 + 1;
        let drawer = Drawer::default();

        let outcome = kept(
            &drawer,
            Draft {
                recipients: "r".repeat(third),
                copies: "c".repeat(third),
                blind: "b".repeat(third),
                ..Draft::default()
            },
        );

        assert!(matches!(outcome, Err(UseCaseError::MessageIsTooLarge)));
        assert!(drawer.kept.lock().expect("test lock").is_empty());
    }

    #[test]
    fn a_name_longer_than_any_identifier_is_not_what_is_being_answered() {
        let drawer = Drawer::default();

        let outcome = kept(
            &drawer,
            Draft {
                answering: Some("a".repeat(LONGEST_NAME + 1)),
                ..half_written()
            },
        );

        assert!(matches!(outcome, Err(UseCaseError::MessageIsTooLarge)));
        assert!(drawer.kept.lock().expect("test lock").is_empty());
    }

    #[test]
    fn the_conversation_is_cut_to_the_names_a_reply_carries() {
        // Not typed: it arrives with the message being answered, out of somebody else's header,
        // so refusing it would make every reply to that message impossible to keep. Both ends, as
        // a reply carries them, and nothing no reply could carry — not even as the newest name.
        let mut chain: Vec<String> = (0..1_000).map(|at| format!("<{at}@example.com>")).collect();
        chain.insert(500, format!("<{}@example.com>", "x".repeat(LONGEST_NAME)));
        chain.push(format!("<{}@example.com>", "y".repeat(LONGEST_NAME)));
        let drawer = Drawer::default();

        kept(
            &drawer,
            Draft {
                chain,
                ..half_written()
            },
        )
        .expect("a reply to a long conversation is kept");

        let kept = drawer.kept.lock().expect("test lock");
        let chain = &kept[0].chain;
        assert_eq!(chain.len(), NAMES_CONSULTED);
        assert_eq!(chain[0], "<0@example.com>", "the oldest name went");
        assert_eq!(chain[NAMES_CONSULTED - 1], "<999@example.com>");
        assert!(
            chain.iter().all(|name| name.len() <= LONGEST_NAME),
            "a name no reply could carry was kept"
        );
    }

    #[test]
    fn a_profile_that_will_not_take_it_says_so() {
        let drawer = Drawer {
            broken: true,
            ..Drawer::default()
        };

        assert!(matches!(
            kept(&drawer, half_written()),
            Err(UseCaseError::StorageUnavailable)
        ));
    }

    #[test]
    fn drafts_with_no_room_left_say_so_rather_than_that_the_profile_failed() {
        // Two different things for whoever reads why a keep was refused: a disk that would not
        // take it, and a window that has written as much into the drafts as an account may hold.
        let drawer = Drawer {
            full: true,
            ..Drawer::default()
        };

        let outcome = kept(&drawer, half_written());

        assert!(
            matches!(outcome, Err(UseCaseError::NoRoomForDrafts)),
            "{outcome:?}"
        );
    }

    #[test]
    fn the_files_attached_are_handed_on_in_the_order_they_are_attached() {
        let drawer = Drawer::default();

        kept_carrying(
            &drawer,
            half_written(),
            &[a_file("t2", 10), a_file("t1", 20)],
        )
        .expect("a draft with files is kept");

        assert_eq!(
            *drawer.files.lock().expect("test lock"),
            vec![vec!["t2".to_owned(), "t1".to_owned()]]
        );
    }

    #[test]
    fn a_draft_is_kept_carrying_files_as_heavy_as_a_message_may_be_and_refused_past_them() {
        // Files no message could carry make a draft that can never be sent, and it would sit in the
        // database nothing clears.
        let drawer = Drawer::default();

        kept_carrying(&drawer, half_written(), &[a_file("t1", HEAVIEST_MESSAGE)])
            .expect("a draft carrying as much as a message may is kept");
        let past = kept_carrying(
            &drawer,
            half_written(),
            &[a_file("t1", HEAVIEST_MESSAGE + 1)],
        );

        assert!(
            matches!(past, Err(UseCaseError::MessageIsTooLarge)),
            "{past:?}"
        );
        assert_eq!(
            drawer.kept.lock().expect("test lock").len(),
            1,
            "what was refused was written all the same"
        );
    }

    #[test]
    fn what_is_typed_is_kept_beside_files_that_leave_it_no_room() {
        // A message too heavy to send, and still somebody's writing. Refused here, every keep after
        // the photograph went on would lose what was typed beside it, and a keep that fails says so
        // to nobody. Sending is what refuses it.
        let drawer = Drawer::default();
        let typed = Draft {
            body: "x".repeat(64 * 1024),
            ..half_written()
        };

        kept_carrying(&drawer, typed.clone(), &[a_file("t1", HEAVIEST_MESSAGE)])
            .expect("the text beside the file is kept");

        assert_eq!(*drawer.kept.lock().expect("test lock"), vec![typed]);
    }

    #[test]
    fn files_small_apart_and_too_heavy_together_are_refused() {
        // Checked one at a time, three files that each fit would arrive as one draft that does
        // not.
        let third = HEAVIEST_MESSAGE / 3 + 1;
        let drawer = Drawer::default();

        let outcome = kept_carrying(
            &drawer,
            Draft::default(),
            &[
                a_file("t1", third),
                a_file("t2", third),
                a_file("t3", third),
            ],
        );

        assert!(matches!(outcome, Err(UseCaseError::MessageIsTooLarge)));
        assert!(drawer.kept.lock().expect("test lock").is_empty());
    }
}
