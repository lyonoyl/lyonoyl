//! The picture the application shows itself as while it runs, chosen in the settings.
//!
//! Worn where a running application can change it: the Dock on macOS, and the window and its
//! taskbar button on Windows. A Linux desktop shows the picture of an application's desktop entry
//! wherever it finds one, rather than its window's, so the choice is not offered there. What the
//! system shows of the application itself — in Finder, in the Start menu, on a notification —
//! stays the picture it was built with, whichever is chosen.

#[cfg(target_os = "macos")]
mod dock;

use lyonoyl_domain::preferences::{AppIcon, Preferences};
use tauri::{AppHandle, Manager};

/// Whether this system lets the running application change the picture it shows itself as.
pub const WEARABLE: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// Puts the chosen picture on: the same picture whether the window is light or dark.
pub fn wear(app: &AppHandle, preferences: &Preferences) {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    put_on(app, picture(preferences.app_icon()));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = (app, preferences);
}

/// The picture for one choice, as this build carries it: square on Apple's grid for
/// the Dock, and to the edges for a window on Windows.
macro_rules! picture_in {
    ($folder:literal, $icon:expr) => {
        match $icon {
            AppIcon::Ink => {
                include_bytes!(concat!("../../../icons/choice/", $folder, "/ink.png"))
            }
            AppIcon::Indigo => {
                include_bytes!(concat!("../../../icons/choice/", $folder, "/indigo.png"))
            }
            AppIcon::Dusk => {
                include_bytes!(concat!("../../../icons/choice/", $folder, "/dusk.png"))
            }
            AppIcon::Plum => {
                include_bytes!(concat!("../../../icons/choice/", $folder, "/plum.png"))
            }
            AppIcon::Graphite => {
                include_bytes!(concat!("../../../icons/choice/", $folder, "/graphite.png"))
            }
            AppIcon::Sky => {
                include_bytes!(concat!("../../../icons/choice/", $folder, "/sky.png"))
            }
            AppIcon::Paper => {
                include_bytes!(concat!("../../../icons/choice/", $folder, "/paper.png"))
            }
            AppIcon::Ember => {
                include_bytes!(concat!("../../../icons/choice/", $folder, "/ember.png"))
            }
            AppIcon::Rust => {
                include_bytes!(concat!("../../../icons/choice/", $folder, "/rust.png"))
            }
        }
    };
}

#[cfg(target_os = "macos")]
fn picture(icon: AppIcon) -> &'static [u8] {
    picture_in!("macos", icon)
}

#[cfg(target_os = "windows")]
fn picture(icon: AppIcon) -> &'static [u8] {
    picture_in!("windows", icon)
}

#[cfg(target_os = "macos")]
fn put_on(app: &AppHandle, png: &'static [u8]) {
    dock::wear(app, png);
}

#[cfg(target_os = "windows")]
fn put_on(app: &AppHandle, png: &'static [u8]) {
    on_the_window(app, png);
}

/// The main window's own picture, and its taskbar button's with it.
///
/// Compiled everywhere and called on Windows alone, so a build on any system checks it.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn on_the_window(app: &AppHandle, png: &[u8]) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Some(image) = decoded(png) {
        let _ = window.set_icon(image);
    }
}

/// The picture as the pixels a window takes, or nothing where it is not the eight-bit RGBA every
/// picture here is written as.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn decoded(png: &[u8]) -> Option<tauri::image::Image<'static>> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(png))
        .read_info()
        .ok()?;
    let mut pixels = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut pixels).ok()?;
    if frame.color_type != png::ColorType::Rgba || frame.bit_depth != png::BitDepth::Eight {
        return None;
    }
    pixels.truncate(frame.buffer_size());
    Some(tauri::image::Image::new_owned(
        pixels,
        frame.width,
        frame.height,
    ))
}

#[cfg(test)]
mod tests;
