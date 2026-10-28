//! The catch that stops a debugging session from deleting someone's mail.
//!
//! `scripts/check-layering.sh` refuses some of the commands that change a mailbox, spelled out or
//! called the plain way in a file they do not belong in, which is the first line and the cheap one.
//! This is the second, and the one every command meets: the bytes on their way to the server are
//! read, and a command that would change the mailbox never leaves the machine — whoever wrote it,
//! whether it came from this adapter or from inside the client library.
//!
//! There are two kinds of connection, and they are guarded differently.
//!
//! A connection opened to **read** — a pass, a watch, fetching a body — may change no mailbox and
//! no message at all. That is where an accident happens: the most likely severe incident of this
//! project is the person writing it losing their own mail to a pass that ran while
//! synchronisation was being debugged. It opens a mailbox with `EXAMINE`, and `SELECT` is refused
//! on it: in a mailbox opened for writing, a fetch of a whole message marks it read, and what asks
//! for that is an argument rather than the command this guard reads.
//!
//! A connection opened to **act** carries what a person asked for, and may send exactly five
//! things that change mail: `UID STORE`, `UID MOVE`, `UID COPY` and `UID EXPUNGE`, which change
//! messages it names, and `APPEND`, which adds one. It opens the mailbox the four change with
//! `SELECT`. Every other command on the list below is refused on this connection too, and the two
//! exclusions are the point:
//!
//! - **The sequence-number forms.** A bare `STORE 3 …` names the third message *right now*, and a
//!   message expunged by another client a moment ago shifts every number after it. Only UID forms
//!   name a message.
//! - **A bare `EXPUNGE`.** It removes every message in the mailbox marked `\Deleted`, including
//!   ones another client marked and has not finished with. `UID EXPUNGE` removes the ones named
//!   and nothing else.
//!
//! So there is no build, and no environment variable, in which this application can create, rename
//! or delete a mailbox, or expunge anything it did not name.
//!
//! What an acting connection can still do that the list does not hold is fetch a whole message in a
//! mailbox it opened for writing, and so mark the message read. The guard reads no further than the
//! command, so it cannot tell that fetch from one that marks nothing. `scripts/check-layering.sh`
//! refuses `BODY[` written in capitals in a string, since one word is all that separates such a
//! fetch from the ones this adapter sends, and nothing checks `RFC822`, `RFC822.TEXT` or `BINARY[`,
//! which mark a message read as well. In what ships, nothing is fetched on an acting connection.
//!
//! The one thing on an acting connection that is not read is the message an `APPEND` adds. Its
//! line ends saying how many bytes follow, and once the server has said to go on, exactly those
//! bytes pass counted rather than read: a line of somebody's writing that begins "Subject: Move"
//! is not a command, and judging it as one refuses to file the message. The line after them is
//! judged like any other, and an `APPEND` the server will not take is followed by no message at
//! all, so nothing written after it passes unread. Nor does a message announced as `{N+}`, which
//! follows without waiting for the server: a server that does not take that form answers the line
//! `BAD` and runs what follows as commands, so what follows is judged as commands.

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// What is refused: every command that changes a mailbox or its messages, and the one that opens a
/// mailbox where a fetch does.
///
/// `CLOSE` is here because on a mailbox opened for writing it expunges what is marked deleted. A
/// session is ended with `LOGOUT`, which does not. `REPLACE` adds a message and expunges the one it
/// names, in one command and in either form. `SETANNOTATION` writes what `SETMETADATA` writes, by
/// the name some servers know it by. `SELECT` opens a mailbox for writing, and there a fetch of a
/// whole message marks it read; `EXAMINE` opens the same mailbox to be read, and nothing fetched
/// from it marks anything.
const CHANGES_THE_SERVER: [&str; 16] = [
    "STORE",
    "APPEND",
    "EXPUNGE",
    "MOVE",
    "COPY",
    "REPLACE",
    "CREATE",
    "DELETE",
    "RENAME",
    "SETACL",
    "DELETEACL",
    "SETQUOTA",
    "SETMETADATA",
    "SETANNOTATION",
    "CLOSE",
    "SELECT",
];

/// As much of a line as can hold a command word. The rest of a line is arguments, and arguments are
/// message contents and passwords — read no further than the decision needs.
const ENOUGH_FOR_A_COMMAND: usize = 64;

/// What a connection was opened to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// Reading mail. Nothing that changes a mailbox or its messages may leave, and a mailbox is
    /// opened only to be read.
    Reading,
    /// Carrying out what a person asked for. The four UID forms below, `APPEND`, and the `SELECT`
    /// the four need may leave, and nothing else on the list.
    Acting,
}

/// The only commands an acting connection may send that change messages already there.
///
/// UID forms only, and `EXPUNGE` among them only in its UID form: see the note at the top of this
/// file for why each exclusion is not a matter of taste.
const WHAT_ACTING_MAY_SEND: [&str; 4] = ["STORE", "MOVE", "COPY", "EXPUNGE"];

/// The one command an acting connection may send that names no message.
///
/// `APPEND` puts a message into a mailbox rather than changing one already in it, so the reason
/// every other allowance is UID-only — a sequence number names whichever message is in that
/// position right now — has nothing to bite on here: it names no message at all.
///
/// What it can do instead is add mail, and the only mail this application adds is a copy of one it
/// has just handed to a submission server, into the folder the account keeps sent mail in. A
/// message sent and filed nowhere is a message the person cannot find afterwards, and servers
/// differ on whether they file it themselves.
const WHAT_ACTING_MAY_ADD: &str = "APPEND";

/// How an acting connection opens a mailbox: for writing, because the four change what is in it,
/// and a server refuses every one of them in a mailbox opened only to be read.
const WHAT_ACTING_MAY_OPEN: &str = "SELECT";

/// The command a line carries, and whether it was addressed by UID.
///
/// A line arrives as `tag COMMAND arguments`, and `UID` is a prefix rather than a command of its
/// own, so `UID FETCH` is a fetch and `UID STORE` is a store — a different one from bare `STORE`.
fn command_of(line: &[u8]) -> Option<(bool, String)> {
    let text = String::from_utf8_lossy(line);
    let mut words = text.split_ascii_whitespace();

    let _tag = words.next()?;
    let first = words.next()?.to_ascii_uppercase();

    if first == "UID" {
        return words.next().map(|word| (true, word.to_ascii_uppercase()));
    }
    Some((false, first))
}

/// Whether this line would change something the connection is not allowed to change.
fn refuses(line: &[u8], purpose: Purpose) -> Option<String> {
    let (by_uid, command) = command_of(line)?;

    if !CHANGES_THE_SERVER.contains(&command.as_str()) {
        return None;
    }

    let permitted = purpose == Purpose::Acting
        && ((by_uid && WHAT_ACTING_MAY_SEND.contains(&command.as_str()))
            || command == WHAT_ACTING_MAY_ADD
            || command == WHAT_ACTING_MAY_OPEN);

    (!permitted).then_some(command)
}

/// Whether this line, already allowed, is the one that adds a message.
fn adds_a_message(line: &[u8], purpose: Purpose) -> bool {
    purpose == Purpose::Acting
        && command_of(line).is_some_and(|(_, command)| command == WHAT_ACTING_MAY_ADD)
}

/// As much of the end of an `APPEND` line as says how long its message is: `{`, twenty digits and
/// `}`, and the carriage return before the line feed.
const LONGEST_ANNOUNCEMENT: usize = 23;

/// Where the message an allowed `APPEND` carries has got to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Message {
    /// None is on its way.
    #[default]
    None,
    /// Announced, this many bytes long, and waiting for the server to say to go on. Anything
    /// written before it says so is not the message.
    Announced(u64),
    /// On its way: this many bytes of it still to pass, unread.
    Going(u64),
}

/// What the end of an `APPEND` line announces: `{N}`, a message that waits for the server to say
/// to go on. Anything else announces nothing, and what follows the line is read like any other
/// line.
///
/// `{N+}` among them. It says the message follows without waiting, and a server that does not
/// take that form answers the line `BAD` and runs the next N bytes as commands: passed unread on
/// the client's word, an `APPEND` announcing `{17+}` carries `a2 DELETE INBOX` past this guard.
fn announced(end: &[u8]) -> Message {
    let end = end.strip_suffix(b"\r").unwrap_or(end);
    let Some(digits) = end.strip_suffix(b"}").and_then(|end| {
        let opening = end.iter().rposition(|byte| *byte == b'{')?;
        Some(&end[opening + 1..])
    }) else {
        return Message::None;
    };

    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return Message::None;
    }
    let Some(length) = std::str::from_utf8(digits)
        .ok()
        .and_then(|digits| digits.parse::<u64>().ok())
    else {
        return Message::None;
    };

    match length {
        0 => Message::None,
        length => Message::Announced(length),
    }
}

/// How far into a line the reading has got.
///
/// Kept separately from the stream so a write can be judged before it happens and the state moved
/// on only by the bytes the stream actually took. A partial write is normal — the caller comes back
/// with the rest — and a state advanced by bytes that never left would misread the line.
#[derive(Clone, Default)]
struct Progress {
    /// The beginning of the line being written, up to the length a command word needs.
    line: Vec<u8>,
    /// Whether this line has already been judged, so what follows the command word — a password,
    /// a message — is passed through without being looked at.
    judged: bool,
    /// The last bytes of an allowed `APPEND` line, and of no other: how it ends says how long the
    /// message after it is. Its arguments are a mailbox, flags and a length, never a password.
    end: Option<Vec<u8>>,
    /// The message an allowed `APPEND` carries, where it has got to.
    message: Message,
}

impl Progress {
    /// Reads outgoing bytes far enough to recognise a command, refusing the ones that write.
    fn advanced(&self, buffer: &[u8], purpose: Purpose) -> Result<Self, io::Error> {
        let mut next = self.clone();
        let mut rest = buffer;

        while let Some((&byte, after)) = rest.split_first() {
            // The message itself: passed on counted, and not read. Exactly as many bytes as its
            // line announced, and the line after them is judged like any other.
            if let Message::Going(left) = next.message {
                let passing = rest.len().min(usize::try_from(left).unwrap_or(usize::MAX));
                rest = &rest[passing..];
                next.message = match left - passing as u64 {
                    0 => Message::None,
                    left => Message::Going(left),
                };
                continue;
            }
            // Written before the server said to go on, so it is not the message: an `APPEND` the
            // server refused is followed by whatever the client says next, and that is judged.
            if let Message::Announced(_) = next.message {
                next.message = Message::None;
            }
            rest = after;

            if byte == b'\n' {
                if let Some(end) = next.end.take() {
                    next.message = announced(&end);
                }
                next.line.clear();
                next.judged = false;
                continue;
            }

            if let Some(end) = next.end.as_mut() {
                if end.len() == LONGEST_ANNOUNCEMENT {
                    end.remove(0);
                }
                end.push(byte);
            }
            if next.judged {
                continue;
            }

            next.line.push(byte);

            // A command word is complete once whitespace follows it, and a line long enough to hold
            // one has been seen in full.
            if next.line.len() >= ENOUGH_FOR_A_COMMAND || byte.is_ascii_whitespace() {
                if let Some(command) = refuses(&next.line, purpose) {
                    return Err(refusal(&command));
                }
                if next.end.is_none() && adds_a_message(&next.line, purpose) {
                    next.end = Some(Vec::with_capacity(LONGEST_ANNOUNCEMENT));
                }
                // Two words settle it, except after `UID`, which needs a third.
                let words = next.line.split(|byte| byte.is_ascii_whitespace()).count();
                next.judged = words > 3 || next.line.len() >= ENOUGH_FOR_A_COMMAND;
            }
        }

        Ok(next)
    }
}

/// The refusal, as an error the protocol adapter reports like any other write failure.
///
/// The command word and nothing else: the rest of the line is a password or a message.
fn refusal(command: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{command} would change mail on the server, and this build only reads"),
    )
}

/// A stream that reads like any other and writes only what its purpose allows.
pub struct ReadOnly<S> {
    inner: S,
    progress: Progress,
    purpose: Purpose,
    /// Whether the next byte the server sends begins a line: how a server's answer to an
    /// announced message is told apart from the middle of something it was already saying.
    at_line_start: bool,
}

impl<S> ReadOnly<S> {
    /// A connection that reads mail. Nothing that changes a mailbox or its messages leaves it.
    pub fn guarding(inner: S) -> Self {
        Self::with(inner, Purpose::Reading)
    }

    /// A connection that carries out what a person asked for.
    pub fn acting(inner: S) -> Self {
        Self::with(inner, Purpose::Acting)
    }

    fn with(inner: S, purpose: Purpose) -> Self {
        Self {
            inner,
            progress: Progress::default(),
            purpose,
            at_line_start: true,
        }
    }

    /// Reads what the server said for the one answer the guard waits on: whether a message that
    /// was announced may follow.
    ///
    /// The first line the server begins after the announcement decides it. A line beginning `+`
    /// says to go on, and any other line means no message is coming — the client library reads the
    /// first answer the same way and gives the message up on anything but a `+`.
    fn heard(&mut self, bytes: &[u8]) {
        // Nothing announced, nothing to wait on: all that matters is where the next line begins,
        // and a message being read whole is not read twice for it.
        if !matches!(self.progress.message, Message::Announced(_)) {
            if let Some(last) = bytes.last() {
                self.at_line_start = *last == b'\n';
            }
            return;
        }

        for byte in bytes {
            if self.at_line_start {
                if let Message::Announced(length) = self.progress.message {
                    self.progress.message = match byte {
                        b'+' => Message::Going(length),
                        _ => Message::None,
                    };
                }
            }
            self.at_line_start = *byte == b'\n';
        }
    }
}

/// Prints what it is for and nothing else. What passes through is a password on the way to a login
/// and the contents of someone's mail on the way back.
impl<S> std::fmt::Debug for ReadOnly<S> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self.purpose {
            Purpose::Reading => "ReadOnly::guarding",
            Purpose::Acting => "ReadOnly::acting",
        })
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for ReadOnly<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let stream = self.get_mut();
        let before = buffer.filled().len();

        let polled = Pin::new(&mut stream.inner).poll_read(context, buffer);
        if let Poll::Ready(Ok(())) = polled {
            stream.heard(&buffer.filled()[before..]);
        }

        polled
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for ReadOnly<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        let stream = self.get_mut();

        // A client that writes anything at all while an announced message waits for the server,
        // refused or not, is not sending that message: it follows only a server that said to go
        // on, and the server's word cannot make what comes after this a message.
        if matches!(stream.progress.message, Message::Announced(_)) && !buffer.is_empty() {
            stream.progress.message = Message::None;
        }

        // Judged before a single byte of it is written: a refused command must not reach the
        // server even in part. Every connection is judged, including one opened to act — what
        // differs is the short list it may send, never whether it is looked at.
        stream.progress.advanced(buffer, stream.purpose)?;

        let written = match Pin::new(&mut stream.inner).poll_write(context, buffer) {
            Poll::Ready(Ok(written)) => written,
            other => return other,
        };

        // Only what the stream took moves the reading on. The rest arrives again in the next call.
        stream.progress = stream
            .progress
            .advanced(&buffer[..written], stream.purpose)
            .expect("a prefix of bytes already accepted is accepted");

        Poll::Ready(Ok(written))
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(context)
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::AsyncWriteExt;

    use super::*;

    /// Writes through a guarded stream into a buffer, and says what got through.
    async fn guarded(lines: &[&str]) -> (Result<(), io::Error>, String) {
        written_through(lines, Purpose::Reading).await
    }

    /// The same, on a connection opened to carry out what a person asked for.
    async fn acting(lines: &[&str]) -> (Result<(), io::Error>, String) {
        written_through(lines, Purpose::Acting).await
    }

    async fn written_through(lines: &[&str], purpose: Purpose) -> (Result<(), io::Error>, String) {
        let mut stream = ReadOnly::with(Vec::new(), purpose);
        let mut outcome = Ok(());

        for line in lines {
            outcome = stream.write_all(line.as_bytes()).await;
            if outcome.is_err() {
                break;
            }
        }

        let written = String::from_utf8(stream.inner.clone()).expect("what was written is text");
        (outcome, written)
    }

    #[tokio::test]
    async fn reading_a_mailbox_goes_through_untouched() {
        let conversation = [
            "a1 LOGIN person correct-horse\r\n",
            "a2 CAPABILITY\r\n",
            "a3 LIST \"\" \"*\"\r\n",
            "a4 EXAMINE INBOX\r\n",
            "a5 UID FETCH 1:* (UID FLAGS BODY.PEEK[HEADER])\r\n",
            "a6 UID SEARCH SINCE 1-Jan-2026\r\n",
            "a7 STATUS INBOX (MESSAGES UIDNEXT)\r\n",
            "a8 NOOP\r\n",
            "a9 IDLE\r\n",
            "DONE\r\n",
            "b1 LOGOUT\r\n",
        ];

        let (outcome, written) = guarded(&conversation).await;

        assert!(outcome.is_ok(), "a read-only conversation was refused");
        assert_eq!(written, conversation.concat());
    }

    #[tokio::test]
    async fn a_command_that_changes_the_server_never_leaves_the_machine() {
        for command in [
            "a1 STORE 1 +FLAGS (\\Deleted)\r\n",
            "a1 UID STORE 42 +FLAGS (\\Seen)\r\n",
            "a1 uid store 42 +flags (\\seen)\r\n",
            "a1 APPEND INBOX {310}\r\n",
            "a1 EXPUNGE\r\n",
            "a1 UID EXPUNGE 42\r\n",
            "a1 UID MOVE 42 Trash\r\n",
            "a1 UID COPY 42 Archive\r\n",
            "a1 CREATE Holiday\r\n",
            "a1 DELETE INBOX\r\n",
            "a1 RENAME INBOX Gone\r\n",
            "a1 CLOSE\r\n",
        ] {
            let (outcome, written) = guarded(&[command]).await;

            let refused = outcome.expect_err(&format!("{command:?} was allowed through"));
            assert_eq!(refused.kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(written, "", "{command:?} reached the server in part");
        }
    }

    #[tokio::test]
    async fn a_mailbox_read_before_it_is_refused_still_stands() {
        // The state that matters: a refusal must be about the line being written, not about the
        // session. Everything sent before it went to the server and is not taken back.
        let (outcome, written) = guarded(&[
            "a1 EXAMINE INBOX\r\n",
            "a2 UID FETCH 1:5 (FLAGS)\r\n",
            "a3 UID STORE 5 +FLAGS (\\Deleted)\r\n",
        ])
        .await;

        assert!(outcome.is_err(), "the store was allowed after two reads");
        assert_eq!(written, "a1 EXAMINE INBOX\r\na2 UID FETCH 1:5 (FLAGS)\r\n");
    }

    #[tokio::test]
    async fn a_command_split_across_writes_is_still_recognised() {
        // async-imap writes a command in pieces. A guard that only looked at whole buffers would
        // see "a1 UID ST" and "ORE 1 +FLAGS" and let both past.
        let (outcome, written) = guarded(&["a1 UID ST", "ORE 1 +FLAGS (\\Deleted)\r\n"]).await;

        assert_eq!(
            outcome
                .expect_err("a store split in two was allowed")
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        // The nine bytes of the first piece did leave, and they are not a command: a server acts
        // on a line, and this line never ends. What must never get through is the whole of it.
        assert!(
            !written.contains('\n'),
            "a complete store command reached the server: {written:?}"
        );
        assert_eq!(refuses(written.as_bytes(), Purpose::Reading), None);
    }

    /// A sink that takes a few bytes at a time, the way a socket does when it is busy.
    #[derive(Default)]
    struct Trickle {
        taken: Vec<u8>,
    }

    impl AsyncWrite for Trickle {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            let take = buffer.len().min(5);
            self.get_mut().taken.extend_from_slice(&buffer[..take]);
            Poll::Ready(Ok(take))
        }

        fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn a_socket_that_takes_a_few_bytes_at_a_time_changes_nothing() {
        // A stream is free to take part of a write and ask for the rest. The reading may only move
        // on by what was actually taken: moved on by the whole buffer, it would then read the tail
        // a second time, out of step with the line, and start finding commands in arguments — this
        // password among them.
        let mut stream = ReadOnly::with(Trickle::default(), Purpose::Reading);

        stream
            .write_all(b"a1 LOGIN person STORE\r\n")
            .await
            .expect("a login is not a store, whatever the password is");
        stream
            .write_all(b"a2 UID FETCH 1:* (FLAGS)\r\n")
            .await
            .expect("and the fetch after it still goes");

        assert_eq!(
            String::from_utf8(stream.inner.taken).expect("text"),
            "a1 LOGIN person STORE\r\na2 UID FETCH 1:* (FLAGS)\r\n"
        );
    }

    #[tokio::test]
    async fn a_reading_command_split_across_writes_goes_through() {
        let (outcome, written) = guarded(&["a1 UID FE", "TCH 1:* (FLAGS)\r\n"]).await;

        assert!(outcome.is_ok(), "a fetch in two pieces was refused");
        assert_eq!(written, "a1 UID FETCH 1:* (FLAGS)\r\n");
    }

    #[tokio::test]
    async fn an_argument_that_reads_like_a_command_is_not_one() {
        // A search for the word, and a password that happens to be it. Judging anything past the
        // command word would refuse both — and would mean reading the password.
        let (outcome, written) = guarded(&[
            "a1 UID SEARCH SUBJECT \"APPEND\"\r\n",
            "a2 LOGIN person EXPUNGE\r\n",
        ])
        .await;

        assert!(outcome.is_ok(), "an argument was mistaken for a command");
        assert_eq!(
            written,
            "a1 UID SEARCH SUBJECT \"APPEND\"\r\na2 LOGIN person EXPUNGE\r\n"
        );
    }

    #[tokio::test]
    async fn what_is_refused_is_said_without_repeating_the_line() {
        // The line holds a password on a login and a message on an append. The refusal names the
        // command and stops there, because it is written into a log.
        let (outcome, _) = guarded(&["a1 APPEND INBOX {310}\r\nFrom: someone\r\n"]).await;

        let refused = outcome.expect_err("an append was allowed").to_string();

        assert!(
            refused.contains("APPEND"),
            "the refusal does not say what: {refused}"
        );
        assert!(
            !refused.contains("INBOX"),
            "the refusal repeats the line: {refused}"
        );
        assert!(
            !refused.contains("310"),
            "the refusal repeats the line: {refused}"
        );
    }

    #[tokio::test]
    async fn a_connection_opened_to_act_sends_the_four_things_it_is_for() {
        let conversation = [
            "a1 SELECT INBOX\r\n",
            "a2 UID STORE 42 +FLAGS (\\Seen)\r\n",
            "a3 UID STORE 42 -FLAGS (\\Seen)\r\n",
            "a4 UID MOVE 42 Archive\r\n",
            "a5 UID COPY 42 Trash\r\n",
            "a6 UID EXPUNGE 42\r\n",
            "a7 LOGOUT\r\n",
        ];

        let (outcome, written) = acting(&conversation).await;

        assert!(outcome.is_ok(), "an acting connection was refused its work");
        assert_eq!(written, conversation.concat());
    }

    #[tokio::test]
    async fn even_a_connection_opened_to_act_may_not_name_a_message_by_its_position() {
        // A bare `STORE 3` names the third message *right now*, and a message expunged by another
        // client a moment ago shifts every number after it. There is no version of this that is
        // safe, so there is no connection on which it is allowed.
        for command in [
            "a1 STORE 3 +FLAGS (\\Deleted)\r\n",
            "a1 MOVE 3 Trash\r\n",
            "a1 COPY 3 Archive\r\n",
        ] {
            let (outcome, written) = acting(&[command]).await;

            assert_eq!(
                outcome
                    .expect_err(&format!("{command:?} was allowed on an acting connection"))
                    .kind(),
                io::ErrorKind::PermissionDenied
            );
            assert_eq!(written, "");
        }
    }

    #[tokio::test]
    async fn a_bare_expunge_is_refused_on_every_connection_there_is() {
        // It removes every message in the mailbox marked `\Deleted`, including ones another client
        // marked and has not finished with. `UID EXPUNGE` removes the ones named and nothing else.
        for purpose in [Purpose::Reading, Purpose::Acting] {
            let (outcome, written) = written_through(&["a1 EXPUNGE\r\n"], purpose).await;

            assert_eq!(
                outcome
                    .expect_err(&format!("a bare expunge was allowed on {purpose:?}"))
                    .kind(),
                io::ErrorKind::PermissionDenied
            );
            assert_eq!(written, "");
        }
    }

    #[tokio::test]
    async fn nothing_may_make_unmake_or_rename_a_mailbox_whatever_it_was_opened_for() {
        // Not one of these is something this application does, on any connection, in any build.
        for command in [
            "a1 CREATE Holiday\r\n",
            "a1 DELETE INBOX\r\n",
            "a1 RENAME INBOX Gone\r\n",
            "a1 SETACL INBOX anyone lr\r\n",
            "a1 DELETEACL INBOX anyone\r\n",
            "a1 SETQUOTA \"\" (STORAGE 512)\r\n",
            "a1 SETMETADATA INBOX (/private/comment \"x\")\r\n",
            "a1 SETANNOTATION INBOX \"/comment\" (\"value.priv\" \"x\")\r\n",
            "a1 CLOSE\r\n",
        ] {
            for purpose in [Purpose::Reading, Purpose::Acting] {
                let (outcome, written) = written_through(&[command], purpose).await;

                assert_eq!(
                    outcome
                        .expect_err(&format!("{command:?} was allowed on {purpose:?}"))
                        .kind(),
                    io::ErrorKind::PermissionDenied
                );
                assert_eq!(written, "");
            }
        }
    }

    #[tokio::test]
    async fn nothing_may_replace_a_message_whatever_it_was_opened_for_or_names_it_by() {
        // One command that adds a message and expunges the one it names. An acting connection
        // adds by `APPEND` and removes by `UID EXPUNGE`, each a line of its own that the guard
        // reads; a `REPLACE` does both under a word that is neither.
        for command in [
            "a1 REPLACE 3 Sent {310}\r\n",
            "a1 UID REPLACE 42 Sent {310}\r\n",
        ] {
            for purpose in [Purpose::Reading, Purpose::Acting] {
                let (outcome, written) = written_through(&[command], purpose).await;

                assert_eq!(
                    outcome
                        .expect_err(&format!("{command:?} was allowed on {purpose:?}"))
                        .kind(),
                    io::ErrorKind::PermissionDenied
                );
                assert_eq!(written, "");
            }
        }
    }

    #[tokio::test]
    async fn adding_a_message_is_for_an_acting_connection_and_no_other() {
        // The one command an acting connection may send that names no message. A pass reads, and
        // a pass that could add mail to somebody's mailbox is a pass that can do it by accident.
        let command = "a1 APPEND \"Sent\" {310}\r\n";

        let (refused, written) = guarded(&[command]).await;
        assert_eq!(
            refused
                .expect_err("adding a message was allowed while reading")
                .kind(),
            io::ErrorKind::PermissionDenied
        );
        assert_eq!(written, "");

        let (allowed, written) = written_through(&[command], Purpose::Acting).await;
        assert!(
            allowed.is_ok(),
            "an acting connection was refused the one thing it adds"
        );
        assert_eq!(written, command);
    }

    #[tokio::test]
    async fn a_connection_opened_to_read_opens_a_mailbox_only_to_read_it() {
        // In a mailbox opened for writing, a fetch of a whole message marks it read, and what asks
        // for that is an argument the guard never reads. Nothing fetched from a mailbox opened with
        // `EXAMINE` changes it, so that is the one way a connection that reads opens one, whatever
        // it asks for on the way in.
        for command in [
            "a1 SELECT INBOX\r\n",
            "a1 select inbox\r\n",
            "a1 SELECT INBOX (CONDSTORE)\r\n",
        ] {
            let (outcome, written) = guarded(&[command]).await;

            assert_eq!(
                outcome
                    .expect_err(&format!("{command:?} was allowed while reading"))
                    .kind(),
                io::ErrorKind::PermissionDenied
            );
            assert_eq!(written, "");
        }
    }

    #[tokio::test]
    async fn a_connection_opened_to_read_still_refuses_the_four() {
        // The four are what an acting connection is *for*. A pass is not one, and a pass is where
        // an accident happens.
        for command in [
            "a1 UID STORE 42 +FLAGS (\\Seen)\r\n",
            "a1 UID MOVE 42 Trash\r\n",
            "a1 UID COPY 42 Archive\r\n",
            "a1 UID EXPUNGE 42\r\n",
        ] {
            let (outcome, written) = guarded(&[command]).await;

            assert_eq!(
                outcome
                    .expect_err(&format!("{command:?} was allowed while reading"))
                    .kind(),
                io::ErrorKind::PermissionDenied
            );
            assert_eq!(written, "");
        }
    }

    /// A server for a conversation in both directions: what it is to say is put there by the test
    /// when the moment comes, and what it is sent is kept. A socket that takes only a few bytes
    /// at a time is the same server with `takes_at_most` set.
    #[derive(Default)]
    struct Talking {
        to_say: std::collections::VecDeque<u8>,
        taken: Vec<u8>,
        takes_at_most: Option<usize>,
    }

    impl AsyncRead for Talking {
        fn poll_read(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let server = self.get_mut();
            while buffer.remaining() > 0 {
                let Some(byte) = server.to_say.pop_front() else {
                    break;
                };
                buffer.put_slice(&[byte]);
            }
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for Talking {
        fn poll_write(
            self: Pin<&mut Self>,
            _context: &mut Context<'_>,
            buffer: &[u8],
        ) -> Poll<io::Result<usize>> {
            let server = self.get_mut();
            let take = buffer.len().min(server.takes_at_most.unwrap_or(usize::MAX));
            server.taken.extend_from_slice(&buffer[..take]);
            Poll::Ready(Ok(take))
        }

        fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    /// The server says `line`, and it is read through the guard, as the client library reads it.
    async fn the_server_says(stream: &mut ReadOnly<Talking>, line: &str) {
        use tokio::io::AsyncReadExt;

        stream.inner.to_say.extend(line.as_bytes());
        let mut heard = vec![0; line.len()];
        stream
            .read_exact(&mut heard)
            .await
            .expect("what the server said is read");
    }

    /// A message somebody sent, with lines in it that read like commands to anything judging
    /// them: a subject that starts `Move`, a sentence that starts `Please delete`, and two lines
    /// quoting a conversation with a mail server.
    const WRITTEN: &str = "From: zeta@example.com\r\n\
                           Subject: Move the meeting\r\n\
                           \r\n\
                           Please delete it\r\n\
                           b1 EXPUNGE\r\n\
                           b2 STORE 1 +FLAGS (\\Deleted)\r\n";

    /// The line that adds `WRITTEN`, announcing its length as `{N}` or, with `at_once`, `{N+}`.
    fn adding(at_once: bool) -> String {
        format!(
            "a1 APPEND \"Sent\" (\\Seen) {{{}{}}}\r\n",
            WRITTEN.len(),
            if at_once { "+" } else { "" }
        )
    }

    #[tokio::test]
    async fn the_message_an_append_carries_is_passed_on_unread() {
        // Judged as commands, the lines of a message refuse it halfway into the folder for sent
        // mail — a subject beginning "Move" is a refused MOVE — and the queue of copies to file
        // stops at that message for good.
        let mut stream = ReadOnly::acting(Talking::default());

        stream
            .write_all(adding(false).as_bytes())
            .await
            .expect("adding a message is what an acting connection may do");
        the_server_says(&mut stream, "+ go ahead\r\n").await;
        stream
            .write_all(WRITTEN.as_bytes())
            .await
            .expect("the message is not a command, whatever its lines say");
        stream.write_all(b"\r\n").await.expect("the line ends");
        the_server_says(&mut stream, "a1 OK APPEND completed\r\n").await;
        stream
            .write_all(b"a2 LOGOUT\r\n")
            .await
            .expect("and the conversation goes on");

        assert_eq!(
            String::from_utf8(stream.inner.taken).expect("text"),
            format!("{}{WRITTEN}\r\na2 LOGOUT\r\n", adding(false))
        );
    }

    #[tokio::test]
    async fn what_is_written_after_the_message_is_judged_again() {
        // Exactly as many bytes as the line announced, and not one more: the next command is a
        // command, whether it comes in a write of its own or in the same one as the message's end.
        for (after, refused) in [
            ("a2 STORE 1 +FLAGS (\\Deleted)\r\n", "STORE"),
            ("a2 EXPUNGE\r\n", "EXPUNGE"),
            ("a2 DELETE INBOX\r\n", "DELETE"),
        ] {
            let mut stream = ReadOnly::acting(Talking::default());
            stream
                .write_all(adding(false).as_bytes())
                .await
                .expect("announced");
            the_server_says(&mut stream, "+ go ahead\r\n").await;
            stream
                .write_all(WRITTEN.as_bytes())
                .await
                .expect("the message");
            let sent_so_far = stream.inner.taken.len();

            let outcome = stream
                .write_all(format!("\r\n{after}").as_bytes())
                .await
                .expect_err(&format!("{after:?} after a message was let through"));

            assert_eq!(outcome.kind(), io::ErrorKind::PermissionDenied);
            assert!(outcome.to_string().contains(refused), "{outcome}");
            assert_eq!(
                stream.inner.taken.len(),
                sent_so_far,
                "part of {after:?} reached the server"
            );
        }

        // The last bytes of the message and the command after it, in one write.
        let mut stream = ReadOnly::acting(Talking::default());
        stream
            .write_all(adding(false).as_bytes())
            .await
            .expect("announced");
        the_server_says(&mut stream, "+ go ahead\r\n").await;
        let (most, last) = WRITTEN.split_at(WRITTEN.len() - 3);
        stream.write_all(most.as_bytes()).await.expect("most of it");

        let outcome = stream
            .write_all(format!("{last}\r\na2 EXPUNGE\r\n").as_bytes())
            .await
            .expect_err("a command sharing a write with the end of a message was let through");
        assert_eq!(outcome.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn a_message_the_server_did_not_ask_for_is_never_passed_unread() {
        // The server answers the announcement with something other than "go on" — a refusal, or
        // a line about the mailbox — and the client library gives the message up. Whatever is
        // written next is a command, and it is judged as one.
        for answer in [
            "a1 NO [TRYCREATE] No such mailbox\r\n",
            "a1 BAD Invalid arguments\r\n",
            "* 4 EXISTS\r\n",
        ] {
            let mut stream = ReadOnly::acting(Talking::default());
            stream
                .write_all(adding(false).as_bytes())
                .await
                .expect("announced");
            the_server_says(&mut stream, answer).await;

            let outcome = stream
                .write_all(b"a2 EXPUNGE\r\n")
                .await
                .expect_err(&format!("an expunge after {answer:?} was let through"));

            assert_eq!(outcome.kind(), io::ErrorKind::PermissionDenied);
            assert_eq!(stream.inner.taken, adding(false).as_bytes());
        }
    }

    #[tokio::test]
    async fn nothing_written_before_the_server_says_to_go_on_is_the_message() {
        // A message announced as `{N}` waits for the server. Bytes written before it answers are
        // not the message, and what they say is judged.
        let mut stream = ReadOnly::acting(Talking::default());
        stream
            .write_all(adding(false).as_bytes())
            .await
            .expect("announced");

        let outcome = stream
            .write_all(b"a2 EXPUNGE\r\n")
            .await
            .expect_err("a command written before the server answered was let through");
        assert_eq!(outcome.kind(), io::ErrorKind::PermissionDenied);

        // Nor does the answer arriving afterwards make the next thing written a message.
        the_server_says(&mut stream, "+ go ahead\r\n").await;
        assert!(stream.write_all(b"a2 EXPUNGE\r\n").await.is_err());
    }

    #[tokio::test]
    async fn a_message_announced_not_to_wait_for_the_server_is_read_like_any_other_line() {
        // `{N+}` is only the client's word that a message follows. A server that does not take
        // that form answers the line `BAD` and runs the next N bytes as commands — here, exactly
        // the bytes of a deletion — so they are judged as commands, and nothing of them leaves.
        let deletion = "a2 DELETE INBOX\r\n";
        let announcing = format!("a1 APPEND \"Sent\" {{{}+}}\r\n", deletion.len());
        let mut stream = ReadOnly::acting(Talking::default());

        stream
            .write_all(announcing.as_bytes())
            .await
            .expect("adding a message is what an acting connection may do");
        let outcome = stream
            .write_all(deletion.as_bytes())
            .await
            .expect_err("what followed `{N+}` passed unread");

        assert_eq!(outcome.kind(), io::ErrorKind::PermissionDenied);
        assert!(outcome.to_string().contains("DELETE"), "{outcome}");
        assert_eq!(stream.inner.taken, announcing.as_bytes());

        // A message of somebody's writing announced the same way is refused at its first line
        // that reads like a command, which is the price of a form nothing here writes.
        let mut stream = ReadOnly::acting(Talking::default());
        stream
            .write_all(adding(true).as_bytes())
            .await
            .expect("announced");

        assert!(stream.write_all(WRITTEN.as_bytes()).await.is_err());
    }

    #[tokio::test]
    async fn the_count_survives_a_socket_that_takes_a_few_bytes_at_a_time() {
        // The count moves by what the socket took, as the line does: moved by what was offered,
        // it would end early or late, and a late end passes the next command unread.
        let mut stream = ReadOnly::acting(Talking {
            takes_at_most: Some(5),
            ..Talking::default()
        });

        stream
            .write_all(adding(false).as_bytes())
            .await
            .expect("announced");
        the_server_says(&mut stream, "+ go ahead\r\n").await;
        stream
            .write_all(WRITTEN.as_bytes())
            .await
            .expect("the message, five bytes at a time");
        stream.write_all(b"\r\n").await.expect("the line ends");

        assert!(stream.write_all(b"a2 EXPUNGE\r\n").await.is_err());
        assert_eq!(
            String::from_utf8(stream.inner.taken).expect("text"),
            format!("{}{WRITTEN}\r\n", adding(false))
        );
    }

    #[tokio::test]
    async fn a_connection_opened_to_read_adds_no_message_and_passes_none() {
        // Refused at its line, so there is no message to pass: nothing of the line and nothing of
        // what follows it reaches the server.
        let mut stream = ReadOnly::guarding(Talking::default());

        let outcome = stream
            .write_all(format!("{}{WRITTEN}\r\n", adding(true)).as_bytes())
            .await
            .expect_err("adding a message was allowed while reading");

        assert_eq!(outcome.kind(), io::ErrorKind::PermissionDenied);
        assert!(stream.inner.taken.is_empty());
    }

    #[test]
    fn only_the_end_of_the_line_says_how_long_the_message_is() {
        assert_eq!(announced(b"{310}\r"), Message::Announced(310));
        assert_eq!(announced(b"(\\Seen) {310}\r"), Message::Announced(310));
        assert_eq!(announced(b"{310}"), Message::Announced(310));

        for nothing in [
            &b"\"Sent\" (\\Seen)\r"[..],
            b"{310+}\r",
            b"{310} \r",
            b"{}\r",
            b"{+}\r",
            b"{0}\r",
            b"{3 10}\r",
            b"{-310}\r",
            b"{99999999999999999999999}\r",
        ] {
            assert_eq!(
                announced(nothing),
                Message::None,
                "{:?} announced a message",
                String::from_utf8_lossy(nothing)
            );
        }
    }

    #[test]
    fn a_line_that_is_not_a_command_is_not_judged() {
        // Continuation data and the end of an idle are single words with no tag before them.
        assert_eq!(refuses(b"DONE\r", Purpose::Reading), None);
        assert_eq!(refuses(b"+ ready\r", Purpose::Reading), None);
        assert_eq!(refuses(b"", Purpose::Reading), None);
    }

    #[test]
    fn a_line_too_long_to_be_a_command_is_judged_by_its_beginning() {
        // Nothing after the first word or two decides anything, so a very long line is not held in
        // memory: it is a message on its way to the server.
        let long = format!("a1 STORE {}\r", "x".repeat(4_096));

        assert_eq!(
            refuses(long.as_bytes(), Purpose::Reading).as_deref(),
            Some("STORE")
        );
    }
}
