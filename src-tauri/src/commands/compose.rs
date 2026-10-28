//! Writing a message and handing it to the outbox.
//!
//! Transport only: what an address is, what makes a message sendable and what it will be called
//! are decided below this file, and are the same wherever they are asked.

use tauri::State;

use serde::Deserialize;
use ts_rs::TS;

use crate::application::answer_message::{AnswerMessage, Answered};
use crate::application::keep_at_the_end::Note;
use crate::application::send_message::SendMessage;
use crate::bootstrap::adapters;
use crate::commands::error::{CommandError, ErrorCode};
use crate::commands::now::asked_for;
use crate::infrastructure::attaching::FilesBeingAttached;
use crate::infrastructure::secure_storage::message_key::MessageKey;
use crate::ports::drafts::{Draft, DraftFile, Drafts};
use crate::read_model::message::MessageReadModel;
use crate::AppState;
use lyonoyl_domain::message::MessageId;
use lyonoyl_domain::outgoing::{EmailAddress, InThread, Outgoing, Subject};

/// A message as the composer has it: text, and nothing decided yet.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct OutgoingDto {
    pub from: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
    /// The message this answers, by the name it carries. Nothing for a message answering nothing.
    pub answering: Option<String>,
    /// The conversation it belongs to, oldest first.
    pub chain: Vec<String>,
    /// The files going with it, by the tokens `attach_file` answered with.
    ///
    /// Tokens rather than bytes: the file was chosen in the core and never left it. A photograph
    /// crossing the boundary twice — out to be listed and back to be sent — is a copy of somebody's
    /// file in a webview for no reason at all.
    #[serde(default)]
    pub files: Vec<String>,
    /// Which composer it comes from, by the name its keeps and notes carry.
    ///
    /// What lets the outbox take the message in turn with the end of the application, and let go
    /// of that composer's note in the same step: an end coming as it is handed over finds it in
    /// the outbox or writes it down as a draft, and never neither.
    pub writing: String,
    /// The draft it was kept as, when it was kept at all.
    ///
    /// Thrown away in the same step as the outbox takes the message. Thrown away by the window once
    /// it heard the message had gone, it would outlive an end of the application coming in
    /// between, an older copy of the message in the drafts for somebody to send again.
    pub kept: Option<u32>,
}

/// A file chosen for a message being written.
#[derive(Debug, Clone, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct AttachingDto {
    /// What the message sends back to attach it, and to change its mind.
    pub token: String,
    /// Already made safe: it is about to be written into a message header.
    pub name: String,
    /// Bytes, as text, for the same reason every other size here is.
    pub bytes: String,
}

/// What the window is told about a file it will never see.
///
/// A name and a count. Its own function because that is the line the whole design rests on, and
/// inside the dialog call it was a line no test could reach: a build that put the file itself in
/// `bytes` — the field is already text — would hand a webview a copy of somebody's document,
/// change no type, regenerate no binding, and break nothing that anybody was watching.
fn told_about(token: String, name: &str, size: usize) -> AttachingDto {
    AttachingDto {
        token,
        name: name.to_owned(),
        bytes: size.to_string(),
    }
}

/// Asks which file to attach, reads it, and keeps it until the message is sent.
///
/// The window never names a file and never sees one. It says "let me attach something", the
/// system's own panel asks, and what comes back is a name and a size. `beside` is the files already
/// on screen, by their tokens: one that would make them heavier than a message may be is refused.
#[tauri::command]
pub async fn attach_file(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    beside: Vec<String>,
) -> Result<Option<AttachingDto>, CommandError> {
    use crate::ports::file_import::{FileImport, FileImportError};

    let event_loop = state.event_loop;
    let attaching = state.attaching.clone();

    // On a blocking thread: the panel holds the event loop until it is answered, and the thread
    // that waits for that answer must not be the one holding it.
    tauri::async_runtime::spawn_blocking(move || {
        let chosen = adapters::open_dialog(app, event_loop)
            .choose()
            .map_err(|why| match why {
                FileImportError::CannotAsk => CommandError::new(
                    ErrorCode::NoPlaceToSave,
                    "There is no way to ask which file to attach.",
                ),
                FileImportError::TooLarge => {
                    CommandError::new(ErrorCode::FileTooLarge, "That file is too large to attach.")
                }
                FileImportError::NotRead => {
                    CommandError::new(ErrorCode::FileNotRead, "That file could not be read.")
                }
            })?;

        let Some(chosen) = chosen else {
            return Ok(None);
        };

        held_beside(chosen, &beside, &attaching, a_fresh_token()).map(Some)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
}

/// A file chosen for a message, held beside the files already on screen, and what the window is
/// told about it.
///
/// Refused, and nothing held, when it would make them heavier than one message may be.
fn held_beside(
    chosen: crate::ports::file_import::ChosenFile,
    beside: &[String],
    attaching: &FilesBeingAttached,
    token: String,
) -> Result<AttachingDto, CommandError> {
    let told = told_about(token.clone(), &chosen.name, chosen.bytes.len());
    let file = lyonoyl_domain::outgoing::OutgoingFile {
        // Guessed from the name, never asked of the contents: what a file *is* is not a question
        // this application answers, and a type read out of the bytes would be this application
        // telling the recipient what to open it with.
        content_type: kind_of(&chosen.name),
        name: chosen.name,
        bytes: chosen.bytes,
    };

    attaching
        .hold_beside(token, file, beside)
        .then_some(told)
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::FileTooLarge,
                "That file would make the message heavier than a message may be.",
            )
        })
}

/// Forgets a file somebody attached and changed their mind about, or one a composer that closed
/// was holding.
#[tauri::command]
pub async fn forget_attachment(
    state: State<'_, AppState>,
    token: String,
) -> Result<(), CommandError> {
    state.attaching.forget(&token);
    Ok(())
}

/// What a file is known by to the window: random, and nothing else.
fn a_fresh_token() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// What to label a file as, from its name alone.
///
/// A short list and a catch-all. Guessing from the extension is what every mail client does, and
/// the alternative — reading the bytes — would be this application deciding what somebody's file
/// is, which is both harder and none of its business. A recipient's own system decides what to
/// open it with either way.
fn kind_of(name: &str) -> String {
    let extension = name
        .rsplit_once('.')
        .map(|(_, tail)| tail.to_ascii_lowercase())
        .unwrap_or_default();

    match extension.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "txt" | "log" | "md" => "text/plain",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "json" => "application/json",
        "zip" => "application/zip",
        // A message kept as a file, which the other end opens as a message rather than as bytes.
        "eml" => "message/rfc822",
        "doc" | "docx" => "application/msword",
        "xls" | "xlsx" => "application/vnd.ms-excel",
        _ => "application/octet-stream",
    }
    .to_owned()
}

/// Puts a message in the outbox. Answers with the name it will carry.
///
/// Nothing is sent here. The message is committed first — this is the one thing on the device that
/// is not a copy of something a server holds — and handed over afterwards. Committed in turn with
/// the end of the application, in a step that also lets go of the note of the composer it comes
/// from and throws away the draft it was kept as.
#[tauri::command]
pub async fn send_message(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    message: OutgoingDto,
) -> Result<String, CommandError> {
    let outgoing = with_its_files(&message, &state.attaching)?;

    // Given back if the message is refused before it is written down: the window still shows the
    // files, and sending again has to find them where they were.
    let profile = match state.storage.profile() {
        Ok(profile) => profile,
        Err(why) => {
            state.attaching.give_back(&message.files, outgoing.files);
            return Err(why);
        }
    };

    // The part of the name nothing can derive. Made here, where there is a source of randomness,
    // rather than in a use case that has to answer the same way every time it is asked.
    let unique = uuid::Uuid::new_v4().simple().to_string();

    let told_about = account_id.clone();
    let being_written = state.being_written.clone();
    let writing = message.writing.clone();
    let kept = message.kept.map(i64::from);

    let (sent, outgoing) = tauri::async_runtime::spawn_blocking(move || {
        let sent = being_written
            .hand_over(adapters::drafts(profile.clone()), &writing, kept, || {
                SendMessage::new(
                    adapters::outbox(profile.clone()),
                    adapters::accounts(profile),
                )
                .execute(&account_id, &outgoing, &unique)
            })
            .map_err(CommandError::from);
        (sent, outgoing)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?;

    let named = match sent {
        Ok(named) => named,
        Err(why) => {
            state.attaching.give_back(&message.files, outgoing.files);
            return Err(why);
        }
    };

    // Handed over by the push that follows, rather than by the next round: a person who pressed
    // send is not kept waiting for a timer to have something to look at.
    asked_for(&app, &state, &told_about);

    Ok(named)
}

/// What an answer to a message starts as: who it goes to, and the message quoted back.
#[derive(Debug, Clone, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct AnswerDto {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    /// The message quoted, every line marked, cut at a line where it is too long.
    ///
    /// Empty when this device does not hold the message or it carries no text part. The line above
    /// a quotation — who wrote it and when — is not here: it is prose a person reads, so it is
    /// written in the language they are reading in, which is the interface's business and not this
    /// one's.
    pub quoted: String,
}

/// Who an answer to this message should be addressed to.
///
/// Asked here rather than worked out in the window, because the answer depends on which addresses
/// belong to the person — every one of them, not the account they happen to be writing from — and
/// that is one rule with one place to live. `everybody` is the difference between answering the
/// person who wrote and answering the room.
#[tauri::command]
pub async fn answer_audience(
    state: State<'_, AppState>,
    message_id: String,
    everybody: bool,
) -> Result<AnswerDto, CommandError> {
    let message = message_id
        .parse::<u64>()
        .ok()
        .and_then(|value| MessageId::new(value).ok())
        .ok_or_else(|| {
            CommandError::new(ErrorCode::InvalidRequest, "Message identifier is invalid.")
        })?;

    let index = state.storage.index_holding_message(message.get())?;
    let profile = state.storage.profile()?;
    // The key the mail on this disk is kept under. Read here rather than inside the store so a
    // device whose secret store cannot be reached says so instead of answering with an empty
    // quotation, which would look like a message that had none.
    let sealing = adapters::message_key(&state.secrets).map_err(|_| {
        CommandError::new(
            ErrorCode::CredentialsUnavailable,
            "The key this device keeps its mail under cannot be reached.",
        )
    })?;
    let held = index.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let Some(details) = MessageReadModel::new(index).get_message(message)? else {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "That message is no longer in the local index.",
            ));
        };

        let answered = Answered {
            from: details.sender_address,
            to: details.to,
            cc: details.cc,
        };

        let audience = AnswerMessage::new(adapters::accounts(profile.clone()))
            .execute(&answered, everybody)?;

        // Quoted from what this device already holds, and never fetched for it. Somebody
        // answering a message has just read it, so it is here; a message that is not is one whose
        // answer starts empty, which they can see. Asking a server at this moment would put a wait
        // — and a failure — between pressing reply and a window opening.
        let quoted = quotation(held, message, sealing);

        Ok(AnswerDto {
            to: audience.to,
            cc: audience.cc,
            quoted,
        })
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
}

/// The message this device holds, written back as a quotation.
///
/// Empty for a message this device does not hold, and for one that carries no text part: both are
/// answers that start empty, and a person can see that. Nothing here reaches a server.
fn quotation(
    index: crate::infrastructure::sqlite::connection::SqlitePool,
    message: MessageId,
    sealing: MessageKey,
) -> String {
    use crate::ports::message_bodies::MessageBodies;

    adapters::message_bodies(index, sealing)
        .held(message)
        .ok()
        .flatten()
        .and_then(|raw| crate::infrastructure::html::text_of(&raw))
        .map(|text| lyonoyl_domain::outgoing::quoted(&text))
        .unwrap_or_default()
}

/// What a message passed on starts as: who it was for, what it says, and the files it carries,
/// already held here for the composer that opens with them.
#[derive(Debug, Clone, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct ForwardDto {
    /// Who the message was addressed to, for the lines above it that say so: the people a message
    /// passed on was for are part of what it was. Not who it goes to now, which is the person's to
    /// choose.
    pub to: Vec<String>,
    pub cc: Vec<String>,
    /// What it says, as text, with its lines as it wrote them. Empty when it says nothing this
    /// build reads as text.
    pub text: String,
    /// The files it carries, in the order it carries them, each held under a token of its own as
    /// a file attached a moment ago is: listed, taken off and sent the same way.
    pub files: Vec<AttachingDto>,
    /// The names of the files it carries that would make it heavier than a message may be beside
    /// the ones before them, which it goes without.
    pub left_behind: Vec<String>,
}

/// What passing a message on starts with.
///
/// Read from the message itself, from this device or else from its server, and never from what
/// the reader drew of it. The files are held here before the window hears of them, so what crosses
/// is a name and a size; the window lets go of each one when the composer it gave them to goes, or
/// at once if it has gone already.
///
/// `whole` passes the message on as one file — its exact copy, with everything it carries — rather
/// than as its text and its files: then nothing else is read out of it, and nothing of it but its
/// name and size crosses.
#[tauri::command]
pub async fn forward_message(
    state: State<'_, AppState>,
    message_id: String,
    whole: bool,
) -> Result<ForwardDto, CommandError> {
    use crate::application::forward_message::ForwardMessage;
    use crate::infrastructure::html::{too_large_to_read, MimeParts};
    use crate::read_model::mailboxes::MailboxReadModel;

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
    let sealing = adapters::message_key(&state.secrets).map_err(|_| {
        CommandError::new(
            ErrorCode::CredentialsUnavailable,
            "The key this device keeps its mail under cannot be reached.",
        )
    })?;
    let runtime = tauri::async_runtime::handle().inner().clone();
    let attaching = state.attaching.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let read = MessageReadModel::new(index.clone());
        let (Some(place), Some(details)) =
            (read.where_to_read(message)?, read.get_message(message)?)
        else {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "That message is no longer in the local index.",
            ));
        };

        // Refused before it is fetched rather than after it is in memory, as opening it is.
        if too_large_to_read(place.size) {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "That message is larger than this build reads.",
            ));
        }

        let Some(account_id) =
            MailboxReadModel::new(profile.clone(), index.clone()).account_of(place.account)?
        else {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "That message belongs to an account that is not here.",
            ));
        };

        let forwarding = ForwardMessage::new(
            adapters::accounts(profile),
            &credentials,
            crate::bootstrap::mail_servers(runtime),
            adapters::message_bodies(index, sealing),
            MimeParts,
        );

        if whole {
            let itself = forwarding
                .whole(
                    &account_id,
                    message,
                    &place.mailbox,
                    place.uid_validity,
                    place.uid,
                    details.subject.as_deref(),
                )
                .map_err(CommandError::from)?;
            let (files, left_behind) = held_to_pass_on(vec![itself], &attaching, a_fresh_token);

            return Ok(ForwardDto {
                to: Vec::new(),
                cc: Vec::new(),
                text: String::new(),
                files,
                left_behind,
            });
        }

        let passed = forwarding
            .execute(
                &account_id,
                message,
                &place.mailbox,
                place.uid_validity,
                place.uid,
            )
            .map_err(CommandError::from)?;

        let (files, left_behind) = held_to_pass_on(passed.files, &attaching, a_fresh_token);

        Ok(ForwardDto {
            to: details.to,
            cc: details.cc,
            text: passed.text,
            files,
            left_behind,
        })
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
}

/// The files of a message passed on, held under fresh tokens in the order it carries them, and the
/// names of those left behind.
///
/// Each is held beside the ones held before it, by the rule a file chosen from the disk is: one
/// that would make them heavier than a message may be is left behind, and a smaller one after it
/// still goes. Held all the same, it would be a message that could never be sent and a draft no
/// keep could write down, with everything typed into it.
fn held_to_pass_on(
    files: Vec<(String, Vec<u8>)>,
    attaching: &FilesBeingAttached,
    mut fresh: impl FnMut() -> String,
) -> (Vec<AttachingDto>, Vec<String>) {
    let mut held: Vec<AttachingDto> = Vec::new();
    let mut left_behind = Vec::new();

    for (name, bytes) in files {
        let token = fresh();
        let told = told_about(token.clone(), &name, bytes.len());
        let beside: Vec<String> = held.iter().map(|one| one.token.clone()).collect();
        let file = lyonoyl_domain::outgoing::OutgoingFile {
            // Guessed from the name, as a file chosen from the disk is: what the message said a
            // file was is written by whoever sent it, and goes no further than here.
            content_type: kind_of(&name),
            name: name.clone(),
            bytes,
        };

        if attaching.hold_beside(token, file, &beside) {
            held.push(told);
        } else {
            left_behind.push(name);
        }
    }

    (held, left_behind)
}

/// A message being written, as the window has it.
///
/// Nothing here is parsed. What a person has typed so far is allowed to be half an address and no
/// subject at all, and refusing to keep that would lose it at the moment they most need it kept.
/// The checking happens when they press send.
#[derive(Debug, Clone, serde::Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct DraftDto {
    pub recipients: String,
    pub copies: String,
    pub blind: String,
    pub subject: String,
    pub body: String,
    pub answering: Option<String>,
    pub chain: Vec<String>,
    /// Every file attached to it at this moment, by its token, in order.
    ///
    /// Everything it carries: a file the draft holds and this does not name leaves it. Required
    /// for that reason, so a window that left it out is refused rather than read as having taken
    /// every file off.
    pub files: Vec<String>,
    /// Which composer it comes from: a name the window makes at random as the composer opens, the
    /// same in every keep and note that composer sends.
    ///
    /// What ties a note to the keeps of the same message, so the end of the application writes
    /// that message down once, and lets go of one composer's note without touching another's.
    pub writing: String,
}

/// One that was kept, and the number to keep it under next time.
#[derive(Debug, Clone, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct KeptDraftDto {
    /// A row identifier, small enough for a JavaScript number — the same choice folders make.
    pub id: u32,
    pub recipients: String,
    pub copies: String,
    pub blind: String,
    pub subject: String,
    pub body: String,
    pub answering: Option<String>,
    pub chain: Vec<String>,
}

/// A message sent from this device, which the folder for sent mail shows from here until the
/// account has been read back with its copy in it.
#[derive(Debug, Clone, serde::Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct SentHereDto {
    /// A row identifier, small enough for a JavaScript number — the same choice drafts make.
    pub id: u32,
    pub sender: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
    /// The opening of the text, as a row of the list shows it.
    pub preview: Option<String>,
    /// The names of the files it carries, in the order they were attached.
    pub files: Vec<String>,
    /// Unix seconds: when it was written, which is the date it goes out with. A number rather than
    /// a big integer, as a row's date is.
    #[ts(type = "number")]
    pub written_at: i64,
    /// Whether no server has taken it yet.
    pub on_its_way: bool,
}

/// Writes down what is in the window, under the number it was written down as before, with the
/// files attached to it.
///
/// Answers that number, so the next keep replaces this one rather than leaving a row behind every
/// keystroke. A file is written into the draft once and its token is not spent: sending takes the
/// file from where it waits, and the keeps that follow name it at no cost.
#[tauri::command]
pub async fn keep_draft(
    state: State<'_, AppState>,
    account_id: String,
    under: Option<u32>,
    draft: DraftDto,
) -> Result<u32, CommandError> {
    let profile = state.storage.profile()?;
    let files = to_keep(&draft.files, &state.attaching)?;
    let being_written = state.being_written.clone();
    let (writing, draft) = written(draft);

    tauri::async_runtime::spawn_blocking(move || {
        being_written
            .keep(
                adapters::drafts(profile),
                &writing,
                &account_id,
                under.map(i64::from),
                draft,
                &files,
            )
            .map_err(CommandError::from)
            .and_then(|id| {
                u32::try_from(id).map_err(|_| {
                    CommandError::new(
                        ErrorCode::StorageCorrupted,
                        "A draft identifier is too large.",
                    )
                })
            })
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
}

/// Holds what is on screen in a composer, for the end of the application to keep.
///
/// Not `async`, and that is the point of it. A command without it runs on the thread that hears
/// the window, as it is heard, and that thread is the one the end of the application comes to: a
/// note sent before the key that quits is held before the end begins. Run anywhere else, it could
/// still be on its way when the end came. Nothing here waits for a disk, and the files are named
/// by their tokens and shared rather than copied.
#[tauri::command]
pub fn note_draft(
    state: State<'_, AppState>,
    account_id: String,
    under: Option<u32>,
    draft: DraftDto,
) -> Result<(), CommandError> {
    let files = to_keep(&draft.files, &state.attaching)?;
    let (writing, draft) = written(draft);

    state
        .being_written
        .note(Note {
            writing,
            account_id,
            under: under.map(i64::from),
            draft,
            files,
        })
        .map_err(CommandError::from)
}

/// Lets go of what the composer `writing` noted: its message was thrown away or kept on the way
/// out, the composer has gone, or it holds nothing a keep would write. On the same thread as the
/// note, for the same reason. A message sent is let go of by `send_message`, in the same step as
/// the outbox takes it.
#[tauri::command]
pub fn forget_noted_draft(state: State<'_, AppState>, writing: String) -> Result<(), CommandError> {
    state.being_written.forget(&writing);
    Ok(())
}

/// Everything an account has written down and not sent, newest first.
///
/// What the drafts folder shows above what the server's drafts folder holds. They live in the
/// profile rather than on the server, so nothing in the index knows about them and no listing of
/// mail will ever find them.
#[tauri::command]
pub async fn kept_drafts(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<KeptDraftDto>, CommandError> {
    let profile = state.storage.profile()?;

    tauri::async_runtime::spawn_blocking(move || {
        adapters::drafts(profile)
            .all(&account_id)
            .map(|kept| kept.into_iter().map(as_dto).collect())
            .map_err(|_| {
                CommandError::new(
                    ErrorCode::StorageUnavailable,
                    "What was being written cannot be read.",
                )
            })
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
}

/// What an account sent from this device and its folder for sent mail does not hold yet, newest
/// first.
///
/// What that folder shows above the server's copy of it, from the moment send is pressed until the
/// account has been read back with the message's copy in it.
#[tauri::command]
pub async fn sent_here(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<SentHereDto>, CommandError> {
    let profile = state.storage.profile()?;
    let number =
        crate::read_model::mailboxes::index_id_of(&profile, &account_id)?.ok_or_else(|| {
            CommandError::new(
                ErrorCode::InvalidRequest,
                "That account is not on this device.",
            )
        })?;
    let index = state.storage.index_of_account(number)?;

    tauri::async_runtime::spawn_blocking(move || {
        crate::read_model::sent_here::of(&profile, &index, &account_id, number)
            .map(|sent| sent.into_iter().map(as_sent_dto).collect())
            .map_err(CommandError::from)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
}

/// The last thing this account was writing, if it was writing anything.
#[tauri::command]
pub async fn latest_draft(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Option<KeptDraftDto>, CommandError> {
    let profile = state.storage.profile()?;

    tauri::async_runtime::spawn_blocking(move || {
        adapters::drafts(profile)
            .latest(&account_id)
            .map(|kept| kept.map(as_dto))
            .map_err(|_| {
                CommandError::new(
                    ErrorCode::StorageUnavailable,
                    "What was being written cannot be read.",
                )
            })
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
}

/// The files a kept draft carries, attached again for the composer that opens it.
///
/// Held here under fresh tokens, each standing for the name the draft keeps its file by, and
/// listed, taken off and sent like any file attached a moment ago. What crosses is a name and a
/// size. Nothing for a draft that is not there any more.
#[tauri::command]
pub async fn reopen_draft(
    state: State<'_, AppState>,
    id: u32,
) -> Result<Vec<AttachingDto>, CommandError> {
    let profile = state.storage.profile()?;
    let attaching = state.attaching.clone();

    tauri::async_runtime::spawn_blocking(move || {
        adapters::drafts(profile)
            .files(i64::from(id))
            .map(|files| held_again(files, &attaching, a_fresh_token))
            .map_err(|_| {
                CommandError::new(
                    ErrorCode::StorageUnavailable,
                    "What was being written cannot be read.",
                )
            })
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
}

/// Forgets it: what happens when somebody throws it away. A message sent takes the draft it was
/// kept as with it, in `send_message`.
///
/// A note kept under that number goes in the same step: written when the application ends, it
/// would bring back as a draft of its own a message somebody threw away.
#[tauri::command]
pub async fn discard_draft(state: State<'_, AppState>, id: u32) -> Result<(), CommandError> {
    let profile = state.storage.profile()?;
    let being_written = state.being_written.clone();

    tauri::async_runtime::spawn_blocking(move || {
        being_written
            .discard(adapters::drafts(profile), i64::from(id))
            .map_err(|_| {
                CommandError::new(
                    ErrorCode::StorageUnavailable,
                    "It could not be thrown away.",
                )
            })
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
}

/// The message as the window wrote it down, and the composer it comes from.
fn written(draft: DraftDto) -> (String, Draft) {
    (
        draft.writing,
        Draft {
            recipients: draft.recipients,
            copies: draft.copies,
            blind: draft.blind,
            subject: draft.subject,
            body: draft.body,
            answering: draft.answering,
            chain: draft.chain,
        },
    )
}

/// The files a keep names, as the draft is to hold them.
///
/// Refused whole when one is not here, the way sending refuses: a draft written without one of the
/// files on screen is a draft that comes back without it, and says nothing.
fn to_keep(
    tokens: &[String],
    attaching: &FilesBeingAttached,
) -> Result<Vec<DraftFile>, CommandError> {
    attaching.to_keep(tokens).ok_or_else(|| {
        CommandError::new(
            ErrorCode::InvalidRequest,
            "One of the files attached to this message is no longer here.",
        )
    })
}

/// A draft's files held again under fresh tokens, and what the window is told about each.
///
/// Fresh, so no composer is handed a token another one had; each stands for the name the draft
/// keeps its file by, so the keeps that follow find the file already there.
fn held_again(
    files: Vec<DraftFile>,
    attaching: &FilesBeingAttached,
    mut fresh: impl FnMut() -> String,
) -> Vec<AttachingDto> {
    files
        .into_iter()
        .map(|one| {
            let token = fresh();
            let told = told_about(token.clone(), &one.file.name, one.file.bytes.len());
            attaching.hold_kept(token, one.kept_as, one.file);
            told
        })
        .collect()
}

/// The message as the window wrote it, with the files it names taken from where they wait.
///
/// Read before anything is taken: a message refused for an address or a subject keeps its files,
/// and the window still showing them sends them once the address is put right. Taken rather than
/// copied — a message is sent once, and a token that survived its message would be a file the next
/// one could attach without anybody choosing it — and all of them or none.
fn with_its_files(
    message: &OutgoingDto,
    attaching: &FilesBeingAttached,
) -> Result<Outgoing, CommandError> {
    let mut outgoing = read(message, Vec::new())?;

    outgoing.files = attaching.take(&message.files).ok_or_else(|| {
        CommandError::new(
            ErrorCode::InvalidRequest,
            "One of the files attached to this message is no longer here.",
        )
    })?;

    Ok(outgoing)
}

/// The message as the domain has it, or a refusal naming what cannot be written.
///
/// Every field that becomes a header is parsed here rather than copied. What a header may contain
/// is a rule about mail, so it lives in the domain, and this is the one place a value crosses from
/// what a window sent into something that will be written onto a wire.
fn read(
    message: &OutgoingDto,
    files: Vec<lyonoyl_domain::outgoing::OutgoingFile>,
) -> Result<Outgoing, CommandError> {
    Ok(Outgoing {
        files,
        from: address(&message.from)?,
        to: addresses(&message.to)?,
        cc: addresses(&message.cc)?,
        bcc: addresses(&message.bcc)?,
        subject: Subject::parse(&message.subject).map_err(|_| {
            CommandError::new(
                ErrorCode::InvalidRequest,
                "That is not a subject a message can carry.",
            )
        })?,
        body: message.body.clone(),
        // Built from what the answered message said about itself rather than taken as given: the
        // rule for what belongs in a chain is one rule, and it lives in the domain. It refuses a
        // name it cannot read, which is the only way a second header could be written into one.
        thread: InThread::answering(message.answering.as_deref(), &message.chain).map_err(
            |_| {
                CommandError::new(
                    ErrorCode::InvalidRequest,
                    "That message cannot be answered into its conversation.",
                )
            },
        )?,
    })
}

fn addresses(values: &[String]) -> Result<Vec<EmailAddress>, CommandError> {
    values.iter().map(|value| address(value)).collect()
}

fn address(value: &str) -> Result<EmailAddress, CommandError> {
    EmailAddress::parse(value).map_err(|_| {
        CommandError::new(
            ErrorCode::InvalidRequest,
            "That is not an address a message can be sent to.",
        )
    })
}

/// One kept draft, as the window is handed it.
fn as_dto(kept: crate::ports::drafts::Kept) -> KeptDraftDto {
    KeptDraftDto {
        id: u32::try_from(kept.id).unwrap_or_default(),
        recipients: kept.draft.recipients,
        copies: kept.draft.copies,
        blind: kept.draft.blind,
        subject: kept.draft.subject,
        body: kept.draft.body,
        answering: kept.draft.answering,
        chain: kept.draft.chain,
    }
}

/// One message sent from this device, as the window is handed it.
fn as_sent_dto(sent: crate::read_model::sent_here::SentHere) -> SentHereDto {
    SentHereDto {
        id: u32::try_from(sent.id).unwrap_or_default(),
        sender: sent.sender,
        to: sent.to,
        cc: sent.cc,
        bcc: sent.bcc,
        subject: sent.subject,
        body: sent.body,
        preview: sent.preview,
        files: sent.files,
        written_at: sent.written_at,
        on_its_way: sent.on_its_way,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(over: impl FnOnce(&mut OutgoingDto)) -> OutgoingDto {
        let mut message = OutgoingDto {
            files: Vec::new(),
            from: "zeta@example.com".to_owned(),
            to: vec!["kappa@example.net".to_owned()],
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: "About Tuesday".to_owned(),
            body: "Shall we?".to_owned(),
            answering: None,
            chain: Vec::new(),
            writing: "w1".to_owned(),
            kept: None,
        };
        over(&mut message);
        message
    }

    fn a_file(name: &str) -> lyonoyl_domain::outgoing::OutgoingFile {
        lyonoyl_domain::outgoing::OutgoingFile {
            name: name.to_owned(),
            content_type: "text/plain".to_owned(),
            bytes: name.as_bytes().to_vec(),
        }
    }

    /// A message refused for what it says keeps its files: the window still shows them, and
    /// sending once the address is put right must find them where they were.
    #[test]
    fn a_message_refused_for_an_address_keeps_its_files() {
        let attaching = crate::infrastructure::attaching::FilesBeingAttached::default();
        attaching.hold("t1".to_owned(), a_file("invoice.pdf"));
        let refused = written(|message| {
            message.files = vec!["t1".to_owned()];
            message.to = vec!["not an address".to_owned()];
        });

        assert!(with_its_files(&refused, &attaching).is_err());

        let corrected = written(|message| message.files = vec!["t1".to_owned()]);
        let outgoing = with_its_files(&corrected, &attaching).expect("the file is still there");
        assert_eq!(outgoing.files.len(), 1);
    }

    /// A message with a file that is gone takes none of the others with it.
    #[test]
    fn a_message_naming_a_file_that_is_gone_takes_nothing() {
        let attaching = crate::infrastructure::attaching::FilesBeingAttached::default();
        attaching.hold("t1".to_owned(), a_file("invoice.pdf"));
        let naming_a_gone_one =
            written(|message| message.files = vec!["t1".to_owned(), "gone".to_owned()]);

        assert!(with_its_files(&naming_a_gone_one, &attaching).is_err());
        assert_eq!(
            attaching.take(&["t1".to_owned()]).map(|files| files.len()),
            Some(1)
        );
    }

    /// Sent once: the files a message took are not there for the next one.
    #[test]
    fn the_files_a_message_takes_are_gone_for_the_next() {
        let attaching = crate::infrastructure::attaching::FilesBeingAttached::default();
        attaching.hold("t1".to_owned(), a_file("invoice.pdf"));
        let message = written(|message| message.files = vec!["t1".to_owned()]);

        assert!(with_its_files(&message, &attaching).is_ok());
        assert!(with_its_files(&message, &attaching).is_err());
    }

    #[test]
    fn a_keep_names_the_files_on_screen_by_their_tokens_and_spends_none() {
        let attaching = FilesBeingAttached::default();
        attaching.hold("t1".to_owned(), a_file("invoice.pdf"));
        attaching.hold("t2".to_owned(), a_file("photo.jpg"));

        let kept = to_keep(&["t2".to_owned(), "t1".to_owned()], &attaching).expect("both are here");

        assert_eq!(
            kept.iter()
                .map(|one| (one.kept_as.as_str(), one.file.name.as_str()))
                .collect::<Vec<_>>(),
            vec![("t2", "photo.jpg"), ("t1", "invoice.pdf")]
        );
        assert!(
            with_its_files(
                &written(|message| message.files = vec!["t1".to_owned(), "t2".to_owned()]),
                &attaching
            )
            .is_ok(),
            "keeping spent a file sending needs"
        );
    }

    #[test]
    fn a_keep_naming_a_file_that_is_not_here_is_refused() {
        // Refused rather than kept without it: a draft written without a file the window shows is
        // a draft that comes back without it, and says nothing.
        let attaching = FilesBeingAttached::default();
        attaching.hold("t1".to_owned(), a_file("invoice.pdf"));

        let refused = to_keep(&["t1".to_owned(), "gone".to_owned()], &attaching)
            .expect_err("one of them is not here");

        assert_eq!(refused.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn a_keep_that_does_not_say_which_files_the_draft_carries_is_refused() {
        // Read as carrying none, it would take every file off the draft it is kept under.
        let text = r#""recipients":"","copies":"","blind":"","subject":"","body":"",
                      "answering":null,"chain":[],"writing":"w1""#;

        assert!(
            serde_json::from_str::<DraftDto>(&format!("{{{text}}}")).is_err(),
            "a keep naming no files was read as carrying none"
        );
        assert!(
            serde_json::from_str::<DraftDto>(&format!(r#"{{{text},"files":null}}"#)).is_err(),
            "a keep naming its files as nothing was read as carrying none"
        );

        let carrying_none = serde_json::from_str::<DraftDto>(&format!(r#"{{{text},"files":[]}}"#))
            .expect("a draft that says it carries nothing");
        assert!(carrying_none.files.is_empty());
    }

    #[test]
    fn a_draft_opened_again_has_its_files_held_under_fresh_tokens() {
        // Listed, taken off and sent like a file attached a moment ago, and each standing for the
        // name the draft keeps it by, so the first keep after opening writes none of them again.
        let attaching = FilesBeingAttached::default();
        let mut tokens = ["f2".to_owned(), "f1".to_owned()]
            .into_iter()
            .collect::<Vec<_>>();
        let carried = vec![
            DraftFile {
                kept_as: "k1".to_owned(),
                file: std::sync::Arc::new(a_file("invoice.pdf")),
            },
            DraftFile {
                kept_as: "k2".to_owned(),
                file: std::sync::Arc::new(a_file("photo.jpg")),
            },
        ];

        let told = held_again(carried, &attaching, || tokens.pop().expect("a token"));

        assert_eq!(
            told.iter()
                .map(|one| (one.token.as_str(), one.name.as_str(), one.bytes.as_str()))
                .collect::<Vec<_>>(),
            vec![("f1", "invoice.pdf", "11"), ("f2", "photo.jpg", "9")]
        );
        assert_eq!(
            to_keep(&["f1".to_owned(), "f2".to_owned()], &attaching)
                .expect("both are held")
                .into_iter()
                .map(|one| one.kept_as)
                .collect::<Vec<_>>(),
            vec!["k1".to_owned(), "k2".to_owned()]
        );
        let sent = with_its_files(
            &written(|message| message.files = vec!["f1".to_owned(), "f2".to_owned()]),
            &attaching,
        )
        .expect("they go with the message");
        assert_eq!(sent.files, vec![a_file("invoice.pdf"), a_file("photo.jpg")]);
    }

    #[test]
    fn a_file_too_heavy_beside_the_ones_on_screen_is_refused_as_too_large_and_held_nowhere() {
        // Said with the sentence about a file too large to attach, which the window already has,
        // and as it is chosen: held, every keep of the draft after it would be refused, with
        // whatever was typed meanwhile.
        use crate::ports::file_import::ChosenFile;
        use lyonoyl_domain::outgoing::HEAVIEST_MESSAGE;

        let attaching = FilesBeingAttached::default();
        attaching.hold(
            "t1".to_owned(),
            lyonoyl_domain::outgoing::OutgoingFile {
                bytes: vec![0; HEAVIEST_MESSAGE - 10],
                ..a_file("video.mov")
            },
        );
        let beside = ["t1".to_owned()];
        let chosen = |size: usize| ChosenFile {
            name: "photo.jpg".to_owned(),
            bytes: vec![0; size],
        };

        let refused = held_beside(chosen(11), &beside, &attaching, "t2".to_owned())
            .expect_err("too heavy beside the video");
        assert_eq!(refused.code, ErrorCode::FileTooLarge);
        assert!(
            to_keep(&["t2".to_owned()], &attaching).is_err(),
            "the file refused is held all the same"
        );

        let told = held_beside(chosen(10), &beside, &attaching, "t3".to_owned())
            .expect("a file that fits is held");
        assert_eq!(
            (told.token.as_str(), told.name.as_str(), told.bytes.as_str()),
            ("t3", "photo.jpg", "10")
        );
        assert_eq!(
            to_keep(&["t1".to_owned(), "t3".to_owned()], &attaching)
                .expect("both are held")
                .into_iter()
                .map(|one| one.file.content_type.clone())
                .collect::<Vec<_>>(),
            vec!["text/plain".to_owned(), "image/jpeg".to_owned()]
        );
    }

    #[test]
    fn the_files_of_a_message_passed_on_are_held_under_fresh_tokens_in_the_order_it_carries_them() {
        // Listed, taken off, kept and sent like a file attached a moment ago, and labelled from
        // their names as one is: what the message said a file was goes no further.
        let attaching = FilesBeingAttached::default();
        let mut tokens = vec!["f2".to_owned(), "f1".to_owned()];

        let (told, left_behind) = held_to_pass_on(
            vec![
                ("invoice.pdf".to_owned(), b"%PDF-1.4".to_vec()),
                ("photo.jpg".to_owned(), b"\xff\xd8\xff".to_vec()),
            ],
            &attaching,
            || tokens.pop().expect("a token"),
        );

        assert_eq!(
            told.iter()
                .map(|one| (one.token.as_str(), one.name.as_str(), one.bytes.as_str()))
                .collect::<Vec<_>>(),
            vec![("f1", "invoice.pdf", "8"), ("f2", "photo.jpg", "3")]
        );
        assert!(left_behind.is_empty());
        assert_eq!(
            to_keep(&["f1".to_owned(), "f2".to_owned()], &attaching)
                .expect("both are held")
                .into_iter()
                .map(|one| one.file.content_type.clone())
                .collect::<Vec<_>>(),
            vec!["application/pdf".to_owned(), "image/jpeg".to_owned()]
        );
        let sent = with_its_files(
            &written(|message| message.files = vec!["f1".to_owned(), "f2".to_owned()]),
            &attaching,
        )
        .expect("they go with the message");
        assert_eq!(sent.files[1].bytes, b"\xff\xd8\xff".to_vec());
    }

    #[test]
    fn a_file_too_heavy_beside_the_ones_before_it_is_left_behind_and_a_lighter_one_after_it_goes() {
        // Held all the same, the message could never be sent and no keep of it could be written
        // down, with everything typed into it. A smaller file after it still fits, and goes.
        use lyonoyl_domain::outgoing::HEAVIEST_MESSAGE;

        let attaching = FilesBeingAttached::default();
        let mut count = 0;

        let (told, left_behind) = held_to_pass_on(
            vec![
                ("video.mov".to_owned(), vec![0; HEAVIEST_MESSAGE - 10]),
                ("photo.jpg".to_owned(), vec![0; 11]),
                ("note.txt".to_owned(), vec![0; 10]),
            ],
            &attaching,
            || {
                count += 1;
                format!("t{count}")
            },
        );

        assert_eq!(
            told.iter().map(|one| one.name.as_str()).collect::<Vec<_>>(),
            vec!["video.mov", "note.txt"]
        );
        assert_eq!(left_behind, vec!["photo.jpg".to_owned()]);
        assert!(
            to_keep(&["t2".to_owned()], &attaching).is_err(),
            "the file left behind is held all the same"
        );
        assert!(to_keep(&["t1".to_owned(), "t3".to_owned()], &attaching).is_ok());
    }

    #[test]
    fn a_file_heavier_than_a_message_may_be_is_left_behind_on_its_own() {
        use lyonoyl_domain::outgoing::HEAVIEST_MESSAGE;

        let attaching = FilesBeingAttached::default();

        let (told, left_behind) = held_to_pass_on(
            vec![("archive.zip".to_owned(), vec![0; HEAVIEST_MESSAGE + 1])],
            &attaching,
            || "t1".to_owned(),
        );

        assert!(told.is_empty());
        assert_eq!(left_behind, vec!["archive.zip".to_owned()]);
        assert!(to_keep(&["t1".to_owned()], &attaching).is_err());
    }

    #[test]
    fn a_message_kept_as_a_file_is_labelled_a_message() {
        // So the other end shows a message passed on whole as a message, and one attached from the
        // disk the same way.
        assert_eq!(kind_of("About Tuesday.eml"), "message/rfc822");
        assert_eq!(kind_of("LUNCH.EML"), "message/rfc822");
        assert_eq!(kind_of("notes.emlx"), "application/octet-stream");
    }

    #[test]
    fn what_the_window_is_told_about_a_file_is_a_count_and_never_the_file() {
        // The size is written as a literal rather than computed here, so a build that put the
        // contents in the field cannot also move the number this compares against.
        let told = told_about("t1".to_owned(), "invoice.pdf", 27);

        assert_eq!(told.name, "invoice.pdf");
        assert_eq!(told.bytes, "27");
    }

    #[test]
    fn what_crosses_does_not_grow_with_the_file() {
        // The property, not the number: whatever the window is told about a five-megabyte
        // photograph is the same handful of characters it is told about an empty file, because
        // megabytes crossing that boundary would be a copy of somebody's file in a webview.
        let empty = told_about("t1".to_owned(), "photo.jpg", 0);
        let large = told_about("t1".to_owned(), "photo.jpg", 5_000_000);

        assert_eq!(large.name.len(), empty.name.len());
        assert!(
            large.bytes.len() < 16,
            "the window was handed something the size of the file: {}",
            large.bytes.len()
        );
    }

    #[test]
    fn what_a_person_writes_crosses_unchanged() {
        let read = read(&written(|_| {}), Vec::new()).expect("an ordinary message");

        assert_eq!(read.from.as_str(), "zeta@example.com");
        assert_eq!(read.subject.as_str(), "About Tuesday");
        assert_eq!(read.to.len(), 1);
    }

    #[test]
    fn an_address_that_is_not_one_is_refused_wherever_it_appears() {
        // Every field, not only `to`: a window that can put an address in one can put it in all
        // three, and a compromised one will.
        for spoil in [
            |m: &mut OutgoingDto| m.from = "not an address".to_owned(),
            |m: &mut OutgoingDto| m.to = vec!["not an address".to_owned()],
            |m: &mut OutgoingDto| m.cc = vec!["kappa@".to_owned()],
            |m: &mut OutgoingDto| m.bcc = vec!["@example.net".to_owned()],
            |m: &mut OutgoingDto| m.to = vec!["a@b.c\r\nBcc: attacker@example.net".to_owned()],
        ] {
            let refused = read(&written(spoil), Vec::new()).expect_err("that is not an address");
            assert_eq!(refused.code, ErrorCode::InvalidRequest);
        }
    }

    #[test]
    fn a_subject_that_would_write_a_second_header_is_refused_here() {
        // The domain refuses it; this is the boundary saying so in the terms the window branches
        // on. Reproduced before it was closed: this subject wrote a real `Bcc:` line.
        let refused = read(
            &written(|m| {
                m.subject = "About Tuesday\r\nBcc: attacker@example.net".to_owned();
            }),
            Vec::new(),
        )
        .expect_err("that is not a subject");

        assert_eq!(refused.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn a_thread_identifier_that_would_write_a_second_header_is_refused_here() {
        // The value an arriving message can carry in its own `Message-ID`, sent back in. Answering
        // it once wrote the sender's `Bcc:` onto the reply.
        let refused = read(
            &written(|m| {
                m.answering = Some("a@b>\r\nBcc: attacker@example.net\r\nX-Junk: <c".to_owned());
            }),
            Vec::new(),
        )
        .expect_err("that is not an identifier");

        assert_eq!(refused.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn a_chain_with_nothing_to_answer_is_dropped_rather_than_refused() {
        // Not the same answer as above, and it took a failing test to notice. A chain says which
        // conversation a reply belongs to, and there is no reply without the message being
        // answered — so with no `answering`, `InThread` returns to nothing and never looks at what
        // was sent with it. The poisoned value reaches no header because it reaches nothing.
        let read = read(
            &written(|m| {
                m.answering = None;
                m.chain = vec!["a@b>\r\nBcc: attacker@example.net\r\nX-Junk: <c".to_owned()];
            }),
            Vec::new(),
        )
        .expect("a message answering nothing is an ordinary message");

        assert_eq!(read.thread.answering, None);
        assert!(
            read.thread.chain.is_empty(),
            "something from the chain survived: {:?}",
            read.thread.chain
        );
    }

    #[test]
    fn a_message_addressed_to_nobody_reaches_the_use_case_rather_than_being_refused_here() {
        // Deliberately: whether a message can be sent is a rule about messages, and it lives in
        // one place. This one only refuses what cannot be turned into a message at all.
        let read =
            read(&written(|m| m.to = Vec::new()), Vec::new()).expect("nothing malformed about it");

        assert!(read.to.is_empty());
        assert!(!read.is_addressed());
    }
}
