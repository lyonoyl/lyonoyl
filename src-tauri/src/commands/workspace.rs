use serde::Serialize;
use tauri::State;
use ts_rs::TS;

use crate::application::workspace::{GetWorkspaceOverview, WorkspaceOverview};
use crate::bootstrap::adapters;
use crate::commands::error::{CommandError, ErrorCode};
use crate::AppState;
use lyonoyl_domain::account::MailAccountSummary;

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct MailAccountSummaryDto {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub provider: String,
    /// Whether a message can be written from it. An account is worth having for reading alone, and
    /// one nobody has given a sending server says so rather than swallowing what was written.
    pub can_send: bool,
}

impl MailAccountSummaryDto {
    pub(crate) fn of(account: &MailAccountSummary, can_send: bool) -> Self {
        Self {
            id: account.id().to_owned(),
            email: account.email().to_owned(),
            display_name: account.display_name().to_owned(),
            provider: account.provider().to_owned(),
            can_send,
        }
    }
}

#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct WorkspaceOverviewDto {
    pub accounts: Vec<MailAccountSummaryDto>,
    pub is_local_first: bool,
}

impl From<WorkspaceOverview> for WorkspaceOverviewDto {
    fn from(overview: WorkspaceOverview) -> Self {
        Self {
            accounts: overview
                .accounts
                .iter()
                .map(|account| {
                    MailAccountSummaryDto::of(
                        account,
                        overview.able_to_send.iter().any(|id| id == account.id()),
                    )
                })
                .collect(),
            is_local_first: overview.is_local_first,
        }
    }
}

/// Reads the workspace overview.
///
/// A synchronous command runs on the main thread, where SQL would freeze the window, so the
/// blocking work goes to `spawn_blocking`.
#[tauri::command]
pub async fn get_workspace_overview(
    state: State<'_, AppState>,
) -> Result<WorkspaceOverviewDto, CommandError> {
    // The first thing the window asks for, so the first moment anything in it is running. What
    // stands between the window appearing and this is the window getting itself ready, and this
    // note is the only thing that bounds it.
    crate::infrastructure::journal::the_window_asked_first_at();

    let pool = state.storage.profile()?;

    let overview = tauri::async_runtime::spawn_blocking(move || {
        GetWorkspaceOverview::new(adapters::accounts(pool)).execute()
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))??;

    Ok(overview.into())
}
