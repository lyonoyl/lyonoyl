use crate::error::DomainError;

/// Smallest window the app is usable at. Mirrors `minWidth`/`minHeight` in `tauri.conf.json`.
pub const MIN_WIDTH: u32 = 960;
pub const MIN_HEIGHT: u32 = 640;

/// How the window is displayed. Maximized and full screen are different states: full screen hides
/// the menu bar and the dock, maximized fills the working area between them.
///
/// The numbers are stored in `device_window_state.mode` and must not be renumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WindowMode {
    #[default]
    Normal = 0,
    Maximized = 1,
    Fullscreen = 2,
}

impl WindowMode {
    pub fn from_stored(value: i64) -> Self {
        match value {
            1 => Self::Maximized,
            2 => Self::Fullscreen,
            _ => Self::Normal,
        }
    }

    pub fn stored(self) -> i64 {
        self as i64
    }
}

/// The panes whose width the user can drag. Stored as text, so the names must not be renamed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    /// Accounts and their folders.
    Sidebar,
    /// The message list.
    List,
}

impl Pane {
    pub fn from_stored(value: &str) -> Option<Self> {
        match value {
            "sidebar" => Some(Self::Sidebar),
            "list" => Some(Self::List),
            _ => None,
        }
    }

    pub fn stored(self) -> &'static str {
        match self {
            Self::Sidebar => "sidebar",
            Self::List => "list",
        }
    }
}

/// How wide a pane the user dragged is.
///
/// Clamped rather than rejected. The value arrives from a drag, and it is also read back from a
/// store an older or newer build may have written: a pane too narrow to see is one the user cannot
/// grab again, and a pane wider than the window hides everything beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PaneWidth(u32);

impl PaneWidth {
    /// Narrow enough to be out of the way, wide enough to still be grabbed.
    pub const MIN: u32 = 140;
    /// Leaves room for the two panes beside it at the smallest window the app runs at.
    pub const MAX: u32 = 720;

    pub fn clamped(value: u32) -> Self {
        Self(value.clamp(Self::MIN, Self::MAX))
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

/// Where the user left the window on this machine.
///
/// Kept out of [`crate::preferences::Preferences`]: geometry belongs to a physical display, and
/// syncing it to another device would place the window off-screen. Losing it is harmless — the app
/// falls back to the configured default size.
///
/// The coordinates always describe the normal, non-maximized window, so leaving full screen
/// restores a usable size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowPlacement {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    mode: WindowMode,
}

impl WindowPlacement {
    pub fn new(
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        mode: WindowMode,
    ) -> Result<Self, DomainError> {
        if width < MIN_WIDTH || height < MIN_HEIGHT {
            return Err(DomainError::WindowTooSmall);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
            mode,
        })
    }

    pub fn x(&self) -> i32 {
        self.x
    }

    pub fn y(&self) -> i32 {
        self.y
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn mode(&self) -> WindowMode {
        self.mode
    }

    /// Replaces the mode, keeping the normal-window coordinates.
    pub fn with_mode(self, mode: WindowMode) -> Self {
        Self { mode, ..self }
    }

    /// Whether the window lands on a usable area of the currently available screens.
    ///
    /// Geometry saved on a monitor that is now disconnected would open the window out of sight.
    pub fn is_visible_on(&self, screens: &[ScreenArea]) -> bool {
        // A window overlapping by a sliver has no title bar to grab, so it counts as off-screen.
        const REQUIRED_VISIBLE: i64 = 120;

        screens.iter().any(|screen| {
            let overlap_x = (self.right().min(screen.right()) - self.x.max(screen.x) as i64).max(0);
            let overlap_y =
                (self.bottom().min(screen.bottom()) - self.y.max(screen.y) as i64).max(0);
            overlap_x >= REQUIRED_VISIBLE && overlap_y >= REQUIRED_VISIBLE
        })
    }

    fn right(&self) -> i64 {
        self.x as i64 + self.width as i64
    }

    fn bottom(&self) -> i64 {
        self.y as i64 + self.height as i64
    }
}

/// A usable screen rectangle, as reported by the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl ScreenArea {
    fn right(&self) -> i64 {
        self.x as i64 + self.width as i64
    }

    fn bottom(&self) -> i64 {
        self.y as i64 + self.height as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn laptop() -> ScreenArea {
        ScreenArea {
            x: 0,
            y: 0,
            width: 1512,
            height: 982,
        }
    }

    fn placement(x: i32, y: i32) -> WindowPlacement {
        WindowPlacement::new(x, y, MIN_WIDTH, MIN_HEIGHT, WindowMode::Normal)
            .expect("fixture is valid")
    }

    #[test]
    fn rejects_geometry_below_the_usable_minimum() {
        let error = WindowPlacement::new(0, 0, 320, 240, WindowMode::Normal)
            .expect_err("too small is rejected");
        assert_eq!(error, DomainError::WindowTooSmall);
    }

    #[test]
    fn a_mode_survives_storage_as_a_number() {
        for mode in [
            WindowMode::Normal,
            WindowMode::Maximized,
            WindowMode::Fullscreen,
        ] {
            assert_eq!(WindowMode::from_stored(mode.stored()), mode);
        }
    }

    #[test]
    fn mode_numbers_are_schema_and_must_not_be_renumbered() {
        assert_eq!(WindowMode::Normal.stored(), 0);
        assert_eq!(WindowMode::Maximized.stored(), 1);
        assert_eq!(WindowMode::Fullscreen.stored(), 2);
    }

    #[test]
    fn an_unknown_stored_mode_reads_as_normal() {
        assert_eq!(WindowMode::from_stored(7), WindowMode::Normal);
        assert_eq!(WindowMode::from_stored(-1), WindowMode::Normal);
    }

    #[test]
    fn changing_the_mode_keeps_the_normal_size() {
        let normal = placement(10, 20);
        let full = normal.with_mode(WindowMode::Fullscreen);

        assert_eq!(full.mode(), WindowMode::Fullscreen);
        assert_eq!(full.width(), normal.width());
        assert_eq!(full.height(), normal.height());
        assert_eq!((full.x(), full.y()), (normal.x(), normal.y()));
    }

    #[test]
    fn a_window_fully_on_screen_is_visible() {
        assert!(placement(100, 100).is_visible_on(&[laptop()]));
    }

    #[test]
    fn a_window_from_a_disconnected_monitor_is_not_visible() {
        // Second display used to be to the right; it is gone now.
        assert!(!placement(3000, 200).is_visible_on(&[laptop()]));
    }

    #[test]
    fn a_window_peeking_by_a_few_pixels_is_not_considered_visible() {
        // Overlapping by a sliver leaves no title bar to grab, so it counts as off-screen.
        let x = laptop().width as i32 - 20;
        assert!(!placement(x, 100).is_visible_on(&[laptop()]));
    }

    #[test]
    fn a_second_monitor_makes_a_far_window_visible_again() {
        let external = ScreenArea {
            x: 1512,
            y: 0,
            width: 2560,
            height: 1440,
        };
        assert!(placement(3000, 200).is_visible_on(&[laptop(), external]));
    }

    #[test]
    fn negative_coordinates_are_valid_a_monitor_may_sit_left_of_the_primary() {
        let left = ScreenArea {
            x: -2560,
            y: 0,
            width: 2560,
            height: 1440,
        };
        assert!(placement(-2000, 100).is_visible_on(&[left, laptop()]));
    }

    #[test]
    fn a_pane_name_is_stored_and_must_not_be_renamed() {
        for pane in [Pane::Sidebar, Pane::List] {
            assert_eq!(Pane::from_stored(pane.stored()), Some(pane));
        }
        assert_eq!(Pane::from_stored("reading"), None);
    }

    #[test]
    fn a_pane_cannot_be_dragged_out_of_reach() {
        // A pane narrowed to nothing has no edge left to grab, and the layout cannot be undone.
        assert_eq!(PaneWidth::clamped(0).get(), PaneWidth::MIN);
        assert_eq!(PaneWidth::clamped(5).get(), PaneWidth::MIN);
        assert_eq!(PaneWidth::clamped(100_000).get(), PaneWidth::MAX);
    }

    #[test]
    fn a_width_within_reach_is_kept_as_it_is() {
        assert_eq!(PaneWidth::clamped(320).get(), 320);
    }
}
