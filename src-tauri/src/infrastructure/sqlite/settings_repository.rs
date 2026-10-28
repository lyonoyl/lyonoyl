use rusqlite::OptionalExtension;

use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::ports::settings_repository::{
    DeviceStateRepository, PreferencesRepository, SettingsError,
};
use lyonoyl_domain::preferences::{
    welcomed_for_store, welcomed_from_store, AppIcon, DarkScheme, Density, Language, LightScheme,
    LookForUpdates, NewMailNotice, NoticeShows, Preferences, ShareAnonymousData, ShowPictures,
    Theme,
};
use lyonoyl_domain::window::{Pane, PaneWidth, WindowMode, WindowPlacement};

const KEY_THEME: &str = "appearance.theme";
const KEY_DENSITY: &str = "appearance.density";
const KEY_LANGUAGE: &str = "interface.language";
const KEY_DARK_SCHEME: &str = "appearance.darkScheme";
const KEY_LIGHT_SCHEME: &str = "appearance.lightScheme";
const KEY_APP_ICON: &str = "appearance.appIcon";
const KEY_NEW_MAIL_NOTICE: &str = "notice.newMail";
const KEY_SHOW_PICTURES: &str = "reading.showPictures";
const KEY_LOOK_FOR_UPDATES: &str = "updates.lookFor";
const KEY_SHARE_ANONYMOUS_DATA: &str = "privacy.shareAnonymousData";
const KEY_WELCOMED: &str = "onboarding.welcomed";
// One key per part rather than one holding three words. Each is read and written on its own, so a
// build that learns a fourth part cannot rewrite the three it already knew, and a row this build
// cannot parse costs the one switch it belongs to rather than all of them.
const KEY_NOTICE_SENDER: &str = "notice.showsSender";
const KEY_NOTICE_SUBJECT: &str = "notice.showsSubject";
const KEY_NOTICE_PREVIEW: &str = "notice.showsPreview";

/// Key-value storage for user preferences.
///
/// Adding a setting needs no migration, and a write touches only the keys it owns, so an older
/// build cannot erase keys it does not recognise.
pub struct SqlitePreferencesRepository {
    pool: SqlitePool,
}

impl SqlitePreferencesRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

fn theme_key(theme: Theme) -> &'static str {
    match theme {
        Theme::System => "system",
        Theme::Light => "light",
        Theme::Dark => "dark",
    }
}

fn density_key(density: Density) -> &'static str {
    match density {
        Density::Comfortable => "comfortable",
        Density::Compact => "compact",
    }
}

impl PreferencesRepository for SqlitePreferencesRepository {
    fn load(&self) -> Result<Preferences, SettingsError> {
        let connection = self.pool.get().map_err(|_| SettingsError::Unavailable)?;
        let mut statement = connection
            .prepare("SELECT key, value FROM preferences")
            .map_err(|_| SettingsError::Unavailable)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|_| SettingsError::Unavailable)?;

        let mut preferences = Preferences::default();
        for row in rows {
            let (key, value) = row.map_err(|_| SettingsError::Unavailable)?;
            // A value this build cannot parse was written by a newer version. Keeping the default
            // is correct: the row stays untouched on disk and the newer build still sees it.
            match key.as_str() {
                KEY_THEME => {
                    if let Ok(theme) = Theme::parse(&value) {
                        preferences = preferences.with_theme(theme);
                    }
                }
                KEY_DARK_SCHEME => {
                    if let Ok(scheme) = DarkScheme::parse(&value) {
                        preferences = preferences.with_dark_scheme(scheme);
                    }
                }
                KEY_LIGHT_SCHEME => {
                    if let Ok(scheme) = LightScheme::parse(&value) {
                        preferences = preferences.with_light_scheme(scheme);
                    }
                }
                KEY_APP_ICON => {
                    if let Ok(icon) = AppIcon::parse(&value) {
                        preferences = preferences.with_app_icon(icon);
                    }
                }
                KEY_DENSITY => {
                    if let Ok(density) = Density::parse(&value) {
                        preferences = preferences.with_density(density);
                    }
                }
                KEY_LANGUAGE => {
                    // A tag with no catalog in this build is kept as stored; only a malformed
                    // value falls back to following the system.
                    if let Ok(language) = Language::parse(&value) {
                        preferences = preferences.with_language(language);
                    }
                }
                KEY_NEW_MAIL_NOTICE => {
                    // No fallible parse: a value this build does not know is the default, which is
                    // "tell me", and being told once too often is the safer of the two mistakes.
                    preferences = preferences.with_new_mail_notice(NewMailNotice::parse(&value));
                }
                KEY_NOTICE_SENDER => {
                    let shows = preferences.notice_shows();
                    preferences = preferences.with_notice_shows(NoticeShows {
                        sender: NoticeShows::parse(&value),
                        ..shows
                    });
                }
                KEY_NOTICE_SUBJECT => {
                    let shows = preferences.notice_shows();
                    preferences = preferences.with_notice_shows(NoticeShows {
                        subject: NoticeShows::parse(&value),
                        ..shows
                    });
                }
                KEY_NOTICE_PREVIEW => {
                    let shows = preferences.notice_shows();
                    preferences = preferences.with_notice_shows(NoticeShows {
                        preview: NoticeShows::parse(&value),
                        ..shows
                    });
                }
                KEY_SHOW_PICTURES => {
                    // Same shape, opposite reason: the default here is the careful one, so a word
                    // this build cannot read never turns fetching on for somebody.
                    preferences = preferences.with_show_pictures(ShowPictures::parse(&value));
                }
                KEY_LOOK_FOR_UPDATES => {
                    // A word this build cannot read is nobody having been asked: nothing reaches
                    // out on the strength of it, and the window asks again.
                    preferences = preferences.with_look_for_updates(LookForUpdates::parse(&value));
                }
                KEY_SHARE_ANONYMOUS_DATA => {
                    // Only a stored yes shares: a word this build cannot read sends nothing.
                    preferences =
                        preferences.with_share_anonymous_data(ShareAnonymousData::parse(&value));
                }
                KEY_WELCOMED => {
                    // Only a stored yes skips it: a word this build cannot read greets them again.
                    preferences = preferences.with_welcomed(welcomed_from_store(&value));
                }
                _ => {}
            }
        }
        Ok(preferences)
    }

    fn save(&self, preferences: &Preferences) -> Result<(), SettingsError> {
        let mut connection = self.pool.get().map_err(|_| SettingsError::Unavailable)?;
        // BEGIN IMMEDIATE: this transaction writes, and a deferred one hits SQLITE_BUSY_SNAPSHOT
        // under a concurrent writer, which busy_timeout does not resolve.
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| SettingsError::Unavailable)?;

        for (key, value) in [
            (KEY_THEME, theme_key(preferences.theme())),
            (KEY_DARK_SCHEME, preferences.dark_scheme().as_str()),
            (KEY_LIGHT_SCHEME, preferences.light_scheme().as_str()),
            (KEY_APP_ICON, preferences.app_icon().as_str()),
            (KEY_DENSITY, density_key(preferences.density())),
            (KEY_LANGUAGE, preferences.language().as_str()),
            (KEY_NEW_MAIL_NOTICE, preferences.new_mail_notice().as_str()),
            (
                KEY_NOTICE_SENDER,
                NoticeShows::as_str(preferences.notice_shows().sender),
            ),
            (
                KEY_NOTICE_SUBJECT,
                NoticeShows::as_str(preferences.notice_shows().subject),
            ),
            (
                KEY_NOTICE_PREVIEW,
                NoticeShows::as_str(preferences.notice_shows().preview),
            ),
            (KEY_SHOW_PICTURES, preferences.show_pictures().as_str()),
            (
                KEY_LOOK_FOR_UPDATES,
                preferences.look_for_updates().as_str(),
            ),
            (
                KEY_SHARE_ANONYMOUS_DATA,
                preferences.share_anonymous_data().as_str(),
            ),
            (KEY_WELCOMED, welcomed_for_store(preferences.welcomed())),
        ] {
            transaction
                .execute(
                    "INSERT INTO preferences (key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    (key, value),
                )
                .map_err(|_| SettingsError::Unavailable)?;
        }

        transaction.commit().map_err(|_| SettingsError::Unavailable)
    }
}

/// Machine-local window geometry. One row, because there is one main window.
pub struct SqliteDeviceStateRepository {
    pool: SqlitePool,
}

impl SqliteDeviceStateRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

impl DeviceStateRepository for SqliteDeviceStateRepository {
    fn load_pane_widths(&self) -> Result<Vec<(Pane, PaneWidth)>, SettingsError> {
        let connection = self.pool.get().map_err(|_| SettingsError::Unavailable)?;

        let mut statement = connection
            .prepare("SELECT pane, width FROM device_pane_width")
            .map_err(|_| SettingsError::Unavailable)?;

        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|_| SettingsError::Unavailable)?;

        let mut widths = Vec::new();
        for row in rows {
            let (pane, width) = row.map_err(|_| SettingsError::Unavailable)?;
            // A pane this build does not know, or a width it cannot represent, is left out rather
            // than failing the read: the rest of the layout is still usable.
            if let Some(pane) = Pane::from_stored(&pane) {
                if let Ok(width) = u32::try_from(width) {
                    widths.push((pane, PaneWidth::clamped(width)));
                }
            }
        }

        Ok(widths)
    }

    fn save_pane_width(&self, pane: Pane, width: PaneWidth) -> Result<(), SettingsError> {
        self.pool
            .get()
            .map_err(|_| SettingsError::Unavailable)?
            .execute(
                "INSERT INTO device_pane_width (pane, width) VALUES (?1, ?2)
                 ON CONFLICT(pane) DO UPDATE SET width = excluded.width",
                (pane.stored(), i64::from(width.get())),
            )
            .map(|_| ())
            .map_err(|_| SettingsError::Unavailable)
    }

    fn load_folded(&self, pane: Pane) -> Result<bool, SettingsError> {
        self.pool
            .get()
            .map_err(|_| SettingsError::Unavailable)?
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM device_pane_folded WHERE pane = ?1)",
                [pane.stored()],
                |row| row.get(0),
            )
            .map_err(|_| SettingsError::Unavailable)
    }

    fn save_folded(&self, pane: Pane, folded: bool) -> Result<(), SettingsError> {
        // The row is the fold: written when the pane folds, and taken away when it opens.
        let statement = if folded {
            "INSERT INTO device_pane_folded (pane) VALUES (?1) ON CONFLICT(pane) DO NOTHING"
        } else {
            "DELETE FROM device_pane_folded WHERE pane = ?1"
        };

        self.pool
            .get()
            .map_err(|_| SettingsError::Unavailable)?
            .execute(statement, [pane.stored()])
            .map(|_| ())
            .map_err(|_| SettingsError::Unavailable)
    }

    fn load_window_placement(&self) -> Result<Option<WindowPlacement>, SettingsError> {
        let connection = self.pool.get().map_err(|_| SettingsError::Unavailable)?;
        let row = connection
            .query_row(
                "SELECT x, y, width, height, mode FROM device_window_state WHERE window = 'main'",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| SettingsError::Unavailable)?;

        let Some((x, y, width, height, mode)) = row else {
            return Ok(None);
        };

        // Geometry is machine-local and disposable: a corrupted row reads as "nothing stored" and
        // the window opens at its default size instead of failing startup.
        Ok(WindowPlacement::new(
            x as i32,
            y as i32,
            width.max(0) as u32,
            height.max(0) as u32,
            WindowMode::from_stored(mode),
        )
        .ok())
    }

    fn save_window_placement(&self, placement: &WindowPlacement) -> Result<(), SettingsError> {
        let connection = self.pool.get().map_err(|_| SettingsError::Unavailable)?;
        connection
            .execute(
                "INSERT INTO device_window_state (window, x, y, width, height, mode)
                 VALUES ('main', ?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(window) DO UPDATE SET
                     x = excluded.x, y = excluded.y,
                     width = excluded.width, height = excluded.height,
                     mode = excluded.mode",
                (
                    placement.x() as i64,
                    placement.y() as i64,
                    placement.width() as i64,
                    placement.height() as i64,
                    placement.mode().stored(),
                ),
            )
            .map(|_| ())
            .map_err(|_| SettingsError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::secure_storage::database_key::DatabaseKey;
    use crate::infrastructure::sqlite::{connection::open_profile, migrations::migrate_profile};
    use lyonoyl_domain::window::{MIN_HEIGHT, MIN_WIDTH};

    struct TempDatabase {
        dir: std::path::PathBuf,
        pool: SqlitePool,
    }

    impl TempDatabase {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "lyonoyl-settings-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            std::fs::create_dir_all(&dir).expect("temp dir");
            let path = dir.join("profile.sqlite3");
            let pool = open_profile(&path, &DatabaseKey::for_a_test()).expect("pool opens");
            let mut connection = pool.get().expect("connection");
            migrate_profile(&mut connection, &path.display().to_string()).expect("schema");
            drop(connection);
            Self { dir, pool }
        }
    }

    impl Drop for TempDatabase {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    #[test]
    fn an_empty_store_reads_as_defaults() {
        let db = TempDatabase::new("empty");
        let repository = SqlitePreferencesRepository::new(db.pool.clone());
        assert_eq!(repository.load().expect("load"), Preferences::default());
    }

    #[test]
    fn preferences_survive_a_round_trip() {
        let db = TempDatabase::new("roundtrip");
        let repository = SqlitePreferencesRepository::new(db.pool.clone());

        // Not one default among them. A round trip that stored what it would have read anyway
        // proves nothing about the key it is meant to be about.
        let wanted = Preferences::default()
            .with_theme(Theme::Dark)
            .with_dark_scheme(DarkScheme::Nord)
            .with_light_scheme(LightScheme::RosePineDawn)
            .with_app_icon(AppIcon::Rust)
            .with_density(Density::Compact)
            .with_language(Language::parse("pt-BR").expect("well formed"))
            .with_new_mail_notice(NewMailNotice::Never)
            .with_show_pictures(ShowPictures::Always)
            .with_look_for_updates(LookForUpdates::Never)
            .with_share_anonymous_data(ShareAnonymousData::Yes)
            .with_welcomed(true)
            // All three off, and each of them has to be. Left at its default, a key that is never
            // written reads back as that default and the round trip passes about a key it never
            // touched.
            .with_notice_shows(NoticeShows {
                sender: false,
                subject: false,
                preview: false,
            });
        repository.save(&wanted).expect("save");

        assert_eq!(repository.load().expect("load"), wanted);
    }

    #[test]
    fn a_light_scheme_written_by_a_newer_build_is_left_where_it_is() {
        // This build wears its own white, and the row is still there for the build that wrote it.
        let db = TempDatabase::new("light-unknown");
        db.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO preferences (key, value) VALUES ('appearance.lightScheme', 'midday-neon')",
                [],
            )
            .expect("a row written by another version");

        let read = SqlitePreferencesRepository::new(db.pool.clone())
            .load()
            .expect("load");
        assert_eq!(read.light_scheme(), LightScheme::Paper);

        let kept: String = db
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT value FROM preferences WHERE key = 'appearance.lightScheme'",
                [],
                |row| row.get(0),
            )
            .expect("the row");
        assert_eq!(kept, "midday-neon");
    }

    #[test]
    fn a_picture_written_by_a_newer_build_is_left_where_it_is() {
        let db = TempDatabase::new("icon-unknown");
        db.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO preferences (key, value) VALUES ('appearance.appIcon', 'neon')",
                [],
            )
            .expect("a row written by another version");

        let read = SqlitePreferencesRepository::new(db.pool.clone())
            .load()
            .expect("load");
        assert_eq!(read.app_icon(), AppIcon::Ember);

        let kept: String = db
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT value FROM preferences WHERE key = 'appearance.appIcon'",
                [],
                |row| row.get(0),
            )
            .expect("the row");
        assert_eq!(kept, "neon");
    }

    #[test]
    fn a_notice_setting_written_by_a_newer_build_does_not_stop_this_one() {
        // The row stays on disk untouched, so the newer build still sees what it wrote; this one
        // carries on with the default rather than refusing to start.
        let db = TempDatabase::new("notice-unknown");
        db.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO preferences (key, value) VALUES ('notice.newMail', 'when-i-whistle')",
                [],
            )
            .expect("a row written by another version");

        let read = SqlitePreferencesRepository::new(db.pool.clone())
            .load()
            .expect("load");

        assert_eq!(read.new_mail_notice(), NewMailNotice::Inbox);
    }

    #[test]
    fn one_part_of_a_notice_can_be_turned_off_without_the_others() {
        // Three keys and not one word holding three answers. Stored together, turning off the
        // sender would rewrite the other two, and a build that learned a fourth part would rewrite
        // the three it already knew.
        let db = TempDatabase::new("notice-parts");
        let repository = SqlitePreferencesRepository::new(db.pool.clone());

        repository
            .save(&Preferences::default().with_notice_shows(NoticeShows {
                sender: false,
                subject: true,
                preview: true,
            }))
            .expect("save");

        let read = repository.load().expect("load").notice_shows();

        assert!(!read.sender, "the one that was turned off");
        assert!(read.subject, "and the two that were not");
        assert!(read.preview);
    }

    #[test]
    fn a_part_of_a_notice_this_build_cannot_read_stays_shown() {
        // The row stays on disk untouched, so the build that wrote it still sees what it wrote.
        // Only the word for off turns a part off, so a value from another build can never hide
        // something somebody asked to see — it can only fail to hide something.
        let db = TempDatabase::new("notice-part-unknown");
        db.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO preferences (key, value) VALUES ('notice.showsSender', 'on-tuesdays')",
                [],
            )
            .expect("a row written by another version");

        let read = SqlitePreferencesRepository::new(db.pool.clone())
            .load()
            .expect("load");

        assert!(read.notice_shows().sender);
    }

    #[test]
    fn a_picture_setting_this_build_cannot_read_never_turns_fetching_on() {
        // The one place in this file where the fallback is a decision rather than a convenience.
        // A word another build wrote must not make this one go out to the hosts a message names on
        // somebody's behalf: unreadable reads as "ask me", which is a button and not a request.
        let db = TempDatabase::new("pictures-unknown");
        db.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO preferences (key, value) VALUES ('reading.showPictures', 'on-tuesdays')",
                [],
            )
            .expect("a row written by another version");

        let read = SqlitePreferencesRepository::new(db.pool.clone())
            .load()
            .expect("load");

        assert_eq!(read.show_pictures(), ShowPictures::WhenAsked);
    }

    #[test]
    fn an_answer_about_updates_this_build_cannot_read_reaches_out_to_nobody() {
        // Read as nobody having been asked: nothing is looked for on the strength of a word another
        // build wrote, and the window asks again.
        let db = TempDatabase::new("updates-unknown");
        db.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO preferences (key, value) VALUES ('updates.lookFor', 'hourly')",
                [],
            )
            .expect("a row written by another version");

        let read = SqlitePreferencesRepository::new(db.pool.clone())
            .load()
            .expect("load");

        assert_eq!(read.look_for_updates(), LookForUpdates::NotAsked);
        assert!(!read.look_for_updates().looks_on_its_own());
    }

    #[test]
    fn an_answer_about_sharing_this_build_cannot_read_shares_nothing() {
        let db = TempDatabase::new("sharing-unknown");
        db.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO preferences (key, value) VALUES ('privacy.shareAnonymousData', 'everything')",
                [],
            )
            .expect("a row written by another version");

        let read = SqlitePreferencesRepository::new(db.pool.clone())
            .load()
            .expect("load");

        assert_eq!(read.share_anonymous_data(), ShareAnonymousData::No);
    }

    #[test]
    fn a_word_about_the_onboarding_this_build_cannot_read_greets_again() {
        let db = TempDatabase::new("welcomed-unknown");
        db.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO preferences (key, value) VALUES ('onboarding.welcomed', 'twice')",
                [],
            )
            .expect("a row written by another version");

        let read = SqlitePreferencesRepository::new(db.pool.clone())
            .load()
            .expect("load");

        assert!(!read.welcomed());
    }

    #[test]
    fn saving_twice_updates_in_place_instead_of_duplicating() {
        let db = TempDatabase::new("upsert");
        let repository = SqlitePreferencesRepository::new(db.pool.clone());

        let rows = || -> i64 {
            db.pool
                .get()
                .expect("connection")
                .query_row("SELECT count(*) FROM preferences", [], |row| row.get(0))
                .expect("count")
        };

        repository
            .save(&Preferences::default().with_theme(Theme::Light))
            .expect("first");
        let after_one = rows();

        repository
            .save(&Preferences::default().with_theme(Theme::Dark))
            .expect("second");

        // Said as the rule rather than as a number: adding a setting should not fail a test about
        // whether saving twice writes twice.
        assert!(after_one > 0, "the first save wrote nothing");
        assert_eq!(
            rows(),
            after_one,
            "saving again added rows instead of updating them"
        );
        assert_eq!(repository.load().expect("load").theme(), Theme::Dark);
    }

    #[test]
    fn a_key_written_by_a_newer_version_is_preserved_not_erased() {
        let db = TempDatabase::new("forward-compat");
        let repository = SqlitePreferencesRepository::new(db.pool.clone());

        db.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO preferences (key, value) VALUES ('appearance.accent', 'violet')",
                [],
            )
            .expect("a future build wrote this");

        repository
            .save(&Preferences::default().with_theme(Theme::Dark))
            .expect("save");

        let survived: String = db
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT value FROM preferences WHERE key = 'appearance.accent'",
                [],
                |row| row.get(0),
            )
            .expect("unknown key must still be there");
        assert_eq!(
            survived, "violet",
            "an older build must not destroy newer settings"
        );
    }

    #[test]
    fn a_language_with_no_catalog_in_this_build_round_trips_unchanged() {
        // A profile written on a build with a Welsh catalog reads back unchanged on one without
        // it.
        let db = TempDatabase::new("exotic-language");
        let repository = SqlitePreferencesRepository::new(db.pool.clone());

        let welsh = Language::parse("cy-GB").expect("well formed");
        repository
            .save(&Preferences::default().with_language(welsh.clone()))
            .expect("save");

        assert_eq!(repository.load().expect("load").language(), &welsh);
    }

    #[test]
    fn a_malformed_stored_language_falls_back_to_following_the_system() {
        let db = TempDatabase::new("malformed-language");
        let repository = SqlitePreferencesRepository::new(db.pool.clone());

        db.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO preferences (key, value) VALUES ('interface.language', 'en_US')",
                [],
            )
            .expect("seed a value no build of ours could have written");

        assert_eq!(
            repository.load().expect("load").language(),
            &Language::System,
            "an unreadable language must not fail the load"
        );
    }

    #[test]
    fn an_unparsable_value_falls_back_to_the_default_without_failing() {
        let db = TempDatabase::new("unparsable");
        let repository = SqlitePreferencesRepository::new(db.pool.clone());

        db.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO preferences (key, value) VALUES ('appearance.theme', 'solarized')",
                [],
            )
            .expect("seed");

        assert_eq!(repository.load().expect("load").theme(), Theme::System);
    }

    #[test]
    fn window_placement_survives_a_round_trip() {
        let db = TempDatabase::new("window");
        let repository = SqliteDeviceStateRepository::new(db.pool.clone());

        assert!(repository.load_window_placement().expect("load").is_none());

        let placement = WindowPlacement::new(
            120,
            80,
            MIN_WIDTH + 40,
            MIN_HEIGHT + 20,
            WindowMode::Fullscreen,
        )
        .expect("valid");
        repository.save_window_placement(&placement).expect("save");

        assert_eq!(
            repository.load_window_placement().expect("load"),
            Some(placement)
        );
    }

    #[test]
    fn a_pane_nobody_folded_is_open() {
        let db = TempDatabase::new("never-folded");
        let repository = SqliteDeviceStateRepository::new(db.pool.clone());

        assert!(!repository.load_folded(Pane::Sidebar).expect("load"));
    }

    #[test]
    fn a_fold_survives_a_round_trip_both_ways() {
        let db = TempDatabase::new("folded");
        let repository = SqliteDeviceStateRepository::new(db.pool.clone());

        repository.save_folded(Pane::Sidebar, true).expect("fold");
        assert!(repository.load_folded(Pane::Sidebar).expect("load"));

        repository.save_folded(Pane::Sidebar, false).expect("open");
        assert!(!repository.load_folded(Pane::Sidebar).expect("load"));
    }

    #[test]
    fn folding_twice_keeps_one_row_and_opening_twice_keeps_none() {
        let db = TempDatabase::new("folded-twice");
        let repository = SqliteDeviceStateRepository::new(db.pool.clone());
        let rows = || -> i64 {
            db.pool
                .get()
                .expect("connection")
                .query_row("SELECT count(*) FROM device_pane_folded", [], |row| {
                    row.get(0)
                })
                .expect("count")
        };

        repository.save_folded(Pane::Sidebar, true).expect("fold");
        repository
            .save_folded(Pane::Sidebar, true)
            .expect("fold again");
        assert_eq!(rows(), 1);
        assert!(repository.load_folded(Pane::Sidebar).expect("load"));

        repository.save_folded(Pane::Sidebar, false).expect("open");
        repository
            .save_folded(Pane::Sidebar, false)
            .expect("open again");
        assert_eq!(rows(), 0);
    }

    #[test]
    fn one_pane_folded_leaves_the_other_open() {
        let db = TempDatabase::new("folded-one");
        let repository = SqliteDeviceStateRepository::new(db.pool.clone());

        repository.save_folded(Pane::Sidebar, true).expect("fold");

        assert!(!repository.load_folded(Pane::List).expect("load"));
    }

    #[test]
    fn corrupted_geometry_reads_as_absent_rather_than_failing_startup() {
        let db = TempDatabase::new("corrupt-window");
        let repository = SqliteDeviceStateRepository::new(db.pool.clone());

        db.pool
            .get()
            .expect("connection")
            .execute(
                "INSERT INTO device_window_state (window, x, y, width, height, mode)
                 VALUES ('main', 0, 0, 10, 10, 0)",
                [],
            )
            .expect("seed a window smaller than the usable minimum");

        assert!(
            repository.load_window_placement().expect("load").is_none(),
            "the app must still start, just at its default size"
        );
    }
}
