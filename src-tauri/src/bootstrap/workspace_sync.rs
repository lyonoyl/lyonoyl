//! Reading an account with the production adapters behind it.
//!
//! In the composition root because that is the only place allowed to name both a use case and the
//! adapters it runs on. The scheduler above it knows only the port.

use std::sync::Arc;

use crate::application::apply_operations::{Applied, ApplyOperations};
use crate::application::deliver_outbox::{DeliverOutbox, Delivered};
use crate::application::error::UseCaseError;
use crate::application::sync::SyncAccount;
use crate::application::watch::WatchAccount;
use crate::infrastructure::sqlite::account_repository::SqliteAccountRepository;
use crate::infrastructure::sqlite::activity::SqliteAccountActivity;
use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::infrastructure::sqlite::indexes::Indexes;
use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
use crate::infrastructure::sqlite::op_log::SqliteOperationLog;
use crate::infrastructure::sqlite::outbox::SqliteOutbox;
use crate::infrastructure::sqlite::placement::SqlitePlacements;
use crate::ports::account_repository::AccountRepository;
use crate::ports::account_sync::{
    AccountChanged, AccountPush, AccountSync, AccountSyncError, AccountWatch, Pushed, ReadBack,
    WatchEnded, Watermark,
};
use crate::ports::activity::{AccountActivity, WhyItStopped};
use crate::ports::credential::Credential;
use crate::ports::credential_store::{CredentialPurpose, CredentialRef, CredentialStore};
use crate::ports::journal::{Journal, Note, Value, Word};
use crate::ports::mail_actions::SentMail;
use crate::ports::mail_submission::MailSubmission;
use crate::ports::outbox::Outbox;

pub struct WorkspaceSync {
    profile: SqlitePool,
    /// Which file each account's mail is kept in.
    ///
    /// A resolver rather than a file, because a pass belongs to one account: every entry point
    /// below asks for the account's number before it asks for anything of the mail, so the file
    /// can be named rather than assumed.
    indexes: Indexes,
    runtime: tokio::runtime::Handle,
    credentials: crate::Credentials,
}

impl WorkspaceSync {
    pub fn new(
        profile: SqlitePool,
        indexes: Indexes,
        runtime: tokio::runtime::Handle,
        credentials: crate::Credentials,
    ) -> Arc<Self> {
        Arc::new(Self {
            profile,
            indexes,
            runtime,
            credentials,
        })
    }
}

impl WorkspaceSync {
    /// What the index knows the account by. Both a pass and a watch start from it.
    fn number_of(&self, account_id: &str) -> Result<i64, AccountSyncError> {
        crate::read_model::mailboxes::index_id_of(&self.profile, account_id)
            .map_err(|_| AccountSyncError::Failed)?
            .ok_or(AccountSyncError::Failed)
    }

    /// The file this account's mail is in.
    fn index_of(&self, number: i64) -> Result<SqlitePool, AccountSyncError> {
        self.indexes
            .of(number)
            .map_err(|_| AccountSyncError::Failed)
    }
}

impl WorkspaceSync {
    /// Tells the server what the person asked for, one round at a time.
    ///
    /// The only path in this application that changes a mailbox, and it starts here: an account,
    /// its password, and a connection opened to act rather than to read.
    fn empty_the_queue(&self, account_id: &str, number: i64) -> Result<Applied, AccountSyncError> {
        let account = SqliteAccountRepository::new(self.profile.clone())
            .find(account_id)
            .map_err(|_| AccountSyncError::Failed)?
            .ok_or(AccountSyncError::Failed)?;

        let password = self
            .credentials
            .read(&CredentialRef::new(
                account_id.to_owned(),
                CredentialPurpose::ImapPassword,
            ))
            .map_err(|_| AccountSyncError::Failed)?;

        let actions = super::mail_actions(
            &account,
            &Credential::password(password.clone()),
            self.runtime.clone(),
        )
        .map_err(|_| AccountSyncError::Failed)?;

        ApplyOperations::new(
            SqliteOperationLog::new(self.profile.clone()),
            SqlitePlacements::new(self.index_of(number)?),
            actions,
        )
        .execute(account_id, number)
        .map_err(|_| AccountSyncError::Failed)
    }

    /// Hands over whatever the person has written and the server has not taken yet, and files a
    /// copy of what it took.
    ///
    /// An account nobody has given a sending server is not a failure and not a warning: mail
    /// arrives either way. A message written from it waits in the outbox, with no reason given, and
    /// once it is late the activity line counts it as on its way, until the account is given a
    /// sending server. Its round runs all the same, because a message sent before the server was
    /// taken away is still to be filed and let go.
    fn send_what_is_waiting(&self, account_id: &str) -> Result<Delivered, AccountSyncError> {
        self.the_outbox_of(account_id)?
            .execute(account_id)
            .map_err(|_| AccountSyncError::Failed)
    }

    /// Files the copies of what went out, once its server has had its moment, and nothing else.
    fn file_what_went_out(&self, account_id: &str) -> Result<Delivered, AccountSyncError> {
        self.the_outbox_of(account_id)?
            .file_copies(account_id)
            .map_err(|_| AccountSyncError::Failed)
    }

    /// The account's outbox with the servers it hands over to and files in.
    fn the_outbox_of(
        &self,
        account_id: &str,
    ) -> Result<
        DeliverOutbox<
            SqliteOutbox,
            impl MailSubmission,
            impl SentMail,
            crate::infrastructure::sqlite::drafts::SqliteDrafts,
        >,
        AccountSyncError,
    > {
        let account = SqliteAccountRepository::new(self.profile.clone())
            .find(account_id)
            .map_err(|_| AccountSyncError::Failed)?
            .ok_or(AccountSyncError::Failed)?;

        // The same secret the reading half uses: one password per account, for reading and sending.
        let password = self
            .credentials
            .read(&CredentialRef::new(
                account_id.to_owned(),
                CredentialPurpose::ImapPassword,
            ))
            .map_err(|_| AccountSyncError::Failed)?;

        // The copy kept in the folder for sent mail goes over IMAP, on the same acting connection
        // the other changes use, with the same password.
        let keeping = super::mail_actions(
            &account,
            &Credential::password(password.clone()),
            self.runtime.clone(),
        )
        .map_err(|_| AccountSyncError::Failed)?;

        let outbox = SqliteOutbox::new(self.profile.clone());
        let drafts = crate::infrastructure::sqlite::drafts::SqliteDrafts::new(self.profile.clone());

        Ok(match account.submission() {
            Some(submission) => DeliverOutbox::new(
                outbox,
                super::mail_submission(
                    submission,
                    Credential::password(password),
                    self.runtime.clone(),
                ),
                keeping,
                drafts,
            ),
            None => DeliverOutbox::with_no_sending_server(outbox, keeping, drafts),
        })
    }
}

/// Writes down what one round of the outbox did, in counts and nothing else, at the level somebody
/// turns on.
///
/// A sent message whose server will not take its copy is counted here with the rest, and nowhere
/// else. Every count in this line is about the messages a round dealt with, and a line about them
/// at a level every build writes is a record of when somebody sent mail, in a file the databases'
/// key does not cover.
fn wrote_down_a_round(journal: &dyn Journal, done: Delivered) {
    if journal.keeps(Note::Detail) && done != Delivered::default() {
        journal.write_down(
            Note::Detail,
            "outbox.worked",
            &[
                ("sent", Value::Count(u64::from(done.sent))),
                ("filed", Value::Count(u64::from(done.filed))),
                ("found", Value::Count(u64::from(done.found))),
                ("unfiled", Value::Count(u64::from(done.unfiled))),
                ("waiting", Value::Count(u64::from(done.waiting))),
                ("givenBack", Value::Count(u64::from(done.given_back))),
            ],
        );
    }
}

/// Which of the two doors behind "this device's storage cannot be read" was the one that shut.
///
/// The sentence a person is given collapses them on purpose — there is one thing to do about
/// either. Somebody reading the log afterwards has two very different things to look at, and
/// without this the log names the failure and not the place.
fn said_which_door(door: Word) {
    let journal = crate::infrastructure::journal::journal();
    if journal.keeps(Note::Trouble) {
        journal.write_down(
            Note::Trouble,
            "sync.storage",
            &[("door", Value::Word(door))],
        );
    }
}

/// The same failure as the journal spells it.
///
/// Its own mapping rather than a second reading of the one below, because the two answer different
/// questions: the interface offers a person the smallest number of things to do, and the journal is
/// read by somebody working out which of them happened. `Other` on screen is eight failures here.
fn as_the_journal_says_it(why: &UseCaseError) -> Word {
    match why {
        UseCaseError::Offline => Word::TheNetworkWasNotThere,
        UseCaseError::Unauthorized
        | UseCaseError::CredentialsMissing
        | UseCaseError::CredentialsUnavailable => Word::TheSignInWasRefused,
        UseCaseError::StorageUnavailable => Word::TheDatabaseWasUnavailable,
        UseCaseError::StorageCorrupted => Word::TheDatabaseWasDamaged,
        _ => Word::SomethingNobodyExpected,
    }
}

/// A failure as somebody would describe it to themselves.
///
/// The failures underneath are more various than four, and collapsing them is deliberate: "the
/// server answered a FETCH in a way this build cannot use" is true and leads nowhere.
fn in_a_persons_terms(why: &UseCaseError) -> WhyItStopped {
    match why {
        UseCaseError::Offline => WhyItStopped::Offline,
        UseCaseError::Unauthorized
        | UseCaseError::CredentialsMissing
        | UseCaseError::CredentialsUnavailable => WhyItStopped::SignIn,
        UseCaseError::StorageUnavailable | UseCaseError::StorageCorrupted => WhyItStopped::Storage,
        _ => WhyItStopped::Other,
    }
}

/// Writes down how one pass over an account ended, whoever started it: a round, or a person pressing
/// refresh. The foot of the window counts the accounts whose last pass failed, and a pass that worked
/// answers "is this account current" whoever asked for it: written down by rounds alone, a refresh
/// that worked left the account counted until the next round came.
pub(crate) fn a_pass_ended(profile: &SqlitePool, account_id: &str, how: Result<(), &UseCaseError>) {
    let activity = SqliteAccountActivity::new(profile.clone());
    match how {
        // A pass that finished is the answer to "is this current", whatever the queue did: the
        // mail on screen is the mail on the server, and an instruction still waiting is a separate
        // sentence with its own count.
        Ok(()) => {
            let _ = activity.finished(account_id, seconds_now());
        }
        // Written down in terms a person can act on: wait, sign in again, or look at a database
        // that will not open. Recording it is not allowed to turn one failure into two, so a store
        // that will not take it is ignored here — the pass has already failed, and that is the
        // news. The sentence on screen and the line in the log say the same thing at once: a pass
        // that stops is the one thing a person sees and reports, and a journal silent about it is
        // read after the fact for nothing. No account and no address: the reason is a word from a
        // closed list.
        Err(why) => {
            let journal = crate::infrastructure::journal::journal();
            if journal.keeps(Note::Trouble) {
                journal.write_down(
                    Note::Trouble,
                    "sync.stopped",
                    &[("why", Value::Word(as_the_journal_says_it(why)))],
                );
            }
            let _ = activity.stopped(account_id, in_a_persons_terms(why));
        }
    }
}

/// Unix seconds, as everything stored in these two databases counts time.
fn seconds_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default()
}

impl WorkspaceSync {
    /// One reading pass over the account: its mailboxes, then what is in them.
    ///
    /// Told to nobody while it runs. Nobody asked for it — a round on the timer, a server saying
    /// something arrived, or a push reading back what it moved — and the foot of the window speaks
    /// only for what a person asked for.
    fn read(
        &self,
        account_id: &str,
        number: i64,
    ) -> Result<crate::application::sync::AccountPass, crate::application::error::UseCaseError>
    {
        // A file that will not open is the account's own failure, not the pass's: it is reported
        // the way a database that cannot be read is, rather than as a mailbox that answered nothing.
        let index = self.indexes.of(number).map_err(|_| {
            said_which_door(Word::TheIndexWouldNotOpen);
            crate::application::error::UseCaseError::StorageUnavailable
        })?;

        // The key the mail is sealed under, asked for once per pass rather than once per message:
        // on macOS every read of a keychain entry a build did not write raises a dialog.
        let sealing =
            crate::infrastructure::secure_storage::message_key::MessageKey::of_this_device(
                &self.credentials,
            )
            .map_err(|_| {
                said_which_door(Word::TheMessageKeyWasRefused);
                crate::application::error::UseCaseError::StorageUnavailable
            })?;

        SyncAccount::new(
            SqliteAccountRepository::new(self.profile.clone()),
            &self.credentials,
            super::mail_servers(self.runtime.clone()),
            SqliteMailIndex::new(index.clone()),
            SqliteOperationLog::new(self.profile.clone()),
            crate::infrastructure::sqlite::message_bodies::SqliteMessageBodies::new(index, sealing),
            (),
        )
        .execute(account_id, number)
    }

    /// Tells the server what the person has done, and hands over what they wrote.
    ///
    /// Nothing is read here. A flag the server confirmed has already been written where every
    /// view reads it, and a move is read back by whoever called this, once they are done pushing:
    /// the number a message has where it went is the server's to give, and reading the whole
    /// account for it after every twenty instructions is fifty reads for one emptied trash.
    fn push_now(&self, account_id: &str, number: i64) -> (Applied, Delivered) {
        let began = std::time::Instant::now();

        let carried = self.empty_the_queue(account_id, number).unwrap_or_default();

        // After the instructions, and for the same reason they come first on the way out: they
        // were asked for first. A failure here is the outbox's own business — every message in it
        // says what became of it — and not a reason to call the push failed.
        //
        // Written down, though, in counts and nothing else: a message that did not arrive has to
        // be told apart from one that was never tried without opening the database by hand.
        let handed_over = match self.send_what_is_waiting(account_id) {
            Ok(done) => {
                wrote_down_a_round(&*crate::infrastructure::journal::journal(), done);
                done
            }
            Err(_) => {
                let journal = crate::infrastructure::journal::journal();
                if journal.keeps(Note::Trouble) {
                    journal.write_down(Note::Trouble, "outbox.stopped", &[]);
                }
                Delivered::default()
            }
        };

        // Counts and a clock, and nothing that names an account, a mailbox or a message.
        let journal = crate::infrastructure::journal::journal();
        if journal.keeps(Note::Detail) {
            journal.write_down(
                Note::Detail,
                "push.finished",
                &[
                    ("took", Value::Millis(began.elapsed().as_millis() as u64)),
                    ("carried", Value::Count(u64::from(carried.carried_out))),
                    ("deferred", Value::Count(u64::from(carried.deferred))),
                    ("conflicted", Value::Count(u64::from(carried.conflicted))),
                    ("relocated", Value::Count(u64::from(carried.relocated))),
                    (
                        "stoppedShort",
                        Value::Count(u64::from(carried.stopped_short)),
                    ),
                    ("sent", Value::Count(u64::from(handed_over.sent))),
                ],
            );
        }

        (carried, handed_over)
    }

    /// Reads the account back after pushes moved mail, so the index learns where it went.
    fn read_back_now(&self, account_id: &str, number: i64) -> Result<ReadBack, AccountSyncError> {
        let reading = std::time::Instant::now();
        let pass = self
            .read(account_id, number)
            .map_err(|_| AccountSyncError::Failed)?;

        // What was filed is let go once the account is read back: the list of sent mail shows the
        // server's copy from here on, and nothing of the message is kept on this device.
        let _ = SqliteOutbox::new(self.profile.clone()).forget_filed(account_id);

        let journal = crate::infrastructure::journal::journal();
        if journal.keeps(Note::Detail) {
            journal.write_down(
                Note::Detail,
                "push.readBack",
                &[
                    ("took", Value::Millis(reading.elapsed().as_millis() as u64)),
                    ("readBack", Value::Count(u64::from(pass.read_back))),
                    ("found", Value::Count(u64::from(pass.operations_found))),
                ],
            );
        }

        Ok(ReadBack {
            changed: changed_by(&pass),
            let_go: pass.operations_found,
        })
    }
}

/// What a pass changed, as the window is told it.
fn changed_by(pass: &crate::application::sync::AccountPass) -> AccountChanged {
    AccountChanged {
        arrived: pass.arrived,
        updated: pass.updated,
        removed: pass.removed,
        folders: pass.folders,
        bodies_read: pass.bodies_read,
        unseen_in_inbox: pass.unseen_in_inbox,
    }
}

impl AccountPush for WorkspaceSync {
    fn push(&self, account_id: &str) -> Result<Pushed, AccountSyncError> {
        let number = self.number_of(account_id)?;
        let (carried, handed_over) = self.push_now(account_id, number);

        Ok(Pushed {
            moved: carried.relocated > 0,
            more: carried.more(),
            sent: handed_over.sent > 0,
        })
    }

    fn file_copies(&self, account_id: &str) -> Result<bool, AccountSyncError> {
        let filed = self.file_what_went_out(account_id)?;
        wrote_down_a_round(&*crate::infrastructure::journal::journal(), filed);
        Ok(filed.filed + filed.found > 0)
    }

    fn read_back(&self, account_id: &str) -> Result<ReadBack, AccountSyncError> {
        let number = self.number_of(account_id)?;
        self.read_back_now(account_id, number)
    }
}

impl AccountSync for WorkspaceSync {
    fn sync(&self, account_id: &str) -> Result<AccountChanged, AccountSyncError> {
        let number = self.number_of(account_id)?;

        // What this pass costs, taken at both ends. Counts and a clock only -- no account, no
        // mailbox name, no address: the question this answers is "where did the time go", and
        // none of those help answer it.
        //
        // How many sessions it opened comes from the pass itself rather than from here, because
        // the answer has to be about this pass: counted across the process, nine accounts read
        // side by side each report the whole round.
        let began = std::time::Instant::now();

        let pass = match self.read(account_id, number) {
            Ok(pass) => pass,
            // Written down before the failure is passed on.
            Err(why) => {
                a_pass_ended(&self.profile, account_id, Err(&why));
                return Err(AccountSyncError::Failed);
            }
        };

        // After the pass, not before it: what an instruction needs is the freshest answer to
        // "where does this message sit", and that is what a pass has just written. A pass that
        // found a folder renumbered has also, by now, sent everything waiting in it back to be
        // found again — so nothing here can act on a position that stopped meaning what it did.
        //
        // The same push a person's change gets, so a round and a push cannot drift into telling a
        // server different things: what failed before is tried again here, after its wait.
        let reading_took = began.elapsed();
        crate::infrastructure::journal::wrote_down_a_pass(&pass, reading_took);

        // What was filed before this pass, the pass has read: its message is let go now, and not
        // by the push below, which reads nothing.
        let _ = SqliteOutbox::new(self.profile.clone()).forget_filed(account_id);
        let (carried, handed_over) = self.push_now(account_id, number);

        // What that moved, read back at once rather than a round later — and so are copies just
        // filed in the folder for sent mail; and what was waiting for that read, carried out behind
        // it. Once each: a queue longer than a round is a push's work, and the push asked for when
        // it was written is doing it.
        let mut again = ReadBack::default();
        if carried.relocated > 0 || handed_over.filed + handed_over.found > 0 {
            if let Ok(read) = self.read_back_now(account_id, number) {
                again = read;
                if read.let_go > 0 {
                    self.push_now(account_id, number);
                }
            }
        }

        a_pass_ended(&self.profile, account_id, Ok(()));

        let read = changed_by(&pass);
        Ok(AccountChanged {
            arrived: read.arrived + again.changed.arrived,
            updated: read.updated + again.changed.updated,
            removed: read.removed + again.changed.removed,
            folders: read.folders,
            bodies_read: read.bodies_read + again.changed.bodies_read,
            unseen_in_inbox: read.unseen_in_inbox + again.changed.unseen_in_inbox,
        })
    }

    fn accounts(&self) -> Result<Vec<String>, AccountSyncError> {
        Ok(SqliteAccountRepository::new(self.profile.clone())
            .list()
            .map_err(|_| AccountSyncError::Failed)?
            .iter()
            .map(|account| account.id().to_owned())
            .collect())
    }
}

impl AccountWatch for WorkspaceSync {
    fn watch(
        &self,
        account_id: &str,
        since: Option<&Watermark>,
    ) -> Result<WatchEnded, AccountSyncError> {
        let number = self.number_of(account_id)?;

        WatchAccount::new(
            SqliteAccountRepository::new(self.profile.clone()),
            &self.credentials,
            super::mail_servers(self.runtime.clone()),
            SqliteMailIndex::new(self.index_of(number)?),
        )
        .execute(account_id, number, since)
        .map_err(|_| AccountSyncError::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::secure_storage::cache::CachedCredentials;
    use crate::infrastructure::secure_storage::database_key::DatabaseKey;
    use crate::infrastructure::secure_storage::master_password::MasterPasswordStore;
    use crate::infrastructure::secure_storage::SecretStore;
    use crate::infrastructure::sqlite::connection::open_profile;
    use crate::infrastructure::sqlite::migrations::migrate_profile;
    use crate::ports::outbox::Outbox;
    use lyonoyl_domain::account::{Encryption, MailAccount, MailAccountSummary, ServerSettings};
    use lyonoyl_domain::outgoing::{EmailAddress, Outgoing, Subject};
    use secrecy::SecretString;

    /// Two accounts, the second already counted as not updating, as the foot of the window reads
    /// them.
    fn two_accounts_one_failing(name: &str) -> crate::infrastructure::fake_provider::SeededProfile {
        let profile = crate::infrastructure::fake_provider::SeededProfile::new(name);
        profile.add_account("acc-1", "zeta@example.com", 1);
        profile.add_account("acc-2", "kappa@example.net", 2);
        let activity = SqliteAccountActivity::new(profile.pool.clone());
        activity.finished("acc-1", 1_700_000_000).expect("written");
        activity
            .stopped("acc-2", WhyItStopped::Storage)
            .expect("written");
        profile
    }

    fn as_the_foot_reads_it(
        profile: &crate::infrastructure::fake_provider::SeededProfile,
    ) -> crate::read_model::activity::HowThingsAre {
        crate::read_model::activity::ActivityReadModel::new(profile.pool.clone())
            .how_things_are()
            .expect("readable")
    }

    #[test]
    fn a_pass_that_worked_takes_the_account_off_the_count_whoever_asked_for_it() {
        // Refresh after the keychain refused an account its key: the pass a person asked for read
        // the account, and the line went on counting it until the next round came.
        let profile = two_accounts_one_failing("pass-ended-worked");
        assert_eq!(as_the_foot_reads_it(&profile).accounts_failing, 1);

        a_pass_ended(&profile.pool, "acc-2", Ok(()));

        let how = as_the_foot_reads_it(&profile);
        assert_eq!(how.accounts_failing, 0);
        assert!(how.last_success_at.expect("a pass has finished") > 1_700_000_000);
    }

    #[test]
    fn a_pass_that_failed_counts_the_account_with_its_reason() {
        let profile = two_accounts_one_failing("pass-ended-failed");

        a_pass_ended(&profile.pool, "acc-1", Err(&UseCaseError::Offline));

        let how = as_the_foot_reads_it(&profile);
        assert_eq!(how.accounts_failing, 2);
        // Both failing, so the line gives one reason for the whole of it, the first of theirs by
        // name: offline before storage.
        assert_eq!(how.stopped_by, Some(WhyItStopped::Offline));
        // When it last worked is kept: "not working" and "how old is what I see" are two sentences.
        assert_eq!(how.last_success_at, Some(1_700_000_000));
    }

    /// A folder of its own, taken away afterwards.
    struct Folder(std::path::PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "lyonoyl-workspace-{name}-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4().simple()
            ));
            std::fs::create_dir_all(&path).expect("a folder");
            Self(path)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// An account that reads and has nowhere to send from, holding a message a round took to hand
    /// over and never said what became of, and one still waiting to go.
    ///
    /// Its round hands nothing over, and that is all it leaves out. A round that stopped at the
    /// missing server would keep the message left unfinished in the outbox for good — everybody it
    /// went to, blind copies included — and never give it back to the drafts.
    #[test]
    fn an_account_with_no_sending_server_still_has_its_outbox_looked_after() {
        let folder = Folder::new("no-sending-server");
        let path = folder.0.join("profile.sqlite3");
        let profile = open_profile(&path, &DatabaseKey::for_a_test()).expect("the profile opens");
        let mut connection = profile.get().expect("a connection");
        migrate_profile(&mut connection, &path.display().to_string()).expect("migrations run");
        drop(connection);

        // A port nothing answers on: this round has nothing to file, and must not go looking.
        let account = MailAccount::new(
            MailAccountSummary::new("acc-1", "zeta@example.com", "", "imap")
                .expect("valid summary"),
            ServerSettings::new("127.0.0.1", 9, Encryption::None).expect("valid server"),
            "zeta@example.com",
        )
        .expect("valid account");
        SqliteAccountRepository::new(profile.clone())
            .save(&account)
            .expect("the account is kept");

        let secrets = MasterPasswordStore::quick(folder.0.join("secrets"));
        secrets
            .choose(&SecretString::from("a master password".to_owned()))
            .expect("the file is made");
        let credentials = CachedCredentials::new(SecretStore::Master(Arc::new(secrets)));
        credentials
            .save(
                &CredentialRef::new("acc-1", CredentialPurpose::ImapPassword),
                &SecretString::from("lyonoyl".to_owned()),
            )
            .expect("the password is kept");

        let outbox = SqliteOutbox::new(profile.clone());
        let message = Outgoing {
            files: Vec::new(),
            from: EmailAddress::parse("zeta@example.com").expect("valid"),
            to: vec![EmailAddress::parse("kappa@example.net").expect("valid")],
            cc: Vec::new(),
            bcc: vec![EmailAddress::parse("sigma@example.org").expect("valid")],
            subject: Subject::parse("").expect("empty is a subject"),
            body: String::new(),
            thread: Default::default(),
        };
        let row_of = |message_id: &str| -> i64 {
            outbox
                .queue("acc-1", message_id, &message)
                .expect("written down");
            profile
                .get()
                .expect("a connection")
                .query_row(
                    "SELECT id FROM outbox WHERE message_id = ?1",
                    [message_id],
                    |row| row.get(0),
                )
                .expect("readable")
        };
        let unfinished = row_of("<unfinished@example.com>");
        let waiting = row_of("<waiting@example.com>");
        outbox.claim(unfinished).expect("taken");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        let sync = WorkspaceSync::new(
            profile.clone(),
            Indexes::under(&folder.0, profile.clone(), DatabaseKey::for_a_test()),
            runtime.handle().clone(),
            credentials,
        );

        sync.send_what_is_waiting("acc-1").expect("the round runs");

        let left: Vec<(i64, String)> = {
            let connection = profile.get().expect("a connection");
            let mut statement = connection
                .prepare("SELECT id, state FROM outbox ORDER BY id")
                .expect("readable");
            let rows = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .expect("rows")
                .map(|row| row.expect("row"))
                .collect();
            rows
        };
        assert_eq!(
            left,
            vec![(waiting, "waiting".to_owned())],
            "a message left unfinished was kept, or one waiting was not left to wait"
        );
        let drafts: i64 = profile
            .get()
            .expect("a connection")
            .query_row("SELECT count(*) FROM drafts", [], |row| row.get(0))
            .expect("countable");
        assert_eq!(
            drafts, 1,
            "the message left unfinished is not in the drafts"
        );
    }

    /// A workspace whose one account reads from the development server under an address of its
    /// own, holding one message its server took and whose copy is not filed yet.
    ///
    /// The folder goes last, after everything that has a file open in it.
    struct SentHere {
        profile: SqlitePool,
        sync: Arc<WorkspaceSync>,
        /// The message's row in the outbox.
        id: i64,
        _runtime: tokio::runtime::Runtime,
        _folder: Folder,
    }

    impl SentHere {
        fn on_the_development_server(name: &str) -> Self {
            let folder = Folder::new(name);
            let path = folder.0.join("profile.sqlite3");
            let profile =
                open_profile(&path, &DatabaseKey::for_a_test()).expect("the profile opens");
            let mut connection = profile.get().expect("a connection");
            migrate_profile(&mut connection, &path.display().to_string()).expect("migrations run");
            drop(connection);

            // An address of its own on the development server, which takes any.
            let address = format!("{}@example.com", uuid::Uuid::new_v4().simple());
            let account = MailAccount::new(
                MailAccountSummary::new("acc-1", &address, "", "imap").expect("valid summary"),
                ServerSettings::new("127.0.0.1", 1143, Encryption::None).expect("valid server"),
                &address,
            )
            .expect("valid account");
            SqliteAccountRepository::new(profile.clone())
                .save(&account)
                .expect("the account is kept");

            let secrets = MasterPasswordStore::quick(folder.0.join("secrets"));
            secrets
                .choose(&SecretString::from("a master password".to_owned()))
                .expect("the file is made");
            let credentials = CachedCredentials::new(SecretStore::Master(Arc::new(secrets)));
            credentials
                .save(
                    &CredentialRef::new("acc-1", CredentialPurpose::ImapPassword),
                    &SecretString::from("lyonoyl".to_owned()),
                )
                .expect("the password is kept");

            let outbox = SqliteOutbox::new(profile.clone());
            let message = Outgoing {
                files: Vec::new(),
                from: EmailAddress::parse(&address).expect("valid"),
                to: vec![EmailAddress::parse("kappa@example.net").expect("valid")],
                cc: Vec::new(),
                bcc: Vec::new(),
                subject: Subject::parse("").expect("empty is a subject"),
                body: String::new(),
                thread: Default::default(),
            };
            outbox
                .queue("acc-1", "<read-back@example.com>", &message)
                .expect("written down");
            let id: i64 = profile
                .get()
                .expect("a connection")
                .query_row(
                    "SELECT id FROM outbox WHERE message_id = '<read-back@example.com>'",
                    [],
                    |row| row.get(0),
                )
                .expect("readable");
            outbox.claim(id).expect("taken");
            outbox.accepted(id).expect("taken by a server");

            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("a runtime");
            let sync = WorkspaceSync::new(
                profile.clone(),
                Indexes::under(&folder.0, profile.clone(), DatabaseKey::for_a_test()),
                runtime.handle().clone(),
                credentials,
            );

            Self {
                profile,
                sync,
                id,
                _runtime: runtime,
                _folder: folder,
            }
        }

        /// How many messages the outbox still holds.
        fn left(&self) -> i64 {
            self.profile
                .get()
                .expect("a connection")
                .query_row("SELECT count(*) FROM outbox", [], |row| row.get(0))
                .expect("countable")
        }
    }

    /// A message whose copy is filed stays on this device until the account is read back, and goes
    /// with that read: the list of sent mail shows it from its row until then, and nothing of it is
    /// kept after.
    #[test]
    fn a_filed_message_is_let_go_once_the_account_is_read_back() {
        if !crate::dev_mail_server::answering() {
            eprintln!("the development mail server is not running; skipping");
            return;
        }
        let sent = SentHere::on_the_development_server("read-back-lets-go");
        SqliteOutbox::new(sent.profile.clone())
            .filed(sent.id)
            .expect("filed");

        AccountPush::read_back(&*sent.sync, "acc-1").expect("the account is read back");

        assert_eq!(
            sent.left(),
            0,
            "a filed message was kept after the account was read back"
        );
    }

    /// A message filed by a push whose read back never came is let go by the next round, once its
    /// pass has read the account. Nothing else in that round reads the account back for it, so a
    /// round that left it would keep everybody it went to on this device until a push came.
    #[test]
    fn a_round_lets_go_of_what_was_filed_before_it_once_its_pass_has_read_the_account() {
        if !crate::dev_mail_server::answering() {
            eprintln!("the development mail server is not running; skipping");
            return;
        }
        let sent = SentHere::on_the_development_server("round-lets-go-of-filed");
        SqliteOutbox::new(sent.profile.clone())
            .filed(sent.id)
            .expect("filed");

        AccountSync::sync(&*sent.sync, "acc-1").expect("the round runs");

        assert_eq!(
            sent.left(),
            0,
            "a message filed before the round was kept after its pass"
        );
    }

    /// A round files the copy of a message its server took, and reads the account back before it
    /// ends, so the message goes from this device in the same round rather than the next.
    #[test]
    fn a_round_files_a_copy_and_lets_its_message_go_before_it_ends() {
        if !crate::dev_mail_server::answering() {
            eprintln!("the development mail server is not running; skipping");
            return;
        }
        let sent = SentHere::on_the_development_server("round-files-and-lets-go");
        sent.profile
            .get()
            .expect("a connection")
            .execute("UPDATE outbox SET accepted_at = unixepoch() - 61", [])
            .expect("the clock moves on");

        AccountSync::sync(&*sent.sync, "acc-1").expect("the round runs");

        assert_eq!(
            sent.left(),
            0,
            "a message whose copy the round filed was kept after the round"
        );
    }

    /// What a journal set to one level was asked to keep.
    struct Written(
        std::sync::Mutex<Vec<(Note, &'static str, Vec<crate::ports::journal::Field>)>>,
        Note,
    );

    impl Written {
        fn set_to(level: Note) -> Self {
            Self(std::sync::Mutex::default(), level)
        }
    }

    impl Journal for Written {
        fn write_down(
            &self,
            note: Note,
            event: &'static str,
            fields: &[crate::ports::journal::Field],
        ) {
            self.0
                .lock()
                .expect("test lock")
                .push((note, event, fields.to_vec()));
        }

        fn keeps(&self, note: Note) -> bool {
            note.reaches(self.1)
        }
    }

    /// A sent message whose copy its server will not take is counted where somebody asked for
    /// detail, and nowhere else.
    ///
    /// A line about it at a level every build writes would say when somebody sent mail, near
    /// enough, in a file the databases' key does not cover — the record AGENTS.md section 6 keeps
    /// out of every build. Counted in the round's own line, as a number and nothing else.
    #[test]
    fn a_sent_message_whose_copy_its_server_will_not_take_is_counted_only_where_detail_is_kept() {
        let round = Delivered {
            filed: 1,
            unfiled: 2,
            ..Delivered::default()
        };

        let every_build = Written::set_to(Note::Worth);
        wrote_down_a_round(&every_build, round);
        assert_eq!(
            *every_build.0.lock().expect("test lock"),
            vec![],
            "a round that let messages go unfiled was written down at a level every build keeps"
        );

        let asked_for = Written::set_to(Note::Detail);
        wrote_down_a_round(&asked_for, round);
        assert_eq!(
            *asked_for.0.lock().expect("test lock"),
            vec![(
                Note::Detail,
                "outbox.worked",
                vec![
                    ("sent", Value::Count(0)),
                    ("filed", Value::Count(1)),
                    ("found", Value::Count(0)),
                    ("unfiled", Value::Count(2)),
                    ("waiting", Value::Count(0)),
                    ("givenBack", Value::Count(0)),
                ]
            )]
        );
    }

    /// The two databases stay apart in the log, and together on screen.
    ///
    /// This is the whole reason the journal has its own mapping. A person is offered one thing to
    /// do about either — look at a database that will not open — and somebody reading the log after
    /// the fact needs to know which of the two happened, because a database that could not be
    /// opened and one that opened damaged are different afternoons. Collapse them here and the log
    /// stops answering the only question it is opened for.
    #[test]
    fn the_journal_says_which_database_failure_it_was() {
        assert_eq!(
            as_the_journal_says_it(&UseCaseError::StorageUnavailable),
            Word::TheDatabaseWasUnavailable
        );
        assert_eq!(
            as_the_journal_says_it(&UseCaseError::StorageCorrupted),
            Word::TheDatabaseWasDamaged
        );

        // The same pair, as a person is offered them: one sentence, one thing to do.
        assert_eq!(
            in_a_persons_terms(&UseCaseError::StorageUnavailable),
            WhyItStopped::Storage
        );
        assert_eq!(
            in_a_persons_terms(&UseCaseError::StorageCorrupted),
            WhyItStopped::Storage
        );
    }

    /// Every stop reaches the log as a word, and none of them arrives as the catch-all by accident.
    #[test]
    fn each_reason_a_person_is_given_has_its_own_word() {
        for (why, said, written) in [
            (
                UseCaseError::Offline,
                WhyItStopped::Offline,
                Word::TheNetworkWasNotThere,
            ),
            (
                UseCaseError::Unauthorized,
                WhyItStopped::SignIn,
                Word::TheSignInWasRefused,
            ),
            (
                UseCaseError::CredentialsMissing,
                WhyItStopped::SignIn,
                Word::TheSignInWasRefused,
            ),
            (
                UseCaseError::CredentialsUnavailable,
                WhyItStopped::SignIn,
                Word::TheSignInWasRefused,
            ),
            (
                UseCaseError::ProviderProtocol,
                WhyItStopped::Other,
                Word::SomethingNobodyExpected,
            ),
        ] {
            assert_eq!(in_a_persons_terms(&why), said, "{why:?} on screen");
            assert_eq!(as_the_journal_says_it(&why), written, "{why:?} in the log");
        }
    }
}
