use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, WebviewWindow, WindowEvent};

use crate::application::settings::{RestoreWindowPlacement, SaveWindowPlacement};
use crate::infrastructure::html::serving;
use crate::infrastructure::sqlite::settings_repository::{
    SqliteDeviceStateRepository, SqlitePreferencesRepository,
};
use crate::ports::settings_repository::{DeviceStateRepository, PreferencesRepository};
use crate::AppState;
use lyonoyl_domain::window::{Pane, ScreenArea, WindowMode, WindowPlacement};

/// Delay between the first event of a burst and the single write that follows it.
/// The write carries the newest value recorded by the time it runs.
const WRITE_DELAY: Duration = Duration::from_millis(500);

/// Window events produced by the restore itself are ignored for this long.
const RESTORE_SETTLE: Duration = Duration::from_millis(500);

/// The page the window opens at when the sidebar was left folded. `sidebarFoldedAtStart` in
/// `src/shared/ipc/client.ts` reads it, and the page is served by its path alone.
const OPENS_FOLDED: &str = "index.html?sidebar=folded";

/// What a window opens at when nothing has been stored yet.
const DEFAULT_WIDTH: f64 = 1280.0;
const DEFAULT_HEIGHT: f64 = 800.0;

/// The one window, built here rather than declared in `tauri.conf.json`.
///
/// It is built in code for one reason: a navigation handler can only be given to a window as it is
/// made, and this window needs one. A message body is shown in a frame with nothing allowed in it,
/// so the only way a click on a link inside it can say anything is by trying to navigate — and the
/// only way to hear that without the message being replaced by whatever it navigated to is to
/// refuse the navigation before it happens.
pub fn build(app: &AppHandle, followed: impl Fn(&str) + Send + 'static) -> tauri::Result<()> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    // Where it was left, asked for before there is a window rather than after. A window made at
    // one size and moved to another afterwards is a window that visibly jumps, and on macOS the
    // jump is animated — so it is not even quick. Made at the right size, there is nothing to
    // animate.
    let placement = where_it_was_left(app);

    // Whether the sidebar was left folded, asked for here for the same reason, and said in the
    // address the page opens at. Learnt after the first frame, a sidebar left folded is drawn open
    // and folds a frame later, on every start.
    let page = if sidebar_was_folded(app) {
        WebviewUrl::App(OPENS_FOLDED.into())
    } else {
        WebviewUrl::default()
    };

    let window = WebviewWindowBuilder::new(app, "main", page)
        .title("Lyonoyl Mail")
        .inner_size(
            opens_at(placement.as_ref()).0,
            opens_at(placement.as_ref()).1,
        )
        .min_inner_size(960.0, 640.0)
        // Shown by `restore_and_show`, once whatever else has to be settled is settled.
        .visible(false)
        // Taken off so the window can move its own rows. The runtime's handler sits between the
        // system and the page and swallows the drop, which is what it is for: it exists to report
        // a file dragged in from outside, and nothing here asks for one — a file to attach is
        // chosen through the system's own panel, because a path may never come from the frontend.
        // Left on, the sidebar could pick a mailbox up and never put it down, on macOS and on
        // Windows alike; the library's own note says as much for Windows.
        .disable_drag_drop_handler()
        .on_navigation(move |url| decide(url, &followed));

    // The position is set the same way, and only when there is one to set: a window with no stored
    // place is put where the system thinks a new window goes, which is better than a corner this
    // application chose.
    let window = match &placement {
        Some(placement) => window.position(f64::from(placement.x()), f64::from(placement.y())),
        None => window,
    };

    // And the mode, for the same reason as the size. A window shown and then told to fill the
    // screen fills it while the person watches; asked for as it is made, it is simply already
    // that way.
    let window = match placement.as_ref().map(WindowPlacement::mode) {
        Some(WindowMode::Maximized) => window.maximized(true),
        Some(WindowMode::Fullscreen) => window.fullscreen(true),
        _ => window,
    };

    // On macOS the window has no bar of its own: the content reaches the top of the window and the
    // buttons float over it. The name is not shown there either — the menu bar already says which
    // application this is, so a strip repeating it is a strip of mail nobody can read.
    //
    // Only there. On Windows and Linux the title bar is how a window is moved, closed and told
    // apart in a task switcher, and an application without one is an application that behaves
    // unlike every other one on the desktop.
    #[cfg(target_os = "macos")]
    let window = window
        .title_bar_style(tauri::TitleBarStyle::Overlay)
        .hidden_title(true);

    window.build().map(|_| ())
}

/// Whether a navigation may happen, and what to do instead when it may not.
///
/// Three answers, and the last one is the point. The application's own pages may be navigated to.
/// A message body and the pictures inside it are fetched from their own scheme, and that is a
/// navigation too. A link inside a message is refused — and refusing it is what keeps the message
/// on screen: the frame stays exactly where it was, and the window is told to ask the person.
///
/// Everything else is refused as well. Nothing in this application should ever navigate the window
/// to somewhere on the network, and a rule that says so is worth more than the confidence that
/// nothing does.
///
/// The scheme and the host are compared as the parser gives them, never as a prefix of the address
/// written out: `http://lyomsg.localhost@somewhere.example/` and `http://lyomsg.localhost.example/`
/// both begin with the letters this used to look for, and neither is this application.
fn decide(url: &tauri::Url, followed: &impl Fn(&str)) -> bool {
    let host = url.host_str();
    let ours = match url.scheme() {
        serving::SCHEME => host == Some("localhost") || host.is_none(),
        // Windows and Android put a custom scheme behind a name of their own.
        "http" | "https" => host == Some(SCHEME_HOST),
        _ => false,
    };

    if ours {
        // A link the message offered. Refused, and reported, so the window can ask the person.
        if let Some(name) = url.path().strip_prefix(&format!("/{}/", serving::LINKS)) {
            followed(name);
            return false;
        }

        return true;
    }

    // The application itself: its own scheme in a built application, the name Windows gives it, and
    // the development server while it is being worked on.
    match url.scheme() {
        "tauri" => true,
        "http" | "https" => {
            host == Some("tauri.localhost") || (cfg!(dev) && host == Some("localhost"))
        }
        _ => false,
    }
}

/// The host Windows and Android put this application's message scheme behind.
const SCHEME_HOST: &str = "lyomsg.localhost";

/// Restores the window where the user left it, if that place still exists./// Restores the window where the user left it, if that place still exists.
/// Puts the window where it was left and shows it.
///
/// The window is created hidden. Showing it first and moving it afterwards is a window that opens
/// at one size and jumps to another while the person watches — and on macOS the jump is animated,
/// so it is not even quick. Everything below happens before anything is on screen.
pub fn restore_and_show(window: &WebviewWindow) {
    // A window manager answers a size request with what it is willing to grant, and that answer
    // arrives as a resize event. Recording it would replace what the person chose with the
    // system's opinion of it, so events are ignored for a moment after the window appears.
    if let Some(state) = window.try_state::<AppState>() {
        state.geometry.ignore_until(Instant::now() + RESTORE_SETTLE);
    }

    // Nothing is moved or resized here: the window was made the size, the place and the mode it
    // was left in. All that is left is to show it.
    let _ = window.show();
    let _ = window.set_focus();
}

/// The size a window opens at: the one it was left at, or the one this application starts with.
///
/// Separate from the building so it can be said what it does without a window to say it to.
fn opens_at(placement: Option<&WindowPlacement>) -> (f64, f64) {
    match placement {
        Some(placement) => (f64::from(placement.width()), f64::from(placement.height())),
        None => (DEFAULT_WIDTH, DEFAULT_HEIGHT),
    }
}

/// Where the window was left, or nothing when there is nowhere to put it back.
///
/// Read before the window exists, so the window can be made that size rather than made and then
/// changed. Nothing here fails loudly: a profile that will not open, a screen that is no longer
/// connected and a first run all mean the same thing — open where the system would open it.
fn where_it_was_left(app: &AppHandle) -> Option<WindowPlacement> {
    let state = app.try_state::<AppState>()?;
    let pool = state.storage.profile().ok()?;

    let screens = screens_of(app);
    if screens.is_empty() {
        return None;
    }

    RestoreWindowPlacement::new(SqliteDeviceStateRepository::new(pool))
        .execute(&screens)
        .ok()
        .flatten()
}

/// Whether the sidebar was left folded. Open where nothing can be read: a first start, and a
/// profile that has not opened.
fn sidebar_was_folded(app: &AppHandle) -> bool {
    let Some(state) = app.try_state::<AppState>() else {
        return false;
    };
    let Ok(pool) = state.storage.profile() else {
        return false;
    };

    SqliteDeviceStateRepository::new(pool)
        .load_folded(Pane::Sidebar)
        .unwrap_or(false)
}

/// Puts on the picture chosen in the settings. The settings are read off the thread that runs the
/// window, as everything in a database is.
pub fn wear_the_chosen_icon(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(state) = app.try_state::<AppState>() else {
            return;
        };
        let Ok(profile) = state.storage.profile() else {
            return;
        };
        let preferences = SqlitePreferencesRepository::new(profile)
            .load()
            .unwrap_or_default();
        crate::infrastructure::app_icon::wear(&app, &preferences);
    });
}

/// Persists geometry as the user moves and resizes the window.
pub fn persist_on_change(window: &WebviewWindow) {
    let handle = window.clone();

    window.on_window_event(move |event| {
        // Before the geometry filter, because focus is neither a move nor a resize and the filter
        // below returns for everything that is not one.
        if let WindowEvent::Focused(has_it) = event {
            if let Some(state) = handle.try_state::<AppState>() {
                state.attention.now(*has_it);
            }
            return;
        }

        if !matches!(
            event,
            WindowEvent::Moved(_) | WindowEvent::Resized(_) | WindowEvent::CloseRequested { .. }
        ) {
            return;
        }

        let Some(placement) = current_placement(&handle) else {
            return;
        };
        let Some(state) = handle.try_state::<AppState>() else {
            return;
        };
        let scheduled_by_us = state.geometry.record(placement, Instant::now());

        // A closing window is the last moment its geometry can be read, so this write is not
        // deferred: a background thread would be dropped when the process exits.
        if matches!(event, WindowEvent::CloseRequested { .. }) {
            flush(handle.app_handle());
            return;
        }

        if !scheduled_by_us {
            return;
        }

        let app = handle.app_handle().clone();
        // SQLite is never touched on the UI thread.
        tauri::async_runtime::spawn_blocking(move || {
            std::thread::sleep(WRITE_DELAY);
            flush(&app);
        });
    });
}

/// Writes the newest recorded geometry, if there is any.
pub fn flush(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let Some(placement) = state.geometry.take() else {
        return;
    };
    let Ok(pool) = state.storage.profile() else {
        return;
    };
    let _ = SaveWindowPlacement::new(SqliteDeviceStateRepository::new(pool)).execute(&placement);
}

/// Reads the window's current geometry, or `None` when it is not meaningful.
fn current_placement(window: &WebviewWindow) -> Option<WindowPlacement> {
    // A minimized window reports the geometry of its dock tile.
    if window.is_minimized().unwrap_or(false) {
        return None;
    }

    let scale = window.scale_factor().ok()?;
    let position = window.outer_position().ok()?.to_logical::<i32>(scale);
    let size = window.inner_size().ok()?.to_logical::<u32>(scale);

    WindowPlacement::new(
        position.x,
        position.y,
        size.width,
        size.height,
        current_mode(window),
    )
    .ok()
}

/// Full screen is checked before maximized on every platform, and macOS makes the order matter:
/// there a full-screen window answers "yes" to both questions.
fn current_mode(window: &WebviewWindow) -> WindowMode {
    if window.is_fullscreen().unwrap_or(false) {
        return WindowMode::Fullscreen;
    }
    if window.is_maximized().unwrap_or(false) {
        return WindowMode::Maximized;
    }
    WindowMode::Normal
}

/// Screen rectangles currently available, in logical coordinates.
/// The screens a window could be put on, asked of the application before there is a window.
fn screens_of(app: &AppHandle) -> Vec<ScreenArea> {
    match app.available_monitors() {
        Ok(monitors) => areas_of(&monitors),
        Err(_) => Vec::new(),
    }
}

fn areas_of(monitors: &[tauri::Monitor]) -> Vec<ScreenArea> {
    monitors
        .iter()
        .map(|monitor| {
            let scale = monitor.scale_factor();
            let position = monitor.position().to_logical::<i32>(scale);
            let size = monitor.size().to_logical::<u32>(scale);
            ScreenArea {
                x: position.x,
                y: position.y,
                width: size.width,
                height: size.height,
            }
        })
        .collect()
}

#[cfg(test)]
mod opening_tests {
    use super::*;
    use lyonoyl_domain::window::WindowMode;

    #[test]
    fn a_window_opens_at_the_size_it_was_left_at() {
        // Made that size rather than made and then changed: a window that jumps from one size to
        // another is a window a person watches instead of reads, and on macOS the jump is
        // animated.
        let left = WindowPlacement::new(100, 120, 1440, 900, WindowMode::Normal)
            .expect("a placement a person could have left");

        assert_eq!(opens_at(Some(&left)), (1440.0, 900.0));
    }

    #[test]
    fn a_first_run_opens_at_the_size_this_application_starts_with() {
        assert_eq!(opens_at(None), (DEFAULT_WIDTH, DEFAULT_HEIGHT));
    }

    #[test]
    fn a_window_whose_sidebar_was_left_folded_opens_at_its_own_page_saying_so() {
        // Joined onto the application's address the way the runtime joins a page, on every
        // system's form of that address, and let through by the window's own navigation rule.
        for app in ["tauri://localhost", "http://tauri.localhost"] {
            let page = tauri::Url::parse(app)
                .and_then(|base| base.join(OPENS_FOLDED))
                .expect("an address");

            assert_eq!(page.path(), "/index.html", "{app}");
            assert_eq!(page.query(), Some("sidebar=folded"), "{app}");
            assert!(decide(&page, &|_: &str| {}), "{app}");
        }
    }
}

#[cfg(test)]
mod navigation_tests {
    use super::*;

    fn asked(address: &str) -> (bool, Option<String>) {
        let followed = std::sync::Mutex::new(None);
        let url = tauri::Url::parse(address).expect("an address");

        let allowed = decide(&url, &|name: &str| {
            *followed.lock().expect("test lock") = Some(name.to_owned());
        });

        (allowed, followed.into_inner().expect("test lock"))
    }

    #[test]
    fn a_link_inside_a_message_is_refused_and_reported() {
        // Refusing it is what keeps the message on screen: the frame stays exactly where it was.
        // Allowing it would replace the message with whatever the address answered.
        let name = "a".repeat(64);

        let (allowed, followed) = asked(&format!("lyomsg://localhost/link/{name}"));

        assert!(!allowed, "the frame was allowed to leave the message");
        assert_eq!(followed.as_deref(), Some(name.as_str()));
    }

    #[test]
    fn the_body_and_its_pictures_are_allowed_and_reported_to_nobody() {
        for address in [
            "lyomsg://localhost/deadbeef",
            "http://lyomsg.localhost/deadbeef",
        ] {
            let (allowed, followed) = asked(address);

            assert!(allowed, "{address} was refused");
            assert_eq!(followed, None);
        }
    }

    #[test]
    fn an_address_that_only_begins_like_ours_is_not_ours() {
        // The finding this closes: the gate compared the address as written, so a host with the
        // right letters at the front — as a user name, or as a longer domain — was let out onto
        // the network.
        for address in [
            "http://lyomsg.localhost@somewhere.example/steal",
            "http://lyomsg.localhost.somewhere.example/steal",
            "http://tauri.localhost@somewhere.example/steal",
            "http://tauri.localhost.somewhere.example/steal",
            "http://localhost.somewhere.example/steal",
            "https://lyomsg.localhost.somewhere.example/steal",
        ] {
            let (allowed, followed) = asked(address);

            assert!(!allowed, "{address} was allowed");
            assert_eq!(followed, None, "{address} was read as a link of ours");
        }
    }

    #[test]
    fn a_link_is_recognised_by_its_path_rather_than_by_the_letters_of_the_address() {
        let name = "b".repeat(64);

        for address in [
            format!("lyomsg://localhost/link/{name}"),
            format!("http://lyomsg.localhost/link/{name}"),
        ] {
            let (allowed, followed) = asked(&address);

            assert!(!allowed, "{address} was allowed");
            assert_eq!(followed.as_deref(), Some(name.as_str()));
        }
    }

    #[test]
    fn the_window_may_never_go_out_onto_the_network() {
        // Nothing in this application navigates the window anywhere. A rule that says so is worth
        // more than the confidence that nothing does.
        for address in [
            "https://example.com/",
            "http://example.com/",
            "file:///etc/passwd",
            "http://localhost.evil.example.com/",
        ] {
            let (allowed, followed) = asked(address);

            assert!(!allowed, "{address} was allowed");
            assert_eq!(followed, None, "{address} was read as a link");
        }
    }

    #[test]
    fn the_application_itself_is_allowed() {
        for address in ["tauri://localhost/index.html", "http://tauri.localhost/"] {
            assert!(asked(address).0, "{address} was refused");
        }
    }
}
