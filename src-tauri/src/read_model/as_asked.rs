//! What a person has asked for, read out of the queue for a view to lay over the index.
//!
//! One statement and one fold for every view. The list, a conversation, every count in the sidebar
//! and the number on the application's icon read the same instructions and fold them the same way
//! ([`lyonoyl_domain::as_asked`]), so none of them can say a message is read while another says it
//! is not. The index is never asked to say what a person asked for: it holds what a server said,
//! and the two are read together here.

use std::collections::HashMap;

use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::read_model::list::ReadModelError;
use lyonoyl_domain::as_asked::{AsAsked, Asked, Change};
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::message::{Flags, FolderId, MessageId};
use lyonoyl_domain::operation::{DeleteIntent, OperationKind, OperationState, Stopped};

/// One instruction a view has to show, as the queue holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Queued {
    /// The number the index knows the account by, which is how its file is found.
    pub account: i64,
    pub message: MessageId,
    pub folder: FolderId,
    pub kind: OperationKind,
    pub state: OperationState,
    pub stopped: Option<Stopped>,
}

/// Which instructions a view shows: everything still to be told, everything that stopped, and
/// every move or delete the server has carried out and no pass has read back.
///
/// Written word for word as the queue's own index is, which is what lets SQLite answer it from that
/// index rather than by reading the queue: a list asks this for every window it draws.
const OUTSTANDING: &str =
    "(o.state IN ('pending', 'needs_reresolve', 'conflicted') OR o.owes_a_read = 1)";

/// What the queue holds for the messages a window is about to show, oldest first.
///
/// `places` is one `?` per message. Named rather than written where it is run, so the test that
/// checks an index answers it can ask the statement itself.
pub(crate) fn what_is_waiting_for(places: &str) -> String {
    format!(
        "{} WHERE {OUTSTANDING} AND o.message_id IN ({places}) ORDER BY o.id ASC",
        selected()
    )
}

fn selected() -> &'static str {
    "SELECT o.id, a.index_id, o.message_id, o.folder_id, o.kind, o.flags_add, o.flags_remove,
            o.move_to, o.delete_intent, o.state, o.last_error
     FROM op_log o
     JOIN mail_accounts a ON a.id = o.account_id"
}

/// Every instruction a view shows for these messages.
pub fn about(profile: &SqlitePool, messages: &[i64]) -> Result<Vec<Queued>, ReadModelError> {
    let mut queued = Vec::new();

    // In slices, because a question has a limit on how much it may be asked about, and emptying a
    // large trash is one instruction per message in it.
    for slice in messages.chunks(500) {
        let places = std::iter::repeat_n("?", slice.len())
            .collect::<Vec<_>>()
            .join(", ");
        queued.extend(read(
            profile,
            &what_is_waiting_for(&places),
            rusqlite::params_from_iter(slice),
        )?);
    }

    queued.sort_by_key(|one| one.order);
    Ok(queued.into_iter().map(|one| one.queued).collect())
}

/// Every instruction a view shows for one account.
pub fn of_account(profile: &SqlitePool, account: i64) -> Result<Vec<Queued>, ReadModelError> {
    Ok(read(
        profile,
        &format!(
            "{} WHERE {OUTSTANDING} AND a.index_id = ?1 ORDER BY o.id ASC",
            selected()
        ),
        [account],
    )?
    .into_iter()
    .map(|one| one.queued)
    .collect())
}

/// Every instruction a view shows, for every account, with how many instructions a person had
/// asked for when the queue was read.
///
/// One reading for both. Read one after the other, an instruction written in between would be in
/// the count and not in the instructions, and a window comparing the count with a change it holds
/// would take the change as read and show counts that do not have it.
pub fn everywhere_as_of(profile: &SqlitePool) -> Result<(Vec<Queued>, u64), ReadModelError> {
    let mut connection = profile.get().map_err(|_| ReadModelError::Unavailable)?;
    let reading = connection
        .transaction()
        .map_err(|_| ReadModelError::Unavailable)?;

    let queued = read_on(
        &reading,
        &format!("{} WHERE {OUTSTANDING} ORDER BY o.id ASC", selected()),
        [],
    )?
    .into_iter()
    .map(|one| one.queued)
    .collect();
    let asked: i64 = reading
        .query_row("SELECT last FROM asked_counter WHERE id = 1", [], |row| {
            row.get(0)
        })
        .map_err(|_| ReadModelError::Unavailable)?;

    Ok((
        queued,
        u64::try_from(asked).map_err(|_| ReadModelError::Corrupted)?,
    ))
}

/// One row, with where it stands in the queue, so slices read apart can be put back in order.
struct InOrder {
    order: i64,
    queued: Queued,
}

type Row = (
    i64,
    i64,
    String,
    i64,
    i64,
    Option<i64>,
    Option<String>,
    String,
    Option<String>,
);

fn read(
    profile: &SqlitePool,
    statement: &str,
    arguments: impl rusqlite::Params,
) -> Result<Vec<InOrder>, ReadModelError> {
    let connection = profile.get().map_err(|_| ReadModelError::Unavailable)?;
    read_on(&connection, statement, arguments)
}

fn read_on(
    connection: &rusqlite::Connection,
    statement: &str,
    arguments: impl rusqlite::Params,
) -> Result<Vec<InOrder>, ReadModelError> {
    let mut prepared = connection
        .prepare(statement)
        .map_err(|_| ReadModelError::Unavailable)?;

    let rows = prepared
        .query_map(arguments, |row| {
            Ok((
                row.get::<_, i64>(0)?,
                (
                    row.get(1)?,
                    (
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                        row.get(10)?,
                    ),
                ),
            ))
        })
        .map_err(|_| ReadModelError::Unavailable)?
        .collect::<Result<Vec<(i64, (i64, Row))>, _>>()
        .map_err(|_| ReadModelError::Unavailable)?;

    rows.into_iter()
        .map(|(order, (account, row))| {
            Ok(InOrder {
                order,
                queued: decoded(account, row)?,
            })
        })
        .collect()
}

/// A row as the domain names it. A row this build cannot read is a queue it cannot read, which is
/// a corrupted answer rather than an instruction quietly left off the screen.
fn decoded(account: i64, row: Row) -> Result<Queued, ReadModelError> {
    let (message, folder, kind, add, remove, move_to, intent, state, reason) = row;
    let folder_id = |value: i64| FolderId::new(value as u64).map_err(|_| ReadModelError::Corrupted);

    Ok(Queued {
        account,
        message: MessageId::new(message as u64).map_err(|_| ReadModelError::Corrupted)?,
        folder: folder_id(folder)?,
        kind: match (kind.as_str(), move_to, intent.as_deref()) {
            ("set_flags", _, _) => OperationKind::SetFlags {
                add: Flags::from_bits(add as u32),
                remove: Flags::from_bits(remove as u32),
            },
            ("move", Some(to), _) => OperationKind::MoveTo {
                folder: folder_id(to)?,
            },
            ("delete", _, Some("to_trash")) => OperationKind::Delete {
                intent: DeleteIntent::ToTrash,
            },
            ("delete", _, Some("permanently")) => OperationKind::Delete {
                intent: DeleteIntent::Permanently,
            },
            _ => return Err(ReadModelError::Corrupted),
        },
        state: match state.as_str() {
            "pending" => OperationState::Pending,
            "needs_reresolve" => OperationState::NeedsReresolve,
            "conflicted" => OperationState::Conflicted,
            "done" => OperationState::Done,
            _ => return Err(ReadModelError::Corrupted),
        },
        // A reason this build has no words for reads as the server refusing: something stopped,
        // and the person still has to see that.
        stopped: reason.as_deref().map(Stopped::parse),
    })
}

/// The instructions, folded, with each account's trash found where a delete needs it.
///
/// `trash_of` is asked once per account, and only for an account with a delete that is meant to
/// be taken back: that is the only instruction whose destination the queue does not name.
pub fn folded(queued: &[Queued], mut trash_of: impl FnMut(i64) -> Option<FolderId>) -> AsAsked {
    let mut trash: HashMap<i64, Option<FolderId>> = HashMap::new();

    AsAsked::of(queued.iter().filter_map(|one| {
        let into_trash = matches!(
            one.kind,
            OperationKind::Delete {
                intent: DeleteIntent::ToTrash
            }
        );
        let trash = match into_trash {
            true => *trash
                .entry(one.account)
                .or_insert_with(|| trash_of(one.account)),
            false => None,
        };

        Change::of(one.kind, one.state, one.stopped, trash).map(|change| Asked {
            message: one.message,
            folder: one.folder,
            change,
        })
    }))
}

/// The folder an account keeps deleted mail in, from its own file.
pub fn trash_in(index: &SqlitePool, account: i64) -> Option<FolderId> {
    let connection = index.get().ok()?;
    trash_on(&connection, account)
}

/// The same, asked on a connection already open, so that it reads whatever snapshot that
/// connection is reading.
pub fn trash_on(connection: &rusqlite::Connection, account: i64) -> Option<FolderId> {
    connection
        .query_row(
            "SELECT id FROM folders WHERE account_id = ?1 AND role = ?2 ORDER BY id LIMIT 1",
            [account, FolderRole::Trash.stored()],
            |row| row.get::<_, i64>(0),
        )
        .ok()
        .and_then(|id| FolderId::new(id as u64).ok())
}
