use crate::commands::list::FolderIdDto;
use serde::Serialize;
use tauri::State;
use ts_rs::TS;

use crate::application::save_attachment::SaveAttachment;
use crate::application::save_message::SaveMessage;
use crate::bootstrap::adapters;
use crate::commands::error::{CommandError, ErrorCode};
use crate::commands::list::MessageIdDto;
use crate::commands::mailboxes::FolderRoleDto;
use crate::read_model::mailboxes::MailboxReadModel;
use crate::read_model::message::{MessageDetails, MessageReadModel, Placement};
use crate::AppState;
use lyonoyl_domain::message::{BodyState, MessageId};

/// How much of a message is stored locally.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum BodyStateDto {
    HeadersOnly,
    PreviewCached,
    TextCached,
    FullCached,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct MessagePlacementDto {
    pub folder_id: FolderIdDto,
    pub folder_name: String,
    pub folder_role: FolderRoleDto,
}

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct MessageDetailsDto {
    pub message_id: MessageIdDto,
    pub subject: Option<String>,
    /// Who it is from, as the pane shows it: the name when the message gave one.
    pub sender: Option<String>,
    /// Who it is from, as an answer has to address it. Never the name beside it, which cannot be
    /// sent to; absent for a message that gave no address at all.
    pub sender_address: Option<String>,
    /// Unix seconds.
    #[ts(type = "number")]
    pub received_at: i64,
    pub size: Option<u32>,
    pub unread: bool,
    pub body_state: BodyStateDto,
    /// What an answer to this message has to carry: the name it goes under, and the conversation it
    /// said it belongs to. Absent for a message that arrived without a name of its own, which
    /// cannot be answered into a thread at all.
    pub named: Option<String>,
    pub chain: Vec<String>,
    /// The account this message arrived in, which is the one an answer to it is written from.
    ///
    /// Nothing when the index knows the message but the profile no longer knows the account —
    /// which is a message left behind by an account that has been removed.
    pub account_id: Option<String>,
    /// Whether this message named anybody besides the person reading it.
    ///
    /// A flag rather than the addresses. Answering everybody is decided below this boundary, where
    /// the person's own addresses are known, and the window needs to know only whether there is a
    /// second answer to offer — so the people on somebody else's message stay out of the one place
    /// in this application where hostile input is rendered every day.
    ///
    /// Conservative where a message reached the person by blind copy: it names one other person,
    /// so the second button is not offered even though answering everybody would reach them. The
    /// answer is still there; the shortcut to it is not.
    pub addressed_to_more: bool,
    /// Every folder the message appears in.
    pub placements: Vec<MessagePlacementDto>,
}

impl From<BodyState> for BodyStateDto {
    fn from(state: BodyState) -> Self {
        match state {
            BodyState::HeadersOnly => Self::HeadersOnly,
            BodyState::PreviewCached => Self::PreviewCached,
            BodyState::TextCached => Self::TextCached,
            BodyState::FullCached => Self::FullCached,
        }
    }
}

impl From<Placement> for MessagePlacementDto {
    fn from(placement: Placement) -> Self {
        Self {
            folder_id: FolderIdDto(placement.folder_id.get().to_string()),
            folder_name: placement.folder_name,
            folder_role: placement.folder_role.into(),
        }
    }
}

impl MessageDetailsDto {
    /// The two databases meet here.
    ///
    /// Which account owns a message is a fact the index holds as a number and the profile holds as
    /// an identifier, and nothing can join them in one query because they are two files. So the
    /// caller looks it up and hands it in, rather than this conversion reaching for a connection.
    fn of(details: MessageDetails, account_id: Option<String>) -> Result<Self, CommandError> {
        Ok(Self {
            account_id,
            addressed_to_more: details.to.len() + details.cc.len() > 1,
            message_id: MessageIdDto(details.message_id.get().to_string()),
            subject: details.subject,
            sender: details.sender,
            sender_address: details.sender_address,
            received_at: details.received_at,
            size: details.size,
            unread: details.unread,
            body_state: details.body_state.into(),
            named: details.named,
            chain: details.chain,
            placements: details
                .placements
                .into_iter()
                .map(MessagePlacementDto::from)
                .collect(),
        })
    }
}

/// The message the window opens on, and the folder it is read in.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct OpeningDto {
    pub message_id: MessageIdDto,
    pub folder_id: FolderIdDto,
}

/// The message the window opens on, if any. Nothing in every build but the one made for
/// screenshots, whose scene may name one; asked once, as the window starts.
#[tauri::command]
pub async fn opening_message() -> Result<Option<OpeningDto>, CommandError> {
    Ok(
        crate::bootstrap::opening().map(|(message, folder)| OpeningDto {
            message_id: MessageIdDto(message.get().to_string()),
            folder_id: FolderIdDto(folder.get().to_string()),
        }),
    )
}

/// Reads one message. `None` means it is no longer in the index.
#[tauri::command]
pub async fn get_message(
    state: State<'_, AppState>,
    message_id: String,
) -> Result<Option<MessageDetailsDto>, CommandError> {
    let parsed = message_id
        .parse::<u64>()
        .ok()
        .and_then(|value| MessageId::new(value).ok())
        .ok_or_else(|| {
            CommandError::new(ErrorCode::InvalidRequest, "Message identifier is invalid.")
        })?;

    let pool = state.storage.index_holding_message(parsed.get())?;
    let profile = state.storage.profile()?;
    let found = tauri::async_runtime::spawn_blocking(move || {
        let details = MessageReadModel::new(pool.clone()).get_message(parsed)?;
        let Some(details) = details else {
            return Ok::<_, crate::read_model::list::ReadModelError>(None);
        };

        // Where the message sits says which account it belongs to; the profile says what that
        // account is called. An answer is written from that account and no other.
        let account_id = match MessageReadModel::new(pool.clone()).where_to_read(parsed)? {
            Some(place) => MailboxReadModel::new(profile, pool).account_of(place.account)?,
            None => None,
        };

        Ok(Some((details, account_id)))
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))??;

    found
        .map(|(details, account_id)| MessageDetailsDto::of(details, account_id))
        .transpose()
}

/// Where a message ended up, said in the only terms the interface may hear.
///
/// One field, because there are only two outcomes and a second field could disagree with the
/// first. Nothing means the person closed the panel, which is an answer rather than a failure; a
/// name means the file is there. It is the file's name and never its path — where somebody keeps
/// their files is not something a webview has any business learning (invariant 12).
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct SavedMessageDto {
    pub name: Option<String>,
}

/// Saves one message as the file it arrived as.
///
/// No path comes in. The message is named, the system's own panel asks where it goes, and what
/// comes back is a name to show.
#[tauri::command]
pub async fn save_message(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    message_id: String,
) -> Result<SavedMessageDto, CommandError> {
    let message = message_id
        .parse::<u64>()
        .ok()
        .and_then(|value| MessageId::new(value).ok())
        .ok_or_else(|| {
            CommandError::new(ErrorCode::InvalidRequest, "Message identifier is invalid.")
        })?;

    let profile = state.storage.profile()?;
    let index = state.storage.index_holding_message(message.get())?;
    let credentials = state.credentials.sharing(state.secrets.clone());
    let runtime = tauri::async_runtime::handle().inner().clone();
    let event_loop = state.event_loop;

    // On a blocking thread on purpose: the panel holds the event loop until it is answered, and
    // the thread that waits for that answer must not be the one holding it.
    tauri::async_runtime::spawn_blocking(move || {
        let Some(place) = MessageReadModel::new(index.clone()).where_to_read(message)? else {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "That message is not in the index.",
            ));
        };

        // The subject a pass already stored, which is only a suggestion: what the file is called
        // is decided in the panel, by the person.
        let subject = MessageReadModel::new(index.clone())
            .get_message(message)?
            .and_then(|details| details.subject);

        let Some(account_id) =
            MailboxReadModel::new(profile.clone(), index).account_of(place.account)?
        else {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "That message belongs to an account that is not here.",
            ));
        };

        let saved = SaveMessage::new(
            adapters::accounts(profile),
            &credentials,
            crate::bootstrap::mail_servers(runtime),
            adapters::save_dialog(app, event_loop),
        )
        .execute(
            &account_id,
            &place.mailbox,
            place.uid_validity,
            place.uid,
            subject.as_deref(),
        )
        .map_err(CommandError::from)?;

        Ok(SavedMessageDto {
            name: saved.map(|file| file.name),
        })
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
}

/// Saves one file that came with a message.
///
/// Asked by naming the message and where the file sits in it. Never a path: where it goes is
/// decided by the person in the system's own dialog, and a path from the window is a path an
/// attacker who reaches the window can choose (invariant 12).
#[tauri::command]
pub async fn save_attachment(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    message_id: String,
    at: u32,
) -> Result<SavedMessageDto, CommandError> {
    let message = message_id
        .parse::<u64>()
        .ok()
        .and_then(|value| MessageId::new(value).ok())
        .ok_or_else(|| {
            CommandError::new(ErrorCode::InvalidRequest, "Message identifier is invalid.")
        })?;

    let profile = state.storage.profile()?;
    let index = state.storage.index_holding_message(message.get())?;
    let credentials = state.credentials.sharing(state.secrets.clone());
    let runtime = tauri::async_runtime::handle().inner().clone();
    let event_loop = state.event_loop;

    tauri::async_runtime::spawn_blocking(move || {
        let Some(place) = MessageReadModel::new(index.clone()).where_to_read(message)? else {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "That message is not in the index.",
            ));
        };

        let Some(account_id) =
            MailboxReadModel::new(profile.clone(), index).account_of(place.account)?
        else {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "That message belongs to an account that is not here.",
            ));
        };

        let saved = SaveAttachment::new(
            adapters::accounts(profile),
            &credentials,
            crate::bootstrap::mail_servers(runtime),
            adapters::save_dialog(app, event_loop),
            crate::infrastructure::html::MimeParts,
        )
        .execute(
            &account_id,
            &place.mailbox,
            place.uid_validity,
            place.uid,
            at,
        )
        .map_err(CommandError::from)?;

        Ok(SavedMessageDto {
            name: saved.map(|file| file.name),
        })
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read_model::message::MessageDetails;
    use lyonoyl_domain::message::MessageId;

    fn details(to: &[&str], cc: &[&str]) -> MessageDetails {
        MessageDetails {
            message_id: MessageId::new(1).expect("non-zero"),
            subject: Some("About Tuesday".to_owned()),
            sender: Some("Zeta Quorin".to_owned()),
            sender_address: Some("zeta@example.com".to_owned()),
            to: to.iter().map(|value| (*value).to_owned()).collect(),
            cc: cc.iter().map(|value| (*value).to_owned()).collect(),
            received_at: 1_700_000_000,
            size: Some(1024),
            unread: false,
            body_state: BodyState::HeadersOnly,
            named: None,
            chain: Vec::new(),
            placements: Vec::new(),
        }
    }

    fn asked(to: &[&str], cc: &[&str]) -> MessageDetailsDto {
        MessageDetailsDto::of(details(to, cc), Some("acc-1".to_owned()))
            .expect("a readable message")
    }

    #[test]
    fn a_message_naming_only_the_person_reading_it_has_one_answer() {
        // Answering everybody would be the same message, and offering it as a second thing to do
        // invites the one that says nothing extra.
        assert!(!asked(&["you@example.com"], &[]).addressed_to_more);
    }

    #[test]
    fn a_message_with_a_room_on_it_has_two() {
        assert!(asked(&["you@example.com", "kappa@example.net"], &[]).addressed_to_more);
        assert!(asked(&["you@example.com"], &["sigma@example.org"]).addressed_to_more);
    }

    #[test]
    fn the_addresses_themselves_do_not_cross_the_boundary() {
        // A flag, not a list. Who else somebody wrote to is not something the window that renders
        // hostile input every day has any business holding, and the second answer is decided in
        // Rust where the person's own addresses are known.
        let dto = asked(
            &["you@example.com", "kappa@example.net"],
            &["sigma@example.org"],
        );
        let written = serde_json::to_string(&dto).expect("the DTO serializes");

        assert!(
            !written.contains("kappa@example.net"),
            "a recipient crossed IPC: {written}"
        );
        assert!(
            !written.contains("sigma@example.org"),
            "a recipient crossed IPC: {written}"
        );
    }
}
