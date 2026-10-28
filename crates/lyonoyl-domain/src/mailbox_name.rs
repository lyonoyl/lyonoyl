//! Turning the name a server puts on the wire into the name a person reads.
//!
//! IMAP has no identifier for a mailbox. The protocol addresses one by its name and by nothing
//! else — `SELECT "INBOX/Work"` — so the string on the wire is not a label, it is the address.
//! RFC 8474 added `MAILBOXID`, and almost nothing implements it; even where it exists, `SELECT`
//! still takes the name.
//!
//! What the wire carries is not text anybody wrote. RFC 3501 §5.1.3 encodes everything outside
//! printable US-ASCII as modified BASE64 of UTF-16BE, between `&` and `-`, with `,` where BASE64
//! would use `/`. So the Russian for "Sent" arrives as `&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-` and,
//! shown as it arrived, reads as line noise.
//!
//! This decodes it **for showing**. The wire name is kept exactly as it came and is what every
//! command is given: decoding what is sent would address a mailbox nobody has.
//!
//! RFC 9051 (IMAP4rev2) and RFC 6855 (UTF8=ACCEPT) let a server send UTF-8 directly. Such a name
//! has no `&` in it and passes through untouched, which is what makes one rule serve both.

/// The name as a person should read it.
///
/// Anything this cannot make sense of comes back exactly as it arrived: a folder shown under an
/// odd name is a folder somebody can still open, and one this refused would be a folder that
/// disappeared.
pub fn readable(wire: &str) -> String {
    let mut read = String::with_capacity(wire.len());
    let mut rest = wire;

    while let Some(at) = rest.find('&') {
        read.push_str(&rest[..at]);
        rest = &rest[at + 1..];

        let Some(end) = rest.find('-') else {
            // An opening with no close. Not a name this understands.
            read.push('&');
            read.push_str(rest);
            return read;
        };

        let encoded = &rest[..end];
        rest = &rest[end + 1..];

        // `&-` is how the wire spells an ampersand.
        if encoded.is_empty() {
            read.push('&');
            continue;
        }

        match decoded(encoded) {
            Some(text) => read.push_str(&text),
            None => {
                read.push('&');
                read.push_str(encoded);
                read.push('-');
            }
        }
    }

    read.push_str(rest);
    read
}

/// One `&…-` run, as the characters it stands for.
fn decoded(encoded: &str) -> Option<String> {
    let mut bits: u32 = 0;
    let mut held = 0u32;
    let mut units: Vec<u16> = Vec::new();

    for character in encoded.chars() {
        let value = sextet(character)?;
        bits = (bits << 6) | u32::from(value);
        held += 6;

        if held >= 16 {
            held -= 16;
            units.push(((bits >> held) & 0xFFFF) as u16);
        }
    }

    // What is left over must be padding, and padding is zero. Anything else is a name that was
    // cut short or was never this encoding at all.
    if held >= 6 || (bits & ((1 << held) - 1)) != 0 {
        return None;
    }

    String::from_utf16(&units).ok()
}

/// The value of one character of modified BASE64, where `,` stands where `/` would.
fn sextet(character: char) -> Option<u8> {
    match character {
        'A'..='Z' => Some(character as u8 - b'A'),
        'a'..='z' => Some(character as u8 - b'a' + 26),
        '0'..='9' => Some(character as u8 - b'0' + 52),
        '+' => Some(62),
        ',' => Some(63),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_that_is_already_readable_is_left_alone() {
        assert_eq!(readable("INBOX"), "INBOX");
        assert_eq!(readable("INBOX/Work"), "INBOX/Work");
        assert_eq!(readable(""), "");
    }

    #[test]
    fn the_examples_the_standard_itself_gives() {
        // RFC 3501 §5.1.3, with the cast in place of the name the standard gives.
        assert_eq!(
            readable("~zeta/mail/&U,BTFw-/&ZeVnLIqe-"),
            "~zeta/mail/台北/日本語"
        );
    }

    #[test]
    fn a_russian_folder_reads_as_russian() {
        // What a provider in Russia calls its sent folder, and what it looks like on the wire.
        assert_eq!(
            readable("&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-"),
            "Отправленные"
        );
    }

    #[test]
    fn an_ampersand_is_spelled_out_rather_than_encoded() {
        assert_eq!(readable("Black &- White"), "Black & White");
        assert_eq!(readable("&-"), "&");
    }

    #[test]
    fn a_name_a_newer_server_sends_as_it_is_passes_through() {
        // RFC 9051 and RFC 6855: no `&`, nothing to decode, nothing to get wrong.
        assert_eq!(readable("Ζήτα"), "Ζήτα");
    }

    #[test]
    fn a_run_that_makes_no_sense_is_shown_as_it_arrived_rather_than_lost() {
        // A folder under an odd name can still be opened. One this refused would be a folder that
        // vanished from the sidebar.
        assert_eq!(readable("&not base64-"), "&not base64-");
        assert_eq!(readable("&AAA"), "&AAA");
    }

    #[test]
    fn several_runs_in_one_name() {
        // Two runs and a delimiter between them: `&A5Y-` is the Greek capital zeta and `&A7Y-` the
        // small one, worked out from the code points rather than guessed.
        assert_eq!(readable("&A5Y-/&A7Y-"), "Ζ/ζ");
    }
}
