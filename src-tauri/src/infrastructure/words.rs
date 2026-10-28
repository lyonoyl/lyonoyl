use std::sync::{Arc, RwLock};

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
    new_mail: Arc<RwLock<Option<String>>>,
}

impl InterfaceWords {
    pub fn new_mail(&self, sentence: &str) {
        if let Ok(mut held) = self.new_mail.write() {
            *held = Some(sentence.to_owned());
        }
    }

    /// The sentence, or nothing if the window has not handed one over yet.
    pub fn for_new_mail(&self) -> Option<String> {
        self.new_mail.read().ok().and_then(|held| held.clone())
    }
}
