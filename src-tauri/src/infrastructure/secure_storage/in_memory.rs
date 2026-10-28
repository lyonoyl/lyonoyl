//! A store kept in memory, for the tests outside this folder that need one, and for the build made
//! for screenshots, whose secrets are made up and gone when it stops.
//!
//! In this folder because it reads what it holds, and a secret is unwrapped only in the places
//! allowed to send it. A test elsewhere asks it whether a password read through some other handle
//! is the one it keeps now, and never unwraps one itself.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use secrecy::{ExposeSecret, SecretString};

use crate::ports::credential_store::{CredentialError, CredentialRef, CredentialStore};

/// Keeps what it is given, and counts how often it is read. Clones share what is kept.
#[derive(Clone, Default)]
pub struct InMemory {
    kept: Arc<Mutex<HashMap<String, SecretString>>>,
    asked: Arc<AtomicUsize>,
}

impl InMemory {
    /// How many times it has been read.
    pub fn asked(&self) -> usize {
        self.asked.load(Ordering::SeqCst)
    }

    /// Whether `read` is the secret kept under `reference` now.
    pub fn keeps(&self, reference: &CredentialRef, read: &SecretString) -> bool {
        self.kept
            .lock()
            .expect("not poisoned")
            .get(&super::entry_name(reference))
            .is_some_and(|kept| kept.expose_secret() == read.expose_secret())
    }
}

impl CredentialStore for InMemory {
    fn save(
        &self,
        reference: &CredentialRef,
        secret: &SecretString,
    ) -> Result<(), CredentialError> {
        self.kept
            .lock()
            .expect("not poisoned")
            .insert(super::entry_name(reference), secret.clone());
        Ok(())
    }

    fn read(&self, reference: &CredentialRef) -> Result<SecretString, CredentialError> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.kept
            .lock()
            .expect("not poisoned")
            .get(&super::entry_name(reference))
            .cloned()
            .ok_or(CredentialError::NotFound)
    }

    fn delete(&self, reference: &CredentialRef) -> Result<(), CredentialError> {
        self.kept
            .lock()
            .expect("not poisoned")
            .remove(&super::entry_name(reference));
        Ok(())
    }
}
