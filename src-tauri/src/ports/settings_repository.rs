use thiserror::Error;

use lyonoyl_domain::preferences::Preferences;
use lyonoyl_domain::window::{Pane, PaneWidth, WindowPlacement};

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("the local settings store is unavailable")]
    Unavailable,
    #[error("a stored setting is invalid")]
    Corrupted,
}

/// User preferences — the half of the settings that belongs to the person, not the machine.
///
/// Reads never fail on unknown or missing keys: a fresh install, a partially written store and a
/// store written by a newer version all yield a usable [`Preferences`] built from defaults.
/// Writes are per-key, so a build that does not know a setting cannot erase it (this is what makes
/// version rollback safe).
pub trait PreferencesRepository: Send + Sync {
    fn load(&self) -> Result<Preferences, SettingsError>;
    fn save(&self, preferences: &Preferences) -> Result<(), SettingsError>;
}

/// Machine-local state. Never synced to another device, and safe to lose.
pub trait DeviceStateRepository: Send + Sync {
    fn load_window_placement(&self) -> Result<Option<WindowPlacement>, SettingsError>;
    fn save_window_placement(&self, placement: &WindowPlacement) -> Result<(), SettingsError>;

    /// How wide each pane was left. A pane with no stored width opens at its default.
    fn load_pane_widths(&self) -> Result<Vec<(Pane, PaneWidth)>, SettingsError>;
    fn save_pane_width(&self, pane: Pane, width: PaneWidth) -> Result<(), SettingsError>;

    /// Whether a pane was left folded. A pane nobody folded is open.
    fn load_folded(&self, pane: Pane) -> Result<bool, SettingsError>;
    fn save_folded(&self, pane: Pane, folded: bool) -> Result<(), SettingsError>;
}
