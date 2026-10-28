//! A scene written into a profile of its own, and what the window then reads out of it.

use super::init_storage;
use super::scene::played_into;
use crate::application::arrange_accounts::ArrangeAccounts;
use crate::application::sync::SyncAccount;
use crate::infrastructure::scene::Scene;
use crate::infrastructure::secure_storage::database_key::DatabaseKey;
use crate::infrastructure::secure_storage::in_memory::InMemory;
use crate::infrastructure::secure_storage::message_key::MessageKey;
use crate::infrastructure::sqlite::account_groups::SqliteAccountGroups;
use crate::infrastructure::sqlite::account_repository::SqliteAccountRepository;
use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
use crate::infrastructure::sqlite::message_bodies::SqliteMessageBodies;
use crate::infrastructure::sqlite::op_log::SqliteOperationLog;
use crate::infrastructure::sqlite::settings_repository::SqlitePreferencesRepository;
use crate::ports::credential_store::{CredentialPurpose, CredentialRef, CredentialStore};
use crate::ports::mail_index::MailIndex;
use crate::ports::settings_repository::PreferencesRepository;
use crate::read_model::list::ListReadModel;
use crate::read_model::mailboxes::{index_id_of, AccountNode, MailboxReadModel};
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::message::MessageId;

/// When the scene is written, in Unix seconds.
const NOW: i64 = 1_800_000_000;

/// The shape of the scene the screenshots are taken of: one account with mail, and a group of two
/// with none.
const SCENE: &str = r#"{
  "language": "en",
  "accounts": [
    {
      "address": "zeta@example.com",
      "mail": [
        {
          "from": "Kappa Vandel <kappa@example.net>",
          "subject": "The plan for Thursday",
          "text": "Shall we start at ten?",
          "ago": "3h",
          "id": "plan"
        },
        {
          "from": "Sigma Ordrin <sigma@example.org>",
          "subject": "Re: The plan for Thursday",
          "text": "Ten works.",
          "ago": "40m",
          "unread": true,
          "answers": "plan"
        },
        {
          "from": "zeta@example.com",
          "to": "kappa@example.net",
          "subject": "Notes from the call",
          "text": "Three things.",
          "ago": "2d",
          "folder": "sent"
        }
      ]
    },
    { "address": "kappa@example.net" },
    { "address": "sigma@example.org", "name": "Sigma Ordrin" }
  ],
  "groups": [
    { "name": "Nine to five", "accounts": ["sigma@example.org", "kappa@example.net"] }
  ]
}"#;

/// A folder of its own for the databases, taken away afterwards.
struct Folder(std::path::PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "lyonoyl-scene-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&path).expect("a folder");
        Self(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The scene written into fresh databases, and what the sidebar then draws.
struct Played {
    _folder: Folder,
    storage: super::Storage,
    secrets: InMemory,
    servers: crate::infrastructure::scene::Servers,
    opening: Option<(MessageId, lyonoyl_domain::message::FolderId)>,
}

impl Played {
    fn new(name: &str) -> Self {
        Self::of(name, SCENE)
    }

    fn of(name: &str, text: &str) -> Self {
        let folder = Folder::new(name);
        let storage = init_storage(&folder.0, &DatabaseKey::for_a_test());
        let secrets = InMemory::default();
        let scene = Scene::parse(text).expect("the scene reads");

        let played = played_into(&storage, &secrets, &scene, NOW).expect("the scene is played");

        Self {
            _folder: folder,
            storage,
            secrets,
            servers: played.servers,
            opening: played.opening,
        }
    }

    /// Everything at the top of the sidebar in the order the window draws it, a group by its name
    /// and an account by its address: one order for both, by the place each was given.
    fn top(&self) -> Vec<String> {
        let groups = ArrangeAccounts::new(SqliteAccountGroups::new(self.profile()))
            .list()
            .expect("the groups read");
        let grouped: Vec<&String> = groups.iter().flat_map(|group| &group.accounts).collect();

        let mut placed: Vec<(i64, String)> = groups
            .iter()
            .map(|group| (group.position, group.name.as_str().to_owned()))
            .collect();
        placed.extend(
            self.sidebar()
                .into_iter()
                .filter(|account| !grouped.contains(&&account.account_id))
                .map(|account| (account.place, account.email)),
        );
        placed.sort();
        placed.into_iter().map(|(_, label)| label).collect()
    }

    fn profile(&self) -> crate::infrastructure::sqlite::connection::SqlitePool {
        self.storage.profile().expect("the profile opens")
    }

    fn sidebar(&self) -> Vec<AccountNode> {
        MailboxReadModel::spanning(
            self.profile(),
            self.storage.index_across().expect("the mail opens"),
        )
        .tree()
        .expect("the sidebar reads")
    }

    fn account(&self, email: &str) -> AccountNode {
        self.sidebar()
            .into_iter()
            .find(|account| account.email == email)
            .unwrap_or_else(|| panic!("{email} is in the sidebar"))
    }

    fn number_of(&self, account: &AccountNode) -> i64 {
        index_id_of(&self.profile(), &account.account_id)
            .expect("the profile reads")
            .expect("the account has a number")
    }
}

/// What each mailbox of an account holds, by what it is for: total, then unread.
fn held(account: &AccountNode) -> Vec<(FolderRole, u32, u32)> {
    account
        .folders
        .iter()
        .map(|folder| (folder.role, folder.total, folder.unread))
        .collect()
}

#[test]
fn every_account_is_added_with_its_name_and_a_password_the_pass_can_read() {
    let played = Played::new("added");

    let sidebar = played.sidebar();
    let mut added: Vec<_> = sidebar
        .iter()
        .map(|account| (account.email.as_str(), account.display_name.as_str()))
        .collect();
    added.sort_unstable();
    assert_eq!(
        added,
        [
            ("kappa@example.net", ""),
            ("sigma@example.org", "Sigma Ordrin"),
            ("zeta@example.com", "")
        ]
    );

    for account in &sidebar {
        let password = played.secrets.read(&CredentialRef::new(
            account.account_id.clone(),
            CredentialPurpose::ImapPassword,
        ));
        assert!(password.is_ok(), "{}", account.email);
    }
}

#[test]
fn the_mail_is_in_the_mailboxes_the_scene_names_and_unread_where_it_says() {
    let played = Played::new("mail");

    assert_eq!(
        held(&played.account("zeta@example.com")),
        [
            (FolderRole::Inbox, 2, 1),
            (FolderRole::Drafts, 0, 0),
            (FolderRole::Sent, 1, 0),
            (FolderRole::Archive, 0, 0),
            (FolderRole::Junk, 0, 0),
            (FolderRole::Trash, 0, 0)
        ]
    );
}

/// The sidebar asks for a first read of every account it finds with no mailboxes, and in this build
/// that read would find nothing to say. An account with no mail has its mailboxes all the same.
#[test]
fn an_account_with_no_mail_has_its_mailboxes_empty_rather_than_none() {
    let played = Played::new("empty");

    for email in ["kappa@example.net", "sigma@example.org"] {
        let account = played.account(email);
        assert_eq!(account.folders.len(), 6, "{email}");
        assert!(
            held(&account).iter().all(|&(_, total, _)| total == 0),
            "{email}"
        );
    }
}

#[test]
fn every_message_is_held_whole_so_it_opens_with_nothing_to_fetch() {
    let played = Played::new("whole");
    let zeta = played.account("zeta@example.com");
    let index = played
        .storage
        .index_of_account(played.number_of(&zeta))
        .expect("its mail opens");

    for folder in &zeta.folders {
        let waiting = SqliteMailIndex::new(index.clone())
            .not_read_whole(folder.folder_id, 100)
            .expect("the index reads");
        assert!(waiting.is_empty(), "{:?}", folder.role);
    }
}

#[test]
fn an_answer_and_what_it_answers_are_one_conversation() {
    let played = Played::new("conversation");
    let zeta = played.account("zeta@example.com");
    let index = played
        .storage
        .index_of_account(played.number_of(&zeta))
        .expect("its mail opens");

    let plan: i64 = index
        .get()
        .expect("a connection")
        .query_row(
            "SELECT id FROM messages WHERE subject = 'The plan for Thursday'",
            [],
            |row| row.get(0),
        )
        .expect("the first message is there");

    let conversation = ListReadModel::spanning(
        played.storage.index_across().expect("the mail opens"),
        played.profile(),
    )
    .conversation_of(MessageId::new(plan as u64).expect("a message number"))
    .expect("its conversation reads");

    let subjects: Vec<_> = conversation
        .iter()
        .map(|row| row.subject.as_deref())
        .collect();
    assert_eq!(
        subjects,
        [
            Some("The plan for Thursday"),
            Some("Re: The plan for Thursday")
        ]
    );
}

/// Every account is added within the same second, so without an order written down the sidebar
/// would draw them in whatever order their identifiers fall: a different screenshot on every start.
#[test]
fn the_top_of_the_sidebar_is_in_the_order_the_file_lists_it() {
    let played = Played::new("order");

    assert_eq!(played.top(), ["zeta@example.com", "Nine to five"]);
}

#[test]
fn a_group_stands_where_its_first_account_is_listed_and_an_empty_one_last() {
    let played = Played::of(
        "order-listed",
        r#"{
          "accounts": [
            { "address": "kappa@example.net" },
            { "address": "zeta@example.com" },
            { "address": "sigma@example.org" }
          ],
          "groups": [
            { "name": "Empty", "accounts": [] },
            { "name": "Work", "accounts": ["sigma@example.org", "kappa@example.net"] }
          ]
        }"#,
    );

    assert_eq!(played.top(), ["Work", "zeta@example.com", "Empty"]);
}

#[test]
fn the_window_is_light_on_paper_where_the_scene_says_nothing_of_it() {
    use lyonoyl_domain::preferences::{LightScheme, Theme};

    let played = Played::new("appearance");
    let preferences = SqlitePreferencesRepository::new(played.profile())
        .load()
        .expect("the preferences read");

    assert_eq!(preferences.theme(), Theme::Light);
    assert_eq!(preferences.light_scheme(), LightScheme::Paper);
}

#[test]
fn the_window_opens_on_the_message_the_scene_names_in_the_folder_it_is_in() {
    let played = Played::of(
        "opening",
        &SCENE.replacen(r#""unread": true,"#, r#""unread": true, "open": true,"#, 1),
    );
    let (message, folder) = played.opening.expect("it names one");

    let zeta = played.account("zeta@example.com");
    let inbox = zeta
        .folders
        .iter()
        .find(|one| one.role == FolderRole::Inbox)
        .expect("an inbox");
    assert_eq!(folder, inbox.folder_id);

    let index = played
        .storage
        .index_of_account(played.number_of(&zeta))
        .expect("its mail opens");
    let details = crate::read_model::message::MessageReadModel::new(index)
        .get_message(message)
        .expect("readable")
        .expect("it is there");
    assert_eq!(
        details.subject.as_deref(),
        Some("Re: The plan for Thursday")
    );
}

#[test]
fn a_scene_asks_for_the_welcome_and_a_scene_that_says_nothing_has_it_behind() {
    let asked = Played::of(
        "welcome",
        &SCENE.replacen(
            "{\n  \"language\"",
            "{\n  \"welcome\": true,\n  \"language\"",
            1,
        ),
    );
    let quiet = Played::new("no-welcome");
    let welcomed = |played: &Played| {
        SqlitePreferencesRepository::new(played.profile())
            .load()
            .expect("the preferences read")
            .welcomed()
    };

    assert!(!welcomed(&asked), "the window opens on the welcome");
    assert!(welcomed(&quiet), "the welcome is behind the person");
}

#[test]
fn a_scene_that_names_no_message_opens_on_none() {
    assert_eq!(Played::new("no-opening").opening, None);
}

/// The scenes the build for screenshots plays, each in a folder named by its language.
fn the_scenes_kept_here() -> Vec<std::path::PathBuf> {
    let kept = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../mocks/screenshots");
    let mut folders: Vec<_> = std::fs::read_dir(&kept)
        .unwrap_or_else(|why| panic!("{}: {why}", kept.display()))
        .map(|entry| entry.expect("an entry").path())
        .filter(|path| {
            path.join(crate::infrastructure::scene::SCENE_FILE)
                .is_file()
        })
        .collect();
    folders.sort();
    folders
}

/// What `npm run app:screenshots` would refuse as the window starts, refused here first.
#[test]
fn every_scene_kept_here_reads_and_plays() {
    let scenes = the_scenes_kept_here();
    assert!(
        scenes.iter().any(|folder| folder.ends_with("en")),
        "English is the scene played where no language is asked for: {scenes:?}"
    );

    for folder in scenes {
        let scene = Scene::read(&folder).unwrap_or_else(|why| panic!("{why}"));
        let databases = Folder::new("kept");
        let storage = init_storage(&databases.0, &DatabaseKey::for_a_test());

        played_into(&storage, &InMemory::default(), &scene, NOW)
            .unwrap_or_else(|why| panic!("{}: {why}", folder.display()));
    }
}

#[test]
fn the_group_holds_its_accounts_in_the_order_the_scene_gives() {
    let played = Played::new("group");
    let email_of = |id: &str| {
        played
            .sidebar()
            .into_iter()
            .find(|account| account.account_id == id)
            .map(|account| account.email)
            .expect("a member is an account")
    };

    let groups = ArrangeAccounts::new(SqliteAccountGroups::new(played.profile()))
        .list()
        .expect("the groups read");
    let [group] = &groups[..] else {
        panic!("one group, not {}", groups.len());
    };

    assert_eq!(group.name.as_str(), "Nine to five");
    assert!(!group.collapsed);
    let members: Vec<_> = group.accounts.iter().map(|id| email_of(id)).collect();
    assert_eq!(members, ["sigma@example.org", "kappa@example.net"]);
}

#[test]
fn the_window_opens_in_the_scenes_language_with_the_welcome_behind_it() {
    let played = Played::new("preferences");

    let preferences = SqlitePreferencesRepository::new(played.profile())
        .load()
        .expect("the preferences read");

    assert!(preferences.welcomed());
    assert_eq!(preferences.language().as_str(), "en");
}

/// What the refresh button does in this build: the same pass, against the same servers.
#[test]
fn reading_an_account_again_finds_nothing_new_and_loses_nothing() {
    let played = Played::new("again");
    let zeta = played.account("zeta@example.com");
    let number = played.number_of(&zeta);
    let index = played
        .storage
        .index_of_account(number)
        .expect("its mail opens");

    let pass = SyncAccount::new(
        SqliteAccountRepository::new(played.profile()),
        &played.secrets,
        played.servers.clone(),
        SqliteMailIndex::new(index.clone()),
        SqliteOperationLog::new(played.profile()),
        SqliteMessageBodies::new(
            index,
            MessageKey::of_this_device(&played.secrets).expect("the key is kept"),
        ),
        (),
    )
    .execute(&zeta.account_id, number)
    .expect("the pass reads");

    assert_eq!((pass.arrived, pass.updated, pass.removed), (0, 0, 0));
    assert_eq!(
        held(&played.account("zeta@example.com"))[0],
        (FolderRole::Inbox, 2, 1)
    );
}

/// A message a scene writes in HTML is shown whole as the window opens it: every picture it shows
/// travelled inside it, and nothing waits for the reader to allow pictures from the web. A banner
/// about blocked pictures is not what a screenshot is taken of.
#[test]
fn every_message_kept_here_in_html_shows_whole_with_its_pictures() {
    use crate::infrastructure::html::body_of;
    use crate::infrastructure::scene::{name_of, Servers};
    use crate::ports::body::{BodyKind, Viewing};

    let mut shown = 0;
    for folder in the_scenes_kept_here() {
        let scene = Scene::read(&folder).unwrap_or_else(|why| panic!("{why}"));
        let servers = Servers::holding(&scene, NOW).expect("written");

        for (at, account) in scene.accounts.iter().enumerate() {
            for (number, message) in account.mail.iter().enumerate() {
                let Some(html) = &message.written else {
                    continue;
                };
                let place = format!("{}: {}", folder.display(), message.subject);
                let raw = servers
                    .written_named(&name_of(at, number))
                    .unwrap_or_else(|| panic!("{place}: not written"));
                let body = body_of(&raw, Viewing::plainly());

                assert_eq!(body.kind, BodyKind::Html, "{place}");
                assert_eq!(
                    (
                        body.remote_blocked,
                        body.remote_not_shown,
                        body.remote_not_had
                    ),
                    (0, 0, 0),
                    "{place}: a picture it shows is not inside it"
                );
                assert_eq!(body.parts.len(), html.pictures.len(), "{place}");
                assert!(body.attachments.is_empty(), "{place}");
                shown += 1;
            }
        }
    }
    assert!(shown > 0, "no scene kept here writes a message in HTML");
}
