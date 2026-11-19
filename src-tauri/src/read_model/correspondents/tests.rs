use super::*;
use crate::application::sync::SyncFolder;
use crate::infrastructure::fake_provider::{ScriptedProvider, SeededIndex};
use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
use lyonoyl_domain::message::{Flags, FolderId, Rfc822MessageId, Uid, UidValidity};
use lyonoyl_domain::sync::{
    ArrivedMessage, FolderDelta, MessageEnvelope, SyncBatch, SyncCursor, SyncScope, VanishedReport,
};

const INBOX: i64 = 1;
const SENT: i64 = 2;

/// An account with an inbox and a folder for sent mail, holding nothing yet.
fn account(name: &str) -> SeededIndex {
    let index = SeededIndex::with(name, FolderId::new(INBOX as u64).expect("non-zero"), 0);
    index.add_folder(SENT, "Sent", FolderRole::Sent);
    index
}

/// A message as a synchronisation stores it: `sender` is the name when the `From` gave one and the
/// address when it did not, and `to` and `cc` are addresses alone.
struct Message<'a> {
    id: &'a str,
    from: (&'a str, &'a str),
    to: &'a [&'a str],
    cc: &'a [&'a str],
    at: i64,
}

impl Message<'_> {
    fn envelope(&self) -> MessageEnvelope {
        MessageEnvelope {
            rfc822_message_id: Some(
                Rfc822MessageId::parse(&format!("<{}@example.com>", self.id)).expect("well formed"),
            ),
            subject: Some("About Tuesday".to_owned()),
            sender: Some(self.from.0.to_owned()),
            sender_address: Some(self.from.1.to_owned()),
            to: self
                .to
                .iter()
                .map(|address| (*address).to_owned())
                .collect(),
            cc: self
                .cc
                .iter()
                .map(|address| (*address).to_owned())
                .collect(),
            received_at: self.at,
            rfc822_size: Some(1024),
            preview: None,
            references: Vec::new(),
            flags: Flags::SEEN,
        }
    }
}

const ME: (&str, &str) = ("User", "user@example.com");

/// Delivers messages into a folder through the synchronisation that stores every other message,
/// continuing from whatever the folder was read to.
fn deliver(index: &SeededIndex, folder: i64, messages: &[Message<'_>]) {
    let stored: Option<String> = index
        .pool
        .get()
        .expect("connection")
        .query_row(
            "SELECT sync_cursor FROM folders WHERE id = ?1",
            [folder],
            |row| row.get(0),
        )
        .expect("the folder is there");
    let read_to: u32 = stored
        .as_deref()
        .map(|cursor| cursor.parse().expect("a cursor this file wrote"))
        .unwrap_or(0);
    let first = read_to + 1;
    let folder_id = FolderId::new(folder as u64).expect("non-zero");

    let batch = SyncBatch {
        scope: SyncScope::Mailbox,
        cursor_before: stored.as_deref().map(SyncCursor::new),
        cursor_after: SyncCursor::new(format!("{}", read_to + messages.len() as u32)),
        per_mailbox: vec![FolderDelta {
            folder_id,
            uid_validity: UidValidity::new(7).expect("non-zero"),
            covered_from: Uid::new(first).ok(),
            stopped_short: false,
            arrived: messages
                .iter()
                .enumerate()
                .map(|(at, message)| ArrivedMessage {
                    uid: Uid::new(first + at as u32).expect("non-zero"),
                    modseq: None,
                    envelope: message.envelope(),
                })
                .collect(),
            flag_updates: Vec::new(),
            vanished: VanishedReport::Unknown,
        }],
    };

    SyncFolder::new(
        ScriptedProvider::new(vec![Ok(batch)]),
        SqliteMailIndex::new(index.pool.clone()),
    )
    .execute(folder_id)
    .expect("the pass applies");
}

/// Who is offered for `typed`, as address and name.
fn offered_for(
    correspondents: &Correspondents,
    index: &SeededIndex,
    account_id: &str,
    typed: &str,
) -> Vec<(String, Option<String>)> {
    correspondents
        .offered(account_id, &index.pool, typed)
        .expect("readable")
        .into_iter()
        .map(|person| (person.address, person.name))
        .collect()
}

fn kappa(name: Option<&str>) -> (String, Option<String>) {
    ("kappa@example.net".to_owned(), name.map(str::to_owned))
}

#[test]
fn the_people_a_sent_message_went_to_are_offered_with_the_names_their_own_mail_gives_them() {
    let index = account("correspondents-sent");
    deliver(
        &index,
        SENT,
        &[Message {
            id: "one",
            from: ME,
            to: &["kappa@example.net"],
            cc: &["sigma@example.org"],
            at: 100,
        }],
    );
    deliver(
        &index,
        INBOX,
        &[Message {
            id: "two",
            from: ("Kappa Vandel", "kappa@example.net"),
            to: &["user@example.com"],
            cc: &[],
            at: 200,
        }],
    );
    let correspondents = Correspondents::default();

    assert_eq!(
        offered_for(&correspondents, &index, "acc-1", "kap"),
        vec![kappa(Some("Kappa Vandel"))]
    );
    assert_eq!(
        offered_for(&correspondents, &index, "acc-1", "vandel"),
        vec![kappa(Some("Kappa Vandel"))]
    );
    assert_eq!(
        offered_for(&correspondents, &index, "acc-1", "sig"),
        vec![("sigma@example.org".to_owned(), None)]
    );
}

#[test]
fn the_people_a_message_anywhere_else_went_to_are_not_offered_nor_who_sent_it() {
    let index = account("correspondents-elsewhere");
    index.add_folder(3, "Archive", FolderRole::Archive);
    let somebody_elses = Message {
        id: "one",
        from: ("Zeta Quorin", "zeta@example.com"),
        to: &["user@example.com"],
        cc: &["sigma@example.org"],
        at: 100,
    };
    deliver(&index, INBOX, &[somebody_elses]);
    deliver(
        &index,
        3,
        &[Message {
            id: "two",
            from: ("Zeta Quorin", "zeta@example.com"),
            to: &["kappa@example.net"],
            cc: &[],
            at: 200,
        }],
    );
    let correspondents = Correspondents::default();

    for typed in ["sig", "zeta", "quorin", "kap", "user"] {
        assert_eq!(
            offered_for(&correspondents, &index, "acc-1", typed),
            Vec::<(String, Option<String>)>::new(),
            "{typed} found somebody the account never wrote to"
        );
    }
}

#[test]
fn a_message_in_two_folders_for_sent_mail_was_sent_once() {
    let index = account("correspondents-twice");
    index.add_folder(3, "Sent Messages", FolderRole::Sent);
    let sent = Message {
        id: "one",
        from: ME,
        to: &["kappa@example.net"],
        cc: &[],
        at: 100,
    };
    deliver(&index, SENT, std::slice::from_ref(&sent));
    deliver(&index, 3, &[sent]);
    let counted = |sql: &str| -> i64 {
        index
            .pool
            .get()
            .expect("connection")
            .query_row(sql, [], |row| row.get(0))
            .expect("counted")
    };
    assert_eq!(
        (
            counted("SELECT count(*) FROM messages"),
            counted("SELECT count(*) FROM message_placements")
        ),
        (1, 2),
        "the fixture did not put one message in two folders"
    );

    let correspondents = Correspondents::default();
    let people = correspondents
        .offered("acc-1", &index.pool, "kappa")
        .expect("readable");

    assert_eq!(people.len(), 1);
    assert_eq!(people[0].times, 1);
}

#[test]
fn what_is_kept_is_gathered_again_once_a_newer_message_arrives() {
    let index = account("correspondents-again");
    deliver(
        &index,
        SENT,
        &[Message {
            id: "one",
            from: ME,
            to: &["sigma@example.org"],
            cc: &[],
            at: 100,
        }],
    );
    let correspondents = Correspondents::default();
    assert_eq!(
        offered_for(&correspondents, &index, "acc-1", "kap"),
        Vec::<(String, Option<String>)>::new()
    );

    deliver(
        &index,
        SENT,
        &[Message {
            id: "two",
            from: ME,
            to: &["kappa@example.net"],
            cc: &[],
            at: 200,
        }],
    );
    assert_eq!(
        offered_for(&correspondents, &index, "acc-1", "kap"),
        vec![kappa(None)]
    );

    // A name arrives with mail from them, which is a newer message too.
    deliver(
        &index,
        INBOX,
        &[Message {
            id: "three",
            from: ("Kappa Vandel", "kappa@example.net"),
            to: &["user@example.com"],
            cc: &[],
            at: 300,
        }],
    );
    assert_eq!(
        offered_for(&correspondents, &index, "acc-1", "kap"),
        vec![kappa(Some("Kappa Vandel"))]
    );
}

#[test]
fn what_one_account_has_written_is_never_offered_for_another() {
    // Two files, as two accounts are: each the same shape, so what tells them apart is only
    // whose mail it is.
    let first = account("correspondents-first");
    let second = account("correspondents-second");
    deliver(
        &first,
        SENT,
        &[Message {
            id: "one",
            from: ME,
            to: &["kappa@example.net"],
            cc: &[],
            at: 100,
        }],
    );
    deliver(
        &second,
        SENT,
        &[Message {
            id: "one",
            from: ME,
            to: &["kappa.vandel@example.net"],
            cc: &[],
            at: 100,
        }],
    );
    let correspondents = Correspondents::default();

    assert_eq!(
        offered_for(&correspondents, &first, "acc-1", "kap"),
        vec![kappa(None)]
    );
    assert_eq!(
        offered_for(&correspondents, &second, "acc-2", "kap"),
        vec![("kappa.vandel@example.net".to_owned(), None)]
    );
    assert_eq!(
        offered_for(&correspondents, &first, "acc-1", "kap"),
        vec![kappa(None)]
    );
}

#[test]
fn typing_meets_a_name_and_an_address_without_their_case_or_accents() {
    let index = account("correspondents-folded");
    deliver(
        &index,
        SENT,
        &[Message {
            id: "one",
            from: ME,
            to: &["zq@example.com"],
            cc: &[],
            at: 100,
        }],
    );
    deliver(
        &index,
        INBOX,
        &[Message {
            id: "two",
            from: ("Zéta Quörin", "zq@example.com"),
            to: &["user@example.com"],
            cc: &[],
            at: 200,
        }],
    );
    let correspondents = Correspondents::default();
    let zeta = vec![("zq@example.com".to_owned(), Some("Zéta Quörin".to_owned()))];

    assert_eq!(offered_for(&correspondents, &index, "acc-1", "zeta"), zeta);
    assert_eq!(
        offered_for(&correspondents, &index, "acc-1", "QUORIN"),
        zeta
    );
    assert_eq!(offered_for(&correspondents, &index, "acc-1", "ZQ@"), zeta);
}

#[test]
fn nothing_typed_offers_nobody_and_gathers_all_the_same() {
    let index = account("correspondents-nothing-typed");
    deliver(
        &index,
        SENT,
        &[Message {
            id: "one",
            from: ME,
            to: &["kappa@example.net"],
            cc: &[],
            at: 100,
        }],
    );
    let correspondents = Correspondents::default();

    assert_eq!(
        correspondents.offered("acc-1", &index.pool, ""),
        Ok(Vec::new())
    );
    assert_eq!(
        correspondents.offered("acc-1", &index.pool, "  "),
        Ok(Vec::new())
    );
    assert_eq!(
        offered_for(&correspondents, &index, "acc-1", "kap"),
        vec![kappa(None)]
    );
}

#[test]
fn a_file_that_stopped_answering_says_so_whatever_is_typed() {
    let index = account("correspondents-unreadable");
    index.stops_answering();
    let correspondents = Correspondents::default();

    // Nothing typed reads the index too: it is how the first key comes to be answered at once.
    assert_eq!(
        correspondents.offered("acc-1", &index.pool, ""),
        Err(ReadModelError::Unavailable)
    );
    assert_eq!(
        correspondents.offered("acc-1", &index.pool, "kap"),
        Err(ReadModelError::Unavailable)
    );
}
