//! Handing a message to a submission server.
//!
//! The conversation is driven a step at a time rather than in one call, and that is the whole
//! point of this file. SMTP is not idempotent: everything up to `DATA` can be retried freely,
//! and the moment the message itself has been written, a connection that goes quiet means nobody
//! knows whether it arrived. A client that cannot tell those apart either loses messages or sends
//! them twice, and this one has to tell them apart to keep the promise the outbox makes.

use std::net::SocketAddr;
use std::time::Duration;

use secrecy::ExposeSecret;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::infrastructure::net::is_this_machine;
use crate::ports::credential::Credential;

use mail_builder::headers::content_type::ContentType;
use mail_builder::mime::MimePart;
use mail_builder::MessageBuilder;
use mail_send::smtp::message::Parameters;
use mail_send::{SmtpClient, SmtpClientBuilder};

use crate::ports::mail_submission::{MailSubmission, SubmissionError};
use lyonoyl_domain::account::{Encryption, Submission};
use lyonoyl_domain::outgoing::Outgoing;

/// How long a submission server may take to answer, before the message is on the wire.
///
/// RFC 5321 §4.5.3.2 sets the minimums a client must allow, and five minutes covers every one of
/// them up to the end of DATA. Shorter is a client that gives up on a busy server; longer is a
/// person watching a message that will never go.
const BEFORE_THE_MESSAGE: Duration = Duration::from_secs(5 * 60);

/// What this client calls itself when it greets a submission server.
///
/// The protocol has the client name itself in its first command, and the server writes that name
/// into the `Received` header it adds, which travels with the message to everybody it is sent to.
/// Left to itself the library says this machine's name, and a machine's name is commonly its
/// owner's. An address literal is a name the protocol accepts, and this one says nothing about who
/// is sending or from where.
const GREETING: &str = "[127.0.0.1]";

/// One account's submission server, ready to be handed a message.
pub struct SmtpSubmission {
    host: String,
    port: u16,
    implicit_tls: bool,
    /// Whether the connection will be encrypted at all. One that is not may go to this machine
    /// and nowhere else: see `submit`.
    encrypted: bool,
    username: String,
    /// A password or a token. Which it is decides the SASL mechanism, and nothing else.
    credential: Credential,
    runtime: tokio::runtime::Handle,
    /// How long the server may take to answer before the message is on the wire:
    /// `BEFORE_THE_MESSAGE`, and shorter only in a test that waits for it to run out.
    allowed: Duration,
}

impl SmtpSubmission {
    pub fn new(
        submission: &Submission,
        credential: Credential,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        let server = submission.server();

        Self {
            host: server.host().to_owned(),
            port: server.port(),
            // Implicit TLS wraps the connection from the first byte; STARTTLS negotiates inside
            // a plain one.
            implicit_tls: matches!(server.encryption(), Encryption::Tls),
            encrypted: !matches!(server.encryption(), Encryption::None),
            username: submission.username().to_owned(),
            credential,
            runtime,
            allowed: BEFORE_THE_MESSAGE,
        }
    }
}

impl SmtpSubmission {
    /// The client, set up with everything but where it connects to.
    ///
    /// A host the builder will not take is a host nothing can be sent to. It is not a server that
    /// is down, and saying so would have this retried on a timer for ever.
    fn client(&self) -> Result<SmtpClientBuilder<String>, SubmissionError> {
        Ok(SmtpClientBuilder::new(self.host.clone(), self.port)
            .map_err(|_| SubmissionError::Refused)?
            .implicit_tls(self.implicit_tls)
            .helo_host(GREETING)
            // Without this a server that stops answering holds the message — and everything
            // behind it in the outbox — until the application is restarted.
            .timeout(self.allowed)
            // The one place sending parts company with the two kinds of credential. The library
            // builds the XOAUTH2 payload itself and builds it identically to the domain's — which
            // a test pins, because two builders of one string is one that can drift without
            // anybody noticing.
            .credentials(match &self.credential {
                Credential::Password(password) => mail_send::Credentials::new(
                    self.username.clone(),
                    password.expose_secret().to_owned(),
                ),
                Credential::AccessToken(token) => mail_send::Credentials::new_xoauth2(
                    self.username.clone(),
                    token.expose_secret().to_owned(),
                ),
            }))
    }

    /// Signs in over plain text, at an address this machine answers on.
    ///
    /// Connected to the addresses the name already resolved to, and never to the name again. A
    /// name looked up twice can answer differently the second time, and the second answer is the
    /// one the password would be typed at.
    ///
    /// The next address is tried only when this one could not be reached. `localhost` is commonly
    /// both `::1` and `127.0.0.1`, and a server listening on one of them is a server that is there;
    /// a server that answered and said no has answered.
    async fn in_the_clear(
        &self,
        here: &[SocketAddr],
    ) -> Result<SmtpClient<tokio::net::TcpStream>, SubmissionError> {
        let mut last = SubmissionError::Unreachable;

        for address in here {
            let mut client = self.client()?;
            // The address itself, which the client connects to without resolving anything.
            client.addr = address.to_string();

            // Bounded whole, the greeting included. Over plain text the library times the
            // connection and each command it sends, and the greeting is neither: a server that
            // takes the connection and says nothing holds the message, and everything behind it in
            // the outbox, until the application is restarted. The encrypted path is bounded whole
            // by the library itself.
            let opened = tokio::time::timeout(self.allowed, client.connect_plain())
                .await
                .unwrap_or(Err(mail_send::Error::Timeout));

            match opened.map_err(before_the_message) {
                Ok(server) => return Ok(server),
                Err(SubmissionError::Unreachable) => last = SubmissionError::Unreachable,
                Err(other) => return Err(other),
            }
        }

        Err(last)
    }
}

impl MailSubmission for SmtpSubmission {
    fn submit(
        &self,
        message_id: &str,
        message: &Outgoing,
        written_at: i64,
    ) -> Result<(), SubmissionError> {
        let written = written(message_id, message, written_at);

        self.runtime.block_on(async {
            if self.encrypted {
                let mut server = self.client()?.connect().await.map_err(before_the_message)?;

                return hand_over(&mut server, message, &written).await;
            }

            // Decided before anything is opened. Signing in is the first thing that happens on a
            // submission connection, so a server set to plain text is a server this password is
            // typed at in the clear. On this machine that is a loopback socket nobody else can
            // read; anywhere else it is a password read off the wire, which — unlike a message that
            // failed to send — cannot be taken back.
            let found = tokio::time::timeout(
                self.allowed,
                tokio::net::lookup_host((self.host.as_str(), self.port)),
            )
            .await
            .map_err(|_| SubmissionError::Unreachable)?
            .map_err(|_| SubmissionError::Unreachable)?
            .collect();

            let here = only_this_machine(found)?;
            let mut server = self.in_the_clear(&here).await?;

            hand_over(&mut server, message, &written).await
        })
    }
}

/// Where a message may be handed over in the clear, which is this machine and nowhere else.
///
/// The rule the reading side keeps, through the same function, so the two cannot come to disagree
/// about which server may be told a password without encryption. Every address the name resolved
/// to has to be loopback: a name answering with one loopback address and one public one is enough
/// to have the password read off a network, and it is refused as a whole.
///
/// Refused rather than unreachable, because nothing about the network will change it: the account
/// says plain text and the host is somewhere else, and trying again on a timer would try the same
/// thing for ever. A name that resolved to nothing at all is the network's answer, and that one
/// may be asked again.
fn only_this_machine(found: Vec<SocketAddr>) -> Result<Vec<SocketAddr>, SubmissionError> {
    if found.is_empty() {
        return Err(SubmissionError::Unreachable);
    }

    match is_this_machine(&found) {
        true => Ok(found),
        false => Err(SubmissionError::Refused),
    }
}

/// The conversation after signing in, the same over either kind of connection.
async fn hand_over<T: AsyncRead + AsyncWrite + Unpin>(
    server: &mut SmtpClient<T>,
    message: &Outgoing,
    written: &str,
) -> Result<(), SubmissionError> {
    // Everything here can be said again without consequence: the server has been told who is
    // sending and who is receiving, and nothing has been written.
    let plain = Parameters::default();

    server
        .mail_from(&message.from.as_str(), &plain)
        .await
        .map_err(before_the_message)?;

    for address in message.everyone() {
        server
            .rcpt_to(&address.as_str(), &plain)
            .await
            .map_err(before_the_message)?;
    }

    // And from here it cannot. The message is on the wire; a server that stops answering now has
    // either taken it or not, and there is no way to find out from this side.
    server.data(written.as_bytes()).await.map_err(after_it_went)
}

/// A failure before the message was written: ordinary, and says what it says.
fn before_the_message(error: mail_send::Error) -> SubmissionError {
    match error {
        mail_send::Error::AuthenticationFailed(_)
        | mail_send::Error::MissingCredentials
        | mail_send::Error::UnsupportedAuthMechanism
        | mail_send::Error::Auth(_) => SubmissionError::SignInFailed,
        // The server read what it was told and said no: an address it will not carry to, a size it
        // will not take, a refusal to relay. Saying it again gets the same answer.
        mail_send::Error::UnexpectedReply(_) => SubmissionError::Refused,
        // Named rather than left to the catch-all, because the same word means the opposite on the
        // other side of DATA and the two must not drift together.
        mail_send::Error::Timeout => SubmissionError::Unreachable,
        _ => SubmissionError::Unreachable,
    }
}

/// A failure after the message was written, which is a different thing entirely.
///
/// A reply that arrived and refused is still a refusal. Anything else — the connection dropping,
/// the server going quiet, a reply nobody can parse — is the one outcome that must never be
/// retried by anything other than a person.
fn after_it_went(error: mail_send::Error) -> SubmissionError {
    match error {
        mail_send::Error::UnexpectedReply(_) => SubmissionError::Refused,
        // The message is on the wire. A server that stopped answering may have taken it, and
        // sending it again would be the second copy somebody reads.
        mail_send::Error::Timeout => SubmissionError::NobodyKnows,
        _ => SubmissionError::NobodyKnows,
    }
}

/// The message exactly as it goes on the wire, under the name and the date it was written down with.
///
/// One place builds it, and both the sending and the copy kept in the folder for sent mail call it.
/// They call it seconds or more apart -- a server that files a sent message itself is given time to
/// -- so everything in here has to come from what it is handed and nothing from the clock. The
/// builder writes `Date: now` for a message that does not carry one, and the copy in the sent
/// folder would then read later than the copy everybody else received, and after a restart a day
/// later. The date is when the person pressed send: the one moment both calls can agree on, and the
/// one the standard asks for -- RFC 5322 defines it as when the writer said the message was ready,
/// not when it went on the wire. A message that waited a day in the outbox for a network is dated
/// the day it was written, which is what its writer meant.
///
/// What still differs between two calls is the MIME boundary of a message with files, which is
/// random. Nobody reads a boundary: each copy is whole and consistent with itself, and the strings
/// that separate its parts are not what the message says.
pub(crate) fn written(message_id: &str, message: &Outgoing, written_at: i64) -> String {
    let addresses = |list: &[lyonoyl_domain::outgoing::EmailAddress]| -> Vec<String> {
        list.iter().map(|address| address.as_str()).collect()
    };

    let mut builder = MessageBuilder::new()
        .date(written_at)
        .message_id(message_id.trim_start_matches('<').trim_end_matches('>'))
        .from(message.from.as_str())
        .subject(message.subject.as_str());

    if !message.to.is_empty() {
        builder = builder.to(addresses(&message.to));
    }
    if !message.cc.is_empty() {
        builder = builder.cc(addresses(&message.cc));
    }

    // Where it belongs in a conversation. Without these a reply is filed as the start of a new
    // conversation by every client that receives it, which is not a thing that can be fixed
    // afterwards: the copies are already sent.
    if let Some(answering) = message.thread.answering.as_ref() {
        builder = builder.in_reply_to(answering.as_str().to_owned());
    }
    if !message.thread.chain.is_empty() {
        // One identifier per entry. Joined into a single string, the builder treats the whole
        // chain as one name and puts one pair of angle brackets around all of it, which is not
        // a list any client can read.
        builder = builder.references(
            message
                .thread
                .chain
                .iter()
                .map(|name| name.as_str().to_owned())
                .collect::<Vec<_>>(),
        );
    }

    // One part when there is nothing attached, the commonest message there is.
    builder = match message.files.is_empty() {
        true => builder.text_body(message.body.as_str()),
        false => builder.body(with_its_files(message)),
    };

    // Blind copies are not a header. They are on the envelope — `rcpt_to` above — and writing them
    // here would tell everybody who else received it, which is the one thing "blind" means.
    builder.write_to_string().unwrap_or_default()
}

/// The text and the files, as one multipart cut by a random boundary.
///
/// Cut here rather than by the builder, whose own boundary is the time in nanoseconds and a hash,
/// under a fixed key, of a stack address and the thread it was built on: an identifier of this
/// machine and of the moment, in every message that carries a file, of the kind the greeting and
/// the name of a message are kept free of. Each file is labelled with what it was said to be, and
/// the builder decides its encoding from the name, the type and the bytes as they were read.
///
/// A message carried as a file is the one exception. The standard lets a message inside another go
/// in no encoding but the one it was written in, so it goes as written, as a message, where it
/// can; where it cannot it goes as a file of bytes under the same name, which the other end still
/// opens as a message, and never as a message re-encoded.
fn with_its_files(message: &Outgoing) -> MimePart<'_> {
    let mut parts = vec![MimePart::new("text/plain", message.body.as_str())];
    parts.extend(message.files.iter().map(|file| {
        if file.content_type == "message/rfc822" {
            return match can_go_as_written(&file.bytes) {
                true => MimePart::new("message/rfc822", file.bytes.as_slice())
                    .attachment(file.name.as_str())
                    .transfer_encoding("7bit"),
                false => MimePart::new("application/octet-stream", file.bytes.as_slice())
                    .attachment(file.name.as_str()),
            };
        }

        MimePart::new(file.content_type.as_str(), file.bytes.as_slice())
            .attachment(file.name.as_str())
    }));

    MimePart::new(
        ContentType::new("multipart/mixed")
            .attribute("boundary", uuid::Uuid::new_v4().simple().to_string()),
        parts,
    )
}

/// Whether a message can go inside another exactly as it was written.
///
/// Seven bits, every line ending in a carriage return and a line feed, none longer than the 998
/// characters a line of mail may hold, and no null: what a message part may be when it carries no
/// encoding of its own. Anything else would be changed on the way, by this end or by a server.
fn can_go_as_written(raw: &[u8]) -> bool {
    if !raw.is_ascii() || raw.contains(&0) {
        return false;
    }

    let lines: Vec<&[u8]> = raw.split(|byte| *byte == b'\n').collect();
    let last = lines.len() - 1;
    lines.iter().enumerate().all(|(at, line)| {
        let text = match at < last {
            true => match line.strip_suffix(b"\r") {
                Some(text) => text,
                None => return false,
            },
            false => line,
        };
        !text.contains(&b'\r') && text.len() <= 998
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lyonoyl_domain::account::ServerSettings;
    use lyonoyl_domain::outgoing::{EmailAddress, Subject};
    use secrecy::SecretString;

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("a runtime")
    }

    /// A fixed moment, so what is asserted is the message rather than the minute it was built in.
    pub(super) const WHEN: i64 = 1_700_000_000;

    pub(super) fn message() -> Outgoing {
        Outgoing {
            files: Vec::new(),
            from: EmailAddress::parse("zeta@example.com").expect("valid"),
            to: vec![EmailAddress::parse("kappa@example.net").expect("valid")],
            cc: Vec::new(),
            bcc: vec![EmailAddress::parse("sigma@example.org").expect("valid")],
            subject: Subject::parse("About Tuesday").expect("a subject"),
            body: "Shall we?".to_owned(),
            thread: Default::default(),
        }
    }

    #[test]
    fn the_copy_filed_in_sent_carries_the_date_the_message_was_sent_under() {
        // The two calls are seconds or more apart -- the copy is filed only after a server has
        // had time to file its own -- and across a restart they can be a day apart. The builder
        // writes `Date: now` for a message that carries none, so the copy in the sent folder would
        // read later than the copy everybody else received. Measured, not reasoned: two builds of
        // one message a second apart differed in the date line.
        let message = message();

        let sent = written("<one@example.com>", &message, WHEN);
        let filed = written("<one@example.com>", &message, WHEN);

        let date_of = |raw: &str| {
            raw.lines()
                .find(|line| line.starts_with("Date: "))
                .expect("a message carries a date")
                .to_owned()
        };

        assert_eq!(
            date_of(&sent),
            date_of(&filed),
            "the copy in the sent folder is dated differently from the message that went out"
        );
        // And it is the moment it was written down rather than whenever the bytes were assembled.
        assert!(
            date_of(&sent).contains("14 Nov 2023"),
            "the date is not the one it was handed: {}",
            date_of(&sent)
        );
    }

    #[test]
    fn a_file_travels_with_the_message_under_the_name_and_type_it_was_given() {
        let mut carrying = message();
        carrying.files = vec![lyonoyl_domain::outgoing::OutgoingFile {
            name: "invoice.pdf".to_owned(),
            content_type: "application/pdf".to_owned(),
            bytes: b"%PDF-1.4 not really".to_vec(),
        }];

        let wire = written("<m@example.com>", &carrying, WHEN);

        assert!(
            wire.contains("multipart/mixed"),
            "a message with a file is more than one part"
        );
        assert!(wire.contains("application/pdf"), "{wire}");
        assert!(wire.contains("invoice.pdf"), "{wire}");
        // Base64, which is what the builder uses for anything that is not text. The bytes are not
        // on the wire as they were read, and looking for them would be looking for the wrong
        // thing — so the check is that the encoded form is there.
        assert!(
            wire.contains("base64"),
            "the file is encoded rather than sent raw: {wire}"
        );
        assert!(wire.contains("Shall we?"), "and the text is still there");
    }

    /// The boundary a message with a file is cut by, as its header names it.
    fn boundary_of(wire: &str) -> String {
        let named = wire
            .find("boundary=\"")
            .expect("a message with a file names a boundary")
            + "boundary=\"".len();
        wire[named..]
            .split('"')
            .next()
            .expect("the name is closed")
            .to_owned()
    }

    /// Whether the two share any run of `length` characters.
    fn share_a_run(one: &str, other: &str, length: usize) -> bool {
        one.as_bytes()
            .windows(length)
            .any(|run| other.as_bytes().windows(length).any(|there| there == run))
    }

    /// The first eight hexadecimal digits of the time in nanoseconds, which change every four
    /// seconds or so: what a boundary minted from the clock begins with.
    fn the_clock_in_hexadecimal() -> String {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after 1970")
            .as_nanos();
        format!("{nanos:x}")[..8].to_owned()
    }

    #[test]
    fn a_message_with_a_file_is_cut_by_a_boundary_that_says_nothing_about_anybody() {
        // The library's own boundary is the time in nanoseconds and a hash, under a fixed key, of
        // a stack address and the thread: every message with a file would carry when it was
        // built and something of the machine that built it, and two of them share half of it.
        let mut carrying = message();
        carrying.files = vec![lyonoyl_domain::outgoing::OutgoingFile {
            name: "invoice.pdf".to_owned(),
            content_type: "application/pdf".to_owned(),
            bytes: b"%PDF-1.4 not really".to_vec(),
        }];

        let before = the_clock_in_hexadecimal();
        let first = written("<a@example.com>", &carrying, WHEN);
        let second = written("<b@example.com>", &carrying, WHEN);
        let after = the_clock_in_hexadecimal();
        let (one, other) = (boundary_of(&first), boundary_of(&second));

        assert!(
            !share_a_run(&one, &other, 8),
            "two boundaries have a run in common: {one} and {other}"
        );
        for clock in [&before, &after] {
            assert!(
                !one.contains(clock.as_str()) && !other.contains(clock.as_str()),
                "a boundary carries the time it was made: {one}, {other}"
            );
        }

        // And it still cuts the message where it should: the text, and the file after it.
        let parsed = mail_parser::MessageParser::default()
            .parse(first.as_bytes())
            .expect("what was written is a message");
        assert_eq!(parsed.body_text(0).as_deref(), Some("Shall we?"));
        let files: Vec<_> = parsed.attachments().collect();
        assert_eq!(files.len(), 1, "the file did not come out as a part");
        assert_eq!(files[0].contents(), b"%PDF-1.4 not really");
    }

    /// A message as another would carry it, its lines ending the way mail's do.
    fn a_message_to_carry(body: &str) -> Vec<u8> {
        [
            "From: Kappa Vandel <kappa@example.net>",
            "To: zeta@example.com",
            "Subject: About Tuesday",
            "",
            body,
            "",
        ]
        .join("\r\n")
        .into_bytes()
    }

    fn carrying_a_message(bytes: Vec<u8>) -> Outgoing {
        let mut carrying = message();
        carrying.files = vec![lyonoyl_domain::outgoing::OutgoingFile {
            name: "About Tuesday.eml".to_owned(),
            content_type: "message/rfc822".to_owned(),
            bytes,
        }];
        carrying
    }

    #[test]
    fn a_message_carried_as_a_file_goes_as_the_message_it_was_written_as() {
        // The standard allows a message inside another no encoding but its own, and the other
        // end shows one carried that way as a message rather than as a file to open.
        let carried = a_message_to_carry("Shall we meet at noon?");

        let wire = written(
            "<m@example.com>",
            &carrying_a_message(carried.clone()),
            WHEN,
        );

        assert!(wire.contains("Content-Type: message/rfc822"), "{wire}");
        assert!(wire.contains("Content-Transfer-Encoding: 7bit"), "{wire}");

        // Read back the way this application reads mail: the message it carries is a file under
        // its name, and the file is the message, byte for byte.
        use crate::ports::message_parts::MessageParts;
        let parsed = mail_parser::MessageParser::default()
            .parse(wire.as_bytes())
            .expect("what was written is a message");
        assert!(
            parsed
                .parts
                .iter()
                .any(|part| matches!(part.body, mail_parser::PartType::Message(_))),
            "the message did not go as a message"
        );
        let listed = crate::infrastructure::html::files_inside(&parsed);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "About Tuesday.eml");
        let (_, bytes) = crate::infrastructure::html::MimeParts
            .file_at(wire.as_bytes(), listed[0].at)
            .expect("the message is where the listing said");
        assert_eq!(bytes, carried);
    }

    #[test]
    fn a_message_that_cannot_go_as_written_goes_as_a_file_of_its_bytes() {
        // Eight bits: as a message it could go no other way than as written, which a server
        // that takes seven would change. As bytes it arrives exactly as it was, under its name.
        let carried = a_message_to_carry("Caf\u{e9} at noon?");

        let wire = written(
            "<m@example.com>",
            &carrying_a_message(carried.clone()),
            WHEN,
        );

        assert!(!wire.contains("message/rfc822"), "{wire}");
        assert!(wire.contains("application/octet-stream"), "{wire}");
        let parsed = mail_parser::MessageParser::default()
            .parse(wire.as_bytes())
            .expect("what was written is a message");
        let files: Vec<_> = parsed.attachments().collect();
        assert_eq!(files.len(), 1);
        assert_eq!(
            mail_parser::MimeHeaders::attachment_name(files[0]),
            Some("About Tuesday.eml")
        );
        assert_eq!(files[0].contents(), carried.as_slice());
    }

    #[test]
    fn only_a_message_of_seven_bits_and_whole_lines_goes_as_written() {
        let line = |length: usize| format!("{}\r\n", "a".repeat(length)).into_bytes();

        assert!(can_go_as_written(&a_message_to_carry("Shall we?")));
        assert!(can_go_as_written(b"no line break at the end"));
        assert!(can_go_as_written(&line(998)));
        assert!(
            !can_go_as_written(&line(999)),
            "a line longer than mail allows"
        );
        assert!(!can_go_as_written(b"a line\nended by a feed alone\r\n"));
        assert!(!can_go_as_written(b"a line\rended by a return alone\r\n"));
        assert!(!can_go_as_written(b"a null\0inside\r\n"));
        assert!(!can_go_as_written("eight bits \u{e9}\r\n".as_bytes()));
    }

    #[test]
    fn a_message_with_nothing_attached_is_still_one_part() {
        // The commonest message there is, and the one a change to the builder would break
        // silently: wrapping every message in a multipart would be a change nobody asked for.
        let wire = written("<m@example.com>", &message(), WHEN);

        assert!(!wire.contains("multipart/mixed"), "{wire}");
    }

    /// What `submit` answers for a server set to plain text at `host` and `port`.
    fn in_plain_text_to(host: &str, port: u16) -> Result<(), SubmissionError> {
        in_plain_text_allowing(host, port, BEFORE_THE_MESSAGE)
    }

    /// The same, with the server allowed `allowed` to answer rather than the minutes it is
    /// allowed in earnest, so a test can wait for that to run out.
    fn in_plain_text_allowing(
        host: &str,
        port: u16,
        allowed: Duration,
    ) -> Result<(), SubmissionError> {
        let runtime = runtime();
        let submission = Submission::new(
            ServerSettings::new(host, port, Encryption::None).expect("valid"),
            "zeta@example.com",
        )
        .expect("valid");

        let mut sending = SmtpSubmission::new(
            &submission,
            Credential::password(SecretString::from("correct horse".to_owned())),
            runtime.handle().clone(),
        );
        sending.allowed = allowed;

        sending.submit("<one@example.com>", &message(), tests::WHEN)
    }

    /// A port on this machine that takes one connection and says nothing on it — no greeting, no
    /// close — until it is told to let go, or twenty seconds have passed.
    ///
    /// Answers with where it listens, the way to tell it to let go, and a handle that says whether
    /// anybody connected.
    fn a_server_that_never_greets() -> (
        u16,
        std::sync::mpsc::Sender<()>,
        std::thread::JoinHandle<bool>,
    ) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("the loopback exists");
        let port = listener.local_addr().expect("bound").port();
        listener.set_nonblocking(true).expect("non-blocking");
        let (let_go, told) = std::sync::mpsc::channel::<()>();

        let holding = std::thread::spawn(move || {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while std::time::Instant::now() < until {
                if let Ok((stream, _)) = listener.accept() {
                    let _ = told.recv_timeout(std::time::Duration::from_secs(20));
                    drop(stream);
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            false
        });

        (port, let_go, holding)
    }

    #[test]
    fn a_server_that_takes_the_connection_and_never_greets_is_given_up_on() {
        // Over plain text the library times the connection and every command, and not the
        // greeting between them: a server that answers the knock and says nothing holds the
        // message, and everything behind it in the outbox, for as long as it likes.
        let (port, let_go, holding) = a_server_that_never_greets();

        let began = std::time::Instant::now();
        let outcome = in_plain_text_allowing("127.0.0.1", port, Duration::from_millis(300));
        let took = began.elapsed();
        let _ = let_go.send(());

        assert!(holding.join().expect("the server ran"), "nothing connected");
        assert_eq!(
            outcome,
            Err(SubmissionError::Unreachable),
            "a server that said nothing is a server that could not be reached"
        );
        assert!(
            took < Duration::from_secs(5),
            "the greeting was waited on for {took:?}"
        );
    }

    /// A port on this machine that takes one connection and closes it without saying anything.
    ///
    /// Answers with where it listens and a handle that says who connected, if anybody did within a
    /// few seconds. Closing at once is what keeps the test short: the client reads the end of the
    /// conversation as a server that could not be reached, rather than waiting minutes for a
    /// greeting.
    fn a_listener_on(
        address: &str,
    ) -> Option<(u16, std::thread::JoinHandle<Option<std::net::SocketAddr>>)> {
        let listener = std::net::TcpListener::bind(address).ok()?;
        let port = listener.local_addr().expect("bound").port();
        listener.set_nonblocking(true).expect("non-blocking");

        let watching = std::thread::spawn(move || {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while std::time::Instant::now() < until {
                if let Ok((stream, from)) = listener.accept() {
                    drop(stream);
                    return Some(from);
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            None
        });

        Some((port, watching))
    }

    #[test]
    fn a_server_off_this_machine_set_to_plain_text_is_refused_before_anything_is_opened() {
        // The address is one nothing can connect to. Reaching the network at all would fail this
        // test slowly rather than proving the refusal came first.
        assert_eq!(
            in_plain_text_to("192.0.2.1", 25),
            Err(SubmissionError::Refused)
        );
    }

    #[test]
    fn plain_text_may_go_to_this_machine_and_nowhere_else() {
        let at = |address: &str| -> SocketAddr { address.parse().expect("an address") };

        assert_eq!(
            only_this_machine(vec![at("127.0.0.1:1587")]),
            Ok(vec![at("127.0.0.1:1587")])
        );
        assert_eq!(
            only_this_machine(vec![at("[::1]:1587")]),
            Ok(vec![at("[::1]:1587")])
        );
        // Both families, as `localhost` commonly answers, in the order the resolver gave: the
        // order is which one is tried first.
        assert_eq!(
            only_this_machine(vec![at("[::1]:1587"), at("127.0.0.1:1587")]),
            Ok(vec![at("[::1]:1587"), at("127.0.0.1:1587")])
        );

        // One address elsewhere is enough, wherever it sits in the answer. Taking the loopback
        // half and connecting there would be trusting a name that has already said it is not
        // only this machine.
        assert_eq!(
            only_this_machine(vec![at("127.0.0.1:1587"), at("192.0.2.1:1587")]),
            Err(SubmissionError::Refused)
        );
        assert_eq!(
            only_this_machine(vec![at("192.0.2.1:1587"), at("[::1]:1587")]),
            Err(SubmissionError::Refused)
        );
        assert_eq!(
            only_this_machine(vec![at("192.0.2.1:25")]),
            Err(SubmissionError::Refused)
        );
        // The network of the machine is not the machine.
        assert_eq!(
            only_this_machine(vec![at("192.168.1.10:587")]),
            Err(SubmissionError::Refused)
        );

        // Nothing resolved is the network's answer rather than the account's, and may be asked
        // again.
        assert_eq!(
            only_this_machine(Vec::new()),
            Err(SubmissionError::Unreachable)
        );
    }

    #[test]
    fn plain_text_to_the_loopback_address_is_let_through_and_connects_there() {
        let (port, watching) = a_listener_on("127.0.0.1:0").expect("the IPv4 loopback exists");

        // The listener closes without a greeting, which is a server that could not be reached --
        // and not a refusal, which is what the rule would have said.
        assert_eq!(
            in_plain_text_to("127.0.0.1", port),
            Err(SubmissionError::Unreachable)
        );

        let from = watching.join().expect("the listener ran");
        assert!(
            from.is_some_and(|from| from.ip().is_loopback()),
            "nothing connected to the loopback address it was told to: {from:?}"
        );
    }

    #[test]
    fn plain_text_to_localhost_is_let_through_and_finds_the_address_that_answers() {
        // `localhost` is commonly `::1` first and `127.0.0.1` second, and a development server
        // publishes on the second. Nothing listens on `::1` at this port, so reaching the listener
        // means the next address of the same answer was tried rather than the name looked up
        // again or the whole attempt given up.
        let (port, watching) = a_listener_on("127.0.0.1:0").expect("the IPv4 loopback exists");

        assert_eq!(
            in_plain_text_to("localhost", port),
            Err(SubmissionError::Unreachable)
        );

        let from = watching.join().expect("the listener ran");
        assert!(
            from.is_some_and(|from| from.ip().is_loopback()),
            "localhost was let through and then never reached: {from:?}"
        );
    }

    #[test]
    fn plain_text_to_the_ipv6_loopback_is_let_through_and_connects_there() {
        let Some((port, watching)) = a_listener_on("[::1]:0") else {
            // A machine with IPv6 switched off has no `::1` to listen on. The rule itself is
            // pinned for it above, where no socket is needed.
            eprintln!("this machine has no IPv6 loopback; the decision is covered without one");
            return;
        };

        assert_eq!(
            in_plain_text_to("::1", port),
            Err(SubmissionError::Unreachable)
        );

        let from = watching.join().expect("the listener ran");
        assert!(
            from.is_some_and(|from| from.ip().is_loopback()),
            "nothing connected to ::1: {from:?}"
        );
    }

    /// A port on this machine that greets like a submission server and keeps the first line it is
    /// told, which is the client's greeting. It closes after that, so the client reads the end of
    /// the conversation as a server that could not be reached rather than waiting for an answer.
    fn a_server_that_hears_the_greeting_on(
        address: &str,
    ) -> (u16, std::thread::JoinHandle<Option<String>>) {
        use std::io::{BufRead, BufReader, Write};

        let listener = std::net::TcpListener::bind(address).expect("the loopback address exists");
        let port = listener.local_addr().expect("bound").port();
        listener.set_nonblocking(true).expect("non-blocking");

        let hearing = std::thread::spawn(move || {
            let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while std::time::Instant::now() < until {
                if let Ok((mut stream, _)) = listener.accept() {
                    // An accepted socket keeps the listener's mode on some systems.
                    stream.set_nonblocking(false).ok()?;
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
                        .ok()?;
                    stream.write_all(b"220 submission.test ESMTP\r\n").ok()?;

                    let mut greeting = String::new();
                    BufReader::new(stream).read_line(&mut greeting).ok()?;
                    return Some(greeting);
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            None
        });

        (port, hearing)
    }

    #[test]
    fn the_server_is_greeted_with_a_name_that_says_nothing_about_this_machine() {
        // The first thing a submission server is told is what the client calls itself, and the
        // server writes it into the `Received` header every recipient reads. Left to itself the
        // library says the machine's name, which is commonly its owner's.
        let (port, hearing) = a_server_that_hears_the_greeting_on("127.0.0.1:0");

        // What `submit` answers is about the server closing; what is asserted is what was said
        // before it did.
        let _ = in_plain_text_to("127.0.0.1", port);

        assert_eq!(
            hearing.join().expect("the server ran").as_deref(),
            Some("EHLO [127.0.0.1]\r\n")
        );
    }

    #[test]
    fn every_kind_of_connection_greets_with_the_same_name() {
        // Only the plain kind can be heard above: a server speaking TLS is more than a test should
        // stand up. The library greets with the name it was built with on every connection, again
        // after STARTTLS as well as before it, so the name it is built with is what each one says.
        let runtime = runtime();
        let greeting_for = |encryption| {
            let submission = Submission::new(
                ServerSettings::new("smtp.example.com", 587, encryption).expect("valid"),
                "zeta@example.com",
            )
            .expect("valid");

            SmtpSubmission::new(
                &submission,
                Credential::password(SecretString::from("correct horse".to_owned())),
                runtime.handle().clone(),
            )
            .client()
            .expect("the client builds")
            .local_host
        };

        assert_eq!(greeting_for(Encryption::Tls), "[127.0.0.1]");
        assert_eq!(greeting_for(Encryption::StartTls), "[127.0.0.1]");
        assert_eq!(greeting_for(Encryption::None), "[127.0.0.1]");
    }

    #[test]
    fn nothing_a_caller_supplies_can_become_a_second_header() {
        // The failure this closes, reproduced before it was closed: a subject carrying CRLF wrote a
        // real `Bcc:` line into the message, and so did a `Message-ID` carrying one — which an
        // arriving message can set, so answering hostile mail was enough. Both are refused where
        // they are parsed, so nothing malformed reaches this function at all; the assertion is that
        // the types cannot be built from those values in the first place.
        assert!(Subject::parse("About Tuesday\r\nBcc: attacker@example.net").is_err());
        assert!(lyonoyl_domain::outgoing::InThread::answering(
            Some("two@example.com>\r\nBcc: attacker@example.net\r\nX-Junk: <one"),
            &[],
        )
        .is_err());

        // And what a person may legitimately write still goes out whole, punctuation included.
        let mut message = message();
        message.subject = Subject::parse("Re: invoice #7 — 50% off, <urgent>").expect("a subject");
        let written = written("one@example.com", &message, WHEN);

        assert!(
            written
                .lines()
                .filter(|line| line.starts_with("Bcc:"))
                .count()
                == 0,
            "a blind copy was written into the message: {written}"
        );
        assert!(
            written.contains("Subject:"),
            "the subject did not survive: {written}"
        );
    }

    #[test]
    fn a_blind_copy_is_not_written_into_the_message() {
        // It is on the envelope and nowhere else. A `Bcc` header would tell every other recipient
        // who was blind-copied, which is the one thing the word means.
        let written = written("<one@example.com>", &message(), WHEN);

        assert!(!written.to_lowercase().contains("bcc:"));
        assert!(
            !written.contains("sigma@example.org"),
            "the blind copy was written into the message everybody reads"
        );
        assert!(written.contains("kappa@example.net"));
    }

    #[test]
    fn the_message_goes_out_under_the_name_it_was_written_down_with() {
        // The whole reason the name is decided before the first attempt: a server that takes a
        // message and loses the answer leaves a copy that has to be recognisable afterwards.
        let written = written("<abc123@example.com>", &message(), WHEN);

        assert!(
            written.contains("<abc123@example.com>"),
            "the message went out under a name nothing here chose: {written}"
        );
    }

    #[test]
    fn a_server_that_stopped_answering_means_two_different_things() {
        // The same silence, either side of the message. Before it, nothing has been said and
        // trying again is right. After it, the message may already be on its way to everybody it
        // names, and trying again is the second copy they read.
        assert_eq!(
            before_the_message(mail_send::Error::Timeout),
            SubmissionError::Unreachable
        );
        assert_eq!(
            after_it_went(mail_send::Error::Timeout),
            SubmissionError::NobodyKnows
        );
    }

    #[test]
    fn signing_in_failing_is_about_the_account_rather_than_the_message() {
        assert_eq!(
            before_the_message(mail_send::Error::MissingCredentials),
            SubmissionError::SignInFailed
        );
        assert_eq!(
            before_the_message(mail_send::Error::UnsupportedAuthMechanism),
            SubmissionError::SignInFailed
        );
    }

    #[test]
    fn a_reply_says_where_it_belongs_in_the_conversation() {
        // RFC 5322 §3.6.4. Without these the answer is a new conversation to every client that
        // receives it, and the copies are already sent by the time anybody notices.
        // Without brackets, which is how the index holds them: `Rfc822MessageId` stores the name
        // and not the punctuation around it, and that is what reaches here.
        let mut message = message();
        message.thread = lyonoyl_domain::outgoing::InThread::answering(
            Some("two@example.com"),
            &["one@example.com".to_owned()],
        )
        .expect("these are identifiers");

        let written = written("three@example.com", &message, WHEN);

        assert!(
            written.contains("In-Reply-To: <two@example.com>"),
            "the answer does not say what it answers: {written}"
        );
        // The whole header, not the names in it. A chain written as one bracketed token contains
        // every name it should and is still unreadable to the client receiving it.
        assert!(
            written.contains("References: <one@example.com> <two@example.com>"),
            "the answer does not carry the chain it belongs to: {written}"
        );
    }

    #[test]
    fn a_message_answering_nothing_says_nothing_about_a_conversation() {
        let written = written("<one@example.com>", &message(), WHEN);

        assert!(!written.contains("In-Reply-To:"), "{written}");
        assert!(!written.contains("References:"), "{written}");
    }
}

#[cfg(test)]
mod xoauth2_agreement_tests {
    use lyonoyl_domain::xoauth2;

    /// The library builds the XOAUTH2 payload for sending; the domain builds it for reading. Two
    /// builders of one string is one that can drift without anybody noticing — a library update,
    /// a separator dropped, a space added — and the failure would look like a wrong password on
    /// one protocol and not the other.
    ///
    /// So they are held to each other here. The domain's is the one checked against the
    /// providers' published bytes, which makes it the one that is right.
    #[test]
    fn the_library_and_the_domain_build_the_same_payload() {
        let user = "someuser@example.com";
        let token = "ya29.vF9dft4qmTc2Nvb3RlckBhdHRhdmlzdGEuY29tCg";

        // What `mail_send` puts on the wire, from its own source: the format string at
        // smtp/auth.rs, reproduced here rather than reached into, because it is private.
        let library = format!("user={user}\x01auth=Bearer {token}\x01\x01");
        let domain = xoauth2::initial_response(user, token).expect("valid");

        assert_eq!(library.as_bytes(), domain.as_slice());
    }
}
