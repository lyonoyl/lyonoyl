//! The words a notification may carry, handed over from the interface.
//!
//! Rust does not hold interface text — every user-facing string lives in one catalogue, in the
//! language the person chose. But a notification is raised in Rust, at a moment the window may
//! not even be awake. So the window hands the words over once, when it starts and whenever the
//! language changes, and Rust keeps them: the sentence a notice says when it may say nothing about
//! the message, and what its two buttons say.
//!
//! What crosses is checked rather than trusted. Each is short, it is one line, and it is the only
//! thing that will ever be shown — so a value that is none of those is refused rather than passed
//! to the operating system.

use tauri::State;

use crate::commands::error::{CommandError, ErrorCode};
use crate::ports::notifier::NoticeWords;
use crate::AppState;

/// The longest any of them may be.
///
/// A notification is one line on a surface this application does not own. Anything past this is
/// either a translation gone wrong or something that is not a translation.
const AT_MOST: usize = 120;

/// Hands over what a notification says by itself.
#[tauri::command]
pub async fn set_notification_words(
    state: State<'_, AppState>,
    new_mail: String,
    into_trash: String,
    into_junk: String,
) -> Result<(), CommandError> {
    // All three or none: a sentence refused while the buttons were kept would leave a notice
    // speaking two languages after the next change of one.
    let words = NoticeWords {
        new_mail: carried(&new_mail)?,
        into_trash: carried(&into_trash)?,
        into_junk: carried(&into_junk)?,
    };

    state.words.hold(words);

    Ok(())
}

/// The words as a notification may carry them, or a refusal.
fn carried(said: &str) -> Result<String, CommandError> {
    let said = said.trim();

    if said.is_empty() || said.chars().count() > AT_MOST {
        return Err(CommandError::new(
            ErrorCode::InvalidRequest,
            "That is not a sentence a notification can carry.",
        ));
    }

    // A control character reaches the system's own renderer, which is not this application's to
    // reason about. A newline in a notification is the least of what one could do there.
    if said.chars().any(char::is_control) {
        return Err(CommandError::new(
            ErrorCode::InvalidRequest,
            "That is not a sentence a notification can carry.",
        ));
    }

    Ok(said.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_a_notification_can_carry_are_kept_as_they_were_given() {
        assert_eq!(
            carried("Move to junk").as_deref().ok(),
            Some("Move to junk")
        );
        // The space around them is the window's, not the words'.
        assert_eq!(
            carried("  Nuevo correo ").as_deref().ok(),
            Some("Nuevo correo")
        );
        // Exactly as long as a notification takes, and in a script of its own.
        let longest = "ж".repeat(AT_MOST);
        assert_eq!(carried(&longest).ok(), Some(longest));
    }

    #[test]
    fn what_is_not_one_line_of_words_is_refused() {
        for refused in [
            String::new(),
            "   ".to_owned(),
            "ж".repeat(AT_MOST + 1),
            "Move to\njunk".to_owned(),
            "Move to junk\u{0007}".to_owned(),
            "\u{001B}[31mMove".to_owned(),
        ] {
            let answer = carried(&refused).expect_err("{refused:?} is not words");
            assert_eq!(answer.code, ErrorCode::InvalidRequest, "{refused:?}");
        }
    }
}
