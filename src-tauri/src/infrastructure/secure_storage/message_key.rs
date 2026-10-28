//! The key the mail on this device is kept under.
//!
//! A message read once is kept whole, up to 8 MiB, in the file of the account it arrived in, so the
//! disk holds somebody's mail. The raw message is sealed before it is written, under a key kept where the
//! account passwords are, inside a database whose every page is encrypted as well.
//!
//! What this is not: protection against something already running as the person. The threat model
//! says so and this changes nothing about it. What it answers is the disk read by somebody else —
//! a machine taken with no full-disk encryption, a data directory swept into a cloud backup, a
//! `.sqlite3` attached to a bug report.
//!
//! **The seal covers the raw message and nothing else.** What the list and the search are built on
//! — who wrote, when, the subject, the preview and the text taken out of a message for search — is
//! protected by the page encryption alone: somebody holding the database's key reads it, and somebody
//! holding neither key reads nothing.

use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use secrecy::{ExposeSecret, SecretString};

use crate::ports::credential_store::{
    CredentialError, CredentialPurpose, CredentialRef, CredentialStore,
};

/// The account name this key is stored under.
///
/// Not an account of the person's: one key for this installation, so a message stays readable when
/// the account it arrived in is removed and the mail is still there. Reserved rather than chosen —
/// an account identifier is a UUID, and this is not one.
const THIS_DEVICE: &str = "this-device";

/// How many bytes the cipher takes.
const KEY_LEN: usize = 32;

/// The key, and what it can do with a message.
///
/// Holds the material rather than the store, so nothing asks the system keychain once per message —
/// on macOS every read of an entry this build did not write raises a dialog, and one per opened
/// message would be unusable.
#[derive(Clone)]
pub struct MessageKey {
    sealing: LessSafeKey,
    random: SystemRandom,
}

impl MessageKey {
    /// The key this device keeps its mail under, made on first use.
    ///
    /// Created rather than demanded: there is nothing for a person to remember here and nothing for
    /// them to decide. The one thing that matters is that it is made once and then found again,
    /// because a key made twice is a mailbox that cannot be read the second time.
    pub fn of_this_device(store: &dyn CredentialStore) -> Result<Self, CredentialError> {
        let reference = CredentialRef::new(THIS_DEVICE, CredentialPurpose::MessageKey);

        let material = match store.read(&reference) {
            Ok(held) => decoded(held.expose_secret()).ok_or(CredentialError::Unavailable)?,
            Err(CredentialError::NotFound) => {
                let made = new_key()?;
                store.save(&reference, &SecretString::from(encoded(&made)))?;
                made
            }
            Err(error) => return Err(error),
        };

        Self::from(&material)
    }

    /// A key from material somebody else holds. What a test uses, and the only way to make one
    /// without touching a store.
    pub fn from(material: &[u8; KEY_LEN]) -> Result<Self, CredentialError> {
        let sealing = UnboundKey::new(&CHACHA20_POLY1305, material)
            .map(LessSafeKey::new)
            .map_err(|_| CredentialError::Unavailable)?;

        Ok(Self {
            sealing,
            random: SystemRandom::new(),
        })
    }

    /// The message, sealed: the nonce it was sealed under, then the sealed bytes.
    ///
    /// A nonce of its own for every message, from the system's randomness. Ninety-six bits chosen
    /// at random collide at a scale no mailbox reaches — a million messages leaves the chance below
    /// one in ten to the seventeen — and the alternative, a counter, is a number that has to be
    /// stored, kept in step with the rows, and never rewound by a restore from backup.
    pub fn seal(&self, message: &[u8]) -> Option<Vec<u8>> {
        let mut nonce = [0u8; NONCE_LEN];
        self.random.fill(&mut nonce).ok()?;

        let mut sealed = message.to_vec();
        self.sealing
            .seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::empty(),
                &mut sealed,
            )
            .ok()?;

        let mut written = Vec::with_capacity(NONCE_LEN + sealed.len());
        written.extend_from_slice(&nonce);
        written.append(&mut sealed);
        Some(written)
    }

    /// The message back, or nothing when these bytes were not sealed by this key.
    ///
    /// Nothing rather than an error, and the caller treats it as a message this device does not
    /// have: it is fetched from the server again. Which is the honest answer — a row that will not
    /// open is a row whose contents are gone, and the message itself is not.
    pub fn open(&self, sealed: &[u8]) -> Option<Vec<u8>> {
        if sealed.len() < NONCE_LEN {
            return None;
        }
        let (nonce, body) = sealed.split_at(NONCE_LEN);
        let nonce: [u8; NONCE_LEN] = nonce.try_into().ok()?;

        let mut opening = body.to_vec();
        self.sealing
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::empty(),
                &mut opening,
            )
            .ok()?;

        opening.truncate(body.len() - CHACHA20_POLY1305.tag_len());
        Some(opening)
    }
}

/// Prints nothing of what it holds.
impl std::fmt::Debug for MessageKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MessageKey(…)")
    }
}

fn new_key() -> Result<[u8; KEY_LEN], CredentialError> {
    let mut material = [0u8; KEY_LEN];
    SystemRandom::new()
        .fill(&mut material)
        .map_err(|_| CredentialError::Unavailable)?;
    Ok(material)
}

/// Hex, because the store keeps text and this has to survive a round trip through it unchanged.
fn encoded(material: &[u8; KEY_LEN]) -> String {
    material
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
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
    use std::sync::Mutex;

    fn a_key() -> MessageKey {
        MessageKey::from(&[7u8; KEY_LEN]).expect("a key of the right size")
    }

    /// A store of this test's own. Nothing here reaches the system keychain: on a development build
    /// every read raises a dialog, and a suite that raised one per test would not run at all.
    #[derive(Default)]
    struct HeldHere {
        entries: Mutex<Vec<(String, String)>>,
    }

    impl CredentialStore for HeldHere {
        fn save(
            &self,
            reference: &CredentialRef,
            secret: &SecretString,
        ) -> Result<(), CredentialError> {
            let name = format!("{}:{}", reference.account_id, reference.purpose.as_str());
            let mut entries = self.entries.lock().expect("nothing poisoned it");
            entries.retain(|(held, _)| held != &name);
            entries.push((name, secret.expose_secret().to_owned()));
            Ok(())
        }

        fn read(&self, reference: &CredentialRef) -> Result<SecretString, CredentialError> {
            let name = format!("{}:{}", reference.account_id, reference.purpose.as_str());
            self.entries
                .lock()
                .expect("nothing poisoned it")
                .iter()
                .find(|(held, _)| held == &name)
                .map(|(_, secret)| SecretString::from(secret.clone()))
                .ok_or(CredentialError::NotFound)
        }

        fn delete(&self, _reference: &CredentialRef) -> Result<(), CredentialError> {
            Ok(())
        }
    }

    #[test]
    fn a_message_comes_back_exactly_as_it_went_in() {
        let key = a_key();
        let message = b"From: zeta@example.com\r\nSubject: About Tuesday\r\n\r\nShall we?";

        let sealed = key.seal(message).expect("it seals");
        assert_eq!(key.open(&sealed).as_deref(), Some(message.as_slice()));
    }

    #[test]
    fn what_is_written_is_not_the_message() {
        // The whole point, and the thing that would be silently lost if the sealing were ever
        // dropped from the path: the bytes on the disk must not be the mail.
        let key = a_key();
        let message = b"Subject: Divorce papers\r\n\r\nplease sign";

        let sealed = key.seal(message).expect("it seals");

        assert!(
            !sealed
                .windows(message.len())
                .any(|window| window == message),
            "the message is on the disk as it was written"
        );
        assert!(!sealed.windows(14).any(|window| window == b"Divorce papers"));
    }

    #[test]
    fn the_same_message_twice_is_not_the_same_bytes_twice() {
        // A nonce of its own each time. Sealing twice to the same bytes would tell anybody reading
        // the file which messages are copies of each other.
        let key = a_key();
        let message = b"Subject: the same thing\r\n\r\nagain";

        assert_ne!(key.seal(message), key.seal(message));
    }

    #[test]
    fn another_key_does_not_open_it() {
        let sealed = a_key()
            .seal(b"Subject: mine\r\n\r\nnot yours")
            .expect("it seals");
        let somebody_else = MessageKey::from(&[9u8; KEY_LEN]).expect("a key");

        assert_eq!(somebody_else.open(&sealed), None);
    }

    #[test]
    fn a_changed_byte_does_not_open_it() {
        // Sealed rather than merely encrypted: a row somebody edited on the disk is refused, not
        // decrypted into something else and handed to a parser.
        let key = a_key();
        let mut sealed = key
            .seal(b"Subject: as written\r\n\r\nintact")
            .expect("it seals");
        let last = sealed.len() - 1;
        sealed[last] ^= 0x01;

        assert_eq!(key.open(&sealed), None);
    }

    #[test]
    fn bytes_that_were_never_sealed_do_not_open() {
        // What every row written before this existed looks like. They are deleted by the migration
        // rather than read, and this is the answer if one is ever read anyway.
        let key = a_key();

        assert_eq!(key.open(b"Subject: in the clear\r\n\r\nhello"), None);
        assert_eq!(key.open(b""), None);
        assert_eq!(key.open(&[0u8; 8]), None);
    }

    #[test]
    fn the_key_is_made_once_and_then_found_again() {
        // A key made twice is a mailbox that cannot be read the second time.
        let store = HeldHere::default();

        let first = MessageKey::of_this_device(&store).expect("a key is made");
        let sealed = first
            .seal(b"Subject: kept\r\n\r\nacross a restart")
            .expect("seals");

        let second = MessageKey::of_this_device(&store).expect("the same key is found");
        assert_eq!(
            second.open(&sealed).as_deref(),
            Some(b"Subject: kept\r\n\r\nacross a restart".as_slice())
        );
    }

    #[test]
    fn what_the_store_holds_is_the_key_and_nothing_else_about_it() {
        let store = HeldHere::default();
        MessageKey::of_this_device(&store).expect("a key is made");

        let entries = store.entries.lock().expect("nothing poisoned it");
        let (name, secret) = entries.first().expect("one entry");

        assert_eq!(name, "this-device:message-key");
        assert_eq!(secret.len(), KEY_LEN * 2, "the key is stored as it is read");
        assert!(secret
            .chars()
            .all(|character| character.is_ascii_hexdigit()));
    }

    #[test]
    fn a_key_the_store_cannot_give_back_is_a_failure_rather_than_a_new_key() {
        // The dangerous middle: a store that answers with something unusable. Making a fresh key
        // there would leave every message already sealed unreadable, silently.
        let store = HeldHere::default();
        store
            .save(
                &CredentialRef::new(THIS_DEVICE, CredentialPurpose::MessageKey),
                &SecretString::from("not a key".to_owned()),
            )
            .expect("it saves");

        assert!(MessageKey::of_this_device(&store).is_err());
    }
}
