// A `VANISHED` report of one range is a report of one range, and these tests are about what this
// build does with an unbounded one. The lint reads `vec![3..=u32::MAX]` as somebody meaning the
// four billion numbers inside it and offers to collect them — which is the allocation the test
// below exists to prove this build refuses. Allowed here, where the single range is the subject,
// and nowhere else.
#![allow(clippy::single_range_in_vec_init)]

use crate::ports::credential::Credential;
use lyonoyl_domain::folder::is_inbox;
use secrecy::SecretString;
use tokio::runtime::Runtime;

use super::corpus_mailbox::holding_the_corpus;
use super::envelope::*;
use super::pass::*;
use super::*;

/// One runtime for every test in this module, as the application has one for every account.
fn shared_runtime() -> &'static Runtime {
    static RUNTIME: std::sync::OnceLock<Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a runtime for the tests")
    })
}

fn account(username: &str, password: &str) -> ImapAccount {
    ImapAccount {
        endpoint: development_endpoint(),
        username: username.to_owned(),
        credential: Credential::password(SecretString::from(password.to_owned())),
    }
}

/// Whether the development mail server is running. Its tests are skipped when it is not, so a
/// checkout without Docker still has a green suite.
fn server_is_running() -> bool {
    crate::dev_mail_server::answering()
}

fn configured_account(host: &str, port: u16, encryption: Encryption) -> MailAccount {
    use lyonoyl_domain::account::{MailAccountSummary, ServerSettings};

    MailAccount::new(
        MailAccountSummary::new("acc-1", "user@example.com", "User", "imap")
            .expect("valid summary"),
        ServerSettings::new(host, port, encryption).expect("valid server"),
        "user@example.com",
    )
    .expect("valid account")
}

#[test]
fn the_settings_a_person_entered_become_the_address_connected_to() {
    let endpoint = endpoint_of(&configured_account(
        "imap.example.com",
        993,
        Encryption::Tls,
    ));

    assert_eq!(endpoint.host, "imap.example.com");
    assert_eq!(endpoint.port, 993);
    assert_eq!(
        endpoint.encryption,
        crate::infrastructure::net::Encryption::Tls
    );
}

#[test]
fn every_encryption_setting_is_carried_through_to_the_connection() {
    // Silently downgrading any of these would send a password in the clear.
    for (chosen, expected) in [
        (Encryption::Tls, crate::infrastructure::net::Encryption::Tls),
        (
            Encryption::StartTls,
            crate::infrastructure::net::Encryption::StartTls,
        ),
        (
            Encryption::None,
            crate::infrastructure::net::Encryption::None,
        ),
    ] {
        let endpoint = endpoint_of(&configured_account("mail.example.com", 143, chosen));
        assert_eq!(endpoint.encryption, expected);
    }
}

/// A header block as a server sends it: lines separated by CRLF, ended by a blank line.
fn header(lines: &[&str]) -> Vec<u8> {
    let mut block = lines.join("\r\n");
    block.push_str("\r\n\r\n");
    block.into_bytes()
}

#[test]
fn a_first_pass_takes_the_newest_messages_rather_than_the_whole_mailbox() {
    // A hundred thousand messages of headers is minutes of transfer before anything appears.
    assert_eq!(fetch_range(None, None, Some(10_000)), "9500:*");
}

#[test]
fn a_first_pass_over_a_small_mailbox_takes_all_of_it() {
    assert_eq!(fetch_range(None, None, Some(12)), "1:*");
    assert_eq!(fetch_range(None, None, Some(1)), "1:*");
    // A server that does not report UIDNEXT leaves nothing to window by.
    assert_eq!(fetch_range(None, None, None), "1:*");
}

#[test]
fn a_later_pass_continues_from_where_the_last_one_stopped() {
    assert_eq!(fetch_range(Some(42), Some(99), Some(100)), "43:99");
}

#[test]
fn a_later_pass_reads_a_window_past_where_the_last_one_stopped_rather_than_all_of_it() {
    // An answer may hold only so much. A folder that gained more mail since its last pass than one
    // answer holds, asked for all of it, is refused on every pass and never read again.
    let waiting: Vec<u32> = (43..=9_999).collect();

    assert!(needs_a_listing(42, 9_999));
    assert_eq!(window_end(42, 9_999, &waiting), 2_042);
    assert_eq!(fetch_range(Some(42), Some(2_042), Some(10_000)), "43:2042");
}

#[test]
fn a_window_is_counted_in_messages_rather_than_in_numbers() {
    // Numbers are spent on mail that has since left: six thousand filed out of an inbox that kept
    // forty, or fifty thousand used by the other folders of a server that numbers an account from
    // one counter. Counted in numbers, the forty would arrive over three passes and the one over
    // twenty-four.
    let forty: Vec<u32> = (0..40).map(|at| 4 + at * 150).collect();
    assert!(needs_a_listing(3, 6_003));
    assert_eq!(window_end(3, 6_003, &forty), 6_003);

    assert_eq!(window_end(3, 49_999, &[48_000]), 49_999);
    assert_eq!(window_end(3, 49_999, &[]), 49_999);
}

#[test]
fn a_window_ends_at_the_last_number_the_server_had_issued() {
    // A number past it may be issued yet, and a position moved over it would pass the message
    // given it. Where the window ends is where a whole answer moves the position to.
    assert!(!needs_a_listing(42, 99));
    assert!(!needs_a_listing(42, 2_042));
    assert_eq!(window_end(42, 99, &[50, 60]), 99);

    // A window's worth exactly is all of it, and reads to the newest number.
    let a_window: Vec<u32> = (43..=2_042).collect();
    assert_eq!(window_end(42, 9_999, &a_window), 9_999);

    // A server that names no next number leaves nothing to end a window at.
    assert_eq!(fetch_range(Some(42), None, None), "43:*");
}

#[test]
fn a_window_counts_only_the_messages_it_asked_about() {
    // A flag changed on another device arrives in the middle of a listing as a line about its
    // message: one already held, or one that came after the mailbox was opened. Counted, they end
    // the window before mail nobody has read, or past the newest number; a line said twice is one
    // message.
    let mut listed: Vec<u32> = (43..=2_041).collect();
    listed.extend([7, 2_041, 50, 2_042, 2_043]);
    assert_eq!(window_end(42, 9_999, &listed), 2_042);

    let mut listed: Vec<u32> = (43..=2_041).collect();
    listed.extend([12_000, 13_000]);
    assert_eq!(window_end(42, 9_999, &listed), 9_999);
}

#[test]
fn a_first_pass_reads_from_its_newest_message_only_where_the_answer_says_it_is_below_the_floor() {
    let described = |uid: u32| answer::Fetched {
        uid: Some(uid),
        header: Some(header(&["Subject: Thirty"])),
        ..answer::Fetched::default()
    };
    let said_in_passing = |uid: u32| answer::Fetched {
        uid: Some(uid),
        flags: Some(Flags::SEEN),
        ..answer::Fetched::default()
    };
    let floor = Uid::new(500).expect("non-zero");
    let at = |uid: u32| Uid::new(uid).expect("non-zero");

    // `500:*` described the newest message, and it is below the floor: nothing between it and the
    // floor is mail.
    assert_eq!(first_floor(floor, &[described(30)], describes), at(30));
    // A flag changed on another device is said about any message at all, and says nothing about
    // the numbers around it.
    assert_eq!(first_floor(floor, &[said_in_passing(30)], describes), floor);
    assert_eq!(first_floor(floor, &[], describes), floor);
    // Mail at or above the floor is what `*` reached, and nothing below the floor was asked for.
    assert_eq!(
        first_floor(floor, &[described(600), described(30)], describes),
        floor
    );
    assert_eq!(
        first_floor(floor, &[described(30), described(500)], describes),
        floor
    );
    // Two below it, which an honest server never sends: the newer is the most the answer vouches
    // for, and a floor lowered past the older would leave the numbers between them unread for good.
    assert_eq!(
        first_floor(floor, &[described(10), described(30)], describes),
        at(30)
    );
}

#[test]
fn a_first_pass_reads_from_its_newest_message_where_the_server_refused_to_describe_it() {
    // Refused, the newest message is not described, and the facts the server lists about what it
    // would not describe are what say it is there: its size and its date. A flag said in passing
    // states neither, and a line of the answer that only described a message is not one of them.
    let stated = |uid: u32| answer::Fetched {
        uid: Some(uid),
        flags: Some(Flags::NONE),
        size: Some(4096),
        internal_date: Some(837_596_665),
        ..answer::Fetched::default()
    };
    let said_in_passing = |uid: u32| answer::Fetched {
        uid: Some(uid),
        flags: Some(Flags::SEEN),
        ..answer::Fetched::default()
    };
    let floor = Uid::new(500).expect("non-zero");

    assert_eq!(
        first_floor(floor, &[stated(30)], states_facts),
        Uid::new(30).expect("non-zero")
    );
    assert_eq!(
        first_floor(floor, &[said_in_passing(30)], states_facts),
        floor
    );
    assert_eq!(
        first_floor(floor, &[stated(600), stated(30)], states_facts),
        floor
    );
    assert!(!describes(&stated(30)), "its facts are not a description");
}

#[test]
fn the_last_message_of_a_folder_is_not_stored_again_on_every_pass() {
    // `UID FETCH n:*` is read as "between n and the highest UID", so a server with nothing new
    // answers with its last message rather than with nothing. Without this the newest message
    // would be re-stored by every pass for as long as the folder stays quiet.
    assert!(!is_new(42, Some(42)), "the message the cursor points at");
    assert!(!is_new(7, Some(42)), "anything older than it");
    assert!(is_new(43, Some(42)));
    assert!(
        is_new(1, None),
        "a first pass has nothing to compare against"
    );
}

fn request_at(cursor: Option<&str>, stored_generation: Option<u32>) -> SyncRequest {
    SyncRequest {
        folder_id: lyonoyl_domain::message::FolderId::new(1).expect("non-zero"),
        intent: SyncIntent::Forward,
        mailbox: "INBOX".to_owned(),
        cursor: cursor.map(|value| SyncCursor::new(value.to_owned())),
        uid_validity: stored_generation.and_then(|value| UidValidity::new(value).ok()),
        known_window: None,
        highest_modseq: None,
        known_count: 0,
        refused_for_now_last_time: false,
    }
}

fn generation(value: u32) -> UidValidity {
    UidValidity::new(value).expect("non-zero")
}

#[test]
fn a_pass_continues_from_the_stored_position_while_the_numbering_holds() {
    assert_eq!(
        resume_from(&request_at(Some("42"), Some(7)), generation(7), Some(500)),
        Some(42)
    );
}

#[test]
fn a_renumbered_mailbox_is_read_from_the_beginning_again() {
    // The stored cursor counts in a numbering the server has thrown away. Continuing from it
    // asks for messages above a boundary that means nothing, so the pass brings back nothing —
    // and the folder it has just emptied stays empty, with a cursor that keeps it that way.
    assert_eq!(
        resume_from(
            &request_at(Some("81080"), Some(7)),
            generation(1_787_365_322),
            Some(500)
        ),
        None
    );
}

#[test]
fn a_folder_never_synced_before_has_nothing_to_continue_from() {
    assert_eq!(
        resume_from(&request_at(None, None), generation(7), Some(500)),
        None
    );
    // A stored cursor with no stored generation cannot be checked, so it is trusted: it was
    // written by a pass over this same folder.
    assert_eq!(
        resume_from(&request_at(Some("42"), None), generation(7), Some(500)),
        Some(42)
    );
}

#[test]
fn a_position_the_mailbox_cannot_hold_is_not_continued_from() {
    // A cursor at or above UIDNEXT names a message that does not exist. A server that
    // renumbered without changing its generation leaves exactly this, and a folder that
    // continues from it is never read again.
    assert_eq!(
        resume_from(
            &request_at(Some("81080"), Some(7)),
            generation(7),
            Some(205)
        ),
        None
    );
    // The highest UID a mailbox holds is one below UIDNEXT, and that is a normal place to be.
    assert_eq!(
        resume_from(&request_at(Some("204"), Some(7)), generation(7), Some(205)),
        Some(204)
    );
}

fn window(low: u32, high: u32) -> RangeInclusive<Uid> {
    Uid::new(low).expect("non-zero")..=Uid::new(high).expect("non-zero")
}

fn request_with_modseq(applied: Option<u64>) -> SyncRequest {
    SyncRequest {
        highest_modseq: applied.map(ModSeq::new),
        ..request_at(Some("42"), Some(7))
    }
}

#[test]
fn a_folder_whose_sequence_moved_is_asked_what_changed() {
    assert_eq!(
        changed_since(&request_with_modseq(Some(12)), Some(20)),
        Some(12)
    );
    assert_eq!(
        changed_since_query(12, false),
        "(UID FLAGS) (CHANGEDSINCE 12)"
    );
}

#[test]
fn a_server_that_took_the_extension_is_also_asked_what_went() {
    assert_eq!(
        changed_since_query(12, true),
        "(UID FLAGS) (CHANGEDSINCE 12 VANISHED)"
    );
}

#[test]
fn a_named_range_becomes_the_numbers_in_it() {
    let ranges = vec![2..=2, 4..=6];

    assert_eq!(
        listed_uids(&ranges, Some(10)).expect("a short report is held"),
        [2, 4, 5, 6].map(|value| Uid::new(value).expect("non-zero"))
    );
}

#[test]
fn an_open_range_ends_at_the_newest_uid_the_mailbox_can_hold() {
    // `VANISHED 3:*` says "everything from three onwards", and a server saying it has claimed
    // nothing about numbers it never issued. Read as written it is four billion of them.
    let ranges = vec![3..=u32::MAX];

    assert_eq!(
        listed_uids(&ranges, Some(6)).expect("a bounded report is held"),
        [3, 4, 5].map(|value| Uid::new(value).expect("non-zero"))
    );
}

#[test]
fn a_report_larger_than_this_build_holds_is_refused_rather_than_expanded() {
    // Refused means the pass falls back to counting, which asks before removing a mailbox's
    // worth of mail. Expanding it means allocating the UID space.
    let ranges = vec![1..=u32::MAX];

    assert_eq!(listed_uids(&ranges, None), None);
}

#[test]
fn a_range_that_ends_below_where_it_starts_names_nothing() {
    // What clamping an open range to a mailbox smaller than its start leaves behind.
    let ranges = vec![9..=u32::MAX];

    assert_eq!(
        listed_uids(&ranges, Some(4)).expect("the report is held"),
        Vec::new()
    );
}

#[test]
fn a_folder_where_nothing_moved_is_not_asked_at_all() {
    // The common case. A pass over a quiet account should cost one command per folder, not a
    // listing of it.
    assert_eq!(
        changed_since(&request_with_modseq(Some(20)), Some(20)),
        None
    );
}

#[test]
fn a_server_that_keeps_no_sequences_is_not_asked_either() {
    // The question means nothing to it, and asking would be a command it answers with an error.
    assert_eq!(changed_since(&request_with_modseq(Some(12)), None), None);
}

#[test]
fn a_folder_with_nothing_applied_has_no_position_to_ask_from() {
    assert_eq!(changed_since(&request_with_modseq(None), Some(20)), None);
}

#[test]
fn a_sequence_behind_the_server_is_never_asked_forward() {
    // A stored sequence above the server's means the mailbox was rebuilt underneath us; asking
    // from there would ask about a future that does not exist.
    assert_eq!(
        changed_since(&request_with_modseq(Some(99)), Some(20)),
        None
    );
}

fn listing_for(
    known: Option<RangeInclusive<Uid>>,
    stored_generation: Option<u32>,
    applied_modseq: Option<u64>,
    missing: bool,
    reported: u32,
) -> Option<RangeInclusive<Uid>> {
    let request = SyncRequest {
        known_window: known,
        highest_modseq: applied_modseq.map(ModSeq::new),
        ..request_at(Some("42"), stored_generation)
    };

    listing_range(&request, generation(reported), missing, Some(10_001))
}

#[test]
fn a_mailbox_that_lost_nothing_is_not_listed_at_all() {
    // The common case, and the whole point of asking the count first: the flags came from
    // asking what changed, and a listing would say nothing a pass does not already know.
    assert_eq!(
        listing_for(Some(window(1, 10_000)), Some(7), Some(42), false, 7),
        None
    );
}

#[test]
fn a_mailbox_that_lost_something_is_listed_whole() {
    // What went is anywhere in it. A window would find only what went recently, and leave the
    // rest showing messages that are no longer there.
    assert_eq!(
        listing_for(Some(window(500, 10_000)), Some(7), Some(42), true, 7),
        Some(window(1, 10_000))
    );
}

#[test]
fn a_server_that_keeps_no_sequences_is_asked_about_its_newest_window() {
    // Without them there is no way to see a flag change except to look.
    assert_eq!(
        listing_for(Some(window(1, 10_000)), Some(7), None, false, 7),
        Some(window(9_501, 10_000))
    );
}

#[test]
fn a_small_folder_without_sequences_is_looked_at_whole() {
    assert_eq!(
        listing_for(Some(window(3, 20)), Some(7), None, false, 7),
        Some(window(3, 20))
    );
}

#[test]
fn a_folder_holding_nothing_is_not_listed() {
    assert_eq!(listing_for(None, Some(7), None, false, 7), None);
    assert_eq!(listing_for(None, Some(7), None, true, 7), None);
}

#[test]
fn a_renumbered_mailbox_is_never_listed() {
    // Those UIDs address a numbering the server threw away. A listing of them comes back empty
    // and reads as "every message you hold has disappeared" — which, with the removal cap out
    // of the way, is how a mailbox gets emptied locally.
    assert_eq!(
        listing_for(Some(window(1, 10_000)), Some(7), Some(42), true, 999),
        None
    );
}

#[test]
fn a_mailbox_holding_fewer_messages_than_the_folder_accounts_for_has_lost_some() {
    assert!(vanished_since(9, 10, 0, false));
    assert!(vanished_since(9, 5, 5, false));
}

#[test]
fn a_mailbox_holding_as_many_as_expected_has_lost_none() {
    assert!(!vanished_since(10, 10, 0, false));
    assert!(!vanished_since(10, 5, 5, false));
}

#[test]
fn a_mailbox_holding_more_than_has_been_read_is_not_a_loss() {
    // A first pass takes a window; what is older than it has simply not been read yet.
    assert!(!vanished_since(10_000, 500, 0, false));
}

#[test]
fn a_pass_reading_further_back_is_not_asked_the_question() {
    // It is filling in history, and the counts are meant to disagree while it does.
    assert!(!vanished_since(9, 10, 0, true));
}

#[test]
fn what_the_audit_saw_is_stated_as_a_listing_of_the_range_it_covered() {
    let report = listed(&window(10, 20), &[]);

    match report {
        VanishedReport::AuthoritativeSet {
            window: covered,
            present,
        } => {
            assert_eq!(covered, window(10, 20));
            assert!(present.is_empty());
        }
        other => panic!("a listing must be authoritative about its own range: {other:?}"),
    }
}

#[test]
fn an_encoded_subject_is_read_as_the_words_it_stands_for() {
    let block = header(&[
        "Subject: =?UTF-8?B?zpbOrs+EzrEsIM6azqzPgM+AzrE=?=",
        "From: Zeta <zeta@example.com>",
        "Message-ID: <abc@example.com>",
    ]);

    let envelope = envelope_of(&block, b"", Some(1_700_000_000), Some(2048), Flags::SEEN);

    assert_eq!(envelope.subject.as_deref(), Some("Ζήτα, Κάππα"));
    assert_eq!(envelope.sender.as_deref(), Some("Zeta"));
    // The name is what a list shows; the address is the only one of the two an answer can be
    // sent to, and reading it back out of the name later is not possible.
    assert_eq!(envelope.sender_address.as_deref(), Some("zeta@example.com"));
    assert_eq!(
        envelope.rfc822_message_id.as_ref().map(|id| id.as_str()),
        Some("abc@example.com")
    );
    assert_eq!(envelope.received_at, 1_700_000_000);
    assert_eq!(envelope.rfc822_size, Some(2048));
    assert!(envelope.flags.contains(Flags::SEEN));
}

#[test]
fn a_message_written_only_in_html_still_gets_a_preview_and_it_is_text() {
    // Pinned because the comment above `preview_of` claimed the opposite for a long time, and a
    // sentence in this repository is what the next person works from. What the list shows is
    // the words, never the markup — and the markup is parsed here, on a two-kilobyte prefix.
    let envelope = envelope_of(
        &header(&[
            "From: zeta@example.com",
            "Content-Type: text/html; charset=utf-8",
        ]),
        b"<p>Shall we meet on <b>Tuesday</b>?</p>",
        Some(1),
        None,
        Flags::NONE,
    );

    let preview = envelope
        .preview
        .expect("an HTML-only message has a preview");
    assert!(
        preview.contains("Shall we meet on"),
        "the preview does not carry the words: {preview:?}"
    );
    assert!(
        !preview.contains('<'),
        "the preview is made of markup: {preview:?}"
    );
}

#[test]
fn a_message_written_in_base64_shows_its_words_and_not_its_encoding() {
    // A body arrives as a prefix -- `BODY.PEEK[TEXT]<0.2048>` -- so its last part is unfinished:
    // the boundary that would end it is past the cut. A parser that cannot classify an unfinished
    // part cannot apply its encoding either, and hands the characters back as they came, so the
    // list showed a wall of base64 under the subject. Quoted-printable had this salvage from the
    // start and base64 never did.
    let envelope = envelope_of(
        &header(&[
            "From: zeta@example.com",
            "Content-Type: multipart/alternative; boundary=\"alt\"",
        ]),
        b"--alt\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\nSGkgWmV0YSwNCg0KSnVzdCBjaGVja2luZyBpbiAtIGFyZSB5b3Ugc3RpbGwgcGxhbm5pbmcgdG8g\r\ndXNlIEV4YW1wbGUgUGF5IGZvciBjcm9zcy1ib3JkZXIgcGF5bWVudHM/IFRvIGdldCBzdGFydGVk\r\nLCBzaW1wbHkgdXBsb2FkIHlvdXIgZG9jdW1lbnRzIHRvIGFjdGl2YXRlIHlvdXIgbXVsdGktY3Vy\r\ncmVuY3kgYWNjb3VudHMuIFdlIGFyZSBoZXJlIHRvIGhlbHAgeW91IHNhdmUgdXAgdG8gNzUgcGVy\r\nY2VudCBvbiBpbnRlcm5hdGlvbmFsIGZlZXMsIGFuZCBhIG1lbWJlciBvZiBvdXIgdGVhbSB3aWxs\r\nIGdldCBiYWNrIHRvIHlvdSBzaG9ydGx5IHdpdGggdGhlIG5leHQgc3RlcHMgZm9yIHlvdXIgYWNj\r\nb3VudC4NCkhpIEhpIFpldGEsDQoNCkp1c3QgY2hlY2tpbmcgaW4gLSBhcmUgeW91IHN0aWxsIHBs\r\nYW5uaW5nIHRvIHVzZSBFeGFtcGxlIFBheSBmb3IgY3Jvc3MtYm9yZGVyIHBheW1lbnRzPyBUbyBn\r\nZXQgc3RhcnRlZCwgc2ltcGx5IHVwbG9hZCB5b3VyIGRvY3VtZW50cyB0byBhY3RpdmF0ZSB5b3Vy\r\nIG11bHRpLWN1cnJlbmN5IGFjY291bnRzLiBXZSBhcmUgaGVyZSB0byBoZWxwIHlvdSBzYXZlIHVw\r\nIHRvIDc1IHBlcmNlbnQgb24gaW50ZXJuYXRpb25hbCBmZWVzLCBhbmQgYSBtZW1iZXIgb2Ygb3Vy\r\nIHRlYW0gd2lsbCBnZXQgYmFjayB0byB5b3Ugc2hvcnRseSB3aXRoIHRoZSBuZXh0IHN0ZXBzIGZv\r\nciB5b3VyIGFjY291bnQuDQpIaSBIaSBaZXRhLA0KDQpKdXN0IGNoZWNraW5nIGluIC0gYXJlIHlv\r\ndSBzdGlsbCBwbGFubmluZyB0byB1c2UgRXhhbXBsZSBQYXkgZm9yIGNyb3NzLWJvcmRlciBwYXlt\r\nZW50cz8gVG8gZ2V0IHN0YXJ0ZWQsIHNpbXBseSB1cGxvYWQgeW91ciBkb2N1bWVudHMgdG8gYWN0\r\naXZhdGUgeW91ciBtdWx0aS1jdXJyZW5jeSBhY2NvdW50cy4gV2UgYXJlIGhlcmUgdG8gaGVscCB5\r\nb3Ugc2F2ZSB1cCB0byA3NSBwZXJjZW50IG9uIGludGVybmF0aW9uYWwgZmVlcywgYW5kIGEgbWVt\r\nYmVyIG9mIG91ciB0ZWFtIHdpbGwgZ2V0IGJhY2sgdG8geW91IHNob3J0bHkgd2l0aCB0aGUgbmV4\r\ndCBzdGVwcyBmb3IgeW91ciBhY2NvdW50Lg0KSGkgSGkgWmV0YSwNCg0KSnVzdCBjaGVja2luZyBp\r\nbiAtIGFyZSB5b3Ugc3RpbGwgcGxhbm5pbmcgdG8gdXNlIEV4YW1wbGUgUGF5IGZvciBjcm9zcy1i\r\nb3JkZXIgcGF5bWVudHM/IFRvIGdldCBzdGFydGVkLCBzaW1wbHkgdXBsb2FkIHlvdXIgZG9jdW1l\r\nbnRzIHRvIGFjdGl2YXRlIHlvdXIgbXVsdGktY3VycmVuY3kgYWNjb3VudHMuIFdlIGFyZSBoZXJl\r\nIHRvIGhlbHAgeW91IHNhdmUgdXAgdG8gNzUgcGVyY2VudCBvbiBpbnRlcm5hdGlvbmFsIGZlZXMs\r\nIGFuZCBhIG1lbWJlciBvZiBvdXIgdGVhbSB3aWxsIGdldCBiYWNrIHRvIHlvdSBzaG9ydGx5IHdp\r\ndGggdGhlIG5leHQgc3RlcHMgZm9yIHlvdXIgYWNjb3VudC4NCkhpIEhpIFpldGEsDQoNCkp1c3Qg\r\nY2hlY2tpbmcga",
        Some(1),
        None,
        Flags::NONE,
    );

    let preview = envelope.preview.expect("a base64 message has a preview");
    assert!(
        preview.contains("Just checking in"),
        "the preview does not carry the words: {preview:?}"
    );
    assert!(
        !preview.starts_with("SGkg"),
        "the preview is the encoding rather than the message: {preview:?}"
    );
}

#[test]
fn markup_sent_as_base64_shows_its_words_and_not_its_tags() {
    // The other half of the same fault. Undoing the encoding is not enough on its own: what comes
    // out is markup, the refusal at the end drops it, and the row is blank rather than wrong --
    // which is quieter and no more use to anybody.
    let envelope = envelope_of(
        &header(&[
            "From: zeta@example.com",
            "Content-Type: multipart/alternative; boundary=\"alt\"",
        ]),
        b"--alt\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\nPGh0bWw+PGJvZHk+PHA+SGkgWmV0YSw8L3A+PHA+VGhhbmsgeW91IGZvciBwcm92aWRpbmcgeW91\r\nciBkZXRhaWxzIGZvciB2ZXJpZmljYXRpb24uIEFmdGVyIGEgdGhvcm91Z2ggcmV2aWV3LCB3ZSBy\r\nZWdyZXQgdG8gaW5mb3JtIHlvdSB0aGF0IHlvdXIgYWNjb3VudCB3YXMgbm90IGFwcHJvdmVkLjwv\r\ncD48cD5XZSB3aXNoIHlvdSBldmVyeSBzdWNjZXNzIGluIHlvdXIgZnV0dXJlIGVuZGVhdm9ycy48\r\nL3A+PC9ib2R5PjwvaHRtbD48aHRtbD48Ym9keT48cD5IaSBaZXRhLDwvcD48cD5UaGFuayB5b3Ug\r\nZm9yIHByb3ZpZGluZyB5b3VyIGRldGFpbHMgZm9yIHZlcmlmaWNhdGlvbi4gQWZ0ZXIgYSB0aG9y\r\nb3VnaCByZXZpZXcsIHdlIHJlZ3JldCB0byBpbmZvcm0geW91IHRoYXQgeW91ciBhY2NvdW50IHdh\r\ncyBub3QgYXBwcm92ZWQuPC9wPjxwPldlIHdpc2ggeW91IGV2ZXJ5IHN1Y2Nlc3MgaW4geW91ciBm\r\ndXR1cmUgZW5kZWF2b3JzLjwvcD48L2JvZHk+PC9odG1sPjxodG1sPjxib2R5PjxwPkhpIFpldGEs\r\nPC9wPjxwPlRoYW5rIHlvdSBmb3IgcHJvdmlkaW5nIHlvdXIgZGV0YWlscyBmb3IgdmVyaWZpY2F0\r\naW9uLiBBZnRlciBhIHRob3JvdWdoIHJldmlldywgd2UgcmVncmV0IHRvIGluZm9ybSB5b3UgdGhh\r\ndCB5b3VyIGFjY291bnQgd2FzIG5vdCBhcHByb3ZlZC48L3A+PHA+V2Ugd2lzaCB5b3UgZXZlcnkg\r\nc3VjY2VzcyBpbiB5b3VyIGZ1dHVyZSBlbmRlYXZvcnMuPC9wPjwvYm9keT48L2h0bWw+PGh0bWw+\r\nPGJvZHk+PHA+SGkgWmV0YSw8L3A+PHA+VGhhbmsgeW91IGZvciBwcm92aWRpbmcgeW91ciBkZXRh\r\naWxzIGZvciB2ZXJpZmljYXRpb24uIEFmdGVyIGEgdGhvcm91Z2ggcmV2aWV3LCB3ZSByZWdyZXQg\r\ndG8gaW5mb3JtIHlvdSB0aGF0IHlvdXIgYWNjb3VudCB3YXMgbm90IGFwcHJvdmVkLjwvcD48cD5X\r\nZSB3aXNoIHlvdSBldmVyeSBzdWNjZXNzIGluIHlvdXIgZnV0dXJlIGVuZGVhdm9ycy48L3A+PC9i\r\nb2R5PjwvaHRtbD4=",
        Some(1),
        None,
        Flags::NONE,
    );

    let preview = envelope
        .preview
        .expect("markup sent as base64 has a preview");
    assert!(
        preview.contains("Thank you for providing"),
        "the preview does not carry the words: {preview:?}"
    );
    assert!(
        !preview.contains('<') && !preview.starts_with("PGh0"),
        "the preview is the markup or its encoding: {preview:?}"
    );
}

#[test]
fn who_a_message_is_from_is_a_name_to_show_and_an_address_to_answer() {
    // Every ordinary `From` carries both, and they are not interchangeable: a reply addressed
    // to "Zeta Quorin" cannot be sent at all, and one addressed to a display name written to
    // look like an address is sent somewhere the message never came from.
    let named = envelope_of(
        &header(&["From: Zeta Quorin <zeta@example.com>"]),
        b"",
        Some(1),
        None,
        Flags::NONE,
    );
    assert_eq!(named.sender.as_deref(), Some("Zeta Quorin"));
    assert_eq!(named.sender_address.as_deref(), Some("zeta@example.com"));

    // No name of its own: the list falls back to the address, and so the two agree.
    let bare = envelope_of(
        &header(&["From: kappa@example.net"]),
        b"",
        Some(1),
        None,
        Flags::NONE,
    );
    assert_eq!(bare.sender.as_deref(), Some("kappa@example.net"));
    assert_eq!(bare.sender_address.as_deref(), Some("kappa@example.net"));

    // A display name shaped like an address stays a display name, and the answer still goes
    // where the message came from.
    let disguised = envelope_of(
        &header(&["From: \"billing@bank.example\" <bounce@other.example>"]),
        b"",
        Some(1),
        None,
        Flags::NONE,
    );
    assert_eq!(disguised.sender.as_deref(), Some("billing@bank.example"));
    assert_eq!(
        disguised.sender_address.as_deref(),
        Some("bounce@other.example")
    );
}

#[test]
fn who_else_a_message_was_addressed_to_is_read_as_addresses() {
    // What an answer to everybody is built from. Names are not answerable and are left out;
    // the order is the one the message wrote, because that is the order the room reads.
    let envelope = envelope_of(
        &header(&[
            "From: Zeta <zeta@example.com>",
            "To: You <you@example.com>, Kappa Vandel <kappa@example.net>",
            "Cc: sigma@example.org",
        ]),
        b"",
        Some(1),
        None,
        Flags::NONE,
    );

    assert_eq!(envelope.to, vec!["you@example.com", "kappa@example.net"]);
    assert_eq!(envelope.cc, vec!["sigma@example.org"]);
}

#[test]
fn a_message_addressed_to_nobody_readable_carries_nobody() {
    // A group is a name for a list whose members the header does not carry, so there is
    // nothing in it to answer; a message with no `To` at all reached the person another way.
    let grouped = envelope_of(
        &header(&["From: zeta@example.com", "To: the team:;"]),
        b"",
        Some(1),
        None,
        Flags::NONE,
    );
    assert!(grouped.to.is_empty());

    let silent = envelope_of(
        &header(&["From: zeta@example.com"]),
        b"",
        Some(1),
        None,
        Flags::NONE,
    );
    assert!(silent.to.is_empty());
    assert!(silent.cc.is_empty());
}

#[test]
fn the_same_person_named_twice_in_one_header_is_carried_once() {
    // Otherwise an answer to everybody sends them two copies of it.
    let envelope = envelope_of(
        &header(&[
            "From: zeta@example.com",
            "To: kappa@example.net, Kappa <kappa@example.net>, you@example.com",
        ]),
        b"",
        Some(1),
        None,
        Flags::NONE,
    );

    assert_eq!(envelope.to, vec!["kappa@example.net", "you@example.com"]);
}

#[test]
fn a_subject_in_a_legacy_charset_is_read_too() {
    // Mail in windows-1251 and KOI8-R is still sent, and a client that shows it as mojibake is
    // not usable in the languages that send it.
    let block = header(&["Subject: =?windows-1251?Q?=CF=F0=E8=E2=E5=F2?="]);

    let envelope = envelope_of(&block, b"", Some(0), None, Flags::NONE);

    assert_eq!(envelope.subject.as_deref(), Some("Привет"));
}

#[test]
fn the_opening_of_a_plain_message_becomes_its_preview() {
    let block = header(&[
        "Subject: Weekly report",
        "Content-Type: text/plain; charset=utf-8",
    ]);
    let body = b"Numbers are up.\r\nDetails inside.\r\n";

    let envelope = envelope_of(&block, body, Some(0), None, Flags::NONE);

    assert_eq!(
        envelope.preview.as_deref(),
        Some("Numbers are up. Details inside."),
        "wrapping is the list's business, not the message's"
    );
}

#[test]
fn a_preview_is_decoded_from_the_encoding_it_arrived_in() {
    let block = header(&[
        "Content-Type: text/plain; charset=windows-1251",
        "Content-Transfer-Encoding: quoted-printable",
    ]);
    let body = b"=CF=F0=E8=E2=E5=F2, =EC=E8=F0";

    let envelope = envelope_of(&block, body, Some(0), None, Flags::NONE);

    assert_eq!(envelope.preview.as_deref(), Some("Привет, мир"));
}

#[test]
fn the_text_part_of_a_message_with_two_is_the_one_used() {
    let block = header(&["Content-Type: multipart/alternative; boundary=\"edge\""]);
    let body = b"--edge\r\n\
Content-Type: text/plain\r\n\r\n\
The words.\r\n\
--edge\r\n\
Content-Type: text/html\r\n\r\n\
<p>The markup.</p>\r\n\
--edge--\r\n";

    let envelope = envelope_of(&block, body, Some(0), None, Flags::NONE);

    assert_eq!(envelope.preview.as_deref(), Some("The words."));
}

#[test]
fn markup_a_parser_took_for_plain_text_is_shown_as_its_words() {
    // What a truncated message does to a parser: the unfinished part cannot be classified, so its
    // bytes come back as plain text. This is exactly what reached the list, and for a long time the
    // answer was to show nothing — a line of tags under a subject being worse than a blank row.
    // It is turned into its words now instead, by the same `html_to_text` the parser would have
    // used had it been able to finish the part, on the same two-kilobyte prefix. Better than the
    // blank row, and the tags still never reach the screen.
    let block = header(&["Content-Type: text/plain; charset=utf-8"]);
    let body = concat!(
        "<p>Hello,</p><p>Thank you for your response. Can you please provide and ",
        "elaborate upon:</p><p></p><ul class=\"tight\" data-tight=\"true\"><li><p>Thank you ",
        "for confirming that the address is correct.</p></li></ul>"
    );

    let preview = envelope_of(&block, body.as_bytes(), Some(0), None, Flags::NONE)
        .preview
        .expect("markup taken for plain text still has words in it");

    assert!(
        preview.contains("Thank you for your response"),
        "the preview does not carry the words: {preview:?}"
    );
    assert!(
        !preview.contains('<') && !preview.contains("tight"),
        "a line of tags reached the subject: {preview:?}"
    );
}

#[test]
fn a_part_that_arrived_still_encoded_is_decoded_anyway() {
    // What a truncated message does: the unfinished part keeps its encoding, and `=E2=80=99`
    // under a subject is not something anyone wrote.
    let block = header(&["Content-Type: text/plain; charset=utf-8"]);
    let body = b"Rescheduled. I=E2=80=99ll be back later for this. Thank =\r\nyou for waiting.";

    let preview = envelope_of(&block, body, Some(0), None, Flags::NONE)
        .preview
        .expect("there is text");

    assert!(!preview.contains("=E2"), "still encoded: {preview}");
    assert!(!preview.contains("= "), "a fold was left in: {preview}");
    assert!(preview.contains("I’ll be back later"), "{preview}");
    assert!(preview.contains("Thank you for waiting"), "{preview}");
}

#[test]
fn ordinary_words_come_back_exactly_as_they_were_written() {
    // The decoder runs over every preview, so what it does to text nobody encoded matters as
    // much as what it does to text somebody did.
    let block = header(&["Content-Type: text/plain; charset=utf-8"]);

    for written in [
        "The rule is a = b + c, which holds everywhere.",
        "Totals: 40% done, 60% left. See §4 — and the note below.",
        "Ζήτα, Κάππα, Σίγμα — 14:00, as agreed.",
        "Path: C:\\Users\\example — mind the backslashes.",
    ] {
        assert_eq!(
            envelope_of(&block, written.as_bytes(), Some(0), None, Flags::NONE)
                .preview
                .as_deref(),
            Some(written)
        );
    }
}

#[test]
fn an_address_in_angle_brackets_is_not_mistaken_for_markup() {
    let block = header(&["Content-Type: text/plain; charset=utf-8"]);
    let body = b"Please write to Zeta Quorin <zeta@example.com> about it.";

    let preview = envelope_of(&block, body, Some(0), None, Flags::NONE)
        .preview
        .expect("this is a person writing, not markup");

    assert!(preview.contains("zeta@example.com"));
}

#[test]
fn a_greater_than_sign_in_the_middle_of_a_sentence_is_not_a_quotation() {
    let block = header(&["Content-Type: text/plain; charset=utf-8"]);
    let body = b"The threshold is 5 > 3, so we proceed.";

    assert_eq!(
        envelope_of(&block, body, Some(0), None, Flags::NONE)
            .preview
            .as_deref(),
        Some("The threshold is 5 > 3, so we proceed.")
    );
}

#[test]
fn a_reply_with_nothing_new_in_it_still_shows_something() {
    // Someone forwarding without a word of their own: the quotation is all there is.
    let block = header(&["Content-Type: text/plain; charset=utf-8"]);
    let body = b"> On 15 Jul 2026, Zeta wrote:\r\n> Could you confirm the meeting?";

    let preview = envelope_of(&block, body, Some(0), None, Flags::NONE)
        .preview
        .expect("a preview of something beats a preview of nothing");

    assert!(preview.contains("Could you confirm"));
}

#[test]
fn a_message_written_only_in_markup_gets_no_preview() {
    // Turning HTML into text means parsing HTML. That is done in one place, under a sandbox,
    // and not for the sake of a line in a list.
    let block = header(&["Content-Type: text/html; charset=utf-8"]);
    let body = b"<p onclick=\"steal()\">Click me</p>";

    let preview = envelope_of(&block, body, Some(0), None, Flags::NONE).preview;

    assert!(
        preview.is_none() || !preview.as_deref().unwrap_or_default().contains('<'),
        "markup must not reach the list: {preview:?}"
    );
}

#[test]
fn a_preview_is_bounded_however_long_the_message_is() {
    let block = header(&["Content-Type: text/plain"]);
    let body = "word ".repeat(2_000);

    let preview = envelope_of(&block, body.as_bytes(), Some(0), None, Flags::NONE)
        .preview
        .expect("there is text");

    assert!(preview.chars().count() <= PREVIEW_CHARS);
}

#[test]
fn a_multipart_message_cut_short_is_still_read_as_words() {
    // Every pass takes a prefix of the body, so a message of any size arrives with its last
    // part unfinished and its closing boundary missing. A parser that cannot close the
    // structure gives back the raw part instead — quoted-printable escapes, soft line breaks
    // and all — and those are what end up under the subject.
    let block = header(&[
        "Content-Type: multipart/alternative; boundary=\"edge\"",
        "Subject: Re: Getting started",
    ]);
    let body = [
        "--edge",
        "Content-Type: text/plain; charset=utf-8",
        "Content-Transfer-Encoding: quoted-printable",
        "",
        "Hi Kappa, I booked a time for a meeting but right now received a =",
        "message about cancelation. I=E2=80=99ll rebook it later.",
        "",
        "--edge",
        "Content-Type: text/html; charset=utf-8",
        "",
        "<p>Hi Kappa,",
    ]
    .join("\r\n");

    let preview = envelope_of(&block, body.as_bytes(), Some(0), None, Flags::NONE)
        .preview
        .expect("there is text in it");

    assert!(
        !preview.contains("=E2") && !preview.contains("= "),
        "quoted-printable reached the list: {preview}"
    );
    assert!(!preview.contains('<'), "markup reached the list: {preview}");
    assert!(
        preview.contains("I’ll rebook it later"),
        "the words are what should be there: {preview}"
    );
}

#[test]
fn what_someone_replied_is_the_preview_not_what_they_replied_to() {
    let block = header(&["Content-Type: text/plain; charset=utf-8"]);
    let body = [
        "Hello Zeta, awaiting your admit.",
        "",
        "> On 15 Jul 2026, at 18:15, Zeta Quorin <zeta@example.com> wrote:",
        "> Could you confirm the meeting?",
    ]
    .join("\r\n");

    let preview = envelope_of(&block, body.as_bytes(), Some(0), None, Flags::NONE)
        .preview
        .expect("there is text");

    assert_eq!(preview, "Hello Zeta, awaiting your admit.");
}

#[test]
fn a_body_cut_in_the_middle_is_not_a_failure() {
    // Every pass takes a prefix, so the last part of what arrives is nearly always incomplete.
    let block = header(&["Content-Type: multipart/mixed; boundary=\"edge\""]);
    let cut = b"--edge\r\nContent-Type: text/plain\r\n\r\nThe words go on and";

    let envelope = envelope_of(&block, cut, Some(0), None, Flags::NONE);

    assert!(envelope
        .preview
        .as_deref()
        .is_none_or(|preview| !preview.contains("--edge")));
}

#[test]
fn a_message_with_no_body_yet_has_no_preview() {
    let block = header(&["Subject: Envelope only"]);

    assert_eq!(
        envelope_of(&block, b"", Some(0), None, Flags::NONE).preview,
        None
    );
}

#[test]
fn a_sender_without_a_name_is_shown_by_address() {
    let block = header(&["From: bare@example.com"]);

    assert_eq!(
        envelope_of(&block, b"", Some(0), None, Flags::NONE)
            .sender
            .as_deref(),
        Some("bare@example.com")
    );
}

#[test]
fn the_server_arrival_time_is_preferred_over_the_one_the_sender_wrote() {
    // A `Date` header is written by whoever sent the message. Sorting by it puts mail from the
    // year 2087 at the top of the list and keeps it there.
    let block = header(&[
        "Date: Tue, 1 Jan 2087 00:00:00 +0000",
        "Subject: From the future",
    ]);

    let envelope = envelope_of(&block, b"", Some(1_700_000_000), None, Flags::NONE);

    assert_eq!(envelope.received_at, 1_700_000_000);
}

#[test]
fn the_date_header_is_used_when_the_server_gives_no_arrival_time() {
    let block = header(&["Date: Tue, 14 Nov 2023 22:13:20 +0000"]);

    let envelope = envelope_of(&block, b"", None, None, Flags::NONE);

    assert_eq!(envelope.received_at, 1_700_000_000);
}

#[test]
fn a_reply_carries_what_it_answers() {
    let block = header(&[
        "Message-ID: <third@example.com>",
        "References: <first@example.com> <second@example.com>",
        "In-Reply-To: <second@example.com>",
    ]);

    let references: Vec<String> = envelope_of(&block, b"", Some(0), None, Flags::NONE)
        .references
        .iter()
        .map(|id| id.as_str().to_owned())
        .collect();

    assert_eq!(references, vec!["first@example.com", "second@example.com"]);
}

#[test]
fn a_parent_missing_from_references_is_taken_from_in_reply_to() {
    // Written by clients that fill one header and not the other.
    let block = header(&["In-Reply-To: <parent@example.com>"]);

    let references: Vec<String> = envelope_of(&block, b"", Some(0), None, Flags::NONE)
        .references
        .iter()
        .map(|id| id.as_str().to_owned())
        .collect();

    assert_eq!(references, vec!["parent@example.com"]);
}

#[test]
fn a_message_with_nothing_readable_in_it_still_becomes_a_row() {
    // Every field here is written by whoever sent the message. A missing field is a missing
    // field; it is never a failed pass, and never a panic.
    let malformed: Vec<Vec<u8>> = vec![
        Vec::new(),
        header(&[]),
        header(&["Subject:"]),
        header(&["Subject: =?UTF-8?B?not-base64!!!?="]),
        header(&["From: ", "Message-ID: <>"]),
        header(&["Subject: unterminated =?UTF-8?B?"]),
        vec![0x00, 0xff, 0xfe, b' ', b'b', b'a', b'd'],
    ];

    for block in malformed {
        let envelope = envelope_of(&block, b"", None, None, Flags::NONE);
        assert!(envelope.received_at >= 0);
    }
}

#[test]
fn a_message_the_parser_panics_on_is_a_row_of_what_the_server_said_and_the_rest_are_read() {
    // The parser runs on the pass, outside the guards the reading path has. Uncaught, a panic in
    // it ends the pass over the whole account, and every pass after it asks for the same message
    // again. Caught one message at a time, it ends that message's description: the message is
    // still a row, and the one beside it is read as usual.
    let header_of = |subject: &str| {
        Some(header(&[
            &format!("Subject: {subject}"),
            "From: zeta@example.com",
        ]))
    };
    let fetched = [
        answer::Fetched {
            uid: Some(5),
            flags: Some(Flags::SEEN),
            size: Some(2048),
            internal_date: Some(1_700_000_000),
            header: header_of("Breaks the parser"),
            ..answer::Fetched::default()
        },
        answer::Fetched {
            uid: Some(6),
            size: Some(1024),
            internal_date: Some(1_700_000_100),
            header: header_of("Reads as usual"),
            ..answer::Fetched::default()
        },
    ];

    let arrived = arrivals(&fetched, None, &(1..=u32::MAX), |message| {
        match message.uid {
            Some(5) => panic!("a parser meeting a message it was not written for"),
            _ => described(message),
        }
    });

    assert_eq!(arrived.len(), 2, "one message's panic took another with it");

    let broken = &arrived[0].envelope;
    assert_eq!(arrived[0].uid, Uid::new(5).expect("non-zero"));
    assert_eq!(
        broken.subject, None,
        "nothing is taken from bytes that broke the parser"
    );
    assert_eq!(broken.received_at, 1_700_000_000);
    assert_eq!(broken.rfc822_size, Some(2048));
    assert!(broken.flags.contains(Flags::SEEN));

    assert_eq!(
        arrived[1].envelope.subject.as_deref(),
        Some("Reads as usual")
    );
}

#[test]
fn a_subject_that_is_only_spaces_is_no_subject_at_all() {
    let block = header(&["Subject:"]);

    assert_eq!(
        envelope_of(&block, b"", Some(0), None, Flags::NONE).subject,
        None
    );
}

#[test]
fn a_server_that_says_nothing_leaves_the_folder_as_the_user_made_it() {
    assert_eq!(role_of(&[]), FolderRole::Custom);
    assert_eq!(
        role_of(&[NameAttribute::NoInferiors, NameAttribute::Marked]),
        FolderRole::Custom
    );
}

#[test]
fn mail_arrived_when_the_count_rose_in_any_order_the_lines_could_have_come() {
    // The line that ended the idle came somewhere among the queued ones, and the queue does not
    // say where. One went and the count is back where it was: one arrived, whichever came first.
    assert!(arrived(3, Some(Heard::Holds(3)), &[Heard::OneGone]));
    assert!(arrived(3, Some(Heard::OneGone), &[Heard::Holds(3)]));
    assert!(
        arrived(3, None, &[Heard::Holds(4)]),
        "heard only while starting"
    );

    assert!(!arrived(3, Some(Heard::Other), &[]), "a flag");
    assert!(!arrived(3, Some(Heard::OneGone), &[Heard::Holds(2)]));
    assert!(
        !arrived(3, Some(Heard::Holds(3)), &[]),
        "the same count again"
    );
    assert!(!arrived(3, None, &[]));
}

#[test]
fn a_name_that_cannot_be_opened_is_not_a_folder() {
    // Recorded, `[Gmail]` would be opened by every pass of every Gmail account and refused every
    // time, and a pass with a failed folder is one that forgets less.
    assert!(!selectable(&[
        NameAttribute::Extension("\\HasChildren".into()),
        NameAttribute::NoSelect,
    ]));
    assert!(!selectable(&[NameAttribute::Extension(
        "\\NonExistent".into()
    )]));
    assert!(!selectable(&[NameAttribute::Extension(
        "\\nonexistent".into()
    )]));

    assert!(selectable(&[]));
    assert!(selectable(&[
        NameAttribute::Extension("\\HasNoChildren".into()),
        NameAttribute::Sent,
    ]));
}

#[test]
fn every_special_use_attribute_becomes_a_role() {
    for (attribute, role) in [
        (NameAttribute::All, FolderRole::AllMail),
        (NameAttribute::Archive, FolderRole::Archive),
        (NameAttribute::Drafts, FolderRole::Drafts),
        (NameAttribute::Flagged, FolderRole::Flagged),
        (NameAttribute::Junk, FolderRole::Junk),
        (NameAttribute::Sent, FolderRole::Sent),
        (NameAttribute::Trash, FolderRole::Trash),
    ] {
        assert_eq!(role_of(&[NameAttribute::NoInferiors, attribute]), role);
    }
}

#[test]
fn a_refused_login_is_not_a_network_problem() {
    assert_eq!(
        login_failed(async_imap::error::Error::No("bad password".into())),
        MailProviderError::Unauthorized
    );
    assert_eq!(
        login_failed(async_imap::error::Error::ConnectionLost),
        MailProviderError::Offline
    );
}

#[test]
fn lists_the_mailboxes_of_the_development_server() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    let provider = ImapProvider::new(
        account("test@lyonoyl.local", "lyonoyl"),
        shared_runtime().handle().clone(),
    )
    .expect("provider builds");
    let mailboxes = provider.list_mailboxes().expect("the server answers");

    let inbox = mailboxes
        .iter()
        .find(|mailbox| is_inbox(&mailbox.name))
        .expect("every account has an inbox");
    assert_eq!(
        inbox.role,
        FolderRole::Custom,
        "the inbox carries no special-use attribute"
    );

    let roles: Vec<FolderRole> = mailboxes.iter().map(|mailbox| mailbox.role).collect();
    for expected in [
        FolderRole::Sent,
        FolderRole::Drafts,
        FolderRole::Trash,
        FolderRole::Junk,
        FolderRole::Archive,
    ] {
        assert!(roles.contains(&expected), "missing role: {expected:?}");
    }
}

#[test]
fn the_provider_works_from_a_blocking_task() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    // The path the commands take: an async command hands the work to the blocking pool, and
    // the adapter drives its session from there.
    let mailboxes = shared_runtime().block_on(async {
        tokio::task::spawn_blocking(|| {
            let provider =
                ImapProvider::new(account("test@lyonoyl.local", "lyonoyl"), Handle::current())
                    .expect("provider builds");
            provider.list_mailboxes()
        })
        .await
        .expect("the blocking task neither panicked nor was cancelled")
    });

    assert!(!mailboxes.expect("the server answers").is_empty());
}

#[test]
fn a_real_server_takes_the_question_about_what_vanished() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    // Two things a scripted server cannot give. That a real one answers `ENABLE QRESYNC` by
    // naming it back, in the shape this build reads — and that the question a pass then asks,
    // carrying the `VANISHED` modifier, is not refused. Dovecot answers a fetch that carries it
    // without the agreement with `BAD Error in IMAP command UID FETCH: QRESYNC not enabled`.
    //
    // A mailbox of its own, holding the corpus: a mailbox nothing has ever changed reports the
    // first modification sequence there is, and a pass asking from that one asks nothing.
    let corpus = holding_the_corpus("vanished-question");
    let (lowest, third) = (corpus.uids[0], corpus.uids[2]);
    let asked = SyncRequest {
        folder_id: lyonoyl_domain::message::FolderId::new(1).expect("non-zero"),
        intent: SyncIntent::Forward,
        mailbox: "INBOX".to_owned(),
        cursor: Some(SyncCursor::new(lowest.get().to_string())),
        uid_validity: None,
        known_window: Some(lowest..=third),
        // Below anything the server can report, so the question is actually asked.
        highest_modseq: Some(ModSeq::new(1)),
        known_count: 3,
        refused_for_now_last_time: false,
    };

    let (agreed, sequence, taken, batch) = shared_runtime().block_on(async {
        let provider = ImapProvider::new(account(&corpus.address, "lyonoyl"), Handle::current())
            .expect("provider builds");

        let mut session = provider.open().await.expect("the server accepts the login");
        let agreed = enable_qresync(&mut session).await;
        let sequence = session
            .examine("INBOX")
            .await
            .ok()
            .and_then(|opened| opened.highest_modseq);
        // The pass's own question, read here to its tagged reply. The client library ends a fetch
        // at that reply whatever it says, so through the pass a refusal reads as a fetch that found
        // nothing changed, and the pass goes on to list the folder instead.
        let question = format!("UID FETCH 1:* {}", changed_since_query(1, true));
        let taken = taken_whole(&mut session, &question).await;
        let _ = session.logout().await;

        let mut session = provider.open().await.expect("the server accepts the login");
        // Settled on the session, as production settles it: once, before any mailbox is open.
        let qresync = super::pass::enable_qresync(&mut session)
            .await
            .unwrap_or(false);
        let batch = one_pass(&mut session, &asked, qresync).await;
        let _ = session.logout().await;

        (agreed, sequence, taken, batch)
    });

    assert!(
        agreed.expect("the server answers the question"),
        "a server that lists QRESYNC did not agree to it"
    );
    assert!(
        sequence > Some(1),
        "the mailbox reports {sequence:?}, so a pass never asks what vanished"
    );
    assert!(
        taken,
        "a real server refused the question about what changed and what vanished"
    );
    assert!(
        batch.is_ok(),
        "a real server refused the pass: {:?}",
        batch.err()
    );
}

/// Whether the server took a command, read up to its own tagged reply.
async fn taken_whole<S>(session: &mut async_imap::Session<S>, command: &str) -> bool
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let asked = session
        .run_command(command)
        .await
        .expect("the command is sent");

    while let Some(response) = session.read_response().await.expect("the server answers") {
        if let Response::Done { tag, status, .. } = response.parsed() {
            if tag == &asked {
                return *status == Status::Ok;
            }
        }
    }

    false
}

#[test]
fn a_watch_against_the_development_server_runs_out_rather_than_fails() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    // What a scripted server cannot prove: that a real one takes the idle, that the read-only
    // guard lets `IDLE` and its untagged `DONE` through, and that a watch nothing interrupts
    // comes back as quiet instead of as a failure — which is the difference between watching
    // again at once and waiting two minutes first.
    //
    // A mailbox of its own, which nothing else delivers to: a message arriving in a shared one
    // while this watches is a watch that rightly ends early, and a test that fails for it.
    let started = std::time::Instant::now();
    let provider = ImapProvider::new(
        account("watch-quiet@lyonoyl.local", "lyonoyl"),
        shared_runtime().handle().clone(),
    )
    .expect("provider builds");
    let first = provider
        .watch("INBOX", None, Duration::from_secs(2))
        .expect("the server answers");

    assert_eq!(first.watched, Watched::Quiet);
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "the watch took {:?} to end",
        started.elapsed()
    );

    // Taken up from where the first one ended, a mailbox nothing arrived in is watched rather
    // than reported: a real server's answer read back as something new would be a watch that
    // ends the moment it starts, and a pass after every one of them.
    let second = provider
        .watch("INBOX", first.seen.as_ref(), Duration::from_secs(2))
        .expect("the server answers");
    assert_eq!(second.watched, Watched::Quiet);
    assert!(second.seen.is_some(), "the next watch would start blind");
}

#[test]
fn where_a_watch_ended_reads_back_as_it_was_written() {
    let stood = |generation, next, holds| Stood {
        generation,
        next,
        holds,
    };

    for one in [
        stood(Some(7), Some(120), 118),
        stood(None, None, 0),
        stood(Some(1), None, 3),
    ] {
        assert_eq!(Stood::read(&one.mark()), Some(one));
    }
    // Written by something else, it is nothing to start from, and the watch starts from where it
    // finds the mailbox.
    for foreign in ["", "7/120", "7/120/118/1", "seven/120/118", "7/120/-1"] {
        assert_eq!(Stood::read(&Watermark::new(foreign)), None, "{foreign:?}");
    }
}

#[test]
fn mail_arriving_is_told_by_the_number_the_next_message_is_given() {
    let stood = |generation, next, holds| Stood {
        generation: Some(generation),
        next,
        holds,
    };
    let ended = stood(1, Some(10), 5);

    assert!(ended.arrived_by(stood(1, Some(11), 6)), "one arrived");
    assert!(
        ended.arrived_by(stood(1, Some(11), 5)),
        "one arrived beside one filed away"
    );
    assert!(
        !ended.arrived_by(stood(1, Some(10), 4)),
        "one filed away and nothing arrived"
    );
    assert!(!ended.arrived_by(ended), "nothing at all");
    assert!(
        ended.arrived_by(stood(2, Some(3), 2)),
        "a mailbox renumbered in between is read again"
    );

    // A server that does not say what the next number is leaves the count as the only sign.
    let counted = stood(1, None, 5);
    assert!(counted.arrived_by(stood(1, None, 6)));
    assert!(!counted.arrived_by(stood(1, None, 5)));
}

#[test]
#[ignore = "opens a hundred sessions against the development server; started explicitly"]
fn a_hundred_accounts_work_at_the_same_time() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    const ACCOUNTS: usize = 100;
    let started = std::time::Instant::now();

    let workers: Vec<_> = (0..ACCOUNTS)
        .map(|number| {
            let runtime = shared_runtime().handle().clone();
            std::thread::spawn(move || {
                let provider = ImapProvider::new(
                    account(&format!("user{number}@lyonoyl.local"), "lyonoyl"),
                    runtime,
                )
                .expect("provider builds");
                provider.list_mailboxes()
            })
        })
        .collect();

    let outcomes: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().expect("no account brought down its thread"))
        .collect();
    let elapsed = started.elapsed();

    let failures: Vec<_> = outcomes
        .iter()
        .filter_map(|outcome| outcome.as_ref().err())
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {ACCOUNTS} accounts failed, first: {:?}",
        failures.len(),
        failures.first()
    );

    // Not a benchmark, a hang detector: a deadlock or a lost connection shows up as a wait
    // rather than as an error.
    assert!(
        elapsed.as_secs() < 60,
        "a hundred accounts took {elapsed:?}, which means something is waiting"
    );

    println!(
        "{ACCOUNTS} accounts listed their mailboxes in {:.2} s",
        elapsed.as_secs_f64()
    );
}

#[test]
fn a_wrong_password_is_reported_as_such() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    let provider = ImapProvider::new(
        account("test@lyonoyl.local", "not-the-password"),
        shared_runtime().handle().clone(),
    )
    .expect("provider builds");

    assert_eq!(
        provider.list_mailboxes().expect_err("the server refuses"),
        MailProviderError::Unauthorized
    );
}

#[test]
fn reads_several_messages_in_one_conversation_with_the_server() {
    // What the batch is for. The counter is what makes it checkable: a round that asks for ten
    // messages and opens ten sessions costs ten sign-ins, and nothing but a count says so.
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    // A mailbox of its own, holding the corpus, so the numbers asked for are numbers it holds.
    let corpus = holding_the_corpus("several-at-once");
    let provider = ImapProvider::new(
        account(&corpus.address, "lyonoyl"),
        shared_runtime().handle().clone(),
    )
    .expect("provider builds");

    let inbox = provider
        .list_mailboxes()
        .expect("the server answers")
        .into_iter()
        .find(|mailbox| is_inbox(&mailbox.name))
        .expect("every account has an inbox");

    // Four the inbox holds, by number. Asking for numbers the server does not have is a separate
    // case; this one is about the cost of asking for several it does.
    let wanted: Vec<Uid> = corpus.uids[..4].to_vec();

    let before = super::sessions_opened_here().0;
    let got = provider
        .whole_messages(&inbox.name, corpus.generation, &wanted)
        .expect("the server answers");
    let opened = super::sessions_opened_here().0 - before;

    // Nothing at all: listing the mailboxes above opened this provider's session and it is still
    // open. A pass over an account is one connection now, not one per thing it asks of the server.
    assert_eq!(opened, 0, "a second connection for the same pass");
    assert_eq!(got.len(), wanted.len(), "asked for four messages it holds");

    for (uid, raw) in &got {
        assert!(
            wanted.contains(uid),
            "answered about {uid:?}, which nobody asked for"
        );
        assert!(!raw.is_empty(), "a message with no bytes in it");
    }

    // The same messages read one at a time, to show both ways agree about the bytes. On a held
    // session neither way opens anything; what asking together saves is a round trip per message.
    let one_at_a_time: Vec<_> = got
        .iter()
        .map(|(uid, _)| {
            (
                *uid,
                provider
                    .whole_message(&inbox.name, corpus.generation, *uid)
                    .expect("the server answers"),
            )
        })
        .collect();
    assert_eq!(one_at_a_time, got, "the same bytes either way");
}

#[test]
fn a_call_that_failed_closes_its_connection_without_another_word() {
    // Signing out reads to the end of whatever the server is still saying, and a call fails when a
    // server went quiet or said more than an answer may hold: reading on is what failing was for.
    // The connection is closed instead, so the server hears nothing after the command it refused.
    use std::io::{BufRead, BufReader, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port on this machine");
    let port = listener.local_addr().expect("the port is bound").port();

    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("the provider connects");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .expect("a deadline for the scripted server");
        let mut heard = BufReader::new(stream.try_clone().expect("the stream clones"));
        let mut answering = stream;
        answering
            .write_all(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .expect("the provider listens");

        let mut line = String::new();
        let mut answer = |heard: &mut BufReader<_>, said: &str| {
            line.clear();
            heard.read_line(&mut line).expect("the provider asks");
            let tag = line
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .to_owned();
            answering
                .write_all(format!("{tag} {said}\r\n").as_bytes())
                .expect("the provider listens");
        };

        answer(&mut heard, "OK LOGIN completed");
        answer(&mut heard, "OK ENABLE completed");
        answer(&mut heard, "NO [NONEXISTENT] No such mailbox");

        let mut after = String::new();
        let read = heard
            .read_line(&mut after)
            .expect("the connection is readable");
        (read, after)
    });

    let provider = ImapProvider::new(
        ImapAccount {
            endpoint: crate::infrastructure::net::Endpoint {
                host: "127.0.0.1".to_owned(),
                port,
                encryption: crate::infrastructure::net::Encryption::None,
            },
            username: "person".to_owned(),
            credential: Credential::password(SecretString::from("secret".to_owned())),
        },
        shared_runtime().handle().clone(),
    )
    .expect("provider builds");

    let failure = provider
        .whole_message(
            "Gone",
            UidValidity::new(1).expect("non-zero"),
            Uid::new(1).expect("non-zero"),
        )
        .expect_err("the server refused the mailbox");
    assert_eq!(failure, MailProviderError::Protocol);

    let (read, after) = server.join().expect("the scripted server finishes");
    assert_eq!(
        (read, after.as_str()),
        (0, ""),
        "the server was told something after the call failed"
    );
}

/// Asks a server on this machine for two messages whole, the server answering each command it is
/// sent with the next of `answers`, `{tag}` standing for the command's own tag.
fn whole_messages_from_a_scripted_server(
    answers: &'static [&'static str],
) -> Result<Vec<(Uid, Vec<u8>)>, Unread> {
    use std::io::{BufRead, BufReader, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port on this machine");
    let port = listener.local_addr().expect("the port is bound").port();

    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("the provider connects");
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .expect("a deadline for the scripted server");
        let mut heard = BufReader::new(stream.try_clone().expect("the stream clones"));
        let mut answering = stream;
        answering
            .write_all(b"* OK [CAPABILITY IMAP4rev1] ready\r\n")
            .expect("the provider listens");

        for said in answers {
            let mut line = String::new();
            heard.read_line(&mut line).expect("the provider asks");
            let tag = line.split_whitespace().next().unwrap_or_default();
            answering
                .write_all(format!("{}\r\n", said.replace("{tag}", tag)).as_bytes())
                .expect("the provider listens");
        }

        // Until the provider lets the connection go.
        let mut rest = String::new();
        let _ = heard.read_line(&mut rest);
    });

    let provider = ImapProvider::new(
        ImapAccount {
            endpoint: crate::infrastructure::net::Endpoint {
                host: "127.0.0.1".to_owned(),
                port,
                encryption: crate::infrastructure::net::Encryption::None,
            },
            username: "person".to_owned(),
            credential: Credential::password(SecretString::from("secret".to_owned())),
        },
        shared_runtime().handle().clone(),
    )
    .expect("provider builds");

    let got = provider.whole_messages(
        "INBOX",
        UidValidity::new(1).expect("non-zero"),
        &[
            Uid::new(1).expect("non-zero"),
            Uid::new(2).expect("non-zero"),
        ],
    );
    drop(provider);
    server.join().expect("the scripted server finishes");

    got
}

#[test]
fn a_request_that_never_went_out_is_told_from_one_that_failed() {
    // The round that reads mail whole asks the messages of a failed request one at a time, which
    // is worth it only where the request went out and failed over something in it. A sign-in the
    // server refused asked nothing: asked again message by message, it is a refused sign-in each
    // time, counted toward locking the account.
    assert_eq!(
        whole_messages_from_a_scripted_server(&[
            "{tag} NO [AUTHENTICATIONFAILED] Authentication failed."
        ]),
        Err(Unread::NotAsked(MailProviderError::Unauthorized))
    );

    // A server that cannot read a message ends the conversation in the middle of the answer —
    // Dovecot's own default. The request went out, and something in it is why it failed.
    assert_eq!(
        whole_messages_from_a_scripted_server(&[
            "{tag} OK LOGIN completed",
            "{tag} OK ENABLE completed",
            "* 2 EXISTS\r\n* OK [UIDVALIDITY 1] .\r\n{tag} OK [READ-ONLY] EXAMINE completed",
            "* BYE Internal error occurred",
        ]),
        Err(Unread::Failed(MailProviderError::Offline))
    );
}

#[test]
fn asking_for_no_messages_opens_nothing() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    let provider = ImapProvider::new(
        account("test@lyonoyl.local", "lyonoyl"),
        shared_runtime().handle().clone(),
    )
    .expect("provider builds");

    let before = super::sessions_opened_here().0;
    let got = provider
        .whole_messages("INBOX", UidValidity::new(1).expect("non-zero"), &[])
        .expect("nothing to ask about is not a failure");

    assert!(got.is_empty());
    assert_eq!(
        super::sessions_opened_here().0,
        before,
        "a request for nothing reached the server"
    );
}

#[test]
fn what_one_thread_opens_is_not_what_another_opens() {
    // The instrument this replaced was a count across the process, and it was wrong in exactly the
    // way that is hardest to see: with nine accounts read side by side, every pass reported the
    // whole round. One pass over six folders looked like sixty-three conversations. It opens
    // seven, and sixty-three was nine accounts times seven.
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    // A provider each time, because a provider keeps its session: what is counted here is whether
    // one thread sees another thread's connections, not how often one thread reuses its own.
    let opening = |how_many: usize| {
        let before = super::sessions_opened_here();

        for _ in 0..how_many {
            ImapProvider::new(
                account("test@lyonoyl.local", "lyonoyl"),
                shared_runtime().handle().clone(),
            )
            .expect("provider builds")
            .list_mailboxes()
            .expect("the server answers");
        }

        let after = super::sessions_opened_here();
        (after.0 - before.0, after.1 - before.1)
    };

    // Two threads, doing different amounts of work at the same time, each asked what it did.
    let busy = std::thread::spawn(move || opening(4));
    let quiet = std::thread::spawn(move || opening(1));

    let (busy_opened, busy_took) = busy.join().expect("the thread finishes");
    let (quiet_opened, _) = quiet.join().expect("the thread finishes");

    assert_eq!(busy_opened, 4, "a thread reported somebody else's work");
    assert_eq!(quiet_opened, 1, "a thread reported somebody else's work");
    assert!(
        busy_took > std::time::Duration::ZERO,
        "opening four connections took no time at all"
    );
}

/// A provider that could not do what it was asked gives every session after that longer to answer:
/// a server slow to answer once is given more time the second time, for the rest of the pass.
#[test]
fn a_provider_that_failed_gives_its_next_session_longer() {
    // A port nothing listens on: taken from the system, then let go.
    let nobody = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = nobody.local_addr().expect("its address").port();
    drop(nobody);

    let mut unanswered = account("test@lyonoyl.local", "lyonoyl");
    unanswered.endpoint.port = port;

    let (before, after, short, patient) = shared_runtime().block_on(async {
        tokio::task::spawn_blocking(move || {
            let provider =
                ImapProvider::new(unanswered, Handle::current()).expect("provider builds");
            let before = provider.the_silence();
            let failed = provider.list_mailboxes();
            assert!(failed.is_err(), "a port nothing listens on answered");

            (
                before,
                provider.the_silence(),
                provider.policy.read_timeout(),
                provider.policy.patient_read_timeout(),
            )
        })
        .await
        .expect("the blocking task neither panicked nor was cancelled")
    });

    assert!(patient > short);
    assert_eq!(
        before, short,
        "a provider that had failed at nothing was patient"
    );
    assert_eq!(
        after, patient,
        "a provider that failed was not given longer"
    );
}

/// A provider whose session opened and then broke off in the middle of a command gives every
/// session after that longer to answer, the same as one that could not open at all.
#[test]
fn a_provider_whose_session_broke_gives_its_next_session_longer() {
    use std::io::{BufRead, BufReader, Write};

    // A server that greets, signs anybody in, answers whatever else is asked, and hangs up on the
    // listing of mailboxes.
    let listening = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    let port = listening.local_addr().expect("its address").port();
    let serving = std::thread::spawn(move || {
        let (stream, _) = listening.accept().expect("a connection");
        let mut writing = stream.try_clone().expect("the same connection");
        let reading = BufReader::new(stream);

        writing.write_all(b"* OK ready\r\n").expect("the greeting");
        for line in reading.lines() {
            let Ok(line) = line else { return };
            let mut words = line.split(' ');
            let tag = words.next().unwrap_or_default().to_owned();
            if words
                .next()
                .is_some_and(|command| command.eq_ignore_ascii_case("LIST"))
            {
                return;
            }
            let _ = writing.write_all(format!("{tag} OK done\r\n").as_bytes());
        }
    });

    let mut breaking = account("test@lyonoyl.local", "lyonoyl");
    breaking.endpoint.port = port;

    let (before, after, short, patient) = shared_runtime().block_on(async {
        tokio::task::spawn_blocking(move || {
            let provider = ImapProvider::new(breaking, Handle::current()).expect("provider builds");
            let before = provider.the_silence();
            let failed = provider.list_mailboxes();
            assert!(
                failed.is_err(),
                "a listing the server hung up on was answered"
            );

            (
                before,
                provider.the_silence(),
                provider.policy.read_timeout(),
                provider.policy.patient_read_timeout(),
            )
        })
        .await
        .expect("the blocking task neither panicked nor was cancelled")
    });
    serving.join().expect("the server ran");

    assert_eq!(before, short);
    assert_eq!(
        after, patient,
        "a provider whose session broke was not given longer"
    );
}
