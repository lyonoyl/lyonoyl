//! The name a file from a message may be offered under.
//!
//! A filename in a message is written by whoever sent it, which is to say by anybody. It reaches a
//! system's save panel, and a save panel that is handed `../../.ssh/authorized_keys` is a save
//! panel doing what it was told. So the name is not used — a name is *made* from it, and the rules
//! below are what makes one.
//!
//! This never produces a path and never can: everything that could separate one directory from
//! another is gone before it returns. Where the file lands is chosen by the person in the system's
//! own dialog, and this is only what that dialog starts with.

/// The longest name offered. Filesystems differ; this is short enough for all of them and long
/// enough that nothing a person would recognise is lost.
pub const LONGEST: usize = 120;

/// What an unnamed part is offered as.
pub const UNNAMED: &str = "attachment";

/// A name safe to hand to a save panel, made from whatever the message said.
///
/// What is removed, and why each one:
///
/// - **Separators** — `/`, `\`, and the colon a Mac once used — because a name holding one is a
///   name that reaches out of wherever the dialog started.
/// - **`..`** — the same reason, one directory further out.
/// - **Control characters**, including the null a C string ends at: a name that ends early in one
///   layer and not in another is two names, and the second one is the attacker's.
/// - **The characters that reorder text**, which are not control characters and are the whole of a
///   known attack: `holiday<U+202E>gnp.exe` is drawn as `holidayexe.png` and runs as an
///   executable. Rust's own `is_control` does not cover them — they are formatting, not control —
///   so they are named here one by one.
/// - **A leading dot**, which hides the file on two of the three systems this ships on. Somebody
///   who saved a file and cannot find it has lost it. A space in front of it, or between two of
///   them, goes with it.
/// - **Trailing dots and spaces**, which Windows silently strips — so `report.txt.` and
///   `report.txt` are one file there and two here.
///
/// What is deliberately *kept*: everything else, including alphabets this build cannot read.
/// A name in Chinese is a name, and replacing it with `attachment` would be this application
/// deciding which languages deserve filenames.
pub fn safe_name(raw: &str) -> String {
    let stripped: String = raw
        .chars()
        .filter(|character| !character.is_control() && !reorders_text(*character))
        // The separators, and the six more Windows refuses outright. A name holding any of them
        // is not a name that saves badly there — it is a save that fails, reported to the person
        // as a failure with nothing in it they can act on. `:` is both: a separator on one system
        // and the alternate-stream marker on another.
        .map(|character| match character {
            '/' | '\\' | ':' | '?' | '*' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect();

    // The leading dot goes before `..` is dealt with, and the order is the point: done the other
    // way, `...hidden` becomes `_.hidden` and stays hidden behind an underscore nobody asked for.
    let visible = stripped.trim_start_matches(at_the_edge);

    // After the separators are gone, `..` is no longer a way out — but it is still a name that
    // reads as one, and a name that reads as an escape is one somebody has to think about.
    let no_climbing = visible.replace("..", "_");

    let trimmed = no_climbing.trim_end_matches(at_the_edge);

    // Counted in characters rather than bytes, and cut on a character boundary: a name cut mid-way
    // through a letter is a name that is not text any more.
    //
    // Trimmed again after the cut, because the cut can land on a dot or a space and put back
    // exactly what the trim above took out — and a name ending in either is the one Windows
    // silently shortens, which is two names for one file.
    let cut: String = trimmed.chars().take(LONGEST).collect();
    let offered = cut.trim_end_matches(at_the_edge);

    // Nothing left but the marks put there by this function. `/` becoming a file called `_` is
    // worse than one called `attachment`: it looks like a name somebody chose.
    //
    // A name Windows keeps for a device goes the same way, and for a sharper reason: creating
    // `COM1.pdf` there does not create a file, it opens a serial port. What the person sees is a
    // save that failed for no reason they can act on.
    //
    // Both are asked of the name as it is offered, after the cut, because the cut can make either
    // out of a name that was neither: `CON` and a long run of spaces in front of the rest is an
    // ordinary name until the cut takes the rest and the trim takes the spaces.
    if reserved_on_windows(offered)
        || offered
            .chars()
            .all(|character| character == '_' || character.is_whitespace())
    {
        return UNNAMED.to_owned();
    }

    offered.to_owned()
}

/// The longest stem a message's file name is given, in characters.
///
/// Filesystems count bytes — 255 on ext4, APFS and NTFS — and a subject can be entirely
/// three-byte characters. Counting characters and staying well under the byte limit costs a few
/// characters of a long subject and never produces a name the filesystem refuses.
pub const LONGEST_MESSAGE_STEM: usize = 60;

/// What a message with nothing to name it by is called as a file.
pub const UNNAMED_MESSAGE: &str = "message";

/// The name a message goes by as a file, built from its subject: saved, attached to another
/// message, or carried inside one with no name of its own.
///
/// A subject is written by whoever sent the message, so it is treated as what it is. Three
/// separate things are being kept away here, and all three are real on at least one platform:
///
/// - a separator or a `..` would make the suggestion point somewhere other than where the dialog
///   is looking;
/// - Windows refuses `< > : " | ? *`, refuses names ending in a dot or a space by silently
///   trimming them, and reserves `CON`, `NUL`, `COM1` and their friends whatever extension
///   follows;
/// - a control character passes through most filesystems and then does whatever it likes in a
///   terminal or a file manager that prints it.
///
/// What survives is what a person would recognise as their subject line.
pub fn message_file_name(subject: Option<&str>) -> String {
    let subject = subject.unwrap_or_default();

    let kept: String = subject
        .chars()
        .map(|character| match character {
            // Separators of every platform, the characters Windows refuses outright, and
            // everything a terminal would act on rather than print.
            '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*' => ' ',
            character if character.is_control() => ' ',
            // The characters that change the direction text is read in, which `is_control` does
            // not cover: it is the Cc category, and these are Cf. U+202E before `gnp.exe` renders
            // as `exe.png` — the name a person reads is not the name the system runs, and the
            // threat model names this one by number (§5). Removed rather than replaced with a
            // space, because they occupy no width: a space where one stood is a gap that was never
            // in what somebody wrote.
            character if reorders_text(character) => '\0',
            character => character,
        })
        .filter(|character| *character != '\0')
        .collect();

    // Runs of space become one, so a subject full of removed characters does not become a name
    // made of gaps.
    let mut stem = kept.split_whitespace().collect::<Vec<_>>().join(" ");

    stem.truncate(
        stem.char_indices()
            .nth(LONGEST_MESSAGE_STEM)
            .map_or(stem.len(), |(at, _)| at),
    );

    // Trailing dots and spaces are removed by Windows itself, which turns "v1.0." into "v1.0" and
    // a comparison of what was asked for against what exists into a surprise. Leading dots hide
    // the file on the other two.
    let stem = stem.trim_matches(|character| character == '.' || character == ' ');

    if stem.is_empty() || reserved_on_windows(stem) {
        return format!("{UNNAMED_MESSAGE}.eml");
    }

    format!("{stem}.eml")
}

/// What a name may neither start nor end with: a dot, and a space of any kind.
///
/// One set, taken off in one pass, because each hides the other. A pass that takes dots and stops
/// at the first space, followed by one that takes spaces, uncovers a dot and leaves it there:
/// `. .zshrc` would be offered as `.zshrc`, hidden after all, and `report.txt.` followed by a
/// no-break space as a name ending in the dot Windows strips.
fn at_the_edge(character: char) -> bool {
    character == '.' || character.is_whitespace()
}

/// Whether a character changes the direction the rest is read in.
///
/// The bidirectional formatting characters of Unicode — every character carrying the standard's
/// `Bidi_Control` property, and no others. Listed rather than asked of a library, because this
/// crate depends on `thiserror` and nothing else — and because the list is short, fixed by the
/// standard, and worth being able to read here.
///
/// Public because there is one such list in this product and this is it: the address a link is
/// offered under is shown elsewhere, and asks this same question.
pub fn reorders_text(character: char) -> bool {
    matches!(
        character,
        // The Arabic letter mark, which is the one the obvious list leaves out.
        '\u{061C}'
        // Left-to-right and right-to-left marks.
        | '\u{200E}' | '\u{200F}'
        // Embeddings and overrides, and the pop that ends them.
        | '\u{202A}'..='\u{202E}'
        // Isolates, and the pop that ends them.
        | '\u{2066}'..='\u{2069}'
    )
}

/// Whether Windows keeps this name for a device.
///
/// The rule is the stem before the first dot, case-insensitively: `CON`, `con.txt` and
/// `CON.pdf.txt` all reach the same reserved name, and creating one fails or opens a device
/// instead of a file.
///
/// Trailing spaces come off that stem first, because Windows takes them off too: `CON .txt` is the
/// console there and a comparison that kept the space would say it is an ordinary name. And the
/// superscript digits count as digits, because they do in the device names Windows itself
/// publishes — `COM¹` is the first serial port written another way.
///
/// Here rather than beside either caller, because both name a file a person is about to be handed
/// in a save panel and there is one answer to that question.
pub fn reserved_on_windows(stem: &str) -> bool {
    const RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];

    let first = stem
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();

    let port = |number: &str| {
        matches!(
            number,
            "1" | "2"
                | "3"
                | "4"
                | "5"
                | "6"
                | "7"
                | "8"
                | "9"
                | "\u{00B9}"
                | "\u{00B2}"
                | "\u{00B3}"
        )
    };

    RESERVED.contains(&first.as_str())
        || matches!(first.strip_prefix("COM").or_else(|| first.strip_prefix("LPT")),
            Some(number) if port(number))
}

#[cfg(test)]
mod tests;
