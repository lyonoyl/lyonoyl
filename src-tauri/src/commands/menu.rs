//! The one item the application's menu carries that the system does not name: the one that opens
//! the settings, in the language the window shows.
//!
//! On macOS the application's menu is the system's own, and the system names every other item in
//! it. This one is named by the window, which hands its word over when it starts and whenever the
//! language changes: the core writes no words for a person, so until the window has spoken the menu
//! has no such item at all. Windows and Linux have no application menu, and the settings open there
//! from the window's own keys.

use tauri::AppHandle;

use crate::commands::error::{CommandError, ErrorCode};

/// What the item is known by when it is chosen.
pub const SETTINGS: &str = "settings";

/// The event the window hears when the item is chosen.
pub const OPEN_SETTINGS: &str = "settings:open";

/// The longest word the item may carry. A menu item is a word or two; anything past this is a
/// translation gone wrong, or not a translation.
const AT_MOST: usize = 40;

/// Hands over the word the menu's settings item says.
#[tauri::command]
pub async fn set_menu_words(app: AppHandle, settings: String) -> Result<(), CommandError> {
    let label = a_label(&settings)?;

    #[cfg(target_os = "macos")]
    the_settings_item(&app, label)
        .map_err(|_| CommandError::new(ErrorCode::Internal, "The menu could not be changed."))?;
    #[cfg(not(target_os = "macos"))]
    let _ = (app, label);

    Ok(())
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

/// Names the item, putting it in the application's menu the first time: after the item that says
/// what the application is, with a rule after it, where every application on this system keeps its
/// settings — and with the key every one of them opens them with.
#[cfg(target_os = "macos")]
fn the_settings_item(app: &AppHandle, label: &str) -> tauri::Result<()> {
    use tauri::menu::{MenuItemBuilder, MenuItemKind, PredefinedMenuItem};

    let Some(menu) = app.menu() else {
        return Ok(());
    };
    let Some(MenuItemKind::Submenu(application)) = menu.items()?.into_iter().next() else {
        return Ok(());
    };

    for item in application.items()? {
        if let MenuItemKind::MenuItem(item) = item {
            if item.id() == SETTINGS {
                return item.set_text(label);
            }
        }
    }

    let item = MenuItemBuilder::with_id(SETTINGS, label)
        .accelerator("CmdOrCtrl+,")
        .build(app)?;
    application.insert(&item, 2)?;
    application.insert(&PredefinedMenuItem::separator(app)?, 3)
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
