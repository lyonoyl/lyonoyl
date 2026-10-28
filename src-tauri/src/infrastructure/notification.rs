use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::ports::notifier::{Notice, Notifier, NotifyError};

/// Hands a notice to the operating system.
///
/// The whole of the adapter, and it is this small because the port is: two strings, already
/// decided and already cleaned. Nothing here chooses what a notice says, and nothing here can add
/// to it — a subject reaches the system only because somebody asked for one in the settings, and
/// the place that read that setting is `AnnounceMail`.
///
/// **The icon and the name beside it are the bundle's, not this code's.** macOS attributes a
/// notification to the application bundle that posted it, so a build with no bundle — which is
/// what `npm run app` runs, a bare binary out of `target` — is attributed to whatever launched it,
/// and the terminal's icon appears instead of this application's. Nothing here can set it; a built
/// application carries `CFBundleIdentifier` and `icon.icns` and is attributed to itself.
///
/// On macOS the library installs a hook that makes the main bundle report a different identifier.
/// In a build the system knows, the identifier it substitutes is this application's own, so both
/// states answer the same string; the one bad state needs a build the system has never seen, and
/// that cannot come before the first keychain read because a notification needs a sync and a sync
/// needs the database key. Written down because it is the kind of thing somebody would otherwise
/// discover by finding the keychain asking for permission.
pub struct SystemNotifier {
    app: AppHandle,
}

impl SystemNotifier {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl Notifier for SystemNotifier {
    fn show(&self, notice: &Notice) -> Result<(), NotifyError> {
        self.app
            .notification()
            .builder()
            .title(&notice.title)
            .body(&notice.body)
            .show()
            .map_err(|_| NotifyError::Refused)
    }
}
