use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;

use crate::application::error::UseCaseError;
use crate::application::settings::{GetPreferences, PreferenceChange, UpdatePreferences};
use crate::bootstrap::adapters;
use crate::commands::error::{CommandError, ErrorCode};
use crate::ports::settings_repository::DeviceStateRepository;
use crate::AppState;
use lyonoyl_domain::preferences::{
    AppIcon, DarkScheme, Density, Language, LightScheme, LookForUpdates, NewMailNotice,
    NoticeShows, Preferences, ShareAnonymousData, ShowPictures, Theme,
};
use lyonoyl_domain::window::{Pane, PaneWidth};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum ThemeDto {
    System,
    Light,
    Dark,
}

/// Whether the system is told that mail arrived.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum NewMailNoticeDto {
    Inbox,
    Never,
}

impl From<NewMailNoticeDto> for NewMailNotice {
    fn from(dto: NewMailNoticeDto) -> Self {
        match dto {
            NewMailNoticeDto::Inbox => Self::Inbox,
            NewMailNoticeDto::Never => Self::Never,
        }
    }
}

impl From<NewMailNotice> for NewMailNoticeDto {
    fn from(notice: NewMailNotice) -> Self {
        match notice {
            NewMailNotice::Inbox => Self::Inbox,
            NewMailNotice::Never => Self::Never,
        }
    }
}

/// Which colours the interface wears when it is dark.
///
/// Named after the schemes people already know from their editors, so somebody with one in mind
/// can find it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum DarkSchemeDto {
    Graphite,
    Dracula,
    Nord,
    SolarizedDark,
    GruvboxDark,
    OneDark,
}

/// Which colours the interface wears when it is light, named the same way.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum LightSchemeDto {
    Paper,
    SolarizedLight,
    GruvboxLight,
    CatppuccinLatte,
    RosePineDawn,
}

/// The picture the application shows itself as while it runs, named the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum AppIconDto {
    Ink,
    Indigo,
    Dusk,
    Plum,
    Graphite,
    Sky,
    Paper,
    Ember,
    Rust,
}

impl From<AppIconDto> for AppIcon {
    fn from(dto: AppIconDto) -> Self {
        match dto {
            AppIconDto::Ink => Self::Ink,
            AppIconDto::Indigo => Self::Indigo,
            AppIconDto::Dusk => Self::Dusk,
            AppIconDto::Plum => Self::Plum,
            AppIconDto::Graphite => Self::Graphite,
            AppIconDto::Sky => Self::Sky,
            AppIconDto::Paper => Self::Paper,
            AppIconDto::Ember => Self::Ember,
            AppIconDto::Rust => Self::Rust,
        }
    }
}

impl From<AppIcon> for AppIconDto {
    fn from(icon: AppIcon) -> Self {
        match icon {
            AppIcon::Ink => Self::Ink,
            AppIcon::Indigo => Self::Indigo,
            AppIcon::Dusk => Self::Dusk,
            AppIcon::Plum => Self::Plum,
            AppIcon::Graphite => Self::Graphite,
            AppIcon::Sky => Self::Sky,
            AppIcon::Paper => Self::Paper,
            AppIcon::Ember => Self::Ember,
            AppIcon::Rust => Self::Rust,
        }
    }
}

/// Whether a message fetches what it points at elsewhere without being asked.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum ShowPicturesDto {
    WhenAsked,
    Always,
}

impl From<ShowPicturesDto> for ShowPictures {
    fn from(dto: ShowPicturesDto) -> Self {
        match dto {
            ShowPicturesDto::WhenAsked => Self::WhenAsked,
            ShowPicturesDto::Always => Self::Always,
        }
    }
}

impl From<ShowPictures> for ShowPicturesDto {
    fn from(pictures: ShowPictures) -> Self {
        match pictures {
            ShowPictures::WhenAsked => Self::WhenAsked,
            ShowPictures::Always => Self::Always,
        }
    }
}

/// Whether this copy looks for a newer version on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum LookForUpdatesDto {
    NotAsked,
    OnceADay,
    Never,
}

impl From<LookForUpdatesDto> for LookForUpdates {
    fn from(dto: LookForUpdatesDto) -> Self {
        match dto {
            LookForUpdatesDto::NotAsked => Self::NotAsked,
            LookForUpdatesDto::OnceADay => Self::OnceADay,
            LookForUpdatesDto::Never => Self::Never,
        }
    }
}

impl From<LookForUpdates> for LookForUpdatesDto {
    fn from(look: LookForUpdates) -> Self {
        match look {
            LookForUpdates::NotAsked => Self::NotAsked,
            LookForUpdates::OnceADay => Self::OnceADay,
            LookForUpdates::Never => Self::Never,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum DensityDto {
    Comfortable,
    Compact,
}

/// `"system"` or a language tag such as `"en"` or `"pt-br"`.
///
/// A bare string rather than an enum of shipped languages: the set of catalogs is decided in the
/// frontend, so adding one needs no change here.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct LanguageDto(pub String);

#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct PreferencesDto {
    pub theme: ThemeDto,
    pub dark_scheme: DarkSchemeDto,
    pub light_scheme: LightSchemeDto,
    pub app_icon: AppIconDto,
    pub density: DensityDto,
    pub language: LanguageDto,
    pub new_mail_notice: NewMailNoticeDto,
    pub notice_shows: NoticeShowsDto,
    pub show_pictures: ShowPicturesDto,
    pub look_for_updates: LookForUpdatesDto,
    /// Whether anonymous reports about failures are shared: a yes or a no, one box to tick.
    pub share_anonymous_data: bool,
    /// Whether the onboarding has been finished, so the window does not show it again.
    pub welcomed: bool,
}

/// What a notification may say about the mail it announces.
///
/// The three travel together, because they are one answer to one question and the screen shows
/// them as one group. Sent whole on every change: the window is handed the full set back after any
/// preference is changed, so it always holds the other two.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct NoticeShowsDto {
    pub sender: bool,
    pub subject: bool,
    pub preview: bool,
}

impl From<NoticeShows> for NoticeShowsDto {
    fn from(shows: NoticeShows) -> Self {
        Self {
            sender: shows.sender,
            subject: shows.subject,
            preview: shows.preview,
        }
    }
}

impl From<NoticeShowsDto> for NoticeShows {
    fn from(dto: NoticeShowsDto) -> Self {
        Self {
            sender: dto.sender,
            subject: dto.subject,
            preview: dto.preview,
        }
    }
}

impl From<Preferences> for PreferencesDto {
    fn from(preferences: Preferences) -> Self {
        Self {
            new_mail_notice: preferences.new_mail_notice().into(),
            notice_shows: preferences.notice_shows().into(),
            show_pictures: preferences.show_pictures().into(),
            look_for_updates: preferences.look_for_updates().into(),
            share_anonymous_data: preferences.share_anonymous_data().shares(),
            app_icon: preferences.app_icon().into(),
            welcomed: preferences.welcomed(),
            theme: match preferences.theme() {
                Theme::System => ThemeDto::System,
                Theme::Light => ThemeDto::Light,
                Theme::Dark => ThemeDto::Dark,
            },
            dark_scheme: match preferences.dark_scheme() {
                DarkScheme::Graphite => DarkSchemeDto::Graphite,
                DarkScheme::Dracula => DarkSchemeDto::Dracula,
                DarkScheme::Nord => DarkSchemeDto::Nord,
                DarkScheme::SolarizedDark => DarkSchemeDto::SolarizedDark,
                DarkScheme::GruvboxDark => DarkSchemeDto::GruvboxDark,
                DarkScheme::OneDark => DarkSchemeDto::OneDark,
            },
            light_scheme: match preferences.light_scheme() {
                LightScheme::Paper => LightSchemeDto::Paper,
                LightScheme::SolarizedLight => LightSchemeDto::SolarizedLight,
                LightScheme::GruvboxLight => LightSchemeDto::GruvboxLight,
                LightScheme::CatppuccinLatte => LightSchemeDto::CatppuccinLatte,
                LightScheme::RosePineDawn => LightSchemeDto::RosePineDawn,
            },
            density: match preferences.density() {
                Density::Comfortable => DensityDto::Comfortable,
                Density::Compact => DensityDto::Compact,
            },
            language: LanguageDto(preferences.language().as_str().to_owned()),
        }
    }
}

impl From<DarkSchemeDto> for DarkScheme {
    fn from(dto: DarkSchemeDto) -> Self {
        match dto {
            DarkSchemeDto::Graphite => Self::Graphite,
            DarkSchemeDto::Dracula => Self::Dracula,
            DarkSchemeDto::Nord => Self::Nord,
            DarkSchemeDto::SolarizedDark => Self::SolarizedDark,
            DarkSchemeDto::GruvboxDark => Self::GruvboxDark,
            DarkSchemeDto::OneDark => Self::OneDark,
        }
    }
}

impl From<LightSchemeDto> for LightScheme {
    fn from(dto: LightSchemeDto) -> Self {
        match dto {
            LightSchemeDto::Paper => Self::Paper,
            LightSchemeDto::SolarizedLight => Self::SolarizedLight,
            LightSchemeDto::GruvboxLight => Self::GruvboxLight,
            LightSchemeDto::CatppuccinLatte => Self::CatppuccinLatte,
            LightSchemeDto::RosePineDawn => Self::RosePineDawn,
        }
    }
}

impl From<ThemeDto> for Theme {
    fn from(dto: ThemeDto) -> Self {
        match dto {
            ThemeDto::System => Self::System,
            ThemeDto::Light => Self::Light,
            ThemeDto::Dark => Self::Dark,
        }
    }
}

impl From<DensityDto> for Density {
    fn from(dto: DensityDto) -> Self {
        match dto {
            DensityDto::Comfortable => Self::Comfortable,
            DensityDto::Compact => Self::Compact,
        }
    }
}

/// How wide each pane was left. A pane with no stored width is absent, and opens at its default.
#[derive(Debug, Clone, Copy, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct PaneWidthsDto {
    pub sidebar: Option<u32>,
    pub list: Option<u32>,
}

/// Which pane a width belongs to.
#[derive(Debug, Clone, Copy, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum PaneDto {
    Sidebar,
    List,
}

impl From<PaneDto> for Pane {
    fn from(pane: PaneDto) -> Self {
        match pane {
            PaneDto::Sidebar => Self::Sidebar,
            PaneDto::List => Self::List,
        }
    }
}

#[tauri::command]
pub async fn get_pane_widths(state: State<'_, AppState>) -> Result<PaneWidthsDto, CommandError> {
    let pool = state.storage.profile()?;

    let stored = tauri::async_runtime::spawn_blocking(move || {
        adapters::device_state(pool).load_pane_widths()
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
    .map_err(UseCaseError::from)?;

    let width_of = |wanted: Pane| {
        stored
            .iter()
            .find(|(pane, _)| *pane == wanted)
            .map(|(_, width)| width.get())
    };

    Ok(PaneWidthsDto {
        sidebar: width_of(Pane::Sidebar),
        list: width_of(Pane::List),
    })
}

/// Stores how wide a pane was dragged. Clamped, so a pane cannot be left too narrow to grab again.
#[tauri::command]
pub async fn set_pane_width(
    state: State<'_, AppState>,
    pane: PaneDto,
    width: u32,
) -> Result<(), CommandError> {
    let pool = state.storage.profile()?;

    tauri::async_runtime::spawn_blocking(move || {
        adapters::device_state(pool).save_pane_width(pane.into(), PaneWidth::clamped(width))
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
    .map_err(UseCaseError::from)?;

    Ok(())
}

/// Whether the sidebar was left folded into its rail. A start with nothing stored opens it.
#[tauri::command]
pub async fn get_sidebar_folded(state: State<'_, AppState>) -> Result<bool, CommandError> {
    let pool = state.storage.profile()?;

    let folded = tauri::async_runtime::spawn_blocking(move || {
        adapters::device_state(pool).load_folded(Pane::Sidebar)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
    .map_err(UseCaseError::from)?;

    Ok(folded)
}

/// Stores whether the sidebar is folded into its rail.
#[tauri::command]
pub async fn set_sidebar_folded(
    state: State<'_, AppState>,
    folded: bool,
) -> Result<(), CommandError> {
    let pool = state.storage.profile()?;

    tauri::async_runtime::spawn_blocking(move || {
        adapters::device_state(pool).save_folded(Pane::Sidebar, folded)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))?
    .map_err(UseCaseError::from)?;

    Ok(())
}

#[tauri::command]
pub async fn get_preferences(state: State<'_, AppState>) -> Result<PreferencesDto, CommandError> {
    let pool = state.storage.profile()?;
    let preferences = tauri::async_runtime::spawn_blocking(move || {
        GetPreferences::new(adapters::preferences(pool)).execute()
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))??;

    Ok(preferences.into())
}

/// The one preference a call changes.
///
/// One argument that can hold exactly one answer, rather than one optional argument per setting
/// and a rule beside them saying that exactly one must be supplied. Written the other way, the
/// rule is something a caller can break, a check that has to exist to catch it, and a signature
/// that grows by one every time a setting is added.
///
/// Serde writes it externally tagged, which is the shape the window sends: `{"theme": "dark"}` is
/// one variant and `{}` is none, so both mistakes are refused before anything in this file runs.
#[derive(Debug, Clone, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum PreferenceChangeDto {
    Theme(ThemeDto),
    DarkScheme(DarkSchemeDto),
    LightScheme(LightSchemeDto),
    AppIcon(AppIconDto),
    Density(DensityDto),
    Language(LanguageDto),
    NewMailNotice(NewMailNoticeDto),
    NoticeShows(NoticeShowsDto),
    ShowPictures(ShowPicturesDto),
    LookForUpdates(LookForUpdatesDto),
    ShareAnonymousData(bool),
    Welcomed(bool),
}

impl TryFrom<PreferenceChangeDto> for PreferenceChange {
    type Error = CommandError;

    fn try_from(change: PreferenceChangeDto) -> Result<Self, Self::Error> {
        Ok(match change {
            PreferenceChangeDto::Theme(theme) => Self::Theme(theme.into()),
            PreferenceChangeDto::DarkScheme(scheme) => Self::DarkScheme(scheme.into()),
            PreferenceChangeDto::LightScheme(scheme) => Self::LightScheme(scheme.into()),
            PreferenceChangeDto::AppIcon(icon) => Self::AppIcon(icon.into()),
            PreferenceChangeDto::Density(density) => Self::Density(density.into()),
            // The settings screen picks from the catalogs it ships, so a malformed tag is a caller
            // error rather than a user-facing failure.
            PreferenceChangeDto::Language(tag) => {
                Self::Language(Language::parse(&tag.0).map_err(|_| {
                    CommandError::new(
                        ErrorCode::InvalidRequest,
                        "Language tag is not well formed.",
                    )
                })?)
            }
            PreferenceChangeDto::NewMailNotice(notice) => Self::NewMailNotice(notice.into()),
            PreferenceChangeDto::NoticeShows(shows) => Self::NoticeShows(shows.into()),
            PreferenceChangeDto::ShowPictures(pictures) => Self::ShowPictures(pictures.into()),
            PreferenceChangeDto::LookForUpdates(look) => Self::LookForUpdates(look.into()),
            PreferenceChangeDto::ShareAnonymousData(share) => Self::ShareAnonymousData(if share {
                ShareAnonymousData::Yes
            } else {
                ShareAnonymousData::No
            }),
            PreferenceChangeDto::Welcomed(welcomed) => Self::Welcomed(welcomed),
        })
    }
}

/// Applies one preference change and returns the full resulting state, so the UI never has to
/// reconstruct it locally and cannot drift from what was actually stored.
///
/// The application's picture is put on again after every change: a new picture changes what it
/// wears, and putting on the one it already wears changes nothing.
#[tauri::command]
pub async fn set_preference(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    change: PreferenceChangeDto,
) -> Result<PreferencesDto, CommandError> {
    let change = PreferenceChange::try_from(change)?;

    let pool = state.storage.profile()?;
    let preferences = tauri::async_runtime::spawn_blocking(move || {
        UpdatePreferences::new(adapters::preferences(pool)).execute(change)
    })
    .await
    .map_err(|_| CommandError::new(ErrorCode::Internal, "A background task failed."))??;

    crate::infrastructure::app_icon::wear(&app, &preferences);
    Ok(preferences.into())
}

/// Whether this system lets the running application change the picture it shows itself as, which
/// is whether the window offers the choice at all.
#[tauri::command]
pub async fn app_icons_wearable() -> Result<bool, CommandError> {
    Ok(crate::infrastructure::app_icon::WEARABLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_crosses_the_wire_as_a_bare_string() {
        // The generated TS says `type LanguageDto = string`. If serde ever wrapped the newtype,
        // the frontend would receive {"0":"ru"} and every comparison against a tag would fail.
        let encoded = serde_json::to_string(&LanguageDto("ru".to_owned())).expect("serializes");
        assert_eq!(encoded, "\"ru\"");

        let decoded: LanguageDto = serde_json::from_str("\"pt-br\"").expect("deserializes");
        assert_eq!(decoded.0, "pt-br");
    }

    #[test]
    fn a_call_can_carry_one_preference_and_cannot_carry_none() {
        // The rule is the shape rather than a check beside it: an externally tagged enum holds
        // exactly one variant, so "none" and "two" are refused before anything in this file runs.
        // Written as one optional argument per setting, each of these would be a call that
        // compiles, reaches the store, and does something nobody asked for.
        let one: PreferenceChangeDto =
            serde_json::from_str(r#"{"showPictures":"always"}"#).expect("one preference");
        assert!(matches!(
            PreferenceChange::try_from(one).expect("a change"),
            PreferenceChange::ShowPictures(ShowPictures::Always)
        ));

        let looking: PreferenceChangeDto =
            serde_json::from_str(r#"{"lookForUpdates":"onceADay"}"#).expect("one preference");
        assert!(matches!(
            PreferenceChange::try_from(looking).expect("a change"),
            PreferenceChange::LookForUpdates(LookForUpdates::OnceADay)
        ));

        // Nothing at all. A call that changed nothing and answered with the settings anyway would
        // look to the screen exactly like one that worked.
        assert!(serde_json::from_str::<PreferenceChangeDto>("{}").is_err());

        // Two. Only one of them could be applied, and which one would be decided by the order the
        // fields happen to be written in.
        assert!(serde_json::from_str::<PreferenceChangeDto>(
            r#"{"theme":"dark","density":"compact"}"#
        )
        .is_err());

        // The three parts of a notice travel as one, so they are one change and not three.
        let parts: PreferenceChangeDto = serde_json::from_str(
            r#"{"noticeShows":{"sender":true,"subject":false,"preview":true}}"#,
        )
        .expect("one preference");
        assert!(matches!(
            PreferenceChange::try_from(parts).expect("a change"),
            PreferenceChange::NoticeShows(NoticeShows {
                sender: true,
                subject: false,
                preview: true
            })
        ));
    }

    /// Every light scheme reaches the window under the name the stylesheet matches and the store
    /// keeps, and comes back from it as itself.
    #[test]
    fn the_answer_about_sharing_travels_as_a_plain_yes_or_no() {
        for (sent, meant) in [
            ("true", ShareAnonymousData::Yes),
            ("false", ShareAnonymousData::No),
        ] {
            let change: PreferenceChangeDto =
                serde_json::from_str(&format!(r#"{{"shareAnonymousData":{sent}}}"#))
                    .expect("one preference");
            assert!(
                matches!(
                    PreferenceChange::try_from(change).expect("a change"),
                    PreferenceChange::ShareAnonymousData(answer) if answer == meant
                ),
                "{sent}"
            );
        }

        // A word is not a yes: only the window's own true shares anything.
        assert!(
            serde_json::from_str::<PreferenceChangeDto>(r#"{"shareAnonymousData":"yes"}"#).is_err()
        );

        assert!(
            PreferencesDto::from(
                Preferences::default().with_share_anonymous_data(ShareAnonymousData::Yes)
            )
            .share_anonymous_data
        );
        assert!(!PreferencesDto::from(Preferences::default()).share_anonymous_data);
    }

    #[test]
    fn finishing_the_onboarding_travels_as_a_plain_yes_or_no() {
        let change: PreferenceChangeDto =
            serde_json::from_str(r#"{"welcomed":true}"#).expect("one preference");
        assert!(matches!(
            PreferenceChange::try_from(change).expect("a change"),
            PreferenceChange::Welcomed(true)
        ));
        assert!(serde_json::from_str::<PreferenceChangeDto>(r#"{"welcomed":"yes"}"#).is_err());

        assert!(PreferencesDto::from(Preferences::default().with_welcomed(true)).welcomed);
        assert!(!PreferencesDto::from(Preferences::default()).welcomed);
    }

    #[test]
    fn every_picture_travels_to_the_window_and_back_as_itself() {
        for icon in AppIcon::ALL {
            let outward: PreferencesDto = Preferences::default().with_app_icon(icon).into();
            let named = serde_json::to_string(&outward.app_icon).expect("a name");
            assert_eq!(named, format!("\"{}\"", icon.as_str()), "{icon:?}");

            let inward: PreferenceChangeDto =
                serde_json::from_str(&format!(r#"{{"appIcon":{named}}}"#)).expect("a change");
            assert!(
                matches!(
                    PreferenceChange::try_from(inward).expect("a change"),
                    PreferenceChange::AppIcon(back) if back == icon
                ),
                "{icon:?} came back as something else"
            );
        }
    }

    #[test]
    fn every_light_scheme_travels_to_the_window_and_back_as_itself() {
        for scheme in LightScheme::ALL {
            let outward: PreferencesDto = Preferences::default().with_light_scheme(scheme).into();
            let named = serde_json::to_string(&outward.light_scheme).expect("a name");
            assert_eq!(named, format!("\"{}\"", scheme.as_str()), "{scheme:?}");

            let inward: PreferenceChangeDto =
                serde_json::from_str(&format!(r#"{{"lightScheme":{named}}}"#)).expect("a change");
            assert!(
                matches!(
                    PreferenceChange::try_from(inward).expect("a change"),
                    PreferenceChange::LightScheme(back) if back == scheme
                ),
                "{scheme:?} came back as something else"
            );
        }
    }

    #[test]
    fn a_language_tag_that_is_not_one_is_refused_rather_than_stored() {
        let malformed: PreferenceChangeDto =
            serde_json::from_str(r#"{"language":"not a tag at all"}"#).expect("shape is fine");

        assert!(PreferenceChange::try_from(malformed).is_err());
    }

    #[test]
    fn preferences_carry_whether_pictures_are_fetched_outward() {
        let dto: PreferencesDto = Preferences::default()
            .with_show_pictures(ShowPictures::Always)
            .into();
        assert!(matches!(dto.show_pictures, ShowPicturesDto::Always));

        // And the default is the careful one, which is what a fresh install wears.
        let fresh: PreferencesDto = Preferences::default().into();
        assert!(matches!(fresh.show_pictures, ShowPicturesDto::WhenAsked));
    }

    /// A problem or a question goes to the product's own site, and the address carries nothing
    /// about anybody. Written out here rather than built the way the code builds it.
    #[test]
    fn a_report_goes_to_the_products_own_site() {
        assert_eq!(SUPPORT, "https://lyonoyl.com/support");
    }

    #[test]
    fn preferences_carry_the_language_outward() {
        let preferences =
            Preferences::default().with_language(Language::parse("ru").expect("well formed"));
        let dto: PreferencesDto = preferences.into();
        assert_eq!(dto.language.0, "ru");
    }
}

/// Opens the folder the journal writes into, in whatever the system uses to look at folders.
///
/// **No path crosses the boundary, in either direction.** The window asks for the folder rather
/// than naming it, and the folder is the one this run is writing to — which the window has no way
/// of knowing and no business deciding. Invariant 12 is about the other direction, and the reason
/// is the same one: a path is not a thing to be told.
///
/// It exists because a log nobody can find is a log nobody sends, and the folder it is in sits
/// inside one the system hides by default on two of the three platforms.
#[tauri::command]
pub fn show_where_the_journal_writes(app: tauri::AppHandle) -> Result<(), CommandError> {
    use tauri_plugin_opener::OpenerExt;

    let directory = crate::bootstrap::data_directory(&app)
        .map(|beside| beside.join(crate::infrastructure::journal::files::DIRECTORY))
        .map_err(|_| {
            CommandError::new(
                ErrorCode::Internal,
                "This device has no data directory to write into.",
            )
        })?;

    // Made rather than assumed: a run that has written nothing yet, or one whose folder somebody
    // has since removed, must still be able to show where it would write.
    let _ = std::fs::create_dir_all(&directory);

    app.opener()
        .open_path(directory.to_string_lossy(), None::<&str>)
        .map_err(|_| {
            CommandError::new(
                ErrorCode::Internal,
                "The system would not open that folder.",
            )
        })
}

/// The languages the person set on this system, in the order they set them.
///
/// Asked of the system rather than of the web view, which reports the first and nothing after it:
/// somebody whose first language has no catalogue here is read their second rather than English.
/// Tags only, and no more than a list a person keeps — which of them has a catalogue is the
/// window's business.
#[tauri::command]
pub fn system_languages() -> Vec<String> {
    sys_locale::get_locales().take(16).collect()
}

/// Where a problem or a question goes: the project's site, which says where to take it from there.
///
/// Written here rather than handed over by the window, for the reason a path never is: a window
/// that could name what to open is a window that could be asked to open something else. It is the
/// product's own address and names nobody.
pub const SUPPORT: &str = "https://lyonoyl.com/support";

/// Opens the project's page for problems and questions in whatever the system uses to show a web
/// page.
#[tauri::command]
pub fn open_the_support_page(app: tauri::AppHandle) -> Result<(), CommandError> {
    use tauri_plugin_opener::OpenerExt;

    app.opener().open_url(SUPPORT, None::<&str>).map_err(|_| {
        CommandError::new(
            ErrorCode::Internal,
            "The system would not open the page for problems and questions.",
        )
    })
}
