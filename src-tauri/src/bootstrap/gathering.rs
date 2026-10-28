//! Where the databases are, and the move a start makes to bring them into their own folder.

use std::path::{Path, PathBuf};

use super::{Storage, Unopened};
use crate::infrastructure::journal::journal;
use crate::infrastructure::sqlite::folder::{
    a_folder_of_their_own, gather, where_they_are, Gathered, NotGathered,
};
use crate::ports::journal::{Journal, Note, Value};

/// Where this run's databases are: in their own folder inside `data`, or at its top while a set
/// found there could not be moved into it.
pub fn databases_of(data: &Path) -> PathBuf {
    where_they_are(data)
}

/// Brings the databases a start finds at the top of `data` into their own folder, writes down what
/// it did, and answers where this run's databases are — with the storage to use instead of opening
/// them, when a move that could not put back what it took has left part of the set in each folder.
///
/// Before anything looks for the databases. The store a key is read from, and whether a key is made
/// at all, are decided by what is beside the profile, and a profile looked for where it is not is a
/// first start: a key made over databases that are only somewhere else.
pub fn gather_the_databases(data: &Path) -> (PathBuf, Option<Storage>) {
    // The data folder as well as the databases': what the updater keeps is in it, and nothing in it
    // is for anybody else. A folder that cannot be narrowed is not a reason to open nothing.
    let _ = a_folder_of_their_own(data);

    after_gathering(data, gather(data))
}

/// What a start makes of what the move did: [`gather_the_databases`] once the move has answered.
pub(super) fn after_gathering(
    data: &Path,
    gathered: Result<Gathered, NotGathered>,
) -> (PathBuf, Option<Storage>) {
    let journal = journal();
    match gathered {
        Ok(Gathered { moved: 0, left: 0 }) => (databases_of(data), None),
        Ok(Gathered { moved, left }) => {
            if journal.keeps(Note::Worth) {
                journal.write_down(
                    Note::Worth,
                    "start.gathered",
                    &[
                        ("moved", Value::Count(u64::from(moved))),
                        ("left", Value::Count(u64::from(left))),
                    ],
                );
            }
            (databases_of(data), None)
        }
        Err(stopped) => {
            if journal.keeps(Note::Trouble) {
                journal.write_down(
                    Note::Trouble,
                    "start.notGathered",
                    &[("putBack", Value::Yes(stopped.put_back))],
                );
            }
            let refused = (!stopped.put_back).then(|| {
                Storage::failed(
                    stopped.path.display().to_string(),
                    format!(
                        "The databases could not be moved into their folder whole: {}",
                        stopped.error
                    ),
                    Unopened::TheFile,
                )
            });
            (databases_of(data), refused)
        }
    }
}
