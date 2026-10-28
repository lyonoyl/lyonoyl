//! What a message may say about how it looks.
//!
//! Its own file because it is a list and a filter over it, read one entry at a time: a message that
//! cannot say how it looks is a message shown as something other than what was written, and inside
//! a frame with no network and no script the things CSS is usually cut for cannot happen.

/// The longest style attribute this build reads.
///
/// A declaration list is a list, and a message can send a very long one. Past this the attribute is
/// dropped whole rather than parsed: nothing a person wrote needs two kilobytes of it.
pub(super) const LONGEST_STYLE: usize = 2048;

/// What a message may say about how it looks.
///
/// Everything here describes text, spacing, colour or the size of a box. What is not here is
/// everything that positions something relative to the window, animates, or names a thing to fetch
/// or run. Mail is written in these properties; the ones left out are not what a newsletter is made
/// of, and each of them is a way to make a frame behave unlike a document.
const STYLE_PROPERTIES: [&str; 50] = [
    "background",
    "background-color",
    // The four a message draws a picture with. Their values reach here already rewritten: an
    // address a message wrote is gone by then and what is left is a name served from this
    // application, or `none`. Without them a background could be named and never drawn, which is
    // how a logo became an empty box that no counter and no notice ever mentioned.
    "background-image",
    "background-position",
    "background-repeat",
    "background-size",
    "border",
    "border-bottom",
    "border-collapse",
    "border-color",
    "border-left",
    "border-radius",
    "border-right",
    "border-spacing",
    "border-style",
    "border-top",
    "border-width",
    "box-sizing",
    "clear",
    "color",
    "display",
    "float",
    "font",
    "font-family",
    "font-size",
    "font-style",
    "font-variant",
    "font-weight",
    "height",
    "letter-spacing",
    "line-height",
    "list-style",
    "list-style-type",
    "margin",
    "margin-bottom",
    "margin-left",
    "margin-right",
    "margin-top",
    "max-height",
    "max-width",
    "min-height",
    "min-width",
    "opacity",
    "overflow-wrap",
    "padding",
    "padding-bottom",
    "padding-left",
    "padding-right",
    "padding-top",
    "text-align",
];

/// The rest of what a message may say about how it looks.
///
/// A second list only because the first is as long as an array literal wants to be.
const MORE_STYLE_PROPERTIES: [&str; 8] = [
    "text-decoration",
    "text-transform",
    "vertical-align",
    "white-space",
    "width",
    "word-break",
    "table-layout",
    "text-indent",
];

/// A presentational attribute's value, when it is one this build passes on.
///
/// The attributes that predate CSS carry a number, a percentage, a colour or a keyword. Anything
/// longer, or carrying anything else, is not one of those.
pub(crate) fn plain_value(value: &str) -> Option<String> {
    let value = value.trim();

    let plain = value.len() <= 32
        && !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "#%. -".contains(character));

    plain.then(|| value.to_owned())
}

/// Whether a value is one this build will pass on.
///
/// Not a CSS parser — a refusal. What it looks for is the shapes that make a declaration something
/// other than a declaration: a thing to fetch, a thing to run, a comment that hides the rest of the
/// list from a reader's eye, an escape that spells one of those in another alphabet.
fn safe_value(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();

    // `url(` is refused unless every one of them is a name this build minted a moment ago —
    // `url(/` and sixty-four hex characters, which is what a picture served from here looks like
    // and what nothing a message can write looks like. The addresses were taken out and replaced
    // before this ran; anything still spelled as an address here was not, and is refused whole.
    if lowered.contains("url(") && !every_url_is_one_of_ours(&lowered) {
        return false;
    }

    !lowered.is_empty()
        && ![
            "expression(",
            "javascript:",
            "image-set(",
            "-moz-binding",
            "behavior",
            "/*",
            "*/",
            "@",
            "\\",
            "<",
            ">",
            "&#",
        ]
        .iter()
        .any(|forbidden| lowered.contains(forbidden))
}

/// What became of one address a stylesheet points at.
pub(crate) enum ThePicture {
    /// Served from here, under this name.
    ServedAs(String),
    /// Not fetched, and the reader may ask: a picture from a host, on a message opened without
    /// asking for them.
    NotAskedFor,
    /// Nothing will make this one appear: not an address this build fetches, or no name to be had.
    NeverArriving,
}

/// The longest stylesheet this build will walk looking for pictures.
///
/// A `<style>` block arrives as one text node and a message chooses how long it is, up to the whole
/// of `LARGEST_BODY`. Past this the block is handed on exactly as it came: nothing is rewritten,
/// nothing is counted, and the frame's own policy is what a picture in it meets — which is what met
/// every stylesheet before any of this existed. A bound that is checked before the walk rather than
/// inside it is the only kind that bounds the walk.
pub(super) const LONGEST_STYLESHEET: usize = 64 * 1024;

/// The properties whose value names a picture this build serves.
///
/// Deliberately two, and deliberately not "every property that can hold a `url(`". `@import`,
/// `@font-face`'s `src`, a `cursor`, a `list-style-image` are all `url(` and none of them is a
/// picture: taking their addresses out would send this application to fetch a stylesheet, a font
/// or a pointer, hold the answer as a picture, and tell the reader a picture did not arrive. What
/// is not named here is left exactly as the message wrote it and reaches the frame under a policy
/// that fetches nothing — which is where it stood before a stylesheet could draw at all.
const PROPERTIES_THAT_DRAW: [&str; 2] = ["background", "background-image"];

fn draws_a_picture(property: &str) -> bool {
    PROPERTIES_THAT_DRAW.contains(&property)
}

/// One of the two places where a message's own writing is put aside unread.
///
/// Neither moves a counter and neither reaches the screen, so from the outside they look exactly
/// like a message that had no pictures — which is the shape of every report that says the notice
/// and the window disagree.
fn said_a_bound_was_crossed(event: &'static str, length: usize) {
    use crate::ports::journal::{Journal, Note, Value, Word};

    let journal = crate::infrastructure::journal::journal();
    if journal.keeps(Note::Detail) {
        journal.write_down(
            Note::Detail,
            event,
            &[
                ("why", Value::Word(Word::TooLongToRead)),
                ("bytes", Value::Bytes(length as u64)),
            ],
        );
    }
}

/// A declaration list, split where a declaration really ends.
///
/// Not on every `;`: one inside `url(...)` belongs to the address — `data:image/gif;base64,…` is a
/// picture a message carries, written with two of them — and splitting there leaves two fragments,
/// neither of which is a declaration and one of which is half a picture.
///
/// Brackets are counted rather than matched, because that is all this needs to know: every byte it
/// looks at is ASCII, so no index it hands out can land inside a character.
fn declarations(style: &str) -> Vec<&str> {
    let bytes = style.as_bytes();
    let mut found = Vec::new();
    let mut from = 0;
    let mut depth = 0usize;

    for at in 0..bytes.len() {
        match bytes[at] {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b';' if depth == 0 => {
                found.push(&style[from..at]);
                from = at + 1;
            }
            _ => {}
        }
    }

    found.push(&style[from..]);
    found
}

/// Where one `url(` token ends and what it names, measured from the byte after `url(`.
///
/// The tokenizer's rule rather than "up to the first `)`". A quoted address runs to its matching
/// quote and the `)` comes after it, so `url("logo(1).png")` is one address and not a truncated
/// one — and truncating it was two faults at once: this application went and fetched
/// `https://host/logo(1`, an address nobody wrote, and what was left of the value re-opened a
/// string that swallowed every rule after it.
///
/// `None` where the shape is not one this build reads: an unclosed quote, a missing `)`, or a
/// backslash. Escapes are refused rather than resolved — CSS spells `url(` as `\75 rl(` and a
/// build that guesses at half of that is a build that reads an address one way and hands the
/// engine another. A token this returns nothing for is left as the message wrote it.
fn one_url_token(after: &str) -> Option<(&str, usize)> {
    let bytes = after.as_bytes();
    let mut at = 0;

    while at < bytes.len() && bytes[at].is_ascii_whitespace() {
        at += 1;
    }

    let (address, mut past) = match bytes.get(at)? {
        quote @ (b'"' | b'\'') => {
            let from = at + 1;
            let close = from + after[from..].find(char::from(*quote))?;
            (&after[from..close], close + 1)
        }
        _ => {
            let close = at + after[at..].find(')')?;
            let address = after[at..close].trim_end();

            // An unquoted address may not hold a bracket — the standard says so, and without this
            // `url(a(b))` would be read as `a(b` and leave the second bracket behind in a
            // declaration this build wrote.
            if address.contains('(') {
                return None;
            }

            (address, close)
        }
    };

    while past < bytes.len() && bytes[past].is_ascii_whitespace() {
        past += 1;
    }

    if bytes.get(past) != Some(&b')') || address.contains('\\') {
        return None;
    }

    Some((address, past + 1))
}

/// Rewrites every picture one declaration's value points at to a name served from here.
///
/// The same road a picture in an `<img>` already travels, and for the same reason: a background
/// fetched from a host tells its owner the message was opened, the hour, and roughly where from.
/// So nothing is fetched by the frame — the address is taken out, this application fetches it if
/// the reader asked, and what goes into the stylesheet is a name that means nothing anywhere else.
///
/// An address that is not to be shown becomes `none`, which is a value every property that takes a
/// picture accepts. The declaration around it — the colour, the position, the repeat — is the
/// message's own and stays.
///
/// **The lowercase copy is made once.** Made inside the loop it was remade for every token, and the
/// walk cost the square of what it was handed: two megabytes of `url(` was minutes of one render
/// thread, there are four of those threads, and a thread that has begun cannot be stopped — four
/// such messages ended every message opened after them. What makes that safe now is the bound on
/// what is walked at all; this keeps the cost linear inside it, which is the difference between a
/// bound that has room to spare and one that is exactly as tight as it has to be.
/// `to_ascii_lowercase` maps only A–Z, one byte to one byte, so an offset found in the copy is the
/// same offset in the original.
fn pictures_a_value_points_at(
    value: &str,
    asked: &mut impl FnMut(&str) -> ThePicture,
) -> (String, u32, u32) {
    let lowered = value.to_ascii_lowercase();
    let mut written = String::with_capacity(value.len());
    let mut from = 0;
    let mut blocked = 0;
    let mut never = 0;

    while let Some(found) = lowered[from..].find("url(") {
        let at = from + found;
        let opens = at + "url(".len();

        written.push_str(&value[from..at]);

        let Some((address, past)) = one_url_token(&value[opens..]) else {
            // Not a shape this build reads. It goes on as the message wrote it, and the policy the
            // frame is served under is what it meets: `default-src 'none'` fetches nothing.
            written.push_str("url(");
            from = opens;
            continue;
        };

        match asked(address.trim()) {
            ThePicture::ServedAs(name) => {
                written.push_str("url(/");
                written.push_str(&name);
                written.push(')');
            }
            ThePicture::NotAskedFor => {
                blocked += 1;
                written.push_str("none");
            }
            ThePicture::NeverArriving => {
                never += 1;
                written.push_str("none");
            }
        }

        from = opens + past;
    }

    written.push_str(&value[from..]);

    (written, blocked, never)
}

/// Every picture a whole stylesheet draws, rewritten to names served from here.
///
/// A stylesheet is not a declaration list, so the declarations are found before anything is read:
/// what sits between a `{` and the next brace is a declaration list, and everything else — the
/// selectors, `@import`, the head of an `@media` — is not, and is passed on untouched. Nesting
/// falls out of that on its own, because the inner `{` ends the outer run.
pub(crate) fn pictures_pointed_at(
    css: &str,
    mut asked: impl FnMut(&str) -> ThePicture,
) -> (String, u32, u32) {
    if css.len() > LONGEST_STYLESHEET {
        // The same silence, one size up: the block goes on unrewritten, every address in it meets
        // the policy instead of this build, and no counter moves. A stylesheet this long is
        // ordinary in the mail this matters for.
        said_a_bound_was_crossed("stylesheet.tooLong", css.len());
        return (css.to_owned(), 0, 0);
    }

    let mut written = String::with_capacity(css.len());
    let mut blocked = 0;
    let mut never = 0;
    let mut from = 0;
    let mut in_a_list = false;

    // Both braces are ASCII, so an index at one is a character boundary and so is the byte after.
    for (at, byte) in css.bytes().enumerate() {
        if byte != b'{' && byte != b'}' {
            continue;
        }

        let run = &css[from..at];
        match in_a_list {
            true => {
                let (served, went, gone) = declarations_that_draw(run, &mut asked);
                written.push_str(&served);
                blocked += went;
                never += gone;
            }
            false => written.push_str(run),
        }

        written.push(char::from(byte));
        from = at + 1;
        in_a_list = byte == b'{';
    }

    let run = &css[from..];
    match in_a_list {
        true => {
            let (served, went, gone) = declarations_that_draw(run, &mut asked);
            written.push_str(&served);
            blocked += went;
            never += gone;
        }
        false => written.push_str(run),
    }

    (written, blocked, never)
}

/// One declaration list, with the pictures its drawing properties point at served from here.
///
/// Everything else is put back exactly as it came. A stylesheet is the message's own document and
/// this is not a filter over it — the filter is the policy it is served under.
fn declarations_that_draw(
    run: &str,
    asked: &mut impl FnMut(&str) -> ThePicture,
) -> (String, u32, u32) {
    let mut written = String::with_capacity(run.len());
    let mut blocked = 0;
    let mut never = 0;

    for (number, declaration) in declarations(run).into_iter().enumerate() {
        if number > 0 {
            written.push(';');
        }

        let drawing = declaration
            .split_once(':')
            .filter(|(property, _)| draws_a_picture(&property.trim().to_ascii_lowercase()));

        let Some((property, value)) = drawing else {
            written.push_str(declaration);
            continue;
        };

        let (served, went, gone) = pictures_a_value_points_at(value, asked);

        written.push_str(property);
        written.push(':');
        written.push_str(&served);
        blocked += went;
        never += gone;
    }

    (written, blocked, never)
}

/// Whether every `url(` in this value names a picture served from here.
///
/// The one shape allowed: `url(/` then the name, then `)`. A name is thirty-two bytes of
/// randomness written as hex, minted for one render, so it cannot be spelled by a message and
/// cannot outlive the message it belongs to. No quotes, no spaces, no scheme, nothing else.
fn every_url_is_one_of_ours(lowered: &str) -> bool {
    let mut rest = lowered;

    while let Some(at) = rest.find("url(") {
        let after = &rest[at + "url(".len()..];
        let Some(close) = after.find(')') else {
            return false;
        };

        let inside = &after[..close];
        let Some(name) = inside.strip_prefix('/') else {
            return false;
        };

        if name.len() != NAME_HEX || !name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return false;
        }

        rest = &after[close..];
    }

    true
}

/// How long a name written as hex is. Thirty-two bytes, two characters each.
const NAME_HEX: usize = 64;

/// Said out loud in a development build when a declaration that was going to draw a picture goes.
///
/// A picture a drawing property points at is counted like any other and the reader is told the
/// number. What this is for is the rest: a declaration dropped because its property is not one this
/// build passes on, or because its value carries something else that is refused. Such a picture
/// never becomes a picture as far as the rest of this application is concerned, so no counter
/// moves, no notice appears, and **Show pictures** cannot bring it back — a reader sees an empty
/// box where the message meant a logo and has nothing to press.
///
/// The declaration is not printed. It is written by whoever sent the message and can carry an
/// address, which section 6 keeps out of a log; what is said is that one went, which is the fact
/// that was missing.
fn a_picture_drawn_in_a_style_is_dropped(value: &str, property: &str) {
    if !value.to_ascii_lowercase().contains("url(") {
        return;
    }

    use crate::ports::journal::{Journal, Note, Value, Word};

    // The last of the quiet ways a picture goes. The declaration is dropped whole — its property is
    // not one this build passes on, or its value carries something else that is refused — and no
    // counter moves, so the reader is offered no number and no press. Which of the two it was is
    // the difference between "this build draws nothing with that property" and "the value had a
    // comment in it", and that is not a thing to work out by re-reading a message.
    let journal = crate::infrastructure::journal::journal();
    if journal.keeps(Note::Detail) {
        let known =
            STYLE_PROPERTIES.contains(&property) || MORE_STYLE_PROPERTIES.contains(&property);

        journal.write_down(
            Note::Detail,
            "picture.dropped",
            &[
                ("where", Value::Word(Word::InAStyleAttribute)),
                (
                    "why",
                    Value::Word(match known {
                        true => Word::NotAKindShownHere,
                        false => Word::NotFetchable,
                    }),
                ),
                ("knownProperty", Value::Yes(known)),
            ],
        );
    }
}

/// The declarations of a style attribute this build keeps, or nothing when it keeps none.
///
/// Rebuilt rather than trimmed: what leaves here is written by this function out of properties it
/// recognises and values it has looked at, so whatever shape the message wrote it in is gone.
pub(crate) fn filtered_style(
    style: &str,
    mut asked: impl FnMut(&str) -> ThePicture,
) -> (Option<String>, u32, u32) {
    // Before the walk and not after it. This is the bound on how much of a message's writing is
    // read at all, and a bound checked once the reading is done bounds nothing.
    if style.len() > LONGEST_STYLE {
        // Said out loud, because crossing it is otherwise the quietest way a picture disappears:
        // the whole attribute goes, nothing is counted, and the reader is told nothing at all.
        said_a_bound_was_crossed("style.tooLong", style.len());
        return (None, 0, 0);
    }

    let mut kept: Vec<String> = Vec::new();
    let mut blocked = 0;
    let mut never = 0;

    for declaration in declarations(style) {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let property = property.trim().to_ascii_lowercase();
        let value = value.trim();

        let known = STYLE_PROPERTIES.contains(&property.as_str())
            || MORE_STYLE_PROPERTIES.contains(&property.as_str());

        if !known {
            a_picture_drawn_in_a_style_is_dropped(value, &property);
            continue;
        }

        // Only a property that draws one, and the rewrite happens here rather than over the whole
        // attribute for a reason a reader is told about: what is counted has to be what survives.
        // Counted before this loop, a picture in a declaration the loop then dropped was a number
        // in front of somebody with nothing behind it and a press that could not help.
        let (value, went, gone) = match draws_a_picture(&property) {
            true => pictures_a_value_points_at(value, &mut asked),
            false => (value.to_owned(), 0, 0),
        };

        if !safe_value(&value) {
            a_picture_drawn_in_a_style_is_dropped(&value, &property);
            continue;
        }

        blocked += went;
        never += gone;
        kept.push(format!("{property}:{value}"));
    }

    ((!kept.is_empty()).then(|| kept.join(";")), blocked, never)
}
