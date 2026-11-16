//! The words of the two items of the menu bar that are the application's own rather than the ones
//! it starts with: the one that opens the settings, and the Edit menu's Undo.
//!
//! The window hands its words over when it starts and whenever the language changes: the core
//! writes no words for a person, so until the window has spoken the settings item is not there at
//! all, and Undo is the one the menu started with. Windows and Linux have no menu bar, and the
//! window's own keys do both there.

use tauri::AppHandle;

use crate::commands::error::{CommandError, ErrorCode};

pub use crate::infrastructure::menu_bar::{SETTINGS, UNDO};

/// The event the window hears when the settings item is chosen.
pub const OPEN_SETTINGS: &str = "settings:open";

/// The event the window hears when Undo is chosen: with the pointer, or by the key wherever the
/// window has left the key to the menu.
pub const UNDO_ASKED: &str = "undo:asked";

/// The longest word the item may carry. A menu item is a word or two; anything past this is a
/// translation gone wrong, or not a translation.
const AT_MOST: usize = 40;

/// Hands over the words the menu's settings item and its Undo say.
///
/// Both are changed in one turn of the event loop, where the menu lives. Changed from here, each
/// look and each change is a trip of its own, and two calls at once — the window starting twice
/// over, or a language chosen while the first words are still on their way — each find the
/// system's Undo before either has replaced it, and the Edit menu ends with two.
#[tauri::command]
pub async fn set_menu_words(
    app: AppHandle,
    settings: String,
    undo: String,
) -> Result<(), CommandError> {
    let settings = a_label(&settings)?.to_owned();
    let undo = a_label(&undo)?.to_owned();

    #[cfg(target_os = "macos")]
    {
        use crate::infrastructure::menu_bar;

        let (answer, changed) = tokio::sync::oneshot::channel();
        let menu = app.clone();
        app.run_on_main_thread(move || {
            let _ = answer.send(
                menu_bar::settings_item(&menu, &settings)
                    .and_then(|()| menu_bar::own_undo(&menu, &undo)),
            );
        })
        .map_err(|_| unchanged())?;
        changed
            .await
            .map_err(|_| unchanged())?
            .map_err(|_| unchanged())?;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, settings, undo);

    Ok(())
}

#[cfg(target_os = "macos")]
fn unchanged() -> CommandError {
    CommandError::new(ErrorCode::Internal, "The menu could not be changed.")
}

/// The word, checked rather than trusted: short, one line, and nothing the system's own menu
/// renders would read as anything but text.
fn a_label(words: &str) -> Result<&str, CommandError> {
    let label = words.trim();

    if label.is_empty() || label.chars().count() > AT_MOST || label.chars().any(char::is_control) {
        return Err(CommandError::new(
            ErrorCode::InvalidRequest,
            "That is not a word a menu item can carry.",
        ));
    }

    Ok(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_in_any_script_is_carried_as_it_was_written() {
        for word in ["Settings…", "Einstellungen …", "設定…", "الإعدادات…"] {
            assert_eq!(a_label(word).expect("a word"), word);
        }
        assert_eq!(a_label("  Settings…  ").expect("trimmed"), "Settings…");
    }

    #[test]
    fn nothing_is_not_a_word() {
        for nothing in ["", "   "] {
            assert_eq!(
                a_label(nothing).expect_err("refused").code,
                ErrorCode::InvalidRequest
            );
        }
    }

    #[test]
    fn a_second_line_is_refused() {
        // What the system's renderer does with a line break or a control character is not this
        // application's to reason about.
        for broken in ["Settings\n…", "Settings\u{7}", "Set\ttings"] {
            assert_eq!(
                a_label(broken).expect_err("refused").code,
                ErrorCode::InvalidRequest,
                "{broken:?}"
            );
        }
    }

    #[test]
    fn a_word_is_as_long_as_a_menu_item_holds_and_no_longer() {
        assert!(a_label(&"a".repeat(AT_MOST)).is_ok());
        assert_eq!(
            a_label(&"a".repeat(AT_MOST + 1)).expect_err("refused").code,
            ErrorCode::InvalidRequest
        );
        // Counted in characters, so a script that takes more bytes is held to the same length.
        assert!(a_label(&"設".repeat(AT_MOST)).is_ok());
    }
}
