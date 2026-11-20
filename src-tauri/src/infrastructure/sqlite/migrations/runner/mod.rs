//! What takes a database from the version it is at to the version this build writes.
//!
//! A schema is a list of steps, kept in the order they were added and never edited once applied.
//! The version is the database's own `user_version`: how many of the steps it has taken. A database
//! at 0 has taken none, one at the length of the list has taken them all, and one past the length
//! was written by a newer build, which this one leaves exactly as it is.
//!
//! The steps a database still has to take are taken as one transaction, opened `IMMEDIATE` like
//! every transaction that writes (invariant 6), with the new version written inside it: a run that
//! fails partway leaves the database at the version it started from, with none of the run's steps
//! in it.

use rusqlite::{Connection, Transaction, TransactionBehavior};

#[cfg(test)]
mod tests;

/// Code a step runs inside its transaction, for what its SQL cannot say on its own.
pub type Hook = fn(&Transaction) -> rusqlite::Result<()>;

/// One step of a schema: what takes a database up one version, and what takes it back.
pub struct Migration {
    up: &'static str,
    up_hook: Option<Hook>,
    down: Option<&'static str>,
    down_hook: Option<Hook>,
}

impl Migration {
    /// A step that is its SQL.
    pub const fn up(sql: &'static str) -> Self {
        Self {
            up: sql,
            up_hook: None,
            down: None,
            down_hook: None,
        }
    }

    /// A step that is its SQL and then its hook, both in the same transaction.
    pub const fn up_with_hook(sql: &'static str, hook: Hook) -> Self {
        Self {
            up: sql,
            up_hook: Some(hook),
            down: None,
            down_hook: None,
        }
    }

    /// The SQL that takes the step back.
    pub const fn down(mut self, sql: &'static str) -> Self {
        self.down = Some(sql);
        self
    }

    /// The hook and then the SQL that take the step back: the way back undoes what the step did
    /// last first.
    pub const fn down_with_hook(mut self, sql: &'static str, hook: Hook) -> Self {
        self.down = Some(sql);
        self.down_hook = Some(hook);
        self
    }
}

/// Why a database could not be taken to the version asked for. Each leaves it as it was.
#[derive(Debug, thiserror::Error)]
pub enum MigrationError {
    /// A step, counted from one, failed in its SQL or its hook.
    #[error("step {step} failed: {source}")]
    Step {
        step: usize,
        #[source]
        source: rusqlite::Error,
    },
    /// The database was written by a build with more steps than this one.
    #[error("the database is at version {found}, past the {known} steps this build has")]
    Ahead { found: usize, known: usize },
    /// A version was asked for that the list does not reach.
    #[error("version {asked} is past the {known} steps this list has")]
    BeyondTheList { asked: usize, known: usize },
    /// Going back would pass a step, counted from one, that has no way back.
    #[error("step {step} has no way back")]
    NoWayBack { step: usize },
    /// The database holds a version no list counts to.
    #[error("the database says its version is {0}")]
    UnreadableVersion(i64),
    /// The database refused the transaction itself, or the version.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

/// The steps of one kind of database, in the order they were added.
pub struct Migrations {
    steps: Vec<Migration>,
}

impl Migrations {
    pub fn new(steps: Vec<Migration>) -> Self {
        Self { steps }
    }

    /// The version this list takes a database to: the number of its steps.
    pub fn latest(&self) -> usize {
        self.steps.len()
    }

    /// The version a database is at.
    pub fn version_of(connection: &Connection) -> Result<usize, MigrationError> {
        let found: i64 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        usize::try_from(found).map_err(|_| MigrationError::UnreadableVersion(found))
    }

    /// Takes the database through every step it has not taken yet.
    pub fn to_latest(&self, connection: &mut Connection) -> Result<(), MigrationError> {
        self.to_version(connection, self.latest())
    }

    /// Takes the database to the version asked for, through the steps up or their ways back down.
    ///
    /// A database already there is not written to at all.
    pub fn to_version(
        &self,
        connection: &mut Connection,
        asked: usize,
    ) -> Result<(), MigrationError> {
        let known = self.latest();
        if asked > known {
            return Err(MigrationError::BeyondTheList { asked, known });
        }

        let found = Self::version_of(connection)?;
        if found > known {
            return Err(MigrationError::Ahead { found, known });
        }

        match found.cmp(&asked) {
            std::cmp::Ordering::Equal => Ok(()),
            std::cmp::Ordering::Less => self.up(connection, found, asked),
            std::cmp::Ordering::Greater => self.down(connection, found, asked),
        }
    }

    /// Takes a database of its own, in memory, through every step: a step whose SQL does not run
    /// fails here rather than on somebody's profile.
    #[cfg(test)]
    pub fn validate(&self) -> Result<(), MigrationError> {
        let mut connection = Connection::open_in_memory()?;
        self.to_latest(&mut connection)
    }

    fn up(
        &self,
        connection: &mut Connection,
        from: usize,
        to: usize,
    ) -> Result<(), MigrationError> {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

        for (at, step) in self.steps.iter().enumerate().take(to).skip(from) {
            let failed = |source| MigrationError::Step {
                step: at + 1,
                source,
            };
            transaction.execute_batch(step.up).map_err(failed)?;
            if let Some(hook) = step.up_hook {
                hook(&transaction).map_err(failed)?;
            }
        }

        stand_at(&transaction, to)?;
        transaction.commit()?;
        Ok(())
    }

    fn down(
        &self,
        connection: &mut Connection,
        from: usize,
        to: usize,
    ) -> Result<(), MigrationError> {
        // Every way back is there before anything is undone, so a refusal leaves nothing half done.
        let mut ways_back = Vec::with_capacity(from - to);
        for (at, step) in self.steps.iter().enumerate().take(from).skip(to).rev() {
            let Some(down) = step.down else {
                return Err(MigrationError::NoWayBack { step: at + 1 });
            };
            ways_back.push((at, down, step.down_hook));
        }

        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

        for (at, down, hook) in ways_back {
            let failed = |source| MigrationError::Step {
                step: at + 1,
                source,
            };
            if let Some(hook) = hook {
                hook(&transaction).map_err(failed)?;
            }
            transaction.execute_batch(down).map_err(failed)?;
        }

        stand_at(&transaction, to)?;
        transaction.commit()?;
        Ok(())
    }
}

/// Writes the version the run leaves the database at, inside the run's own transaction.
fn stand_at(transaction: &Transaction, version: usize) -> Result<(), MigrationError> {
    // SQLite keeps the version in 32 bits, and no list here comes near that many steps.
    let version = i32::try_from(version).map_err(|_| MigrationError::BeyondTheList {
        asked: version,
        known: i32::MAX as usize,
    })?;
    transaction.pragma_update(None, "user_version", version)?;
    Ok(())
}
