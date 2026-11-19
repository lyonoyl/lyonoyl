use tauri::State;

use crate::application::set_signature::SetSignature;
use crate::bootstrap::adapters;
use crate::commands::error::{CommandError, ErrorCode};
use crate::AppState;

/// Keeps what a mailbox puts at the foot of the messages written from it, and answers with it as
/// kept, an empty one being none.
#[tauri::command]
pub async fn set_signature(
    state: State<'_, AppState>,
    account_id: String,
    signature: String,
) -> Result<String, CommandError> {
    let pool = state.storage.profile()?;

    let kept = tauri::async_runtime::spawn_blocking(move || {
        SetSignature::new(adapters::accounts(pool)).execute(&account_id, &signature)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))??;

    Ok(kept.as_str().to_owned())
}
