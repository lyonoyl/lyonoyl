//! The words of the menu bar: every menu and every item in it, the settings item and the Edit
//! menu's own Undo among them.
//!
//! The window hands its words over when it starts and whenever the language changes, and the menu
//! bar is made again in them: the core writes no words for a person, so until the window has spoken
//! the menu bar is the one the framework starts with, in its own English, with no settings item at
//! all. Windows and Linux have no menu bar, and the window's own keys do there what its items do.

use serde::Deserialize;
use tauri::AppHandle;
use ts_rs::TS;

use crate::commands::error::{CommandError, ErrorCode};
use crate::infrastructure::menu_bar::MenuWords;

pub use crate::infrastructure::menu_bar::{SETTINGS, UNDO};

/// The event the window hears when the settings item is chosen.
pub const OPEN_SETTINGS: &str = "settings:open";

/// The event the window hears when Undo is chosen: with the pointer, or by the key wherever the
/// window has left the key to the menu.
pub const UNDO_ASKED: &str = "undo:asked";

/// The longest word the item may carry. A menu item is a word or two; anything past this is a
/// translation gone wrong, or not a translation.
const AT_MOST: usize = 40;

/// Every word of the menu bar, as the window's catalogue says it.
#[derive(Debug, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct MenuWordsDto {
    pub settings: String,
    pub about: String,
    pub services: String,
    pub hide: String,
    pub hide_others: String,
    pub quit: String,
    pub file: String,
    /// Close Window in the File menu.
    pub file_close: String,
    pub edit: String,
    pub undo: String,
    pub redo: String,
    pub cut: String,
    pub copy: String,
    pub paste: String,
    pub select_all: String,
    pub view: String,
    pub window: String,
    pub minimize: String,
    pub zoom: String,
    /// Close Window in the Window menu.
    pub window_close: String,
    pub help: String,
}

/// Hands over the words of the menu bar, which is made again in them.
///
/// Made in one turn of the event loop, where the menu lives. Changed from here, each look and each
/// change would be a trip of its own, and two calls at once — the window starting twice over, or a
/// language chosen while the first words are still on their way — would each change a menu the
/// other was changing.
#[tauri::command]
pub async fn set_menu_words(app: AppHandle, words: MenuWordsDto) -> Result<(), CommandError> {
    let words = checked(words)?;

    #[cfg(target_os = "macos")]
    {
        use crate::infrastructure::menu_bar;

        let (answer, changed) = tokio::sync::oneshot::channel();
        let menu = app.clone();
        app.run_on_main_thread(move || {
            let _ = answer.send(menu_bar::name_the_menu_bar(&menu, &words));
        })
        .map_err(|_| unchanged())?;
        changed
            .await
            .map_err(|_| unchanged())?
            .map_err(|_| unchanged())?;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, words);

    Ok(())
}

/// Every word checked, and the menu bar's words only if each of them is one: a menu half in one
/// language and half refused is not one to make.
fn checked(words: MenuWordsDto) -> Result<MenuWords, CommandError> {
    let word = |said: &str| a_label(said).map(str::to_owned);
    Ok(MenuWords {
        settings: word(&words.settings)?,
        about: word(&words.about)?,
        services: word(&words.services)?,
        hide: word(&words.hide)?,
        hide_others: word(&words.hide_others)?,
        quit: word(&words.quit)?,
        file: word(&words.file)?,
        file_close: word(&words.file_close)?,
        edit: word(&words.edit)?,
        undo: word(&words.undo)?,
        redo: word(&words.redo)?,
        cut: word(&words.cut)?,
        copy: word(&words.copy)?,
        paste: word(&words.paste)?,
        select_all: word(&words.select_all)?,
        view: word(&words.view)?,
        window: word(&words.window)?,
        minimize: word(&words.minimize)?,
        zoom: word(&words.zoom)?,
        window_close: word(&words.window_close)?,
        help: word(&words.help)?,
    })
}

/// Says whether choosing the menu's Undo does anything now, so that it is greyed out when it would
/// not, as the system's is with nothing to undo.
#[tauri::command]
pub async fn set_menu_undo_offered(app: AppHandle, offered: bool) -> Result<(), CommandError> {
    #[cfg(target_os = "macos")]
    {
        use crate::infrastructure::menu_bar;

        let (answer, changed) = tokio::sync::oneshot::channel();
        let menu = app.clone();
        app.run_on_main_thread(move || {
            let _ = answer.send(menu_bar::offer_undo(&menu, offered));
        })
        .map_err(|_| unchanged())?;
        changed
            .await
            .map_err(|_| unchanged())?
            .map_err(|_| unchanged())?;
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (app, offered);

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

    /// Every word of a menu bar, each saying which it is.
    fn every_word() -> MenuWordsDto {
        MenuWordsDto {
            settings: "Settings…".into(),
            about: "About Lyonoyl Mail".into(),
            services: "Services".into(),
            hide: "Hide Lyonoyl Mail".into(),
            hide_others: "Hide Others".into(),
            quit: "Quit Lyonoyl Mail".into(),
            file: "File".into(),
            file_close: "Close Window (File)".into(),
            edit: "Edit".into(),
            undo: "Undo".into(),
            redo: "Redo".into(),
            cut: "Cut".into(),
            copy: "Copy".into(),
            paste: "Paste".into(),
            select_all: "Select All".into(),
            view: "View".into(),
            window: "Window".into(),
            minimize: "Minimize".into(),
            zoom: "Zoom".into(),
            window_close: "Close Window (Window)".into(),
            help: "Help".into(),
        }
    }

    #[test]
    fn every_word_reaches_its_own_place_in_the_menu_bar() {
        let words = checked(MenuWordsDto {
            settings: "  Settings…  ".into(),
            ..every_word()
        })
        .expect("every word is one");

        assert_eq!(
            words,
            MenuWords {
                settings: "Settings…".into(),
                about: "About Lyonoyl Mail".into(),
                services: "Services".into(),
                hide: "Hide Lyonoyl Mail".into(),
                hide_others: "Hide Others".into(),
                quit: "Quit Lyonoyl Mail".into(),
                file: "File".into(),
                file_close: "Close Window (File)".into(),
                edit: "Edit".into(),
                undo: "Undo".into(),
                redo: "Redo".into(),
                cut: "Cut".into(),
                copy: "Copy".into(),
                paste: "Paste".into(),
                select_all: "Select All".into(),
                view: "View".into(),
                window: "Window".into(),
                minimize: "Minimize".into(),
                zoom: "Zoom".into(),
                window_close: "Close Window (Window)".into(),
                help: "Help".into(),
            }
        );
    }

    #[test]
    fn one_word_refused_refuses_the_whole_menu_bar() {
        for refused in [
            MenuWordsDto {
                help: String::new(),
                ..every_word()
            },
            MenuWordsDto {
                quit: "Quit\nLyonoyl Mail".into(),
                ..every_word()
            },
            MenuWordsDto {
                zoom: "z".repeat(AT_MOST + 1),
                ..every_word()
            },
        ] {
            assert_eq!(
                checked(refused).expect_err("refused").code,
                ErrorCode::InvalidRequest
            );
        }
    }

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
