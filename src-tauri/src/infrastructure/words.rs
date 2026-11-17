use std::sync::{Arc, RwLock};

use crate::ports::notifier::NoticeWords;

/// The interface's own words, kept where Rust can reach them.
///
/// Held here rather than asked for, because the moment it is needed — the end of a
/// round on a background thread — is a moment the window may be busy or asleep, and a notification
/// that waited for the window would arrive after the mail it is about.
///
/// Starts empty, and empty means silent: a notification with no words is not a notification, and
/// Rust inventing English prose would be exactly the thing that is forbidden.
#[derive(Clone, Default)]
pub struct InterfaceWords {
    for_notices: Arc<RwLock<Option<NoticeWords>>>,
}

impl InterfaceWords {
    /// Everything a notice says of its own, handed over together so a notice never carries a
    /// sentence in one language beside a button in another.
    pub fn hold(&self, words: NoticeWords) {
        if let Ok(mut held) = self.for_notices.write() {
            *held = Some(words);
        }
    }

    /// The words, or nothing if the window has not handed them over yet.
    pub fn for_notices(&self) -> Option<NoticeWords> {
        self.for_notices.read().ok().and_then(|held| held.clone())
    }
}
