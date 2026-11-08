//! Whether a database another build wrote opens in this one.
//!
//! A person's files outlive the build that wrote them: an update, a copy restored onto another
//! machine, a build of this tree made with another library behind SQLCipher. Each directory under
//! `tests/fixtures/databases/` holds a database a build wrote through `open_profile`, named by
//! what the library encrypting it answered, and every build opens every one of them.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use rusqlite::types::ValueRef;

use super::*;

/// Where the databases other builds wrote are kept.
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/fixtures/databases");

/// The file a fixture is, beside its write-ahead log.
const FILE: &str = "written.sqlite3";

/// The libraries a build of this tree encrypts with, each of which has left a database here.
const EXPECTED: [&str; 2] = ["commoncrypto", "openssl"];

/// The key the fixtures are written under.
///
/// Not the test key: one byte repeated reads the same in any order, so a library that took the
/// bytes in another order, or stopped at a zero, would open those files and refuse a real key. Every
/// byte here differs from the others, and one of them is zero, as about one key in eight made at
/// random has.
const FIXTURE_KEY: [u8; 32] = [
    0x5a, 0x01, 0xc3, 0x7e, 0x22, 0x9b, 0x40, 0xe8, 0x16, 0xad, 0x00, 0x6f, 0xd4, 0x38, 0x81, 0xf5,
    0x0c, 0x93, 0x4e, 0xb7, 0x2a, 0xcd, 0x65, 0x19, 0xfe, 0x72, 0x07, 0xbe, 0x54, 0x8a, 0xe1, 0x3d,
];

/// The one table a fixture holds, as the generator makes it and the reader expects to find it.
const TABLE: &str = "CREATE TABLE kept (name TEXT NOT NULL, address TEXT NOT NULL)";

/// What a fixture holds, and all it holds: the first row checkpointed into the file, the second
/// left in the log beside it. The log holds the newer copy of the page both rows sit on, so the
/// file is opened on its own as well, for its own copy of that page to be read.
const KEPT: [(&str, &str); 2] = [
    ("Zeta Quorin", "zeta@example.com"),
    ("Kappa Vandel", "kappa@example.net"),
];

/// What the generator leaves: in the file two pages, the schema and the table; in the log a
/// header of 32 bytes and one frame, 24 bytes and a page.
const FILE_BYTES: u64 = 2 * 4096;
const LOG_BYTES: u64 = 32 + 24 + 4096;

/// What SQLCipher answers for the library it encrypts with. macOS is the system the table in
/// `src-tauri/Cargo.toml` leaves out, where libsqlite3-sys compiles SQLCipher against CommonCrypto;
/// everywhere else SQLCipher answers openssl.
#[cfg(target_os = "macos")]
const THIS_SYSTEMS_LIBRARY: &str = "commoncrypto";
#[cfg(not(target_os = "macos"))]
const THIS_SYSTEMS_LIBRARY: &str = "openssl";

/// A directory of this test's own, gone with it.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lyonoyl-encryption-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

fn beside(dir: &Path, suffix: &str) -> PathBuf {
    dir.join(format!("{FILE}{suffix}"))
}

/// What the system or an editor leaves in a directory it shows, which `.gitignore` keeps out of the
/// tree in the same words, and which is no fixture's.
fn left_by_the_system(name: &str) -> bool {
    matches!(
        name,
        ".DS_Store" | "Thumbs.db" | "desktop.ini" | "Desktop.ini" | ".directory"
    ) || name.starts_with("._")
        || name.ends_with('~')
}

/// Everything a directory holds, by name, but what the system left there.
fn listed(dir: &Path) -> BTreeSet<String> {
    std::fs::read_dir(dir)
        .expect("the directory is there")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| !left_by_the_system(name))
        .collect()
}

/// A whole answer, each value as text, so an extra row or column is as visible as a wrong one.
fn answer(connection: &rusqlite::Connection, sql: &str) -> Vec<Vec<String>> {
    let mut statement = connection.prepare(sql).expect("the question is asked");
    let width = statement.column_count();
    statement
        .query_map([], |row| {
            (0..width)
                .map(|at| {
                    Ok(match row.get_ref(at)? {
                        ValueRef::Null => "NULL".to_owned(),
                        ValueRef::Integer(number) => number.to_string(),
                        ValueRef::Real(number) => number.to_string(),
                        ValueRef::Text(text) => String::from_utf8_lossy(text).into_owned(),
                        ValueRef::Blob(bytes) => format!("{} bytes", bytes.len()),
                    })
                })
                .collect()
        })
        .and_then(|rows| rows.collect())
        .expect("the answer is read")
}

fn kept(rows: &[(&str, &str)]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|&(name, address)| vec![name.to_owned(), address.to_owned()])
        .collect()
}

#[test]
fn the_databases_are_encrypted_by_the_library_this_system_is_built_with() {
    let dir = scratch("library");
    let pool =
        open_index(dir.join("index.sqlite3"), &DatabaseKey::for_a_test()).expect("a database");
    let library: String = pool
        .get()
        .expect("a connection")
        .query_row("PRAGMA cipher_provider", [], |row| row.get(0))
        .expect("the library answers");

    assert_eq!(
        library, THIS_SYSTEMS_LIBRARY,
        "SQLCipher takes its ciphers from {library} where this build expects {THIS_SYSTEMS_LIBRARY}. On \
         macOS a build is sent elsewhere by OPENSSL_DIR, or OPENSSL_LIB_DIR with OPENSSL_INCLUDE_DIR, \
         plain or prefixed with the target, or by LIBSQLITE3_SYS_USE_PKG_CONFIG, set in the \
         environment or in a Cargo `[env]` table: remove them, then run `cargo clean -p \
         libsqlite3-sys` and `cargo clean -p libsqlite3-sys --release`"

    );

    drop(pool);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_database_another_build_wrote_opens_in_this_one() {
    let expected: BTreeSet<String> = EXPECTED.iter().map(|&name| name.to_owned()).collect();
    assert_eq!(
        listed(Path::new(FIXTURES)),
        expected,
        "the fixtures hold something nothing expects, or lack a library"
    );
    let pair: BTreeSet<String> = [FILE.to_owned(), format!("{FILE}-wal")].into();
    let key = DatabaseKey::from(&FIXTURE_KEY);

    for library in EXPECTED {
        let kept_at = Path::new(FIXTURES).join(library);
        assert_eq!(listed(&kept_at), pair, "{library}");
        let size = |suffix| {
            std::fs::metadata(beside(&kept_at, suffix))
                .expect("readable")
                .len()
        };
        assert_eq!(
            size(""),
            FILE_BYTES,
            "{library}: the file the generator writes"
        );
        assert_eq!(
            size("-wal"),
            LOG_BYTES,
            "{library}: the log the generator leaves"
        );

        // The file and its log together, as a person's files are. A copy is opened, never the
        // fixture: opening folds the log into the file.
        let dir = scratch(library);
        for suffix in ["", "-wal"] {
            std::fs::copy(beside(&kept_at, suffix), beside(&dir, suffix)).expect("copied");
        }
        let pool = open_profile(beside(&dir, ""), &key)
            .unwrap_or_else(|error| panic!("{library}: {error}"));
        let connection = pool.get().expect("a connection");

        assert_eq!(
            answer(&connection, "PRAGMA cipher_integrity_check"),
            Vec::<Vec<String>>::new(),
            "{library}"
        );
        assert_eq!(
            answer(&connection, "PRAGMA integrity_check"),
            [["ok"]],
            "{library}"
        );
        // Every table, column, row, and page it holds, each against what the generator writes.
        assert_eq!(
            answer(
                &connection,
                "SELECT type, name, tbl_name, sql FROM sqlite_schema"
            ),
            [["table", "kept", "kept", TABLE]],
            "{library}"
        );
        assert_eq!(
            answer(
                &connection,
                "SELECT name, pageno, pagetype FROM dbstat ORDER BY pageno"
            ),
            [["sqlite_schema", "1", "leaf"], ["kept", "2", "leaf"]],
            "{library}"
        );
        assert_eq!(
            answer(&connection, "PRAGMA freelist_count"),
            [["0"]],
            "{library}"
        );
        assert_eq!(
            answer(&connection, "SELECT * FROM kept ORDER BY rowid"),
            kept(&KEPT),
            "{library}: the row only the log held is the last one"
        );
        drop(connection);
        drop(pool);
        std::fs::remove_dir_all(&dir).ok();

        // The file alone: its own copy of the page the log holds a newer one of, read under this
        // library, holding the first row and nothing after it.
        let alone = scratch(&format!("{library}-alone"));
        std::fs::copy(beside(&kept_at, ""), beside(&alone, "")).expect("copied");
        let pool = open_profile(beside(&alone, ""), &key)
            .unwrap_or_else(|error| panic!("{library}, the file alone: {error}"));
        assert_eq!(
            answer(
                &pool.get().expect("a connection"),
                "SELECT * FROM kept ORDER BY rowid"
            ),
            kept(&KEPT[..1]),
            "{library}: the file's own copy of the table"
        );
        drop(pool);
        std::fs::remove_dir_all(&alone).ok();
    }
}

/// Writes the database this build leaves for the others.
///
/// Run by hand, once for each library a build can encrypt with, and never by the preflight:
/// `cargo test -p lyonoyl --lib -- --ignored writes_the_database_this_build_leaves_for_the_others`.
/// The directory is named by what the library answers, and one already there is refused, so no
/// build writes over a fixture another one left.
#[test]
#[ignore = "writes a fixture into the tree; run by hand for a library no fixture covers"]
fn writes_the_database_this_build_leaves_for_the_others() {
    let dir = scratch("writer");
    let pool =
        open_profile(beside(&dir, ""), &DatabaseKey::from(&FIXTURE_KEY)).expect("a database");
    let connection = pool.get().expect("a connection");

    let library: String = connection
        .query_row("PRAGMA cipher_provider", [], |row| row.get(0))
        .expect("the library answers");
    let destination = Path::new(FIXTURES).join(&library);
    assert!(
        !destination.exists(),
        "{} is already there, written by another build",
        destination.display()
    );

    let [(first_name, first_address), (second_name, second_address)] = KEPT;
    connection
        .execute_batch(&format!("{TABLE};"))
        .expect("a table");
    connection
        .execute(
            "INSERT INTO kept VALUES (?1, ?2)",
            [first_name, first_address],
        )
        .expect("the first row");
    let (busy, logged, moved): (i64, i64, i64) = connection
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("a checkpoint");
    assert!(
        busy == 0 && logged == moved,
        "the first row was not folded into the file: busy {busy}, {logged} logged, {moved} moved"
    );
    connection
        .execute(
            "INSERT INTO kept VALUES (?1, ?2)",
            [second_name, second_address],
        )
        .expect("the second row");

    // Copied while the pool is open: the last connection to close folds the log into the file,
    // and the second row would go with it.
    let size = |suffix| {
        std::fs::metadata(beside(&dir, suffix))
            .expect("written")
            .len()
    };
    assert_eq!(
        size(""),
        FILE_BYTES,
        "the file is not the two pages it should be"
    );
    assert_eq!(
        size("-wal"),
        LOG_BYTES,
        "the log is not the one frame it should be"
    );

    std::fs::create_dir_all(&destination).expect("the fixture's directory");
    for suffix in ["", "-wal"] {
        std::fs::copy(beside(&dir, suffix), beside(&destination, suffix)).expect("copied");
    }

    drop(connection);
    drop(pool);
    std::fs::remove_dir_all(&dir).ok();
}
