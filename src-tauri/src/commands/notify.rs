//! The one sentence a notification may carry, handed over from the interface.
//!
//! Rust does not hold interface text — every user-facing string lives in one catalogue, in the
//! language the person chose. But a notification is raised in Rust, at a moment the window may
//! not even be awake. So the window hands the sentence over once, when it starts and whenever the
//! language changes, and Rust keeps that one string.
//!
//! What crosses is checked rather than trusted. It is short, it is one line, and it is the only
//! thing that will ever be shown — so a value that is none of those is refused rather than passed
//! to the operating system.

use tauri::State;

use crate::commands::error::{CommandError, ErrorCode};
use crate::AppState;

/// The longest a notification's body may be.
///
/// A notification is one line on a surface this application does not own. Anything past this is
/// either a translation gone wrong or something that is not a translation.
const AT_MOST: usize = 120;

/// Hands over the sentence a notification says.
#[tauri::command]
pub async fn set_notification_words(
    state: State<'_, AppState>,
    new_mail: String,
) -> Result<(), CommandError> {
    let sentence = new_mail.trim();

    if sentence.is_empty() || sentence.chars().count() > AT_MOST {
        return Err(CommandError::new(
            ErrorCode::InvalidRequest,
            "That is not a sentence a notification can carry.",
        ));
    }

    // A control character reaches the system's own renderer, which is not this application's to
    // reason about. A newline in a notification is the least of what one could do there.
    if sentence.chars().any(char::is_control) {
        return Err(CommandError::new(
            ErrorCode::InvalidRequest,
            "That is not a sentence a notification can carry.",
        ));
    }

    state.words.new_mail(sentence);

    Ok(())
}
