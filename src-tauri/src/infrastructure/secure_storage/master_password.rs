//! Secrets on a system that has no store of its own, kept behind a master password.
//!
//! Closed until someone opens it, and it stays closed rather than making a file quietly: the whole
//! point is that nobody ends up believing a password is in system storage when it is in a file.
//! Every read and every write while it is closed answers the way an unreachable system store
//! answers, and the interface already knows what to do with that.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use secrecy::SecretString;

use crate::infrastructure::secure_storage::vault::{Vault, VaultError, Work};
use crate::ports::credential_store::{CredentialError, CredentialRef, CredentialStore};

/// What the interface has to know about where this device keeps its secrets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretStoreState {
    /// The system holds them. Nothing to ask anyone.
    System,
    /// There is no system store and no file yet: a master password has to be chosen before an
    /// account can be added.
    NeedsChoosing,
    /// There is a file, closed until the master password is given.
    NeedsUnlocking,
    /// Open for this run.
    Unlocked,
}

pub struct MasterPasswordStore {
    path: PathBuf,
    /// What making the file costs. Read back from the file when opening one, so this is only ever
    /// used for a file that does not exist yet.
    work: Work,
    open: Mutex<Option<Vault>>,
    /// Held while the file is made or opened, which is slow on purpose. A second press arriving
    /// meanwhile would otherwise put a copy of its own in place of the first one's after the first
    /// was written to — and what is written to it first, on a device whose databases wait for the
    /// file, is the key they are written under.
    opening: Mutex<()>,
}

impl MasterPasswordStore {
    pub fn at(path: PathBuf) -> Self {
        Self {
            path,
            work: Work::SHIPPED,
            open: Mutex::new(None),
            opening: Mutex::new(()),
        }
    }

    /// The same store, at a cost that does not put a quarter of a gigabyte and half a second into
    /// every test that makes a file.
    #[cfg(test)]
    pub(crate) fn quick(path: PathBuf) -> Self {
        Self {
            path,
            work: Work {
                memory_kib: 8 * 1024,
                iterations: 1,
                lanes: 1,
            },
            open: Mutex::new(None),
            opening: Mutex::new(()),
        }
    }

    pub fn state(&self) -> SecretStoreState {
        if self
            .open
            .lock()
            .expect("the lock is never poisoned")
            .is_some()
        {
            return SecretStoreState::Unlocked;
        }

        match Vault::exists(&self.path) {
            true => SecretStoreState::NeedsUnlocking,
            false => SecretStoreState::NeedsChoosing,
        }
    }

    /// Makes the file for the first time, with the master password the person chose.
    pub fn choose(&self, master: &SecretString) -> Result<(), VaultError> {
        let _one_at_a_time = self.opening.lock().unwrap_or_else(PoisonError::into_inner);
        let made = Vault::create(&self.path, master, self.work)?;
        *self.open.lock().expect("the lock is never poisoned") = Some(made);

        Ok(())
    }

    /// Opens the file for this run. A password that does not open it leaves it closed. A file that
    /// is already open stays the one that is, and the password is not checked against it then: read
    /// again from the disk, it could be a copy from before the last write.
    pub fn unlock(&self, master: &SecretString) -> Result<(), VaultError> {
        let _one_at_a_time = self.opening.lock().unwrap_or_else(PoisonError::into_inner);
        if self
            .open
            .lock()
            .expect("the lock is never poisoned")
            .is_some()
        {
            return Ok(());
        }

        let opened = Vault::unlock(&self.path, master)?;
        *self.open.lock().expect("the lock is never poisoned") = Some(opened);

        Ok(())
    }

    /// Closes it again, without touching what is on disk.
    pub fn lock(&self) {
        *self.open.lock().expect("the lock is never poisoned") = None;
    }

    fn entry_name(reference: &CredentialRef) -> String {
        super::entry_name(reference)
    }
}

impl CredentialStore for MasterPasswordStore {
    fn save(
        &self,
        reference: &CredentialRef,
        secret: &SecretString,
    ) -> Result<(), CredentialError> {
        let mut open = self.open.lock().expect("the lock is never poisoned");

        match open.as_mut() {
            Some(vault) => vault
                .put(&Self::entry_name(reference), secret)
                .map_err(from_vault),
            // Not "make the file and put it in": a file made without anyone asking for one is the
            // silent downgrade the decision is about.
            None => Err(CredentialError::Unavailable),
        }
    }

    fn read(&self, reference: &CredentialRef) -> Result<SecretString, CredentialError> {
        let open = self.open.lock().expect("the lock is never poisoned");

        match open.as_ref() {
            Some(vault) => vault
                .get(&Self::entry_name(reference))
                .ok_or(CredentialError::NotFound),
            None => Err(CredentialError::Unavailable),
        }
    }

    fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError> {
        let mut open = self.open.lock().expect("the lock is never poisoned");

        match open.as_mut() {
            Some(vault) => vault
                .remove(&Self::entry_name(reference))
                .map_err(from_vault),
            None => Err(CredentialError::Unavailable),
        }
    }
}

/// A file that cannot be written is the same situation as a system store that cannot be reached:
/// the secret is not somewhere it can be had right now.
fn from_vault(error: VaultError) -> CredentialError {
    match error {
        VaultError::WrongPassword
        | VaultError::Damaged
        | VaultError::Unavailable
        | VaultError::AlreadyThere => CredentialError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;
    use crate::ports::credential_store::CredentialPurpose;

    struct Scratch {
        directory: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "lyonoyl-master-{name}-{}",
                std::process::id() as u64 + name.bytes().map(u64::from).sum::<u64>()
            ));
            std::fs::create_dir_all(&directory).expect("a directory to write in");

            Self { directory }
        }

        fn store(&self) -> MasterPasswordStore {
            MasterPasswordStore::quick(self.directory.join("secrets.vault"))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    fn password() -> SecretString {
        SecretString::from("open sesame".to_owned())
    }

    fn secret() -> SecretString {
        SecretString::from("correct horse".to_owned())
    }

    fn reference() -> CredentialRef {
        CredentialRef::new("acc-1", CredentialPurpose::ImapPassword)
    }

    #[test]
    fn a_closed_store_writes_nothing_and_makes_nothing() {
        // The decision this is about: nobody may end up believing a password went to system storage
        // when it went into a file. A store nobody has opened does not quietly make one.
        let scratch = Scratch::new("closed");
        let store = scratch.store();

        assert_eq!(store.state(), SecretStoreState::NeedsChoosing);
        assert_eq!(
            store.save(&reference(), &secret()).expect_err("refused"),
            CredentialError::Unavailable
        );
        assert_eq!(
            store.read(&reference()).expect_err("refused"),
            CredentialError::Unavailable
        );
        assert_eq!(
            store.delete(&reference()).expect_err("refused"),
            CredentialError::Unavailable
        );
        assert!(
            !scratch.directory.join("secrets.vault").exists(),
            "a file was made without anyone asking for one"
        );
    }

    #[test]
    fn what_was_stored_after_the_password_was_chosen_is_read_back() {
        let scratch = Scratch::new("chosen");
        let store = scratch.store();

        store.choose(&password()).expect("the file is made");
        assert_eq!(store.state(), SecretStoreState::Unlocked);

        store.save(&reference(), &secret()).expect("stored");

        assert_eq!(
            store.read(&reference()).expect("read back").expose_secret(),
            "correct horse"
        );
    }

    #[test]
    fn a_secret_stored_in_one_run_is_there_in_the_next() {
        let scratch = Scratch::new("across-runs");

        let first = scratch.store();
        first.choose(&password()).expect("the file is made");
        first.save(&reference(), &secret()).expect("stored");
        drop(first);

        let next = scratch.store();
        assert_eq!(next.state(), SecretStoreState::NeedsUnlocking);
        assert_eq!(
            next.read(&reference()).expect_err("closed until opened"),
            CredentialError::Unavailable
        );

        next.unlock(&password()).expect("it opens");

        assert_eq!(
            next.read(&reference()).expect("read back").expose_secret(),
            "correct horse"
        );
    }

    #[test]
    fn a_password_that_does_not_open_it_leaves_it_closed() {
        let scratch = Scratch::new("wrong");
        let first = scratch.store();
        first.choose(&password()).expect("the file is made");
        first.save(&reference(), &secret()).expect("stored");
        drop(first);

        let next = scratch.store();
        assert_eq!(
            next.unlock(&SecretString::from("not it".to_owned()))
                .expect_err("it must not open"),
            VaultError::WrongPassword
        );

        assert_eq!(next.state(), SecretStoreState::NeedsUnlocking);
        assert_eq!(
            next.read(&reference()).expect_err("still closed"),
            CredentialError::Unavailable
        );
    }

    #[test]
    fn a_second_choosing_does_not_throw_away_the_first() {
        // Every secret on the device is in that file. Answering the "choose a password" prompt
        // again must not clear it.
        let scratch = Scratch::new("second-choosing");
        let first = scratch.store();
        first.choose(&password()).expect("the file is made");
        first.save(&reference(), &secret()).expect("stored");
        drop(first);

        let next = scratch.store();
        assert_eq!(
            next.choose(&SecretString::from("another".to_owned()))
                .expect_err("refused"),
            VaultError::AlreadyThere
        );

        next.unlock(&password())
            .expect("the first password still opens it");
        assert!(next.read(&reference()).is_ok(), "the secret survived");
    }

    #[test]
    fn locking_it_again_costs_the_run_its_secrets_and_nothing_on_disk() {
        let scratch = Scratch::new("relock");
        let store = scratch.store();
        store.choose(&password()).expect("the file is made");
        store.save(&reference(), &secret()).expect("stored");

        store.lock();

        assert_eq!(store.state(), SecretStoreState::NeedsUnlocking);
        store.unlock(&password()).expect("it opens again");
        assert!(store.read(&reference()).is_ok());
    }

    #[test]
    fn a_name_that_was_never_stored_is_missing_rather_than_unreachable() {
        // The interface tells the two apart: one sends the person to type their password again,
        // the other says the store cannot be reached at all.
        let scratch = Scratch::new("missing");
        let store = scratch.store();
        store.choose(&password()).expect("the file is made");

        assert_eq!(
            store.read(&reference()).expect_err("nothing stored"),
            CredentialError::NotFound
        );
    }

    #[test]
    fn opening_it_again_keeps_what_was_written_since() {
        // A second press, answered after the first. What is on the disk is put back to what it was
        // before the write, which is what that press would hold had it read the file a moment
        // earlier: opened again from that, the secret written in between is gone.
        let scratch = Scratch::new("open-twice");
        let store = scratch.store();
        store.choose(&password()).expect("chosen");
        let file = scratch.directory.join("secrets.vault");
        let before = std::fs::read(&file).expect("the file");
        store.save(&reference(), &secret()).expect("kept");
        std::fs::write(&file, before).expect("the earlier copy");

        store.unlock(&password()).expect("already open");

        assert_eq!(
            store
                .read(&reference())
                .expect("what was written since")
                .expose_secret(),
            "correct horse"
        );
    }

    #[test]
    fn two_choices_at_once_make_one_file() {
        // Choosing is slow on purpose. Both presses would otherwise find no file, make one each, and
        // leave the second in place of the first after the first was written to.
        let scratch = Scratch::new("choose-twice");
        let store = std::sync::Arc::new(scratch.store());
        let together = std::sync::Arc::new(std::sync::Barrier::new(2));
        let presses: Vec<_> = (0..2)
            .map(|_| {
                let store = std::sync::Arc::clone(&store);
                let together = std::sync::Arc::clone(&together);

                std::thread::spawn(move || {
                    together.wait();
                    store.choose(&password())
                })
            })
            .collect();
        let answers: Vec<_> = presses
            .into_iter()
            .map(|press| press.join().expect("a press"))
            .collect();

        assert_eq!(
            answers.iter().filter(|answer| answer.is_ok()).count(),
            1,
            "{answers:?}"
        );
        assert!(answers.contains(&Err(VaultError::AlreadyThere)));
    }

    #[test]
    fn opening_it_waits_for_an_opening_already_under_way() {
        // Both would otherwise find it closed and read it, and the second would put its copy in
        // place after the first one's caller wrote to it — the key the databases are written under.
        // No file here, so an opening that does not wait is over at once.
        let scratch = Scratch::new("open-waits");
        let store = std::sync::Arc::new(scratch.store());
        let under_way = store.opening.lock().expect("the first opening");

        let second = {
            let store = std::sync::Arc::clone(&store);
            std::thread::spawn(move || store.unlock(&password()))
        };
        std::thread::sleep(std::time::Duration::from_millis(100));

        assert!(
            !second.is_finished(),
            "a second opening ran beside the first"
        );
        drop(under_way);
        let _ = second.join().expect("the second opening");
    }
}
