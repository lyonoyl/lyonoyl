//! A made-up mailbox for the build made for screenshots: the accounts, the groups and the mail one
//! scene file describes.
//!
//! A scene is read once, at the start of that build, and checked whole before anything is written:
//! a mistake in it is said with the place it is in, rather than shown as a window with something
//! missing. [`Servers`] then holds its mail the way the accounts' servers would, and the pass every
//! account is read with reads it from there — so what the window shows of a scene is what it shows
//! of anybody's mail, put there by the same code.
//!
//! A scene is a folder: `scene.json`, and beside it whatever a message written in HTML shows. The
//! file is JSON, and every field it may hold is below. A name nothing reads is refused rather than
//! ignored, because a misspelt field is otherwise a message that quietly lost its flag.

mod agrees;
mod servers;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;

use lyonoyl_domain::account_group::GroupName;
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::outgoing::EmailAddress;
use lyonoyl_domain::preferences::{DarkScheme, Language, LightScheme, Preferences, Theme};

/// The file in a scene's folder that is the scene.
pub const SCENE_FILE: &str = "scene.json";

pub use agrees::Agrees;
pub use servers::{name_of, Servers};

/// What is wrong with a scene, and where.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct SceneError(String);

/// A scene: the language the window is in, the accounts, and the groups they are arranged in.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    /// The language the window opens in, as its catalogue is named: `en`, `ru`, `ja`. The system's
    /// own where there is none.
    #[serde(default)]
    pub language: Option<String>,
    /// `light`, `dark` or `system`, as the settings name them. Light where there is none.
    #[serde(default)]
    pub appearance: Option<String>,
    /// One of that appearance's palettes, as the settings store it: `paper`, `nord`. Paper for
    /// light where there is none, and the first of the dark ones for dark.
    #[serde(default)]
    pub palette: Option<String>,
    /// The window opens on the welcome, as it does on a first start. Behind it where false or
    /// absent.
    #[serde(default)]
    pub welcome: bool,
    /// In the order the sidebar draws them.
    pub accounts: Vec<Account>,
    #[serde(default)]
    pub groups: Vec<Group>,
}

/// One account, and the mail it holds.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub address: String,
    /// The name the account goes by. The sidebar shows the address where there is none.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub mail: Vec<Message>,
}

/// One message, as whoever wrote it sent it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    /// `Name <address>`, or an address alone.
    pub from: String,
    /// The same, and the account's own address where there is none.
    #[serde(default)]
    pub to: Option<String>,
    pub subject: String,
    /// What the message says, as plain text.
    pub text: String,
    /// How long before the build starts it arrived: `40m`, `5h`, `3d`. The list then reads as
    /// mail from this morning on whichever day the window is opened.
    pub ago: String,
    /// `inbox`, `sent`, `archive`, `junk` or `trash`. The inbox where there is none.
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub unread: bool,
    #[serde(default)]
    pub flagged: bool,
    /// A name for this message, for another message of the same account to answer.
    #[serde(default)]
    pub id: Option<String>,
    /// The `id` of the message this one answers, which puts the two in one conversation. It has
    /// to have arrived first.
    #[serde(default)]
    pub answers: Option<String>,
    /// A file in the scene's folder holding the message as HTML, which is what the reader shows.
    /// `text` stays what the list shows under the subject. A picture the HTML names by a path in
    /// that folder travels inside the message, as a picture a message carries does.
    #[serde(default)]
    pub html: Option<String>,
    /// The message the window opens on. One in a scene at most.
    #[serde(default)]
    pub open: bool,
    /// What `html` names, once [`Scene::read`] has read it.
    #[serde(skip)]
    pub(crate) written: Option<Html>,
}

/// A message's HTML, with each picture it names travelling inside it.
#[derive(Debug, Clone)]
pub(crate) struct Html {
    /// The HTML, every picture it named by a path naming the part it travels in instead.
    pub(crate) text: String,
    pub(crate) pictures: Vec<Picture>,
}

/// A picture that travels inside a message.
#[derive(Debug, Clone)]
pub(crate) struct Picture {
    pub(crate) content_id: String,
    pub(crate) content_type: &'static str,
    pub(crate) bytes: Vec<u8>,
}

/// A group in the sidebar, and the accounts in it in the order they are drawn.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String,
    pub accounts: Vec<String>,
    #[serde(default)]
    pub collapsed: bool,
}

/// The mailboxes every account in a scene has, as its server lists them.
///
/// The ones a message may be put in are named in a scene by their purpose rather than by these
/// names. Drafts is listed and never written to: a message being written is kept on the device, and
/// one in the server's drafts folder would be a second kind of draft the window does not have.
const MAILBOXES: [(&str, FolderRole); 6] = [
    ("INBOX", FolderRole::Inbox),
    ("Drafts", FolderRole::Drafts),
    ("Sent", FolderRole::Sent),
    ("Archive", FolderRole::Archive),
    ("Junk", FolderRole::Junk),
    ("Trash", FolderRole::Trash),
];

impl Scene {
    /// The scene in `folder`, with every file its messages name read from beside it.
    pub fn read(folder: &Path) -> Result<Self, SceneError> {
        let path = folder.join(SCENE_FILE);
        let text = std::fs::read_to_string(&path)
            .map_err(|why| SceneError(format!("{}: {why}", path.display())))?;
        let mut scene =
            Self::parse(&text).map_err(|why| SceneError(format!("{}: {why}", path.display())))?;

        for (at, account) in scene.accounts.iter_mut().enumerate() {
            for (number, message) in account.mail.iter_mut().enumerate() {
                if let Some(name) = &message.html {
                    let place = format!("accounts[{at}].mail[{number}].html");
                    message.written = Some(html_in(folder, name, &place)?);
                }
            }
        }

        Ok(scene)
    }

    /// A scene from what its file holds, refused whole if any of it cannot be played. What its
    /// messages name beside it is not read: [`Scene::read`] reads that.
    pub fn parse(text: &str) -> Result<Self, SceneError> {
        let scene: Self = serde_json::from_str(text).map_err(|why| SceneError(why.to_string()))?;
        scene.check()?;
        Ok(scene)
    }

    /// `preferences` in the appearance the scene is shown in.
    pub fn dressed(&self, preferences: Preferences) -> Result<Preferences, SceneError> {
        let theme = match self.appearance.as_deref().map(str::trim) {
            None => Theme::Light,
            Some(named) => Theme::parse(named).map_err(|_| {
                SceneError(format!(
                    "appearance: {named:?} is not light, dark or system"
                ))
            })?,
        };
        let preferences = preferences.with_theme(theme);
        let palette = self.palette.as_deref().map(str::trim);

        match (theme, palette) {
            (Theme::Light, None) => Ok(preferences.with_light_scheme(LightScheme::Paper)),
            (Theme::Light, Some(named)) => Ok(preferences.with_light_scheme(
                LightScheme::parse(named).map_err(|_| {
                    SceneError(format!("palette: {named:?} is not a light palette"))
                })?,
            )),
            (Theme::Dark, None) => Ok(preferences),
            (Theme::Dark, Some(named)) => Ok(preferences
                .with_dark_scheme(DarkScheme::parse(named).map_err(|_| {
                    SceneError(format!("palette: {named:?} is not a dark palette"))
                })?)),
            (Theme::System, None) => Ok(preferences),
            (Theme::System, Some(_)) => Err(SceneError(
                "palette: one appearance that follows the system has two palettes; \
                 name light or dark to choose one"
                    .to_owned(),
            )),
        }
    }

    /// The account and the message the window opens on, by their places in the scene.
    pub fn opening(&self) -> Option<(usize, usize)> {
        self.accounts.iter().enumerate().find_map(|(at, account)| {
            account
                .mail
                .iter()
                .position(|message| message.open)
                .map(|number| (at, number))
        })
    }

    /// The language the window opens in.
    pub fn language(&self) -> Result<Language, SceneError> {
        match &self.language {
            Some(tag) => Language::parse(tag)
                .map_err(|_| SceneError(format!("language: {tag:?} is not a language tag"))),
            None => Ok(Language::System),
        }
    }

    /// Everything [`Scene::parse`] promises, said about the first thing that breaks it.
    fn check(&self) -> Result<(), SceneError> {
        if self.accounts.is_empty() {
            return Err(SceneError(
                "accounts: a scene needs at least one".to_owned(),
            ));
        }
        self.language()?;
        self.dressed(Preferences::default())?;

        let opened = self
            .accounts
            .iter()
            .flat_map(|account| account.mail.iter())
            .filter(|message| message.open)
            .count();
        if opened > 1 {
            return Err(SceneError(format!(
                "open: {opened} messages say it, and the window opens on one"
            )));
        }

        let mut addresses = HashSet::new();
        for (at, account) in self.accounts.iter().enumerate() {
            let place = format!("accounts[{at}]");
            let address = address_in(&account.address, &place)?;
            if !addresses.insert(address.to_ascii_lowercase()) {
                return Err(SceneError(format!("{place}: {address} is here twice")));
            }
            account.check(&place)?;
        }

        let mut grouped = HashSet::new();
        for (at, group) in self.groups.iter().enumerate() {
            let place = format!("groups[{at}]");
            GroupName::new(&group.name)
                .map_err(|why| SceneError(format!("{place}.name: {why}")))?;

            for member in &group.accounts {
                let member = member.trim().to_ascii_lowercase();
                if !addresses.contains(&member) {
                    return Err(SceneError(format!(
                        "{place}: {member} is not an account here"
                    )));
                }
                // The rule the sidebar keeps: an account is drawn in one place.
                if !grouped.insert(member.clone()) {
                    return Err(SceneError(format!("{place}: {member} is in another group")));
                }
            }
        }

        Ok(())
    }
}

impl Account {
    fn check(&self, place: &str) -> Result<(), SceneError> {
        let mut named: HashMap<&str, u64> = HashMap::new();
        for (at, message) in self.mail.iter().enumerate() {
            let place = format!("{place}.mail[{at}]");
            let ago = message.seconds_ago(&place)?;
            message.mailbox(&place)?;
            if let Some(name) = &message.html {
                beside(Path::new(""), name, &format!("{place}.html"))?;
            }
            sender_of(&message.from, &format!("{place}.from"))?;
            if let Some(to) = &message.to {
                sender_of(to, &format!("{place}.to"))?;
            }

            if let Some(id) = &message.id {
                if named.insert(id.as_str(), ago).is_some() {
                    return Err(SceneError(format!("{place}.id: {id:?} names two messages")));
                }
            }
        }

        // Once every name is known, so a message may answer one written further down the file.
        for (at, message) in self.mail.iter().enumerate() {
            let Some(answered) = &message.answers else {
                continue;
            };
            let place = format!("{place}.mail[{at}].answers");
            let Some(&first) = named.get(answered.as_str()) else {
                return Err(SceneError(format!(
                    "{place}: no message here is named {answered:?}"
                )));
            };
            if message.seconds_ago(&place)? >= first {
                return Err(SceneError(format!(
                    "{place}: it arrives before what it answers"
                )));
            }
        }

        Ok(())
    }
}

impl Message {
    /// `ago` in seconds.
    fn seconds_ago(&self, place: &str) -> Result<u64, SceneError> {
        let refused = || SceneError(format!("{place}.ago: {:?} is not 40m, 5h or 3d", self.ago));

        // Cut before the last character rather than the last byte: a unit typed in another script
        // is a mistake to report, not a place to split a letter in two.
        let ago = self.ago.trim();
        let Some((last, _)) = ago.char_indices().last() else {
            return Err(refused());
        };
        let (count, unit) = ago.split_at(last);
        let count: u64 = count.parse().map_err(|_| refused())?;
        let unit: u64 = match unit {
            "m" => 60,
            "h" => 60 * 60,
            "d" => 24 * 60 * 60,
            _ => return Err(refused()),
        };

        count.checked_mul(unit).ok_or_else(refused)
    }

    /// The name of the mailbox the message is in, as the server lists it.
    pub(crate) fn mailbox(&self, place: &str) -> Result<&'static str, SceneError> {
        match self.folder.as_deref().map(str::trim) {
            None | Some("inbox") => Ok("INBOX"),
            Some("sent") => Ok("Sent"),
            Some("archive") => Ok("Archive"),
            Some("junk") => Ok("Junk"),
            Some("trash") => Ok("Trash"),
            Some(other) => Err(SceneError(format!(
                "{place}.folder: {other:?} is not inbox, sent, archive, junk or trash"
            ))),
        }
    }
}

/// A file named in a scene, in its folder and nowhere else: a path of plain names, with no way up
/// and no way to start somewhere else.
fn beside(folder: &Path, name: &str, place: &str) -> Result<PathBuf, SceneError> {
    let named = Path::new(name.trim());
    let plain = named
        .components()
        .all(|component| matches!(component, Component::Normal(_)));

    if name.trim().is_empty() || !plain {
        return Err(SceneError(format!(
            "{place}: {name:?} is not a file in the scene's folder"
        )));
    }

    Ok(folder.join(named))
}

/// The HTML in `name`, read from `folder`, with each picture it names there read in with it.
///
/// A picture is named the way a page names one beside it, `<img src="pictures/fern.png">`, so the
/// file opens in a browser as it will look. What is read in is what an `src` names with no scheme;
/// an address that names one, `https:` or `cid:`, is left as it is.
fn html_in(folder: &Path, name: &str, place: &str) -> Result<Html, SceneError> {
    let path = beside(folder, name, place)?;
    let html = std::fs::read_to_string(&path)
        .map_err(|why| SceneError(format!("{place}: {}: {why}", path.display())))?;

    let mut text = String::with_capacity(html.len());
    let mut pictures: Vec<Picture> = Vec::new();
    // Each file once, however often the HTML shows it: by the path it was named by.
    let mut read_in: Vec<(&str, String)> = Vec::new();
    let mut rest = html.as_str();

    while let Some(at) = rest.find("src=\"") {
        let (before, after) = rest.split_at(at + "src=\"".len());
        text.push_str(before);
        let Some(end) = after.find('"') else {
            return Err(SceneError(format!("{place}: an src that does not end")));
        };
        let (named, after) = after.split_at(end);

        if named.contains(':') || named.is_empty() {
            text.push_str(named);
        } else {
            let content_id = match read_in.iter().find(|(path, _)| *path == named) {
                Some((_, content_id)) => content_id.clone(),
                None => {
                    // A name of this build's making rather than the path: a path may hold what an
                    // address would have to escape, and the part is found by this name exactly.
                    let content_id = format!("picture-{}@scene.invalid", read_in.len() + 1);
                    pictures.push(picture_in(folder, named, &content_id, place)?);
                    read_in.push((named, content_id.clone()));
                    content_id
                }
            };
            text.push_str("cid:");
            text.push_str(&content_id);
        }
        rest = after;
    }
    text.push_str(rest);

    Ok(Html { text, pictures })
}

/// A picture beside a scene, by the name its HTML gives it, to travel as `content_id`.
fn picture_in(
    folder: &Path,
    named: &str,
    content_id: &str,
    place: &str,
) -> Result<Picture, SceneError> {
    let path = beside(folder, named, place)?;
    let content_type = match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        _ => {
            return Err(SceneError(format!(
                "{place}: {named:?} is not a PNG, JPEG, GIF or WebP picture"
            )))
        }
    };
    let bytes = std::fs::read(&path)
        .map_err(|why| SceneError(format!("{place}: {}: {why}", path.display())))?;

    Ok(Picture {
        content_id: content_id.to_owned(),
        content_type,
        bytes,
    })
}

/// An address that can stand on an account, checked the way the account screen checks one.
fn address_in<'a>(value: &'a str, place: &str) -> Result<&'a str, SceneError> {
    EmailAddress::parse(value)
        .map(|_| value.trim())
        .map_err(|why| SceneError(format!("{place}.address: {why}")))
}

/// `Name <address>` taken apart, or an address with no name.
fn sender_of<'a>(value: &'a str, place: &str) -> Result<(&'a str, &'a str), SceneError> {
    let value = value.trim();
    let (name, address) = match value
        .strip_suffix('>')
        .and_then(|rest| rest.rsplit_once('<'))
    {
        Some((name, address)) => (name.trim(), address.trim()),
        None => ("", value),
    };

    EmailAddress::parse(address).map_err(|why| SceneError(format!("{place}: {why}")))?;
    Ok((name, address))
}
