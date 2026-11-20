//! The menu bar, in the words the window hands over: every menu and every item in it, the settings
//! item and the Edit menu's own Undo among them.
//!
//! On macOS the menu bar starts as the one the framework builds, in its own English, and is made
//! again in the window's words the moment the window names them, and again at every language the
//! person chooses. What the system titles itself stays the system's: the full-screen item, which
//! AppKit retitles as it is shown, and what it adds on its own. Windows and Linux have no menu bar,
//! and nothing here does anything there.
//!
//! Every function here runs on the event loop's thread, where the menu lives: called from there, the
//! framework changes the menu at once rather than queueing each change behind whatever else is
//! waiting, so a run of changes is one change to anybody else.

use tauri::{AppHandle, Runtime};

/// What the settings item is known by when it is chosen.
pub const SETTINGS: &str = "settings";

/// What the application's own Undo is known by when it is chosen.
pub const UNDO: &str = "undo";

/// Whether choosing the application's own Undo does anything now, as the window last said: greyed
/// out when it would do nothing, as the system's is with nothing to undo. Kept here rather than
/// on the item alone, because the item is made again — the first time the window names it, and
/// after each of the system's panels — and each time it is made with this. Offered until the window
/// says otherwise, so a window that never says leaves it working.
#[cfg(target_os = "macos")]
static OFFERED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// The words of the menu bar, every one as the window hands it over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuWords {
    pub settings: String,
    pub about: String,
    pub services: String,
    pub hide: String,
    pub hide_others: String,
    pub quit: String,
    pub file: String,
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
    pub window_close: String,
    pub help: String,
}

/// Makes the menu bar again in these words: the menus and items the framework starts with, in the
/// same places, with the settings item after the one saying what the application is, where every
/// application on this system keeps its settings, and the application's own Undo where the
/// system's would be.
///
/// Made whole rather than renamed item by item: the framework's items are known by nothing but
/// their words, which are the very thing being changed. The Window and Help menus keep the names the
/// framework gives them, by which it hands them to the system as the menus it fills on its own.
#[cfg(target_os = "macos")]
pub fn name_the_menu_bar<R: Runtime>(app: &AppHandle<R>, words: &MenuWords) -> tauri::Result<()> {
    use tauri::menu::{
        AboutMetadata, Menu, MenuItemBuilder, PredefinedMenuItem, Submenu, HELP_SUBMENU_ID,
        WINDOW_SUBMENU_ID,
    };

    let package = app.package_info();
    let config = app.config();
    let about = AboutMetadata {
        name: Some(package.name.clone()),
        version: Some(package.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config
            .bundle
            .publisher
            .clone()
            .map(|publisher| vec![publisher]),
        ..Default::default()
    };

    let settings = MenuItemBuilder::with_id(SETTINGS, &words.settings)
        .accelerator("CmdOrCtrl+,")
        .build(app)?;
    // The application's own: the system's sends its undo to the window's text and nowhere else.
    // Chosen with nobody typing, it undid what was last typed into a field that had lost the focus,
    // the search box behind the list, and brought back no message. This one asks the window, which
    // gives a field its own undo and takes back the last message put away where nobody types; the
    // same key, so the key reaches it wherever the window leaves the key to the menu.
    let undo = MenuItemBuilder::with_id(UNDO, &words.undo)
        .accelerator("CmdOrCtrl+Z")
        .enabled(OFFERED.load(std::sync::atomic::Ordering::Relaxed))
        .build(app)?;

    let application = Submenu::with_items(
        app,
        package.name.clone(),
        true,
        &[
            &PredefinedMenuItem::about(app, Some(&words.about), Some(about))?,
            &PredefinedMenuItem::separator(app)?,
            &settings,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::services(app, Some(&words.services))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, Some(&words.hide))?,
            &PredefinedMenuItem::hide_others(app, Some(&words.hide_others))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, Some(&words.quit))?,
        ],
    )?;
    let file = Submenu::with_items(
        app,
        &words.file,
        true,
        &[&PredefinedMenuItem::close_window(
            app,
            Some(&words.file_close),
        )?],
    )?;
    let edit = Submenu::with_items(
        app,
        &words.edit,
        true,
        &[
            &undo,
            &PredefinedMenuItem::redo(app, Some(&words.redo))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some(&words.cut))?,
            &PredefinedMenuItem::copy(app, Some(&words.copy))?,
            &PredefinedMenuItem::paste(app, Some(&words.paste))?,
            &PredefinedMenuItem::select_all(app, Some(&words.select_all))?,
        ],
    )?;
    // Its one item is titled by the system, which calls it Enter or Exit Full Screen as it shows
    // the menu, over any word given it.
    let view = Submenu::with_items(
        app,
        &words.view,
        true,
        &[&PredefinedMenuItem::fullscreen(app, None)?],
    )?;
    let window = Submenu::with_id_and_items(
        app,
        WINDOW_SUBMENU_ID,
        &words.window,
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some(&words.minimize))?,
            &PredefinedMenuItem::maximize(app, Some(&words.zoom))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, Some(&words.window_close))?,
        ],
    )?;
    let help = Submenu::with_id_and_items(app, HELP_SUBMENU_ID, &words.help, true, &[])?;

    app.set_menu(Menu::with_items(
        app,
        &[&application, &file, &edit, &view, &window, &help],
    )?)?;
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
        .enabled(OFFERED.load(std::sync::atomic::Ordering::Relaxed))
        .build(app)?;
    submenu.insert(&own, at)?;
    submenu.remove(system)
}

/// Greys the application's own Undo out, or offers it again, as the window says choosing it would
/// or would not do anything.
#[cfg(target_os = "macos")]
pub fn offer_undo<R: Runtime>(app: &AppHandle<R>, offered: bool) -> tauri::Result<()> {
    use tauri::menu::MenuItemKind;

    OFFERED.store(offered, std::sync::atomic::Ordering::Relaxed);

    let Some(menu) = app.menu() else {
        return Ok(());
    };

    for submenu in menu.items()? {
        let MenuItemKind::Submenu(submenu) = submenu else {
            continue;
        };

        for item in submenu.items()? {
            if let MenuItemKind::MenuItem(own) = item {
                if own.id() == UNDO {
                    return own.set_enabled(offered);
                }
            }
        }
    }

    // Not in the menu: not named yet, or a system panel is up. It is made with this when it is.
    Ok(())
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
