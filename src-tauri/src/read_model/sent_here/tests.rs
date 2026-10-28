use super::*;
use crate::infrastructure::fake_provider::SeededIndex;
use crate::infrastructure::sqlite::outbox::SqliteOutbox;
use crate::ports::outbox::{Outbox, WhyHeld};
use lyonoyl_domain::message::FolderId;
use lyonoyl_domain::outgoing::{EmailAddress, Outgoing, OutgoingFile, Subject};

/// An account with an inbox and a folder for sent mail, holding nothing yet.
fn account(name: &str) -> SeededIndex {
    let index = SeededIndex::with(name, FolderId::new(1).expect("non-zero"), 0);
    index.add_folder(2, "Sent", FolderRole::Sent);
    index
}

fn address(value: &str) -> EmailAddress {
    EmailAddress::parse(value).expect("valid")
}

/// A message from Zeta to Kappa, copied to Sigma.
fn message(subject: &str) -> Outgoing {
    Outgoing {
        from: address("zeta@example.com"),
        to: vec![address("kappa@example.net")],
        cc: vec![address("sigma@example.org")],
        bcc: Vec::new(),
        subject: Subject::parse(subject).expect("a subject"),
        body: "Shall we?".to_owned(),
        thread: Default::default(),
        files: Vec::new(),
    }
}

/// Writes a message down as pressing send does, from `account_id`, and answers its row.
fn written_from(
    index: &SeededIndex,
    account_id: &str,
    message_id: &str,
    message: &Outgoing,
) -> i64 {
    SqliteOutbox::new(index.profile.clone())
        .queue(account_id, message_id, message)
        .expect("written down");

    index
        .profile
        .get()
        .expect("a connection")
        .query_row(
            "SELECT id FROM outbox WHERE message_id = ?1",
            [message_id],
            |row| row.get(0),
        )
        .expect("its row")
}

fn written(index: &SeededIndex, message_id: &str, subject: &str) -> i64 {
    written_from(index, "acc-1", message_id, &message(subject))
}

/// Handed over and taken by a server.
fn taken(index: &SeededIndex, id: i64) {
    let outbox = SqliteOutbox::new(index.profile.clone());
    assert_eq!(outbox.claim(id), Ok(true), "the message was not waiting");
    outbox.accepted(id).expect("taken by a server");
}

fn shown(index: &SeededIndex) -> Vec<SentHere> {
    of(&index.profile, &index.pool, "acc-1", 1).expect("readable")
}

fn subjects(index: &SeededIndex) -> Vec<String> {
    shown(index).into_iter().map(|sent| sent.subject).collect()
}

#[test]
fn a_message_no_server_has_taken_yet_is_shown_on_its_way_with_everything_it_carries() {
    let index = account("sent-here-whole");
    let mut message = message("About Tuesday");
    message.bcc = vec![address("records@example.com")];
    message.body = "Shall we\nmeet   at noon?\n".to_owned();
    message.files = vec![
        OutgoingFile {
            name: "agenda.pdf".to_owned(),
            content_type: "application/pdf".to_owned(),
            bytes: vec![1, 2, 3],
        },
        OutgoingFile {
            name: "notes.txt".to_owned(),
            content_type: "text/plain".to_owned(),
            bytes: b"notes".to_vec(),
        },
    ];
    let id = written_from(&index, "acc-1", "<whole@example.com>", &message);
    let written_at: i64 = index
        .profile
        .get()
        .expect("a connection")
        .query_row("SELECT written_at FROM outbox WHERE id = ?1", [id], |row| {
            row.get(0)
        })
        .expect("its date");

    assert_eq!(
        shown(&index),
        vec![SentHere {
            id,
            sender: "zeta@example.com".to_owned(),
            to: vec!["kappa@example.net".to_owned()],
            cc: vec!["sigma@example.org".to_owned()],
            bcc: vec!["records@example.com".to_owned()],
            subject: "About Tuesday".to_owned(),
            body: "Shall we\nmeet   at noon?\n".to_owned(),
            preview: Some("Shall we meet at noon?".to_owned()),
            files: vec!["agenda.pdf".to_owned(), "notes.txt".to_owned()],
            written_at,
            on_its_way: true,
        }]
    );
    assert!(written_at > 0, "the fixture wrote no date");
}

#[test]
fn a_message_being_handed_over_is_on_its_way() {
    let index = account("sent-here-sending");
    let id = written(&index, "<sending@example.com>", "Being handed over");
    assert_eq!(SqliteOutbox::new(index.profile.clone()).claim(id), Ok(true));

    let shown = shown(&index);

    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(shown[0].on_its_way, "{shown:?}");
}

#[test]
fn a_message_a_server_took_is_shown_as_sent() {
    let index = account("sent-here-taken");
    let id = written(&index, "<taken@example.com>", "Taken");
    taken(&index, id);

    let shown = shown(&index);

    assert_eq!(shown.len(), 1, "{shown:?}");
    assert!(!shown[0].on_its_way, "{shown:?}");
}

#[test]
fn a_message_whose_copy_is_filed_is_shown_until_the_index_holds_the_copy() {
    // Filed, and the account not read back since: the server has the copy and this device does not
    // know it yet, so this is the only place the message can be shown from.
    let index = account("sent-here-filed");
    let id = written(&index, "<filed@example.com>", "Filed");
    taken(&index, id);
    SqliteOutbox::new(index.profile.clone())
        .filed(id)
        .expect("filed");

    assert_eq!(subjects(&index), vec!["Filed".to_owned()]);
}

#[test]
fn a_message_whose_copy_the_index_holds_in_sent_is_not_shown_twice() {
    // The outbox keeps the header's value in its angle brackets and the index without them: the
    // copy is found all the same.
    let index = account("sent-here-in-sent");
    let id = written(&index, "<copied@example.com>", "Copied");
    taken(&index, id);
    index.holds_a_copy_of(2, 1, "copied@example.com");

    assert_eq!(shown(&index), Vec::new());
}

#[test]
fn a_copy_anywhere_but_sent_hides_nothing() {
    // Somebody who wrote to themselves has it in the inbox before the copy reaches Sent, and the
    // message stays in Sent meanwhile.
    let index = account("sent-here-in-inbox");
    let id = written(&index, "<to-myself@example.com>", "To myself");
    taken(&index, id);
    index.holds_a_copy_of(1, 1, "to-myself@example.com");

    assert_eq!(subjects(&index), vec!["To myself".to_owned()]);
}

#[test]
fn a_message_held_is_not_sent_and_is_not_shown() {
    let index = account("sent-here-held");
    let id = written(&index, "<held@example.com>", "Held");
    let outbox = SqliteOutbox::new(index.profile.clone());
    assert_eq!(outbox.claim(id), Ok(true));
    outbox
        .stopped(id, WhyHeld::TheServerRefused, None)
        .expect("held");

    assert_eq!(shown(&index), Vec::new());
}

#[test]
fn a_message_given_back_to_the_drafts_is_a_draft_and_is_not_shown() {
    let index = account("sent-here-came-back");
    let id = written(&index, "<came-back@example.com>", "Came back");
    index.names_its_draft(id);

    assert_eq!(shown(&index), Vec::new());
}

#[test]
fn what_another_account_sent_is_not_shown() {
    let index = account("sent-here-other-account");
    index.add_account("acc-2", 2);
    written_from(
        &index,
        "acc-2",
        "<elsewhere@example.com>",
        &message("Elsewhere"),
    );
    written(&index, "<here@example.com>", "Here");

    assert_eq!(subjects(&index), vec!["Here".to_owned()]);
}

#[test]
fn the_newest_is_shown_first_by_when_it_was_written() {
    // Dated against the order the rows were written in, so the date decides and not the row: the
    // list under these orders by date, and the two must agree.
    let index = account("sent-here-order");
    let older_row_newer_date = written(&index, "<first@example.com>", "Written second");
    let newer_row_older_date = written(&index, "<second@example.com>", "Written first");
    index.date_written(older_row_newer_date, 1_700_000_200);
    index.date_written(newer_row_older_date, 1_700_000_100);
    let tied = written(&index, "<third@example.com>", "Tied, later row");
    index.date_written(tied, 1_700_000_100);

    assert_eq!(
        subjects(&index),
        vec![
            "Written second".to_owned(),
            "Tied, later row".to_owned(),
            "Written first".to_owned(),
        ]
    );
}

#[test]
fn a_message_whose_name_does_not_read_as_one_is_still_shown() {
    // Nothing the index holds can match it, so nothing hides it; and one row is not a reason to
    // show none of the others.
    let index = account("sent-here-unnamed");
    written(&index, "", "Unnamed");
    written(&index, "<named@example.com>", "Named");

    let mut shown = subjects(&index);
    shown.sort();

    assert_eq!(shown, vec!["Named".to_owned(), "Unnamed".to_owned()]);
}

#[test]
fn an_account_that_sent_nothing_shows_nothing() {
    let index = account("sent-here-nothing");

    assert_eq!(shown(&index), Vec::new());
}

#[test]
fn an_index_that_cannot_be_read_is_said_rather_than_taken_for_one_holding_nothing() {
    // Shown regardless, the copy that is in Sent would be shown twice; left out, the message would
    // be missing with nothing saying why.
    let index = account("sent-here-unreadable");
    written(&index, "<unreadable@example.com>", "Unreadable");
    index.stops_answering();

    assert_eq!(
        of(&index.profile, &index.pool, "acc-1", 1),
        Err(ReadModelError::Unavailable)
    );
}

#[test]
fn an_opening_is_one_line_cut_where_the_list_cuts_one() {
    assert_eq!(
        preview_of("  Shall we\r\n\tmeet?  "),
        Some("Shall we meet?".to_owned())
    );
    assert_eq!(preview_of(" \n\t "), None);
    assert_eq!(preview_of(""), None);

    // Counted in characters, not bytes: a cut by bytes lands inside a letter.
    let long = "\u{e9}".repeat(PREVIEW_CHARS + 1);
    assert_eq!(
        preview_of(&long),
        Some("\u{e9}".repeat(PREVIEW_CHARS)),
        "cut anywhere but at the length the list is given"
    );
    let exact = "a".repeat(PREVIEW_CHARS);
    assert_eq!(preview_of(&exact), Some(exact.clone()));
}
