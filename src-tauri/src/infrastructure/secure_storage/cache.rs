//! Reading a secret once per run instead of once per pass.
//!
//! Every round asks each account for its password, and a round comes every couple of minutes: a
//! day at the desk is thousands of reads of a value that did not change. On macOS each of those is
//! also a chance for the system to ask the person about it.
//!
//! The trade is stated plainly: the secret is in this process's memory for as long as the window is
//! open, rather than for the length of one pass. It is already in memory whenever mail is being
//! read, `SecretString` clears it when the process ends, and holding a credential for the session
//! is what a connection that stays open — the next thing this needs — requires anyway.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use secrecy::SecretString;

use crate::ports::credential_store::{CredentialError, CredentialRef, CredentialStore};

pub struct CachedCredentials<C: CredentialStore> {
    inner: C,
    /// Cleared when the process ends, and never written anywhere.
    shared: Arc<Shared>,
}

/// What every handle sharing one set holds, and what wakes a reader waiting on another's ask.
#[derive(Default)]
struct Shared {
    held: Mutex<Held>,
    answered: Condvar,
}

/// What every handle sharing one set holds.
#[derive(Default)]
struct Held {
    /// One entry per name.
    secrets: HashMap<String, SecretString>,
    /// How many times a secret has been written or taken out through any of the handles.
    ///
    /// A read asks the store with nothing locked, and a password written or taken out while the
    /// store is being asked leaves its answer behind: it is what the store held before. Kept, that
    /// answer would be sent in front of the new password until the application starts again — to
    /// whichever server the account was just pointed at — or would put back one whose account is
    /// gone. A read that sees this move while it asked keeps nothing. One count for every name: a
    /// change to another account's password costs a read no more than asking the store once more.
    changes: u64,
    /// The names the store is being asked for right now, each with the number of its ask.
    ///
    /// A second reader of a name being asked waits for that answer instead of asking again. On
    /// macOS an ask of an entry this build did not write is a dialog. The first round after a new
    /// build starts every account's pass at once and each asks for the key the mail is sealed
    /// under: two of those asks were refused while the others were answered, and those two
    /// accounts stopped.
    asking: HashMap<String, u64>,
    /// How the last ask of a name ended where it brought no secret, with the number of that ask:
    /// what a reader that waited on it is answered. A later read asks again.
    refused: HashMap<String, (u64, CredentialError)>,
    /// How many asks have begun, every name together, which numbers each one.
    asks: u64,
}

impl<C: CredentialStore> CachedCredentials<C> {
    pub fn new(inner: C) -> Self {
        Self {
            inner,
            shared: Arc::new(Shared::default()),
        }
    }

    /// Another handle on the same held secrets, so the background rounds and the commands share
    /// one set rather than each keeping their own.
    pub fn sharing(&self, inner: C) -> Self {
        Self {
            inner,
            shared: Arc::clone(&self.shared),
        }
    }

    fn held(&self) -> MutexGuard<'_, Held> {
        self.shared.held.lock().expect("the lock is never poisoned")
    }

    fn key(reference: &CredentialRef) -> String {
        format!("{}:{}", reference.account_id, reference.purpose.as_str())
    }
}

impl<C: CredentialStore> CredentialStore for CachedCredentials<C> {
    fn read(&self, reference: &CredentialRef) -> Result<SecretString, CredentialError> {
        let key = Self::key(reference);

        let mut held = self.held();
        let (ask, changes) = loop {
            if let Some(secret) = held.secrets.get(&key) {
                return Ok(secret.clone());
            }
            let Some(&theirs) = held.asking.get(&key) else {
                held.asks += 1;
                let ask = held.asks;
                held.asking.insert(key.clone(), ask);
                break (ask, held.changes);
            };
            // Somebody is asking the store for this one now, and their answer is this reader's
            // too: a secret they bring is held by the time this wakes, and a refusal is taken as
            // it is. Woken by an ask of another name, or by one whose answer was kept nowhere, it
            // looks again.
            held = self
                .shared
                .answered
                .wait(held)
                .expect("the lock is never poisoned");
            if let Some(&(ended, why)) = held.refused.get(&key) {
                if ended == theirs {
                    return Err(why);
                }
            }
        };
        drop(held);

        // Asked with nothing locked. The system may be waiting on a person for this one, and every
        // other password held is read meanwhile rather than waiting behind it.
        let asking = Asking {
            shared: &self.shared,
            key: &key,
            ended: false,
        };
        let read = self.inner.read(reference);
        asking.ends(ask, changes, read)
    }

    fn save(
        &self,
        reference: &CredentialRef,
        secret: &SecretString,
    ) -> Result<(), CredentialError> {
        self.inner.save(reference, secret)?;
        // Held straight away: the password was just typed, and asking the system for it again
        // would put a dialog in front of someone who is plainly there and plainly allowed.
        let mut held = self.held();
        held.secrets.insert(Self::key(reference), secret.clone());
        held.changes += 1;
        Ok(())
    }

    fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError> {
        // The store first, and what is held after it. Let go of first, the password is gone from
        // here and still in the store, and a read arriving in between asks the store and puts it
        // back. Let go of whatever the store answers: one that refused still has the password, and
        // the next read asks it again.
        let taken_out = self.inner.delete(reference);

        let mut held = self.held();
        held.secrets.remove(&Self::key(reference));
        held.changes += 1;

        taken_out
    }
}

/// One ask of the store, which ends by telling everybody waiting on it what it brought.
///
/// Ended on the way out as well, should the store's read unwind: a name left marked as being
/// asked would keep every later reader of it waiting for an answer that never comes.
struct Asking<'a> {
    shared: &'a Shared,
    key: &'a str,
    ended: bool,
}

impl Asking<'_> {
    fn ends(
        mut self,
        ask: u64,
        changes: u64,
        read: Result<SecretString, CredentialError>,
    ) -> Result<SecretString, CredentialError> {
        let mut held = self.shared.held.lock().expect("the lock is never poisoned");
        held.asking.remove(self.key);
        let answer = match read {
            Err(why) => {
                held.refused.insert(self.key.to_owned(), (ask, why));
                Err(why)
            }
            Ok(read) => {
                held.refused.remove(self.key);
                match held.changes == changes {
                    true => {
                        held.secrets.insert(self.key.to_owned(), read.clone());
                        Ok(read)
                    }
                    // A password was written or taken out while the store was asked: this one, or
                    // another, since the count is shared. Whatever is held for this one now arrived
                    // after the read began, and is the answer. Where nothing is, either this
                    // password was taken out or the change was to another one: what the store said
                    // goes to the one who asked, and is kept nowhere.
                    false => Ok(held.secrets.get(self.key).cloned().unwrap_or(read)),
                }
            }
        };
        self.ended = true;
        drop(held);
        self.shared.answered.notify_all();
        answer
    }
}

impl Drop for Asking<'_> {
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        let mut held = self
            .shared
            .held
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        held.asking.remove(self.key);
        drop(held);
        self.shared.answered.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use secrecy::ExposeSecret;

    use super::*;
    use crate::ports::credential_store::CredentialPurpose;

    #[derive(Default)]
    struct CountingStore {
        reads: AtomicUsize,
        saves: AtomicUsize,
        deletes: AtomicUsize,
        missing: bool,
    }

    impl CredentialStore for &CountingStore {
        fn read(&self, _reference: &CredentialRef) -> Result<SecretString, CredentialError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            if self.missing {
                return Err(CredentialError::NotFound);
            }
            Ok(SecretString::from("correct horse".to_owned()))
        }

        fn save(
            &self,
            _reference: &CredentialRef,
            _secret: &SecretString,
        ) -> Result<(), CredentialError> {
            self.saves.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn delete(&self, _reference: &CredentialRef) -> Result<(), CredentialError> {
            self.deletes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn reference(account: &str) -> CredentialRef {
        CredentialRef::new(account, CredentialPurpose::ImapPassword)
    }

    /// Which step a [`HoldingBack`] store keeps waiting until the test lets it go.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Step {
        /// The first read, which has taken what is kept and not yet answered with it.
        Read,
        /// A removal, which has not yet taken the password out.
        Removal,
    }

    /// One account's password, kept as a system store keeps it, with one step held back so that
    /// something can happen to the same password while the store is busy with it.
    struct HoldingBack {
        step: Step,
        kept: Mutex<Option<SecretString>>,
        reads: AtomicUsize,
        reached: Mutex<mpsc::Sender<()>>,
        released: Mutex<mpsc::Receiver<()>>,
    }

    /// The store, what tells the test the step is reached, and what lets it go on.
    fn holding_back(step: Step, kept: &str) -> (HoldingBack, mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (reached, at_the_step) = mpsc::channel();
        let (go_on, released) = mpsc::channel();
        let store = HoldingBack {
            step,
            kept: Mutex::new(Some(SecretString::from(kept.to_owned()))),
            reads: AtomicUsize::new(0),
            reached: Mutex::new(reached),
            released: Mutex::new(released),
        };
        (store, at_the_step, go_on)
    }

    /// How long either side waits for the other. Bounded, so a cache that kept its lock while the
    /// store was busy fails a test rather than hanging the suite.
    const PATIENCE: Duration = Duration::from_secs(5);

    impl HoldingBack {
        fn wait_to_be_let_go(&self) {
            let _ = self.reached.lock().expect("not poisoned").send(());
            let _ = self
                .released
                .lock()
                .expect("not poisoned")
                .recv_timeout(PATIENCE);
        }
    }

    impl CredentialStore for &HoldingBack {
        fn read(&self, _reference: &CredentialRef) -> Result<SecretString, CredentialError> {
            let first = self.reads.fetch_add(1, Ordering::SeqCst) == 0;
            let kept = self.kept.lock().expect("not poisoned").clone();
            if first && self.step == Step::Read {
                self.wait_to_be_let_go();
            }
            kept.ok_or(CredentialError::NotFound)
        }

        fn save(
            &self,
            _reference: &CredentialRef,
            secret: &SecretString,
        ) -> Result<(), CredentialError> {
            *self.kept.lock().expect("not poisoned") = Some(secret.clone());
            Ok(())
        }

        fn delete(&self, _reference: &CredentialRef) -> Result<(), CredentialError> {
            if self.step == Step::Removal {
                self.wait_to_be_let_go();
            }
            *self.kept.lock().expect("not poisoned") = None;
            Ok(())
        }
    }

    #[test]
    fn a_password_saved_while_the_store_was_asked_for_it_is_the_one_held() {
        // The first round of a run asks for an account's password while the system waits on a
        // dialog, and the person saves a new one meanwhile. What the store then answers is the
        // password from before, and held, it is what every round sends from then on — to the
        // server the account was just pointed at.
        let (store, at_the_store, let_go) = holding_back(Step::Read, "old");
        let rounds = CachedCredentials::new(&store);
        let commands = rounds.sharing(&store);

        let answered = std::thread::scope(|scope| {
            let asking = scope.spawn(|| rounds.read(&reference("acc-1")));
            at_the_store
                .recv_timeout(PATIENCE)
                .expect("the store was asked");
            commands
                .save(&reference("acc-1"), &SecretString::from("new".to_owned()))
                .expect("stored");
            let_go.send(()).expect("the store is waiting");
            asking.join().expect("the read ran")
        });

        assert_eq!(
            answered.expect("answered").expose_secret(),
            "new",
            "the read that was waiting on the store answered with the password from before"
        );
        let next = rounds.read(&reference("acc-1")).expect("held");
        assert_eq!(next.expose_secret(), "new");
    }

    #[test]
    fn a_password_taken_out_while_the_store_was_asked_for_it_is_not_held_again() {
        // The same wait, with the account removed meanwhile: what the store answers is a password
        // nothing should hold any more.
        let (store, at_the_store, let_go) = holding_back(Step::Read, "old");
        let rounds = CachedCredentials::new(&store);
        let commands = rounds.sharing(&store);

        std::thread::scope(|scope| {
            let asking = scope.spawn(|| rounds.read(&reference("acc-1")));
            at_the_store
                .recv_timeout(PATIENCE)
                .expect("the store was asked");
            commands.delete(&reference("acc-1")).expect("taken out");
            let_go.send(()).expect("the store is waiting");
            asking.join().expect("the read ran").expect("answered");
        });

        assert!(
            matches!(
                rounds.read(&reference("acc-1")),
                Err(CredentialError::NotFound)
            ),
            "a password taken out is held again"
        );
    }

    #[test]
    fn a_password_saved_for_another_account_while_the_store_was_asked_leaves_this_one_answered() {
        // One count for every name, so the usual way a read sees it move is a change to another
        // account. This account's password is then what the store said, and nothing took it out:
        // refusing the read would fail this account's pass whenever another one is saved. The
        // other account is saved through a store of its own, so the store asked about this one
        // still holds what it held.
        let (store, at_the_store, let_go) = holding_back(Step::Read, "this one");
        let (elsewhere, _, _) = holding_back(Step::Removal, "the other");
        let rounds = CachedCredentials::new(&store);
        let commands = rounds.sharing(&elsewhere);

        let answered = std::thread::scope(|scope| {
            let asking = scope.spawn(|| rounds.read(&reference("acc-2")));
            at_the_store
                .recv_timeout(PATIENCE)
                .expect("the store was asked");
            commands
                .save(&reference("acc-1"), &SecretString::from("new".to_owned()))
                .expect("stored");
            let_go.send(()).expect("the store is waiting");
            asking.join().expect("the read ran")
        });

        assert_eq!(
            answered
                .expect("a password saved for another account failed this read")
                .expose_secret(),
            "this one"
        );
        let next = rounds.read(&reference("acc-2")).expect("still there");
        assert_eq!(next.expose_secret(), "this one");
    }

    #[test]
    fn a_read_while_the_store_takes_a_password_out_does_not_put_it_back() {
        let (store, at_the_store, let_go) = holding_back(Step::Removal, "old");
        let rounds = CachedCredentials::new(&store);
        let commands = rounds.sharing(&store);
        rounds.read(&reference("acc-1")).expect("held");

        std::thread::scope(|scope| {
            let removing = scope.spawn(|| commands.delete(&reference("acc-1")));
            at_the_store
                .recv_timeout(PATIENCE)
                .expect("the store was asked to take it out");
            rounds
                .read(&reference("acc-1"))
                .expect("still there while it is being taken out");
            let_go.send(()).expect("the store is waiting");
            removing
                .join()
                .expect("the removal ran")
                .expect("taken out");
        });

        assert!(
            matches!(
                rounds.read(&reference("acc-1")),
                Err(CredentialError::NotFound)
            ),
            "a password taken out is held again"
        );
    }

    /// What a [`Gate`] read answers once the test lets it go.
    enum Answer {
        Secret,
        Refused,
        GivesWay,
    }

    /// A store whose every read says which account it was asked for and then waits for the test
    /// to say what it answers: a store the system is holding a dialog open for.
    struct Gate {
        reads: AtomicUsize,
        reached: Mutex<mpsc::Sender<String>>,
        answers: Mutex<mpsc::Receiver<Answer>>,
    }

    /// The store, what says it was reached and for which account, and what answers it.
    fn gate() -> (&'static Gate, mpsc::Receiver<String>, mpsc::Sender<Answer>) {
        let (reached, at_the_store) = mpsc::channel();
        let (answer, answers) = mpsc::channel();
        // For as long as the test binary runs: a reader the cache wrongly left waiting holds it, and
        // the test that finds it so ends rather than waiting with it.
        let store = Box::leak(Box::new(Gate {
            reads: AtomicUsize::new(0),
            reached: Mutex::new(reached),
            answers: Mutex::new(answers),
        }));
        (store, at_the_store, answer)
    }

    /// Long enough for a second reader that is going to ask the store to have asked it.
    const A_MOMENT: Duration = Duration::from_millis(300);

    impl CredentialStore for &'static Gate {
        fn read(&self, reference: &CredentialRef) -> Result<SecretString, CredentialError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            let _ = self
                .reached
                .lock()
                .expect("not poisoned")
                .send(reference.account_id.clone());
            let answered = self
                .answers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .recv_timeout(PATIENCE)
                .expect("the test answered");
            match answered {
                Answer::Secret => Ok(SecretString::from("correct horse".to_owned())),
                Answer::Refused => Err(CredentialError::Unavailable),
                Answer::GivesWay => panic!("the store gave way mid-read"),
            }
        }

        fn save(&self, _: &CredentialRef, _: &SecretString) -> Result<(), CredentialError> {
            Ok(())
        }

        fn delete(&self, _: &CredentialRef) -> Result<(), CredentialError> {
            Ok(())
        }
    }

    /// A read on a thread of its own, whose answer arrives on the channel it hands back. Not
    /// scoped: a reader left waiting must not keep the test waiting with it.
    fn reading(
        cache: &Arc<CachedCredentials<&'static Gate>>,
        account: &'static str,
    ) -> mpsc::Receiver<Result<String, CredentialError>> {
        let (said, answer) = mpsc::channel();
        let cache = Arc::clone(cache);
        std::thread::spawn(move || {
            let read = cache.read(&reference(account));
            let _ = said.send(read.map(|secret| secret.expose_secret().to_owned()));
        });
        answer
    }

    #[test]
    fn two_reads_of_one_secret_at_once_ask_the_store_once() {
        // The first round after a new build starts every account's pass at once, and each asks for
        // the key the mail is sealed under. On macOS each ask is a dialog: two of them were refused,
        // and two accounts stopped.
        let (store, at_the_store, answer) = gate();
        let cache = Arc::new(CachedCredentials::new(store));

        let first = reading(&cache, "this-device");
        assert_eq!(
            at_the_store.recv_timeout(PATIENCE).expect("asked"),
            "this-device"
        );
        let second = reading(&cache, "this-device");
        assert!(
            at_the_store.recv_timeout(A_MOMENT).is_err(),
            "the store was asked a second time while the first ask was open"
        );
        answer.send(Answer::Secret).expect("the store is waiting");

        assert_eq!(
            first.recv_timeout(PATIENCE).expect("answered"),
            Ok("correct horse".to_owned())
        );
        assert_eq!(
            second.recv_timeout(PATIENCE).expect("answered"),
            Ok("correct horse".to_owned())
        );
        assert_eq!(store.reads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_refusal_answers_everybody_who_waited_on_it_and_the_next_read_asks_again() {
        // One question, one answer: a person who refused a dialog is not shown it again by every
        // pass that was waiting on it. The next pass asks afresh.
        let (store, at_the_store, answer) = gate();
        let cache = Arc::new(CachedCredentials::new(store));

        let first = reading(&cache, "this-device");
        at_the_store.recv_timeout(PATIENCE).expect("asked");
        let second = reading(&cache, "this-device");
        assert!(
            at_the_store.recv_timeout(A_MOMENT).is_err(),
            "asked twice at once"
        );
        answer.send(Answer::Refused).expect("the store is waiting");

        assert_eq!(
            first.recv_timeout(PATIENCE).expect("answered"),
            Err(CredentialError::Unavailable)
        );
        assert_eq!(
            second.recv_timeout(PATIENCE).expect("answered"),
            Err(CredentialError::Unavailable)
        );
        assert_eq!(store.reads.load(Ordering::SeqCst), 1);

        let later = reading(&cache, "this-device");
        at_the_store.recv_timeout(PATIENCE).expect("asked again");
        answer.send(Answer::Secret).expect("the store is waiting");
        assert_eq!(
            later.recv_timeout(PATIENCE).expect("answered"),
            Ok("correct horse".to_owned())
        );
        assert_eq!(store.reads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn secrets_of_different_accounts_are_asked_for_side_by_side() {
        // Waiting is for the one name being asked: a dialog open for one account holds back no
        // other account's password.
        let (store, at_the_store, answer) = gate();
        let cache = Arc::new(CachedCredentials::new(store));

        let first = reading(&cache, "acc-1");
        assert_eq!(at_the_store.recv_timeout(PATIENCE).expect("asked"), "acc-1");
        let second = reading(&cache, "acc-2");
        assert_eq!(
            at_the_store
                .recv_timeout(PATIENCE)
                .expect("the other account was asked meanwhile"),
            "acc-2"
        );
        answer.send(Answer::Secret).expect("the store is waiting");
        answer.send(Answer::Secret).expect("the store is waiting");

        assert!(first.recv_timeout(PATIENCE).expect("answered").is_ok());
        assert!(second.recv_timeout(PATIENCE).expect("answered").is_ok());
        assert_eq!(store.reads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_read_that_gives_way_lets_the_one_waiting_on_it_ask_for_itself() {
        // A name left marked as being asked would keep every later reader of it waiting for good.
        let (store, at_the_store, answer) = gate();
        let cache = Arc::new(CachedCredentials::new(store));

        let first = reading(&cache, "this-device");
        at_the_store.recv_timeout(PATIENCE).expect("asked");
        let second = reading(&cache, "this-device");
        assert!(
            at_the_store.recv_timeout(A_MOMENT).is_err(),
            "asked twice at once"
        );
        answer.send(Answer::GivesWay).expect("the store is waiting");

        assert!(
            first.recv_timeout(PATIENCE).is_err(),
            "the read that gave way answered nevertheless"
        );
        at_the_store
            .recv_timeout(PATIENCE)
            .expect("the reader that was waiting asked the store itself");
        answer.send(Answer::Secret).expect("the store is waiting");
        assert_eq!(
            second.recv_timeout(PATIENCE).expect("answered"),
            Ok("correct horse".to_owned())
        );
    }

    #[test]
    fn the_system_store_is_asked_once_however_many_passes_run() {
        let store = CountingStore::default();
        let cache = CachedCredentials::new(&store);

        for _ in 0..50 {
            let read = cache
                .read(&reference("acc-1"))
                .expect("the secret is there");
            assert_eq!(read.expose_secret(), "correct horse");
        }

        assert_eq!(store.reads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn each_account_is_asked_for_on_its_own() {
        let store = CountingStore::default();
        let cache = CachedCredentials::new(&store);

        cache.read(&reference("acc-1")).expect("stored");
        cache.read(&reference("acc-2")).expect("stored");
        cache.read(&reference("acc-1")).expect("stored");

        assert_eq!(store.reads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_secret_that_is_not_there_is_not_remembered_as_an_answer() {
        // Otherwise an account whose password was added a moment later stays broken until restart.
        let store = CountingStore {
            missing: true,
            ..CountingStore::default()
        };
        let cache = CachedCredentials::new(&store);

        assert!(cache.read(&reference("acc-1")).is_err());
        assert!(cache.read(&reference("acc-1")).is_err());

        assert_eq!(store.reads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_password_just_typed_is_not_asked_for_again() {
        let store = CountingStore::default();
        let cache = CachedCredentials::new(&store);

        cache
            .save(&reference("acc-1"), &SecretString::from("typed".to_owned()))
            .expect("stored");
        let read = cache.read(&reference("acc-1")).expect("held");

        assert_eq!(read.expose_secret(), "typed");
        assert_eq!(store.reads.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_removed_account_is_forgotten_here_too() {
        // A secret held after its account is gone is a secret nothing can reach and nothing clears.
        let store = CountingStore::default();
        let cache = CachedCredentials::new(&store);
        cache.read(&reference("acc-1")).expect("stored");

        cache.delete(&reference("acc-1")).expect("removed");
        cache.read(&reference("acc-1")).expect("read again");

        assert_eq!(store.deletes.load(Ordering::SeqCst), 1);
        assert_eq!(
            store.reads.load(Ordering::SeqCst),
            2,
            "the system store is asked again after a removal"
        );
    }

    #[test]
    fn two_handles_share_what_one_of_them_read() {
        let store = CountingStore::default();
        let commands = CachedCredentials::new(&store);
        let rounds = commands.sharing(&store);

        commands.read(&reference("acc-1")).expect("stored");
        rounds.read(&reference("acc-1")).expect("held");

        assert_eq!(store.reads.load(Ordering::SeqCst), 1);
    }
}
