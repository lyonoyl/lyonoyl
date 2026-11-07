//! A message passed on in its own form.
//!
//! What goes beneath what the person writes when a message is passed on with its look: its markup,
//! read into a tree and written back out as one document; what it says as text, for whoever reads no
//! markup; and the pictures its markup draws by name. Cut down from the message as it is passed on,
//! behind the guards a body is shown behind, because reading the markup is parsing what a stranger
//! wrote.
//!
//! Joining it to what the person writes reads nothing. The document is kept as the two pieces it
//! was written from, which meet where its drawing begins, and what the person wrote goes between
//! them, escaped, when the message is written for sending. A place found by looking through the
//! markup for a tag would be a second reader of what the parser read, and the two disagree wherever
//! a stranger makes them.

use std::collections::HashSet;

use dom_query::{Document, NodeData, NodeRef};
use mail_builder::headers::content_type::ContentType;
use mail_builder::mime::{BodyPart, MimePart};
use mail_parser::{Message, MimeHeaders, PartType};

use lyonoyl_domain::outgoing::{Original, OriginalPicture, HEAVIEST_MESSAGE};

use super::nesting::{elements_beyond, nesting_beyond, DEEPEST_NESTING, MOST_ELEMENTS};
use super::styling::{filtered_style, pictures_pointed_at, ThePicture};
use super::{picture_served_as, LARGEST_BODY};

/// The longest name a picture may be drawn by and still be passed on with it.
///
/// The name is written back into a header of the message that carries it, and a header line holds
/// 998 characters with its name in front.
const LONGEST_PICTURE_NAME: usize = 250;

/// The message cut down to what draws it, with `text` as what it says, and where in the message
/// each picture it takes along sits, for the files passed on beside it to leave out.
///
/// Nothing for a message with no markup, one whose markup has no body to draw in — a page of
/// frames — one past what this build reads into a tree, or one heavier than a message may be on its
/// own. Each of those is passed on as text.
///
/// A picture goes inside when its markup draws it, by the name after `cid:` in a picture's source
/// or in a style that draws it, once, if it is of a kind this build draws and under a name that can
/// be written back into a header as it was. Any other part stays a file beside the message, as it
/// is when a message is passed on as text, and that includes an attachment carrying such a name.
pub(super) fn cut_down(parsed: &Message<'_>, text: &str) -> Option<(Original, HashSet<u32>)> {
    let markup =
        parsed
            .html_body
            .iter()
            .find_map(|&at| match parsed.part(at).map(|part| &part.body) {
                Some(PartType::Html(html)) => Some(html.as_ref()),
                _ => None,
            })?;

    // The bounds a body is drawn within, and for the same reason: reading a tree costs what the
    // tree is, and its size, its count of elements and its depth are whoever wrote it's to choose.
    if markup.len() > LARGEST_BODY
        || elements_beyond(markup, MOST_ELEMENTS)
        || nesting_beyond(markup, DEEPEST_NESTING)
    {
        return None;
    }

    let (opening, drawing, drawn) = as_one_document(markup)?;

    let mut inside = HashSet::new();
    let mut taken: std::collections::HashMap<&str, (usize, u32)> = std::collections::HashMap::new();
    let mut pictures: Vec<OriginalPicture> = Vec::new();
    for (at, part) in parsed.parts.iter().enumerate() {
        let bytes: &[u8] = match &part.body {
            PartType::Binary(bytes) | PartType::InlineBinary(bytes) => bytes,
            _ => continue,
        };
        let Some(named) = part
            .content_id()
            .map(|named| named.trim_matches(|c| c == '<' || c == '>'))
            .filter(|named| writable(named) && drawn.contains(*named))
        else {
            continue;
        };
        let Some(content_type) = part
            .content_type()
            .filter(|kind| kind.ctype().eq_ignore_ascii_case("image"))
            .and_then(|kind| kind.subtype())
            .and_then(picture_served_as)
        else {
            continue;
        };
        let picture = OriginalPicture {
            named: named.to_owned(),
            content_type: content_type.to_owned(),
            bytes: bytes.to_vec(),
        };
        // The last part under a name is the one the reader draws, so it is the one that goes.
        match taken.get(named).copied() {
            Some((position, earlier)) => {
                inside.remove(&earlier);
                pictures[position] = picture;
                taken.insert(named, (position, at as u32));
            }
            None => {
                taken.insert(named, (pictures.len(), at as u32));
                pictures.push(picture);
            }
        }
        inside.insert(at as u32);
    }

    let original = Original {
        opening,
        drawing,
        text: text.to_owned(),
        pictures,
    };

    (original.weighs() <= HEAVIEST_MESSAGE).then_some((original, inside))
}

/// Whether a picture can go inside the message that passes its original on: one of a kind this build
/// draws, labelled as this build labels that kind, under a name that can be written into a header
/// as it is.
fn goes_inside(picture: &OriginalPicture) -> bool {
    writable(&picture.named)
        && picture_served_as(&picture.content_type) == Some(picture.content_type.as_str())
}

/// Whether a picture's name can be written back into a header as it was: letters, digits and marks
/// a header may hold, without the brackets that close a name or a space that ends one, and short
/// enough for one line.
fn writable(named: &str) -> bool {
    !named.is_empty()
        && named.len() <= LONGEST_PICTURE_NAME
        && named
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'<' && byte != b'>')
}

/// The markup read into a tree and written back out as one document, in two pieces that meet just
/// inside its body, where the drawing begins, and the names its pictures are drawn by.
///
/// Written in pieces this build puts together, so the place is known rather than looked for: the
/// document's declaration as the parser read it, its root and its body as they open with nothing
/// inside them, and what its head and its body hold as the parser writes them. A declaration of the
/// characters it is written in goes, and one saying what it is written in now takes its place.
///
/// The declaration is kept as it was, because whether a document declares itself, and as what,
/// decides how an engine lays it out: a picture in a table that sits flush in one mode has a gap
/// under it in another, and a message cut into pictures is drawn with lines through it.
///
/// A picture is drawn by the name after `cid:` in its source, in a style attribute or in a
/// stylesheet, read by the readers the sanitizer reads them with, so a picture goes inside the
/// message where the reader would draw it.
fn as_one_document(markup: &str) -> Option<(String, String, HashSet<String>)> {
    let document = Document::from(markup);
    let tree = document.root();

    let root = tree
        .element_children()
        .into_iter()
        .find(|node| named(node, "html"))?;
    let parts = root.element_children();
    let head = parts.iter().find(|node| named(node, "head"))?;
    let body = parts.iter().find(|node| named(node, "body"))?;

    for meta in head.element_children() {
        let declares_characters = named(&meta, "meta")
            && (meta.has_attr("charset")
                || meta
                    .attr("http-equiv")
                    .is_some_and(|value| value.eq_ignore_ascii_case("content-type")));
        if declares_characters {
            meta.remove_from_parent();
        }
    }

    let declared = tree
        .children()
        .iter()
        .find_map(|node| {
            node.query(|node| match &node.data {
                NodeData::Doctype {
                    name,
                    public_id,
                    system_id,
                } => Some(declaration(name, public_id, system_id)),
                _ => None,
            })
            .flatten()
        })
        .unwrap_or_default();

    let mut drawn = HashSet::new();
    let mut named = |address: &str| {
        if let Some(carried) = address.strip_prefix("cid:") {
            drawn.insert(carried.trim_matches(['<', '>']).to_owned());
        }
        ThePicture::NeverArriving
    };
    for picture in document.select("img[src]").nodes() {
        if let Some(source) = picture.attr("src") {
            named(&source);
        }
    }
    for styled in document.select("[style]").nodes() {
        if let Some(style) = styled.attr("style") {
            let _ = filtered_style(&style, &mut named);
        }
    }
    for sheet in document.select("style").nodes() {
        let _ = pictures_pointed_at(&sheet.text(), &mut named);
    }

    let in_head = head.inner_html().to_string();
    let in_body = body.inner_html().to_string();

    body.remove_children();
    let body_opens = body.html().strip_suffix("</body>")?.to_owned();
    root.remove_children();
    let root_opens = root.html().strip_suffix("</html>")?.to_owned();

    let opening =
        format!("{declared}{root_opens}<head><meta charset=\"utf-8\">{in_head}</head>{body_opens}");

    Some((opening, format!("{in_body}</body></html>"), drawn))
}

/// Whether a node is the element of that name.
fn named(node: &NodeRef<'_>, name: &str) -> bool {
    node.node_name()
        .is_some_and(|written| written.eq_ignore_ascii_case(name))
}

/// A document's declaration, written back as the parser read it: its name, and the public and
/// system identifiers it gave.
///
/// An identifier ends at the quotation mark it opened with, so it can hold only the other kind, and
/// is written between the kind it does not hold.
fn declaration(name: &str, public: &str, system: &str) -> String {
    let quoted = |identifier: &str| match identifier.contains('"') {
        true => format!("'{identifier}'"),
        false => format!("\"{identifier}\""),
    };

    match (public.is_empty(), system.is_empty()) {
        (true, true) => format!("<!DOCTYPE {name}>"),
        (false, true) => format!("<!DOCTYPE {name} PUBLIC {}>", quoted(public)),
        (false, false) => format!(
            "<!DOCTYPE {name} PUBLIC {} {}>",
            quoted(public),
            quoted(system)
        ),
        (true, false) => format!("<!DOCTYPE {name} SYSTEM {}>", quoted(system)),
    }
}

/// The markup of a message that passes one on in its own form: what its writer wrote, drawn first,
/// and the message passed on beneath it.
///
/// What was written is text, and goes as text: every character that means something to markup is
/// written as the character itself, its lines are parted by breaks, spaces are kept as typed, and its
/// direction is the reader's to find from its words. It carries its own colours, because it sits
/// inside the original's body, whose colours were chosen for what that body draws.
pub fn beneath(written: &str, original: &Original) -> String {
    let mut lines = String::with_capacity(written.len());
    for (at, line) in written.split('\n').enumerate() {
        if at > 0 {
            lines.push_str("<br>");
        }
        super::escape_into(&mut lines, line.trim_end_matches('\r'));
    }

    format!(
        "{}<div dir=\"auto\" style=\"white-space: pre-wrap; color: #000000; background-color: #ffffff\">{lines}</div>{}",
        original.opening, original.drawing
    )
}

/// What a message passing one on in its own form goes as: its text, and its markup with the
/// pictures that markup draws beside it, for whoever reads the one rather than the other.
///
/// The one writer of it, for the message sent and for the one the composer draws. Each picture is
/// asked again whether it may go inside: what is written comes from the drafts or the outbox, and a
/// label or a name read back from a damaged row is the one way a second header could be written
/// into a message. Cut by random boundaries, as a message sent from here is.
pub fn written_with<'x>(
    text: impl Into<BodyPart<'x>>,
    markup: impl Into<BodyPart<'x>>,
    pictures: &'x [OriginalPicture],
) -> MimePart<'x> {
    let boundary = || uuid::Uuid::new_v4().simple().to_string();

    let mut drawn = vec![MimePart::new("text/html", markup)];
    drawn.extend(
        pictures
            .iter()
            .filter(|picture| goes_inside(picture))
            .map(|picture| {
                MimePart::new(picture.content_type.as_str(), picture.bytes.as_slice())
                    .inline()
                    .cid(picture.named.as_str())
            }),
    );

    MimePart::new(
        ContentType::new("multipart/alternative").attribute("boundary", boundary()),
        vec![
            MimePart::new("text/plain", text),
            MimePart::new(
                ContentType::new("multipart/related").attribute("boundary", boundary()),
                drawn,
            ),
        ],
    )
}

/// The original as a message of its own, as a reader that shows a message would be handed one.
///
/// What the composer shows beneath what is being written, drawn the way the reader draws any
/// message, through the same sanitizer and the same frame.
pub fn as_message(original: &Original) -> Vec<u8> {
    let mut written = Vec::new();
    // Into memory, which does not fail.
    let _ = written_with(
        original.text.as_str(),
        format!("{}{}", original.opening, original.drawing),
        &original.pictures,
    )
    .write_part(&mut written);
    written
}

#[cfg(test)]
#[path = "original_tests.rs"]
mod tests;
