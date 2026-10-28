//! Folders and the roles a provider assigns to them.

/// What a folder is for.
///
/// Providers name folders differently and localise them, so the name is never used to decide
/// behaviour. The adapter maps whatever the server reports onto this list.
///
/// The numbers are stored in `folders.role` and must not be renumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FolderRole {
    /// A folder the user made, or one whose purpose the provider did not state.
    #[default]
    Custom = 0,
    Inbox = 1,
    Sent = 2,
    Drafts = 3,
    Trash = 4,
    Junk = 5,
    Archive = 6,
    /// Every message of the account, as Gmail exposes it.
    AllMail = 7,
    /// Starred, flagged, important — a saved view rather than a place.
    Flagged = 8,
}

impl FolderRole {
    /// An unknown number was written by a newer version and reads as a plain folder.
    pub fn from_stored(value: i64) -> Self {
        match value {
            1 => Self::Inbox,
            2 => Self::Sent,
            3 => Self::Drafts,
            4 => Self::Trash,
            5 => Self::Junk,
            6 => Self::Archive,
            7 => Self::AllMail,
            8 => Self::Flagged,
            _ => Self::Custom,
        }
    }

    pub fn stored(self) -> i64 {
        self as i64
    }

    /// Whether deleting a message from this folder removes it from the account entirely.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Trash | Self::Junk)
    }
}

/// The one mailbox name the protocol defines rather than the person who made it.
///
/// Everything else is matched by what the server declares. Names are never inspected beyond this:
/// providers translate them, and the Russian for "Sent" is not a role.
pub fn is_inbox(name: &str) -> bool {
    name.eq_ignore_ascii_case("INBOX")
}

/// What a mailbox is for, given its name and what the server said about it.
///
/// A declared role always wins. Without one, the inbox is still recognisable — no server marks it
/// with SPECIAL-USE, and a mail client that lists it as an ordinary folder is broken.
pub fn role_of_mailbox(name: &str, declared: FolderRole) -> FolderRole {
    match declared {
        FolderRole::Custom if is_inbox(name) => FolderRole::Inbox,
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_role_survives_storage_as_a_number() {
        for role in [
            FolderRole::Custom,
            FolderRole::Inbox,
            FolderRole::Sent,
            FolderRole::Drafts,
            FolderRole::Trash,
            FolderRole::Junk,
            FolderRole::Archive,
            FolderRole::AllMail,
            FolderRole::Flagged,
        ] {
            assert_eq!(FolderRole::from_stored(role.stored()), role);
        }
    }

    #[test]
    fn role_numbers_are_schema_and_must_not_be_renumbered() {
        assert_eq!(FolderRole::Custom.stored(), 0);
        assert_eq!(FolderRole::Inbox.stored(), 1);
        assert_eq!(FolderRole::Flagged.stored(), 8);
    }

    #[test]
    fn an_unknown_stored_role_reads_as_a_plain_folder() {
        assert_eq!(FolderRole::from_stored(99), FolderRole::Custom);
        assert_eq!(FolderRole::from_stored(-1), FolderRole::Custom);
    }

    #[test]
    fn the_inbox_is_recognised_however_it_is_spelled() {
        assert!(is_inbox("INBOX"));
        assert!(is_inbox("Inbox"));
        assert!(is_inbox("inbox"));
        assert!(
            !is_inbox("Inbox/2025"),
            "a child of the inbox is not the inbox"
        );
        assert!(!is_inbox("Входящие"));
    }

    #[test]
    fn a_mailbox_named_inbox_is_the_inbox_even_though_no_server_says_so() {
        assert_eq!(
            role_of_mailbox("INBOX", FolderRole::Custom),
            FolderRole::Inbox
        );
    }

    #[test]
    fn what_the_server_declares_wins_over_the_name() {
        // A mailbox may be named INBOX and be declared something else; the declaration is the
        // server's own statement about it.
        assert_eq!(
            role_of_mailbox("INBOX", FolderRole::Archive),
            FolderRole::Archive
        );
        assert_eq!(
            role_of_mailbox("Sent Items", FolderRole::Sent),
            FolderRole::Sent
        );
    }

    #[test]
    fn a_folder_the_user_made_keeps_no_role() {
        assert_eq!(
            role_of_mailbox("Projects 2026", FolderRole::Custom),
            FolderRole::Custom
        );
        // Translated names are not inspected: guessing at them is how a client decides that a
        // folder called "Archive" in somebody's own language is the archive and then files their
        // mail into the wrong place.
        assert_eq!(
            role_of_mailbox("Отправленные", FolderRole::Custom),
            FolderRole::Custom
        );
    }
}
