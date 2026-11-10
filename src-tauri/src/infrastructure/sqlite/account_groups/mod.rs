use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::ports::account_groups::{AccountGroupError, AccountGroups};
use lyonoyl_domain::account_group::{sorted, AccountGroup, GroupName, Member, Placed, Sorting};

/// SQLite-backed implementation of [`AccountGroups`], over the profile database.
///
/// The profile is authoritative: a group is something a person made and no synchronisation will
/// bring it back. All `rusqlite` types are mapped inside this adapter and do not escape it.
pub struct SqliteAccountGroups {
    pool: SqlitePool,
}

impl SqliteAccountGroups {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl AccountGroups for SqliteAccountGroups {
    fn list(&self) -> Result<Vec<AccountGroup>, AccountGroupError> {
        let connection = self
            .pool
            .get()
            .map_err(|_| AccountGroupError::Unavailable)?;

        let mut groups = connection
            .prepare(
                "SELECT id, name, position, collapsed, sorting FROM account_groups
                     ORDER BY position, name",
            )
            .map_err(|_| AccountGroupError::Unavailable)?
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)? != 0,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|_| AccountGroupError::Unavailable)?
            .map(|row| {
                let (id, name, position, collapsed, sorting) =
                    row.map_err(|_| AccountGroupError::Unavailable)?;
                // A name that no longer passes the rule was written by another version. Reported
                // rather than repaired: quietly renaming somebody's group is a worse answer than
                // saying the store holds something this build does not understand.
                let name = GroupName::new(&name).map_err(|_| AccountGroupError::Corrupted)?;

                Ok(AccountGroup {
                    id,
                    name,
                    position,
                    collapsed,
                    sorting: sorting_from(&sorting)?,
                    accounts: Vec::new(),
                })
            })
            .collect::<Result<Vec<_>, AccountGroupError>>()?;

        // One query for the memberships rather than one per group: fifty groups is fifty round
        // trips on every draw of the sidebar, and the answer is one small table either way.
        let mut statement = connection
            .prepare(
                "SELECT group_id, id, display_name, email, created_at FROM mail_accounts
                     WHERE group_id IS NOT NULL
                     ORDER BY position_in_group, created_at, id",
            )
            .map_err(|_| AccountGroupError::Unavailable)?;

        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })
            .map_err(|_| AccountGroupError::Unavailable)?;

        // Read in the order they were put in by hand, which is the order a sort keeps for a tie.
        let mut members: Vec<Vec<Member>> = groups.iter().map(|_| Vec::new()).collect();
        for row in rows {
            let (group_id, account_id, display_name, email, added) =
                row.map_err(|_| AccountGroupError::Unavailable)?;

            if let Some(at) = groups.iter().position(|group| group.id == group_id) {
                members[at].push(Member::new(
                    account_id,
                    &display_name,
                    &email,
                    added,
                    folded,
                ));
            }
        }

        for (group, members) in groups.iter_mut().zip(members) {
            group.accounts = sorted(members, group.sorting);
        }

        Ok(groups)
    }

    fn create(&self, name: &GroupName) -> Result<String, AccountGroupError> {
        let id = uuid::Uuid::new_v4().to_string();

        let mut connection = self
            .pool
            .get()
            .map_err(|_| AccountGroupError::Unavailable)?;

        // Immediate, because the position is read and then written: two groups made at once would
        // otherwise both read the same last place and both take it.
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AccountGroupError::Unavailable)?;

        let last = last_at_the_top(&transaction)?;

        transaction
            .execute(
                "INSERT INTO account_groups (id, name, position) VALUES (?1, ?2, ?3)",
                (&id, name.as_str(), last.map_or(0, |last| last + 1)),
            )
            .map_err(|_| AccountGroupError::Unavailable)?;

        transaction
            .commit()
            .map_err(|_| AccountGroupError::Unavailable)?;

        Ok(id)
    }

    fn rename(&self, group_id: &str, name: &GroupName) -> Result<(), AccountGroupError> {
        let changed = self
            .pool
            .get()
            .map_err(|_| AccountGroupError::Unavailable)?
            .execute(
                "UPDATE account_groups SET name = ?2 WHERE id = ?1",
                (group_id, name.as_str()),
            )
            .map_err(|_| AccountGroupError::Unavailable)?;

        // Nothing changed means nothing was there. Said rather than passed over: a rename that
        // quietly did nothing leaves a screen showing the old name and no reason why.
        if changed == 0 {
            return Err(AccountGroupError::NoSuchGroup);
        }

        Ok(())
    }

    fn remove(&self, group_id: &str) -> Result<(), AccountGroupError> {
        let mut connection = self
            .pool
            .get()
            .map_err(|_| AccountGroupError::Unavailable)?;

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AccountGroupError::Unavailable)?;

        let Some(position) = transaction
            .query_row(
                "SELECT position FROM account_groups WHERE id = ?1",
                [group_id],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|_| AccountGroupError::Unavailable)?
        else {
            return Err(AccountGroupError::NoSuchGroup);
        };

        let members = drawn(&transaction, group_id)?;

        // Its accounts stand where it stood, in the order they were drawn in it: the heading goes,
        // and nothing that was under it moves anywhere else.
        make_room(&transaction, position, members.len() as i64 - 1)?;
        {
            let mut writing = transaction
                .prepare(
                    "UPDATE mail_accounts SET group_id = NULL, position_in_group = ?2
                         WHERE id = ?1",
                )
                .map_err(|_| AccountGroupError::Unavailable)?;

            for (offset, account_id) in members.iter().enumerate() {
                writing
                    .execute(rusqlite::params![account_id, position + offset as i64])
                    .map_err(|_| AccountGroupError::Unavailable)?;
            }
        }

        transaction
            .execute("DELETE FROM account_groups WHERE id = ?1", [group_id])
            .map_err(|_| AccountGroupError::Unavailable)?;

        transaction
            .commit()
            .map_err(|_| AccountGroupError::Unavailable)?;

        Ok(())
    }

    fn place(&self, account_id: &str, group_id: Option<&str>) -> Result<(), AccountGroupError> {
        let mut connection = self
            .pool
            .get()
            .map_err(|_| AccountGroupError::Unavailable)?;

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AccountGroupError::Unavailable)?;

        // Asked before the write so that naming a group that is not here is told apart from naming
        // an account that is not here. The database would refuse both, and with the same error.
        if let Some(group_id) = group_id {
            let known: Option<i64> = transaction
                .query_row(
                    "SELECT 1 FROM account_groups WHERE id = ?1",
                    [group_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|_| AccountGroupError::Unavailable)?;

            if known.is_none() {
                return Err(AccountGroupError::NoSuchGroup);
            }
        }

        let changed = match group_id {
            // Last in the group it joins. Where it sat in the group it left is not kept: coming
            // back to a group is joining it, and a remembered place would put it somewhere nobody
            // chose.
            Some(group_id) => {
                let last: Option<i64> = transaction
                    .query_row(
                        "SELECT max(position_in_group) FROM mail_accounts WHERE group_id = ?1",
                        [group_id],
                        |row| row.get(0),
                    )
                    .map_err(|_| AccountGroupError::Unavailable)?;

                transaction
                    .execute(
                        "UPDATE mail_accounts SET group_id = ?2, position_in_group = ?3
                             WHERE id = ?1",
                        (account_id, group_id, last.map_or(0, |last| last + 1)),
                    )
                    .map_err(|_| AccountGroupError::Unavailable)?
            }
            // Directly below the group it leaves, where the person who took it out is looking. One
            // already in no group is where somebody put it, and stays there.
            None => {
                let left: Option<Option<i64>> = transaction
                    .query_row(
                        "SELECT groups.position FROM mail_accounts AS accounts
                             LEFT JOIN account_groups AS groups ON groups.id = accounts.group_id
                             WHERE accounts.id = ?1",
                        [account_id],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|_| AccountGroupError::Unavailable)?;

                match left {
                    None => 0,
                    Some(None) => 1,
                    Some(Some(position)) => {
                        make_room(&transaction, position, 1)?;
                        transaction
                            .execute(
                                "UPDATE mail_accounts SET group_id = NULL, position_in_group = ?2
                                     WHERE id = ?1",
                                (account_id, position + 1),
                            )
                            .map_err(|_| AccountGroupError::Unavailable)?
                    }
                }
            }
        };

        if changed == 0 {
            return Err(AccountGroupError::NoSuchAccount);
        }

        transaction
            .commit()
            .map_err(|_| AccountGroupError::Unavailable)?;

        Ok(())
    }

    fn reorder(&self, account_ids: &[String]) -> Result<(), AccountGroupError> {
        let mut connection = self
            .pool
            .get()
            .map_err(|_| AccountGroupError::Unavailable)?;

        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| AccountGroupError::Unavailable)?;

        // One statement prepared once and run down the list: a sidebar of a hundred mailboxes is
        // a hundred of these, and preparing each would be a hundred parses of the same line.
        {
            let mut writing = transaction
                .prepare("UPDATE mail_accounts SET position_in_group = ?2 WHERE id = ?1")
                .map_err(|_| AccountGroupError::Unavailable)?;

            for (place, account_id) in account_ids.iter().enumerate() {
                let changed = writing
                    .execute(rusqlite::params![account_id, place as i64])
                    .map_err(|_| AccountGroupError::Unavailable)?;

                // An account the caller named and this device does not hold. Refused rather than
                // skipped: the list is the order, and an order missing one of its entries would
                // leave that account wherever it happened to be.
                if changed == 0 {
                    return Err(AccountGroupError::NoSuchAccount);
                }
            }

            // An order made by hand is the group's order now. A group that kept its sort by name or
            // by date would put the mailbox somebody just dragged straight back.
            let mut by_hand = transaction
                .prepare(
                    "UPDATE account_groups SET sorting = 'manual'
                         WHERE id = (SELECT group_id FROM mail_accounts WHERE id = ?1)",
                )
                .map_err(|_| AccountGroupError::Unavailable)?;

            for account_id in account_ids {
                by_hand
                    .execute([account_id])
                    .map_err(|_| AccountGroupError::Unavailable)?;
            }
        }

        transaction
            .commit()
            .map_err(|_| AccountGroupError::Unavailable)?;

        Ok(())
    }

    fn arrange(&self, order: &[Placed]) -> Result<(), AccountGroupError> {
        let mut connection = self
            .pool
            .get()
            .map_err(|_| AccountGroupError::Unavailable)?;

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| AccountGroupError::Unavailable)?;

        // What is at the top now and the run does not name, in the order it has: read before the
        // run is written over it, because afterwards the old places are gone.
        let named = |kind: i64, id: &str| {
            order.iter().any(|placed| match placed {
                Placed::Group(named) => kind == GROUP && named == id,
                Placed::Account(named) => kind == ACCOUNT && named == id,
            })
        };
        let rest: Vec<(i64, String)> = at_the_top(&transaction)?
            .into_iter()
            .filter(|(kind, id)| !named(*kind, id))
            .collect();

        {
            let mut group_at = transaction
                .prepare("UPDATE account_groups SET position = ?2 WHERE id = ?1")
                .map_err(|_| AccountGroupError::Unavailable)?;
            // An account in a group is not at the top, so naming it here is naming one that is not
            // there. Moving it out is a decision of its own, and `place` is where it is made.
            let mut account_at = transaction
                .prepare(
                    "UPDATE mail_accounts SET position_in_group = ?2
                         WHERE id = ?1 AND group_id IS NULL",
                )
                .map_err(|_| AccountGroupError::Unavailable)?;

            let run = order
                .iter()
                .map(|placed| match placed {
                    Placed::Group(id) => (GROUP, id.as_str()),
                    Placed::Account(id) => (ACCOUNT, id.as_str()),
                })
                .chain(rest.iter().map(|(kind, id)| (*kind, id.as_str())));

            for (place, (kind, id)) in run.enumerate() {
                let (writing, missing) = match kind {
                    GROUP => (&mut group_at, AccountGroupError::NoSuchGroup),
                    _ => (&mut account_at, AccountGroupError::NoSuchAccount),
                };
                let changed = writing
                    .execute(rusqlite::params![id, place as i64])
                    .map_err(|_| AccountGroupError::Unavailable)?;

                if changed == 0 {
                    return Err(missing);
                }
            }
        }

        transaction
            .commit()
            .map_err(|_| AccountGroupError::Unavailable)?;

        Ok(())
    }

    fn set_collapsed(&self, group_id: &str, collapsed: bool) -> Result<(), AccountGroupError> {
        let changed = self
            .pool
            .get()
            .map_err(|_| AccountGroupError::Unavailable)?
            .execute(
                "UPDATE account_groups SET collapsed = ?2 WHERE id = ?1",
                (group_id, i64::from(collapsed)),
            )
            .map_err(|_| AccountGroupError::Unavailable)?;

        if changed == 0 {
            return Err(AccountGroupError::NoSuchGroup);
        }

        Ok(())
    }

    fn sort(&self, group_id: &str, sorting: Sorting) -> Result<(), AccountGroupError> {
        let changed = self
            .pool
            .get()
            .map_err(|_| AccountGroupError::Unavailable)?
            .execute(
                "UPDATE account_groups SET sorting = ?2 WHERE id = ?1",
                (group_id, stored(sorting)),
            )
            .map_err(|_| AccountGroupError::Unavailable)?;

        if changed == 0 {
            return Err(AccountGroupError::NoSuchGroup);
        }

        Ok(())
    }
}

/// How a sorting is written in its column.
fn stored(sorting: Sorting) -> &'static str {
    match sorting {
        Sorting::Manual => "manual",
        Sorting::DateAdded => "date_added",
        Sorting::Name => "name",
    }
}

/// A sorting read back from its column.
fn sorting_from(stored: &str) -> Result<Sorting, AccountGroupError> {
    match stored {
        "manual" => Ok(Sorting::Manual),
        "date_added" => Ok(Sorting::DateAdded),
        "name" => Ok(Sorting::Name),
        // Written by another version, and reported as a name that no longer passes the rule is.
        _ => Err(AccountGroupError::Corrupted),
    }
}

/// Takes the case and the accents off a name, so that a sort by name reads letters rather than the
/// numbers they are stored as: "école" stands among the names in e, not after every one in z.
///
/// Only the marks any alphabet lays over or under a letter are taken off. A vowel sign in
/// Devanagari or a point in Arabic is part of the letter it sits on, and taking it off would make
/// two different names one.
fn folded(name: &str) -> String {
    icu_normalizer::DecomposingNormalizerBorrowed::new_nfd()
        .normalize(name)
        .chars()
        .filter(|character| !laid_over(*character))
        .flat_map(char::to_lowercase)
        .collect()
}

/// The blocks of marks drawn over or under the letters of any alphabet, rather than of one script.
fn laid_over(character: char) -> bool {
    matches!(
        character,
        '\u{0300}'..='\u{036F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}'
    )
}

/// The accounts in one group, in the order the sidebar draws them.
fn drawn(connection: &Connection, group_id: &str) -> Result<Vec<String>, AccountGroupError> {
    let sorting: String = connection
        .query_row(
            "SELECT sorting FROM account_groups WHERE id = ?1",
            [group_id],
            |row| row.get(0),
        )
        .map_err(|_| AccountGroupError::Unavailable)?;

    let members = connection
        .prepare(
            "SELECT id, display_name, email, created_at FROM mail_accounts WHERE group_id = ?1
                 ORDER BY position_in_group, created_at, id",
        )
        .map_err(|_| AccountGroupError::Unavailable)?
        .query_map([group_id], |row| {
            Ok(Member::new(
                row.get(0)?,
                &row.get::<_, String>(1)?,
                &row.get::<_, String>(2)?,
                row.get(3)?,
                folded,
            ))
        })
        .map_err(|_| AccountGroupError::Unavailable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AccountGroupError::Unavailable)?;

    Ok(sorted(members, sorting_from(&sorting)?))
}

/// What a row at the top of the sidebar is, where the two kinds are read together.
const GROUP: i64 = 0;
const ACCOUNT: i64 = 1;

/// Everything at the top of the sidebar, groups and the accounts in no group, in the order drawn.
///
/// At one place, a group comes first: the order every profile had before the two were counted
/// together.
fn at_the_top(connection: &Connection) -> Result<Vec<(i64, String)>, AccountGroupError> {
    connection
        .prepare(
            "SELECT kind, id FROM (
                 SELECT 0 AS kind, id, position AS place, name AS tie FROM account_groups
                 UNION ALL
                 SELECT 1, id, position_in_group, created_at FROM mail_accounts
                     WHERE group_id IS NULL
             )
             ORDER BY place, kind, tie, id",
        )
        .map_err(|_| AccountGroupError::Unavailable)?
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|_| AccountGroupError::Unavailable)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| AccountGroupError::Unavailable)
}

/// The last place taken at the top of the sidebar, by a group or by an account in none.
pub(crate) fn last_at_the_top(connection: &Connection) -> Result<Option<i64>, AccountGroupError> {
    connection
        .query_row(
            "SELECT max(place) FROM (
                 SELECT position AS place FROM account_groups
                 UNION ALL
                 SELECT position_in_group FROM mail_accounts WHERE group_id IS NULL
             )",
            [],
            |row| row.get(0),
        )
        .map_err(|_| AccountGroupError::Unavailable)
}

/// Moves everything at the top of the sidebar that stands below `after` down by `by` places, so
/// that many can be put directly under it.
fn make_room(connection: &Connection, after: i64, by: i64) -> Result<(), AccountGroupError> {
    if by <= 0 {
        return Ok(());
    }

    connection
        .execute(
            "UPDATE account_groups SET position = position + ?2 WHERE position > ?1",
            (after, by),
        )
        .map_err(|_| AccountGroupError::Unavailable)?;
    connection
        .execute(
            "UPDATE mail_accounts SET position_in_group = position_in_group + ?2
                 WHERE group_id IS NULL AND position_in_group > ?1",
            (after, by),
        )
        .map_err(|_| AccountGroupError::Unavailable)?;

    Ok(())
}

#[cfg(test)]
mod tests;
