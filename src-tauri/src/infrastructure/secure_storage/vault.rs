//! The secrets file for a system that has no store of its own.
//!
//! Linux is the case: a desktop without a Secret Service has nowhere to put a password, and the
//! answer is not to write one to disk quietly. It is to say so, ask for a master password, and turn
//! that password into the key to a file nobody else can read.
//!
//! What is on disk is a header and one sealed block. The header names the work the key cost, so a
//! build that raises it can still open a file written by an earlier one, and it is sealed along
//! with the secrets: lowering the work in the header damages the file rather than weakening it.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use argon2::{Algorithm, Argon2, Params, Version};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};
use secrecy::zeroize::Zeroize;
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

/// What the file starts with. Sixteen bytes, so the header is a fixed size to read.
const MAGIC: &[u8; 16] = b"LYONOYL VAULT 1\n";

const SALT_LEN: usize = 16;
const KEY_LEN: usize = 32;

/// Where each field sits. The whole header is sealed with the secrets, so these offsets are part of
/// the format and cannot move without a new magic.
const MEMORY_AT: usize = MAGIC.len();
const ITERATIONS_AT: usize = MEMORY_AT + 4;
const LANES_AT: usize = ITERATIONS_AT + 4;
const SALT_AT: usize = LANES_AT + 4;
const NONCE_AT: usize = SALT_AT + SALT_LEN;
const HEADER_LEN: usize = NONCE_AT + NONCE_LEN;

/// How hard it is to turn the master password into a key.
///
/// Stored in the file rather than assumed, because the numbers below rise from build to build and
/// a file this build writes has to keep opening in every build after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    /// Memory in kibibytes. The one that matters: it is what makes a rented machine expensive.
    pub memory_kib: u32,
    pub iterations: u32,
    pub lanes: u32,
}

impl Work {
    /// What this build writes with. The floor is 256 MB of memory.
    pub const SHIPPED: Self = Self {
        memory_kib: 256 * 1024,
        iterations: 3,
        lanes: 4,
    };

    /// Whether the numbers are ones this build is willing to open a file with.
    ///
    /// A file names its own work, and a file naming almost none would be a way to hand this build a
    /// key that costs nothing to guess.
    fn credible(self) -> bool {
        self.memory_kib >= 8 * 1024 && self.iterations >= 1 && self.lanes >= 1
    }
}

/// The floor for the shipped numbers, checked when this file is compiled rather than when a test
/// runs: lowering it does not build.
const _: () = assert!(Work::SHIPPED.memory_kib >= 256 * 1024);
const _: () = assert!(Work::SHIPPED.iterations >= 1);
const _: () = assert!(Work::SHIPPED.lanes >= 1);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VaultError {
    /// The master password does not open this file. Told apart from damage because it is the one a
    /// person can do something about.
    #[error("the master password does not open the secrets file")]
    WrongPassword,
    /// The file is not one of ours, or not the whole of one, or has been changed since it was
    /// written.
    #[error("the secrets file cannot be read")]
    Damaged,
    /// The file could not be reached at all: permissions, a full disk, a missing directory.
    #[error("the secrets file is unavailable")]
    Unavailable,
    /// There is already a file there. Writing a new one over it would throw away every secret in
    /// it, which is not something a mistyped password should be able to do.
    #[error("a secrets file is already there")]
    AlreadyThere,
}

/// The secrets of one device, sealed with a key derived from a master password.
///
/// Holds them decrypted while it is open, which is the point: the alternative is deriving the key
/// again for every read, and the derivation is deliberately expensive.
pub struct Vault {
    path: PathBuf,
    key: LessSafeKey,
    work: Work,
    salt: [u8; SALT_LEN],
    entries: BTreeMap<String, String>,
}

/// Nothing about the contents reaches a log line.
impl std::fmt::Debug for Vault {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Vault")
            .field("path", &self.path)
            .field("entries", &self.entries.len())
            .finish()
    }
}

/// What it holds decrypted goes out of memory with it, rather than being left for whatever writes
/// over that page next.
impl Drop for Vault {
    fn drop(&mut self) {
        for secret in self.entries.values_mut() {
            secret.zeroize();
        }
    }
}

impl Vault {
    /// Whether a file has been made here already.
    pub fn exists(path: &Path) -> bool {
        path.exists()
    }

    /// Makes the file, with nothing in it yet.
    ///
    /// Refuses to write over one that is there: the secrets in it would be gone, and no answer to a
    /// password prompt should be able to do that.
    pub fn create(path: &Path, master: &SecretString, work: Work) -> Result<Self, VaultError> {
        if Self::exists(path) {
            return Err(VaultError::AlreadyThere);
        }

        let mut salt = [0u8; SALT_LEN];
        SystemRandom::new()
            .fill(&mut salt)
            .map_err(|_| VaultError::Unavailable)?;

        let vault = Self {
            path: path.to_path_buf(),
            key: derive(master, &salt, work)?,
            work,
            salt,
            entries: BTreeMap::new(),
        };

        vault.write()?;

        Ok(vault)
    }

    /// Opens the file with the master password, or says which way it failed.
    pub fn unlock(path: &Path, master: &SecretString) -> Result<Self, VaultError> {
        let stored = std::fs::read(path).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => VaultError::Damaged,
            _ => VaultError::Unavailable,
        })?;

        if stored.len() <= HEADER_LEN || !stored.starts_with(MAGIC) {
            return Err(VaultError::Damaged);
        }

        let work = Work {
            memory_kib: number_at(&stored, MEMORY_AT),
            iterations: number_at(&stored, ITERATIONS_AT),
            lanes: number_at(&stored, LANES_AT),
        };
        if !work.credible() {
            return Err(VaultError::Damaged);
        }

        let mut salt = [0u8; SALT_LEN];
        salt.copy_from_slice(&stored[SALT_AT..SALT_AT + SALT_LEN]);

        let key = derive(master, &salt, work)?;
        let nonce = Nonce::try_assume_unique_for_key(&stored[NONCE_AT..HEADER_LEN])
            .map_err(|_| VaultError::Damaged)?;

        let mut sealed = stored[HEADER_LEN..].to_vec();
        // The header is sealed with the secrets: changing the work it names damages the file
        // instead of making the key cheaper to find.
        let opened = key
            .open_in_place(nonce, Aad::from(&stored[..HEADER_LEN]), &mut sealed)
            // Both cases arrive here as one: a key that does not open the block is indistinguishable
            // from a block that was changed. A file this build wrote is only ever the first.
            .map_err(|_| VaultError::WrongPassword)?;

        let entries = serde_json::from_slice(opened).map_err(|_| VaultError::Damaged);
        sealed.zeroize();

        Ok(Self {
            path: path.to_path_buf(),
            key,
            work,
            salt,
            entries: entries?,
        })
    }

    pub fn get(&self, name: &str) -> Option<SecretString> {
        self.entries
            .get(name)
            .map(|secret| SecretString::from(secret.clone()))
    }

    /// Keeps a secret, and keeps it only once it is in the file. Held here alone it would be read
    /// back as if it were there, and gone with the application — the key the databases are written
    /// under made, used, and lost.
    pub fn put(&mut self, name: &str, secret: &SecretString) -> Result<(), VaultError> {
        let before = self
            .entries
            .insert(name.to_owned(), secret.expose_secret().to_owned());

        let written = self.write();
        if written.is_err() {
            let mut unwritten = match before {
                Some(before) => self.entries.insert(name.to_owned(), before),
                None => self.entries.remove(name),
            };
            if let Some(secret) = unwritten.as_mut() {
                secret.zeroize();
            }
        }

        written
    }

    /// Removes a secret. A name that is not there is the state the caller asked for.
    ///
    /// Kept when the file cannot be written, as [`Vault::put`] keeps what was there: the file still
    /// holds it, and a second try that found it already gone here would answer "removed" about a
    /// secret the next opening brings straight back.
    pub fn remove(&mut self, name: &str) -> Result<(), VaultError> {
        let Some(mut secret) = self.entries.remove(name) else {
            return Ok(());
        };

        let written = self.write();
        if written.is_ok() {
            secret.zeroize();
        } else {
            self.entries.insert(name.to_owned(), secret);
        }

        written
    }

    /// Writes the whole file, and only then puts it in place.
    ///
    /// Through a neighbouring file and a rename, so an interrupted write leaves the previous
    /// secrets rather than half of the new ones. The permissions are set as the file is made, not
    /// after: a file that is readable for even a moment has been readable.
    fn write(&self) -> Result<(), VaultError> {
        let mut plain = serde_json::to_vec(&self.entries).map_err(|_| VaultError::Unavailable)?;

        let mut nonce = [0u8; NONCE_LEN];
        SystemRandom::new()
            .fill(&mut nonce)
            .map_err(|_| VaultError::Unavailable)?;

        let mut header = Vec::with_capacity(HEADER_LEN);
        header.extend_from_slice(MAGIC);
        header.extend_from_slice(&self.work.memory_kib.to_le_bytes());
        header.extend_from_slice(&self.work.iterations.to_le_bytes());
        header.extend_from_slice(&self.work.lanes.to_le_bytes());
        header.extend_from_slice(&self.salt);
        header.extend_from_slice(&nonce);

        let sealing = self
            .key
            .seal_in_place_separate_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(&header),
                &mut plain,
            )
            .map_err(|_| VaultError::Unavailable);

        let outcome = sealing.and_then(|tag| {
            let mut file = Vec::with_capacity(HEADER_LEN + plain.len() + tag.as_ref().len());
            file.extend_from_slice(&header);
            file.extend_from_slice(&plain);
            file.extend_from_slice(tag.as_ref());

            self.replace(&file)
        });

        plain.zeroize();

        outcome
    }

    fn replace(&self, contents: &[u8]) -> Result<(), VaultError> {
        let beside = self.path.with_extension("writing");

        let mut file = private_file(&beside)?;
        file.write_all(contents)
            .and_then(|()| file.sync_all())
            .map_err(|_| VaultError::Unavailable)?;
        drop(file);

        std::fs::rename(&beside, &self.path).map_err(|_| VaultError::Unavailable)
    }
}

/// A file only its owner can read, made that way from the moment it exists.
fn private_file(path: &Path) -> Result<std::fs::File, VaultError> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);

    // Windows has no mode bits; what protects the file there is the user profile directory it sits
    // in, which is the same protection the databases beside it have.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    options.open(path).map_err(|_| VaultError::Unavailable)
}

fn number_at(stored: &[u8], at: usize) -> u32 {
    let mut bytes = [0u8; 4];
    bytes.copy_from_slice(&stored[at..at + 4]);

    u32::from_le_bytes(bytes)
}

/// Turns the master password into the key, at the cost the file names.
fn derive(master: &SecretString, salt: &[u8], work: Work) -> Result<LessSafeKey, VaultError> {
    let parameters = Params::new(work.memory_kib, work.iterations, work.lanes, Some(KEY_LEN))
        .map_err(|_| VaultError::Damaged)?;

    let mut derived = [0u8; KEY_LEN];
    let outcome = Argon2::new(Algorithm::Argon2id, Version::V0x13, parameters)
        .hash_password_into(master.expose_secret().as_bytes(), salt, &mut derived)
        // A derivation that cannot run is a machine that cannot open the file — not a wrong
        // password, and saying so would send the person to change something that is right.
        .map_err(|_| VaultError::Unavailable);

    let key = outcome.and_then(|()| {
        UnboundKey::new(&CHACHA20_POLY1305, &derived).map_err(|_| VaultError::Unavailable)
    });
    derived.zeroize();

    Ok(LessSafeKey::new(key?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the tests derive with. The shipped numbers take a quarter of a gigabyte and the better
    /// part of a second by design; every test here is about the file, not about the cost of the key.
    const QUICK: Work = Work {
        memory_kib: 8 * 1024,
        iterations: 1,
        lanes: 1,
    };

    /// A directory of its own per test, removed when the test ends however it ends.
    struct Scratch {
        directory: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "lyonoyl-vault-{name}-{}",
                std::process::id() as u64 + line_of(name)
            ));
            std::fs::create_dir_all(&directory).expect("a directory to write in");

            Self { directory }
        }

        fn file(&self) -> PathBuf {
            self.directory.join("secrets.vault")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    /// Keeps two tests running at once out of each other's directory without a clock.
    fn line_of(name: &str) -> u64 {
        name.bytes().map(u64::from).sum()
    }

    fn master(text: &str) -> SecretString {
        SecretString::from(text.to_owned())
    }

    #[test]
    fn a_secret_put_in_is_there_after_it_is_opened_again() {
        let scratch = Scratch::new("round-trip");

        let mut vault =
            Vault::create(&scratch.file(), &master("open sesame"), QUICK).expect("a new file");
        vault
            .put("acc-1:imap-password", &master("correct horse"))
            .expect("stored");
        drop(vault);

        let reopened = Vault::unlock(&scratch.file(), &master("open sesame")).expect("it opens");

        assert_eq!(
            reopened
                .get("acc-1:imap-password")
                .expect("the secret is there")
                .expose_secret(),
            "correct horse"
        );
        assert!(reopened.get("acc-2:imap-password").is_none());
    }

    #[test]
    fn another_password_does_not_open_it() {
        let scratch = Scratch::new("wrong-password");
        let mut vault = Vault::create(&scratch.file(), &master("open sesame"), QUICK).expect("new");
        vault
            .put("acc-1", &master("correct horse"))
            .expect("stored");

        assert_eq!(
            Vault::unlock(&scratch.file(), &master("open sesamf")).expect_err("it must not open"),
            VaultError::WrongPassword
        );
    }

    #[test]
    fn a_file_changed_since_it_was_written_does_not_open_either() {
        // Reported as a wrong password because the two cannot be told apart: a key that does not
        // open the block and a block that was altered fail the same check. The interface says the
        // password did not open the file, which is true of both.
        let scratch = Scratch::new("tampered");
        let mut vault = Vault::create(&scratch.file(), &master("open sesame"), QUICK).expect("new");
        vault
            .put("acc-1", &master("correct horse"))
            .expect("stored");
        drop(vault);

        let mut stored = std::fs::read(scratch.file()).expect("readable");
        let last = stored.len() - 1;
        stored[last] ^= 0x01;
        std::fs::write(scratch.file(), &stored).expect("writable");

        assert_eq!(
            Vault::unlock(&scratch.file(), &master("open sesame")).expect_err("it must not open"),
            VaultError::WrongPassword
        );
    }

    #[test]
    fn a_file_naming_work_nobody_would_have_written_is_refused() {
        // The header says what the key cost, so a file can name almost nothing and hand this build
        // a key that is cheap to guess. It is refused before the derivation runs.
        let scratch = Scratch::new("cheap-work");
        Vault::create(&scratch.file(), &master("open sesame"), QUICK).expect("new");

        let mut stored = std::fs::read(scratch.file()).expect("readable");
        stored[MEMORY_AT..MEMORY_AT + 4].copy_from_slice(&8u32.to_le_bytes());
        std::fs::write(scratch.file(), &stored).expect("writable");

        assert_eq!(
            Vault::unlock(&scratch.file(), &master("open sesame")).expect_err("refused"),
            VaultError::Damaged
        );
    }

    #[test]
    fn something_that_is_not_one_of_ours_is_not_read_as_one() {
        let scratch = Scratch::new("not-ours");
        std::fs::write(scratch.file(), b"this is a text file, not a vault").expect("writable");

        assert_eq!(
            Vault::unlock(&scratch.file(), &master("open sesame")).expect_err("refused"),
            VaultError::Damaged
        );
    }

    #[test]
    fn half_a_file_is_refused_rather_than_read() {
        let scratch = Scratch::new("truncated");
        Vault::create(&scratch.file(), &master("open sesame"), QUICK).expect("new");

        let stored = std::fs::read(scratch.file()).expect("readable");
        std::fs::write(scratch.file(), &stored[..HEADER_LEN]).expect("writable");

        assert_eq!(
            Vault::unlock(&scratch.file(), &master("open sesame")).expect_err("refused"),
            VaultError::Damaged
        );
    }

    #[test]
    fn a_file_that_is_already_there_is_never_written_over() {
        // Every secret on the device is in it. A second answer to a password prompt must not be
        // able to throw them away.
        let scratch = Scratch::new("already-there");
        let mut vault = Vault::create(&scratch.file(), &master("open sesame"), QUICK).expect("new");
        vault
            .put("acc-1", &master("correct horse"))
            .expect("stored");
        drop(vault);

        assert_eq!(
            Vault::create(&scratch.file(), &master("another one"), QUICK).expect_err("refused"),
            VaultError::AlreadyThere
        );

        let reopened = Vault::unlock(&scratch.file(), &master("open sesame")).expect("still opens");
        assert!(reopened.get("acc-1").is_some(), "the secret survived");
    }

    #[test]
    fn a_secret_that_was_removed_is_gone_from_the_file() {
        let scratch = Scratch::new("removed");
        let mut vault = Vault::create(&scratch.file(), &master("open sesame"), QUICK).expect("new");
        vault
            .put("acc-1", &master("correct horse"))
            .expect("stored");
        vault.remove("acc-1").expect("removed");
        vault.remove("acc-1").expect("removing twice is allowed");
        drop(vault);

        let reopened = Vault::unlock(&scratch.file(), &master("open sesame")).expect("it opens");
        assert!(reopened.get("acc-1").is_none());

        let stored = std::fs::read(scratch.file()).expect("readable");
        assert!(
            !contains(&stored, b"correct horse"),
            "the secret is still in the bytes on disk"
        );
    }

    #[test]
    fn what_is_on_disk_is_not_the_secret() {
        let scratch = Scratch::new("sealed");
        let mut vault = Vault::create(&scratch.file(), &master("open sesame"), QUICK).expect("new");
        vault
            .put("acc-1:imap-password", &master("correct horse"))
            .expect("stored");
        drop(vault);

        let stored = std::fs::read(scratch.file()).expect("readable");

        assert!(
            !contains(&stored, b"correct horse"),
            "the secret is in clear"
        );
        assert!(
            !contains(&stored, b"open sesame"),
            "the master password is in the file"
        );
        assert!(
            !contains(&stored, b"acc-1:imap-password"),
            "which accounts exist is in clear"
        );
    }

    #[test]
    fn writing_leaves_nothing_beside_the_file() {
        // The write goes through a neighbour and a rename, so an interrupted one leaves the
        // previous secrets. What it must not leave is the neighbour.
        let scratch = Scratch::new("atomic");
        let mut vault = Vault::create(&scratch.file(), &master("open sesame"), QUICK).expect("new");
        vault
            .put("acc-1", &master("correct horse"))
            .expect("stored");

        let left: Vec<_> = std::fs::read_dir(&scratch.directory)
            .expect("readable")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name())
            .collect();

        assert_eq!(left.len(), 1, "left behind: {left:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_secret_that_could_not_be_written_is_not_kept_either() {
        use std::os::unix::fs::PermissionsExt;

        let scratch = Scratch::new("unwritten");
        let mut vault = Vault::create(&scratch.file(), &master("open sesame"), QUICK).expect("new");
        vault.put("acc-1", &master("first")).expect("stored");
        std::fs::set_permissions(&scratch.directory, std::fs::Permissions::from_mode(0o500))
            .expect("a folder nothing can be written in");

        let replaced = vault.put("acc-1", &master("second"));
        let added = vault.put("acc-2", &master("third"));

        std::fs::set_permissions(&scratch.directory, std::fs::Permissions::from_mode(0o700))
            .expect("writable again");
        assert!(replaced.is_err() && added.is_err());
        assert_eq!(
            vault
                .get("acc-1")
                .map(|secret| secret.expose_secret().to_owned()),
            Some("first".to_owned()),
            "what the file holds was replaced by what it does not"
        );
        assert!(
            vault.get("acc-2").is_none(),
            "kept although it is in no file"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_secret_that_could_not_be_removed_from_the_file_is_still_there_to_remove() {
        // Dropped here and still in the file, the second try would find nothing to remove and
        // answer "removed", and the next opening would bring the password back with no account
        // left to say whose it is.
        use std::os::unix::fs::PermissionsExt;

        let scratch = Scratch::new("unremoved");
        let mut vault = Vault::create(&scratch.file(), &master("open sesame"), QUICK).expect("new");
        vault.put("acc-1", &master("first")).expect("stored");
        std::fs::set_permissions(&scratch.directory, std::fs::Permissions::from_mode(0o500))
            .expect("a folder nothing can be written in");

        let refused = vault.remove("acc-1");

        std::fs::set_permissions(&scratch.directory, std::fs::Permissions::from_mode(0o700))
            .expect("writable again");
        assert!(refused.is_err());
        assert_eq!(
            vault
                .get("acc-1")
                .map(|secret| secret.expose_secret().to_owned()),
            Some("first".to_owned()),
            "not kept as the file holds it"
        );

        vault.remove("acc-1").expect("removed on the second try");
        let reopened = Vault::unlock(&scratch.file(), &master("open sesame")).expect("it opens");
        assert!(reopened.get("acc-1").is_none(), "the file still holds it");
    }

    #[cfg(unix)]
    #[test]
    fn nobody_else_can_read_it() {
        use std::os::unix::fs::PermissionsExt;

        let scratch = Scratch::new("mode");
        let mut vault = Vault::create(&scratch.file(), &master("open sesame"), QUICK).expect("new");
        vault
            .put("acc-1", &master("correct horse"))
            .expect("stored");

        let mode = std::fs::metadata(scratch.file())
            .expect("readable")
            .permissions()
            .mode()
            & 0o777;

        assert_eq!(
            mode, 0o600,
            "the file is readable by someone else: {mode:o}"
        );
    }

    #[test]
    fn the_shipped_work_is_one_this_build_would_open_a_file_with() {
        // The floor is checked where the numbers are, when the file is compiled. What is left for a
        // test is that the shipped numbers pass the same check a file's own numbers have to pass,
        // so this build cannot write a file it would then refuse to open.
        assert!(Work::SHIPPED.credible());
    }

    #[test]
    fn nothing_of_what_it_holds_reaches_a_log_line() {
        let scratch = Scratch::new("debug");
        let mut vault = Vault::create(&scratch.file(), &master("open sesame"), QUICK).expect("new");
        vault
            .put("acc-1:imap-password", &master("correct horse"))
            .expect("stored");

        let printed = format!("{vault:?}");

        assert!(!printed.contains("correct horse"));
        assert!(!printed.contains("open sesame"));
        assert!(!printed.contains("acc-1"));
    }

    #[test]
    #[ignore = "derives a key at the shipped cost; started explicitly"]
    fn the_shipped_work_costs_what_a_person_would_wait() {
        let scratch = Scratch::new("shipped-cost");
        let started = std::time::Instant::now();

        Vault::create(&scratch.file(), &master("open sesame"), Work::SHIPPED).expect("new");
        let taken = started.elapsed();

        eprintln!("deriving at the shipped cost took {taken:?}");
        assert!(
            taken < std::time::Duration::from_secs(10),
            "unlocking would take {taken:?}"
        );
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }
}
