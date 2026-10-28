//! Whole-stack check of adding an account: the real use case, the real IMAP adapter against the
//! development mail server, and the real SQLite profile.
//!
//! The credential store is the one part left as a fake. The real one writes to the system
//! keychain, which asks the person running the tests for permission and leaves entries behind.

use std::sync::Mutex;

use secrecy::SecretString;

use crate::application::accounts::{AddAccount, TestConnection};
use crate::infrastructure::imap::corpus_mailbox::holding_the_corpus;
use crate::infrastructure::imap::{development_endpoint, ImapProviderFactory};
use crate::infrastructure::secure_storage::database_key::DatabaseKey;
use crate::infrastructure::sqlite::account_repository::SqliteAccountRepository;
use crate::infrastructure::sqlite::connection::{open_profile, SqlitePool};
use crate::infrastructure::sqlite::migrations::migrate_profile;
use crate::ports::account_repository::AccountRepository;
use crate::ports::credential::Credential;
use crate::ports::credential_store::{CredentialError, CredentialRef, CredentialStore};
use lyonoyl_domain::account::{Encryption, MailAccount, MailAccountSummary, ServerSettings};

/// Whether the development mail server is running. Its tests are skipped when it is not.
fn server_is_running() -> bool {
    crate::dev_mail_server::answering()
}

struct Profile {
    dir: std::path::PathBuf,
    pool: SqlitePool,
}

impl Profile {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("lyonoyl-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("a temporary directory");

        let path = dir.join("profile.sqlite3");
        let pool = open_profile(&path, &DatabaseKey::for_a_test()).expect("the profile opens");
        let mut connection = pool.get().expect("a connection");
        migrate_profile(&mut connection, &path.display().to_string()).expect("migrations run");
        drop(connection);

        Self { dir, pool }
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

/// Shared by handle so the test can read which accounts got a secret.
///
/// The secret itself is never unwrapped here: only the places allowed to send it may do that, and
/// what these tests need to know is that one was stored, under the right account.
#[derive(Default, Clone)]
struct FakeCredentials {
    stored: std::sync::Arc<Mutex<Vec<String>>>,
}

impl CredentialStore for FakeCredentials {
    fn save(
        &self,
        reference: &CredentialRef,
        _secret: &SecretString,
    ) -> Result<(), CredentialError> {
        self.stored
            .lock()
            .expect("test lock")
            .push(reference.account_id.clone());
        Ok(())
    }

    fn read(&self, _reference: &CredentialRef) -> Result<SecretString, CredentialError> {
        Err(CredentialError::NotFound)
    }

    fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError> {
        self.stored
            .lock()
            .expect("test lock")
            .retain(|account| account != &reference.account_id);
        Ok(())
    }
}

fn development_account(id: &str, address: &str) -> MailAccount {
    let endpoint = development_endpoint();

    MailAccount::new(
        MailAccountSummary::new(id, address, "Test", "imap").expect("valid summary"),
        ServerSettings::new(endpoint.host, endpoint.port, Encryption::None).expect("valid server"),
        address,
    )
    .expect("valid account")
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

fn factory() -> ImapProviderFactory {
    ImapProviderFactory::new(runtime().handle().clone())
}

#[test]
fn an_account_is_added_from_the_settings_a_person_would_type() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    let profile = Profile::new("add-account");
    let credentials = FakeCredentials::default();
    let account = development_account("acc-1", "test@lyonoyl.local");

    AddAccount::new(
        SqliteAccountRepository::new(profile.pool.clone()),
        credentials.clone(),
        factory(),
    )
    .execute(account, SecretString::from("lyonoyl".to_owned()))
    .expect("the server accepts the account");

    let stored = SqliteAccountRepository::new(profile.pool.clone())
        .list()
        .expect("the profile can be read");
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].email(), "test@lyonoyl.local");

    let secrets = credentials.stored.lock().expect("test lock");
    assert_eq!(secrets.len(), 1, "the password is stored once");
    assert_eq!(secrets[0], "acc-1", "under the account it belongs to");
}

#[test]
fn a_password_the_server_refuses_leaves_the_profile_empty() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    let profile = Profile::new("refused-account");
    let credentials = FakeCredentials::default();

    AddAccount::new(
        SqliteAccountRepository::new(profile.pool.clone()),
        credentials.clone(),
        factory(),
    )
    .execute(
        development_account("acc-1", "test@lyonoyl.local"),
        SecretString::from("not-the-password".to_owned()),
    )
    .expect_err("the server refuses");

    assert!(
        SqliteAccountRepository::new(profile.pool.clone())
            .list()
            .expect("the profile can be read")
            .is_empty(),
        "an account that cannot sign in must not be stored"
    );
    assert!(
        credentials.stored.lock().expect("test lock").is_empty(),
        "and neither must its password"
    );
}

#[test]
fn the_mailboxes_of_a_real_server_become_the_folders_of_an_account() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    use crate::application::mailboxes::SyncMailboxes;
    use crate::infrastructure::fake_provider::SeededIndex;
    use crate::infrastructure::imap::ImapProviderFactory;
    use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
    use crate::ports::mail_provider::ProviderFactory;
    use lyonoyl_domain::folder::FolderRole;
    use lyonoyl_domain::message::FolderId;

    let index = SeededIndex::with("real-mailboxes", FolderId::new(1).expect("non-zero"), 0);
    let account = development_account("acc-1", "test@lyonoyl.local");

    let discovery = ImapProviderFactory::new(runtime().handle().clone())
        .discovery(
            &account,
            &Credential::password(SecretString::from("lyonoyl".to_owned())),
        )
        .expect("the provider builds");

    // Account 9: the seeded index already holds an INBOX for account 1, and this must not touch it.
    let recorded = SyncMailboxes::new(discovery, SqliteMailIndex::new(index.pool.clone()))
        .execute(9)
        .expect("the server lists its mailboxes");

    assert!(
        recorded.folders.len() >= 6,
        "the development server has an inbox and five special-use mailboxes: {:?}",
        recorded.folders
    );
    assert!(
        recorded.inbox.is_some(),
        "the inbox is recognised although no server marks it"
    );

    let roles: Vec<(String, i64)> = {
        let connection = index.pool.get().expect("connection");
        let mut statement = connection
            .prepare("SELECT name, role FROM folders WHERE account_id = 9 ORDER BY name")
            .expect("folders are readable");
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .expect("rows")
            .map(|row| row.expect("row"))
            .collect();
        rows
    };

    let inbox_role = roles
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("INBOX"))
        .map(|(_, role)| FolderRole::from_stored(*role));
    assert_eq!(inbox_role, Some(FolderRole::Inbox));

    let stored: Vec<FolderRole> = roles
        .iter()
        .map(|(_, role)| FolderRole::from_stored(*role))
        .collect();
    for expected in [
        FolderRole::Sent,
        FolderRole::Drafts,
        FolderRole::Trash,
        FolderRole::Junk,
        FolderRole::Archive,
    ] {
        assert!(stored.contains(&expected), "missing role: {expected:?}");
    }

    // The first account's folders are untouched by a listing recorded against another account.
    let others: i64 = index
        .pool
        .get()
        .expect("connection")
        .query_row(
            "SELECT count(*) FROM folders WHERE account_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("count is readable");
    assert_eq!(others, 1);
}

#[test]
fn a_pass_over_a_real_mailbox_brings_its_mail_into_the_index() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    use crate::application::mailboxes::SyncMailboxes;
    use crate::application::sync::SyncFolder;
    use crate::infrastructure::fake_provider::SeededIndex;
    use crate::infrastructure::imap::ImapProviderFactory;
    use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
    use crate::ports::mail_provider::ProviderFactory;
    use lyonoyl_domain::message::FolderId;

    // A mailbox of its own, holding the corpus. The development accounts hold whatever somebody
    // left in them, and on a server started fresh they hold nothing at all.
    let mailbox = holding_the_corpus("real-pass");

    let index = SeededIndex::with("real-pass", FolderId::new(1).expect("non-zero"), 0);
    let account = development_account("acc-1", &mailbox.address);
    let password = || SecretString::from("lyonoyl".to_owned());
    let factory = ImapProviderFactory::new(runtime().handle().clone());

    let discovery = factory
        .discovery(&account, &Credential::password(password()))
        .expect("the provider builds");
    let recorded = SyncMailboxes::new(discovery, SqliteMailIndex::new(index.pool.clone()))
        .execute(9)
        .expect("the server lists its mailboxes");
    let inbox = recorded.inbox.expect("the account has an inbox");

    let pass = |folder| {
        SyncFolder::new(
            factory
                .sync(&account, &Credential::password(password()))
                .expect("the provider builds"),
            SqliteMailIndex::new(index.pool.clone()),
        )
        .execute(folder)
    };

    let first = pass(inbox).expect("the first pass applies");
    assert_eq!(
        first.summary.added,
        mailbox.uids.len(),
        "every message in the mailbox arrives, and arrives once"
    );
    assert_eq!(first.summary.removed, 0, "a read-only pass deletes nothing");

    // A pass says how far down it looked. This mailbox is smaller than a first window and its
    // numbers start at the first UID — the fixture starts it over rather than let them climb — so
    // the pass looked all the way down and the folder is finished, without anyone asking for older
    // mail. A pass that does not say this leaves a folder that is never read again.
    {
        use crate::ports::mail_index::MailIndex;
        let state = SqliteMailIndex::new(index.pool.clone())
            .folder_state(inbox)
            .expect("state is readable");
        assert_eq!(
            state.window_floor,
            lyonoyl_domain::message::Uid::new(1).ok(),
            "the pass looked from the first UID"
        );
        assert!(
            state.history_complete,
            "and there is nothing older left to read"
        );
    }

    let stored: Vec<(Option<String>, Option<String>, i64)> = {
        let connection = index.pool.get().expect("connection");
        let mut statement = connection
            .prepare(
                "SELECT m.subject, m.sender, m.received_at
                 FROM message_placements p JOIN messages m ON m.id = p.message_id
                 WHERE p.folder_id = ?1",
            )
            .expect("messages are readable");
        statement
            .query_map([inbox.get() as i64], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .expect("rows")
            .map(|row| row.expect("row"))
            .collect()
    };

    assert_eq!(stored.len(), first.summary.added);
    assert!(
        stored.iter().all(|(_, _, received_at)| *received_at > 0),
        "every message has the arrival time the server gave it"
    );
    assert!(
        stored
            .iter()
            .any(|(subject, _, _)| subject.as_deref().is_some_and(|text| !text.is_empty())),
        "the corpus has subjects: {stored:?}"
    );
    assert!(
        !stored
            .iter()
            .any(|(subject, _, _)| subject.as_deref().is_some_and(|text| text.contains("=?"))),
        "a subject still MIME-encoded reached the index: {stored:?}"
    );

    // The opening of the message, which is what the list shows under the subject. Without this the
    // whole path from the server to that line is untested, and an empty area looks like a design.
    let previews: Vec<Option<String>> = {
        let connection = index.pool.get().expect("connection");
        let mut statement = connection
            .prepare(
                "SELECT m.preview FROM message_placements p JOIN messages m ON m.id = p.message_id
                 WHERE p.folder_id = ?1",
            )
            .expect("messages are readable");
        statement
            .query_map([inbox.get() as i64], |row| row.get(0))
            .expect("rows")
            .map(|row| row.expect("row"))
            .collect()
    };
    assert!(
        previews.iter().any(|preview| preview.is_some()),
        "not one message came back with its opening line: {previews:?}"
    );

    // The second pass is the one that catches the trap: `UID FETCH n:*` answers with the last
    // message of the folder when nothing is newer, and a pass that trusts it stores that message
    // again on every run.
    let second = pass(inbox).expect("the second pass applies");
    assert_eq!(
        second.summary.added, 0,
        "a pass with nothing new must add nothing"
    );
    assert_eq!(second.summary.removed, 0);
}

#[test]
fn a_folder_whose_numbering_the_server_replaced_is_read_again_rather_than_left_empty() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    use crate::application::mailboxes::SyncMailboxes;
    use crate::application::sync::SyncFolder;
    use crate::infrastructure::fake_provider::SeededIndex;
    use crate::infrastructure::imap::ImapProviderFactory;
    use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
    use crate::ports::mail_provider::ProviderFactory;
    use lyonoyl_domain::message::FolderId;

    let mailbox = holding_the_corpus("resync");

    let index = SeededIndex::with("resync", FolderId::new(1).expect("non-zero"), 0);
    let account = development_account("acc-1", &mailbox.address);
    let password = || SecretString::from("lyonoyl".to_owned());
    let factory = ImapProviderFactory::new(runtime().handle().clone());

    let recorded = SyncMailboxes::new(
        factory
            .discovery(&account, &Credential::password(password()))
            .expect("the provider builds"),
        SqliteMailIndex::new(index.pool.clone()),
    )
    .execute(9)
    .expect("the server lists its mailboxes");
    let inbox = recorded.inbox.expect("the account has an inbox");

    let pass = |folder| {
        SyncFolder::new(
            factory
                .sync(&account, &Credential::password(password()))
                .expect("the provider builds"),
            SqliteMailIndex::new(index.pool.clone()),
        )
        .execute(folder)
    };

    let first = pass(inbox).expect("the first pass applies");
    assert_eq!(
        first.summary.added,
        mailbox.uids.len(),
        "the inbox has the corpus to read"
    );

    // What a server does when it renumbers a mailbox: a different generation, and every stored UID
    // now addresses nothing. The cursor is left as it was, which is the situation that matters —
    // it counts in a numbering that no longer exists.
    index
        .pool
        .get()
        .expect("connection")
        .execute(
            "UPDATE folders SET uid_validity = 7 WHERE id = ?1",
            [inbox.get() as i64],
        )
        .expect("the folder is renumbered");

    let after = pass(inbox).expect("the pass applies");

    assert!(after.resynced, "the pass must notice the renumbering");
    assert_eq!(
        after.summary.added, first.summary.added,
        "the folder must be read again, not emptied and left empty"
    );

    let placements: i64 = index
        .pool
        .get()
        .expect("connection")
        .query_row(
            "SELECT count(*) FROM message_placements WHERE folder_id = ?1",
            [inbox.get() as i64],
            |row| row.get(0),
        )
        .expect("count is readable");
    assert_eq!(
        placements as usize, first.summary.added,
        "every message is back in the folder"
    );
}

/// Runs `doveadm` inside the development server. The test drives the *server*, which is the only
/// way to see the client notice a change it did not make.
fn on_the_server(arguments: &[&str]) -> String {
    let output = std::process::Command::new("docker")
        .arg("exec")
        .arg("lyonoyl-mail-server")
        .arg("doveadm")
        .args(arguments)
        .output()
        .expect("docker is available");

    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// The generation a mailbox on the development server is at right now.
///
/// Asked rather than assumed: everything addressed by UID is only addressed at all together with
/// the generation the number was counted in, and a test that made one up would be testing that the
/// application ignores it.
fn generation_of(address: &str, mailbox: &str) -> lyonoyl_domain::message::UidValidity {
    let said = on_the_server(&["mailbox", "status", "-u", address, "uidvalidity", mailbox]);
    let value: u32 = said
        .split_whitespace()
        .find_map(|word| word.strip_prefix("uidvalidity="))
        .unwrap_or_else(|| panic!("doveadm did not state a generation: {said}"))
        .parse()
        .expect("a generation is a number");

    lyonoyl_domain::message::UidValidity::new(value).expect("non-zero")
}

/// Puts one message into a mailbox and answers with its UID.
///
/// The lines are joined explicitly rather than written as one literal: a header block built with
/// escaped line continuations comes out folded into a single header, and the message then has no
/// subject at all. The check at the end is there because that mistake is invisible otherwise.
fn deliver(address: &str, subject: &str) -> u32 {
    deliver_saying(address, subject, "Body.")
}

/// The same, with a body somebody chose — for a test about what a message says rather than what it
/// is called.
fn deliver_saying(address: &str, subject: &str, body: &str) -> u32 {
    let message = [
        "From: Someone <someone@example.com>".to_owned(),
        format!("To: {address}"),
        format!("Subject: {subject}"),
        format!("Message-ID: <{subject}@example.com>"),
        "Date: Tue, 14 Nov 2023 22:13:20 +0000".to_owned(),
        String::new(),
        body.to_owned(),
        String::new(),
    ]
    .join("\r\n");

    let mut child = std::process::Command::new("docker")
        .args([
            "exec",
            "-i",
            "lyonoyl-mail-server",
            "doveadm",
            "save",
            "-u",
            address,
            "-m",
            "INBOX",
        ])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("docker is available");

    use std::io::Write;
    child
        .stdin
        .as_mut()
        .expect("stdin is piped")
        .write_all(message.as_bytes())
        .expect("the message is delivered");
    child.wait().expect("doveadm finishes");

    // The one just delivered is the one with the highest UID: a mailbox hands them out in order
    // and never reuses them while its generation holds.
    let listing = on_the_server(&[
        "fetch",
        "-u",
        address,
        "uid hdr.subject",
        "mailbox",
        "INBOX",
        "ALL",
    ]);
    let uid = listing
        .lines()
        .filter_map(|line| line.trim().strip_prefix("uid:"))
        .filter_map(|value| value.trim().parse::<u32>().ok())
        .max()
        .unwrap_or_else(|| panic!("the mailbox lists no messages: {listing}"));

    assert!(
        listing.contains(subject),
        "the delivered message did not keep its subject, so the fixture is wrong: {listing}"
    );

    uid
}

/// Removes anything an earlier run of this test left behind, so failures do not pile up in the
/// development mailbox.
///
/// Matched on the subject, and also on its absence: a run that failed between delivering a message
/// and checking it leaves one with no subject at all, and nothing else in the corpus is like that.
fn remove_leftovers(address: &str) {
    // Every subject these tests deliver, not only this test's own: two of them run at once, and
    // each is as able to tidy up after the other as after itself. A prefix missing from this list
    // is mail nobody clears, accumulating in a mailbox the next run then reads.
    let prefixes = ["acting-", "audit-", "condstore-", "vanish-"];
    let listing = on_the_server(&[
        "fetch",
        "-u",
        address,
        "uid hdr.subject",
        "mailbox",
        "INBOX",
        "ALL",
    ]);

    let mut uid = None;
    for line in listing.lines() {
        let line = line.trim();
        if let Some(value) = line.strip_prefix("uid:") {
            uid = value.trim().parse::<u32>().ok();
        } else if let Some(found) = line.strip_prefix("hdr.subject:") {
            let subject = found.trim();
            if subject.is_empty() || prefixes.iter().any(|start| subject.starts_with(start)) {
                if let Some(uid) = uid {
                    on_the_server(&[
                        "expunge",
                        "-u",
                        address,
                        "mailbox",
                        "INBOX",
                        "UID",
                        &uid.to_string(),
                    ]);
                }
            }
        }
    }
}

#[test]
fn a_folder_follows_the_server_when_someone_else_changes_it() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    use crate::application::mailboxes::SyncMailboxes;
    use crate::application::sync::SyncFolder;
    use crate::infrastructure::fake_provider::SeededIndex;
    use crate::infrastructure::imap::ImapProviderFactory;
    use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
    use crate::ports::mail_provider::ProviderFactory;
    use lyonoyl_domain::message::FolderId;

    // Its own mailbox on the development server. A test that delivers and expunges must not share
    // one with a test that counts what is there.
    let address = "flags-test@lyonoyl.local";
    remove_leftovers(address);
    let subject = format!("audit-{}", std::process::id());
    let uid = deliver(address, &subject);

    let index = SeededIndex::with("audit", FolderId::new(1).expect("non-zero"), 0);
    let account = development_account("acc-1", address);
    let password = || SecretString::from("lyonoyl".to_owned());
    let factory = ImapProviderFactory::new(runtime().handle().clone());

    let recorded = SyncMailboxes::new(
        factory
            .discovery(&account, &Credential::password(password()))
            .expect("the provider builds"),
        SqliteMailIndex::new(index.pool.clone()),
    )
    .execute(9)
    .expect("the server lists its mailboxes");
    let inbox = recorded.inbox.expect("the account has an inbox");

    let pass = |folder| {
        SyncFolder::new(
            factory
                .sync(&account, &Credential::password(password()))
                .expect("the provider builds"),
            SqliteMailIndex::new(index.pool.clone()),
        )
        .execute(folder)
    };

    pass(inbox).expect("the first pass applies");

    let flags_of_delivered = || -> Option<i64> {
        index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT p.flags
                 FROM message_placements p JOIN messages m ON m.id = p.message_id
                 WHERE p.folder_id = ?1 AND p.uid = ?2",
                (inbox.get() as i64, i64::from(uid)),
                |row| row.get(0),
            )
            .ok()
    };

    let before = flags_of_delivered().expect("the delivered message is stored");
    assert_eq!(before & 1, 0, "it arrives unread");

    // Someone reads it in another client.
    on_the_server(&[
        "flags",
        "add",
        "-u",
        address,
        "\\Seen",
        "mailbox",
        "INBOX",
        "UID",
        &uid.to_string(),
    ]);

    let second = pass(inbox).expect("the second pass applies");
    assert!(
        second.summary.updated > 0,
        "a flag changed on the server must reach the index"
    );
    assert_eq!(
        flags_of_delivered().expect("still stored") & 1,
        1,
        "the message is read here too now"
    );

    // And then they delete it.
    on_the_server(&[
        "expunge",
        "-u",
        address,
        "mailbox",
        "INBOX",
        "UID",
        &uid.to_string(),
    ]);

    let third = pass(inbox).expect("the third pass applies");
    assert_eq!(
        third.summary.removed, 1,
        "a message gone from the server goes from here"
    );
    assert_eq!(
        flags_of_delivered(),
        None,
        "and nothing of it is left behind"
    );
}

/// A pass reads the messages whole, so they are here and findable by what they say.
///
/// Against a real server, because the thing being proved is that a whole message comes back and
/// what came back is the message: a fake that hands over the same bytes it was given proves the
/// plumbing and nothing about the mail.
#[test]
fn a_pass_reads_the_messages_whole_and_what_they_say_becomes_findable() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    use crate::application::mailboxes::SyncMailboxes;
    use crate::application::sync::{FetchBodies, SyncFolder};
    use crate::infrastructure::fake_provider::SeededIndex;
    use crate::infrastructure::imap::ImapProviderFactory;
    use crate::infrastructure::secure_storage::message_key::MessageKey;
    use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
    use crate::infrastructure::sqlite::message_bodies::SqliteMessageBodies;
    use crate::ports::mail_provider::ProviderFactory;
    use lyonoyl_domain::message::FolderId;

    let address = a_mailbox_of_its_own("wholemail");
    let address = address.as_str();
    // A word past the opening. A pass keeps 320 characters of what a message says, so a word
    // inside those is one the index already had — and a test about reading the message whole has
    // to put its word where only the whole message reaches.
    let far_down = format!(
        "{} The lighthouse keeper sends regards.",
        "Filler that goes on for a while so the opening is used up. ".repeat(10)
    );
    deliver_saying(address, "A note", &far_down);

    let index = SeededIndex::with("wholemail", FolderId::new(1).expect("non-zero"), 0);
    let account = development_account("acc-1", address);
    let password = || SecretString::from("lyonoyl".to_owned());
    let factory = ImapProviderFactory::new(runtime().handle().clone());
    let store = || SqliteMailIndex::new(index.pool.clone());

    let recorded = SyncMailboxes::new(
        factory
            .discovery(&account, &Credential::password(password()))
            .expect("the provider builds"),
        store(),
    )
    .execute(9)
    .expect("the server lists its mailboxes");
    let inbox = recorded.inbox.expect("the account has an inbox");

    SyncFolder::new(
        factory
            .sync(&account, &Credential::password(password()))
            .expect("builds"),
        store(),
    )
    .execute(inbox)
    .expect("the forward pass applies");

    let bodies = SqliteMessageBodies::new(
        index.pool.clone(),
        MessageKey::from(&[9u8; 32]).expect("a key of the right size"),
    );

    let found = |word: &str| -> i64 {
        index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT count(*) FROM message_search WHERE message_search MATCH ?1",
                [lyonoyl_domain::search::asked_for(word).expect("something typed")],
                |row| row.get(0),
            )
            .expect("readable")
    };

    assert_eq!(
        found("lighthouse"),
        0,
        "the opening already carried the word, so reading the message whole proves nothing"
    );

    let round = FetchBodies::new(
        factory
            .reading(&account, &Credential::password(password()))
            .expect("builds"),
        store(),
        bodies,
    )
    .execute(inbox)
    .expect("the round runs");

    assert!(round.fetched >= 1, "nothing was read whole: {round:?}");
    assert_eq!(round.failed, 0, "{round:?}");
    assert_eq!(
        found("lighthouse"),
        1,
        "a word from the body of a message this device now holds does not find it"
    );

    // Asked again, there is nothing left to read: what was fetched is not fetched twice.
    let again = FetchBodies::new(
        factory
            .reading(&account, &Credential::password(password()))
            .expect("builds"),
        store(),
        SqliteMessageBodies::new(
            index.pool.clone(),
            MessageKey::from(&[9u8; 32]).expect("a key of the right size"),
        ),
    )
    .execute(inbox)
    .expect("the second round runs");
    assert_eq!(again.fetched, 0, "{again:?}");
    assert!(again.complete, "{again:?}");
}

#[test]
fn reading_further_back_brings_in_what_the_first_pass_left_behind() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    use crate::application::mailboxes::SyncMailboxes;
    use crate::application::sync::{BackfillFolder, SyncFolder};
    use crate::infrastructure::fake_provider::SeededIndex;
    use crate::infrastructure::imap::ImapProviderFactory;
    use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
    use crate::ports::mail_index::MailIndex;
    use crate::ports::mail_provider::ProviderFactory;
    use lyonoyl_domain::message::{FolderId, Uid};

    // A mailbox of its own, delivered into here. Sharing one with the development accounts made
    // this test a hostage to whatever somebody had clicked in the running application: a message
    // archived there took the fixture's messages with it, and the test then failed for a reason
    // that had nothing to do with reading further back.
    let address = a_mailbox_of_its_own("backfill");
    let address = address.as_str();
    for at in 0..5 {
        deliver(address, &format!("backfill-{at}"));
    }

    let index = SeededIndex::with("backfill", FolderId::new(1).expect("non-zero"), 0);
    let account = development_account("acc-1", address);
    let password = || SecretString::from("lyonoyl".to_owned());
    let factory = ImapProviderFactory::new(runtime().handle().clone());
    let store = || SqliteMailIndex::new(index.pool.clone());

    let recorded = SyncMailboxes::new(
        factory
            .discovery(&account, &Credential::password(password()))
            .expect("the provider builds"),
        store(),
    )
    .execute(9)
    .expect("the server lists its mailboxes");
    let inbox = recorded.inbox.expect("the account has an inbox");

    let forward = SyncFolder::new(
        factory
            .sync(&account, &Credential::password(password()))
            .expect("builds"),
        store(),
    )
    .execute(inbox)
    .expect("the forward pass applies");
    assert!(
        forward.summary.added >= 3,
        "the fixture needs a few messages to leave behind: {forward:?}"
    );

    let placements = |index: &SeededIndex| -> i64 {
        index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT count(*) FROM message_placements WHERE folder_id = ?1",
                [inbox.get() as i64],
                |row| row.get(0),
            )
            .expect("count is readable")
    };
    let whole_mailbox = placements(&index);

    // The numbers a mailbox hands out keep climbing, so nothing here may name one. The floor is
    // the third message this mailbox actually holds, whatever it is called.
    let floor: i64 = {
        let connection = index.pool.get().expect("connection");
        let mut statement = connection
            .prepare(
                "SELECT uid FROM message_placements
                 WHERE folder_id = ?1 ORDER BY uid ASC LIMIT 3",
            )
            .expect("uids are readable");
        let uids: Vec<i64> = statement
            .query_map([inbox.get() as i64], |row| row.get(0))
            .expect("uids")
            .map(|uid| uid.expect("uid"))
            .collect();
        uids[2]
    };

    // The development mailbox is smaller than a first pass window, so the pass above already read
    // all of it. This puts the folder where a large mailbox would be: read from the floor upwards,
    // and knowing nothing below it.
    {
        let connection = index.pool.get().expect("connection");
        connection
            .execute(
                "DELETE FROM message_placements WHERE folder_id = ?1 AND uid < ?2",
                rusqlite::params![inbox.get() as i64, floor],
            )
            .expect("the rows below the floor are dropped");
        connection
            .execute(
                "UPDATE folders
                 SET sync_window_floor_uid = ?2, full_history_synced = 0
                 WHERE id = ?1",
                rusqlite::params![inbox.get() as i64, floor],
            )
            .expect("the folder is put where a large mailbox would be");
    }

    let shortened = placements(&index);
    assert!(
        shortened < whole_mailbox,
        "the fixture must actually leave something behind"
    );

    let cursor_before = store()
        .folder_state(inbox)
        .expect("state is readable")
        .cursor;

    // Round after round until it says it has reached the beginning, which is what a pass does: one
    // round per pass, every two minutes, until the folder is read to its first message.
    //
    // A round walks down 500 UIDs at a time, so how many rounds this takes is decided by the
    // number the mailbox's oldest message happens to carry — and that number climbs every time this
    // fixture empties the mailbox and delivers into it again. Asserting that one round reaches the
    // beginning is asserting that the server's counter has not yet passed five hundred, which is a
    // fact about the server rather than about reading further back.
    let backfill = || {
        BackfillFolder::new(
            factory
                .sync(&account, &Credential::password(password()))
                .expect("builds"),
            store(),
        )
        .execute(inbox)
        .expect("the round applies")
    };

    let mut brought_back = 0_i64;
    let mut rounds = 0;
    let round = loop {
        let round = backfill();
        brought_back += round.summary.added as i64;
        rounds += 1;

        if round.complete {
            break round;
        }

        assert!(
            rounds < 200,
            "the walk down never reached the beginning: {rounds} rounds, \
             {brought_back} messages brought back"
        );
    };
    assert_eq!(
        brought_back,
        whole_mailbox - shortened,
        "the rounds bring back exactly what was below the floor and nothing else"
    );
    assert!(
        round.complete,
        "and the walk down reached the first message"
    );

    let state = store().folder_state(inbox).expect("state is readable");
    assert_eq!(
        state.cursor, cursor_before,
        "reading backwards must not move the forward position"
    );
    assert!(state.history_complete);
    // One, not the lowest number this mailbox happens to hold: the floor says how far down has
    // been read, and a folder read to its beginning has been read down to the first number there
    // could ever have been.
    assert_eq!(state.window_floor, Uid::new(1).ok());
    assert_eq!(placements(&index), whole_mailbox);

    // A second round has nothing to do and must not ask the server for anything.
    let again = BackfillFolder::new(
        factory
            .sync(&account, &Credential::password(password()))
            .expect("builds"),
        store(),
    )
    .execute(inbox)
    .expect("there is nothing left");
    assert_eq!(again.summary.added, 0);
    assert!(again.complete);
}

#[test]
fn a_flag_changed_outside_the_window_still_reaches_the_index() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    use crate::application::mailboxes::SyncMailboxes;
    use crate::application::sync::SyncFolder;
    use crate::infrastructure::fake_provider::SeededIndex;
    use crate::infrastructure::imap::ImapProviderFactory;
    use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
    use crate::ports::mail_provider::ProviderFactory;
    use lyonoyl_domain::message::FolderId;

    let address = "condstore-test@lyonoyl.local";
    remove_leftovers(address);
    let subject = format!("condstore-{}", std::process::id());
    let uid = deliver(address, &subject);

    let index = SeededIndex::with("condstore", FolderId::new(1).expect("non-zero"), 0);
    let account = development_account("acc-1", address);
    let password = || SecretString::from("lyonoyl".to_owned());
    let factory = ImapProviderFactory::new(runtime().handle().clone());
    let store = || SqliteMailIndex::new(index.pool.clone());

    let recorded = SyncMailboxes::new(
        factory
            .discovery(&account, &Credential::password(password()))
            .expect("the provider builds"),
        store(),
    )
    .execute(9)
    .expect("the server lists its mailboxes");
    let inbox = recorded.inbox.expect("the account has an inbox");

    let pass = || {
        SyncFolder::new(
            factory
                .sync(&account, &Credential::password(password()))
                .expect("builds"),
            store(),
        )
        .execute(inbox)
    };

    pass().expect("the first pass applies");

    // Put the message out of reach of the window a pass lists. A pass looks at the newest five
    // hundred UIDs it holds, so the folder is given one far above this message — which is the
    // situation on any mailbox of thousands, where the flag of an older message can only be found
    // by asking the server what changed.
    let far_above = i64::from(uid) + 600;
    let stored_message: i64 = index
        .pool
        .get()
        .expect("connection")
        .query_row(
            "SELECT message_id FROM message_placements WHERE folder_id = ?1 LIMIT 1",
            [inbox.get() as i64],
            |row| row.get(0),
        )
        .expect("the folder holds something");
    index.place_existing_message(inbox.get() as i64, far_above, stored_message);

    let flags_of = || -> Option<i64> {
        index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT p.flags
                 FROM message_placements p JOIN messages m ON m.id = p.message_id
                 WHERE p.folder_id = ?1 AND p.uid = ?2",
                (inbox.get() as i64, i64::from(uid)),
                |row| row.get(0),
            )
            .ok()
    };
    assert_eq!(flags_of().expect("stored") & 1, 0, "it arrives unread");

    // Someone reads it in another client.
    on_the_server(&[
        "flags",
        "add",
        "-u",
        address,
        "\\Seen",
        "mailbox",
        "INBOX",
        "UID",
        &uid.to_string(),
    ]);

    let second = pass().expect("the second pass applies");

    assert!(
        second.summary.updated > 0,
        "a server that keeps modification sequences can be asked what changed anywhere in the \
         folder; without that this flag is only found by listing the part of it a pass looks at"
    );
    assert_eq!(flags_of().expect("still stored") & 1, 1);

    on_the_server(&[
        "expunge",
        "-u",
        address,
        "mailbox",
        "INBOX",
        "UID",
        &uid.to_string(),
    ]);
}

#[test]
fn a_message_removed_outside_the_window_goes_from_here_too() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    use crate::application::mailboxes::SyncMailboxes;
    use crate::application::sync::SyncFolder;
    use crate::infrastructure::fake_provider::SeededIndex;
    use crate::infrastructure::imap::ImapProviderFactory;
    use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
    use crate::ports::mail_provider::ProviderFactory;
    use lyonoyl_domain::message::FolderId;

    let address = "vanish-test@lyonoyl.local";
    remove_leftovers(address);
    let subject = format!("vanish-{}", std::process::id());
    let uid = deliver(address, &subject);
    let keep = deliver(address, &format!("{subject}-keep"));

    let index = SeededIndex::with("vanish", FolderId::new(1).expect("non-zero"), 0);
    let account = development_account("acc-1", address);
    let password = || SecretString::from("lyonoyl".to_owned());
    let factory = ImapProviderFactory::new(runtime().handle().clone());
    let store = || SqliteMailIndex::new(index.pool.clone());

    let recorded = SyncMailboxes::new(
        factory
            .discovery(&account, &Credential::password(password()))
            .expect("the provider builds"),
        store(),
    )
    .execute(9)
    .expect("the server lists its mailboxes");
    let inbox = recorded.inbox.expect("the account has an inbox");

    let pass = || {
        SyncFolder::new(
            factory
                .sync(&account, &Credential::password(password()))
                .expect("builds"),
            store(),
        )
        .execute(inbox)
    };

    pass().expect("the first pass applies");

    let holds = |uid: u32| -> bool {
        index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT count(*) FROM message_placements WHERE folder_id = ?1 AND uid = ?2",
                (inbox.get() as i64, i64::from(uid)),
                |row| row.get::<_, i64>(0),
            )
            .expect("count is readable")
            > 0
    };
    assert!(holds(uid) && holds(keep), "both arrived");

    // Put the older one out of reach of the window a pass would look at, the way a mailbox of
    // thousands does. Only the count the server states when the mailbox is opened can reveal it.
    index.place_existing_message(
        inbox.get() as i64,
        i64::from(keep) + 900,
        index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT message_id FROM message_placements WHERE folder_id = ?1 AND uid = ?2",
                (inbox.get() as i64, i64::from(keep)),
                |row| row.get::<_, i64>(0),
            )
            .expect("the message is there"),
    );

    on_the_server(&[
        "expunge",
        "-u",
        address,
        "mailbox",
        "INBOX",
        "UID",
        &uid.to_string(),
    ]);

    pass().expect("the second pass applies");

    assert!(
        !holds(uid),
        "a message the server no longer has must not stay here"
    );
    assert!(holds(keep), "and the one it still has must stay");

    on_the_server(&[
        "expunge",
        "-u",
        address,
        "mailbox",
        "INBOX",
        "UID",
        &keep.to_string(),
    ]);
}

#[test]
fn the_check_button_reports_what_the_development_server_holds() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    let found = TestConnection::new(factory())
        .execute(
            &development_account("acc-1", "test@lyonoyl.local"),
            &SecretString::from("lyonoyl".to_owned()),
        )
        .expect("the server answers");

    assert!(found >= 6, "inbox and the special-use mailboxes: {found}");
}

#[test]
fn a_second_account_on_the_same_server_is_stored_beside_the_first() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    let profile = Profile::new("two-accounts");
    let credentials = FakeCredentials::default();
    let password = || SecretString::from("lyonoyl".to_owned());

    for (id, address) in [
        ("acc-1", "zeta@first-domain.test"),
        ("acc-2", "kappa@second-domain.test"),
    ] {
        AddAccount::new(
            SqliteAccountRepository::new(profile.pool.clone()),
            credentials.clone(),
            factory(),
        )
        .execute(development_account(id, address), password())
        .expect("the server accepts the account");
    }

    let stored = SqliteAccountRepository::new(profile.pool.clone())
        .list()
        .expect("the profile can be read");
    assert_eq!(stored.len(), 2, "two addresses, two accounts");
    assert_eq!(
        credentials.stored.lock().expect("test lock").len(),
        2,
        "each with its own password"
    );
}

/// A mailbox for one test and nobody else, emptied before it starts.
///
/// The server takes any address with the same password, so a mailbox each costs nothing — and it
/// settles the one thing that cannot be shared. These tests *remove* mail: one that empties a
/// mailbox another test is reading fails that other test, at a different moment every run, for a
/// reason nobody can find from the failure.
///
/// One per test rather than one for the pair, because two tests are as able to do this to each
/// other as to anybody else.
#[cfg(test)]
fn a_mailbox_of_its_own(test: &str) -> String {
    let address = format!("{test}@lyonoyl.local");
    on_the_server(&["expunge", "-u", &address, "mailbox", "INBOX", "all"]);

    address
}

/// Marking a message read on a real server, and seeing the server say so afterwards.
///
/// Everything up to here was proved against a scripted server, which answers whatever the script
/// says. This is the first thing that changes somebody's mailbox for real, so it is worth watching
/// a real one change: the flag is set through the same path the application uses — an acting
/// connection, past the guard, through the adapter — and read back by asking the server itself.
#[test]
fn a_flag_set_through_the_acting_path_is_there_when_the_server_is_asked() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    use crate::infrastructure::imap::actions::ImapMailActions;
    use crate::infrastructure::imap::imap_account;
    use crate::ports::mail_actions::{MailActions, ServerAction};
    use lyonoyl_domain::message::{Flags, Uid};

    let address = a_mailbox_of_its_own("acting-flags");
    let address = address.as_str();

    let uid = deliver(address, "acting-flag");
    let account = development_account("acc-1", address);
    let actions = ImapMailActions::new(
        imap_account(
            &account,
            &Credential::password(SecretString::from("lyonoyl".to_owned())),
        ),
        runtime().handle().clone(),
    )
    .expect("the acting adapter builds");

    let seen_now = || {
        on_the_server(&[
            "fetch",
            "-u",
            address,
            "flags",
            "mailbox",
            "INBOX",
            "uid",
            &uid.to_string(),
        ])
    };

    assert!(
        !seen_now().contains("\\Seen"),
        "the message arrived already read: {}",
        seen_now()
    );

    // One conversation for both, as a round has: the second command goes down the same one.
    let mut acting = actions.reach().expect("a conversation opens");

    acting
        .carry_out(ServerAction::SetFlags {
            mailbox: "INBOX",
            uid: Uid::new(uid).expect("non-zero"),
            uid_validity: generation_of(address, "INBOX"),
            add: Flags::SEEN,
            remove: Flags::NONE,
        })
        .expect("the server marks it read");

    assert!(
        seen_now().contains("\\Seen"),
        "the server did not mark it read: {}",
        seen_now()
    );

    // And back again, because taking a flag away is its own command and its own mistake.
    acting
        .carry_out(ServerAction::SetFlags {
            mailbox: "INBOX",
            uid: Uid::new(uid).expect("non-zero"),
            uid_validity: generation_of(address, "INBOX"),
            add: Flags::NONE,
            remove: Flags::SEEN,
        })
        .expect("the server marks it unread");

    assert!(
        !seen_now().contains("\\Seen"),
        "the server did not take the flag away: {}",
        seen_now()
    );
}

/// Erasing one message on a real server, and the other one still being there.
///
/// The guarantee `UID EXPUNGE` gives, watched rather than argued: two messages, one of them marked
/// deleted and removed, and the second untouched. On a server without `UIDPLUS` the only expunge
/// available would have taken both — which is why this application refuses to delete there at all.
#[test]
fn erasing_a_message_on_a_real_server_takes_that_one_and_leaves_the_other() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    use crate::infrastructure::imap::actions::ImapMailActions;
    use crate::infrastructure::imap::imap_account;
    use crate::ports::mail_actions::{MailActions, ServerAction};
    use lyonoyl_domain::message::Uid;

    let address = a_mailbox_of_its_own("acting-erase");
    let address = address.as_str();

    let going = deliver(address, "acting-erase-going");
    let staying = deliver(address, "acting-erase-staying");

    // Marked deleted by somebody else and not finished with — another client mid-work, a phone
    // that has not synchronised yet. A bare `EXPUNGE` would take this one too, and it is the whole
    // reason this application will not send one.
    on_the_server(&[
        "flags",
        "add",
        "-u",
        address,
        "\\Deleted",
        "mailbox",
        "INBOX",
        "uid",
        &staying.to_string(),
    ]);

    let account = development_account("acc-1", address);
    let actions = ImapMailActions::new(
        imap_account(
            &account,
            &Credential::password(SecretString::from("lyonoyl".to_owned())),
        ),
        runtime().handle().clone(),
    )
    .expect("the acting adapter builds");

    let subjects = || on_the_server(&["fetch", "-u", address, "hdr.subject", "mailbox", "INBOX"]);

    assert!(subjects().contains("acting-erase-going"));
    assert!(subjects().contains("acting-erase-staying"));

    actions
        .reach()
        .expect("a conversation opens")
        .carry_out(ServerAction::Erase {
            mailbox: "INBOX",
            uid: Uid::new(going).expect("non-zero"),
            uid_validity: generation_of(address, "INBOX"),
        })
        .expect("the server removes it");

    let left = subjects();

    assert!(
        !left.contains("acting-erase-going"),
        "the message named is still there: {left}"
    );
    assert!(
        left.contains("acting-erase-staying"),
        "a message marked deleted by somebody else went with it: {left}"
    );
    remove_leftovers(address);
}

/// The whole path a click takes: written down, told to the server, and gone from the folder it left.
///
/// The third part is what this is for. The instruction reached the server and the index went on
/// saying the message was in the inbox, so archiving looked like a click that did nothing — which
/// is exactly what it looked like to the person who reported it.
#[test]
fn a_message_filed_away_stops_being_shown_in_the_folder_it_left() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    use crate::application::act_on_mail::ActOnMail;
    use crate::application::apply_operations::ApplyOperations;
    use crate::application::mailboxes::SyncMailboxes;
    use crate::application::sync::SyncFolder;
    use crate::infrastructure::fake_provider::SeededIndex;
    use crate::infrastructure::imap::actions::ImapMailActions;
    use crate::infrastructure::imap::{imap_account, ImapProviderFactory};
    use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
    use crate::infrastructure::sqlite::op_log::SqliteOperationLog;
    use crate::infrastructure::sqlite::placement::SqlitePlacements;
    use crate::ports::mail_provider::ProviderFactory;
    use lyonoyl_domain::folder::FolderRole;
    use lyonoyl_domain::message::{FolderId, MessageId};
    use lyonoyl_domain::operation::OperationTarget;

    let address = a_mailbox_of_its_own("filing-away");
    deliver(&address, "to-be-filed");

    let index = SeededIndex::with("filing-away", FolderId::new(1).expect("non-zero"), 0);
    let profile = Profile::new("filing-away");
    let account = development_account("acc-1", &address);
    let password = || SecretString::from("lyonoyl".to_owned());
    let factory = ImapProviderFactory::new(runtime().handle().clone());

    AddAccount::new(
        SqliteAccountRepository::new(profile.pool.clone()),
        FakeCredentials::default(),
        ImapProviderFactory::new(runtime().handle().clone()),
    )
    .execute(account.clone(), password())
    .expect("the server accepts the account");

    let number: i64 = profile
        .pool
        .get()
        .expect("connection")
        .query_row(
            "SELECT index_id FROM mail_accounts WHERE id = 'acc-1'",
            [],
            |row| row.get(0),
        )
        .expect("the account knows its number");

    let recorded = SyncMailboxes::new(
        factory
            .discovery(&account, &Credential::password(password()))
            .expect("the provider builds"),
        SqliteMailIndex::new(index.pool.clone()),
    )
    .execute(number)
    .expect("the server lists its mailboxes");
    let inbox = recorded.inbox.expect("the account has an inbox");

    let pass = || {
        SyncFolder::new(
            factory
                .sync(&account, &Credential::password(password()))
                .expect("the provider builds"),
            SqliteMailIndex::new(index.pool.clone()),
        )
        .execute(inbox)
        .expect("the pass applies")
    };

    pass();

    let held_by = |folder: FolderId| -> i64 {
        index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT count(*) FROM message_placements WHERE folder_id = ?1",
                [folder.get() as i64],
                |row| row.get(0),
            )
            .expect("placements are readable")
    };

    assert_eq!(held_by(inbox), 1, "the delivered message is in the inbox");

    let (message, archive): (i64, i64) = {
        let connection = index.pool.get().expect("connection");
        (
            connection
                .query_row(
                    "SELECT message_id FROM message_placements WHERE folder_id = ?1",
                    [inbox.get() as i64],
                    |row| row.get(0),
                )
                .expect("the message is placed"),
            connection
                .query_row(
                    "SELECT id FROM folders WHERE account_id = ?1 AND role = ?2",
                    rusqlite::params![number, FolderRole::Archive.stored()],
                    |row| row.get(0),
                )
                .expect("the account has an archive"),
        )
    };

    let target = OperationTarget::new(MessageId::new(message as u64).expect("non-zero"), inbox);

    ActOnMail::new(SqliteOperationLog::new(profile.pool.clone()))
        .move_to(
            "acc-1",
            target,
            FolderId::new(archive as u64).expect("non-zero"),
        )
        .expect("the instruction is written down");

    assert_eq!(
        held_by(inbox),
        1,
        "writing it down alone changes nothing about where the message sits"
    );

    let applied = ApplyOperations::new(
        SqliteOperationLog::new(profile.pool.clone()),
        SqlitePlacements::new(index.pool.clone()),
        ImapMailActions::new(
            imap_account(&account, &Credential::password(password())),
            runtime().handle().clone(),
        )
        .expect("the acting adapter builds"),
    )
    .execute("acc-1", number)
    .expect("the queue empties");

    assert_eq!(applied.carried_out, 1, "the server was told");

    // The pass that follows the instruction, which is what a round now runs. Without it the index
    // still says the message is in the inbox and the list goes on showing it.
    pass();

    assert_eq!(
        held_by(inbox),
        0,
        "the message is still shown in the folder it was filed out of"
    );
}

/// A password store that hands back the development password, for a pass that reads it itself.
#[derive(Clone)]
struct DevelopmentPassword;

impl CredentialStore for DevelopmentPassword {
    fn save(&self, _: &CredentialRef, _: &SecretString) -> Result<(), CredentialError> {
        Ok(())
    }

    fn read(&self, _: &CredentialRef) -> Result<SecretString, CredentialError> {
        Ok(SecretString::from("lyonoyl".to_owned()))
    }

    fn delete(&self, _: &CredentialRef) -> Result<(), CredentialError> {
        Ok(())
    }
}

/// One account on the development server, added and read once, with a message in its inbox.
struct Pushing {
    address: String,
    index: crate::infrastructure::fake_provider::SeededIndex,
    profile: Profile,
    account: MailAccount,
    number: i64,
}

impl Pushing {
    fn with_one_message(name: &str, subject: &str) -> Self {
        let address = a_mailbox_of_its_own(name);
        on_the_server(&["expunge", "-u", &address, "mailbox", "Archive", "all"]);
        deliver(&address, subject);

        let index = crate::infrastructure::fake_provider::SeededIndex::with(
            name,
            lyonoyl_domain::message::FolderId::new(1).expect("non-zero"),
            0,
        );
        let profile = Profile::new(name);
        let account = development_account("acc-1", &address);

        AddAccount::new(
            SqliteAccountRepository::new(profile.pool.clone()),
            FakeCredentials::default(),
            factory(),
        )
        .execute(account.clone(), SecretString::from("lyonoyl".to_owned()))
        .expect("the server accepts the account");

        let number: i64 = profile
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT index_id FROM mail_accounts WHERE id = 'acc-1'",
                [],
                |row| row.get(0),
            )
            .expect("the account knows its number");

        let pushing = Self {
            address,
            index,
            profile,
            account,
            number,
        };
        pushing.read_the_account();
        pushing
    }

    /// The whole pass a round runs, reading back what the server carried out.
    fn read_the_account(&self) -> crate::application::sync::AccountPass {
        use crate::application::sync::SyncAccount;
        use crate::infrastructure::secure_storage::message_key::MessageKey;
        use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
        use crate::infrastructure::sqlite::message_bodies::SqliteMessageBodies;
        use crate::infrastructure::sqlite::op_log::SqliteOperationLog;

        SyncAccount::new(
            SqliteAccountRepository::new(self.profile.pool.clone()),
            DevelopmentPassword,
            factory(),
            SqliteMailIndex::new(self.index.pool.clone()),
            SqliteOperationLog::new(self.profile.pool.clone()),
            SqliteMessageBodies::new(
                self.index.pool.clone(),
                MessageKey::from(&[7; 32]).expect("a key"),
            ),
            (),
        )
        .execute("acc-1", self.number)
        .expect("the account is read")
    }

    /// What a push does before it reads anything: the queue, told to the server.
    fn push(&self) -> crate::application::apply_operations::Applied {
        use crate::application::apply_operations::ApplyOperations;
        use crate::infrastructure::imap::actions::ImapMailActions;
        use crate::infrastructure::imap::imap_account;
        use crate::infrastructure::sqlite::op_log::SqliteOperationLog;
        use crate::infrastructure::sqlite::placement::SqlitePlacements;

        ApplyOperations::new(
            SqliteOperationLog::new(self.profile.pool.clone()),
            SqlitePlacements::new(self.index.pool.clone()),
            ImapMailActions::new(
                imap_account(
                    &self.account,
                    &Credential::password(SecretString::from("lyonoyl".to_owned())),
                ),
                runtime().handle().clone(),
            )
            .expect("the acting adapter builds"),
        )
        .execute("acc-1", self.number)
        .expect("the queue empties")
    }

    fn folder(
        &self,
        role: lyonoyl_domain::folder::FolderRole,
    ) -> lyonoyl_domain::message::FolderId {
        let id: i64 = self
            .index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT id FROM folders WHERE account_id = ?1 AND role = ?2",
                rusqlite::params![self.number, role.stored()],
                |row| row.get(0),
            )
            .expect("the account has the folder");
        lyonoyl_domain::message::FolderId::new(id as u64).expect("non-zero")
    }

    /// The one message, and the flags of its copy in a folder, if it has one there.
    fn in_folder(&self, folder: lyonoyl_domain::message::FolderId) -> Option<(i64, i64)> {
        self.index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT message_id, flags FROM message_placements WHERE folder_id = ?1",
                [folder.get() as i64],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok()
    }

    fn sidebar(&self) -> Vec<crate::read_model::mailboxes::AccountNode> {
        crate::read_model::mailboxes::MailboxReadModel::new(
            self.profile.pool.clone(),
            self.index.pool.clone(),
        )
        .tree()
        .expect("the sidebar is readable")
    }

    fn on_the_server(&self, mailbox: &str) -> String {
        on_the_server(&[
            "fetch",
            "-u",
            &self.address,
            "flags hdr.subject",
            "mailbox",
            mailbox,
            "ALL",
        ])
    }
}

/// Opening a message: counted read everywhere at once, told to the server by one push, and written
/// into the index by the server's own answer — with no pass over the mailbox at all.
///
/// The flag is one command, and the server's `OK` to it is all the index needs: no pass over the
/// account is waited for before every count agrees.
#[test]
fn a_message_opened_is_read_on_the_server_after_one_push_and_no_pass() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    use crate::application::act_on_mail::ActOnMail;
    use crate::infrastructure::sqlite::op_log::SqliteOperationLog;
    use lyonoyl_domain::folder::FolderRole;
    use lyonoyl_domain::message::MessageId;
    use lyonoyl_domain::operation::OperationTarget;

    let pushing = Pushing::with_one_message("pushed-read", "pushed-read");
    let inbox = pushing.folder(FolderRole::Inbox);
    let (message, flags) = pushing.in_folder(inbox).expect("the message is in");
    assert_eq!(flags & 1, 0, "it arrives unread");
    assert_eq!(pushing.sidebar()[0].unread, 1);

    ActOnMail::new(SqliteOperationLog::new(pushing.profile.pool.clone()))
        .set_read(
            "acc-1",
            OperationTarget::new(MessageId::new(message as u64).expect("non-zero"), inbox),
            true,
        )
        .expect("written down");

    assert_eq!(
        pushing.sidebar()[0].unread,
        0,
        "counted read the moment it is written down, before any server has heard"
    );

    let applied = pushing.push();

    assert_eq!(applied.carried_out, 1, "the server was told");
    assert_eq!(
        applied.relocated, 0,
        "and nothing moved, so nothing asks for the account to be read"
    );
    assert_eq!(
        pushing.in_folder(inbox).map(|(_, flags)| flags & 1),
        Some(1),
        "the index holds what the server said, with no pass having run"
    );
    assert!(
        pushing.on_the_server("INBOX").contains("\\Seen"),
        "the server holds it: {}",
        pushing.on_the_server("INBOX")
    );
    assert_eq!(pushing.sidebar()[0].unread, 0, "and it stays read");
}

/// Archiving a message and taking it back before any pass has read it into the archive.
///
/// The way back names the archive, where the index does not have the message yet. Refused there,
/// it would tell the person their undo came too late; stopped once sent, it would show them a
/// conflict about their own undo.
#[test]
fn a_message_archived_and_taken_back_at_once_ends_up_home_with_nothing_stopped() {
    if !server_is_running() {
        eprintln!("the development mail server is not running; skipping");
        return;
    }

    use crate::application::act_on_mail::ActOnMail;
    use crate::application::take_back::{TakeBack, TakenBack};
    use crate::infrastructure::sqlite::op_log::SqliteOperationLog;
    use crate::infrastructure::sqlite::placement::SqlitePlacements;
    use lyonoyl_domain::folder::FolderRole;
    use lyonoyl_domain::message::MessageId;
    use lyonoyl_domain::operation::OperationTarget;

    let pushing = Pushing::with_one_message("pushed-back", "pushed-back");
    let inbox = pushing.folder(FolderRole::Inbox);
    let archive = pushing.folder(FolderRole::Archive);
    let (message, _) = pushing.in_folder(inbox).expect("the message is in");
    let target = OperationTarget::new(MessageId::new(message as u64).expect("non-zero"), inbox);
    let queue = || SqliteOperationLog::new(pushing.profile.pool.clone());

    ActOnMail::new(queue())
        .move_to("acc-1", target, archive)
        .expect("written down");
    assert_eq!(pushing.push().relocated, 1, "the server moved it");
    assert!(
        pushing.in_folder(archive).is_none(),
        "and no pass has read it into the archive"
    );

    assert_eq!(
        TakeBack::new(queue(), SqlitePlacements::new(pushing.index.pool.clone()))
            .execute("acc-1", pushing.number, target)
            .expect("asked"),
        TakenBack::AskedFor,
        "the way back is not refused for want of a read"
    );

    let waiting = pushing.push();
    assert_eq!(waiting.conflicted, 0, "nothing is shown as stopped");

    // The read a push runs after a move, which finds the message in the archive and puts the way
    // back in the queue again.
    let read = pushing.read_the_account();
    assert_eq!(read.read_back, 1, "the move is read back");
    assert!(read.operations_found > 0, "and the way back is found");

    assert_eq!(
        pushing.push().relocated,
        1,
        "the way back reached the server"
    );
    pushing.read_the_account();

    assert!(
        pushing.in_folder(inbox).is_some(),
        "the message is home in the index"
    );
    assert!(pushing.in_folder(archive).is_none());
    assert!(
        pushing.on_the_server("INBOX").contains("pushed-back"),
        "and on the server: {}",
        pushing.on_the_server("INBOX")
    );
    let stopped: i64 = pushing
        .profile
        .pool
        .get()
        .expect("connection")
        .query_row(
            "SELECT count(*) FROM op_log WHERE state = 'conflicted'",
            [],
            |row| row.get(0),
        )
        .expect("readable");
    assert_eq!(stopped, 0);
}
