//! A message on its way out: who it is addressed to, and what it may be called.
//!
//! Nothing here sends anything. What lives here is the handful of rules that decide whether a
//! message is well formed and where it goes — the ones worth being sure of before a person presses
//! send, because after that the mistake is in somebody else's mailbox.

use crate::error::DomainError;
use crate::message::Rfc822MessageId;
use crate::thread::{both_ends, NAMES_CONSULTED};

/// The most of a message that is quoted back into an answer.
///
/// A quotation is context, not a copy. A message can be megabytes, and carrying all of it into
/// every answer makes a thread that doubles with each turn — for the person writing, for everyone
/// reading, and on every server it passes through. What is cut is said in the answer rather than
/// left for somebody to discover.
pub const LONGEST_QUOTATION: usize = 16 * 1024;

/// What is written after a quotation that had to be cut.
pub const AND_MORE: &str = "> [...]";

/// One message written back as a quotation of it.
///
/// Every line is prefixed, including the empty ones: a blank line inside a quotation that is not
/// marked reads as the answer starting, and every client on earth draws the rest as though the
/// person wrote it. Lines that are already quoted gain another mark, which is how the depth of a
/// thread stays readable.
///
/// Trailing whitespace is dropped from each line, because a quotation is not a place to preserve
/// somebody's stray spaces and some servers rewrite them anyway.
pub fn quoted(text: &str) -> String {
    let mut written = String::new();

    for line in text.lines() {
        let line = line.trim_end();
        let next = match line.is_empty() {
            true => ">".to_owned(),
            false => format!("> {line}"),
        };

        // Cut at a line rather than in the middle of one: half a sentence with no mark of being
        // cut is a quotation that says something the person never wrote.
        if written.len() + next.len() + 1 > LONGEST_QUOTATION {
            written.push_str(AND_MORE);
            written.push('\n');
            return written;
        }

        written.push_str(&next);
        written.push('\n');
    }

    written
}

/// The longest address this build accepts.
///
/// RFC 5321 puts the limit at 254 characters for a path, and nothing a person types comes close.
/// The bound exists so that a pasted page of text is refused as an address rather than stored as
/// one.
pub const LONGEST_ADDRESS: usize = 254;

/// An address, kept as it was typed and compared the way the world compares them.
///
/// Two rules that pull against each other, and both matter.
///
/// **Kept as typed.** The standard says the part before the `@` belongs to the receiving server,
/// so lowercasing somebody's address because most servers do not care is the sort of helpfulness
/// that loses a reply. The domain is lowercased, because domains are case-insensitive by the
/// standard itself.
///
/// **Compared without case.** Every mail server anybody uses treats the local part that way, so a
/// thread carrying `Kappa@example.net` and `kappa@example.net` carries one person. Comparing them
/// as different text would send that person two copies of a reply, which is a thing they notice
/// and the standard's purity is no comfort about.
#[derive(Debug, Clone, Eq)]
pub struct EmailAddress {
    local: String,
    domain: String,
}

impl PartialEq for EmailAddress {
    fn eq(&self, other: &Self) -> bool {
        self.domain == other.domain && self.local.eq_ignore_ascii_case(&other.local)
    }
}

impl std::hash::Hash for EmailAddress {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Matches `eq`: two addresses that compare equal must hash the same, or a set holding one
        // of them would not find the other.
        self.local.to_ascii_lowercase().hash(state);
        self.domain.hash(state);
    }
}

impl EmailAddress {
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        let value = value.trim();

        if value.is_empty() {
            return Err(DomainError::EmptyEmail);
        }
        if value.len() > LONGEST_ADDRESS {
            return Err(DomainError::MalformedEmail);
        }
        // Nothing that would end a header line or start a second one. A newline in an address is
        // how a header is injected, and an address is not the place to discover that.
        if value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(DomainError::MalformedEmail);
        }

        // The last `@`, because a local part may contain one when it is quoted, and a domain may
        // never.
        let (local, domain) = value.rsplit_once('@').ok_or(DomainError::MalformedEmail)?;

        if local.is_empty() || domain.is_empty() {
            return Err(DomainError::MalformedEmail);
        }
        // A domain with no dot is a host on somebody's local network, not a mail domain. Refusing
        // it here is what stops `user@localhost` reaching a real server as an address.
        if !domain.contains('.') || domain.starts_with('.') || domain.ends_with('.') {
            return Err(DomainError::MalformedEmail);
        }
        if domain.contains("..") {
            return Err(DomainError::MalformedEmail);
        }

        Ok(Self {
            local: local.to_owned(),
            domain: domain.to_ascii_lowercase(),
        })
    }

    /// The part after the `@`, which is what a `Message-ID` is built from.
    pub fn domain(&self) -> &str {
        &self.domain
    }

    pub fn as_str(&self) -> String {
        format!("{}@{}", self.local, self.domain)
    }
}

/// The longest subject this build will write.
///
/// RFC 5322 folds a header line at 998 characters. A subject longer than that is not a subject, and
/// the bound is what stops a caller making a header of any size out of it.
pub const LONGEST_SUBJECT: usize = 998;

/// What a message is called, as one line of a header.
///
/// A subject is free text and almost anything belongs in it — except the two characters that end a
/// header line. `Subject: Hi\r\nBcc: somebody@example.net` is not a subject with an odd character
/// in it; it is a subject and a second header, and the second one is the attacker's. The same rule
/// the domain already applies to an address applies here, and for the same reason.
///
/// Refused rather than repaired. The field a person types into is one line, so a newline cannot be
/// typed or pasted into it — which means a subject carrying one did not come from the person, and
/// quietly rewriting what was asked for would hide that.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Subject(String);

impl Subject {
    pub fn parse(value: &str) -> Result<Self, DomainError> {
        // Trimmed rather than refused when empty: a message with no subject is an ordinary thing
        // to send, and arguing about it would be arguing with the person.
        let value = value.trim();

        if value.len() > LONGEST_SUBJECT {
            return Err(DomainError::MalformedSubject);
        }
        // Every control character, not only CR and LF: a bare CR folds a line on some servers, and
        // NUL truncates the header on others. None of them belong in something a person typed.
        if value.chars().any(char::is_control) {
            return Err(DomainError::MalformedSubject);
        }

        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Prints nothing of what it holds. A subject is what somebody wrote about, and a log is not the
/// place to learn it.
impl std::fmt::Debug for Subject {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Subject({} characters)", self.0.chars().count())
    }
}

/// The name a message is known by, made here rather than by whatever sends it.
///
/// The domain comes from the sender's address and never from the machine. A `Message-ID` built
/// from a hostname tells every recipient, and every server on the way, what somebody's laptop is
/// called — which is a fact about a person that has nothing to do with the message.
///
/// `unique` is supplied rather than generated: this crate has no randomness and wants none. What
/// matters here is the shape and where the domain comes from.
pub fn message_id_for(from: &EmailAddress, unique: &str) -> String {
    format!("<{unique}@{}>", from.domain())
}

/// Who a reply goes to when the reply is to everybody.
///
/// Every address the person has is taken out — not only the one they are writing from. Someone
/// with a work address and an alias, replying to a thread that has both, would otherwise send
/// themselves a copy and, worse, look to everyone else as though they had.
///
/// The order of the original is kept, and a duplicate address appears once: a thread that has gone
/// round a few times collects the same people several times over.
pub fn reply_all_to(original: &[EmailAddress], mine: &[EmailAddress]) -> Vec<EmailAddress> {
    let mut kept: Vec<EmailAddress> = Vec::new();

    for address in original {
        if mine.contains(address) || kept.contains(address) {
            continue;
        }
        kept.push(address.clone());
    }

    kept
}

/// Where a reply belongs in a conversation.
///
/// Two headers, and RFC 5322 §3.6.4 says exactly what goes in them. `In-Reply-To` names the one
/// message being answered. `References` is the whole chain: what the original carried, with the
/// original's own name added at the end.
///
/// This is not decoration. A reply without them is a message every client on the other end files as
/// the start of a new conversation, and a thread of twelve becomes twelve threads — for everybody,
/// not just here.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InThread {
    /// The message being answered, by the name it carries.
    pub answering: Option<Rfc822MessageId>,
    /// The chain it belongs to, oldest first.
    pub chain: Vec<Rfc822MessageId>,
}

impl InThread {
    /// Where a reply to this message belongs.
    ///
    /// `carried` is the `References` of the message being answered — what it said about its own
    /// chain — and `named` is its `Message-ID`. A message with no name of its own cannot be
    /// answered into a thread: there is nothing to point at, and pointing at nothing is worse than
    /// saying nothing.
    /// Refuses rather than repairs a name it would write and cannot read.
    ///
    /// Every identifier here has been through `Rfc822MessageId` once already — it was read off the
    /// message being answered — so in the ordinary case nothing can fail. What this refuses is a
    /// caller that did not come from there, which is the only way a header could be written into
    /// one of these fields.
    ///
    /// The chain holds no more names than a conversation is threaded by: the oldest, which the
    /// conversation is known by, and the newest, which end with the message being answered. What
    /// was carried is somebody else's header and has no length of its own, so only the names that
    /// are kept are read at all.
    pub fn answering(named: Option<&str>, carried: &[String]) -> Result<Self, DomainError> {
        let Some(named) = named.map(str::trim).filter(|name| !name.is_empty()) else {
            return Ok(Self::default());
        };
        let named = Rfc822MessageId::parse(named)?;

        let carried: Vec<&str> = carried
            .iter()
            .map(|name| name.trim())
            .filter(|name| !name.is_empty())
            .collect();

        let mut chain: Vec<Rfc822MessageId> = Vec::new();
        for name in both_ends(&carried, NAMES_CONSULTED) {
            let name = Rfc822MessageId::parse(name)?;
            // A chain that names the same message twice is a chain a client somewhere will read as
            // a loop, and the same name arriving twice is the ordinary way that happens.
            if !chain.contains(&name) {
                chain.push(name);
            }
        }

        if !chain.contains(&named) {
            chain.push(named.clone());
        }
        // The message being answered is the newest name of all, and the room it takes is the
        // room of the name after the oldest.
        if chain.len() > NAMES_CONSULTED {
            chain.remove(1);
        }

        Ok(Self {
            answering: Some(named),
            chain,
        })
    }
}

/// A message someone has written, before anything has been done with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outgoing {
    pub from: EmailAddress,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub subject: Subject,
    pub body: String,
    /// Where it belongs in a conversation, for a message that is an answer to another.
    pub thread: InThread,
    /// The files going with it.
    pub files: Vec<OutgoingFile>,
}

/// A file being sent with a message.
///
/// The bytes are held, not a path. A path would be a thing to read *later*, and later the file may
/// be gone, changed, or on a volume that has been unplugged — and the message would go out with
/// something other than what its writer attached. What is attached is attached at the moment it is
/// chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingFile {
    /// What it is called, made safe when it was chosen.
    pub name: String,
    /// What it will be labelled as. A guess from the name, never asked of the file's contents.
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// How many people one message may be addressed to, across `To`, `Cc` and `Bcc` together.
///
/// Generous by the standards of mail somebody writes, and a bound all the same. Every recipient is
/// a row in the authoritative database — the one this application never re-derives — so the count
/// is the difference between a message and a way to fill a disk that nothing clears.
pub const MOST_RECIPIENTS: usize = 500;

/// How long the text of one message may be.
///
/// Sixteen megabytes of typing. Nobody reaches it and a submission server would refuse long before,
/// which is the point: what this stops is not a person, it is a caller writing into the one database
/// that is nobody's copy of anything.
pub const LONGEST_BODY: usize = 16 * 1024 * 1024;

/// How much one message may weigh in total, before it is encoded.
///
/// Eighteen megabytes, and the number comes from the other end rather than from this one: a file
/// travels base64, which is four bytes for every three, so eighteen becomes twenty-four on the
/// wire — just inside the twenty-five most submission servers accept.
///
/// Refusing here rather than letting the server refuse is the difference between "this is too
/// large to send" said while somebody can still do something about it, and a message that sits in
/// the outbox being retried for ever. The files a draft carries are held to it as well: they are
/// going with a message somebody means to send.
pub const HEAVIEST_MESSAGE: usize = 18 * 1024 * 1024;

/// How many drafts one account keeps on this device.
///
/// Ten thousand. A person keeps the handful they are in the middle of, and somebody who wrote a
/// draft every day and never sent or threw one away would take more than twenty-five years to
/// reach it. What reaches it is a window writing a draft of its own at every keep: each one a row
/// in the database nothing clears, and each listed whole by the drafts folder.
pub const MOST_DRAFTS: usize = 10_000;

/// How much one account's drafts may weigh together on this device: their text and the files
/// they carry, in bytes.
///
/// Two hundred and fifty-six messages as heavy as a message may be, four and a half gigabytes. A
/// person keeps a few drafts with files in them, each until it goes, and nobody's own drafts come
/// near. What it stops is a window gone wrong filling a disk through the database nothing clears:
/// one keeping every new draft with the same photograph in it, or growing every draft it keeps to
/// what one may hold. How many there may be does not bound that on its own, because each draft may
/// hold sixteen megabytes each of body, subject and addresses, and eighteen megabytes of files.
pub const HEAVIEST_DRAFTS: u64 = 256 * HEAVIEST_MESSAGE as u64;

/// What a message of this text and these files weighs before encoding.
///
/// Saturating, because an overflow would wrap round to a small number and pass the check this
/// exists to fail. Not covered by a test, and deliberately: arranging one means allocating the
/// memory it claims, and a test that allocated nothing would be asserting about two zeroes.
pub fn weight_of<'a>(body: &str, files: impl IntoIterator<Item = &'a OutgoingFile>) -> usize {
    files.into_iter().fold(body.len(), |so_far, file| {
        so_far.saturating_add(file.bytes.len())
    })
}

/// Whether these files could go with one message at all: together, and with nothing written beside
/// them, they weigh no more than a message may.
///
/// Files past it are a message nobody can send whatever is taken out of its text, so they are
/// refused as they are chosen and wherever they are kept. Files within it and text that together
/// are too heavy are a message being written, and only sending refuses that.
pub fn fit_in_one_message<'a>(files: impl IntoIterator<Item = &'a OutgoingFile>) -> bool {
    weight_of("", files) <= HEAVIEST_MESSAGE
}

impl Outgoing {
    /// Whether this message is small enough to write down.
    ///
    /// Checked where the message is made rather than where it is stored, so the refusal happens
    /// before anything has been committed. The outbox is the only unbounded writer this application
    /// has into `profile.sqlite3`, and that database is never dropped and never rebuilt.
    pub fn is_within_bounds(&self) -> bool {
        self.body.len() <= LONGEST_BODY
            && self.to.len() + self.cc.len() + self.bcc.len() <= MOST_RECIPIENTS
            && self.weighs() <= HEAVIEST_MESSAGE
            // Each name in it is an identifier, bounded where it is parsed; the count is bounded
            // here, as threading bounds it.
            && self.thread.chain.len() <= NAMES_CONSULTED
    }

    /// What it weighs before encoding: the text, and everything attached to it.
    pub fn weighs(&self) -> usize {
        weight_of(&self.body, &self.files)
    }

    /// Whether this is a message that can be sent at all.
    ///
    /// One rule, and it is about addresses rather than content: a message with nothing to say is a
    /// message somebody meant to send, and refusing it would be arguing with them. A message with
    /// nobody to send it to is not a message.
    pub fn is_addressed(&self) -> bool {
        !self.to.is_empty() || !self.cc.is_empty() || !self.bcc.is_empty()
    }

    /// Everyone who will receive it, each once.
    ///
    /// What a sending adapter needs and what a person is shown before they press send. Blind
    /// copies are in it — they receive the message — and the fact that they are blind is about the
    /// headers, not about who it goes to.
    pub fn everyone(&self) -> Vec<EmailAddress> {
        let mut all: Vec<EmailAddress> = Vec::new();

        for address in self.to.iter().chain(&self.cc).chain(&self.bcc) {
            if !all.contains(address) {
                all.push(address.clone());
            }
        }

        all
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_line_is_marked_including_the_empty_ones() {
        // An unmarked blank line inside a quotation reads as the answer starting, and everything
        // after it is drawn as though this person wrote it.
        assert_eq!(quoted("one\n\ntwo\n"), "> one\n>\n> two\n");
    }

    #[test]
    fn a_quotation_of_a_quotation_goes_one_deeper() {
        assert_eq!(quoted("> theirs\nmine\n"), "> > theirs\n> mine\n");
    }

    #[test]
    fn stray_spaces_at_the_end_of_a_line_are_not_kept() {
        assert_eq!(quoted("said   \n"), "> said\n");
    }

    #[test]
    fn a_message_too_long_to_quote_is_cut_at_a_line_and_says_so() {
        // Cut mid-sentence and unmarked, a quotation says something nobody wrote.
        let long = "abcdefghij ".repeat(4_000);
        let written = quoted(&long);

        assert!(written.len() <= LONGEST_QUOTATION);
        assert!(written.ends_with(&format!("{AND_MORE}\n")));
    }

    #[test]
    fn a_message_that_fits_is_not_marked_as_cut() {
        let written = quoted("short enough\n");

        assert!(!written.contains(AND_MORE));
    }

    #[test]
    fn nothing_written_quotes_as_nothing() {
        assert_eq!(quoted(""), "");
    }

    use super::*;

    fn address(value: &str) -> EmailAddress {
        EmailAddress::parse(value).expect("a well-formed address")
    }

    #[test]
    fn an_ordinary_address_is_kept_as_it_was_typed() {
        let parsed = address("Zeta.Quorin@Example.COM");

        // The domain is compared without case, so it is stored without case. The local part is the
        // receiving server's business, and lowercasing it is how a reply goes nowhere.
        assert_eq!(parsed.as_str(), "Zeta.Quorin@example.com");
        assert_eq!(parsed.domain(), "example.com");
    }

    #[test]
    fn two_spellings_of_one_domain_are_one_person() {
        assert_eq!(address("zeta@example.com"), address("zeta@EXAMPLE.com"));
    }

    #[test]
    fn what_is_certainly_not_an_address_is_refused() {
        for refused in [
            "",
            "   ",
            "zeta",
            "@example.com",
            "zeta@",
            // A host on somebody's own machine, not a mail domain.
            "zeta@localhost",
            "zeta@.example.com",
            "zeta@example.com.",
            "zeta@example..com",
        ] {
            assert!(
                EmailAddress::parse(refused).is_err(),
                "{refused:?} was accepted"
            );
        }
    }

    #[test]
    fn an_address_cannot_carry_a_second_header_into_a_message() {
        // A newline in an address is how a header is injected: `To` ends, and whatever follows
        // becomes a header of its own — a `Bcc` nobody asked for, or a different `From`.
        for injected in [
            "zeta@example.com\r\nBcc: someone@example.net",
            "zeta@example.com\nSubject: elsewhere",
            "zeta @example.com",
            "zeta@exa\tmple.com",
        ] {
            assert!(
                EmailAddress::parse(injected).is_err(),
                "{injected:?} was accepted"
            );
        }
    }

    #[test]
    fn space_around_an_address_is_taken_off_rather_than_refused() {
        // Pasted addresses arrive with a newline or a space on the end. Taking it off leaves a
        // clean address; what is refused is a break *inside* one, because that is a second header.
        let parsed = address(" zeta@example.com\r\n");

        assert_eq!(parsed.as_str(), "zeta@example.com");
        assert!(!parsed.as_str().chars().any(char::is_control));
    }

    #[test]
    fn a_pasted_page_is_not_an_address() {
        let long = format!("{}@example.com", "a".repeat(LONGEST_ADDRESS));

        assert!(EmailAddress::parse(&long).is_err());
    }

    #[test]
    fn the_last_at_sign_separates_the_address() {
        // A quoted local part may hold one; a domain may not.
        let parsed = address("\"odd@name\"@example.com");

        assert_eq!(parsed.domain(), "example.com");
    }

    #[test]
    fn a_message_is_named_after_the_sender_rather_than_the_machine() {
        // A `Message-ID` built from a hostname tells every recipient, and every server on the way,
        // what somebody's laptop is called.
        let named = message_id_for(&address("zeta@example.com"), "abc123");

        assert_eq!(named, "<abc123@example.com>");
        assert!(named.starts_with('<') && named.ends_with('>'));
    }

    #[test]
    fn replying_to_everybody_leaves_out_every_address_of_mine() {
        // Not only the one being written from. Somebody with a work address and an alias would
        // otherwise send themselves a copy — and look to everyone else as though they had.
        let original = [
            address("kappa@example.net"),
            address("zeta@example.com"),
            address("zeta+lists@example.com"),
            address("sigma@example.org"),
        ];
        let mine = [
            address("zeta@example.com"),
            address("zeta+lists@example.com"),
        ];

        let replying: Vec<String> = reply_all_to(&original, &mine)
            .iter()
            .map(EmailAddress::as_str)
            .collect();

        assert_eq!(replying, vec!["kappa@example.net", "sigma@example.org"]);
    }

    #[test]
    fn a_thread_that_has_gone_round_does_not_collect_the_same_person_twice() {
        let original = [
            address("kappa@example.net"),
            address("KAPPA@Example.NET"),
            address("sigma@example.org"),
        ];

        assert_eq!(reply_all_to(&original, &[]).len(), 2);
    }

    #[test]
    fn replying_to_everybody_keeps_the_order_the_message_had() {
        let original = [address("sigma@example.org"), address("kappa@example.net")];

        let order: Vec<String> = reply_all_to(&original, &[])
            .iter()
            .map(EmailAddress::as_str)
            .collect();

        assert_eq!(order, vec!["sigma@example.org", "kappa@example.net"]);
    }

    fn written(to: Vec<EmailAddress>, cc: Vec<EmailAddress>, bcc: Vec<EmailAddress>) -> Outgoing {
        Outgoing {
            files: Vec::new(),
            from: address("zeta@example.com"),
            to,
            cc,
            bcc,
            subject: Subject::parse("").expect("empty is a subject"),
            body: String::new(),
            thread: Default::default(),
        }
    }

    /// A message with one file of the given size attached.
    fn carrying(bytes: usize) -> Outgoing {
        Outgoing {
            files: vec![OutgoingFile {
                name: "big.bin".to_owned(),
                content_type: "application/octet-stream".to_owned(),
                bytes: vec![0; bytes],
            }],
            ..written(vec![address("kappa@example.net")], Vec::new(), Vec::new())
        }
    }

    #[test]
    fn what_a_message_weighs_is_its_text_and_everything_attached() {
        // The body alone was the whole of the old rule, and a body is the smaller half of any
        // message somebody attaches a photograph to.
        let mut message = carrying(1_000);
        message.body = "x".repeat(500);

        assert_eq!(message.weighs(), 1_500);
    }

    #[test]
    fn a_message_light_enough_to_send_is_accepted() {
        // The ordinary case, and the one a change to the limit would break silently.
        assert!(carrying(HEAVIEST_MESSAGE).is_within_bounds());
    }

    #[test]
    fn one_byte_past_the_limit_is_refused() {
        // Refused here rather than by the server, which is the difference between being told while
        // something can still be done about it and a message retried in the outbox for ever.
        assert!(!carrying(HEAVIEST_MESSAGE + 1).is_within_bounds());
    }

    #[test]
    fn several_files_that_are_small_apart_and_large_together_are_refused() {
        // Checked one at a time, three files of ten megabytes each pass three times and arrive as
        // thirty. The limit is about the message, not about a file.
        let mut message = written(vec![address("kappa@example.net")], Vec::new(), Vec::new());
        message.files = (0..3)
            .map(|at| OutgoingFile {
                name: format!("part-{at}.bin"),
                content_type: "application/octet-stream".to_owned(),
                bytes: vec![0; 8 * 1024 * 1024],
            })
            .collect();

        assert!(!message.is_within_bounds());
    }

    #[test]
    fn files_fit_in_one_message_until_together_they_weigh_more_than_one_may() {
        // Together, as a message carries them: two files that each fit are refused past it.
        let file = |bytes: usize| OutgoingFile {
            name: "part.bin".to_owned(),
            content_type: "application/octet-stream".to_owned(),
            bytes: vec![0; bytes],
        };
        let half = HEAVIEST_MESSAGE / 2;
        let rest = HEAVIEST_MESSAGE - half;

        assert!(fit_in_one_message(&[file(half), file(rest)]));
        assert!(!fit_in_one_message(&[file(half), file(rest + 1)]));
        assert!(fit_in_one_message(&[]), "no file at all is a message too");
    }

    #[test]
    fn a_message_with_nobody_to_send_it_to_is_not_a_message() {
        assert!(!written(Vec::new(), Vec::new(), Vec::new()).is_addressed());
    }

    #[test]
    fn a_blind_copy_alone_is_still_somewhere_to_send_it() {
        assert!(written(Vec::new(), Vec::new(), vec![address("kappa@example.net")]).is_addressed());
    }

    #[test]
    fn an_empty_message_to_somebody_is_a_message() {
        // Refusing it would be arguing with a person about what they meant to send.
        assert!(written(vec![address("kappa@example.net")], Vec::new(), Vec::new()).is_addressed());
    }

    #[test]
    fn everyone_it_goes_to_is_counted_once_across_the_three_lines() {
        let message = written(
            vec![address("kappa@example.net")],
            vec![address("kappa@example.net"), address("sigma@example.org")],
            vec![address("records@example.com")],
        );

        let all: Vec<String> = message
            .everyone()
            .iter()
            .map(EmailAddress::as_str)
            .collect();

        assert_eq!(
            all,
            vec![
                "kappa@example.net",
                "sigma@example.org",
                "records@example.com"
            ]
        );
    }
}

#[cfg(test)]
mod where_a_reply_belongs {
    use super::*;

    /// The thread a reply belongs to, for a call that cannot fail.
    ///
    /// Every identifier in these tests is one the index would have produced, and the index produces
    /// them by parsing. What the parse refuses has its own tests below.
    fn thread(named: Option<&str>, carried: &[&str]) -> InThread {
        let carried: Vec<String> = carried.iter().map(|name| (*name).to_owned()).collect();
        InThread::answering(named, &carried).expect("these are identifiers")
    }

    /// The chain as plain names, which is what the assertions are about.
    fn names(thread: &InThread) -> Vec<&str> {
        thread.chain.iter().map(Rfc822MessageId::as_str).collect()
    }

    #[test]
    fn a_reply_names_the_message_it_answers() {
        // Stored without the angle brackets: they are punctuation belonging to the header, and the
        // writer puts them back. A name kept with them would be written as `<<one@example.com>>`.
        let thread = thread(Some("<one@example.com>"), &[]);

        assert_eq!(
            thread.answering.as_ref().map(Rfc822MessageId::as_str),
            Some("one@example.com")
        );
        assert_eq!(names(&thread), vec!["one@example.com"]);
    }

    #[test]
    fn the_chain_is_what_came_before_plus_the_message_being_answered() {
        // RFC 5322 §3.6.4. Twelve messages in and the chain is still one line, oldest first.
        let thread = thread(
            Some("<three@example.com>"),
            &["<one@example.com>", "<two@example.com>"],
        );

        assert_eq!(
            names(&thread),
            vec!["one@example.com", "two@example.com", "three@example.com"]
        );
    }

    #[test]
    fn a_name_that_is_already_in_the_chain_is_not_added_twice() {
        // A chain naming the same message twice is one a client somewhere reads as a loop, and the
        // same name arriving twice is the ordinary way that happens.
        let thread = thread(
            Some("<two@example.com>"),
            &["<one@example.com>", "<two@example.com>"],
        );

        assert_eq!(names(&thread), vec!["one@example.com", "two@example.com"]);
    }

    /// An ordinary message to somebody, for a thread to be carried by.
    fn a_message() -> Outgoing {
        Outgoing {
            files: Vec::new(),
            from: EmailAddress::parse("zeta@example.com").expect("an address"),
            to: vec![EmailAddress::parse("kappa@example.net").expect("an address")],
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: Subject::parse("").expect("empty is a subject"),
            body: String::new(),
            thread: InThread::default(),
        }
    }

    /// A chain of `count` names, oldest first, as a message's `References` would carry it.
    fn carried(count: usize) -> Vec<String> {
        (0..count).map(|at| format!("<{at}@example.com>")).collect()
    }

    #[test]
    fn a_chain_longer_than_a_conversation_is_threaded_by_keeps_both_ends() {
        // The answered message's `References` is somebody else's header, and nothing limits it.
        // Copied whole, a thousand names go into the reply, into the outbox row and onto the
        // wire, every one compared with every other on the way.
        let long = InThread::answering(Some("<answered@example.com>"), &carried(1_000))
            .expect("these are identifiers");

        let kept = names(&long);
        assert_eq!(kept.len(), NAMES_CONSULTED);
        assert_eq!(kept[0], "0@example.com", "the name it is known by went");
        assert_eq!(
            kept[NAMES_CONSULTED - 1],
            "answered@example.com",
            "the message being answered is not last"
        );
        assert_eq!(kept[NAMES_CONSULTED - 2], "999@example.com");
        assert_eq!(
            kept[1],
            format!("{}@example.com", 1_000 - (NAMES_CONSULTED - 2)),
            "the names kept are not the newest"
        );
        assert!(Outgoing {
            thread: long,
            ..a_message()
        }
        .is_within_bounds());
    }

    #[test]
    fn a_chain_that_fits_is_kept_whole_and_one_more_costs_the_name_after_the_oldest() {
        let fits = InThread::answering(
            Some("<answered@example.com>"),
            &carried(NAMES_CONSULTED - 1),
        )
        .expect("these are identifiers");
        assert_eq!(fits.chain.len(), NAMES_CONSULTED);
        assert_eq!(
            names(&fits)[1],
            "1@example.com",
            "a chain that fits was cut"
        );

        let over = InThread::answering(Some("<answered@example.com>"), &carried(NAMES_CONSULTED))
            .expect("these are identifiers");
        let kept = names(&over);
        assert_eq!(kept.len(), NAMES_CONSULTED);
        assert_eq!(kept[0], "0@example.com");
        assert_eq!(kept[1], "2@example.com");
        assert_eq!(kept[NAMES_CONSULTED - 1], "answered@example.com");
    }

    #[test]
    fn a_message_whose_chain_is_longer_than_a_conversation_is_threaded_by_is_not_written_down() {
        // Built by hand rather than through `answering`, which never makes one: the bound is on
        // the message, wherever its thread came from.
        let chain = |count: usize| -> Vec<Rfc822MessageId> {
            (0..count)
                .map(|at| Rfc822MessageId::parse(&format!("<{at}@example.com>")).expect("a name"))
                .collect()
        };
        let carrying = |count: usize| Outgoing {
            thread: InThread {
                answering: None,
                chain: chain(count),
            },
            ..a_message()
        };

        assert!(carrying(NAMES_CONSULTED).is_within_bounds());
        assert!(!carrying(NAMES_CONSULTED + 1).is_within_bounds());
    }

    #[test]
    fn a_message_with_no_name_of_its_own_cannot_be_answered_into_a_thread() {
        // There is nothing to point at, and pointing at nothing is worse than saying nothing: a
        // header naming an empty message is one every client has to guess about.
        assert_eq!(thread(None, &["<one@example.com>"]), InThread::default());
        assert_eq!(thread(Some("   "), &[]), InThread::default());
    }

    #[test]
    fn a_message_nobody_is_answering_belongs_to_no_thread() {
        assert_eq!(InThread::default().answering, None);
        assert!(InThread::default().chain.is_empty());
    }

    #[test]
    fn a_name_carrying_a_second_header_is_refused_rather_than_written() {
        // The whole reason these are parsed rather than copied. This value is what an arriving
        // message can carry in its own `Message-ID`, and answering it would put the sender's
        // `Bcc:` on the reply — no compromised window required, only somebody answering their mail.
        let injected = "one@example.com>\r\nBcc: attacker@example.net\r\nX-Junk: <two";

        assert!(InThread::answering(Some(injected), &[]).is_err());
        assert!(InThread::answering(Some("one@example.com"), &[injected.to_owned()]).is_err());
    }
}

#[cfg(test)]
mod what_a_message_is_called {
    use super::*;

    #[test]
    fn an_ordinary_subject_is_kept_as_it_was_written() {
        let subject = Subject::parse("  About Tuesday  ").expect("a subject");
        assert_eq!(subject.as_str(), "About Tuesday");
    }

    #[test]
    fn a_message_may_have_no_subject_at_all() {
        // Somebody meant to send it. Arguing about it would be arguing with them.
        assert_eq!(Subject::parse("").expect("a subject").as_str(), "");
    }

    #[test]
    fn a_subject_that_would_start_a_second_header_is_refused() {
        // `Subject: Hi` and then `Bcc: somebody` is not a subject with an odd character in it. The
        // field a person types into is one line, so this did not come from them.
        assert!(Subject::parse("Hi\r\nBcc: attacker@example.net").is_err());
        assert!(Subject::parse("Hi\nBcc: attacker@example.net").is_err());
        assert!(Subject::parse("Hi\rBcc: attacker@example.net").is_err());
        // Not only CR and LF: NUL truncates the header on some servers.
        assert!(Subject::parse("Hi\0Bcc: attacker@example.net").is_err());
    }

    #[test]
    fn a_subject_too_long_to_be_one_line_is_refused() {
        assert!(Subject::parse(&"a".repeat(998)).is_ok());
        assert!(Subject::parse(&"a".repeat(999)).is_err());
    }

    #[test]
    fn what_it_holds_stays_out_of_a_log() {
        // A subject is what somebody wrote about.
        let subject = Subject::parse("Divorce papers").expect("a subject");
        assert_eq!(format!("{subject:?}"), "Subject(14 characters)");
    }
}
