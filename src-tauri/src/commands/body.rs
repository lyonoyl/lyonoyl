//! Opening the body of a message.
//!
//! What crosses the boundary is an address, never the body: the frame fetches it from a scheme of
//! its own, under a policy of its own, and the window that holds IPC never parses a byte of it.

use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;

use crate::application::open_message::OpenMessage;
use crate::bootstrap::adapters;
use crate::commands::error::{CommandError, ErrorCode};
use crate::infrastructure::html::Sanitizer;
use crate::infrastructure::html::{self, serving};
use crate::ports::body::{BodyForm, Pictures, Viewing};
use crate::read_model::mailboxes::MailboxReadModel;
use crate::read_model::message::MessageReadModel;
use crate::AppState;
use lyonoyl_domain::message::MessageId;

/// How the reader is looking at a message.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum BodyFormDto {
    Sent,
    Text,
    Source,
}

impl From<BodyFormDto> for BodyForm {
    fn from(form: BodyFormDto) -> Self {
        match form {
            BodyFormDto::Sent => Self::Sent,
            BodyFormDto::Text => Self::Text,
            BodyFormDto::Source => Self::Source,
        }
    }
}

/// What the interface needs in order to show a body it is never given.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct MessageBodyDto {
    /// The name this body is held under. What the window sends back to close it.
    pub name: String,
    /// Where the frame fetches it from. It answers until the window closes this body: when the
    /// message is left, or a redraw replaces it.
    pub url: String,
    /// `html`, `plainText`, or `nothing` when there was nothing that could be shown.
    pub kind: String,
    /// How many pictures the message wanted from elsewhere and did not get.
    ///
    /// A host that was unreachable or answered with something else. One of these may be there the
    /// next time somebody asks.
    pub remote_blocked: u32,
    /// How many are not shown here: a kind this build does not draw, or one too large to hold.
    ///
    /// Apart from the count above on purpose: asking again cannot help with one of these, and a
    /// notice that invites someone to try is a notice that wastes their time.
    pub remote_not_shown: u32,
    /// Asked for and did not arrive. Trying again is worth doing; for `remote_blocked` it is not.
    pub remote_not_had: u32,
    /// Whether what is shown is the beginning of the body rather than all of it.
    pub truncated: bool,
    /// The files the message carried beside its body.
    ///
    /// Names and sizes only. The bytes stay in the message until somebody asks to save one, and
    /// what is asked for is a position rather than a name: two files may share a name, and a
    /// message is free to give none.
    pub attachments: Vec<AttachedDto>,
}

/// A file a message carried.
///
/// Where it sits, what it is called and how large it is, and nothing more. Not the type the message
/// declares for it: the window names a file's kind by its name, which is what the system goes by
/// once the file is saved, and a declared type beside the name is the sender's claim handed to the
/// one place that could repeat it.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct AttachedDto {
    /// Where it sits in the message, and what is sent back to save it.
    pub at: u32,
    /// Already made safe to show and to hand to a save panel — never the string the message
    /// carried.
    pub name: String,
    /// Bytes, as text: a file can be larger than a JavaScript number holds exactly.
    pub bytes: String,
}

impl AttachedDto {
    fn of(file: &crate::ports::body::Attached) -> Self {
        Self {
            at: file.at,
            name: file.name.clone(),
            bytes: file.bytes.to_string(),
        }
    }
}

/// `form` says what the reader is looking at: the message as sent, its text alone, or the message
/// itself. The last two are the way out when what was sent renders as almost nothing.
#[tauri::command]
pub async fn open_message_body(
    state: State<'_, AppState>,
    message_id: String,
    form: BodyFormDto,
    pictures: bool,
    background: Option<String>,
    foreground: Option<String>,
) -> Result<MessageBodyDto, CommandError> {
    // A string, like everywhere else a message identifier crosses: it is a 64-bit number, and a
    // 64-bit number does not survive a round trip through JSON.
    let message = message_id
        .parse::<u64>()
        .ok()
        .and_then(|value| MessageId::new(value).ok())
        .ok_or_else(|| {
            CommandError::new(ErrorCode::InvalidRequest, "Message identifier is invalid.")
        })?;

    // What the window is wearing, so a message that says nothing about colour can wear it too.
    // Refused unless both halves are plainly colours; refused, the message is shown on white,
    // which is what a message that says nothing about colour was written for anyway.
    let surface = background
        .zip(foreground)
        .and_then(|(background, foreground)| serving::Surface::of(&background, &foreground));

    let viewing = Viewing {
        form: BodyForm::from(form),
        pictures: match pictures {
            true => Pictures::Shown,
            false => Pictures::Blocked,
        },
    };
    // The same trust policy the mail transport uses, built the same way it builds it. One policy
    // per open rather than one for the application: two places make one each, and nothing shares
    // one between them.
    let road =
        std::sync::Arc::new(adapters::transport().map_err(|_| {
            CommandError::new(ErrorCode::Internal, "The trust policy is unavailable.")
        })?);
    let profile = state.storage.profile()?;
    let index = state.storage.index_holding_message(message.get())?;
    let credentials = state.credentials.sharing(state.secrets.clone());
    // The key the mail on this disk is kept under. Read here rather than inside the store so the
    // failure has somewhere to go: a device whose secret store cannot be reached cannot read its
    // own mail, and saying so is better than an empty message.
    let sealing = adapters::message_key(&state.secrets).map_err(|_| {
        CommandError::new(
            ErrorCode::CredentialsUnavailable,
            "The key this device keeps its mail under cannot be reached.",
        )
    })?;
    let held = state.bodies.clone();
    let runtime = tauri::async_runtime::handle().inner().clone();

    let rendered = tauri::async_runtime::spawn_blocking(move || {
        let Some(place) = MessageReadModel::new(index.clone())
            .where_to_read(message)
            .map_err(read_failed)?
        else {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "That message is not in the index.",
            ));
        };

        // Refused before it is fetched rather than after it is in memory.
        if html::too_large_to_read(place.size) {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "That message is larger than this build reads.",
            ));
        }

        let Some(account_id) = MailboxReadModel::new(profile.clone(), index.clone())
            .account_of(place.account)
            .map_err(read_failed)?
        else {
            return Err(CommandError::new(
                ErrorCode::InvalidRequest,
                "That message belongs to an account that is not here.",
            ));
        };

        let client = adapters::http_client(road);

        OpenMessage::new(
            adapters::accounts(profile),
            &credentials,
            crate::bootstrap::mail_servers(runtime.clone()),
            Sanitizer,
            adapters::remote_pictures(std::sync::Arc::new(client), runtime),
            adapters::message_bodies(index, sealing),
        )
        .execute(
            &account_id,
            message,
            &place.mailbox,
            place.uid_validity,
            place.uid,
            viewing,
        )
        .map_err(CommandError::from)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))??;

    // Held for the frame that is about to ask for it, and for nothing else.
    // Copied out before the body is handed to the store, which takes it by value.
    let attachments: Vec<AttachedDto> = rendered.attachments.iter().map(AttachedDto::of).collect();

    let token = held.hold(&rendered, surface.as_ref()).ok_or_else(|| {
        CommandError::new(
            ErrorCode::Internal,
            "This machine has no randomness to spare.",
        )
    })?;

    Ok(MessageBodyDto {
        name: token.clone(),
        url: serving::url_for(&token),
        kind: match rendered.kind {
            crate::ports::body::BodyKind::Html => "html",
            crate::ports::body::BodyKind::PlainText => "plainText",
            crate::ports::body::BodyKind::Nothing => "nothing",
            crate::ports::body::BodyKind::Busy => "busy",
        }
        .to_owned(),
        remote_blocked: rendered.remote_blocked,
        remote_not_shown: rendered.remote_not_shown,
        remote_not_had: rendered.remote_not_had,
        truncated: rendered.truncated,
        attachments,
    })
}

/// Opens a link the message offered, after the person said yes to it.
///
/// The address is looked up here rather than sent in: the window knows a name, and a name that does
/// not belong to the message being read now opens nothing. The scheme is checked again at the last
/// moment — the check that decided a link could be offered ran when the message was rendered, and
/// this is the one that runs when something is actually handed to the system.
#[tauri::command]
pub fn open_link(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<(), CommandError> {
    use tauri_plugin_opener::OpenerExt;

    let Some(url) = state.bodies.link(&name) else {
        return Err(CommandError::new(
            ErrorCode::InvalidRequest,
            "That link does not belong to the message being read.",
        ));
    };

    // Read once more by the parser that named the host in the panel, and what is handed over is
    // what that parser writes out — never the text as it was held.
    let Some(url) = crate::infrastructure::html::openable(&url) else {
        return Err(CommandError::new(
            ErrorCode::InvalidRequest,
            "That address is not one this application opens.",
        ));
    };

    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|_| CommandError::new(ErrorCode::Internal, "The link could not be opened."))
}

/// Asks about an address in a message sent from this device and not read back yet, the way a link
/// in a message being read is asked about.
///
/// Such a message is shown before its copy is read back, from what was written here, and is not
/// served as a body. What the window sends is the text an address takes up, and it is found again
/// here, in that message and by the reading every text gets: what that message does not hold as an
/// address is not one this opens. Held under a name, as any link is, and the name is what the
/// window sends back when the person says yes.
#[tauri::command]
pub async fn ask_about_a_written_link(
    state: State<'_, AppState>,
    account_id: String,
    id: u32,
    written: String,
) -> Result<LinkAsked, CommandError> {
    let profile = state.storage.profile()?;
    let number =
        crate::read_model::mailboxes::index_id_of(&profile, &account_id)?.ok_or_else(|| {
            CommandError::new(
                ErrorCode::InvalidRequest,
                "That account is not on this device.",
            )
        })?;
    let index = state.storage.index_of_account(number)?;

    let sent = tauri::async_runtime::spawn_blocking(move || {
        crate::read_model::sent_here::of(&profile, &index, &account_id, number)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
    .map_err(CommandError::from)?;

    let address = sent
        .iter()
        .find(|one| one.id == i64::from(id))
        .and_then(|one| address_written_in(&one.body, &written))
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::InvalidRequest,
                "That is not an address in a message sent from here.",
            )
        })?;

    let name = state
        .bodies
        .hold_written_link(address.clone())
        .ok_or_else(|| CommandError::new(ErrorCode::Internal, "The link could not be held."))?;

    Ok(asked_about(&name, &address))
}

/// What the address that takes up the text `written` in `text` opens: only one written there whole,
/// and only as the reading every text gets finds it.
fn address_written_in(text: &str, written: &str) -> Option<String> {
    html::links_in(text)
        .into_iter()
        .find(|link| text[link.at.clone()] == *written)
        .map(|link| String::from(link.url))
}

/// Closes one body, by the name it was served under.
///
/// Addressed rather than "close whatever is open": these calls and the ones that open a body race
/// each other, and an unaddressed close that arrives late takes the body that replaced the one it
/// meant — which is a message that renders as nothing, for no reason a reader can see.
#[tauri::command]
pub fn close_message_body(state: State<'_, AppState>, name: String) {
    state.bodies.forget(&name);
}

fn read_failed(_error: crate::read_model::list::ReadModelError) -> CommandError {
    CommandError::new(
        ErrorCode::StorageUnavailable,
        "The mail index could not be read.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::body::Attached;

    const WRITTEN: &str = "Read https://example.com/a and www.example.org, then reply.";

    #[test]
    fn an_address_written_in_a_message_sent_from_here_opens_what_it_says() {
        assert_eq!(
            address_written_in(WRITTEN, "https://example.com/a").as_deref(),
            Some("https://example.com/a")
        );
        // As the parser writes it out, which is what the person is asked about and what is opened.
        assert_eq!(
            address_written_in(WRITTEN, "www.example.org").as_deref(),
            Some("https://www.example.org/")
        );
    }

    #[test]
    fn nothing_the_message_does_not_hold_as_an_address_is_opened() {
        // The window names an address by its text; one that text is not, or a piece of one, or
        // something only near it, opens nothing.
        for asked in [
            "https://elsewhere.example/",
            "https://example.com",
            "https://example.com/a and",
            "example.org",
            "reply",
            "",
        ] {
            assert_eq!(address_written_in(WRITTEN, asked), None, "{asked:?}");
        }
    }

    #[test]
    fn a_held_address_is_found_by_its_name_and_the_next_lets_it_go() {
        let bodies = serving::BodyBeingRead::default();

        let first = bodies
            .hold_written_link("https://example.com/a".to_owned())
            .expect("named");
        assert_eq!(
            bodies.link(&first).as_deref(),
            Some("https://example.com/a")
        );

        let second = bodies
            .hold_written_link("https://example.org/b".to_owned())
            .expect("named");
        assert_eq!(
            bodies.link(&first),
            None,
            "the one asked about before is let go"
        );
        assert_eq!(
            bodies.link(&second).as_deref(),
            Some("https://example.org/b")
        );
        assert_eq!(bodies.link("a name nobody gave"), None);
    }

    #[test]
    fn a_file_crosses_as_where_it_sits_what_it_is_called_and_how_large_it_is() {
        // And as nothing else. A field that crosses is one the window may show, and the type a
        // message declares for a file is the sender's word for it: a message calls an application
        // a PDF, and the window names a file's kind by its name. The size is a number past what a
        // JavaScript number holds exactly, so it is seen to cross as text.
        let crossed = serde_json::to_value(AttachedDto::of(&Attached {
            at: 2,
            name: "Statement.pdf.app".to_owned(),
            bytes: 9_007_199_254_740_993,
        }))
        .expect("written out");

        assert_eq!(
            crossed,
            serde_json::json!({
                "at": 2,
                "name": "Statement.pdf.app",
                "bytes": "9007199254740993",
            })
        );
    }
}

/// What the window is told about a link, and what it needs to ask about it.
#[derive(Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct LinkAsked {
    /// The name the link is known by here, and what the window sends back if the person says yes.
    /// The window is shown the address and never hands one back to be opened.
    pub name: String,
    /// The host, in the letters it stands for. `xn--jxagn1d.example` reads as the Greek letters of
    /// "zeta" followed by `.example`, and a person cannot be asked about a name they cannot read.
    pub host: String,
    /// The host exactly as written, when that is not the same text. A name that looks like one
    /// thing and is written as another is the whole of a homograph attack, and hiding either half
    /// is how it works.
    pub host_as_written: Option<String>,
    /// The whole address, to show under the host.
    pub url: String,
}

/// What the window is told, from a name and the address behind it.
pub fn asked_about(name: &str, url: &str) -> LinkAsked {
    let written = url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned));

    let (decoded, host_as_written) = match &written {
        Some(written) => {
            let (decoded, _) = idna::domain_to_unicode(written);
            let differs = decoded != *written;

            (decoded, differs.then(|| written.clone()))
        }
        // No host at all is what `mailto:` looks like; the address itself is then the whole story.
        None => (String::new(), None),
    };

    LinkAsked {
        name: name.to_owned(),
        host: decoded,
        host_as_written,
        // Shown whole, so what a message wrote into it is shown whole. A right-to-left override in
        // the middle of an address reverses everything after it on the screen while the bytes stay
        // as they are, which turns this panel — the one that exists so a person can read where
        // they are going — into the message's own way of saying somewhere else. Replaced rather
        // than dropped, so a reader sees that something was taken out.
        url: url
            .chars()
            .map(|character| {
                if lyonoyl_domain::attachment::reorders_text(character) {
                    '\u{FFFD}'
                } else {
                    character
                }
            })
            .collect(),
    }
}
