use std::sync::Mutex;
use std::time::Instant;

use crate::application::error::UseCaseError;
use crate::ports::settings_repository::{DeviceStateRepository, PreferencesRepository};
use lyonoyl_domain::preferences::{
    AppIcon, DarkScheme, Density, Language, LightScheme, LookForUpdates, NewMailNotice,
    NoticeShows, Preferences, ShareAnonymousData, ShowPictures, Theme,
};
use lyonoyl_domain::window::{ScreenArea, WindowMode, WindowPlacement};

/// Reads user preferences. Missing data reads as defaults rather than as a failure.
pub struct GetPreferences<R: PreferencesRepository> {
    preferences: R,
}

impl<R: PreferencesRepository> GetPreferences<R> {
    pub fn new(preferences: R) -> Self {
        Self { preferences }
    }

    pub fn execute(&self) -> Result<Preferences, UseCaseError> {
        Ok(self.preferences.load()?)
    }
}

/// Applies a single preference change.
///
/// Read-modify-write: the caller changes one field, the others keep their values, and settings
/// this build does not know are left untouched.
pub struct UpdatePreferences<R: PreferencesRepository> {
    preferences: R,
}

/// Not `Copy`: a language carries a tag, and the tag owns its string.
#[derive(Debug, Clone)]
pub enum PreferenceChange {
    Theme(Theme),
    DarkScheme(DarkScheme),
    LightScheme(LightScheme),
    AppIcon(AppIcon),
    Density(Density),
    Language(Language),
    NewMailNotice(NewMailNotice),
    /// All three parts at once, because they are one answer to one question — what a notification
    /// may say — and the window always holds the current three: every change to a preference comes
    /// back as the whole set.
    NoticeShows(NoticeShows),
    ShowPictures(ShowPictures),
    LookForUpdates(LookForUpdates),
    ShareAnonymousData(ShareAnonymousData),
    /// The onboarding has been finished, or is to be shown again.
    Welcomed(bool),
}

impl<R: PreferencesRepository> UpdatePreferences<R> {
    pub fn new(preferences: R) -> Self {
        Self { preferences }
    }

    pub fn execute(&self, change: PreferenceChange) -> Result<Preferences, UseCaseError> {
        let current = self.preferences.load()?;
        let updated = match change {
            PreferenceChange::Theme(theme) => current.with_theme(theme),
            PreferenceChange::DarkScheme(scheme) => current.with_dark_scheme(scheme),
            PreferenceChange::LightScheme(scheme) => current.with_light_scheme(scheme),
            PreferenceChange::AppIcon(icon) => current.with_app_icon(icon),
            PreferenceChange::Density(density) => current.with_density(density),
            PreferenceChange::Language(language) => current.with_language(language),
            PreferenceChange::NewMailNotice(notice) => current.with_new_mail_notice(notice),
            PreferenceChange::NoticeShows(shows) => current.with_notice_shows(shows),
            PreferenceChange::ShowPictures(pictures) => current.with_show_pictures(pictures),
            PreferenceChange::LookForUpdates(look) => current.with_look_for_updates(look),
            PreferenceChange::ShareAnonymousData(share) => current.with_share_anonymous_data(share),
            PreferenceChange::Welcomed(welcomed) => current.with_welcomed(welcomed),
        };
        self.preferences.save(&updated)?;
        Ok(updated)
    }
}

/// Restores window geometry, discarding it when it would land off-screen.
pub struct RestoreWindowPlacement<R: DeviceStateRepository> {
    device: R,
}

impl<R: DeviceStateRepository> RestoreWindowPlacement<R> {
    pub fn new(device: R) -> Self {
        Self { device }
    }

    /// Returns geometry only when it is reachable on the currently connected displays.
    pub fn execute(&self, screens: &[ScreenArea]) -> Result<Option<WindowPlacement>, UseCaseError> {
        let stored = self.device.load_window_placement()?;
        Ok(stored.filter(|placement| placement.is_visible_on(screens)))
    }
}

/// The geometry waiting to be written, and whether a writer is already scheduled.
///
/// Move and resize events arrive dozens of times per second while dragging, so writes are batched
/// and the state written is the last one recorded.
///
/// The value and the flag live under one mutex: with two separate locks an event landing between
/// them would be recorded with no writer left to pick it up.
#[derive(Default)]
pub struct PendingGeometry {
    state: Mutex<PendingState>,
}

#[derive(Default)]
struct PendingState {
    latest: Option<WindowPlacement>,
    writer_scheduled: bool,
    ignore_until: Option<Instant>,
}

impl PendingGeometry {
    /// Drops events until `moment`.
    ///
    /// A window manager answers a size request with what it grants, and the resize event that
    /// follows carries that answer rather than the size the user chose.
    pub fn ignore_until(&self, moment: Instant) {
        if let Ok(mut state) = self.state.lock() {
            state.ignore_until = Some(moment);
        }
    }

    /// Records the newest geometry. Returns `true` when the caller must schedule the write —
    /// exactly one caller is told to, so a burst of events produces a single writer.
    pub fn record(&self, placement: WindowPlacement, now: Instant) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.ignore_until.is_some_and(|until| now < until) {
            return false;
        }
        state.latest = Some(placement);
        if state.writer_scheduled {
            return false;
        }
        state.writer_scheduled = true;
        true
    }

    /// Hands the geometry over to be written and re-arms scheduling for the next burst.
    pub fn take(&self) -> Option<WindowPlacement> {
        let Ok(mut state) = self.state.lock() else {
            return None;
        };
        state.writer_scheduled = false;
        state.latest.take()
    }
}

/// Persists window geometry. Invalid geometry is dropped rather than stored.
pub struct SaveWindowPlacement<R: DeviceStateRepository> {
    device: R,
}

impl<R: DeviceStateRepository> SaveWindowPlacement<R> {
    pub fn new(device: R) -> Self {
        Self { device }
    }

    pub fn execute(&self, placement: &WindowPlacement) -> Result<(), UseCaseError> {
        // A maximized or full-screen window reports the size of the screen, not the size the user
        // picked. Storing it would leave nothing to return to when that mode is turned off.
        let to_store = match placement.mode() {
            WindowMode::Normal => *placement,
            mode => match self.device.load_window_placement()? {
                Some(previous) => previous.with_mode(mode),
                None => *placement,
            },
        };
        Ok(self.device.save_window_placement(&to_store)?)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::ports::settings_repository::SettingsError;
    use lyonoyl_domain::window::{MIN_HEIGHT, MIN_WIDTH};

    #[derive(Default)]
    struct FakePreferences {
        stored: Mutex<Option<Preferences>>,
        fail: bool,
    }

    impl PreferencesRepository for FakePreferences {
        fn load(&self) -> Result<Preferences, SettingsError> {
            if self.fail {
                return Err(SettingsError::Unavailable);
            }
            Ok(self
                .stored
                .lock()
                .expect("test lock")
                .clone()
                .unwrap_or_default())
        }

        fn save(&self, preferences: &Preferences) -> Result<(), SettingsError> {
            if self.fail {
                return Err(SettingsError::Unavailable);
            }
            *self.stored.lock().expect("test lock") = Some(preferences.clone());
            Ok(())
        }
    }

    #[derive(Default, Clone)]
    struct FakeDeviceState {
        stored: std::sync::Arc<Mutex<Option<WindowPlacement>>>,
    }

    impl DeviceStateRepository for FakeDeviceState {
        fn load_pane_widths(
            &self,
        ) -> Result<
            Vec<(
                lyonoyl_domain::window::Pane,
                lyonoyl_domain::window::PaneWidth,
            )>,
            SettingsError,
        > {
            Ok(Vec::new())
        }

        fn save_pane_width(
            &self,
            _pane: lyonoyl_domain::window::Pane,
            _width: lyonoyl_domain::window::PaneWidth,
        ) -> Result<(), SettingsError> {
            Ok(())
        }

        fn load_folded(&self, _pane: lyonoyl_domain::window::Pane) -> Result<bool, SettingsError> {
            Ok(false)
        }

        fn save_folded(
            &self,
            _pane: lyonoyl_domain::window::Pane,
            _folded: bool,
        ) -> Result<(), SettingsError> {
            Ok(())
        }

        fn load_window_placement(&self) -> Result<Option<WindowPlacement>, SettingsError> {
            Ok(*self.stored.lock().expect("test lock"))
        }

        fn save_window_placement(&self, placement: &WindowPlacement) -> Result<(), SettingsError> {
            *self.stored.lock().expect("test lock") = Some(*placement);
            Ok(())
        }
    }

    fn laptop() -> ScreenArea {
        ScreenArea {
            x: 0,
            y: 0,
            width: 1512,
            height: 982,
        }
    }

    #[test]
    fn an_empty_store_yields_defaults() {
        let prefs = GetPreferences::new(FakePreferences::default())
            .execute()
            .expect("defaults are always available");
        assert_eq!(prefs, Preferences::default());
    }

    #[test]
    fn changing_the_theme_preserves_density() {
        let repository = FakePreferences::default();
        repository
            .save(&Preferences::default().with_density(Density::Compact))
            .expect("seed");

        let updated = UpdatePreferences::new(repository)
            .execute(PreferenceChange::Theme(Theme::Dark))
            .expect("update succeeds");

        assert_eq!(updated.theme(), Theme::Dark);
        assert_eq!(
            updated.density(),
            Density::Compact,
            "an unrelated setting must survive"
        );
    }

    fn geometry(width: u32) -> WindowPlacement {
        WindowPlacement::new(10, 20, width, MIN_HEIGHT, WindowMode::Normal)
            .expect("fixture is valid")
    }

    #[test]
    fn the_first_event_of_a_burst_is_the_one_that_schedules_the_write() {
        let pending = PendingGeometry::default();
        assert!(pending.record(geometry(MIN_WIDTH), Instant::now()));
    }

    #[test]
    fn a_burst_schedules_exactly_one_write_but_keeps_the_last_value() {
        // A drag stores the geometry it ends at, not an intermediate one.
        let pending = PendingGeometry::default();

        assert!(pending.record(geometry(MIN_WIDTH), Instant::now()));
        assert!(!pending.record(geometry(MIN_WIDTH + 100), Instant::now()));
        assert!(!pending.record(geometry(MIN_WIDTH + 200), Instant::now()));

        assert_eq!(pending.take(), Some(geometry(MIN_WIDTH + 200)));
    }

    #[test]
    fn taking_re_arms_scheduling_for_the_next_burst() {
        let pending = PendingGeometry::default();
        pending.record(geometry(MIN_WIDTH), Instant::now());
        pending.take();

        assert!(
            pending.record(geometry(MIN_WIDTH + 10), Instant::now()),
            "the next drag must schedule its own write"
        );
    }

    #[test]
    fn taking_an_empty_slot_yields_nothing_rather_than_writing_stale_geometry() {
        let pending = PendingGeometry::default();
        assert_eq!(pending.take(), None);

        pending.record(geometry(MIN_WIDTH), Instant::now());
        assert!(pending.take().is_some());
        assert_eq!(
            pending.take(),
            None,
            "a value is written once, not repeatedly"
        );
    }

    #[test]
    fn events_caused_by_the_restore_itself_are_dropped() {
        let pending = PendingGeometry::default();
        let now = Instant::now();
        pending.ignore_until(now + Duration::from_millis(500));

        assert!(!pending.record(geometry(MIN_WIDTH), now));
        assert_eq!(
            pending.take(),
            None,
            "the system's answer is not the user's choice"
        );

        assert!(pending.record(geometry(MIN_WIDTH + 100), now + Duration::from_millis(501)));
        assert!(pending.take().is_some());
    }

    #[test]
    fn a_full_screen_window_keeps_the_size_it_had_before() {
        let repository = FakeDeviceState::default();
        let chosen = WindowPlacement::new(120, 90, MIN_WIDTH, MIN_HEIGHT, WindowMode::Normal)
            .expect("valid");
        repository.save_window_placement(&chosen).expect("seed");

        let filling_the_screen = WindowPlacement::new(
            0,
            0,
            MIN_WIDTH + 600,
            MIN_HEIGHT + 300,
            WindowMode::Fullscreen,
        )
        .expect("valid");
        SaveWindowPlacement::new(repository.clone())
            .execute(&filling_the_screen)
            .expect("save succeeds");

        let stored = repository
            .load_window_placement()
            .expect("load")
            .expect("a row was written");
        assert_eq!(stored.mode(), WindowMode::Fullscreen);
        assert_eq!(
            (stored.x(), stored.y(), stored.width(), stored.height()),
            (chosen.x(), chosen.y(), chosen.width(), chosen.height()),
            "leaving full screen must land on the size the user picked"
        );
    }

    #[test]
    fn a_full_screen_window_with_nothing_stored_yet_saves_what_it_has() {
        let repository = FakeDeviceState::default();
        let filling_the_screen =
            WindowPlacement::new(0, 0, MIN_WIDTH + 600, MIN_HEIGHT, WindowMode::Fullscreen)
                .expect("valid");

        SaveWindowPlacement::new(repository.clone())
            .execute(&filling_the_screen)
            .expect("save succeeds");

        assert_eq!(
            repository.load_window_placement().expect("load"),
            Some(filling_the_screen)
        );
    }

    #[test]
    fn concurrent_events_still_schedule_a_single_writer() {
        use std::sync::Arc;

        // Events arrive from the UI thread while the exit hook and the delayed writer run
        // elsewhere.
        let pending = Arc::new(PendingGeometry::default());
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let pending = Arc::clone(&pending);
                std::thread::spawn(move || {
                    pending.record(geometry(MIN_WIDTH + i * 10), Instant::now())
                })
            })
            .collect();

        let scheduled = threads
            .into_iter()
            .filter_map(|handle| handle.join().ok())
            .filter(|scheduled| *scheduled)
            .count();

        assert_eq!(
            scheduled, 1,
            "exactly one writer, however the events interleave"
        );
        assert!(
            pending.take().is_some(),
            "and the newest value is still there"
        );
    }

    #[test]
    fn changing_the_language_preserves_the_appearance() {
        let repository = FakePreferences::default();
        repository
            .save(&Preferences::default().with_theme(Theme::Dark))
            .expect("seed");

        let updated = UpdatePreferences::new(repository)
            .execute(PreferenceChange::Language(
                Language::parse("ru").expect("well formed"),
            ))
            .expect("update succeeds");

        assert_eq!(updated.language().as_str(), "ru");
        assert_eq!(
            updated.theme(),
            Theme::Dark,
            "an unrelated setting must survive"
        );
    }

    #[test]
    fn choosing_a_picture_keeps_it_and_leaves_the_look_as_it_was() {
        let repository = FakePreferences::default();
        repository
            .save(&Preferences::default().with_theme(Theme::Dark))
            .expect("seed");

        let updated = UpdatePreferences::new(repository)
            .execute(PreferenceChange::AppIcon(AppIcon::Graphite))
            .expect("update succeeds");

        assert_eq!(updated.app_icon(), AppIcon::Graphite);
        assert_eq!(updated.theme(), Theme::Dark, "the look must survive");
    }

    #[test]
    fn choosing_a_light_scheme_keeps_it_and_leaves_the_dark_one() {
        let repository = FakePreferences::default();
        repository
            .save(&Preferences::default().with_dark_scheme(DarkScheme::Nord))
            .expect("seed");

        let updated = UpdatePreferences::new(repository)
            .execute(PreferenceChange::LightScheme(LightScheme::CatppuccinLatte))
            .expect("update succeeds");

        assert_eq!(updated.light_scheme(), LightScheme::CatppuccinLatte);
        assert_eq!(
            updated.dark_scheme(),
            DarkScheme::Nord,
            "the other side must survive"
        );
    }

    #[test]
    fn saying_yes_to_sharing_keeps_it_and_leaves_the_answer_about_updates() {
        let repository = FakePreferences::default();
        repository
            .save(&Preferences::default().with_look_for_updates(LookForUpdates::Never))
            .expect("seed");

        let updated = UpdatePreferences::new(repository)
            .execute(PreferenceChange::ShareAnonymousData(
                ShareAnonymousData::Yes,
            ))
            .expect("update succeeds");

        assert_eq!(updated.share_anonymous_data(), ShareAnonymousData::Yes);
        assert_eq!(
            updated.look_for_updates(),
            LookForUpdates::Never,
            "the other answer must survive"
        );
    }

    #[test]
    fn finishing_the_onboarding_keeps_the_answers_it_was_given() {
        let repository = FakePreferences::default();
        repository
            .save(
                &Preferences::default()
                    .with_look_for_updates(LookForUpdates::OnceADay)
                    .with_share_anonymous_data(ShareAnonymousData::Yes),
            )
            .expect("seed");

        let updated = UpdatePreferences::new(repository)
            .execute(PreferenceChange::Welcomed(true))
            .expect("update succeeds");

        assert!(updated.welcomed());
        assert_eq!(updated.look_for_updates(), LookForUpdates::OnceADay);
        assert_eq!(updated.share_anonymous_data(), ShareAnonymousData::Yes);
    }

    #[test]
    fn an_unavailable_store_surfaces_as_a_typed_error() {
        let repository = FakePreferences {
            fail: true,
            ..Default::default()
        };
        let error = GetPreferences::new(repository)
            .execute()
            .expect_err("failure is reported, not hidden behind defaults");
        assert!(matches!(error, UseCaseError::StorageUnavailable));
    }

    #[test]
    fn nothing_stored_means_nothing_to_restore() {
        let restored = RestoreWindowPlacement::new(FakeDeviceState::default())
            .execute(&[laptop()])
            .expect("missing geometry is not an error");
        assert!(restored.is_none());
    }

    #[test]
    fn geometry_from_a_disconnected_monitor_is_discarded() {
        let device = FakeDeviceState::default();
        let far_away = WindowPlacement::new(4000, 200, MIN_WIDTH, MIN_HEIGHT, WindowMode::Normal)
            .expect("valid");
        device.save_window_placement(&far_away).expect("seed");

        let restored = RestoreWindowPlacement::new(device)
            .execute(&[laptop()])
            .expect("restore succeeds");

        assert!(
            restored.is_none(),
            "the app must not open a window the user cannot see"
        );
    }

    #[test]
    fn geometry_on_a_present_screen_is_restored() {
        let device = FakeDeviceState::default();
        let here = WindowPlacement::new(80, 60, MIN_WIDTH, MIN_HEIGHT, WindowMode::Maximized)
            .expect("valid");
        device.save_window_placement(&here).expect("seed");

        let restored = RestoreWindowPlacement::new(device)
            .execute(&[laptop()])
            .expect("restore succeeds")
            .expect("geometry is usable");

        assert_eq!(restored, here);
        assert_eq!(restored.mode(), WindowMode::Maximized);
    }
}
