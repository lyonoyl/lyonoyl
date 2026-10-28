//! A scene read, refused, and asked the way a pass asks a server.

use secrecy::SecretString;

use super::{Scene, Servers};
use crate::infrastructure::imap::described_whole;
use crate::ports::credential::Credential;
use crate::ports::mail_provider::{
    MailFetch, MailProviderError, MailSync, ProviderFactory, SyncIntent, SyncRequest,
};
use lyonoyl_domain::account::{Encryption, MailAccount, MailAccountSummary, ServerSettings};
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::message::{FolderId, Uid, UidValidity};
use lyonoyl_domain::sync::{MessageEnvelope, SyncBatch, SyncCursor};

/// When the scene is written, in Unix seconds.
const NOW: i64 = 1_800_000_000;

/// One account with a conversation of two, a message in its sent mail and one put away, and two
/// accounts with nothing in them, grouped.
const SCENE: &str = r#"{
  "language": "en",
  "accounts": [
    {
      "address": "zeta@example.com",
      "mail": [
        {
          "from": "Kappa Vandel <kappa@example.net>",
          "subject": "The plan for Thursday",
          "text": "Shall we start at ten?\nThe room is booked until noon.",
          "ago": "3h",
          "id": "plan"
        },
        {
          "from": "Sigma Ordrin <sigma@example.org>",
          "subject": "Re: The plan for Thursday",
          "text": "Ten works. I will bring the drawings.",
          "ago": "40m",
          "unread": true,
          "flagged": true,
          "answers": "plan"
        },
        {
          "from": "zeta@example.com",
          "to": "Kappa Vandel <kappa@example.net>",
          "subject": "Notes from the call",
          "text": "Three things we agreed on.",
          "ago": "2d",
          "folder": "sent"
        },
        {
          "from": "Kappa Vandel <kappa@example.net>",
          "subject": "Last year's drawings",
          "text": "Kept for reference.",
          "ago": "30d",
          "folder": "archive"
        }
      ]
    },
    { "address": "kappa@example.net" },
    { "address": "sigma@example.org", "name": "Sigma Ordrin" }
  ],
  "groups": [
    { "name": "Nine to five", "accounts": ["kappa@example.net", "sigma@example.org"] }
  ]
}"#;

fn account(address: &str) -> MailAccount {
    MailAccount::new(
        MailAccountSummary::new("any", address, "", "imap").expect("an account"),
        ServerSettings::new("imap.example.com", 993, Encryption::Tls).expect("a server"),
        address,
    )
    .expect("an account")
}

fn folder() -> FolderId {
    FolderId::new(7).expect("not zero")
}

fn credential() -> Credential {
    Credential::password(SecretString::from("anything".to_owned()))
}

fn asked(mailbox: &str, intent: SyncIntent, cursor: Option<&str>) -> SyncRequest {
    SyncRequest {
        folder_id: folder(),
        intent,
        mailbox: mailbox.to_owned(),
        cursor: cursor.map(SyncCursor::new),
        uid_validity: None,
        known_window: None,
        highest_modseq: None,
        known_count: 0,
        refused_for_now_last_time: false,
    }
}

fn servers() -> Servers {
    Servers::holding(&Scene::parse(SCENE).expect("the scene reads"), NOW).expect("it is written")
}

fn first_reading(mailbox: &str) -> SyncBatch {
    servers()
        .sync(&account("zeta@example.com"), &credential())
        .expect("its server")
        .fetch(&asked(mailbox, SyncIntent::Forward, None))
        .expect("an answer")
}

fn uid(value: u32) -> Uid {
    Uid::new(value).expect("not zero")
}

#[test]
fn every_account_has_the_mailboxes_a_server_lists_whether_or_not_it_holds_mail() {
    let servers = servers();

    for address in ["zeta@example.com", "kappa@example.net", "SIGMA@example.org"] {
        let listed = servers
            .discovery(&account(address), &credential())
            .expect("its server")
            .list_mailboxes()
            .expect("its mailboxes");

        let roles: Vec<_> = listed.iter().map(|mailbox| mailbox.role).collect();
        assert_eq!(
            roles,
            [
                FolderRole::Inbox,
                FolderRole::Drafts,
                FolderRole::Sent,
                FolderRole::Archive,
                FolderRole::Junk,
                FolderRole::Trash
            ],
            "{address}"
        );
        assert!(listed.iter().all(|mailbox| mailbox.uid_validity.is_some()));
    }
}

#[test]
fn the_first_reading_finds_the_inbox_oldest_first_and_described_from_its_own_bytes() {
    let batch = first_reading("INBOX");
    let delta = &batch.per_mailbox[0];

    assert_eq!(batch.cursor_before, None);
    assert_eq!(delta.folder_id, folder());
    assert_eq!(delta.covered_from, Some(uid(1)));
    assert!(!delta.stopped_short);

    let arrived: Vec<_> = delta
        .arrived
        .iter()
        .map(|message| {
            (
                message.uid,
                message.envelope.subject.as_deref(),
                message.envelope.sender.as_deref(),
                message.envelope.received_at,
            )
        })
        .collect();
    assert_eq!(
        arrived,
        [
            (
                uid(1),
                Some("The plan for Thursday"),
                Some("Kappa Vandel"),
                NOW - 3 * 60 * 60
            ),
            (
                uid(2),
                Some("Re: The plan for Thursday"),
                Some("Sigma Ordrin"),
                NOW - 40 * 60
            ),
        ]
    );

    let first = &delta.arrived[0].envelope;
    assert_eq!(first.sender_address.as_deref(), Some("kappa@example.net"));
    assert_eq!(first.to, ["zeta@example.com"]);
    assert!(
        first
            .preview
            .as_deref()
            .is_some_and(|preview| preview.starts_with("Shall we start at ten?")),
        "{:?}",
        first.preview
    );
}

#[test]
fn an_answer_names_what_it_answers_so_the_two_are_one_conversation() {
    let batch = first_reading("INBOX");
    let [plan, answer] = &batch.per_mailbox[0].arrived[..] else {
        panic!("two messages in the inbox");
    };

    let named = plan
        .envelope
        .rfc822_message_id
        .clone()
        .expect("the first message has a name");
    assert_eq!(answer.envelope.references, [named]);
}

#[test]
fn what_is_read_and_flagged_is_what_the_scene_says() {
    let batch = first_reading("INBOX");
    let [plan, answer] = &batch.per_mailbox[0].arrived[..] else {
        panic!("two messages in the inbox");
    };

    assert!(!plan.envelope.flags.is_unread());
    assert!(!plan
        .envelope
        .flags
        .contains(lyonoyl_domain::message::Flags::FLAGGED));
    assert!(answer.envelope.flags.is_unread());
    assert!(answer
        .envelope
        .flags
        .contains(lyonoyl_domain::message::Flags::FLAGGED));
}

#[test]
fn a_message_goes_in_the_mailbox_it_names_and_to_whoever_it_names() {
    let sent = first_reading("Sent");
    let [notes] = &sent.per_mailbox[0].arrived[..] else {
        panic!("one message in the sent mail");
    };
    assert_eq!(
        notes.envelope.subject.as_deref(),
        Some("Notes from the call")
    );
    assert_eq!(
        notes.envelope.sender_address.as_deref(),
        Some("zeta@example.com")
    );
    assert_eq!(notes.envelope.to, ["kappa@example.net"]);

    assert_eq!(first_reading("Archive").per_mailbox[0].arrived.len(), 1);
    assert!(first_reading("Trash").per_mailbox[0].arrived.is_empty());
}

#[test]
fn a_reading_after_the_first_finds_nothing_new_and_stays_where_it_was() {
    let servers = servers();
    let reading = servers
        .sync(&account("zeta@example.com"), &credential())
        .expect("its server");

    let first = reading
        .fetch(&asked("INBOX", SyncIntent::Forward, None))
        .expect("an answer");
    let again = reading
        .fetch(&asked(
            "INBOX",
            SyncIntent::Forward,
            Some(first.cursor_after.as_str()),
        ))
        .expect("an answer");

    assert_eq!(again.cursor_before, Some(first.cursor_after.clone()));
    assert_eq!(again.cursor_after, first.cursor_after);
    assert!(again.per_mailbox[0].arrived.is_empty());
}

#[test]
fn reading_further_back_finds_what_is_in_that_range_and_nothing_else() {
    let older = servers()
        .sync(&account("zeta@example.com"), &credential())
        .expect("its server")
        .fetch(&asked(
            "INBOX",
            SyncIntent::Older(uid(1)..=uid(1)),
            Some("scene"),
        ))
        .expect("an answer");

    let arrived: Vec<_> = older.per_mailbox[0]
        .arrived
        .iter()
        .map(|message| message.uid)
        .collect();
    assert_eq!(arrived, [uid(1)]);
    assert_eq!(older.per_mailbox[0].covered_from, Some(uid(1)));
}

#[test]
fn a_message_read_whole_is_the_message_the_reading_described() {
    let servers = servers();
    let zeta = account("zeta@example.com");
    let batch = servers
        .sync(&zeta, &credential())
        .expect("its server")
        .fetch(&asked("INBOX", SyncIntent::Forward, None))
        .expect("an answer");
    let delta = &batch.per_mailbox[0];
    let described = &delta.arrived[1];

    let whole = servers
        .reading(&zeta, &credential())
        .expect("its server")
        .whole_message("INBOX", delta.uid_validity, described.uid)
        .expect("the message");

    assert_eq!(
        MessageEnvelope {
            flags: described.envelope.flags,
            rfc822_size: u32::try_from(whole.len()).ok(),
            ..described_whole(&whole)
        },
        described.envelope
    );
}

#[test]
fn a_number_from_another_generation_or_one_that_is_not_there_is_refused() {
    let reading = servers()
        .reading(&account("zeta@example.com"), &credential())
        .expect("its server");

    assert_eq!(
        reading.whole_message("INBOX", UidValidity::new(2).expect("not zero"), uid(1)),
        Err(MailProviderError::Protocol)
    );
    assert_eq!(
        reading.whole_message("INBOX", described_generation(), uid(9)),
        Err(MailProviderError::Protocol)
    );
}

#[test]
fn an_account_the_scene_does_not_have_is_a_server_with_no_connection() {
    let servers = servers();
    let stranger = account("nobody@example.com");

    assert!(matches!(
        servers.discovery(&stranger, &credential()),
        Err(MailProviderError::Offline)
    ));
    assert!(matches!(
        servers.sync(&stranger, &credential()),
        Err(MailProviderError::Offline)
    ));
    assert!(matches!(
        servers.reading(&stranger, &credential()),
        Err(MailProviderError::Offline)
    ));
}

#[test]
fn a_name_in_another_script_survives_being_written_and_read_back() {
    let scene = Scene::parse(
        r#"{ "accounts": [ { "address": "zeta@example.com", "mail": [ {
            "from": "Σίγμα Ordrin <sigma@example.org>",
            "subject": "Ζήτα, the drawings",
            "text": "Κάππα has them.",
            "ago": "5m"
        } ] } ] }"#,
    )
    .expect("the scene reads");
    let batch = Servers::holding(&scene, NOW)
        .expect("it is written")
        .sync(&account("zeta@example.com"), &credential())
        .expect("its server")
        .fetch(&asked("INBOX", SyncIntent::Forward, None))
        .expect("an answer");

    let envelope = &batch.per_mailbox[0].arrived[0].envelope;
    assert_eq!(envelope.subject.as_deref(), Some("Ζήτα, the drawings"));
    assert_eq!(envelope.sender.as_deref(), Some("Σίγμα Ordrin"));
}

/// The generation the reading above stored the numbers under, as the index would have kept it.
fn described_generation() -> UidValidity {
    first_reading("INBOX").per_mailbox[0].uid_validity
}

/// What [`Scene::parse`] says about `text`, which must be a refusal.
fn refusal(text: &str) -> String {
    Scene::parse(text)
        .expect_err("the scene is refused")
        .to_string()
}

/// A scene of one account holding one message, with `field` added to the message.
fn one_message_with(field: &str) -> String {
    format!(
        r#"{{ "accounts": [ {{ "address": "zeta@example.com", "mail": [ {{
            "from": "kappa@example.net", "subject": "Hello", "text": "Hi.", {field}
        }} ] }} ] }}"#
    )
}

#[test]
fn a_whole_scene_reads() {
    let scene = Scene::parse(SCENE).expect("the scene reads");

    assert_eq!(scene.accounts.len(), 3);
    assert_eq!(scene.groups[0].accounts.len(), 2);
    assert_eq!(scene.language().expect("a language").as_str(), "en");
}

#[test]
fn a_scene_with_no_language_opens_in_the_systems() {
    let scene = Scene::parse(r#"{ "accounts": [ { "address": "zeta@example.com" } ] }"#)
        .expect("the scene reads");

    assert_eq!(scene.language().expect("a language").as_str(), "system");
}

#[test]
fn a_field_nothing_reads_is_refused_rather_than_ignored() {
    let refused = refusal(&one_message_with(r#""ago": "5m", "unraed": true"#));
    assert!(refused.contains("unraed"), "{refused}");
}

#[test]
fn a_scene_with_no_account_is_refused() {
    assert!(refusal(r#"{ "accounts": [] }"#).contains("at least one"));
}

#[test]
fn an_account_here_twice_is_refused_whatever_its_case() {
    let refused = refusal(
        r#"{ "accounts": [
            { "address": "zeta@example.com" },
            { "address": "Zeta@Example.com" }
        ] }"#,
    );
    assert!(refused.contains("accounts[1]"), "{refused}");
}

#[test]
fn an_address_that_is_not_one_is_refused_where_it_stands() {
    let refused = refusal(r#"{ "accounts": [ { "address": "zeta at example" } ] }"#);
    assert!(refused.contains("accounts[0].address"), "{refused}");

    let refused = refusal(
        r#"{ "accounts": [ { "address": "zeta@example.com", "mail": [ {
            "from": "Kappa <kappa at example>", "subject": "Hello", "text": "Hi.", "ago": "5m"
        } ] } ] }"#,
    );
    assert!(refused.contains("accounts[0].mail[0].from"), "{refused}");
}

#[test]
fn a_time_ago_is_minutes_hours_or_days_and_nothing_else() {
    for ago in ["40m", "5h", "3d", "0m"] {
        Scene::parse(&one_message_with(&format!(r#""ago": "{ago}""#)))
            .unwrap_or_else(|why| panic!("{ago}: {why}"));
    }

    // A unit in another script is a mistake to report, not a letter to cut in two.
    for ago in [
        "",
        "5",
        "m",
        "5x",
        "5 days",
        "-5m",
        "5\u{03bc}",
        "99999999999999999999d",
    ] {
        let refused = refusal(&one_message_with(&format!(r#""ago": "{ago}""#)));
        assert!(refused.contains(".ago"), "{ago}: {refused}");
    }
}

#[test]
fn a_mailbox_that_cannot_be_put_in_is_refused() {
    for folder in ["drafts", "Outbox", ""] {
        let refused = refusal(&one_message_with(&format!(
            r#""ago": "5m", "folder": "{folder}""#
        )));
        assert!(refused.contains(".folder"), "{folder}: {refused}");
    }
}

#[test]
fn an_answer_has_to_answer_a_message_that_is_here_and_arrived_before_it() {
    let scene = |first: &str, second: &str| {
        format!(
            r#"{{ "accounts": [ {{ "address": "zeta@example.com", "mail": [
                {{ "from": "kappa@example.net", "subject": "A", "text": "a", {first} }},
                {{ "from": "sigma@example.org", "subject": "B", "text": "b", {second} }}
            ] }} ] }}"#
        )
    };

    // Named further down the file than the answer is: still found.
    Scene::parse(&scene(
        r#""ago": "5m", "answers": "start""#,
        r#""ago": "1h", "id": "start""#,
    ))
    .expect("an answer to a message written below it");

    let refused = refusal(&scene(
        r#""ago": "5m", "answers": "nothing""#,
        r#""ago": "1h", "id": "start""#,
    ));
    assert!(refused.contains("no message here is named"), "{refused}");

    let refused = refusal(&scene(
        r#""ago": "2h", "answers": "start""#,
        r#""ago": "1h", "id": "start""#,
    ));
    assert!(refused.contains("before what it answers"), "{refused}");

    let refused = refusal(&scene(
        r#""ago": "2h", "id": "same""#,
        r#""ago": "1h", "id": "same""#,
    ));
    assert!(refused.contains("names two messages"), "{refused}");
}

#[test]
fn a_group_holds_accounts_that_are_here_each_in_one_group() {
    let accounts =
        r#""accounts": [ { "address": "zeta@example.com" }, { "address": "kappa@example.net" } ]"#;

    let refused = refusal(&format!(
        r#"{{ {accounts}, "groups": [ {{ "name": "Work", "accounts": ["sigma@example.org"] }} ] }}"#
    ));
    assert!(refused.contains("not an account here"), "{refused}");

    let refused = refusal(&format!(
        r#"{{ {accounts}, "groups": [
            {{ "name": "Work", "accounts": ["kappa@example.net"] }},
            {{ "name": "Home", "accounts": ["Kappa@example.net"] }}
        ] }}"#
    ));
    assert!(refused.contains("in another group"), "{refused}");

    let refused = refusal(&format!(
        r#"{{ {accounts}, "groups": [ {{ "name": "  ", "accounts": [] }} ] }}"#
    ));
    assert!(refused.contains("groups[0].name"), "{refused}");
}

#[test]
fn a_language_that_is_not_a_tag_is_refused() {
    let refused =
        refusal(r#"{ "language": "", "accounts": [ { "address": "zeta@example.com" } ] }"#);
    assert!(refused.contains("language"), "{refused}");
}

#[test]
fn the_window_opens_on_one_message_at_most() {
    let scene =
        Scene::parse(&one_message_with(r#""ago": "5m", "open": true"#)).expect("the scene reads");
    assert_eq!(scene.opening(), Some((0, 0)));

    let refused = refusal(
        r#"{ "accounts": [ { "address": "zeta@example.com", "mail": [
            { "from": "kappa@example.net", "subject": "A", "text": "a", "ago": "5m", "open": true },
            { "from": "kappa@example.net", "subject": "B", "text": "b", "ago": "9m", "open": true }
        ] } ] }"#,
    );
    assert!(refused.contains("open: 2 messages"), "{refused}");
}

#[test]
fn a_scene_that_says_nothing_of_it_is_shown_light_on_paper() {
    use lyonoyl_domain::preferences::{LightScheme, Preferences, Theme};

    let scene = Scene::parse(r#"{ "accounts": [ { "address": "zeta@example.com" } ] }"#)
        .expect("the scene reads");
    // Dressed over settings that say otherwise, so the answer is the scene's and not a default.
    let dressed = scene
        .dressed(
            Preferences::default()
                .with_theme(Theme::Dark)
                .with_light_scheme(LightScheme::CatppuccinLatte),
        )
        .expect("dressed");

    assert_eq!(dressed.theme(), Theme::Light);
    assert_eq!(dressed.light_scheme(), LightScheme::Paper);
}

#[test]
fn a_scene_names_its_appearance_and_one_of_its_palettes() {
    use lyonoyl_domain::preferences::{DarkScheme, Preferences, Theme};

    let scene = Scene::parse(
        r#"{ "appearance": "dark", "palette": "nord",
             "accounts": [ { "address": "zeta@example.com" } ] }"#,
    )
    .expect("the scene reads");
    let dressed = scene.dressed(Preferences::default()).expect("dressed");

    assert_eq!(dressed.theme(), Theme::Dark);
    assert_eq!(dressed.dark_scheme(), DarkScheme::Nord);
}

#[test]
fn an_appearance_or_a_palette_that_is_not_one_is_refused() {
    let account = r#""accounts": [ { "address": "zeta@example.com" } ]"#;

    for (fields, said) in [
        (r#""appearance": "sepia""#, "appearance"),
        (r#""palette": "nord""#, "not a light palette"),
        (
            r#""appearance": "dark", "palette": "paper""#,
            "not a dark palette",
        ),
        (
            r#""appearance": "system", "palette": "paper""#,
            "name light or dark",
        ),
    ] {
        let refused = refusal(&format!("{{ {fields}, {account} }}"));
        assert!(refused.contains(said), "{fields}: {refused}");
    }
}

#[test]
fn a_file_a_message_names_is_one_in_the_scenes_folder() {
    for named in [
        "../outside.html",
        "/etc/hosts",
        "",
        "pictures/../../up.html",
    ] {
        let refused = refusal(&one_message_with(&format!(
            r#""ago": "5m", "html": {}"#,
            serde_json::to_string(named).expect("a string")
        )));
        assert!(refused.contains(".html"), "{named}: {refused}");
    }
}

/// A scene's folder on disk, taken away afterwards.
struct SceneFolder(std::path::PathBuf);

impl SceneFolder {
    fn holding(files: &[(&str, &[u8])]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "lyonoyl-scene-files-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        for (name, bytes) in files {
            let file = path.join(name);
            std::fs::create_dir_all(file.parent().expect("a folder")).expect("a folder");
            std::fs::write(file, bytes).expect("written");
        }
        Self(path)
    }
}

impl Drop for SceneFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const PICTURE: &[u8] = b"\x89PNG\r\n\x1a\nnot a real picture";

fn with_html(html: &str) -> SceneFolder {
    SceneFolder::holding(&[
        (
            "scene.json",
            one_message_with(r#""ago": "5m", "html": "letter.html""#).as_bytes(),
        ),
        ("letter.html", html.as_bytes()),
        ("pictures/fern.png", PICTURE),
    ])
}

#[test]
fn a_message_written_in_html_carries_the_pictures_it_shows() {
    use mail_parser::MimeHeaders;

    let folder = with_html(
        r#"<p><img src="pictures/fern.png" alt="Fern"> <img src="pictures/fern.png">
           <img src="https://example.com/far.png"></p>"#,
    );
    let scene = Scene::read(&folder.0).expect("the scene reads");
    let raw = Servers::holding(&scene, NOW)
        .expect("written")
        .reading(&account("zeta@example.com"), &credential())
        .expect("its server")
        .whole_message("INBOX", described_generation(), uid(1))
        .expect("the message");

    let message = mail_parser::MessageParser::default()
        .parse(&raw)
        .expect("a message");
    let html = message.body_html(0).expect("an HTML body");
    assert_eq!(
        html.matches(r#"src="cid:picture-1@scene.invalid""#).count(),
        2,
        "the picture named twice is one part, named twice: {html}"
    );
    assert!(
        html.contains(r#"src="https://example.com/far.png""#),
        "{html}"
    );
    assert_eq!(message.body_text(0).as_deref(), Some("Hi."));

    let pictures: Vec<_> = message
        .parts
        .iter()
        .filter(|part| part.content_id().is_some())
        .collect();
    assert_eq!(pictures.len(), 1);
    assert_eq!(pictures[0].content_id(), Some("picture-1@scene.invalid"));
    assert_eq!(pictures[0].contents(), PICTURE);
}

#[test]
fn a_picture_that_is_not_there_or_not_a_picture_is_refused_where_it_is_named() {
    let missing = with_html(r#"<img src="pictures/gone.png">"#);
    let refused = Scene::read(&missing.0).expect_err("refused").to_string();
    assert!(refused.contains("accounts[0].mail[0].html"), "{refused}");

    let not_one = with_html(r#"<img src="letter.html">"#);
    let refused = Scene::read(&not_one.0).expect_err("refused").to_string();
    assert!(refused.contains("not a PNG"), "{refused}");
}

/// The build made for screenshots answers what a person does the way a server answers what it has
/// done, without a connection to anybody's: the window goes on as it would, and nothing waits.
#[test]
fn what_a_person_does_is_taken_and_goes_nowhere() {
    use crate::ports::mail_actions::{Filing, MailActions, RefusedSearch, SentMail, ServerAction};
    use crate::ports::mail_submission::MailSubmission;
    use lyonoyl_domain::message::Flags;
    use lyonoyl_domain::outgoing::{EmailAddress, Outgoing, Subject};

    let agrees = super::Agrees;
    let generation = described_generation();
    let mut acting = agrees.reach().expect("reached, with nothing to reach");
    for action in [
        ServerAction::SetFlags {
            mailbox: "INBOX",
            uid: uid(1),
            uid_validity: generation,
            add: Flags::SEEN,
            remove: Flags::NONE,
        },
        ServerAction::MoveTo {
            mailbox: "INBOX",
            uid: uid(1),
            uid_validity: generation,
            destination: "Archive",
        },
        ServerAction::Erase {
            mailbox: "Trash",
            uid: uid(1),
            uid_validity: generation,
        },
    ] {
        acting.carry_out(action).expect("taken");
    }

    let address = |text: &str| EmailAddress::parse(text).expect("an address");
    let written = Outgoing {
        from: address("zeta@example.com"),
        to: vec![address("kappa@example.net")],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: Subject::parse("Hello").expect("a subject"),
        body: "Hi.".to_owned(),
        thread: Default::default(),
        files: Vec::new(),
    };
    agrees
        .submit("1@example.com", &written, NOW)
        .expect("sent, as far as anybody can tell");
    assert_eq!(
        agrees.file("1@example.com", &written, NOW, RefusedSearch::Heeded),
        Ok(Filing::Added)
    );
}
