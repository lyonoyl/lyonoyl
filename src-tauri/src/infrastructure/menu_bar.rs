//! The two items of the menu bar that are the application's own rather than the ones it starts
//! with: the one that opens the settings, and the Edit menu's Undo.
//!
//! On macOS the menu bar is the one the framework builds, with the items every application has, and
//! these two are put in it with the words the window hands over. Windows and Linux have no menu
//! bar, and nothing here does anything there.
//!
//! Every function here runs on the event loop's thread, where the menu lives: called from there, the
//! framework changes the menu at once rather than queueing each change behind whatever else is
//! waiting, so a run of changes is one change to anybody else.

use tauri::{AppHandle, Runtime};

/// What the settings item is known by when it is chosen.
pub const SETTINGS: &str = "settings";

/// What the application's own Undo is known by when it is chosen.
pub const UNDO: &str = "undo";

/// The title the framework gives the Undo the menu starts with, which is the one thing that tells it
/// apart from the rest of the Edit menu (`Menu::default` in Tauri, whose items are muda's). The
/// system's Undo put up while a panel is up carries the window's word, and is found by what it is
/// known by instead.
#[cfg(target_os = "macos")]
const SYSTEM_UNDO: &str = "Undo";

/// Names the settings item, putting it in the application's menu the first time: after the item
/// that says what the application is, with a rule after it, where every application on this system
/// keeps its settings — and with the key every one of them opens them with.
#[cfg(target_os = "macos")]
pub fn settings_item<R: Runtime>(app: &AppHandle<R>, label: &str) -> tauri::Result<()> {
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

/// Names the application's own Undo, putting it where the system's was the first time.
///
/// The system's sends its undo to the window's text and nowhere else: chosen with nobody typing,
/// it undid what was last typed into a field that had lost the focus, the search box behind the
/// list, and brought back no message. This one asks the window, which gives a field its own undo
/// and takes back the last message put away where nobody types. The same key, so the key reaches it
/// wherever the window leaves the key to the menu: in a field, and behind a panel.
#[cfg(target_os = "macos")]
pub fn own_undo<R: Runtime>(app: &AppHandle<R>, label: &str) -> tauri::Result<()> {
    use tauri::menu::MenuItemKind;

    let Some(menu) = app.menu() else {
        return Ok(());
    };

    for submenu in menu.items()? {
        let MenuItemKind::Submenu(submenu) = submenu else {
            continue;
        };

        for (at, item) in submenu.items()?.into_iter().enumerate() {
            match item {
                MenuItemKind::MenuItem(own) if own.id() == UNDO => return own.set_text(label),
                MenuItemKind::Predefined(system) if system.text()? == SYSTEM_UNDO => {
                    return in_place_of(app, &submenu, at, &system, label);
                }
                _ => {}
            }
        }
    }

    Ok(())
}

/// The application's own Undo, with the window's word, where the system's stands.
#[cfg(target_os = "macos")]
fn in_place_of<R: Runtime>(
    app: &AppHandle<R>,
    submenu: &tauri::menu::Submenu<R>,
    at: usize,
    system: &tauri::menu::PredefinedMenuItem<R>,
    label: &str,
) -> tauri::Result<()> {
    let own = tauri::menu::MenuItemBuilder::with_id(UNDO, label)
        .accelerator("CmdOrCtrl+Z")
        .build(app)?;
    submenu.insert(&own, at)?;
    submenu.remove(system)
}

/// Raises one of the system's panels — the one a file is saved from, or chosen in — with the
/// system's Undo in the Edit menu for as long as it is up, and the application's own back after it,
/// with the word it had.
///
/// The panel is not the window. Its name field gets its undo the way every field outside the window
/// does, from the system's item, which sends it to whatever has the focus; the application's own
/// item only ever asks the window, so with it in the menu the key undid nothing in the panel.
#[cfg(target_os = "macos")]
pub fn with_system_undo<R: Runtime, T>(app: &AppHandle<R>, panel: impl FnOnce() -> T) -> T {
    let aside = stand_aside(app).ok().flatten();
    let answer = panel();
    if let Some((stand_in, label)) = aside {
        // Nothing to say if it fails: the menu keeps the system's Undo, in the window's word, and
        // the window's own key still takes back what was put away.
        let _ = come_back(app, &stand_in, &label);
    }
    answer
}

/// The same, where there is no menu bar to change.
#[cfg(not(target_os = "macos"))]
pub fn with_system_undo<R: Runtime, T>(_app: &AppHandle<R>, panel: impl FnOnce() -> T) -> T {
    panel()
}

/// Puts the system's Undo where the application's own is, in the word the window gave the
/// application's — the core writes no word of its own for a person — and answers with what the
/// stand-in is known by and that word: nothing when the application's own is not in the menu, the
/// window not having named it yet.
#[cfg(target_os = "macos")]
fn stand_aside<R: Runtime>(
    app: &AppHandle<R>,
) -> tauri::Result<Option<(tauri::menu::MenuId, String)>> {
    use tauri::menu::{MenuItemKind, PredefinedMenuItem};

    let Some(menu) = app.menu() else {
        return Ok(None);
    };

    for submenu in menu.items()? {
        let MenuItemKind::Submenu(submenu) = submenu else {
            continue;
        };

        for (at, item) in submenu.items()?.into_iter().enumerate() {
            if let MenuItemKind::MenuItem(own) = item {
                if own.id() == UNDO {
                    let label = own.text()?;
                    let stand_in = PredefinedMenuItem::undo(app, Some(&label))?;
                    submenu.insert(&stand_in, at)?;
                    submenu.remove(&own)?;
                    return Ok(Some((stand_in.id().clone(), label)));
                }
            }
        }
    }

    Ok(None)
}

/// Puts the application's own Undo back where the stand-in is, found by what it is known by.
#[cfg(target_os = "macos")]
fn come_back<R: Runtime>(
    app: &AppHandle<R>,
    stand_in: &tauri::menu::MenuId,
    label: &str,
) -> tauri::Result<()> {
    use tauri::menu::MenuItemKind;

    let Some(menu) = app.menu() else {
        return Ok(());
    };

    for submenu in menu.items()? {
        let MenuItemKind::Submenu(submenu) = submenu else {
            continue;
        };

        for (at, item) in submenu.items()?.into_iter().enumerate() {
            if let MenuItemKind::Predefined(system) = item {
                if system.id() == stand_in {
                    return in_place_of(app, &submenu, at, &system, label);
                }
            }
        }
    }

    Ok(())
}
