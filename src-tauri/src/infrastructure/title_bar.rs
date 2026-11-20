//! What the strip at the top of the window does in place of the title bar macOS no longer draws
//! there (`bootstrap/window.rs`): the window says a press or a double click landed on the strip
//! (`src/app/title-bar.ts`), and this does what the system's own bar would have done.

use tauri::{Runtime, Window};

/// Moves the window with the pointer, from the press until the button comes up, as pressing a
/// title bar does. The system does the moving, from the press it is handling now.
pub fn pressed<R: Runtime>(window: &Window<R>) -> tauri::Result<()> {
    window.start_dragging()
}

/// Does what double-clicking a title bar does, which a person chooses in System Settings, under
/// Desktop & Dock: zooms the window, puts it in the Dock, or leaves it as it is.
#[cfg(target_os = "macos")]
pub fn double_clicked<R: Runtime>(window: &Window<R>) -> tauri::Result<()> {
    use objc2_foundation::{NSString, NSUserDefaults};

    let setting = NSUserDefaults::standardUserDefaults()
        .stringForKey(&NSString::from_str(SETTING))
        .map(|word| word.to_string());

    match on_double_click(setting.as_deref()) {
        DoubleClick::Zoom if window.is_maximized()? => window.unmaximize(),
        DoubleClick::Zoom => window.maximize(),
        DoubleClick::Minimize => window.minimize(),
        DoubleClick::Nothing => Ok(()),
    }
}

/// Elsewhere the window keeps the system's own title bar, which answers its double clicks itself.
#[cfg(not(target_os = "macos"))]
pub fn double_clicked<R: Runtime>(_window: &Window<R>) -> tauri::Result<()> {
    Ok(())
}

/// Where System Settings keeps what a double click on a title bar does, for every application.
#[cfg(target_os = "macos")]
const SETTING: &str = "AppleActionOnDoubleClick";

/// What a double click on a title bar does.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DoubleClick {
    Zoom,
    Minimize,
    Nothing,
}

/// The setting's word as AppKit reads it on a title bar of its own, measured in a stand: "Minimize"
/// puts the window in the Dock, "None" leaves it, and every other word zooms it — "Maximize", which
/// AppKit itself registers as the default, "Fill", a word it does not know, and no word at all. The
/// older `AppleMiniaturizeOnDoubleClick` changes nothing beside that default, so it is not read.
#[cfg(target_os = "macos")]
fn on_double_click(setting: Option<&str>) -> DoubleClick {
    match setting {
        Some("Minimize") => DoubleClick::Minimize,
        Some("None") => DoubleClick::Nothing,
        _ => DoubleClick::Zoom,
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::{on_double_click, DoubleClick};

    #[test]
    fn minimize_puts_the_window_in_the_dock() {
        assert_eq!(on_double_click(Some("Minimize")), DoubleClick::Minimize);
    }

    #[test]
    fn none_leaves_the_window_as_it_is() {
        assert_eq!(on_double_click(Some("None")), DoubleClick::Nothing);
    }

    #[test]
    fn the_default_and_fill_zoom_the_window() {
        assert_eq!(on_double_click(Some("Maximize")), DoubleClick::Zoom);
        assert_eq!(on_double_click(Some("Fill")), DoubleClick::Zoom);
    }

    #[test]
    fn no_word_and_a_word_nobody_wrote_zoom_the_window() {
        assert_eq!(on_double_click(None), DoubleClick::Zoom);
        assert_eq!(on_double_click(Some("")), DoubleClick::Zoom);
        assert_eq!(on_double_click(Some("Shrink")), DoubleClick::Zoom);
    }

    #[test]
    fn the_word_is_read_as_written() {
        // AppKit compares the words exactly, so a word in another case is a word it does not know.
        assert_eq!(on_double_click(Some("minimize")), DoubleClick::Zoom);
        assert_eq!(on_double_click(Some("none")), DoubleClick::Zoom);
    }
}
