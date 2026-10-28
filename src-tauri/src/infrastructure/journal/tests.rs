use super::*;
use crate::ports::journal::Word;

/// A folder that exists while a test runs.
struct ADirectory(PathBuf);

impl ADirectory {
    fn named(what: &str) -> Self {
        let at = std::env::temp_dir().join(format!(
            "lyonoyl-journal-{what}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).expect("a directory to write into");
        Self(at)
    }
}

impl Drop for ADirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Everything the run wrote, in order.
fn everything_written(inside: &std::path::Path) -> String {
    let directory = inside.join(files::DIRECTORY);
    let mut names: Vec<PathBuf> = std::fs::read_dir(&directory)
        .expect("the folder is there")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    names.sort();

    names
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .collect()
}

#[test]
fn a_record_reaches_the_file_it_was_promised() {
    let at = ADirectory::named("writes");
    let journal = TheJournal::in_directory(&at.0, Note::Worth).expect("a journal");

    journal.write_down(
        Note::Worth,
        "body.rendered",
        &[
            ("kind", Value::Word(Word::Html)),
            ("pictures", Value::Count(11)),
            ("held", Value::Bytes(9_000)),
            ("took", Value::Millis(42)),
            ("truncated", Value::Yes(false)),
        ],
    );
    journal.flush_within(std::time::Duration::from_secs(5));

    let written = everything_written(&at.0);

    assert!(written.contains("journal.began"), "{written}");
    assert!(written.contains("body.rendered"), "{written}");
    assert!(written.contains("kind=html"), "{written}");
    assert!(written.contains("pictures=11"), "{written}");
    assert!(written.contains("took=42ms"), "{written}");
    assert!(written.contains("truncated=no"), "{written}");
    // Rounded, so a length is an order of magnitude rather than a fingerprint of which message.
    assert!(written.contains("held=~16384"), "{written}");
    assert!(
        !written.contains("held=9000"),
        "an exact size was written: {written}"
    );
}

#[test]
fn a_record_below_what_was_asked_for_costs_a_comparison_and_no_line() {
    let at = ADirectory::named("level");
    let journal = TheJournal::in_directory(&at.0, Note::Worth).expect("a journal");

    assert!(!journal.keeps(Note::Detail));
    journal.write_down(Note::Detail, "picture.asked", &[("n", Value::Count(1))]);
    journal.write_down(Note::Trouble, "picture.lost", &[("n", Value::Count(1))]);
    journal.flush_within(std::time::Duration::from_secs(5));

    let written = everything_written(&at.0);

    assert!(!written.contains("picture.asked"), "{written}");
    assert!(written.contains("picture.lost"), "{written}");
}

#[test]
fn a_line_is_never_torn_in_half_by_another_thread() {
    // One writer and whole lines through the channel, so what a reader meets is always a record.
    let at = ADirectory::named("threads");
    let journal = Arc::new(TheJournal::in_directory(&at.0, Note::Detail).expect("a journal"));
    let mut hands = Vec::new();

    for which in 0..8u64 {
        let journal = Arc::clone(&journal);
        hands.push(std::thread::spawn(move || {
            for at in 0..200u64 {
                journal.write_down(
                    Note::Detail,
                    "picture.asked",
                    &[("thread", Value::Count(which)), ("n", Value::Count(at))],
                );
            }
        }));
    }
    for hand in hands {
        hand.join().expect("the thread finished");
    }
    journal.flush_within(std::time::Duration::from_secs(10));

    let written = everything_written(&at.0);
    let lines: Vec<&str> = written.lines().collect();

    for line in &lines {
        assert!(
            line.contains("journal.began") || line.contains("picture.asked"),
            "a line was torn in half: {line:?}"
        );
    }
    // Everything either arrived or was counted as dropped. A journal that quietly loses records
    // answers the next question wrongly.
    let arrived = lines
        .iter()
        .filter(|line| line.contains("picture.asked"))
        .count() as u64;
    let dropped: u64 = written
        .split("dropped=")
        .skip(1)
        .filter_map(|rest| {
            rest.split(|c: char| !c.is_ascii_digit())
                .next()
                .and_then(|digits| digits.parse::<u64>().ok())
        })
        .sum();

    assert_eq!(
        arrived + dropped,
        1600,
        "records went missing without being counted"
    );
}

#[test]
fn a_run_stops_at_its_ceiling_rather_than_filling_the_disk() {
    let at = ADirectory::named("ceiling");
    let journal = TheJournal::in_directory(&at.0, Note::Detail).expect("a journal");

    // Enough to cross a file boundary several times if the boundary were not there.
    for n in 0..20_000u64 {
        journal.write_down(Note::Detail, "picture.asked", &[("n", Value::Count(n))]);
    }
    journal.flush_within(std::time::Duration::from_secs(20));

    let directory = at.0.join(files::DIRECTORY);
    let held: Vec<PathBuf> = std::fs::read_dir(&directory)
        .expect("the folder is there")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();

    assert!(
        held.len() <= files::MOST_PER_RUN as usize,
        "one run filled {} files",
        held.len()
    );
    let together: u64 = held
        .iter()
        .filter_map(|path| path.metadata().ok().map(|about| about.len()))
        .sum();
    assert!(
        together <= files::LONGEST_FILE * u64::from(files::MOST_PER_RUN),
        "one run took {together} bytes"
    );
}

#[test]
fn every_file_of_a_run_belongs_to_it_and_the_runs_are_told_apart() {
    assert_eq!(
        files::belongs_to_a_run("lyonoyl-20260907-114641-31811.log"),
        Some("20260907-114641-31811")
    );
    assert_eq!(
        files::belongs_to_a_run("lyonoyl-20260907-114641-31811.3.log"),
        Some("20260907-114641-31811")
    );
    // Another run of the same second, another process.
    assert_ne!(
        files::belongs_to_a_run("lyonoyl-20260907-114641-31811.log"),
        files::belongs_to_a_run("lyonoyl-20260907-114641-9002.log")
    );
    for not_ours in [
        "notes.log",
        "lyonoyl.log",
        "lyonoyl-.log",
        "profile.sqlite3",
        "lyonoyl-20260907-114641-31811.log.old",
    ] {
        assert_eq!(files::belongs_to_a_run(not_ours), None, "{not_ours}");
    }
}

#[test]
fn what_is_thrown_away_is_a_whole_run_and_never_this_one() {
    // Half a run is worse than none: the record somebody needs is as likely to be in the part that
    // went as in the part that stayed.
    let at = ADirectory::named("room");
    let directory = files::ready(&at.0).expect("the folder");

    for run in 0..(files::RUNS_KEPT + 4) {
        for which in 0..3u32 {
            let name = files::named(&format!("2026090{}-000000", run % 9), run as u32, which);
            std::fs::write(directory.join(name), "x").expect("a file");
        }
    }
    let ours = format!("20260907-000000-{}", files::RUNS_KEPT + 3);
    files::make_room(&directory, &ours).expect("room is made");

    let left: Vec<String> = std::fs::read_dir(&directory)
        .expect("the folder is there")
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .collect();

    let runs: std::collections::HashSet<&str> = left
        .iter()
        .filter_map(|name| files::belongs_to_a_run(name))
        .collect();

    assert!(
        runs.len() <= files::RUNS_KEPT,
        "{} runs are held",
        runs.len()
    );
    for run in &runs {
        let of_this_run = left
            .iter()
            .filter(|name| files::belongs_to_a_run(name) == Some(run))
            .count();
        assert_eq!(of_this_run, 3, "half of run {run} was thrown away");
    }
}

#[test]
fn a_journal_with_nowhere_to_write_is_still_a_journal() {
    // A folder that cannot be made must not be a client that will not start.
    let nowhere = TheJournal::nowhere();

    assert!(!nowhere.keeps(Note::Trouble));
    nowhere.write_down(Note::Trouble, "picture.lost", &[("n", Value::Count(1))]);
    nowhere.flush_within(std::time::Duration::from_millis(10));
}

#[test]
fn the_folder_is_readable_by_nobody_else() {
    let at = ADirectory::named("permissions");
    let directory = files::ready(&at.0).expect("the folder");

    assert!(directory.is_dir());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = directory
            .metadata()
            .expect("the folder is there")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700, "the folder is open to other accounts");
    }
}

#[test]
fn a_day_is_turned_into_a_date_the_way_a_calendar_would() {
    // Written out here rather than taken from a crate, so it is worth pinning against days a
    // calendar gets wrong.
    assert_eq!(civil(0), (1970, 1, 1));
    assert_eq!(civil(59), (1970, 3, 1));
    // 2000 was a leap year and 1900 was not; 2024-02-29 is the one to get wrong.
    assert_eq!(civil(19_782), (2024, 2, 29));
    assert_eq!(civil(19_783), (2024, 3, 1));
    assert_eq!(civil(11_017), (2000, 3, 1));
}
