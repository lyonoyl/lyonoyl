//! Reading a picture a message wrote into its own body.
//!
//! `data:image/png;base64,…`. Its own file because it is a decoder and nothing else: what a body is
//! allowed to contain is decided next door, and this only answers what these characters are.

use super::{picture_served_as, LARGEST_PICTURE};

/// A picture written into the body itself, decoded.
///
/// `data:image/png;base64,…`. Messages carry small pictures this way — the stars of a rating, an
/// icon in a signature — and the sanitizer dropped the whole element before anything here could
/// count it: no picture, no notice, and nothing on screen to say why. It is the safest kind there
/// is: the bytes are already in hand, nothing is fetched, and nobody learns the message was
/// opened.
///
/// Refused unless it is a kind this build serves and small enough to hold, and served under that
/// kind rather than under the one the message wrote.
pub(crate) fn picture_written_in(value: &str) -> Option<(&'static str, Vec<u8>)> {
    let described = value.strip_prefix("data:")?;
    let (about, data) = described.split_once(',')?;
    let (kind, encoding) = match about.rsplit_once(';') {
        Some((kind, encoding)) => (kind, encoding),
        None => (about, ""),
    };

    let served = picture_served_as(kind)?;

    let bytes = match encoding.eq_ignore_ascii_case("base64") {
        true => from_base64(data)?,
        // Written out, with anything that means something in a URL escaped. Rare and legal.
        false => percent_decoded(data),
    };

    (!bytes.is_empty() && bytes.len() <= LARGEST_PICTURE).then_some((served, bytes))
}

/// Base64 as the data URL writes it, ignoring the whitespace a folded message leaves in it.
fn from_base64(written: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut bytes = Vec::with_capacity(written.len() / 4 * 3);
    let mut held = 0u32;
    let mut bits = 0u32;

    for character in written.bytes() {
        if character.is_ascii_whitespace() || character == b'=' {
            continue;
        }

        let value = ALPHABET.iter().position(|allowed| *allowed == character)? as u32;
        held = (held << 6) | value;
        bits += 6;

        if bits >= 8 {
            bits -= 8;
            bytes.push((held >> bits) as u8);
        }
    }

    Some(bytes)
}

/// `%41` and the rest, for the data URLs that are not base64.
fn percent_decoded(written: &str) -> Vec<u8> {
    let raw = written.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut at = 0;

    while at < raw.len() {
        if raw[at] == b'%' && at + 2 < raw.len() {
            if let Some(byte) = std::str::from_utf8(&raw[at + 1..at + 3])
                .ok()
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            {
                bytes.push(byte);
                at += 3;
                continue;
            }
        }

        bytes.push(raw[at]);
        at += 1;
    }

    bytes
}
