use super::*;

/// A data folder of its own, taken away afterwards.
struct Data(PathBuf);

impl Data {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "lyonoyl-folder-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&path).expect("a folder");
        Self(path)
    }

    /// A file at `relative`, holding its own name, so a file that went anywhere else is told apart.
    fn file(&self, relative: &str) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("its folder");
        std::fs::write(&path, relative.as_bytes()).expect("written");
    }

    /// What the file at `relative` holds, or nothing when there is none.
    fn holds(&self, relative: &str) -> Option<String> {
        std::fs::read_to_string(self.0.join(relative)).ok()
    }

    /// The set as a start finds it at the top: every file of it, an account's mail among them.
    fn the_whole_set_at_the_top(&self) {
        for name in [
            SECRETS_FILE,
            "profile.sqlite3",
            "profile.sqlite3-wal",
            "profile.sqlite3-shm",
            "profile.sqlite3-journal",
            "accounts/1.sqlite3",
            "accounts/1.sqlite3-wal",
        ] {
            self.file(name);
        }
    }
}

impl Drop for Data {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Every file of the whole set, by where it is kept relative to the data folder.
const EVERY_FILE: [&str; 7] = [
    SECRETS_FILE,
    "profile.sqlite3",
    "profile.sqlite3-wal",
    "profile.sqlite3-shm",
    "profile.sqlite3-journal",
    "accounts/1.sqlite3",
    "accounts/1.sqlite3-wal",
];

#[test]
fn the_whole_set_moves_into_its_own_folder_and_nothing_else_does() {
    let data = Data::new("whole");
    data.the_whole_set_at_the_top();
    data.file("logs/run.log");
    data.file("updates/notes.json");
    data.file("detail");

    let gathered = gather(&data.0).expect("gathered");

    assert_eq!(gathered, Gathered { moved: 6, left: 0 });
    for name in EVERY_FILE {
        assert_eq!(
            data.holds(&format!("databases/{name}")).as_deref(),
            Some(name),
            "{name} did not arrive whole"
        );
        assert_eq!(data.holds(name), None, "{name} is still at the top");
    }
    for name in ["logs/run.log", "updates/notes.json", "detail"] {
        assert_eq!(data.holds(name).as_deref(), Some(name), "{name} was moved");
    }
    assert_eq!(where_they_are(&data.0), data.0.join("databases"));
}

#[test]
fn what_there_is_of_the_set_moves_and_nothing_is_made_up() {
    // A profile that has never held a key in a file, and whose log was folded in: three of six.
    let data = Data::new("part");
    data.file("profile.sqlite3");
    data.file("profile.sqlite3-wal");
    data.file("accounts/3.sqlite3");

    let gathered = gather(&data.0).expect("gathered");

    assert_eq!(gathered, Gathered { moved: 3, left: 0 });
    let mut arrived: Vec<String> = std::fs::read_dir(data.0.join("databases"))
        .expect("the folder")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    arrived.sort();
    assert_eq!(
        arrived,
        ["accounts", "profile.sqlite3", "profile.sqlite3-wal"]
    );
}

#[test]
fn a_data_folder_with_nothing_of_the_set_is_left_as_it_is() {
    // A first start: the folder is made by whatever opens the first database, and not by this.
    let data = Data::new("nothing");
    data.file("logs/run.log");

    let gathered = gather(&data.0).expect("nothing to gather");

    assert_eq!(gathered, Gathered::default());
    assert!(
        !data.0.join("databases").exists(),
        "a folder was made for nothing"
    );
    assert_eq!(where_they_are(&data.0), data.0.join("databases"));
}

#[test]
fn a_second_start_finds_nothing_to_move() {
    let data = Data::new("again");
    data.the_whole_set_at_the_top();
    gather(&data.0).expect("gathered");

    let again = gather(&data.0).expect("nothing left to gather");

    assert_eq!(again, Gathered::default());
    for name in EVERY_FILE {
        assert_eq!(
            data.holds(&format!("databases/{name}")).as_deref(),
            Some(name)
        );
    }
}

#[test]
fn a_profile_in_the_folder_is_the_one_read_and_a_set_at_the_top_is_left_alone() {
    // A set written at the top beside one in the folder: the one in the folder is not overwritten,
    // and the one at the top is not taken apart.
    let data = Data::new("both");
    data.file("databases/profile.sqlite3");
    data.file("databases/profile.sqlite3-wal");
    std::fs::write(data.0.join("profile.sqlite3"), b"the one at the top").expect("written");
    data.file("profile.sqlite3-wal");
    data.file("accounts/1.sqlite3");

    let gathered = gather(&data.0).expect("left where they are");

    assert_eq!(gathered, Gathered { moved: 0, left: 3 });
    assert_eq!(
        data.holds("databases/profile.sqlite3").as_deref(),
        Some("databases/profile.sqlite3"),
        "the profile in the folder was replaced"
    );
    assert_eq!(
        data.holds("profile.sqlite3").as_deref(),
        Some("the one at the top"),
        "the profile at the top was moved or changed"
    );
    assert_eq!(
        data.holds("profile.sqlite3-wal").as_deref(),
        Some("profile.sqlite3-wal")
    );
    assert_eq!(
        data.holds("accounts/1.sqlite3").as_deref(),
        Some("accounts/1.sqlite3")
    );
    assert!(!data.0.join("databases/accounts").exists());
    assert_eq!(where_they_are(&data.0), data.0.join("databases"));
}

#[test]
fn a_move_stopped_part_of_the_way_puts_back_what_it_took() {
    // The fourth rename refused: the three before it go back, and this run reads the set at the top.
    let data = Data::new("stopped");
    data.the_whole_set_at_the_top();
    let mut asked = 0;

    let stopped = gather_with(&data.0, |from, to| {
        asked += 1;
        match asked {
            4 => Err(std::io::Error::other("held by somebody else")),
            _ => std::fs::rename(from, to),
        }
    })
    .expect_err("stopped");

    assert!(stopped.put_back, "{stopped:?}");
    assert_eq!(stopped.path, data.0.join("profile.sqlite3-shm"));
    for name in EVERY_FILE {
        assert_eq!(
            data.holds(name).as_deref(),
            Some(name),
            "{name} did not go back"
        );
        assert_eq!(data.holds(&format!("databases/{name}")), None);
    }
    assert_eq!(where_they_are(&data.0), data.0.clone());
}

#[test]
fn a_move_that_could_not_put_everything_back_says_so_and_the_next_start_finishes_it() {
    // The log refused on its way back too: it is in the folder and the profile is at the top, which
    // no run may open, and which the next start makes whole.
    let data = Data::new("split");
    data.the_whole_set_at_the_top();
    let mut asked = 0;

    let stopped = gather_with(&data.0, |from, to| {
        asked += 1;
        match asked {
            // secrets, accounts, journal, shm and the log go; the profile is refused; on the way
            // back the log is refused and the rest go.
            6 | 7 => Err(std::io::Error::other("held by somebody else")),
            _ => std::fs::rename(from, to),
        }
    })
    .expect_err("stopped");

    assert!(!stopped.put_back, "{stopped:?}");
    assert_eq!(stopped.path, data.0.join("profile.sqlite3"));
    assert_eq!(
        data.holds("databases/profile.sqlite3-wal").as_deref(),
        Some("profile.sqlite3-wal")
    );
    assert_eq!(
        data.holds("profile.sqlite3").as_deref(),
        Some("profile.sqlite3")
    );

    let finished = gather(&data.0).expect("the rest of it");

    assert_eq!(finished, Gathered { moved: 5, left: 0 });
    for name in EVERY_FILE {
        assert_eq!(
            data.holds(&format!("databases/{name}")).as_deref(),
            Some(name),
            "{name} is not with the rest"
        );
    }
}

#[test]
fn what_is_in_the_folder_already_is_not_replaced_by_what_joins_it() {
    // Part of a set without its profile, as a move stopped part of the way leaves it, beside a
    // file of the same name at the top: the one in the folder stays as it is.
    let data = Data::new("joining");
    data.file("databases/profile.sqlite3-wal");
    data.file("profile.sqlite3");
    std::fs::write(data.0.join("profile.sqlite3-wal"), b"a second log").expect("written");

    let gathered = gather(&data.0).expect("gathered");

    assert_eq!(gathered, Gathered { moved: 1, left: 1 });
    assert_eq!(
        data.holds("databases/profile.sqlite3-wal").as_deref(),
        Some("databases/profile.sqlite3-wal")
    );
    assert_eq!(
        data.holds("profile.sqlite3-wal").as_deref(),
        Some("a second log")
    );
    assert_eq!(
        data.holds("databases/profile.sqlite3").as_deref(),
        Some("profile.sqlite3")
    );
}

#[test]
fn a_folder_that_cannot_be_made_moves_nothing() {
    // A file where the folder goes.
    let data = Data::new("blocked");
    data.file("profile.sqlite3");
    std::fs::write(data.0.join("databases"), b"not a folder").expect("written");

    let stopped = gather(&data.0).expect_err("nowhere to move to");

    assert!(stopped.put_back);
    assert_eq!(
        data.holds("profile.sqlite3").as_deref(),
        Some("profile.sqlite3")
    );
    assert_eq!(where_they_are(&data.0), data.0.clone());
}

#[cfg(unix)]
#[test]
fn the_folder_is_reachable_by_this_person_alone() {
    use std::os::unix::fs::PermissionsExt;

    let data = Data::new("mode");
    data.file("profile.sqlite3");

    gather(&data.0).expect("gathered");

    let mode = std::fs::metadata(data.0.join("databases"))
        .expect("the folder")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o700);
}

#[test]
fn a_rename_refused_for_a_moment_is_asked_again() {
    let mut asked = 0;

    let done = again_and_again(|| {
        asked += 1;
        match asked {
            1 | 2 => Err(std::io::Error::other("held for a moment")),
            _ => Ok(()),
        }
    });

    assert!(done.is_ok(), "{done:?}");
    assert_eq!(asked, 3);
}

#[test]
fn a_rename_refused_every_time_is_given_up_after_its_tries() {
    let mut asked = 0;

    let done = again_and_again(|| {
        asked += 1;
        Err(std::io::Error::other("held for good"))
    });

    assert!(done.is_err());
    assert_eq!(asked, TRIES);
}

#[test]
fn what_waiting_cannot_change_is_not_asked_again() {
    for kind in [
        std::io::ErrorKind::NotFound,
        std::io::ErrorKind::AlreadyExists,
    ] {
        let mut asked = 0;
        let began = std::time::Instant::now();

        let done = again_and_again(|| {
            asked += 1;
            Err(std::io::Error::from(kind))
        });

        assert_eq!(done.expect_err("refused").kind(), kind);
        assert_eq!(asked, 1, "{kind:?} was asked again");
        assert!(began.elapsed() < BETWEEN_TRIES, "{kind:?} was waited for");
    }
}

#[test]
fn a_rename_that_goes_through_moves_the_file() {
    let data = Data::new("patient");
    data.file("from");

    patiently(&data.0.join("from"), &data.0.join("to")).expect("moved");

    assert_eq!(data.holds("to").as_deref(), Some("from"));
    assert_eq!(data.holds("from"), None);
}
