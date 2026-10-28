//! A mailbox on the development server that holds the message corpus, for one test.
//!
//! The development accounts hold the corpus only once `scripts/reset-mail-server.sh` has filled
//! them, and from then on they hold whatever a person clicked. A test that reads them is a test of
//! that afternoon: on a server started fresh they hold nothing, and the test fails for a reason
//! that has nothing to do with what it checks. So a test that needs mail to read names a mailbox of
//! its own here, and this puts the corpus into it over IMAP.
//!
//! Filled once and then left alone. A mailbox that holds the corpus exactly as filling it leaves it
//! — every message byte for byte, in the order of the file names, numbered from the first UID, and
//! nothing numbered since — is answered from as it stands, so a second run does not pile a second
//! copy on the first. One that holds anything else is started over and filled again: a run stopped
//! halfway through filling it, a message added or taken away, a corpus file edited since — to the
//! same length too, which is how an address is scrubbed.
//!
//! Started over rather than emptied. A mailbox emptied message by message keeps every number it has
//! handed out, so the corpus would land higher on each refill until a first pass, which reads the
//! newest few hundred numbers, no longer reached the first message — and a test asserting that it
//! does would fail about the server's counter rather than about the code.
//!
//! One mailbox per test, never shared: the tests in a run go at once, and two of them filling the
//! same mailbox would each find it half full and start it over under the other.

use futures::TryStreamExt;
use secrecy::SecretString;

use super::read_only::ReadOnly;
use super::{development_endpoint, open_session, ImapAccount, Session};
use crate::infrastructure::net::{Deadline, Silence, TransportPolicy};
use crate::ports::credential::Credential;
use lyonoyl_domain::message::{Uid, UidValidity};

/// The container the development server runs in, as `docker/mail-server/compose.yml` names it.
const SERVER: &str = "lyonoyl-mail-server";

/// Where that server keeps each user's mail: `mail_home` in `docker/mail-server/config/dovecot.conf`.
const MAIL_HOMES: &str = "/srv/mail";

/// A mailbox holding the corpus, and the number the server gave each message in it.
pub struct CorpusMailbox {
    /// The address that signs in to it, which is also its user name on the development server.
    pub address: String,
    /// Every message's UID, lowest first: one to the size of the corpus.
    pub uids: Vec<Uid>,
    /// The generation those numbers count in.
    pub generation: UidValidity,
}

/// The mailbox `test` reads the corpus from, filled first unless it holds exactly the corpus.
///
/// `test` names one test, and no other test may pass the same name. Called only once the
/// development server is known to answer.
pub fn holding_the_corpus(test: &str) -> CorpusMailbox {
    // The name becomes a directory on the server, and starting the mailbox over removes it: one
    // word of lower-case letters, digits and hyphens cannot name any directory but its own.
    assert!(
        !test.is_empty()
            && test
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'),
        "{test:?} is not a test name a mailbox can be called by"
    );

    let address = format!("corpus-{test}@lyonoyl.local");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime to fill the mailbox on");
    let inbox = filled(&runtime, &address, &corpus());

    CorpusMailbox {
        address,
        uids: inbox.uids(),
        generation: inbox.generation,
    }
}

/// Every message in `tests/fixtures/eml`, in the order of the file names, ended the way IMAP ends a
/// line.
fn corpus() -> Vec<Vec<u8>> {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/eml");

    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&directory)
        .unwrap_or_else(|error| {
            panic!(
                "the corpus at {} is unreadable: {error}",
                directory.display()
            )
        })
        .map(|entry| entry.expect("an entry in the corpus directory").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "eml"))
        .collect();
    files.sort();

    // An empty corpus fills an empty mailbox, and every test reading it would pass about nothing.
    assert!(
        !files.is_empty(),
        "the corpus at {} holds no messages",
        directory.display()
    );

    files
        .iter()
        .map(|file| with_crlf(&std::fs::read(file).expect("a corpus message is readable")))
        .collect()
}

/// The same bytes with every line ended by CR LF.
///
/// The files end their lines with LF alone. IMAP carries a message with CR LF, and that is how the
/// server hands a message back — so it is the form a stored message is compared with.
fn with_crlf(raw: &[u8]) -> Vec<u8> {
    let mut ended = Vec::with_capacity(raw.len() + raw.len() / 32);
    let mut previous = None;

    for &byte in raw {
        if byte == b'\n' && previous != Some(b'\r') {
            ended.push(b'\r');
        }
        ended.push(byte);
        previous = Some(byte);
    }

    ended
}

/// What an inbox holds, as the server states it.
struct Inbox {
    /// The generation its numbers are counted in.
    generation: UidValidity,
    /// The number the next message will be given, where the server states one.
    next: Option<u32>,
    /// Every message, lowest UID first, as the server hands it back.
    messages: Vec<(Uid, Vec<u8>)>,
}

impl Inbox {
    fn uids(&self) -> Vec<Uid> {
        self.messages.iter().map(|(uid, _)| *uid).collect()
    }
}

/// Whether an inbox holds the corpus exactly as filling a fresh mailbox with it leaves it.
///
/// The bytes and not their lengths: a corpus file edited to the same length — an address replaced
/// by one of the same shape — must not be taken for the one it was. The numbers too: as many
/// messages as the corpus and a next number one past that is the corpus at one to its size with
/// nothing numbered since, so a first pass over it reaches the first UID.
fn holds_the_corpus(inbox: &Inbox, corpus: &[Vec<u8>]) -> bool {
    inbox.next == u32::try_from(corpus.len() + 1).ok()
        && inbox.messages.len() == corpus.len()
        && inbox
            .messages
            .iter()
            .zip(corpus)
            .all(|((_, held), wanted)| held == wanted)
}

/// Fills the inbox unless it already holds the corpus, and answers with what it then holds.
fn filled(runtime: &tokio::runtime::Runtime, address: &str, corpus: &[Vec<u8>]) -> Inbox {
    let found = runtime.block_on(inbox_of(address));
    if holds_the_corpus(&found, corpus) {
        return found;
    }

    start_over(address);

    runtime.block_on(async {
        let mut session = signed_in(address).await;

        // Asked rather than assumed: a removal that did not happen, or a session still writing
        // into the directory as it went, would put the corpus on top of what was there.
        let fresh = holding(&mut session).await;
        assert!(
            fresh.messages.is_empty() && fresh.next == Some(1),
            "{address} did not start over: {} messages, next UID {:?}",
            fresh.messages.len(),
            fresh.next
        );

        for message in corpus {
            session
                .append("INBOX", None, None, message)
                .await
                .unwrap_or_else(|error| panic!("{address} did not take a corpus message: {error}"));
        }

        let _ = session.logout().await;
    });

    let filled = runtime.block_on(inbox_of(address));
    assert!(
        holds_the_corpus(&filled, corpus),
        "{address} does not hold the corpus after being filled with it"
    );

    filled
}

/// Removes everything the development server holds for `address`, so that the next sign-in starts
/// it over: a new generation, and numbering from the first UID.
///
/// IMAP has no way to ask for this. A removed message keeps its number, and the inbox can be
/// neither deleted nor renamed — Dovecot refuses both. On the development server a user's mail is
/// one directory, and removing it is how `scripts/reset-mail-server.sh` starts every account over.
fn start_over(address: &str) {
    let removed = std::process::Command::new("docker")
        .args(["exec", SERVER, "rm", "-rf", "--"])
        .arg(format!("{MAIL_HOMES}/{address}"))
        .status()
        .expect("docker is available");

    assert!(
        removed.success(),
        "{address} could not be started over on the development server"
    );
}

/// What `address`'s inbox holds, in a session of its own.
async fn inbox_of(address: &str) -> Inbox {
    let mut session = signed_in(address).await;
    let inbox = holding(&mut session).await;
    let _ = session.logout().await;

    inbox
}

/// The inbox's generation, its next number, and every message in it.
async fn holding(session: &mut Session) -> Inbox {
    let opened = session.select("INBOX").await.expect("the inbox opens");
    let generation = opened
        .uid_validity
        .and_then(|value| UidValidity::new(value).ok())
        .expect("the development server states a generation");

    // Asked only of a mailbox that holds something: a range ending at `*` names the newest message,
    // and a mailbox with none has no newest message to name.
    if opened.exists == 0 {
        return Inbox {
            generation,
            next: opened.uid_next,
            messages: Vec::new(),
        };
    }

    // `PEEK`, so reading what a message holds does not mark it read.
    let fetched: Vec<async_imap::types::Fetch> = session
        .uid_fetch("1:*", "BODY.PEEK[]")
        .await
        .expect("the server lists the inbox")
        .try_collect()
        .await
        .expect("the server lists the inbox");

    let mut messages: Vec<(Uid, Vec<u8>)> = fetched
        .iter()
        .map(|message| {
            let uid = message
                .uid
                .and_then(|uid| Uid::new(uid).ok())
                .expect("a listed message has a UID");
            let raw = message.body().expect("a listed message has its bytes");
            (uid, raw.to_vec())
        })
        .collect();
    messages.sort_unstable_by_key(|(uid, _)| uid.get());

    Inbox {
        generation,
        next: opened.uid_next,
        messages,
    }
}

/// A session that may add to the mailbox and act on a message by UID.
///
/// The acting kind, which is the one the stream guard lets an append and a change by UID through:
/// the reading kind refuses both on the wire, whoever sends them.
async fn signed_in(address: &str) -> Session {
    let policy = TransportPolicy::new().expect("a transport policy");
    let account = ImapAccount {
        endpoint: development_endpoint(),
        username: address.to_owned(),
        credential: Credential::password(SecretString::from("lyonoyl".to_owned())),
    };

    let stream = policy
        .connect_retrying(&account.endpoint)
        .await
        .expect("the development mail server answers");
    let client = async_imap::Client::new(Deadline::over(
        ReadOnly::acting(stream),
        Silence::of(policy.read_timeout()),
    ));

    open_session(client, &account)
        .await
        .unwrap_or_else(|error| panic!("{address} could not sign in: {error}"))
}

#[test]
fn every_line_ends_the_way_imap_ends_one_and_no_line_ends_twice() {
    assert_eq!(with_crlf(b"a\nb\n"), b"a\r\nb\r\n");
    assert_eq!(with_crlf(b"a\r\nb\r\n"), b"a\r\nb\r\n", "already ended");
    assert_eq!(
        with_crlf(b"a\r\nb\nc"),
        b"a\r\nb\r\nc",
        "mixed, no last end"
    );
    assert_eq!(with_crlf(b"\n\n"), b"\r\n\r\n", "empty lines");
    assert_eq!(with_crlf(b""), b"");
}

#[test]
#[should_panic(expected = "is not a test name a mailbox can be called by")]
fn a_name_that_could_reach_another_directory_is_refused_before_anything_is_removed() {
    // Starting a mailbox over removes the directory its name leads to, so a name that climbs out of
    // it must stop here, before the server is asked anything.
    holding_the_corpus("x/../../everyone");
}

#[test]
fn a_mailbox_holds_the_corpus_only_exactly_as_filling_it_leaves_it() {
    let corpus: Vec<Vec<u8>> = ["one\r\n", "two\r\n", "three\r\n"]
        .iter()
        .map(|message| message.as_bytes().to_vec())
        .collect();
    let inbox = |next: Option<u32>, messages: &[(u32, &str)]| Inbox {
        generation: UidValidity::new(1).expect("non-zero"),
        next,
        messages: messages
            .iter()
            .map(|(uid, raw)| (Uid::new(*uid).expect("non-zero"), raw.as_bytes().to_vec()))
            .collect(),
    };
    let (one, two, three) = ("one\r\n", "two\r\n", "three\r\n");

    assert!(
        holds_the_corpus(&inbox(Some(4), &[(1, one), (2, two), (3, three)]), &corpus),
        "as a fresh fill leaves it"
    );

    let refused = [
        (
            "a file edited to the same length",
            inbox(Some(4), &[(1, one), (2, "twO\r\n"), (3, three)]),
        ),
        (
            "one message more",
            inbox(Some(5), &[(1, one), (2, two), (3, three), (4, "four\r\n")]),
        ),
        ("one message fewer", inbox(Some(4), &[(1, one), (2, two)])),
        (
            "emptied and filled again, so numbered higher",
            inbox(Some(7), &[(4, one), (5, two), (6, three)]),
        ),
        (
            "a message added and taken away since",
            inbox(Some(5), &[(1, one), (2, two), (3, three)]),
        ),
        (
            "in another order",
            inbox(Some(4), &[(1, two), (2, one), (3, three)]),
        ),
        ("empty", inbox(Some(1), &[])),
        (
            "no next number stated",
            inbox(None, &[(1, one), (2, two), (3, three)]),
        ),
    ];
    for (case, held) in &refused {
        assert!(
            !holds_the_corpus(held, &corpus),
            "taken for the corpus: {case}"
        );
    }
}

#[test]
fn a_mailbox_is_filled_with_the_corpus_once_and_started_over_when_it_holds_anything_else() {
    if !crate::dev_mail_server::answering() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    use super::actions::carry_out_on;
    use crate::ports::mail_actions::ServerAction;
    use async_imap::types::Flag;
    use lyonoyl_domain::message::Flags;

    let corpus = corpus();
    let numbered: Vec<Uid> = (1..=corpus.len())
        .map(|uid| Uid::new(uid as u32).expect("non-zero"))
        .collect();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a runtime");

    // Whatever an earlier run left, it holds the corpus afterwards, numbered from the first UID.
    let first = holding_the_corpus("fixture-check");
    let address = first.address.as_str();
    assert_eq!(first.uids, numbered);

    let inbox = || runtime.block_on(inbox_of(address));
    let act = |action: fn(Uid, UidValidity) -> ServerAction<'static>| {
        runtime.block_on(async {
            let mut session = signed_in(address).await;
            let held = holding(&mut session).await;
            carry_out_on(&mut session, action(held.messages[0].0, held.generation))
                .await
                .expect("the server acts on the first message");
            let _ = session.logout().await;
        })
    };
    // A mark on the first message: whatever is answered from as it stands keeps it, and a mailbox
    // filled again does not. Not something the fixture looks at, so it cannot answer for it.
    let mark = || {
        act(|uid, uid_validity| ServerAction::SetFlags {
            mailbox: "INBOX",
            uid,
            uid_validity,
            add: Flags::FLAGGED,
            remove: Flags::NONE,
        })
    };
    let marked = || {
        runtime.block_on(async {
            let mut session = signed_in(address).await;
            session.select("INBOX").await.expect("the inbox opens");
            let fetched: Vec<async_imap::types::Fetch> = session
                .uid_fetch("1", "FLAGS")
                .await
                .expect("the server answers")
                .try_collect()
                .await
                .expect("the server answers");
            let _ = session.logout().await;
            fetched
                .iter()
                .any(|message| message.flags().any(|flag| matches!(flag, Flag::Flagged)))
        })
    };

    // Asked again, it is answered from as it stands: nothing filled, nothing renumbered.
    mark();
    let again = holding_the_corpus("fixture-check");
    assert_eq!(again.uids, numbered);
    assert!(marked(), "a mailbox holding the corpus was filled again");

    // One message more than the corpus: started over, so the extra one is gone and the corpus is
    // back at the first UID rather than above every number the mailbox had handed out.
    runtime.block_on(async {
        let mut session = signed_in(address).await;
        session
            .append(
                "INBOX",
                None,
                None,
                with_crlf(b"Subject: extra\n\nNot in the corpus.\n"),
            )
            .await
            .expect("the extra message is taken");
        let _ = session.logout().await;
    });
    let refilled = holding_the_corpus("fixture-check");
    assert_eq!(refilled.uids, numbered, "filled again higher up");
    assert!(!marked(), "answered from with the extra message in it");
    let held = inbox();
    assert_eq!(
        held.messages.len(),
        corpus.len(),
        "the extra message is there"
    );
    assert_eq!(held.next, Some(corpus.len() as u32 + 1));

    // One message fewer: filled again too, rather than taken for the corpus.
    act(|uid, uid_validity| ServerAction::Erase {
        mailbox: "INBOX",
        uid,
        uid_validity,
    });
    let topped_up = holding_the_corpus("fixture-check");
    assert_eq!(topped_up.uids, numbered, "a message is missing");
    assert_eq!(inbox().messages.len(), corpus.len());

    // A corpus file edited to the same length, which is how an address is scrubbed: the mailbox
    // holding the earlier bytes is filled with the new ones.
    let mut edited = corpus.clone();
    let letter = edited[0]
        .iter()
        .rposition(u8::is_ascii_alphabetic)
        .expect("the first message has a letter in it");
    edited[0][letter] ^= 0x20;
    assert_eq!(edited[0].len(), corpus[0].len());
    assert_ne!(edited[0], corpus[0]);

    mark();
    assert_eq!(filled(&runtime, address, &edited).uids(), numbered);
    assert_eq!(
        inbox().messages[0].1,
        edited[0],
        "a file edited to the same length was taken for the one it was"
    );
    assert!(!marked());

    // And back: holding the edited bytes, it is filled with the corpus as it is in the tree.
    holding_the_corpus("fixture-check");
    assert_eq!(inbox().messages[0].1, corpus[0]);
}
