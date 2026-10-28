//! How many files this process may hold open at once.
//!
//! A launch from Finder or the Dock starts with a soft limit of 256; a terminal starts with far more.
//! Every account holds its database, the database's log and its connections to the server, and
//! AppKit, Metal, WebKit and the keychain open their own files out of the same allowance. When it
//! runs out they cannot read those files, and the application stops before its window is drawn.

use rustix::process::{getrlimit, setrlimit, Resource, Rlimit};

/// The soft limit asked for: `OPEN_MAX`, the most macOS accepts for it.
const ENOUGH: u64 = 10_240;

/// Raises the soft limit on open files to [`ENOUGH`], or to the hard limit where that is lower.
/// A limit already that high is left as it is, and a refusal leaves the limit as it was.
pub fn allow_enough() {
    let limit = getrlimit(Resource::Nofile);
    if let Some(current) = raised(limit.current, limit.maximum) {
        let _ = setrlimit(
            Resource::Nofile,
            Rlimit {
                current: Some(current),
                maximum: limit.maximum,
            },
        );
    }
}

/// The soft limit to ask for, or nothing when the one in force is already enough. `None` is no
/// limit at all, as `getrlimit` reports it.
fn raised(current: Option<u64>, maximum: Option<u64>) -> Option<u64> {
    let wanted = maximum.map_or(ENOUGH, |maximum| maximum.min(ENOUGH));
    current.filter(|current| *current < wanted).map(|_| wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test below that holds a thousand files, by the name the test binary knows it by.
    const HOLDS_A_THOUSAND: &str =
        "infrastructure::open_files::tests::a_thousand_files_open_once_allowed_from_256";

    #[test]
    fn the_limit_a_launch_from_finder_gets_is_raised_to_enough() {
        assert_eq!(raised(Some(256), None), Some(ENOUGH));
    }

    #[test]
    fn a_limit_already_higher_is_never_lowered() {
        assert_eq!(raised(Some(1_048_576), None), None);
        assert_eq!(raised(Some(ENOUGH), None), None);
        assert_eq!(raised(None, None), None);
    }

    #[test]
    fn a_hard_limit_below_enough_is_as_far_as_it_goes() {
        assert_eq!(raised(Some(256), Some(4_096)), Some(4_096));
        assert_eq!(raised(Some(4_096), Some(4_096)), None);
        assert_eq!(raised(Some(256), Some(ENOUGH * 4)), Some(ENOUGH));
    }

    #[test]
    fn a_process_started_with_the_limit_from_finder_holds_a_thousand_files() {
        // This test binary again, started through `sh` with the soft limit a launch from Finder
        // gets and the hard limit left as it was.
        let binary = std::env::current_exe().expect("the test binary knows where it is");
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(r#"ulimit -S -n 256 && exec "$0" --exact --ignored "$1""#)
            .arg(&binary)
            .arg(HOLDS_A_THOUSAND)
            .output()
            .expect("the test binary started again");

        let said = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "{said}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        // A name that matches no test passes as well, so the run has to say it ran this one.
        assert!(said.contains("test result: ok. 1 passed"), "{said}");
    }

    #[test]
    #[ignore = "started by the test above, in a process with the open-file limit a launch from Finder gets"]
    fn a_thousand_files_open_once_allowed_from_256() {
        assert_eq!(
            getrlimit(Resource::Nofile).current,
            Some(256),
            "started with another limit"
        );

        allow_enough();

        let held: Vec<std::fs::File> = (0..1_000)
            .map(|n| {
                std::fs::File::open("/dev/null").unwrap_or_else(|why| panic!("file {n}: {why}"))
            })
            .collect();
        assert_eq!(held.len(), 1_000);
    }
}
