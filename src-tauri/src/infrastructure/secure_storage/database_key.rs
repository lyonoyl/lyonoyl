//! The key the databases on this device are encrypted with.
//!
//! Every page of `profile.sqlite3` and of every account's index is written encrypted, so a file
//! read by somebody else holds no subject, no address, no draft and no message — only how many
//! files there are and how large each one is. What it does not answer is something already running
//! as this person: the key is in the system store, unlocked for them, or in the file a master
//! password opens, held open by this application once it is given — the threat model says so.
//!
//! Given to SQLite as raw material rather than as a passphrase. A passphrase is stretched by
//! 256,000 rounds of PBKDF2 on **every connection**, and this application opens a pool per account
//! — a hundred mailboxes would spend that on every start.

use ring::rand::{SecureRandom, SystemRandom};
use secrecy::{ExposeSecret, SecretString};

use crate::ports::credential_store::{
    CredentialError, CredentialPurpose, CredentialRef, CredentialStore,
};

/// One installation, one key. Not one per account: a file belongs to an account, and a key that
/// went with it would leave the profile — which names them all — under a key of nobody's.
const THIS_DEVICE: &str = "this-device";

/// How many bytes SQLCipher takes as a raw key.
const KEY_LEN: usize = 32;

/// The key material, in the form SQLite is handed it.
#[derive(Clone)]
pub struct DatabaseKey(SecretString);

impl DatabaseKey {
    /// The key this installation uses, made on first use and kept where this device keeps its
    /// secrets.
    pub fn of_this_device(store: &dyn CredentialStore) -> Result<Self, CredentialError> {
        let reference = CredentialRef::new(THIS_DEVICE, CredentialPurpose::DatabaseKey);

        let material = match store.read(&reference) {
            Ok(held) => decoded(held.expose_secret()).ok_or(CredentialError::Unavailable)?,
            Err(CredentialError::NotFound) => {
                let made = new_key()?;
                store.save(&reference, &SecretString::from(encoded(&made)))?;
                made
            }
            Err(error) => return Err(error),
        };

        Ok(Self::from(&material))
    }

    /// The key this installation already uses, without making one when there is none.
    ///
    /// For a store asked after the databases exist: a key made then would be one they were not
    /// written under, kept for good in place of the one that is missing, and the files it cannot
    /// open would read as broken.
    pub fn already_kept_in(store: &dyn CredentialStore) -> Result<Self, CredentialError> {
        let reference = CredentialRef::new(THIS_DEVICE, CredentialPurpose::DatabaseKey);
        let held = store.read(&reference)?;
        let material = decoded(held.expose_secret()).ok_or(CredentialError::Unavailable)?;

        Ok(Self::from(&material))
    }

    /// A key from material somebody else holds. What a test uses, and the only way to make one
    /// without touching a store.
    pub fn from(material: &[u8; KEY_LEN]) -> Self {
        Self(SecretString::from(format!("\"x'{}'\"", encoded(material))))
    }

    /// Unlocks a connection with this key, which is the only thing this key is for.
    ///
    /// Read here and nowhere else: this is where it leaves for its destination, the same way a
    /// password is read only where it goes onto the wire.
    ///
    /// Its refusal is answered with a sentence of our own rather than with what SQLite said,
    /// because what SQLite says about a key it would not take is a message quoting that key — and
    /// an error text holding the key is the key in a log.
    ///
    /// The quoting around the material is not decoration: a bare `x'…'` is a blob literal and the
    /// pragma refuses it outright. Raw material is given as a string holding that literal.
    pub fn unlock(&self, connection: &rusqlite::Connection) -> Result<(), rusqlite::Error> {
        connection
            .execute_batch(&format!("PRAGMA key = {};", self.0.expose_secret()))
            .map_err(|_| {
                rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_NOTADB),
                    Some("the database key was refused".to_owned()),
                )
            })
    }
}

#[cfg(test)]
impl DatabaseKey {
    /// The key a test opens its databases with. One value, so two pools over one file agree.
    pub fn for_a_test() -> Self {
        Self::from(&[0x5a; KEY_LEN])
    }
}

fn new_key() -> Result<[u8; KEY_LEN], CredentialError> {
    let mut material = [0u8; KEY_LEN];
    SystemRandom::new()
        .fill(&mut material)
        .map_err(|_| CredentialError::Unavailable)?;
    Ok(material)
}

fn encoded(material: &[u8; KEY_LEN]) -> String {
    material.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decoded(text: &str) -> Option<[u8; KEY_LEN]> {
    if text.len() != KEY_LEN * 2 {
        return None;
    }

    let mut material = [0u8; KEY_LEN];
    for (at, byte) in material.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(at * 2..at * 2 + 2)?, 16).ok()?;
    }
    Some(material)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_handed_over_as_raw_material_rather_than_as_a_passphrase() {
        // A passphrase costs 256,000 rounds of PBKDF2 per connection, and this application opens a
        // pool per account. The `x'...'` form is what tells SQLite these bytes are the key itself.
        let key = DatabaseKey::from(&[7u8; KEY_LEN]);
        let written = key.0.expose_secret();

        assert!(written.starts_with("\"x'"), "{written}");
        assert!(written.ends_with("'\""), "{written}");
        assert_eq!(written.len(), KEY_LEN * 2 + 5);
    }

    /// A store that keeps what it is given, which is all this needs of one.
    #[derive(Default)]
    struct HeldSecrets(std::sync::Mutex<std::collections::HashMap<String, String>>);

    impl CredentialStore for HeldSecrets {
        fn save(
            &self,
            reference: &CredentialRef,
            secret: &SecretString,
        ) -> Result<(), CredentialError> {
            self.0.lock().expect("test lock").insert(
                reference.account_id.clone(),
                secret.expose_secret().to_owned(),
            );
            Ok(())
        }

        fn read(&self, reference: &CredentialRef) -> Result<SecretString, CredentialError> {
            self.0
                .lock()
                .expect("test lock")
                .get(&reference.account_id)
                .map(|held| SecretString::from(held.clone()))
                .ok_or(CredentialError::NotFound)
        }

        fn delete(&self, _reference: &CredentialRef) -> Result<(), CredentialError> {
            Ok(())
        }
    }

    #[test]
    fn the_same_installation_gets_the_same_key_every_time_it_is_asked() {
        // A second start that made a new key would find every file written under one nothing holds
        // any more — every message, every draft, every account, unreadable and not recoverable.
        let store = HeldSecrets::default();

        let first = DatabaseKey::of_this_device(&store).expect("a key is made");
        let again = DatabaseKey::of_this_device(&store).expect("a key is read back");

        assert_eq!(first.0.expose_secret(), again.0.expose_secret());
        assert_ne!(
            first.0.expose_secret(),
            DatabaseKey::from(&[0u8; KEY_LEN]).0.expose_secret(),
            "the key made for this installation is not material anybody could guess"
        );

        // And it opens a database, which is the only thing it is for.
        let connection = rusqlite::Connection::open_in_memory().expect("a database");
        first.unlock(&connection).expect("the key is taken");
    }
}
