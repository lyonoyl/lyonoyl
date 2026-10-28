use tauri::State;

use crate::bootstrap::{MailAsked, MAIL_ASKED};
use crate::AppState;

/// What follows a person's change being written down: the window counts it, and the server is
/// told.
///
/// The change is already the state of the application — every view lays the queue over the index
/// — so the window only has to read its counts again, which it does when it hears this. The number
/// on the icon is the same fold and is redrawn with it, off the thread that answers the window.
/// The server is told by a push a moment later, which reads the account again only where the change
/// moved a message.
///
/// Not an error when there is nowhere to push: what was written down stays written down, and the
/// next round tells the server.
pub fn asked_for(app: &tauri::AppHandle, state: &State<'_, AppState>, account_id: &str) {
    use tauri::Emitter;

    let _ = app.emit(
        MAIL_ASKED,
        MailAsked {
            account_id: account_id.to_owned(),
        },
    );

    crate::bootstrap::redraw_the_badge_soon(app);

    if let Some(pushes) = state.pushes.get() {
        pushes.please_push(account_id);
    }
}
