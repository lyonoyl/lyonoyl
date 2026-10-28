//! Getting the application open again when its own files will not open.
//!
//! Refusing to destroy what a person wrote is right, and it is what happens when the profile
//! cannot be read: nothing is removed and nothing is guessed at. Leaving them with a window that
//! says so and offers nothing is a separate thing, and a worse one — the application becomes a
//! notice about itself.
//!
//! So this is the way out, and it is a person's decision rather than the application's: the files
//! that will not open are moved aside, keeping every byte, and the application starts as it does
//! on a machine it has never run on. What was in them is still there to be looked at, and where
//! it went is said out loud.
//!
//! Not for a profile a newer build wrote. It is whole, and that build reads it: moved, it would
//! leave that build a fresh, empty profile to find. The screen offers nothing for it, and this
//! refuses it as well.

use serde::Serialize;
use tauri::{AppHandle, State};
use ts_rs::TS;

use crate::commands::error::{CommandError, ErrorCode};
use crate::AppState;

/// Where the old files went.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct SetAsideDto {
    /// One path per file moved, so the person can find them. Empty when there was nothing to move,
    /// which is itself worth knowing: the trouble was not the files.
    pub moved: Vec<String>,
}

/// Moves aside the databases that will not open, and answers where they went. A profile a newer
/// build wrote is refused rather than moved, with everything beside it.
///
/// Answers rather than restarts. Restarting is a second press, `restart_after_setting_aside`, and
/// the two are apart for a plain reason: if the move fails there is something to say, and a window
/// that has already gone cannot say it. So this one reports, the window shows where the files
/// went, and then it asks for the restart itself.
#[tauri::command]
pub async fn set_aside_local_storage(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<SetAsideDto, CommandError> {
    state.storage.may_be_set_aside()?;
    // Before anything moves, and for good: the application restarts after this, and the message
    // being written that it would keep on the way out would go into the files set aside.
    state.being_written.set_aside();

    let directory = crate::bootstrap::data_directory(&app)
        .map(|data| crate::bootstrap::databases_of(&data))
        .map_err(|_| {
            CommandError::new(
                ErrorCode::Internal,
                "This device has nowhere for the application to keep its files.",
            )
        })?;

    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default();

    let moved = crate::bootstrap::set_aside_what_will_not_open(&directory, at)
        .map_err(|reason| CommandError::new(ErrorCode::StorageUnavailable, &reason))?;

    Ok(SetAsideDto { moved })
}

/// Starts the application again, now that what would not open is out of the way.
///
/// Its own command rather than the tail of the one above: that one can fail, and a person whose
/// files could not be moved needs to read why — which they cannot do through a window that has
/// already been replaced.
///
/// Nothing comes back, because nothing here returns: the process is replaced.
#[tauri::command]
pub async fn restart_after_setting_aside(app: AppHandle) -> Result<(), CommandError> {
    app.restart()
}
