//! The Dock's picture, put there by the one call in this application the compiler cannot check.
//!
//! AppKit changes a running application's picture only through a call Rust marks `unsafe`. It is
//! the only one here: the workspace denies `unsafe_code`, the function below alone allows it, and
//! `scripts/check-layering.sh` refuses `unsafe` in every other file.

use objc2::{AllocAnyThread, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::NSData;

/// Puts `png` in the Dock, on the main thread, where AppKit has to be asked.
pub fn wear(app: &tauri::AppHandle, png: &'static [u8]) {
    let _ = app.run_on_main_thread(move || {
        let Some(main) = MainThreadMarker::new() else {
            return;
        };
        let Some(image) = NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(png)) else {
            return;
        };
        put(main, &image);
    });
}

#[allow(unsafe_code)]
fn put(main: MainThreadMarker, image: &NSImage) {
    let application = NSApplication::sharedApplication(main);
    // SAFETY: on the main thread, which the marker proves, with an image AppKit itself decoded from
    // a picture compiled into this application.
    unsafe { application.setApplicationIconImage(Some(image)) };
}
