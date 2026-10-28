//! One pass over one mailbox, and everything it is allowed to conclude from the answer.
//!
//! The bounds live here because they are one argument: what a pass asks for, how far it reads, and
//! when it may say a message is gone. The rule is enforced by the handful of functions at the
//! bottom of this file deciding together, and having them in one place is what makes that
//! auditable in one sitting.

use super::answer::{answer_to, by_message, Fetched, IfRefused, Refusal, MOST_A_PASS_ANSWER_HOLDS};
use super::envelope::{envelope_of, nothing_read};
use super::*;

/// How many of the newest messages a first pass over a folder brings in.
///
/// A window rather than the whole mailbox: a first pass must put something on screen in seconds,
/// and a hundred thousand messages of headers is minutes of transfer. What is older than the window
/// is fetched later, backwards, by the backfill.
const FIRST_PASS_WINDOW: u32 = 500;
/// How many messages past where the last pass stopped one pass reads.
///
/// A window for the same reason a first pass has one, and for one more: an answer may hold only so
/// much, so a folder that gained more mail since its last pass than one answer holds would be
/// asked for all of it on every pass and refused every time, and never read again. Four times a
/// first pass's window, so as many new messages as that arrive in one pass; past it, mail arrives
/// over several passes, oldest first, each taking up where the one before it stopped.
///
/// Messages rather than numbers. A folder spends numbers on mail that has since left it — six
/// thousand filed out of an inbox from a phone, a junk folder its server empties, a server that
/// numbers every folder of an account from one counter — and a window of numbers spends a pass on
/// each stretch of them, so forty messages one answer holds easily would arrive over minutes, and
/// one message past fifty thousand unused numbers over most of an hour.
const FORWARD_WINDOW: u32 = 4 * FIRST_PASS_WINDOW;
/// What a pass asks for to learn which numbers past its position are mail.
///
/// The number alone: a line of a few dozen bytes a message, charged like a line of a listing.
const NUMBERS_QUERY: &str = "(UID)";
/// What one pass asks the server for.
///
/// `BODY.PEEK[HEADER]` rather than `BODY[HEADER]`: the unpeeked form sets `\Seen` on every message
/// it returns, which would mark a person's mail read on the server just for synchronising it.
///
/// Headers rather than `ENVELOPE`: the envelope arrives as raw bytes, still MIME-encoded, and would
/// have to be decoded here anyway. The header block goes through the same parser that reads the
/// message itself, so a subject is decoded once, in one place, by code written for hostile input.
pub(super) const PASS_QUERY: &str =
    "(UID FLAGS MODSEQ RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER] BODY.PEEK[TEXT]<0.2048>)";
/// What a pass asks about the mail a server refused to describe: what the server knows of each
/// message without reading what was written in it.
///
/// A server may refuse to read one message on every pass — a damaged file, a message its own
/// parser chokes on — and refuse it the same way each time it is asked. Asked for again, it would
/// stop the folder there for good: no new mail, no flag, no removal, nothing further back. Listed
/// by this instead, it is a row of what the server said, the folder moves past it, and the round
/// that reads mail whole fills in what it says once the server hands it over — the name it is
/// found by included, so a copy the account already holds becomes that copy again.
///
/// A refusal and nothing else. A server that ends the conversation over such a message instead —
/// Dovecot's own default — fails the pass before anything is listed, and the folder stays where it
/// was, on every pass. And a refusal that says it is a moment's matter is not a message the server
/// will not describe: the pass fails, and the next one asks again — once, because a server may say
/// that of the same message for ever. Refused that way again before a reading of the same kind has
/// been applied, the range is listed by this like any other refusal.
const FACTS_QUERY: &str = "(UID FLAGS RFC822.SIZE INTERNALDATE)";
/// What checking on messages already stored asks for. No headers: they were read once and do not
/// change.
/// Asking for `MODSEQ` is also what turns modification sequences on for the session: a server that
/// keeps them starts reporting them, and the next pass has a position to ask "what changed since"
/// from. A server that does not keep them ignores the request.
const AUDIT_QUERY: &str = "(UID FLAGS MODSEQ)";
/// How far back a pass checks what is still there and what its flags are now.
///
/// A folder of a hundred thousand messages cannot be listed on every pass, and a baseline server
/// offers no way to ask for "what changed": that is what CONDSTORE and QRESYNC are for. Until then
/// a pass makes an honest statement about a window it actually listed, and says nothing about the
/// rest — which is what stops it from deleting anything outside it.
const AUDIT_WINDOW: u32 = 500;

/// The position to ask "what changed since" from, or nothing when the question cannot be asked.
///
/// Three ways it cannot. A server that keeps no modification sequences reports none. A folder
/// nothing has been applied to has no position to ask from — its first pass reads it anyway. And a
/// mailbox whose sequence has not moved has nothing to tell: that is the common case, and skipping
/// it is what makes a pass over a quiet account cost one command.
pub(super) fn changed_since(request: &SyncRequest, reported: Option<u64>) -> Option<u64> {
    let reported = reported?;
    let applied = request.highest_modseq?.get();

    (applied < reported).then_some(applied)
}

/// Asks for the flags of everything that changed since a modification sequence.
///
/// Using the modifier is also what turns CONDSTORE on for this session: a server that understands
/// it needs no separate arrangement, and one that does not never reported a sequence to ask from.
///
/// `VANISHED` is added only where the server has agreed to it. It is what turns the same question
/// into "and what went away since then", answered in one line for the whole mailbox instead of a
/// listing of it — but a server that has not enabled QRESYNC answers the whole fetch with `BAD`
/// rather than ignoring the word.
pub(super) fn changed_since_query(modseq: u64, vanishing: bool) -> String {
    match vanishing {
        true => format!("(UID FLAGS) (CHANGEDSINCE {modseq} VANISHED)"),
        false => format!("(UID FLAGS) (CHANGEDSINCE {modseq})"),
    }
}

/// The extension that lets a server name what disappeared instead of being asked to list what has
/// not.
const QRESYNC: &str = "QRESYNC";

/// How many vanished UIDs one report may name before this build stops trying to hold them all.
///
/// A server may legitimately answer `VANISHED 1:*`, which as a list of numbers is the whole UID
/// space. Past this the report is dropped and the pass falls back to the audit that counts — which
/// asks for confirmation before removing a mailbox's worth of mail, rather than removing it.
const MOST_VANISHED_AT_ONCE: usize = 50_000;

/// Turns on the extension that lets a server say what disappeared, and answers whether it took.
///
/// One command rather than two: a server without the extension answers `OK` and names nothing as
/// enabled, so its own answer is the capability check. It has to be sent before a mailbox is
/// opened, because `ENABLE` is only valid in the authenticated state.
///
/// A refusal is not a failure of the pass. The pass then asks the way it always did, and finds out
/// what disappeared by counting instead of by being told.
pub(crate) async fn enable_qresync<S>(
    session: &mut async_imap::Session<S>,
) -> Result<bool, MailProviderError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    let asked = session
        .run_command("ENABLE QRESYNC")
        .await
        .map_err(protocol_failed)?;

    let mut named = false;

    while let Some(response) = session
        .read_response()
        .await
        .map_err(|_| MailProviderError::Protocol)?
    {
        match response.parsed() {
            Response::Capabilities(capabilities) => {
                named |= capabilities.iter().any(|capability| match capability {
                    Capability::Atom(name) => name.eq_ignore_ascii_case(QRESYNC),
                    _ => false,
                });
            }
            Response::Done { tag, status, .. } if tag == &asked => {
                return Ok(named && *status == Status::Ok);
            }
            _ => {}
        }
    }

    // The connection ended in the middle of a command, which is not an answer at all.
    Err(MailProviderError::Protocol)
}

/// The UIDs the server named as gone, or nothing when it named none.
///
/// Read from the answers to this pass's own fetches and from nowhere else. `VANISHED` may arrive
/// at any point rather than as the answer to anything, and a line said while the mailbox open
/// before this one was being closed is about that mailbox: taken as this one's, an expunge there
/// is a deletion here.
fn named_gone(ranges: &[RangeInclusive<u32>], uid_next: Option<u32>) -> Option<Vec<Uid>> {
    match listed_uids(ranges, uid_next) {
        Some(uids) if !uids.is_empty() => Some(uids),
        _ => None,
    }
}

/// The numbers a set of ranges names, or nothing when there are more of them than this build holds.
///
/// An open range ends at the newest UID the mailbox can hold: `*` means "the last message", and a
/// server naming it has not claimed anything about numbers that were never issued.
pub(super) fn listed_uids(
    ranges: &[RangeInclusive<u32>],
    uid_next: Option<u32>,
) -> Option<Vec<Uid>> {
    let ceiling = uid_next.map_or(u32::MAX, |next| next.saturating_sub(1));
    let mut uids: Vec<Uid> = Vec::new();

    for range in ranges {
        let first = *range.start();
        let last = (*range.end()).min(ceiling);

        if last < first {
            continue;
        }

        let count = u64::from(last - first) + 1;
        if uids.len() as u64 + count > MOST_VANISHED_AT_ONCE as u64 {
            return None;
        }

        uids.extend((first..=last).filter_map(|value| Uid::new(value).ok()));
    }

    Some(uids)
}

/// Whether the server has renumbered the mailbox since the index last read it.
fn replaced(request: &SyncRequest, reported: UidValidity) -> bool {
    request
        .uid_validity
        .is_some_and(|stored| stored != reported)
}

/// Whether the mailbox holds fewer messages than this folder accounts for.
///
/// The server states how many it has as soon as it is opened; the index knows how many it holds,
/// and this pass has just read the ones that arrived. Fewer on the server than the two together
/// means something was removed there. More is not a discrepancy: a first pass takes a window, and
/// what is older than it has not been read yet.
///
/// A pass reading further back is not asked the question at all: it is filling in history, and the
/// counts are meant to disagree while it does.
pub(super) fn vanished_since(
    exists: u32,
    known: usize,
    arrived: usize,
    reading_older: bool,
) -> bool {
    if reading_older {
        return false;
    }

    (exists as usize) < known + arrived
}

/// The range a pass lists, and nothing when it has no reason to list one.
///
/// Three cases. A mailbox that has lost messages is listed whole: what went is anywhere in it, and
/// a window would find only what went recently. A server that keeps no modification sequences is
/// asked about its newest window, because that is the only way to see a flag change at all. And a
/// mailbox that lost nothing on a server that reports sequences is not listed: the flags came from
/// asking what changed, and there is nothing else a listing would tell.
///
/// Never after a renumbering: the stored UIDs address a numbering the server has thrown away, and
/// listing them would come back empty and read as "all of it disappeared".
pub(super) fn listing_range(
    request: &SyncRequest,
    reported: UidValidity,
    missing: bool,
    uid_next: Option<u32>,
) -> Option<RangeInclusive<Uid>> {
    if replaced(request, reported) {
        return None;
    }

    let known = request.known_window.as_ref()?;

    if missing {
        // The whole of it, up to the newest UID either side knows about.
        let highest = known
            .end()
            .get()
            .max(uid_next.unwrap_or(1).saturating_sub(1))
            .max(1);
        return Some(Uid::new(1).ok()?..=Uid::new(highest).ok()?);
    }

    if request.highest_modseq.is_some() {
        return None;
    }

    let highest = known.end().get();
    let lowest = known
        .start()
        .get()
        .max(highest.saturating_sub(AUDIT_WINDOW - 1));

    Some(Uid::new(lowest).ok()?..=Uid::new(highest).ok()?)
}

/// The flags of every message the audit listed.
///
/// All of them, not only the ones that look different: the index compares against what it holds
/// and writes nothing when nothing changed. A line that said nothing about flags is not a message
/// with none, and is not reported as one.
fn flags_reported(fetched: &[Fetched]) -> Vec<FlagUpdate> {
    fetched
        .iter()
        .filter_map(|message| {
            Some(FlagUpdate {
                uid: Uid::new(message.uid?).ok()?,
                flags: message.flags?,
                modseq: message.modseq.map(ModSeq::new),
            })
        })
        .collect()
}

/// What the audit says about disappearances.
///
/// A complete listing of one range: whatever the index holds inside it and the server did not name
/// is gone from the server. Outside the range this says nothing, which is what keeps a pass from
/// deleting mail it never asked about.
pub(super) fn listed(window: &RangeInclusive<Uid>, fetched: &[Fetched]) -> VanishedReport {
    VanishedReport::AuthoritativeSet {
        window: window.clone(),
        present: fetched
            .iter()
            .filter_map(|message| Uid::new(message.uid?).ok())
            .collect(),
    }
}

/// The position the last pass reached, as this adapter writes it: the highest UID it has accounted
/// for — the last one it stored, or the end of a window the server answered whole.
fn stored_uid(request: &SyncRequest) -> Option<u32> {
    request
        .cursor
        .as_ref()
        .and_then(|cursor| cursor.as_str().parse::<u32>().ok())
        .filter(|uid| *uid > 0)
}

/// Where this pass resumes from, which is nowhere when the stored position cannot belong to the
/// mailbox the server is describing.
///
/// Two ways that happens. A server that reports a different `UIDVALIDITY` has renumbered the
/// mailbox, so the stored cursor counts in a numbering that no longer exists. And a cursor at or
/// above `UIDNEXT` names a message the mailbox cannot have, whatever the generation says — a
/// server that renumbered without admitting it, or a cursor left behind by an older build.
///
/// Both are resolved the same way: read the window again. Nothing is duplicated by doing so —
/// arrivals are matched on their `Message-ID` — whereas continuing from a position that means
/// nothing asks for messages above a boundary that does not exist, and the folder stays empty.
pub(super) fn resume_from(
    request: &SyncRequest,
    reported: UidValidity,
    uid_next: Option<u32>,
) -> Option<u32> {
    if replaced(request, reported) {
        return None;
    }

    stored_uid(request).filter(|last| uid_next.is_none_or(|next| *last < next))
}

/// Whether the folder holds anything this device has not read.
///
/// The number the server gives when a mailbox is opened is the one the next message will be given,
/// so there is new mail only where it is past the last one held. Asked before the fetch rather
/// than filtered after it, because `UID FETCH n:*` is not empty when nothing is newer than `n`:
/// the range reads as "between n and the highest", and a server answers with the last message in
/// the folder — its envelope and the opening of its body — every time. Six mailboxes across nine
/// accounts, every two minutes, was fifty-four messages fetched and thrown away.
pub(super) fn anything_newer(stored: Option<u32>, uid_next: Option<u32>) -> bool {
    match (stored, uid_next) {
        // Nothing held, so whatever is there is new. A first pass reads its window.
        (None, _) => true,
        // The server did not say. Asking is the only way to find out.
        (Some(_), None) => true,
        (Some(last), Some(next)) => last.saturating_add(1) < next,
    }
}

/// Which UIDs to ask for.
///
/// A first pass takes the newest `FIRST_PASS_WINDOW`; every pass after it continues from where the
/// last one stopped, as far as `through` says — the end of its window, from [`window_end`]. A
/// server that names no next number leaves nothing to end a window at, and is asked for
/// everything past the position.
pub(super) fn fetch_range(
    stored: Option<u32>,
    through: Option<u32>,
    uid_next: Option<u32>,
) -> String {
    match (stored, through) {
        (Some(last), Some(end)) => format!("{}:{end}", last.saturating_add(1)),
        (Some(last), None) => format!("{}:*", last.saturating_add(1)),
        (None, _) => {
            let newest = uid_next.unwrap_or(1).max(1);
            let floor = newest.saturating_sub(FIRST_PASS_WINDOW).max(1);
            format!("{floor}:*")
        }
    }
}

/// Whether more numbers have been issued past `stored` than a window holds, so a pass has to ask
/// which of them are messages before it knows where its window ends.
///
/// No more numbers than a window are no more messages than a window, and are read in one answer
/// without asking. Past that, most of the numbers may name nothing.
pub(super) fn needs_a_listing(stored: u32, newest: u32) -> bool {
    newest.saturating_sub(stored) > FORWARD_WINDOW
}

/// The last UID a pass continuing from `stored` reads, given the UIDs a listing named past it: the
/// `FORWARD_WINDOW`th message, or `newest` when no more than a window's worth are waiting.
///
/// `newest` is the last number the server had issued when the mailbox was opened, and the window
/// never ends past it. That is what lets an answer completed with `OK` move the cursor to the end
/// of the window rather than to the highest message in it: a number below `UIDNEXT` has been
/// issued for good, so one the whole answer did not name is a message that is gone — whereas a
/// number past it may be issued yet, and a cursor moved over it would pass the message given it.
///
/// Only numbers the listing asked about count. A line about any other message is a server saying
/// something in passing, and counted, it would end the window short of mail nobody has read.
pub(super) fn window_end(stored: u32, newest: u32, listed: &[u32]) -> u32 {
    let mut named: Vec<u32> = listed
        .iter()
        .copied()
        .filter(|uid| is_new(*uid, Some(stored)) && *uid <= newest)
        .collect();
    named.sort_unstable();
    named.dedup();

    match named.len() > FORWARD_WINDOW as usize {
        true => named[FORWARD_WINDOW as usize - 1],
        false => newest,
    }
}

/// The UIDs this pass's fetch asks about, and so the only ones an answer to it can be about: from
/// the lowest number it asked for — where the last pass stopped, a first pass's floor, or the
/// bottom of a range read further back — through `up_to`: the end of the window or of that range,
/// or for a first pass the newest number the server had issued when the mailbox was opened.
///
/// A server says things nobody asked it: a flag another device changed while the fetch ran arrives
/// in the middle of its answer, as a line naming that message by its UID — and with QRESYNC on, a
/// server must name it so. Taken for an answer, a line above a window's end is a row with nothing
/// in it and a cursor moved past numbers nobody read; one about a message already stored, below a
/// first pass's floor or outside a range read further back, is a blank row put in that message's
/// place.
///
/// A first pass asks from its floor to `*`, and `*` names the last message wherever it is: below
/// the floor too, when nothing newer is there. Where the answer says that is so, the reading starts
/// at that message instead — see [`first_floor`].
pub(super) fn asked_about(from: Option<Uid>, up_to: Option<u32>) -> RangeInclusive<u32> {
    from.map_or(1, Uid::get)..=up_to.unwrap_or(u32::MAX)
}

/// Where a first pass's reading starts, given the lines of an answer to `floor:*`, of which
/// `answers` picks the ones that answered the question.
///
/// Its floor, unless nothing at or above the floor was answered about and something below it was.
/// `floor:*` names the newest message wherever it is, so a newest message below the floor is the
/// one message the answer can be about there, and it says every number between it and the floor
/// holds nothing. Read from there, it arrives with this pass and the reading further back takes up
/// below it. Read from the floor, the folder shows nothing until the reading further back has
/// walked five hundred empty numbers a pass down to it: hours for an inbox kept empty sixty
/// thousand numbers in, and days on a server that numbers every folder from one counter.
///
/// Asked of the pass's own answer, a line answers when it [`describes`] its message. Asked of the
/// facts a server lists about mail it refused to describe, a line answers when it [`states_facts`]:
/// the newest message may be the one refused, and its facts are then all that says it is there.
/// A flag changed on another device is said in passing about any message at all, and does
/// neither. And of two below the floor, which an honest server never sends, the newer: the numbers
/// above it are all the answer vouches for.
pub(super) fn first_floor(
    floor: Uid,
    lines: &[Fetched],
    answers: impl Fn(&Fetched) -> bool,
) -> Uid {
    let mut newest_below = None;

    for uid in lines
        .iter()
        .filter(|line| answers(line))
        .filter_map(|line| line.uid)
    {
        if uid >= floor.get() {
            return floor;
        }
        newest_below = newest_below.max(Some(uid));
    }

    newest_below
        .and_then(|uid| Uid::new(uid).ok())
        .unwrap_or(floor)
}

/// Whether a line of the pass's own answer describes its message: it carries the header asked for.
pub(super) fn describes(line: &Fetched) -> bool {
    line.header.is_some()
}

/// Whether a line of the answer to [`FACTS_QUERY`] states its message's facts — its size or its
/// date — rather than a flag said in passing.
pub(super) fn states_facts(line: &Fetched) -> bool {
    line.size.is_some() || line.internal_date.is_some()
}

/// Every message a fetch brought that this pass had not already stored, among the ones it asked
/// about.
///
/// One line a message: `fetched` is what [`by_message`] made of the answer, so a line said in
/// passing about a message that is arriving is that message rather than a second one.
///
/// `describe` is what turns a message's bytes into what the index keeps: the parser, everywhere
/// but in a test that needs one to fail.
pub(super) fn arrivals(
    fetched: &[Fetched],
    stored: Option<u32>,
    asked: &RangeInclusive<u32>,
    describe: impl Fn(&Fetched) -> MessageEnvelope,
) -> Vec<ArrivedMessage> {
    fetched
        .iter()
        .filter_map(|message| arrival(message, stored, asked, &describe))
        .collect()
}

/// One fetched message, or nothing when its UID is not one this pass asked about or is one it had
/// already stored.
///
/// `UID FETCH n:*` does not answer with an empty result when nothing is newer than `n`: the range
/// is read as "between n and the highest UID", so a server returns the last message in the folder
/// over and over. Without this filter every pass would re-store the newest message.
fn arrival(
    message: &Fetched,
    stored: Option<u32>,
    asked: &RangeInclusive<u32>,
    describe: &impl Fn(&Fetched) -> MessageEnvelope,
) -> Option<ArrivedMessage> {
    let uid = message.uid?;
    if !is_new(uid, stored) || !asked.contains(&uid) {
        return None;
    }

    // One message at a time, because the bytes are a stranger's and the parser reading them runs
    // here, on the pass, rather than behind the guards the reading path has. A panic in it ends
    // this message's description and nothing else: the message is still a row, made of what the
    // server said about it, and the round that reads messages whole and the opening of one each
    // catch a panic for themselves. Uncaught, it ends the pass over the whole account, and every
    // pass after it asks for the same message again.
    let envelope = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| describe(message)))
        .unwrap_or_else(|_| what_the_server_said(message));

    Some(ArrivedMessage {
        uid: Uid::new(uid).ok()?,
        modseq: message.modseq.map(ModSeq::new),
        envelope,
    })
}

/// What a message's bytes say, read by the parser written for hostile input.
pub(super) fn described(message: &Fetched) -> MessageEnvelope {
    envelope_of(
        message.header.as_deref().unwrap_or_default(),
        message.text.as_deref().unwrap_or_default(),
        message.internal_date,
        message.size,
        message.flags.unwrap_or(Flags::NONE),
    )
}

/// A message described by what the server said about it and by nothing it holds.
fn what_the_server_said(message: &Fetched) -> MessageEnvelope {
    nothing_read(
        message.internal_date,
        message.size,
        message.flags.unwrap_or(Flags::NONE),
    )
}

/// The messages a refused fetch did not describe, as rows of what the server knows about each.
///
/// `listed` is the answer to [`FACTS_QUERY`] over the same range, made one line a message. Every
/// number it names in the range asked about, that this pass had not stored and that did not
/// arrive, is a message the server holds and would not describe: stored as a row of its date, its
/// size and its flags, it is listed, counted and opened like any other, and the folder's position
/// moves past it. The round that reads mail whole fills in what it says when the server hands it
/// over, and opening it does the same.
pub(super) fn withheld(
    listed: &[Fetched],
    stored: Option<u32>,
    asked: &RangeInclusive<u32>,
    arrived: &[ArrivedMessage],
) -> Vec<ArrivedMessage> {
    let came: std::collections::HashSet<u32> =
        arrived.iter().map(|message| message.uid.get()).collect();

    listed
        .iter()
        .filter_map(|message| {
            let uid = message.uid?;
            if !is_new(uid, stored) || !asked.contains(&uid) || came.contains(&uid) {
                return None;
            }

            Some(ArrivedMessage {
                uid: Uid::new(uid).ok()?,
                modseq: message.modseq.map(ModSeq::new),
                envelope: what_the_server_said(message),
            })
        })
        .collect()
}

/// The lowest UID a fetch range covers, read back from the range itself so the two cannot drift.
fn covered_from(range: &str) -> Option<Uid> {
    range
        .split_once(':')
        .and_then(|(low, _)| low.parse::<u32>().ok())
        .and_then(|low| Uid::new(low).ok())
}

/// Whether a fetched UID is one this pass had not already stored.
pub(super) fn is_new(uid: u32, stored: Option<u32>) -> bool {
    stored.is_none_or(|last| uid > last)
}

/// One pass over one mailbox, on a session that is already open.
///
/// Generic over the stream, and separate from `fetch`, so a scripted conversation can drive it. A
/// pass is decided entirely by what the server answers, and the answers worth testing — a mailbox
/// that reports a different `UIDVALIDITY`, a fetch that stops halfway, a `NO` — are the ones no
/// live server gives on demand.
pub(crate) async fn one_pass<S>(
    session: &mut async_imap::Session<S>,
    request: &SyncRequest,
    // Whether this session has QRESYNC on. Handed in rather than asked for here, because `ENABLE`
    // is only valid before a mailbox is opened and one session now reads several mailboxes: asked
    // for on the second folder it arrives in the wrong state, the server answers `BAD`, and the
    // pass goes on quietly without exact deletions.
    qresync: bool,
) -> Result<SyncBatch, MailProviderError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + std::fmt::Debug + Send,
{
    // Only for a folder that has a position to ask "what changed since" from: without one there
    // is nothing to ask what vanished after, and the first pass over a folder reads it whole
    // anyway. Whether the session can answer at all was settled when it was opened.
    let vanishing = qresync && request.highest_modseq.is_some();

    // EXAMINE rather than SELECT: the read-only form. A server that honours it cannot be
    // asked to change anything through this session even by mistake.
    let mailbox = session
        .examine(&request.mailbox)
        .await
        .map_err(protocol_failed)?;

    // What the session was told before this mailbox was open is about another one. A server with
    // QRESYNC on reports what left the mailbox it is closing as `VANISHED`, in the middle of the
    // answer that opens this one, and the client library queues it with everything else said in
    // passing: left there, it is waiting for whoever reads the queue next, as if it were news
    // about this mailbox.
    while session.unsolicited_responses.try_recv().is_ok() {}

    let generation = mailbox
        .uid_validity
        .and_then(|value| UidValidity::new(value).ok())
        .ok_or(MailProviderError::Protocol)?;

    // Reading further back is a different job: it takes the range it was given, moves no
    // cursor and states nothing about what disappeared — a range being read for the first
    // time has nothing to compare against.
    let older = match &request.intent {
        SyncIntent::Older(range) => Some(range.clone()),
        SyncIntent::Forward => None,
    };

    let stored = match &older {
        Some(_) => None,
        None => resume_from(request, generation, mailbox.uid_next),
    };

    // What the answers below name as gone, wherever in them it is said. Every one of them is about
    // this mailbox, the one open on the session while they are read.
    let mut heard_gone: Vec<RangeInclusive<u32>> = Vec::new();

    // The last number the server had issued when the mailbox was opened.
    let newest = mailbox.uid_next.and_then(|next| next.checked_sub(1));

    // How far a pass continuing from a position reads: to the newest number, or, where more numbers
    // than a window have been issued since, to the end of a window of the messages among them —
    // which only asking which of them are messages can say. The window ends at a message the
    // listing named, and the fetch below is what moves the cursor, so a listing that missed one
    // ends the window later rather than passing it.
    let through = match (&older, stored, newest) {
        (None, Some(last), Some(newest)) if newest > last => {
            Some(match needs_a_listing(last, newest) {
                false => newest,
                true => {
                    let listing = answer_to(
                        session,
                        &format!(
                            "UID FETCH {}:{newest} {NUMBERS_QUERY}",
                            last.saturating_add(1)
                        ),
                        MOST_A_PASS_ANSWER_HOLDS,
                        IfRefused::Nothing,
                    )
                    .await?;
                    heard_gone.extend(listing.vanished);

                    let listed: Vec<u32> =
                        listing.fetched.iter().filter_map(|line| line.uid).collect();
                    window_end(last, newest, &listed)
                }
            })
        }
        _ => None,
    };

    let range = match &older {
        Some(range) => format!("{}:{}", range.start().get(), range.end().get()),
        None => fetch_range(stored, through, mailbox.uid_next),
    };

    // How far down this pass looked. Recorded whether or not anything was there: a folder
    // whose mail all sits below the first window would otherwise look empty for ever.
    let mut covered = match &older {
        Some(range) => Some(*range.start()),
        None => covered_from(&range),
    };

    // The last number this pass asks about: the end of the range read further back, or reading
    // forwards the end of the window — for a first pass, the newest number there was.
    let up_to = match &older {
        Some(range) => Some(range.end().get()),
        None => through.or(newest),
    };

    // Not asked at all where the mailbox holds nothing newer. Reading further back is its own
    // job with its own range and always has something to ask for.
    //
    // Read to its last line, and a refusal accounted for message by message, or not at all:
    // messages missing from the middle of an answer that was cut short would be passed by a cursor
    // moved to the end of what was asked, and a range read further back would be recorded as read.
    let arrived = match older.is_some() || anything_newer(stored, mailbox.uid_next) {
        false => Vec::new(),
        true => {
            let answer = answer_to(
                session,
                &format!("UID FETCH {range} {PASS_QUERY}"),
                MOST_A_PASS_ANSWER_HOLDS,
                IfRefused::WhatArrived,
            )
            .await?;
            heard_gone.extend(answer.vanished);
            let lines = by_message(answer.fetched);

            // A first pass's newest message may sit below its floor, and the answer says when it
            // does: the reading starts at it rather than at the floor.
            let first_pass = older.is_none() && stored.is_none();
            if first_pass {
                covered = covered.map(|floor| first_floor(floor, &lines, describes));
            }
            let asked = asked_about(covered, up_to);

            match answer.refused {
                None => arrivals(&lines, stored, &asked, described),
                // Refused for a moment, which says nothing about any message in the range: kept
                // as rows of what the server knows, they would be mail described by nothing — no
                // name to be found by, so a copy the account already holds is a stranger — when
                // the next pass would describe them whole. Nothing is kept, and that pass asks.
                //
                // Once. A server may say it of the same message on every pass, and taken at its
                // word each time, the folder stops there for good: no new mail, no flag, no
                // removal, nothing older. Said again before a reading of this kind has been
                // applied, it is a refusal like any other.
                Some(Refusal::ForNow) if !request.refused_for_now_last_time => {
                    return Err(MailProviderError::RefusedForNow)
                }
                // `NO` after part of the answer, or before any of it: a server that would not
                // describe one message, or would not describe any. What arrived described is mail.
                // For the rest, the server is asked what it knows about each message without
                // reading it — and that answer has to be whole, because it is what says no message
                // in the range is left behind a position moved past it.
                Some(_) => {
                    let described_here: Vec<Fetched> =
                        lines.into_iter().filter(describes).collect();
                    let mut arrived = arrivals(&described_here, stored, &asked, described);

                    let listed = answer_to(
                        session,
                        &format!("UID FETCH {range} {FACTS_QUERY}"),
                        MOST_A_PASS_ANSWER_HOLDS,
                        IfRefused::Nothing,
                    )
                    .await?;
                    heard_gone.extend(listed.vanished);
                    let listed = by_message(listed.fetched);

                    // The newest message below a first pass's floor may be the one refused, and
                    // then only its facts say it is there: the reading starts at it all the same,
                    // and it is kept as a row like any other the server would not describe.
                    let asked = match first_pass {
                        true => {
                            covered =
                                covered.map(|floor| first_floor(floor, &listed, states_facts));
                            asked_about(covered, up_to)
                        }
                        false => asked,
                    };

                    arrived.extend(withheld(&listed, stored, &asked, &arrived));
                    arrived.sort_unstable_by_key(|message| message.uid);
                    arrived
                }
            }
        }
    };

    // What changed anywhere in the folder since the last pass, asked rather than looked
    // for. A server that keeps modification sequences answers this over the whole mailbox
    // for the cost of the messages that actually changed — and a folder where nothing did
    // costs nothing at all.
    let changed = match changed_since(request, mailbox.highest_modseq) {
        Some(modseq) => {
            // Whole or not at all, like the rest: every line here moves the position the next pass
            // asks from, so a change that did not arrive before a refusal would be passed for good.
            let answer = answer_to(
                session,
                &format!("UID FETCH 1:* {}", changed_since_query(modseq, vanishing)),
                MOST_A_PASS_ANSWER_HOLDS,
                IfRefused::Nothing,
            )
            .await?;
            heard_gone.extend(answer.vanished);

            answer.fetched
        }
        None => Vec::new(),
    };

    // What the server named as gone, asked for in the same breath as what changed.
    let named_as_gone = match vanishing {
        true => named_gone(&heard_gone, mailbox.uid_next),
        false => None,
    };

    // Whether anything disappeared, answered by a number the server gave when the mailbox
    // was opened. A mailbox holding as many messages as this folder holds, plus the ones
    // just read, has lost none — and there is nothing to look for.
    let missing = vanished_since(
        mailbox.exists,
        request.known_count,
        arrived.len(),
        older.is_some(),
    );

    let listing = match named_as_gone.is_some() {
        // The server named what went, so there is nothing to look for: no listing this pass. The
        // next pass counts again, and lists the mailbox if what was named did not explain it.
        true => None,
        // A listing is what removes mail, so only a whole one is a listing: an answer the server
        // refused halfway names some of the window, and the rest of it would go.
        false => match listing_range(request, generation, missing, mailbox.uid_next) {
            Some(window) => {
                let answer = answer_to(
                    session,
                    &format!(
                        "UID FETCH {}:{} {AUDIT_QUERY}",
                        window.start().get(),
                        window.end().get()
                    ),
                    MOST_A_PASS_ANSWER_HOLDS,
                    IfRefused::Nothing,
                )
                .await?;

                Some((window, answer.fetched))
            }
            None => None,
        },
    };

    let (mut flag_updates, vanished) = match (listing, named_as_gone) {
        // Named, and therefore exact: the flags came from asking what changed, which is the same
        // command that answered this.
        (_, Some(gone)) => (Vec::new(), VanishedReport::Exact(gone)),
        // A listing that came back empty is not proof of an empty mailbox, even one the server
        // completed with `OK`: a mailbox the server has just said holds messages cannot have
        // answered honestly with none. Believing it would propose removing the window.
        (Some((_, fetched)), None) if fetched.is_empty() && mailbox.exists > 0 => {
            (Vec::new(), VanishedReport::Unknown)
        }
        (Some((window, fetched)), None) => (flags_reported(&fetched), listed(&window, &fetched)),
        // Without a listing nothing is known about what disappeared, and `Unknown` is what
        // forbids this pass from deleting anything at all.
        (None, None) => (Vec::new(), VanishedReport::Unknown),
    };

    // Flags from outside the window, which the listing above cannot reach. The index
    // compares before it writes, so the two answers overlapping costs nothing.
    flag_updates.extend(flags_reported(&changed));

    // Reading backwards leaves the forward position exactly where it was: those messages
    // are older than everything the cursor already covers.
    //
    // Reading forwards moves it to the end of what was asked — the end of the window, or for a
    // first pass the newest number — because every message there was accounted for: the answer
    // came back whole, or refused and then listed whole. Every number in it was issued before the
    // mailbox was opened, so a number nobody named is a message that is gone rather than one still
    // to come. A pass that asked for nothing ends where it started. Nothing arrives from past the
    // end, so nothing moves the cursor past it.
    let highest = match &older {
        Some(_) => stored_uid(request).unwrap_or(0),
        None => arrived
            .iter()
            .map(|message| message.uid.get())
            .chain(stored)
            .chain(up_to)
            .max()
            .unwrap_or(0),
    };

    Ok(SyncBatch {
        scope: SyncScope::Mailbox,
        cursor_before: request.cursor.clone(),
        cursor_after: SyncCursor::new(highest.to_string()),
        per_mailbox: vec![FolderDelta {
            folder_id: request.folder_id,
            uid_validity: generation,
            covered_from: covered,
            // A window that ended before the newest number: the messages past it are for the
            // passes after this one.
            stopped_short: through
                .zip(newest)
                .is_some_and(|(end, newest)| end < newest),
            arrived,
            flag_updates,
            vanished,
        }],
    })
}

#[cfg(test)]
mod asking_only_when_there_is_something_to_ask {
    use super::anything_newer;

    #[test]
    fn a_mailbox_whose_next_number_is_the_one_after_the_last_held_has_nothing_new() {
        // The number a server gives is the one the next message will be given. Held up to 40 and
        // the next is 41 means the folder ends where this device does.
        assert!(!anything_newer(Some(40), Some(41)));
    }

    #[test]
    fn a_mailbox_whose_next_number_has_moved_has_something_new() {
        assert!(anything_newer(Some(40), Some(42)));
        assert!(anything_newer(Some(40), Some(9_000)));
    }

    #[test]
    fn a_folder_nothing_is_held_from_is_always_worth_asking() {
        // A first pass reads its window, and there is no number to compare against.
        assert!(anything_newer(None, Some(41)));
        assert!(anything_newer(None, None));
    }

    #[test]
    fn a_server_that_did_not_say_is_asked() {
        // Silence is not an answer of "nothing". Asking costs a round trip; assuming costs mail.
        assert!(anything_newer(Some(40), None));
    }

    #[test]
    fn a_number_at_the_top_of_its_range_does_not_wrap_into_saying_yes() {
        // `last + 1` overflowing would compare a small number against a large one and ask for a
        // range no message is in, every pass, for ever.
        assert!(!anything_newer(Some(u32::MAX), Some(u32::MAX)));
        assert!(!anything_newer(Some(u32::MAX), Some(1)));
    }
}
