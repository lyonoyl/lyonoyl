use crate::error::DomainError;

/// Colour theme the user chose. `System` follows the OS setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// Which set of colours the interface wears when it is dark.
///
/// Separate from [`Theme`] on purpose: the theme decides *whether* it is dark — including by
/// asking the system — and this decides *what dark looks like*. One setting for both would make
/// "follow the system" impossible to express without repeating every palette twice.
///
/// The names below are the dark schemes people already know from their editors, so somebody who has
/// one in mind can find it by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DarkScheme {
    /// The neutral grey this application has always used, and what it keeps using unless somebody
    /// picks otherwise: an update should not repaint a client while its owner is reading.
    #[default]
    Graphite,
    Dracula,
    Nord,
    SolarizedDark,
    GruvboxDark,
    OneDark,
}

/// Which set of colours the interface wears when it is light.
///
/// The light twin of [`DarkScheme`], and its own setting for the same reason that one is not the
/// theme: the theme decides whether the window is light, and each side keeps its own choice, so a
/// window that follows the system has both ready and changes nothing but what the system asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LightScheme {
    /// The plain white this application has always worn when light, and what it keeps wearing
    /// unless somebody picks otherwise.
    #[default]
    Paper,
    SolarizedLight,
    GruvboxLight,
    CatppuccinLatte,
    RosePineDawn,
}

/// The picture the application shows itself as while it runs: the kitten on one of nine
/// backgrounds, each a shade deeper when the window is dark.
///
/// Worn where a running application can change its own picture, the Dock on macOS and the window and
/// its taskbar button on Windows. What the system shows of the application itself — in Finder, in the
/// Start menu, on a notification — is the picture it was built with, which is [`AppIcon::Ember`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppIcon {
    Ink,
    Indigo,
    Dusk,
    Plum,
    Graphite,
    Sky,
    Paper,
    /// The picture the application is built with, so the one worn until somebody picks otherwise.
    #[default]
    Ember,
    Rust,
}

/// What a notification is allowed to say about the mail it announces.
///
/// Three switches rather than one, because they are three different things to mind. A sender's
/// name on a locked screen says who writes to this person; a subject says what about; the opening
/// says the rest. Somebody who is content with two of those and not the third has no way to say so
/// with one switch, and a single "show details" is that person being told to choose all or
/// nothing.
///
/// All three on by default. A notification that says only that mail arrived does not answer the
/// question a person looks at it to answer — whether this is worth stopping for — so they open the
/// application to find out, which is the interruption the notification was meant to save them.
/// Turning a part off is one press, and what it costs is written beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoticeShows {
    /// Who it is from, as they wrote their own name. Never an address: an address is how a message
    /// is delivered rather than how a person is known, and what the list shows is the name.
    pub sender: bool,
    /// What the message says it is about.
    pub subject: bool,
    /// The opening of the message, as far as it has been read.
    pub preview: bool,
}

impl Default for NoticeShows {
    fn default() -> Self {
        Self {
            sender: true,
            subject: true,
            preview: true,
        }
    }
}

impl NoticeShows {
    /// Whether anything at all beyond "mail arrived" may be said.
    ///
    /// Asked before the mail is read rather than after: with every part turned off there is
    /// nothing a notification could carry, and going to the database to find that out is a
    /// question asked for nothing every time mail arrives.
    pub fn anything(self) -> bool {
        self.sender || self.subject || self.preview
    }

    /// What the store holds for one switch. Stored, so these must not be renamed.
    pub fn as_str(on: bool) -> &'static str {
        if on {
            "yes"
        } else {
            "no"
        }
    }

    /// What the store holds, read back.
    ///
    /// Only the word for off turns a part off. A value this build cannot read was written by
    /// another one, and of the two mistakes available the quieter is to show less than somebody
    /// asked for rather than more — but "less" here is this application's own default, which is to
    /// show it. What must never happen is a stored *no* being read as yes, and only an exact match
    /// turns it off, so that cannot.
    pub fn parse(value: &str) -> bool {
        value != "no"
    }
}

/// Whether the system is told when mail arrives.
///
/// Two values and not three. A per-account or per-folder switch would need somewhere to keep it —
/// the settings store is one flat table of keys shared by every account — and it would be a second
/// answer to "which mail matters" beside the one the sidebar already gives. What is here is the
/// question a person actually asks: interrupt me, or do not.
///
/// Inbox only, always. Mail arriving anywhere else is usually the person's own filing coming back
/// from the server, and being interrupted by one's own tidying is worse than not being told.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NewMailNotice {
    /// The system is told, for mail that arrives in an inbox and that nobody has seen.
    #[default]
    Inbox,
    /// Nothing is ever shown.
    Never,
}

impl NewMailNotice {
    /// Every value, so a settings screen can offer them without listing them itself.
    pub const ALL: [Self; 2] = [Self::Inbox, Self::Never];

    /// The text that goes into the settings store. Stored, so it must not be renamed.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Never => "never",
        }
    }

    /// What the store holds, read back.
    ///
    /// Anything this build does not know reads as the default rather than failing. A settings file
    /// written by a newer version must not stop an older one from starting, and the cost of being
    /// wrong here is one notification too many or too few.
    pub fn parse(value: &str) -> Self {
        match value {
            "never" => Self::Never,
            _ => Self::Inbox,
        }
    }
}

/// Whether a message fetches the pictures it points at elsewhere without being asked.
///
/// A picture a message points at lives on somebody else's machine, and fetching one tells that
/// machine the message was opened, by whoever is at this address, at this minute. That is the whole
/// of what is at stake: the frame has no network of its own under any setting, the bytes are
/// fetched here and served back from this process, and the policy the frame reads them under does
/// not change. So this is one question — who learns that you opened your mail — and not a lock
/// being taken off.
///
/// The default is the careful one, and a person who would rather see their mail than be careful
/// about it says so once instead of once per message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShowPictures {
    /// Nothing is fetched until the reader asks, and the asking is good for that message only.
    #[default]
    WhenAsked,
    /// Fetched as the message is opened.
    Always,
}

impl ShowPictures {
    /// Every value, so a settings screen can offer them without listing them itself.
    pub const ALL: [Self; 2] = [Self::WhenAsked, Self::Always];

    /// The text that goes into the settings store. Stored, so it must not be renamed.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WhenAsked => "whenAsked",
            Self::Always => "always",
        }
    }

    /// What the store holds, read back.
    ///
    /// No fallible parse, and the fallback is deliberate rather than convenient: a value this build
    /// does not know was written by another one, and the two mistakes available here are not equal.
    /// Reading it as `Always` would fetch on somebody's behalf because their settings file said a
    /// word this build cannot read; reading it as `WhenAsked` shows them a button.
    pub fn parse(value: &str) -> Self {
        match value {
            "always" => Self::Always,
            _ => Self::WhenAsked,
        }
    }
}

/// Whether this copy asks the project once a day if a newer version is out.
///
/// Asked of the person before it happens rather than decided for them: looking is a request to the
/// project's address every day, and nothing reaches out from here that the person did not ask for.
/// So the default is that nobody has been asked, and nothing is looked for until somebody answers.
/// A person pressing a button to look is not this question at all — that is asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LookForUpdates {
    /// Nobody has been asked, so nothing is looked for, and the window asks.
    #[default]
    NotAsked,
    /// Once a day.
    OnceADay,
    /// Never on its own.
    Never,
}

impl LookForUpdates {
    /// Every value, so a settings screen can offer them without listing them itself.
    pub const ALL: [Self; 3] = [Self::NotAsked, Self::OnceADay, Self::Never];

    /// The text that goes into the settings store. Stored, so it must not be renamed.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotAsked => "notAsked",
            Self::OnceADay => "onceADay",
            Self::Never => "never",
        }
    }

    /// What the store holds, read back.
    ///
    /// A value this build does not know was written by another one, and it is read as nobody having
    /// been asked: that looks for nothing and asks again, which is the side every mistake should
    /// land on.
    pub fn parse(value: &str) -> Self {
        match value {
            "onceADay" => Self::OnceADay,
            "never" => Self::Never,
            _ => Self::NotAsked,
        }
    }

    /// Whether a copy may look on its own, without somebody pressing anything.
    pub fn looks_on_its_own(self) -> bool {
        matches!(self, Self::OnceADay)
    }
}

/// Whether this copy shares anonymous reports about what fails in it.
///
/// Off until the person turns it on. A report leaves this device for the project, and nothing
/// leaves it that the person did not ask for, so the default sends nothing and only a stored *yes*
/// sends anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShareAnonymousData {
    /// Nothing is shared.
    #[default]
    No,
    /// Anonymous reports about failures are shared.
    Yes,
}

impl ShareAnonymousData {
    /// The text that goes into the settings store. Stored, so it must not be renamed.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::No => "no",
            Self::Yes => "yes",
        }
    }

    /// What the store holds, read back.
    ///
    /// Only an exact *yes* shares. A value this build cannot read was written by another one, and
    /// reading it as yes would send reports on the strength of a word nobody here can read.
    pub fn parse(value: &str) -> Self {
        match value {
            "yes" => Self::Yes,
            _ => Self::No,
        }
    }

    /// Whether anything may be shared.
    pub fn shares(self) -> bool {
        matches!(self, Self::Yes)
    }
}

/// Whether the person has been through the onboarding, as the settings store writes it.
///
/// Only an exact *yes* says so. A value this build cannot read greets them again, which is the side
/// such a word should land on: being shown the welcome twice costs a minute, and never being asked
/// the two questions it asks costs the answers.
pub fn welcomed_from_store(value: &str) -> bool {
    value == "yes"
}

/// The words [`welcomed_from_store`] reads. Stored, so they must not be renamed.
pub fn welcomed_for_store(welcomed: bool) -> &'static str {
    if welcomed {
        "yes"
    } else {
        "no"
    }
}

/// How tightly the message list is packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Density {
    #[default]
    Comfortable,
    Compact,
}

/// Longest tag we accept. BCP-47 allows more, but nothing a mail client ships comes close, and an
/// unbounded string would land in the settings store unchecked.
pub const MAX_LANGUAGE_TAG_LEN: usize = 35;

/// A well-formed language tag, stored lowercase.
///
/// Validated by shape rather than by membership in a list: which catalogs the interface ships is
/// not decided here. A tag this build cannot render still round-trips through the settings store
/// unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageTag(String);

impl LanguageTag {
    /// Accepts `language` or `language-subtag(-subtag)*`: a 2–3 letter primary subtag followed by
    /// alphanumeric subtags of 2–8 characters — `en`, `ru`, `pt-BR`, `sr-Latn-RS`.
    ///
    /// The result is lowercased. BCP-47 matching is case-insensitive, and one canonical spelling in
    /// storage is what lets `pt-BR` from the OS and `pt-br` from the settings screen be one value.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        if value.is_empty() || value.len() > MAX_LANGUAGE_TAG_LEN {
            return Err(DomainError::MalformedLanguageTag);
        }

        let mut subtags = value.split('-');
        let primary = subtags.next().ok_or(DomainError::MalformedLanguageTag)?;
        let primary_ok =
            matches!(primary.len(), 2..=3) && primary.bytes().all(|b| b.is_ascii_alphabetic());
        if !primary_ok {
            return Err(DomainError::MalformedLanguageTag);
        }

        for subtag in subtags {
            let ok =
                matches!(subtag.len(), 2..=8) && subtag.bytes().all(|b| b.is_ascii_alphanumeric());
            if !ok {
                return Err(DomainError::MalformedLanguageTag);
            }
        }

        Ok(Self(value.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Language of the interface. `System` follows the OS, exactly as [`Theme::System`] does.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    System,
    Tag(LanguageTag),
}

impl Language {
    /// `"system"` or a language tag. Anything else was written by a build with different rules.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "system" => Ok(Self::System),
            tag => LanguageTag::parse(tag).map(Self::Tag),
        }
    }

    /// The storage form, and the same string the frontend matches its catalogs against.
    pub fn as_str(&self) -> &str {
        match self {
            Self::System => "system",
            Self::Tag(tag) => tag.as_str(),
        }
    }
}

/// User preferences: the settings that belong to the person rather than to this machine.
///
/// Window geometry, which belongs to the machine, lives in [`crate::window::WindowPlacement`] instead.
///
/// Every field has a default, so a fresh install and a partially written store behave identically.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Preferences {
    theme: Theme,
    dark_scheme: DarkScheme,
    light_scheme: LightScheme,
    density: Density,
    language: Language,
    new_mail_notice: NewMailNotice,
    notice_shows: NoticeShows,
    show_pictures: ShowPictures,
    look_for_updates: LookForUpdates,
    share_anonymous_data: ShareAnonymousData,
    app_icon: AppIcon,
    /// Whether the onboarding has been finished, so it is not shown again.
    welcomed: bool,
}

impl Preferences {
    pub fn new_mail_notice(&self) -> NewMailNotice {
        self.new_mail_notice
    }

    pub fn show_pictures(&self) -> ShowPictures {
        self.show_pictures
    }

    pub fn look_for_updates(&self) -> LookForUpdates {
        self.look_for_updates
    }

    pub fn share_anonymous_data(&self) -> ShareAnonymousData {
        self.share_anonymous_data
    }

    pub fn welcomed(&self) -> bool {
        self.welcomed
    }

    pub fn notice_shows(&self) -> NoticeShows {
        self.notice_shows
    }

    pub fn theme(&self) -> Theme {
        self.theme
    }

    pub fn dark_scheme(&self) -> DarkScheme {
        self.dark_scheme
    }

    pub fn light_scheme(&self) -> LightScheme {
        self.light_scheme
    }

    pub fn app_icon(&self) -> AppIcon {
        self.app_icon
    }

    pub fn density(&self) -> Density {
        self.density
    }

    pub fn language(&self) -> &Language {
        &self.language
    }

    pub fn with_theme(self, theme: Theme) -> Self {
        Self { theme, ..self }
    }

    pub fn with_dark_scheme(self, dark_scheme: DarkScheme) -> Self {
        Self {
            dark_scheme,
            ..self
        }
    }

    pub fn with_light_scheme(self, light_scheme: LightScheme) -> Self {
        Self {
            light_scheme,
            ..self
        }
    }

    pub fn with_density(self, density: Density) -> Self {
        Self { density, ..self }
    }

    pub fn with_app_icon(self, app_icon: AppIcon) -> Self {
        Self { app_icon, ..self }
    }

    pub fn with_new_mail_notice(mut self, notice: NewMailNotice) -> Self {
        self.new_mail_notice = notice;
        self
    }

    pub fn with_language(self, language: Language) -> Self {
        Self { language, ..self }
    }

    pub fn with_show_pictures(mut self, show_pictures: ShowPictures) -> Self {
        self.show_pictures = show_pictures;
        self
    }

    pub fn with_notice_shows(mut self, notice_shows: NoticeShows) -> Self {
        self.notice_shows = notice_shows;
        self
    }

    pub fn with_look_for_updates(mut self, look_for_updates: LookForUpdates) -> Self {
        self.look_for_updates = look_for_updates;
        self
    }

    pub fn with_share_anonymous_data(mut self, share: ShareAnonymousData) -> Self {
        self.share_anonymous_data = share;
        self
    }

    pub fn with_welcomed(mut self, welcomed: bool) -> Self {
        self.welcomed = welcomed;
        self
    }
}

impl Theme {
    /// Parses a stored value. An unknown string is a defect in stored data, not a crash:
    /// a newer version may have written a theme this build does not know.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "system" => Ok(Self::System),
            "light" => Ok(Self::Light),
            "dark" => Ok(Self::Dark),
            _ => Err(DomainError::UnknownPreferenceValue),
        }
    }
}

impl DarkScheme {
    /// Parses a stored value. Unknown means a newer build wrote a scheme this one does not have,
    /// which is not a failure: the default is worn and the stored row is left as it is.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "graphite" => Ok(Self::Graphite),
            "dracula" => Ok(Self::Dracula),
            "nord" => Ok(Self::Nord),
            "solarizedDark" => Ok(Self::SolarizedDark),
            "gruvboxDark" => Ok(Self::GruvboxDark),
            "oneDark" => Ok(Self::OneDark),
            _ => Err(DomainError::UnknownPreferenceValue),
        }
    }

    /// The storage form, and the same string the interface names its palette by.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Graphite => "graphite",
            Self::Dracula => "dracula",
            Self::Nord => "nord",
            Self::SolarizedDark => "solarizedDark",
            Self::GruvboxDark => "gruvboxDark",
            Self::OneDark => "oneDark",
        }
    }

    /// Every scheme, in the order a settings screen offers them.
    pub const ALL: [Self; 6] = [
        Self::Graphite,
        Self::Dracula,
        Self::Nord,
        Self::SolarizedDark,
        Self::GruvboxDark,
        Self::OneDark,
    ];
}

impl LightScheme {
    /// Parses a stored value. Unknown means a newer build wrote a scheme this one does not have,
    /// which is not a failure: the default is worn and the stored row is left as it is.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "paper" => Ok(Self::Paper),
            "solarizedLight" => Ok(Self::SolarizedLight),
            "gruvboxLight" => Ok(Self::GruvboxLight),
            "catppuccinLatte" => Ok(Self::CatppuccinLatte),
            "rosePineDawn" => Ok(Self::RosePineDawn),
            _ => Err(DomainError::UnknownPreferenceValue),
        }
    }

    /// The storage form, and the same string the interface names its palette by.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Paper => "paper",
            Self::SolarizedLight => "solarizedLight",
            Self::GruvboxLight => "gruvboxLight",
            Self::CatppuccinLatte => "catppuccinLatte",
            Self::RosePineDawn => "rosePineDawn",
        }
    }

    /// Every scheme, in the order a settings screen offers them.
    pub const ALL: [Self; 5] = [
        Self::Paper,
        Self::SolarizedLight,
        Self::GruvboxLight,
        Self::CatppuccinLatte,
        Self::RosePineDawn,
    ];
}

impl AppIcon {
    /// Parses a stored value. Unknown means a newer build wrote a picture this one does not have,
    /// which is not a failure: the default is worn and the stored row is left as it is.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        Self::ALL
            .into_iter()
            .find(|icon| icon.as_str() == value)
            .ok_or(DomainError::UnknownPreferenceValue)
    }

    /// The storage form, and the same string the interface names its picture by.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ink => "ink",
            Self::Indigo => "indigo",
            Self::Dusk => "dusk",
            Self::Plum => "plum",
            Self::Graphite => "graphite",
            Self::Sky => "sky",
            Self::Paper => "paper",
            Self::Ember => "ember",
            Self::Rust => "rust",
        }
    }

    /// Every picture, in the order they are offered: the deep backgrounds, the light ones, and the
    /// warm ones.
    pub const ALL: [Self; 9] = [
        Self::Ink,
        Self::Indigo,
        Self::Dusk,
        Self::Plum,
        Self::Graphite,
        Self::Sky,
        Self::Paper,
        Self::Ember,
        Self::Rust,
    ];
}

impl Density {
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        match value {
            "comfortable" => Ok(Self::Comfortable),
            "compact" => Ok(Self::Compact),
            _ => Err(DomainError::UnknownPreferenceValue),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_install_asks_before_it_fetches_a_picture_from_anywhere() {
        // The default here is the one that tells nobody anything, and it is the default because
        // somebody who has not answered the question has not agreed to be counted.
        assert_eq!(ShowPictures::default(), ShowPictures::WhenAsked);
        assert_eq!(
            Preferences::default().show_pictures(),
            ShowPictures::WhenAsked
        );
    }

    #[test]
    fn every_answer_about_pictures_survives_being_written_down_and_read_back() {
        for answer in ShowPictures::ALL {
            assert_eq!(
                ShowPictures::parse(answer.as_str()),
                answer,
                "{answer:?} did not survive storage"
            );
        }
    }

    #[test]
    fn an_answer_about_pictures_this_build_cannot_read_is_read_as_asking() {
        // Every way of being wrong lands on the careful side. A newer build's word for something
        // else must not read here as permission to go out to a host.
        for written in ["", "always ", "Always", "yes", "sometimes", "whenasked"] {
            assert_eq!(
                ShowPictures::parse(written),
                ShowPictures::WhenAsked,
                "{written:?} was read as something other than asking"
            );
        }
    }

    #[test]
    fn a_fresh_install_looks_for_no_update_until_somebody_says_it_may() {
        assert_eq!(LookForUpdates::default(), LookForUpdates::NotAsked);
        assert_eq!(
            Preferences::default().look_for_updates(),
            LookForUpdates::NotAsked
        );
        assert!(!Preferences::default().look_for_updates().looks_on_its_own());
    }

    #[test]
    fn only_once_a_day_looks_on_its_own() {
        for answer in LookForUpdates::ALL {
            assert_eq!(
                answer.looks_on_its_own(),
                answer == LookForUpdates::OnceADay,
                "{answer:?}"
            );
        }
    }

    #[test]
    fn every_answer_about_updates_survives_being_written_down_and_read_back() {
        for answer in LookForUpdates::ALL {
            assert_eq!(LookForUpdates::parse(answer.as_str()), answer, "{answer:?}");
        }
    }

    #[test]
    fn an_answer_about_updates_this_build_cannot_read_is_read_as_not_asked() {
        // A newer build's word for something else must not read here as permission to reach out.
        for written in ["", "OnceADay", "onceaday", "always", "daily", "yes"] {
            assert_eq!(
                LookForUpdates::parse(written),
                LookForUpdates::NotAsked,
                "{written:?} was read as an answer"
            );
        }
    }

    #[test]
    fn answering_about_updates_changes_nothing_else() {
        let before = Preferences::default()
            .with_theme(Theme::Dark)
            .with_show_pictures(ShowPictures::Always)
            .with_new_mail_notice(NewMailNotice::Never);

        let after = before
            .clone()
            .with_look_for_updates(LookForUpdates::OnceADay);

        assert_eq!(after.look_for_updates(), LookForUpdates::OnceADay);
        assert_eq!(after.theme(), before.theme());
        assert_eq!(after.show_pictures(), before.show_pictures());
        assert_eq!(after.new_mail_notice(), before.new_mail_notice());
        assert_eq!(after.language(), before.language());
    }

    #[test]
    fn a_fresh_install_shares_nothing() {
        assert_eq!(ShareAnonymousData::default(), ShareAnonymousData::No);
        assert!(!Preferences::default().share_anonymous_data().shares());
    }

    #[test]
    fn both_answers_about_sharing_survive_being_written_down_and_read_back() {
        for answer in [ShareAnonymousData::No, ShareAnonymousData::Yes] {
            assert_eq!(
                ShareAnonymousData::parse(answer.as_str()),
                answer,
                "{answer:?}"
            );
        }
        assert!(ShareAnonymousData::Yes.shares());
    }

    #[test]
    fn only_a_stored_yes_shares_anything() {
        // A newer build's word for something else must not read here as permission to send.
        for written in ["", "Yes", "YES", "true", "1", "on", "always", "anonymously"] {
            assert_eq!(
                ShareAnonymousData::parse(written),
                ShareAnonymousData::No,
                "{written:?} was read as a yes"
            );
        }
    }

    #[test]
    fn a_fresh_install_has_not_been_welcomed() {
        assert!(!Preferences::default().welcomed());
    }

    #[test]
    fn only_a_stored_yes_says_the_onboarding_was_finished() {
        assert!(welcomed_from_store(welcomed_for_store(true)));
        assert!(!welcomed_from_store(welcomed_for_store(false)));
        // A newer build's word for something else greets the person again rather than skipping it.
        for written in ["", "Yes", "true", "1", "done", "always"] {
            assert!(
                !welcomed_from_store(written),
                "{written:?} was read as done"
            );
        }
    }

    #[test]
    fn finishing_the_onboarding_changes_nothing_else() {
        let before = Preferences::default()
            .with_theme(Theme::Dark)
            .with_look_for_updates(LookForUpdates::OnceADay)
            .with_share_anonymous_data(ShareAnonymousData::Yes);

        let after = before.clone().with_welcomed(true);

        assert!(after.welcomed());
        assert_eq!(after.theme(), before.theme());
        assert_eq!(after.look_for_updates(), before.look_for_updates());
        assert_eq!(after.share_anonymous_data(), before.share_anonymous_data());
    }

    #[test]
    fn saying_yes_to_sharing_changes_nothing_else() {
        let before = Preferences::default()
            .with_theme(Theme::Dark)
            .with_look_for_updates(LookForUpdates::OnceADay)
            .with_show_pictures(ShowPictures::Always);

        let after = before
            .clone()
            .with_share_anonymous_data(ShareAnonymousData::Yes);

        assert_eq!(after.share_anonymous_data(), ShareAnonymousData::Yes);
        assert_eq!(after.theme(), before.theme());
        assert_eq!(after.look_for_updates(), before.look_for_updates());
        assert_eq!(after.show_pictures(), before.show_pictures());
        assert_eq!(after.language(), before.language());
    }

    #[test]
    fn saying_yes_to_pictures_changes_nothing_else() {
        let before = Preferences::default()
            .with_theme(Theme::Dark)
            .with_language(Language::parse("ru").expect("well formed"))
            .with_new_mail_notice(NewMailNotice::Never);

        let after = before.clone().with_show_pictures(ShowPictures::Always);

        assert_eq!(after.show_pictures(), ShowPictures::Always);
        assert_eq!(after.theme(), before.theme());
        assert_eq!(after.language(), before.language());
        assert_eq!(after.new_mail_notice(), before.new_mail_notice());
        assert_eq!(after.density(), before.density());
        assert_eq!(after.dark_scheme(), before.dark_scheme());
    }

    #[test]
    fn an_update_does_not_repaint_a_client_that_never_asked() {
        // Whatever the list of schemes grows to, the one somebody gets without choosing is the one
        // the application has always worn.
        assert_eq!(DarkScheme::default(), DarkScheme::Graphite);
        assert_eq!(Preferences::default().dark_scheme(), DarkScheme::Graphite);
    }

    #[test]
    fn every_scheme_survives_being_written_down_and_read_back() {
        for scheme in DarkScheme::ALL {
            assert_eq!(
                DarkScheme::parse(scheme.as_str()),
                Ok(scheme),
                "{scheme:?} did not survive storage"
            );
        }
    }

    #[test]
    fn the_offered_schemes_are_all_of_them_and_each_one_once() {
        // The settings screen offers this list. A scheme missing from it is one nobody can choose;
        // a duplicate is one offered twice.
        let mut names: Vec<&str> = DarkScheme::ALL
            .iter()
            .map(|scheme| scheme.as_str())
            .collect();
        let offered = names.len();
        names.sort_unstable();
        names.dedup();

        assert_eq!(names.len(), offered, "a scheme is offered twice");
        assert_eq!(offered, 6);
    }

    #[test]
    fn a_scheme_written_by_a_newer_build_leaves_the_stored_value_alone() {
        // Not a crash and not a rewrite: this build wears its default, the row stays as it is, and
        // the build that wrote it still finds what it wrote.
        assert_eq!(
            DarkScheme::parse("midnight-neon"),
            Err(DomainError::UnknownPreferenceValue)
        );
    }

    #[test]
    fn choosing_a_scheme_changes_that_and_nothing_else() {
        let before = Preferences::default()
            .with_theme(Theme::Light)
            .with_dark_scheme(DarkScheme::Graphite)
            .with_density(Density::Compact)
            .with_language(Language::parse("ru").expect("well formed"));

        let after = before.clone().with_dark_scheme(DarkScheme::Nord);

        assert_eq!(after.dark_scheme(), DarkScheme::Nord);
        assert_eq!(after.light_scheme(), before.light_scheme());
        assert_eq!(after.theme(), before.theme());
        assert_eq!(after.density(), before.density());
        assert_eq!(after.language(), before.language());
    }

    #[test]
    fn a_window_nobody_chose_a_picture_for_wears_the_one_it_was_built_with() {
        assert_eq!(AppIcon::default(), AppIcon::Ember);
        assert_eq!(Preferences::default().app_icon(), AppIcon::Ember);
    }

    #[test]
    fn every_picture_survives_being_written_down_and_read_back_and_is_offered_once() {
        for icon in AppIcon::ALL {
            assert_eq!(AppIcon::parse(icon.as_str()), Ok(icon), "{icon:?}");
        }

        let mut names: Vec<&str> = AppIcon::ALL.iter().map(|icon| icon.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 9, "a picture is offered twice");
    }

    #[test]
    fn a_picture_written_by_a_newer_build_is_refused_rather_than_guessed() {
        assert_eq!(
            AppIcon::parse("neon"),
            Err(DomainError::UnknownPreferenceValue)
        );
        assert_eq!(
            AppIcon::parse("Ember"),
            Err(DomainError::UnknownPreferenceValue)
        );
    }

    #[test]
    fn choosing_a_picture_changes_that_and_nothing_else() {
        let before = Preferences::default()
            .with_theme(Theme::Dark)
            .with_light_scheme(LightScheme::GruvboxLight)
            .with_welcomed(true);

        let after = before.clone().with_app_icon(AppIcon::Rust);

        assert_eq!(after.app_icon(), AppIcon::Rust);
        assert_eq!(after.theme(), Theme::Dark);
        assert_eq!(after.light_scheme(), LightScheme::GruvboxLight);
        assert!(after.welcomed());
    }

    #[test]
    fn an_update_does_not_repaint_a_light_window_that_never_asked() {
        assert_eq!(LightScheme::default(), LightScheme::Paper);
        assert_eq!(Preferences::default().light_scheme(), LightScheme::Paper);
    }

    #[test]
    fn every_light_scheme_survives_being_written_down_and_read_back() {
        for scheme in LightScheme::ALL {
            assert_eq!(
                LightScheme::parse(scheme.as_str()),
                Ok(scheme),
                "{scheme:?} did not survive storage"
            );
        }
    }

    #[test]
    fn the_offered_light_schemes_are_all_of_them_and_each_one_once() {
        let mut names: Vec<&str> = LightScheme::ALL
            .iter()
            .map(|scheme| scheme.as_str())
            .collect();
        let offered = names.len();
        names.sort_unstable();
        names.dedup();

        assert_eq!(names.len(), offered, "a scheme is offered twice");
        assert_eq!(offered, 5);
    }

    #[test]
    fn a_light_scheme_written_by_a_newer_build_leaves_the_stored_value_alone() {
        assert_eq!(
            LightScheme::parse("midday-neon"),
            Err(DomainError::UnknownPreferenceValue)
        );
        // A dark scheme's name is not a light one, and the two settings never read each other's.
        assert_eq!(
            LightScheme::parse(DarkScheme::Nord.as_str()),
            Err(DomainError::UnknownPreferenceValue)
        );
    }

    #[test]
    fn choosing_a_light_scheme_changes_that_and_nothing_else() {
        let before = Preferences::default()
            .with_theme(Theme::Dark)
            .with_dark_scheme(DarkScheme::Nord)
            .with_density(Density::Compact)
            .with_language(Language::parse("ru").expect("well formed"));

        let after = before
            .clone()
            .with_light_scheme(LightScheme::CatppuccinLatte);

        assert_eq!(after.light_scheme(), LightScheme::CatppuccinLatte);
        assert_eq!(after.dark_scheme(), before.dark_scheme());
        assert_eq!(after.theme(), before.theme());
        assert_eq!(after.density(), before.density());
        assert_eq!(after.language(), before.language());
    }

    #[test]
    fn defaults_follow_the_system_and_the_roomier_layout() {
        let prefs = Preferences::default();
        assert_eq!(prefs.theme(), Theme::System);
        assert_eq!(prefs.density(), Density::Comfortable);
        assert_eq!(prefs.language(), &Language::System);
    }

    #[test]
    fn changing_one_field_leaves_the_other_untouched() {
        let prefs = Preferences::default()
            .with_theme(Theme::Dark)
            .with_language(Language::Tag(
                LanguageTag::parse("ru").expect("well formed"),
            ));
        assert_eq!(prefs.theme(), Theme::Dark);
        assert_eq!(prefs.density(), Density::Comfortable);
        assert_eq!(prefs.language().as_str(), "ru");
    }

    #[test]
    fn accepts_the_tag_shapes_a_mail_client_actually_ships() {
        for tag in ["en", "ru", "haw", "pt-BR", "sr-Latn-RS", "zh-Hans-CN"] {
            assert!(
                LanguageTag::parse(tag).is_ok(),
                "{tag} is a well-formed tag and must be accepted"
            );
        }
    }

    #[test]
    fn normalises_case_so_the_os_and_the_settings_screen_agree() {
        // navigator.language reports pt-BR; a settings screen may store pt-br. One value, not two.
        let from_os = LanguageTag::parse("pt-BR").expect("well formed");
        let from_ui = LanguageTag::parse("pt-br").expect("well formed");
        assert_eq!(from_os, from_ui);
        assert_eq!(from_os.as_str(), "pt-br");
    }

    #[test]
    fn parsing_its_own_output_yields_the_same_tag() {
        // Storage depends on this: load(save(x)) must not drift, or a language would decay over time.
        let once = LanguageTag::parse("sr-Latn-RS").expect("well formed");
        let twice = LanguageTag::parse(once.as_str()).expect("its own output stays valid");
        assert_eq!(once, twice);
    }

    #[test]
    fn rejects_tags_that_are_not_well_formed() {
        for tag in [
            "",       // nothing at all
            "e",      // one letter, below the ISO minimum
            "engl",   // four letters: reserved, never a language subtag
            "en_US",  // underscore is the POSIX spelling, not BCP-47
            "en--US", // empty subtag
            "en-",    // trailing separator
            "-en",    // missing primary subtag
            "en-toolongsubtag",
            "en-US-",
            "12", // digits cannot open a tag
            "ζα", // non-ASCII
        ] {
            assert!(
                LanguageTag::parse(tag).is_err(),
                "{tag:?} is malformed and must be rejected"
            );
        }
    }

    #[test]
    fn rejects_a_tag_longer_than_the_stored_maximum() {
        let too_long = format!("en-{}", "a".repeat(MAX_LANGUAGE_TAG_LEN));
        assert!(LanguageTag::parse(&too_long).is_err());
    }

    #[test]
    fn rejects_separators_that_could_escape_a_catalog_lookup() {
        // The tag reaches the frontend and selects a catalog. Nothing that looks like a path may
        // survive parsing, even though the catalogs are a static map rather than files on disk.
        for hostile in [
            "../en",
            "en/../ru",
            "en\\ru",
            "en.json",
            "en ru",
            "en\u{0000}",
        ] {
            assert!(
                LanguageTag::parse(hostile).is_err(),
                "{hostile:?} must never become a language tag"
            );
        }
    }

    #[test]
    fn system_is_a_language_of_its_own_rather_than_a_tag() {
        assert_eq!(Language::parse("system").expect("known"), Language::System);
        assert_eq!(Language::System.as_str(), "system");
        assert_eq!(
            Language::parse("ru").expect("well formed"),
            Language::Tag(LanguageTag::parse("ru").expect("well formed"))
        );
        assert!(Language::parse("ζήτα κάππα").is_err());
    }

    #[test]
    fn a_language_this_build_cannot_render_still_parses() {
        // The core does not know which catalogs exist, so a language added by a newer build
        // survives a downgrade.
        let exotic = Language::parse("cy-GB").expect("well formed even with no catalog");
        assert_eq!(exotic.as_str(), "cy-gb");
    }

    #[test]
    fn parses_every_known_value() {
        assert_eq!(Theme::parse("dark").expect("known"), Theme::Dark);
        assert_eq!(Theme::parse("system").expect("known"), Theme::System);
        assert_eq!(Density::parse("compact").expect("known"), Density::Compact);
    }

    #[test]
    fn an_unknown_value_is_an_error_rather_than_a_silent_default() {
        // A newer build may have stored something this one does not know; the caller decides
        // whether to fall back to the default, and that decision must be explicit.
        assert!(Theme::parse("solarized").is_err());
        assert!(Density::parse("cosy").is_err());
    }
}

#[cfg(test)]
mod new_mail_notice_tests {
    use super::*;

    #[test]
    fn telling_somebody_is_what_happens_unless_they_say_otherwise() {
        // A mail client that has to be configured before it says mail arrived is one that looks
        // broken to everybody who did not read the settings.
        assert_eq!(NewMailNotice::default(), NewMailNotice::Inbox);
    }

    #[test]
    fn a_value_this_build_does_not_know_reads_as_the_default() {
        // Written by a newer version. Being told once too often is the safer of the two mistakes,
        // and refusing to start is not one of the options.
        assert_eq!(NewMailNotice::parse("every-folder"), NewMailNotice::Inbox);
        assert_eq!(NewMailNotice::parse(""), NewMailNotice::Inbox);
    }

    #[test]
    fn what_is_written_is_what_is_read_back() {
        // The strings go into the settings store, so a rename is a setting somebody loses.
        for notice in NewMailNotice::ALL {
            assert_eq!(NewMailNotice::parse(notice.as_str()), notice);
        }
    }

    #[test]
    fn every_value_is_offered_so_a_screen_cannot_list_them_itself() {
        // A screen holding its own list is a screen that goes stale the day a value is added.
        assert_eq!(NewMailNotice::ALL.len(), 2);
        assert!(NewMailNotice::ALL.contains(&NewMailNotice::Never));
    }

    #[test]
    fn choosing_it_changes_that_and_nothing_else() {
        let before = Preferences::default()
            .with_theme(Theme::Light)
            .with_dark_scheme(DarkScheme::Nord)
            .with_density(Density::Compact)
            .with_language(Language::parse("ru").expect("well formed"))
            .with_new_mail_notice(NewMailNotice::Inbox)
            .with_show_pictures(ShowPictures::WhenAsked);

        let after = before.clone().with_new_mail_notice(NewMailNotice::Never);

        assert_eq!(after.new_mail_notice(), NewMailNotice::Never);
        assert_eq!(after.theme(), before.theme());
        assert_eq!(after.dark_scheme(), before.dark_scheme());
        assert_eq!(after.density(), before.density());
        assert_eq!(after.language(), before.language());
    }
}
