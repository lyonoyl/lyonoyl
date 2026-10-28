use rusqlite::{OptionalExtension, TransactionBehavior};

use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::ports::account_repository::{AccountRepository, AccountRepositoryError};
use lyonoyl_domain::account::{
    Encryption, MailAccount, MailAccountSummary, ServerSettings, Submission,
};

/// SQLite-backed implementation of [`AccountRepository`].
///
/// All `rusqlite` types are mapped inside this adapter and do not escape it.
pub struct SqliteAccountRepository {
    pool: SqlitePool,
}

impl SqliteAccountRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl AccountRepository for SqliteAccountRepository {
    fn able_to_send(&self) -> Result<Vec<String>, AccountRepositoryError> {
        let connection = self
            .pool
            .get()
            .map_err(|_| AccountRepositoryError::Unavailable)?;

        let mut statement = connection
            .prepare(
                // All four or none, the same rule the account itself is read by: a host with no
                // port is not half a sending server.
                "SELECT id FROM mail_accounts
                 WHERE submission_host IS NOT NULL
                   AND submission_port IS NOT NULL
                   AND submission_encryption IS NOT NULL
                   AND submission_username IS NOT NULL",
            )
            .map_err(|_| AccountRepositoryError::Unavailable)?;

        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|_| AccountRepositoryError::Unavailable)?;

        let mut able = Vec::new();
        for row in rows {
            able.push(row.map_err(|_| AccountRepositoryError::Unavailable)?);
        }

        Ok(able)
    }

    fn find(&self, account_id: &str) -> Result<Option<MailAccount>, AccountRepositoryError> {
        self.read_one(account_id)
    }

    fn find_by_email(&self, email: &str) -> Result<Option<String>, AccountRepositoryError> {
        self.pool
            .get()
            .map_err(|_| AccountRepositoryError::Unavailable)?
            .query_row(
                // Addresses are compared without case: a mail server does not distinguish them.
                "SELECT id FROM mail_accounts WHERE lower(email) = lower(?1)",
                [email],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| AccountRepositoryError::Unavailable)
    }

    fn list(&self) -> Result<Vec<MailAccountSummary>, AccountRepositoryError> {
        let connection = self
            .pool
            .get()
            .map_err(|_| AccountRepositoryError::Unavailable)?;

        let mut statement = connection
            .prepare(
                "SELECT id, email, display_name, provider
                 FROM mail_accounts
                 ORDER BY created_at ASC, id ASC",
            )
            .map_err(|_| AccountRepositoryError::Unavailable)?;

        // Rows are read as plain strings and only then validated by the domain: a row written by
        // an older version may violate this build's invariants, and that must not read as "unavailable".
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|_| AccountRepositoryError::Unavailable)?;

        let mut accounts = Vec::new();
        for row in rows {
            let (id, email, display_name, provider) =
                row.map_err(|_| AccountRepositoryError::Unavailable)?;
            let account = MailAccountSummary::new(id, email, display_name, provider)
                .map_err(|_| AccountRepositoryError::Corrupted)?;
            accounts.push(account);
        }
        Ok(accounts)
    }

    /// Stores an account, giving a new one the number the mail index will know it by.
    ///
    /// One transaction: the number is taken from the counter and the row is written together, so
    /// two accounts added at the same moment cannot leave with the same number.
    fn save(&self, account: &MailAccount) -> Result<(), AccountRepositoryError> {
        let mut connection = self
            .pool
            .get()
            .map_err(|_| AccountRepositoryError::Unavailable)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AccountRepositoryError::Unavailable)?;

        let summary = account.summary();
        let imap = account.imap();

        let existing: Option<i64> = transaction
            .query_row(
                "SELECT index_id FROM mail_accounts WHERE id = ?1",
                [summary.id()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| AccountRepositoryError::Unavailable)?;

        let index_id = match existing {
            Some(number) => number,
            None => take_next_index_id(&transaction)?,
        };

        // Where a new account stands: last at the top of the sidebar, below every group and every
        // account in none. One already here keeps its place, which the update below leaves alone.
        let place = crate::infrastructure::sqlite::account_groups::last_at_the_top(&transaction)
            .map_err(|_| AccountRepositoryError::Unavailable)?
            .map_or(0, |last| last + 1);

        transaction
            .execute(
                "INSERT INTO mail_accounts
                     (id, email, display_name, provider,
                      imap_host, imap_port, imap_encryption, imap_username, index_id,
                      submission_host, submission_port, submission_encryption,
                      submission_username, position_in_group)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
                 ON CONFLICT(id) DO UPDATE SET
                     email           = excluded.email,
                     display_name    = excluded.display_name,
                     provider        = excluded.provider,
                     imap_host       = excluded.imap_host,
                     imap_port       = excluded.imap_port,
                     imap_encryption = excluded.imap_encryption,
                     imap_username   = excluded.imap_username,
                     submission_host       = excluded.submission_host,
                     submission_port       = excluded.submission_port,
                     submission_encryption = excluded.submission_encryption,
                     submission_username   = excluded.submission_username",
                (
                    summary.id(),
                    summary.email(),
                    summary.display_name(),
                    summary.provider(),
                    imap.host(),
                    i64::from(imap.port()),
                    imap.encryption().stored(),
                    account.imap_username(),
                    index_id,
                    account.submission().map(|sending| sending.server().host()),
                    account
                        .submission()
                        .map(|sending| i64::from(sending.server().port())),
                    account
                        .submission()
                        .map(|sending| sending.server().encryption().stored()),
                    account.submission().map(|sending| sending.username()),
                    place,
                ),
            )
            .map_err(|_| AccountRepositoryError::Unavailable)?;

        transaction
            .commit()
            .map_err(|_| AccountRepositoryError::Unavailable)
    }

    fn set_collapsed(
        &self,
        account_id: &str,
        collapsed: bool,
    ) -> Result<(), AccountRepositoryError> {
        self.pool
            .get()
            .map_err(|_| AccountRepositoryError::Unavailable)?
            .execute(
                "UPDATE mail_accounts SET sidebar_collapsed = ?2 WHERE id = ?1",
                (account_id, i64::from(collapsed)),
            )
            .map(|_| ())
            .map_err(|_| AccountRepositoryError::Unavailable)
    }

    fn number_of(&self, account_id: &str) -> Result<Option<i64>, AccountRepositoryError> {
        self.pool
            .get()
            .map_err(|_| AccountRepositoryError::Unavailable)?
            .query_row(
                "SELECT index_id FROM mail_accounts WHERE id = ?1",
                [account_id],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                _ => Err(AccountRepositoryError::Unavailable),
            })
    }

    fn delete(&self, account_id: &str) -> Result<(), AccountRepositoryError> {
        self.pool
            .get()
            .map_err(|_| AccountRepositoryError::Unavailable)?
            .execute("DELETE FROM mail_accounts WHERE id = ?1", [account_id])
            .map(|_| ())
            .map_err(|_| AccountRepositoryError::Unavailable)
    }
}

/// How many accounts this device can number.
///
/// A message identifier carries the index it came from in its top bits and the row in the rest, so
/// the two share one signed 64-bit number: 40 bits for the row, and what is left below the sign bit
/// for the account. Past this, two accounts would mint the same identifier.
const MOST_ACCOUNTS: i64 = (1 << 23) - 1;

/// Hands out the next account number and moves the counter past it.
///
/// A counter rather than `max(index_id) + 1`: removing the newest account would otherwise free its
/// number, and the next account added would inherit its folders and its mail from the index. That
/// makes the counter climb even as accounts come and go, so it is the counter that has a ceiling
/// rather than the number of accounts kept.
fn take_next_index_id(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<i64, AccountRepositoryError> {
    let next: i64 = transaction
        .query_row("SELECT next FROM account_index_counter", [], |row| {
            row.get(0)
        })
        .map_err(|_| AccountRepositoryError::Unavailable)?;

    if next > MOST_ACCOUNTS {
        return Err(AccountRepositoryError::NoNumbersLeft);
    }

    transaction
        .execute("UPDATE account_index_counter SET next = next + 1", [])
        .map_err(|_| AccountRepositoryError::Unavailable)?;

    Ok(next)
}

impl SqliteAccountRepository {
    fn read_one(&self, id: &str) -> Result<Option<MailAccount>, AccountRepositoryError> {
        let connection = self
            .pool
            .get()
            .map_err(|_| AccountRepositoryError::Unavailable)?;

        let row = connection
            .query_row(
                "SELECT id, email, display_name, provider,
                        imap_host, imap_port, imap_encryption, imap_username,
                        submission_host, submission_port, submission_encryption,
                        submission_username
                 FROM mail_accounts WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, Option<i64>>(9)?,
                        row.get::<_, Option<i64>>(10)?,
                        row.get::<_, Option<String>>(11)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| AccountRepositoryError::Unavailable)?;

        let Some((
            id,
            email,
            display_name,
            provider,
            host,
            port,
            encryption,
            username,
            sending_host,
            sending_port,
            sending_encryption,
            sending_username,
        )) = row
        else {
            return Ok(None);
        };

        // Rows are validated by the domain after reading: one written by an older version may
        // violate this build's invariants, and that is corruption rather than a missing store.
        let summary = MailAccountSummary::new(id, email, display_name, provider)
            .map_err(|_| AccountRepositoryError::Corrupted)?;
        let settings = ServerSettings::new(
            host,
            u16::try_from(port).map_err(|_| AccountRepositoryError::Corrupted)?,
            Encryption::from_stored(encryption),
        )
        .map_err(|_| AccountRepositoryError::Corrupted)?;

        let account = MailAccount::new(summary, settings, username)
            .map_err(|_| AccountRepositoryError::Corrupted)?;

        // All four or none. A row with a host and no port is not half a sending server, it is a
        // row nothing wrote — and treating it as one would hand a message to port zero.
        let sending = match (
            sending_host,
            sending_port,
            sending_encryption,
            sending_username,
        ) {
            (Some(host), Some(port), Some(encryption), Some(username)) => Some(
                Submission::new(
                    ServerSettings::new(
                        host,
                        u16::try_from(port).map_err(|_| AccountRepositoryError::Corrupted)?,
                        Encryption::from_stored(encryption),
                    )
                    .map_err(|_| AccountRepositoryError::Corrupted)?,
                    username,
                )
                .map_err(|_| AccountRepositoryError::Corrupted)?,
            ),
            _ => None,
        };

        Ok(Some(match sending {
            Some(sending) => account.sending_through(sending),
            None => account,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::secure_storage::database_key::DatabaseKey;
    use crate::infrastructure::sqlite::{connection::open_profile, migrations::migrate_profile};

    struct TempDatabase {
        dir: std::path::PathBuf,
        pool: SqlitePool,
    }

    impl TempDatabase {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "lyonoyl-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::create_dir_all(&dir).expect("temp dir is creatable");
            let path = dir.join("profile.sqlite3");
            let pool = open_profile(&path, &DatabaseKey::for_a_test()).expect("pool opens");
            let mut connection = pool.get().expect("connection is available");
            migrate_profile(&mut connection, &path.display().to_string()).expect("schema applies");
            drop(connection);
            Self { dir, pool }
        }
    }

    impl Drop for TempDatabase {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    fn summary(id: &str, email: &str) -> MailAccountSummary {
        MailAccountSummary::new(id, email, "User", "imap").expect("fixture is well-formed")
    }

    fn account(id: &str, email: &str) -> MailAccount {
        MailAccount::new(
            summary(id, email),
            ServerSettings::new("imap.example.com", 993, Encryption::Tls).expect("valid server"),
            email,
        )
        .expect("fixture is well-formed")
    }

    #[test]
    fn an_empty_store_lists_nothing() {
        let db = TempDatabase::new("empty");
        let repository = SqliteAccountRepository::new(db.pool.clone());

        assert!(repository.list().expect("listing succeeds").is_empty());
    }

    #[test]
    fn saves_and_reads_back_an_account() {
        let db = TempDatabase::new("roundtrip");
        let repository = SqliteAccountRepository::new(db.pool.clone());

        repository
            .save(&account("acc-1", "user@example.com"))
            .expect("saving succeeds");

        let stored = repository.list().expect("listing succeeds");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].email(), "user@example.com");
    }

    #[test]
    fn saving_the_same_identifier_twice_updates_instead_of_duplicating() {
        let db = TempDatabase::new("idempotent");
        let repository = SqliteAccountRepository::new(db.pool.clone());

        repository
            .save(&account("acc-1", "first@example.com"))
            .expect("first save");
        repository
            .save(&account("acc-1", "second@example.com"))
            .expect("second save");

        let stored = repository.list().expect("listing succeeds");
        assert_eq!(stored.len(), 1, "upsert must not duplicate the row");
        assert_eq!(stored[0].email(), "second@example.com");
    }

    #[test]
    fn a_row_violating_a_domain_invariant_is_reported_as_corrupted_not_unavailable() {
        let db = TempDatabase::new("corrupted");
        let repository = SqliteAccountRepository::new(db.pool.clone());

        // Simulates a row written by an older version, bypassing the domain constructor.
        db.pool
            .get()
            .expect("connection is available")
            .execute(
                "INSERT INTO mail_accounts (id, email, display_name, provider)
                 VALUES ('acc-broken', 'not-an-address', 'User', 'imap')",
                [],
            )
            .expect("raw insert succeeds");

        let error = repository
            .list()
            .expect_err("an invalid row surfaces as an error");
        assert!(
            matches!(error, AccountRepositoryError::Corrupted),
            "the user must be told the data is invalid, not that the store is unreachable"
        );
    }

    #[test]
    fn an_account_keeps_the_settings_needed_to_reach_its_mail() {
        let db = TempDatabase::new("settings");
        let repository = SqliteAccountRepository::new(db.pool.clone());

        let stored = MailAccount::new(
            summary("acc-1", "user@example.com"),
            ServerSettings::new("mail.example.com", 143, Encryption::StartTls).expect("valid"),
            "user",
        )
        .expect("valid account");
        repository.save(&stored).expect("save succeeds");

        let read = repository
            .find("acc-1")
            .expect("read succeeds")
            .expect("the account is there");

        assert_eq!(read.imap().host(), "mail.example.com");
        assert_eq!(read.imap().port(), 143);
        assert_eq!(read.imap().encryption(), Encryption::StartTls);
        assert_eq!(read.imap_username(), "user");
    }

    #[test]
    fn an_account_can_be_removed_after_a_failed_attempt() {
        let db = TempDatabase::new("delete");
        let repository = SqliteAccountRepository::new(db.pool.clone());
        repository
            .save(&account("acc-1", "user@example.com"))
            .expect("save succeeds");

        repository.delete("acc-1").expect("delete succeeds");

        assert!(repository.find("acc-1").expect("read succeeds").is_none());
        // Removing what is not there is not a failure.
        repository
            .delete("acc-1")
            .expect("deleting twice is allowed");
    }

    fn index_id_of(pool: &SqlitePool, account_id: &str) -> i64 {
        pool.get()
            .expect("connection")
            .query_row(
                "SELECT index_id FROM mail_accounts WHERE id = ?1",
                [account_id],
                |row| row.get(0),
            )
            .expect("the account is there")
    }

    #[test]
    fn each_account_gets_its_own_number_for_the_index() {
        let db = TempDatabase::new("index-numbers");
        let repository = SqliteAccountRepository::new(db.pool.clone());

        repository
            .save(&account("acc-1", "one@example.com"))
            .expect("the first account is stored");
        repository
            .save(&account("acc-2", "two@example.com"))
            .expect("the second account is stored");

        let first = index_id_of(&db.pool, "acc-1");
        let second = index_id_of(&db.pool, "acc-2");

        assert!(first >= 1 && second >= 1);
        assert_ne!(
            first, second,
            "one number for two accounts would mix their mail"
        );
    }

    #[test]
    fn saving_an_account_again_keeps_the_number_it_already_had() {
        // The index addresses folders and messages by it; a new number would orphan all of them.
        let db = TempDatabase::new("index-stable");
        let repository = SqliteAccountRepository::new(db.pool.clone());

        repository
            .save(&account("acc-1", "user@example.com"))
            .expect("the account is stored");
        let before = index_id_of(&db.pool, "acc-1");

        repository
            .save(&account("acc-1", "corrected@example.com"))
            .expect("the account is stored again");

        assert_eq!(index_id_of(&db.pool, "acc-1"), before);
    }

    #[test]
    fn a_number_freed_by_a_removed_account_is_not_handed_out_again() {
        // It would hand the new account the folders and the mail the old one left in the index.
        let db = TempDatabase::new("index-no-reuse");
        let repository = SqliteAccountRepository::new(db.pool.clone());

        repository
            .save(&account("acc-1", "one@example.com"))
            .expect("stored");
        let freed = index_id_of(&db.pool, "acc-1");
        repository.delete("acc-1").expect("removed");

        repository
            .save(&account("acc-2", "two@example.com"))
            .expect("stored");

        assert_ne!(index_id_of(&db.pool, "acc-2"), freed);
    }

    #[test]
    fn a_folded_away_account_is_still_folded_away_after_a_restart() {
        let db = TempDatabase::new("collapsed");
        let repository = SqliteAccountRepository::new(db.pool.clone());
        repository
            .save(&account("acc-1", "user@example.com"))
            .expect("stored");

        let collapsed = |pool: &SqlitePool| -> i64 {
            pool.get()
                .expect("connection")
                .query_row(
                    "SELECT sidebar_collapsed FROM mail_accounts WHERE id = 'acc-1'",
                    [],
                    |row| row.get(0),
                )
                .expect("the account is there")
        };

        assert_eq!(collapsed(&db.pool), 0, "an account starts unfolded");

        repository
            .set_collapsed("acc-1", true)
            .expect("folded away");
        assert_eq!(collapsed(&db.pool), 1);

        repository.set_collapsed("acc-1", false).expect("unfolded");
        assert_eq!(collapsed(&db.pool), 0);
    }

    #[test]
    fn saving_an_account_again_does_not_unfold_it() {
        let db = TempDatabase::new("collapsed-kept");
        let repository = SqliteAccountRepository::new(db.pool.clone());
        repository
            .save(&account("acc-1", "user@example.com"))
            .expect("stored");
        repository
            .set_collapsed("acc-1", true)
            .expect("folded away");

        repository
            .save(&account("acc-1", "user@example.com"))
            .expect("stored again");

        let collapsed: i64 = db
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT sidebar_collapsed FROM mail_accounts WHERE id = 'acc-1'",
                [],
                |row| row.get(0),
            )
            .expect("the account is there");
        assert_eq!(collapsed, 1);
    }

    #[test]
    fn a_new_account_stands_last_at_the_top_and_keeps_its_place_when_saved_again() {
        let db = TempDatabase::new("placed-last");
        let repository = SqliteAccountRepository::new(db.pool.clone());
        let connection = db.pool.get().expect("connection is available");
        connection
            .execute(
                "INSERT INTO account_groups (id, name, position) VALUES ('g-work', 'Work', 5)",
                [],
            )
            .expect("a group stands at five");
        let place_of = |id: &str| -> i64 {
            connection
                .query_row(
                    "SELECT position_in_group FROM mail_accounts WHERE id = ?1",
                    [id],
                    |row| row.get(0),
                )
                .expect("readable")
        };

        repository
            .save(&account("acc-1", "zeta@example.com"))
            .expect("first");
        repository
            .save(&account("acc-2", "kappa@example.net"))
            .expect("second");
        assert_eq!((place_of("acc-1"), place_of("acc-2")), (6, 7));

        repository
            .save(&account("acc-1", "zeta@example.com"))
            .expect("saved again");
        assert_eq!(
            place_of("acc-1"),
            6,
            "an account already here keeps its place"
        );
    }

    #[test]
    fn find_returns_none_for_an_unknown_identifier() {
        let db = TempDatabase::new("find-missing");
        let repository = SqliteAccountRepository::new(db.pool.clone());

        assert!(repository.find("nope").expect("lookup succeeds").is_none());
    }

    #[test]
    fn where_an_account_sends_from_survives_being_stored_and_read_back() {
        let profile = TempDatabase::new("sending");
        let repository = SqliteAccountRepository::new(profile.pool.clone());

        let sending = Submission::new(
            ServerSettings::new("smtp.example.com", 587, Encryption::StartTls).expect("valid"),
            "zeta@example.com",
        )
        .expect("valid");

        repository
            .save(
                &MailAccount::new(
                    MailAccountSummary::new("acc-1", "zeta@example.com", "Zeta", "imap")
                        .expect("valid"),
                    ServerSettings::new("imap.example.com", 993, Encryption::Tls).expect("valid"),
                    "zeta@example.com",
                )
                .expect("valid")
                .sending_through(sending.clone()),
            )
            .expect("it is stored");

        let read = repository
            .find("acc-1")
            .expect("readable")
            .expect("the account is there");

        assert_eq!(read.submission(), Some(&sending));
    }

    #[test]
    fn an_account_nobody_has_given_a_sending_server_reads_back_without_one() {
        let profile = TempDatabase::new("no-sending");
        let repository = SqliteAccountRepository::new(profile.pool.clone());

        repository
            .save(
                &MailAccount::new(
                    MailAccountSummary::new("acc-1", "zeta@example.com", "Zeta", "imap")
                        .expect("valid"),
                    ServerSettings::new("imap.example.com", 993, Encryption::Tls).expect("valid"),
                    "zeta@example.com",
                )
                .expect("valid"),
            )
            .expect("it is stored");

        let read = repository
            .find("acc-1")
            .expect("readable")
            .expect("the account is there");

        assert_eq!(read.submission(), None);
    }

    #[test]
    fn half_a_sending_server_is_no_sending_server() {
        // A host with no port is not something to hand a message to on port zero.
        let profile = TempDatabase::new("half");
        let repository = SqliteAccountRepository::new(profile.pool.clone());

        repository
            .save(
                &MailAccount::new(
                    MailAccountSummary::new("acc-1", "zeta@example.com", "Zeta", "imap")
                        .expect("valid"),
                    ServerSettings::new("imap.example.com", 993, Encryption::Tls).expect("valid"),
                    "zeta@example.com",
                )
                .expect("valid"),
            )
            .expect("it is stored");

        profile
            .pool
            .get()
            .expect("connection")
            .execute(
                "UPDATE mail_accounts SET submission_host = 'smtp.example.com' WHERE id = 'acc-1'",
                [],
            )
            .expect("a row written by something else");

        assert_eq!(
            repository
                .find("acc-1")
                .expect("readable")
                .expect("the account is there")
                .submission(),
            None
        );
    }
}
