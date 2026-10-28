//! The IMAP adapter.
//!
//! Reading only. No command that changes anything on the server appears here: no STORE, no MOVE,
//! no EXPUNGE, no APPEND. Sending one from here would make "debugging the synchronisation" and
//! "losing the mailbox" the same action.
//!
//! The commands that do change a mailbox live in `actions.rs` and are sent on a connection the
//! stream guard opened for that purpose: the four UID forms, which get there only through the
//! operation log — which knows how to try again and what to show a person when it cannot — and
//! `APPEND`, which files the copy of a message the outbox has handed to a submission server.

// Reached from the composition root once an account can be added. Until then the module is
// exercised by its tests, including two that talk to the development server.
#![allow(dead_code)]

pub(crate) mod answer;
mod envelope;
pub(crate) mod pass;

use answer::{answer_to, IfRefused};
pub(crate) use envelope::described_whole;
pub(crate) use pass::one_pass;

pub mod actions;
pub mod read_only;

use std::ops::RangeInclusive;
use std::time::Duration;

use async_imap::imap_proto::{Capability, Response, Status};
use async_imap::types::{NameAttribute, UnsolicitedResponse};
use mail_parser::decoders::base64::base64_decode;
use mail_parser::decoders::quoted_printable::quoted_printable_decode;
use mail_parser::{Address, HeaderValue, Message, MessageParser};
use secrecy::ExposeSecret;
use tokio::runtime::Handle;

use crate::infrastructure::net::{
    self, Deadline, Endpoint, Silence, Stream, TransportError, TransportPolicy,
};
use crate::ports::credential::Credential;
use crate::ports::mail_provider::{
    DiscoveredMailbox, MailFetch, MailProviderError, MailSync, MailWatch, MailboxDiscovery,
    ProviderFactory, SyncIntent, SyncRequest, Unread, WatchEnded, Watched, Watermark,
};
use lyonoyl_domain::account::{Encryption, MailAccount};
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::message::{FlagScope, Flags, ModSeq, Rfc822MessageId, Uid, UidValidity};
use lyonoyl_domain::sync::{
    ArrivedMessage, FlagUpdate, FolderDelta, MessageEnvelope, SyncBatch, SyncCursor, SyncScope,
    VanishedReport,
};
use lyonoyl_domain::xoauth2;

/// What is needed to open a session. The password is only ever read at the moment it is sent.
pub struct ImapAccount {
    pub endpoint: Endpoint,
    pub username: String,
    /// A password or a token. Which it is decides the command that opens the session, and that is
    /// the only place in this adapter where the difference exists.
    pub credential: Credential,
}

use read_only::ReadOnly;

/// Every session writes through the read-only guard, so a command that would change a mailbox
/// cannot reach the server whatever issues it.
type Session = async_imap::Session<Deadline<ReadOnly<Stream>>>;

/// A provider backed by an IMAP server.
///
/// Each call opens its own session and closes it. Keeping a connection alive between calls is worth
/// doing later, and needs a way to notice a server that dropped it; a session per call is correct
/// first.
///
/// The runtime is shared, not owned: an account is one of many, and a hundred accounts must not
/// mean a hundred runtimes with their own threads.
/// A session kept open between calls, and what was settled when it was opened.
struct Held {
    session: Session,
    /// Whether the server took `ENABLE QRESYNC`. Settled once, before any mailbox was opened,
    /// because that is the only state the command is valid in.
    qresync: bool,
}

pub struct ImapProvider {
    /// The session this provider keeps between calls, once it has one.
    ///
    /// One provider is one pass over one account, so this is a session per pass rather than a
    /// pool: it is opened on the first call and closed when the provider goes.
    held: std::sync::Mutex<Option<Held>>,
    /// Whether something on this provider has failed, which gives the sessions it opens after
    /// that the patient deadline rather than the short one. One provider is one pass, so the
    /// patience lasts as long as the pass and the next pass starts short again.
    patient: std::sync::atomic::AtomicBool,
    account: ImapAccount,
    policy: TransportPolicy,
    runtime: Handle,
}

impl ImapProvider {
    pub fn new(account: ImapAccount, runtime: Handle) -> Result<Self, MailProviderError> {
        let policy = TransportPolicy::new().map_err(transport_failed)?;

        Ok(Self {
            account,
            policy,
            runtime,
            held: std::sync::Mutex::new(None),
            patient: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// Runs `work` on this provider's session, opening one if it has none.
    ///
    /// **The session stays open afterwards.** A pass over an account lists its mailboxes and then
    /// reads six of them, and every one of those was its own connection and its own sign-in: with
    /// a server a second away that was seven seconds of a twelve-second pass spent saying hello.
    ///
    /// A failure throws the session away rather than handing it back. Half a conversation is worse
    /// than none — the next call would send a command into a stream the server has given up on —
    /// and one reopen is what it costs.
    async fn on_the_session<T, F>(&self, work: F) -> Result<T, MailProviderError>
    where
        F: AsyncFnOnce(&mut Session, bool) -> Result<T, MailProviderError>,
    {
        self.opened_then(work).await?
    }

    /// `on_the_session`, saying apart a session that could not be opened — the outer failure —
    /// from work that failed on one that was.
    async fn opened_then<T, E, F>(&self, work: F) -> Result<Result<T, E>, MailProviderError>
    where
        F: AsyncFnOnce(&mut Session, bool) -> Result<T, E>,
    {
        // Taken out rather than borrowed: nothing may hold this lock across an await, and the
        // work below is entirely awaits.
        let taken = self.held.lock().expect("the lock is never poisoned").take();
        let mut held = match taken {
            Some(held) => held,
            None => {
                let mut session = match self.open().await {
                    Ok(session) => session,
                    Err(failed) => {
                        self.become_patient();
                        return Err(failed);
                    }
                };
                // Once, here, before any mailbox is opened — the only state `ENABLE` is valid in.
                // A refusal is not a failure: the pass then finds out what disappeared by counting
                // instead of by being told.
                let qresync = pass::enable_qresync(&mut session).await.unwrap_or(false);

                Held { session, qresync }
            }
        };

        let done = work(&mut held.session, held.qresync).await;

        // Dropped rather than signed out of. Signing out first reads to the end of whatever the
        // server is still saying, and among the ways a call fails are a server that went quiet and
        // one that said more than an answer may hold: waiting on the first again, or reading the
        // rest of the second, is what failing was for. Closing the connection is an answer every
        // server understands.
        match done.is_ok() {
            true => *self.held.lock().expect("the lock is never poisoned") = Some(held),
            false => self.become_patient(),
        }

        Ok(done)
    }

    /// Gives every session opened from now on the patient deadline.
    fn become_patient(&self) {
        self.patient
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// How long the server may stay silent on the next session this provider opens.
    fn the_silence(&self) -> std::time::Duration {
        match self.patient.load(std::sync::atomic::Ordering::Relaxed) {
            false => self.policy.read_timeout(),
            true => self.policy.patient_read_timeout(),
        }
    }

    async fn open(&self) -> Result<Session, MailProviderError> {
        self.open_allowing(Silence::of(self.the_silence())).await
    }

    /// Opens a session whose deadline reads what `silence` allows, so the caller holding it can
    /// allow more for a stretch that is meant to be quiet.
    async fn open_allowing(&self, silence: Silence) -> Result<Session, MailProviderError> {
        // Counted and timed, because the cost of a round is mostly this line. A number nobody can
        // see is how one connection per message lived here unnoticed for as long as it did: every
        // measure said "a pass finished" and none said what the pass spent.
        let began = std::time::Instant::now();

        let stream = self
            .policy
            .connect_retrying(&self.account.endpoint)
            .await
            .map_err(transport_failed)?;

        // Wrapped so every command this session sends has a deadline, including the ones
        // nobody has written yet. A deadline remembered per command is a deadline that will be
        // forgotten the next time a command is added.
        let client = async_imap::Client::new(Deadline::over(ReadOnly::guarding(stream), silence));

        let session = open_session(client, &self.account).await;
        opening_took(began.elapsed());

        session
    }
}

thread_local! {
    /// What this thread has spent on opening sessions: how many, and how long.
    ///
    /// **Per thread, and that is the whole point.** A count across the process attributes every
    /// other account's connections to whichever pass happens to be running, and with nine accounts
    /// read side by side and eight more holding a watch, every pass reports the whole round's
    /// total. Measured that way, one pass over six folders appeared to open sixty-three sessions;
    /// it opens seven, and sixty-three was nine accounts times seven.
    ///
    /// A pass is one blocking thread and every session it opens is opened on that thread, so this
    /// is exactly the work asked about and nothing else.
    static OPENED_HERE: std::cell::Cell<(u64, std::time::Duration)> =
        const { std::cell::Cell::new((0, std::time::Duration::ZERO)) };
}

/// How many sessions this thread has opened, and how long it spent doing it.
///
/// Only ever goes up, so what a stretch of work cost is the difference across it.
pub fn sessions_opened_here() -> (u64, std::time::Duration) {
    OPENED_HERE.with(std::cell::Cell::get)
}

fn opening_took(how_long: std::time::Duration) {
    OPENED_HERE.with(|held| {
        let (count, spent) = held.get();
        held.set((count + 1, spent + how_long));
    });
}

impl MailboxDiscovery for ImapProvider {
    fn list_mailboxes(&self) -> Result<Vec<DiscoveredMailbox>, MailProviderError> {
        self.runtime
            .block_on(self.on_the_session(async |session, _qresync| list_on(session).await))
    }
}

/// The listing itself, on a session that is already open: every mailbox that can be opened.
///
/// Whole or not at all, read by [`answer::mailboxes_listed`]: a mailbox missing from a listing cut
/// short is read by the rest of a pass as a mailbox the account no longer has.
///
/// Generic over the stream for the same reason the pass and the watch are — a scripted server can
/// drive it.
pub(crate) async fn list_on<S>(
    session: &mut async_imap::Session<S>,
) -> Result<Vec<DiscoveredMailbox>, MailProviderError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    Ok(
        answer::mailboxes_listed(session, answer::MOST_A_PASS_ANSWER_HOLDS)
            .await?
            .into_iter()
            .filter(|listed| selectable(&listed.attributes))
            .map(|listed| DiscoveredMailbox {
                role: role_of(&listed.attributes),
                name: listed.name,
                // LIST does not carry them; a STATUS or a SELECT does.
                uid_validity: None,
                uid_next: None,
            })
            .collect(),
    )
}

#[cfg(test)]
mod what_a_pass_reads {
    use super::envelope::PREVIEW_BYTES;
    use super::pass::PASS_QUERY;

    /// A pass asks for exactly the bytes the parser is allowed to be given.
    ///
    /// The two are eight lines apart and were kept equal by nothing. The bound is the whole reason
    /// this path may parse markup at all: it is what the comment on `preview_of` rests on, and a
    /// query widened without the constant beside it would hand a whole message to a parser running
    /// with none of the guards the reading path has.
    #[test]
    fn asks_for_the_bytes_the_parser_is_bounded_to() {
        assert!(
            PASS_QUERY.contains(&format!("<0.{PREVIEW_BYTES}>")),
            "the pass query and the bound it rests on have drifted apart: {PASS_QUERY}"
        );
    }
}

impl MailSync for ImapProvider {
    /// IMAP keeps flags per mailbox: the same message in two folders may be read in one and unread
    /// in the other, and both are true.
    fn flag_scope(&self) -> FlagScope {
        FlagScope::Placement
    }

    fn fetch(&self, request: &SyncRequest) -> Result<SyncBatch, MailProviderError> {
        self.runtime.block_on(
            self.on_the_session(async |session, qresync| one_pass(session, request, qresync).await),
        )
    }
}

impl MailFetch for ImapProvider {
    fn whole_message(
        &self,
        mailbox: &str,
        uid_validity: UidValidity,
        uid: Uid,
    ) -> Result<Vec<u8>, MailProviderError> {
        self.runtime
            .block_on(self.on_the_session(async |session, _qresync| {
                whole_message_on(session, mailbox, uid_validity, uid).await
            }))
    }

    fn whole_messages(
        &self,
        mailbox: &str,
        uid_validity: UidValidity,
        uids: &[Uid],
    ) -> Result<Vec<(Uid, Vec<u8>)>, Unread> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }

        // A session that could not be opened asked nothing.
        self.runtime
            .block_on(self.opened_then(async |session, _qresync| {
                whole_messages_on(session, mailbox, uid_validity, uids).await
            }))
            .map_err(Unread::NotAsked)?
    }
}

/// Opens a mailbox to read by numbers stored under `uid_validity`, and refuses when the server
/// counts in any other generation.
///
/// A folder that renumbered has handed the same numbers to different messages, so a number from
/// before it reads somebody else's message — and that message would be kept as this one's body,
/// opened under this one's subject, and found by searching for what this one says. A server that
/// states no generation at all is one nothing can be checked against, and is refused the same way.
async fn examined_under<S>(
    session: &mut async_imap::Session<S>,
    mailbox: &str,
    uid_validity: UidValidity,
) -> Result<(), MailProviderError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let opened = session.examine(mailbox).await.map_err(protocol_failed)?;

    match opened.uid_validity == Some(uid_validity.get()) {
        true => Ok(()),
        false => Err(MailProviderError::Protocol),
    }
}

/// Several messages of one mailbox, on one session, in one request.
///
/// One `EXAMINE` and one `UID FETCH` naming every number, rather than a pair of them per message.
/// The saving is the round trips: what a message costs is mostly not the message.
///
/// A number the server says nothing about is simply absent from the answer, and so is one it
/// refuses with `NO` after answering with the rest: what arrived is kept rather than thrown away
/// with it. A server may instead end the conversation at a message it cannot read — Dovecot does
/// by default — and then nothing past that message arrives and the request fails whole, as
/// [`Unread::Failed`]: asked, and failed over something in it. A mailbox that would not open
/// under the generation the numbers were stored in is [`Unread::NotAsked`], because the question
/// about the messages never went out. `BODY.PEEK[]` for the same reason it is used below — reading
/// a message here must not be what marks it read.
pub(crate) async fn whole_messages_on<S>(
    session: &mut async_imap::Session<S>,
    mailbox: &str,
    uid_validity: UidValidity,
    uids: &[Uid],
) -> Result<Vec<(Uid, Vec<u8>)>, Unread>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    examined_under(session, mailbox, uid_validity)
        .await
        .map_err(Unread::NotAsked)?;

    let asked = uids
        .iter()
        .map(|uid| uid.get().to_string())
        .collect::<Vec<_>>()
        .join(",");

    let answer = answer_to(
        session,
        &format!("UID FETCH {asked} (BODY.PEEK[])"),
        answer::most_whole_messages_hold(uids.len()),
        IfRefused::WhatArrived,
    )
    .await
    .map_err(Unread::Failed)?;

    // Only the numbers that were asked for. A server answering about another message is answering
    // about another message, and pairing it to one of these by position would put somebody else's
    // mail under a subject that is not theirs.
    let wanted: std::collections::HashSet<u32> = uids.iter().map(|uid| uid.get()).collect();

    Ok(answer
        .fetched
        .into_iter()
        .filter_map(|message| {
            let number = message.uid?;
            if !wanted.contains(&number) {
                return None;
            }

            Some((Uid::new(number).ok()?, message.whole?))
        })
        .collect())
}

/// One message read whole, on a session that is already open.
///
/// `BODY.PEEK[]` rather than `BODY[]`: opening a message here must not be what marks it read on the
/// server. What marks it read is a decision of its own, and it belongs to the stage that writes.
pub(crate) async fn whole_message_on<S>(
    session: &mut async_imap::Session<S>,
    mailbox: &str,
    uid_validity: UidValidity,
    uid: Uid,
) -> Result<Vec<u8>, MailProviderError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    examined_under(session, mailbox, uid_validity).await?;

    let answer = answer_to(
        session,
        &format!("UID FETCH {} (BODY.PEEK[])", uid.get()),
        answer::MOST_A_WHOLE_MESSAGE_HOLDS,
        IfRefused::WhatArrived,
    )
    .await?;

    // The one asked for, by its own number: a server answering about another message is answering
    // about another message, not about a mistake worth guessing through. Arrived whole and then
    // refused is still the message; refused before it arrived is no message at all. The line that
    // carries it, which need not be the first about it: a flag another device changed a moment ago
    // is a line about the same number, and it may come first.
    answer
        .fetched
        .into_iter()
        .filter(|message| message.uid == Some(uid.get()))
        .find_map(|message| message.whole)
        .ok_or(MailProviderError::Protocol)
}

/// The longest a single watch may last.
///
/// RFC 2177 lets a server drop an idle connection after thirty minutes, and tells clients to renew
/// before that. Renewing is one command on a connection that is already open, so the ceiling costs
/// nothing and the alternative is a watch that silently stopped watching.
const WATCH_LIMIT: Duration = Duration::from_secs(29 * 60);

impl MailWatch for ImapProvider {
    /// Holds a connection open until the server says something happened in the mailbox.
    ///
    /// A server that cannot push says so through its capabilities, and then the answer is
    /// `CannotPush` rather than an error: nothing is wrong, and the timer above covers it.
    fn watch(
        &self,
        mailbox: &str,
        since: Option<&Watermark>,
        at_most: Duration,
    ) -> Result<WatchEnded, MailProviderError> {
        // Kept here as well as in the session's deadline, so the watch can allow the idle the
        // silence an idle is for, and nothing else in the conversation more than any other.
        let silence = Silence::of(self.policy.read_timeout());

        self.runtime.block_on(async {
            let session = self.open_allowing(silence.clone()).await?;
            watch_on(session, mailbox, since, at_most, &silence).await
        })
    }
}

/// The watch itself, on a session that is already open.
///
/// Takes the session rather than borrowing it: starting an idle consumes the session and ending it
/// gives it back, which is how the protocol works and what keeps a half-ended idle from existing.
/// Generic over the stream for the same reason as the pass — a scripted server can drive it.
///
/// `silence` is what the session's deadline allows, held here so the idle can be allowed its own.
pub(crate) async fn watch_on<S>(
    mut session: async_imap::Session<S>,
    mailbox: &str,
    since: Option<&Watermark>,
    at_most: Duration,
    silence: &Silence,
) -> Result<WatchEnded, MailProviderError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let capabilities = session.capabilities().await.map_err(protocol_failed)?;
    if !capabilities.has_str("IDLE") {
        let _ = session.logout().await;
        return Ok(Watched::CannotPush.into());
    }

    // The mailbox has to be open for the server to have anything to say about it, and EXAMINE is
    // the form that changes nothing. How it stands is what says, at the end, whether anything
    // arrived.
    let opened = Stood::of(&session.examine(mailbox).await.map_err(protocol_failed)?);

    // Mail that arrived after the last watch ended was heard by nobody: it came while that watch's
    // news was being read, or while this connection was being opened. It is news now, and an idle
    // started from here would count it as already there.
    if since
        .and_then(Stood::read)
        .is_some_and(|ended| ended.arrived_by(opened))
    {
        let _ = session.logout().await;
        return Ok(WatchEnded {
            watched: Watched::Arrived,
            seen: Some(opened.mark()),
        });
    }

    // Whatever earlier commands left in the queue was said before this watch began.
    while session.unsolicited_responses.try_recv().is_ok() {}

    let mut idle = session.idle();
    idle.init().await.map_err(protocol_failed)?;

    // The one stretch of a conversation in which a server saying nothing is the answer expected,
    // so it has a wait of its own. Held to the session's deadline, a quiet minute ends the watch
    // as a failure; allowed its own length and the minute any answer is given, it ends on its own
    // clock — and that clock runs from when the idle began, so a server that says "still here"
    // every few minutes does not keep one idle going past the half hour after which a server may
    // drop it. Everything after the idle is held to the ordinary deadline again.
    let lasts = at_most.min(WATCH_LIMIT);
    let outcome = {
        let _idling = silence.for_a_while(lasts + silence.allowed());
        let (waiting, _stop) = idle.wait_with_timeout(lasts);
        tokio::time::timeout(lasts, waiting)
            .await
            .unwrap_or(Ok(async_imap::extensions::idle::IdleResponse::Timeout))
    };

    // Ending the idle before the session is dropped: a server left mid-idle keeps the connection
    // and everything behind it until it times out on its own.
    let session = idle.done().await.map_err(protocol_failed)?;

    // The idle ends on the first line it hears. What came while it was starting and while it was
    // ending waits in the queue, in order — a flag changing and mail arriving a moment later is
    // both, and only the queue has the second.
    let first = match outcome.map_err(protocol_failed)? {
        async_imap::extensions::idle::IdleResponse::NewData(data) => Some(heard_in(data.parsed())),
        _ => None,
    };
    let mut queued = Vec::new();
    while let Ok(response) = session.unsolicited_responses.try_recv() {
        queued.push(heard_unsolicited(response));
    }

    // How the mailbox stands now the idle is over, which is where the next watch takes up. Asked
    // after the queue is emptied, so that mail the server announces from here on is in this
    // answer rather than lost between two watches. A mailbox that will not say leaves the next
    // watch to start from where this one did, which at worst reads the account once more.
    let mut session = session;
    let closed = session
        .examine(mailbox)
        .await
        .ok()
        .map(|mailbox| Stood::of(&mailbox));
    let _ = session.logout().await;

    let arrived = arrived(opened.holds, first, &queued)
        || closed.is_some_and(|closed| opened.arrived_by(closed));

    Ok(WatchEnded {
        watched: if arrived {
            Watched::Arrived
        } else if first.is_some() {
            Watched::Changed
        } else {
            Watched::Quiet
        },
        seen: Some(closed.unwrap_or(opened).mark()),
    })
}

/// How a mailbox stood when a watch looked at it: its generation, the number its next message
/// would be given, and how many it held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stood {
    generation: Option<u32>,
    next: Option<u32>,
    holds: u32,
}

impl Stood {
    fn of(mailbox: &async_imap::types::Mailbox) -> Self {
        Self {
            generation: mailbox.uid_validity,
            next: mailbox.uid_next,
            holds: mailbox.exists,
        }
    }

    /// Written down for the next watch, which is the only reader.
    pub(crate) fn mark(self) -> Watermark {
        let known = |value: Option<u32>| value.map_or_else(String::new, |value| value.to_string());
        Watermark::new(format!(
            "{}/{}/{}",
            known(self.generation),
            known(self.next),
            self.holds
        ))
    }

    /// Read back from a mark, or nothing from one this adapter did not write — which leaves the
    /// watch to start from where it finds the mailbox, as a first one does.
    pub(crate) fn read(mark: &Watermark) -> Option<Self> {
        let known = |part: &str| match part {
            "" => Some(None),
            number => number.parse().ok().map(Some),
        };
        let mut parts = mark.as_str().split('/');
        let stood = Self {
            generation: known(parts.next()?)?,
            next: known(parts.next()?)?,
            holds: parts.next()?.parse().ok()?,
        };
        parts.next().is_none().then_some(stood)
    }

    /// Whether mail arrived between this and `later`.
    ///
    /// The number the next message will be given grows with every message that arrives, and with
    /// nothing else — not with one filed away, which is what a person going through their inbox
    /// does every few seconds — so it is what is compared wherever the server gives it. A count
    /// that grew is the sign only where it does not, and a message filed away hides one that
    /// arrived beside it until the next round. A mailbox renumbered in between counts as arrived:
    /// its numbers say nothing about the ones before, and reading it is how to find out.
    pub(crate) fn arrived_by(self, later: Self) -> bool {
        let renumbered = matches!(
            (self.generation, later.generation),
            (Some(before), Some(after)) if before != after
        );
        if renumbered {
            return true;
        }
        match (self.next, later.next) {
            (Some(before), Some(after)) => after > before,
            _ => later.holds > self.holds,
        }
    }
}

/// Whether mail arrived, from the line that ended a watch and the lines queued around it.
///
/// Where the first line falls among the queued ones the queue does not say, so every place it could
/// fall is tried. A pass too many is one pass; mail taken for this device's own change coming back
/// waits for the next round.
pub(crate) fn arrived(held: u32, first: Option<Heard>, queued: &[Heard]) -> bool {
    match first {
        None => grew(held, queued.iter().copied()),
        Some(first) => (0..=queued.len()).any(|at| {
            let mut heard = queued.to_vec();
            heard.insert(at, first);
            grew(held, heard)
        }),
    }
}

/// What one line a watch heard does to how many messages the mailbox holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Heard {
    /// How many it holds now.
    Holds(u32),
    /// One went.
    OneGone,
    /// Nothing about how many: a flag changed, or the server said something else.
    Other,
}

fn heard_in(response: &Response<'_>) -> Heard {
    match response {
        Response::MailboxData(async_imap::imap_proto::MailboxDatum::Exists(count)) => {
            Heard::Holds(*count)
        }
        Response::Expunge(_) => Heard::OneGone,
        _ => Heard::Other,
    }
}

fn heard_unsolicited(response: UnsolicitedResponse) -> Heard {
    match response {
        UnsolicitedResponse::Exists(count) => Heard::Holds(count),
        UnsolicitedResponse::Expunge(_) => Heard::OneGone,
        UnsolicitedResponse::Other(data) => heard_in(data.parsed()),
        _ => Heard::Other,
    }
}

/// Whether the mailbox came to hold more than it did while it was watched — mail arriving, as
/// against what was there changing.
///
/// Counted rather than read off one line: a server may say how many it holds after it says one
/// went, and a count that only went down is somebody filing mail away, this device included.
fn grew(held: u32, heard: impl IntoIterator<Item = Heard>) -> bool {
    let mut holds = held;
    for one in heard {
        match one {
            Heard::Holds(count) if count > holds => return true,
            Heard::Holds(count) => holds = count,
            Heard::OneGone => holds = holds.saturating_sub(1),
            Heard::Other => {}
        }
    }
    false
}

/// Builds IMAP providers for accounts.
///
/// One runtime serves every account: a hundred accounts must not mean a hundred sets of worker
/// threads.
pub struct ImapProviderFactory {
    runtime: Handle,
}

impl ImapProviderFactory {
    pub fn new(runtime: Handle) -> Self {
        Self { runtime }
    }
}

impl ImapProviderFactory {
    fn provider_for(
        &self,
        account: &MailAccount,
        credential: &Credential,
    ) -> Result<ImapProvider, MailProviderError> {
        ImapProvider::new(
            ImapAccount {
                endpoint: endpoint_of(account),
                username: account.imap_username().to_owned(),
                credential: credential.clone(),
            },
            self.runtime.clone(),
        )
    }
}

impl ProviderFactory for ImapProviderFactory {
    fn conversations(&self) -> (u64, std::time::Duration) {
        sessions_opened_here()
    }

    fn discovery(
        &self,
        account: &MailAccount,
        credential: &Credential,
    ) -> Result<Box<dyn MailboxDiscovery>, MailProviderError> {
        let provider = self.provider_for(account, credential)?;

        Ok(Box::new(provider))
    }

    fn sync(
        &self,
        account: &MailAccount,
        credential: &Credential,
    ) -> Result<Box<dyn MailSync>, MailProviderError> {
        Ok(Box::new(self.provider_for(account, credential)?))
    }

    fn watch(
        &self,
        account: &MailAccount,
        credential: &Credential,
    ) -> Result<Box<dyn MailWatch>, MailProviderError> {
        Ok(Box::new(self.provider_for(account, credential)?))
    }

    fn reading(
        &self,
        account: &MailAccount,
        credential: &Credential,
    ) -> Result<Box<dyn MailFetch>, MailProviderError> {
        Ok(Box::new(self.provider_for(account, credential)?))
    }
}

/// Turns the settings a person entered into an address to connect to.
/// What is needed to open a session for an account, wherever that session is opened from.
///
/// The credential is cloned rather than borrowed because it is read at the moment it is sent, and
/// only then.
pub fn imap_account(account: &MailAccount, credential: &Credential) -> ImapAccount {
    ImapAccount {
        endpoint: endpoint_of(account),
        username: account.imap_username().to_owned(),
        credential: credential.clone(),
    }
}

fn endpoint_of(account: &MailAccount) -> Endpoint {
    let settings = account.imap();

    Endpoint {
        host: settings.host().to_owned(),
        port: settings.port(),
        encryption: match settings.encryption() {
            Encryption::Tls => net::Encryption::Tls,
            Encryption::StartTls => net::Encryption::StartTls,
            Encryption::None => net::Encryption::None,
        },
    }
}

/// Reads what the server says a mailbox is for.
///
/// A server that says nothing leaves the folder as one the user made. Names are never inspected:
/// providers translate them, and the Russian for "Sent" is not a role.
fn role_of(attributes: &[NameAttribute<'_>]) -> FolderRole {
    for attribute in attributes {
        let role = match attribute {
            NameAttribute::All => FolderRole::AllMail,
            NameAttribute::Archive => FolderRole::Archive,
            NameAttribute::Drafts => FolderRole::Drafts,
            NameAttribute::Flagged => FolderRole::Flagged,
            NameAttribute::Junk => FolderRole::Junk,
            NameAttribute::Sent => FolderRole::Sent,
            NameAttribute::Trash => FolderRole::Trash,
            _ => continue,
        };
        return role;
    }
    FolderRole::Custom
}

/// Whether a name the server listed is a mailbox that can be opened.
///
/// `\Noselect` is a place in the hierarchy and not a mailbox — `[Gmail]` on every Gmail account —
/// and `\NonExistent` is a name with nothing there. Recorded, either is a folder that shows nothing
/// and that every pass opens and fails on.
fn selectable(attributes: &[NameAttribute<'_>]) -> bool {
    !attributes.iter().any(|attribute| match attribute {
        NameAttribute::NoSelect => true,
        NameAttribute::Extension(name) => name.eq_ignore_ascii_case("\\NonExistent"),
        _ => false,
    })
}

fn transport_failed(error: TransportError) -> MailProviderError {
    match error {
        TransportError::Unreachable | TransportError::Timeout => MailProviderError::Offline,
        TransportError::Tls
        | TransportError::PlaintextRefused
        | TransportError::UnsupportedEncryption => MailProviderError::Protocol,
    }
}

/// A refused login is told apart from everything else: the user has to act on it, and no amount of
/// retrying will help.
///
/// Except when it is not about the sign-in at all. A server that already has as many connections
/// from this account as it will allow refuses the same way — Gmail allows fifteen, Dovecot ten by
/// default — and calling that a wrong password sends somebody to re-type a password that was
/// right. What such a refusal says is that this is a bad moment, which is a thing to wait out.
fn login_failed(error: async_imap::error::Error) -> MailProviderError {
    match &error {
        async_imap::error::Error::No(said) | async_imap::error::Error::Bad(said)
            if too_busy_for_us(&said.to_string()) =>
        {
            MailProviderError::Offline
        }
        async_imap::error::Error::No(_) | async_imap::error::Error::Bad(_) => {
            MailProviderError::Unauthorized
        }
        _ => protocol_failed(error),
    }
}

/// Whether a refusal is the server saying "not now" rather than "not you".
///
/// Read from the words because that is what servers give: RFC 5530 named response codes for this
/// (`[LIMIT]`, `[INUSE]`, `[UNAVAILABLE]`), and the ones people actually use answer in prose. The
/// test is deliberately narrow — a phrase that is about capacity, never one that is merely
/// unfriendly — because reading a real refusal as "wait a while" would retry a wrong password
/// until the account is locked.
fn too_busy_for_us(said: &str) -> bool {
    let said = said.to_ascii_lowercase();

    said.contains("[limit]")
        || said.contains("[inuse]")
        || said.contains("[unavailable]")
        || said.contains("too many simultaneous connections")
        || said.contains("maximum number of connections")
        || said.contains("too many connections")
}

fn protocol_failed(error: async_imap::error::Error) -> MailProviderError {
    match error {
        async_imap::error::Error::Io(_) | async_imap::error::Error::ConnectionLost => {
            MailProviderError::Offline
        }
        _ => MailProviderError::Protocol,
    }
}

/// The endpoint of the development mail server, used by the tests that need a real conversation.
#[cfg(test)]
pub fn development_endpoint() -> Endpoint {
    use crate::infrastructure::net::Encryption;

    Endpoint {
        host: "127.0.0.1".to_owned(),
        port: 1143,
        encryption: Encryption::None,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod corpus_mailbox;

#[cfg(test)]
mod not_you_or_not_now {
    use super::*;

    fn refused(said: &str) -> MailProviderError {
        login_failed(async_imap::error::Error::No(said.to_owned()))
    }

    #[test]
    fn a_server_with_no_room_left_is_a_bad_moment_rather_than_a_bad_password() {
        // What Gmail says at fifteen connections, and what Dovecot says at ten. Calling either a
        // wrong password sends somebody to re-type a password that was right — and, with a
        // retrying client, to keep failing until the account is locked.
        for said in [
            "[ALERT] Too many simultaneous connections. (Failure)",
            "Maximum number of connections from user+IP exceeded",
            "[LIMIT] Too many connections",
            "[INUSE] Mailbox is locked by another session",
        ] {
            assert_eq!(refused(said), MailProviderError::Offline, "{said}");
        }
    }

    #[test]
    fn a_password_the_server_refused_is_still_a_refused_password() {
        // The narrow half of the rule. Reading an ordinary refusal as "wait a while" would retry a
        // wrong password for ever and never tell anybody why nothing arrives.
        for said in [
            "[AUTHENTICATIONFAILED] Authentication failed.",
            "Invalid credentials (Failure)",
            "LOGIN failed",
        ] {
            assert_eq!(refused(said), MailProviderError::Unauthorized, "{said}");
        }
    }
}

/// Opens a session, by whichever command the credential calls for.
///
/// The one place the two kinds part company, and it is one place on purpose: a password goes in
/// `LOGIN`, a token in `AUTHENTICATE XOAUTH2`, and every connection this application opens — the
/// one that reads and the one that acts — comes through here. Two copies of this would be two
/// answers to "how does this build sign in", and one of them would go stale.
pub(crate) async fn open_session<T>(
    client: async_imap::Client<T>,
    account: &ImapAccount,
) -> Result<async_imap::Session<T>, MailProviderError>
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + std::fmt::Debug,
{
    let mut client = client;

    // The greeting, read here rather than left in the buffer. `LOGIN` walks past it by itself — it
    // reads until it sees its own tag — but `AUTHENTICATE` reads exactly one response, finds the
    // greeting is not a continuation, and hands it to a helper that then swallows the `+` the
    // mechanism was waiting for. The payload is never sent and the exchange stops dead.
    //
    // Read rather than discarded, because the greeting is the one place a server says it will not
    // serve this connection at all.
    let greeting = client
        .read_response()
        .await
        .map_err(|_| MailProviderError::Offline)?
        .ok_or(MailProviderError::Offline)?;

    if let Response::Data { status, .. } = greeting.parsed() {
        // `* BYE` before anything is asked is a server turning the connection away — too many of
        // them, a maintenance window, an address that is not serving today. Not a wrong password.
        if matches!(status, Status::Bye) {
            return Err(MailProviderError::Offline);
        }
    }

    match &account.credential {
        // The client is returned with the error so it can be reused; there is nothing to reuse
        // after a refused login, and holding on to it would keep the secret in memory longer.
        Credential::Password(password) => client
            .login(&account.username, password.expose_secret())
            .await
            .map_err(|(error, _client)| login_failed(error)),
        Credential::AccessToken(token) => {
            // A token this build will not put on the wire — one carrying the separator, or empty —
            // is a refusal rather than a protocol fault, because what follows is the same either
            // way: sign in again. Said as our own error rather than dressed up as one the client
            // library might have produced.
            let response = xoauth2::initial_response(&account.username, token.expose_secret())
                .map_err(|_| MailProviderError::Unauthorized)?;

            client
                .authenticate("XOAUTH2", Xoauth2 { response })
                .await
                .map_err(|(error, _client)| login_failed(error))
        }
    }
}

/// Hands the XOAUTH2 payload to the client library, which encodes it.
///
/// The server sends an empty continuation and expects the whole credential in one go, so there is
/// nothing to react to: whatever is asked, the answer is the same string. A challenge that arrives
/// after that one is the server describing why it refused, and the mechanism's answer to that is
/// an empty line — which is what returning nothing produces.
struct Xoauth2 {
    response: Vec<u8>,
}

impl async_imap::Authenticator for Xoauth2 {
    type Response = Vec<u8>;

    fn process(&mut self, _challenge: &[u8]) -> Self::Response {
        std::mem::take(&mut self.response)
    }
}

#[cfg(test)]
mod signing_in_tests;

pub mod as_account_provider;

#[cfg(test)]
mod as_account_provider_tests;
