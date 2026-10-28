use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::ports::attention::Attention;

/// What the windowing system last said about the focus.
///
/// A flag rather than a question asked on demand, because the answer is needed on a background
/// thread at the end of a round and the window can only be asked from the thread that owns it.
/// The events write it; everything else reads it.
#[derive(Clone, Default)]
pub struct Focus {
    /// Starts **true**: the window is being opened, and the moments before the first event are
    /// exactly the moments nothing should interrupt anybody.
    watching: Arc<AtomicBool>,
}

impl Focus {
    pub fn assuming_it_does() -> Self {
        let watching = Arc::new(AtomicBool::new(true));
        Self { watching }
    }

    /// Told by the window event.
    pub fn now(&self, has_it: bool) {
        self.watching.store(has_it, Ordering::Relaxed);
    }
}

impl Attention for Focus {
    fn has_it(&self) -> bool {
        self.watching.load(Ordering::Relaxed)
    }
}
