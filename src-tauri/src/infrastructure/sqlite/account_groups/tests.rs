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

    /// An account, written directly: what this adapter arranges is accounts that already exist.
    ///
    /// The index number is given here rather than left to a default, because it is unique and a
    /// default would let only one account exist — which the fixture would report as a failure of
    /// the thing under test rather than of itself.
    fn with_account(&self, id: &str) -> &Self {
        let connection = self.pool.get().expect("connection is available");
        let next: i64 = connection
            .query_row("SELECT count(*) + 1 FROM mail_accounts", [], |row| {
                row.get(0)
            })
            .expect("countable");

        // Last at the top of the sidebar, where an account added on the account screen stands.
        connection
            .execute(
                "INSERT INTO mail_accounts
                     (id, email, display_name, provider, index_id, position_in_group)
                     VALUES (?1, ?1 || '@example.com', '', 'imap', ?2, (
                         SELECT coalesce(max(place), -1) + 1 FROM (
                             SELECT position AS place FROM account_groups
                             UNION ALL
                             SELECT position_in_group FROM mail_accounts WHERE group_id IS NULL
                         )
                     ))",
                (id, next),
            )
            .expect("an account is written");
        self
    }

    /// Everything at the top of the sidebar, in the order it is drawn: a group by its name, an
    /// account by its identifier. Read from the tables directly, and refused where two places
    /// agree, which would be an order with two answers.
    fn top(&self) -> Vec<String> {
        let connection = self.pool.get().expect("connection is available");
        let mut placed: Vec<(i64, String)> = Vec::new();

        for sql in [
            "SELECT position, name FROM account_groups",
            "SELECT position_in_group, id FROM mail_accounts WHERE group_id IS NULL",
        ] {
            let mut statement = connection.prepare(sql).expect("readable");
            let rows = statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .expect("readable");
            for row in rows {
                placed.push(row.expect("a row"));
            }
        }

        placed.sort();
        let places: std::collections::HashSet<i64> =
            placed.iter().map(|(place, _)| *place).collect();
        assert_eq!(places.len(), placed.len(), "two share a place: {placed:?}");

        placed.into_iter().map(|(_, label)| label).collect()
    }
}

impl Drop for TempDatabase {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

fn name(text: &str) -> GroupName {
    GroupName::new(text).expect("fixture is a valid name")
}

#[test]
fn an_arrangement_nobody_has_made_is_empty() {
    let store = SqliteAccountGroups::new(TempDatabase::new("groups-empty").pool.clone());

    assert_eq!(store.list().expect("readable"), Vec::new());
}

#[test]
fn a_group_is_made_empty_and_last() {
    let database = TempDatabase::new("groups-order");
    let store = SqliteAccountGroups::new(database.pool.clone());

    store.create(&name("Work")).expect("first group");
    store.create(&name("Personal")).expect("second group");

    let groups = store.list().expect("readable");

    // In the order they were made, not in the order of their names: the sidebar is the person's
    // arrangement, and sorting it alphabetically would be the application rearranging it.
    let drawn: Vec<&str> = groups.iter().map(|group| group.name.as_str()).collect();
    assert_eq!(drawn, vec!["Work", "Personal"]);
    assert!(
        groups.iter().all(|group| group.accounts.is_empty()),
        "a new group holds nothing"
    );
}

#[test]
fn an_account_is_in_one_group_and_moving_it_does_not_leave_it_in_two() {
    let database = TempDatabase::new("groups-one-place");
    database.with_account("acc-a");
    let store = SqliteAccountGroups::new(database.pool.clone());

    let work = store.create(&name("Work")).expect("a group");
    let personal = store.create(&name("Personal")).expect("another group");

    store.place("acc-a", Some(&work)).expect("joins the first");
    store
        .place("acc-a", Some(&personal))
        .expect("moves to the second");

    let groups = store.list().expect("readable");
    let holding: Vec<&str> = groups
        .iter()
        .filter(|group| !group.accounts.is_empty())
        .map(|group| group.name.as_str())
        .collect();

    assert_eq!(holding, vec!["Personal"], "an account is in one group");
}

#[test]
fn taking_an_account_out_returns_it_to_the_top_of_the_sidebar() {
    let database = TempDatabase::new("groups-out");
    database.with_account("acc-a");
    let store = SqliteAccountGroups::new(database.pool.clone());

    let work = store.create(&name("Work")).expect("a group");
    store.place("acc-a", Some(&work)).expect("joins");
    store.place("acc-a", None).expect("leaves");

    let groups = store.list().expect("readable");

    assert!(
        groups.iter().all(|group| group.accounts.is_empty()),
        "nothing holds it any more"
    );
}

#[test]
fn accounts_are_drawn_in_the_order_they_joined() {
    let database = TempDatabase::new("groups-member-order");
    database.with_account("acc-a").with_account("acc-b");
    let store = SqliteAccountGroups::new(database.pool.clone());

    let work = store.create(&name("Work")).expect("a group");
    store.place("acc-b", Some(&work)).expect("joins first");
    store.place("acc-a", Some(&work)).expect("joins second");

    let groups = store.list().expect("readable");

    // `acc-b` before `acc-a` although the identifiers sort the other way: the order is the one the
    // person made, and a list that fell back on the identifier would look sorted by accident.
    assert_eq!(groups[0].accounts, vec!["acc-b", "acc-a"]);
}

#[test]
fn coming_back_to_a_group_joins_it_rather_than_reclaiming_a_place() {
    let database = TempDatabase::new("groups-rejoin");
    database.with_account("acc-a").with_account("acc-b");
    let store = SqliteAccountGroups::new(database.pool.clone());

    let work = store.create(&name("Work")).expect("a group");
    store.place("acc-a", Some(&work)).expect("first");
    store.place("acc-b", Some(&work)).expect("second");

    store.place("acc-a", None).expect("leaves");
    store.place("acc-a", Some(&work)).expect("comes back");

    let groups = store.list().expect("readable");

    assert_eq!(
        groups[0].accounts,
        vec!["acc-b", "acc-a"],
        "it joins at the end rather than retaking the place it had"
    );
}

#[test]
fn removing_a_group_keeps_the_accounts_that_were_in_it() {
    let database = TempDatabase::new("groups-remove");
    database.with_account("acc-a");
    let store = SqliteAccountGroups::new(database.pool.clone());

    let work = store.create(&name("Work")).expect("a group");
    store.place("acc-a", Some(&work)).expect("joins");
    store.remove(&work).expect("the group goes");

    let still_here: i64 = database
        .pool
        .get()
        .expect("connection")
        .query_row(
            "SELECT count(*) FROM mail_accounts WHERE id = 'acc-a'",
            [],
            |row| row.get(0),
        )
        .expect("readable");

    assert_eq!(store.list().expect("readable"), Vec::new());
    assert_eq!(still_here, 1, "the mailbox outlives the arrangement");
}

#[test]
fn renaming_a_group_that_is_not_here_is_said_rather_than_passed_over() {
    let database = TempDatabase::new("groups-rename-missing");
    let store = SqliteAccountGroups::new(database.pool.clone());

    assert_eq!(
        store.rename("no-such-group", &name("Work")),
        Err(AccountGroupError::NoSuchGroup)
    );
    assert_eq!(
        store.remove("no-such-group"),
        Err(AccountGroupError::NoSuchGroup)
    );
}

#[test]
fn a_group_and_an_account_that_are_not_here_are_told_apart() {
    let database = TempDatabase::new("groups-missing-apart");
    database.with_account("acc-a");
    let store = SqliteAccountGroups::new(database.pool.clone());

    let work = store.create(&name("Work")).expect("a group");

    // The database would refuse both, and with one error. Told apart here because the screen has
    // two different things to say: one is a group somebody just deleted in another window, the
    // other is a mailbox that is gone.
    assert_eq!(
        store.place("acc-a", Some("no-such-group")),
        Err(AccountGroupError::NoSuchGroup)
    );
    assert_eq!(
        store.place("no-such-account", Some(&work)),
        Err(AccountGroupError::NoSuchAccount)
    );
}

#[test]
fn renaming_shows_the_new_name() {
    let database = TempDatabase::new("groups-rename");
    let store = SqliteAccountGroups::new(database.pool.clone());

    let work = store.create(&name("Work")).expect("a group");
    store.rename(&work, &name("Projects")).expect("renamed");

    let groups = store.list().expect("readable");

    assert_eq!(groups[0].name.as_str(), "Projects");
}

#[test]
fn a_name_this_build_cannot_read_is_reported_rather_than_repaired() {
    let database = TempDatabase::new("groups-corrupt");
    database
        .pool
        .get()
        .expect("connection")
        .execute(
            "INSERT INTO account_groups (id, name, position) VALUES ('g1', '   ', 0)",
            [],
        )
        .expect("a row written by another version");

    let store = SqliteAccountGroups::new(database.pool.clone());

    // Quietly renaming somebody's group would be a worse answer than saying the store holds
    // something this build does not understand.
    assert_eq!(store.list(), Err(AccountGroupError::Corrupted));
}

#[test]
fn a_group_is_open_when_it_is_made_and_stays_as_it_was_left() {
    let database = TempDatabase::new("groups-collapse");
    let store = SqliteAccountGroups::new(database.pool.clone());

    let work = store.create(&name("Work")).expect("a group");
    assert!(
        !store.list().expect("readable")[0].collapsed,
        "a new group is open, so what was just made is visible"
    );

    store.set_collapsed(&work, true).expect("folds away");
    assert!(store.list().expect("readable")[0].collapsed);

    store.set_collapsed(&work, false).expect("opens again");
    assert!(!store.list().expect("readable")[0].collapsed);
}

#[test]
fn folding_a_group_that_is_not_here_is_said_rather_than_passed_over() {
    let database = TempDatabase::new("groups-collapse-missing");
    let store = SqliteAccountGroups::new(database.pool.clone());

    assert_eq!(
        store.set_collapsed("no-such-group", true),
        Err(AccountGroupError::NoSuchGroup)
    );
}

#[test]
fn the_order_dragged_into_is_the_order_a_group_answers_with() {
    // What the sidebar reads back, through the same store that wrote it: a fake agreeing with
    // itself proves the fake.
    let database = TempDatabase::new("groups-reorder");
    database
        .with_account("acc-1")
        .with_account("acc-2")
        .with_account("acc-3");
    let store = SqliteAccountGroups::new(database.pool.clone());

    let work = store
        .create(&GroupName::new("Work").expect("a name"))
        .expect("made");
    for account in ["acc-1", "acc-2", "acc-3"] {
        store.place(account, Some(&work)).expect("placed");
    }

    assert_eq!(
        store.list().expect("readable")[0].accounts,
        vec!["acc-1".to_owned(), "acc-2".to_owned(), "acc-3".to_owned()],
        "joined a group in the order they were put in it"
    );

    store
        .reorder(&["acc-3".to_owned(), "acc-1".to_owned(), "acc-2".to_owned()])
        .expect("written");

    assert_eq!(
        store.list().expect("readable")[0].accounts,
        vec!["acc-3".to_owned(), "acc-1".to_owned(), "acc-2".to_owned()]
    );
}

#[test]
fn an_order_naming_a_mailbox_that_is_gone_writes_none_of_it() {
    // One transaction, and the refusal is the point: a list half applied leaves every account
    // after the missing one at a number that means something else.
    let database = TempDatabase::new("groups-reorder-missing");
    database.with_account("acc-1").with_account("acc-2");
    let store = SqliteAccountGroups::new(database.pool.clone());
    let before = database.top();

    assert_eq!(
        store.reorder(&["acc-2".to_owned(), "gone".to_owned(), "acc-1".to_owned()]),
        Err(AccountGroupError::NoSuchAccount)
    );

    assert_eq!(
        database.top(),
        before,
        "nothing was written before the refusal"
    );
}

#[test]
fn reordering_does_not_move_an_account_between_groups() {
    // Where an account belongs is a different decision, made by a different call. An order that
    // also re-homed accounts would be two things behind one gesture.
    let database = TempDatabase::new("groups-reorder-home");
    database.with_account("acc-1").with_account("acc-2");
    let store = SqliteAccountGroups::new(database.pool.clone());

    let work = store
        .create(&GroupName::new("Work").expect("a name"))
        .expect("made");
    store.place("acc-1", Some(&work)).expect("placed");

    store
        .reorder(&["acc-2".to_owned(), "acc-1".to_owned()])
        .expect("written");

    assert_eq!(
        store.list().expect("readable")[0].accounts,
        vec!["acc-1".to_owned()],
        "the group still holds the one account that was put in it"
    );
}

fn group(id: &str) -> Placed {
    Placed::Group(id.to_owned())
}

fn account(id: &str) -> Placed {
    Placed::Account(id.to_owned())
}

#[test]
fn a_new_group_stands_last_below_the_accounts_in_none() {
    let database = TempDatabase::new("top-new-group");
    database.with_account("acc-a").with_account("acc-b");
    let store = SqliteAccountGroups::new(database.pool.clone());

    store.create(&name("Work")).expect("a group");

    assert_eq!(database.top(), vec!["acc-a", "acc-b", "Work"]);
}

#[test]
fn an_account_in_no_group_stands_above_a_group_or_below_it() {
    let database = TempDatabase::new("top-arrange");
    database.with_account("acc-a").with_account("acc-b");
    let store = SqliteAccountGroups::new(database.pool.clone());
    let work = store.create(&name("Work")).expect("a group");
    store.place("acc-b", Some(&work)).expect("joins");

    store
        .arrange(&[group(&work), account("acc-a")])
        .expect("below");
    assert_eq!(database.top(), vec!["Work", "acc-a"]);

    store
        .arrange(&[account("acc-a"), group(&work)])
        .expect("above");
    assert_eq!(database.top(), vec!["acc-a", "Work"]);
    assert_eq!(
        store.list().expect("readable")[0].accounts,
        vec!["acc-b"],
        "arranging the top moves nothing in or out of a group"
    );
}

#[test]
fn an_order_for_the_top_naming_an_account_in_a_group_writes_none_of_it() {
    let database = TempDatabase::new("top-grouped");
    database.with_account("acc-a").with_account("acc-b");
    let store = SqliteAccountGroups::new(database.pool.clone());
    let work = store.create(&name("Work")).expect("a group");
    store.place("acc-b", Some(&work)).expect("joins");
    let before = database.top();

    assert_eq!(
        store.arrange(&[group(&work), account("acc-b"), account("acc-a")]),
        Err(AccountGroupError::NoSuchAccount),
        "inside a group is not the top of the sidebar"
    );
    assert_eq!(database.top(), before);
}

#[test]
fn an_order_for_the_top_naming_a_group_that_is_gone_writes_none_of_it() {
    let database = TempDatabase::new("top-gone");
    database.with_account("acc-a").with_account("acc-b");
    let store = SqliteAccountGroups::new(database.pool.clone());
    let before = database.top();

    assert_eq!(
        store.arrange(&[account("acc-b"), group("no-such-group"), account("acc-a")]),
        Err(AccountGroupError::NoSuchGroup)
    );
    assert_eq!(database.top(), before);
}

#[test]
fn what_an_order_for_the_top_leaves_out_goes_after_it_as_it_was() {
    let database = TempDatabase::new("top-rest");
    database
        .with_account("acc-a")
        .with_account("acc-b")
        .with_account("acc-c");
    let store = SqliteAccountGroups::new(database.pool.clone());
    let work = store.create(&name("Work")).expect("a group");

    store
        .arrange(&[group(&work), account("acc-c")])
        .expect("written");

    assert_eq!(database.top(), vec!["Work", "acc-c", "acc-a", "acc-b"]);
}

#[test]
fn an_account_taken_out_of_a_group_stands_directly_below_it() {
    let database = TempDatabase::new("top-out");
    database
        .with_account("acc-a")
        .with_account("acc-b")
        .with_account("acc-x")
        .with_account("acc-y");
    let store = SqliteAccountGroups::new(database.pool.clone());
    let work = store.create(&name("Work")).expect("a group");
    let home = store.create(&name("Home")).expect("a group");
    store.place("acc-a", Some(&work)).expect("joins");
    store.place("acc-b", Some(&work)).expect("joins");
    store
        .arrange(&[
            group(&work),
            account("acc-x"),
            group(&home),
            account("acc-y"),
        ])
        .expect("written");

    store.place("acc-a", None).expect("leaves");

    assert_eq!(
        database.top(),
        vec!["Work", "acc-a", "acc-x", "Home", "acc-y"]
    );
    assert_eq!(store.list().expect("readable")[0].accounts, vec!["acc-b"]);
}

#[test]
fn taking_out_an_account_in_no_group_leaves_it_where_it_stands() {
    let database = TempDatabase::new("top-out-loose");
    database.with_account("acc-x").with_account("acc-y");
    let store = SqliteAccountGroups::new(database.pool.clone());
    let work = store.create(&name("Work")).expect("a group");
    store
        .arrange(&[account("acc-x"), group(&work), account("acc-y")])
        .expect("written");

    store.place("acc-x", None).expect("nothing to do");

    assert_eq!(database.top(), vec!["acc-x", "Work", "acc-y"]);
}

#[test]
fn removing_a_group_puts_its_accounts_where_it_stood() {
    let database = TempDatabase::new("top-remove");
    database
        .with_account("acc-a")
        .with_account("acc-b")
        .with_account("acc-x")
        .with_account("acc-y");
    let store = SqliteAccountGroups::new(database.pool.clone());
    let work = store.create(&name("Work")).expect("a group");
    store.place("acc-b", Some(&work)).expect("joins first");
    store.place("acc-a", Some(&work)).expect("joins second");
    store
        .arrange(&[account("acc-x"), group(&work), account("acc-y")])
        .expect("written");

    store.remove(&work).expect("the group goes");

    assert_eq!(database.top(), vec!["acc-x", "acc-b", "acc-a", "acc-y"]);
}

#[test]
fn removing_an_empty_group_moves_nothing_else() {
    let database = TempDatabase::new("top-remove-empty");
    database.with_account("acc-x").with_account("acc-y");
    let store = SqliteAccountGroups::new(database.pool.clone());
    let empty = store.create(&name("Empty")).expect("a group");
    store
        .arrange(&[account("acc-x"), group(&empty), account("acc-y")])
        .expect("written");

    store.remove(&empty).expect("the group goes");

    assert_eq!(database.top(), vec!["acc-x", "acc-y"]);
}

#[test]
fn removing_a_group_that_is_not_here_is_said_and_moves_nothing() {
    let database = TempDatabase::new("top-remove-gone");
    database.with_account("acc-x");
    let store = SqliteAccountGroups::new(database.pool.clone());
    let before = database.top();

    assert_eq!(
        store.remove("no-such-group"),
        Err(AccountGroupError::NoSuchGroup)
    );
    assert_eq!(database.top(), before);
}
