//! The seal on an update, and what it takes to break it.
//!
//! An update is code that will run as the person who installed it. The only thing standing between
//! that and somebody else's code is this check, so it is written out here rather than taken from a
//! crate that does it in one call — the four pieces it needs already compile into this application,
//! and what they do is short enough to read.
//!
//! The format is minisign, which is Ed25519 over the file and a second signature over a comment
//! nobody can change. Choosing an established format rather than inventing one means the key is
//! made with a tool that already exists and has been looked at by people who do this for a living.
//!
//! **A signature says who made a file, not which version it is.** Without something authenticated
//! naming the version, an attacker who can influence what is offered serves an older release —
//! signed, genuine, and with a hole that was fixed since. The version is therefore named inside the
//! signed manifest, whose own text is what the signature covers, rather than in the trusted comment
//! beside it: measured, the signing tool this project has writes its own timestamp and file name
//! there and leaves no room for anything else.
//!
//! The trusted comment is verified all the same. It is part of what a correct reader of this format
//! checks, it costs ten lines, and a reader that skipped it would leave the comment forgeable while
//! still calling the file verified.

use blake2::Blake2b512;
use digest::Digest;
use ring::signature::{UnparsedPublicKey, ED25519};

use crate::ports::updates::UpdateError;

/// What a public key is, unwrapped from the line it is written on.
pub(super) struct TheKey {
    /// Which key this is. Carried in the signature too, and compared before anything else — a
    /// mismatch here is an ordinary mistake rather than an attack, and saying so saves an afternoon.
    id: [u8; 8],
    public: [u8; 32],
}

impl TheKey {
    /// Reads the second line of a minisign public key file, or the whole of a one-line key.
    ///
    /// The comment line above it is untrusted by name and by nature: nothing signs it, so nothing
    /// here reads it.
    pub(super) fn written_as(key: &str) -> Result<Self, UpdateError> {
        let line = key
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty() && !line.starts_with("untrusted comment:"))
            .ok_or(UpdateError::NotOurs)?;

        let raw = from_base64(line)?;

        // Two bytes of algorithm, eight of key identifier, thirty-two of key.
        if raw.len() != 42 || &raw[..2] != b"Ed" {
            return Err(UpdateError::NotOurs);
        }

        let mut id = [0u8; 8];
        let mut public = [0u8; 32];
        id.copy_from_slice(&raw[2..10]);
        public.copy_from_slice(&raw[10..]);

        Ok(Self { id, public })
    }
}

/// Whether this file is what it says it is, and what the signer said about it.
///
/// Answers the trusted comment, which is the only text in a signature file anybody may believe —
/// the line above it is written by whoever made the file and signed by nobody. Nothing in this
/// application depends on what it says; it is returned because it was verified.
pub(super) fn what_the_seal_says(
    key: &TheKey,
    file: &[u8],
    signature: &str,
) -> Result<String, UpdateError> {
    let mut lines = signature
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty());

    let mut untrusted = lines.next().ok_or(UpdateError::NotOurs)?;
    if untrusted.starts_with("untrusted comment:") {
        untrusted = lines.next().ok_or(UpdateError::NotOurs)?;
    }

    let sealed = from_base64(untrusted)?;
    if sealed.len() != 74 {
        return Err(UpdateError::NotOurs);
    }

    // `Ed` signs the file; `ED` signs its hash, which is what a file of any size wants. Both are
    // minisign, and both are accepted — refusing one would refuse a key made with a tool that
    // happens to default the other way, for no gain at all.
    let prehashed = match &sealed[..2] {
        b"ED" => true,
        b"Ed" => false,
        _ => return Err(UpdateError::NotOurs),
    };

    if sealed[2..10] != key.id {
        return Err(UpdateError::NotOurs);
    }

    let over = match prehashed {
        true => Blake2b512::digest(file).to_vec(),
        false => file.to_vec(),
    };

    let public = UnparsedPublicKey::new(&ED25519, key.public);
    public
        .verify(&over, &sealed[10..])
        .map_err(|_| UpdateError::NotOurs)?;

    // The line naming what was signed, and the signature that covers it together with the one
    // above. Without this second check the comment is text anybody may rewrite, and the version it
    // names means nothing.
    let trusted = lines.next().ok_or(UpdateError::NotOurs)?;
    let said = trusted
        .strip_prefix("trusted comment:")
        .ok_or(UpdateError::NotOurs)?
        .trim()
        .to_owned();

    let globally = from_base64(lines.next().ok_or(UpdateError::NotOurs)?)?;

    let mut both = sealed[10..].to_vec();
    both.extend_from_slice(said.as_bytes());

    public
        .verify(&both, &globally)
        .map_err(|_| UpdateError::NotOurs)?;

    Ok(said)
}

/// Base64 as the format writes it, refusing anything that is not exactly that.
fn from_base64(line: &str) -> Result<Vec<u8>, UpdateError> {
    use base64::Engine;

    base64::engine::general_purpose::STANDARD
        .decode(line)
        .map_err(|_| UpdateError::NotOurs)
}

/// A key and a signature made by the signing tool this project already has, over a file whose bytes
/// are written out below.
///
/// The private half was thrown away and never left the machine that made it. Generating a fixture
/// from inside the test with the code under test would prove only that the code agrees with itself;
/// this was made by something else, which is the whole point of it.
#[cfg(test)]
pub(super) mod made_elsewhere {
    pub(in crate::infrastructure::updates) const PUBLIC: &str =
        "untrusted comment: minisign public key: DA86C534D71366A7\n\
        RWSnZhPXNMWG2onfZ1B7eL0km+PNv9zS6v47Xqzo3m0tZKMTM8CbNET5";

    pub(in crate::infrastructure::updates) const SEALED: &str =
        "untrusted comment: signature from tauri secret key\n\
        RUSnZhPXNMWG2m5Dhi0KbYwFUj0HRKl6TAnlW2oA8K0+q6jpYDF26vVwxKlC5bSpa/4hUAUZKZxMuskUiy4yX9FFkB+ITowf6Qg=\n\
        trusted comment: timestamp:1789471201\tfile:thing.txt\n\
        S9OiJWvbmz//Nh0Uu7s0P38XuwOUjbQX4D2UZVfG1hxqG8lQCcGRCnczw9U/PA6MwzAsNfXh+0fs3Y56akrWAg==";

    pub(in crate::infrastructure::updates) const SEALED_FILE: &[u8] = b"a file to be sealed\n";
}

#[cfg(test)]
mod tests {
    use super::made_elsewhere::{PUBLIC, SEALED, SEALED_FILE};
    use super::*;

    #[test]
    fn a_real_seal_over_the_real_file_is_accepted() {
        let key = TheKey::written_as(PUBLIC).expect("a key");
        let said = what_the_seal_says(&key, SEALED_FILE, SEALED).expect("the seal holds");

        // Read back because it is verified, not because anything depends on it: the version an
        // update is offered as is carried by the signed manifest, which names it in its own text.
        assert!(said.starts_with("timestamp:"));
    }

    /// One byte different is a different file.
    #[test]
    fn a_file_that_was_changed_after_it_was_sealed_is_refused() {
        let key = TheKey::written_as(PUBLIC).expect("a key");

        let mut tampered = SEALED_FILE.to_vec();
        tampered[0] = b'A';

        assert_eq!(
            what_the_seal_says(&key, &tampered, SEALED),
            Err(UpdateError::NotOurs)
        );
    }

    /// The trusted comment is covered by a signature of its own, and that one is checked too.
    ///
    /// Without the second check the comment is text anybody may rewrite between the signer and the
    /// reader, while the file itself still verifies — which is exactly how a signed thing gets
    /// described as something it is not.
    #[test]
    fn a_trusted_comment_somebody_rewrote_is_refused() {
        let key = TheKey::written_as(PUBLIC).expect("a key");
        let rewritten = SEALED.replace("file:thing.txt", "file:something-else.txt");

        assert_eq!(
            what_the_seal_says(&key, SEALED_FILE, &rewritten),
            Err(UpdateError::NotOurs)
        );
    }

    /// A seal made by a different key is not this build's seal.
    #[test]
    fn a_seal_from_another_key_is_refused() {
        // The same public key with one character of the key material changed, which is what an
        // attacker's key looks like from here: well-formed, and not ours.
        let elsewhere = PUBLIC.replace("RWSnZhPXNMWG2onf", "RWSnZhPXNMWG2onZ");
        let key = TheKey::written_as(&elsewhere).expect("a key");

        assert_eq!(
            what_the_seal_says(&key, SEALED_FILE, SEALED),
            Err(UpdateError::NotOurs)
        );
    }

    #[test]
    fn a_key_that_is_not_a_key_is_refused() {
        for refused in [
            "",
            "untrusted comment: only a comment",
            "bm90IGEga2V5",
            "!!!!not base64!!!!",
            "RWQ=",
        ] {
            assert!(
                TheKey::written_as(refused).is_err(),
                "{refused:?} was read as a key"
            );
        }
    }

    /// Nothing at all is accepted without a signature that verifies.
    #[test]
    fn a_file_with_no_seal_on_it_is_refused() {
        let key = TheKey::written_as(PUBLIC).expect("a key");

        for refused in [
            "",
            "untrusted comment: nothing follows",
            "untrusted comment: x\nbm90IGEgc2lnbmF0dXJl\ntrusted comment: 1.0.0\nbm90IGEgc2ln",
        ] {
            assert!(
                what_the_seal_says(&key, b"anything", refused).is_err(),
                "{refused:?} passed for a signature"
            );
        }
    }
}
