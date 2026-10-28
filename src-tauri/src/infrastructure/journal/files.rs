//! Where the records go, what a file is called, and what is thrown away to make room.

use std::path::{Path, PathBuf};

/// The folder, beside the databases rather than in the system's own log directory.
///
/// Not `~/Library/Logs` on a Mac: `sysdiagnose` gathers that folder into a bundle people send to
/// their vendor, and mail diagnostics would ride along in it. Beside the databases is where the
/// owner already looks, and it is the folder this application already treats as its own.
pub const DIRECTORY: &str = "logs";

/// The most one file holds before the next one is started.
pub const LONGEST_FILE: u64 = 8 * 1024 * 1024;

/// How many files one run may fill. Past this the run stops writing rather than rolling for ever:
/// a run that logs in a loop must not be able to push out every other run's records.
pub const MOST_PER_RUN: u32 = 8;

/// How many runs are kept, and how much they may take together. Both, because either alone is a
/// hole: twenty runs of one line is nothing, and one run of sixty-four megabytes is every other
/// run gone.
pub const RUNS_KEPT: usize = 20;
pub const KEPT_TOGETHER: u64 = 64 * 1024 * 1024;

/// What a file of this run is called.
///
/// The moment the run started, then the process, then which file of the run it is. The process is
/// in the name because one process per data directory is not something this build can promise —
/// the single-instance plugin holds a name that is per session on Windows and per session bus on
/// Linux — and two processes appending to one file interleave halfway through a line.
pub fn named(started: &str, process: u32, which: u32) -> String {
    match which {
        0 => format!("lyonoyl-{started}-{process}.log"),
        more => format!("lyonoyl-{started}-{process}.{more}.log"),
    }
}

/// Whether this is one of ours, and which run it belongs to.
///
/// The run is everything up to the last dot-separated part, so every file of one run sorts together
/// and is thrown away together. Half a run is worse than none: the interesting record is as likely
/// to be in the part that went.
pub fn belongs_to_a_run(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("lyonoyl-")?.strip_suffix(".log")?;
    let run = match rest.rsplit_once('.') {
        // `<started>-<process>.<which>` -- the tail is a number, so the run is what precedes it.
        Some((run, which)) if which.bytes().all(|byte| byte.is_ascii_digit()) => run,
        _ => rest,
    };

    (!run.is_empty()).then_some(run)
}

/// One run held in the folder: what it is called, what it takes, and when it was written.
struct ARun {
    run: String,
    files: Vec<PathBuf>,
    bytes: u64,
    newest: std::time::SystemTime,
}

/// Throws away whole runs until what is left is inside both bounds, and never touches this one.
///
/// Ordered by the newest file each run holds rather than by the name, because a name is a string
/// somebody could have written by hand and a run started before the clock was corrected sorts
/// wherever its name says. Failure to remove one is not failure to log: what cannot be removed is
/// left and the next run tries again.
pub fn make_room(directory: &Path, ours: &str) -> std::io::Result<()> {
    let mut runs: Vec<ARun> = Vec::new();

    for entry in std::fs::read_dir(directory)? {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(run) = belongs_to_a_run(name) else {
            continue;
        };
        if run == ours {
            continue;
        }

        let Ok(about) = entry.metadata() else {
            continue;
        };
        let written = about.modified().unwrap_or(std::time::UNIX_EPOCH);

        match runs.iter_mut().find(|held| held.run == run) {
            Some(held) => {
                held.files.push(path);
                held.bytes += about.len();
                held.newest = held.newest.max(written);
            }
            None => runs.push(ARun {
                run: run.to_owned(),
                files: vec![path],
                bytes: about.len(),
                newest: written,
            }),
        }
    }

    // Newest first, so what goes is taken off the end.
    runs.sort_by_key(|run| std::cmp::Reverse(run.newest));

    let mut held: u64 = runs.iter().map(|run| run.bytes).sum();

    while runs.len() > RUNS_KEPT.saturating_sub(1) || held > KEPT_TOGETHER {
        let Some(oldest) = runs.pop() else { break };

        for file in &oldest.files {
            let _ = std::fs::remove_file(file);
        }
        held = held.saturating_sub(oldest.bytes);
    }

    Ok(())
}

/// The folder, made if it is not there, and readable by nobody else.
///
/// The permissions are set here rather than left to the umask: a log of when somebody read their
/// mail is not a file for every account on a shared machine. On Windows a folder inside the user's
/// local application data already inherits an access list that names the user, and there is no
/// portable call to narrow it further, so this is where the two systems differ and the difference
/// is left where it is.
pub fn ready(inside: &Path) -> std::io::Result<PathBuf> {
    let directory = inside.join(DIRECTORY);

    std::fs::create_dir_all(&directory)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700));
    }

    Ok(directory)
}
