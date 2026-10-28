use crate::commands::list::FolderIdDto;
use serde::Serialize;
use tauri::State;
use ts_rs::TS;

use crate::application::error::UseCaseError;
use crate::bootstrap::adapters;
use crate::commands::error::{CommandError, ErrorCode};
use crate::ports::account_repository::AccountRepository;
use crate::read_model::mailboxes::{AccountNode, FolderNode, MailboxReadModel};
use crate::AppState;
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::operation::may_receive;

/// What a folder is for. The interface names roles in the user's language and shows the provider's
/// own name only for folders that have none.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum FolderRoleDto {
    Custom,
    Inbox,
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
    AllMail,
    Flagged,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct FolderDto {
    /// Text, like a message identifier and for the same reason: the number carries the file the
    /// folder is kept in, which is larger than a JavaScript number holds exactly.
    pub folder_id: FolderIdDto,
    pub name: String,
    pub role: FolderRoleDto,
    /// What the folder holds on the server.
    pub total: u32,
    pub unread: u32,
    /// Messages being written from this account and kept on this device, counted on its drafts
    /// folder and nought everywhere else. The drafts folder shows these above what the server
    /// holds, so its badge is the two added together.
    pub drafts_here: u32,
    /// Whether a message may be filed here.
    ///
    /// A flag rather than the rule: an interface that decided this from the role would carry a
    /// second copy of a rule the domain already holds, and the two would part company on the day
    /// a role is added. What the interface needs to know is whether to offer the folder, and that
    /// is what it is told.
    pub can_receive: bool,
}

impl From<FolderRole> for FolderRoleDto {
    fn from(role: FolderRole) -> Self {
        match role {
            FolderRole::Custom => Self::Custom,
            FolderRole::Inbox => Self::Inbox,
            FolderRole::Sent => Self::Sent,
            FolderRole::Drafts => Self::Drafts,
            FolderRole::Trash => Self::Trash,
            FolderRole::Junk => Self::Junk,
            FolderRole::Archive => Self::Archive,
            FolderRole::AllMail => Self::AllMail,
            FolderRole::Flagged => Self::Flagged,
        }
    }
}

impl From<FolderNode> for FolderDto {
    fn from(row: FolderNode) -> Self {
        Self {
            folder_id: FolderIdDto(row.folder_id.get().to_string()),
            name: row.name,
            role: row.role.into(),
            total: row.total,
            unread: row.unread,
            drafts_here: row.drafts_here,
            can_receive: may_receive(row.role),
        }
    }
}

/// One account in the sidebar, with its folders under it.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct AccountNodeDto {
    pub account_id: String,
    pub email: String,
    pub display_name: String,
    /// Whether its folders are folded away. Remembered with the account.
    pub collapsed: bool,
    /// Where it stands: at the top of the sidebar among the groups and the other accounts in no
    /// group, or, in a group, among the accounts in that group. The sidebar draws an account in no
    /// group and a group by this one order.
    #[ts(type = "number")]
    pub place: i64,
    /// Unread across the whole account, whatever folder the mail sits in.
    pub unread: u32,
    pub folders: Vec<FolderDto>,
    /// Whether this account's mail could be read at all.
    ///
    /// False leaves the account in the sidebar with nothing under it and a reason, rather than
    /// looking like an account that simply has no folders.
    pub answered: bool,
}

impl TryFrom<AccountNode> for AccountNodeDto {
    type Error = CommandError;

    fn try_from(node: AccountNode) -> Result<Self, Self::Error> {
        Ok(Self {
            account_id: node.account_id,
            email: node.email,
            display_name: node.display_name,
            collapsed: node.collapsed,
            place: node.place,
            unread: node.unread,
            folders: node.folders.into_iter().map(FolderDto::from).collect(),
            answered: node.answered,
        })
    }
}

/// The sidebar, and how far what a person asked for had got when it was read.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct MailboxTreeDto {
    pub accounts: Vec<AccountNodeDto>,
    /// How many instructions a person had asked for when the counts were read. A change whose
    /// write answered with this number or less is in them; one that answered with more is not.
    #[ts(type = "number")]
    pub asked_through: u64,
}

/// Reads the sidebar: every account on this device and the folders under it.
#[tauri::command]
pub async fn get_mailbox_tree(state: State<'_, AppState>) -> Result<MailboxTreeDto, CommandError> {
    let profile = state.storage.profile()?;
    let index = state.storage.index_across()?;

    let (tree, asked_through) = tauri::async_runtime::spawn_blocking(move || {
        MailboxReadModel::spanning(profile, index).tree_as_of()
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))??;

    Ok(MailboxTreeDto {
        accounts: tree
            .into_iter()
            .map(AccountNodeDto::try_from)
            .collect::<Result<_, _>>()?,
        asked_through,
    })
}

/// Folds an account's folders away, or brings them back. Stored, so the sidebar opens the way it
/// was left.
#[tauri::command]
pub async fn set_account_collapsed(
    state: State<'_, AppState>,
    account_id: String,
    collapsed: bool,
) -> Result<(), CommandError> {
    let pool = state.storage.profile()?;

    tauri::async_runtime::spawn_blocking(move || {
        adapters::accounts(pool).set_collapsed(&account_id, collapsed)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
    .map_err(UseCaseError::from)?;

    Ok(())
}
