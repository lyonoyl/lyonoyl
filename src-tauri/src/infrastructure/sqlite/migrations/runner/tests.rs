use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use rusqlite::{Connection, Transaction};

use super::{Migration, MigrationError, Migrations};

fn database() -> Connection {
    Connection::open_in_memory().expect("a database in memory opens")
}

fn version(connection: &Connection) -> usize {
    Migrations::version_of(connection).expect("the version reads")
}

fn tables(connection: &Connection) -> Vec<String> {
    let mut statement = connection
        .prepare("SELECT name FROM sqlite_schema WHERE type = 'table' ORDER BY name")
        .expect("the schema reads");
    statement
        .query_map([], |row| row.get(0))
        .expect("the schema reads")
        .collect::<Result<_, _>>()
        .expect("every name reads")
}

/// Three steps that each go back, the last indexing the table the second makes.
fn three_steps() -> Migrations {
    Migrations::new(vec![
        Migration::up("CREATE TABLE first (a INTEGER);").down("DROP TABLE first;"),
        Migration::up("CREATE TABLE second (b INTEGER);").down("DROP TABLE second;"),
        Migration::up("CREATE INDEX second_b ON second (b);").down("DROP INDEX second_b;"),
    ])
}

fn a_row_into_first(transaction: &Transaction) -> rusqlite::Result<()> {
    transaction.execute_batch("INSERT INTO first VALUES (7);")
}

fn refusing(_transaction: &Transaction) -> rusqlite::Result<()> {
    Err(rusqlite::Error::InvalidParameterName("refused".into()))
}

fn reading_second(transaction: &Transaction) -> rusqlite::Result<()> {
    transaction.query_row("SELECT count(*) FROM second", [], |_| Ok(()))
}

#[test]
fn a_new_database_takes_every_step_in_order_and_stands_at_the_last() {
    let mut connection = database();

    three_steps()
        .to_latest(&mut connection)
        .expect("every step runs");

    assert_eq!(version(&connection), 3);
    assert_eq!(tables(&connection), ["first", "second"]);
}

#[test]
fn a_database_part_of_the_way_takes_only_the_steps_after_its_version() {
    let mut connection = database();
    three_steps()
        .to_version(&mut connection, 1)
        .expect("the first step runs");

    // The first step again would fail: its table is already there.
    three_steps()
        .to_latest(&mut connection)
        .expect("the rest run");

    assert_eq!(version(&connection), 3);
}

#[test]
fn a_database_at_the_last_version_is_not_written_to() {
    let mut connection = database();
    let commits = Arc::new(AtomicUsize::new(0));
    let counting = Arc::clone(&commits);
    connection
        .commit_hook(Some(move || {
            counting.fetch_add(1, Ordering::SeqCst);
            false
        }))
        .expect("commits are counted");
    let steps = three_steps();
    steps.to_latest(&mut connection).expect("every step runs");
    assert_eq!(commits.load(Ordering::SeqCst), 1, "the run is one commit");

    // Run again, its first step would fail on a table already there, and a run that wrote only the
    // version would still be a commit.
    steps
        .to_latest(&mut connection)
        .expect("nothing is left to run");

    assert_eq!(commits.load(Ordering::SeqCst), 1);
    assert_eq!(version(&connection), 3);
}

#[test]
fn a_step_that_fails_takes_the_whole_run_back() {
    let mut connection = database();
    let broken = Migrations::new(vec![
        Migration::up("CREATE TABLE first (a INTEGER);"),
        Migration::up("CREATE TABLE second (b INTEGER);"),
        Migration::up("THIS IS NOT SQL;"),
    ]);

    let refused = broken.to_latest(&mut connection);

    assert!(matches!(refused, Err(MigrationError::Step { step: 3, .. })));
    assert_eq!(version(&connection), 0);
    assert!(tables(&connection).is_empty());
}

#[test]
fn a_hook_runs_after_its_steps_sql_in_the_same_run() {
    let mut connection = database();
    let hooked = Migrations::new(vec![Migration::up_with_hook(
        "CREATE TABLE first (a INTEGER);",
        a_row_into_first,
    )]);

    hooked
        .to_latest(&mut connection)
        .expect("the step and its hook run");

    let held: i64 = connection
        .query_row("SELECT a FROM first", [], |row| row.get(0))
        .expect("the hook's row is there");
    assert_eq!(held, 7);
}

#[test]
fn a_hook_that_fails_takes_the_whole_run_back() {
    let mut connection = database();
    let hooked = Migrations::new(vec![
        Migration::up("CREATE TABLE first (a INTEGER);"),
        Migration::up_with_hook("CREATE TABLE second (b INTEGER);", refusing),
    ]);

    let refused = hooked.to_latest(&mut connection);

    assert!(matches!(refused, Err(MigrationError::Step { step: 2, .. })));
    assert_eq!(version(&connection), 0);
    assert!(tables(&connection).is_empty());
}

#[test]
fn going_back_takes_the_newest_step_back_first() {
    let mut connection = database();
    let steps = three_steps();
    steps.to_latest(&mut connection).expect("every step runs");

    // The index goes before its table, or dropping the table takes the index with it and the
    // index's own way back finds nothing to drop.
    steps
        .to_version(&mut connection, 1)
        .expect("two steps go back");

    assert_eq!(version(&connection), 1);
    assert_eq!(tables(&connection), ["first"]);
}

#[test]
fn going_back_to_nothing_takes_every_step_back() {
    let mut connection = database();
    let steps = three_steps();
    steps.to_latest(&mut connection).expect("every step runs");

    steps
        .to_version(&mut connection, 0)
        .expect("every step goes back");

    assert_eq!(version(&connection), 0);
    assert!(tables(&connection).is_empty());
}

#[test]
fn a_hook_on_the_way_back_runs_before_its_sql() {
    let mut connection = database();
    let steps = Migrations::new(vec![Migration::up("CREATE TABLE second (b INTEGER);")
        .down_with_hook("DROP TABLE second;", reading_second)]);
    steps.to_latest(&mut connection).expect("the step runs");

    // The hook reads the table its step's SQL drops, so it only succeeds before that SQL.
    steps
        .to_version(&mut connection, 0)
        .expect("the step goes back");

    assert!(tables(&connection).is_empty());
}

#[test]
fn going_back_past_a_step_with_no_way_back_is_refused_and_undoes_nothing() {
    let mut connection = database();
    let steps = Migrations::new(vec![
        Migration::up("CREATE TABLE first (a INTEGER);").down("DROP TABLE first;"),
        Migration::up("CREATE TABLE second (b INTEGER);"),
        Migration::up("CREATE TABLE third (c INTEGER);").down("DROP TABLE third;"),
    ]);
    steps.to_latest(&mut connection).expect("every step runs");

    let refused = steps.to_version(&mut connection, 1);

    assert!(matches!(
        refused,
        Err(MigrationError::NoWayBack { step: 2 })
    ));
    assert_eq!(version(&connection), 3);
    assert_eq!(tables(&connection), ["first", "second", "third"]);
}

#[test]
fn a_database_from_a_newer_build_is_refused_and_left_as_it_is() {
    let mut connection = database();
    connection
        .execute_batch("PRAGMA user_version = 5;")
        .expect("a newer build's version is written");

    let refused = three_steps().to_latest(&mut connection);

    assert!(matches!(
        refused,
        Err(MigrationError::Ahead { found: 5, known: 3 })
    ));
    assert_eq!(version(&connection), 5);
    assert!(tables(&connection).is_empty());
}

#[test]
fn a_version_past_the_list_is_refused_and_the_last_is_reached() {
    let mut connection = database();

    let refused = three_steps().to_version(&mut connection, 4);

    assert!(matches!(
        refused,
        Err(MigrationError::BeyondTheList { asked: 4, known: 3 })
    ));
    assert_eq!(version(&connection), 0);
    three_steps()
        .to_version(&mut connection, 3)
        .expect("the last version is one the list reaches");
    assert_eq!(version(&connection), 3);
}

#[test]
fn a_version_below_zero_is_refused_and_left_as_it_is() {
    let mut connection = database();
    connection
        .execute_batch("PRAGMA user_version = -1;")
        .expect("a version below zero is written");

    let refused = three_steps().to_latest(&mut connection);

    assert!(matches!(
        refused,
        Err(MigrationError::UnreadableVersion(-1))
    ));
    assert!(tables(&connection).is_empty());
}

#[test]
fn validating_takes_a_database_of_its_own_through_every_step() {
    three_steps().validate().expect("every step runs");

    let broken = Migrations::new(vec![Migration::up("THIS IS NOT SQL;")]);
    assert!(matches!(
        broken.validate(),
        Err(MigrationError::Step { step: 1, .. })
    ));
}
