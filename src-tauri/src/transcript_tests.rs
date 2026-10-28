//! A pass driven by a scripted server.
//!
//! What the adapter sends is asserted byte for byte, and what it is answered includes the things a
//! live server does not do on request: renumber a mailbox between passes, refuse to open one, or
//! stop talking in the middle of a fetch. The development server proves the ordinary path; this
//! proves the ones that only happen on someone else's machine, at the worst moment.

use lyonoyl_domain::message::{FolderId, ModSeq, Uid, UidValidity};
use lyonoyl_domain::sync::{SyncCursor, VanishedReport};
use tokio_test::io::Builder;

use std::time::Duration;

use crate::infrastructure::imap::{list_on, one_pass, watch_on};

/// The generation a placement in these tests was taken under.
fn generation() -> lyonoyl_domain::message::UidValidity {
    lyonoyl_domain::message::UidValidity::new(7).expect("non-zero")
}
use crate::ports::mail_provider::{
    MailProviderError, SyncIntent, SyncRequest, Unread, WatchEnded, Watched, Watermark,
};

/// The greeting and the login, which every conversation starts with.
///
/// The tags are the client's own: `A0001` for the login, `A0002` for what follows it.
fn opening(script: Builder) -> Builder {
    let mut script = script;
    script
        .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
        .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
        .read(b"A0001 OK LOGIN completed\r\n");
    script
}

/// A session on the scripted server, logged in and ready for what the test is about.
async fn session_against(script: Builder) -> async_imap::Session<tokio_test::io::Mock> {
    let mut script = script;

    async_imap::Client::new(script.build())
        .login("person", "secret")
        .await
        .map_err(|(error, _client)| error)
        .expect("the scripted server accepts the login")
}

/// Runs a pass against the scripted server and gives back what the adapter made of it.
async fn pass_against(
    script: Builder,
    request: &SyncRequest,
) -> Result<lyonoyl_domain::sync::SyncBatch, MailProviderError> {
    one_pass(&mut session_against(script).await, request, false).await
}

/// The same, on a session whose server has been asked for `QRESYNC`.
///
/// Where production settles it: once, when the session is opened, before any mailbox is. A pass no
/// longer asks for itself, because one session now reads several mailboxes and `ENABLE` is only
/// valid before the first of them — asked again on the second folder it arrives in the wrong state
/// and the server answers `BAD`.
async fn pass_against_enabling(
    script: Builder,
    request: &SyncRequest,
) -> Result<lyonoyl_domain::sync::SyncBatch, MailProviderError> {
    let mut session = session_against(script).await;
    let qresync = crate::infrastructure::imap::pass::enable_qresync(&mut session)
        .await
        .unwrap_or(false);

    one_pass(&mut session, request, qresync).await
}

fn request() -> SyncRequest {
    SyncRequest {
        folder_id: FolderId::new(1).expect("non-zero"),
        intent: SyncIntent::Forward,
        mailbox: "INBOX".to_owned(),
        cursor: None,
        uid_validity: None,
        known_window: None,
        highest_modseq: None,
        known_count: 0,
        refused_for_now_last_time: false,
    }
}

/// What a server says when a mailbox is opened, with the numbering it claims.
///
/// The untagged lines come first and the tagged completion last, which is the order that decides
/// whether the client sees them at all.
fn examined(tag: &str, uid_validity: u32, exists: u32, uid_next: u32) -> String {
    told(tag, uid_validity, exists, uid_next, None)
}

/// The same, from a server that keeps modification sequences.
fn examined_with_modseq(
    tag: &str,
    uid_validity: u32,
    exists: u32,
    uid_next: u32,
    highest: u64,
) -> String {
    told(tag, uid_validity, exists, uid_next, Some(highest))
}

fn told(tag: &str, uid_validity: u32, exists: u32, uid_next: u32, highest: Option<u64>) -> String {
    let mut lines = vec![
        format!("* {exists} EXISTS"),
        "* 0 RECENT".to_owned(),
        "* OK [UNSEEN 1] first unseen".to_owned(),
        format!("* OK [UIDVALIDITY {uid_validity}] UIDs valid"),
        format!("* OK [UIDNEXT {uid_next}] predicted next"),
    ];
    if let Some(highest) = highest {
        lines.push(format!("* OK [HIGHESTMODSEQ {highest}] modseq"));
    }
    lines.push(format!("{tag} OK [READ-ONLY] EXAMINE completed"));
    lines.push(String::new());

    lines.join("\r\n")
}

#[tokio::test]
async fn a_mailbox_that_was_renumbered_is_read_from_its_first_message() {
    // The failure this is about happened on this project: a stored cursor addresses a numbering the
    // server has thrown away, and a pass that resumes from it reads nothing, for ever. The server
    // states a different UIDVALIDITY, and the fetch has to start at 1 — which is what the write
    // expectation below pins.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        // A cursor the numbering could plausibly hold: what decides here is the generation the
        // server reports, not a stored position that is obviously past the end.
        .read(examined("A0002", 77, 1, 2_000).as_bytes())
        // A first pass again, from the newest window of the numbering the server now has — not
        // from the stored position, which belongs to a numbering that no longer exists.
        .write(b"A0003 UID FETCH 1500:* (UID FLAGS MODSEQ RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT]<0.2048>)\r\n")
        .read(b"A0003 OK FETCH completed\r\n");

    let asked = SyncRequest {
        cursor: Some(SyncCursor::new("900".to_owned())),
        uid_validity: UidValidity::new(1).ok(),
        ..request()
    };

    let batch = pass_against(script, &asked)
        .await
        .expect("a renumbered mailbox is read, not refused");

    let delta = &batch.per_mailbox[0];
    assert_eq!(delta.uid_validity, UidValidity::new(77).expect("non-zero"));
    assert_eq!(
        delta.vanished,
        VanishedReport::Unknown,
        "a renumbered mailbox must not be audited: the old UIDs address nothing"
    );
}

#[tokio::test]
async fn a_mailbox_that_cannot_be_opened_is_reported_rather_than_guessed() {
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(b"A0002 NO [NONEXISTENT] Mailbox doesn't exist\r\n");

    let failure = pass_against(script, &request())
        .await
        .expect_err("a refused mailbox is not an empty folder");

    assert_eq!(failure, MailProviderError::Protocol);
}

#[tokio::test]
async fn a_server_that_stops_talking_mid_fetch_fails_the_pass() {
    // Not "nothing arrived": a pass that reports success here moves the cursor past messages it
    // never read, and they are then never read again.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined("A0002", 1, 2, 3).as_bytes())
        .write(b"A0003 UID FETCH 1:* (UID FLAGS MODSEQ RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT]<0.2048>)\r\n")
        // Cut in the middle of the first message: the line never ends and the server says no more.
        .read(b"* 1 FETCH (UID 1 FLAGS (\\Seen) RFC822.SIZE 4");

    let failure = pass_against(script, &request())
        .await
        .expect_err("a conversation that ends mid-message is a failure");

    // Offline rather than a defect: a connection that stopped is a network event, and the account
    // is read again in the next round. What matters is that it is not success.
    assert_eq!(failure, MailProviderError::Offline);
}

#[tokio::test]
async fn a_mailbox_with_no_uid_validity_is_refused() {
    // Without it there is nothing to tie stored UIDs to, and storing them anyway is how a later
    // pass deletes mail that never went anywhere.
    let mut script = opening(Builder::new());
    script.write(b"A0002 EXAMINE \"INBOX\"\r\n").read(
        b"* 1 EXISTS\r\n* OK [UIDNEXT 2] predicted next\r\nA0002 OK [READ-ONLY] EXAMINE completed\r\n",
    );

    let failure = pass_against(script, &request())
        .await
        .expect_err("a mailbox with no numbering is refused");

    assert_eq!(failure, MailProviderError::Protocol);
}

#[tokio::test]
async fn what_changed_is_asked_for_by_sequence_when_the_server_keeps_them() {
    // The CONDSTORE question, pinned as bytes: `CHANGEDSINCE` is what makes a flag changed on an
    // old message arrive at all, and the position asked from is what has been applied, not what the
    // server last reported.
    let mut script = opening(Builder::new());
    script
        // Asked for, and refused by silence: this server names nothing as enabled, so the fetch
        // below carries no `VANISHED` — a server that has not enabled QRESYNC answers a fetch
        // that carries it with `BAD`, and the pass would fail rather than degrade.
        .write(b"A0002 ENABLE QRESYNC\r\n")
        .read(b"A0002 OK ENABLE completed\r\n")
        .write(b"A0003 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0003", 1, 3, 4, 90).as_bytes())
        // No fetch for arrivals, and the script is the assertion: this device holds up to 3 and
        // the server says the next message will be 4, so the folder ends where this device does.
        // Asked anyway, `UID FETCH 4:*` is not empty — the range reads as "between 4 and the
        // highest" and the server answers with the last message every time.
        .write(b"A0004 UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 42)\r\n")
        .read(b"A0004 OK FETCH completed\r\n");

    let asked = SyncRequest {
        cursor: Some(SyncCursor::new("3".to_owned())),
        uid_validity: UidValidity::new(1).ok(),
        known_window: Some(Uid::new(1).expect("non-zero")..=Uid::new(3).expect("non-zero")),
        highest_modseq: Some(ModSeq::new(42)),
        known_count: 3,
        ..request()
    };

    pass_against_enabling(script, &asked)
        .await
        .expect("the pass completes");
}

#[tokio::test]
async fn a_mailbox_with_nothing_new_in_it_is_not_asked_for_anything() {
    // Pinned as bytes, because this is what a quiet round costs. The mailbox holds three messages,
    // this device holds all three, and the next number the server will give is four — so there is
    // nothing to ask and the script has no fetch in it at all. The fixture fails on an unscripted
    // write, so a pass that asked anyway would be caught here.
    //
    // Asked anyway it is not a wasted round trip but a wasted message: `UID FETCH 4:*` reads as
    // "between four and the highest", and a server answers with the last message in the folder —
    // its envelope and two kilobytes of its body — every pass, for every mailbox, for ever.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0002", 1, 3, 4, 42).as_bytes());

    let asked = SyncRequest {
        cursor: Some(SyncCursor::new("3".to_owned())),
        uid_validity: UidValidity::new(1).ok(),
        known_window: Some(Uid::new(1).expect("non-zero")..=Uid::new(3).expect("non-zero")),
        // The same sequence the server reports, so nothing changed either.
        highest_modseq: Some(ModSeq::new(42)),
        known_count: 3,
        ..request()
    };

    pass_against(script, &asked)
        .await
        .expect("the pass completes");
}

#[tokio::test]
async fn a_server_that_names_what_went_is_believed_instead_of_being_asked_to_list_a_mailbox() {
    // The point of QRESYNC, pinned as bytes. The mailbox holds three messages and the folder
    // accounts for eight, so the pass has every reason to list the whole of it — and does not,
    // because the server already said which ones went. The script is the assertion: a listing this
    // pass did not need would be an unscripted write, and the fixture fails on one.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 ENABLE QRESYNC\r\n")
        .read(b"* ENABLED QRESYNC\r\nA0002 OK ENABLE completed\r\n")
        .write(b"A0003 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0003", 1, 3, 10, 90).as_bytes())
        .write(b"A0004 UID FETCH 4:9 (UID FLAGS MODSEQ RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT]<0.2048>)\r\n")
        .read(b"A0004 OK FETCH completed\r\n")
        .write(b"A0005 UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 42 VANISHED)\r\n")
        .read(b"* VANISHED (EARLIER) 2,5:7\r\nA0005 OK FETCH completed\r\n");

    let asked = SyncRequest {
        cursor: Some(SyncCursor::new("3".to_owned())),
        uid_validity: UidValidity::new(1).ok(),
        known_window: Some(Uid::new(1).expect("non-zero")..=Uid::new(9).expect("non-zero")),
        highest_modseq: Some(ModSeq::new(42)),
        known_count: 8,
        ..request()
    };

    let batch = pass_against_enabling(script, &asked)
        .await
        .expect("the pass completes");

    assert_eq!(
        batch.per_mailbox[0].vanished,
        VanishedReport::Exact(
            [2, 5, 6, 7]
                .map(|value| Uid::new(value).expect("non-zero"))
                .to_vec()
        ),
        "what the server named is what the pass reports"
    );
}

#[tokio::test]
async fn a_server_that_names_nothing_is_still_audited_by_counting() {
    // The other half of the rule. The extension was enabled and the question was asked, and the
    // answer named nothing — which does not explain a mailbox holding three where the folder
    // accounts for eight. Something removed before the position this pass asked from would never
    // be named, so the listing has to happen anyway.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 ENABLE QRESYNC\r\n")
        .read(b"* ENABLED QRESYNC\r\nA0002 OK ENABLE completed\r\n")
        .write(b"A0003 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0003", 1, 3, 10, 90).as_bytes())
        .write(b"A0004 UID FETCH 4:9 (UID FLAGS MODSEQ RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT]<0.2048>)\r\n")
        .read(b"A0004 OK FETCH completed\r\n")
        .write(b"A0005 UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 42 VANISHED)\r\n")
        .read(b"A0005 OK FETCH completed\r\n")
        .write(b"A0006 UID FETCH 1:9 (UID FLAGS MODSEQ)\r\n")
        .read(b"* 1 FETCH (UID 1 FLAGS ())\r\nA0006 OK FETCH completed\r\n");

    let asked = SyncRequest {
        cursor: Some(SyncCursor::new("3".to_owned())),
        uid_validity: UidValidity::new(1).ok(),
        known_window: Some(Uid::new(1).expect("non-zero")..=Uid::new(9).expect("non-zero")),
        highest_modseq: Some(ModSeq::new(42)),
        known_count: 8,
        ..request()
    };

    let batch = pass_against_enabling(script, &asked)
        .await
        .expect("the pass completes");

    assert_eq!(
        batch.per_mailbox[0].vanished,
        VanishedReport::AuthoritativeSet {
            window: Uid::new(1).expect("non-zero")..=Uid::new(9).expect("non-zero"),
            present: vec![Uid::new(1).expect("non-zero")],
        },
        "a pass that was told nothing falls back to what it can see"
    );
}

/// A folder holding UIDs 1 to 9 of which the server now has three, so the pass lists the whole
/// of it — the listing is what may remove mail.
fn missing_some() -> SyncRequest {
    SyncRequest {
        cursor: Some(SyncCursor::new("3".to_owned())),
        uid_validity: UidValidity::new(1).ok(),
        known_window: Some(Uid::new(1).expect("non-zero")..=Uid::new(9).expect("non-zero")),
        known_count: 8,
        ..request()
    }
}

/// The conversation up to the listing of a folder that is [`missing_some`], with nothing new in it.
fn up_to_the_listing() -> Builder {
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined("A0002", 1, 3, 10).as_bytes())
        .write(b"A0003 UID FETCH 4:9 (UID FLAGS MODSEQ RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT]<0.2048>)\r\n")
        .read(b"A0003 OK FETCH completed\r\n")
        .write(b"A0004 UID FETCH 1:9 (UID FLAGS MODSEQ)\r\n");
    script
}

#[tokio::test]
async fn a_listing_the_server_refused_is_not_read_as_an_empty_mailbox() {
    // A refusal before a single line: "the mailbox holds three" and "the listing named none"
    // arrive together, and read as a listing that is every message in the window gone at once.
    let mut script = up_to_the_listing();
    script.read(b"A0004 BAD invalid arguments\r\n");

    let failure = pass_against(script, &missing_some())
        .await
        .expect_err("a refused listing is no listing at all");

    assert_eq!(failure, MailProviderError::Protocol);
}

#[tokio::test]
async fn a_listing_that_came_back_empty_is_not_read_as_an_empty_mailbox() {
    // Completed with `OK` and naming nothing, from a mailbox that has just said it holds three. A
    // server cannot have answered that honestly, and believing it would propose removing the
    // folder.
    let mut script = up_to_the_listing();
    script.read(b"A0004 OK FETCH completed\r\n");

    let batch = pass_against(script, &missing_some())
        .await
        .expect("the pass completes");

    assert_eq!(
        batch.per_mailbox[0].vanished,
        VanishedReport::Unknown,
        "an answer that named nothing must claim nothing"
    );
}

#[tokio::test]
async fn a_listing_refused_halfway_removes_nothing() {
    // The client library's own fetch ends at the command's tagged line whatever the line says, so
    // the first line and a refusal read as a listing of one message — and every other message in
    // the window would be proposed for removal. A listing is whole or it is not a listing.
    let mut script = up_to_the_listing();
    script.read(b"* 1 FETCH (UID 1 FLAGS (\\Seen))\r\nA0004 NO [SERVERBUG] Internal error\r\n");

    let failure = pass_against(script, &missing_some())
        .await
        .expect_err("half a listing is not a listing");

    assert_eq!(failure, MailProviderError::Protocol);
}

#[tokio::test]
async fn a_listing_cut_off_by_the_server_leaving_removes_nothing() {
    // `BYE` and then the end of the connection: the client library hands the first to a queue and
    // takes the second for the end of the answer, so through it the line before them reads as the
    // whole listing.
    let mut script = up_to_the_listing();
    script.read(b"* 1 FETCH (UID 1 FLAGS (\\Seen))\r\n* BYE Server shutting down\r\n");

    let failure = pass_against(script, &missing_some())
        .await
        .expect_err("a listing the server left in the middle of is not a listing");

    assert_eq!(failure, MailProviderError::Offline);
}

#[tokio::test]
async fn a_listing_cut_off_by_the_connection_ending_removes_nothing() {
    // The same without a word from the server: whole lines, and then nothing at all. Every line
    // ended cleanly, so nothing failed to parse — the only sign is the tagged line that never came.
    let mut script = up_to_the_listing();
    script.read(b"* 1 FETCH (UID 1 FLAGS (\\Seen))\r\n");

    let failure = pass_against(script, &missing_some())
        .await
        .expect_err("a listing with no end is not a listing");

    assert_eq!(failure, MailProviderError::Offline);
}

#[tokio::test]
async fn new_mail_refused_halfway_does_not_move_the_cursor_past_what_never_came() {
    // Five and eight arrive, six and seven do not, and the server ends with a refusal — and then
    // refuses to say what it holds in that range as well. Taken as an answer, the cursor moves to
    // the end of the window and six and seven are behind it for ever.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined("A0002", 1, 7, 10).as_bytes())
        .write(b"A0003 UID FETCH 4:9 (UID FLAGS MODSEQ RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT]<0.2048>)\r\n")
        .read(
            b"* 4 FETCH (UID 5 FLAGS () RFC822.SIZE 20)\r\n\
              * 5 FETCH (UID 8 FLAGS () RFC822.SIZE 20)\r\n\
              A0003 NO [SERVERBUG] Internal error occurred\r\n",
        )
        .write(format!("A0004 UID FETCH 4:9 {FACTS_QUERY}\r\n").as_bytes())
        .read(b"* 4 FETCH (UID 5 FLAGS ())\r\nA0004 NO [SERVERBUG] Internal error occurred\r\n");

    let asked = SyncRequest {
        cursor: Some(SyncCursor::new("3".to_owned())),
        uid_validity: UidValidity::new(1).ok(),
        known_window: Some(Uid::new(1).expect("non-zero")..=Uid::new(3).expect("non-zero")),
        known_count: 3,
        ..request()
    };

    let failure = pass_against(script, &asked)
        .await
        .expect_err("an answer cut short moves nothing");

    assert_eq!(failure, MailProviderError::Protocol);
}

#[tokio::test]
async fn a_range_read_further_back_and_refused_halfway_is_not_recorded_as_read() {
    // A round further back that succeeds records its range as read, down to its lowest number,
    // and is never asked for again. One message and a refusal is not that range read.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined("A0002", 1, 900, 1_000).as_bytes())
        .write(b"A0003 UID FETCH 1:500 (UID FLAGS MODSEQ RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT]<0.2048>)\r\n")
        .read(b"* 1 FETCH (UID 3 FLAGS () RFC822.SIZE 20)\r\nA0003 NO [SERVERBUG] Internal error\r\n")
        // What the server holds in the range, which would let the range be read past what it
        // refused — and which it will not say either.
        .write(format!("A0004 UID FETCH 1:500 {FACTS_QUERY}\r\n").as_bytes())
        .read(b"A0004 NO [SERVERBUG] Internal error\r\n");

    let asked = SyncRequest {
        intent: SyncIntent::Older(
            Uid::new(1).expect("non-zero")..=Uid::new(500).expect("non-zero"),
        ),
        cursor: Some(SyncCursor::new("999".to_owned())),
        uid_validity: UidValidity::new(1).ok(),
        known_window: Some(Uid::new(501).expect("non-zero")..=Uid::new(999).expect("non-zero")),
        known_count: 499,
        ..request()
    };

    let failure = pass_against(script, &asked)
        .await
        .expect_err("a range read halfway is not a range read");

    assert_eq!(failure, MailProviderError::Protocol);
}

#[tokio::test]
async fn a_line_that_says_nothing_about_flags_does_not_clear_them() {
    // Asked what changed, the server names 2 with its sequence and nothing else — a change that is
    // not to its flags. Read as a message with no flags, it is a message marked unread, unflagged
    // and unanswered on this device that nobody touched anywhere.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0002", 1, 3, 4, 90).as_bytes())
        .write(b"A0003 UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 42)\r\n")
        .read(
            b"* 1 FETCH (UID 1 MODSEQ (80) FLAGS (\\Seen))\r\n\
              * 2 FETCH (UID 2 MODSEQ (85))\r\n\
              A0003 OK FETCH completed\r\n",
        );

    let batch = pass_against(script, &holding("3", 3, 3))
        .await
        .expect("the pass completes");

    let updated: Vec<u32> = batch.per_mailbox[0]
        .flag_updates
        .iter()
        .map(|update| update.uid.get())
        .collect();
    assert_eq!(updated, vec![1], "2 said nothing about its flags");
}

#[tokio::test]
async fn what_changed_refused_halfway_moves_nothing() {
    // Each line of this answer writes its sequence to the message it is about, and the next pass
    // asks what changed since the highest of them. Half an answer taken as whole moves that
    // position past the changes that never arrived, and they are never asked for again.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0002", 1, 3, 4, 90).as_bytes())
        .write(b"A0003 UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 42)\r\n")
        .read(
            b"* 1 FETCH (UID 2 MODSEQ (80) FLAGS (\\Seen))\r\n\
              A0003 NO [SERVERBUG] Internal error\r\n",
        );

    let asked = SyncRequest {
        cursor: Some(SyncCursor::new("3".to_owned())),
        uid_validity: UidValidity::new(1).ok(),
        known_window: Some(Uid::new(1).expect("non-zero")..=Uid::new(3).expect("non-zero")),
        highest_modseq: Some(ModSeq::new(42)),
        known_count: 3,
        ..request()
    };

    let failure = pass_against(script, &asked)
        .await
        .expect_err("half of what changed is not what changed");

    assert_eq!(failure, MailProviderError::Protocol);
}

/// What a pass asks for the mail it has not read.
const PASS_QUERY: &str =
    "(UID FLAGS MODSEQ RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT]<0.2048>)";

/// What a pass asks about a range whose mail the server refused to describe.
const FACTS_QUERY: &str = "(UID FLAGS RFC822.SIZE INTERNALDATE)";

/// A folder this device read up to `cursor`, holding `count` messages up to `highest`, on a server
/// that keeps modification sequences and whose sequence has not moved — so the mail past the
/// cursor is the only question a pass asks.
fn holding(cursor: &str, highest: u32, count: usize) -> SyncRequest {
    SyncRequest {
        cursor: Some(SyncCursor::new(cursor.to_owned())),
        uid_validity: UidValidity::new(1).ok(),
        known_window: Some(Uid::new(1).expect("non-zero")..=Uid::new(highest).expect("non-zero")),
        highest_modseq: Some(ModSeq::new(42)),
        known_count: count,
        ..request()
    }
}

/// One line for each of `uids`, saying `rest` after the number, and the tagged line that ends it.
fn lines_naming(uids: &[u32], rest: &str, tag: &str) -> String {
    let mut answer: String = uids
        .iter()
        .enumerate()
        .map(|(at, uid)| format!("* {} FETCH (UID {uid}{rest})\r\n", at + 1))
        .collect();
    answer.push_str(&format!("{tag} OK FETCH completed\r\n"));
    answer
}

/// The UIDs of what a pass brought in.
fn arrived_in(batch: &lyonoyl_domain::sync::SyncBatch) -> Vec<u32> {
    batch.per_mailbox[0]
        .arrived
        .iter()
        .map(|message| message.uid.get())
        .collect()
}

/// A folder read up to 3 whose server has issued up to 8999 since: 10, 1500 and 5000 to 6998
/// are mail, and every other number went to mail that has since left. `before_the_end` is said in
/// the middle of the answer to the fetch, just before it ends.
fn two_thousand_and_one_waiting(before_the_end: &str) -> Builder {
    let mut waiting = vec![10, 1500];
    waiting.extend(5_000..=6_998);

    let mut answer = lines_naming(&waiting[..2_000], " FLAGS () RFC822.SIZE 20", "A0004");
    answer.insert_str(
        answer.len() - "A0004 OK FETCH completed\r\n".len(),
        before_the_end,
    );

    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0002", 1, 6_000, 9_000, 42).as_bytes())
        .write(b"A0003 UID FETCH 4:8999 (UID)\r\n")
        .read(lines_naming(&waiting, "", "A0003").as_bytes())
        .write(format!("A0004 UID FETCH 4:6997 {PASS_QUERY}\r\n").as_bytes())
        .read(answer.as_bytes());
    script
}

#[tokio::test]
async fn new_mail_past_what_one_answer_holds_arrives_a_window_of_messages_at_a_time() {
    // Asked for all at once, a folder that gained more mail since its last pass than one answer
    // may hold is refused on every pass and never read again. Asked for a window at a time, it
    // arrives over several passes, each taking up where the one before it stopped — and a window
    // answered whole moves the position to its end, whatever numbers in it went unused. The
    // window is counted in messages: which numbers are mail is asked first, by number alone.
    let mut script = two_thousand_and_one_waiting("");
    script
        .write(b"A0005 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0005", 1, 6_000, 9_000, 42).as_bytes())
        .write(b"A0006 UID FETCH 6998:8999 (UID)\r\n")
        .read(lines_naming(&[6_998], "", "A0006").as_bytes())
        .write(format!("A0007 UID FETCH 6998:8999 {PASS_QUERY}\r\n").as_bytes())
        .read(lines_naming(&[6_998], " FLAGS () RFC822.SIZE 20", "A0007").as_bytes());

    let mut session = session_against(script).await;

    let first = one_pass(&mut session, &holding("3", 3, 3), false)
        .await
        .expect("the first window is read");
    let mut window = vec![10, 1500];
    window.extend(5_000..=6_997);
    assert_eq!(arrived_in(&first), window);
    assert_eq!(first.cursor_after, SyncCursor::new("6997".to_owned()));
    assert!(
        first.per_mailbox[0].stopped_short,
        "mail past the window is still to come"
    );

    let second = one_pass(&mut session, &holding("6997", 6_997, 2_003), false)
        .await
        .expect("the next window is read");
    assert_eq!(arrived_in(&second), vec![6_998]);
    assert_eq!(second.cursor_after, SyncCursor::new("8999".to_owned()));
    assert!(
        !second.per_mailbox[0].stopped_short,
        "read to the newest number"
    );
}

#[tokio::test]
async fn numbers_that_name_no_mail_cost_no_pass_of_their_own() {
    // A server that numbers every folder of an account from one counter, or a junk folder it
    // empties, leaves a quiet folder fifty thousand numbers behind its next one. Read a window of
    // numbers at a time, the one message past them arrives on the twenty-fourth pass, most of an
    // hour later; asked which numbers are mail, it arrives on this one.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0002", 1, 4, 50_000, 42).as_bytes())
        .write(b"A0003 UID FETCH 4:49999 (UID)\r\n")
        .read(lines_naming(&[48_000], "", "A0003").as_bytes())
        .write(format!("A0004 UID FETCH 4:49999 {PASS_QUERY}\r\n").as_bytes())
        .read(lines_naming(&[48_000], " FLAGS () RFC822.SIZE 20", "A0004").as_bytes());

    let batch = pass_against(script, &holding("3", 3, 3))
        .await
        .expect("the pass completes");

    assert_eq!(arrived_in(&batch), vec![48_000]);
    assert_eq!(batch.cursor_after, SyncCursor::new("49999".to_owned()));
    assert!(!batch.per_mailbox[0].stopped_short);
}

#[tokio::test]
async fn a_line_about_a_message_past_the_window_is_not_mail_arriving() {
    // A phone marks the newest message read while the window below it is being fetched, and a
    // server with QRESYNC on says so in the middle of the answer, naming the message by its UID.
    // Taken for an answer, 8000 is a row with nothing in it, and the cursor moves to 8000 — past
    // every number from the window's end to it, none of which is ever asked for again.
    let script =
        two_thousand_and_one_waiting("* 5999 FETCH (UID 8000 MODSEQ (50) FLAGS (\\Seen))\r\n");

    let batch = pass_against(script, &holding("3", 3, 3))
        .await
        .expect("the pass completes");

    assert!(
        !arrived_in(&batch).contains(&8_000),
        "a message past the window arrived"
    );
    assert_eq!(arrived_in(&batch).len(), 2_000);
    assert_eq!(batch.cursor_after, SyncCursor::new("6997".to_owned()));
}

#[tokio::test]
async fn a_line_about_a_message_outside_a_range_read_further_back_is_not_mail_arriving() {
    // 900 is held already, above the range being read. A flag changed on it while the range is
    // fetched arrives as a line about it, and taken for an answer it is a blank row that takes
    // its place in the folder — the message it replaces then belongs to no folder at all.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0002", 1, 900, 1_000, 42).as_bytes())
        .write(format!("A0003 UID FETCH 1:500 {PASS_QUERY}\r\n").as_bytes())
        .read(
            b"* 1 FETCH (UID 3 FLAGS () RFC822.SIZE 20)\r\n\
              * 600 FETCH (UID 900 MODSEQ (50) FLAGS (\\Seen))\r\n\
              A0003 OK FETCH completed\r\n",
        );

    let asked = SyncRequest {
        intent: SyncIntent::Older(
            Uid::new(1).expect("non-zero")..=Uid::new(500).expect("non-zero"),
        ),
        cursor: Some(SyncCursor::new("999".to_owned())),
        uid_validity: UidValidity::new(1).ok(),
        known_window: Some(Uid::new(501).expect("non-zero")..=Uid::new(999).expect("non-zero")),
        highest_modseq: Some(ModSeq::new(42)),
        known_count: 499,
        ..request()
    };

    let batch = pass_against(script, &asked)
        .await
        .expect("the range is read");

    assert_eq!(arrived_in(&batch), vec![3]);
}

#[tokio::test]
async fn mail_numbered_after_a_first_pass_opened_the_mailbox_is_left_for_the_next_pass() {
    // 10 and 11 arrive while the fetch runs, and a phone marks 11 read at once. The line saying so
    // names 11 by its UID; taken for an answer, the first pass stores a row with nothing in it and
    // moves the cursor to 11, and 10 is never asked for.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined("A0002", 1, 2, 10).as_bytes())
        .write(format!("A0003 UID FETCH 1:* {PASS_QUERY}\r\n").as_bytes())
        .read(
            b"* 1 FETCH (UID 5 FLAGS () RFC822.SIZE 20)\r\n\
              * 2 FETCH (UID 8 FLAGS () RFC822.SIZE 20)\r\n\
              * 4 EXISTS\r\n\
              * 4 FETCH (UID 11 MODSEQ (50) FLAGS (\\Seen))\r\n\
              A0003 OK FETCH completed\r\n",
        );

    let batch = pass_against(script, &request())
        .await
        .expect("the first pass completes");

    assert_eq!(arrived_in(&batch), vec![5, 8]);
    // The end of what it asked about: the newest number when the mailbox was opened. Ten and
    // eleven are past it, and the next pass asks for them.
    assert_eq!(batch.cursor_after, SyncCursor::new("9".to_owned()));
}

#[tokio::test]
async fn a_line_about_a_message_below_a_first_pass_floor_is_not_mail_arriving() {
    // A first pass reads the newest five hundred numbers, and a phone marks an old message read
    // while it does. The line saying so names 3 by its UID; taken for an answer, it is a row with
    // nothing in it, put in the place of a message the reading further back would have stored
    // whole.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined("A0002", 1, 2, 1_000).as_bytes())
        .write(format!("A0003 UID FETCH 500:* {PASS_QUERY}\r\n").as_bytes())
        .read(
            format!(
                "* 1 FETCH (UID 3 FLAGS (\\Seen))\r\n{}A0003 OK FETCH completed\r\n",
                arriving(2, 600, "Six hundred", "")
            )
            .as_bytes(),
        );

    let batch = pass_against(script, &request())
        .await
        .expect("the first pass completes");

    assert_eq!(arrived_in(&batch), vec![600]);
    assert_eq!(
        batch.per_mailbox[0].covered_from,
        Uid::new(500).ok(),
        "what is below the floor is read further back"
    );
    assert_eq!(batch.cursor_after, SyncCursor::new("999".to_owned()));
}

#[tokio::test]
async fn a_folder_whose_newest_message_is_below_a_first_pass_floor_shows_it_on_that_pass() {
    // An inbox kept empty, or a folder of drafts a phone saves over and over: sixty thousand
    // numbers spent, and three messages left far below the newest five hundred. `59500:*` names the
    // newest of them, 30, and says every number from it to the floor holds nothing. Read from the
    // floor, the folder shows nothing until the reading further back has walked down to 30 five
    // hundred numbers a pass — four hours. Read from 30, it arrives now, and the reading further
    // back takes up below it.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined("A0002", 1, 3, 60_000).as_bytes())
        .write(format!("A0003 UID FETCH 59500:* {PASS_QUERY}\r\n").as_bytes())
        .read(
            format!(
                "{}A0003 OK FETCH completed\r\n",
                arriving(3, 30, "Thirty", "")
            )
            .as_bytes(),
        );

    let batch = pass_against(script, &request())
        .await
        .expect("the first pass completes");

    assert_eq!(arrived_in(&batch), vec![30]);
    assert_eq!(
        batch.per_mailbox[0].arrived[0].envelope.subject.as_deref(),
        Some("Thirty")
    );
    assert_eq!(
        batch.per_mailbox[0].covered_from,
        Uid::new(30).ok(),
        "the reading further back starts below the newest message, not below the floor"
    );
    // Nothing between 30 and the newest number there was is mail, and the next pass asks from
    // past it.
    assert_eq!(batch.cursor_after, SyncCursor::new("59999".to_owned()));
}

#[tokio::test]
async fn a_folder_whose_newest_message_below_a_first_pass_floor_is_refused_shows_it_on_that_pass() {
    // The same folder, and the server will not describe 30. Its facts are what say it is there,
    // and it is kept as a row of them like any other message the server will not describe: read
    // from the floor instead, the folder shows nothing for four hours, and a refused message never
    // arrives before the reading further back walks down to it.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined("A0002", 1, 3, 60_000).as_bytes())
        .write(format!("A0003 UID FETCH 59500:* {PASS_QUERY}\r\n").as_bytes())
        .read(b"A0003 NO [SERVERBUG] Internal error occurred\r\n")
        .write(format!("A0004 UID FETCH 59500:* {FACTS_QUERY}\r\n").as_bytes())
        .read(
            b"* 3 FETCH (UID 30 FLAGS () RFC822.SIZE 4096 INTERNALDATE \"17-Jul-1996 02:44:25 -0700\")\r\n\
              A0004 OK FETCH completed\r\n",
        );

    let batch = pass_against(script, &request())
        .await
        .expect("the first pass completes");

    assert_eq!(arrived_in(&batch), vec![30]);
    let refused = &batch.per_mailbox[0].arrived[0].envelope;
    assert_eq!(refused.subject, None, "nothing was read from it");
    assert_eq!(refused.rfc822_size, Some(4096));
    assert_eq!(
        batch.per_mailbox[0].covered_from,
        Uid::new(30).ok(),
        "the reading further back starts below the newest message, not below the floor"
    );
    assert_eq!(batch.cursor_after, SyncCursor::new("59999".to_owned()));
}

/// The line a server answers a pass's question about one message with: its flags, its size, its
/// date, its header with `subject` in it, and the opening of its text.
fn arriving(at: u32, uid: u32, subject: &str, flags: &str) -> String {
    let header = format!(
        "Subject: {subject}\r\nFrom: zeta@example.com\r\nMessage-ID: <{uid}@example.com>\r\n\r\n"
    );
    format!(
        "* {at} FETCH (UID {uid} FLAGS ({flags}) RFC822.SIZE 120 INTERNALDATE \"17-Jul-1996 02:44:25 -0700\" \
         {} {} {} {})\r\n",
        labelled("HEADER]"),
        literal(&header),
        labelled("TEXT]<0>"),
        literal("See you then."),
    )
}

#[tokio::test]
async fn a_second_line_about_a_message_while_it_arrives_is_the_same_message() {
    // The message is fetched, and a phone marks it read while the answer is being sent: a server
    // with QRESYNC on says so in the middle of the answer, naming it by its UID. Taken for a second
    // arrival, it is a row with no subject and no sender that takes the message's place in the
    // folder, and the message itself is forgotten at the end of the pass.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0002", 1, 4, 5, 42).as_bytes())
        .write(format!("A0003 UID FETCH 4:4 {PASS_QUERY}\r\n").as_bytes())
        .read(
            format!(
                "{}* 1 FETCH (UID 4 MODSEQ (50) FLAGS (\\Seen))\r\nA0003 OK FETCH completed\r\n",
                arriving(1, 4, "Tuesday", "")
            )
            .as_bytes(),
        );

    let batch = pass_against(script, &holding("3", 3, 3))
        .await
        .expect("the pass completes");

    let arrived = &batch.per_mailbox[0].arrived;
    assert_eq!(arrived.len(), 1, "one message, said about twice");
    assert_eq!(arrived[0].uid, Uid::new(4).expect("non-zero"));
    assert_eq!(arrived[0].envelope.subject.as_deref(), Some("Tuesday"));
    assert!(
        arrived[0].envelope.rfc822_message_id.is_some(),
        "the message keeps the name it is found by"
    );
    assert!(
        arrived[0]
            .envelope
            .flags
            .contains(lyonoyl_domain::message::Flags::SEEN),
        "the later line is the later moment"
    );
    assert_eq!(arrived[0].modseq, Some(ModSeq::new(50)));
}

#[tokio::test]
async fn a_message_the_server_refuses_on_every_pass_does_not_stop_the_mail_after_it() {
    // The server will not describe 5, now or ever: it answers with 4 and 6 and refuses. Asked for
    // the same range again on every pass, the folder stops at 5 for good — no new mail, no flag,
    // no removal, nothing further back. Asked what it holds without reading it, the server names
    // 5, and 5 is a row of what it said: listed, counted, opened like any other, filled in once
    // its bytes are read. The next pass reads past it.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0002", 1, 6, 7, 42).as_bytes())
        .write(format!("A0003 UID FETCH 4:6 {PASS_QUERY}\r\n").as_bytes())
        // A phone marks 5 read while the answer is sent, and the server says so: a line about 5
        // that describes nothing of it, which is not 5 arriving.
        .read(
            format!(
                "{}* 2 FETCH (UID 5 MODSEQ (50) FLAGS (\\Seen))\r\n{}\
                 A0003 NO [SERVERBUG] Internal error occurred\r\n",
                arriving(1, 4, "Four", ""),
                arriving(3, 6, "Six", "")
            )
            .as_bytes(),
        )
        .write(format!("A0004 UID FETCH 4:6 {FACTS_QUERY}\r\n").as_bytes())
        .read(
            b"* 1 FETCH (UID 4 FLAGS () RFC822.SIZE 120 INTERNALDATE \"17-Jul-1996 02:44:25 -0700\")\r\n\
              * 2 FETCH (UID 5 FLAGS (\\Seen) RFC822.SIZE 4096 INTERNALDATE \"17-Jul-1996 02:44:25 -0700\")\r\n\
              * 3 FETCH (UID 6 FLAGS () RFC822.SIZE 120 INTERNALDATE \"17-Jul-1996 02:44:25 -0700\")\r\n\
              A0004 OK FETCH completed\r\n",
        )
        .write(b"A0005 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0005", 1, 8, 9, 42).as_bytes())
        .write(format!("A0006 UID FETCH 7:8 {PASS_QUERY}\r\n").as_bytes())
        .read(
            format!(
                "{}{}A0006 OK FETCH completed\r\n",
                arriving(4, 7, "Seven", ""),
                arriving(5, 8, "Eight", "")
            )
            .as_bytes(),
        );

    let mut session = session_against(script).await;

    let first = one_pass(&mut session, &holding("3", 3, 3), false)
        .await
        .expect("a message the server will not describe does not fail the pass");
    assert_eq!(arrived_in(&first), vec![4, 5, 6]);
    assert_eq!(first.cursor_after, SyncCursor::new("6".to_owned()));

    let arrived = &first.per_mailbox[0].arrived;
    assert_eq!(arrived[0].envelope.subject.as_deref(), Some("Four"));
    assert_eq!(arrived[2].envelope.subject.as_deref(), Some("Six"));

    let refused = &arrived[1].envelope;
    assert_eq!(refused.subject, None, "nothing was read from it");
    assert_eq!(refused.sender, None);
    // 17 July 1996, 09:44:25 in Greenwich, and the rest as the server said it.
    assert_eq!(refused.received_at, 837_596_665);
    assert_eq!(refused.rfc822_size, Some(4096));
    assert!(refused.flags.contains(lyonoyl_domain::message::Flags::SEEN));

    let second = one_pass(&mut session, &holding("6", 6, 6), false)
        .await
        .expect("the mail after it arrives");
    assert_eq!(arrived_in(&second), vec![7, 8]);
    assert_eq!(second.cursor_after, SyncCursor::new("8".to_owned()));
}

#[tokio::test]
async fn a_range_read_further_back_keeps_a_message_the_server_will_not_describe() {
    // The same refusal, a round further back: the range is read past it, and the round after this
    // one starts below it rather than asking for the same five hundred numbers for ever.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0002", 1, 900, 1_000, 42).as_bytes())
        .write(format!("A0003 UID FETCH 1:500 {PASS_QUERY}\r\n").as_bytes())
        .read(
            format!(
                "{}A0003 NO [SERVERBUG] Internal error occurred\r\n",
                arriving(1, 3, "Three", "")
            )
            .as_bytes(),
        )
        .write(format!("A0004 UID FETCH 1:500 {FACTS_QUERY}\r\n").as_bytes())
        .read(
            b"* 1 FETCH (UID 3 FLAGS () RFC822.SIZE 120 INTERNALDATE \"17-Jul-1996 02:44:25 -0700\")\r\n\
              * 2 FETCH (UID 4 FLAGS () RFC822.SIZE 4096 INTERNALDATE \"17-Jul-1996 02:44:25 -0700\")\r\n\
              * 600 FETCH (UID 900 FLAGS (\\Seen))\r\n\
              A0004 OK FETCH completed\r\n",
        );

    let asked = SyncRequest {
        intent: SyncIntent::Older(
            Uid::new(1).expect("non-zero")..=Uid::new(500).expect("non-zero"),
        ),
        cursor: Some(SyncCursor::new("999".to_owned())),
        uid_validity: UidValidity::new(1).ok(),
        known_window: Some(Uid::new(501).expect("non-zero")..=Uid::new(999).expect("non-zero")),
        highest_modseq: Some(ModSeq::new(42)),
        known_count: 499,
        ..request()
    };

    let batch = pass_against(script, &asked)
        .await
        .expect("the range is read past what the server would not describe");

    assert_eq!(
        arrived_in(&batch),
        vec![3, 4],
        "and 900 is not in the range"
    );
    let arrived = &batch.per_mailbox[0].arrived;
    assert_eq!(arrived[0].envelope.subject.as_deref(), Some("Three"));
    assert_eq!(arrived[1].envelope.subject, None);
    assert_eq!(arrived[1].envelope.rfc822_size, Some(4096));
    assert_eq!(
        batch.cursor_after,
        SyncCursor::new("999".to_owned()),
        "reading further back moves no cursor"
    );
}

#[tokio::test]
async fn a_refusal_that_is_not_a_refusal_of_messages_still_moves_nothing() {
    // `BAD` refuses the question rather than a message, and a server leaving says nothing about
    // which messages it would have given. Neither is an answer to list past: the script holds no
    // second question, and a pass that asked one would write past its end.
    for (ending, expected) in [
        (
            "A0003 BAD Command syntax error\r\n",
            MailProviderError::Protocol,
        ),
        ("* BYE Server shutting down\r\n", MailProviderError::Offline),
    ] {
        let mut script = opening(Builder::new());
        script
            .write(b"A0002 EXAMINE \"INBOX\"\r\n")
            .read(examined_with_modseq("A0002", 1, 6, 7, 42).as_bytes())
            .write(format!("A0003 UID FETCH 4:6 {PASS_QUERY}\r\n").as_bytes())
            .read(format!("{}{ending}", arriving(1, 4, "Four", "")).as_bytes());

        let failure = pass_against(script, &holding("3", 3, 3))
            .await
            .expect_err("an answer cut short moves nothing");

        assert_eq!(failure, expected, "{ending:?}");
    }
}

#[tokio::test]
async fn a_refusal_for_a_moment_is_asked_again_next_pass_rather_than_kept_as_rows() {
    // A part of the server down, the mailbox in use, a message being removed by another session:
    // asked again in two minutes, the same range is described whole. Kept as rows of what the
    // server knows, 5 and 6 would be mail with no name to be found by — a message filed here from
    // another folder a moment ago would be a stranger beside its own row, which then loses its last
    // copy and is forgotten. The script holds no second question, and a pass that listed the range
    // would write past its end.
    for ending in [
        "A0003 NO [UNAVAILABLE] Try again later\r\n",
        "A0003 NO [INUSE] Mailbox is locked\r\n",
        "A0003 NO [EXPUNGEISSUED] Some messages were removed\r\n",
        "A0003 NO [OVERQUOTA] Quota exceeded\r\n",
        "A0003 NO [unavailable] try again later\r\n",
    ] {
        let mut script = opening(Builder::new());
        script
            .write(b"A0002 EXAMINE \"INBOX\"\r\n")
            .read(examined_with_modseq("A0002", 1, 6, 7, 42).as_bytes())
            .write(format!("A0003 UID FETCH 4:6 {PASS_QUERY}\r\n").as_bytes())
            .read(format!("{}{ending}", arriving(1, 4, "Four", "")).as_bytes());

        let failure = pass_against(script, &holding("3", 3, 3))
            .await
            .expect_err("a refusal for a moment moves nothing");

        assert_eq!(failure, MailProviderError::RefusedForNow, "{ending:?}");
    }
}

#[tokio::test]
async fn a_refusal_for_a_moment_twice_in_a_row_is_a_refusal_like_any_other() {
    // A server that cannot read a message may answer `[UNAVAILABLE] ... Please try again later`,
    // and say the same of the same message on every pass. Asked again each time, the folder stops
    // at 5 for good: no new mail, no flag, no removal, nothing further back. The last pass was
    // refused so already, and this one keeps 4 and lists the rest, as it does for any refusal.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0002", 1, 6, 7, 42).as_bytes())
        .write(format!("A0003 UID FETCH 4:6 {PASS_QUERY}\r\n").as_bytes())
        .read(
            format!(
                "{}A0003 NO [UNAVAILABLE] UID FETCH Server error - Please try again later\r\n",
                arriving(1, 4, "Four", "")
            )
            .as_bytes(),
        )
        .write(format!("A0004 UID FETCH 4:6 {FACTS_QUERY}\r\n").as_bytes())
        .read(
            b"* 1 FETCH (UID 4 FLAGS () RFC822.SIZE 120 INTERNALDATE \"17-Jul-1996 02:44:25 -0700\")\r\n\
              * 2 FETCH (UID 5 FLAGS () RFC822.SIZE 4096 INTERNALDATE \"17-Jul-1996 02:44:25 -0700\")\r\n\
              * 3 FETCH (UID 6 FLAGS () RFC822.SIZE 120 INTERNALDATE \"17-Jul-1996 02:44:25 -0700\")\r\n\
              A0004 OK FETCH completed\r\n",
        );

    let batch = pass_against(
        script,
        &SyncRequest {
            refused_for_now_last_time: true,
            ..holding("3", 3, 3)
        },
    )
    .await
    .expect("a moment that has lasted a pass does not fail the folder again");

    assert_eq!(arrived_in(&batch), vec![4, 5, 6]);
    let arrived = &batch.per_mailbox[0].arrived;
    assert_eq!(arrived[0].envelope.subject.as_deref(), Some("Four"));
    assert_eq!(
        arrived[1].envelope.subject, None,
        "nothing was read from it"
    );
    assert_eq!(arrived[1].envelope.rfc822_size, Some(4096));
    assert_eq!(batch.cursor_after, SyncCursor::new("6".to_owned()));
}

#[tokio::test]
async fn what_the_message_says_is_read_out_of_the_sections_a_server_labels() {
    // The labels a real server puts on what `BODY.PEEK` asked for, the opening of the text marked
    // with where it starts. The development server proves the same through the whole adapter; this
    // pins the parts a scripted one can.
    let header = "Subject: Tuesday\r\nMessage-ID: <tuesday@example.com>\r\n\r\n";
    let text = "See you then.";
    let line = format!(
        "* 1 FETCH (UID 4 FLAGS (\\Seen) RFC822.SIZE 120 INTERNALDATE \"17-Jul-1996 02:44:25 -0700\" \
         {} {} {} {})\r\nA0003 OK FETCH completed\r\n",
        labelled("HEADER]"),
        literal(header),
        labelled("TEXT]<0>"),
        literal(text),
    );

    // A server that keeps modification sequences and whose sequence has not moved, so the fetch of
    // what is new is the only question this pass asks.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0002", 1, 4, 5, 42).as_bytes())
        .write(b"A0003 UID FETCH 4:4 (UID FLAGS MODSEQ RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT]<0.2048>)\r\n")
        .read(line.as_bytes());

    let asked = SyncRequest {
        cursor: Some(SyncCursor::new("3".to_owned())),
        uid_validity: UidValidity::new(1).ok(),
        known_window: Some(Uid::new(1).expect("non-zero")..=Uid::new(3).expect("non-zero")),
        highest_modseq: Some(ModSeq::new(42)),
        known_count: 3,
        ..request()
    };

    let batch = pass_against(script, &asked)
        .await
        .expect("the pass completes");

    let arrived = &batch.per_mailbox[0].arrived;
    assert_eq!(arrived.len(), 1);
    assert_eq!(arrived[0].uid, Uid::new(4).expect("non-zero"));
    assert_eq!(arrived[0].envelope.subject.as_deref(), Some("Tuesday"));
    assert_eq!(
        arrived[0].envelope.preview.as_deref(),
        Some("See you then.")
    );
    assert_eq!(arrived[0].envelope.rfc822_size, Some(120));
    // 17 July 1996, 09:44:25 in Greenwich.
    assert_eq!(arrived[0].envelope.received_at, 837_596_665);
    assert_eq!(batch.cursor_after, SyncCursor::new("4".to_owned()));
}

/// How a server labels a section in what it sends back: `labelled("HEADER]")` is the header's
/// label, and `labelled("]")` the whole message's.
///
/// Put together rather than written out, because `scripts/check-layering.sh` refuses the label
/// spelled whole in a string: spelled in a request it is the form that marks mail read. Here it is
/// the label on an answer, which is the form a server answers the `PEEK` request with.
fn labelled(rest: &str) -> String {
    format!("BODY{}{rest}", '[')
}

/// Bytes sent the way IMAP sends a string of any length: its length in braces, then the bytes.
fn literal(bytes: &str) -> String {
    format!("{{{}}}\r\n{bytes}", bytes.len())
}

#[tokio::test]
async fn what_left_one_mailbox_is_not_taken_for_what_left_the_next() {
    // One session reads several mailboxes. A server with QRESYNC on reports what left the mailbox
    // it is closing as `VANISHED`, in the middle of the answer that opens the next one — and the
    // client library queues it with everything else said in passing. The inbox's 7 is not the
    // archive's 7: taken as the archive's, it is an exact deletion of a message nothing removed.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 ENABLE QRESYNC\r\n")
        .read(b"* ENABLED QRESYNC\r\nA0002 OK ENABLE completed\r\n")
        .write(b"A0003 EXAMINE \"INBOX\"\r\n")
        .read(examined_with_modseq("A0003", 1, 9, 10, 90).as_bytes())
        .write(b"A0004 EXAMINE \"Archive\"\r\n")
        .read(
            format!(
                "* VANISHED 7\r\n{}",
                examined_with_modseq("A0004", 5, 9, 10, 90)
            )
            .as_bytes(),
        )
        .write(b"A0005 UID FETCH 1:* (UID FLAGS) (CHANGEDSINCE 42 VANISHED)\r\n")
        .read(b"A0005 OK FETCH completed\r\n");

    let holding_nine = |mailbox: &str, folder: u64, generation: u32, modseq: u64| SyncRequest {
        folder_id: FolderId::new(folder).expect("non-zero"),
        mailbox: mailbox.to_owned(),
        cursor: Some(SyncCursor::new("9".to_owned())),
        uid_validity: UidValidity::new(generation).ok(),
        known_window: Some(Uid::new(1).expect("non-zero")..=Uid::new(9).expect("non-zero")),
        highest_modseq: Some(ModSeq::new(modseq)),
        known_count: 9,
        ..request()
    };

    let mut session = session_against(script).await;
    let qresync = crate::infrastructure::imap::pass::enable_qresync(&mut session)
        .await
        .expect("the server answers");
    assert!(qresync);

    one_pass(&mut session, &holding_nine("INBOX", 1, 1, 90), qresync)
        .await
        .expect("the inbox is read");
    let archive = one_pass(&mut session, &holding_nine("Archive", 2, 5, 42), qresync)
        .await
        .expect("the archive is read");

    assert_eq!(
        archive.per_mailbox[0].vanished,
        VanishedReport::Unknown,
        "the archive was asked what left it and named nothing"
    );
    assert!(
        session.unsolicited_responses.try_recv().is_err(),
        "something said about the inbox is still waiting where the archive's news is read"
    );
}

/// A watch, driven the same way: the adapter opens its own session, so the whole conversation is
/// scripted from the greeting.
async fn watch_against(
    script: Builder,
    since: Option<&Watermark>,
    at_most: Duration,
) -> Result<WatchEnded, MailProviderError> {
    // A scripted stream answers at once and carries no deadline, so what the watch is allowed to
    // wait is not what these tests are about. The ones below the next helper are.
    let silence = crate::infrastructure::net::Silence::of(Duration::from_secs(60));
    watch_on(
        session_against(script).await,
        "INBOX",
        since,
        at_most,
        &silence,
    )
    .await
}

/// A watch on a session with the deadline every real one has, and how long it took by a clock
/// that only the waits move: the ordinary silence a minute, and the idle allowed its own.
async fn watch_against_a_deadline(
    script: Builder,
    at_most: Duration,
) -> (Result<WatchEnded, MailProviderError>, Duration) {
    use crate::infrastructure::net::{Deadline, Silence};

    let mut script = script;
    let silence = Silence::of(Duration::from_secs(60));
    let session = async_imap::Client::new(Deadline::over(script.build(), silence.clone()))
        .login("person", "secret")
        .await
        .map_err(|(error, _client)| error)
        .expect("the scripted server accepts the login");

    let began = tokio::time::Instant::now();
    let ended = watch_on(session, "INBOX", None, at_most, &silence).await;
    (ended, began.elapsed())
}

/// The idle begun on a mailbox of three, and the server's word that it is listening.
fn idling() -> Builder {
    let mut script = watch_opening(3, 4);
    script.write(b"A0004 IDLE\r\n").read(b"+ idling\r\n");
    script
}

#[tokio::test(start_paused = true)]
async fn a_quiet_inbox_is_watched_past_the_minute_any_other_answer_is_given() {
    // Ten quiet minutes, which is what an inbox is most of the time. Held to the deadline every
    // other answer has, the watch ends after one of them as a failure, and every watch of a quiet
    // inbox is a failure a minute.
    let mut script = idling();
    script
        .wait(Duration::from_secs(600))
        .read(b"* 4 EXISTS\r\n")
        .write(b"DONE\r\n")
        .read(b"A0004 OK IDLE terminated\r\n")
        .write(b"A0005 EXAMINE \"INBOX\"\r\n")
        .read(examined("A0005", 1, 4, 5).as_bytes())
        .write(b"A0006 LOGOUT\r\n")
        .read(b"* BYE\r\nA0006 OK LOGOUT completed\r\n");

    let (ended, took) = watch_against_a_deadline(script, Duration::from_secs(25 * 60)).await;

    assert_eq!(
        ended.expect("a quiet inbox is not a failure").watched,
        Watched::Arrived
    );
    assert_eq!(took, Duration::from_secs(600));
}

#[tokio::test(start_paused = true)]
async fn an_idle_ends_on_its_own_clock_however_often_the_server_says_it_is_still_there() {
    // A server that says "still here" every fifty seconds starts a wait counted from each line
    // all over again, and an idle counted that way lasts for as long as the server keeps
    // talking — past the half hour after which a server may drop it. Counted from when it began,
    // it ends when it was asked to, and the next one begins.
    let mut script = idling();
    for _ in 0..3 {
        script
            .wait(Duration::from_secs(50))
            .read(b"* OK Still here\r\n");
    }
    script
        .write(b"DONE\r\n")
        .read(b"A0004 OK IDLE terminated\r\n")
        .write(b"A0005 EXAMINE \"INBOX\"\r\n")
        .read(examined("A0005", 1, 3, 4).as_bytes())
        .write(b"A0006 LOGOUT\r\n")
        .read(b"* BYE\r\nA0006 OK LOGOUT completed\r\n");

    let (ended, took) = watch_against_a_deadline(script, Duration::from_secs(180)).await;

    assert_eq!(
        ended
            .expect("an idle that ran its length is not a failure")
            .watched,
        Watched::Quiet
    );
    assert_eq!(
        took,
        Duration::from_secs(180),
        "the idle outlived its length"
    );
}

#[tokio::test(start_paused = true)]
async fn what_follows_an_idle_is_held_to_the_ordinary_deadline_again() {
    // The idle's own wait is for the idle. The server that goes quiet after being told the idle is
    // over is a server that stopped answering, and it is given the minute every answer is given —
    // counted from the end of the idle, not from the last thing it said inside one.
    let mut script = idling();
    script
        .wait(Duration::from_secs(50))
        .read(b"* OK Still here\r\n")
        .write(b"DONE\r\n")
        .wait(Duration::from_secs(600));

    let (ended, took) = watch_against_a_deadline(script, Duration::from_secs(120)).await;

    assert!(
        ended.is_err(),
        "a server that never answered the end of the idle was taken for one that did"
    );
    assert_eq!(
        took,
        Duration::from_secs(120 + 60),
        "the end of the idle was waited on longer than any other answer"
    );
}

/// The opening of every watch: the server says it can push, and the inbox holds `holds`, with
/// `next` the number its next message will be given.
fn watch_opening(holds: u32, next: u32) -> Builder {
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 CAPABILITY\r\n")
        .read(b"* CAPABILITY IMAP4rev1 IDLE\r\nA0002 OK CAPABILITY completed\r\n")
        .write(b"A0003 EXAMINE \"INBOX\"\r\n")
        .read(examined("A0003", 1, holds, next).as_bytes());
    script
}

/// A watch on a mailbox of three, told `told` while idle and `after` while the idle ends, which
/// finds the inbox holding `closed` — how many, and the next number — once it has ended.
async fn watch_hearing(
    since: Option<&Watermark>,
    told: &[u8],
    after: &[u8],
    closed: (u32, u32),
) -> WatchEnded {
    let mut script = watch_opening(3, 4);
    script
        .write(b"A0004 IDLE\r\n")
        .read(b"+ idling\r\n")
        .read(told)
        // The idle is ended before the session is: a server left mid-idle holds the connection.
        .write(b"DONE\r\n");
    if !after.is_empty() {
        script.read(after);
    }
    script
        .read(b"A0004 OK IDLE terminated\r\n")
        // Where the next watch takes up, asked once the idle is over.
        .write(b"A0005 EXAMINE \"INBOX\"\r\n")
        .read(examined("A0005", 1, closed.0, closed.1).as_bytes())
        .write(b"A0006 LOGOUT\r\n")
        .read(b"* BYE\r\nA0006 OK LOGOUT completed\r\n");

    watch_against(script, since, Duration::from_secs(60))
        .await
        .expect("the watch ends with what the server said")
}

#[tokio::test]
async fn a_server_that_says_something_arrived_ends_the_watch() {
    assert_eq!(
        watch_hearing(None, b"* 4 EXISTS\r\n", b"", (4, 5))
            .await
            .watched,
        Watched::Arrived
    );
}

#[tokio::test]
async fn a_flag_changing_is_news_and_not_mail_arriving() {
    // What a push from this device looks like to the watch on the same inbox: the only kind of
    // news the scheduler may take for that push coming back.
    assert_eq!(
        watch_hearing(None, b"* 2 FETCH (FLAGS (\\Seen))\r\n", b"", (3, 4))
            .await
            .watched,
        Watched::Changed
    );
    assert_eq!(
        watch_hearing(None, b"* 2 EXPUNGE\r\n", b"* 2 EXISTS\r\n", (2, 4))
            .await
            .watched,
        Watched::Changed,
        "a server saying how many are left after one went"
    );
}

#[tokio::test]
async fn mail_arriving_behind_a_flag_change_is_mail_arriving() {
    // The idle ends on the first line, and the mail that came a moment later is in the lines that
    // arrive while it is ending. Judged by the first alone, it is taken for an echo and waits two
    // minutes for a round.
    assert_eq!(
        watch_hearing(
            None,
            b"* 2 FETCH (FLAGS (\\Seen))\r\n",
            b"* 4 EXISTS\r\n",
            (4, 5)
        )
        .await
        .watched,
        Watched::Arrived
    );
    assert_eq!(
        watch_hearing(None, b"* 2 EXPUNGE\r\n", b"* 3 EXISTS\r\n", (3, 5))
            .await
            .watched,
        Watched::Arrived,
        "one filed away and one arrived"
    );
}

#[tokio::test]
async fn mail_arriving_as_a_watch_ends_is_mail_arriving() {
    // Said by the server after the idle ended and before the mailbox was asked again, it is in no
    // line the watch heard — only in the number the next message will be given.
    assert_eq!(
        watch_hearing(None, b"* 2 FETCH (FLAGS (\\Seen))\r\n", b"", (4, 5))
            .await
            .watched,
        Watched::Arrived
    );
}

#[tokio::test]
async fn mail_that_arrived_between_two_watches_is_news_to_the_second() {
    // The first watch hears this device's own push come back and ends. While the next one is
    // being opened nothing is listening, and the mail that lands then is already counted when the
    // mailbox is opened again. Started from there, the idle would never mention it.
    let first = watch_hearing(None, b"* 2 FETCH (FLAGS (\\Seen))\r\n", b"", (3, 4)).await;
    assert_eq!(first.watched, Watched::Changed);

    for (holds, next, what) in [
        (4, 5, "one arrived"),
        (3, 5, "one arrived and one was filed away"),
    ] {
        // Reported the moment the mailbox is opened, without an idle: there is nothing to wait for.
        let mut script = watch_opening(holds, next);
        script
            .write(b"A0004 LOGOUT\r\n")
            .read(b"* BYE\r\nA0004 OK LOGOUT completed\r\n");

        let second = watch_against(script, first.seen.as_ref(), Duration::from_secs(60))
            .await
            .expect("the watch ends with what the server said");
        assert_eq!(second.watched, Watched::Arrived, "{what}");
    }
}

#[tokio::test]
async fn mail_a_watch_reported_is_not_reported_again_by_the_next() {
    // Where a watch ends is after what it heard. Taken from before it, every watch after an
    // arrival would end at once, and every arrival would be two passes.
    let first = watch_hearing(None, b"* 4 EXISTS\r\n", b"", (4, 5)).await;
    assert_eq!(first.watched, Watched::Arrived);

    let mut script = watch_opening(4, 5);
    script
        .write(b"A0004 IDLE\r\n")
        .read(b"+ idling\r\n")
        .read(b"* 2 FETCH (FLAGS (\\Seen))\r\n")
        .write(b"DONE\r\n")
        .read(b"A0004 OK IDLE terminated\r\n")
        .write(b"A0005 EXAMINE \"INBOX\"\r\n")
        .read(examined("A0005", 1, 4, 5).as_bytes())
        .write(b"A0006 LOGOUT\r\n")
        .read(b"* BYE\r\nA0006 OK LOGOUT completed\r\n");

    let second = watch_against(script, first.seen.as_ref(), Duration::from_secs(60))
        .await
        .expect("the watch ends with what the server said");
    assert_eq!(second.watched, Watched::Changed);
}

#[tokio::test]
async fn a_name_the_server_lists_and_will_not_open_is_not_a_folder() {
    // Every Gmail account lists `[Gmail]`, and a pass that opens it fails on it every time.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 LIST \"\" *\r\n")
        .read(b"* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n")
        .read(b"* LIST (\\HasChildren \\Noselect) \"/\" \"[Gmail]\"\r\n")
        .read(b"* LIST (\\HasNoChildren \\Sent) \"/\" \"[Gmail]/Sent Mail\"\r\n")
        .read(b"* LIST (\\NonExistent \\HasChildren) \"/\" \"Gone\"\r\n")
        .read(b"A0002 OK LIST completed\r\n");

    let listed = list_on(&mut session_against(script).await)
        .await
        .expect("the listing is read");

    assert_eq!(
        listed
            .iter()
            .map(|mailbox| mailbox.name.as_str())
            .collect::<Vec<_>>(),
        vec!["INBOX", "[Gmail]/Sent Mail"]
    );
}

/// What ends a listing cut short, and what each is to whoever asked: a refusal, the server
/// leaving, and the connection ending with nothing said at all.
fn listings_cut_short() -> [(&'static str, MailProviderError); 3] {
    [
        (
            "A0002 NO [SERVERBUG] Internal error\r\n",
            MailProviderError::Protocol,
        ),
        ("* BYE Server shutting down\r\n", MailProviderError::Offline),
        ("", MailProviderError::Offline),
    ]
}

#[tokio::test]
async fn a_list_of_folders_cut_short_is_no_list_of_folders() {
    // The client library's own listing ends at the tagged line whatever it says, and at a
    // connection that closed between two lines, so each of these read as an account holding the
    // inbox and nothing else. What the pass is not told about it takes for gone: a move into a
    // folder left out is taken as read back, and what waits to be found in one goes back in the
    // queue to be carried out against a folder nobody read.
    for (ending, expected) in listings_cut_short() {
        let mut script = opening(Builder::new());
        script
            .write(b"A0002 LIST \"\" *\r\n")
            .read(b"* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n");
        if !ending.is_empty() {
            script.read(ending.as_bytes());
        }

        let failure = list_on(&mut session_against(script).await)
            .await
            .expect_err("a listing cut short is not the account's folders");

        assert_eq!(failure, expected, "{ending:?}");
    }
}

#[tokio::test]
async fn a_server_that_cannot_push_is_not_asked_to() {
    // Not an error: nothing is wrong with such a server, and the timer above covers it. An error
    // here would show the person a failure for a mailbox that is working.
    let mut script = opening(Builder::new());
    script
        .write(b"A0002 CAPABILITY\r\n")
        .read(b"* CAPABILITY IMAP4rev1 CONDSTORE\r\nA0002 OK CAPABILITY completed\r\n")
        .write(b"A0003 LOGOUT\r\n")
        .read(b"* BYE\r\nA0003 OK LOGOUT completed\r\n");

    let watched = watch_against(script, None, Duration::from_secs(60))
        .await
        .expect("a server without IDLE is not a failure");

    assert_eq!(watched.watched, Watched::CannotPush);
}

/// What the application sends when it changes somebody's mailbox.
///
/// Asserted byte for byte, because the difference between a command that names a message and one
/// that names a position is a few characters — and the second one, on a mailbox another client is
/// expunging from, acts on a message nobody chose.
mod acting {
    use super::*;
    use crate::infrastructure::imap::actions::{carry_out_on, file_and_leave, file_on};
    use crate::infrastructure::imap::read_only::ReadOnly;
    use crate::ports::mail_actions::{
        Filing, FilingError, MailActionError, RefusedSearch, ServerAction,
    };
    use lyonoyl_domain::message::Flags;

    /// A session on the scripted server, wrapped in the guard an acting connection really uses.
    ///
    /// Wrapped rather than raw on purpose: this way the transcript proves the commands and the
    /// guard together, in the arrangement that ships.
    async fn acting_session(
        script: Builder,
    ) -> async_imap::Session<ReadOnly<tokio_test::io::Mock>> {
        let mut script = script;

        async_imap::Client::new(ReadOnly::acting(script.build()))
            .login("person", "secret")
            .await
            .map_err(|(error, _client)| error)
            .expect("the scripted server accepts the login")
    }

    fn uid(value: u32) -> Uid {
        Uid::new(value).expect("non-zero")
    }

    #[tokio::test]
    async fn marking_a_message_read_names_it_by_uid_and_says_so_silently() {
        let mut script = Builder::new();
        script
            .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
            .read(b"A0001 OK LOGIN completed\r\n")
            .write(b"A0002 SELECT \"INBOX\"\r\n")
            .read(b"* 3 EXISTS\r\n")
            .read(b"* OK [UIDVALIDITY 7] .\r\n")
            .read(b"A0002 OK [READ-WRITE] SELECT completed\r\n")
            .write(b"A0003 UID STORE 42 +FLAGS.SILENT (\\Seen)\r\n")
            .read(b"A0003 OK STORE completed\r\n");

        let mut session = acting_session(script).await;

        carry_out_on(
            &mut session,
            ServerAction::SetFlags {
                mailbox: "INBOX",
                uid: uid(42),
                uid_validity: generation(),
                add: Flags::SEEN,
                remove: Flags::NONE,
            },
        )
        .await
        .expect("the server did it");
    }

    /// The message a copy is kept of, and what its literal length comes to.
    const A_SENT_MESSAGE: &str = "From: person@example.com\r\n\r\nsaid";

    #[tokio::test]
    async fn a_copy_is_added_where_the_server_says_sent_mail_goes() {
        // Every byte of it: the folder is the one the server marked, the question asked first is
        // whether a copy is already there, and what is added is marked read.
        let mut script = Builder::new();
        script
            .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
            .read(b"A0001 OK LOGIN completed\r\n")
            .write(b"A0002 LIST \"\" *\r\n")
            .read(b"* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n")
            .read(b"* LIST (\\Sent \\HasNoChildren) \"/\" \"Sent Items\"\r\n")
            .read(b"A0002 OK LIST completed\r\n")
            .write(b"A0003 SELECT \"Sent Items\"\r\n")
            .read(b"* 0 EXISTS\r\n")
            .read(b"* OK [UIDVALIDITY 7] .\r\n")
            .read(b"A0003 OK [READ-WRITE] SELECT completed\r\n")
            .write(b"A0004 UID SEARCH HEADER Message-ID \"<name@example.com>\"\r\n")
            .read(b"* SEARCH\r\n")
            .read(b"A0004 OK SEARCH completed\r\n")
            .write(b"A0005 APPEND \"Sent Items\" (\\Seen) {32}\r\n")
            .read(b"+ ready\r\n")
            .write(A_SENT_MESSAGE.as_bytes())
            .write(b"\r\n")
            .read(b"A0005 OK APPEND completed\r\n");

        let mut session = acting_session(script).await;

        let outcome = file_on(
            &mut session,
            "name@example.com",
            A_SENT_MESSAGE,
            RefusedSearch::Heeded,
        )
        .await;

        assert_eq!(outcome, Ok(Filing::Added));
    }

    #[tokio::test]
    async fn a_server_that_kept_its_own_copy_is_given_no_second_one() {
        // The one mistake this path exists to avoid. Nothing is appended, and the search is what
        // decides it.
        let mut script = Builder::new();
        script
            .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
            .read(b"A0001 OK LOGIN completed\r\n")
            .write(b"A0002 LIST \"\" *\r\n")
            .read(b"* LIST (\\Sent) \"/\" \"Sent\"\r\n")
            .read(b"A0002 OK LIST completed\r\n")
            .write(b"A0003 SELECT \"Sent\"\r\n")
            .read(b"* 1 EXISTS\r\n")
            .read(b"* OK [UIDVALIDITY 7] .\r\n")
            .read(b"A0003 OK [READ-WRITE] SELECT completed\r\n")
            .write(b"A0004 UID SEARCH HEADER Message-ID \"<name@example.com>\"\r\n")
            .read(b"* SEARCH 12\r\n")
            .read(b"A0004 OK SEARCH completed\r\n");

        let mut session = acting_session(script).await;

        let outcome = file_on(
            &mut session,
            "name@example.com",
            A_SENT_MESSAGE,
            RefusedSearch::Heeded,
        )
        .await;

        assert_eq!(outcome, Ok(Filing::AlreadyThere));
    }

    #[tokio::test]
    async fn the_copy_a_server_filed_itself_is_found_by_the_name_the_outbox_keeps() {
        // The outbox keeps a name as the header carries it, angle brackets and all, and that is
        // what filing is handed. Asked for inside a second pair, it is a name no message carries:
        // the search finds nothing, and the server that filed its own copy is given another.
        let mut script = Builder::new();
        script
            .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
            .read(b"A0001 OK LOGIN completed\r\n")
            .write(b"A0002 LIST \"\" *\r\n")
            .read(b"* LIST (\\Sent) \"/\" \"Sent\"\r\n")
            .read(b"A0002 OK LIST completed\r\n")
            .write(b"A0003 SELECT \"Sent\"\r\n")
            .read(b"* 1 EXISTS\r\n")
            .read(b"* OK [UIDVALIDITY 7] .\r\n")
            .read(b"A0003 OK [READ-WRITE] SELECT completed\r\n")
            .write(b"A0004 UID SEARCH HEADER Message-ID \"<name@example.com>\"\r\n")
            .read(b"* SEARCH 12\r\n")
            .read(b"A0004 OK SEARCH completed\r\n");

        let mut session = acting_session(script).await;

        let outcome = file_on(
            &mut session,
            "<name@example.com>",
            A_SENT_MESSAGE,
            RefusedSearch::Heeded,
        )
        .await;

        assert_eq!(outcome, Ok(Filing::AlreadyThere));
    }

    /// A sent message with lines in it that read like commands to anything judging them as such:
    /// a subject beginning `Move`, and a sentence beginning `Please delete`.
    const LINES_LIKE_COMMANDS: &str = "From: zeta@example.com\r\n\
                                       Subject: Move the meeting\r\n\
                                       \r\n\
                                       Please delete it\r\n";

    #[tokio::test]
    async fn a_message_whose_lines_read_like_commands_is_filed_whole() {
        // Through the guard an acting connection really uses. Judged line by line, the message is
        // refused halfway into the folder for sent mail, and the queue of copies to file stops at
        // it for good.
        let mut script = Builder::new();
        script
            .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
            .read(b"A0001 OK LOGIN completed\r\n")
            .write(b"A0002 LIST \"\" *\r\n")
            .read(b"* LIST (\\Sent) \"/\" \"Sent\"\r\n")
            .read(b"A0002 OK LIST completed\r\n")
            .write(b"A0003 SELECT \"Sent\"\r\n")
            .read(b"* 0 EXISTS\r\n")
            .read(b"* OK [UIDVALIDITY 7] .\r\n")
            .read(b"A0003 OK [READ-WRITE] SELECT completed\r\n")
            .write(b"A0004 UID SEARCH HEADER Message-ID \"<name@example.com>\"\r\n")
            .read(b"* SEARCH\r\n")
            .read(b"A0004 OK SEARCH completed\r\n")
            .write(
                format!(
                    "A0005 APPEND \"Sent\" (\\Seen) {{{}}}\r\n",
                    LINES_LIKE_COMMANDS.len()
                )
                .as_bytes(),
            )
            .read(b"+ ready\r\n")
            .write(LINES_LIKE_COMMANDS.as_bytes())
            .write(b"\r\n")
            .read(b"A0005 OK APPEND completed\r\n");

        let mut session = acting_session(script).await;

        let outcome = file_on(
            &mut session,
            "<name@example.com>",
            LINES_LIKE_COMMANDS,
            RefusedSearch::Heeded,
        )
        .await;

        assert_eq!(outcome, Ok(Filing::Added));
    }

    #[tokio::test]
    async fn a_server_that_names_no_folder_for_sent_mail_is_left_alone() {
        // Nothing is selected and nothing is added. A folder chosen because its name resembles one
        // is how a copy of somebody's mail ends up where they did not put it.
        let mut script = Builder::new();
        script
            .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
            .read(b"A0001 OK LOGIN completed\r\n")
            .write(b"A0002 LIST \"\" *\r\n")
            .read(b"* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n")
            .read(b"* LIST (\\HasNoChildren) \"/\" \"Sent\"\r\n")
            .read(b"A0002 OK LIST completed\r\n");

        let mut session = acting_session(script).await;

        let outcome = file_on(
            &mut session,
            "name@example.com",
            A_SENT_MESSAGE,
            RefusedSearch::Heeded,
        )
        .await;

        assert_eq!(outcome, Ok(Filing::Nowhere));
    }

    #[tokio::test]
    async fn a_folder_list_cut_short_before_sent_mail_is_not_a_server_without_a_folder_for_it() {
        // Read as a whole listing, it names no folder for sent mail, and the copy is written down
        // as having nowhere to go: never filed, and never asked about again. Cut short, it is a
        // moment, and the copy is filed in a round that hears the whole of it.
        for (ending, _) in listings_cut_short() {
            let mut script = Builder::new();
            script
                .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
                .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
                .read(b"A0001 OK LOGIN completed\r\n")
                .write(b"A0002 LIST \"\" *\r\n")
                .read(b"* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n");
            if !ending.is_empty() {
                script.read(ending.as_bytes());
            }

            let mut session = acting_session(script).await;

            let outcome = file_on(
                &mut session,
                "name@example.com",
                A_SENT_MESSAGE,
                RefusedSearch::Heeded,
            )
            .await;

            assert_eq!(outcome, Err(FilingError::FolderUnreachable), "{ending:?}");
        }
    }

    /// The conversation up to the question whether a copy is there already: signed in, and the
    /// folder for sent mail listed and opened.
    fn sent_mail_opened() -> Builder {
        let mut script = Builder::new();
        script
            .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
            .read(b"A0001 OK LOGIN completed\r\n")
            .write(b"A0002 LIST \"\" *\r\n")
            .read(b"* LIST (\\Sent) \"/\" \"Sent\"\r\n")
            .read(b"A0002 OK LIST completed\r\n")
            .write(b"A0003 SELECT \"Sent\"\r\n")
            .read(b"* 0 EXISTS\r\n")
            .read(b"* OK [UIDVALIDITY 7] .\r\n")
            .read(b"A0003 OK [READ-WRITE] SELECT completed\r\n");
        script
    }

    /// The question it asks first: whether the copy is there already.
    const ASKED_WHETHER_IT_IS_THERE: &[u8] =
        b"A0004 UID SEARCH HEADER Message-ID \"<name@example.com>\"\r\n";

    /// The same conversation once that is answered: no copy there yet, and the message announced.
    fn announced() -> Builder {
        let mut script = sent_mail_opened();
        script
            .write(ASKED_WHETHER_IT_IS_THERE)
            .read(b"* SEARCH\r\n")
            .read(b"A0004 OK SEARCH completed\r\n")
            .write(b"A0005 APPEND \"Sent\" (\\Seen) {32}\r\n");
        script
    }

    /// The copy sent once the server says to go on.
    fn sent_whole(script: &mut Builder) {
        script
            .read(b"+ ready\r\n")
            .write(A_SENT_MESSAGE.as_bytes())
            .write(b"\r\n");
    }

    /// Reads what a scripted server has left to say, so the script is spent and says so: a
    /// conversation abandoned partway is ended by dropping it, which a script takes for a line
    /// it was never asked for.
    async fn heard_out(session: &mut async_imap::Session<ReadOnly<tokio_test::io::Mock>>) {
        while let Ok(Some(_)) = session.read_response().await {}
    }

    #[tokio::test]
    async fn a_name_with_a_quote_in_it_is_searched_for_as_one_string() {
        // The domain a name is made from holds no line break, and nothing stops it holding a
        // quote. Written into the search as it stands, the quote would end the string there.
        let mut script = sent_mail_opened();
        script
            .write(b"A0004 UID SEARCH HEADER Message-ID \"<name@ex\\\"am\\\\ple.com>\"\r\n")
            .read(b"* SEARCH 12\r\n")
            .read(b"A0004 OK SEARCH completed\r\n");

        let mut session = acting_session(script).await;

        let outcome = file_on(
            &mut session,
            "name@ex\"am\\ple.com",
            A_SENT_MESSAGE,
            RefusedSearch::Heeded,
        )
        .await;

        assert_eq!(outcome, Ok(Filing::AlreadyThere));
    }

    #[tokio::test]
    async fn a_copy_refused_before_it_is_sent_is_refused_as_the_server_says() {
        // A server that knows how long a message is says before taking a byte of it that it will
        // not: too large, or over quota. Read through the client library, that refusal arrives as
        // no refusal at all, with the words that say which lost. Nothing of the message is sent.
        //
        // A limit reached is for now, whatever the words after it: `[LIMIT]` is given to a limit
        // of any kind, a limit on how often an account may ask among them, and one taken for good
        // while it lasts lets the message go with no copy anywhere its sender can find it.
        for (refusal, expected) in [
            (
                "A0005 NO [OVERQUOTA] Quota exceeded (mailbox for user is full)\r\n",
                FilingError::RefusedForNow,
            ),
            (
                "A0005 NO [TOOBIG] Mail is larger than the server takes\r\n",
                FilingError::RefusedForGood,
            ),
            (
                "A0005 NO [LIMIT] Mail size exceeds the limit\r\n",
                FilingError::RefusedForNow,
            ),
            (
                "A0005 NO [LIMIT] APPEND Rate limit hit.\r\n",
                FilingError::RefusedForNow,
            ),
        ] {
            let mut script = announced();
            script.read(refusal.as_bytes());

            let mut session = acting_session(script).await;

            let outcome = file_on(
                &mut session,
                "name@example.com",
                A_SENT_MESSAGE,
                RefusedSearch::Heeded,
            )
            .await;

            assert_eq!(outcome, Err(expected), "{refusal:?}");
        }
    }

    #[tokio::test]
    async fn a_copy_refused_once_it_is_sent_is_refused_as_the_server_says() {
        // Told apart by the code the words open with. Whatever says "later", or nothing about
        // when, is refused for now: taken for good, a refusal that would have passed leaves the
        // message with no copy anywhere its sender can find it. A limit reached is one of those, a
        // limit that passes being one. Whatever says "never" is refused for good, and whatever says
        // the folder is not there, or not this account's to add to, is the folder not reached,
        // which the copy behind this one would meet as well.
        for (refusal, expected) in [
            (
                "A0005 NO [OVERQUOTA] Quota exceeded\r\n",
                FilingError::RefusedForNow,
            ),
            (
                "A0005 NO [UNAVAILABLE] Try again later\r\n",
                FilingError::RefusedForNow,
            ),
            (
                "A0005 NO [INUSE] Mailbox is locked\r\n",
                FilingError::RefusedForNow,
            ),
            (
                "A0005 NO [ALERT] You have exceeded your mail quota\r\n",
                FilingError::RefusedForNow,
            ),
            ("A0005 NO Message refused\r\n", FilingError::RefusedForNow),
            ("A0005 NO\r\n", FilingError::RefusedForNow),
            (
                "A0005 BAD Invalid arguments\r\n",
                FilingError::RefusedForNow,
            ),
            (
                "A0005 NO [TOOBIG] Message too large\r\n",
                FilingError::RefusedForGood,
            ),
            (
                "A0005 NO [toobig] Message too large\r\n",
                FilingError::RefusedForGood,
            ),
            (
                "A0005 NO [LIMIT] Too many messages in the mailbox\r\n",
                FilingError::RefusedForNow,
            ),
            (
                "A0005 NO [CANNOT] This can never be stored\r\n",
                FilingError::RefusedForGood,
            ),
            (
                "A0005 NO [TRYCREATE] Mailbox does not exist\r\n",
                FilingError::FolderUnreachable,
            ),
            (
                "A0005 NO [NONEXISTENT] No such mailbox\r\n",
                FilingError::FolderUnreachable,
            ),
            (
                "A0005 NO [NOPERM] Permission denied\r\n",
                FilingError::FolderUnreachable,
            ),
        ] {
            let mut script = announced();
            sent_whole(&mut script);
            script.read(refusal.as_bytes());

            let mut session = acting_session(script).await;

            let outcome = file_on(
                &mut session,
                "name@example.com",
                A_SENT_MESSAGE,
                RefusedSearch::Heeded,
            )
            .await;

            assert_eq!(outcome, Err(expected), "{refusal:?}");
        }
    }

    #[tokio::test]
    async fn a_copy_is_filed_once_its_own_line_says_so_whatever_comes_before_it() {
        // The folder the copy goes into is the one open on the session, so a server says what
        // changed in it before it says the copy is there. Those lines are not the answer.
        let mut script = announced();
        sent_whole(&mut script);
        script
            .read(b"* 1 EXISTS\r\n")
            .read(b"* 1 RECENT\r\n")
            .read(b"A0005 OK [APPENDUID 7 1] APPEND completed\r\n");

        let mut session = acting_session(script).await;

        let outcome = file_on(
            &mut session,
            "name@example.com",
            A_SENT_MESSAGE,
            RefusedSearch::Heeded,
        )
        .await;

        assert_eq!(outcome, Ok(Filing::Added));
    }

    #[tokio::test]
    async fn a_search_the_server_does_not_answer_whole_adds_no_copy() {
        // The question that keeps a server which files its own copy from being given a second one.
        // The client library ends a search at its tagged line whatever that line says, and at a
        // connection that closed, and either reads as a folder without the copy: a second one is
        // added. Here, on a try with waits left, a refusal that names no copy and is not about the
        // folder is one of this copy for now, whatever else its code says, one that says never
        // among them: a search a server will never answer is otherwise a copy never offered. A
        // refusal about the folder is the folder not reached, and so is a search cut short or
        // answered out of step. Nothing is added either way, and the script holds no line for an
        // append to be written.
        for (answer, expected) in [
            (
                "A0004 NO [UNAVAILABLE] Search is not available now\r\n",
                FilingError::RefusedForNow,
            ),
            (
                "A0004 NO [LIMIT] UID SEARCH Rate limit hit.\r\n",
                FilingError::RefusedForNow,
            ),
            (
                "A0004 NO [CANNOT] Cannot search headers\r\n",
                FilingError::RefusedForNow,
            ),
            ("A0004 NO Search failed\r\n", FilingError::RefusedForNow),
            ("A0004 NO\r\n", FilingError::RefusedForNow),
            (
                "A0004 BAD Unknown search key\r\n",
                FilingError::RefusedForNow,
            ),
            (
                "A0004 NO [TRYCREATE] Mailbox does not exist\r\n",
                FilingError::FolderUnreachable,
            ),
            (
                "A0004 NO [NONEXISTENT] No such mailbox\r\n",
                FilingError::FolderUnreachable,
            ),
            (
                "A0004 NO [NOPERM] Permission denied\r\n",
                FilingError::FolderUnreachable,
            ),
            (
                "A0004 BYE Server shutting down\r\n",
                FilingError::FolderUnreachable,
            ),
            (
                "* BYE Server shutting down\r\n",
                FilingError::FolderUnreachable,
            ),
            ("", FilingError::FolderUnreachable),
        ] {
            let mut script = sent_mail_opened();
            script
                .write(ASKED_WHETHER_IT_IS_THERE)
                .read(b"* SEARCH\r\n");
            if !answer.is_empty() {
                script.read(answer.as_bytes());
            }

            let mut session = acting_session(script).await;

            let outcome = file_on(
                &mut session,
                "name@example.com",
                A_SENT_MESSAGE,
                RefusedSearch::Heeded,
            )
            .await;

            assert_eq!(outcome, Err(expected), "{answer:?}");
        }
    }

    #[tokio::test]
    async fn a_search_refused_once_the_waits_are_over_is_passed_over_and_the_copy_offered() {
        // A server that does not file sent mail itself and will not search for the copy, whatever
        // it says about when or why, so long as its refusal names no copy and is not about the
        // folder. Heeded on this try as well, the message would go with no copy anywhere its
        // sender can find it. Passed over, the copy is offered as it is to a folder searched and
        // found without it, and the script fails on a line left unwritten.
        for refusal in [
            "A0004 NO [CANNOT] Cannot search headers\r\n",
            "A0004 NO Search failed\r\n",
            "A0004 NO\r\n",
            "A0004 BAD Unknown search key\r\n",
        ] {
            let mut script = sent_mail_opened();
            script
                .write(ASKED_WHETHER_IT_IS_THERE)
                .read(refusal.as_bytes())
                .write(b"A0005 APPEND \"Sent\" (\\Seen) {32}\r\n");
            sent_whole(&mut script);
            script.read(b"A0005 OK APPEND completed\r\n");

            let mut session = acting_session(script).await;

            let outcome = file_on(
                &mut session,
                "name@example.com",
                A_SENT_MESSAGE,
                RefusedSearch::PassedOver,
            )
            .await;

            assert_eq!(outcome, Ok(Filing::Added), "{refusal:?}");
        }
    }

    #[tokio::test]
    async fn a_copy_the_search_finds_once_the_waits_are_over_is_not_added_again() {
        // Only a refusal is passed over: the search is asked on the last try as on every other,
        // and a server that files its own copy and says so is not given a second one. The script
        // holds no line for an append to be written.
        let mut script = sent_mail_opened();
        script
            .write(ASKED_WHETHER_IT_IS_THERE)
            .read(b"* SEARCH 12\r\n")
            .read(b"A0004 OK SEARCH completed\r\n");

        let mut session = acting_session(script).await;

        let outcome = file_on(
            &mut session,
            "name@example.com",
            A_SENT_MESSAGE,
            RefusedSearch::PassedOver,
        )
        .await;

        assert_eq!(outcome, Ok(Filing::AlreadyThere));
    }

    #[tokio::test]
    async fn a_copy_the_search_names_before_it_is_refused_is_not_added_again() {
        // A server that names the copy and then refuses the search has said it has it: a search
        // that stops partway may leave a message out, and the one it named carries the name. Taken
        // for a refusal, the copy is offered again once the waits are over, and a server that
        // files its own keeps two. A refusal about the folder is the folder not reached all the
        // same. The script holds no line for an append to be written.
        for refused_search in [RefusedSearch::Heeded, RefusedSearch::PassedOver] {
            for (answer, expected) in [
                (
                    "A0004 NO [CANNOT] Cannot search headers\r\n",
                    Ok(Filing::AlreadyThere),
                ),
                (
                    "A0004 NO [LIMIT] UID SEARCH Rate limit hit.\r\n",
                    Ok(Filing::AlreadyThere),
                ),
                ("A0004 NO\r\n", Ok(Filing::AlreadyThere)),
                ("A0004 BAD Unknown search key\r\n", Ok(Filing::AlreadyThere)),
                (
                    "A0004 NO [NONEXISTENT] No such mailbox\r\n",
                    Err(FilingError::FolderUnreachable),
                ),
            ] {
                let mut script = sent_mail_opened();
                script
                    .write(ASKED_WHETHER_IT_IS_THERE)
                    .read(b"* SEARCH 12\r\n")
                    .read(answer.as_bytes());

                let mut session = acting_session(script).await;

                let outcome = file_on(
                    &mut session,
                    "name@example.com",
                    A_SENT_MESSAGE,
                    refused_search,
                )
                .await;

                assert_eq!(outcome, expected, "{answer:?}, {refused_search:?}");
            }
        }
    }

    #[tokio::test]
    async fn a_search_refused_for_the_folder_once_the_waits_are_over_adds_no_copy() {
        // What is passed over is a refusal of this copy, and nothing else. One that says the
        // folder is not there or not this account's to add to is about every copy behind this one
        // as well, and is the folder not reached on the last try as on any other; so is a search
        // cut short or answered out of step. The script holds no line for an append to be written.
        for answer in [
            "A0004 NO [TRYCREATE] Mailbox does not exist\r\n",
            "A0004 NO [NONEXISTENT] No such mailbox\r\n",
            "A0004 NO [NOPERM] Permission denied\r\n",
            "A0004 BYE Server shutting down\r\n",
            "* BYE Server shutting down\r\n",
            "",
        ] {
            let mut script = sent_mail_opened();
            script
                .write(ASKED_WHETHER_IT_IS_THERE)
                .read(b"* SEARCH\r\n");
            if !answer.is_empty() {
                script.read(answer.as_bytes());
            }

            let mut session = acting_session(script).await;

            let outcome = file_on(
                &mut session,
                "name@example.com",
                A_SENT_MESSAGE,
                RefusedSearch::PassedOver,
            )
            .await;

            assert_eq!(outcome, Err(FilingError::FolderUnreachable), "{answer:?}");
        }
    }

    #[tokio::test]
    async fn a_message_is_not_sent_unless_the_first_thing_the_server_says_is_to_go_on() {
        // The guard on the connection passes the message unread after a `+` and only then, and
        // reads it as commands otherwise. A server that says something else first will not have
        // the message follow, or has lost step with the conversation: the message is not sent, and
        // the script fails on a byte of it written.
        let mut script = announced();
        script.read(b"* 1 EXISTS\r\n").read(b"+ ready\r\n");

        let mut session = acting_session(script).await;

        let outcome = file_on(
            &mut session,
            "name@example.com",
            A_SENT_MESSAGE,
            RefusedSearch::Heeded,
        )
        .await;
        heard_out(&mut session).await;

        assert_eq!(outcome, Err(FilingError::FolderUnreachable));
    }

    #[tokio::test]
    async fn a_conversation_that_ends_once_the_copy_is_offered_counts_against_that_copy_for_now() {
        // Offered, the copy is the server's to answer, and a server may hang up on one it will not
        // take rather than refuse it. Taken for the folder not reached, that copy would be asked
        // about first in every round, uploaded whole each time, with every copy behind it held up
        // for good. For now, it is counted and bounded: a copy the server kept is found by the
        // search a later try opens with, and offered a second time only where the server refuses
        // that search up to the try after the last wait.
        //
        // And the conversation is dropped rather than signed out of: one that broke carries
        // nothing further, and the script fails on a line written after the end of it.
        let gone = || std::io::Error::new(std::io::ErrorKind::ConnectionReset, "gone");
        let mut endings = Vec::new();

        let mut said_goodbye_when_offered = announced();
        said_goodbye_when_offered.read(b"* BYE Server shutting down\r\n");
        endings.push(("a goodbye when offered", said_goodbye_when_offered));

        endings.push(("closed when offered", announced()));

        let mut cut_off_partway = announced();
        cut_off_partway.read(b"+ ready\r\n").write_error(gone());
        endings.push(("closed with the message partway", cut_off_partway));

        let mut said_goodbye_once_sent = announced();
        sent_whole(&mut said_goodbye_once_sent);
        said_goodbye_once_sent.read(b"* BYE Server shutting down\r\n");
        endings.push(("a goodbye once sent", said_goodbye_once_sent));

        let mut closed_once_sent = announced();
        sent_whole(&mut closed_once_sent);
        endings.push(("closed once sent", closed_once_sent));

        let mut failed_once_sent = announced();
        sent_whole(&mut failed_once_sent);
        failed_once_sent.read_error(gone());
        endings.push(("failed once sent", failed_once_sent));

        for (ending, script) in endings {
            let session = acting_session(script).await;

            let outcome = file_and_leave(
                session,
                "name@example.com",
                A_SENT_MESSAGE,
                RefusedSearch::Heeded,
            )
            .await;

            assert_eq!(outcome, Err(FilingError::RefusedForNow), "{ending}");
        }
    }

    #[tokio::test]
    async fn a_conversation_answered_to_the_end_is_signed_out_of() {
        // Whole, it is ended the polite way, and the script fails on a line it holds that was
        // never written.
        for (answer, expected) in [
            (
                "A0005 OK [APPENDUID 7 1] APPEND completed\r\n",
                Ok(Filing::Added),
            ),
            (
                "A0005 NO [TOOBIG] Message too large\r\n",
                Err(FilingError::RefusedForGood),
            ),
        ] {
            let mut script = announced();
            sent_whole(&mut script);
            script
                .read(answer.as_bytes())
                .write(b"A0006 LOGOUT\r\n")
                .read(b"* BYE\r\nA0006 OK LOGOUT completed\r\n");

            let session = acting_session(script).await;

            let outcome = file_and_leave(
                session,
                "name@example.com",
                A_SENT_MESSAGE,
                RefusedSearch::Heeded,
            )
            .await;

            assert_eq!(outcome, expected, "{answer:?}");
        }
    }

    #[tokio::test]
    async fn a_folder_not_reached_ends_the_conversation_without_a_sign_out() {
        // Nothing was heard about the copy, and the conversation may be one that broke: a search
        // that will not stop talking, a goodbye, an offer answered out of step. A sign-out written
        // into it waits on a server that never falls silent, which the deadline on the connection
        // does not meet, on one that is no longer listening, or on one still waiting for the
        // message it was offered. Dropped instead: each script ends where its answer does, and
        // fails on a line written after that.
        let mut endings = Vec::new();

        let mut search_kept_talking = sent_mail_opened();
        search_kept_talking.write(ASKED_WHETHER_IT_IS_THERE);
        for _ in 0..crate::infrastructure::imap::actions::LINES_ONE_COPY_IS_ANSWERED_IN {
            search_kept_talking.read(b"* 1 EXISTS\r\n");
        }
        endings.push(("a search that kept talking", search_kept_talking));

        let mut search_said_goodbye = sent_mail_opened();
        search_said_goodbye
            .write(ASKED_WHETHER_IT_IS_THERE)
            .read(b"* BYE Server shutting down\r\n");
        endings.push(("a goodbye to the search", search_said_goodbye));

        let mut offer_out_of_step = announced();
        offer_out_of_step.read(b"* 1 EXISTS\r\n");
        endings.push(("an offer answered out of step", offer_out_of_step));

        for (ending, script) in endings {
            let session = acting_session(script).await;

            let outcome = file_and_leave(
                session,
                "name@example.com",
                A_SENT_MESSAGE,
                RefusedSearch::Heeded,
            )
            .await;

            assert_eq!(outcome, Err(FilingError::FolderUnreachable), "{ending}");
        }
    }

    #[tokio::test]
    async fn a_server_that_keeps_talking_instead_of_answering_is_not_listened_to_for_ever() {
        // The deadline on the connection measures silence, and a server that never falls silent
        // never meets it. Past the lines an honest answer takes, it is a conversation that broke,
        // once the copy was offered, and that counts against the copy for now.
        let mut script = announced();
        sent_whole(&mut script);
        for _ in 0..=crate::infrastructure::imap::actions::LINES_ONE_COPY_IS_ANSWERED_IN {
            script.read(b"* 1 EXISTS\r\n");
        }
        script.read(b"A0005 OK APPEND completed\r\n");

        let mut session = acting_session(script).await;

        let outcome = file_on(
            &mut session,
            "name@example.com",
            A_SENT_MESSAGE,
            RefusedSearch::Heeded,
        )
        .await;
        heard_out(&mut session).await;

        assert_eq!(outcome, Err(FilingError::RefusedForNow));
    }

    #[tokio::test]
    async fn a_server_that_keeps_talking_instead_of_answering_the_search_adds_no_copy() {
        // The same bound on the question asked first. Past it, the search is a conversation that
        // broke before the copy was offered: the folder not reached, and no copy added, so a
        // server that files its own is not given a second one. The script holds no line for an
        // append to be written.
        let mut script = sent_mail_opened();
        script.write(ASKED_WHETHER_IT_IS_THERE);
        for _ in 0..=crate::infrastructure::imap::actions::LINES_ONE_COPY_IS_ANSWERED_IN {
            script.read(b"* 1 EXISTS\r\n");
        }
        script.read(b"A0004 OK SEARCH completed\r\n");

        let mut session = acting_session(script).await;

        let outcome = file_on(
            &mut session,
            "name@example.com",
            A_SENT_MESSAGE,
            RefusedSearch::Heeded,
        )
        .await;
        heard_out(&mut session).await;

        assert_eq!(outcome, Err(FilingError::FolderUnreachable));
    }

    #[tokio::test]
    async fn a_folder_for_sent_mail_the_server_will_not_open_is_not_reached() {
        // Whatever the copy, it is not getting in, and neither is the one behind it.
        let mut script = Builder::new();
        script
            .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
            .read(b"A0001 OK LOGIN completed\r\n")
            .write(b"A0002 LIST \"\" *\r\n")
            .read(b"* LIST (\\Sent) \"/\" \"Sent\"\r\n")
            .read(b"A0002 OK LIST completed\r\n")
            .write(b"A0003 SELECT \"Sent\"\r\n")
            .read(b"A0003 NO [NONEXISTENT] Mailbox does not exist\r\n");

        let mut session = acting_session(script).await;

        let outcome = file_on(
            &mut session,
            "name@example.com",
            A_SENT_MESSAGE,
            RefusedSearch::Heeded,
        )
        .await;

        assert_eq!(outcome, Err(FilingError::FolderUnreachable));
    }

    #[tokio::test]
    async fn a_server_that_states_no_generation_is_not_acted_on() {
        // RFC 3501 requires a server to state UIDVALIDITY when it opens a mailbox. One that does
        // not is one there is nothing to check a number against, and every command below this
        // point is addressed by number.
        let mut script = Builder::new();
        script
            .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
            .read(b"A0001 OK LOGIN completed\r\n")
            .write(b"A0002 SELECT \"INBOX\"\r\n")
            .read(b"* 3 EXISTS\r\n")
            .read(b"A0002 OK [READ-WRITE] SELECT completed\r\n");

        let mut session = acting_session(script).await;

        let outcome = carry_out_on(
            &mut session,
            ServerAction::SetFlags {
                mailbox: "INBOX",
                uid: uid(42),
                uid_validity: generation(),
                add: Flags::SEEN,
                remove: Flags::NONE,
            },
        )
        .await;

        assert_eq!(outcome, Err(MailActionError::FolderRenumbered));
    }

    #[tokio::test]
    async fn a_mailbox_that_renumbered_is_left_alone() {
        // The number was taken under one generation and the server states another, so it names
        // whichever message holds it now — somebody else's. Nothing is sent: not the flag, not
        // anything. And it is not a refusal either, because the pass that reads the folder is what
        // brings the address up to date.
        let mut script = Builder::new();
        script
            .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
            .read(b"A0001 OK LOGIN completed\r\n")
            .write(b"A0002 SELECT \"INBOX\"\r\n")
            .read(b"* 3 EXISTS\r\n")
            .read(b"* OK [UIDVALIDITY 99] .\r\n")
            .read(b"A0002 OK [READ-WRITE] SELECT completed\r\n");

        let mut session = acting_session(script).await;

        let outcome = carry_out_on(
            &mut session,
            ServerAction::SetFlags {
                mailbox: "INBOX",
                uid: uid(42),
                uid_validity: generation(),
                add: Flags::SEEN,
                remove: Flags::NONE,
            },
        )
        .await;

        assert_eq!(outcome, Err(MailActionError::FolderRenumbered));
    }

    #[tokio::test]
    async fn taking_a_flag_away_is_its_own_command() {
        let mut script = Builder::new();
        script
            .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
            .read(b"A0001 OK LOGIN completed\r\n")
            .write(b"A0002 SELECT \"INBOX\"\r\n")
            .read(b"* OK [UIDVALIDITY 7] .\r\n")
            .read(b"A0002 OK [READ-WRITE] SELECT completed\r\n")
            .write(b"A0003 UID STORE 42 -FLAGS.SILENT (\\Seen)\r\n")
            .read(b"A0003 OK STORE completed\r\n");

        let mut session = acting_session(script).await;

        carry_out_on(
            &mut session,
            ServerAction::SetFlags {
                mailbox: "INBOX",
                uid: uid(42),
                uid_validity: generation(),
                add: Flags::NONE,
                remove: Flags::SEEN,
            },
        )
        .await
        .expect("the server did it");
    }

    #[tokio::test]
    async fn filing_a_message_is_one_command_where_the_server_can_do_it() {
        // One step is the only way the message is never in both places or in neither.
        let mut script = Builder::new();
        script
            .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
            .read(b"A0001 OK LOGIN completed\r\n")
            .write(b"A0002 SELECT \"INBOX\"\r\n")
            .read(b"* OK [UIDVALIDITY 7] .\r\n")
            .read(b"A0002 OK [READ-WRITE] SELECT completed\r\n")
            .write(b"A0003 CAPABILITY\r\n")
            .read(b"* CAPABILITY IMAP4rev1 MOVE UIDPLUS\r\n")
            .read(b"A0003 OK CAPABILITY completed\r\n")
            .write(b"A0004 UID MOVE 42 \"Archive\"\r\n")
            .read(b"A0004 OK MOVE completed\r\n");

        let mut session = acting_session(script).await;

        carry_out_on(
            &mut session,
            ServerAction::MoveTo {
                mailbox: "INBOX",
                uid: uid(42),
                uid_validity: generation(),
                destination: "Archive",
            },
        )
        .await
        .expect("the server did it");
    }

    #[tokio::test]
    async fn a_message_is_never_erased_on_a_server_that_cannot_name_what_it_removes() {
        // Without UIDPLUS the only expunge there is removes every message in the mailbox marked
        // deleted, including ones another client marked. Refusing is a thing a person can see;
        // removing somebody else's mail is not.
        let mut script = Builder::new();
        script
            .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
            .read(b"A0001 OK LOGIN completed\r\n")
            .write(b"A0002 SELECT \"INBOX\"\r\n")
            .read(b"* OK [UIDVALIDITY 7] .\r\n")
            .read(b"A0002 OK [READ-WRITE] SELECT completed\r\n")
            .write(b"A0003 CAPABILITY\r\n")
            .read(b"* CAPABILITY IMAP4rev1\r\n")
            .read(b"A0003 OK CAPABILITY completed\r\n");

        let mut session = acting_session(script).await;

        let outcome = carry_out_on(
            &mut session,
            ServerAction::Erase {
                mailbox: "INBOX",
                uid: uid(42),
                uid_validity: generation(),
            },
        )
        .await;

        assert_eq!(outcome, Err(MailActionError::Refused));
    }

    #[tokio::test]
    async fn erasing_a_message_marks_that_one_and_removes_that_one() {
        let mut script = Builder::new();
        script
            .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
            .read(b"A0001 OK LOGIN completed\r\n")
            .write(b"A0002 SELECT \"INBOX\"\r\n")
            .read(b"* OK [UIDVALIDITY 7] .\r\n")
            .read(b"A0002 OK [READ-WRITE] SELECT completed\r\n")
            .write(b"A0003 CAPABILITY\r\n")
            .read(b"* CAPABILITY IMAP4rev1 UIDPLUS\r\n")
            .read(b"A0003 OK CAPABILITY completed\r\n")
            .write(b"A0004 UID STORE 42 +FLAGS.SILENT (\\Deleted)\r\n")
            .read(b"A0004 OK STORE completed\r\n")
            .write(b"A0005 UID EXPUNGE 42\r\n")
            .read(b"* 3 EXPUNGE\r\n")
            .read(b"A0005 OK EXPUNGE completed\r\n");

        let mut session = acting_session(script).await;

        carry_out_on(
            &mut session,
            ServerAction::Erase {
                mailbox: "INBOX",
                uid: uid(42),
                uid_validity: generation(),
            },
        )
        .await
        .expect("the server did it");
    }

    #[tokio::test]
    async fn a_server_that_refuses_is_not_asked_again() {
        // A refusal spelled out by the server is an answer. Trying again gets the same one, and
        // the person is shown it instead.
        let mut script = Builder::new();
        script
            .read(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .write(b"A0001 LOGIN \"person\" \"secret\"\r\n")
            .read(b"A0001 OK LOGIN completed\r\n")
            .write(b"A0002 SELECT \"Gone\"\r\n")
            .read(b"A0002 NO [NONEXISTENT] Mailbox does not exist\r\n");

        let mut session = acting_session(script).await;

        let outcome = carry_out_on(
            &mut session,
            ServerAction::SetFlags {
                mailbox: "Gone",
                uid: uid(42),
                uid_validity: generation(),
                add: Flags::SEEN,
                remove: Flags::NONE,
            },
        )
        .await;

        assert_eq!(outcome, Err(MailActionError::Refused));
    }
}

/// Reading a message whole, which is done by a number the index stored long before.
mod reading_whole {
    use super::*;
    use crate::infrastructure::imap::{whole_message_on, whole_messages_on};

    const MESSAGE: &str = "Subject: Tuesday\r\n\r\nSee you then.\r\n";

    /// The conversation up to the fetch, the mailbox stating `uid_validity` when it is opened.
    fn opened_in(uid_validity: Option<u32>) -> Builder {
        let mut script = opening(Builder::new());
        script.write(b"A0002 EXAMINE \"INBOX\"\r\n");
        match uid_validity {
            Some(generation) => script.read(examined("A0002", generation, 50, 60).as_bytes()),
            None => script.read(
                b"* 50 EXISTS\r\n* OK [UIDNEXT 60] predicted next\r\nA0002 OK [READ-ONLY] EXAMINE completed\r\n",
            ),
        };
        script
    }

    #[tokio::test]
    async fn a_message_is_read_whole_in_the_generation_its_number_was_stored_in() {
        let mut script = opened_in(Some(7));
        script.write(b"A0003 UID FETCH 42 (BODY.PEEK[])\r\n").read(
            format!(
                "* 1 FETCH (UID 42 {} {})\r\nA0003 OK FETCH completed\r\n",
                labelled("]"),
                literal(MESSAGE)
            )
            .as_bytes(),
        );

        let raw = whole_message_on(
            &mut session_against(script).await,
            "INBOX",
            generation(),
            Uid::new(42).expect("non-zero"),
        )
        .await
        .expect("the message is read");

        assert_eq!(raw, MESSAGE.as_bytes());
    }

    #[tokio::test]
    async fn a_mailbox_that_renumbered_is_not_read_by_a_number_from_before() {
        // The index stored 42 in generation 7, and the mailbox counts in 9 now: its 42 is whoever
        // was given the number since. The script holds no fetch, and a fetch sent anyway is a write
        // the scripted server fails on.
        let script = opened_in(Some(9));

        let failure = whole_message_on(
            &mut session_against(script).await,
            "INBOX",
            generation(),
            Uid::new(42).expect("non-zero"),
        )
        .await
        .expect_err("a number from another generation reads nothing");

        assert_eq!(failure, MailProviderError::Protocol);
    }

    #[tokio::test]
    async fn a_mailbox_that_states_no_generation_is_not_read_by_number() {
        // Nothing to check the stored number against is not a match.
        let script = opened_in(None);

        let failure = whole_message_on(
            &mut session_against(script).await,
            "INBOX",
            generation(),
            Uid::new(42).expect("non-zero"),
        )
        .await
        .expect_err("a number nothing can check reads nothing");

        assert_eq!(failure, MailProviderError::Protocol);
    }

    #[tokio::test]
    async fn several_messages_are_read_only_in_the_generation_their_numbers_were_stored_in() {
        // The round that fills in bodies asks for ten at a time, and every one of them would be
        // kept as the body of a message it is not.
        let script = opened_in(Some(9));

        let failure = whole_messages_on(
            &mut session_against(script).await,
            "INBOX",
            generation(),
            &[
                Uid::new(41).expect("non-zero"),
                Uid::new(42).expect("non-zero"),
            ],
        )
        .await
        .expect_err("numbers from another generation read nothing");

        assert_eq!(
            failure,
            Unread::NotAsked(MailProviderError::Protocol),
            "the question about the messages never went out"
        );
    }

    #[tokio::test]
    async fn several_messages_in_the_generation_they_were_stored_in_are_read() {
        let mut script = opened_in(Some(7));
        script
            .write(b"A0003 UID FETCH 41,42 (BODY.PEEK[])\r\n")
            .read(
                format!(
                    "* 1 FETCH (UID 41 {whole} {first})\r\n* 2 FETCH (UID 42 {whole} {second})\r\n\
                     A0003 OK FETCH completed\r\n",
                    whole = labelled("]"),
                    first = literal("Subject: One\r\n\r\n1\r\n"),
                    second = literal(MESSAGE),
                )
                .as_bytes(),
            );

        let got = whole_messages_on(
            &mut session_against(script).await,
            "INBOX",
            generation(),
            &[
                Uid::new(41).expect("non-zero"),
                Uid::new(42).expect("non-zero"),
            ],
        )
        .await
        .expect("the messages are read");

        assert_eq!(got.len(), 2);
        assert_eq!(got[1], (Uid::new(42).expect("non-zero"), MESSAGE.into()));
    }

    /// Two messages asked for at once, the first answered with `MESSAGE` and then `ended`.
    fn the_first_of_two_and_then(ended: &str) -> Builder {
        let mut script = opened_in(Some(7));
        script
            .write(b"A0003 UID FETCH 41,42 (BODY.PEEK[])\r\n")
            .read(
                format!(
                    "* 1 FETCH (UID 41 {} {})\r\n{ended}\r\n",
                    labelled("]"),
                    literal(MESSAGE)
                )
                .as_bytes(),
            );
        script
    }

    fn forty_one_and_forty_two() -> [Uid; 2] {
        [
            Uid::new(41).expect("non-zero"),
            Uid::new(42).expect("non-zero"),
        ]
    }

    #[tokio::test]
    async fn what_arrived_before_a_refusal_is_kept_and_the_refused_one_is_absent() {
        // A server that will not serve one message answers with the rest and then `NO`. The round
        // that reads mail whole asks for ten at a time, newest first, so thrown away together, the
        // one the server never serves keeps the nine beside it from ever being read.
        let script = the_first_of_two_and_then("A0003 NO [SERVERBUG] Internal error");

        let got = whole_messages_on(
            &mut session_against(script).await,
            "INBOX",
            generation(),
            &forty_one_and_forty_two(),
        )
        .await
        .expect("what arrived is read");

        assert_eq!(got, vec![(Uid::new(41).expect("non-zero"), MESSAGE.into())]);
    }

    #[tokio::test]
    async fn what_arrived_before_the_question_was_called_malformed_is_not_kept() {
        // `BAD` refuses the question rather than a message, and vouches for nothing before it.
        let script = the_first_of_two_and_then("A0003 BAD Command syntax error");

        let failure = whole_messages_on(
            &mut session_against(script).await,
            "INBOX",
            generation(),
            &forty_one_and_forty_two(),
        )
        .await
        .expect_err("an answer to a malformed question is no answer");

        assert_eq!(failure, Unread::Failed(MailProviderError::Protocol));
    }

    #[tokio::test]
    async fn a_message_that_arrived_whole_before_a_refusal_is_the_message() {
        let mut script = opened_in(Some(7));
        script.write(b"A0003 UID FETCH 42 (BODY.PEEK[])\r\n").read(
            format!(
                "* 1 FETCH (UID 42 {} {})\r\nA0003 NO [SERVERBUG] Internal error\r\n",
                labelled("]"),
                literal(MESSAGE)
            )
            .as_bytes(),
        );

        let raw = whole_message_on(
            &mut session_against(script).await,
            "INBOX",
            generation(),
            Uid::new(42).expect("non-zero"),
        )
        .await
        .expect("the message arrived whole");

        assert_eq!(raw, MESSAGE.as_bytes());
    }

    #[tokio::test]
    async fn a_flag_said_about_a_message_before_it_arrives_does_not_hide_it() {
        // Somebody marks the message read on a phone while it is being opened, and the server says
        // so first. The line that carries the message is the second line about its number.
        let mut script = opened_in(Some(7));
        script.write(b"A0003 UID FETCH 42 (BODY.PEEK[])\r\n").read(
            format!(
                "* 1 FETCH (UID 42 FLAGS (\\Seen))\r\n* 1 FETCH (UID 42 {} {})\r\n\
                 A0003 OK FETCH completed\r\n",
                labelled("]"),
                literal(MESSAGE)
            )
            .as_bytes(),
        );

        let raw = whole_message_on(
            &mut session_against(script).await,
            "INBOX",
            generation(),
            Uid::new(42).expect("non-zero"),
        )
        .await
        .expect("the message arrived");

        assert_eq!(raw, MESSAGE.as_bytes());
    }

    #[tokio::test]
    async fn a_message_refused_before_it_arrived_is_no_message() {
        let mut script = opened_in(Some(7));
        script
            .write(b"A0003 UID FETCH 42 (BODY.PEEK[])\r\n")
            .read(b"A0003 NO [SERVERBUG] Internal error\r\n");

        let failure = whole_message_on(
            &mut session_against(script).await,
            "INBOX",
            generation(),
            Uid::new(42).expect("non-zero"),
        )
        .await
        .expect_err("nothing arrived");

        assert_eq!(failure, MailProviderError::Protocol);
    }

    #[tokio::test]
    async fn a_message_larger_than_its_size_said_is_read_when_it_is_asked_for_alone() {
        // The round asks for what the server said is no larger than this build reads, and for what
        // it said nothing about. A size can be lower than the message, and refused for being
        // larger, a message alone in its request is asked for again on every round, whole each
        // time, for ever. Read, it is recorded as read and left alone.
        let past = crate::infrastructure::html::LARGEST_MESSAGE + 1;
        let mut line = format!("* 1 FETCH (UID 41 {} {{{past}}}\r\n", labelled("]")).into_bytes();
        line.resize(line.len() + past, b'x');
        line.extend_from_slice(b")\r\nA0003 OK FETCH completed\r\n");

        let mut script = opened_in(Some(7));
        script
            .write(b"A0003 UID FETCH 41 (BODY.PEEK[])\r\n")
            .read(&line);

        let got = whole_messages_on(
            &mut session_against(script).await,
            "INBOX",
            generation(),
            &[Uid::new(41).expect("non-zero")],
        )
        .await
        .expect("one message has the room one opened has");

        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, Uid::new(41).expect("non-zero"));
        assert_eq!(got[0].1.len(), past);
    }

    #[test]
    fn a_round_holds_room_for_each_message_it_asks_for_and_never_less_than_one_opened() {
        // Ten at the largest size this build reads fit to the byte, lines and all, and a request
        // holds no more than that; a request of one has what a person opening that message has.
        use crate::infrastructure::html::LARGEST_MESSAGE;
        use crate::infrastructure::imap::answer::{
            most_whole_messages_hold, Fetched, MOST_A_WHOLE_MESSAGE_HOLDS,
        };

        let a_line = std::mem::size_of::<Fetched>();

        assert_eq!(
            most_whole_messages_hold(10),
            10 * (LARGEST_MESSAGE + a_line)
        );
        assert_eq!(most_whole_messages_hold(1), MOST_A_WHOLE_MESSAGE_HOLDS);
    }
}

/// How much of an answer is held, and what ends one.
mod answers {
    use super::*;
    use crate::infrastructure::imap::answer::{answer_to, mailboxes_listed, Fetched, IfRefused};

    /// What one line costs before anything it carries.
    fn a_line() -> usize {
        std::mem::size_of::<Fetched>()
    }

    /// A session that sends `UID FETCH 1:* (UID FLAGS)` and is answered with `lines`, all at once.
    async fn answered(lines: &str) -> async_imap::Session<tokio_test::io::Mock> {
        let mut script = opening(Builder::new());
        script
            .write(b"A0002 UID FETCH 1:* (UID FLAGS)\r\n")
            .read(lines.as_bytes());

        session_against(script).await
    }

    const THREE: &str = "* 1 FETCH (UID 1 FLAGS ())\r\n\
                         * 2 FETCH (UID 2 FLAGS ())\r\n\
                         * 3 FETCH (UID 3 FLAGS ())\r\n\
                         A0002 OK FETCH completed\r\n";

    #[tokio::test]
    async fn an_answer_within_what_it_may_hold_is_read_to_its_end() {
        let answer = answer_to(
            &mut answered(THREE).await,
            "UID FETCH 1:* (UID FLAGS)",
            3 * a_line(),
            IfRefused::Nothing,
        )
        .await
        .expect("three lines fit in three lines' room");

        assert_eq!(answer.fetched.len(), 3);
    }

    #[tokio::test]
    async fn an_answer_past_what_it_may_hold_is_refused() {
        // A server that keeps answering is refused rather than held, however honest each line
        // looks. Room for two, and a third arrives.
        let failure = answer_to(
            &mut answered(THREE).await,
            "UID FETCH 1:* (UID FLAGS)",
            2 * a_line(),
            IfRefused::Nothing,
        )
        .await
        .expect_err("a third line past room for two");

        assert_eq!(failure, MailProviderError::Protocol);
    }

    #[tokio::test]
    async fn what_a_line_carries_counts_as_well_as_the_line() {
        // The bytes are what a server can make large, so they are what has to be counted. One
        // line with a hundred bytes in it, against room for the line and ninety-nine.
        let hundred = "x".repeat(100);
        let lines = format!(
            "* 1 FETCH (UID 1 RFC822 {})\r\nA0002 OK FETCH completed\r\n",
            literal(&hundred)
        );

        let failure = answer_to(
            &mut answered(&lines).await,
            "UID FETCH 1:* (UID FLAGS)",
            a_line() + 99,
            IfRefused::Nothing,
        )
        .await
        .expect_err("a hundred bytes past room for ninety-nine");
        assert_eq!(failure, MailProviderError::Protocol);

        let answer = answer_to(
            &mut answered(&lines).await,
            "UID FETCH 1:* (UID FLAGS)",
            a_line() + 100,
            IfRefused::Nothing,
        )
        .await
        .expect("a hundred bytes in room for a hundred");
        assert_eq!(answer.fetched[0].whole.as_deref(), Some(hundred.as_bytes()));
    }

    #[tokio::test]
    async fn lines_that_are_not_kept_still_count() {
        // Nothing here is kept, and a server that never stops saying it would otherwise be read
        // for as long as it keeps talking.
        let lines = "* OK still here\r\n* OK still here\r\n* OK still here\r\nA0002 OK done\r\n";

        let failure = answer_to(
            &mut answered(lines).await,
            "UID FETCH 1:* (UID FLAGS)",
            2 * a_line(),
            IfRefused::Nothing,
        )
        .await
        .expect_err("noise past room for two lines");

        assert_eq!(failure, MailProviderError::Protocol);
    }

    /// A session that asks for the account's mailboxes and is answered with `lines`, all at once.
    async fn listed_as(lines: &str) -> async_imap::Session<tokio_test::io::Mock> {
        let mut script = opening(Builder::new());
        script
            .write(b"A0002 LIST \"\" *\r\n")
            .read(lines.as_bytes());

        session_against(script).await
    }

    /// What one thing a listing says about a mailbox costs once it is kept, before its words.
    fn an_attribute() -> usize {
        std::mem::size_of::<async_imap::types::NameAttribute<'static>>()
    }

    /// Three mailboxes, whose names come to sixteen bytes between them, each said to have no
    /// children in fourteen bytes.
    const THREE_MAILBOXES: &str = "* LIST (\\HasNoChildren) \"/\" \"INBOX\"\r\n\
                                   * LIST (\\HasNoChildren) \"/\" \"Archive\"\r\n\
                                   * LIST (\\HasNoChildren) \"/\" \"Sent\"\r\n\
                                   A0002 OK LIST completed\r\n";

    #[tokio::test]
    async fn a_listing_of_mailboxes_is_held_to_what_it_may_hold_name_by_name() {
        // A server that keeps naming mailboxes is read until memory runs out, while the account
        // is being read and while a sent message waits for its folder alike: the names keep
        // arriving, so no silence ends it. Each mailbox is charged its line, its name, and what is
        // said about it.
        let room_for_three =
            3 * (a_line() + an_attribute() + "\\HasNoChildren".len()) + "INBOXArchiveSent".len();

        let listed = mailboxes_listed(&mut listed_as(THREE_MAILBOXES).await, room_for_three)
            .await
            .expect("three mailboxes in room for three");
        assert_eq!(
            listed
                .iter()
                .map(|mailbox| mailbox.name.as_str())
                .collect::<Vec<_>>(),
            vec!["INBOX", "Archive", "Sent"]
        );

        let failure = mailboxes_listed(&mut listed_as(THREE_MAILBOXES).await, room_for_three - 1)
            .await
            .expect_err("a byte past the room for three");
        assert_eq!(failure, MailProviderError::Protocol);
    }

    #[tokio::test]
    async fn what_a_listing_says_about_one_mailbox_counts_as_well_as_its_name() {
        // A line names one mailbox and may say as much about it as a line holds, all of it kept:
        // charged for the name alone, ten thousand such lines are a few hundred megabytes read and
        // gigabytes held. One mailbox, said to be sent mail, and to be something this build has no
        // name for, a thousand times each — against room for its line and its name and a little
        // over.
        let said = ["\\Sent", "\\Xyz"].repeat(1_000).join(" ");
        let lines = format!("* LIST ({said}) \"/\" \"Sent\"\r\nA0002 OK LIST completed\r\n");

        let failure = mailboxes_listed(&mut listed_as(&lines).await, 2 * a_line())
            .await
            .expect_err("two thousand things said past room for a line and a name");
        assert_eq!(failure, MailProviderError::Protocol);

        // Said once each, it fits: a thing it has a name for costs its room, one it has none for
        // that and its words.
        let lines = "* LIST (\\Sent \\Xyz) \"/\" \"Sent\"\r\nA0002 OK LIST completed\r\n";
        let room = a_line() + "Sent".len() + 2 * an_attribute() + "\\Xyz".len();

        let listed = mailboxes_listed(&mut listed_as(lines).await, room)
            .await
            .expect("a mailbox and two things said about it, in room for them");
        assert_eq!(listed.len(), 1);

        let failure = mailboxes_listed(&mut listed_as(lines).await, room - 1)
            .await
            .expect_err("a byte past the room for them");
        assert_eq!(failure, MailProviderError::Protocol);
    }

    #[tokio::test]
    async fn what_a_listing_says_besides_mailboxes_counts_too() {
        // Not kept, and counted all the same: a server saying it is still there for ever is read
        // for ever otherwise.
        let lines = "* OK still here\r\n* OK still here\r\n* OK still here\r\nA0002 OK done\r\n";

        let failure = mailboxes_listed(&mut listed_as(lines).await, 2 * a_line())
            .await
            .expect_err("noise past room for two lines");

        assert_eq!(failure, MailProviderError::Protocol);
    }
}
