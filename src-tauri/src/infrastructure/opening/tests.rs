use super::*;

/// A folder of its own for one test, empty, under the system's folder for temporary files.
fn a_folder(test: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("lyonoyl-opened-{test}-{}", std::process::id()));
    if folder.exists() {
        let _ = taken_away(&folder);
    }
    std::fs::create_dir_all(&folder).expect("a folder");
    folder
}

/// Everything under a folder, made writable and taken away, once a test is done with it.
fn done_with(folder: &Path) {
    taken_away(folder).expect("the test's folder is taken away");
}

#[test]
fn every_copy_is_written_in_a_folder_of_its_own_under_the_name_given() {
    let root = a_folder("own");
    let folder = root.join(OPENED);

    let first = written_to_open(&folder, "Statement.pdf", b"%PDF-1.7 first").expect("written");
    let second = written_to_open(&folder, "Statement.pdf", b"%PDF-1.7 second").expect("written");

    assert_ne!(first.parent(), second.parent(), "two files of one name met");
    for (copy, bytes) in [
        (&first, b"%PDF-1.7 first".as_slice()),
        (&second, b"%PDF-1.7 second".as_slice()),
    ] {
        assert_eq!(
            copy.file_name().and_then(|n| n.to_str()),
            Some("Statement.pdf")
        );
        assert_eq!(copy.parent().and_then(Path::parent), Some(folder.as_path()));
        assert_eq!(std::fs::read(copy).expect("readable"), bytes);
    }

    done_with(&root);
}

#[cfg(unix)]
#[test]
fn a_copy_and_its_folder_are_read_only_and_the_folder_for_copies_is_its_owners() {
    use std::os::unix::fs::PermissionsExt;

    let root = a_folder("read-only");
    let folder = root.join(OPENED);
    let copy = written_to_open(&folder, "scan.png", b"\x89PNG\r\n\x1a\n").expect("written");

    let mode = |path: &Path| std::fs::metadata(path).expect("there").permissions().mode() & 0o777;
    assert_eq!(mode(&copy), 0o400, "the copy");
    assert_eq!(
        mode(copy.parent().expect("its folder")),
        0o500,
        "its folder"
    );
    assert_eq!(mode(&folder), 0o700, "the folder for copies");

    // A program asked to save a change there is refused, and saves it somewhere else.
    assert!(std::fs::OpenOptions::new().write(true).open(&copy).is_err());
    assert!(std::fs::write(copy.with_file_name("scan copy.png"), b"changed").is_err());

    done_with(&root);
}

#[cfg(target_os = "macos")]
#[test]
fn a_copy_carries_the_mark_of_a_download() {
    let root = a_folder("marked");
    let copy = written_to_open(&root.join(OPENED), "Statement.pdf", b"%PDF-1.7").expect("written");

    let mark = xattr::get(&copy, crate::infrastructure::dialog::QUARANTINE)
        .expect("readable")
        .expect("the mark is on the copy");
    let mark = String::from_utf8(mark).expect("text");
    assert!(mark.starts_with("0081;"), "{mark}");
    assert!(mark.ends_with(";Lyonoyl Mail;"), "{mark}");

    done_with(&root);
}

#[test]
fn a_copy_that_cannot_be_written_leaves_nothing_behind() {
    // A name no filesystem takes: a copy refused that way is a folder emptied and taken away, not
    // one the next start has to find.
    let root = a_folder("refused");
    let folder = root.join(OPENED);
    let too_long = format!("{}.pdf", "a".repeat(400));

    assert_eq!(
        written_to_open(&folder, &too_long, b"%PDF-1.7"),
        Err(FileOpeningError::NotWritten)
    );
    assert_eq!(
        std::fs::read_dir(&folder)
            .expect("the folder for copies")
            .count(),
        0,
        "half a copy was left behind"
    );

    done_with(&root);
}

#[test]
fn the_start_takes_away_every_copy_an_earlier_run_opened_and_nothing_else() {
    let root = a_folder("start");
    let folder = root.join(OPENED);
    let first = written_to_open(&folder, "Statement.pdf", b"%PDF-1.7").expect("written");
    let second = written_to_open(&folder, "photo.jpg", b"\xFF\xD8\xFF").expect("written");

    // Beside them, what this application did not make: a file and a folder under other names, and
    // a folder named as a copy's is that is a link to somewhere else.
    let stranger = folder.join("notes.txt");
    std::fs::write(&stranger, b"not a copy").expect("a stranger");
    let strangers_folder = folder.join("Kept");
    std::fs::create_dir(&strangers_folder).expect("a stranger's folder");
    let elsewhere = root.join("elsewhere");
    std::fs::create_dir(&elsewhere).expect("somewhere else");
    std::fs::write(elsewhere.join("keep.txt"), b"somebody's").expect("a file elsewhere");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&elsewhere, folder.join("0".repeat(NAME_BYTES * 2)))
        .expect("a link named as a copy's folder");

    clear_what_an_earlier_run_opened(&folder);

    assert!(
        !first.exists() && !second.exists(),
        "a copy outlived the start"
    );
    assert!(
        !first.parent().expect("its folder").exists(),
        "a copy's folder stayed"
    );
    assert!(
        stranger.exists(),
        "the start took a file that is not a copy"
    );
    assert!(
        strangers_folder.exists(),
        "the start took a folder that is not a copy's"
    );
    assert!(
        elsewhere.join("keep.txt").exists(),
        "the start reached through a link"
    );
    assert!(
        std::fs::symlink_metadata(folder.join("0".repeat(NAME_BYTES * 2))).is_err(),
        "the link named as a copy's folder stayed"
    );

    done_with(&root);
}

#[cfg(unix)]
#[test]
fn a_folder_for_copies_that_is_a_link_is_not_looked_into() {
    let root = a_folder("linked");
    let elsewhere = root.join("elsewhere");
    let named_as_a_copy = elsewhere.join("f".repeat(NAME_BYTES * 2));
    std::fs::create_dir_all(&named_as_a_copy).expect("somewhere else");
    std::fs::write(named_as_a_copy.join("keep.txt"), b"somebody's").expect("a file elsewhere");
    let folder = root.join(OPENED);
    std::os::unix::fs::symlink(&elsewhere, &folder).expect("the folder for copies, a link");

    clear_what_an_earlier_run_opened(&folder);

    assert!(
        named_as_a_copy.join("keep.txt").exists(),
        "the start reached through a link"
    );

    done_with(&root);
}

#[test]
fn a_start_with_no_folder_for_copies_has_nothing_to_take() {
    let root = a_folder("none");

    clear_what_an_earlier_run_opened(&root.join(OPENED));

    assert!(
        !root.join(OPENED).exists(),
        "the start made a folder it had nothing to put in"
    );
    done_with(&root);
}

/// A bound of bytes for a test, each copy counting for at least one byte.
#[cfg(unix)]
const SMALL: Bound = Bound {
    most: 100,
    least: 1,
};

/// The time a copy's folder says it was made, set by hand: two copies made in one tick of the
/// clock are otherwise told apart by nothing.
#[cfg(unix)]
fn made_at(copy: &Path, seconds: u64) {
    let place = copy.parent().expect("its folder");
    let at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
    std::fs::File::open(place)
        .expect("the folder opens")
        .set_modified(at)
        .expect("its time is set");
}

#[cfg(unix)]
#[test]
fn the_oldest_copies_go_first_once_the_copies_would_weigh_more_than_they_may() {
    let root = a_folder("bound");
    let folder = root.join(OPENED);

    let oldest = written_within(&folder, "oldest.pdf", &[b'1'; 30], SMALL).expect("written");
    let middle = written_within(&folder, "middle.pdf", &[b'2'; 30], SMALL).expect("written");
    let newest = written_within(&folder, "newest.pdf", &[b'3'; 30], SMALL).expect("written");
    // Made in an order the names do not give, so the folders' order on the disk says nothing.
    made_at(&middle, 2_000);
    made_at(&oldest, 1_000);
    made_at(&newest, 3_000);

    let fourth = written_within(&folder, "fourth.pdf", &[b'4'; 30], SMALL).expect("written");

    assert!(!oldest.exists(), "the oldest copy was kept past the bound");
    assert!(!oldest.parent().expect("its folder").exists());
    for kept in [&middle, &newest, &fourth] {
        assert!(kept.exists(), "{} went before the oldest", kept.display());
    }

    // One copy heavier than everything may weigh together is refused, and takes nothing away.
    assert_eq!(
        written_within(&folder, "heavy.pdf", &[b'5'; 150], SMALL),
        Err(FileOpeningError::NotWritten)
    );
    for kept in [&middle, &newest, &fourth] {
        assert!(kept.exists(), "a refused copy took another one away");
    }

    done_with(&root);
}

#[test]
fn copies_of_a_few_bytes_are_bounded_by_their_number() {
    // Each counts for at least the least a copy counts for, so a hundred copies of one byte each
    // are not a hundred bytes.
    let root = a_folder("tiny");
    let folder = root.join(OPENED);
    let bound = Bound {
        most: 300,
        least: 100,
    };

    for at in 0..12 {
        written_within(&folder, &format!("{at}.pdf"), b"%", bound).expect("written");
        assert!(
            ours(&folder).count() <= 3,
            "{} copies held after {}",
            ours(&folder).count(),
            at + 1
        );
    }

    done_with(&root);
}

#[test]
fn the_copies_held_at_once_are_bounded_by_what_eight_messages_weigh_and_by_their_number() {
    assert_eq!(
        MOST_HELD,
        8 * crate::infrastructure::html::LARGEST_MESSAGE as u64
    );
    assert_eq!(MOST_HELD / LEAST_COUNTED, 4_096);
}

#[cfg(unix)]
#[test]
fn a_folder_above_the_copies_that_is_a_link_is_not_written_into() {
    let root = a_folder("above-linked");
    let elsewhere = root.join("elsewhere");
    std::fs::create_dir(&elsewhere).expect("somewhere else");
    let above = root.join("Lyonoyl Mail");
    std::os::unix::fs::symlink(&elsewhere, &above).expect("the folder above, a link");

    assert_eq!(
        written_to_open(&above.join(OPENED), "Statement.pdf", b"%PDF-1.7"),
        Err(FileOpeningError::NotWritten)
    );
    assert_eq!(
        std::fs::read_dir(&elsewhere).expect("readable").count(),
        0,
        "a copy was written through the link above"
    );

    done_with(&root);
}

#[cfg(unix)]
#[test]
fn a_folder_above_the_copies_that_others_could_write_in_is_made_this_persons_alone() {
    use std::os::unix::fs::PermissionsExt;

    let root = a_folder("above-open");
    let above = root.join("Lyonoyl Mail");
    std::fs::create_dir(&above).expect("the folder above");
    std::fs::set_permissions(&above, std::fs::Permissions::from_mode(0o777)).expect("open to all");

    written_to_open(&above.join(OPENED), "Statement.pdf", b"%PDF-1.7").expect("written");

    let mode = std::fs::metadata(&above)
        .expect("there")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);

    done_with(&root);
}

#[cfg(unix)]
#[test]
fn a_folder_for_copies_that_is_a_link_is_not_written_into() {
    // The sweep never looks into a link, so a copy written through one would stay wherever it
    // led for good.
    let root = a_folder("written-through");
    let elsewhere = root.join("elsewhere");
    std::fs::create_dir(&elsewhere).expect("somewhere else");
    let folder = root.join(OPENED);
    std::os::unix::fs::symlink(&elsewhere, &folder).expect("the folder for copies, a link");

    assert_eq!(
        written_to_open(&folder, "Statement.pdf", b"%PDF-1.7"),
        Err(FileOpeningError::NotWritten)
    );
    assert_eq!(
        std::fs::read_dir(&elsewhere).expect("readable").count(),
        0,
        "a copy was written through the link"
    );

    done_with(&root);
}

#[test]
fn a_file_where_the_folder_for_copies_should_be_is_not_written_into() {
    let root = a_folder("a-file");
    let folder = root.join(OPENED);
    std::fs::write(&folder, b"not a folder").expect("a file under the folder's name");

    assert_eq!(
        written_to_open(&folder, "Statement.pdf", b"%PDF-1.7"),
        Err(FileOpeningError::NotWritten)
    );
    assert_eq!(
        std::fs::read(&folder).expect("still there"),
        b"not a folder"
    );

    done_with(&root);
}
