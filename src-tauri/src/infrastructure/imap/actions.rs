//! The one place in this application that changes somebody else's mailbox.
//!
//! Everything here is addressed by UID, because a sequence number names whichever message is in
//! that position *right now* and a message expunged by another client a moment ago shifts every
//! number after it. The guard on the stream refuses the positional forms outright, so this is
//! belt and braces rather than a rule to remember.
//!
//! Removing a message for good is refused unless the server offers `UIDPLUS`. Without it the only
//! way to expunge is a bare `EXPUNGE`, which removes every message in the mailbox marked
//! `\Deleted` — including ones another client marked and has not finished with. Refusing to delete
//! is a thing a person can see and act on; deleting somebody else's mail is not.

use async_imap::error::Error as ImapError;
use async_imap::imap_proto::{MailboxDatum, Response, ResponseCode, Status};
use futures::{pin_mut, TryStreamExt};
use tokio::runtime::Handle;

use super::answer::opens_with;
use super::open_session;
use crate::infrastructure::imap::read_only::ReadOnly;
use crate::infrastructure::imap::ImapAccount;
use crate::infrastructure::net::{Deadline, Silence, TransportPolicy};
use crate::ports::mail_actions::{
    Acting, Filing, FilingError, MailActionError, MailActions, RefusedSearch, SentMail,
    ServerAction,
};
use crate::ports::mail_provider::MailProviderError;
use async_imap::types::NameAttribute;
use lyonoyl_domain::message::Flags;
use lyonoyl_domain::outgoing::Outgoing;

/// Acts on one account's mailboxes, one conversation per round of instructions.
pub struct ImapMailActions {
    account: ImapAccount,
    policy: TransportPolicy,
    runtime: Handle,
}

impl ImapMailActions {
    pub fn new(account: ImapAccount, runtime: Handle) -> Result<Self, MailActionError> {
        Ok(Self {
            account,
            policy: TransportPolicy::new().map_err(|_| MailActionError::NotNow)?,
            runtime,
        })
    }
}

impl MailActions for ImapMailActions {
    fn reach(&self) -> Result<Box<dyn Acting + '_>, MailActionError> {
        let session = self.runtime.block_on(async {
            let stream = self
                .policy
                .connect_retrying(&self.account.endpoint)
                .await
                .map_err(|_| MailActionError::NotNow)?;

            // The one kind of connection that may send the four commands below. A pass opens the
            // other kind and may send none of them. The same deadline as a reading session:
            // nothing on an acting connection is defined to be silent, and a server that goes
            // quiet holding an instruction is the one case where waiting for ever is worst.
            let client = async_imap::Client::new(Deadline::over(
                ReadOnly::acting(stream),
                Silence::of(self.policy.read_timeout()),
            ));

            open_session(client, &self.account)
                .await
                .map_err(not_now_or_refused)
        })?;

        Ok(Box::new(ImapActing {
            runtime: &self.runtime,
            session: Some(session),
        }))
    }
}

/// One acting conversation, and the runtime its commands are run on.
struct ImapActing<'a, S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    runtime: &'a Handle,
    session: Option<async_imap::Session<S>>,
}

impl<S> Acting for ImapActing<'_, S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    fn carry_out(&mut self, action: ServerAction<'_>) -> Result<(), MailActionError> {
        let Some(session) = self.session.as_mut() else {
            return Err(MailActionError::NotNow);
        };

        let outcome = self.runtime.block_on(carry_out_on(session, action));

        // A conversation that failed without an answer — dropped, or gone silent — carries nothing
        // further: whatever is sent down it next is sent into nothing. Closed where it is, inside
        // the runtime its connection belongs to.
        if outcome == Err(MailActionError::NotNow) {
            if let Some(broken) = self.session.take() {
                self.runtime.block_on(async move { drop(broken) });
            }
        }

        outcome
    }
}

impl<S> Drop for ImapActing<'_, S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    fn drop(&mut self) {
        // Ended with LOGOUT rather than CLOSE: on a mailbox opened for writing, CLOSE expunges
        // whatever is marked deleted, which is the one thing this file is careful about.
        if let Some(mut session) = self.session.take() {
            self.runtime.block_on(async move {
                let _ = session.logout().await;
            });
        }
    }
}

impl SentMail for ImapMailActions {
    fn file(
        &self,
        message_id: &str,
        message: &Outgoing,
        written_at: i64,
        refused_search: RefusedSearch,
    ) -> Result<Filing, FilingError> {
        // Written here, which is where writing a message belongs. One adapter asking another is
        // ordinary; a use case asking either is the boundary this crossed.
        let raw = crate::infrastructure::smtp::written(message_id, message, written_at);

        self.runtime.block_on(async {
            let stream = self
                .policy
                .connect_retrying(&self.account.endpoint)
                .await
                .map_err(|_| FilingError::FolderUnreachable)?;

            let client = async_imap::Client::new(Deadline::over(
                ReadOnly::acting(stream),
                Silence::of(self.policy.read_timeout()),
            ));

            // A sign-in refused is the account's, and the copy behind this one would meet it too.
            let session = open_session(client, &self.account)
                .await
                .map_err(|_| FilingError::FolderUnreachable)?;

            file_and_leave(session, message_id, &raw, refused_search).await
        })
    }
}

/// Puts a copy in the folder for sent mail on a session that is already open, and ends the
/// conversation.
///
/// Ended the polite way only where the conversation is known to be whole: a copy filed, found
/// there already, with nowhere to go or refused for good was answered to the end. A folder not
/// reached or a copy refused for now may be a conversation that broke, and one of those carries
/// nothing further, the same rule `carry_out` keeps: a sign-out written into it is a minute spent
/// waiting on a server that is not listening, on one still counting the bytes of a message that
/// stopped halfway, or on one that will not stop talking, which the deadline on the connection
/// never meets. Dropped instead, inside the runtime its connection belongs to. A server that said
/// "not now" in so many words loses a goodbye and nothing else.
pub(crate) async fn file_and_leave<S>(
    mut session: async_imap::Session<S>,
    message_id: &str,
    raw: &str,
    refused_search: RefusedSearch,
) -> Result<Filing, FilingError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let outcome = file_on(&mut session, message_id, raw, refused_search).await;

    match outcome {
        Ok(_) | Err(FilingError::RefusedForGood) => {
            let _ = session.logout().await;
        }
        Err(FilingError::FolderUnreachable | FilingError::RefusedForNow) => drop(session),
    }

    outcome
}

/// Puts a copy in the folder for sent mail, on a session that is already open.
///
/// Which folder that is comes from the server rather than from anything stored here: `LIST` states
/// it with `\Sent`, and a server that states none has nowhere this device is willing to guess at.
/// Filing a message into a folder chosen by resemblance is how a copy of somebody's mail ends up
/// somewhere they did not put it.
///
/// Whether the copy is there already is asked before it is offered, on every try, and an answer to
/// that decides on every one of them. A refusal that names no copy and is not about the folder is
/// what `refused_search` makes it: heeded, nothing is added and the copy counts as refused for now;
/// passed over, the copy is offered as if the folder had been searched and found without it.
pub(crate) async fn file_on<S>(
    session: &mut async_imap::Session<S>,
    message_id: &str,
    raw: &str,
    refused_search: RefusedSearch,
) -> Result<Filing, FilingError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let Some(mailbox) = where_sent_mail_goes(session).await? else {
        return Ok(Filing::Nowhere);
    };

    // A folder the server will not open is one no copy can be put in, the next one included.
    session
        .select(&mailbox)
        .await
        .map_err(|_| FilingError::FolderUnreachable)?;

    // Asked by the name this device gave the message before the first attempt to send it. A server
    // that files a sent message itself has already put this very message there, and adding another
    // would leave the person with two of everything they send.
    //
    // In one pair of angle brackets, as the header carries it. The outbox keeps the name with its
    // brackets, and wrapped in a second pair it is a name no message carries: the search finds
    // nothing, and a server that files its own copy is given a second one.
    let named = message_id
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>');
    match (already_there(session, named).await?, refused_search) {
        (Some(true), _) => return Ok(Filing::AlreadyThere),
        // A server that will not say whether it has the copy may have it.
        (None, RefusedSearch::Heeded) => return Err(FilingError::RefusedForNow),
        (Some(false), _) | (None, RefusedSearch::PassedOver) => {}
    }

    appended(session, &mailbox, raw).await?;

    Ok(Filing::Added)
}

/// The most lines an answer about one copy is read for.
///
/// An honest answer to one search or to one message added is its own line and a few about the
/// mailbox. The deadline on the connection measures silence, so a server that keeps talking would
/// hold the round for as long as it talks; past this many lines it is a conversation that broke.
pub(crate) const LINES_ONE_COPY_IS_ANSWERED_IN: usize = 1024;

/// Whether a message carrying the name `named` is in the mailbox open on the session, or nothing
/// when the server refuses to say.
///
/// Read to the line that ends the answer, rather than through the client library, which ends a
/// search at the command's own tagged line whatever that line says, and at a connection that
/// closed: a search the server refused, or one cut short, reads there as a folder without the
/// message, and a server that filed its own copy is given a second one. Here an answer cut short or
/// out of step is the folder not reached, and so is a refusal about the folder
/// ([`about_the_folder`]). Every other refusal is one and the same, `[CANNOT]` and a `BAD` among
/// them: it says the server will not search, and nothing about whether it would take the copy.
/// Taken for good, it would end the copy's filing with the copy never offered.
///
/// A message the server named before such a refusal is the copy found all the same. A search that
/// stops partway may leave out a message carrying the name, and the one it named carries it: taken
/// for a refusal, it is a copy the server said it has, offered a second time once the waits are
/// over.
async fn already_there<S>(
    session: &mut async_imap::Session<S>,
    named: &str,
) -> Result<Option<bool>, FilingError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    // Written as the mailbox is. The name is made on this device from the sender's domain, and a
    // quote or a backslash in that would end the string partway, leaving the rest of the name to be
    // read as more of the command. A name no line can carry is one no search can ask about, which
    // is about this copy alone.
    let named = quoted(&format!("<{named}>")).ok_or(FilingError::RefusedForNow)?;

    let asked = session
        .run_command(format!("UID SEARCH HEADER Message-ID {named}"))
        .await
        .map_err(|_| FilingError::FolderUnreachable)?;

    let mut found = false;
    for _ in 0..LINES_ONE_COPY_IS_ANSWERED_IN {
        let Some(line) = session
            .read_response()
            .await
            .map_err(|_| FilingError::FolderUnreachable)?
        else {
            return Err(FilingError::FolderUnreachable);
        };

        match line.parsed() {
            Response::Done {
                tag,
                status,
                code,
                information,
            } if tag == &asked => {
                return match status {
                    Status::Ok => Ok(Some(found)),
                    Status::No | Status::Bad
                        if about_the_folder(code.as_ref(), information.as_deref()) =>
                    {
                        Err(FilingError::FolderUnreachable)
                    }
                    Status::No | Status::Bad => Ok(found.then_some(true)),
                    _ => Err(FilingError::FolderUnreachable),
                };
            }
            Response::Data {
                status: Status::Bye,
                ..
            } => return Err(FilingError::FolderUnreachable),
            Response::MailboxData(MailboxDatum::Search(numbers)) => {
                found |= !numbers.is_empty();
            }
            _ => {}
        }
    }

    Err(FilingError::FolderUnreachable)
}

/// Adds `raw` to `mailbox`, marked read, and reads the answer to the line that ends it.
///
/// Sent here rather than by the client library, which reads one line after the command and gives
/// the message up on anything but the server's word to go on, keeping none of what that line said.
/// A server that knows how long a message is refuses one it will not take before a byte of it is
/// sent — too large, over quota — and there that refusal arrives as no refusal at all, with the
/// words that say which lost.
///
/// The first line the server begins decides whether the message follows, as it decides for the
/// guard on this connection, which passes the message unread after a `+` and only then: anything
/// else first is a server that will not have the message follow, or one this conversation has lost
/// step with, and the message is not sent.
///
/// Once the command is out, the server has been offered this copy, its size among it, and a
/// conversation that ends without answering — the server saying goodbye, the connection closing or
/// failing with the message partway, silence past the deadline, more lines than an answer takes —
/// is taken for its answer, for now. Some servers hang up on a message they will not take rather
/// than refuse it. Taken for the folder not reached, that copy would be the first asked about in
/// every round, uploaded whole each time, and every copy behind it would wait for good. Taken for
/// now, it is counted and bounded like any other refusal, and one the server kept after all is
/// found by the search a later try opens with. Where every later try meets a refusal of that search
/// that names no copy and is not about the folder, the one after the last wait offers the copy
/// again, and the server can keep it twice.
async fn appended<S>(
    session: &mut async_imap::Session<S>,
    mailbox: &str,
    raw: &str,
) -> Result<(), FilingError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let mailbox = quoted(mailbox).ok_or(FilingError::FolderUnreachable)?;

    // Marked read: a copy of what this person has just written is not something to come back to,
    // and an unread count that goes up every time they send is a count that means nothing.
    let asked = session
        .run_command(format!("APPEND {mailbox} (\\Seen) {{{}}}", raw.len()))
        .await
        .map_err(|_| FilingError::FolderUnreachable)?;

    let mut sent = false;
    for _ in 0..LINES_ONE_COPY_IS_ANSWERED_IN {
        let Some(line) = session
            .read_response()
            .await
            .map_err(|_| FilingError::RefusedForNow)?
        else {
            return Err(FilingError::RefusedForNow);
        };

        match line.parsed() {
            Response::Continue { .. } if !sent => {
                session
                    .run_command_untagged(raw)
                    .await
                    .map_err(|_| FilingError::RefusedForNow)?;
                sent = true;
            }
            Response::Done {
                tag,
                status,
                code,
                information,
            } if tag == &asked => {
                return match status {
                    Status::Ok if sent => Ok(()),
                    Status::No | Status::Bad => {
                        Err(refusal_of_a_copy(code.as_ref(), information.as_deref()))
                    }
                    _ => Err(FilingError::FolderUnreachable),
                };
            }
            Response::Data {
                status: Status::Bye,
                ..
            } => return Err(FilingError::RefusedForNow),
            _ if !sent => return Err(FilingError::FolderUnreachable),
            _ => {}
        }
    }

    Err(FilingError::RefusedForNow)
}

/// A mailbox name written as a quoted string, the way the client library writes one, or nothing
/// for a name that a line cannot carry.
fn quoted(name: &str) -> Option<String> {
    (!name.contains(['\r', '\n']))
        .then(|| format!("\"{}\"", name.replace('\\', "\\\\").replace('"', "\\\"")))
}

/// The codes a refusal of one copy, or of the search for it, opens with when it is about the folder
/// rather than the copy: not there (RFC 5530), or not this account's to add to. Every copy after it
/// meets the same.
const NOT_THIS_ACCOUNTS: [&str; 2] = ["[NONEXISTENT]", "[NOPERM]"];

/// The codes a refusal of one copy opens with when the server will never take it: too large for
/// it (RFC 7889, and RFC 4469 before it), or something it can never do (RFC 5530).
///
/// `[LIMIT]` is not one of them. RFC 5530 gives it to a limit of any kind, ones that pass among
/// them — signing in reads it as "not now" (`too_busy_for_us`, in `mod.rs`) — so it says nothing
/// about whether this copy will ever be taken. Taken for good, it would let every copy asked while
/// a passing limit lasts go with no copy anywhere its sender can find it.
const NEVER: [&str; 2] = ["[TOOBIG]", "[CANNOT]"];

/// What a refusal of one copy says about itself.
///
/// Told apart the way a reading pass tells its own refusals apart (`refusal_in`, in `answer.rs`):
/// by the code the server's words open with. A refusal about the folder is the folder not reached
/// ([`about_the_folder`]), one that says never is the copy refused for good, and the search asked
/// before the copy is offered is told apart by the first of those alone ([`already_there`]).
///
/// Everything else is taken for now: what says "later" — over quota, part of the server down, the
/// folder in use, the codes a reading pass takes for now — and what says nothing about until when:
/// a limit reached, a refusal with no code, a `BAD`, a code in neither list above. The two ways of
/// being wrong about one of those cost differently. Taken for good, a refusal that would have
/// passed ends the copy's filing, and its message goes with no copy anywhere its sender can find
/// it. Taken for now, one that never passes costs a few more requests, as many as the waits the
/// round gives a refused copy, before it is taken for good all the same.
fn refusal_of_a_copy(code: Option<&ResponseCode<'_>>, said: Option<&str>) -> FilingError {
    if about_the_folder(code, said) {
        return FilingError::FolderUnreachable;
    }

    match opens_with(said, &NEVER) {
        true => FilingError::RefusedForGood,
        false => FilingError::RefusedForNow,
    }
}

/// Whether a refusal is about the folder for sent mail rather than the copy asked about.
///
/// The one code the client library knows is `TRYCREATE`, which RFC 3501 gives an `APPEND` into a
/// mailbox that is not there; the others arrive as words ([`NOT_THIS_ACCOUNTS`]).
fn about_the_folder(code: Option<&ResponseCode<'_>>, said: Option<&str>) -> bool {
    matches!(code, Some(ResponseCode::TryCreate)) || opens_with(said, &NOT_THIS_ACCOUNTS)
}

/// The mailbox the server itself says holds sent mail, if it says.
///
/// Said by a whole listing and by nothing less. Read as whole, a listing cut short before the
/// folder is a server that named none, and the copy is written down as having nowhere to go and is
/// never filed — so one refused, left by the server or ended by the connection is a moment to come
/// back from.
async fn where_sent_mail_goes<S>(
    session: &mut async_imap::Session<S>,
) -> Result<Option<String>, FilingError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let listed = super::answer::mailboxes_listed(session, super::answer::MOST_A_PASS_ANSWER_HOLDS)
        .await
        .map_err(|_| FilingError::FolderUnreachable)?;

    Ok(listed
        .into_iter()
        .find(|mailbox| {
            mailbox
                .attributes
                .iter()
                .any(|attribute| matches!(attribute, NameAttribute::Sent))
        })
        .map(|mailbox| mailbox.name))
}

/// Carries out one instruction on a session that is already open.
///
/// Free and generic over the stream so that a scripted server can drive it: what this sends is
/// asserted byte for byte in `transcript_tests`, which is the only way to see the difference
/// between a command that names a message and one that names a position.
pub(crate) async fn carry_out_on<S>(
    session: &mut async_imap::Session<S>,
    action: ServerAction<'_>,
) -> Result<(), MailActionError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let (mailbox, addressed_under) = match action {
        ServerAction::SetFlags {
            mailbox,
            uid_validity,
            ..
        }
        | ServerAction::MoveTo {
            mailbox,
            uid_validity,
            ..
        }
        | ServerAction::Erase {
            mailbox,
            uid_validity,
            ..
        } => (mailbox, uid_validity),
    };

    // Opened for writing. A mailbox the server will not open is a mailbox nothing can be done
    // in, and that is a refusal rather than something to try again.
    let opened = session
        .select(mailbox)
        .await
        .map_err(|error| refused_or_not_now(&error))?;

    // The generation the server states now, against the one the number was taken under. A folder
    // that renumbered has handed the same numbers to different messages, so a UID from before it
    // names somebody else's message — and everything below this line is addressed by UID.
    //
    // A server that states no generation at all is one nothing can be checked against, and acting
    // on a number that counts nothing is the thing this exists to prevent.
    if opened.uid_validity != Some(addressed_under.get()) {
        return Err(MailActionError::FolderRenumbered);
    }

    match action {
        ServerAction::SetFlags {
            uid, add, remove, ..
        } => {
            let uid = uid.get().to_string();

            if let Some(named) = named(add) {
                store(session, &uid, &format!("+FLAGS.SILENT ({named})")).await?;
            }
            if let Some(named) = named(remove) {
                store(session, &uid, &format!("-FLAGS.SILENT ({named})")).await?;
            }

            Ok(())
        }

        ServerAction::MoveTo {
            uid, destination, ..
        } => {
            let uid = uid.get().to_string();

            // A server that offers MOVE does it as one step, which is the only way the message
            // is never in both places or in neither. Without it the three steps below are what
            // there is, and the copy comes first: a copy that fails leaves the original where
            // it was, while a delete that fails after a copy leaves two.
            if offers(session, "MOVE").await {
                return session
                    .uid_mv(&uid, destination)
                    .await
                    .map_err(|error| refused_or_not_now(&error));
            }

            session
                .uid_copy(&uid, destination)
                .await
                .map_err(|error| refused_or_not_now(&error))?;

            erase(session, &uid).await
        }

        ServerAction::Erase { uid, .. } => {
            let uid = uid.get().to_string();

            erase(session, &uid).await
        }
    }
}

/// Marks one message deleted and removes that one message.
///
/// Refused outright on a server without `UIDPLUS`: see the note at the top of this file.
async fn erase<S>(session: &mut async_imap::Session<S>, uid: &str) -> Result<(), MailActionError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    if !offers(session, "UIDPLUS").await {
        return Err(MailActionError::Refused);
    }

    store(session, uid, "+FLAGS.SILENT (\\Deleted)").await?;

    let removed = session
        .uid_expunge(uid)
        .await
        .map_err(|error| refused_or_not_now(&error))?;
    pin_mut!(removed);

    // Drained to the end, because the command is not finished until the server has stopped
    // talking — and a refusal arrives here rather than above. What it names is of no interest:
    // what was removed is what was named.
    while removed
        .try_next()
        .await
        .map_err(|error| refused_or_not_now(&error))?
        .is_some()
    {}

    Ok(())
}

async fn store<S>(
    session: &mut async_imap::Session<S>,
    uid: &str,
    query: &str,
) -> Result<(), MailActionError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let updates = session
        .uid_store(uid, query)
        .await
        .map_err(|error| refused_or_not_now(&error))?;
    pin_mut!(updates);

    // `.SILENT` asks the server to say nothing back, but a server is free to speak anyway, and the
    // command is not finished until it has stopped. A refusal arrives here rather than above.
    while updates
        .try_next()
        .await
        .map_err(|error| refused_or_not_now(&error))?
        .is_some()
    {}

    Ok(())
}

/// Whether the server said it can do something.
async fn offers<S>(session: &mut async_imap::Session<S>, capability: &str) -> bool
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    match session.capabilities().await {
        Ok(capabilities) => capabilities.has_str(capability),
        // A server that will not say what it can do is treated as unable to do the careful thing.
        Err(_) => false,
    }
}

/// The flag names a server understands, or nothing when there are none to name.
fn named(flags: Flags) -> Option<String> {
    let named: Vec<&str> = [
        (Flags::SEEN, "\\Seen"),
        (Flags::ANSWERED, "\\Answered"),
        (Flags::FLAGGED, "\\Flagged"),
        (Flags::DRAFT, "\\Draft"),
        (Flags::DELETED, "\\Deleted"),
    ]
    .into_iter()
    .filter(|(flag, _)| flags.contains(*flag))
    .map(|(_, name)| name)
    .collect();

    (!named.is_empty()).then(|| named.join(" "))
}

/// Whether a failure is worth trying again.
///
/// A refusal the server spelled out is not: asking a second time gets the same answer, and the
/// person is shown it instead. Everything else — a dropped connection, a timeout, a server being
/// restarted — is a moment rather than an answer.
/// What a refused sign-in means to an action waiting to be carried out.
///
/// A credential the server would not take is a refusal — trying again with the same one changes
/// nothing. Everything else is a moment to come back from.
fn not_now_or_refused(error: MailProviderError) -> MailActionError {
    match error {
        MailProviderError::Unauthorized => MailActionError::Refused,
        _ => MailActionError::NotNow,
    }
}

fn refused_or_not_now(error: &ImapError) -> MailActionError {
    match error {
        ImapError::No(_) | ImapError::Bad(_) => MailActionError::Refused,
        _ => MailActionError::NotNow,
    }
}
