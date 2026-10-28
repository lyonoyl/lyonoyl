//! Where this device keeps its secrets, and opening it when that is a file.
//!
//! Only a Linux desktop whose first start found no credential store has anything to open.
//! Everywhere else these commands answer `System` and refuse the rest: there is no master password
//! to ask for, and a screen that asked for one would be inventing a step.

use secrecy::SecretString;
use serde::Serialize;
use tauri::{AppHandle, State};
use ts_rs::TS;

use crate::commands::error::{CommandError, ErrorCode};
use crate::infrastructure::secure_storage::master_password::SecretStoreState;
use crate::infrastructure::secure_storage::vault::VaultError;
use crate::AppState;

/// What the interface has to know before anything opens: whether there is a file of secrets, and
/// whether it is open.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum SecretStoreStateDto {
    /// The system holds the secrets. Nothing to ask anyone.
    System,
    /// No system store and no file yet: a master password has to be chosen.
    NeedsChoosing,
    /// There is a file, closed until the master password is given.
    NeedsUnlocking,
    /// Open for this run.
    Unlocked,
}

impl From<SecretStoreState> for SecretStoreStateDto {
    fn from(state: SecretStoreState) -> Self {
        match state {
            SecretStoreState::System => Self::System,
            SecretStoreState::NeedsChoosing => Self::NeedsChoosing,
            SecretStoreState::NeedsUnlocking => Self::NeedsUnlocking,
            SecretStoreState::Unlocked => Self::Unlocked,
        }
    }
}

#[tauri::command]
pub fn get_secret_store_state(state: State<'_, AppState>) -> SecretStoreStateDto {
    state.secrets.state().into()
}

/// Chooses the master password for a device that has no store of its own, and makes the file.
#[tauri::command]
pub async fn choose_master_password(
    app: AppHandle,
    state: State<'_, AppState>,
    password: String,
) -> Result<(), CommandError> {
    with_the_file(app, &state, password, |store, master| store.choose(master)).await
}

/// Opens the file for this run.
#[tauri::command]
pub async fn unlock_secrets(
    app: AppHandle,
    state: State<'_, AppState>,
    password: String,
) -> Result<(), CommandError> {
    with_the_file(app, &state, password, |store, master| store.unlock(master)).await
}

/// The part both share: there has to be a file to open, and the derivation is slow on purpose, so
/// it does not run on the thread that draws.
///
/// The file is also where the key the databases are written under is kept, so opening it is the
/// moment they can be opened too, and it is done here before the window is answered: the window
/// starts again on the answer and reads them.
async fn with_the_file(
    app: AppHandle,
    state: &State<'_, AppState>,
    password: String,
    act: fn(
        &crate::infrastructure::secure_storage::master_password::MasterPasswordStore,
        &SecretString,
    ) -> Result<(), VaultError>,
) -> Result<(), CommandError> {
    let store = state.secrets.clone();
    let master = SecretString::from(password);

    tauri::async_runtime::spawn_blocking(move || {
        let Some(file) = store.master_password() else {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "This device keeps its secrets in the system store.",
            ));
        };

        act(file, &master).map_err(from_vault)?;
        crate::bootstrap::open_what_waited_for_its_key(&app);
        Ok(())
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
}

fn from_vault(error: VaultError) -> CommandError {
    match error {
        VaultError::WrongPassword => CommandError::new(
            ErrorCode::MasterPasswordRejected,
            "That password did not open the secrets file.",
        ),
        VaultError::Damaged => CommandError::new(
            ErrorCode::StorageCorrupted,
            "The secrets file could not be read.",
        ),
        VaultError::Unavailable => CommandError::new(
            ErrorCode::StorageUnavailable,
            "The secrets file could not be reached.",
        ),
        // The interface does not offer choosing when there is a file, so this is a caller bug.
        VaultError::AlreadyThere => CommandError::new(
            ErrorCode::InvalidRequest,
            "A secrets file is already there.",
        ),
    }
}
