use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;

use crate::commands::error::{CommandError, ErrorCode};
use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::read_model::list::{
    ListReadModel, ListRow, ReadModelError, ViewQuery, ViewScope, ViewWindow,
};
use crate::AppState;
use lyonoyl_domain::message::{FolderId, MessageId};

/// The largest window one call may ask for. A virtualized list needs tens of rows at a time.
const MAX_LIMIT: u32 = 500;

/// A message identifier as text.
///
/// The identifier is a 64-bit number and JavaScript numbers are not, so it crosses as a decimal
/// string and is treated as opaque on the other side.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct MessageIdDto(pub String);

/// A folder identifier as text, for the same reason a message identifier is.
///
/// It was a `u32` while a folder number counted rows in one file. Once the mail is kept per
/// account the number carries the file it came from, and what did not fit was quietly dropped:
/// a row arrived with no folder, and with it went its read toggle and its retry link.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct FolderIdDto(pub String);

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct ListRowDto {
    /// How many messages the conversation this row stands for holds.
    ///
    /// One when the row is a message rather than a conversation, and one for a conversation of
    /// one — the interface shows a count only where there is more than one thing to count.
    pub in_conversation: u32,
    /// The conversation this row stands for, where it stands for one.
    ///
    /// A row shows a conversation's newest message, and a reply arriving makes a different message
    /// the newest — so what a row shows is not a name the conversation keeps. The interface holds
    /// a conversation open by this instead, and a row still naming it is the same conversation.
    ///
    /// Absent where the list is showing messages rather than conversations.
    pub conversation_id: Option<MessageIdDto>,
    pub message_id: MessageIdDto,
    /// Why an instruction about this message stopped, when one has: the key the interface reads
    /// its explanation by. Absent for a row nothing has stopped on.
    pub stopped: Option<String>,
    /// The folder this row stands for, which is what anything done to it is done in.
    ///
    /// Absent only for a message that sits in no folder this device knows; the interface offers
    /// nothing to do with such a row rather than guessing at one.
    pub folder_id: Option<FolderIdDto>,
    pub subject: Option<String>,
    pub sender: Option<String>,
    /// The opening of the message, as plain text. Absent until its body has been read.
    pub preview: Option<String>,
    /// Unix seconds. Declared as a number rather than a big integer: seconds fit a JavaScript
    /// number exactly, and JSON delivers one whatever the Rust type is.
    #[ts(type = "number")]
    pub received_at: i64,
    pub unread: bool,
    /// Whether this message has been told to go somewhere else and the server has not answered yet.
    /// The row is shown on its way out rather than removed — it is still counted where it is.
    pub leaving: bool,
    /// Whether it is on its way *into* the folder being looked at, and not there yet.
    ///
    /// The other half of `leaving`. Between the instruction and the pass that carries it out, a
    /// message was gone from one folder and not yet in the other -- which reads as one that was
    /// lost.
    pub arriving: bool,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct ViewWindowDto {
    pub rows: Vec<ListRowDto>,
    /// The size of the whole set, not of this window.
    pub total: u32,
    /// Changes when the set changes. The list re-anchors on it instead of reloading.
    pub view_epoch: String,
    /// Accounts whose mail could not be read for this window, by the identifier they go by.
    ///
    /// Their mail is missing from the rows and from the total, and the window says so rather than
    /// coming back shorter with no explanation. Empty is the ordinary case.
    pub not_answering: Vec<String>,
}

impl From<ListRow> for ListRowDto {
    fn from(row: ListRow) -> Self {
        Self {
            in_conversation: row.in_conversation,
            conversation_id: row
                .conversation_id
                .map(|id| MessageIdDto(id.get().to_string())),
            message_id: MessageIdDto(row.message_id.get().to_string()),
            folder_id: row.folder_id.map(|id| FolderIdDto(id.get().to_string())),
            stopped: row.stopped.map(|reason| reason.as_str().to_owned()),
            subject: row.subject,
            sender: row.sender,
            preview: row.preview,
            received_at: row.received_at,
            unread: row.unread,
            leaving: row.leaving,
            arriving: row.arriving,
        }
    }
}

impl ViewWindowDto {
    /// Turns a window into what crosses, naming the accounts that would not answer.
    ///
    /// The names are in the profile, not in the mail — which is why this takes it. An account whose
    /// file could not be read is named by what a person calls it, because a number would tell them
    /// nothing about which of their mailboxes is missing.
    fn of(window: ViewWindow, profile: &SqlitePool) -> Self {
        let not_answering = window
            .not_answering
            .into_iter()
            .filter_map(|account| {
                crate::read_model::mailboxes::account_named(profile, account)
                    .ok()
                    .flatten()
            })
            .collect();

        Self {
            rows: window.rows.into_iter().map(ListRowDto::from).collect(),
            total: window.total,
            view_epoch: window.view_epoch.to_string(),
            not_answering,
        }
    }
}

impl From<ReadModelError> for CommandError {
    fn from(error: ReadModelError) -> Self {
        match error {
            ReadModelError::Unavailable => Self::new(
                ErrorCode::StorageUnavailable,
                "The local database is unavailable.",
            ),
            ReadModelError::Corrupted => Self::new(
                ErrorCode::StorageCorrupted,
                "Locally stored data could not be read.",
            ),
        }
    }
}

/// Which mail the list is asked for.
///
/// An account is named by its own identifier rather than by the number the index uses: that number
/// is a storage detail, and nothing outside the core has any business holding it.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "kind"
)]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum ViewScopeDto {
    Folder {
        /// Text, like the identifier the row carries back: a folder number holds the file the
        /// folder is kept in, and that is larger than a JavaScript number holds exactly.
        folder_id: String,
    },
    Account {
        account_id: String,
    },
    Everything,
    /// The mail of the accounts in one group.
    ///
    /// Named by the group rather than by the accounts in it, so that a group edited in another
    /// window cannot leave the list showing a set nobody chose.
    Group {
        group_id: String,
    },
    /// What somebody typed into the search box, as they typed it.
    ///
    /// It crosses as text and is turned into a question for the index in one place inside the
    /// core. Nothing the interface sends is a query.
    Looking {
        typed: String,
    },
}

/// Reads one window of the message list.
/// The messages of the conversation a row stands for, oldest first.
///
/// Asked by the message the row shows: a conversation has no identifier of its own that crosses,
/// and the row already knows which message it is standing for.
#[tauri::command]
pub async fn get_conversation(
    state: State<'_, AppState>,
    message_id: String,
) -> Result<Vec<ListRowDto>, CommandError> {
    let message = message_id
        .parse::<u64>()
        .ok()
        .and_then(|value| MessageId::new(value).ok())
        .ok_or_else(|| {
            CommandError::new(ErrorCode::InvalidRequest, "Message identifier is invalid.")
        })?;

    let profile = state.storage.profile()?;
    let indexes = state.storage.index_across()?;

    let rows = tauri::async_runtime::spawn_blocking(move || {
        ListReadModel::spanning(indexes, profile).conversation_of(message)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))??;

    Ok(rows.into_iter().map(ListRowDto::from).collect())
}

#[tauri::command]
pub async fn get_view_window(
    state: State<'_, AppState>,
    scope: ViewScopeDto,
    unread_only: bool,
    offset: u32,
    limit: u32,
) -> Result<ViewWindowDto, CommandError> {
    a_window_this_build_will_read(limit)?;

    let profile = state.storage.profile()?;
    let index = state.storage.index_across()?;

    let window = tauri::async_runtime::spawn_blocking(move || {
        let scope = resolve(scope, &profile)?;
        ListReadModel::spanning(index, profile.clone())
            .get_view_window(
                &ViewQuery {
                    scope,
                    unread_only,
                    // Conversations wherever they can be answered. A folder keeps messages.
                    as_conversations: true,
                },
                offset,
                limit,
            )
            .map_err(CommandError::from)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))??;

    Ok(ViewWindowDto::of(window, &state.storage.profile()?))
}

/// Whether this is a window this build will read.
///
/// A named function rather than two lines inside the command, so that it can be asked the question
/// without a running application. The rule is the only bound on how much one call may read, and it
/// was pinned by nothing: raising `MAX_LIMIT`, or dropping the check, broke no test at all.
fn a_window_this_build_will_read(limit: u32) -> Result<(), CommandError> {
    if limit == 0 || limit > MAX_LIMIT {
        return Err(CommandError::new(
            ErrorCode::InvalidRequest,
            "A window holds between 1 and 500 rows.",
        ));
    }

    Ok(())
}

/// Turns what the interface asked for into what the index can answer.
fn resolve(
    scope: ViewScopeDto,
    profile: &crate::infrastructure::sqlite::connection::SqlitePool,
) -> Result<ViewScope, CommandError> {
    match scope {
        ViewScopeDto::Folder { folder_id } => folder_id
            .parse::<u64>()
            .map_err(|_| ())
            .and_then(|value| FolderId::new(value).map_err(|_| ()))
            .map(ViewScope::Folder)
            .map_err(|_| {
                CommandError::new(ErrorCode::InvalidRequest, "Folder identifier is invalid.")
            }),
        ViewScopeDto::Account { account_id } => {
            let number = crate::read_model::mailboxes::index_id_of(profile, &account_id)?
                .ok_or_else(|| {
                    CommandError::new(
                        ErrorCode::InvalidRequest,
                        "That account is not on this device.",
                    )
                })?;
            Ok(ViewScope::Account(number))
        }
        ViewScopeDto::Everything => Ok(ViewScope::Everything),
        // Read here rather than sent from the window: the accounts in a group are looked up at
        // the moment the list is drawn, so a group changed elsewhere is answered as it is now.
        //
        // A group that is gone reads as one holding nothing, which is an empty list. Not an
        // error: a person who removed a group in the settings screen while the list stood on it
        // has done nothing wrong, and a red panel would be the application blaming them for it.
        ViewScopeDto::Group { group_id } => Ok(ViewScope::Group(
            crate::read_model::mailboxes::accounts_in_group(profile, &group_id)?,
        )),
        ViewScopeDto::Looking { typed } => Ok(ViewScope::Looking(typed)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_of_nothing_is_refused() {
        // `limit == 0` asks the read model for a page of no rows, which is a caller mistake rather
        // than an empty folder — and an empty folder answers with an empty window, not an error.
        let refused = a_window_this_build_will_read(0).expect_err("a window of nothing");
        assert_eq!(refused.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn a_window_larger_than_this_build_reads_is_refused() {
        // The only bound on how much one call may read, and until now nothing pinned it: raising
        // MAX_LIMIT, or deleting the check, failed no test at all.
        assert!(a_window_this_build_will_read(MAX_LIMIT + 1).is_err());
        assert!(a_window_this_build_will_read(u32::MAX).is_err());

        let refused = a_window_this_build_will_read(MAX_LIMIT + 1).expect_err("too large");
        assert_eq!(refused.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn the_windows_a_person_actually_asks_for_are_allowed() {
        // Both ends of what is allowed, so a bound that moved by one is caught.
        assert!(a_window_this_build_will_read(1).is_ok());
        assert!(a_window_this_build_will_read(MAX_LIMIT).is_ok());
        assert!(a_window_this_build_will_read(50).is_ok());
    }

    #[test]
    fn an_identifier_crosses_as_text_rather_than_a_number() {
        // 2^53 + 1 cannot be represented by a JavaScript number, and identifiers are allowed to
        // reach that size.
        let row = ListRow {
            conversation_id: None,
            in_conversation: 1,
            message_id: lyonoyl_domain::message::MessageId::new(9_007_199_254_740_993)
                .expect("non-zero"),
            folder_id: None,
            stopped: None,
            subject: None,
            sender: None,
            preview: None,
            received_at: 0,
            unread: true,
            leaving: false,
            arriving: false,
        };

        let dto = ListRowDto::from(row);
        assert_eq!(dto.message_id.0, "9007199254740993");
    }

    #[test]
    fn a_row_keeps_the_folder_it_sits_in_however_large_its_number() {
        // A folder number carries the file it came from once the mail is split per account, so it
        // outgrows a 32-bit field. Losing it takes the row's read toggle and its retry link with
        // it, and losing it quietly is worse than refusing to answer.
        let row = ListRow {
            conversation_id: None,
            in_conversation: 1,
            message_id: lyonoyl_domain::message::MessageId::new(1).expect("non-zero"),
            folder_id: Some(FolderId::new(u64::from(u32::MAX) + 1).expect("non-zero")),
            stopped: None,
            subject: None,
            sender: None,
            preview: None,
            received_at: 0,
            unread: true,
            leaving: false,
            arriving: false,
        };

        let dto = ListRowDto::from(row);
        assert_eq!(
            dto.folder_id
                .expect("the row was handed over without the folder it sits in")
                .0,
            "4294967296"
        );
    }

    #[test]
    fn a_scope_is_read_in_the_shape_the_interface_sends_it() {
        // The generated TypeScript says `{kind, folderId}`. A field left in snake_case here would
        // compile, generate matching types, and fail only when a person clicks a folder.
        let folder: ViewScopeDto =
            serde_json::from_str(r#"{"kind":"folder","folderId":"7"}"#).expect("deserializes");
        assert!(matches!(&folder, ViewScopeDto::Folder { folder_id } if folder_id == "7"));

        let account: ViewScopeDto =
            serde_json::from_str(r#"{"kind":"account","accountId":"acc-1"}"#)
                .expect("deserializes");
        assert!(matches!(account, ViewScopeDto::Account { account_id } if account_id == "acc-1"));

        let everything: ViewScopeDto =
            serde_json::from_str(r#"{"kind":"everything"}"#).expect("deserializes");
        assert!(matches!(everything, ViewScopeDto::Everything));

        // A search is the one scope carrying something a person typed, so it is the one whose
        // wire shape can be wrong about more than a name.
        let looking: ViewScopeDto =
            serde_json::from_str(r#"{"kind":"looking","typed":"quarterly numbers"}"#)
                .expect("deserializes");
        assert!(
            matches!(&looking, ViewScopeDto::Looking { typed } if typed == "quarterly numbers")
        );
    }

    #[test]
    fn an_epoch_crosses_as_text_for_the_same_reason() {
        let window = ViewWindow {
            rows: Vec::new(),
            total: 0,
            view_epoch: u64::MAX,
            not_answering: Vec::new(),
        };

        let profile = crate::infrastructure::fake_provider::SeededProfile::new("epoch-as-text");
        assert_eq!(
            ViewWindowDto::of(window, &profile.pool).view_epoch,
            u64::MAX.to_string()
        );
    }
}
