use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use super::*;
use crate::ports::settings_repository::SettingsError;
use lyonoyl_domain::preferences::Preferences;

struct Settings {
    notice: NewMailNotice,
    shows: NoticeShows,
    readable: bool,
}

impl PreferencesRepository for Settings {
    fn load(&self) -> Result<Preferences, SettingsError> {
        if !self.readable {
            return Err(SettingsError::Unavailable);
        }

        Ok(Preferences::default()
            .with_new_mail_notice(self.notice)
            .with_notice_shows(self.shows))
    }

    fn save(&self, _preferences: &Preferences) -> Result<(), SettingsError> {
        Ok(())
    }
}

struct Looking(bool);

impl Attention for Looking {
    fn has_it(&self) -> bool {
        self.0
    }
}

#[derive(Default)]
struct Shown {
    notices: Mutex<Vec<Notice>>,
    refuses: bool,
}

impl Notifier for &Shown {
    fn show(&self, notice: &Notice) -> Result<(), NotifyError> {
        if self.refuses {
            return Err(NotifyError::Refused);
        }

        self.notices
            .lock()
            .expect("not poisoned")
            .push(notice.clone());
        Ok(())
    }
}

/// What arrived, and a count of how often anybody asked.
///
/// The count is half the subject: a notice that may say nothing about the mail must not go to the
/// database to find that out, and nothing about the notice it produces would show that it did.
#[derive(Default)]
struct Waiting {
    arrived: Option<Arrived>,
    unreadable: bool,
    asked: AtomicUsize,
}

impl Arrivals for &Waiting {
    fn newest_unseen(&self) -> Option<Arrived> {
        self.asked.fetch_add(1, Ordering::SeqCst);

        // A file that would not open reaches the caller as nothing to say about the mail, which is
        // the same answer an empty inbox gives and the same thing to do about it.
        match self.unreadable {
            true => None,
            false => self.arrived.clone(),
        }
    }
}

fn from_somebody(sender: &str, subject: &str, preview: &str) -> Waiting {
    Waiting {
        arrived: Some(Arrived {
            sender: Some(sender.to_owned()),
            subject: Some(subject.to_owned()),
            preview: Some(preview.to_owned()),
        }),
        ..Waiting::default()
    }
}

fn deciding(
    unseen: u32,
    looking: bool,
    notice: NewMailNotice,
    readable: bool,
    shown: &Shown,
) -> Result<Result<(), Silent>, NotifyError> {
    announcing(
        unseen,
        looking,
        notice,
        NoticeShows::default(),
        readable,
        shown,
        &Waiting::default(),
    )
}

#[allow(clippy::too_many_arguments)]
fn announcing(
    unseen: u32,
    looking: bool,
    notice: NewMailNotice,
    shows: NoticeShows,
    readable: bool,
    shown: &Shown,
    waiting: &Waiting,
) -> Result<Result<(), Silent>, NotifyError> {
    AnnounceMail::new(
        Settings {
            notice,
            shows,
            readable,
        },
        Looking(looking),
        shown,
        waiting,
        "Lyonoyl Mail",
    )
    .execute(unseen, "New mail")
}

/// What a notice said, where exactly one was shown.
fn the_one(shown: &Shown) -> Notice {
    let notices = shown.notices.lock().expect("not poisoned");
    assert_eq!(notices.len(), 1, "one notice, and one only");
    notices[0].clone()
}

#[test]
fn mail_nobody_has_seen_is_worth_saying_when_nobody_is_looking() {
    let shown = Shown::default();

    let answer = deciding(3, false, NewMailNotice::Inbox, true, &shown).expect("shown");

    assert_eq!(answer, Ok(()));
    let said = the_one(&shown);
    assert_eq!(said.title, "Lyonoyl Mail");
    assert_eq!(said.body, "New mail");
}

#[test]
fn a_notice_says_who_wrote_what_about_and_how_it_opens() {
    // The whole of what was asked for. A notice saying only that mail arrived does not answer the
    // question somebody looks at it to answer, so they open the application to find out — which is
    // the interruption it was supposed to save them.
    let shown = Shown::default();
    let waiting = from_somebody("Zeta Quorin", "Lunch on Friday?", "Are you free at one");

    announcing(
        1,
        false,
        NewMailNotice::Inbox,
        NoticeShows::default(),
        true,
        &shown,
        &waiting,
    )
    .expect("shown")
    .expect("said");

    let said = the_one(&shown);
    assert_eq!(said.title, "Zeta Quorin", "who it is from goes first");
    assert_eq!(said.body, "Lunch on Friday?\nAre you free at one");
}

#[test]
fn each_part_can_be_turned_off_by_itself() {
    // Three switches rather than one. Somebody content with a subject and not a sender has no way
    // to say so with a single "show details", and being told to choose all or nothing is how a
    // person ends up choosing nothing.
    for (shows, title, body) in [
        (
            NoticeShows {
                sender: false,
                ..NoticeShows::default()
            },
            "Lyonoyl Mail",
            "Lunch on Friday?\nAre you free at one",
        ),
        (
            NoticeShows {
                subject: false,
                ..NoticeShows::default()
            },
            "Zeta Quorin",
            "Are you free at one",
        ),
        (
            NoticeShows {
                preview: false,
                ..NoticeShows::default()
            },
            "Zeta Quorin",
            "Lunch on Friday?",
        ),
    ] {
        let shown = Shown::default();
        let waiting = from_somebody("Zeta Quorin", "Lunch on Friday?", "Are you free at one");

        announcing(
            1,
            false,
            NewMailNotice::Inbox,
            shows,
            true,
            &shown,
            &waiting,
        )
        .expect("shown")
        .expect("said");

        let said = the_one(&shown);
        assert_eq!(said.title, title, "{shows:?}");
        assert_eq!(said.body, body, "{shows:?}");
    }
}

#[test]
fn with_every_part_off_the_mail_is_not_even_read() {
    // Silence about the message is not the same as reading it and saying nothing. Asked anyway,
    // this is a question put to the database every time mail arrives for an answer nobody sees —
    // and nothing about the notice that comes out would show that it happened.
    let shown = Shown::default();
    let waiting = from_somebody("Zeta Quorin", "Lunch on Friday?", "Are you free at one");
    let nothing = NoticeShows {
        sender: false,
        subject: false,
        preview: false,
    };

    announcing(
        1,
        false,
        NewMailNotice::Inbox,
        nothing,
        true,
        &shown,
        &waiting,
    )
    .expect("shown")
    .expect("said");

    assert_eq!(
        waiting.asked.load(Ordering::SeqCst),
        0,
        "the mail was read for a notice that could not mention it"
    );

    let said = the_one(&shown);
    assert_eq!(said.title, "Lyonoyl Mail");
    assert_eq!(said.body, "New mail", "and it says what it always said");
}

#[test]
fn a_message_cannot_write_the_notice_around_itself() {
    // Every one of these is a stranger's text, and a notification is drawn by the system on a
    // surface this application does not own and cannot withdraw. A right-to-left override reverses
    // what is shown after it, and a newline in a subject makes one field read as two.
    let shown = Shown::default();
    let waiting = from_somebody(
        "Zeta \u{202E}Quorin",
        "Approved\nFrom: your bank",
        "nothing\u{0007}to see",
    );

    announcing(
        1,
        false,
        NewMailNotice::Inbox,
        NoticeShows::default(),
        true,
        &shown,
        &waiting,
    )
    .expect("shown")
    .expect("said");

    let said = the_one(&shown);
    assert!(!said.title.contains('\u{202E}'), "{}", said.title);
    assert_eq!(said.body, "Approved From: your bank\nnothingto see");
    assert_eq!(
        said.body.matches('\n').count(),
        1,
        "the only line break is the one put between the two parts: {}",
        said.body
    );
}

#[test]
fn a_message_that_named_nobody_is_headed_by_the_product() {
    // Rather than by a blank, which is what a notice with an empty title renders as.
    let shown = Shown::default();
    let waiting = Waiting {
        arrived: Some(Arrived {
            sender: None,
            subject: Some("Lunch on Friday?".to_owned()),
            preview: None,
        }),
        ..Waiting::default()
    };

    announcing(
        1,
        false,
        NewMailNotice::Inbox,
        NoticeShows::default(),
        true,
        &shown,
        &waiting,
    )
    .expect("shown")
    .expect("said");

    let said = the_one(&shown);
    assert_eq!(said.title, "Lyonoyl Mail");
    assert_eq!(said.body, "Lunch on Friday?");
}

#[test]
fn a_message_with_nothing_legible_in_it_still_announces_itself() {
    // A subject of nothing but marks that cannot be shown is a message like any other, and mail
    // arrived either way. What must not happen is a notice with an empty body.
    let shown = Shown::default();
    let waiting = from_somebody("  ", "\u{202E}", "   \n  ");

    announcing(
        1,
        false,
        NewMailNotice::Inbox,
        NoticeShows::default(),
        true,
        &shown,
        &waiting,
    )
    .expect("shown")
    .expect("said");

    let said = the_one(&shown);
    assert_eq!(said.title, "Lyonoyl Mail");
    assert_eq!(said.body, "New mail");
}

#[test]
fn mail_that_cannot_be_read_is_still_announced() {
    // The notice is about mail arriving, and that much is known from the count. A database that
    // would not answer is a reason to say less, not a reason to say nothing.
    let shown = Shown::default();
    let waiting = Waiting {
        unreadable: true,
        ..Waiting::default()
    };

    let answer = announcing(
        2,
        false,
        NewMailNotice::Inbox,
        NoticeShows::default(),
        true,
        &shown,
        &waiting,
    )
    .expect("shown");

    assert_eq!(answer, Ok(()));
    let said = the_one(&shown);
    assert_eq!(said.title, "Lyonoyl Mail");
    assert_eq!(said.body, "New mail");
}

#[test]
fn what_is_too_long_for_a_notice_is_cut_rather_than_shown_whole() {
    // The system cuts what does not fit, and where it cuts is its business. What this decides is
    // that a subject of ten thousand characters is not handed over at all.
    let shown = Shown::default();
    let waiting = from_somebody(&"n".repeat(500), &"s".repeat(5_000), &"p".repeat(9_000));

    announcing(
        1,
        false,
        NewMailNotice::Inbox,
        NoticeShows::default(),
        true,
        &shown,
        &waiting,
    )
    .expect("shown")
    .expect("said");

    let said = the_one(&shown);
    assert_eq!(said.title.chars().count(), NAME_AT_MOST);
    assert_eq!(
        said.body.chars().count(),
        SUBJECT_AT_MOST + PREVIEW_AT_MOST + 1,
        "two parts and the break between them"
    );
}

#[test]
fn the_count_decides_whether_to_speak_and_never_what_is_said() {
    // A number needs plural rules, plural rules live in the interface, and a count of somebody's
    // unread mail on a locked screen buys nothing: what they do about it is open the application
    // either way.
    let shown = Shown::default();

    let answer = deciding(41, false, NewMailNotice::Inbox, true, &shown).expect("shown");
    assert_eq!(answer, Ok(()));

    let said = the_one(&shown);
    assert_eq!(said.body, "New mail");
    assert!(!said.body.contains("41"));
}

#[test]
fn nothing_new_is_nothing_to_say() {
    let shown = Shown::default();

    let answer = deciding(0, false, NewMailNotice::Inbox, true, &shown).expect("no failure");

    assert_eq!(answer, Err(Silent::NothingNew));
    assert!(shown.notices.lock().expect("not poisoned").is_empty());
}

#[test]
fn somebody_reading_their_mail_is_not_told_that_mail_arrived() {
    // The application talking over itself. This is the commonest reason for silence and the one
    // whose absence would be noticed first.
    let shown = Shown::default();

    let answer = deciding(5, true, NewMailNotice::Inbox, true, &shown).expect("no failure");

    assert_eq!(answer, Err(Silent::TheyAreLooking));
    assert!(shown.notices.lock().expect("not poisoned").is_empty());
}

#[test]
fn somebody_who_turned_it_off_stays_off() {
    let shown = Shown::default();

    let answer = deciding(5, false, NewMailNotice::Never, true, &shown).expect("no failure");

    assert_eq!(answer, Err(Silent::TheyAskedNotToBe));
    assert!(shown.notices.lock().expect("not poisoned").is_empty());
}

#[test]
fn settings_that_cannot_be_read_mean_silence_rather_than_a_guess() {
    // A store that will not open is not a reason to start interrupting somebody who may have
    // turned this off. Unsure is silent.
    let shown = Shown::default();

    let answer = deciding(5, false, NewMailNotice::Inbox, false, &shown).expect("no failure");

    assert_eq!(answer, Err(Silent::SettingsUnreadable));
    assert!(shown.notices.lock().expect("not poisoned").is_empty());
}

#[test]
fn the_settings_are_not_even_read_while_somebody_is_looking() {
    // Ordering, not decoration: the cheap question first. A store read once per round for an
    // answer that was already decided is a hundred reads a person paid for and nobody used.
    let shown = Shown::default();

    let answer = deciding(5, true, NewMailNotice::Inbox, false, &shown).expect("no failure");

    assert_eq!(
        answer,
        Err(Silent::TheyAreLooking),
        "an unreadable store is not reached when the window has the focus"
    );
}

#[test]
fn a_system_that_refuses_says_so_rather_than_pretending() {
    let shown = Shown {
        refuses: true,
        ..Shown::default()
    };

    assert_eq!(
        deciding(5, false, NewMailNotice::Inbox, true, &shown),
        Err(NotifyError::Refused)
    );
}
