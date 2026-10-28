//! Message identity and flags.
//!
//! A message carries three separate identifiers: a local surrogate, a protocol address and the
//! `Message-ID` header. They are not interchangeable.

use std::fmt;

use crate::error::DomainError;

/// Local surrogate identifier: `INTEGER PRIMARY KEY` in `messages`.
///
/// The only identifier used by the interface, the router, the thread graph, the body cache and
/// `op_log`. Unaffected by `UIDVALIDITY` changes and by moves between folders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MessageId(u64);

/// Local surrogate identifier of a mailbox.
///
/// Folder paths are renamed by the user, re-cased by the server and re-encoded by the protocol,
/// so they are not used as keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FolderId(u64);

/// IMAP `UID`: unique within one folder and one `UIDVALIDITY` generation.
///
/// Not a primary key, not stored in `op_log`, not shown in the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uid(u32);

/// Generation marker of a folder's UID space. A change invalidates every `(folder_id, uid)` pair
/// of that folder; the messages remain, their addresses do not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UidValidity(u32);

/// CONDSTORE modification sequence. Monotonic within a folder and only compared, never parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModSeq(u64);

/// Protocol address of a message.
///
/// Valid only with the folder's current [`UidValidity`], and stored in `message_placements`
/// rather than in `messages`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Placement {
    pub folder_id: FolderId,
    pub uid: Uid,
}

macro_rules! nonzero_id {
    ($name:ident, $inner:ty, $error:expr, $what:literal) => {
        impl $name {
            #[doc = concat!("Rejects zero: ", $what)]
            pub fn new(value: $inner) -> Result<Self, DomainError> {
                if value == 0 {
                    return Err($error);
                }
                Ok(Self(value))
            }

            pub fn get(self) -> $inner {
                self.0
            }
        }
    };
}

nonzero_id!(
    MessageId,
    u64,
    DomainError::InvalidMessageId,
    "SQLite row ids start at 1."
);
nonzero_id!(
    FolderId,
    u64,
    DomainError::InvalidFolderId,
    "SQLite row ids start at 1."
);
nonzero_id!(Uid, u32, DomainError::InvalidUid, "RFC 3501 forbids UID 0.");
nonzero_id!(
    UidValidity,
    u32,
    DomainError::InvalidUidValidity,
    "RFC 3501 requires UIDVALIDITY to be a non-zero 32-bit value."
);

impl ModSeq {
    /// Not validated: `HIGHESTMODSEQ 0` means the server keeps no modification sequences.
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl Placement {
    pub fn new(folder_id: FolderId, uid: Uid) -> Self {
        Self { folder_id, uid }
    }
}

/// The `Message-ID` header.
///
/// Used as a hint for deduplication and threading. Not unique across messages and not a key.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rfc822MessageId(String);

/// The longest identifier this build accepts.
///
/// RFC 5322 folds a header line at 998 characters. Nothing legitimate comes close; the bound exists
/// so that a field nobody reads cannot be used to write a header of any length.
pub const LONGEST_MESSAGE_ID: usize = 998;

impl Rfc822MessageId {
    /// Accepts the header value with or without angle brackets and stores it without them.
    ///
    /// Case is preserved: RFC 5322 defines `msg-id` as case-sensitive.
    ///
    /// **One identifier, and nothing that could end a header line.** This value travels in both
    /// directions: it is read off an arriving message, and it is written back out as `In-Reply-To`
    /// and `References` when somebody answers that message. So a `Message-ID` carrying a newline is
    /// not a curiosity in somebody else's mail — it is a header the person's own reply would carry,
    /// with whatever the sender put after the newline. Refusing it here closes both directions at
    /// once, and a message whose identifier cannot be read is simply stored without one.
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        let trimmed = value.trim();
        let unwrapped = trimmed
            .strip_prefix('<')
            .and_then(|rest| rest.strip_suffix('>'))
            .unwrap_or(trimmed)
            .trim();

        if unwrapped.is_empty() {
            return Err(DomainError::EmptyMessageIdHeader);
        }
        // RFC 5322 puts a header line at 998 characters before folding, and an identifier that
        // long is not an identifier. The bound is here so a caller cannot make a header of any
        // size out of a field nobody looks at.
        if unwrapped.len() > LONGEST_MESSAGE_ID {
            return Err(DomainError::MalformedMessageIdHeader);
        }
        // No control character, no whitespace — a newline is how a second header is written — and
        // no angle bracket, because one identifier cannot contain the punctuation that separates
        // it from the next.
        if unwrapped.chars().any(|character| {
            character.is_control()
                || character.is_whitespace()
                || character == '<'
                || character == '>'
        }) {
            return Err(DomainError::MalformedMessageIdHeader);
        }

        Ok(Self(unwrapped.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Prints the domain part only. The header identifies a specific message and is treated as
/// personal data in logs.
impl fmt::Debug for Rfc822MessageId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.split_once('@') {
            Some((_, domain)) => write!(formatter, "Rfc822MessageId(\"***@{domain}\")"),
            None => formatter.write_str("Rfc822MessageId(\"[redacted]\")"),
        }
    }
}

/// System flags packed into one integer, stored in `messages.flags`.
///
/// The bit values are part of the schema: they are written to the database and to `op_log`
/// payloads. New flags take new bits; existing bits are never renumbered.
///
/// `\Recent` is not stored: it is session-scoped and the server clears it independently.
#[derive(Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Flags(u32);

impl Flags {
    pub const NONE: Self = Self(0);
    pub const SEEN: Self = Self(1 << 0);
    pub const ANSWERED: Self = Self(1 << 1);
    pub const FLAGGED: Self = Self(1 << 2);
    pub const DRAFT: Self = Self(1 << 3);
    pub const DELETED: Self = Self(1 << 4);

    /// Keeps bits this build does not know, so flags written by a newer version survive.
    pub fn from_bits(bits: u32) -> Self {
        Self(bits)
    }

    pub fn bits(self) -> u32 {
        self.0
    }

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// A message without `\Seen` is unread.
    pub fn is_unread(self) -> bool {
        !self.contains(Self::SEEN)
    }
}

/// Prints flag names in full: flags carry no message content.
impl fmt::Debug for Flags {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let named = [
            (Self::SEEN, "Seen"),
            (Self::ANSWERED, "Answered"),
            (Self::FLAGGED, "Flagged"),
            (Self::DRAFT, "Draft"),
            (Self::DELETED, "Deleted"),
        ];

        let mut listed = named
            .iter()
            .filter(|(flag, _)| self.contains(*flag))
            .map(|(_, name)| *name)
            .collect::<Vec<_>>();

        let known: u32 = named.iter().map(|(flag, _)| flag.0).sum();
        let unknown = self.0 & !known;
        let unknown_label;
        if unknown != 0 {
            unknown_label = format!("unknown({unknown:#x})");
            listed.push(&unknown_label);
        }

        write!(formatter, "Flags({})", listed.join("|"))
    }
}

/// How much of a message is stored locally.
///
/// A mailbox is synchronised as envelopes first and bodies later, so the interface has to be able
/// to say what it has rather than show an empty message.
///
/// The numbers are stored in `messages.body_state` and must not be renumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BodyState {
    /// Envelope only: subject, sender, date, size.
    #[default]
    HeadersOnly = 0,
    /// A short text preview is stored.
    PreviewCached = 1,
    /// The text part is stored in full.
    TextCached = 2,
    /// Every part is stored, attachments included.
    FullCached = 3,
}

impl BodyState {
    /// An unknown number was written by a newer version and reads as "nothing but the envelope".
    pub fn from_stored(value: i64) -> Self {
        match value {
            1 => Self::PreviewCached,
            2 => Self::TextCached,
            3 => Self::FullCached,
            _ => Self::HeadersOnly,
        }
    }

    pub fn stored(self) -> i64 {
        self as i64
    }

    /// Whether the message can be read without going to the server.
    pub fn has_text(self) -> bool {
        matches!(self, Self::TextCached | Self::FullCached)
    }
}

/// Where a provider keeps flags. Declared by the adapter.
///
/// IMAP keeps flags per mailbox; Gmail keeps one set per message across all of its labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagScope {
    /// One canonical value per message. `flags_override` stays `NULL`.
    Message,
    /// Per-placement values; `flags_override` carries them.
    Placement,
}

/// Flags to display for one placement: the override when present, otherwise the canonical value.
/// Mirrors `COALESCE(p.flags_override, m.flags)`.
pub fn effective_flags(canonical: Flags, override_for_placement: Option<Flags>) -> Flags {
    override_for_placement.unwrap_or(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_reject_zero_because_it_means_uninitialised() {
        assert_eq!(MessageId::new(0), Err(DomainError::InvalidMessageId));
        assert_eq!(FolderId::new(0), Err(DomainError::InvalidFolderId));
        assert_eq!(Uid::new(0), Err(DomainError::InvalidUid));
        assert_eq!(UidValidity::new(0), Err(DomainError::InvalidUidValidity));
    }

    #[test]
    fn identifiers_round_trip_their_value() {
        assert_eq!(MessageId::new(42).expect("valid").get(), 42);
        assert_eq!(Uid::new(u32::MAX).expect("valid").get(), u32::MAX);
        assert_eq!(
            ModSeq::new(0).get(),
            0,
            "a server may keep no modseq at all"
        );
    }

    #[test]
    fn a_message_id_header_is_stored_without_its_brackets() {
        let parsed = Rfc822MessageId::parse("  <abc@example.com>  ").expect("well formed");
        assert_eq!(parsed.as_str(), "abc@example.com");
    }

    #[test]
    fn a_message_id_header_keeps_its_case() {
        // RFC 5322 defines msg-id as case-sensitive.
        let upper = Rfc822MessageId::parse("<ABC@example.com>").expect("well formed");
        let lower = Rfc822MessageId::parse("<abc@example.com>").expect("well formed");
        assert_ne!(upper, lower);
    }

    #[test]
    fn an_empty_message_id_header_is_rejected() {
        for candidate in ["", "   ", "<>", "< >"] {
            assert_eq!(
                Rfc822MessageId::parse(candidate),
                Err(DomainError::EmptyMessageIdHeader),
                "input: {candidate:?}"
            );
        }
    }

    #[test]
    fn a_message_id_never_prints_its_local_part() {
        let parsed = Rfc822MessageId::parse("<secret-thread-42@example.com>").expect("well formed");
        let printed = format!("{parsed:?}");
        assert!(!printed.contains("secret-thread-42"), "printed: {printed}");
        assert!(
            printed.contains("example.com"),
            "the domain stays diagnostic"
        );
    }

    #[test]
    fn flags_combine_and_separate() {
        let flags = Flags::NONE.with(Flags::SEEN).with(Flags::FLAGGED);
        assert!(flags.contains(Flags::SEEN));
        assert!(flags.contains(Flags::FLAGGED));
        assert!(!flags.contains(Flags::DRAFT));

        let cleared = flags.without(Flags::SEEN);
        assert!(!cleared.contains(Flags::SEEN));
        assert!(cleared.contains(Flags::FLAGGED), "unrelated flags survive");
    }

    #[test]
    fn an_empty_flag_set_means_unread() {
        assert!(Flags::default().is_empty());
        assert!(Flags::default().is_unread());
        assert!(!Flags::SEEN.is_unread());
    }

    #[test]
    fn a_flag_written_by_a_newer_build_survives_a_round_trip() {
        // A flag written by a newer build survives a read-modify-write cycle.
        let from_the_future = Flags::from_bits(Flags::SEEN.bits() | 1 << 20);
        assert_eq!(from_the_future.bits(), Flags::SEEN.bits() | 1 << 20);
        assert!(from_the_future.contains(Flags::SEEN));

        let toggled = from_the_future.without(Flags::SEEN).with(Flags::SEEN);
        assert_eq!(
            toggled, from_the_future,
            "an unknown bit is not lost by editing"
        );
    }

    #[test]
    fn flag_bits_are_schema_and_must_not_be_renumbered() {
        // These numbers are written to the database; changing one reinterprets stored flags.
        assert_eq!(Flags::SEEN.bits(), 1);
        assert_eq!(Flags::ANSWERED.bits(), 2);
        assert_eq!(Flags::FLAGGED.bits(), 4);
        assert_eq!(Flags::DRAFT.bits(), 8);
        assert_eq!(Flags::DELETED.bits(), 16);
    }

    #[test]
    fn debug_lists_known_flags_and_reports_unknown_bits() {
        let printed = format!("{:?}", Flags::SEEN.with(Flags::from_bits(1 << 20)));
        assert!(printed.contains("Seen"), "printed: {printed}");
        assert!(printed.contains("unknown"), "printed: {printed}");
    }

    #[test]
    fn a_body_state_survives_storage_as_a_number() {
        for state in [
            BodyState::HeadersOnly,
            BodyState::PreviewCached,
            BodyState::TextCached,
            BodyState::FullCached,
        ] {
            assert_eq!(BodyState::from_stored(state.stored()), state);
        }
    }

    #[test]
    fn body_state_numbers_are_schema_and_must_not_be_renumbered() {
        assert_eq!(BodyState::HeadersOnly.stored(), 0);
        assert_eq!(BodyState::FullCached.stored(), 3);
    }

    #[test]
    fn an_unknown_body_state_reads_as_the_envelope_only() {
        assert_eq!(BodyState::from_stored(99), BodyState::HeadersOnly);
        assert!(!BodyState::from_stored(99).has_text());
    }

    #[test]
    fn only_a_stored_text_counts_as_readable() {
        assert!(!BodyState::HeadersOnly.has_text());
        assert!(!BodyState::PreviewCached.has_text());
        assert!(BodyState::TextCached.has_text());
        assert!(BodyState::FullCached.has_text());
    }

    #[test]
    fn an_override_wins_over_the_canonical_value_for_that_placement() {
        let canonical = Flags::SEEN;
        assert_eq!(effective_flags(canonical, None), Flags::SEEN);
        assert_eq!(effective_flags(canonical, Some(Flags::NONE)), Flags::NONE);
    }
}
