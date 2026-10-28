use serde::Serialize;
use ts_rs::TS;

use crate::application::error::UseCaseError;

/// The error shape that crosses the IPC boundary.
///
/// `code` is a stable machine-readable discriminator; the frontend branches on it and does not
/// parse `message`. No SQL text, message content or secret is carried here.
#[derive(Debug, Clone, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub struct CommandError {
    pub code: ErrorCode,
    /// Short English text, for a developer reading a report. **Never displayed.** The window
    /// turns `code` into a sentence from its own catalog, in the language the person chose; showing
    /// this would put one hard-coded language on screen and hide the translated one.
    pub message: String,
    /// Filesystem path, present only for storage failures so the recovery screen or the sidebar
    /// can show it.
    pub path: Option<String>,
}

// `PartialEq` so a test can say which refusal it expected. This is the discriminator the window
// branches on, and a refusal nobody can compare is a refusal nobody writes a test for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, export_to = "../../src/shared/ipc/generated/")]
pub enum ErrorCode {
    /// The local database cannot be opened or migrated. The UI shows a recovery screen.
    StorageUnavailable,
    /// The key the databases are written under could not be had.
    ///
    /// Apart from the one above, and the difference is what a person is told to do. Nothing is
    /// damaged: the files are whole and the key is where it was — in a system store that would not
    /// hand it over (on macOS, because a rebuilt development binary is a different application to
    /// the keychain), or in a file of secrets that is not open yet, or not chosen yet and holding
    /// nothing. Setting the files aside then repairs nothing and costs the account settings and
    /// everything unsent, so the screen offers it only where the key may be gone for good: a Linux
    /// keyring that does not hand it over, and a file of secrets that is open with no key for these
    /// databases in it.
    StorageKeyUnavailable,
    /// The files were written by a newer build of the application.
    ///
    /// Nothing is wrong with them and nothing in them was changed: that build reads them as they
    /// are, and this one does not migrate downward. So the screen names the file and offers nothing
    /// that moves it — setting it aside would leave the newer build a fresh, empty profile to find.
    StorageFromANewerBuild,
    /// Locally stored data violates an invariant — usually written by another app version.
    StorageCorrupted,
    /// The request itself is malformed — a caller bug, not a user-facing failure.
    InvalidRequest,
    /// A background task failed unexpectedly.
    Internal,
    /// The mail provider could not be reached. Retrying later is the answer.
    Offline,
    /// The mail provider rejected the credentials. The user has to sign in again.
    Unauthorized,
    /// The mail provider answered in a way this build cannot use.
    ProviderProtocol,
    /// The account is being read right now, by a background round or by another request.
    ///
    /// Not a failure and not nothing: what was asked for is happening, and the caller should let
    /// it finish rather than record the account as one it has already dealt with.
    AlreadyReading,
    /// The copy running is not one an update can replace in place.
    NotOursToReplace,
    /// The operation was cancelled, by the user or by shutdown.
    Cancelled,
    /// The store this device keeps its secrets in cannot be reached: locked, absent, refused, or a
    /// file of secrets closed or unwritable.
    CredentialsUnavailable,
    /// The account has no stored password. The user has to enter it again.
    CredentialsMissing,
    /// The address in the account form is not an address.
    InvalidEmail,
    /// The server address in the account form is not a host name.
    InvalidHost,
    /// The port in the account form is not a port.
    InvalidPort,
    /// The user name in the account form is empty.
    InvalidUsername,
    /// An account with this address is already on this device.
    AccountAlreadyAdded,
    NoNumbersLeftForAnAccount,
    /// The master password did not open the secrets file. Only a Linux desktop whose first start
    /// found no credential store has one.
    MasterPasswordRejected,
    /// The folder chosen as a destination is one that means something else — where drafts are
    /// kept, or what has already been sent.
    FolderCannotReceive,
    /// There is no system dialog to ask in, so nobody can say where a file should go.
    NoPlaceToSave,
    /// Somewhere was chosen and the writing failed: a full disk, a read-only volume, a name the
    /// filesystem refused.
    FileNotWritten,
    /// The file chosen is larger than this build attaches.
    FileTooLarge,
    /// The file chosen could not be read: gone, not permitted, or not a file.
    FileNotRead,
}

impl CommandError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            path: None,
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }
}

impl From<UseCaseError> for CommandError {
    fn from(error: UseCaseError) -> Self {
        match error {
            UseCaseError::StorageUnavailable => Self::new(
                ErrorCode::StorageUnavailable,
                "The local database is unavailable.",
            ),
            UseCaseError::StorageCorrupted => Self::new(
                ErrorCode::StorageCorrupted,
                "Locally stored data could not be read.",
            ),
            UseCaseError::Offline => {
                Self::new(ErrorCode::Offline, "The mail server could not be reached.")
            }
            UseCaseError::Unauthorized => Self::new(
                ErrorCode::Unauthorized,
                "The mail server rejected the credentials.",
            ),
            UseCaseError::ProviderProtocol => Self::new(
                ErrorCode::ProviderProtocol,
                "The mail server answered in an unsupported way.",
            ),
            UseCaseError::NotOursToReplace => Self::new(
                ErrorCode::NotOursToReplace,
                "This copy cannot be replaced in place.",
            ),
            UseCaseError::Cancelled => {
                Self::new(ErrorCode::Cancelled, "The operation was cancelled.")
            }
            // Not something a person can do anything about, and not something they should have to
            // see: the use case folds and writes again by itself, and reaching here means it kept
            // losing, which is a defect rather than a state of the mailbox.
            UseCaseError::OperationRaced => Self::new(
                ErrorCode::Internal,
                "The queue of pending actions could not be updated.",
            ),
            UseCaseError::MessageHasNoRecipients => Self::new(
                ErrorCode::InvalidRequest,
                "That message has nobody to go to.",
            ),
            // Pressing send twice. The first press is the one that counts, and saying so is
            // better than a second copy of the message.
            UseCaseError::MessageAlreadyWaiting => Self::new(
                ErrorCode::InvalidRequest,
                "That message is already waiting to be sent.",
            ),
            UseCaseError::UnexpectedFolder => Self::new(
                ErrorCode::Internal,
                "The mail server answered about a different folder.",
            ),
            UseCaseError::CredentialsUnavailable => Self::new(
                ErrorCode::CredentialsUnavailable,
                "The credential store could not be reached.",
            ),
            UseCaseError::CredentialsMissing => Self::new(
                ErrorCode::CredentialsMissing,
                "No password is stored for this account.",
            ),
            UseCaseError::MessageIsTooLarge => Self::new(
                ErrorCode::InvalidRequest,
                "That message is larger than this build will write down.",
            ),
            // Only a window gone wrong reaches it, and it is that window's request that is
            // refused: the window says so the way it says any keep that failed.
            UseCaseError::NoRoomForDrafts => Self::new(
                ErrorCode::InvalidRequest,
                "That account keeps as much in its drafts as this build will write down.",
            ),
            UseCaseError::NotThisAccountsAddress => Self::new(
                ErrorCode::InvalidRequest,
                "That account cannot send as that address.",
            ),
            UseCaseError::UnknownAccount => Self::new(
                ErrorCode::InvalidRequest,
                "That account is not on this device.",
            ),
            // A group removed in another window, most likely. Told apart from the account above
            // because the screen has two different things to say about them.
            // The message was fetched again and its parts moved. Ordinary enough to say plainly
            // and ask them to look again, rather than to report as a fault.
            UseCaseError::NoSuchPart => Self::new(
                ErrorCode::InvalidRequest,
                "That message no longer has that file in it.",
            ),
            // Reached only while other messages take every place one is read in, which is a
            // machine under a load nobody reading their mail makes.
            UseCaseError::TooBusyToRead => Self::new(
                ErrorCode::Internal,
                "Every place a message is read in is taken.",
            ),
            UseCaseError::UnknownGroup => Self::new(
                ErrorCode::InvalidRequest,
                "That group is not on this device.",
            ),
            UseCaseError::GroupNameRequired => {
                Self::new(ErrorCode::InvalidRequest, "A group needs a name.")
            }
            UseCaseError::GroupNameTooLong => {
                Self::new(ErrorCode::InvalidRequest, "That group name is too long.")
            }
            UseCaseError::NoPlaceToSave => Self::new(
                ErrorCode::NoPlaceToSave,
                "There is no way to ask where the file should go.",
            ),
            UseCaseError::FileNotWritten => {
                Self::new(ErrorCode::FileNotWritten, "The file could not be written.")
            }
            UseCaseError::AccountAlreadyAdded => Self::new(
                ErrorCode::AccountAlreadyAdded,
                "This address is already on this device.",
            ),
            UseCaseError::NoNumbersLeftForAnAccount => Self::new(
                ErrorCode::NoNumbersLeftForAnAccount,
                "This device cannot number another account.",
            ),
            UseCaseError::StaleSyncBatch => Self::new(
                ErrorCode::Internal,
                "A synchronization pass started from an outdated position.",
            ),
            // Never reaches a command: only the background watcher can ask about an account
            // before its first pass.
            UseCaseError::MailboxesNotKnownYet => Self::new(
                ErrorCode::Internal,
                "The account's mailboxes have not been read yet.",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keep_refused_for_room_reaches_the_window_as_a_request_the_core_refused() {
        // Only a window gone wrong fills an account's drafts, and the window already has a sentence
        // for a request the core refused: no code of its own, and so no sentence of its own either.
        assert_eq!(
            CommandError::from(UseCaseError::NoRoomForDrafts).code,
            ErrorCode::InvalidRequest
        );
    }
}
