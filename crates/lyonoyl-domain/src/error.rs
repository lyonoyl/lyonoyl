use thiserror::Error;

/// Violations of domain invariants.
///
/// Carries no transport, storage or provider detail.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DomainError {
    #[error("email address must not be empty")]
    EmptyEmail,
    #[error("server address must not be empty")]
    EmptyHost,
    #[error("server address is not a host name")]
    MalformedHost,
    #[error("port must not be zero")]
    InvalidPort,
    #[error("user name must not be empty")]
    EmptyUsername,
    #[error("email address must contain a local part and a domain separated by '@'")]
    MalformedEmail,
    #[error("account identifier must not be empty")]
    EmptyAccountId,
    #[error("stored preference value is not recognised by this version")]
    UnknownPreferenceValue,
    #[error("language tag is not well formed")]
    MalformedLanguageTag,
    #[error("window is smaller than the usable minimum")]
    WindowTooSmall,
    #[error("message identifier must not be zero")]
    InvalidMessageId,
    #[error("folder identifier must not be zero")]
    InvalidFolderId,
    #[error("UID must not be zero")]
    InvalidUid,
    #[error("UIDVALIDITY must not be zero")]
    InvalidUidValidity,
    #[error("Message-ID header must not be empty")]
    EmptyMessageIdHeader,
    #[error("Message-ID header is not a single identifier")]
    MalformedMessageIdHeader,
    #[error("subject is not one line of text")]
    MalformedSubject,
    #[error("a version is three numbers and nothing else")]
    InvalidVersion,
}
