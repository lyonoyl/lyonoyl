//! One sentence about the whole workspace: is the mail current, and did what was asked for land.
//!
//! Read from both databases because the answer lives in both — when a pass last finished is in the
//! profile, and nothing here writes to either.

use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::ports::activity::WhyItStopped;
use crate::read_model::list::ReadModelError;

/// What the interface shows in one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HowThingsAre {
    /// Unix seconds of the most recent pass that finished, across every account. `None` before any
    /// ever has.
    ///
    /// The most recent rather than the oldest: the line says when this window last learned
    /// anything, and an account that has never worked is said separately below.
    pub last_success_at: Option<i64>,
    /// Why passes are not finishing, when none of the accounts is finishing them. `None` while at
    /// least one is: one account signed out does not mean the mail on screen is stale.
    pub stopped_by: Option<WhyItStopped>,
    /// How many accounts are failing right now.
    pub accounts_failing: u32,
    /// Instructions a server has not taken yet, and is late taking: tried at least once, or asked
    /// for longer ago than a push and a round take.
    pub waiting: u32,
    /// Messages somebody pressed send on that no server has taken yet, counted the same way.
    ///
    /// Its own count and not part of `waiting`: an instruction to a mailbox and a message a person
    /// wrote are different news, and the one about the message is the one they are watching for.
    /// Without it nothing would say that such a message exists: until a server takes it, it is in
    /// no folder.
    pub sending: u32,
    /// Instructions that stopped and are waiting for a person.
    pub stopped: u32,
    /// Messages somebody pressed send on that are back in the drafts, because the outbox will not
    /// try them again. A server would not take them, would not let the account sign in, or could
    /// not be reached on any of the outbox's tries; or nobody knows whether one has them, because
    /// the conversation ended after the message was handed over or a round took them and never
    /// wrote down what became of them.
    ///
    /// Counted by the mark the draft was given when it came back, because a draft is only a draft:
    /// what makes this worth a line is that somebody pressed send and the message is in the drafts
    /// again rather than known to have gone. The mark is the draft's own, so the line says so for
    /// exactly as long as the draft exists, and stops once it is sent again or thrown away. A
    /// draft without the mark is not counted, even where the outbox holds the message it came from
    /// as stopped: nothing ties that draft to the message.
    pub came_back: u32,
}

/// How long a change may be on its way before the line says it is.
///
/// Longer than a push and the round after it together — a round comes every two minutes — so
/// something waiting for its turn is never mentioned, and something the servers are not taking is
/// mentioned within a round of it being late.
const NEWS_AFTER: std::time::Duration = std::time::Duration::from_secs(150);

pub struct ActivityReadModel {
    profile: SqlitePool,
}

impl ActivityReadModel {
    pub fn new(profile: SqlitePool) -> Self {
        Self { profile }
    }

    pub fn how_things_are(&self) -> Result<HowThingsAre, ReadModelError> {
        let connection = self
            .profile
            .get()
            .map_err(|_| ReadModelError::Unavailable)?;

        let accounts: i64 = connection
            .query_row("SELECT count(*) FROM mail_accounts", [], |row| row.get(0))
            .map_err(|_| ReadModelError::Unavailable)?;

        let (last_success_at, failing, reason): (Option<i64>, i64, Option<String>) = connection
            .query_row(
                // The newest success anywhere, how many accounts are failing, and one of their
                // reasons — one, because a line that lists four reasons is a line nobody reads.
                "SELECT max(last_success_at),
                        count(last_error),
                        min(last_error)
                 FROM account_activity",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|_| ReadModelError::Unavailable)?;

        // A change and a message on their way are news only once they are late: tried and not
        // taken, or older than a push and the round after it take together. Until then they are
        // what every change is for the second or two it takes, and saying so would turn opening a
        // message into a line about the server.
        let sending: i64 = connection
            .query_row(
                "SELECT count(*) FILTER (WHERE state IN ('waiting', 'sending')
                                           AND (attempts > 0 OR written_at < unixepoch() - ?1))
                 FROM outbox",
                [NEWS_AFTER.as_secs() as i64],
                |row| row.get(0),
            )
            .map_err(|_| ReadModelError::Unavailable)?;

        let came_back: i64 = connection
            .query_row(
                "SELECT count(*) FROM drafts WHERE came_back = 1",
                [],
                |row| row.get(0),
            )
            .map_err(|_| ReadModelError::Unavailable)?;

        let (waiting, stopped): (i64, i64) = connection
            .query_row(
                "SELECT
                     count(*) FILTER (WHERE state IN ('pending', 'needs_reresolve')
                                        AND (attempts > 0 OR asked_at < unixepoch() - ?1)),
                     count(*) FILTER (WHERE state = 'conflicted')
                 FROM op_log",
                [NEWS_AFTER.as_secs() as i64],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|_| ReadModelError::Unavailable)?;

        Ok(HowThingsAre {
            last_success_at,
            // Only when every account is failing. One signed-out account among four does not make
            // the mail on screen stale, and a line that says it does teaches people to ignore it.
            stopped_by: match failing > 0 && failing >= accounts {
                true => reason.as_deref().map(WhyItStopped::parse),
                false => None,
            },
            accounts_failing: u32::try_from(failing).unwrap_or(u32::MAX),
            waiting: u32::try_from(waiting).unwrap_or(u32::MAX),
            sending: u32::try_from(sending).unwrap_or(u32::MAX),
            stopped: u32::try_from(stopped).unwrap_or(u32::MAX),
            came_back: u32::try_from(came_back).unwrap_or(u32::MAX),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::fake_provider::SeededProfile;
    use crate::infrastructure::sqlite::activity::SqliteAccountActivity;
    use crate::ports::activity::AccountActivity;

    fn workspace(name: &str, accounts: &[(&str, i64)]) -> SeededProfile {
        let profile = SeededProfile::new(name);
        for (id, number) in accounts {
            profile.add_account(id, &format!("{id}@example.com"), *number);
        }

        profile
    }

    #[test]
    fn a_workspace_nothing_has_happened_in_says_nothing() {
        let profile = workspace("activity-nothing", &[("acc-1", 1)]);

        assert_eq!(
            ActivityReadModel::new(profile.pool.clone())
                .how_things_are()
                .expect("readable"),
            HowThingsAre::default()
        );
    }

    #[test]
    fn the_line_says_when_this_window_last_learned_anything() {
        // The newest success anywhere, not the oldest: what it answers is "how fresh is what I am
        // looking at", and that is decided by the account that worked most recently.
        let profile = workspace("activity-newest", &[("acc-1", 1), ("acc-2", 2)]);
        let activity = SqliteAccountActivity::new(profile.pool.clone());

        activity.finished("acc-1", 1_700_000_000).expect("written");
        activity.finished("acc-2", 1_700_000_500).expect("written");

        assert_eq!(
            ActivityReadModel::new(profile.pool.clone())
                .how_things_are()
                .expect("readable")
                .last_success_at,
            Some(1_700_000_500)
        );
    }

    #[test]
    fn one_account_failing_among_several_does_not_call_the_whole_thing_stopped() {
        // The mail on screen is not stale because one of four accounts is signed out, and a line
        // that says it is teaches a person to stop reading it.
        let profile = workspace("activity-one-of-two", &[("acc-1", 1), ("acc-2", 2)]);
        let activity = SqliteAccountActivity::new(profile.pool.clone());

        activity.finished("acc-1", 1_700_000_000).expect("written");
        activity
            .stopped("acc-2", WhyItStopped::SignIn)
            .expect("written");

        let how = ActivityReadModel::new(profile.pool.clone())
            .how_things_are()
            .expect("readable");

        assert_eq!(how.stopped_by, None, "one account decided for all of them");
        assert_eq!(how.accounts_failing, 1, "and it is still counted");
    }

    #[test]
    fn every_account_failing_is_the_whole_thing_stopped() {
        let profile = workspace("activity-all", &[("acc-1", 1), ("acc-2", 2)]);
        let activity = SqliteAccountActivity::new(profile.pool.clone());

        for account in ["acc-1", "acc-2"] {
            activity
                .stopped(account, WhyItStopped::Offline)
                .expect("written");
        }

        let how = ActivityReadModel::new(profile.pool.clone())
            .how_things_are()
            .expect("readable");

        assert_eq!(how.stopped_by, Some(WhyItStopped::Offline));
        assert_eq!(how.accounts_failing, 2);
    }

    #[test]
    fn what_is_on_its_way_and_what_stopped_are_counted_apart() {
        // Two different sentences: one is a matter of waiting, the other is a matter of deciding.
        let profile = workspace("activity-queue", &[("acc-1", 1)]);

        for state in ["pending", "pending", "needs_reresolve", "conflicted"] {
            profile.queued("acc-1", state);
        }

        let how = ActivityReadModel::new(profile.pool.clone())
            .how_things_are()
            .expect("readable");

        assert_eq!(how.waiting, 3);
        assert_eq!(how.stopped, 1);
    }

    #[test]
    fn a_message_no_server_has_taken_yet_is_counted() {
        // The whole of what a person sees between pressing send and a server taking the message.
        // Counted by nothing, that stretch would be an application given a message and saying
        // nothing about it anywhere.
        let profile = workspace("activity-sending", &[("acc-1", 1)]);

        profile.written("acc-1", "waiting");
        profile.written("acc-1", "sending");
        profile.written("acc-1", "sent");
        profile.written("acc-1", "held");

        let how = ActivityReadModel::new(profile.pool.clone())
            .how_things_are()
            .expect("readable");

        assert_eq!(
            how.sending, 2,
            "waiting and sending, and neither of the rest"
        );
    }

    #[test]
    fn a_sent_message_let_go_once_its_copy_is_filed_changes_nothing_the_line_says() {
        // The line counts what is on its way and what came back, never what was sent, so a sent
        // message leaving the outbox once its copy is filed and read back is not news.
        use crate::infrastructure::sqlite::outbox::SqliteOutbox;
        use crate::ports::outbox::Outbox;

        let profile = workspace("activity-filed", &[("acc-1", 1)]);
        for state in ["waiting", "sending", "held", "sent"] {
            profile.written("acc-1", state);
        }
        profile.filed_and_kept("acc-1");
        let filing_now: i64 = profile
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT id FROM outbox WHERE state = 'sent' AND filed_at IS NULL",
                [],
                |row| row.get(0),
            )
            .expect("readable");
        let read = || {
            ActivityReadModel::new(profile.pool.clone())
                .how_things_are()
                .expect("readable")
        };
        let before = read();

        let outbox = SqliteOutbox::new(profile.pool.clone());
        outbox.filed(filing_now).expect("filed");
        outbox.forget_filed("acc-1").expect("let go");

        let left: i64 = profile
            .pool
            .get()
            .expect("connection")
            .query_row("SELECT count(*) FROM outbox", [], |row| row.get(0))
            .expect("readable");
        assert_eq!(left, 3, "the two sent messages were not let go");
        assert_eq!(read(), before);
        assert_eq!(before.sending, 2);
    }

    #[test]
    fn a_message_that_came_back_is_counted_while_its_draft_is_there() {
        // Until it is sent again or thrown away. Counted from the outbox instead, the line would say
        // so for good once one message had come back, about a draft long since sent or deleted.
        use crate::infrastructure::sqlite::drafts::SqliteDrafts;
        use crate::ports::drafts::{Draft, Drafts};

        let profile = workspace("activity-came-back", &[("acc-1", 1)]);
        let drafts = SqliteDrafts::new(profile.pool.clone());
        let read = || {
            ActivityReadModel::new(profile.pool.clone())
                .how_things_are()
                .expect("readable")
                .came_back
        };
        let mut back = Vec::new();
        for _ in 0..2 {
            let refused = profile.written("acc-1", "sending");
            back.push(
                drafts
                    .came_back("acc-1", refused, &Draft::default())
                    .expect("readable")
                    .expect("it is back"),
            );
        }
        assert_eq!(read(), 2, "the drafts they came back as are there");

        drafts
            .keep("acc-1", Some(back[0]), &Draft::default(), &[])
            .expect("written into again");
        assert_eq!(
            read(),
            2,
            "a draft that came back stopped saying so once it was kept"
        );

        // Sent again, the way the window sends one: the message written down, and its draft let go
        // once it is.
        profile.written("acc-1", "waiting");
        drafts.discard(back[0]).expect("let go once it is sent");
        assert_eq!(read(), 1, "the line outlived a draft that was sent");

        drafts.discard(back[1]).expect("thrown away");
        assert_eq!(read(), 0, "the line outlived a draft that was thrown away");
    }

    #[test]
    fn a_message_back_in_the_drafts_is_not_also_counted_as_on_its_way() {
        // Taken by a round, late, and given back: the line says it came back, and once. Its row
        // goes in the change that writes its draft, so there is no moment at which it is both in
        // the drafts and on its way.
        use crate::infrastructure::sqlite::drafts::SqliteDrafts;
        use crate::ports::drafts::{Draft, Drafts};

        let profile = workspace("activity-back-once", &[("acc-1", 1)]);
        let taken = profile.written("acc-1", "sending");
        let read = || {
            ActivityReadModel::new(profile.pool.clone())
                .how_things_are()
                .expect("readable")
        };
        assert_eq!(
            (read().sending, read().came_back),
            (1, 0),
            "the message was not on its way to begin with"
        );

        SqliteDrafts::new(profile.pool.clone())
            .came_back("acc-1", taken, &Draft::default())
            .expect("readable")
            .expect("it is back");

        assert_eq!((read().sending, read().came_back), (0, 1));
    }

    #[test]
    fn neither_a_held_message_that_names_no_draft_nor_a_draft_kept_from_the_window_came_back() {
        // A held row that names no draft leaves nothing to say the message is still there to be
        // sent, and a draft somebody kept is only a draft.
        let profile = workspace("activity-not-came-back", &[("acc-1", 1)]);
        profile.written("acc-1", "held");
        profile.kept_draft("acc-1");

        let how = ActivityReadModel::new(profile.pool.clone())
            .how_things_are()
            .expect("readable");

        assert_eq!(how.came_back, 0);
    }

    #[test]
    fn a_message_on_its_way_is_not_counted_as_an_instruction() {
        // Two different sentences about two different things, and the outbox had been left out of
        // both: "changes are not on the server yet" is about a mailbox, not about somebody's mail.
        let profile = workspace("activity-apart", &[("acc-1", 1)]);

        profile.written("acc-1", "waiting");

        let how = ActivityReadModel::new(profile.pool.clone())
            .how_things_are()
            .expect("readable");

        assert_eq!((how.waiting, how.stopped), (0, 0));
        assert_eq!(how.sending, 1);
    }

    #[test]
    fn a_change_asked_for_a_moment_ago_is_not_news() {
        // It is on its way, which is what every change is for the second or two a push takes.
        // Said at the foot of the window, it would turn opening a message into a line about the
        // server -- the background talking to a person about work that is going fine.
        let profile = workspace("activity-fresh", &[("acc-1", 1)]);

        profile.queued_just_now("acc-1");
        profile.written_just_now("acc-1", 0);

        let how = ActivityReadModel::new(profile.pool.clone())
            .how_things_are()
            .expect("readable");

        assert_eq!((how.waiting, how.sending), (0, 0));
    }

    /// Instructions about messages 1, 2, … in folder 1, each marking one read, handed out once.
    fn handed_out(
        profile: &SeededProfile,
        messages: &[u64],
    ) -> (
        crate::infrastructure::sqlite::op_log::SqliteOperationLog,
        Vec<crate::ports::op_log::OperationId>,
    ) {
        use crate::ports::op_log::OperationLog;
        use lyonoyl_domain::message::{Flags, FolderId, MessageId};
        use lyonoyl_domain::operation::{MailOperation, OperationKind, OperationTarget};

        let log =
            crate::infrastructure::sqlite::op_log::SqliteOperationLog::new(profile.pool.clone());
        for message in messages {
            let asked = MailOperation::asked(
                OperationTarget::new(
                    MessageId::new(*message).expect("non-zero"),
                    FolderId::new(1).expect("non-zero"),
                ),
                OperationKind::SetFlags {
                    add: Flags::SEEN,
                    remove: Flags::NONE,
                },
            );
            log.record("acc-1", &[], asked).expect("it is written");
        }
        let ids = log
            .due("acc-1", 10)
            .expect("readable")
            .iter()
            .map(|one| one.id)
            .collect();
        (log, ids)
    }

    #[test]
    fn a_change_the_server_heard_and_did_not_take_is_news_at_once() {
        // The quiet is for work that is going fine. One the server was told and answered "not
        // now" has been tried and not taken, which is the thing the line is there to say, however
        // young it is. One whose server could not be reached has not been tried at all: nothing
        // is counted against it, and it is mentioned once it is older than a push and a round.
        use crate::ports::op_log::OperationLog;
        use lyonoyl_domain::operation::OperationState;

        let profile = workspace("activity-tried", &[("acc-1", 1)]);
        let (log, handed) = handed_out(&profile, &[1, 2]);

        log.about_to_tell(handed[0]).expect("on its way");
        log.attempted(
            handed[0],
            OperationState::Pending,
            None,
            std::time::Duration::from_secs(60),
        )
        .expect("the server said not now");
        log.release(&handed[1..]).expect("never said at all");
        profile.written_just_now("acc-1", 1);

        let how = ActivityReadModel::new(profile.pool.clone())
            .how_things_are()
            .expect("readable");

        assert_eq!((how.waiting, how.sending), (1, 1));
    }

    #[test]
    fn a_way_back_waiting_for_this_devices_own_read_is_not_news() {
        // Archived and put back at once: the way back waits for the pass reading the archive,
        // which the push runs a moment later. Nothing is late, and the foot of the window says
        // nothing about an undo that is going exactly as it should.
        use crate::ports::op_log::OperationLog;
        use lyonoyl_domain::operation::OperationState;

        let profile = workspace("activity-way-back", &[("acc-1", 1)]);
        let (log, handed) = handed_out(&profile, &[1]);

        log.attempted(
            handed[0],
            OperationState::NeedsReresolve,
            None,
            std::time::Duration::ZERO,
        )
        .expect("waiting for the read");
        log.folder_found(
            "acc-1",
            lyonoyl_domain::message::FolderId::new(1).expect("non-zero"),
        )
        .expect("the read came");

        let how = ActivityReadModel::new(profile.pool.clone())
            .how_things_are()
            .expect("readable");

        assert_eq!(how.waiting, 0);
    }

    #[test]
    fn what_reached_the_server_is_counted_as_neither() {
        let profile = workspace("activity-done", &[("acc-1", 1)]);

        profile.queued("acc-1", "done");

        let how = ActivityReadModel::new(profile.pool.clone())
            .how_things_are()
            .expect("readable");

        assert_eq!((how.waiting, how.stopped), (0, 0));
    }
}
