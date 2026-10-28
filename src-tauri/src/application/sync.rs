// Wired to a command once there is a provider to sync from. Until then the module is reached only
// by its tests.
#![allow(dead_code)]

use crate::application::error::UseCaseError;
use crate::application::mailboxes::SyncMailboxes;
use crate::ports::account_repository::AccountRepository;
use crate::ports::credential::Credential;
use crate::ports::credential_store::{CredentialPurpose, CredentialRef, CredentialStore};
use crate::ports::mail_index::{AppliedSummary, MailIndex, Reading, UidScope};
use crate::ports::mail_provider::{
    MailFetch, MailProviderError, MailSync, ProviderFactory, SyncIntent, SyncRequest, Unread,
};
use crate::ports::message_bodies::MessageBodies;
use crate::ports::op_log::{OperationLog, OwedRead, WentTo};
use crate::ports::progress::Progress;
use lyonoyl_domain::message::{Flags, FolderId, MessageId, Uid};
use lyonoyl_domain::sync::{
    deletions_for, verify_uid_validity, Deletions, DeltaVerdict, FolderDelta, VanishedReport,
};

/// What one pass over one folder did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncOutcome {
    pub summary: AppliedSummary,
    /// The folder's UID space had been reset, so its placements were dropped and re-read.
    pub resynced: bool,
    /// Deletions the pass refused to apply by itself and left for the user to confirm.
    pub deletions_awaiting_confirmation: usize,
    /// Mail that arrived unread, and that is new to this person rather than new to this device.
    ///
    /// Zero on a folder being read for the first time and on one whose numbering was just thrown
    /// away: a mailbox arriving is not mail arriving. Somebody adding an account, or opening a
    /// build that dropped its index, has not been sent four thousand messages — and being told
    /// they have is the difference between a notification and an alarm.
    pub unseen_arrivals: u32,
    /// The pass read new mail only as far as one window, and more is waiting past it.
    pub stopped_short: bool,
}

/// Reads changes for one folder and applies them.
///
/// The order is fixed: read what is stored, ask the provider, decide what may be deleted, then
/// write everything and the cursor together.
pub struct SyncFolder<P: MailSync, I: MailIndex> {
    provider: P,
    index: I,
}

impl<P: MailSync, I: MailIndex> SyncFolder<P, I> {
    pub fn new(provider: P, index: I) -> Self {
        Self { provider, index }
    }

    pub fn execute(&self, folder_id: FolderId) -> Result<SyncOutcome, UseCaseError> {
        let state = self.index.folder_state(folder_id)?;

        let batch = asked_noting_a_moment(
            &self.provider,
            &self.index,
            Reading::NewMail,
            &SyncRequest {
                folder_id,
                intent: SyncIntent::Forward,
                mailbox: state.name.clone(),
                cursor: state.cursor.clone(),
                uid_validity: state.uid_validity,
                known_window: state.known_window.clone(),
                highest_modseq: state.highest_modseq,
                known_count: self.index.placement_count(folder_id)?,
                refused_for_now_last_time: state.new_mail_refused_for_now,
            },
        )?;

        let delta = batch
            .per_mailbox
            .iter()
            .find(|delta| delta.folder_id == folder_id)
            .ok_or(UseCaseError::UnexpectedFolder)?;

        let resynced = verify_uid_validity(state.uid_validity, delta.uid_validity)
            == DeltaVerdict::ResyncFolder;

        // Counted here, where both halves are in scope and neither has been thrown away yet: what
        // the folder knew before the pass, and what each arrival says about itself.
        let unseen_arrivals = if state.cursor.is_none() || resynced {
            0
        } else {
            delta
                .arrived
                .iter()
                .filter(|arrival| !arrival.envelope.flags.contains(Flags::SEEN))
                .count() as u32
        };

        if resynced {
            self.index.reset_folder(folder_id, delta.uid_validity)?;
        } else if batch.cursor_before != state.cursor {
            // The pass started from a position that is no longer ours, so its arrivals and
            // disappearances describe a state we never had.
            return Err(UseCaseError::StaleSyncBatch);
        }

        let (deletions, awaiting_confirmation) =
            self.decide_deletions(folder_id, delta, resynced)?;
        let summary = self.index.apply(
            delta,
            &deletions,
            &batch.cursor_after,
            self.provider.flag_scope(),
        )?;

        // How far down this pass looked, whether or not anything was there. A folder whose mail all
        // sits below the first window has nothing arrive and would otherwise never be read again.
        if let Some(floor) = delta.covered_from {
            self.index
                .record_window_floor(folder_id, floor, floor.get() == 1)?;
        }

        // Applied, so the moment a pass was refused for has passed, and the next refusal of that
        // kind is a moment of its own.
        if state.new_mail_refused_for_now {
            self.index
                .refused_for_now(folder_id, Reading::NewMail, false)?;
        }

        Ok(SyncOutcome {
            summary,
            resynced,
            deletions_awaiting_confirmation: awaiting_confirmation,
            unseen_arrivals,
            stopped_short: delta.stopped_short,
        })
    }

    /// Turns a report into the list of placements to remove, loading only the UIDs it concerns.
    fn decide_deletions(
        &self,
        folder_id: FolderId,
        delta: &FolderDelta,
        resynced: bool,
    ) -> Result<(Vec<Uid>, usize), UseCaseError> {
        if resynced {
            // The reset already dropped every placement of this folder.
            return Ok((Vec::new(), 0));
        }

        let scope = match &delta.vanished {
            VanishedReport::Unknown => return Ok((Vec::new(), 0)),
            VanishedReport::Exact(uids) => UidScope::Listed(uids.clone()),
            VanishedReport::AuthoritativeSet { window, .. } => UidScope::Window(window.clone()),
        };

        let known = self.index.known_uids(folder_id, &scope)?;
        let folder_size = self.index.placement_count(folder_id)?;

        Ok(match deletions_for(&delta.vanished, &known, folder_size) {
            Deletions::None => (Vec::new(), 0),
            Deletions::Remove(uids) => (uids, 0),
            Deletions::NeedsConfirmation { proposed, .. } => (Vec::new(), proposed.len()),
        })
    }
}

/// How many conversations with a server each phase of a pass opened.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct OpenedBy {
    /// Asking the server which mailboxes it has.
    pub discovery: u64,
    /// Reading what is new in each mailbox.
    pub folders: u64,
    /// Reading further back in each mailbox.
    pub backfill: u64,
    /// Reading messages whole.
    pub bodies: u64,
}

/// How long each phase of a pass took.
///
/// Beside the connections each opened, because they answer different questions and the second one
/// is the one that was missing: a phase that opens no connection of its own still sends commands
/// over the one it was handed, and every one of those is a wait on the network that nothing here
/// could see.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SpentOn {
    pub discovery: std::time::Duration,
    pub folders: std::time::Duration,
    pub backfill: std::time::Duration,
    pub bodies: std::time::Duration,
}

impl OpenedBy {
    /// Every conversation this pass opened, whatever it was for.
    pub fn altogether(self) -> u64 {
        self.discovery + self.folders + self.backfill + self.bodies
    }
}

/// What one pass over a whole account did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AccountPass {
    pub folders: u32,
    pub arrived: u32,
    pub updated: u32,
    pub removed: u32,
    /// Folders whose pass failed. The rest of the account still synchronised.
    pub folders_failed: u32,
    /// Folders whose first reading failed and that were read again at the end of the pass, whether
    /// or not the second reading went through.
    pub folders_read_again: u32,
    /// Folders that still have history below what has been read.
    pub folders_incomplete: u32,
    /// Folders whose UID space was thrown away and read again during this pass.
    pub folders_reset: u32,
    /// Messages this pass asked a server for whole and did not get.
    ///
    /// Beside the ones it did get, because the two answer different questions. A pass that read
    /// nothing because there was nothing to read and a pass that asked fifty times and came back
    /// empty look identical without this, and only one of them is a working application.
    pub bodies_failed: u32,
    /// Where this pass's conversations with the server went, phase by phase.
    pub opened: OpenedBy,
    /// How long this pass spent opening connections and signing in, out of its whole.
    pub opening_took: std::time::Duration,
    /// Where the rest of it went.
    pub spent: SpentOn,
    /// Mail nobody has seen, that arrived in this account's inbox during this pass.
    ///
    /// The inbox and nothing else. Mail appearing in Sent or Archive is usually the person's own
    /// filing coming back from the server, and being interrupted by one's own tidying is worse
    /// than not being told.
    pub unseen_in_inbox: u32,
    /// Instructions that were waiting in those folders and now have to be found again before they
    /// can be carried out.
    pub operations_to_reresolve: u32,
    /// Instructions a pass found again and put back in the queue.
    pub operations_found: u32,
    /// Moves and deletes the server had carried out, now read back into the index.
    pub read_back: u32,
    /// Messages read whole during this pass, so they open with no network and can be searched by
    /// what they say rather than by what they open with.
    pub bodies_read: u32,
}

/// Reads one account: which mailboxes it has, then what is in each of them.
///
/// The order and the bounds are the point. Mailboxes first, because a folder that is not recorded
/// cannot be read. Then one folder at a time, and a folder that fails does not stop the others: a
/// single mailbox a server refuses must not leave the rest of the account unread.
pub struct SyncAccount<R, C, F, I, L, B, P>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    I: MailIndex + Clone,
    L: OperationLog,
    B: MessageBodies + Clone,
    P: Progress,
{
    accounts: R,
    credentials: C,
    providers: F,
    /// Told how far this pass has got, while it is still getting there.
    progress: P,
    index: I,
    operations: L,
    bodies: B,
}

impl<R, C, F, I, L, B, P> SyncAccount<R, C, F, I, L, B, P>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    I: MailIndex + Clone,
    L: OperationLog,
    B: MessageBodies + Clone,
    P: Progress,
{
    pub fn new(
        accounts: R,
        credentials: C,
        providers: F,
        index: I,
        operations: L,
        bodies: B,
        progress: P,
    ) -> Self {
        Self {
            accounts,
            credentials,
            providers,
            progress,
            index,
            operations,
            bodies,
        }
    }

    /// `account_id` is the identifier the rest of the application uses; `number` is what the index
    /// knows the account by.
    pub fn execute(&self, account_id: &str, number: i64) -> Result<AccountPass, UseCaseError> {
        let began_pass = std::time::Instant::now();
        let (opened_before, opening_before) = self.providers.conversations();
        let account = self
            .accounts
            .find(account_id)?
            .ok_or(UseCaseError::UnknownAccount)?;

        // Read at the moment it is needed and handed straight to the adapter that sends it.
        let password = self.credentials.read(&CredentialRef::new(
            account_id.to_owned(),
            CredentialPurpose::ImapPassword,
        ))?;

        let recorded = SyncMailboxes::new(
            self.providers
                .discovery(&account, &Credential::password(password.clone()))?,
            self.index.clone(),
        )
        .execute(number)?;

        let mut pass = AccountPass {
            folders: recorded.folders.len() as u32,
            ..AccountPass::default()
        };

        // What the server has carried out and the index has not read back, asked before a single
        // folder is read. Anything carried out after this moment may have happened after its
        // folder was read, and settling it would take away the only thing that says where its
        // message went. A queue that cannot be asked is a pass that settles nothing, which the
        // next pass does instead.
        let owed = self.operations.owed_a_read(account_id).unwrap_or_default();
        let mut read_whole: Vec<FolderId> = Vec::new();
        let mut a_folder_failed = false;
        let mut a_folder_stopped_short = false;

        // What no folder held before this pass read anything, which is what a pass that then
        // cannot read every folder may forget. Asked before the first folder for the same reason
        // the queue is: after it, what lost its last copy while this pass read looks the same as
        // what had none to begin with.
        let unplaced_before: Option<std::collections::HashSet<_>> = self
            .index
            .unplaced(number)
            .ok()
            .map(|unplaced| unplaced.into_iter().collect());

        // Said before the reading rather than after it: a bar that learns the size of the job only
        // when the job is done is a bar that goes from nothing to full in one frame.
        self.progress.reading(account_id, pass.folders);

        // Where the conversations with the server go, phase by phase. `opened` on its own says a
        // pass cost sixty of them and not which sixty, and the three phases below are three
        // different defects if any of them is the one doing it.
        let mut spent = SpentOn::default();
        let mut opened = OpenedBy {
            // Everything before this point is the mailbox listing and nothing else.
            discovery: self.providers.conversations().0 - opened_before,
            ..OpenedBy::default()
        };
        spent.discovery = began_pass.elapsed();
        let mut clock = std::time::Instant::now();
        let took = |clock: &mut std::time::Instant| {
            let now = std::time::Instant::now();
            let spent = now - *clock;
            *clock = now;
            spent
        };

        let mut mark = self.providers.conversations().0;
        let since_mark = |mark: &mut u64| {
            let now = self.providers.conversations().0;
            let grew = now - *mark;
            *mark = now;
            grew
        };

        // The inbox first, whatever order the server listed its mailboxes in. Dovecot lists it
        // last, and every other server is free to: a first pass that fills Archive, Junk, Sent and
        // Drafts before it leaves a person watching an empty inbox while mail is visible in
        // folders they never opened. Stable, so the rest keep the order the server gave.
        let mut ordered: Vec<_> = recorded.folders.iter().map(|(_, folder)| *folder).collect();
        ordered.sort_by_key(|folder| Some(*folder) != recorded.inbox);

        // One provider for every folder, because a provider keeps its session: built inside the
        // loop it was a connection and a sign-in per mailbox, and against a server a second away
        // that was seven of them in a twelve-second pass.
        let reading = self
            .providers
            .sync(&account, &Credential::password(password.clone()))?;

        // The same again for reading messages whole, and it is a third connection rather than a
        // second: `MailSync` and `MailFetch` are two boxes even when they are one provider
        // underneath, and joining them is a change to the port rather than to this loop. It opens
        // nothing where nothing is waiting to be read whole — a provider signs in on its first use
        // — so a quiet account pays for two and an account with mail to fetch pays for three.
        let reading_bodies = self
            .providers
            .reading(&account, &Credential::password(password.clone()))
            .ok();

        // Every folder once, and one that fails once more at the end. What fails a reading is most
        // often a server that went quiet or a connection that broke, and a second reading on a
        // fresh connection a few seconds later goes through; left for the next round, mail that
        // arrived in the inbox waited for minutes and for as long again each time it failed.
        let mut queue: std::collections::VecDeque<(FolderId, bool)> =
            ordered.iter().map(|folder| (*folder, false)).collect();

        while let Some((folder, again)) = queue.pop_front() {
            match SyncFolder::new(&reading, self.index.clone()).execute(folder) {
                Ok(outcome) => {
                    // Read to its end, or only as far as one window of new mail. A folder that
                    // stopped short may hold, past where it stopped, exactly the message another
                    // folder just gave up: filed into it a moment ago, it has the newest number
                    // there is.
                    match outcome.stopped_short {
                        false => read_whole.push(folder),
                        true => a_folder_stopped_short = true,
                    }
                    pass.arrived += outcome.summary.added as u32;
                    pass.updated += outcome.summary.updated as u32;
                    pass.removed += outcome.summary.removed as u32;

                    // Only the inbox. `recorded.inbox` is computed on every pass and was read by
                    // nothing until now.
                    if recorded.inbox == Some(folder) {
                        pass.unseen_in_inbox += outcome.unseen_arrivals;
                    }

                    opened.folders += since_mark(&mut mark);

                    // The folder's UID space was thrown away and read again, so every position in
                    // it now names a different message. Anything waiting to be carried out there
                    // has to be found again before it is, and skipping that is how a deferred
                    // `\Deleted` lands on a stranger's message.
                    //
                    // A failure here is not a reason to abandon the pass — the mail is already
                    // applied — but it is a reason not to carry anything out, and the operations
                    // stay `pending` only until the next pass moves them.
                    if outcome.resynced {
                        pass.folders_reset += 1;
                        pass.operations_to_reresolve +=
                            self.operations
                                .folder_reset(account_id, folder)
                                .unwrap_or(0) as u32;
                    } else if !outcome.stopped_short {
                        // A pass that read the folder to its end without throwing its numbering
                        // away has just written where every message is. That is what anything sent
                        // back to be found again was waiting for, and nothing else was ever going
                        // to say so. One that stopped short has not: let go now, an instruction
                        // about a message past where it stopped finds no message and stops.
                        pass.operations_found += self
                            .operations
                            .folder_found(account_id, folder)
                            .unwrap_or(0) as u32;
                    }
                }
                Err(_) if !again => {
                    pass.folders_read_again += 1;
                    queue.push_back((folder, true));
                    continue;
                }
                Err(_) => {
                    pass.folders_failed += 1;
                    a_folder_failed = true;
                    // A folder whose last pass stopped short has not been seen to its end, and
                    // failing now it is still unseen: what another device filed into it past the
                    // window may be what lost its last copy, and nothing is forgotten while it may
                    // be. Once, and said so in the index — a folder that then fails on every pass
                    // must not keep every message the account ever deleted.
                    if self
                        .index
                        .folder_state(folder)
                        .is_ok_and(|state| state.stopped_short)
                    {
                        a_folder_stopped_short = true;
                        let _ = self.index.forget_it_stopped_short(folder);
                    }
                    // A folder this pass could not read is a folder it is finished with. Left
                    // uncounted, `folders_done` can never reach `folders`, and an account holding
                    // one mailbox its server refuses draws a bar that stops short of full and
                    // stays there for every round it ever runs.
                    self.progress.folder_read(account_id);
                    continue;
                }
            }

            // The forward pass ends here, and so does its share of the clock. Without this mark
            // every phase below is stamped with the one before it: the reading of whole messages
            // is recorded as backfill, and the bodies figure is about zero on every pass there is.
            spent.folders += took(&mut clock);

            // One round of older mail per folder per pass. Reading a whole mailbox in one go would
            // hold everything else for as long as the mailbox is large.
            match BackfillFolder::new(&reading, self.index.clone()).execute(folder) {
                Ok(round) => {
                    pass.arrived += round.summary.added as u32;
                    if !round.complete {
                        pass.folders_incomplete += 1;
                    }
                }
                Err(_) => pass.folders_failed += 1,
            }
            spent.backfill += took(&mut clock);
            opened.backfill += since_mark(&mut mark);

            // One round of reading messages whole. What the pass above brought is a description of
            // each message and its opening; this is the message. Last of the three because it is
            // the one nothing waits for: mail that has arrived is on the screen either way, and
            // this is what makes it open without a network and findable by what it says.
            if let Some(fetching) = reading_bodies.as_ref() {
                match FetchBodies::new(fetching, self.index.clone(), self.bodies.clone())
                    .execute(folder)
                {
                    Ok(round) => {
                        pass.bodies_read += round.fetched;
                        // Counted too. A pass that tried fifty times and got nothing looked
                        // exactly like a pass that had nothing to try, and the two are opposite
                        // things: one is a quiet account, the other is fifty conversations with a
                        // server that ended in nothing.
                        pass.bodies_failed += round.failed;
                        if !round.complete {
                            pass.folders_incomplete += 1;
                        }
                    }
                    Err(_) => pass.folders_failed += 1,
                }
            }
            spent.bodies += took(&mut clock);
            opened.bodies += since_mark(&mut mark);

            // One more done, said as it happens. Folders finish one after another while nine
            // accounts read at once, so this is the only thing in a round that moves steadily —
            // counted by mailbox, "how far" is nought for five seconds and then everything.
            self.progress.folder_read(account_id);
        }

        // What this pass has read both ends of, each to its end, and what the index now agrees
        // with, stops being laid over it. Both, because a folder can be read without a word about
        // what left it — a server that says only what changed and not what went — and settling on
        // the reading alone would put a filed message back where it was filed out of. To its end,
        // because a message filed a moment ago is the newest in the folder it went to, and a
        // folder read one window short of that has not seen it arrive.
        //
        // A folder the account no longer has is an end nothing will ever read, and waiting for it
        // would lay the instruction over the index for the life of the profile. So is one that
        // was carried out longer ago than anything done is kept: every pass since has had its
        // chance, and what the index says by now is what the server says, read or not.
        let known: Vec<FolderId> = recorded.folders.iter().map(|(_, folder)| *folder).collect();
        let settled = |folder: FolderId| read_whole.contains(&folder) || !known.contains(&folder);
        let now_read: Vec<_> = owed
            .iter()
            .filter(|one| {
                let both_ends = settled(one.from)
                    && match one.to {
                        WentTo::Folder(to) => settled(to),
                        // The folder the account keeps deleted mail in, as the server listed it
                        // this pass — not every folder there is: one mailbox the server refuses
                        // must not hold every delete of the account owed for ever.
                        WentTo::Trash => recorded.trash.is_none_or(settled),
                        WentTo::Nowhere => true,
                    };

                one.overdue
                    || (both_ends
                        && (!known.contains(&one.from)
                            || self.index.holds(one.from, one.message).ok()
                                == Some(left_holding(&owed, one, recorded.trash))))
            })
            .map(|one| one.id)
            .collect();
        pass.read_back = self.operations.read_back(&now_read).unwrap_or(0) as u32;

        // What waits to be found again in a folder the server no longer lists will never be found
        // there, and everything asked after it about the same message waits behind it.
        pass.operations_found += self
            .operations
            .folders_gone(account_id, &known)
            .unwrap_or(0) as u32;

        // A message whose last copy went while this pass read is forgotten now, and not before:
        // one moved between two folders is back under its own name by the time both are read to
        // their end, whichever was read first. Never one the queue still speaks about.
        //
        // Nothing goes while a folder has been read only as far as one window of new mail. The
        // message may be past where it stopped, and forgotten now it comes back under a new name,
        // its body read again, and whatever was asked about it meanwhile finds nothing. Each pass
        // takes a window more, so the wait ends. A folder that stopped short and then fails holds
        // the sweep back as well, for that one failing pass: it is no more read to its end than
        // it was.
        //
        // After a pass that could not read a folder, what lost its last copy during it is kept for
        // one more pass — the message may be in exactly that folder, and the next pass that reads
        // it finds it under its own name. What had no copy before this pass began has had a whole
        // pass to turn up, and goes. Kept for as long as any folder fails instead, it would stay
        // for ever on an account with a mailbox its server lists and will not open — its body on
        // disk and its words in every search, after it was deleted for good.
        if let Ok(mut keep) = self.operations.still_named(account_id) {
            let spared = match unplaced_before {
                _ if a_folder_stopped_short => None,
                _ if !a_folder_failed => Some(Vec::new()),
                Some(before) => self.index.unplaced(number).ok().map(|now| {
                    now.into_iter()
                        .filter(|message| !before.contains(message))
                        .collect()
                }),
                // Nothing says what had no copy before, so nothing may go.
                None => None,
            };
            if let Some(spared) = spared {
                keep.extend(spared);
                let _ = self.index.forget_the_unplaced(number, &keep);
            }
        }

        pass.opened = opened;
        pass.spent = spent;
        // How much of the pass was hello and goodbye, rather than mail. The count says how many
        // times; this says what they cost, which is the number that decides whether the connection
        // or the server is the slow part.
        pass.opening_took = self.providers.conversations().1 - opening_before;

        Ok(pass)
    }
}

/// Whether the moves this device made leave a message in the folder one of them took it from.
///
/// The last of them to touch that folder says: taken out of it, the message is not there; put
/// back into it — a way back, or a second move home — it is. Asked of the moves still owed a read,
/// oldest first, because together they are what the server did and the index is catching up with.
fn left_holding(owed: &[OwedRead], one: &OwedRead, trash: Option<FolderId>) -> bool {
    owed.iter()
        .filter(|other| other.message == one.message)
        .fold(false, |holds, other| {
            let into = match other.to {
                WentTo::Folder(to) => Some(to),
                WentTo::Trash => trash,
                WentTo::Nowhere => None,
            };
            if into == Some(one.from) {
                true
            } else if other.from == one.from {
                false
            } else {
                holds
            }
        })
}

/// How much older mail one round reads.
///
/// A round is one request and one transaction, so it is also the unit of "how long before something
/// appears on screen". Small enough to feel immediate, large enough that a mailbox of thousands is
/// not read in hundreds of round trips.
pub const BACKFILL_ROUND: u32 = 500;

/// How many messages one pass reads whole from one folder.
///
/// A round rather than the folder, for the same reason reading older mail is a round: a mailbox of
/// a hundred thousand read in one go holds every other account behind it for as long as that takes.
/// Small enough that a folder gives up its turn quickly, large enough that a mailbox fills in over
/// an evening rather than a month.
const BODIES_PER_ROUND: u32 = 50;

/// How many messages are asked for in one request.
///
/// Not all fifty at once: what comes back is held whole while it is stored, and a mailbox of
/// twenty-five-megabyte attachments would be fifty of them in memory together. Not one at a time
/// either — that is the shape this replaced, where the connection and the sign-in cost three times
/// what the message did. Ten is a request every fifth of the round with no more than ten messages
/// in hand at once.
const ASKED_FOR_AT_ONCE: usize = 10;

/// What one round of reading messages whole did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BodyRound {
    pub fetched: u32,
    pub failed: u32,
    /// Whether every message in the folder is now held whole.
    pub complete: bool,
}

/// Reads messages whole so they open without a network and can be searched by what they say.
///
/// The pass that lists a folder reads two kilobytes of each message, which is the opening and no
/// more. Everything past it — the rest of what was written, and every message written only in
/// markup — was on the server and nowhere else until somebody opened it one at a time.
///
/// Newest first: if only some of a mailbox can be here, it is the mail somebody is still dealing
/// with. A message that will not come is skipped and tried again next time rather than stopping
/// the round — one message a server refuses must not hold up the folder behind it.
pub struct FetchBodies<P: MailFetch, I: MailIndex, B: MessageBodies> {
    provider: P,
    index: I,
    bodies: B,
}

impl<P: MailFetch, I: MailIndex, B: MessageBodies> FetchBodies<P, I, B> {
    pub fn new(provider: P, index: I, bodies: B) -> Self {
        Self {
            provider,
            index,
            bodies,
        }
    }

    pub fn execute(&self, folder_id: FolderId) -> Result<BodyRound, UseCaseError> {
        let state = self.index.folder_state(folder_id)?;
        let waiting = self.index.not_read_whole(folder_id, BODIES_PER_ROUND)?;

        let mut round = BodyRound {
            complete: waiting.is_empty(),
            ..BodyRound::default()
        };

        // The generation the numbers below were stored in, which the server is asked to still be
        // counting in. A folder with none stored has numbers nothing can check, and reading by them
        // could keep somebody else's message as the body of one of these: nothing is asked, and
        // the folder stays one with messages still to read.
        let Some(uid_validity) = state.uid_validity else {
            return Ok(round);
        };

        // How many messages asked for alone have failed since the server last handed one over.
        let mut failed_alone_in_a_row = 0;

        'round: for batch in waiting.chunks(ASKED_FOR_AT_ONCE) {
            let uids: Vec<_> = batch.iter().map(|(_, uid)| *uid).collect();

            match self
                .provider
                .whole_messages(&state.name, uid_validity, &uids)
            {
                Ok(got) => {
                    let mut by_uid: std::collections::HashMap<_, _> = got.into_iter().collect();

                    for (message, uid) in batch {
                        // Paired back by number rather than by position. A server may answer about
                        // fewer than it was asked, and in any order; lining the answers up against
                        // the question would put one person's mail under another's subject.
                        match by_uid.remove(uid) {
                            Some(raw) => {
                                failed_alone_in_a_row = 0;
                                self.read_whole(*message, &raw, &mut round);
                            }
                            None => round.failed += 1,
                        }
                    }
                }
                // A server that cannot read one message may end the conversation rather than
                // answer with the rest — Dovecot does by default — and the messages asked for
                // beside it would then fail with it on every round, for ever. Asked for one at a
                // time, that message costs only itself, and each of the others has the room a
                // message has when a person opens it.
                //
                // For as long as the server can still be asked, and no longer. Whether a run of
                // messages before this one failed alone says nothing about the next: a damaged
                // file can hold ten of them in a row. Each is kept as it arrives rather than once
                // all have, so one request's worth of messages is all that is ever held at once.
                //
                // And only until more have failed alone in a row than one request names. Asked a
                // message at a time for every request of the round, a server that fails every read
                // — one that leaves over each, or goes silent once a message is on its way — would
                // be a request and a new connection for every message, and silent, a minute's wait
                // for each: fifty-five of them a folder, on every pass. A run longer than a request
                // is not one damaged message beside readable ones, and what is left waits for the
                // next round.
                Err(Unread::Failed(about_a_message))
                    if may_be_one(about_a_message) && uids.len() > 1 =>
                {
                    for (message, uid) in batch {
                        match self
                            .provider
                            .whole_messages(&state.name, uid_validity, &[*uid])
                        {
                            Ok(got) => match got.into_iter().find(|(one, _)| one == uid) {
                                Some((_, raw)) => {
                                    failed_alone_in_a_row = 0;
                                    self.read_whole(*message, &raw, &mut round);
                                }
                                None => round.failed += 1,
                            },
                            Err(Unread::Failed(failed)) if may_be_one(failed) => {
                                round.failed += 1;
                                failed_alone_in_a_row += 1;

                                if failed_alone_in_a_row > ASKED_FOR_AT_ONCE {
                                    round.failed += left_unread(&waiting, round);
                                    break 'round;
                                }
                            }
                            Err(_) => {
                                round.failed += left_unread(&waiting, round);
                                break 'round;
                            }
                        }
                    }
                }
                // The request failed rather than one message in it. The rest of the folder is not
                // written off for that: the next round asks again for exactly what is still
                // missing, because what is missing is what the index has not got.
                Err(Unread::Failed(failed)) if may_be_one(failed) => {
                    round.failed += batch.len() as u32;
                }
                // Nothing could be asked, or what was asked was refused for who was asking, and
                // the next request would be asked the same way: each is another connection tried
                // and given up on, or another refused sign-in counted toward locking the account.
                // The rest waits for the next round.
                Err(_) => {
                    round.failed += left_unread(&waiting, round);
                    break 'round;
                }
            }
        }

        Ok(round)
    }

    /// Keeps one message read whole and reads it for what it says.
    ///
    /// Kept first and read second. Keeping is what makes the next opening instant; reading it for
    /// what it says is what makes it findable, and a failure of either is not a reason to lose the
    /// other.
    fn read_whole(&self, message: MessageId, raw: &[u8], round: &mut BodyRound) {
        if self.bodies.keep(message, raw).is_err() {
            round.failed += 1;
            return;
        }
        let _ = self.bodies.fill_in_missing(message, raw);
        round.fetched += 1;
    }
}

/// How many of the messages a round set out to read it has not yet counted as read or failed.
fn left_unread(waiting: &[(MessageId, Uid)], round: BodyRound) -> u32 {
    (waiting.len() as u32).saturating_sub(round.fetched + round.failed)
}

/// Whether a request that went out and failed this way may have failed over one message in it: a
/// conversation ended in the middle, or an answer this build could not use. A refused sign-in and
/// a cancellation are about who is asking and whether to, never about a message.
fn may_be_one(failed: MailProviderError) -> bool {
    matches!(
        failed,
        MailProviderError::Offline | MailProviderError::Protocol
    )
}

/// What one round of reading further back did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackfillOutcome {
    pub summary: AppliedSummary,
    /// Whether the folder has now been read all the way back to its first message.
    pub complete: bool,
}

/// Reads one round of what is older than a folder has been read so far.
///
/// A forward pass only ever brings in what is newer than the cursor, so without this a folder shows
/// its newest few hundred messages and nothing else, for ever.
pub struct BackfillFolder<P: MailSync, I: MailIndex> {
    provider: P,
    index: I,
}

impl<P: MailSync, I: MailIndex> BackfillFolder<P, I> {
    pub fn new(provider: P, index: I) -> Self {
        Self { provider, index }
    }

    pub fn execute(&self, folder_id: FolderId) -> Result<BackfillOutcome, UseCaseError> {
        let state = self.index.folder_state(folder_id)?;

        let nothing = BackfillOutcome {
            summary: AppliedSummary::default(),
            complete: state.history_complete,
        };

        if state.history_complete {
            return Ok(nothing);
        }

        // How far back the folder has been read, which the forward pass records even when it found
        // nothing.
        let Some(floor) = state.window_floor else {
            // A folder no pass has looked at yet: the forward pass comes first, and it is what says
            // where the looking stopped.
            return Ok(nothing);
        };

        let Some(highest) = floor.get().checked_sub(1).filter(|uid| *uid > 0) else {
            // The floor is already the first UID a mailbox can have.
            self.index.record_window_floor(folder_id, floor, true)?;
            return Ok(BackfillOutcome {
                summary: AppliedSummary::default(),
                complete: true,
            });
        };

        let lowest = highest.saturating_sub(BACKFILL_ROUND - 1).max(1);
        let range = uid(lowest)?..=uid(highest)?;
        let reaches_the_beginning = lowest == 1;

        let mut batch = asked_noting_a_moment(
            &self.provider,
            &self.index,
            Reading::FurtherBack,
            &SyncRequest {
                folder_id,
                intent: SyncIntent::Older(range),
                mailbox: state.name.clone(),
                cursor: state.cursor.clone(),
                uid_validity: state.uid_validity,
                known_window: state.known_window.clone(),
                highest_modseq: state.highest_modseq,
                known_count: self.index.placement_count(folder_id)?,
                refused_for_now_last_time: state.further_back_refused_for_now,
            },
        )?;

        let delta = batch
            .per_mailbox
            .iter_mut()
            .find(|delta| delta.folder_id == folder_id)
            .ok_or(UseCaseError::UnexpectedFolder)?;

        // Reading further back says nothing about how far forward the folder has been read, and
        // what the index holds about that goes back to it as it was. Written as this round found
        // it, the last window's stopping short would be forgotten by the round after it.
        delta.stopped_short = state.stopped_short;

        if verify_uid_validity(state.uid_validity, delta.uid_validity) == DeltaVerdict::ResyncFolder
        {
            // The mailbox was renumbered while this was in flight. Reading further back into a
            // numbering that no longer exists would store messages under UIDs that address nothing;
            // the forward pass resets the folder, and the floor goes with it.
            return Ok(nothing);
        }

        // A round reads a range for the first time, so it says nothing about disappearances and
        // moves no cursor: `cursor_after` is what the folder already had.
        let summary =
            self.index
                .apply(delta, &[], &batch.cursor_after, self.provider.flag_scope())?;

        // Recorded whatever came back: the range has been read, and an empty one that left the
        // floor where it was would be asked for again on every round from now on.
        self.index
            .record_window_floor(folder_id, uid(lowest)?, reaches_the_beginning)?;

        // Applied, as the reading of new mail is: the next refusal is a moment of its own.
        if state.further_back_refused_for_now {
            self.index
                .refused_for_now(folder_id, Reading::FurtherBack, false)?;
        }

        Ok(BackfillOutcome {
            summary,
            complete: reaches_the_beginning,
        })
    }
}

fn uid(value: u32) -> Result<Uid, UseCaseError> {
    Uid::new(value).map_err(|_| UseCaseError::UnexpectedFolder)
}

/// Asks for one reading of a folder, and writes down a refusal the server says is for a moment.
///
/// A moment lasts until a reading of the same kind is applied. Written down, every reading of that
/// kind is told so until then, and a server that refuses it the same way again meanwhile is
/// answered as if it had refused it outright: what arrived is kept, and the rest is listed. Taken
/// at its word every time, a server that says it of one message on every pass stops the folder at
/// that message for good — the reading of new mail at it, or the reading further back at the
/// range holding it.
fn asked_noting_a_moment<P: MailSync, I: MailIndex>(
    provider: &P,
    index: &I,
    reading: Reading,
    request: &SyncRequest,
) -> Result<lyonoyl_domain::sync::SyncBatch, UseCaseError> {
    provider.fetch(request).map_err(|failed| {
        if failed == MailProviderError::RefusedForNow {
            // Not written down is asked the same way next pass, and refused the same way: one
            // more pass for the moment to pass in, which is all a failure here costs.
            let _ = index.refused_for_now(request.folder_id, reading, true);
        }
        failed.into()
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::ports::mail_index::{
        AppliedSummary, FolderSyncState, MailIndex, MailIndexError, UidScope,
    };
    use crate::ports::mail_provider::MailProviderError;
    use lyonoyl_domain::message::{FlagScope, Flags, ModSeq, UidValidity};
    use lyonoyl_domain::sync::{ArrivedMessage, MessageEnvelope, SyncBatch, SyncCursor, SyncScope};

    fn folder() -> FolderId {
        FolderId::new(1).expect("non-zero")
    }

    fn uid(value: u32) -> Uid {
        Uid::new(value).expect("non-zero")
    }

    fn generation(value: u32) -> UidValidity {
        UidValidity::new(value).expect("non-zero")
    }

    fn arrival(value: u32) -> ArrivedMessage {
        ArrivedMessage {
            uid: uid(value),
            modseq: Some(ModSeq::new(u64::from(value))),
            envelope: MessageEnvelope {
                rfc822_message_id: None,
                subject: Some("Subject".to_owned()),
                sender: Some("sender@example.com".to_owned()),
                sender_address: Some("sender@example.com".to_owned()),
                to: Vec::new(),
                cc: Vec::new(),
                received_at: 1_700_000_000,
                rfc822_size: Some(2048),
                preview: None,
                references: Vec::new(),
                flags: Flags::NONE,
            },
        }
    }

    fn delta(vanished: VanishedReport) -> FolderDelta {
        FolderDelta {
            folder_id: folder(),
            uid_validity: generation(7),
            covered_from: None,
            stopped_short: false,
            arrived: vec![arrival(10)],
            flag_updates: Vec::new(),
            vanished,
        }
    }

    fn batch(delta: FolderDelta, cursor_before: Option<&str>) -> SyncBatch {
        SyncBatch {
            scope: SyncScope::Mailbox,
            cursor_before: cursor_before.map(SyncCursor::new),
            cursor_after: SyncCursor::new("100"),
            per_mailbox: vec![delta],
        }
    }

    /// Everything an account pass needs, in the smallest form that still tells the truth.
    ///
    /// Purpose-built rather than grown from the fakes above: those describe one folder in
    /// isolation, and what is being watched here is a whole pass — which mailboxes it finds, and
    /// what it does with the queue of instructions when one of them resets.
    mod account_pass {
        use std::sync::{Arc, Mutex};

        use secrecy::SecretString;

        use super::*;
        use crate::ports::account_repository::AccountRepositoryError;
        use crate::ports::credential_store::CredentialError;
        use crate::ports::mail_index::{AppliedSummary, FolderRecord, FolderSyncState, UidScope};
        use crate::ports::mail_provider::{
            DiscoveredMailbox, MailFetch, MailProviderError, MailWatch, MailboxDiscovery,
        };
        use crate::ports::op_log::{OperationId, OperationLogError, RecordedOperation};
        use lyonoyl_domain::account::{
            Encryption, MailAccount, MailAccountSummary, ServerSettings,
        };
        use lyonoyl_domain::folder::FolderRole;
        use lyonoyl_domain::operation::{MailOperation, OperationState, OperationTarget};

        const FOLDER: u64 = 1;

        fn folder() -> FolderId {
            FolderId::new(FOLDER).expect("non-zero")
        }

        fn account() -> MailAccount {
            MailAccount::new(
                MailAccountSummary::new("acc-1", "user@example.com", "User", "imap")
                    .expect("valid summary"),
                ServerSettings::new("imap.example.com", 993, Encryption::Tls).expect("valid"),
                "user@example.com",
            )
            .expect("valid account")
        }

        struct Accounts;

        impl AccountRepository for Accounts {
            fn find(&self, _id: &str) -> Result<Option<MailAccount>, AccountRepositoryError> {
                Ok(Some(account()))
            }
            fn able_to_send(&self) -> Result<Vec<String>, AccountRepositoryError> {
                unreachable!("nothing here asks who can send")
            }

            fn list(&self) -> Result<Vec<MailAccountSummary>, AccountRepositoryError> {
                unreachable!("a pass over one account lists none")
            }
            fn find_by_email(&self, _e: &str) -> Result<Option<String>, AccountRepositoryError> {
                unreachable!("a pass looks nothing up by address")
            }
            fn save(&self, _a: &MailAccount) -> Result<(), AccountRepositoryError> {
                unreachable!("a pass stores no account")
            }
            fn delete(&self, _id: &str) -> Result<(), AccountRepositoryError> {
                unreachable!("a pass removes nothing")
            }
            fn set_collapsed(&self, _i: &str, _c: bool) -> Result<(), AccountRepositoryError> {
                unreachable!("a pass does not touch the sidebar")
            }

            fn number_of(&self, _: &str) -> Result<Option<i64>, AccountRepositoryError> {
                unreachable!("this fake is not asked which file the mail is in")
            }
        }

        struct Credentials;

        impl CredentialStore for Credentials {
            fn read(&self, _r: &CredentialRef) -> Result<SecretString, CredentialError> {
                Ok(SecretString::from("correct horse".to_owned()))
            }
            fn save(&self, _r: &CredentialRef, _s: &SecretString) -> Result<(), CredentialError> {
                unreachable!("a pass stores no secret")
            }
            fn delete(&self, _r: &CredentialRef) -> Result<(), CredentialError> {
                unreachable!("a pass removes no secret")
            }
        }

        /// An index holding one folder, whose generation is whatever the test says.
        #[derive(Clone)]
        struct Index {
            generation: UidValidity,
            /// What the server turned out to have, in the order it listed them.
            folders: Vec<(String, FolderId)>,
            /// Which folders were read, in the order the pass read them.
            visited: std::sync::Arc<Mutex<Vec<FolderId>>>,
            /// How long the reading of whole messages takes, so a test can tell one phase of a
            /// pass from another. Nought everywhere except where that is the subject.
            bodies_take: std::time::Duration,
            /// The copies still held once the pass has read, as `(folder, message)`.
            holding: Vec<(u64, u64)>,
            /// How many folders had been read when the pass forgot what no folder holds, and which
            /// messages it was told to keep.
            forgotten: std::sync::Arc<Mutex<Option<Swept>>>,
            /// What no folder holds, answered in turn each time the pass asks, and nothing once
            /// these have run out.
            unplaced: std::sync::Arc<Mutex<Vec<Vec<u64>>>>,
            /// Folders whose last pass, as the index has it, stopped short of their newest mail.
            recorded_short: Vec<u64>,
            /// Folders the pass told the index no longer to take as having stopped short.
            forgot_short: std::sync::Arc<Mutex<Vec<FolderId>>>,
        }

        /// How many folders had been read when the sweep ran, and the messages it was told to keep.
        type Swept = (usize, Vec<u64>);

        /// An account, and the folders a pass said it still has.
        type StillThere = (String, Vec<FolderId>);

        /// A store that keeps nothing: this pass is about what the index does, and the fake index
        /// says every message is already held.
        #[derive(Clone)]
        struct NoBodies;

        /// The provider a round of reading messages whole gets. It is asked for one and never
        /// used, because the fake index above answers that nothing is waiting.
        struct NothingLeftToRead;

        impl MailFetch for NothingLeftToRead {
            fn whole_message(
                &self,
                _mailbox: &str,
                _uid_validity: UidValidity,
                _uid: Uid,
            ) -> Result<Vec<u8>, MailProviderError> {
                unreachable!("the index says every message is already here")
            }
        }

        impl crate::ports::message_bodies::MessageBodies for NoBodies {
            fn held(
                &self,
                _message: lyonoyl_domain::message::MessageId,
            ) -> Result<Option<Vec<u8>>, crate::ports::message_bodies::MessageBodyError>
            {
                Ok(None)
            }

            fn keep(
                &self,
                _message: lyonoyl_domain::message::MessageId,
                _raw: &[u8],
            ) -> Result<(), crate::ports::message_bodies::MessageBodyError> {
                Ok(())
            }

            fn fill_in_missing(
                &self,
                _message: lyonoyl_domain::message::MessageId,
                _raw: &[u8],
            ) -> Result<(), crate::ports::message_bodies::MessageBodyError> {
                Ok(())
            }
        }

        impl MailIndex for Index {
            fn not_read_whole(
                &self,
                _folder_id: FolderId,
                _at_most: u32,
            ) -> Result<Vec<(lyonoyl_domain::message::MessageId, Uid)>, MailIndexError>
            {
                // Asked once per folder by the round that reads messages whole, so this is inside
                // the bodies phase and nowhere else.
                std::thread::sleep(self.bodies_take);
                Ok(Vec::new())
            }
            fn record_folders(
                &self,
                _account: i64,
                _folders: &[FolderRecord],
            ) -> Result<Vec<(String, FolderId)>, MailIndexError> {
                Ok(self.folders.clone())
            }

            fn folder_state(&self, folder: FolderId) -> Result<FolderSyncState, MailIndexError> {
                // Where the order a pass takes is recorded: this is the first thing it asks about
                // a folder, so the order these arrive in is the order the folders are read in.
                self.visited.lock().expect("test lock").push(folder);

                Ok(FolderSyncState {
                    name: self
                        .folders
                        .iter()
                        .find(|(_, id)| *id == folder)
                        .map(|(name, _)| name.clone())
                        .unwrap_or_else(|| "INBOX".to_owned()),
                    uid_validity: Some(self.generation),
                    cursor: None,
                    known_window: None,
                    highest_modseq: None,
                    history_complete: true,
                    window_floor: None,
                    stopped_short: self.recorded_short.contains(&folder.get()),
                    new_mail_refused_for_now: false,
                    further_back_refused_for_now: false,
                })
            }

            fn forget_it_stopped_short(&self, folder: FolderId) -> Result<(), MailIndexError> {
                self.forgot_short.lock().expect("test lock").push(folder);
                Ok(())
            }

            fn refused_for_now(
                &self,
                _folder: FolderId,
                _reading: Reading,
                _refused: bool,
            ) -> Result<(), MailIndexError> {
                unreachable!("nothing these passes read is refused for a moment")
            }

            fn inbox_of(&self, _a: i64) -> Result<Option<String>, MailIndexError> {
                Ok(Some("INBOX".to_owned()))
            }

            fn known_uids(&self, _f: FolderId, _s: &UidScope) -> Result<Vec<Uid>, MailIndexError> {
                Ok(Vec::new())
            }

            fn placement_count(&self, _f: FolderId) -> Result<usize, MailIndexError> {
                Ok(0)
            }

            fn record_window_floor(
                &self,
                _f: FolderId,
                _floor: Uid,
                _complete: bool,
            ) -> Result<(), MailIndexError> {
                Ok(())
            }

            fn apply(
                &self,
                _d: &FolderDelta,
                _deletions: &[Uid],
                _c: &SyncCursor,
                _s: FlagScope,
            ) -> Result<AppliedSummary, MailIndexError> {
                Ok(AppliedSummary::default())
            }

            fn reset_folder(&self, _f: FolderId, _g: UidValidity) -> Result<(), MailIndexError> {
                Ok(())
            }

            fn holds(
                &self,
                folder: FolderId,
                message: lyonoyl_domain::message::MessageId,
            ) -> Result<bool, MailIndexError> {
                Ok(self.holding.contains(&(folder.get(), message.get())))
            }

            fn unplaced(
                &self,
                _account: i64,
            ) -> Result<Vec<lyonoyl_domain::message::MessageId>, MailIndexError> {
                let mut answers = self.unplaced.lock().expect("test lock");
                let answer = if answers.is_empty() {
                    Vec::new()
                } else {
                    answers.remove(0)
                };
                Ok(answer
                    .into_iter()
                    .map(|id| lyonoyl_domain::message::MessageId::new(id).expect("non-zero"))
                    .collect())
            }

            fn forget_the_unplaced(
                &self,
                _account: i64,
                named: &[lyonoyl_domain::message::MessageId],
            ) -> Result<(), MailIndexError> {
                *self.forgotten.lock().expect("test lock") = Some((
                    self.visited.lock().expect("test lock").len(),
                    named.iter().map(|message| message.get()).collect(),
                ));
                Ok(())
            }
        }

        /// A server whose folder carries the generation the test chose.
        struct Server {
            generation: UidValidity,
            /// Folders it will not let the pass read.
            refusing: Vec<FolderId>,
            /// Folders it will not let the pass read the first time it is asked, and lets it read
            /// after that: a server that went quiet once.
            failing_once: Mutex<Vec<FolderId>>,
            /// Folders whose pass reads only as far as one window of new mail.
            stopping_short: Vec<FolderId>,
            /// How many times each folder has been asked for. A pass that would ask for one for
            /// ever fails the test that runs it, rather than leaving it running.
            asked: Mutex<Vec<FolderId>>,
        }

        impl MailSync for Server {
            fn flag_scope(&self) -> FlagScope {
                FlagScope::Message
            }

            fn fetch(&self, request: &SyncRequest) -> Result<SyncBatch, MailProviderError> {
                let mut asked = self.asked.lock().expect("test lock");
                asked.push(request.folder_id);
                let times = asked
                    .iter()
                    .filter(|folder| **folder == request.folder_id)
                    .count();
                assert!(
                    times <= 10,
                    "one folder was asked for {times} times in one pass"
                );
                drop(asked);
                if self.refusing.contains(&request.folder_id) {
                    return Err(MailProviderError::Protocol);
                }
                let mut failing = self.failing_once.lock().expect("test lock");
                if let Some(at) = failing
                    .iter()
                    .position(|folder| *folder == request.folder_id)
                {
                    failing.remove(at);
                    return Err(MailProviderError::Protocol);
                }
                drop(failing);

                Ok(SyncBatch {
                    scope: SyncScope::Mailbox,
                    per_mailbox: vec![FolderDelta {
                        folder_id: request.folder_id,
                        uid_validity: self.generation,
                        covered_from: None,
                        stopped_short: self.stopping_short.contains(&request.folder_id),
                        arrived: Vec::new(),
                        flag_updates: Vec::new(),
                        vanished: VanishedReport::Exact(Vec::new()),
                    }],
                    cursor_before: request.cursor.clone(),
                    cursor_after: SyncCursor::new("after"),
                })
            }
        }

        struct Mailboxes {
            named: Vec<String>,
        }

        impl MailboxDiscovery for Mailboxes {
            fn list_mailboxes(&self) -> Result<Vec<DiscoveredMailbox>, MailProviderError> {
                Ok(self
                    .named
                    .iter()
                    .map(|name| DiscoveredMailbox {
                        name: name.clone(),
                        role: match name.as_str() {
                            "INBOX" => FolderRole::Inbox,
                            "Trash" => FolderRole::Trash,
                            _ => FolderRole::Custom,
                        },
                        uid_validity: None,
                        uid_next: None,
                    })
                    .collect())
            }
        }

        /// Everything the pass said about how far it had got, in the order it said it.
        #[derive(Default)]
        struct Watching {
            said: Mutex<Vec<String>>,
        }

        impl crate::ports::progress::Progress for &Watching {
            fn reading(&self, account_id: &str, folders: u32) {
                self.said
                    .lock()
                    .expect("test lock")
                    .push(format!("{account_id} has {folders}"));
            }

            fn folder_read(&self, account_id: &str) {
                self.said
                    .lock()
                    .expect("test lock")
                    .push(format!("{account_id} read one"));
            }
        }

        struct Factory {
            generation: UidValidity,
            named: Vec<String>,
            /// How many providers this pass asked for, by kind.
            ///
            /// A provider keeps its connection, so this is how many connections the pass costs:
            /// asked for inside the folder loop it was one per mailbox, and against a server a
            /// second away that was most of a twelve-second pass spent saying hello.
            built: std::sync::Arc<Mutex<Vec<&'static str>>>,
            refusing: Vec<FolderId>,
            failing_once: Vec<FolderId>,
            stopping_short: Vec<FolderId>,
        }

        impl ProviderFactory for Factory {
            fn sync(
                &self,
                _a: &MailAccount,
                _p: &Credential,
            ) -> Result<Box<dyn MailSync>, MailProviderError> {
                self.built.lock().expect("not poisoned").push("sync");
                Ok(Box::new(Server {
                    generation: self.generation,
                    refusing: self.refusing.clone(),
                    failing_once: Mutex::new(self.failing_once.clone()),
                    stopping_short: self.stopping_short.clone(),
                    asked: Mutex::new(Vec::new()),
                }))
            }

            fn discovery(
                &self,
                _a: &MailAccount,
                _p: &Credential,
            ) -> Result<Box<dyn MailboxDiscovery>, MailProviderError> {
                self.built.lock().expect("not poisoned").push("discovery");
                Ok(Box::new(Mailboxes {
                    named: self.named.clone(),
                }))
            }

            fn reading(
                &self,
                _a: &MailAccount,
                _p: &Credential,
            ) -> Result<Box<dyn MailFetch>, MailProviderError> {
                self.built.lock().expect("not poisoned").push("reading");
                Ok(Box::new(NothingLeftToRead))
            }

            fn watch(
                &self,
                _a: &MailAccount,
                _p: &Credential,
            ) -> Result<Box<dyn MailWatch>, MailProviderError> {
                unreachable!("a pass watches nothing")
            }
        }

        /// A queue that remembers which folders it was told about.
        #[derive(Default, Clone)]
        struct Queue {
            told: Arc<Mutex<Vec<(String, FolderId)>>>,
            /// Folders a pass said it had read, which is what puts instructions back in the queue.
            found: Arc<Mutex<Vec<(String, FolderId)>>>,
            waiting: usize,
            /// What the server has carried out and no pass has read back.
            owed: Vec<crate::ports::op_log::OwedRead>,
            /// What the pass said it had read back.
            settled: Arc<Mutex<Vec<OperationId>>>,
            owed_asked_after: Arc<Mutex<Option<usize>>>,
            /// Messages the queue still speaks about.
            named: Vec<u64>,
            /// The folders a pass said the account still has, by account.
            gone_asked: Arc<Mutex<Vec<StillThere>>>,
        }

        impl OperationLog for Queue {
            fn cancel(
                &self,
                _account_id: &str,
                _target: OperationTarget,
            ) -> Result<usize, OperationLogError> {
                unreachable!("a pass takes nothing back")
            }

            fn try_again(
                &self,
                _account_id: &str,
                _target: OperationTarget,
            ) -> Result<usize, OperationLogError> {
                unreachable!("a pass does not put anything back")
            }

            fn settled_for(
                &self,
                _account_id: &str,
                _target: OperationTarget,
            ) -> Result<Option<RecordedOperation>, OperationLogError> {
                Ok(None)
            }

            fn outstanding_for(
                &self,
                _a: &str,
                _t: OperationTarget,
            ) -> Result<Vec<RecordedOperation>, OperationLogError> {
                unreachable!("a pass folds nothing")
            }

            fn record(
                &self,
                _a: &str,
                _r: &[OperationId],
                _o: Option<MailOperation>,
            ) -> Result<u64, OperationLogError> {
                unreachable!("a pass asks for nothing")
            }

            fn due(
                &self,
                _a: &str,
                _at_most: usize,
            ) -> Result<Vec<RecordedOperation>, OperationLogError> {
                unreachable!("a pass carries nothing out")
            }

            fn about_to_tell(&self, _id: OperationId) -> Result<(), OperationLogError> {
                Ok(())
            }

            fn release(&self, _ids: &[OperationId]) -> Result<(), OperationLogError> {
                unreachable!("a pass hands nothing out")
            }

            fn attempted(
                &self,
                _id: OperationId,
                _became: OperationState,
                _stopped: Option<lyonoyl_domain::operation::Stopped>,
                _retry: std::time::Duration,
            ) -> Result<(), OperationLogError> {
                unreachable!("a pass tells no server about an instruction")
            }

            fn folder_reset(
                &self,
                account_id: &str,
                folder: FolderId,
            ) -> Result<usize, OperationLogError> {
                self.told
                    .lock()
                    .expect("test lock")
                    .push((account_id.to_owned(), folder));

                Ok(self.waiting)
            }

            fn folder_found(
                &self,
                account_id: &str,
                folder: FolderId,
            ) -> Result<usize, OperationLogError> {
                self.found
                    .lock()
                    .expect("test lock")
                    .push((account_id.to_owned(), folder));

                Ok(self.waiting)
            }

            fn owed_a_read(
                &self,
                _account_id: &str,
            ) -> Result<Vec<crate::ports::op_log::OwedRead>, OperationLogError> {
                // How many folders had been read when the pass asked, which has to be none: what
                // becomes owed while the pass is reading was carried out after the reading began.
                *self.owed_asked_after.lock().expect("test lock") =
                    Some(self.found.lock().expect("test lock").len());
                Ok(self.owed.clone())
            }

            fn read_back(&self, ids: &[OperationId]) -> Result<usize, OperationLogError> {
                self.settled
                    .lock()
                    .expect("test lock")
                    .extend_from_slice(ids);
                Ok(ids.len())
            }

            fn owes_a_read(
                &self,
                _account_id: &str,
                _message: lyonoyl_domain::message::MessageId,
            ) -> Result<bool, OperationLogError> {
                unreachable!("a pass looks for no message")
            }

            fn still_named(
                &self,
                _account_id: &str,
            ) -> Result<Vec<lyonoyl_domain::message::MessageId>, OperationLogError> {
                Ok(self
                    .named
                    .iter()
                    .map(|message| {
                        lyonoyl_domain::message::MessageId::new(*message).expect("non-zero")
                    })
                    .collect())
            }

            fn folders_gone(
                &self,
                account_id: &str,
                still_there: &[FolderId],
            ) -> Result<usize, OperationLogError> {
                self.gone_asked
                    .lock()
                    .expect("test lock")
                    .push((account_id.to_owned(), still_there.to_vec()));
                Ok(0)
            }
        }

        fn pass_over(stored: u32, reported: u32, waiting: usize) -> (AccountPass, Queue) {
            let (pass, queue, _, _, _) = pass_over_mailboxes(stored, reported, waiting, &["INBOX"]);
            (pass, queue)
        }

        /// A pass over the mailboxes named, answering with the order the folders were read in.
        fn pass_over_mailboxes(
            stored: u32,
            reported: u32,
            waiting: usize,
            named: &[&str],
        ) -> (AccountPass, Queue, Vec<String>, Vec<&'static str>, Watching) {
            pass_over_mailboxes_taking(stored, reported, waiting, named, std::time::Duration::ZERO)
        }

        /// The same, with the reading of whole messages made to cost something.
        fn pass_over_mailboxes_taking(
            stored: u32,
            reported: u32,
            waiting: usize,
            named: &[&str],
            bodies_take: std::time::Duration,
        ) -> (AccountPass, Queue, Vec<String>, Vec<&'static str>, Watching) {
            pass_over_owing(
                stored,
                reported,
                Queue {
                    waiting,
                    ..Queue::default()
                },
                named,
                bodies_take,
                &[],
            )
        }

        /// The same, with a queue the test built and folders the server refuses to let be read.
        fn pass_over_owing(
            stored: u32,
            reported: u32,
            queue: Queue,
            named: &[&str],
            bodies_take: std::time::Duration,
            refusing: &[u64],
        ) -> (AccountPass, Queue, Vec<String>, Vec<&'static str>, Watching) {
            pass_over_holding(stored, reported, queue, named, bodies_take, refusing, &[]).0
        }

        /// The same, with copies the index still holds after the pass — `(folder, message)` —
        /// and how many folders had been read when the pass forgot what no folder holds.
        #[allow(clippy::type_complexity)]
        fn pass_over_holding(
            stored: u32,
            reported: u32,
            queue: Queue,
            named: &[&str],
            bodies_take: std::time::Duration,
            refusing: &[u64],
            holding: &[(u64, u64)],
        ) -> (
            (AccountPass, Queue, Vec<String>, Vec<&'static str>, Watching),
            Option<Swept>,
        ) {
            pass_finding_unplaced(
                stored,
                reported,
                queue,
                named,
                bodies_take,
                refusing,
                holding,
                Vec::new(),
            )
        }

        /// The same pass, over an index that answers `unplaced` in turn when asked what no folder
        /// holds: before the pass reads, and again after.
        #[allow(clippy::too_many_arguments, clippy::type_complexity)]
        fn pass_finding_unplaced(
            stored: u32,
            reported: u32,
            queue: Queue,
            named: &[&str],
            bodies_take: std::time::Duration,
            refusing: &[u64],
            holding: &[(u64, u64)],
            unplaced: Vec<Vec<u64>>,
        ) -> (
            (AccountPass, Queue, Vec<String>, Vec<&'static str>, Watching),
            Option<Swept>,
        ) {
            pass_stopping_short(
                stored,
                reported,
                queue,
                named,
                bodies_take,
                refusing,
                holding,
                unplaced,
                &[],
            )
        }

        /// The same, with folders whose pass reads only as far as one window of new mail.
        #[allow(clippy::too_many_arguments, clippy::type_complexity)]
        fn pass_stopping_short(
            stored: u32,
            reported: u32,
            queue: Queue,
            named: &[&str],
            bodies_take: std::time::Duration,
            refusing: &[u64],
            holding: &[(u64, u64)],
            unplaced: Vec<Vec<u64>>,
            stopping_short: &[u64],
        ) -> (
            (AccountPass, Queue, Vec<String>, Vec<&'static str>, Watching),
            Option<Swept>,
        ) {
            let (pass, forgotten, _) = pass_after_one_that_stopped_short(
                stored,
                reported,
                queue,
                named,
                bodies_take,
                refusing,
                holding,
                unplaced,
                stopping_short,
                &[],
                &[],
            );
            (pass, forgotten)
        }

        /// The same, over an index that says the last pass over `recorded_short` stopped short,
        /// answering also with the folders the pass told it no longer to say so about.
        #[allow(clippy::too_many_arguments, clippy::type_complexity)]
        fn pass_after_one_that_stopped_short(
            stored: u32,
            reported: u32,
            queue: Queue,
            named: &[&str],
            bodies_take: std::time::Duration,
            refusing: &[u64],
            holding: &[(u64, u64)],
            unplaced: Vec<Vec<u64>>,
            stopping_short: &[u64],
            recorded_short: &[u64],
            failing_once: &[u64],
        ) -> (
            (AccountPass, Queue, Vec<String>, Vec<&'static str>, Watching),
            Option<Swept>,
            Vec<FolderId>,
        ) {
            let forgotten = std::sync::Arc::new(Mutex::new(None));
            let forgot_short = std::sync::Arc::new(Mutex::new(Vec::new()));
            let folders: Vec<(String, FolderId)> = named
                .iter()
                .enumerate()
                .map(|(at, name)| {
                    (
                        (*name).to_owned(),
                        FolderId::new(at as u64 + 1).expect("non-zero"),
                    )
                })
                .collect();
            let visited = std::sync::Arc::new(Mutex::new(Vec::new()));
            let built = std::sync::Arc::new(Mutex::new(Vec::new()));
            let told = Watching::default();

            let pass = SyncAccount::new(
                Accounts,
                Credentials,
                Factory {
                    generation: generation(reported),
                    named: named.iter().map(|name| (*name).to_owned()).collect(),
                    built: built.clone(),
                    refusing: refusing
                        .iter()
                        .map(|id| FolderId::new(*id).expect("non-zero"))
                        .collect(),
                    failing_once: failing_once
                        .iter()
                        .map(|id| FolderId::new(*id).expect("non-zero"))
                        .collect(),
                    stopping_short: stopping_short
                        .iter()
                        .map(|id| FolderId::new(*id).expect("non-zero"))
                        .collect(),
                },
                Index {
                    generation: generation(stored),
                    folders: folders.clone(),
                    visited: visited.clone(),
                    bodies_take,
                    holding: holding.to_vec(),
                    forgotten: forgotten.clone(),
                    unplaced: std::sync::Arc::new(Mutex::new(unplaced)),
                    recorded_short: recorded_short.to_vec(),
                    forgot_short: forgot_short.clone(),
                },
                queue.clone(),
                NoBodies,
                &told,
            )
            .execute("acc-1", 1)
            .expect("the pass runs");

            // One folder is asked about more than once while it is being read. What is under test
            // is the order they are taken in, so a run of the same name counts as one visit.
            let mut read: Vec<String> = Vec::new();
            for id in visited.lock().expect("test lock").iter() {
                let name = folders
                    .iter()
                    .find(|(_, folder)| folder == id)
                    .map(|(name, _)| name.clone())
                    .unwrap_or_default();
                if read.last() != Some(&name) {
                    read.push(name);
                }
            }

            let built = built.lock().expect("test lock").clone();
            let forgotten = forgotten.lock().expect("test lock").clone();
            let forgot_short = forgot_short.lock().expect("test lock").clone();
            ((pass, queue, read, built, told), forgotten, forgot_short)
        }

        /// A move or a delete of a message of its own, owed a read, asked for a moment ago.
        fn owed(id: i64, from: u64, to: WentTo) -> crate::ports::op_log::OwedRead {
            crate::ports::op_log::OwedRead {
                id: OperationId(id),
                message: lyonoyl_domain::message::MessageId::new(id as u64).expect("non-zero"),
                from: FolderId::new(from).expect("non-zero"),
                to,
                overdue: false,
            }
        }

        fn to_folder(id: u64) -> WentTo {
            WentTo::Folder(FolderId::new(id).expect("non-zero"))
        }

        fn settled_by(pass: &AccountPass, queue: &Queue) -> Vec<i64> {
            let settled: Vec<i64> = queue
                .settled
                .lock()
                .expect("test lock")
                .iter()
                .map(|id| id.0)
                .collect();
            assert_eq!(pass.read_back as usize, settled.len());
            settled
        }

        #[test]
        fn what_is_owed_a_read_is_asked_before_any_folder_is_read() {
            // Carried out after the pass began, a move may have reached the server after its folder
            // was read -- and settling it then takes away the only thing saying where it went.
            let (_, queue, _, _, _) = pass_over_owing(
                7,
                7,
                Queue::default(),
                &["INBOX", "Archive"],
                std::time::Duration::ZERO,
                &[],
            );

            assert_eq!(*queue.owed_asked_after.lock().expect("test lock"), Some(0));
        }

        #[test]
        fn a_move_is_read_back_once_both_its_folders_have_been_read() {
            let queue = Queue {
                owed: vec![owed(1, 1, to_folder(2))],
                ..Queue::default()
            };

            let (pass, queue, _, _, _) = pass_over_owing(
                7,
                7,
                queue,
                &["INBOX", "Archive"],
                std::time::Duration::ZERO,
                &[],
            );

            assert_eq!(settled_by(&pass, &queue), vec![1]);
        }

        #[test]
        fn what_no_folder_holds_is_forgotten_only_after_every_folder_is_read() {
            // A message is known here by its name, found again by its `Message-ID` only while the
            // index still has it. Forgotten as its last copy goes, a message moved from one folder
            // to another -- by this device or anybody -- comes back under a new name whenever the
            // folder it left is read first, and the way back somebody asked for a moment ago
            // names nothing. Two moves crossing each other mean one of them always is.
            let queue = Queue {
                owed: vec![owed(1, 1, to_folder(2)), owed(2, 2, to_folder(1))],
                ..Queue::default()
            };

            let ((_, _, read, _, _), forgotten) = pass_over_holding(
                7,
                7,
                queue,
                &["INBOX", "Archive", "Trash"],
                std::time::Duration::ZERO,
                &[],
                &[],
            );

            assert_eq!(read, vec!["INBOX", "Archive", "Trash"]);
            assert!(
                forgotten.as_ref().is_some_and(|(after, _)| *after >= 3),
                "forgotten before the last folder was read: {forgotten:?}"
            );
        }

        #[test]
        fn a_move_is_still_owed_while_the_index_holds_the_copy_it_took_away() {
            // A folder can be read without a word about what left it: a server that tracks changes
            // but does not name what went, with most of the mailbox not read yet. The copy the
            // move took stays in the index, and settling now would put the message back in the
            // folder the person filed it out of.
            let queue = Queue {
                owed: vec![owed(1, 1, to_folder(2))],
                ..Queue::default()
            };

            let ((pass, queue, _, _, _), _) = pass_over_holding(
                7,
                7,
                queue,
                &["INBOX", "Archive"],
                std::time::Duration::ZERO,
                &[],
                &[(1, 1), (2, 1)],
            );

            assert!(settled_by(&pass, &queue).is_empty());
        }

        #[test]
        fn a_move_and_its_way_back_are_settled_together_once_the_message_is_home() {
            // Archived and put back before a pass read either: the index then holds it in the
            // inbox again, which is where the two moves together left it.
            let queue = Queue {
                owed: vec![
                    crate::ports::op_log::OwedRead {
                        message: lyonoyl_domain::message::MessageId::new(5).expect("non-zero"),
                        ..owed(1, 1, to_folder(2))
                    },
                    crate::ports::op_log::OwedRead {
                        message: lyonoyl_domain::message::MessageId::new(5).expect("non-zero"),
                        ..owed(2, 2, to_folder(1))
                    },
                ],
                ..Queue::default()
            };

            let ((pass, queue, _, _, _), _) = pass_over_holding(
                7,
                7,
                queue,
                &["INBOX", "Archive"],
                std::time::Duration::ZERO,
                &[],
                &[(1, 5)],
            );

            assert_eq!(settled_by(&pass, &queue), vec![1, 2]);
        }

        #[test]
        fn a_move_owed_for_longer_than_anything_done_is_kept_is_settled_by_the_next_pass() {
            // Whatever the index says after every pass since is what the server says, and a
            // message the person moved back from another device must not stay hidden for ever.
            let queue = Queue {
                owed: vec![crate::ports::op_log::OwedRead {
                    overdue: true,
                    ..owed(1, 1, to_folder(2))
                }],
                ..Queue::default()
            };

            let ((pass, queue, _, _, _), _) = pass_over_holding(
                7,
                7,
                queue,
                &["INBOX", "Archive"],
                std::time::Duration::ZERO,
                &[],
                &[(1, 1)],
            );

            assert_eq!(settled_by(&pass, &queue), vec![1]);
        }

        #[test]
        fn a_move_whose_either_end_could_not_be_read_is_still_owed() {
            for refused in [1, 2] {
                let queue = Queue {
                    owed: vec![owed(1, 1, to_folder(2))],
                    ..Queue::default()
                };

                let (pass, queue, _, _, _) = pass_over_owing(
                    7,
                    7,
                    queue,
                    &["INBOX", "Archive"],
                    std::time::Duration::ZERO,
                    &[refused],
                );

                assert!(
                    settled_by(&pass, &queue).is_empty(),
                    "settled with folder {refused} unread"
                );
            }
        }

        #[test]
        fn a_move_into_a_folder_the_account_no_longer_has_is_read_back_all_the_same() {
            // Nothing will ever read that end, and waiting for it would lay the move over the index
            // for the life of the profile.
            let queue = Queue {
                owed: vec![owed(1, 1, to_folder(9))],
                ..Queue::default()
            };

            let (pass, queue, _, _, _) = pass_over_owing(
                7,
                7,
                queue,
                &["INBOX", "Archive"],
                std::time::Duration::ZERO,
                &[],
            );

            assert_eq!(settled_by(&pass, &queue), vec![1]);
        }

        #[test]
        fn a_delete_into_the_trash_is_read_back_once_the_trash_has_been_read() {
            // The folder the account keeps deleted mail in is the one end a delete has besides the
            // folder it left. Nothing else the server refuses to let be read has anything to say
            // about where it went.
            let owing = || Queue {
                owed: vec![owed(1, 1, WentTo::Trash), owed(2, 1, WentTo::Nowhere)],
                ..Queue::default()
            };

            let (pass, queue, _, _, _) = pass_over_owing(
                7,
                7,
                owing(),
                &["INBOX", "Trash", "Junk"],
                std::time::Duration::ZERO,
                &[3],
            );
            assert_eq!(
                settled_by(&pass, &queue),
                vec![1, 2],
                "a folder that is not the trash held a delete owed"
            );

            let (pass, queue, _, _, _) = pass_over_owing(
                7,
                7,
                owing(),
                &["INBOX", "Trash"],
                std::time::Duration::ZERO,
                &[2],
            );
            assert_eq!(
                settled_by(&pass, &queue),
                vec![2],
                "a delete for good needs only the folder it left"
            );
        }

        #[test]
        fn a_move_carried_out_longer_ago_than_anything_done_is_kept_is_settled_unread() {
            // A folder that fails every pass is an end no pass will ever read, and a move into it
            // waiting for that read would be laid over the index -- and counted in the trash --
            // for the life of the profile.
            for refused in [1, 2] {
                let queue = Queue {
                    owed: vec![crate::ports::op_log::OwedRead {
                        overdue: true,
                        ..owed(1, 1, to_folder(2))
                    }],
                    ..Queue::default()
                };

                let (pass, queue, _, _, _) = pass_over_owing(
                    7,
                    7,
                    queue,
                    &["INBOX", "Archive"],
                    std::time::Duration::ZERO,
                    &[refused],
                );

                assert_eq!(
                    settled_by(&pass, &queue),
                    vec![1],
                    "still owed with folder {refused} unread"
                );
            }
        }

        #[test]
        fn a_pass_that_could_not_read_a_folder_keeps_what_lost_its_last_copy_during_it() {
            // The message whose last copy the pass took away may be in exactly the folder it could
            // not read -- archived from the inbox a moment ago, with the archive refusing. Forgotten
            // now, it comes back from the archive under a new name, and the way back the person
            // pressed names a message nothing holds. Message 4 lost its copy during this pass.
            let ((pass, _, _, _, _), forgotten) = pass_finding_unplaced(
                7,
                7,
                Queue {
                    named: vec![9],
                    ..Queue::default()
                },
                &["INBOX", "Archive"],
                std::time::Duration::ZERO,
                &[2],
                &[],
                vec![vec![3], vec![3, 4]],
            );

            assert_eq!(pass.folders_failed, 1);
            assert_eq!(
                forgotten.map(|(_, kept)| kept),
                Some(vec![9, 4]),
                "what the queue names and what went during this pass"
            );
        }

        /// A pass over a server that answers `INBOX` and `Archive`, refusing `refusing` for good and
        /// `failing_once` the first time it is asked: what the pass did, the folders in the order it
        /// read them, and how many times it said a folder was done.
        fn pass_failing(
            refusing: &[u64],
            failing_once: &[u64],
        ) -> (AccountPass, Vec<String>, usize) {
            let ((pass, _, read, _, told), _, _) = pass_after_one_that_stopped_short(
                7,
                7,
                Queue::default(),
                &["INBOX", "Archive"],
                std::time::Duration::ZERO,
                refusing,
                &[],
                Vec::new(),
                &[],
                &[],
                failing_once,
            );
            let done = told
                .said
                .lock()
                .expect("test lock")
                .iter()
                .filter(|said| said.ends_with("read one"))
                .count();

            (pass, read, done)
        }

        /// A folder whose reading fails once is read again at the end of the same pass, so mail in
        /// an inbox that went quiet for a moment is not left for the next round.
        #[test]
        fn a_folder_that_fails_once_is_read_again_in_the_same_pass() {
            let (pass, read, done) = pass_failing(&[], &[1]);

            assert_eq!(
                pass.folders_failed, 0,
                "a folder read the second time failed"
            );
            assert_eq!(pass.folders_read_again, 1);
            assert_eq!(
                read,
                ["INBOX", "Archive", "INBOX"],
                "the inbox was not read again after the rest"
            );
            assert_eq!(done, 2, "a folder read twice was said to be done twice");
        }

        /// A folder that fails the second time as well is one failure, said done once, and read no
        /// more than twice.
        #[test]
        fn a_folder_that_fails_again_is_one_failure() {
            let (pass, read, done) = pass_failing(&[1], &[]);

            assert_eq!(pass.folders_failed, 1);
            assert_eq!(pass.folders_read_again, 1);
            assert_eq!(read, ["INBOX", "Archive", "INBOX"]);
            assert_eq!(done, 2);
        }

        /// A pass where nothing fails reads nothing twice.
        #[test]
        fn a_pass_where_nothing_fails_reads_each_folder_once() {
            let (pass, read, done) = pass_failing(&[], &[]);

            assert_eq!(pass.folders_failed, 0);
            assert_eq!(pass.folders_read_again, 0);
            assert_eq!(read, ["INBOX", "Archive"]);
            assert_eq!(done, 2);
        }

        #[test]
        fn a_folder_that_fails_on_every_pass_does_not_keep_what_is_deleted_for_ever() {
            // A mailbox the server lists and will not open -- `[Gmail]` on every Gmail account --
            // fails every pass. Message 3 had no copy left when this pass began, so it has had a
            // whole pass to turn up somewhere, and kept any longer it stays on disk and in every
            // search after it was deleted for good.
            let ((_, _, _, _, _), forgotten) = pass_finding_unplaced(
                7,
                7,
                Queue::default(),
                &["INBOX", "[Gmail]"],
                std::time::Duration::ZERO,
                &[2],
                &[],
                vec![vec![3], vec![3]],
            );

            let kept = forgotten.map(|(_, kept)| kept);
            assert_eq!(kept, Some(Vec::new()), "the sweep ran and kept nothing");
        }

        #[test]
        fn a_move_into_a_folder_read_one_window_short_is_still_owed_and_its_message_kept() {
            // Archive is more than a window behind its newest number, and message 1, filed into it
            // a moment ago, has the newest number there is. The inbox has given it up and Archive's
            // window ends below it. Settled now, the move stops saying where the message went, and
            // the sweep forgets a message no folder holds -- its row, and its body with it -- to
            // read it again as a stranger once the window gets there.
            let queue = Queue {
                owed: vec![owed(1, 1, to_folder(2))],
                ..Queue::default()
            };

            let ((pass, queue, _, _, _), forgotten) = pass_stopping_short(
                7,
                7,
                queue,
                &["INBOX", "Archive"],
                std::time::Duration::ZERO,
                &[],
                &[],
                vec![Vec::new(), vec![1]],
                &[2],
            );

            assert!(
                settled_by(&pass, &queue).is_empty(),
                "settled with its folder read short"
            );
            assert_eq!(
                forgotten, None,
                "forgotten while a folder was read only part of the way"
            );
        }

        #[test]
        fn mail_filed_by_another_device_into_a_folder_read_one_window_short_is_not_forgotten() {
            // Nothing here asked for the move, so nothing in the queue names message 4: only the
            // sweep's waiting keeps it until Archive's window reaches it.
            let ((_, _, _, _, _), forgotten) = pass_stopping_short(
                7,
                7,
                Queue::default(),
                &["INBOX", "Archive"],
                std::time::Duration::ZERO,
                &[],
                &[],
                vec![Vec::new(), vec![4]],
                &[2],
            );

            assert_eq!(forgotten, None);
        }

        #[test]
        fn a_folder_that_stopped_short_and_then_fails_keeps_what_was_filed_past_its_window() {
            // The last pass read Archive one window short, and another device had filed message 4
            // into it past that window: the inbox gave it up, the sweep waited, and 4 had no copy
            // when this pass began. Now Archive cannot be read at all. It is still not seen to its
            // end, and forgotten now, 4 comes back a stranger when the window gets there — its body
            // read again and nothing asked about it finding it.
            let ((pass, _, _, _, _), forgotten, forgot_short) = pass_after_one_that_stopped_short(
                7,
                7,
                Queue::default(),
                &["INBOX", "Archive"],
                std::time::Duration::ZERO,
                &[2],
                &[],
                vec![vec![4], vec![4]],
                &[],
                &[2],
                &[],
            );

            assert_eq!(pass.folders_failed, 1);
            assert_eq!(forgotten, None, "forgotten while Archive was still unseen");
            assert_eq!(
                forgot_short,
                vec![FolderId::new(2).expect("non-zero")],
                "once: a folder that fails on every pass must not keep every deletion for good"
            );
        }

        #[test]
        fn a_folder_that_stopped_short_and_is_read_again_is_not_taken_back_by_hand() {
            // Read this time, the pass writes down how far it got with its cursor, and nothing has
            // to be taken back.
            let ((_, _, _, _, _), forgotten, forgot_short) = pass_after_one_that_stopped_short(
                7,
                7,
                Queue::default(),
                &["INBOX", "Archive"],
                std::time::Duration::ZERO,
                &[],
                &[],
                vec![vec![4], vec![4]],
                &[],
                &[2],
                &[],
            );

            assert!(forgot_short.is_empty());
            assert_eq!(
                forgotten.map(|(_, kept)| kept),
                Some(Vec::new()),
                "read to its end, the account is swept as usual"
            );
        }

        #[test]
        fn a_folder_read_one_window_short_lets_go_of_nothing_waiting_to_be_found_in_it() {
            // What waits to be found again is let go when a pass has written where every message
            // is, and one that stopped short has not: an instruction about a message past its
            // window would find nothing there and stop.
            let ((pass, queue, _, _, _), _) = pass_stopping_short(
                7,
                7,
                Queue {
                    waiting: 3,
                    ..Queue::default()
                },
                &["INBOX", "Archive"],
                std::time::Duration::ZERO,
                &[],
                &[],
                Vec::new(),
                &[2],
            );

            assert_eq!(
                *queue.found.lock().expect("test lock"),
                vec![("acc-1".to_owned(), folder())]
            );
            assert_eq!(pass.operations_found, 3);
        }

        #[test]
        fn what_the_queue_still_speaks_about_is_kept_by_the_sweep() {
            // An instruction waiting to be found again after its folder renumbered finds its
            // message under the same name only while the index still has that name.
            let ((_, _, _, _, _), forgotten) = pass_over_holding(
                7,
                7,
                Queue {
                    named: vec![5, 9],
                    ..Queue::default()
                },
                &["INBOX", "Archive"],
                std::time::Duration::ZERO,
                &[],
                &[],
            );

            assert_eq!(forgotten.map(|(_, kept)| kept), Some(vec![5, 9]));
        }

        #[test]
        fn what_waits_for_a_folder_the_server_no_longer_lists_is_asked_about_every_pass() {
            // A folder renamed on another device is never read again, and what waits for it to be
            // read waits for ever -- with everything asked after it about the same message
            // behind it. The queue is told every folder the account still has.
            let ((_, queue, _, _, _), _) = pass_over_holding(
                7,
                7,
                Queue::default(),
                &["INBOX", "Archive"],
                std::time::Duration::ZERO,
                &[2],
                &[],
            );

            assert_eq!(
                *queue.gone_asked.lock().expect("test lock"),
                vec![(
                    "acc-1".to_owned(),
                    vec![
                        FolderId::new(1).expect("non-zero"),
                        FolderId::new(2).expect("non-zero")
                    ]
                )]
            );
        }

        #[test]
        fn each_phase_of_a_pass_is_timed_as_itself() {
            // The journal's split of where a pass went is what any question about speed is
            // answered from, and a stamp in the wrong place makes it answer about the phase
            // before. Reading whole messages is made to cost something here and nothing else is,
            // so exactly one figure may carry it.
            let cost = std::time::Duration::from_millis(40);
            let (pass, _, _, _, _) = pass_over_mailboxes_taking(7, 7, 0, &["INBOX"], cost);

            assert!(
                pass.spent.bodies >= cost,
                "the reading of whole messages was timed as something else: {:?}",
                pass.spent
            );
            assert!(
                pass.spent.backfill < cost,
                "the backfill figure carries time nothing spent there: {:?}",
                pass.spent
            );
            assert!(
                pass.spent.folders < cost,
                "the folders figure carries time nothing spent there: {:?}",
                pass.spent
            );
        }

        #[test]
        fn a_pass_says_how_far_it_has_got_as_it_goes() {
            // The line at the foot of the window is drawn from this. Said only at the end, it
            // would be a bar that stands still for the whole of a round and fills in one frame,
            // because nine mailboxes read at once finish together and the only thing that
            // finishes steadily is a folder.
            let (_, _, _, _, told) = pass_over_mailboxes(
                7,
                7,
                0,
                &["INBOX", "Archive", "Trash", "Junk", "Sent", "Drafts"],
            );

            let said = told.said.lock().expect("test lock").clone();
            assert_eq!(
                said.first().map(String::as_str),
                Some("acc-1 has 6"),
                "the size of the job comes first, or the bar has no length to fill: {said:?}"
            );
            assert_eq!(
                said.iter().filter(|one| one.ends_with("read one")).count(),
                6,
                "one for every mailbox read: {said:?}"
            );
        }

        #[test]
        fn a_pass_asks_for_one_provider_however_many_mailboxes_it_reads() {
            // A provider keeps its connection, so this is the number of times the pass says hello.
            // Asked for inside the folder loop it was one per mailbox: measured against a real
            // server, seven connections at about a second each, in a twelve-second pass.
            let (_, _, _, built, _) = pass_over_mailboxes(
                7,
                7,
                0,
                &["INBOX", "Archive", "Trash", "Junk", "Sent", "Drafts"],
            );

            assert_eq!(
                built,
                vec!["discovery", "sync", "reading"],
                "one for the mailbox listing, one for reading them and one for the messages — \
                 and six mailboxes did not add six more"
            );
        }

        #[test]
        fn the_inbox_is_read_first_whatever_order_the_server_listed_its_mailboxes_in() {
            // Dovecot lists INBOX last, and every other server is free to. Read in that order, a
            // first pass fills Archive, Junk, Sent and Drafts before it — and a person watches an
            // empty inbox with mail visible in folders they never opened.
            let (_, _, read, _, _) = pass_over_mailboxes(
                7,
                7,
                0,
                &["Archive", "Trash", "Junk", "Sent", "Drafts", "INBOX"],
            );

            assert_eq!(
                read.first().map(String::as_str),
                Some("INBOX"),
                "the folder a person is looking at was not the first one filled: {read:?}"
            );
            // And the rest keep the order the server gave, which is the order they are shown in.
            assert_eq!(
                read,
                vec!["INBOX", "Archive", "Trash", "Junk", "Sent", "Drafts"],
            );
        }

        #[test]
        fn a_folder_that_renumbered_sends_its_waiting_instructions_to_be_found_again() {
            // The folder's UID space was thrown away, so every position in it now names a
            // different message — and an instruction carried out against one of those positions
            // lands on a stranger's message, on the server, irreversibly.
            let (pass, queue) = pass_over(7, 9, 3);

            assert_eq!(pass.folders_reset, 1);
            assert_eq!(pass.operations_to_reresolve, 3);
            assert_eq!(
                *queue.told.lock().expect("test lock"),
                vec![("acc-1".to_owned(), folder())],
                "the queue was not told, or was told about the wrong folder"
            );
        }

        #[test]
        fn an_ordinary_pass_leaves_the_queue_alone() {
            // Sending everything back to be found again on every pass would mean nothing is ever
            // carried out.
            let (pass, queue) = pass_over(7, 7, 3);

            assert_eq!(pass.folders_reset, 0);
            assert_eq!(pass.operations_to_reresolve, 0);
            assert!(queue.told.lock().expect("test lock").is_empty());
        }

        #[test]
        fn a_reset_with_nothing_waiting_is_not_reported_as_work() {
            let (pass, _) = pass_over(7, 9, 0);

            assert_eq!(pass.folders_reset, 1, "the folder did renumber");
            assert_eq!(pass.operations_to_reresolve, 0, "but nothing was waiting");
        }
    }

    struct FakeProvider {
        answer: Mutex<Option<Result<SyncBatch, MailProviderError>>>,
        asked: Mutex<Vec<SyncIntent>>,
        /// What each request said about the last reading of its kind being refused for a moment.
        told_of_a_refusal: Mutex<Vec<bool>>,
    }

    impl FakeProvider {
        fn returning(batch: SyncBatch) -> Self {
            Self {
                answer: Mutex::new(Some(Ok(batch))),
                asked: Mutex::new(Vec::new()),
                told_of_a_refusal: Mutex::new(Vec::new()),
            }
        }

        fn asked(&self) -> Vec<SyncIntent> {
            self.asked.lock().expect("test lock").clone()
        }

        fn told_of_a_refusal(&self) -> Vec<bool> {
            self.told_of_a_refusal.lock().expect("test lock").clone()
        }

        fn failing(error: MailProviderError) -> Self {
            Self {
                asked: Mutex::new(Vec::new()),
                answer: Mutex::new(Some(Err(error))),
                told_of_a_refusal: Mutex::new(Vec::new()),
            }
        }
    }

    impl MailSync for FakeProvider {
        fn flag_scope(&self) -> FlagScope {
            FlagScope::Message
        }

        fn fetch(&self, request: &SyncRequest) -> Result<SyncBatch, MailProviderError> {
            self.asked
                .lock()
                .expect("test lock")
                .push(request.intent.clone());
            self.told_of_a_refusal
                .lock()
                .expect("test lock")
                .push(request.refused_for_now_last_time);
            self.answer
                .lock()
                .expect("test lock")
                .take()
                .expect("the provider is asked exactly once per pass")
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    enum Call {
        KnownUidsListed(Vec<Uid>),
        KnownUidsWindow(Uid, Uid),
        Reset(UidValidity),
        Apply { deletions: Vec<Uid>, cursor: String },
        RecordedFloor { floor: Uid, complete: bool },
        RefusedForNow { reading: Reading, refused: bool },
    }

    #[derive(Clone)]
    struct FakeIndex {
        stored_generation: Option<UidValidity>,
        stored_cursor: Option<SyncCursor>,
        known: Vec<Uid>,
        unavailable: bool,
        stored_floor: Option<Uid>,
        history_complete: bool,
        /// Whether the folder's last pass stopped short, as the index has it.
        stored_short: bool,
        /// Which reading of the folder the server last refused for a moment, as the index has it.
        stored_refused: Option<Reading>,
        /// What each delta applied said about stopping short, in the order they were applied.
        applied_short: std::sync::Arc<Mutex<Vec<bool>>>,
        calls: std::sync::Arc<Mutex<Vec<Call>>>,
    }

    impl Default for FakeIndex {
        fn default() -> Self {
            Self {
                stored_generation: Some(generation(7)),
                stored_cursor: None,
                known: Vec::new(),
                unavailable: false,
                stored_floor: None,
                history_complete: false,
                stored_short: false,
                stored_refused: None,
                applied_short: std::sync::Arc::new(Mutex::new(Vec::new())),
                calls: std::sync::Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl FakeIndex {
        fn holding(known: &[u32]) -> Self {
            Self {
                known: known.iter().copied().map(uid).collect(),
                ..Default::default()
            }
        }

        fn calls(&self) -> Vec<Call> {
            std::mem::take(&mut self.calls.lock().expect("test lock"))
        }
    }

    impl MailIndex for FakeIndex {
        fn not_read_whole(
            &self,
            _folder_id: FolderId,
            _at_most: u32,
        ) -> Result<Vec<(lyonoyl_domain::message::MessageId, Uid)>, MailIndexError> {
            Ok(Vec::new())
        }
        fn record_window_floor(
            &self,
            _folder_id: FolderId,
            floor: Uid,
            complete: bool,
        ) -> Result<(), MailIndexError> {
            self.calls
                .lock()
                .expect("test lock")
                .push(Call::RecordedFloor { floor, complete });
            Ok(())
        }

        fn record_folders(
            &self,
            _account: i64,
            _folders: &[crate::ports::mail_index::FolderRecord],
        ) -> Result<Vec<(String, FolderId)>, MailIndexError> {
            unreachable!("a pass over one folder records no mailboxes")
        }

        fn folder_state(&self, folder_id: FolderId) -> Result<FolderSyncState, MailIndexError> {
            if self.unavailable {
                return Err(MailIndexError::Unavailable);
            }
            let _ = folder_id;
            Ok(FolderSyncState {
                name: "INBOX".to_owned(),
                known_window: None,
                window_floor: self.stored_floor,
                history_complete: self.history_complete,
                highest_modseq: None,
                uid_validity: self.stored_generation,
                cursor: self.stored_cursor.clone(),
                stopped_short: self.stored_short,
                new_mail_refused_for_now: self.stored_refused == Some(Reading::NewMail),
                further_back_refused_for_now: self.stored_refused == Some(Reading::FurtherBack),
            })
        }

        fn inbox_of(&self, _account: i64) -> Result<Option<String>, MailIndexError> {
            unreachable!("a pass over one folder does not look for the inbox")
        }

        fn known_uids(
            &self,
            _folder_id: FolderId,
            scope: &UidScope,
        ) -> Result<Vec<Uid>, MailIndexError> {
            let call = match scope {
                UidScope::Listed(uids) => Call::KnownUidsListed(uids.clone()),
                UidScope::Window(window) => Call::KnownUidsWindow(*window.start(), *window.end()),
            };
            self.calls.lock().expect("test lock").push(call);

            Ok(match scope {
                UidScope::Listed(uids) => self
                    .known
                    .iter()
                    .copied()
                    .filter(|held| uids.contains(held))
                    .collect(),
                UidScope::Window(window) => self
                    .known
                    .iter()
                    .copied()
                    .filter(|held| window.contains(held))
                    .collect(),
            })
        }

        fn placement_count(&self, _folder_id: FolderId) -> Result<usize, MailIndexError> {
            Ok(self.known.len())
        }

        fn apply(
            &self,
            delta: &FolderDelta,
            deletions: &[Uid],
            cursor: &SyncCursor,
            _flag_scope: FlagScope,
        ) -> Result<AppliedSummary, MailIndexError> {
            self.calls.lock().expect("test lock").push(Call::Apply {
                deletions: deletions.to_vec(),
                cursor: cursor.as_str().to_owned(),
            });
            self.applied_short
                .lock()
                .expect("test lock")
                .push(delta.stopped_short);
            Ok(AppliedSummary {
                added: delta.arrived.len(),
                updated: delta.flag_updates.len(),
                removed: deletions.len(),
            })
        }

        fn reset_folder(
            &self,
            _folder_id: FolderId,
            uid_validity: UidValidity,
        ) -> Result<(), MailIndexError> {
            self.calls
                .lock()
                .expect("test lock")
                .push(Call::Reset(uid_validity));
            Ok(())
        }

        fn forget_it_stopped_short(&self, _folder: FolderId) -> Result<(), MailIndexError> {
            unreachable!("reading one folder takes nothing back about it")
        }

        fn refused_for_now(
            &self,
            _folder: FolderId,
            reading: Reading,
            refused: bool,
        ) -> Result<(), MailIndexError> {
            self.calls
                .lock()
                .expect("test lock")
                .push(Call::RefusedForNow { reading, refused });
            Ok(())
        }

        fn holds(
            &self,
            _folder: FolderId,
            _message: lyonoyl_domain::message::MessageId,
        ) -> Result<bool, MailIndexError> {
            unreachable!("reading one folder asks after no copy")
        }

        fn unplaced(
            &self,
            _account: i64,
        ) -> Result<Vec<lyonoyl_domain::message::MessageId>, MailIndexError> {
            unreachable!("reading one folder asks after no message")
        }

        fn forget_the_unplaced(
            &self,
            _account: i64,
            _named: &[lyonoyl_domain::message::MessageId],
        ) -> Result<(), MailIndexError> {
            unreachable!("reading one folder forgets nothing")
        }
    }

    #[test]
    fn arrivals_are_applied_together_with_the_new_cursor() {
        let index = FakeIndex::default();
        let outcome = SyncFolder::new(
            FakeProvider::returning(batch(delta(VanishedReport::Unknown), None)),
            index.clone(),
        )
        .execute(folder())
        .expect("the pass succeeds");

        assert_eq!(outcome.summary.added, 1);
        assert!(!outcome.resynced);
        assert_eq!(
            index.calls(),
            vec![Call::Apply {
                deletions: Vec::new(),
                cursor: "100".to_owned(),
            }]
        );
    }

    #[test]
    fn an_incomplete_pass_deletes_nothing_and_does_not_even_ask_what_we_hold() {
        let index = FakeIndex::holding(&[1, 2, 3]);
        let outcome = SyncFolder::new(
            FakeProvider::returning(batch(delta(VanishedReport::Unknown), None)),
            index.clone(),
        )
        .execute(folder())
        .expect("the pass succeeds");

        assert_eq!(outcome.summary.removed, 0);
        assert_eq!(
            index.calls(),
            vec![Call::Apply {
                deletions: Vec::new(),
                cursor: "100".to_owned(),
            }],
            "an unknown report cannot lead to a deletion, so nothing is even looked up"
        );
    }

    #[test]
    fn an_exact_report_removes_only_what_we_hold() {
        let index = FakeIndex::holding(&[1, 2, 3]);
        let vanished = VanishedReport::Exact(vec![uid(2), uid(99)]);
        let outcome = SyncFolder::new(
            FakeProvider::returning(batch(delta(vanished), None)),
            index.clone(),
        )
        .execute(folder())
        .expect("the pass succeeds");

        assert_eq!(outcome.summary.removed, 1);
        assert_eq!(
            index.calls(),
            vec![
                Call::KnownUidsListed(vec![uid(2), uid(99)]),
                Call::Apply {
                    deletions: vec![uid(2)],
                    cursor: "100".to_owned(),
                },
            ]
        );
    }

    #[test]
    fn a_listing_asks_only_for_the_window_it_covered() {
        let index = FakeIndex::holding(&[1, 5, 9]);
        let vanished = VanishedReport::AuthoritativeSet {
            window: uid(4)..=uid(10),
            present: vec![uid(9)],
        };
        SyncFolder::new(
            FakeProvider::returning(batch(delta(vanished), None)),
            index.clone(),
        )
        .execute(folder())
        .expect("the pass succeeds");

        assert_eq!(
            index.calls(),
            vec![
                Call::KnownUidsWindow(uid(4), uid(10)),
                Call::Apply {
                    deletions: vec![uid(5)],
                    cursor: "100".to_owned(),
                },
            ],
            "history below the window is neither read nor touched"
        );
    }

    #[test]
    fn a_mass_disappearance_is_reported_instead_of_applied() {
        let held: Vec<u32> = (1..=1000).collect();
        let index = FakeIndex::holding(&held);
        let vanished = VanishedReport::Exact((1..=300).map(uid).collect());

        let outcome = SyncFolder::new(
            FakeProvider::returning(batch(delta(vanished), None)),
            index.clone(),
        )
        .execute(folder())
        .expect("the pass succeeds");

        assert_eq!(outcome.deletions_awaiting_confirmation, 300);
        assert_eq!(outcome.summary.removed, 0);
        let applied = index
            .calls()
            .into_iter()
            .find_map(|call| match call {
                Call::Apply { deletions, .. } => Some(deletions),
                _ => None,
            })
            .expect("the arrivals are still applied");
        assert!(
            applied.is_empty(),
            "nothing is removed without confirmation"
        );
    }

    #[test]
    fn a_changed_generation_resets_the_folder_before_applying() {
        let index = FakeIndex {
            stored_generation: Some(generation(6)),
            ..FakeIndex::holding(&[1, 2, 3])
        };
        let vanished = VanishedReport::Exact(vec![uid(1)]);

        let outcome = SyncFolder::new(
            FakeProvider::returning(batch(delta(vanished), Some("99"))),
            index.clone(),
        )
        .execute(folder())
        .expect("the pass succeeds");

        assert!(outcome.resynced);
        assert_eq!(
            index.calls(),
            vec![
                Call::Reset(generation(7)),
                Call::Apply {
                    deletions: Vec::new(),
                    cursor: "100".to_owned(),
                },
            ],
            "after a reset there is nothing left to delete, and the stale cursor is not a failure"
        );
    }

    /// A pass that has read this folder before, with mail nobody has seen in it.
    fn a_second_pass_over(arrivals: Vec<ArrivedMessage>) -> SyncOutcome {
        let index = FakeIndex {
            stored_cursor: Some(SyncCursor::new("50")),
            ..FakeIndex::default()
        };

        let mut delta = delta(VanishedReport::Unknown);
        delta.arrived = arrivals;

        SyncFolder::new(FakeProvider::returning(batch(delta, Some("50"))), index)
            .execute(folder())
            .expect("the pass succeeds")
    }

    #[test]
    fn mail_that_arrived_unread_is_counted_and_mail_that_did_not_is_not() {
        // Read on another device between two passes, and it arrives here already seen. Being told
        // about mail somebody has just finished reading on their phone is the notification nobody
        // wants twice.
        let mut seen = arrival(11);
        seen.envelope.flags = Flags::SEEN;

        let outcome = a_second_pass_over(vec![arrival(10), seen, arrival(12)]);

        assert_eq!(outcome.summary.added, 3, "all three are stored");
        assert_eq!(outcome.unseen_arrivals, 2, "and two of them are unread");
    }

    #[test]
    fn the_first_pass_over_a_folder_is_a_mailbox_arriving_rather_than_mail() {
        // Adding an account brings four thousand messages. Nobody sent them just now, and being
        // told they did is the difference between a notification and an alarm.
        let outcome = SyncFolder::new(
            FakeProvider::returning(batch(delta(VanishedReport::Unknown), None)),
            FakeIndex::default(),
        )
        .execute(folder())
        .expect("the pass succeeds");

        assert_eq!(outcome.summary.added, 1, "the mail is stored");
        assert_eq!(outcome.unseen_arrivals, 0, "and nobody is interrupted");
    }

    #[test]
    fn a_folder_read_again_from_the_start_is_not_mail_arriving_either() {
        // The server threw away its numbering, so the folder is read from the beginning. Every
        // message in it looks like an arrival and not one of them is news.
        let index = FakeIndex {
            stored_cursor: Some(SyncCursor::new("50")),
            stored_generation: Some(generation(6)),
            ..FakeIndex::default()
        };

        let outcome = SyncFolder::new(
            FakeProvider::returning(batch(delta(VanishedReport::Unknown), None)),
            index,
        )
        .execute(folder())
        .expect("the pass succeeds");

        assert!(
            outcome.resynced,
            "the fixture must actually reset the folder"
        );
        assert_eq!(outcome.unseen_arrivals, 0);
    }

    #[test]
    fn a_batch_that_started_from_another_position_is_refused() {
        let index = FakeIndex {
            stored_cursor: Some(SyncCursor::new("50")),
            ..FakeIndex::default()
        };

        let error = SyncFolder::new(
            FakeProvider::returning(batch(delta(VanishedReport::Unknown), Some("10"))),
            index.clone(),
        )
        .execute(folder())
        .expect_err("a stale batch is refused");

        assert!(matches!(error, UseCaseError::StaleSyncBatch));
        assert!(index.calls().is_empty(), "nothing is written");
    }

    #[test]
    fn an_answer_about_another_folder_is_refused() {
        let index = FakeIndex::default();
        let other = FolderDelta {
            folder_id: FolderId::new(2).expect("non-zero"),
            ..delta(VanishedReport::Unknown)
        };

        let error = SyncFolder::new(FakeProvider::returning(batch(other, None)), index.clone())
            .execute(folder())
            .expect_err("an answer about another folder is refused");

        assert!(matches!(error, UseCaseError::UnexpectedFolder));
        assert!(index.calls().is_empty(), "nothing is written");
    }

    #[test]
    fn a_provider_failure_keeps_its_meaning_and_writes_nothing() {
        let index = FakeIndex::default();
        let error = SyncFolder::new(
            FakeProvider::failing(MailProviderError::Offline),
            index.clone(),
        )
        .execute(folder())
        .expect_err("an offline provider fails the pass");

        assert!(matches!(error, UseCaseError::Offline));
        assert!(index.calls().is_empty());
    }

    #[test]
    fn an_unavailable_index_is_reported_before_the_provider_is_asked() {
        let index = FakeIndex {
            unavailable: true,
            ..FakeIndex::default()
        };

        let error = SyncFolder::new(
            FakeProvider::returning(batch(delta(VanishedReport::Unknown), None)),
            index.clone(),
        )
        .execute(folder())
        .expect_err("an unavailable index fails the pass");

        assert!(matches!(error, UseCaseError::StorageUnavailable));
    }

    fn uid_of(value: u32) -> Uid {
        Uid::new(value).expect("non-zero")
    }

    #[test]
    fn reading_back_asks_for_the_round_below_the_floor() {
        let index = FakeIndex {
            stored_floor: Some(uid_of(1001)),
            ..FakeIndex::default()
        };
        let provider = FakeProvider::returning(batch(delta(VanishedReport::Unknown), None));

        let outcome = BackfillFolder::new(&provider, index.clone())
            .execute(folder())
            .expect("the round applies");

        assert_eq!(
            provider.asked(),
            vec![SyncIntent::Older(uid_of(501)..=uid_of(1000))]
        );
        assert!(!outcome.complete);
        assert!(index.calls().contains(&Call::RecordedFloor {
            floor: uid_of(501),
            complete: false
        }));
    }

    #[test]
    fn the_last_round_reaches_the_first_message_and_says_so() {
        let index = FakeIndex {
            stored_floor: Some(uid_of(300)),
            ..FakeIndex::default()
        };
        let provider = FakeProvider::returning(batch(delta(VanishedReport::Unknown), None));

        let outcome = BackfillFolder::new(&provider, index.clone())
            .execute(folder())
            .expect("the round applies");

        assert_eq!(
            provider.asked(),
            vec![SyncIntent::Older(uid_of(1)..=uid_of(299))]
        );
        assert!(outcome.complete, "there is nothing below UID 1");
        assert!(index.calls().contains(&Call::RecordedFloor {
            floor: uid_of(1),
            complete: true
        }));
    }

    #[test]
    fn a_range_that_held_nothing_still_moves_the_floor() {
        // Otherwise the same empty range is asked for on every round, for ever.
        let index = FakeIndex {
            stored_floor: Some(uid_of(1001)),
            ..FakeIndex::default()
        };
        let mut empty = delta(VanishedReport::Unknown);
        empty.arrived = Vec::new();
        let provider = FakeProvider::returning(batch(empty, None));

        BackfillFolder::new(&provider, index.clone())
            .execute(folder())
            .expect("the round applies");

        assert!(index.calls().contains(&Call::RecordedFloor {
            floor: uid_of(501),
            complete: false
        }));
    }

    #[test]
    fn reading_further_back_leaves_the_folder_as_far_forward_as_it_was() {
        // The last forward pass stopped one window short of the newest mail, and the round further
        // back that follows it in the same pass knows nothing about that. Written as it found it,
        // the folder would read as one seen to its end before any pass had seen past the window.
        for stored_short in [true, false] {
            let index = FakeIndex {
                stored_floor: Some(uid_of(1001)),
                stored_short,
                ..FakeIndex::default()
            };
            let provider = FakeProvider::returning(batch(delta(VanishedReport::Unknown), None));

            BackfillFolder::new(&provider, index.clone())
                .execute(folder())
                .expect("the round applies");

            assert_eq!(
                *index.applied_short.lock().expect("test lock"),
                vec![stored_short]
            );
        }
    }

    #[test]
    fn a_pass_writes_down_whether_it_stopped_short_with_its_cursor() {
        // What the pass after it reads if it then cannot read the folder at all.
        let index = FakeIndex::default();
        let mut short = delta(VanishedReport::Unknown);
        short.stopped_short = true;

        SyncFolder::new(FakeProvider::returning(batch(short, None)), index.clone())
            .execute(folder())
            .expect("the pass succeeds");

        assert_eq!(*index.applied_short.lock().expect("test lock"), vec![true]);
    }

    #[test]
    fn a_reading_refused_for_a_moment_is_written_down_for_the_next_one_of_its_kind() {
        // A server may say it of the same message on every pass, and taken at its word each time,
        // the folder stops there for good. What the next reading of the same kind needs is that
        // this one was refused so, and the two readings of a pass are refused apart.
        let index = FakeIndex::default();
        let failure = SyncFolder::new(
            FakeProvider::failing(MailProviderError::RefusedForNow),
            index.clone(),
        )
        .execute(folder())
        .expect_err("a refusal for a moment applies nothing");

        assert!(matches!(failure, UseCaseError::Offline), "{failure:?}");
        assert_eq!(
            index.calls(),
            vec![Call::RefusedForNow {
                reading: Reading::NewMail,
                refused: true
            }]
        );

        let index = FakeIndex {
            stored_floor: Some(uid_of(1001)),
            ..FakeIndex::default()
        };
        BackfillFolder::new(
            FakeProvider::failing(MailProviderError::RefusedForNow),
            index.clone(),
        )
        .execute(folder())
        .expect_err("a refusal for a moment reads nothing further back");

        assert_eq!(
            index.calls(),
            vec![Call::RefusedForNow {
                reading: Reading::FurtherBack,
                refused: true
            }]
        );

        // Any other failure is not one: a network gone for a minute says nothing about what the
        // server will say of a message.
        let index = FakeIndex::default();
        SyncFolder::new(
            FakeProvider::failing(MailProviderError::Offline),
            index.clone(),
        )
        .execute(folder())
        .expect_err("nothing was read");
        assert_eq!(index.calls(), Vec::new());
    }

    #[test]
    fn a_reading_after_one_refused_for_a_moment_is_told_so_and_answered_takes_it_back() {
        // Told, the adapter takes a second refusal for a moment as a refusal like any other.
        // Answered, the moment has passed, and the next refusal of that kind is a moment of its
        // own rather than the second in a row.
        let index = FakeIndex {
            stored_refused: Some(Reading::NewMail),
            ..FakeIndex::default()
        };
        let provider = FakeProvider::returning(batch(delta(VanishedReport::Unknown), None));
        SyncFolder::new(&provider, index.clone())
            .execute(folder())
            .expect("the pass succeeds");

        assert_eq!(provider.told_of_a_refusal(), vec![true]);
        assert_eq!(
            index.calls(),
            vec![
                Call::Apply {
                    deletions: Vec::new(),
                    cursor: "100".to_owned(),
                },
                Call::RefusedForNow {
                    reading: Reading::NewMail,
                    refused: false
                },
            ]
        );

        let index = FakeIndex {
            stored_refused: Some(Reading::FurtherBack),
            stored_floor: Some(uid_of(1001)),
            ..FakeIndex::default()
        };
        let provider = FakeProvider::returning(batch(delta(VanishedReport::Unknown), None));
        BackfillFolder::new(&provider, index.clone())
            .execute(folder())
            .expect("the round applies");

        assert_eq!(provider.told_of_a_refusal(), vec![true]);
        assert_eq!(
            index.calls().last(),
            Some(&Call::RefusedForNow {
                reading: Reading::FurtherBack,
                refused: false
            })
        );
    }

    #[test]
    fn a_reading_is_told_only_of_a_refusal_of_its_own_kind() {
        // One refused for a moment further back on every pass, and a reading of new mail that
        // never is: told of the other's refusal, new mail would be kept as rows with no name the
        // first time it is refused, and taking it back would leave the reading further back
        // refused for a moment on every pass, for ever.
        for (stored, forward) in [(Reading::FurtherBack, true), (Reading::NewMail, false)] {
            let index = FakeIndex {
                stored_refused: Some(stored),
                stored_floor: Some(uid_of(1001)),
                ..FakeIndex::default()
            };
            let provider = FakeProvider::returning(batch(delta(VanishedReport::Unknown), None));
            match forward {
                true => SyncFolder::new(&provider, index.clone())
                    .execute(folder())
                    .map(|_| ()),
                false => BackfillFolder::new(&provider, index.clone())
                    .execute(folder())
                    .map(|_| ()),
            }
            .expect("the reading applies");

            assert_eq!(provider.told_of_a_refusal(), vec![false], "{stored:?}");
            assert!(
                !index
                    .calls()
                    .iter()
                    .any(|call| matches!(call, Call::RefusedForNow { .. })),
                "{stored:?}"
            );
        }
    }

    #[test]
    fn a_refusal_for_a_moment_stands_until_a_reading_of_its_kind_is_applied() {
        // What the next reading is told is that one of its kind was refused for a moment and none
        // has been applied since, which is what the adapter and the index say it means. A reading
        // answered and then not applied — it started from a position that is no longer the
        // folder's — has not carried the folder past the moment, and neither has one that failed
        // another way.
        let answered = FakeIndex {
            stored_cursor: Some(SyncCursor::new("50")),
            stored_refused: Some(Reading::NewMail),
            ..FakeIndex::default()
        };
        SyncFolder::new(
            FakeProvider::returning(batch(delta(VanishedReport::Unknown), Some("10"))),
            answered.clone(),
        )
        .execute(folder())
        .expect_err("a batch from another position applies nothing");

        let failed = FakeIndex {
            stored_refused: Some(Reading::NewMail),
            ..FakeIndex::default()
        };
        SyncFolder::new(
            FakeProvider::failing(MailProviderError::Offline),
            failed.clone(),
        )
        .execute(folder())
        .expect_err("nothing was read");

        for index in [answered, failed] {
            let calls = index.calls();
            assert!(
                !calls
                    .iter()
                    .any(|call| matches!(call, Call::RefusedForNow { .. })),
                "{calls:?}"
            );
        }
    }

    #[test]
    fn a_folder_read_to_its_first_message_is_left_alone() {
        let index = FakeIndex {
            history_complete: true,
            stored_floor: Some(uid_of(1)),
            ..FakeIndex::default()
        };
        let provider = FakeProvider::returning(batch(delta(VanishedReport::Unknown), None));

        let outcome = BackfillFolder::new(&provider, index.clone())
            .execute(folder())
            .expect("there is nothing to do");

        assert!(outcome.complete);
        assert!(
            provider.asked().is_empty(),
            "a finished folder must not be asked again"
        );
    }

    #[test]
    fn a_floor_already_at_the_first_uid_finishes_without_asking() {
        let index = FakeIndex {
            stored_floor: Some(uid_of(1)),
            ..FakeIndex::default()
        };
        let provider = FakeProvider::returning(batch(delta(VanishedReport::Unknown), None));

        let outcome = BackfillFolder::new(&provider, index.clone())
            .execute(folder())
            .expect("there is nothing below the first UID");

        assert!(outcome.complete);
        assert!(provider.asked().is_empty());
        assert!(index.calls().contains(&Call::RecordedFloor {
            floor: uid_of(1),
            complete: true
        }));
    }

    #[test]
    fn a_pass_records_how_far_down_it_looked_even_when_nothing_was_there() {
        // A mailbox whose messages all sit below the first window answers it with nothing. Without
        // this the folder looks empty, no floor is recorded, and no round ever goes looking lower.
        let index = FakeIndex::default();
        let mut empty = delta(VanishedReport::Unknown);
        empty.arrived = Vec::new();
        empty.covered_from = Some(uid_of(9_500));

        SyncFolder::new(FakeProvider::returning(batch(empty, None)), index.clone())
            .execute(folder())
            .expect("the pass applies");

        let calls = index.calls();
        assert!(
            calls.contains(&Call::RecordedFloor {
                floor: uid_of(9_500),
                complete: false
            }),
            "the pass must say where it stopped looking: {calls:?}"
        );
    }

    #[test]
    fn a_pass_that_reached_the_first_uid_says_the_folder_is_finished() {
        let index = FakeIndex::default();
        let mut whole = delta(VanishedReport::Unknown);
        whole.covered_from = Some(uid_of(1));

        SyncFolder::new(FakeProvider::returning(batch(whole, None)), index.clone())
            .execute(folder())
            .expect("the pass applies");

        assert!(index.calls().contains(&Call::RecordedFloor {
            floor: uid_of(1),
            complete: true
        }));
    }

    #[test]
    fn a_folder_nothing_has_been_read_from_yet_waits_for_the_forward_pass() {
        let index = FakeIndex::default();
        let provider = FakeProvider::returning(batch(delta(VanishedReport::Unknown), None));

        let outcome = BackfillFolder::new(&provider, index.clone())
            .execute(folder())
            .expect("there is nothing to go back through");

        assert!(!outcome.complete);
        assert!(provider.asked().is_empty());
        assert!(index.calls().is_empty());
    }

    #[test]
    fn a_mailbox_renumbered_mid_round_stores_nothing_from_it() {
        // Those messages would be stored under UIDs that address nothing. The forward pass resets
        // the folder; this round steps aside.
        let index = FakeIndex {
            stored_floor: Some(uid_of(1001)),
            ..FakeIndex::default()
        };
        let mut renumbered = delta(VanishedReport::Unknown);
        renumbered.uid_validity = generation(99);
        let provider = FakeProvider::returning(batch(renumbered, None));

        BackfillFolder::new(&provider, index.clone())
            .execute(folder())
            .expect("the round steps aside");

        let calls = index.calls();
        assert!(
            !calls.iter().any(|call| matches!(call, Call::Apply { .. })),
            "nothing may be stored: {calls:?}"
        );
        assert!(
            !calls
                .iter()
                .any(|call| matches!(call, Call::RecordedFloor { .. })),
            "and the floor may not move: {calls:?}"
        );
    }
}

/// Reading messages whole: how many conversations it takes, and whose bytes land where.
#[cfg(test)]
mod reading_bodies {
    use super::*;
    use crate::ports::mail_index::{FolderRecord, FolderSyncState, MailIndexError};
    use crate::ports::mail_provider::MailProviderError;
    use crate::ports::message_bodies::{MessageBodies, MessageBodyError};
    use lyonoyl_domain::message::{FlagScope, MessageId, Uid, UidValidity};
    use lyonoyl_domain::sync::{FolderDelta, SyncCursor};
    use std::sync::Mutex;

    fn uid(number: u32) -> Uid {
        Uid::new(number).expect("non-zero")
    }

    fn message(number: u64) -> MessageId {
        MessageId::new(number).expect("non-zero")
    }

    /// A server that hands back the bytes it was asked for, and counts how it was asked.
    struct Server {
        /// One entry per request, holding the numbers that request named.
        asked: Mutex<Vec<Vec<u32>>>,
        /// The generation each request was made under.
        under: Mutex<Vec<u32>>,
        /// Numbers the server will say nothing about, however often it is asked.
        silent_about: Vec<u32>,
    }

    impl Server {
        fn answering() -> Self {
            Self {
                asked: Mutex::new(Vec::new()),
                under: Mutex::new(Vec::new()),
                silent_about: Vec::new(),
            }
        }

        fn silent_about(numbers: &[u32]) -> Self {
            Self {
                silent_about: numbers.to_vec(),
                ..Self::answering()
            }
        }

        fn requests(&self) -> Vec<Vec<u32>> {
            self.asked.lock().expect("not poisoned").clone()
        }
    }

    impl MailFetch for &Server {
        fn whole_message(
            &self,
            _mailbox: &str,
            _uid_validity: UidValidity,
            _uid: Uid,
        ) -> Result<Vec<u8>, MailProviderError> {
            unreachable!("a round asks for messages together")
        }

        fn whole_messages(
            &self,
            _mailbox: &str,
            uid_validity: UidValidity,
            uids: &[Uid],
        ) -> Result<Vec<(Uid, Vec<u8>)>, Unread> {
            self.asked
                .lock()
                .expect("not poisoned")
                .push(uids.iter().map(|one| one.get()).collect());
            self.under
                .lock()
                .expect("not poisoned")
                .push(uid_validity.get());

            Ok(uids
                .iter()
                .filter(|one| !self.silent_about.contains(&one.get()))
                // The bytes say which number they came from, so a test can tell whose mail landed
                // where rather than only that something did.
                .map(|one| (*one, format!("message {}", one.get()).into_bytes()))
                .collect())
        }
    }

    /// A server that refuses every request it is asked.
    struct Refuses;

    impl MailFetch for Refuses {
        fn whole_message(
            &self,
            _mailbox: &str,
            _uid_validity: UidValidity,
            _uid: Uid,
        ) -> Result<Vec<u8>, MailProviderError> {
            Err(MailProviderError::Protocol)
        }

        fn whole_messages(
            &self,
            _mailbox: &str,
            _uid_validity: UidValidity,
            _uids: &[Uid],
        ) -> Result<Vec<(Uid, Vec<u8>)>, Unread> {
            Err(Unread::Failed(MailProviderError::Protocol))
        }
    }

    /// A server that cannot read some messages, and ends the conversation when a request names one
    /// of them — Dovecot's own default — so nothing asked for beside it arrives either. It answers
    /// `answering` requests, and after that it is gone: nothing more can be asked of it.
    struct Drops {
        unreadable: Vec<u32>,
        answering: usize,
        /// How a request naming one of them fails: the conversation ended, unless said otherwise.
        failing_with: MailProviderError,
        /// One entry per request, holding the numbers it named.
        asked: Mutex<Vec<Vec<u32>>>,
    }

    impl Drops {
        fn over(unreadable: &[u32]) -> Self {
            Self {
                unreadable: unreadable.to_vec(),
                answering: usize::MAX,
                failing_with: MailProviderError::Offline,
                asked: Mutex::new(Vec::new()),
            }
        }

        fn gone_after(self, answering: usize) -> Self {
            Self { answering, ..self }
        }

        /// The same server, failing a request that names one of them with an answer this build
        /// cannot use rather than with the conversation ending.
        fn failing_with(self, failing_with: MailProviderError) -> Self {
            Self {
                failing_with,
                ..self
            }
        }

        fn requests(&self) -> Vec<Vec<u32>> {
            self.asked.lock().expect("not poisoned").clone()
        }
    }

    impl MailFetch for &Drops {
        fn whole_message(
            &self,
            _mailbox: &str,
            _uid_validity: UidValidity,
            _uid: Uid,
        ) -> Result<Vec<u8>, MailProviderError> {
            unreachable!("a round asks through requests, of one message too")
        }

        fn whole_messages(
            &self,
            _mailbox: &str,
            _uid_validity: UidValidity,
            uids: &[Uid],
        ) -> Result<Vec<(Uid, Vec<u8>)>, Unread> {
            let named: Vec<u32> = uids.iter().map(|one| one.get()).collect();
            let mut asked = self.asked.lock().expect("not poisoned");
            let gone = asked.len() >= self.answering;
            let reads = named.iter().all(|one| !self.unreadable.contains(one));
            asked.push(named);

            if gone {
                return Err(Unread::NotAsked(MailProviderError::Offline));
            }
            if !reads {
                return Err(Unread::Failed(self.failing_with));
            }
            Ok(uids
                .iter()
                .map(|one| (*one, format!("message {}", one.get()).into_bytes()))
                .collect())
        }
    }

    /// A server nothing can be asked of — it cannot be reached, or it will not take the sign-in —
    /// or one that refuses every request for who is asking.
    struct CannotBeAsked {
        why: Unread,
        asked: Mutex<Vec<Vec<u32>>>,
    }

    impl MailFetch for &CannotBeAsked {
        fn whole_message(
            &self,
            _mailbox: &str,
            _uid_validity: UidValidity,
            _uid: Uid,
        ) -> Result<Vec<u8>, MailProviderError> {
            unreachable!("a round asks through requests, of one message too")
        }

        fn whole_messages(
            &self,
            _mailbox: &str,
            _uid_validity: UidValidity,
            uids: &[Uid],
        ) -> Result<Vec<(Uid, Vec<u8>)>, Unread> {
            self.asked
                .lock()
                .expect("not poisoned")
                .push(uids.iter().map(|one| one.get()).collect());
            Err(self.why)
        }
    }

    /// What happened to messages read one at a time, in the order it happened: handed over by the
    /// server, or kept by the store.
    #[derive(Default)]
    struct Happened(Mutex<Vec<String>>);

    impl Happened {
        fn say(&self, what: String) {
            self.0.lock().expect("not poisoned").push(what);
        }
    }

    /// A server that drops every request naming several messages and hands each over alone.
    struct OneAtATime<'a>(&'a Happened);

    impl MailFetch for OneAtATime<'_> {
        fn whole_message(
            &self,
            _mailbox: &str,
            _uid_validity: UidValidity,
            _uid: Uid,
        ) -> Result<Vec<u8>, MailProviderError> {
            unreachable!("a round asks through requests, of one message too")
        }

        fn whole_messages(
            &self,
            _mailbox: &str,
            _uid_validity: UidValidity,
            uids: &[Uid],
        ) -> Result<Vec<(Uid, Vec<u8>)>, Unread> {
            let [one] = uids else {
                return Err(Unread::Failed(MailProviderError::Offline));
            };
            self.0.say(format!("handed {}", one.get()));
            Ok(vec![(*one, format!("message {}", one.get()).into_bytes())])
        }
    }

    /// A store that says what it kept where the server says what it handed over.
    struct KeptAsItCame<'a>(&'a Happened);

    impl MessageBodies for KeptAsItCame<'_> {
        fn held(&self, _message: MessageId) -> Result<Option<Vec<u8>>, MessageBodyError> {
            Ok(None)
        }

        fn keep(&self, message: MessageId, _raw: &[u8]) -> Result<(), MessageBodyError> {
            self.0.say(format!("kept {}", message.get()));
            Ok(())
        }

        fn fill_in_missing(
            &self,
            _message: MessageId,
            _raw: &[u8],
        ) -> Result<(), MessageBodyError> {
            Ok(())
        }
    }

    /// An index that says which messages are still missing their bytes, and the generation their
    /// numbers were stored in.
    #[derive(Clone)]
    struct Waiting(Vec<(MessageId, Uid)>, Option<UidValidity>);

    /// The generation the index holds the numbers in these tests under.
    const STORED_GENERATION: u32 = 7;

    /// Messages still missing their bytes, numbered in the stored generation.
    fn stored(messages: Vec<(MessageId, Uid)>) -> Waiting {
        Waiting(messages, UidValidity::new(STORED_GENERATION).ok())
    }

    impl MailIndex for Waiting {
        fn folder_state(&self, _folder_id: FolderId) -> Result<FolderSyncState, MailIndexError> {
            Ok(FolderSyncState {
                name: "INBOX".to_owned(),
                uid_validity: self.1,
                cursor: None,
                known_window: None,
                window_floor: None,
                highest_modseq: None,
                history_complete: false,
                stopped_short: false,
                new_mail_refused_for_now: false,
                further_back_refused_for_now: false,
            })
        }

        fn not_read_whole(
            &self,
            _folder_id: FolderId,
            at_most: u32,
        ) -> Result<Vec<(MessageId, Uid)>, MailIndexError> {
            Ok(self.0.iter().take(at_most as usize).copied().collect())
        }

        // Nothing below is reached: this round reads messages whole and writes no index at all.
        fn record_folders(
            &self,
            _account: i64,
            _folders: &[FolderRecord],
        ) -> Result<Vec<(String, FolderId)>, MailIndexError> {
            unreachable!("reading bodies records no folder")
        }

        fn inbox_of(&self, _account: i64) -> Result<Option<String>, MailIndexError> {
            unreachable!("reading bodies asks for no inbox")
        }

        fn known_uids(
            &self,
            _folder_id: FolderId,
            _scope: &UidScope,
        ) -> Result<Vec<Uid>, MailIndexError> {
            unreachable!("reading bodies asks for no numbers")
        }

        fn placement_count(&self, _folder_id: FolderId) -> Result<usize, MailIndexError> {
            unreachable!("reading bodies counts nothing")
        }

        fn record_window_floor(
            &self,
            _folder_id: FolderId,
            _floor: Uid,
            _complete: bool,
        ) -> Result<(), MailIndexError> {
            unreachable!("reading bodies moves no floor")
        }

        fn apply(
            &self,
            _delta: &FolderDelta,
            _deletions: &[Uid],
            _cursor: &SyncCursor,
            _flag_scope: FlagScope,
        ) -> Result<AppliedSummary, MailIndexError> {
            unreachable!("reading bodies applies nothing")
        }

        fn reset_folder(
            &self,
            _folder_id: FolderId,
            _uid_validity: UidValidity,
        ) -> Result<(), MailIndexError> {
            unreachable!("reading bodies resets nothing")
        }

        fn forget_it_stopped_short(&self, _folder: FolderId) -> Result<(), MailIndexError> {
            unreachable!("reading bodies takes nothing back about a folder")
        }

        fn refused_for_now(
            &self,
            _folder: FolderId,
            _reading: Reading,
            _refused: bool,
        ) -> Result<(), MailIndexError> {
            unreachable!("reading bodies reads no new mail and nothing further back")
        }

        fn holds(
            &self,
            _folder: FolderId,
            _message: lyonoyl_domain::message::MessageId,
        ) -> Result<bool, MailIndexError> {
            unreachable!("reading bodies asks after no copy")
        }

        fn unplaced(
            &self,
            _account: i64,
        ) -> Result<Vec<lyonoyl_domain::message::MessageId>, MailIndexError> {
            unreachable!("reading bodies asks after no message")
        }

        fn forget_the_unplaced(
            &self,
            _account: i64,
            _named: &[lyonoyl_domain::message::MessageId],
        ) -> Result<(), MailIndexError> {
            unreachable!("reading bodies forgets nothing")
        }
    }

    /// What was kept, and for which message.
    #[derive(Default)]
    struct Kept(Mutex<Vec<(u64, String)>>);

    impl MessageBodies for &Kept {
        fn held(&self, _message: MessageId) -> Result<Option<Vec<u8>>, MessageBodyError> {
            Ok(None)
        }

        fn keep(&self, message: MessageId, raw: &[u8]) -> Result<(), MessageBodyError> {
            self.0
                .lock()
                .expect("not poisoned")
                .push((message.get(), String::from_utf8_lossy(raw).into_owned()));
            Ok(())
        }

        fn fill_in_missing(
            &self,
            _message: MessageId,
            _raw: &[u8],
        ) -> Result<(), MessageBodyError> {
            Ok(())
        }
    }

    fn folder() -> FolderId {
        FolderId::new(1).expect("non-zero")
    }

    #[test]
    fn asks_for_messages_together_rather_than_one_at_a_time() {
        // What a message costs is mostly not the message: against a server on this machine,
        // opening the connection and signing in was 74% of it, and fifty read one at a time cost
        // 4.4 times what the same fifty cost on one connection.
        let waiting: Vec<_> = (1..=25).map(|n| (message(n), uid(n as u32))).collect();
        let server = Server::answering();
        let kept = Kept::default();

        let round = FetchBodies::new(&server, stored(waiting), &kept)
            .execute(folder())
            .expect("the round runs");

        assert_eq!(round.fetched, 25);
        let requests = server.requests();
        assert_eq!(requests.len(), 3, "twenty-five in tens: {requests:?}");
        assert_eq!(requests[0].len(), 10);
        assert_eq!(requests[2].len(), 5);
    }

    #[test]
    fn never_asks_for_more_than_one_batch_at_a_time() {
        // The answer is held whole while it is stored, so a mailbox of large attachments would be
        // the whole batch in memory together. The cap is what keeps that bounded.
        let waiting: Vec<_> = (1..=50).map(|n| (message(n), uid(n as u32))).collect();
        let server = Server::answering();
        let kept = Kept::default();

        FetchBodies::new(&server, stored(waiting), &kept)
            .execute(folder())
            .expect("the round runs");

        for request in server.requests() {
            assert!(
                request.len() <= ASKED_FOR_AT_ONCE,
                "asked for {} at once",
                request.len()
            );
        }
    }

    #[test]
    fn gives_each_message_its_own_bytes_however_the_server_orders_them() {
        // Paired by number rather than by position. A server may answer about fewer than it was
        // asked and in any order; lining the answers up against the question would put one
        // person's mail under another's subject.
        let waiting = vec![
            (message(70), uid(7)),
            (message(80), uid(8)),
            (message(90), uid(9)),
        ];
        let server = Server::silent_about(&[8]);
        let kept = Kept::default();

        let round = FetchBodies::new(&server, stored(waiting), &kept)
            .execute(folder())
            .expect("the round runs");

        assert_eq!((round.fetched, round.failed), (2, 1));
        let stored = kept.0.lock().expect("not poisoned").clone();
        assert_eq!(
            stored,
            vec![(70, "message 7".to_owned()), (90, "message 9".to_owned())],
            "the one the server skipped took nobody else's bytes"
        );
    }

    #[test]
    fn a_refused_request_is_that_request_and_not_the_folder() {
        // One request failing must not write off the rest of the round: what is still missing is
        // what the index has not got, and the next round asks for exactly that.
        let waiting: Vec<_> = (1..=15).map(|n| (message(n), uid(n as u32))).collect();
        let kept = Kept::default();

        let round = FetchBodies::new(Refuses, stored(waiting), &kept)
            .execute(folder())
            .expect("the round runs");

        assert_eq!((round.fetched, round.failed), (0, 15));
        assert!(kept.0.lock().expect("not poisoned").is_empty());
    }

    /// The messages a store kept, by the number the index gave each.
    fn kept_in(kept: &Kept) -> Vec<u64> {
        kept.0
            .lock()
            .expect("not poisoned")
            .iter()
            .map(|(message, _)| *message)
            .collect()
    }

    /// Asked for together, then each alone.
    fn together_then_alone(numbers: std::ops::RangeInclusive<u32>) -> Vec<Vec<u32>> {
        let mut asked = vec![numbers.clone().collect::<Vec<u32>>()];
        asked.extend(numbers.map(|one| vec![one]));
        asked
    }

    #[test]
    fn a_message_the_server_cannot_read_costs_only_itself() {
        // The server drops the conversation over 8, and nothing asked for beside it arrives. Asked
        // for together on every round, the nine beside it are never read; asked for one at a time
        // once the request fails, they are, and 8 is the one message left waiting. 11 is alone in
        // its request, and asking it again by itself would be the same question twice.
        let waiting: Vec<_> = (1..=11).map(|n| (message(n), uid(n as u32))).collect();
        let server = Drops::over(&[8, 11]);
        let kept = Kept::default();

        let round = FetchBodies::new(&server, stored(waiting), &kept)
            .execute(folder())
            .expect("the round runs");

        assert_eq!(kept_in(&kept), vec![1, 2, 3, 4, 5, 6, 7, 9, 10]);
        assert_eq!((round.fetched, round.failed), (9, 2));

        let mut expected = together_then_alone(1..=10);
        expected.push(vec![11]);
        assert_eq!(server.requests(), expected);
    }

    #[test]
    fn messages_beside_one_the_server_cannot_read_are_read_however_many_before_them_could_not_be() {
        // Ten in a row the server cannot read — one damaged file holding all of them — and the
        // newest, so every round starts with them. A run of messages that failed alone says
        // nothing about the next request: the nine readable ones beside 15 are read in this round,
        // and in every round they would be.
        let waiting: Vec<_> = (1..=20).map(|n| (message(n), uid(n as u32))).collect();
        let mut unreadable: Vec<u32> = (1..=10).collect();
        unreadable.push(15);
        let server = Drops::over(&unreadable);
        let kept = Kept::default();

        let round = FetchBodies::new(&server, stored(waiting), &kept)
            .execute(folder())
            .expect("the round runs");

        assert_eq!(kept_in(&kept), vec![11, 12, 13, 14, 16, 17, 18, 19, 20]);
        assert_eq!((round.fetched, round.failed), (9, 11));

        let mut expected = together_then_alone(1..=10);
        expected.extend(together_then_alone(11..=20));
        assert_eq!(server.requests(), expected);
    }

    #[test]
    fn a_request_whose_answer_could_not_be_used_is_asked_again_a_message_at_a_time() {
        // Ten messages whose server stated sizes far below what they are: together they come to
        // more than one request may hold, and the answer is refused as one this build cannot use.
        // Asked for together on every round, none of them is ever read. Asked for alone, each has
        // the room a message has when a person opens it — all but 8, too large even for that.
        let waiting: Vec<_> = (1..=10).map(|n| (message(n), uid(n as u32))).collect();
        let server = Drops::over(&[8]).failing_with(MailProviderError::Protocol);
        let kept = Kept::default();

        let round = FetchBodies::new(&server, stored(waiting), &kept)
            .execute(folder())
            .expect("the round runs");

        assert_eq!(kept_in(&kept), vec![1, 2, 3, 4, 5, 6, 7, 9, 10]);
        assert_eq!((round.fetched, round.failed), (9, 1));
        assert_eq!(server.requests(), together_then_alone(1..=10));
    }

    #[test]
    fn a_server_that_fails_every_read_stops_being_asked_once_more_fail_alone_than_one_request_names(
    ) {
        // It takes the connection, the sign-in and the mailbox, and then fails every message asked
        // of it: it leaves over each, or it goes silent once a message is on its way and each
        // request waits out a minute. Asked a message at a time for every request of the round,
        // that is fifty-five requests and fifty-five connections a folder, on every pass. A run of
        // failures longer than one request is not one damaged message beside readable ones: the
        // round asks one message past it and stops.
        for failing_with in [MailProviderError::Offline, MailProviderError::Protocol] {
            let waiting: Vec<_> = (1..=25).map(|n| (message(n), uid(n as u32))).collect();
            let unreadable: Vec<u32> = (1..=25).collect();
            let server = Drops::over(&unreadable).failing_with(failing_with);
            let kept = Kept::default();

            let round = FetchBodies::new(&server, stored(waiting), &kept)
                .execute(folder())
                .expect("the round runs");

            assert!(kept_in(&kept).is_empty(), "{failing_with:?}");
            assert_eq!((round.fetched, round.failed), (0, 25), "{failing_with:?}");

            let mut expected = together_then_alone(1..=10);
            expected.extend([(11..=20).collect(), vec![11]]);
            assert_eq!(server.requests(), expected, "{failing_with:?}");
        }
    }

    #[test]
    fn a_request_answered_whole_starts_the_count_of_failures_again() {
        // Ten the server cannot read, then ten it hands over together, then one more it cannot
        // read beside nine it can. The run of failures ended with the ten that arrived: the one
        // after them is a damaged message beside readable ones, and the nine are read.
        let waiting: Vec<_> = (1..=30).map(|n| (message(n), uid(n as u32))).collect();
        let mut unreadable: Vec<u32> = (1..=10).collect();
        unreadable.push(21);
        let server = Drops::over(&unreadable);
        let kept = Kept::default();

        let round = FetchBodies::new(&server, stored(waiting), &kept)
            .execute(folder())
            .expect("the round runs");

        let mut read: Vec<u64> = (11..=20).collect();
        read.extend(22..=30);
        assert_eq!(kept_in(&kept), read);
        assert_eq!((round.fetched, round.failed), (19, 11));
    }

    #[test]
    fn each_message_read_alone_is_kept_before_the_next_is_asked_for() {
        // A server that states no size, or one lower than the message, may hand over as much as a
        // person opening a message is allowed, for each message asked for alone. Held until all of
        // them have come, that is ten of those allowances at once; kept as each arrives, it is one.
        let waiting: Vec<_> = (1..=3).map(|n| (message(n), uid(n as u32))).collect();
        let happened = Happened::default();

        let round = FetchBodies::new(
            OneAtATime(&happened),
            stored(waiting),
            KeptAsItCame(&happened),
        )
        .execute(folder())
        .expect("the round runs");

        assert_eq!((round.fetched, round.failed), (3, 0));
        assert_eq!(
            *happened.0.lock().expect("not poisoned"),
            ["handed 1", "kept 1", "handed 2", "kept 2", "handed 3", "kept 3"]
        );
    }

    #[test]
    fn a_server_nothing_can_be_asked_of_is_asked_once_a_round() {
        // Gone, or refusing the sign-in — before the question, or in answer to it. Nothing says one
        // message is the matter, and asking the rest one at a time — or the next ten — is a
        // connection tried and given up on each time, or a refused sign-in each time, counted
        // toward locking the account.
        for why in [
            Unread::NotAsked(MailProviderError::Offline),
            Unread::NotAsked(MailProviderError::Unauthorized),
            Unread::Failed(MailProviderError::Unauthorized),
            Unread::Failed(MailProviderError::Cancelled),
        ] {
            let waiting: Vec<_> = (1..=25).map(|n| (message(n), uid(n as u32))).collect();
            let server = CannotBeAsked {
                why,
                asked: Mutex::new(Vec::new()),
            };
            let kept = Kept::default();

            let round = FetchBodies::new(&server, stored(waiting), &kept)
                .execute(folder())
                .expect("the round runs");

            assert_eq!((round.fetched, round.failed), (0, 25), "{why:?}");
            assert_eq!(
                *server.asked.lock().expect("not poisoned"),
                vec![(1..=10).collect::<Vec<u32>>()],
                "{why:?}"
            );
        }
    }

    #[test]
    fn a_server_that_goes_while_messages_are_asked_for_alone_is_asked_nothing_more() {
        // The request of ten fails over 3, the first two come alone, 3 fails alone, and then the
        // server cannot be reached at all: neither the rest of those ten nor the next fifteen are
        // a question worth a connection this round.
        let waiting: Vec<_> = (1..=25).map(|n| (message(n), uid(n as u32))).collect();
        let server = Drops::over(&[3]).gone_after(4);
        let kept = Kept::default();

        let round = FetchBodies::new(&server, stored(waiting), &kept)
            .execute(folder())
            .expect("the round runs");

        assert_eq!(kept_in(&kept), vec![1, 2]);
        assert_eq!((round.fetched, round.failed), (2, 23));
        assert_eq!(
            server.requests(),
            vec![(1..=10).collect(), vec![1], vec![2], vec![3], vec![4]]
        );
    }

    #[test]
    fn every_request_is_made_under_the_generation_the_numbers_were_stored_in() {
        // A number means a message in its own generation and nowhere else. Asked for without it,
        // a mailbox renumbered since would hand over another message, kept as this one's body.
        let waiting: Vec<_> = (1..=15).map(|n| (message(n), uid(n as u32))).collect();
        let server = Server::answering();
        let kept = Kept::default();

        FetchBodies::new(&server, stored(waiting), &kept)
            .execute(folder())
            .expect("the round runs");

        assert_eq!(
            *server.under.lock().expect("not poisoned"),
            vec![STORED_GENERATION, STORED_GENERATION]
        );
    }

    #[test]
    fn a_folder_with_no_generation_stored_is_not_read_by_its_numbers() {
        // Nothing can check those numbers against the mailbox. The folder stays one with messages
        // still to read rather than one that was read by numbers that may name somebody else.
        let waiting: Vec<_> = (1..=3).map(|n| (message(n), uid(n as u32))).collect();
        let server = Server::answering();
        let kept = Kept::default();

        let round = FetchBodies::new(&server, Waiting(waiting, None), &kept)
            .execute(folder())
            .expect("the round runs");

        assert!(server.requests().is_empty(), "asked by unchecked numbers");
        assert!(kept.0.lock().expect("not poisoned").is_empty());
        assert!(!round.complete, "a folder with messages still to read");
    }

    #[test]
    fn a_folder_with_nothing_waiting_says_so_and_asks_nothing() {
        let server = Server::answering();
        let kept = Kept::default();

        let round = FetchBodies::new(&server, stored(Vec::new()), &kept)
            .execute(folder())
            .expect("the round runs");

        assert!(round.complete);
        assert!(server.requests().is_empty(), "a request for nothing");
    }
}
