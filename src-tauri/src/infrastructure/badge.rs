use tauri::AppHandle;

use crate::ports::badge::{Badge, BadgeError};

/// Puts the count on the application's icon.
///
/// **Two of the three systems draw it, and the third needs a different thing entirely.** macOS
/// draws a number on the dock icon and Linux does the same where the desktop supports it, both
/// from one call. Windows has no count: its taskbar takes a small picture laid over the icon, so a
/// number there means drawing the number into an image at the size the taskbar asks for, and the
/// call below does nothing on that system. An icon with no number is what this application looked
/// like before there was one, so what Windows gets is what it always had rather than a failure.
pub struct WindowBadge {
    app: AppHandle,
}

impl WindowBadge {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl Badge for WindowBadge {
    fn show(&self, unread: u32) -> Result<(), BadgeError> {
        use tauri::Manager;

        let Some(window) = self.app.webview_windows().into_values().next() else {
            // Nothing is open. The badge is drawn on the icon of a window that exists, and this is
            // the ordinary state while the application is starting or after the last one closes.
            return Ok(());
        };

        // Zero takes it off rather than drawing a nought, which is what `None` means here.
        let count = (unread > 0).then(|| i64::from(unread));

        #[cfg(target_os = "windows")]
        {
            let _ = (window, count);
            Ok(())
        }

        #[cfg(not(target_os = "windows"))]
        window
            .set_badge_count(count)
            .map_err(|_| BadgeError::Refused)
    }
}
