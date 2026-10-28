//! The build made for screenshots: `npm run app:screenshots`, the `screenshots` feature.
//!
//! It is this application in every part a person sees, over a profile of its own. At every start
//! that profile is emptied and a scene is written into it: the accounts and their groups, the
//! language, and the mail — read by the pass every account is read with, from servers that answer
//! out of the scene. No mail is read from a server and none is sent, the system's store of secrets
//! is never asked, and the real profile is never opened: the folder, the secrets and the
//! application's identity are all its own. What a person asks the window for still goes where it
//! would: a link, the pictures in a message, the settings a domain publishes for its mail.
//!
//! The scene is the folder the build carries as `scene`, which `scripts/screenshots.mjs` puts
//! there from `mocks/screenshots/`; `docs/development/guide.md` says what it may hold.

use secrecy::SecretString;

use super::Storage;
use crate::application::arrange_accounts::ArrangeAccounts;
use crate::application::sync::SyncAccount;
use crate::infrastructure::scene::{name_of, Account, Scene, Servers};
use crate::infrastructure::secure_storage::message_key::MessageKey;
use crate::infrastructure::sqlite::account_groups::SqliteAccountGroups;
use crate::infrastructure::sqlite::account_repository::SqliteAccountRepository;
use crate::infrastructure::sqlite::activity::SqliteAccountActivity;
use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
use crate::infrastructure::sqlite::message_bodies::SqliteMessageBodies;
use crate::infrastructure::sqlite::op_log::SqliteOperationLog;
use crate::infrastructure::sqlite::settings_repository::SqlitePreferencesRepository;
use crate::ports::account_repository::AccountRepository;
use crate::ports::activity::AccountActivity;
use crate::ports::credential_store::{CredentialPurpose, CredentialRef, CredentialStore};
use crate::ports::settings_repository::PreferencesRepository;
use lyonoyl_domain::account::{
    Encryption, MailAccount, MailAccountSummary, ServerSettings, Submission,
};
use lyonoyl_domain::account_group::Placed;
use lyonoyl_domain::message::{FolderId, MessageId};
use lyonoyl_domain::preferences::Preferences;

/// The folder the build carries its scene in, among its resources.
#[cfg(feature = "screenshots")]
pub const SCENE_FOLDER: &str = "scene";

/// A scene once it is written: the servers its mail is held on, and the message the window opens
/// on, with the folder it is read in, where the scene names one.
pub struct Played {
    pub servers: Servers,
    pub opening: Option<(MessageId, FolderId)>,
}

/// The password every account in a scene has. No server ever checks it: the servers are the scene.
const MADE_UP: &str = "scene";

/// The scene once it is played, which every later question about the mail is put to.
#[cfg(feature = "screenshots")]
static PLAYED: std::sync::OnceLock<Played> = std::sync::OnceLock::new();

/// The servers of the scene this build played, or servers holding nothing before it has played one.
#[cfg(feature = "screenshots")]
pub fn played() -> Servers {
    PLAYED
        .get()
        .map(|played| played.servers.clone())
        .unwrap_or_default()
}

/// The message the scene this build played opens on, and the folder it is read in.
#[cfg(feature = "screenshots")]
pub fn opened() -> Option<(MessageId, FolderId)> {
    PLAYED.get().and_then(|played| played.opening)
}

/// Empties the folder this build keeps its profile in, before anything opens a file there.
///
/// That folder and no other: its name is one no other build uses, and it is checked here rather
/// than trusted, because what follows removes everything in it.
#[cfg(feature = "screenshots")]
pub fn start_over(data: &std::path::Path) -> Result<(), String> {
    if data.file_name() != Some(std::ffi::OsStr::new(super::DATA_DIRECTORY)) {
        return Err(format!(
            "{} is not the folder a build for screenshots keeps its profile in",
            data.display()
        ));
    }

    match std::fs::remove_dir_all(data) {
        Ok(()) => {}
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => {}
        Err(why) => return Err(format!("{}: {why}", data.display())),
    }

    std::fs::create_dir_all(data).map_err(|why| format!("{}: {why}", data.display()))
}

/// Reads the scene this build carries and writes it into `storage`, dated from now.
///
/// A scene that cannot be read or played stops the start, saying which file and what is wrong with
/// it: a window over a scene played halfway is a screenshot of something that does not exist.
#[cfg(feature = "screenshots")]
pub fn play(
    app: &tauri::AppHandle,
    storage: &Storage,
    secrets: &crate::infrastructure::secure_storage::SecretStore,
) -> Result<(), String> {
    use tauri::Manager;

    let folder = app.path().resource_dir().map_err(said)?.join(SCENE_FOLDER);
    let scene = Scene::read(&folder).map_err(said)?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default();

    let played = played_into(storage, secrets, &scene, now)?;
    let _ = PLAYED.set(played);
    Ok(())
}

/// Writes `scene` into `storage` as of `now`, in Unix seconds, and answers with the servers its
/// mail is held on and the message the window opens on.
///
/// In the order a person would have made it: what they chose, then the accounts and the password
/// each was added with, then the groups, then every account read once. The reading is the pass any
/// account is read with, so the mailboxes, the conversations and the messages held whole are put
/// there by the code that puts anybody's there.
pub fn played_into<C: CredentialStore>(
    storage: &Storage,
    secrets: &C,
    scene: &Scene,
    now: i64,
) -> Result<Played, String> {
    let servers = Servers::holding(scene, now).map_err(said)?;
    let profile = storage.profile().map_err(|why| why.message)?;

    // Somebody who has used the application already, the welcome behind them, unless the scene
    // asks for it.
    SqlitePreferencesRepository::new(profile.clone())
        .save(
            &scene
                .dressed(Preferences::default())
                .map_err(said)?
                .with_language(scene.language().map_err(said)?)
                .with_welcomed(!scene.welcome),
        )
        .map_err(said)?;

    let accounts = SqliteAccountRepository::new(profile.clone());
    let mut added = Vec::with_capacity(scene.accounts.len());
    for account in &scene.accounts {
        let id = uuid::Uuid::new_v4().to_string();
        accounts.save(&as_added(&id, account)?).map_err(said)?;
        secrets
            .save(
                &CredentialRef::new(id.clone(), CredentialPurpose::ImapPassword),
                &SecretString::from(MADE_UP.to_owned()),
            )
            .map_err(said)?;
        added.push((account.address.trim().to_ascii_lowercase(), id));
    }
    let id_of = |address: &str| {
        let address = address.trim().to_ascii_lowercase();
        added
            .iter()
            .find(|(added, _)| *added == address)
            .map(|(_, id)| id.clone())
            .ok_or_else(|| format!("{address} is not an account in the scene"))
    };

    let arranging = ArrangeAccounts::new(SqliteAccountGroups::new(profile.clone()));
    let mut made = Vec::with_capacity(scene.groups.len());
    for group in &scene.groups {
        let group_id = arranging.create(&group.name).map_err(said)?;
        for member in &group.accounts {
            arranging
                .place(&id_of(member)?, Some(&group_id))
                .map_err(said)?;
        }
        if group.collapsed {
            arranging.set_collapsed(&group_id, true).map_err(said)?;
        }
        made.push(group_id);
    }

    // The top of the sidebar in the order the file lists it: an account in no group where it is
    // listed, a group where its first account is, and a group with nothing in it last. Inside a
    // group, the order its own list gives, which joining it one by one has already written.
    let mut order: Vec<Placed> = Vec::with_capacity(added.len() + made.len());
    for (address, id) in &added {
        let placed = match scene.groups.iter().position(|group| {
            group
                .accounts
                .iter()
                .any(|member| member.trim().eq_ignore_ascii_case(address))
        }) {
            Some(at) => Placed::Group(made[at].clone()),
            None => Placed::Account(id.clone()),
        };
        if !order.contains(&placed) {
            order.push(placed);
        }
    }
    for group_id in &made {
        let placed = Placed::Group(group_id.clone());
        if !order.contains(&placed) {
            order.push(placed);
        }
    }
    arranging.arrange(&order).map_err(said)?;

    let sealing = MessageKey::of_this_device(secrets).map_err(said)?;
    let activity = SqliteAccountActivity::new(profile.clone());
    for (address, id) in &added {
        let number = accounts
            .number_of(id)
            .map_err(said)?
            .ok_or_else(|| format!("{address} was added and has no number"))?;
        let index = storage
            .index_of_account(number)
            .map_err(|why| why.message)?;

        let pass = SyncAccount::new(
            SqliteAccountRepository::new(profile.clone()),
            secrets,
            servers.clone(),
            SqliteMailIndex::new(index.clone()),
            SqliteOperationLog::new(profile.clone()),
            SqliteMessageBodies::new(index, sealing.clone()),
            (),
        )
        .execute(id, number)
        .map_err(|why| format!("{address}: {why}"))?;

        if pass.folders_failed > 0 || pass.bodies_failed > 0 {
            return Err(format!(
                "{address}: {} mailboxes and {} messages could not be read",
                pass.folders_failed, pass.bodies_failed
            ));
        }

        // Read a moment ago, as an account that has just been checked is.
        activity.finished(id, now).map_err(said)?;
    }

    // Found by the name it was written under, in the mailbox the scene put it in.
    let opening = match scene.opening() {
        None => None,
        Some((at, number)) => {
            let (address, id) = &added[at];
            let message = &scene.accounts[at].mail[number];
            let mailbox = message
                .mailbox(&format!("accounts[{at}].mail[{number}]"))
                .map_err(said)?;
            let index = storage
                .index_of_account(
                    accounts
                        .number_of(id)
                        .map_err(said)?
                        .ok_or_else(|| format!("{address} was added and has no number"))?,
                )
                .map_err(|why| why.message)?;

            Some(
                crate::read_model::message::named_in(&index, &name_of(at, number), mailbox)
                    .map_err(said)?
                    .ok_or_else(|| {
                        format!("{address}: the message to open on was not read into {mailbox}")
                    })?,
            )
        }
    };

    Ok(Played { servers, opening })
}

/// The account as the account screen would have added it, with the servers its domain usually has.
///
/// Nothing connects to them. They are what the account's settings show, and a host made up from the
/// address is what a person adding that account would have been offered.
fn as_added(id: &str, account: &Account) -> Result<MailAccount, String> {
    let address = account.address.trim();
    let domain = address
        .rsplit_once('@')
        .map(|(_, domain)| domain)
        .unwrap_or(address);

    let summary =
        MailAccountSummary::new(id, address, account.name.trim(), "imap").map_err(said)?;
    let reading =
        ServerSettings::new(format!("imap.{domain}"), 993, Encryption::Tls).map_err(said)?;
    let sending =
        ServerSettings::new(format!("smtp.{domain}"), 465, Encryption::Tls).map_err(said)?;

    Ok(MailAccount::new(summary, reading, address)
        .map_err(said)?
        .sending_through(Submission::new(sending, address).map_err(said)?))
}

fn said(why: impl std::fmt::Display) -> String {
    why.to_string()
}
