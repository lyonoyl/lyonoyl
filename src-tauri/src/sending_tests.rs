//! Sending, against the development mail server: a message handed to its submission port, and what
//! arrived in the recipient's inbox read back over IMAP.
//!
//! Everything below the command is otherwise a fake, and a fake answers the way it was written to.
//! What only a real server shows is whether the message this application builds survives a real
//! conversation whole — its encoded subject, its attachment, its name — and whether a refusal
//! comes back as the error the outbox acts on. The adapters here are the production ones; the
//! credential store is the one part left out, because the password is handed over directly.

use std::time::{Duration, Instant};

use mail_parser::MimeHeaders;
use secrecy::SecretString;

use crate::application::deliver_outbox::{DeliverOutbox, Delivered};
use crate::application::keep_draft::KeepDraft;
use crate::application::send_message::SendMessage;
use crate::infrastructure::attaching::FilesBeingAttached;
use crate::infrastructure::imap::actions::ImapMailActions;
use crate::infrastructure::imap::{development_endpoint, imap_account, ImapProvider};
use crate::infrastructure::secure_storage::database_key::DatabaseKey;
use crate::infrastructure::smtp::SmtpSubmission;
use crate::infrastructure::sqlite::account_repository::SqliteAccountRepository;
use crate::infrastructure::sqlite::connection::{open_profile, SqlitePool};
use crate::infrastructure::sqlite::drafts::SqliteDrafts;
use crate::infrastructure::sqlite::migrations::migrate_profile;
use crate::infrastructure::sqlite::outbox::SqliteOutbox;
use crate::ports::account_repository::AccountRepository;
use crate::ports::credential::Credential;
use crate::ports::drafts::{Draft, Drafts};
use crate::ports::mail_provider::MailFetch;
use crate::ports::mail_submission::{MailSubmission, SubmissionError};
use crate::ports::outbox::Outbox;
use lyonoyl_domain::account::{
    Encryption, MailAccount, MailAccountSummary, ServerSettings, Submission,
};
use lyonoyl_domain::message::{Uid, UidValidity};
use lyonoyl_domain::outgoing::{EmailAddress, Outgoing, OutgoingFile, Subject};

/// The password every account on the development server takes.
const PASSWORD: &str = "lyonoyl";

/// The container the development server's mailboxes live in.
const SERVER: &str = "lyonoyl-mail-server";

/// The container that takes mail to send, whose log says what it refused.
const SUBMITTING: &str = "lyonoyl-mail-submission";

/// How long a message accepted for delivery may take to reach an inbox before it is called lost.
///
/// Delivery is a queue and an LMTP conversation on the same machine, measured at well under a
/// second. The margin is for a machine busy building something else.
const ARRIVAL: Duration = Duration::from_secs(30);

/// Whether both halves of the server are there. Each test sends and then reads.
fn server_is_running() -> bool {
    crate::dev_mail_server::submission_answering()
}

fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a runtime for the tests")
    })
}

fn password(which: &str) -> Credential {
    Credential::password(SecretString::from(which.to_owned()))
}

/// An account on the development server that can send, as a person would set it up.
fn development_account(id: &str, address: &str) -> MailAccount {
    let reading = development_endpoint();
    let (host, port) = crate::dev_mail_server::SUBMISSION;

    MailAccount::new(
        MailAccountSummary::new(id, address, "", "imap").expect("valid summary"),
        ServerSettings::new(reading.host, reading.port, Encryption::None).expect("valid server"),
        address,
    )
    .expect("valid account")
    .sending_through(
        Submission::new(
            ServerSettings::new(host, port, Encryption::None).expect("valid server"),
            address,
        )
        .expect("valid submission"),
    )
}

fn submission_for(account: &MailAccount, credential: Credential) -> SmtpSubmission {
    SmtpSubmission::new(
        account.submission().expect("the account sends"),
        credential,
        runtime().handle().clone(),
    )
}

/// A name for one message that no other run, and no other test in this one, will use.
fn unique() -> String {
    format!("sending-{}", uuid::Uuid::new_v4().simple())
}

fn address(value: &str) -> EmailAddress {
    EmailAddress::parse(value).expect("a valid address")
}

/// Bytes a text encoding would damage: every value a byte can hold, line breaks and zeroes among
/// them. What comes back has to be these, byte for byte.
fn a_binary_file() -> Vec<u8> {
    (0..=255u8).cycle().take(4096).collect()
}

/// A profile database of its own, in a directory that is removed afterwards.
struct Profile {
    dir: std::path::PathBuf,
    pool: SqlitePool,
}

impl Profile {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "lyonoyl-sending-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).expect("a temporary directory");

        let path = dir.join("profile.sqlite3");
        let pool = open_profile(&path, &DatabaseKey::for_a_test()).expect("the profile opens");
        let mut connection = pool.get().expect("a connection");
        migrate_profile(&mut connection, &path.display().to_string()).expect("migrations run");
        drop(connection);

        Self { dir, pool }
    }

    /// What the outbox row holding this message says about it.
    fn state_of(&self, message_id: &str) -> String {
        self.pool
            .get()
            .expect("a connection")
            .query_row(
                "SELECT state FROM outbox WHERE message_id = ?1",
                [message_id],
                |row| row.get(0),
            )
            .expect("the message has a row")
    }

    /// Moves the clock past the wait a copy refused for now was given, the way time passing would.
    fn wait_is_over(&self, message_id: &str) {
        self.pool
            .get()
            .expect("a connection")
            .execute(
                "UPDATE outbox SET file_after = unixepoch() WHERE message_id = ?1",
                [message_id],
            )
            .expect("the clock moves on");
    }

    /// What the outbox holds of every message in it: their rows, everybody they go to, and the
    /// files they carry.
    fn left(&self) -> (i64, i64, i64) {
        let connection = self.pool.get().expect("a connection");
        let rows = |table: &str| -> i64 {
            connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .expect("countable")
        };
        (
            rows("outbox"),
            rows("outbox_recipients"),
            rows("outbox_files"),
        )
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// Runs `doveadm` inside the development server, which is how a test finds a message by its name.
fn on_the_server(arguments: &[&str]) -> String {
    let output = std::process::Command::new("docker")
        .arg("exec")
        .arg(SERVER)
        .arg("doveadm")
        .args(arguments)
        .output()
        .expect("docker is available");

    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The numbers of the messages in an inbox that carry this name, as the server has them now.
fn carrying(recipient: &str, message_id: &str) -> Vec<Uid> {
    let listing = on_the_server(&[
        "search",
        "-u",
        recipient,
        "mailbox",
        "INBOX",
        "HEADER",
        "Message-ID",
        message_id,
    ]);

    // One line per message: the mailbox's identifier, then the number.
    listing
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1)?.parse::<u32>().ok())
        .filter_map(|number| Uid::new(number).ok())
        .collect()
}

/// Waits for a message to arrive, and answers with where it is.
fn arrival(recipient: &str, message_id: &str) -> Uid {
    let until = Instant::now() + ARRIVAL;

    loop {
        if let Some(uid) = carrying(recipient, message_id).first() {
            return *uid;
        }
        assert!(
            Instant::now() < until,
            "{message_id} did not arrive in the inbox of {recipient} within {ARRIVAL:?}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The message as the recipient's own IMAP login reads it.
///
/// Through the production adapter rather than through `doveadm`: the claim is that the recipient
/// can read it, and the recipient reads over IMAP.
fn read_by(recipient: &str, uid: Uid) -> Vec<u8> {
    let reader = development_account("reader", recipient);

    ImapProvider::new(
        imap_account(&reader, &password(PASSWORD)),
        runtime().handle().clone(),
    )
    .expect("the provider builds")
    .whole_message("INBOX", generation_of(recipient), uid)
    .expect("the recipient reads the message")
}

/// The generation the numbers in a recipient's inbox count in, as the server has it now.
fn generation_of(recipient: &str) -> UidValidity {
    // One line: the mailbox, then `uidvalidity=` and the number.
    on_the_server(&["mailbox", "status", "-u", recipient, "uidvalidity", "INBOX"])
        .split_whitespace()
        .find_map(|word| word.strip_prefix("uidvalidity="))
        .and_then(|number| number.parse::<u32>().ok())
        .and_then(|number| UidValidity::new(number).ok())
        .unwrap_or_else(|| panic!("the server states no generation for {recipient}'s inbox"))
}

/// Takes a test's own messages out again, so the inboxes it uses do not grow run after run.
fn remove(recipient: &str, message_id: &str) {
    on_the_server(&[
        "expunge",
        "-u",
        recipient,
        "mailbox",
        "INBOX",
        "HEADER",
        "Message-ID",
        message_id,
    ]);
}

/// The `Received` headers of a message, each unfolded onto one line.
///
/// A server folds a long header across lines, and what a test looks for can sit on either side of
/// a fold.
fn received_headers(raw: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(raw).replace("\r\n", "\n");
    let head = text.split("\n\n").next().unwrap_or_default();

    head.replace("\n ", " ")
        .replace("\n\t", " ")
        .lines()
        .filter(|line| line.starts_with("Received:"))
        .map(str::to_owned)
        .collect()
}

fn written_by_zeta(to: &[&str], bcc: &[&str], subject: &str) -> Outgoing {
    Outgoing {
        from: address("zeta@example.com"),
        to: to.iter().map(|one| address(one)).collect(),
        cc: Vec::new(),
        bcc: bcc.iter().map(|one| address(one)).collect(),
        subject: Subject::parse(subject).expect("a subject"),
        body: "Ζήτα, Κάππα, Σίγμα.\nLine two, in plain ASCII.".to_owned(),
        thread: Default::default(),
        files: vec![OutgoingFile {
            name: "ledger.bin".to_owned(),
            content_type: "application/octet-stream".to_owned(),
            bytes: a_binary_file(),
        }],
    }
}

#[test]
fn a_message_handed_to_the_development_server_arrives_whole_in_the_recipients_inbox() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    // The cast written in another script: a subject a server has to carry encoded, and a reader
    // has to decode back to exactly this.
    let subject = "Ζήτα Κουόριν, Κάππα Βάντελ, Σίγμα Όρντριν";
    let message = written_by_zeta(&["kappa@example.net"], &["sigma@example.org"], subject);
    let name = unique();
    let message_id = format!("<{name}@example.com>");

    let zeta = development_account("acc-zeta", "zeta@example.com");
    submission_for(&zeta, password(PASSWORD))
        .submit(&message_id, &message, 1_700_000_000)
        .expect("the development server takes the message");

    let raw = read_by(
        "kappa@example.net",
        arrival("kappa@example.net", &message_id),
    );
    let parsed = mail_parser::MessageParser::default()
        .parse(&raw)
        .expect("what arrived is a message");

    assert_eq!(
        parsed.message_id().map(|id| format!("<{id}>")),
        Some(message_id.clone()),
        "the message arrived under a name it was not given"
    );
    assert_eq!(
        parsed.from().and_then(|from| from.first()?.address()),
        Some("zeta@example.com")
    );
    assert_eq!(
        parsed.to().and_then(|to| to.first()?.address()),
        Some("kappa@example.net")
    );
    assert_eq!(
        parsed.subject(),
        Some(subject),
        "the subject did not survive"
    );
    assert_eq!(
        parsed
            .body_text(0)
            .map(|body| body.replace("\r\n", "\n").trim_end().to_owned()),
        Some("Ζήτα, Κάππα, Σίγμα.\nLine two, in plain ASCII.".to_owned()),
        "the text did not survive"
    );

    let files: Vec<_> = parsed.attachments().collect();
    assert_eq!(files.len(), 1, "one file went, and one file arrived");
    assert_eq!(files[0].attachment_name(), Some("ledger.bin"));
    assert!(
        files[0].contents() == a_binary_file().as_slice(),
        "the file arrived with different bytes from the ones attached"
    );

    // The blind copy went where the envelope said, and the copy everybody else reads does not say
    // who else got one.
    let blind = read_by(
        "sigma@example.org",
        arrival("sigma@example.org", &message_id),
    );
    assert!(!blind.is_empty(), "the blind copy did not arrive");
    assert!(parsed.bcc().is_none(), "the message names its blind copies");
    for listed in [parsed.to(), parsed.cc()].into_iter().flatten() {
        assert!(
            listed
                .iter()
                .all(|one| one.address() != Some("sigma@example.org")),
            "the blind copy is named in a header everybody reads"
        );
    }

    remove("kappa@example.net", &message_id);
    remove("sigma@example.org", &message_id);
}

#[test]
fn what_arrives_does_not_name_the_machine_it_was_sent_from() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    // What the system calls this machine, which is what the library says when left to itself. A
    // machine named after something every message carries would fail this for the wrong reason;
    // a development machine's name is its own.
    let machine = std::process::Command::new("hostname")
        .output()
        .map(|output| {
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .to_lowercase()
        })
        .expect("the system says what this machine is called");
    assert!(!machine.is_empty(), "the system calls this machine nothing");

    let message = written_by_zeta(&["kappa@example.net"], &[], "Where this came from");
    let message_id = format!("<{}@example.com>", unique());

    let zeta = development_account("acc-zeta", "zeta@example.com");
    submission_for(&zeta, password(PASSWORD))
        .submit(&message_id, &message, 1_700_000_000)
        .expect("the development server takes the message");

    let raw = read_by(
        "kappa@example.net",
        arrival("kappa@example.net", &message_id),
    );
    remove("kappa@example.net", &message_id);
    let received = received_headers(&raw);

    // The submission server writes down the greeting it was given, as the first word after `from`.
    assert!(
        received
            .iter()
            .any(|header| header.starts_with("Received: from [127.0.0.1] ")),
        "the server did not write down the greeting this client gave: {received:#?}"
    );
    assert!(
        !String::from_utf8_lossy(&raw)
            .to_lowercase()
            .contains(&machine),
        "the message every recipient reads names the machine it was sent from: {received:#?}"
    );
}

#[test]
fn a_message_written_down_in_the_outbox_is_handed_over_once_and_arrives() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    let profile = Profile::new("outbox");
    let zeta = development_account("acc-zeta", "zeta@example.com");
    SqliteAccountRepository::new(profile.pool.clone())
        .save(&zeta)
        .expect("the account is stored");

    // Written down the way the composer's command writes it: through the use case, into the
    // profile, under a name decided before anything is sent.
    let message = written_by_zeta(&["kappa@example.net"], &[], "From the outbox");
    let message_id = SendMessage::new(
        SqliteOutbox::new(profile.pool.clone()),
        SqliteAccountRepository::new(profile.pool.clone()),
    )
    .execute("acc-zeta", &message, &unique())
    .expect("the message is written down");

    let deliver = || -> Delivered {
        DeliverOutbox::new(
            SqliteOutbox::new(profile.pool.clone()),
            submission_for(&zeta, password(PASSWORD)),
            ImapMailActions::new(
                imap_account(&zeta, &password(PASSWORD)),
                runtime().handle().clone(),
            )
            .expect("the acting connection builds"),
            SqliteDrafts::new(profile.pool.clone()),
        )
        .execute("acc-zeta")
        .expect("the round runs")
    };

    let first = deliver();
    assert_eq!(first.sent, 1, "{first:?}");
    assert_eq!((first.waiting, first.given_back), (0, 0));

    // Accepted, and no longer offered for sending.
    assert_eq!(profile.state_of(&message_id), "sent");
    assert!(
        SqliteOutbox::new(profile.pool.clone())
            .ready("acc-zeta", 10)
            .expect("readable")
            .is_empty(),
        "a message the server took is still waiting to be sent"
    );

    let raw = read_by(
        "kappa@example.net",
        arrival("kappa@example.net", &message_id),
    );
    let parsed = mail_parser::MessageParser::default()
        .parse(&raw)
        .expect("what arrived is a message");
    assert_eq!(
        parsed.message_id().map(|id| format!("<{id}>")),
        Some(message_id.clone()),
        "the message arrived under a name the outbox did not give it"
    );
    assert_eq!(parsed.subject(), Some("From the outbox"));

    // Asked again, there is nothing to send: the second copy somebody reads is the fault the
    // outbox exists to prevent.
    let second = deliver();
    assert_eq!(second.sent, 0, "{second:?}");
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(
        carrying("kappa@example.net", &message_id).len(),
        1,
        "the message arrived more than once"
    );

    remove("kappa@example.net", &message_id);
}

#[test]
fn a_password_the_server_refuses_is_said_once_and_the_message_waits_for_a_person() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    // The adapter first: the server's answer to a wrong password is about the account, and it has
    // to arrive as that rather than as a server that could not be reached — which is retried.
    let zeta = development_account("acc-zeta", "zeta@example.com");
    let refused = submission_for(&zeta, password("not-the-password")).submit(
        "<wrong-password@example.com>",
        &written_by_zeta(&["kappa@example.net"], &[], "Never sent"),
        1_700_000_000,
    );
    assert_eq!(refused, Err(SubmissionError::SignInFailed));

    // And the outbox after it: given back to the drafts, where the person decides, and nothing of
    // it left in the outbox to be offered again.
    let profile = Profile::new("wrong-password");
    SqliteAccountRepository::new(profile.pool.clone())
        .save(&zeta)
        .expect("the account is stored");

    let message = written_by_zeta(&["kappa@example.net"], &[], "Held for a person");
    let message_id = SendMessage::new(
        SqliteOutbox::new(profile.pool.clone()),
        SqliteAccountRepository::new(profile.pool.clone()),
    )
    .execute("acc-zeta", &message, &unique())
    .expect("the message is written down");

    let round = DeliverOutbox::new(
        SqliteOutbox::new(profile.pool.clone()),
        submission_for(&zeta, password("not-the-password")),
        ImapMailActions::new(
            imap_account(&zeta, &password(PASSWORD)),
            runtime().handle().clone(),
        )
        .expect("the acting connection builds"),
        SqliteDrafts::new(profile.pool.clone()),
    )
    .execute("acc-zeta")
    .expect("the round runs");

    assert_eq!(
        (round.sent, round.waiting, round.given_back),
        (0, 0, 1),
        "{round:?}"
    );
    assert!(
        SqliteOutbox::new(profile.pool.clone())
            .ready("acc-zeta", 10)
            .expect("readable")
            .is_empty(),
        "a message refused for its password is offered again on a timer"
    );

    let drafts = SqliteDrafts::new(profile.pool.clone());
    let given_back = drafts
        .all("acc-zeta")
        .expect("readable")
        .into_iter()
        .find(|kept| kept.draft.subject == "Held for a person")
        .expect("the message was not given back to be sent again");
    assert_eq!(
        drafts
            .files(given_back.id)
            .expect("readable")
            .into_iter()
            .map(|one| (*one.file).clone())
            .collect::<Vec<_>>(),
        message.files,
        "the message came back without the file it carried"
    );
    assert_eq!(
        profile.left(),
        (0, 0, 0),
        "the outbox kept part of a message the drafts hold"
    );

    assert!(
        carrying("kappa@example.net", &message_id).is_empty(),
        "a message sent with the wrong password arrived"
    );
}

#[test]
fn a_recipient_outside_the_fixture_is_refused_and_nobody_is_sent_anything() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    // The fixture relays nowhere, and the refusal comes while recipients are still being named —
    // before the message is written, so the recipient it would have reached gets nothing either.
    // The outsider's name is this run's own, so the server's log can be asked about this refusal
    // and no other.
    let outsider = format!("{}@elsewhere.invalid", unique());
    let message = written_by_zeta(&["kappa@example.net", &outsider], &[], "Going nowhere");
    let message_id = format!("<{}@example.com>", unique());

    let zeta = development_account("acc-zeta", "zeta@example.com");
    let outcome =
        submission_for(&zeta, password(PASSWORD)).submit(&message_id, &message, 1_700_000_000);

    assert_eq!(outcome, Err(SubmissionError::Refused));

    // Refused by the server, and not by this application before it got there: the adapter says
    // the same word for a plain-text server it will not sign in to, and that refusal would pass
    // the assertion above without a conversation ever taking place.
    let said = format!("<{outsider}>: Relay access denied");
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let log = std::process::Command::new("docker")
            .args(["logs", "--since", "5m", SUBMITTING])
            .output()
            .expect("docker is available");
        if String::from_utf8_lossy(&log.stdout).contains(&said) {
            break;
        }
        assert!(
            Instant::now() < until,
            "the submission server never refused {outsider}, so it was never asked"
        );
        std::thread::sleep(Duration::from_millis(200));
    }

    std::thread::sleep(Duration::from_secs(1));
    assert!(
        carrying("kappa@example.net", &message_id).is_empty(),
        "half a refused message was delivered"
    );
}

/// A submission server that counts what it is handed and takes all of it. No network is involved.
#[derive(Default)]
struct Counting {
    handed: std::sync::Mutex<u32>,
}

impl MailSubmission for Counting {
    fn submit(
        &self,
        _message_id: &str,
        _message: &Outgoing,
        _written_at: i64,
    ) -> Result<(), SubmissionError> {
        *self.handed.lock().expect("test lock") += 1;
        Ok(())
    }
}

/// The folder for sent mail, which a round that sends nothing never asks about.
struct NothingToFile;

impl crate::ports::mail_actions::SentMail for NothingToFile {
    fn file(
        &self,
        _message_id: &str,
        _message: &Outgoing,
        _written_at: i64,
        _refused_search: crate::ports::mail_actions::RefusedSearch,
    ) -> Result<crate::ports::mail_actions::Filing, crate::ports::mail_actions::FilingError> {
        unreachable!("nothing was sent, so nothing is filed")
    }
}

#[test]
fn a_message_a_round_took_and_never_finished_is_given_back_once_and_never_sent_again() {
    // Needs no server: what is asserted is that nothing reaches one. A round took the message and
    // the machine stopped while a server was taking it, so the row still says it was on its way.
    // Every round after that finds it there, and none of them may hand it over: the server may
    // already have it, and a second copy is the one thing the outbox exists to prevent.
    let profile = Profile::new("interrupted");
    let zeta = development_account("acc-zeta", "zeta@example.com");
    SqliteAccountRepository::new(profile.pool.clone())
        .save(&zeta)
        .expect("the account is stored");

    let message = written_by_zeta(&["kappa@example.net"], &[], "Taken and never finished");
    SendMessage::new(
        SqliteOutbox::new(profile.pool.clone()),
        SqliteAccountRepository::new(profile.pool.clone()),
    )
    .execute("acc-zeta", &message, &unique())
    .expect("the message is written down");

    let outbox = SqliteOutbox::new(profile.pool.clone());
    let id = outbox.ready("acc-zeta", 10).expect("readable")[0].id;
    assert_eq!(outbox.claim(id), Ok(true), "the round that stopped took it");

    let server = Counting::default();
    let round = || -> Delivered {
        DeliverOutbox::new(
            SqliteOutbox::new(profile.pool.clone()),
            &server,
            NothingToFile,
            SqliteDrafts::new(profile.pool.clone()),
        )
        .execute("acc-zeta")
        .expect("the round runs")
    };

    let first = round();
    let second = round();

    assert_eq!(
        *server.handed.lock().expect("test lock"),
        0,
        "a message that may already have gone was handed over again"
    );
    assert_eq!(first.given_back, 1, "{first:?}");
    assert_eq!(second, Delivered::default(), "{second:?}");

    let drafts = SqliteDrafts::new(profile.pool.clone());
    let came_back: Vec<_> = drafts
        .all("acc-zeta")
        .expect("readable")
        .into_iter()
        .filter(|kept| kept.draft.subject == "Taken and never finished")
        .collect();
    assert_eq!(
        came_back.len(),
        1,
        "the message did not come back to the person once"
    );
    assert_eq!(
        drafts
            .files(came_back[0].id)
            .expect("readable")
            .into_iter()
            .map(|one| (*one.file).clone())
            .collect::<Vec<_>>(),
        message.files,
        "the message came back without the file it carried"
    );
    assert_eq!(
        profile.left(),
        (0, 0, 0),
        "the outbox kept part of a message the drafts hold"
    );
}

/// A submission server that answers every message it is handed with the same refusal. No network
/// is involved.
struct Refusing(SubmissionError);

impl MailSubmission for Refusing {
    fn submit(
        &self,
        _message_id: &str,
        _message: &Outgoing,
        _written_at: i64,
    ) -> Result<(), SubmissionError> {
        Err(self.0)
    }
}

#[test]
fn every_message_the_outbox_will_not_try_again_is_left_in_its_draft_and_nowhere_else() {
    // Needs no server: each answer is one a submission server gives. Refused, its account's
    // sign-in refused, unreachable on the outbox's last try, or ended after it was handed over:
    // each goes back to the drafts with its file, the outbox holds nothing of it, not even who it
    // was blind-copied to, and the next round finds nothing to do.
    for answer in [
        SubmissionError::Refused,
        SubmissionError::SignInFailed,
        SubmissionError::Unreachable,
        SubmissionError::NobodyKnows,
    ] {
        let profile = Profile::new("left-in-its-draft");
        let zeta = development_account("acc-zeta", "zeta@example.com");
        SqliteAccountRepository::new(profile.pool.clone())
            .save(&zeta)
            .expect("the account is stored");
        let message = written_by_zeta(&["kappa@example.net"], &["sigma@example.org"], "Not going");
        SendMessage::new(
            SqliteOutbox::new(profile.pool.clone()),
            SqliteAccountRepository::new(profile.pool.clone()),
        )
        .execute("acc-zeta", &message, &unique())
        .expect("the message is written down");
        // Past every wait the outbox gives a server it could not reach.
        profile
            .pool
            .get()
            .expect("a connection")
            .execute("UPDATE outbox SET attempts = 100", [])
            .expect("tried and tried again");
        let round = || {
            DeliverOutbox::new(
                SqliteOutbox::new(profile.pool.clone()),
                Refusing(answer),
                NothingToFile,
                SqliteDrafts::new(profile.pool.clone()),
            )
            .execute("acc-zeta")
            .expect("the round runs")
        };

        let first = round();
        let second = round();

        assert_eq!(
            first,
            Delivered {
                given_back: 1,
                ..Delivered::default()
            },
            "{answer:?}"
        );
        assert_eq!(second, Delivered::default(), "{answer:?}");
        assert_eq!(
            profile.left(),
            (0, 0, 0),
            "{answer:?}: the outbox kept part of a message the drafts hold"
        );
        let drafts = SqliteDrafts::new(profile.pool.clone());
        let back = drafts.all("acc-zeta").expect("readable");
        assert_eq!(back.len(), 1, "{answer:?}");
        assert_eq!(
            (back[0].draft.subject.as_str(), back[0].draft.blind.as_str()),
            ("Not going", "sigma@example.org"),
            "{answer:?}: the message came back changed"
        );
        assert_eq!(
            drafts
                .files(back[0].id)
                .expect("readable")
                .into_iter()
                .map(|one| (*one.file).clone())
                .collect::<Vec<_>>(),
            message.files,
            "{answer:?}: the message came back without the file it carried"
        );
    }
}

#[test]
fn a_message_given_back_by_a_round_that_then_stopped_comes_back_once_with_its_file() {
    // Needs no server: what is handed over goes to one that counts. The round gives back a message
    // an earlier round took and never finished, and then stops -- here, at a disk that refuses to
    // take the next message to hand over. The message is back in the drafts once, with the file it
    // carried, and the outbox holds nothing of it, so the next round has nothing to give back.
    let profile = Profile::new("given-back-then-stopped");
    let zeta = development_account("acc-zeta", "zeta@example.com");
    SqliteAccountRepository::new(profile.pool.clone())
        .save(&zeta)
        .expect("the account is stored");
    let write_down = |message: &Outgoing| {
        SendMessage::new(
            SqliteOutbox::new(profile.pool.clone()),
            SqliteAccountRepository::new(profile.pool.clone()),
        )
        .execute("acc-zeta", message, &unique())
        .expect("the message is written down")
    };
    let message = written_by_zeta(&["kappa@example.net"], &[], "Given back, then stopped");
    write_down(&message);
    let outbox = SqliteOutbox::new(profile.pool.clone());
    let id = outbox.ready("acc-zeta", 10).expect("readable")[0].id;
    assert_eq!(outbox.claim(id), Ok(true), "the round that stopped took it");
    write_down(&written_by_zeta(
        &["kappa@example.net"],
        &[],
        "Waiting behind it",
    ));

    let server = Counting::default();
    let round = || {
        DeliverOutbox::new(
            SqliteOutbox::new(profile.pool.clone()),
            &server,
            NothingToFile,
            SqliteDrafts::new(profile.pool.clone()),
        )
        .execute("acc-zeta")
    };
    let taking = |refused: bool| {
        let change = if refused {
            "CREATE TRIGGER not_taken BEFORE UPDATE OF state ON outbox WHEN NEW.state = 'sending'
             BEGIN SELECT RAISE(ABORT, 'refused'); END;"
        } else {
            "DROP TRIGGER not_taken;"
        };
        profile
            .pool
            .get()
            .expect("a connection")
            .execute_batch(change)
            .expect("the disk is told");
    };

    taking(true);
    assert!(round().is_err(), "the round took the message behind it");
    taking(false);
    let second = round().expect("the round runs");

    assert_eq!((second.given_back, second.sent), (0, 1), "{second:?}");
    let drafts = SqliteDrafts::new(profile.pool.clone());
    let came_back: Vec<_> = drafts
        .all("acc-zeta")
        .expect("readable")
        .into_iter()
        .filter(|kept| kept.draft.subject == "Given back, then stopped")
        .collect();
    assert_eq!(
        came_back.len(),
        1,
        "the message came back twice, or not at all"
    );
    assert_eq!(
        drafts
            .files(came_back[0].id)
            .expect("readable")
            .into_iter()
            .map(|one| (*one.file).clone())
            .collect::<Vec<_>>(),
        message.files,
        "the message came back without the file it carried"
    );
    assert_eq!(
        profile.left(),
        (1, 1, 1),
        "the outbox holds more than the message handed over behind it"
    );
    assert_eq!(*server.handed.lock().expect("test lock"), 1);
}

#[test]
fn a_held_message_that_names_its_draft_goes_with_the_next_round_and_one_that_names_none_stays() {
    // Needs no server: nothing is waiting to go. Two messages a profile holds for a person, each
    // carrying a file in the outbox's own rows: one whose row names the draft it came back as,
    // which holds it, and one whose row names none, whose file may be nowhere but here.
    let profile = Profile::new("held-before");
    let zeta = development_account("acc-zeta", "zeta@example.com");
    SqliteAccountRepository::new(profile.pool.clone())
        .save(&zeta)
        .expect("the account is stored");
    let outbox = SqliteOutbox::new(profile.pool.clone());
    let mut rows = Vec::new();
    for subject in ["Named", "Named by nothing"] {
        let message_id = SendMessage::new(
            SqliteOutbox::new(profile.pool.clone()),
            SqliteAccountRepository::new(profile.pool.clone()),
        )
        .execute(
            "acc-zeta",
            &written_by_zeta(&["kappa@example.net"], &["sigma@example.org"], subject),
            &unique(),
        )
        .expect("the message is written down");
        let id = outbox.ready("acc-zeta", 10).expect("readable")[0].id;
        outbox.claim(id).expect("taken");
        outbox
            .stopped(id, crate::ports::outbox::WhyHeld::SignInFailed, None)
            .expect("held for a person");
        rows.push((id, message_id));
    }
    let drafts = SqliteDrafts::new(profile.pool.clone());
    let draft = drafts
        .keep(
            "acc-zeta",
            None,
            &Draft {
                subject: "Named".to_owned(),
                ..Draft::default()
            },
            &[],
        )
        .expect("the draft it came back as");
    profile
        .pool
        .get()
        .expect("a connection")
        .execute(
            "UPDATE outbox SET came_back_as = ?2 WHERE id = ?1",
            [rows[0].0, draft],
        )
        .expect("the row names it");

    let done = DeliverOutbox::new(
        SqliteOutbox::new(profile.pool.clone()),
        Counting::default(),
        NothingToFile,
        SqliteDrafts::new(profile.pool.clone()),
    )
    .execute("acc-zeta")
    .expect("the round runs");

    assert_eq!(done, Delivered::default(), "{done:?}");
    assert_eq!(
        profile.left(),
        (1, 2, 1),
        "the row that names its draft stayed, or the one that names none went"
    );
    assert_eq!(profile.state_of(&rows[1].1), "held");
    assert_eq!(
        drafts.all("acc-zeta").expect("readable").len(),
        1,
        "a draft was written or let go"
    );
}

#[test]
fn a_message_sent_from_a_draft_carries_its_files_and_leaves_none_of_them_behind() {
    // Needs no server: what is asserted is what this device holds once the message is written
    // down and its draft let go, which is all that sending from a draft does here. The draft is
    // kept carrying a file, opened again the way a composer opens it — under a fresh token — and
    // sent the way a composer sends: the tokens on screen, then the draft let go.
    let profile = Profile::new("from-a-draft");
    let zeta = development_account("acc-zeta", "zeta@example.com");
    SqliteAccountRepository::new(profile.pool.clone())
        .save(&zeta)
        .expect("the account is stored");
    let written = written_by_zeta(&["kappa@example.net"], &[], "From a draft");
    let drafts = SqliteDrafts::new(profile.pool.clone());

    let attached = FilesBeingAttached::default();
    attached.hold("t1".to_owned(), written.files[0].clone());
    let draft_id = KeepDraft::new(&drafts)
        .execute(
            "acc-zeta",
            None,
            Draft {
                recipients: "kappa@example.net".to_owned(),
                subject: "From a draft".to_owned(),
                ..Draft::default()
            },
            &attached.to_keep(&["t1".to_owned()]).expect("it is held"),
        )
        .expect("the draft is kept with its file");

    let opened = FilesBeingAttached::default();
    let mut tokens = Vec::new();
    for one in drafts.files(draft_id).expect("readable") {
        let token = unique();
        opened.hold_kept(token.clone(), one.kept_as, one.file);
        tokens.push(token);
    }
    let message = Outgoing {
        files: opened.take(&tokens).expect("the draft's files are held"),
        ..written.clone()
    };
    let message_id = SendMessage::new(
        SqliteOutbox::new(profile.pool.clone()),
        SqliteAccountRepository::new(profile.pool.clone()),
    )
    .execute("acc-zeta", &message, &unique())
    .expect("the message is written down");
    drafts.discard(draft_id).expect("the draft is let go");

    let queued = SqliteOutbox::new(profile.pool.clone())
        .ready("acc-zeta", 10)
        .expect("readable");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].message_id, message_id);
    assert_eq!(
        queued[0].message.files, written.files,
        "the message went without the file its draft carried"
    );
    let connection = profile.pool.get().expect("a connection");
    let count = |table: &str| -> i64 {
        connection
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .expect("countable")
    };
    assert_eq!(
        (count("drafts"), count("draft_files")),
        (0, 0),
        "the draft or its file outlived the message"
    );
}

/// What the folder for sent mail answers about one copy.
type Answer = Result<crate::ports::mail_actions::Filing, crate::ports::mail_actions::FilingError>;

/// The folder for sent mail, giving each copy it is asked to keep the next of its answers, in the
/// order it is asked: oldest first.
struct Answers(std::sync::Mutex<std::collections::VecDeque<Answer>>);

impl Answers {
    fn in_turn(answers: impl IntoIterator<Item = Answer>) -> Self {
        Self(std::sync::Mutex::new(answers.into_iter().collect()))
    }
}

impl crate::ports::mail_actions::SentMail for Answers {
    fn file(
        &self,
        _message_id: &str,
        _message: &Outgoing,
        _written_at: i64,
        _refused_search: crate::ports::mail_actions::RefusedSearch,
    ) -> Answer {
        self.0
            .lock()
            .expect("test lock")
            .pop_front()
            .expect("a copy was asked about that this test gave no answer for")
    }
}

/// A message written down and handed to a server, taken longer ago than its copy is looked for.
///
/// Addressed to somebody, copied to somebody else and blind-copied to a third, and carrying a
/// file, so a row of every kind the outbox keeps is there to be let go or to stay. The round that
/// hands it over looks for no copy: a server that files its own is given time to.
fn sent_a_while_ago(profile: &Profile) -> String {
    let message_id = sent_a_while_ago_each(profile, &["Gone"]).remove(0);
    assert_eq!(
        profile.left(),
        (1, 3, 1),
        "the message was not written down whole"
    );

    message_id
}

/// The same for one message under each subject, written in that order, and their names.
fn sent_a_while_ago_each(profile: &Profile, subjects: &[&str]) -> Vec<String> {
    let zeta = development_account("acc-zeta", "zeta@example.com");
    SqliteAccountRepository::new(profile.pool.clone())
        .save(&zeta)
        .expect("the account is stored");

    let names: Vec<String> = subjects
        .iter()
        .map(|subject| {
            let mut message =
                written_by_zeta(&["kappa@example.net"], &["sigma@example.org"], subject);
            message.cc = vec![address("records@example.com")];
            SendMessage::new(
                SqliteOutbox::new(profile.pool.clone()),
                SqliteAccountRepository::new(profile.pool.clone()),
            )
            .execute("acc-zeta", &message, &unique())
            .expect("the message is written down")
        })
        .collect();

    let handed_over = DeliverOutbox::new(
        SqliteOutbox::new(profile.pool.clone()),
        Counting::default(),
        NothingToFile,
        SqliteDrafts::new(profile.pool.clone()),
    )
    .execute("acc-zeta")
    .expect("the round runs");
    assert_eq!(handed_over.sent as usize, names.len(), "{handed_over:?}");

    profile
        .pool
        .get()
        .expect("a connection")
        .execute("UPDATE outbox SET accepted_at = unixepoch() - 61", [])
        .expect("the clock moves on");

    names
}

/// One round, with nothing to send, whose folder for sent mail gives `answer` to every copy it is
/// asked about.
fn a_round_filing(profile: &Profile, answer: Answer) -> Delivered {
    a_round_answering(profile, std::iter::repeat_n(answer, 8))
}

/// One round, with nothing to send, whose folder for sent mail gives the copies it is asked about
/// these answers in turn, and the account read back after it, which lets go of what it filed.
fn a_round_answering(profile: &Profile, answers: impl IntoIterator<Item = Answer>) -> Delivered {
    let done = DeliverOutbox::new(
        SqliteOutbox::new(profile.pool.clone()),
        Counting::default(),
        Answers::in_turn(answers),
        SqliteDrafts::new(profile.pool.clone()),
    )
    .execute("acc-zeta")
    .expect("the round runs");
    read_back(profile, "acc-zeta");
    done
}

/// What reading the account back after a round does to the outbox: what is filed is let go.
fn read_back(profile: &Profile, account_id: &str) {
    SqliteOutbox::new(profile.pool.clone())
        .forget_filed(account_id)
        .expect("let go");
}

#[test]
fn a_sent_message_leaves_nothing_on_this_device_once_its_copy_is_filed_and_read_back() {
    // Needs no server. The copy in the folder for sent mail is the message now, and who it went
    // to, the blind copy among them, and the bytes of its file would otherwise stay in the one
    // database nothing drops, read by nothing.
    let profile = Profile::new("filed-here");
    sent_a_while_ago(&profile);

    let done = a_round_filing(&profile, Ok(crate::ports::mail_actions::Filing::Added));

    assert_eq!(done.filed, 1, "{done:?}");
    assert_eq!(
        profile.left(),
        (0, 0, 0),
        "a sent message was kept after its copy was filed"
    );
}

#[test]
fn a_sent_message_its_server_filed_itself_leaves_nothing_on_this_device() {
    // Found already there by the name it was sent under: filed all the same, by the server.
    let profile = Profile::new("filed-by-the-server");
    sent_a_while_ago(&profile);

    let done = a_round_filing(
        &profile,
        Ok(crate::ports::mail_actions::Filing::AlreadyThere),
    );

    assert_eq!(done.filed, 0, "{done:?}");
    assert_eq!(
        profile.left(),
        (0, 0, 0),
        "a sent message was kept after its copy was found filed"
    );
}

#[test]
fn a_sent_message_with_nowhere_to_file_a_copy_leaves_nothing_on_this_device() {
    // The one way filing ends without a copy: the server names no folder for sent mail, and asking
    // again will not make one appear.
    let profile = Profile::new("filed-nowhere");
    sent_a_while_ago(&profile);

    a_round_filing(&profile, Ok(crate::ports::mail_actions::Filing::Nowhere));

    assert_eq!(
        profile.left(),
        (0, 0, 0),
        "a sent message with nowhere to be filed was kept"
    );
}

#[test]
fn a_sent_message_whose_copy_could_not_be_filed_stays_whole_until_it_is() {
    // The folder for sent mail not reached: nothing was heard about the copy, so the next round
    // asks about it again, and the message waits for it whole. A copy not filed yet may still be,
    // and letting the message go first would leave none.
    use crate::ports::mail_actions::{Filing, FilingError};

    let profile = Profile::new("filing-fails");
    let message_id = sent_a_while_ago(&profile);

    for _ in 0..3 {
        let done = a_round_filing(&profile, Err(FilingError::FolderUnreachable));

        assert_eq!(done, Delivered::default());
        assert_eq!(
            profile.left(),
            (1, 3, 1),
            "a message whose copy was not filed lost part of itself"
        );
        assert_eq!(profile.state_of(&message_id), "sent");
    }

    a_round_filing(&profile, Ok(Filing::Added));
    assert_eq!(profile.left(), (0, 0, 0), "filed at last, and kept");
}

#[test]
fn a_sent_message_whose_copy_is_refused_for_now_stays_whole_and_waits_to_be_asked_again() {
    // Needs no server. Over quota, say: the message waits whole for a copy that may yet be taken,
    // and the rounds that come before its wait is over leave it alone rather than uploading it at
    // every one of them.
    use crate::ports::mail_actions::{Filing, FilingError};

    let profile = Profile::new("filing-refused-for-now");
    let message_id = sent_a_while_ago(&profile);

    let done = a_round_filing(&profile, Err(FilingError::RefusedForNow));
    assert_eq!(done, Delivered::default());
    assert_eq!(
        profile.left(),
        (1, 3, 1),
        "a message whose copy may still be taken lost part of itself"
    );

    // A folder that would take it now is not asked before the wait is over.
    a_round_filing(&profile, Ok(Filing::Added));
    assert_eq!(
        profile.left(),
        (1, 3, 1),
        "the copy was asked about again before its wait was over"
    );

    profile.wait_is_over(&message_id);
    a_round_filing(&profile, Ok(Filing::Added));
    assert_eq!(
        profile.left(),
        (0, 0, 0),
        "a copy taken once its wait was over left its message on this device"
    );
}

#[test]
fn a_sent_message_whose_copy_is_refused_for_good_goes_and_the_one_behind_it_is_filed() {
    // Needs no server. The first is too large for the folder for sent mail and will never be
    // filed; the second is filed in the same round. Neither is kept on this device: a sent message
    // stays here only while there is something left to do with it.
    use crate::ports::mail_actions::{Filing, FilingError};

    let profile = Profile::new("filing-refused-for-good");
    sent_a_while_ago_each(&profile, &["Too large", "Short"]);
    assert_eq!(
        profile.left(),
        (2, 6, 2),
        "the messages were not written down"
    );

    let done = a_round_answering(
        &profile,
        [Err(FilingError::RefusedForGood), Ok(Filing::Added)],
    );

    assert_eq!((done.filed, done.unfiled), (1, 1), "{done:?}");
    assert_eq!(
        profile.left(),
        (0, 0, 0),
        "a message whose copy will never be taken, or the one filed behind it, was kept"
    );
}

#[test]
fn a_copy_refused_for_now_round_after_round_is_let_go_once_its_last_wait_is_over() {
    // Needs no server. The bound, on the rows a round really writes: a copy refused for now waits
    // and is asked again, and refused once more after its last wait it goes the way a copy refused
    // for good does. Each refusal is counted on the row itself, so the count holds across rounds,
    // and across restarts with them.
    use crate::ports::mail_actions::FilingError;

    let profile = Profile::new("filing-refused-too-often");
    let message_id = sent_a_while_ago(&profile);

    let mut refused = 0;
    let done = loop {
        let done = a_round_filing(&profile, Err(FilingError::RefusedForNow));
        refused += 1;
        if profile.left() == (0, 0, 0) {
            break done;
        }

        assert_eq!(
            profile.left(),
            (1, 3, 1),
            "a message whose copy may still be taken lost part of itself"
        );
        assert!(
            refused < 10,
            "a copy refused for now is asked about for ever"
        );
        profile.wait_is_over(&message_id);
    };

    assert!(
        refused > 1,
        "a copy refused for now was not asked about again"
    );
    assert_eq!(done.unfiled, 1, "{done:?}");
}

#[test]
fn a_message_filed_before_a_round_is_kept_until_the_account_is_read_back() {
    // Sent, its copy filed, and every row of it still here: the list of sent mail shows it from
    // that row until the server's copy is read. A round reads nothing, so it keeps the message,
    // and the read lets it go.
    let profile = Profile::new("filed-before");
    let filed = sent_a_while_ago(&profile);
    profile
        .pool
        .get()
        .expect("a connection")
        .execute(
            "UPDATE outbox SET filed_at = unixepoch() WHERE message_id = ?1",
            [&filed],
        )
        .expect("filed, and kept");

    // And one written since, which this round hands over and keeps until its own copy is filed.
    let waiting = SendMessage::new(
        SqliteOutbox::new(profile.pool.clone()),
        SqliteAccountRepository::new(profile.pool.clone()),
    )
    .execute(
        "acc-zeta",
        &written_by_zeta(&["kappa@example.net"], &[], "Still to be filed"),
        &unique(),
    )
    .expect("the message is written down");

    let done = DeliverOutbox::new(
        SqliteOutbox::new(profile.pool.clone()),
        Counting::default(),
        NothingToFile,
        SqliteDrafts::new(profile.pool.clone()),
    )
    .execute("acc-zeta")
    .expect("the round runs");

    assert_eq!(done.sent, 1, "{done:?}");
    assert_eq!(
        profile.left(),
        (2, 4, 2),
        "the round let go of the message filed before the account was read back"
    );

    read_back(&profile, "acc-zeta");

    assert_eq!(
        profile.left(),
        (1, 1, 1),
        "the message filed before was kept, or the one awaiting its copy was not"
    );
    assert_eq!(profile.state_of(&waiting), "sent");
}

#[test]
fn a_copy_whose_lines_read_like_commands_is_filed_once_on_the_development_server() {
    use crate::ports::mail_actions::{Filing, RefusedSearch, SentMail};

    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    // An address of its own, so the folder for sent mail holds this test's copy and nothing else.
    // The message carries lines a guard judging them would refuse — a subject beginning "Move",
    // a line beginning "Please delete" — and it goes through the guard every acting connection
    // has, to a real server that asks for it the way real servers do.
    let user = format!("{}@example.com", unique());
    let account = development_account("acc-filing", &user);
    let keeping = ImapMailActions::new(
        imap_account(&account, &password(PASSWORD)),
        runtime().handle().clone(),
    )
    .expect("the acting connection builds");

    let mut message = written_by_zeta(&["kappa@example.net"], &[], "Move the meeting");
    message.body = "Please delete it\nand everything about it.".to_owned();
    let message_id = format!("<{}@example.com>", unique());

    assert_eq!(
        keeping.file(&message_id, &message, 1_700_000_000, RefusedSearch::Heeded),
        Ok(Filing::Added),
        "the copy was not filed"
    );
    // Asked again, the copy just filed is found by the name the outbox keeps: a server that files
    // its own copy is not given a second one.
    assert_eq!(
        keeping.file(&message_id, &message, 1_700_000_000, RefusedSearch::Heeded),
        Ok(Filing::AlreadyThere),
        "the copy already there was not found, and a second one was added"
    );

    on_the_server(&[
        "expunge",
        "-u",
        &user,
        "mailbox",
        "Sent",
        "HEADER",
        "Message-ID",
        &message_id,
    ]);
}

#[test]
fn a_sent_message_is_let_go_once_its_copy_is_in_sent_and_read_back_on_the_development_server() {
    use crate::ports::mail_actions::{Filing, RefusedSearch, SentMail};

    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    // An address of its own, so the folder for sent mail holds this test's copies and nothing
    // else. Two messages a server has taken: one whose copy the server filed itself, under the
    // name it was sent with, and one whose copy nobody has filed. The round finds the first by
    // that name and files the second, and keeps neither on this device.
    let user = format!("{}@example.com", unique());
    let account = development_account("acc-filing", &user);
    let profile = Profile::new("filed-live");
    SqliteAccountRepository::new(profile.pool.clone())
        .save(&account)
        .expect("the account is stored");

    let written = |subject: &str| -> (String, Outgoing) {
        let mut message = written_by_zeta(&["kappa@example.net"], &["sigma@example.org"], subject);
        message.from = address(&user);
        let message_id = SendMessage::new(
            SqliteOutbox::new(profile.pool.clone()),
            SqliteAccountRepository::new(profile.pool.clone()),
        )
        .execute("acc-filing", &message, &unique())
        .expect("the message is written down");
        (message_id, message)
    };
    let (filed_by_the_server, as_written) = written("Filed by the server");
    let (filed_here, _) = written("Filed here");

    // Handed over without a network, and taken longer ago than a copy is looked for.
    let handed_over = DeliverOutbox::new(
        SqliteOutbox::new(profile.pool.clone()),
        Counting::default(),
        NothingToFile,
        SqliteDrafts::new(profile.pool.clone()),
    )
    .execute("acc-filing")
    .expect("the round runs");
    assert_eq!(handed_over.sent, 2, "{handed_over:?}");
    profile
        .pool
        .get()
        .expect("a connection")
        .execute("UPDATE outbox SET accepted_at = unixepoch() - 61", [])
        .expect("the clock moves on");

    let keeping = || {
        ImapMailActions::new(
            imap_account(&account, &password(PASSWORD)),
            runtime().handle().clone(),
        )
        .expect("the acting connection builds")
    };
    assert_eq!(
        keeping().file(
            &filed_by_the_server,
            &as_written,
            1_700_000_000,
            RefusedSearch::Heeded
        ),
        Ok(Filing::Added),
        "the server's own copy was not put there"
    );

    let done = DeliverOutbox::new(
        SqliteOutbox::new(profile.pool.clone()),
        Counting::default(),
        keeping(),
        SqliteDrafts::new(profile.pool.clone()),
    )
    .execute("acc-filing")
    .expect("the round runs");
    read_back(&profile, "acc-filing");

    assert_eq!(done.filed, 1, "{done:?}");
    assert_eq!(
        profile.left(),
        (0, 0, 0),
        "a sent message was kept after its copy was in the folder for sent mail and read back"
    );
    for message_id in [&filed_by_the_server, &filed_here] {
        let in_sent = [
            "-u",
            &user,
            "mailbox",
            "Sent",
            "HEADER",
            "Message-ID",
            message_id,
        ];
        let copies = on_the_server(&[&["search"][..], &in_sent[..]].concat())
            .lines()
            .count();
        assert_eq!(copies, 1, "{message_id} is in the folder {copies} times");
        on_the_server(&[&["expunge"][..], &in_sent[..]].concat());
    }
}
