use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;

use crate::application::accounts::{AddAccount, TestConnection};
use crate::application::error::UseCaseError;
use crate::application::find_server_settings::{FindServerSettings, Settings};
use crate::application::remove_account::RemoveAccount;
use crate::application::what_is_held::WhatIsHeld;
use crate::bootstrap::adapters;
use crate::commands::error::{CommandError, ErrorCode};
use crate::commands::workspace::MailAccountSummaryDto;
use crate::infrastructure::secure_storage::cache::CachedCredentials;
use crate::ports::account_repository::AccountRepository;
use crate::ports::credential_store::CredentialStore;
use crate::AppState;
use lyonoyl_domain::account::{
    default_imap_port, Encryption, MailAccount, MailAccountSummary, ServerSettings, Submission,
};
use lyonoyl_domain::error::DomainError;

/// How the connection is protected, as the form offers it.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum EncryptionDto {
    Tls,
    StartTls,
    None,
}

/// The other direction, which settings found for a person travel in.
impl From<Encryption> for EncryptionDto {
    fn from(encryption: Encryption) -> Self {
        match encryption {
            Encryption::Tls => Self::Tls,
            Encryption::StartTls => Self::StartTls,
            Encryption::None => Self::None,
        }
    }
}

impl From<EncryptionDto> for Encryption {
    fn from(encryption: EncryptionDto) -> Self {
        match encryption {
            EncryptionDto::Tls => Self::Tls,
            EncryptionDto::StartTls => Self::StartTls,
            EncryptionDto::None => Self::None,
        }
    }
}

/// What the user typed into the account form.
///
/// The password is a separate argument rather than a field here: it is never part of a structure
/// that could end up in a log line or in an error message alongside the settings.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct AccountSettingsDto {
    pub email: String,
    pub display_name: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_encryption: EncryptionDto,
    /// Usually the address, but some servers want a different one.
    pub imap_username: String,
    /// Where messages written from this account are handed over.
    ///
    /// Absent is an ordinary answer: reading and sending are separate servers, and an account
    /// nobody has given a sending server still receives mail. What it cannot do is send, and it
    /// says so rather than losing what was written.
    pub submission: Option<SubmissionDto>,
}

/// The sending half of an account's settings.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct SubmissionDto {
    pub host: String,
    pub port: u16,
    pub encryption: EncryptionDto,
    /// Usually the address, and usually the same as the one for reading.
    pub username: String,
}

/// What the server answered when the settings were tried.
#[derive(Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct ConnectionReportDto {
    /// How many mailboxes the server offered. Evidence that the settings work, rather than a claim.
    pub mailbox_count: u32,
}

/// The settings of an account already here, to fill the form with.
///
/// No password: it is in this device's secret store, and the form is being opened precisely because
/// it has to be typed again.
#[tauri::command]
pub async fn get_account_settings(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<AccountSettingsDto, CommandError> {
    let pool = state.storage.profile()?;

    let account =
        tauri::async_runtime::spawn_blocking(move || adapters::accounts(pool).find(&account_id))
            .await
            .map_err(background_task_failed)?
            .map_err(UseCaseError::from)?
            .ok_or_else(|| {
                CommandError::new(
                    ErrorCode::InvalidRequest,
                    "That account is not on this device.",
                )
            })?;

    Ok(AccountSettingsDto {
        email: account.summary().email().to_owned(),
        display_name: account.summary().display_name().to_owned(),
        imap_host: account.imap().host().to_owned(),
        imap_port: account.imap().port(),
        imap_encryption: match account.imap().encryption() {
            Encryption::Tls => EncryptionDto::Tls,
            Encryption::StartTls => EncryptionDto::StartTls,
            Encryption::None => EncryptionDto::None,
        },
        imap_username: account.imap_username().to_owned(),
        submission: account.submission().map(|sending| SubmissionDto {
            host: sending.server().host().to_owned(),
            port: sending.server().port(),
            encryption: match sending.server().encryption() {
                Encryption::Tls => EncryptionDto::Tls,
                Encryption::StartTls => EncryptionDto::StartTls,
                Encryption::None => EncryptionDto::None,
            },
            username: sending.username().to_owned(),
        }),
    })
}

/// The port a provider is expected on, so the form can offer it when the encryption changes.
#[tauri::command]
pub fn default_port_for(encryption: EncryptionDto) -> u16 {
    default_imap_port(encryption.into())
}

/// Tries the settings against the server without storing anything.
#[tauri::command]
pub async fn test_account_connection(
    settings: AccountSettingsDto,
    password: String,
    state: State<'_, AppState>,
) -> Result<ConnectionReportDto, CommandError> {
    // Storage is not touched, but a workspace with broken storage cannot add the account that
    // would follow, and saying so here is better than after the password was typed.
    state.storage.profile()?;

    let account = build_account(new_account_id(), settings)?;
    let password = SecretString::from(password);
    let runtime = tauri::async_runtime::handle().inner().clone();

    let found = tauri::async_runtime::spawn_blocking(move || {
        TestConnection::new(crate::bootstrap::mail_servers(runtime)).execute(&account, &password)
    })
    .await
    .map_err(background_task_failed)??;

    Ok(ConnectionReportDto {
        mailbox_count: found as u32,
    })
}

/// Takes an account off this device: its mail, its secrets and what it was asked to do.
///
/// Not undoable, and deliberately not softened into something that looks like it is. What comes
/// back is nothing rather than a summary, because there is nothing left to describe.
#[tauri::command]
pub async fn remove_account(
    account_id: String,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let profile = state.storage.profile()?;
    let secrets = passwords(&state.credentials, &state.secrets);
    // Cloned rather than reached through the state inside the task: choosing a file is done on the
    // thread that reads, never on the one answering the window.
    let indexes = state.storage.every_index()?;

    tauri::async_runtime::spawn_blocking(move || {
        RemoveAccount::new(adapters::accounts(profile), secrets, indexes).execute(&account_id)
    })
    .await
    .map_err(background_task_failed)??;

    Ok(())
}

/// One account and what its mail weighs on this device.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct HeldDto {
    pub account_id: String,
    pub email: String,
    pub display_name: String,
    /// Bytes, as text: a mailbox can hold more than a JavaScript number holds exactly.
    pub bytes: String,
}

/// How much of this device the mail on it is taking, per account, largest first.
///
/// Nothing here clears anything. What a person is owed is the number, so that clearing is a thing
/// they can choose rather than something the application decides for them.
#[tauri::command]
pub async fn what_is_held(state: State<'_, AppState>) -> Result<Vec<HeldDto>, CommandError> {
    let profile = state.storage.profile()?;
    let indexes = state.storage.every_index()?;

    let held = tauri::async_runtime::spawn_blocking(move || {
        WhatIsHeld::new(adapters::accounts(profile), indexes).execute()
    })
    .await
    .map_err(background_task_failed)??;

    Ok(held
        .into_iter()
        .map(|one| HeldDto {
            account_id: one.account_id,
            email: one.email,
            display_name: one.display_name,
            bytes: one.bytes.to_string(),
        })
        .collect())
}

/// Adds the account: verifies it against the server, stores the password in this device's secret
/// store, then writes the account itself.
#[tauri::command]
pub async fn add_account(
    settings: AccountSettingsDto,
    password: String,
    state: State<'_, AppState>,
) -> Result<MailAccountSummaryDto, CommandError> {
    let pool = state.storage.profile()?;

    let account = build_account(new_account_id(), settings)?;
    // An account just added can send exactly when the form said where to send from.
    let summary = MailAccountSummaryDto::of(account.summary(), account.submission().is_some());
    let password = SecretString::from(password);
    let runtime = tauri::async_runtime::handle().inner().clone();
    // Where this device keeps secrets, which on a system without a store of its own is a file that
    // has to have been opened first.
    let secrets = passwords(&state.credentials, &state.secrets);

    tauri::async_runtime::spawn_blocking(move || {
        AddAccount::new(
            adapters::accounts(pool),
            secrets,
            crate::bootstrap::mail_servers(runtime),
        )
        .execute(account, password)
    })
    .await
    .map_err(background_task_failed)??;

    // Read by the sidebar, not from here: it asks for the first read of every account with no
    // folders the moment it sees one, and that read is a person's and is shown as one. A second,
    // quiet read asked for here would race it for the account, and the one the person can see
    // would be the one to lose.
    Ok(summary)
}

#[tauri::command]
/// Changes where an account's mail is fetched from, and the password it is fetched with.
///
/// Separate from [`add_account`] because the two refuse opposite things: adding refuses an address
/// already here, changing refuses one that is not. The identifier is the caller's, not the form's —
/// it names the account's folders, its mail and its place in the operation log.
pub async fn change_account(
    account_id: String,
    settings: AccountSettingsDto,
    password: String,
    state: State<'_, AppState>,
) -> Result<MailAccountSummaryDto, CommandError> {
    let pool = state.storage.profile()?;

    let account = build_account(account_id.clone(), settings)?;
    let summary = MailAccountSummaryDto::of(account.summary(), account.submission().is_some());
    let password = SecretString::from(password);
    let runtime = tauri::async_runtime::handle().inner().clone();
    let secrets = passwords(&state.credentials, &state.secrets);

    tauri::async_runtime::spawn_blocking(move || {
        crate::application::accounts::ChangeAccount::new(
            adapters::accounts(pool),
            secrets,
            crate::bootstrap::mail_servers(runtime),
        )
        .execute(&account_id, account, password)
    })
    .await
    .map_err(background_task_failed)??;

    Ok(summary)
}

/// What an account's passwords are written and taken out through: the passwords held this run,
/// which every pass, push, watch, body fetch and send reads, over the store this device keeps its
/// secrets in.
///
/// Never the store alone. What is held is read once per run and never let go of on its own, so a
/// password written past it leaves the old one held — and sent, to whichever server the account
/// was just pointed at — until the application starts again, and one taken out past it goes on
/// being sent for an account that is gone.
fn passwords<S: CredentialStore + Clone>(
    held: &CachedCredentials<S>,
    store: &S,
) -> CachedCredentials<S> {
    held.sharing(store.clone())
}

/// A new opaque identifier.
///
/// Opaque rather than derived from the address: an account keeps its identity — its folders, its
/// stored password, its place in the operation log — when the person corrects a typo in their
/// address or moves it to another server.
fn new_account_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Maps the form onto the domain, which is where the settings are actually validated.
fn build_account(id: String, settings: AccountSettingsDto) -> Result<MailAccount, CommandError> {
    let summary = MailAccountSummary::new(
        id,
        settings.email,
        settings.display_name,
        // The field names the provider, and `imap` is the only one (AGENTS.md section 6).
        "imap",
    )
    .map_err(invalid_settings)?;

    let server = ServerSettings::new(
        settings.imap_host,
        settings.imap_port,
        settings.imap_encryption.into(),
    )
    .map_err(invalid_settings)?;

    let account =
        MailAccount::new(summary, server, settings.imap_username).map_err(invalid_settings)?;

    let Some(sending) = settings.submission else {
        return Ok(account);
    };

    let where_to = ServerSettings::new(sending.host, sending.port, sending.encryption.into())
        .map_err(invalid_settings)?;

    Ok(account
        .sending_through(Submission::new(where_to, sending.username).map_err(invalid_settings)?))
}

/// Points at the field the person has to fix, rather than reporting that something was wrong.
fn invalid_settings(error: DomainError) -> CommandError {
    let code = match error {
        DomainError::EmptyEmail | DomainError::MalformedEmail => ErrorCode::InvalidEmail,
        DomainError::EmptyHost | DomainError::MalformedHost => ErrorCode::InvalidHost,
        DomainError::InvalidPort => ErrorCode::InvalidPort,
        DomainError::EmptyUsername => ErrorCode::InvalidUsername,
        _ => ErrorCode::InvalidRequest,
    };

    CommandError::new(code, error.to_string())
}

fn background_task_failed(_error: tauri::Error) -> CommandError {
    CommandError::new(ErrorCode::Internal, "A background task failed.")
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::application::accounts::ChangeAccount;
    use crate::infrastructure::secure_storage::in_memory::InMemory;
    use crate::ports::account_repository::AccountRepositoryError;
    use crate::ports::credential::Credential;
    use crate::ports::credential_store::{CredentialError, CredentialPurpose, CredentialRef};
    use crate::ports::mail_files::{MailFiles, MailFilesError};
    use crate::ports::mail_provider::{
        DiscoveredMailbox, MailFetch, MailProviderError, MailSync, MailWatch, MailboxDiscovery,
        ProviderFactory,
    };
    use lyonoyl_domain::folder::FolderRole;

    fn settings() -> AccountSettingsDto {
        AccountSettingsDto {
            email: "user@example.com".to_owned(),
            display_name: "User".to_owned(),
            imap_host: "imap.example.com".to_owned(),
            imap_port: 993,
            imap_encryption: EncryptionDto::Tls,
            imap_username: "user@example.com".to_owned(),
            submission: None,
        }
    }

    fn sending() -> SubmissionDto {
        SubmissionDto {
            host: "smtp.example.com".to_owned(),
            port: 587,
            encryption: EncryptionDto::StartTls,
            username: "user@example.com".to_owned(),
        }
    }

    #[test]
    fn a_filled_in_form_becomes_an_account() {
        let account =
            build_account("acc-1".to_owned(), settings()).expect("the settings are valid");

        assert_eq!(account.summary().email(), "user@example.com");
        assert_eq!(account.imap().host(), "imap.example.com");
        assert_eq!(account.imap().port(), 993);
        assert_eq!(account.imap().encryption(), Encryption::Tls);
        assert_eq!(account.imap_username(), "user@example.com");
    }

    #[test]
    fn each_mistake_names_the_field_it_belongs_to() {
        // A form that cannot highlight the wrong field makes the person re-read all of them.
        for (broken, expected) in [
            (
                AccountSettingsDto {
                    email: "not-an-address".to_owned(),
                    ..settings()
                },
                ErrorCode::InvalidEmail,
            ),
            (
                AccountSettingsDto {
                    imap_host: "imaps://imap.example.com".to_owned(),
                    ..settings()
                },
                ErrorCode::InvalidHost,
            ),
            (
                AccountSettingsDto {
                    imap_port: 0,
                    ..settings()
                },
                ErrorCode::InvalidPort,
            ),
            (
                AccountSettingsDto {
                    imap_username: "   ".to_owned(),
                    ..settings()
                },
                ErrorCode::InvalidUsername,
            ),
        ] {
            let error = build_account("acc-1".to_owned(), broken).expect_err("the form is wrong");
            assert_eq!(
                std::mem::discriminant(&error.code),
                std::mem::discriminant(&expected),
                "wrong field reported: {:?}",
                error.code
            );
        }
    }

    #[test]
    fn two_accounts_never_share_an_identifier() {
        // The identifier names the folder rows and the keychain entry; a collision would hand one
        // account another's mail.
        let first = new_account_id();
        let second = new_account_id();

        assert_ne!(first, second);
        assert_eq!(first.len(), 36, "an opaque identifier, not an address");
    }

    #[test]
    fn the_form_is_offered_the_port_each_encryption_is_expected_on() {
        assert_eq!(default_port_for(EncryptionDto::Tls), 993);
        assert_eq!(default_port_for(EncryptionDto::StartTls), 143);
        assert_eq!(default_port_for(EncryptionDto::None), 143);
    }

    #[test]
    fn a_form_that_says_where_to_send_makes_an_account_that_can() {
        let account = build_account(
            "acc-1".to_owned(),
            AccountSettingsDto {
                submission: Some(sending()),
                ..settings()
            },
        )
        .expect("the settings are valid");

        let where_to = account.submission().expect("it can send");
        assert_eq!(where_to.server().host(), "smtp.example.com");
        assert_eq!(where_to.server().port(), 587);
        assert_eq!(where_to.username(), "user@example.com");
    }

    #[test]
    fn a_form_that_says_nothing_about_sending_makes_an_account_that_reads() {
        // Not a refusal. Mail arrives either way, and demanding an SMTP port before the account
        // can be added is how somebody gives up on the first screen.
        let account = build_account("acc-1".to_owned(), settings()).expect("valid");

        assert_eq!(account.submission(), None);
    }

    #[test]
    fn a_sending_server_with_no_host_is_pointed_at_rather_than_stored() {
        let outcome = build_account(
            "acc-1".to_owned(),
            AccountSettingsDto {
                submission: Some(SubmissionDto {
                    host: "   ".to_owned(),
                    ..sending()
                }),
                ..settings()
            },
        );

        assert!(outcome.is_err(), "an empty host was accepted as a server");
    }

    #[test]
    fn servers_taken_from_another_account_carry_this_address_and_nothing_of_theirs() {
        use crate::application::find_server_settings::{Found, SignIn, SignInAs};
        use lyonoyl_domain::provider::{ServerSettings as Servers, Submission as Sending};

        let looked = Settings::Known {
            servers: Box::new(Servers {
                imap_host: "imap.example.net".to_owned(),
                imap_port: 143,
                imap_encryption: Encryption::StartTls,
                smtp: Some(Sending {
                    host: "smtp.example.net".to_owned(),
                    port: 587,
                    encryption: Encryption::StartTls,
                }),
            }),
            sign_in: SignIn {
                imap: SignInAs::LocalPart,
                submission: SignInAs::Address,
            },
            found: Found::OnThisDevice,
        };

        let form = found_settings(" kappa@example.com ", looked).expect("something was found");

        assert_eq!(form.imap_host, "imap.example.net");
        assert_eq!(form.imap_port, 143);
        assert!(matches!(form.imap_encryption, EncryptionDto::StartTls));
        assert_eq!(form.imap_username, "kappa");
        assert_eq!(form.display_name, "", "a name is the person's to give");
        let sending = form.submission.expect("the sending server came with it");
        assert_eq!(sending.host, "smtp.example.net");
        assert_eq!(sending.port, 587);
        assert_eq!(sending.username, "kappa@example.com");
    }

    #[test]
    fn nothing_found_fills_nothing_in() {
        assert!(found_settings("kappa@example.com", Settings::Unknown).is_none());
    }

    /// The profile, holding the one account or none.
    struct TheProfile(Option<MailAccount>);

    impl AccountRepository for TheProfile {
        fn list(&self) -> Result<Vec<MailAccountSummary>, AccountRepositoryError> {
            unreachable!("nothing here lists the accounts")
        }
        fn able_to_send(&self) -> Result<Vec<String>, AccountRepositoryError> {
            unreachable!("nothing here sends")
        }
        fn find(&self, _: &str) -> Result<Option<MailAccount>, AccountRepositoryError> {
            Ok(self.0.clone())
        }
        fn find_by_email(&self, _: &str) -> Result<Option<String>, AccountRepositoryError> {
            Ok(self
                .0
                .as_ref()
                .map(|account| account.summary().id().to_owned()))
        }
        fn save(&self, _: &MailAccount) -> Result<(), AccountRepositoryError> {
            Ok(())
        }
        fn delete(&self, _: &str) -> Result<(), AccountRepositoryError> {
            Ok(())
        }
        fn number_of(&self, _: &str) -> Result<Option<i64>, AccountRepositoryError> {
            Ok(self.0.as_ref().map(|_| 1))
        }
        fn set_collapsed(&self, _: &str, _: bool) -> Result<(), AccountRepositoryError> {
            unreachable!("nothing here folds anything away")
        }
    }

    /// A server that takes whatever password it is offered.
    struct TakesAnyPassword;

    struct OneMailbox;

    impl MailboxDiscovery for OneMailbox {
        fn list_mailboxes(&self) -> Result<Vec<DiscoveredMailbox>, MailProviderError> {
            Ok(vec![DiscoveredMailbox {
                name: "INBOX".to_owned(),
                role: FolderRole::Custom,
                uid_validity: None,
                uid_next: None,
            }])
        }
    }

    impl ProviderFactory for TakesAnyPassword {
        fn discovery(
            &self,
            _: &MailAccount,
            _: &Credential,
        ) -> Result<Box<dyn MailboxDiscovery>, MailProviderError> {
            Ok(Box::new(OneMailbox))
        }
        fn sync(
            &self,
            _: &MailAccount,
            _: &Credential,
        ) -> Result<Box<dyn MailSync>, MailProviderError> {
            unreachable!("nothing here reads mail")
        }
        fn watch(
            &self,
            _: &MailAccount,
            _: &Credential,
        ) -> Result<Box<dyn MailWatch>, MailProviderError> {
            unreachable!("nothing here watches")
        }
        fn reading(
            &self,
            _: &MailAccount,
            _: &Credential,
        ) -> Result<Box<dyn MailFetch>, MailProviderError> {
            unreachable!("nothing here opens a message")
        }
    }

    /// An account whose mail is never on the disk.
    struct NoMail;

    impl MailFiles for NoMail {
        fn remove(&self, _: i64) -> Result<(), MailFilesError> {
            Ok(())
        }
        fn held_by(&self, _: i64) -> Result<u64, MailFilesError> {
            Ok(0)
        }
    }

    fn imap_password() -> CredentialRef {
        CredentialRef::new("acc-1", CredentialPurpose::ImapPassword)
    }

    fn password(typed: &str) -> SecretString {
        SecretString::from(typed.to_owned())
    }

    /// What every pass reads, after the account screen corrected the password and then took
    /// the account off: the new password, and after that nothing — the store is asked again
    /// rather than the held password sent on.
    #[test]
    fn a_password_changed_or_taken_out_here_is_what_every_pass_reads_next() {
        // This device's store, holding the password the account works with now.
        let store = InMemory::default();
        store
            .save(&imap_password(), &password("old"))
            .expect("stored");
        // What the application holds, and what the background rounds read through.
        let held = CachedCredentials::new(store.clone());
        let rounds = held.sharing(store.clone());
        let read = rounds.read(&imap_password()).expect("held from now on");
        assert!(store.keeps(&imap_password(), &read));

        let account = build_account("acc-1".to_owned(), settings()).expect("valid");
        ChangeAccount::new(
            TheProfile(Some(account.clone())),
            passwords(&held, &store),
            TakesAnyPassword,
        )
        .execute("acc-1", account.clone(), password("new"))
        .expect("changed");

        let read = rounds.read(&imap_password()).expect("held");
        assert!(
            store.keeps(&imap_password(), &read),
            "the rounds go on sending the password the account screen replaced"
        );

        RemoveAccount::new(TheProfile(Some(account)), passwords(&held, &store), NoMail)
            .execute("acc-1")
            .expect("removed");

        let asked = store.asked();
        assert_eq!(
            rounds.read(&imap_password()).err(),
            Some(CredentialError::NotFound),
            "the rounds go on sending a password for an account that is gone"
        );
        assert_eq!(store.asked(), asked + 1, "the store was not asked");
    }

    /// A password typed when an account is added is held for every pass from then on, rather
    /// than asked of the store again — which on macOS can be a dialog in front of somebody who
    /// has just typed it.
    #[test]
    fn a_password_added_here_is_held_for_every_pass() {
        let store = InMemory::default();
        let held = CachedCredentials::new(store.clone());
        let rounds = held.sharing(store.clone());

        AddAccount::new(TheProfile(None), passwords(&held, &store), TakesAnyPassword)
            .execute(
                build_account("acc-1".to_owned(), settings()).expect("valid"),
                password("typed"),
            )
            .expect("added");

        let asked = store.asked();
        let read = rounds.read(&imap_password()).expect("held");
        assert!(store.keeps(&imap_password(), &read));
        assert_eq!(
            store.asked(),
            asked,
            "the store was asked for a password typed a moment ago"
        );
    }
}

/// What was found for an address, in the shape the window shows it in.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct FoundSettingsDto {
    /// Nothing when nobody who was asked knows. The window then leaves the fields as they are, and
    /// the person types them, which is what they would have done anyway.
    pub settings: Option<AccountSettingsDto>,
}

/// One spelling of the encryption, so three places do not each write their own.
fn encryption_dto(encryption: Encryption) -> EncryptionDto {
    match encryption {
        Encryption::Tls => EncryptionDto::Tls,
        Encryption::StartTls => EncryptionDto::StartTls,
        Encryption::None => EncryptionDto::None,
    }
}

/// A provider offered by name on the account screen.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct PresetDto {
    pub label: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_encryption: EncryptionDto,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_encryption: EncryptionDto,
    /// A catalogue key the window looks the sentence up under, or nothing.
    pub hint: Option<String>,
}

/// What a person needs to know about their provider before they type a password.
///
/// A catalogue key or nothing, and no network at all: this is a lookup in a table compiled into
/// the application, so it can be asked the moment an address is typed rather than waiting for the
/// button that goes looking for servers. By then the password has been typed, which is exactly
/// what this exists to get in front of — three of the providers here want an app password, and a
/// person who types the other one is told by their own server that their password is wrong.
///
/// The key rather than the sentence, because the core does not write words for a person. The
/// window holds no copy of which providers these are: a second table would be a second table to
/// keep level with the first.
#[tauri::command]
pub async fn account_hint(email: String) -> Result<Option<String>, CommandError> {
    Ok(lyonoyl_domain::provider::hint_for(&email).map(str::to_owned))
}

/// The providers offered by name, and the settings each one fills in.
///
/// Recognition and nothing else: a preset fills in the servers and stops. Nothing branches on
/// which was pressed, which is why this answers with settings rather than with a name the window
/// would have to interpret.
#[tauri::command]
pub async fn account_presets() -> Result<Vec<PresetDto>, CommandError> {
    Ok(lyonoyl_domain::provider::PRESETS
        .iter()
        .filter_map(|preset| {
            let settings = lyonoyl_domain::provider::known_for(preset.domain)?;
            let sending = settings.smtp.as_ref();

            Some(PresetDto {
                label: preset.label.to_owned(),
                imap_host: settings.imap_host.clone(),
                imap_port: settings.imap_port,
                imap_encryption: encryption_dto(settings.imap_encryption),
                smtp_host: sending.map(|s| s.host.clone()).unwrap_or_default(),
                smtp_port: sending.map(|s| s.port).unwrap_or(465),
                smtp_encryption: sending
                    .map(|s| encryption_dto(s.encryption))
                    .unwrap_or(EncryptionDto::Tls),
                hint: preset.hint.map(str::to_owned),
            })
        })
        .collect())
}

/// Looks for the servers behind an address, and stores nothing.
///
/// Asked for by a person pressing a button and never while they type: a search on every keystroke
/// would send half-finished domains to whoever owns them, and the half-finished ones are not even
/// this person's.
///
/// `correcting` names the account the screen is correcting, when it is correcting one: its own
/// servers are no answer about where its mail lives.
#[tauri::command]
pub async fn find_server_settings(
    email: String,
    correcting: Option<String>,
    state: State<'_, AppState>,
) -> Result<FoundSettingsDto, CommandError> {
    // The accounts already here are the first place asked, and a workspace whose storage is broken
    // cannot add the account that would follow anyway.
    let profile = state.storage.profile()?;

    let road =
        std::sync::Arc::new(adapters::transport().map_err(|_| {
            CommandError::new(ErrorCode::Internal, "The trust policy is unavailable.")
        })?);
    let runtime = tauri::async_runtime::handle().inner().clone();

    let found = tauri::async_runtime::spawn_blocking(move || {
        let client = adapters::http_client(road);

        let looked = FindServerSettings::new(
            adapters::accounts(profile),
            adapters::published_by_the_domain(std::sync::Arc::new(client), runtime),
        )
        .execute(&email, correcting.as_deref())
        .map_err(CommandError::from)?;

        Ok::<_, CommandError>(found_settings(&email, looked))
    })
    .await
    .map_err(background_task_failed)??;

    Ok(FoundSettingsDto { settings: found })
}

/// What was found, in the form's own shape.
///
/// The user names are spelled here out of the address being added and the shape the answer
/// carries, and out of nothing else: the answer has no field another account's user name could
/// arrive in. The display name is left for the person, and the password is not part of this at
/// all.
fn found_settings(email: &str, looked: Settings) -> Option<AccountSettingsDto> {
    let Settings::Known {
        servers, sign_in, ..
    } = looked
    else {
        return None;
    };

    let servers = *servers;

    Some(AccountSettingsDto {
        email: email.to_owned(),
        display_name: String::new(),
        imap_host: servers.imap_host,
        imap_port: servers.imap_port,
        imap_encryption: servers.imap_encryption.into(),
        imap_username: sign_in.imap.user_name_for(email),
        submission: servers.smtp.map(|sending| SubmissionDto {
            host: sending.host,
            port: sending.port,
            encryption: sending.encryption.into(),
            username: sign_in.submission.user_name_for(email),
        }),
    })
}
