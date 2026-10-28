use thiserror::Error;

use crate::ports::account_repository::AccountRepositoryError;
use crate::ports::credential_store::CredentialError;
use crate::ports::file_export::FileExportError;
use crate::ports::mail_index::MailIndexError;
use crate::ports::mail_provider::MailProviderError;
use crate::ports::op_log::OperationLogError;
use crate::ports::settings_repository::SettingsError;

/// What a use case can fail with.
///
/// Every variant maps to a distinct action for the user, so the boundary keeps them apart
/// instead of collapsing into a `String`: "offline", "wrong password" and "database corrupted"
/// lead to different screens.
#[derive(Debug, Error)]
pub enum UseCaseError {
    #[error("local storage is unavailable")]
    StorageUnavailable,
    #[error("locally stored data is invalid")]
    StorageCorrupted,
    #[error("the mail provider cannot be reached")]
    Offline,
    #[error("the mail provider rejected the credentials")]
    Unauthorized,
    #[error("the mail provider answered in a way this build cannot use")]
    ProviderProtocol,
    /// The copy running is not one this build can replace in place.
    ///
    /// Its own kind rather than a storage failure, because it is not one: on Linux an application
    /// is a package the system owns, on Windows nothing swaps a running installation, and a copy on
    /// a disk image cannot be written to at all. A person reading a disk error about this would go
    /// looking at their disk.
    #[error("this copy cannot be replaced in place")]
    NotOursToReplace,
    #[error("the operation was cancelled")]
    Cancelled,
    #[error("the provider answered about a folder that was not asked for")]
    UnexpectedFolder,
    #[error("the pass started from a position that is no longer stored")]
    StaleSyncBatch,
    #[error("the credential store is unavailable")]
    CredentialsUnavailable,
    #[error("the stored password is missing")]
    CredentialsMissing,
    #[error("an account with this address is already on this device")]
    AccountAlreadyAdded,
    /// This device has numbered as many accounts as an identifier can carry.
    #[error("this device cannot number another account")]
    NoNumbersLeftForAnAccount,
    #[error("that account is not on this device")]
    UnknownAccount,
    /// Nothing has been read from the account yet, so there is no mailbox to watch. Not a failure
    /// of anything: the first pass has not run.
    #[error("the account's mailboxes have not been read yet")]
    MailboxesNotKnownYet,
    /// There is no system dialog to ask in. A person cannot choose where a file goes, and this
    /// build will not choose for them.
    #[error("there is no way to ask where a file should go")]
    NoPlaceToSave,
    #[error("the file could not be written where it was asked to go")]
    FileNotWritten,
    /// Something else changed the queue while this instruction was being folded into it.
    #[error("the operation log changed while this was being decided")]
    OperationRaced,

    /// A message with nobody to send it to. Not a refusal about its contents: a message with
    /// nothing written in it is a message somebody meant to send.
    #[error("that message has no recipients")]
    MessageHasNoRecipients,
    /// The message is larger than this build will write down. Refused before it is committed: the
    /// outbox is the one database here that is never dropped and never rebuilt.
    #[error("that message is larger than this build will write down")]
    MessageIsTooLarge,
    /// The message says it is from an address this account cannot send as. Refused before it is
    /// written down: a message bearing an address the person does not own is one they appear to
    /// have written, and it sits in their own outbox looking like it.
    #[error("that account cannot send as that address")]
    NotThisAccountsAddress,
    /// The same message written down twice. A `Message-ID` names one message for ever.
    #[error("that message is already waiting to be sent")]
    MessageAlreadyWaiting,
    /// A draft of its own past how many an account keeps, or a keep that would leave an account's
    /// drafts heavier together than they may be. Refused before anything is committed, in the
    /// change that would have written it: the drafts are in the database nothing clears.
    #[error("that account keeps as much in its drafts as this build will write down")]
    NoRoomForDrafts,

    /// A group with no name. Refused rather than stored: a row in the sidebar whose label draws
    /// as nothing cannot be told from the gap above it.
    #[error("a group name is required")]
    GroupNameRequired,
    #[error("that group name is too long")]
    GroupNameTooLong,
    /// Named a group this device does not hold — usually one removed in another window.
    #[error("that group is not on this device")]
    UnknownGroup,
    /// The message has no such file in it. Not a fault: a message can be fetched again between
    /// being listed and being asked about, and what was third a moment ago may be second now.
    #[error("that message has no such file in it")]
    NoSuchPart,
    /// Every place a message is read in was taken. Nothing is known about the message, which is
    /// not the same as knowing it says nothing: asking again when the machine is quieter is a
    /// different question rather than the same one repeated.
    #[error("every place a message is read in is taken")]
    TooBusyToRead,
}

impl From<MailProviderError> for UseCaseError {
    fn from(error: MailProviderError) -> Self {
        match error {
            MailProviderError::Offline => Self::Offline,
            MailProviderError::Unauthorized => Self::Unauthorized,
            MailProviderError::Protocol => Self::ProviderProtocol,
            MailProviderError::Cancelled => Self::Cancelled,
            // A moment's matter, like a network that is not there: the next round asks again.
            MailProviderError::RefusedForNow => Self::Offline,
        }
    }
}

impl From<FileExportError> for UseCaseError {
    fn from(error: FileExportError) -> Self {
        match error {
            FileExportError::CannotAsk => Self::NoPlaceToSave,
            FileExportError::NotWritten => Self::FileNotWritten,
        }
    }
}

impl From<OperationLogError> for UseCaseError {
    fn from(error: OperationLogError) -> Self {
        match error {
            OperationLogError::Unavailable => Self::StorageUnavailable,
            OperationLogError::Corrupted => Self::StorageCorrupted,
            // Read again, fold again. What was folded described a moment that has passed.
            OperationLogError::ChangedUnderneath => Self::OperationRaced,
        }
    }
}

impl From<crate::ports::message_bodies::MessageBodyError> for UseCaseError {
    fn from(_: crate::ports::message_bodies::MessageBodyError) -> Self {
        Self::StorageUnavailable
    }
}

impl From<crate::ports::drafts::DraftError> for UseCaseError {
    fn from(error: crate::ports::drafts::DraftError) -> Self {
        use crate::ports::drafts::DraftError;

        match error {
            DraftError::Unavailable => Self::StorageUnavailable,
            DraftError::NoRoom => Self::NoRoomForDrafts,
        }
    }
}

impl From<crate::ports::outbox::OutboxError> for UseCaseError {
    fn from(error: crate::ports::outbox::OutboxError) -> Self {
        use crate::ports::outbox::OutboxError;

        match error {
            OutboxError::Unavailable => Self::StorageUnavailable,
            OutboxError::Corrupted => Self::StorageCorrupted,
            OutboxError::AlreadyWaiting => Self::MessageAlreadyWaiting,
        }
    }
}

impl From<crate::ports::placement::PlacementError> for UseCaseError {
    fn from(_: crate::ports::placement::PlacementError) -> Self {
        Self::StorageUnavailable
    }
}

impl From<MailIndexError> for UseCaseError {
    fn from(error: MailIndexError) -> Self {
        match error {
            MailIndexError::Unavailable => Self::StorageUnavailable,
            MailIndexError::Corrupted => Self::StorageCorrupted,
            MailIndexError::UnknownFolder => Self::UnexpectedFolder,
        }
    }
}

impl From<SettingsError> for UseCaseError {
    fn from(error: SettingsError) -> Self {
        match error {
            SettingsError::Unavailable => Self::StorageUnavailable,
            SettingsError::Corrupted => Self::StorageCorrupted,
        }
    }
}

impl From<CredentialError> for UseCaseError {
    fn from(error: CredentialError) -> Self {
        match error {
            CredentialError::Unavailable => Self::CredentialsUnavailable,
            CredentialError::NotFound => Self::CredentialsMissing,
        }
    }
}

impl From<AccountRepositoryError> for UseCaseError {
    fn from(error: AccountRepositoryError) -> Self {
        match error {
            AccountRepositoryError::Unavailable => Self::StorageUnavailable,
            AccountRepositoryError::Corrupted => Self::StorageCorrupted,
            AccountRepositoryError::NoNumbersLeft => Self::NoNumbersLeftForAnAccount,
        }
    }
}
