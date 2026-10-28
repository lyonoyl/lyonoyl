//! Handing what is waiting to a server, one account at a time.
//!
//! The rule that shapes everything here: SMTP is not idempotent. A conversation that ends after
//! the message has been handed over leaves nobody knowing whether it was accepted, and the only
//! safe thing to do with that is nothing — the message waits for a person. Every other outcome is
//! either worth trying again on a timer or worth stopping and saying so.

use crate::application::error::UseCaseError;
use crate::ports::drafts::{Draft, Drafts};
use crate::ports::mail_actions::{Filing, FilingError, RefusedSearch, SentMail};
use crate::ports::mail_submission::{MailSubmission, SubmissionError};
use crate::ports::outbox::{Held, Outbox, WhyHeld, A_COPY_SETTLES};
use lyonoyl_domain::outgoing::{EmailAddress, Outgoing};

/// How many messages one round hands over.
///
/// Small: each one is a conversation with a server, and a person watching an outbox wants the
/// first message gone rather than all of them at once.
const ROUND: usize = 5;

/// How long before a message that could not be handed over is offered again.
///
/// Climbing, and ending: a server that has refused this many times is not going to take it in the
/// next minute, and a queue that retries for ever is a queue nobody is told about.
const RETRY_SECONDS: [u64; 4] = [30, 120, 600, 3600];

/// A message going back to the drafts, written the way a draft is written.
///
/// The addresses go back as the person typed them rather than as the parser left them: a draft
/// holds one unsplit line per field, and that is what the composer puts back in the box.
fn as_a_draft(message: &Outgoing) -> Draft {
    let joined = |addresses: &[EmailAddress]| {
        addresses
            .iter()
            .map(|address| address.as_str().to_owned())
            .collect::<Vec<_>>()
            .join(", ")
    };

    Draft {
        recipients: joined(&message.to),
        copies: joined(&message.cc),
        blind: joined(&message.bcc),
        subject: message.subject.as_str().to_owned(),
        body: message.body.clone(),
        answering: message
            .thread
            .answering
            .as_ref()
            .map(|named| named.as_str().to_owned()),
        chain: message
            .thread
            .chain
            .iter()
            .map(|one| one.as_str().to_owned())
            .collect(),
    }
}

/// What one round did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Delivered {
    /// Messages that will not be tried again and went back to the drafts in this round. One
    /// another round gave back already is back once, and counted once.
    pub given_back: u32,
    pub sent: u32,
    /// Copies put in the folder the account keeps sent mail in, for the servers that do not do it.
    pub filed: u32,
    /// Copies a server had already put there itself.
    pub found: u32,
    /// Sent messages let go with no copy filed, because their server will not take one.
    pub unfiled: u32,
    /// Left for later: the server could not be reached.
    pub waiting: u32,
}

pub struct DeliverOutbox<O: Outbox, S: MailSubmission, K: SentMail, D: Drafts> {
    outbox: O,
    /// Where a message is handed over. An account nobody has given a sending server has nowhere,
    /// and only the handing over waits for one: nothing else a round does needs it.
    submission: Option<S>,
    keeping: K,
    drafts: D,
}

impl<O: Outbox, S: MailSubmission, K: SentMail, D: Drafts> DeliverOutbox<O, S, K, D> {
    pub fn new(outbox: O, submission: S, keeping: K, drafts: D) -> Self {
        Self {
            outbox,
            submission: Some(submission),
            keeping,
            drafts,
        }
    }

    /// The round of an account nobody has given a sending server, or whose server was taken away.
    ///
    /// It hands nothing over, and what is waiting stays waiting, with no reason given: once it is
    /// late, the activity line counts it as on its way, until the account is given a sending server
    /// again. The rest of the round runs as it does anywhere: a message a round left unfinished goes
    /// back to the drafts, and one a server took is filed. Without that, a message sent
    /// before its account lost the server would stay on this device for good — everybody it went
    /// to, blind copies included, and every file it carried — and never reach the folder for sent
    /// mail.
    pub fn with_no_sending_server(outbox: O, keeping: K, drafts: D) -> Self {
        Self {
            outbox,
            submission: None,
            keeping,
            drafts,
        }
    }

    pub fn execute(&self, account_id: &str) -> Result<Delivered, UseCaseError> {
        let mut done = Delivered::default();

        // Rows that name the draft their message came back as go before anything else in the
        // round, so a row that stops the round further down cannot keep them: that draft holds the
        // message, and a message back in the drafts is kept nowhere else. A message whose copy is
        // filed is not let go here: the read of the account that follows lets it go, and until
        // then the list of sent mail shows it from its row.
        self.outbox.forget_given_back(account_id)?;

        // What a round took to hand over and never said what became of: the machine stopped, or
        // the outcome could not be written down. The server may have it, and nobody can find out
        // from here, so it is what a conversation that ended after the message was handed over
        // is — back to the drafts, where nothing but the person sends it again.
        for message in self.outbox.interrupted(account_id)? {
            if self.give_back(account_id, &message)? {
                done.given_back += 1;
            }
        }

        if let Some(submission) = &self.submission {
            self.hand_over(submission, account_id, &mut done)?;
        }

        self.keep_copies(account_id, &mut done)?;

        Ok(done)
    }

    /// Files the copies that are due, and nothing else: nothing is handed over and nothing is let
    /// go. What follows a message going out once its server has had its moment.
    pub fn file_copies(&self, account_id: &str) -> Result<Delivered, UseCaseError> {
        let mut done = Delivered::default();
        self.keep_copies(account_id, &mut done)?;
        Ok(done)
    }

    /// Hands what is ready to the server, a round's worth at most.
    fn hand_over(
        &self,
        submission: &S,
        account_id: &str,
        done: &mut Delivered,
    ) -> Result<(), UseCaseError> {
        for message in self.outbox.ready(account_id, ROUND)? {
            // Taken, on the disk, before a byte of it leaves. A message that is not waiting any
            // more is not this round's to hand over, and one taken here and never given an outcome
            // is found by the next round above rather than handed over a second time.
            if !self.outbox.claim(message.id)? {
                continue;
            }

            match submission.submit(&message.message_id, &message.message, message.written_at) {
                Ok(()) => {
                    self.outbox.accepted(message.id)?;
                    done.sent += 1;
                }
                Err(why) => {
                    match what_now(why, message.attempts) {
                        // Offered again once the wait is over: a server that could not be reached
                        // says nothing about the message.
                        (held, Some(wait)) => {
                            self.outbox.stopped(message.id, held, Some(wait))?;
                            done.waiting += 1;
                        }
                        // A message that will not be tried again goes back to the drafts, and the
                        // outbox lets go of it in the change that writes the draft. Without that,
                        // what somebody wrote would end in a row of the outbox that nothing in the
                        // product reads: the composer discards the draft when the message is
                        // queued, so a refused password would leave the first message a person
                        // wrote gone, with the window saying it had been sent. Nothing is said on
                        // the row afterwards, because there is no row.
                        //
                        // The failure to write it is not swallowed: the message stays on its way,
                        // and the next round gives it back.
                        (_, None) => {
                            if self.give_back(account_id, &message)? {
                                done.given_back += 1;
                            }
                        }
                    }

                    // A server that cannot be reached will not be reached by the next message
                    // either, and asking it four more times is four more waits for somebody
                    // watching one message that has not gone.
                    if why == SubmissionError::Unreachable {
                        break;
                    }
                }
            }
        }

        Ok(())
    }

    /// Puts a message that will not be tried again back where the person can send it again, and
    /// answers whether it came back now.
    ///
    /// By its row, so the files it carried go back with it in the change that writes the draft,
    /// and the outbox lets go of the row in that change: once it is back in the drafts, the draft
    /// is the only place on this device it is. A message given back without its files is sent
    /// again without them, by somebody who never notices. By its row as well, so a message the
    /// outbox no longer holds, because it came back already, does not come back again.
    fn give_back(&self, account_id: &str, message: &Held) -> Result<bool, UseCaseError> {
        Ok(self
            .drafts
            .came_back(account_id, message.id, &as_a_draft(&message.message))?
            .is_some())
    }

    /// Puts a copy of what has been sent where the account keeps sent mail.
    ///
    /// Never in the same breath as sending it. A server that files a sent message itself takes a
    /// moment about it, and asking too soon finds nothing and leaves the person with two copies of
    /// everything they send — which is worse than the wait.
    ///
    /// Filing is the half of this that may be tried again. Sending is not, and the row says which
    /// has happened, so a failure here can never turn into a message handed over twice.
    ///
    /// Each copy is filed or not on its own. A server that refuses one has said something about
    /// that copy and nothing about the next, so the next is asked all the same; what ends the round
    /// is a folder that could not be reached, which every copy after it would meet as well.
    ///
    /// A copy's filing is over once it is there, once the account turns out to have nowhere to put
    /// it, and once its server will not take it, and that ends the message's stay on this device:
    /// the outbox lets it go in the change that records it. A copy refused for now is asked about
    /// again after a wait, and stays until then.
    ///
    /// A server that refuses to search for the copy before it is offered does not end its filing.
    /// While a refused copy has waits left, a refusal that names no copy and does not say the
    /// folder is not there or not this account's to add to counts as one of the copy for now; on
    /// the try after the last of them, it is passed over and the copy offered all the same. Kept
    /// back for good, a copy on a server that will not search is never filed anywhere. Offered, it
    /// can be a second one: on a server that files its own, and on one that kept a copy this device
    /// offered on an earlier try whose answer never came, which only the search would have found.
    fn keep_copies(&self, account_id: &str, done: &mut Delivered) -> Result<(), UseCaseError> {
        for message in self
            .outbox
            .awaiting_filing(account_id, A_COPY_SETTLES, ROUND)?
        {
            let next_wait = COPY_RETRY_SECONDS.get(message.copy_refused as usize);
            let refused_search = match next_wait {
                Some(_) => RefusedSearch::Heeded,
                None => RefusedSearch::PassedOver,
            };

            match self.keeping.file(
                &message.message_id,
                &message.message,
                message.written_at,
                refused_search,
            ) {
                Ok(Filing::Added) => {
                    self.outbox.filed(message.id)?;
                    done.filed += 1;
                }
                Ok(Filing::AlreadyThere) => {
                    self.outbox.filed(message.id)?;
                    done.found += 1;
                }
                // Nowhere to put it: an answer, and not worth asking about again.
                Ok(Filing::Nowhere) => self.outbox.filed(message.id)?,
                Err(FilingError::RefusedForNow) => match next_wait {
                    Some(&wait) => self.outbox.refused_for_now(message.id, wait)?,
                    // Refused after the last wait as well: a refusal that has not passed in more
                    // than a day is taken for one that will not.
                    None => self.unfiled(message.id, done)?,
                },
                Err(FilingError::RefusedForGood) => self.unfiled(message.id, done)?,
                // Nothing is written down, so it comes back next round.
                Err(FilingError::FolderUnreachable) => break,
            }
        }

        Ok(())
    }

    /// Lets go of a sent message whose server will not take its copy.
    ///
    /// Its filing is over all the same, and a sent message is not kept on this device once there is
    /// nothing left to do with it: kept, it would be everybody it went to and every file it
    /// carried, waiting here for a copy that is never coming.
    fn unfiled(&self, id: i64, done: &mut Delivered) -> Result<(), UseCaseError> {
        self.outbox.filed(id)?;
        done.unfiled += 1;
        Ok(())
    }
}

/// How long a copy its server refused for now waits before it is asked about again.
///
/// Climbing, so a refusal that passes in a moment — part of the server down, the folder in use —
/// costs minutes, and a mailbox over quota is given a day for somebody to make room in it. And
/// ending: a copy refused once more after the last of these counts as refused for good. Asked about
/// every round instead, a copy that will never be taken is the whole message uploaded again at
/// every one of them, for ever.
///
/// A search for the copy that the server refuses without naming it, and without saying the folder
/// is not there or not this account's to add to, waits these out as a refused copy does, and on
/// the try after the last of them it is passed over, so the copy is offered before its message
/// goes.
const COPY_RETRY_SECONDS: [u64; 4] = [600, 3600, 6 * 3600, 24 * 3600];

/// What becomes of a message the server would not take, and whether it is offered again.
///
/// `None` means it is not: a person decides. That is the answer for everything except a server
/// that could not be reached, which is the one failure that says nothing about the message.
fn what_now(why: SubmissionError, attempts: u32) -> (WhyHeld, Option<u64>) {
    match why {
        SubmissionError::Unreachable => (
            WhyHeld::TheServerRefused,
            RETRY_SECONDS.get(attempts as usize).copied(),
        ),
        SubmissionError::SignInFailed => (WhyHeld::SignInFailed, None),
        SubmissionError::Refused => (WhyHeld::TheServerRefused, None),
        // The one that must never be tried again by anything other than a person.
        SubmissionError::NobodyKnows => (WhyHeld::NobodyKnows, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::drafts::{DraftError, DraftFile, Kept};
    use crate::ports::outbox::OutboxError;
    use lyonoyl_domain::outgoing::{EmailAddress, Outgoing, Subject};
    use std::sync::Mutex;

    #[derive(Default)]
    struct Queue {
        waiting: Mutex<Vec<Held>>,
        /// Taken by a round that never said what became of them.
        interrupted: Mutex<Vec<Held>>,
        /// A disk that cannot say which messages a round took and never finished.
        interrupted_unreadable: bool,
        /// Taken to be handed over, in the order they were taken.
        claimed: Mutex<Vec<i64>>,
        /// Not waiting any more by the time they are taken: another round has them.
        gone: Vec<i64>,
        /// A disk that will not take a claim.
        claims_fail: bool,
        accepted: Mutex<Vec<i64>>,
        stopped: Mutex<Vec<(i64, WhyHeld, Option<u64>)>>,
        /// Messages a server has taken, waiting for a copy to be kept.
        to_file: Mutex<Vec<Held>>,
        filed: Mutex<Vec<i64>>,
        /// Copies refused for now, and how long each was told to wait before it is asked again.
        refused_for_now: Mutex<Vec<(i64, u64)>>,
        /// Accounts whose messages already filed were let go, in the order it was asked.
        forgotten: Mutex<Vec<String>>,
        /// Accounts whose rows naming the draft their message came back as were let go.
        forgotten_given_back: Mutex<Vec<String>>,
    }

    /// A place that will not keep anything, however often it is asked.
    struct Refuses;

    impl SentMail for Refuses {
        fn file(
            &self,
            _message_id: &str,
            _message: &Outgoing,
            _written_at: i64,
            _refused_search: RefusedSearch,
        ) -> Result<Filing, FilingError> {
            Err(FilingError::FolderUnreachable)
        }
    }

    /// A server that does not file sent mail itself and will not search for a copy, and what it
    /// answers once the copy is offered all the same.
    struct RefusesTheSearch {
        offered: Result<Filing, FilingError>,
        /// What each try was told to make of the refusal, in the order they came.
        tries: Mutex<Vec<RefusedSearch>>,
    }

    impl SentMail for RefusesTheSearch {
        fn file(
            &self,
            _message_id: &str,
            _message: &Outgoing,
            _written_at: i64,
            refused_search: RefusedSearch,
        ) -> Result<Filing, FilingError> {
            self.tries.lock().expect("test lock").push(refused_search);

            match refused_search {
                RefusedSearch::Heeded => Err(FilingError::RefusedForNow),
                RefusedSearch::PassedOver => self.offered,
            }
        }
    }

    /// Somewhere for a message given back to come back to.
    #[derive(Default)]
    struct CameBack {
        drafts: Mutex<Vec<Draft>>,
        /// The outbox row each was asked to come back from, in the order they were asked.
        from: Mutex<Vec<i64>>,
        /// Rows the outbox no longer holds: another round gave their messages back already.
        gone: Vec<i64>,
    }

    impl Drafts for &CameBack {
        fn keep(
            &self,
            _account_id: &str,
            _under: Option<i64>,
            _draft: &Draft,
            _files: &[DraftFile],
        ) -> Result<i64, DraftError> {
            unreachable!("a message comes back by its row, with what it carried")
        }

        fn files(&self, _id: i64) -> Result<Vec<DraftFile>, DraftError> {
            unreachable!("delivering reads no draft")
        }

        fn came_back(
            &self,
            _account_id: &str,
            from_outbox: i64,
            draft: &Draft,
        ) -> Result<Option<i64>, DraftError> {
            self.from.lock().expect("the drafts").push(from_outbox);
            if self.gone.contains(&from_outbox) {
                return Ok(None);
            }
            let mut drafts = self.drafts.lock().expect("the drafts");
            drafts.push(draft.clone());
            Ok(Some(drafts.len() as i64))
        }

        fn latest(&self, _account_id: &str) -> Result<Option<Kept>, DraftError> {
            Ok(None)
        }

        fn all(&self, _account_id: &str) -> Result<Vec<Kept>, DraftError> {
            Ok(Vec::new())
        }

        fn discard(&self, _id: i64) -> Result<(), DraftError> {
            Ok(())
        }
    }

    /// A place that keeps a copy, remembering what it was asked to keep.
    #[derive(Default)]
    struct Keeps {
        kept: Mutex<Vec<String>>,
        /// What it was told to make of a refused search, copy by copy, in the order it was asked.
        searches: Mutex<Vec<RefusedSearch>>,
        answer: Option<Filing>,
        /// The rows whose copies it refuses, and how.
        refusing: Vec<(i64, FilingError)>,
    }

    impl SentMail for Keeps {
        fn file(
            &self,
            message_id: &str,
            message: &Outgoing,
            written_at: i64,
            refused_search: RefusedSearch,
        ) -> Result<Filing, FilingError> {
            self.searches
                .lock()
                .expect("test lock")
                .push(refused_search);
            self.kept
                .lock()
                .expect("test lock")
                // What went in rather than how long it was: the use case no longer assembles the
                // text, and a test asserting on its length would be asserting about the adapter.
                // The date travels with it, because the copy filed here and the message that went
                // out are dated alike only if both were handed the same one.
                .push(format!(
                    "{message_id}:{}@{written_at}",
                    message.subject.as_str()
                ));

            match self
                .refusing
                .iter()
                .find(|(id, _)| message_id == format!("<{id}@example.com>"))
            {
                Some((_, refusal)) => Err(*refusal),
                None => Ok(self.answer.unwrap_or(Filing::Added)),
            }
        }
    }

    impl Keeps {
        /// The rows whose copies it was asked about, in the order it was asked: `held` names a
        /// message after its row.
        fn asked(&self) -> Vec<i64> {
            self.kept
                .lock()
                .expect("test lock")
                .iter()
                .map(|entry| {
                    entry
                        .trim_start_matches('<')
                        .split('@')
                        .next()
                        .and_then(|number| number.parse().ok())
                        .expect("a name `held` gave")
                })
                .collect()
        }

        /// The rows whose copies it was asked about, each with what it was told to make of a
        /// refused search, in the order it was asked.
        fn told(&self) -> Vec<(i64, RefusedSearch)> {
            self.asked()
                .into_iter()
                .zip(self.searches.lock().expect("test lock").iter().copied())
                .collect()
        }
    }

    impl Outbox for Queue {
        fn queue(&self, _: &str, _: &str, _: &Outgoing) -> Result<(), OutboxError> {
            unreachable!("delivering writes nothing down")
        }

        fn ready(&self, _account_id: &str, at_most: usize) -> Result<Vec<Held>, OutboxError> {
            let waiting = self.waiting.lock().expect("test lock");
            Ok(waiting.iter().take(at_most).cloned().collect())
        }

        fn claim(&self, id: i64) -> Result<bool, OutboxError> {
            if self.claims_fail {
                return Err(OutboxError::Unavailable);
            }
            if self.gone.contains(&id) {
                return Ok(false);
            }
            self.claimed.lock().expect("test lock").push(id);
            Ok(true)
        }

        fn interrupted(&self, _account_id: &str) -> Result<Vec<Held>, OutboxError> {
            if self.interrupted_unreadable {
                return Err(OutboxError::Unavailable);
            }
            Ok(self.interrupted.lock().expect("test lock").clone())
        }

        fn awaiting_filing(
            &self,

            _account_id: &str,

            _settled: std::time::Duration,

            _at_most: usize,
        ) -> Result<Vec<Held>, OutboxError> {
            Ok(self.to_file.lock().expect("test lock").clone())
        }

        fn refused_for_now(&self, id: i64, retry_after: u64) -> Result<(), OutboxError> {
            self.refused_for_now
                .lock()
                .expect("test lock")
                .push((id, retry_after));
            // Counted on the row, as the outbox counts it, so the round that asks again reads it.
            self.to_file
                .lock()
                .expect("test lock")
                .iter_mut()
                .filter(|held| held.id == id)
                .for_each(|held| held.copy_refused += 1);
            Ok(())
        }

        fn filed(&self, id: i64) -> Result<(), OutboxError> {
            self.filed.lock().expect("test lock").push(id);
            self.to_file
                .lock()
                .expect("test lock")
                .retain(|held| held.id != id);
            Ok(())
        }

        fn forget_filed(&self, account_id: &str) -> Result<(), OutboxError> {
            self.forgotten
                .lock()
                .expect("test lock")
                .push(account_id.to_owned());
            Ok(())
        }

        fn forget_given_back(&self, account_id: &str) -> Result<(), OutboxError> {
            self.forgotten_given_back
                .lock()
                .expect("test lock")
                .push(account_id.to_owned());
            Ok(())
        }

        fn accepted(&self, id: i64) -> Result<(), OutboxError> {
            self.accepted.lock().expect("test lock").push(id);
            Ok(())
        }

        fn stopped(
            &self,
            id: i64,
            why: WhyHeld,
            retry_after: Option<u64>,
        ) -> Result<(), OutboxError> {
            self.stopped
                .lock()
                .expect("test lock")
                .push((id, why, retry_after));
            Ok(())
        }
    }

    struct Server {
        answers: Mutex<Vec<Result<(), SubmissionError>>>,
        asked: Mutex<Vec<String>>,
    }

    impl Server {
        fn saying(answers: Vec<Result<(), SubmissionError>>) -> Self {
            Self {
                answers: Mutex::new(answers.into_iter().rev().collect()),
                asked: Mutex::new(Vec::new()),
            }
        }
    }

    impl MailSubmission for Server {
        fn submit(
            &self,
            message_id: &str,
            _message: &Outgoing,
            written_at: i64,
        ) -> Result<(), SubmissionError> {
            self.asked
                .lock()
                .expect("test lock")
                .push(format!("{message_id}@{written_at}"));
            self.answers
                .lock()
                .expect("test lock")
                .pop()
                .unwrap_or(Ok(()))
        }
    }

    fn held(id: i64, attempts: u32) -> Held {
        Held {
            written_at: 1_700_000_000,
            id,
            message_id: format!("<{id}@example.com>"),
            message: Outgoing {
                files: Vec::new(),
                from: EmailAddress::parse("zeta@example.com").expect("valid"),
                to: vec![EmailAddress::parse("kappa@example.net").expect("valid")],
                cc: Vec::new(),
                bcc: Vec::new(),
                subject: Subject::parse("").expect("empty is a subject"),
                body: String::new(),
                thread: Default::default(),
            },
            attempts,
            copy_refused: 0,
        }
    }

    /// A message a server took, whose copy has been refused for now `times` already.
    fn refused(id: i64, times: u32) -> Held {
        Held {
            copy_refused: times,
            ..held(id, 0)
        }
    }

    fn queue_of(messages: Vec<Held>) -> Queue {
        Queue {
            waiting: Mutex::new(messages),
            ..Queue::default()
        }
    }

    #[test]
    fn a_copy_is_kept_where_the_account_keeps_sent_mail() {
        // A message sent and filed nowhere is one the person cannot find afterwards, and servers
        // differ on whether they file it themselves.
        let queue = Queue {
            to_file: Mutex::new(vec![held(7, 0)]),
            ..Queue::default()
        };
        let keeps = Keeps::default();

        let done = DeliverOutbox::new(
            &queue,
            &Server::saying(Vec::new()),
            &keeps,
            &CameBack::default(),
        )
        .execute("acc-1")
        .expect("the round runs");

        assert_eq!(done.filed, 1);
        assert_eq!(*queue.filed.lock().expect("test lock"), vec![7]);
        assert_eq!(keeps.kept.lock().expect("test lock").len(), 1);
    }

    #[test]
    fn a_server_that_filed_it_itself_is_not_given_a_second_copy() {
        // The one mistake this whole path exists to avoid: two of everything the person sends.
        let queue = Queue {
            to_file: Mutex::new(vec![held(7, 0)]),
            ..Queue::default()
        };
        let keeps = Keeps {
            answer: Some(Filing::AlreadyThere),
            ..Keeps::default()
        };

        let done = DeliverOutbox::new(
            &queue,
            &Server::saying(Vec::new()),
            &keeps,
            &CameBack::default(),
        )
        .execute("acc-1")
        .expect("the round runs");

        assert_eq!(done.filed, 0, "nothing was added");
        assert_eq!(
            *queue.filed.lock().expect("test lock"),
            vec![7],
            "and it is not asked about again"
        );
    }

    #[test]
    fn filing_the_copies_files_what_is_due_and_hands_nothing_over() {
        // What follows a message going out once its server has had its moment: the copies, and
        // nothing a round does besides. A message still waiting is not handed over here.
        let queue = Queue {
            waiting: Mutex::new(vec![held(3, 0)]),
            to_file: Mutex::new(vec![held(7, 0)]),
            ..Queue::default()
        };
        let keeps = Keeps::default();

        let done = DeliverOutbox::new(
            &queue,
            &Server::saying(Vec::new()),
            &keeps,
            &CameBack::default(),
        )
        .file_copies("acc-1")
        .expect("the copies are filed");

        assert_eq!(
            done,
            Delivered {
                filed: 1,
                ..Delivered::default()
            }
        );
        assert_eq!(*queue.filed.lock().expect("test lock"), vec![7]);
        assert!(
            queue.claimed.lock().expect("test lock").is_empty(),
            "a message waiting was taken to be handed over"
        );
    }

    #[test]
    fn a_copy_its_server_filed_itself_is_counted_as_found() {
        // Found is what says the copy is on the server to be read back, as much as one filed here.
        let queue = Queue {
            to_file: Mutex::new(vec![held(7, 0)]),
            ..Queue::default()
        };
        let keeps = Keeps {
            answer: Some(Filing::AlreadyThere),
            ..Keeps::default()
        };

        let done = DeliverOutbox::new(
            &queue,
            &Server::saying(Vec::new()),
            &keeps,
            &CameBack::default(),
        )
        .file_copies("acc-1")
        .expect("the copies are filed");

        assert_eq!(
            done,
            Delivered {
                found: 1,
                ..Delivered::default()
            }
        );
    }

    #[test]
    fn a_copy_that_could_not_be_kept_is_never_a_message_sent_twice() {
        // Filing is the half that may be tried again. Sending is not, and the row says which has
        // happened, so a server that could not be reached here costs a copy and never a duplicate.
        let queue = Queue {
            to_file: Mutex::new(vec![held(7, 0)]),
            ..Queue::default()
        };
        let keeps = Refuses;

        let done = DeliverOutbox::new(
            &queue,
            &Server::saying(Vec::new()),
            &keeps,
            &CameBack::default(),
        )
        .execute("acc-1")
        .expect("the round runs");

        assert_eq!(done.filed, 0);
        assert!(
            queue.filed.lock().expect("test lock").is_empty(),
            "it must come back next round"
        );
        assert!(
            queue.accepted.lock().expect("test lock").is_empty(),
            "nothing was handed to a submission server here"
        );
    }

    #[test]
    fn an_account_with_nowhere_to_file_a_copy_lets_the_message_go_all_the_same() {
        // No folder for sent mail is an answer, and the one way filing ends without a copy:
        // asking again will not make a folder appear, and a sent message is not kept on this
        // device once there is nothing left to do with it.
        let queue = Queue {
            to_file: Mutex::new(vec![held(7, 0)]),
            ..Queue::default()
        };
        let keeps = Keeps {
            answer: Some(Filing::Nowhere),
            ..Keeps::default()
        };

        let done = DeliverOutbox::new(
            &queue,
            &Server::saying(Vec::new()),
            &keeps,
            &CameBack::default(),
        )
        .execute("acc-1")
        .expect("the round runs");

        assert_eq!(done.filed, 0, "nothing was added");
        assert_eq!(
            *queue.filed.lock().expect("test lock"),
            vec![7],
            "a message with nowhere to be filed was kept to be asked about again"
        );
    }

    #[test]
    fn a_copy_its_server_refuses_does_not_stop_the_copies_behind_it() {
        // Oldest first, and each on its own: a message too large for the server says nothing about
        // the short one sent after it, and one refused for now says nothing about it either.
        // Stopped at the first refusal, every later message of the account stays out of its
        // folder for sent mail, and on this device with everybody it went to.
        let queue = Queue {
            to_file: Mutex::new(vec![held(1, 0), held(2, 0), held(3, 0)]),
            ..Queue::default()
        };
        let keeps = Keeps {
            refusing: vec![
                (1, FilingError::RefusedForGood),
                (2, FilingError::RefusedForNow),
            ],
            ..Keeps::default()
        };

        let done = DeliverOutbox::new(
            &queue,
            &Server::saying(Vec::new()),
            &keeps,
            &CameBack::default(),
        )
        .execute("acc-1")
        .expect("the round runs");

        assert_eq!(
            keeps.asked(),
            vec![1, 2, 3],
            "a refused copy kept the ones behind it from being asked about"
        );
        assert_eq!(*queue.filed.lock().expect("test lock"), vec![1, 3]);
        assert_eq!(
            done,
            Delivered {
                filed: 1,
                unfiled: 1,
                ..Delivered::default()
            }
        );
    }

    #[test]
    fn a_copy_refused_for_now_is_asked_about_again_after_a_wait_and_its_message_stays() {
        // Over quota, a part of the server down, the folder in use: it may well be taken later, so
        // the message stays whole until it is, and the next round that comes before the wait is
        // over does not upload it again.
        let queue = Queue {
            to_file: Mutex::new(vec![held(7, 0)]),
            ..Queue::default()
        };
        let keeps = Keeps {
            refusing: vec![(7, FilingError::RefusedForNow)],
            ..Keeps::default()
        };

        let done = DeliverOutbox::new(
            &queue,
            &Server::saying(Vec::new()),
            &keeps,
            &CameBack::default(),
        )
        .execute("acc-1")
        .expect("the round runs");

        assert!(
            queue.filed.lock().expect("test lock").is_empty(),
            "a message whose copy may still be taken was let go"
        );
        assert_eq!(
            *queue.refused_for_now.lock().expect("test lock"),
            vec![(7, COPY_RETRY_SECONDS[0])],
            "a copy refused for now was not told to wait before it is asked about again"
        );
        assert_eq!(done, Delivered::default());
    }

    #[test]
    fn a_copy_refused_for_good_is_not_asked_about_again_and_its_message_goes() {
        // Too large for the server, or something it can never do: no round will see it taken. Its
        // filing is over, and a sent message is not kept on this device once there is nothing left
        // to do with it.
        let queue = Queue {
            to_file: Mutex::new(vec![held(7, 0)]),
            ..Queue::default()
        };
        let keeps = Keeps {
            refusing: vec![(7, FilingError::RefusedForGood)],
            ..Keeps::default()
        };

        let done = DeliverOutbox::new(
            &queue,
            &Server::saying(Vec::new()),
            &keeps,
            &CameBack::default(),
        )
        .execute("acc-1")
        .expect("the round runs");

        assert_eq!(
            *queue.filed.lock().expect("test lock"),
            vec![7],
            "a message whose copy will never be taken was kept"
        );
        assert!(
            queue.refused_for_now.lock().expect("test lock").is_empty(),
            "a copy that will never be taken was told to wait for another try"
        );
        assert_eq!(
            done,
            Delivered {
                unfiled: 1,
                ..Delivered::default()
            }
        );
    }

    #[test]
    fn a_copy_refused_for_now_waits_longer_each_time_and_then_counts_as_refused_for_good() {
        // The bound that stops a copy being uploaded on every round for ever. One copy at each
        // count of refusals a row can hold before the last: each waits the next of the waits, and
        // the copy refused after all of them goes the way a copy refused for good does.
        let times = COPY_RETRY_SECONDS.len() as u32;
        let rows: Vec<i64> = (1..=i64::from(times) + 1).collect();
        let queue = Queue {
            to_file: Mutex::new(
                rows.iter()
                    .map(|&id| refused(id, (id - 1) as u32))
                    .collect(),
            ),
            ..Queue::default()
        };
        let keeps = Keeps {
            refusing: rows
                .iter()
                .map(|&id| (id, FilingError::RefusedForNow))
                .collect(),
            ..Keeps::default()
        };

        let done = DeliverOutbox::new(
            &queue,
            &Server::saying(Vec::new()),
            &keeps,
            &CameBack::default(),
        )
        .execute("acc-1")
        .expect("the round runs");

        assert_eq!(
            *queue.refused_for_now.lock().expect("test lock"),
            rows.iter()
                .zip(COPY_RETRY_SECONDS)
                .map(|(&id, wait)| (id, wait))
                .collect::<Vec<_>>(),
            "a copy refused for now did not wait the next of the waits"
        );
        assert!(
            COPY_RETRY_SECONDS.windows(2).all(|pair| pair[0] < pair[1]),
            "the waits do not climb"
        );
        assert_eq!(
            *queue.filed.lock().expect("test lock"),
            vec![i64::from(times) + 1],
            "a copy refused after the last wait was waited for again"
        );
        assert_eq!(done.unfiled, 1);
    }

    #[test]
    fn a_refused_search_is_passed_over_only_once_the_waits_are_over() {
        // The search for a copy already there is what keeps a server that files its own from being
        // given a second one. Passed over while a refused copy has waits left, it gives one to every
        // server that refuses it for a moment; heeded once they are over, a server that goes on
        // refusing it has every message the account sends go with its copy never offered. One copy
        // at each count of refusals that leaves a wait, one at the count that leaves none, and one
        // past it.
        let waits = COPY_RETRY_SECONDS.len() as u32;
        let queue = Queue {
            to_file: Mutex::new(
                (0..=waits + 1)
                    .map(|times| refused(i64::from(times) + 1, times))
                    .collect(),
            ),
            ..Queue::default()
        };
        let keeps = Keeps::default();

        DeliverOutbox::new(
            &queue,
            &Server::saying(Vec::new()),
            &keeps,
            &CameBack::default(),
        )
        .execute("acc-1")
        .expect("the round runs");

        let heeded = (0..waits).map(|times| (i64::from(times) + 1, RefusedSearch::Heeded));
        let passed_over =
            [waits, waits + 1].map(|times| (i64::from(times) + 1, RefusedSearch::PassedOver));
        assert_eq!(
            keeps.told(),
            heeded.chain(passed_over).collect::<Vec<_>>(),
            "a refused search was passed over with a wait left, or heeded with none"
        );
    }

    #[test]
    fn a_message_whose_search_is_refused_round_after_round_is_offered_on_the_last_try() {
        // A server that does not file sent mail itself and will not search for the copy. Every
        // round before the last wait is over counts the refusal against the copy and lets nothing
        // go; the round after it offers the copy though the search is refused, and the message
        // goes once that offer is answered or refused. A folder not reached on that try ends the
        // round as on any other, and the round after it offers the copy again.
        let waits = COPY_RETRY_SECONDS.len();
        for (offered, on_the_last_try, passed_over, let_go) in [
            (
                Ok(Filing::Added),
                Delivered {
                    filed: 1,
                    ..Delivered::default()
                },
                1,
                vec![7],
            ),
            (
                Err(FilingError::RefusedForGood),
                Delivered {
                    unfiled: 1,
                    ..Delivered::default()
                },
                1,
                vec![7],
            ),
            (
                Err(FilingError::RefusedForNow),
                Delivered {
                    unfiled: 1,
                    ..Delivered::default()
                },
                1,
                vec![7],
            ),
            (
                Err(FilingError::FolderUnreachable),
                Delivered::default(),
                2,
                vec![],
            ),
        ] {
            let queue = Queue {
                to_file: Mutex::new(vec![held(7, 0)]),
                ..Queue::default()
            };
            let server = RefusesTheSearch {
                offered,
                tries: Mutex::new(Vec::new()),
            };
            let round = || {
                DeliverOutbox::new(
                    &queue,
                    &Server::saying(Vec::new()),
                    &server,
                    &CameBack::default(),
                )
                .execute("acc-1")
                .expect("the round runs")
            };

            // A round for each wait, the one after the last of them, and one more.
            let rounds: Vec<Delivered> = (0..waits + 2).map(|_| round()).collect();

            assert_eq!(
                rounds[..waits],
                vec![Delivered::default(); waits],
                "a message went before the last wait was over: {offered:?}"
            );
            assert_eq!(
                *queue.refused_for_now.lock().expect("test lock"),
                COPY_RETRY_SECONDS.map(|wait| (7, wait)),
                "a refused search did not wait as a refused copy does: {offered:?}"
            );
            assert_eq!(
                *server.tries.lock().expect("test lock"),
                [
                    vec![RefusedSearch::Heeded; waits],
                    vec![RefusedSearch::PassedOver; passed_over],
                ]
                .concat(),
                "the copy was not offered on the last try, or offered before it: {offered:?}"
            );
            assert_eq!(rounds[waits], on_the_last_try, "{offered:?}");
            assert_eq!(
                *queue.filed.lock().expect("test lock"),
                let_go,
                "what the offer met did not decide whether the message goes: {offered:?}"
            );
        }
    }

    #[test]
    fn a_folder_that_cannot_be_reached_ends_the_round_and_counts_against_no_copy() {
        // Not reached, not signed in, not listed, not opened, a conversation that broke before the
        // copy was offered, or an offer answered out of step: the next copy would meet it too.
        // Nothing is written down about the copy, which is asked about again next round as if this
        // one had not happened.
        let queue = Queue {
            to_file: Mutex::new(vec![held(1, 0), held(2, 0)]),
            ..Queue::default()
        };
        let keeps = Keeps {
            refusing: vec![(1, FilingError::FolderUnreachable)],
            ..Keeps::default()
        };

        let done = DeliverOutbox::new(
            &queue,
            &Server::saying(Vec::new()),
            &keeps,
            &CameBack::default(),
        )
        .execute("acc-1")
        .expect("the round runs");

        assert_eq!(
            keeps.asked(),
            vec![1],
            "a folder nothing reached was asked about again for the next copy"
        );
        assert!(queue.filed.lock().expect("test lock").is_empty());
        assert!(
            queue.refused_for_now.lock().expect("test lock").is_empty(),
            "a copy nobody answered about was counted as refused"
        );
        assert_eq!(done, Delivered::default());
    }

    #[test]
    fn a_round_lets_go_of_nothing_filed() {
        // A message whose copy is filed stays in the list of sent mail from its row until the
        // account is read back. A round reads nothing, so a round that let it go would take it out
        // of that list for as long as the read takes.
        let queue = Queue {
            to_file: Mutex::new(vec![held(7, 0)]),
            ..Queue::default()
        };

        DeliverOutbox::new(
            &queue,
            &Server::saying(Vec::new()),
            &Keeps::default(),
            &CameBack::default(),
        )
        .execute("acc-1")
        .expect("the round runs");

        assert_eq!(*queue.filed.lock().expect("test lock"), vec![7]);
        assert!(
            queue.forgotten.lock().expect("test lock").is_empty(),
            "a round let go of what is filed before the account was read back"
        );
    }

    #[test]
    fn rows_that_name_the_draft_their_message_came_back_as_go_before_anything_else_in_the_round() {
        // The draft holds everything such a row does, so nothing in a round reads one, and a
        // round stopped further down must not keep it either: stopped here at the first thing it
        // asks after that, it is stopped before every other step it has.
        let queue = Queue {
            interrupted_unreadable: true,
            ..Queue::default()
        };

        let outcome = DeliverOutbox::new(
            &queue,
            &Server::saying(Vec::new()),
            &Keeps::default(),
            &CameBack::default(),
        )
        .execute("acc-1");

        assert!(
            matches!(outcome, Err(UseCaseError::StorageUnavailable)),
            "{outcome:?}"
        );
        assert_eq!(
            *queue.forgotten_given_back.lock().expect("test lock"),
            vec!["acc-1".to_owned()],
            "the account's rows of messages back in the drafts were kept"
        );
    }

    #[test]
    fn an_account_with_no_sending_server_still_files_what_was_sent() {
        // A sending server taken away after a message went out and before its copy was filed. The
        // round hands nothing over and does the rest: what a round left unfinished goes back to
        // the drafts, and what a server took is filed.
        // A round skipped whole keeps that message on this device for good, and out of Sent.
        let queue = Queue {
            waiting: Mutex::new(vec![held(1, 0)]),
            interrupted: Mutex::new(vec![held(2, 0)]),
            to_file: Mutex::new(vec![held(3, 0)]),
            ..Queue::default()
        };
        let keeps = Keeps::default();
        let came_back = CameBack::default();

        let done =
            DeliverOutbox::<_, &Server, _, _>::with_no_sending_server(&queue, &keeps, &came_back)
                .execute("acc-1")
                .expect("the round runs");

        assert_eq!(
            done,
            Delivered {
                given_back: 1,
                filed: 1,
                ..Delivered::default()
            }
        );
        assert_eq!(
            *queue.filed.lock().expect("test lock"),
            vec![3],
            "a message a server took was not filed"
        );
        assert_eq!(
            *came_back.from.lock().expect("test lock"),
            vec![2],
            "a message a round left unfinished did not go back to the drafts"
        );
        assert!(
            queue.claimed.lock().expect("test lock").is_empty(),
            "a message was taken to be handed over with nowhere to hand it"
        );
    }

    #[test]
    fn a_message_a_server_takes_stops_waiting() {
        let queue = queue_of(vec![held(1, 0)]);
        let server = Server::saying(vec![Ok(())]);

        let done = DeliverOutbox::new(&queue, &server, &Keeps::default(), &CameBack::default())
            .execute("acc-1")
            .expect("the round runs");

        assert_eq!(
            done,
            Delivered {
                sent: 1,
                ..Delivered::default()
            }
        );
        assert_eq!(*queue.accepted.lock().expect("test lock"), vec![1]);
        assert_eq!(
            *server.asked.lock().expect("test lock"),
            vec!["<1@example.com>@1700000000".to_owned()],
            "the name and the date it was written down under are what it went out under"
        );
    }

    #[test]
    fn the_copy_filed_in_sent_is_dated_the_same_as_the_message_that_went_out() {
        // The seam the whole arrangement rests on. Both ports are handed a date, and the only date
        // either may be handed is the one the row carries: the message that goes out and the copy
        // filed seconds later are then dated alike, and both say when the person pressed send.
        //
        // Nothing asserted this. The fakes took the argument and dropped it, so passing `0` here,
        // or the clock, would have left every test in this file green.
        // One row on its way out and the same row waiting to be filed: a round does both, which is
        // what lets the two dates be compared at all.
        let queue = Queue {
            waiting: Mutex::new(vec![held(1, 0)]),
            to_file: Mutex::new(vec![held(1, 0)]),
            ..Queue::default()
        };
        let server = Server::saying(Vec::new());
        let keeping = Keeps::default();

        DeliverOutbox::new(&queue, &server, &keeping, &CameBack::default())
            .execute("acc-1")
            .expect("the round runs");

        let went_out = server.asked.lock().expect("test lock").clone();
        let filed = keeping.kept.lock().expect("test lock").clone();

        let date_of = |entry: &str| entry.rsplit('@').next().expect("a date").to_owned();

        assert_eq!(went_out.len(), 1, "one message went out");
        assert_eq!(filed.len(), 1, "one copy was filed");
        assert_eq!(
            date_of(&went_out[0]),
            date_of(&filed[0]),
            "the copy in the sent folder is dated differently from the message that was sent"
        );
        assert_eq!(
            date_of(&went_out[0]),
            "1700000000",
            "the date is not the one the row carries"
        );
    }

    /// Whether a round gave exactly these messages back to the drafts, and left nothing about any
    /// of them in the outbox: nothing that says when to try one again, and nothing on a row the
    /// drafts have let go.
    fn given_back_and_nothing_else(queue: &Queue, came_back: &CameBack, rows: &[i64]) {
        assert_eq!(
            *came_back.from.lock().expect("the drafts"),
            rows,
            "what went back to the drafts"
        );
        assert!(
            queue.stopped.lock().expect("test lock").is_empty(),
            "a message back in the drafts was given a time, or written on after its row went"
        );
    }

    #[test]
    fn a_message_whose_outcome_nobody_knows_is_never_handed_over_again() {
        // The rule the whole use case is shaped around: it may already be on its way to everybody
        // it names, and sending it again would be the second copy they read. Back in the drafts,
        // only the person sends it again.
        let queue = queue_of(vec![held(1, 0)]);
        let server = Server::saying(vec![Err(SubmissionError::NobodyKnows)]);
        let came_back = CameBack::default();

        let done = DeliverOutbox::new(&queue, &server, &Keeps::default(), &came_back)
            .execute("acc-1")
            .expect("the round runs");

        assert_eq!(done.given_back, 1);
        given_back_and_nothing_else(&queue, &came_back, &[1]);
    }

    #[test]
    fn a_message_a_server_read_and_refused_is_not_offered_again_either() {
        let queue = queue_of(vec![held(1, 0)]);
        let server = Server::saying(vec![Err(SubmissionError::Refused)]);
        let came_back = CameBack::default();

        DeliverOutbox::new(&queue, &server, &Keeps::default(), &came_back)
            .execute("acc-1")
            .expect("the round runs");

        given_back_and_nothing_else(&queue, &came_back, &[1]);
    }

    #[test]
    fn a_server_that_could_not_be_reached_is_the_one_failure_worth_repeating() {
        let queue = queue_of(vec![held(1, 0)]);
        let server = Server::saying(vec![Err(SubmissionError::Unreachable)]);

        let done = DeliverOutbox::new(&queue, &server, &Keeps::default(), &CameBack::default())
            .execute("acc-1")
            .expect("the round runs");

        assert_eq!(done.waiting, 1);
        assert_eq!(
            queue.stopped.lock().expect("test lock")[0],
            (1, WhyHeld::TheServerRefused, Some(RETRY_SECONDS[0]))
        );
    }

    #[test]
    fn a_server_that_has_been_unreachable_long_enough_stops_being_asked() {
        let queue = queue_of(vec![held(1, RETRY_SECONDS.len() as u32)]);
        let server = Server::saying(vec![Err(SubmissionError::Unreachable)]);
        let came_back = CameBack::default();

        let done = DeliverOutbox::new(&queue, &server, &Keeps::default(), &came_back)
            .execute("acc-1")
            .expect("the round runs");

        assert_eq!(
            done.given_back, 1,
            "a queue that retries for ever tells nobody"
        );
        given_back_and_nothing_else(&queue, &came_back, &[1]);
    }

    #[test]
    fn an_unreachable_server_is_not_asked_about_the_rest_of_the_queue() {
        let queue = queue_of(vec![held(1, 0), held(2, 0), held(3, 0)]);
        let server = Server::saying(vec![Err(SubmissionError::Unreachable)]);

        DeliverOutbox::new(&queue, &server, &Keeps::default(), &CameBack::default())
            .execute("acc-1")
            .expect("the round runs");

        assert_eq!(
            server.asked.lock().expect("test lock").len(),
            1,
            "three waits for a server that is not there is three times the wait"
        );
    }

    #[test]
    fn one_message_a_server_refuses_does_not_stop_the_ones_behind_it() {
        // Refusing this message says nothing about the next one: a bad address is about an
        // address.
        let queue = queue_of(vec![held(1, 0), held(2, 0)]);
        let server = Server::saying(vec![Err(SubmissionError::Refused), Ok(())]);

        let done = DeliverOutbox::new(&queue, &server, &Keeps::default(), &CameBack::default())
            .execute("acc-1")
            .expect("the round runs");

        assert_eq!(done.sent, 1);
        assert_eq!(done.given_back, 1);
    }

    #[test]
    fn sign_in_failing_stops_the_message_rather_than_timing_it() {
        let queue = queue_of(vec![held(1, 0)]);
        let server = Server::saying(vec![Err(SubmissionError::SignInFailed)]);
        let came_back = CameBack::default();

        DeliverOutbox::new(&queue, &server, &Keeps::default(), &came_back)
            .execute("acc-1")
            .expect("the round runs");

        given_back_and_nothing_else(&queue, &came_back, &[1]);
    }

    /// A message the server will not take comes back to the person who wrote it.
    ///
    /// This is the one that matters. The composer discards its draft the moment the message is
    /// queued, so without this a password the server refuses leaves somebody's writing in a row of
    /// the outbox that nothing in this product shows.
    #[test]
    fn a_message_that_will_not_be_tried_again_goes_back_to_the_drafts() {
        // Written out here rather than taken from `held`, which carries an empty subject and an
        // empty body: a test that what came back is what went in has to send something that could
        // come back wrong.
        let mut wrote = held(1, 0);
        wrote.message.subject = Subject::parse("About the thing").expect("a subject");
        wrote.message.body = "The body".to_owned();
        wrote.message.cc = vec![EmailAddress::parse("sigma@example.org").expect("valid")];

        let came_back = CameBack::default();
        let queue = queue_of(vec![wrote]);
        let server = Server::saying(vec![Err(SubmissionError::SignInFailed)]);

        let done = DeliverOutbox::new(&queue, &server, &Keeps::default(), &came_back)
            .execute("acc-1")
            .expect("the round runs");

        assert_eq!(done.given_back, 1);
        given_back_and_nothing_else(&queue, &came_back, &[1]);

        let drafts = came_back.drafts.lock().expect("the drafts");
        assert_eq!(drafts.len(), 1, "the message did not come back");
        assert_eq!(drafts[0].subject, "About the thing");
        assert_eq!(drafts[0].body, "The body");
        assert_eq!(
            drafts[0].recipients, "kappa@example.net",
            "it came back without the address it was going to"
        );
        assert_eq!(
            drafts[0].copies, "sigma@example.org",
            "the copies did not come back"
        );
    }

    /// A message that comes back is given back by its row, which is what its files are held under.
    ///
    /// Given back by its text alone, it comes back without them, and goes out again without the
    /// invoice it was written to carry while nothing says so.
    #[test]
    fn a_message_that_will_not_be_tried_again_comes_back_by_its_row_with_what_it_carried() {
        let mut wrote = held(3, 0);
        wrote.message.files = vec![lyonoyl_domain::outgoing::OutgoingFile {
            name: "invoice.pdf".to_owned(),
            content_type: "application/pdf".to_owned(),
            bytes: vec![1, 2, 3],
        }];
        let came_back = CameBack::default();
        let queue = queue_of(vec![held(1, 0), wrote]);
        let server = Server::saying(vec![Ok(()), Err(SubmissionError::Refused)]);

        DeliverOutbox::new(&queue, &server, &Keeps::default(), &came_back)
            .execute("acc-1")
            .expect("the round runs");

        assert_eq!(*came_back.from.lock().expect("the drafts"), vec![3]);
    }

    #[test]
    fn a_message_a_round_took_and_never_finished_comes_back_by_its_row() {
        let came_back = CameBack::default();
        let queue = Queue {
            interrupted: Mutex::new(vec![held(7, 0)]),
            ..Queue::default()
        };

        DeliverOutbox::new(
            &queue,
            &Server::saying(Vec::new()),
            &Keeps::default(),
            &came_back,
        )
        .execute("acc-1")
        .expect("the round runs");

        assert_eq!(*came_back.from.lock().expect("the drafts"), vec![7]);
    }

    /// A message that will be tried again is not handed back at the same time.
    ///
    /// Otherwise every temporary failure leaves a draft behind, and four retries leave four —
    /// a person watching one message go out would find five copies of it.
    #[test]
    fn a_message_that_is_waiting_to_be_tried_again_stays_where_it_is() {
        let came_back = CameBack::default();
        let queue = queue_of(vec![held(1, 0)]);
        let server = Server::saying(vec![Err(SubmissionError::Unreachable)]);

        let done = DeliverOutbox::new(&queue, &server, &Keeps::default(), &came_back)
            .execute("acc-1")
            .expect("the round runs");

        assert_eq!(done.given_back, 0);
        assert!(
            came_back.drafts.lock().expect("the drafts").is_empty(),
            "a message still waiting was handed back early"
        );
    }

    /// A server that, as each message reaches it, looks at whether the queue had taken it first.
    struct Witness<'a> {
        queue: &'a Queue,
        saw_it_taken: Mutex<Vec<bool>>,
    }

    impl MailSubmission for Witness<'_> {
        fn submit(
            &self,
            message_id: &str,
            _message: &Outgoing,
            _written_at: i64,
        ) -> Result<(), SubmissionError> {
            let id: i64 = message_id
                .trim_start_matches('<')
                .split('@')
                .next()
                .and_then(|number| number.parse().ok())
                .expect("a name `held` gave");
            let taken = self.queue.claimed.lock().expect("test lock").contains(&id);
            self.saw_it_taken.lock().expect("test lock").push(taken);
            Ok(())
        }
    }

    #[test]
    fn a_message_is_taken_before_it_is_handed_over() {
        // Taken on the disk before a byte of it leaves. Handed over first, a machine that stops
        // while the server is taking it leaves a row that still says "waiting", and the next round
        // sends the message a second time.
        let queue = queue_of(vec![held(1, 0), held(2, 0)]);
        let server = Witness {
            queue: &queue,
            saw_it_taken: Mutex::new(Vec::new()),
        };

        let done = DeliverOutbox::new(&queue, &server, &Keeps::default(), &CameBack::default())
            .execute("acc-1")
            .expect("the round runs");

        assert_eq!(done.sent, 2);
        assert_eq!(
            *server.saw_it_taken.lock().expect("test lock"),
            vec![true, true],
            "a message reached the server before it was taken"
        );
    }

    #[test]
    fn a_message_another_round_has_taken_is_not_handed_over() {
        let queue = Queue {
            gone: vec![1],
            ..queue_of(vec![held(1, 0), held(2, 0)])
        };
        let server = Server::saying(Vec::new());

        let done = DeliverOutbox::new(&queue, &server, &Keeps::default(), &CameBack::default())
            .execute("acc-1")
            .expect("the round runs");

        assert_eq!(
            *server.asked.lock().expect("test lock"),
            vec!["<2@example.com>@1700000000".to_owned()],
            "a message somebody else had taken was handed over"
        );
        assert_eq!(done.sent, 1);
        assert_eq!(*queue.accepted.lock().expect("test lock"), vec![2]);
    }

    #[test]
    fn a_message_that_cannot_be_taken_is_not_handed_over() {
        // A disk that will not say the message is on its way is a disk that could not say so
        // after a stop either, and nothing is handed over without it.
        let queue = Queue {
            claims_fail: true,
            ..queue_of(vec![held(1, 0)])
        };
        let server = Server::saying(Vec::new());

        let outcome = DeliverOutbox::new(&queue, &server, &Keeps::default(), &CameBack::default())
            .execute("acc-1");

        assert!(
            matches!(outcome, Err(UseCaseError::StorageUnavailable)),
            "{outcome:?}"
        );
        assert!(
            server.asked.lock().expect("test lock").is_empty(),
            "a message was handed over without being taken"
        );
    }

    #[test]
    fn a_message_a_round_took_and_never_finished_is_never_handed_over_again() {
        // The machine stopped while a server was taking it, or the outcome could not be written
        // down. The server may have it, so it is what an answer nobody heard is: given back to
        // the person, and not handed to any server by anything but them.
        let mut taken = held(7, 0);
        taken.message.subject = Subject::parse("About the thing").expect("a subject");
        let queue = Queue {
            interrupted: Mutex::new(vec![taken]),
            ..Queue::default()
        };
        let server = Server::saying(Vec::new());
        let came_back = CameBack::default();

        let done = DeliverOutbox::new(&queue, &server, &Keeps::default(), &came_back)
            .execute("acc-1")
            .expect("the round runs");

        assert!(
            server.asked.lock().expect("test lock").is_empty(),
            "a message that may already have gone was handed over again"
        );
        given_back_and_nothing_else(&queue, &came_back, &[7]);
        assert_eq!(
            came_back.drafts.lock().expect("the drafts")[0].subject,
            "About the thing",
            "the message did not come back to the person who wrote it"
        );
        assert_eq!(
            done,
            Delivered {
                given_back: 1,
                ..Delivered::default()
            }
        );
    }

    #[test]
    fn a_message_another_round_gave_back_already_is_not_counted_as_given_back_again() {
        // Read by this round before another gave it back: the drafts hold it once, and the count
        // that reaches the journal says so once.
        let queue = Queue {
            interrupted: Mutex::new(vec![held(7, 0)]),
            ..queue_of(vec![held(8, 0)])
        };
        let server = Server::saying(vec![Err(SubmissionError::Refused)]);
        let came_back = CameBack {
            gone: vec![7, 8],
            ..CameBack::default()
        };

        let done = DeliverOutbox::new(&queue, &server, &Keeps::default(), &came_back)
            .execute("acc-1")
            .expect("the round runs");

        assert_eq!(done, Delivered::default());
        assert!(came_back.drafts.lock().expect("the drafts").is_empty());
        given_back_and_nothing_else(&queue, &came_back, &[7, 8]);
    }

    #[test]
    fn an_empty_outbox_asks_no_server_anything() {
        let queue = queue_of(Vec::new());
        let server = Server::saying(Vec::new());

        let done = DeliverOutbox::new(&queue, &server, &Keeps::default(), &CameBack::default())
            .execute("acc-1")
            .expect("the round runs");

        assert_eq!(done, Delivered::default());
        assert!(server.asked.lock().expect("test lock").is_empty());
    }
}
