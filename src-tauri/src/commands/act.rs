//! Acting on a message: the thin edge of it.
//!
//! What arrives is a message and the folder it is being acted on in — a person looking at the
//! same message in two folders is acting on the copy in front of them. No position in a mailbox
//! crosses here, and none is written down: where the message actually sits is looked up when the
//! server is finally told.
//!
//! The window is one way in and a notification's button is the other: the same instructions, put
//! in the same queue, by the same few lines.

use tauri::State;

use crate::application::act_on_mail::ActOnMail;
use crate::application::error::UseCaseError;
use crate::application::put_away::{PutAway, PutAwayOutcome};
use crate::application::take_back::{TakeBack, TakenBack};
use crate::bootstrap::adapters;
use crate::commands::error::{CommandError, ErrorCode};
use crate::commands::list::FolderIdDto;
use crate::commands::now::asked_for;
use crate::ports::notifier::PutInto;
use crate::ports::op_log::OperationLog;
use crate::read_model::mailboxes::MailboxReadModel;
use crate::AppState;
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::message::{FolderId, MessageId};
use lyonoyl_domain::operation::{may_receive, DeleteIntent, OperationTarget};

/// Marks a message read, or unread.
///
/// Answers as soon as the intention is written down, not when the server has been told: the point
/// of writing it down is that the two are different moments. What it answers with is where the
/// write stands among everything asked for, which a read of the sidebar answers with too: the
/// window lays the change over counts read before it and over none read after.
#[tauri::command]
pub async fn mark_message_read(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    message_id: String,
    folder_id: String,
    read: bool,
) -> Result<u64, CommandError> {
    let target = target_of(&message_id, &folder_id)?;
    let profile = state.storage.profile()?;

    let (account_id, asked) = tauri::async_runtime::spawn_blocking(move || {
        let account_id = account_owning(&profile, target.folder)?;

        ActOnMail::new(adapters::operation_log(profile))
            .set_read(&account_id, target, read)
            .map(|asked| (account_id, asked))
            .map_err(CommandError::from)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))??;

    asked_for(&app, &state, &account_id);
    Ok(asked)
}

/// Files a message in another folder.
///
/// The destination is refused if it is a folder that means something else: a message sitting where
/// the interface shows drafts, or what has already been sent, is a message shown as something it
/// is not.
#[tauri::command]
pub async fn move_message(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    message_id: String,
    folder_id: String,
    to_folder_id: String,
) -> Result<(), CommandError> {
    let target = target_of(&message_id, &folder_id)?;
    let destination = to_folder_id
        .parse::<u64>()
        .ok()
        .and_then(|value| FolderId::new(value).ok())
        .ok_or_else(|| {
            CommandError::new(ErrorCode::InvalidRequest, "Folder identifier is invalid.")
        })?;
    let profile = state.storage.profile()?;
    let index = state.storage.index_holding_folder(target.folder.get())?;

    tauri::async_runtime::spawn_blocking(move || {
        let mailboxes = MailboxReadModel::new(profile.clone(), index);

        // The account the message is in, first. Everything after is asked about that account and
        // no other: a destination is resolved inside it, so a folder belonging to somebody else's
        // account is not a folder this move can name.
        let Some(account) = mailboxes.account_number_of_folder(target.folder)? else {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "That folder is not on this device.",
            ));
        };

        let Some(role) = mailboxes.folder_purpose_in(account, destination)? else {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "That folder is not on this device.",
            ));
        };

        if !may_receive(role) {
            return Err(CommandError::new(
                ErrorCode::FolderCannotReceive,
                "Messages cannot be filed in that folder.",
            ));
        }

        let account_id = account_owning(&profile, target.folder)?;

        ActOnMail::new(adapters::operation_log(profile))
            .move_to(&account_id, target, destination)
            .map(|()| account_id)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))
    .and_then(|account| account.map(|account_id| asked_for(&app, &state, &account_id)))
}

/// Files a message where the account keeps mail it has finished with.
///
/// Which folder that is, is resolved here rather than sent in: an interface that had to know would
/// have to keep a map of folder roles, and it would be wrong on the day an account gains one.
#[tauri::command]
pub async fn archive_message(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    message_id: String,
    folder_id: String,
) -> Result<(), CommandError> {
    let target = target_of(&message_id, &folder_id)?;
    let profile = state.storage.profile()?;
    let index = state.storage.index_holding_folder(target.folder.get())?;

    tauri::async_runtime::spawn_blocking(move || {
        let mailboxes = MailboxReadModel::new(profile.clone(), index);

        let Some(account) = mailboxes.account_number_of_folder(target.folder)? else {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "That folder is not on this device.",
            ));
        };

        // An account with nowhere to file finished mail is told so rather than having a folder
        // chosen for it.
        let Some(archive) = mailboxes.folder_for_role(account, FolderRole::Archive)? else {
            return Err(CommandError::new(
                ErrorCode::FolderCannotReceive,
                "This account has no archive folder.",
            ));
        };

        let account_id = account_owning(&profile, target.folder)?;

        ActOnMail::new(adapters::operation_log(profile))
            .move_to(&account_id, target, archive)
            .map(|()| account_id)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))
    .and_then(|account| account.map(|account_id| asked_for(&app, &state, &account_id)))
}

/// Throws a message away, in the sense the person meant.
///
/// `permanently` is the difference between something they can get back and something they cannot,
/// and it is carried as what was meant rather than as how it will be done: a server with somewhere
/// to keep deleted mail is told to move, one without is told to mark and expunge, and which of
/// those happens is decided against that server when the moment comes.
#[tauri::command]
pub async fn delete_message(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    message_id: String,
    folder_id: String,
    permanently: bool,
) -> Result<(), CommandError> {
    let target = target_of(&message_id, &folder_id)?;
    let intent = match permanently {
        true => DeleteIntent::Permanently,
        false => DeleteIntent::ToTrash,
    };
    let profile = state.storage.profile()?;

    tauri::async_runtime::spawn_blocking(move || {
        let account_id = account_owning(&profile, target.folder)?;

        ActOnMail::new(adapters::operation_log(profile))
            .delete(&account_id, target, intent)
            .map(|()| account_id)
            .map_err(CommandError::from)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))
    .and_then(|account| account.map(|account_id| asked_for(&app, &state, &account_id)))
}

/// Throws away everything one folder shows, for good: what a person has asked out of it stays
/// out, and what they have sent into it goes too.
///
/// One instruction per message rather than one for the folder, because that is what the server is
/// told either way and because it is the same instruction `delete_message` already queues: what
/// the carrier does with it, how a server without a trash folder is handled, what happens when one
/// of them fails -- all of it is the path that is already there. A second path for the same act
/// would be a second set of answers to those questions.
///
/// Answers with how many were asked for, which is what the window says out loud.
#[tauri::command]
pub async fn empty_folder(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    folder_id: String,
) -> Result<u32, CommandError> {
    let folder = folder_id
        .parse::<u64>()
        .ok()
        .and_then(|value| FolderId::new(value).ok())
        .ok_or_else(|| {
            CommandError::new(ErrorCode::InvalidRequest, "Folder identifier is invalid.")
        })?;

    let profile = state.storage.profile()?;
    let indexes = state.storage.index_across()?;

    let asked = tauri::async_runtime::spawn_blocking(move || {
        let account_id = account_owning(&profile, folder)?;

        let holds = crate::read_model::list::ListReadModel::spanning(indexes, profile.clone())
            .what_emptying_takes(folder)
            .map_err(CommandError::from)?;

        let acting = ActOnMail::new(adapters::operation_log(profile));
        let mut asked = 0u32;

        for message in holds {
            acting
                .delete(
                    &account_id,
                    OperationTarget::new(message, folder),
                    DeleteIntent::Permanently,
                )
                .map_err(CommandError::from)?;
            asked += 1;
        }

        Ok::<_, CommandError>((account_id, asked))
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))??;

    asked_for(&app, &state, &asked.0);

    Ok(asked.1)
}

/// Takes back the last filing of a message — the move, or the delete — that the window offers to
/// undo.
///
/// Three answers, and the window hears two of them as one. Removed: the filing was still waiting,
/// or stopped before the server heard of it. Asked for: the server had carried it out, and a way
/// back is now in the queue like any other instruction. Or nothing that can be done, which is the
/// one answered `false`: deleted for good, forgotten, no longer where it was asked about, or stopped
/// after the server was told of it.
#[tauri::command]
pub async fn undo_action(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    message_id: String,
    folder_id: String,
) -> Result<bool, CommandError> {
    let target = target_of(&message_id, &folder_id)?;
    let profile = state.storage.profile()?;
    let index = state.storage.index_holding_folder(target.folder.get())?;

    tauri::async_runtime::spawn_blocking(move || {
        let mailboxes = MailboxReadModel::new(profile.clone(), index.clone());
        let account_id = account_owning(&profile, target.folder)?;

        let Some(number) = mailboxes.account_number_of_folder(target.folder)? else {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "That folder is not on this device.",
            ));
        };

        let outcome = TakeBack::new(
            adapters::operation_log(profile),
            adapters::placements(index),
        )
        .execute(&account_id, number, target)
        .map_err(CommandError::from)?;

        Ok((outcome, account_id))
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))
    .and_then(|answer| {
        answer.map(|(outcome, account_id)| {
            // Either way the queue changed: taken out of it, or a way back written into it. Both
            // move what every view shows, and the second has to reach the server.
            if outcome != TakenBack::NotPossible {
                asked_for(&app, &state, &account_id);
            }

            outcome != TakenBack::NotPossible
        })
    })
}

/// Puts an instruction that stopped back in the queue.
///
/// What a person does with the explanation on the row. The count and the wait go back to nothing:
/// asking again is a decision, and answering a decision with an hour of backoff is answering it
/// with a timer.
#[tauri::command]
pub async fn try_action_again(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    message_id: String,
    folder_id: String,
) -> Result<(), CommandError> {
    let target = target_of(&message_id, &folder_id)?;
    let profile = state.storage.profile()?;

    tauri::async_runtime::spawn_blocking(move || {
        let account_id = account_owning(&profile, target.folder)?;

        adapters::operation_log(profile)
            .try_again(&account_id, target)
            .map(|_| account_id)
            .map_err(|error| CommandError::from(UseCaseError::from(error)))
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))
    .and_then(|account| account.map(|account_id| asked_for(&app, &state, &account_id)))
}

/// What the window hears a message put away from a notification on.
pub const PUT_AWAY_FROM_A_NOTICE: &str = "notice:put-away";

/// A message somebody put away from a notification, which the window then holds as a message it
/// filed itself: the row goes from the list, the line under the list says what became of it, and
/// the key takes it back.
#[derive(Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct PutAwayFromANotice {
    pub message_id: String,
    /// The inbox it was in, which is where the key puts it back.
    pub folder_id: FolderIdDto,
    pub what: FiledAs,
}

/// What became of it, in the window's own words for a message it filed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum FiledAs {
    /// In the trash.
    Deleted,
    /// In another folder: the junk.
    Moved,
}

/// Puts away the message a notification named, where its button said, with the window nowhere in
/// sight.
///
/// What is asked for, and when nothing is, is [`PutAway`]'s to decide. This finds the account, and
/// tells the window last, once the instruction is written.
///
/// Off the thread the system answered on, because SQLite is never touched there. Nothing tells the
/// person when nothing was put away: they are not looking at the window, and a notification about a
/// notification is noise. The journal is told, at the level of what is about one message.
pub fn put_away_from_a_notice(app: &tauri::AppHandle, message: OperationTarget, into: PutInto) {
    use crate::ports::journal::{Journal, Note};
    use tauri::{Emitter, Manager};

    let app = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };

        let not_put_away = |event: &'static str| {
            let journal = crate::infrastructure::journal::journal();
            if journal.keeps(Note::Detail) {
                journal.write_down(Note::Detail, event, &[]);
            }
        };

        let (account_id, what) = match put_away(&state, message, into) {
            Ok((account_id, PutAwayOutcome::Deleted)) => (account_id, FiledAs::Deleted),
            Ok((account_id, PutAwayOutcome::Moved)) => (account_id, FiledAs::Moved),
            Ok((_, PutAwayOutcome::NotThere)) => return not_put_away("notice.notThere"),
            Ok((_, PutAwayOutcome::NowhereToPut)) => return not_put_away("notice.nowhereToPut"),
            Err(_) => return not_put_away("notice.notPutAway"),
        };

        asked_for(&app, &state, &account_id);
        let _ = app.emit(
            PUT_AWAY_FROM_A_NOTICE,
            PutAwayFromANotice {
                message_id: message.message.get().to_string(),
                folder_id: FolderIdDto(message.folder.get().to_string()),
                what,
            },
        );
    });
}

/// What the press came to, and the account it was for.
///
/// The account first, from the profile: a press after it was removed is refused here, where asking
/// for the folder's file first would make that account's file again, empty.
fn put_away(
    state: &AppState,
    message: OperationTarget,
    into: PutInto,
) -> Result<(String, PutAwayOutcome), CommandError> {
    let profile = state.storage.profile()?;
    let account_id = account_owning(&profile, message.folder)?;
    let index = state.storage.index_holding_folder(message.folder.get())?;

    let Some(number) = MailboxReadModel::new(profile.clone(), index.clone())
        .account_number_of_folder(message.folder)?
    else {
        return Err(CommandError::new(
            ErrorCode::InvalidRequest,
            "That folder is not on this device.",
        ));
    };

    PutAway::new(
        adapters::operation_log(profile),
        adapters::placements(index),
    )
    .execute(&account_id, number, message, into)
    .map(|outcome| (account_id, outcome))
    .map_err(CommandError::from)
}

/// What the account a folder belongs to is called, or a refusal naming what is missing.
///
/// Takes the profile alone: which account owns a folder is in the folder's own number now, and what
/// that account is called is in the profile. Neither is in the mail, so no file of it is opened —
/// three of the commands below open none at all.
fn account_owning(
    profile: &crate::infrastructure::sqlite::connection::SqlitePool,
    folder: FolderId,
) -> Result<String, CommandError> {
    let number =
        crate::read_model::mailboxes::account_that_minted(folder.get()).ok_or_else(|| {
            CommandError::new(
                ErrorCode::InvalidRequest,
                "That folder is not on this device.",
            )
        })?;

    crate::read_model::mailboxes::account_named(profile, number)?.ok_or_else(|| {
        CommandError::new(
            ErrorCode::InvalidRequest,
            "That folder is not on this device.",
        )
    })
}

/// A message and a folder, as the domain names them.
fn target_of(message_id: &str, folder_id: &str) -> Result<OperationTarget, CommandError> {
    let message = message_id
        .parse::<u64>()
        .ok()
        .and_then(|value| MessageId::new(value).ok())
        .ok_or_else(|| {
            CommandError::new(ErrorCode::InvalidRequest, "Message identifier is invalid.")
        })?;

    let folder = folder_id
        .parse::<u64>()
        .ok()
        .and_then(|value| FolderId::new(value).ok())
        .ok_or_else(|| {
            CommandError::new(ErrorCode::InvalidRequest, "Folder identifier is invalid.")
        })?;

    Ok(OperationTarget::new(message, folder))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything a window can send as a message and a folder, refused or accepted here.
    ///
    /// This is the boundary: `message_id` crosses as text because a 64-bit number does not survive
    /// JSON, so what arrives is whatever the window put in a string. Nothing pinned what happens to
    /// the ones that are not numbers — the checks were there and no test would have noticed them
    /// going.
    #[test]
    fn a_message_identifier_that_is_not_one_is_refused() {
        for written in [
            "",
            " ",
            "abc",
            "1.5",
            "-1",
            "0",                    // zero is not an identifier: `MessageId` refuses it
            "18446744073709551616", // one past what a u64 holds
            "1e3",
            "0x10",
            "1 ",
            "'; DROP TABLE messages;--",
        ] {
            let refused = target_of(written, "1").expect_err("{written:?} is not an identifier");
            assert_eq!(
                refused.code,
                ErrorCode::InvalidRequest,
                "{written:?} was refused for the wrong reason"
            );
        }
    }

    #[test]
    fn a_folder_that_is_not_one_is_refused() {
        // Zero is the only value a folder identifier cannot take, and it is the one a window sends
        // when it has nothing.
        let refused = target_of("1", "0").expect_err("zero is not a folder");
        assert_eq!(refused.code, ErrorCode::InvalidRequest);

        // And a folder that is not a number at all, which a text field can now carry.
        let nonsense = target_of("1", "seven").expect_err("a word is not a folder");
        assert_eq!(nonsense.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn the_window_hears_a_message_put_away_from_a_notice_in_its_own_words() {
        // The words the window already uses of a message it filed, so what it does with the one it
        // hears of is what it does with its own.
        let heard = serde_json::to_value(PutAwayFromANotice {
            message_id: "9007199254740993".to_owned(),
            folder_id: FolderIdDto("4294967296".to_owned()),
            what: FiledAs::Deleted,
        })
        .expect("written");

        assert_eq!(
            heard,
            serde_json::json!({
                "messageId": "9007199254740993",
                "folderId": "4294967296",
                "what": "deleted",
            })
        );
        assert_eq!(
            serde_json::to_value(FiledAs::Moved).expect("written"),
            serde_json::json!("moved")
        );
    }

    #[test]
    fn what_a_window_legitimately_sends_is_accepted() {
        // Both past what a JavaScript number holds exactly, which is why each crosses as text: a
        // message identifier always was, and a folder identifier outgrows a 32-bit field the
        // moment it carries the file the folder is kept in.
        let target = target_of("9007199254740993", "4294967296").expect("a real target");

        assert_eq!(target.message.get(), 9_007_199_254_740_993);
        assert_eq!(target.folder.get(), 4_294_967_296);
    }
}
