//! How each account is doing, in the authoritative database.

use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::ports::activity::{AccountActivity, ActivityError, WhyItStopped};

pub struct SqliteAccountActivity {
    profile: SqlitePool,
}

impl SqliteAccountActivity {
    pub fn new(profile: SqlitePool) -> Self {
        Self { profile }
    }
}

impl AccountActivity for SqliteAccountActivity {
    fn finished(&self, account_id: &str, at: i64) -> Result<(), ActivityError> {
        self.profile
            .get()
            .map_err(|_| ActivityError::Unavailable)?
            .execute(
                // The reason goes with it: a pass that worked is the answer, and an old reason
                // beside a fresh success is a sentence that contradicts itself.
                "INSERT INTO account_activity (account_id, last_success_at, last_error)
                 VALUES (?1, ?2, NULL)
                 ON CONFLICT (account_id) DO UPDATE
                 SET last_success_at = ?2, last_error = NULL",
                rusqlite::params![account_id, at],
            )
            .map(|_| ())
            .map_err(|_| ActivityError::Unavailable)
    }

    fn stopped(&self, account_id: &str, why: WhyItStopped) -> Result<(), ActivityError> {
        self.profile
            .get()
            .map_err(|_| ActivityError::Unavailable)?
            .execute(
                // When it last worked is kept: "not working" and "you are looking at yesterday"
                // are two different sentences, and a person needs both.
                "INSERT INTO account_activity (account_id, last_success_at, last_error)
                 VALUES (?1, NULL, ?2)
                 ON CONFLICT (account_id) DO UPDATE SET last_error = ?2",
                rusqlite::params![account_id, why.as_str()],
            )
            .map(|_| ())
            .map_err(|_| ActivityError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::fake_provider::SeededProfile;
    use crate::ports::activity::Activity;

    /// Reads it back the way the read model will.
    fn how_it_is_doing(pool: &SqlitePool, account_id: &str) -> Activity {
        pool.get()
            .expect("connection")
            .query_row(
                "SELECT last_success_at, last_error FROM account_activity WHERE account_id = ?1",
                [account_id],
                |row| {
                    Ok(Activity {
                        last_success_at: row.get(0)?,
                        stopped_by: row
                            .get::<_, Option<String>>(1)?
                            .as_deref()
                            .map(WhyItStopped::parse),
                    })
                },
            )
            .unwrap_or_default()
    }

    fn with_an_account(name: &str) -> SeededProfile {
        let profile = SeededProfile::new(name);
        profile.add_account("acc-1", "user@example.com", 1);

        profile
    }

    #[test]
    fn nothing_is_known_about_an_account_no_pass_has_run_for() {
        let profile = with_an_account("activity-empty");

        assert_eq!(how_it_is_doing(&profile.pool, "acc-1"), Activity::default());
    }

    #[test]
    fn a_pass_that_finished_says_when() {
        let profile = with_an_account("activity-finished");
        let activity = SqliteAccountActivity::new(profile.pool.clone());

        activity.finished("acc-1", 1_700_000_000).expect("written");

        assert_eq!(
            how_it_is_doing(&profile.pool, "acc-1"),
            Activity {
                last_success_at: Some(1_700_000_000),
                stopped_by: None
            }
        );
    }

    #[test]
    fn a_pass_that_stopped_keeps_when_one_last_worked() {
        // "Not working" and "you are looking at yesterday" are two different sentences, and
        // somebody deciding whether to trust what is on screen needs both.
        let profile = with_an_account("activity-stopped");
        let activity = SqliteAccountActivity::new(profile.pool.clone());

        activity.finished("acc-1", 1_700_000_000).expect("written");
        activity
            .stopped("acc-1", WhyItStopped::Offline)
            .expect("written");

        assert_eq!(
            how_it_is_doing(&profile.pool, "acc-1"),
            Activity {
                last_success_at: Some(1_700_000_000),
                stopped_by: Some(WhyItStopped::Offline)
            }
        );
    }

    #[test]
    fn a_pass_that_worked_clears_what_stopped_the_last_one() {
        let profile = with_an_account("activity-recovered");
        let activity = SqliteAccountActivity::new(profile.pool.clone());

        activity
            .stopped("acc-1", WhyItStopped::SignIn)
            .expect("written");
        activity.finished("acc-1", 1_700_000_100).expect("written");

        assert_eq!(
            how_it_is_doing(&profile.pool, "acc-1"),
            Activity {
                last_success_at: Some(1_700_000_100),
                stopped_by: None
            }
        );
    }

    #[test]
    fn one_account_is_not_the_other() {
        let profile = with_an_account("activity-accounts");
        profile.add_account("acc-2", "other@example.com", 2);
        let activity = SqliteAccountActivity::new(profile.pool.clone());

        activity.finished("acc-1", 1_700_000_000).expect("written");
        activity
            .stopped("acc-2", WhyItStopped::Storage)
            .expect("written");

        assert_eq!(
            how_it_is_doing(&profile.pool, "acc-1").stopped_by,
            None,
            "one account's failure was written on the other"
        );
        assert_eq!(
            how_it_is_doing(&profile.pool, "acc-2").last_success_at,
            None
        );
    }

    #[test]
    fn a_reason_this_build_has_no_words_for_is_not_invented() {
        for reason in ["offline", "signIn", "storage", "other"] {
            assert_eq!(WhyItStopped::parse(reason).as_str(), reason);
        }
        assert_eq!(WhyItStopped::parse("quota-exceeded"), WhyItStopped::Other);
    }
}
