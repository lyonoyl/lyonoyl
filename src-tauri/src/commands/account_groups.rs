//! The sidebar's arrangement, across the boundary.
//!
//! Transport only: unpack the state, hand the strings to the use case, map the error. The rule
//! about what a name may be, and the difference between a group that is gone and an account that
//! is gone, are both decided one layer in.

use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;

use crate::application::arrange_accounts::ArrangeAccounts;
use crate::bootstrap::adapters;
use crate::commands::error::CommandError;
use crate::AppState;
use lyonoyl_domain::account_group::Placed;

/// One group as the sidebar draws it.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct AccountGroupDto {
    pub id: String,
    pub name: String,
    /// Where it stands at the top of the sidebar, in one order with the accounts in no group.
    #[ts(type = "number")]
    pub place: i64,
    /// Folded away or open, as the person last left it.
    pub collapsed: bool,
    /// The accounts in it, in the order they are drawn. Identifiers rather than whole accounts:
    /// the sidebar already holds the accounts, and sending them twice would give it two versions
    /// of the same unread count.
    pub accounts: Vec<String>,
}

#[tauri::command]
pub async fn list_account_groups(
    state: State<'_, AppState>,
) -> Result<Vec<AccountGroupDto>, CommandError> {
    let pool = state.storage.profile()?;

    let groups = tauri::async_runtime::spawn_blocking(move || {
        ArrangeAccounts::new(adapters::account_groups(pool)).list()
    })
    .await
    .map_err(background_task_failed)??;

    Ok(groups
        .into_iter()
        .map(|group| AccountGroupDto {
            id: group.id,
            name: group.name.as_str().to_owned(),
            place: group.position,
            collapsed: group.collapsed,
            accounts: group.accounts,
        })
        .collect())
}

#[tauri::command]
pub async fn create_account_group(
    state: State<'_, AppState>,
    name: String,
) -> Result<String, CommandError> {
    let pool = state.storage.profile()?;

    Ok(tauri::async_runtime::spawn_blocking(move || {
        ArrangeAccounts::new(adapters::account_groups(pool)).create(&name)
    })
    .await
    .map_err(background_task_failed)??)
}

#[tauri::command]
pub async fn rename_account_group(
    state: State<'_, AppState>,
    group_id: String,
    name: String,
) -> Result<(), CommandError> {
    let pool = state.storage.profile()?;

    tauri::async_runtime::spawn_blocking(move || {
        ArrangeAccounts::new(adapters::account_groups(pool)).rename(&group_id, &name)
    })
    .await
    .map_err(background_task_failed)??;

    Ok(())
}

#[tauri::command]
pub async fn remove_account_group(
    state: State<'_, AppState>,
    group_id: String,
) -> Result<(), CommandError> {
    let pool = state.storage.profile()?;

    tauri::async_runtime::spawn_blocking(move || {
        ArrangeAccounts::new(adapters::account_groups(pool)).remove(&group_id)
    })
    .await
    .map_err(background_task_failed)??;

    Ok(())
}

/// Puts an account in a group, or — with no group — at the top of the sidebar, directly below the
/// group it was in.
#[tauri::command]
pub async fn place_account_in_group(
    state: State<'_, AppState>,
    account_id: String,
    group_id: Option<String>,
) -> Result<(), CommandError> {
    let pool = state.storage.profile()?;

    tauri::async_runtime::spawn_blocking(move || {
        ArrangeAccounts::new(adapters::account_groups(pool)).place(&account_id, group_id.as_deref())
    })
    .await
    .map_err(background_task_failed)??;

    Ok(())
}

/// Writes down the order accounts were dragged into.
///
/// The whole run in the order it should be drawn, rather than one account and a place: what a
/// place means depends on every other account, and two windows dragging at once would each be
/// right about a list neither of them has.
#[tauri::command]
pub async fn reorder_accounts(
    state: State<'_, AppState>,
    account_ids: Vec<String>,
) -> Result<(), CommandError> {
    let pool = state.storage.profile()?;

    tauri::async_runtime::spawn_blocking(move || {
        ArrangeAccounts::new(adapters::account_groups(pool)).reorder(&account_ids)
    })
    .await
    .map_err(background_task_failed)??;

    Ok(())
}

/// One thing at the top of the sidebar, as the window names it in an order it writes down.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum PlacedDto {
    Group { id: String },
    Account { id: String },
}

impl From<PlacedDto> for Placed {
    fn from(placed: PlacedDto) -> Self {
        match placed {
            PlacedDto::Group { id } => Self::Group(id),
            PlacedDto::Account { id } => Self::Account(id),
        }
    }
}

/// Writes down the order of everything at the top of the sidebar: the groups and the accounts in
/// no group, as one run, the way `reorder_accounts` writes down a run inside a group.
#[tauri::command]
pub async fn arrange_sidebar(
    state: State<'_, AppState>,
    order: Vec<PlacedDto>,
) -> Result<(), CommandError> {
    let pool = state.storage.profile()?;
    let order: Vec<Placed> = order.into_iter().map(Placed::from).collect();

    tauri::async_runtime::spawn_blocking(move || {
        ArrangeAccounts::new(adapters::account_groups(pool)).arrange(&order)
    })
    .await
    .map_err(background_task_failed)??;

    Ok(())
}

/// Folds a group away, or opens it again.
#[tauri::command]
pub async fn set_account_group_collapsed(
    state: State<'_, AppState>,
    group_id: String,
    collapsed: bool,
) -> Result<(), CommandError> {
    let pool = state.storage.profile()?;

    tauri::async_runtime::spawn_blocking(move || {
        ArrangeAccounts::new(adapters::account_groups(pool)).set_collapsed(&group_id, collapsed)
    })
    .await
    .map_err(background_task_failed)??;

    Ok(())
}

fn background_task_failed(_error: tauri::Error) -> CommandError {
    CommandError::new(
        crate::commands::error::ErrorCode::Internal,
        "A background task did not finish.",
    )
}
