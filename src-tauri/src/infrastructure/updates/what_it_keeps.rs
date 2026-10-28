//! What the updater keeps on this machine between runs: when this copy last looked, what changed in
//! the version it put in place until somebody has read it, and the folder a version waits in.
//!
//! About the update mechanism rather than about anything a person chose, so none of it belongs in
//! the settings they can see.

use std::path::{Path, PathBuf};

use crate::ports::updates::{Notes, UpdateError};

/// The updater's own folder, where a version waits between coming down and going in.
///
/// Beside the application's own data rather than in a shared temporary directory: what is in there
/// is code about to be run, and a directory anybody on the machine can write is a directory
/// somebody can swap it in.
///
/// **Handed in rather than worked out here.** Where this application keeps its files is one question
/// with one answer, and `bootstrap::data_directory` is where it is answered. A path built here from
/// `HOME` is the databases' directory on one of the three systems and somewhere else on the other
/// two — and Windows sets no `HOME` at all, so the mark saying when this copy last looked is never
/// written and the once-a-day rule is gone.
pub(super) fn the_folder(beside: &Path) -> Result<PathBuf, UpdateError> {
    let waiting = beside.join("updates");

    std::fs::create_dir_all(&waiting).map_err(|_| UpdateError::Unavailable)?;

    Ok(waiting)
}

/// What changed, kept until somebody has read it.
///
/// Written when the version goes in rather than read from the manifest afterwards: by the time the
/// application has restarted the manifest may already name something newer, and a pane fetching it
/// then would show the notes for a version nobody has.
pub(super) fn remember_what_changed(
    beside: &Path,
    version: &str,
    notes: &Notes,
) -> Result<(), UpdateError> {
    let waiting = the_folder(beside)?;

    let written = serde_json::json!({ "version": version, "notes": notes });

    std::fs::write(
        waiting.join("what-changed.json"),
        written.to_string().as_bytes(),
    )
    .map_err(|_| UpdateError::Unavailable)
}

/// What changed in the version now running, if it has not been read.
///
/// `None` is the ordinary answer. It is also the answer when what was kept is about some other
/// version, which is what a copy that was updated and then replaced by hand looks like.
pub(super) fn what_changed(beside: &Path, running: &str) -> Option<(String, Notes)> {
    let at = the_folder(beside).ok()?.join("what-changed.json");
    let text = std::fs::read_to_string(&at).ok()?;
    let read: serde_json::Value = serde_json::from_str(&text).ok()?;

    let version = read.get("version")?.as_str()?.to_owned();
    let notes: Notes = serde_json::from_value(read.get("notes")?.clone()).ok()?;

    (version == running).then_some((version, notes))
}

/// When this copy last asked the project anything.
pub(super) fn when_it_last_looked(beside: &Path) -> Option<u64> {
    let at = the_folder(beside).ok()?.join("last-looked");
    std::fs::read_to_string(at).ok()?.trim().parse().ok()
}

/// Written down before the answer is read, so a project that cannot be reached still counts as
/// having been asked. Without that, a machine that is offline asks on every start for as long as it
/// stays offline.
pub(super) fn remember_it_looked(beside: &Path, now: u64) {
    if let Ok(waiting) = the_folder(beside) {
        let _ = std::fs::write(waiting.join("last-looked"), now.to_string());
    }
}

/// Forgotten once it has been read.
pub(super) fn forget_what_changed(beside: &Path) {
    if let Ok(waiting) = the_folder(beside) {
        let _ = std::fs::remove_file(waiting.join("what-changed.json"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Somewhere of its own for one test, empty.
    fn a_place(name: &str) -> PathBuf {
        let at = std::env::temp_dir().join(format!("lyonoyl-kept-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&at);
        std::fs::create_dir_all(&at).expect("a place");
        at
    }

    /// Where an update waits is inside the directory it was handed, and nowhere else.
    ///
    /// Asserted against the directory given rather than against the end of a path: a path ending in
    /// the right name is also what a directory worked out from `HOME` looks like, somewhere the
    /// databases are not.
    #[test]
    fn a_version_waits_inside_the_directory_it_was_given() {
        let given = a_place("folder");
        let waiting = the_folder(&given).expect("somewhere to wait");

        assert_eq!(
            waiting,
            given.join("updates"),
            "an update waits somewhere other than where this was told to put it"
        );
        assert!(waiting.starts_with(&given));

        let _ = std::fs::remove_dir_all(&given);
    }

    /// What changed is kept in every language the release wrote it in, and comes back only while
    /// it is about the version running.
    #[test]
    fn what_changed_comes_back_whole_and_only_for_its_own_version() {
        let given = a_place("notes");
        let notes = Notes::from([
            ("en".to_owned(), "Pictures load again.".to_owned()),
            ("ru".to_owned(), "in the second language".to_owned()),
        ]);

        remember_what_changed(&given, "1.1.0", &notes).expect("kept");

        assert_eq!(
            what_changed(&given, "1.1.0"),
            Some(("1.1.0".to_owned(), notes.clone()))
        );
        assert_eq!(
            what_changed(&given, "1.2.0"),
            None,
            "a note about another version was shown"
        );

        forget_what_changed(&given);
        assert_eq!(
            what_changed(&given, "1.1.0"),
            None,
            "a note read once came back"
        );

        std::fs::remove_dir_all(&given).ok();
    }

    /// When this copy last looked comes back as it was written, and is nothing before it ever has.
    #[test]
    fn when_it_last_looked_comes_back_as_it_was_written() {
        let given = a_place("looked");

        assert_eq!(when_it_last_looked(&given), None);
        remember_it_looked(&given, 1_700_000_000);
        assert_eq!(when_it_last_looked(&given), Some(1_700_000_000));

        std::fs::remove_dir_all(&given).ok();
    }
}
