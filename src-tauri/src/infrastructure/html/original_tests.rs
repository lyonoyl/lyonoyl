use mail_parser::MessageParser;

use super::*;

/// A message from its header lines and its body lines, joined as mail joins them.
fn message(headers: &[&str], body: &[&str]) -> Vec<u8> {
    let mut lines = headers.to_vec();
    lines.push("");
    lines.extend_from_slice(body);
    lines.join("\r\n").into_bytes()
}

/// A message of markup alone, written by the cast.
fn markup_alone(markup: &str) -> Vec<u8> {
    message(
        &[
            "From: zeta@example.com",
            "To: kappa@example.net",
            "Subject: Its own form",
            "MIME-Version: 1.0",
            "Content-Type: text/html; charset=utf-8",
        ],
        &[markup],
    )
}

/// A message of markup, its text, a picture the markup draws, and an invoice beside them.
fn with_a_picture(picture_type: &str, named: &str) -> Vec<u8> {
    message(
        &[
            "From: zeta@example.com",
            "To: kappa@example.net",
            "Subject: The chart and the invoice",
            "MIME-Version: 1.0",
            "Content-Type: multipart/mixed; boundary=\"outer\"",
        ],
        &[
            "--outer",
            "Content-Type: multipart/alternative; boundary=\"both\"",
            "",
            "--both",
            "Content-Type: text/plain; charset=utf-8",
            "",
            "The chart is in the message.",
            "--both",
            "Content-Type: multipart/related; boundary=\"inner\"",
            "",
            "--inner",
            "Content-Type: text/html; charset=utf-8",
            "",
            &format!("<p>The chart:</p><img src=\"cid:{named}\">"),
            "--inner",
            &format!("Content-Type: {picture_type}"),
            &format!("Content-ID: <{named}>"),
            "Content-Disposition: inline; filename=\"chart.png\"",
            "Content-Transfer-Encoding: base64",
            "",
            "iVBORw0KGgo=",
            "--inner--",
            "--both--",
            "--outer",
            "Content-Type: application/pdf; name=\"invoice.pdf\"",
            "Content-Disposition: attachment; filename=\"invoice.pdf\"",
            "Content-Transfer-Encoding: base64",
            "",
            "JVBERi0xLjQK",
            "--outer--",
        ],
    )
}

fn cut(raw: &[u8]) -> Option<(Original, HashSet<u32>)> {
    cut_down(
        &MessageParser::default().parse(raw).expect("a message"),
        "What it says.",
    )
}

/// What the markup draws, from where its drawing begins to where the body closes.
fn drawn(original: &Original) -> &str {
    original
        .drawing
        .strip_suffix("</body></html>")
        .expect("the document ends as it was written")
}

#[test]
fn its_markup_its_text_and_the_picture_it_draws_go_together() {
    let (original, inside) =
        cut(&with_a_picture("image/png", "chart@example.com")).expect("markup to draw");

    assert_eq!(
        drawn(&original),
        "<p>The chart:</p><img src=\"cid:chart@example.com\">"
    );
    assert_eq!(original.text, "What it says.");
    assert_eq!(
        original.pictures,
        vec![OriginalPicture {
            named: "chart@example.com".to_owned(),
            content_type: "image/png".to_owned(),
            bytes: b"\x89PNG\r\n\x1a\n".to_vec(),
        }]
    );
    assert_eq!(inside.len(), 1, "the picture is the one part taken inside");
}

#[test]
fn its_drawing_begins_just_inside_its_body_whatever_the_body_opens_with() {
    let (original, _) = cut(&markup_alone(
        "<html><body bgcolor=\"#f4f4f4\"><!-- a note -->Text first<p>then a paragraph</p></body></html>",
    ))
    .expect("markup to draw");

    assert!(
        original.opening.ends_with("<body bgcolor=\"#f4f4f4\">"),
        "the place is not just inside the body: {:?}",
        original.opening
    );
    assert_eq!(
        drawn(&original),
        "<!-- a note -->Text first<p>then a paragraph</p>"
    );
}

#[test]
fn the_declaration_it_was_written_under_is_kept_as_it_was() {
    // Declared or not decides how an engine lays it out: a picture in a table sits flush under a
    // transitional declaration and has a gap under it under the plain one.
    let transitional = "<!DOCTYPE html PUBLIC \"-//W3C//DTD XHTML 1.0 Transitional//EN\" \"http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd\">";
    let (declared, _) = cut(&markup_alone(&format!(
        "{transitional}<html><body><p>Hello</p></body></html>"
    )))
    .expect("markup to draw");
    let (undeclared, _) = cut(&markup_alone("<p>Hello</p>")).expect("markup to draw");

    assert!(
        declared
            .opening
            .starts_with(&format!("{transitional}<html>")),
        "{:?}",
        declared.opening
    );
    assert!(
        undeclared.opening.starts_with("<html><head>"),
        "a declaration was made up: {:?}",
        undeclared.opening
    );
}

#[test]
fn its_root_and_its_body_keep_what_they_opened_with() {
    let (original, _) = cut(&markup_alone(
        "<html dir=\"rtl\" lang=\"ar\" xmlns:v=\"urn:schemas-microsoft-com:vml\"><body style=\"margin: 0\" bgcolor=\"#000000\" text=\"#ffffff\"><p>Hello</p></body></html>",
    ))
    .expect("markup to draw");

    assert!(
        original.opening.starts_with(
            "<html dir=\"rtl\" lang=\"ar\" xmlns:v=\"urn:schemas-microsoft-com:vml\"><head>"
        ),
        "{:?}",
        original.opening
    );
    assert!(original
        .opening
        .ends_with("<body style=\"margin: 0\" bgcolor=\"#000000\" text=\"#ffffff\">"));
}

#[test]
fn its_styles_stay_in_its_head_and_it_says_it_is_written_in_utf_8() {
    let (original, _) = cut(&markup_alone(
        "<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=windows-1251\"><meta charset=\"koi8-r\"><style>.wide { width: 600px }</style></head><body><p class=\"wide\">Hello</p></body></html>",
    ))
    .expect("markup to draw");

    assert!(
        original
            .opening
            .contains("<head><meta charset=\"utf-8\"><style>.wide { width: 600px }</style></head>"),
        "{:?}",
        original.opening
    );
    for piece in [&original.opening, &original.drawing] {
        assert!(!piece.contains("windows-1251"));
        assert!(!piece.contains("koi8-r"));
    }
}

#[test]
fn markup_written_loosely_goes_as_the_parser_read_it() {
    let (original, _) = cut(&markup_alone("<p>One<p>Two<b>bold")).expect("markup to draw");

    assert_eq!(drawn(&original), "<p>One</p><p>Two<b>bold</b></p>");
}

#[test]
fn a_message_with_no_markup_is_not_cut_down() {
    let raw = message(
        &[
            "From: zeta@example.com",
            "To: kappa@example.net",
            "Subject: Text alone",
            "",
        ],
        &["Just words."],
    );

    assert!(cut(&raw).is_none());
}

/// A message this application wrote: its text, and the markup it sends beside it.
fn typed_here(text: &str) -> Vec<u8> {
    mail_builder::MessageBuilder::new()
        .from("zeta@example.com")
        .to("kappa@example.net")
        .subject("Typed here")
        .body(super::super::typed::written(text, usize::MAX))
        .write_to_string()
        .expect("written")
        .into_bytes()
}

/// What passing a message on hands the cutting: its text as a text box holds it.
fn as_a_text_box_holds_it(parsed: &mail_parser::Message<'_>) -> String {
    super::super::words::written_text(parsed)
        .map(|text| text.lines().collect::<Vec<_>>().join("\n"))
        .unwrap_or_default()
}

#[test]
fn a_message_whose_markup_is_its_text_is_passed_on_as_text() {
    // What a person sent from here is a text they can edit when they pass it on, as it was when
    // the message carried its text alone.
    for text in [
        "Meeting at 5\n> see below\nhttps://example.com",
        "Meeting at 5\n",
    ] {
        let raw = typed_here(text);
        let parsed = MessageParser::default().parse(&raw).expect("a message");

        assert!(
            cut_down(&parsed, &as_a_text_box_holds_it(&parsed)).is_none(),
            "passed on in its own form: {text:?}"
        );
    }
}

#[test]
fn a_page_of_frames_has_no_body_to_draw_in_and_is_not_cut_down() {
    let framed = markup_alone(
        "<html><frameset cols=\"50%,50%\"><frame src=\"a.html\"><frame src=\"b.html\"></frameset></html>",
    );

    assert!(cut(&framed).is_none());
}

#[test]
fn markup_past_what_a_body_is_drawn_within_is_not_cut_down() {
    let crowded = "<b>x</b>".repeat(MOST_ELEMENTS + 1);
    let deep = "<div>".repeat(DEEPEST_NESTING + 1);
    let long = format!("<p>{}</p>", "x".repeat(LARGEST_BODY));

    assert!(cut(&markup_alone(&crowded)).is_none(), "too many elements");
    assert!(cut(&markup_alone(&deep)).is_none(), "nested too deep");
    assert!(
        cut(&markup_alone(&long)).is_none(),
        "longer than a body is read"
    );
}

/// A message of markup and of one picture or more under `named`, each a part of its own.
fn carrying_pictures(markup: &str, named: &str, pictures: usize, disposition: &str) -> Vec<u8> {
    let mut body = vec![
        "--outer".to_owned(),
        "Content-Type: text/html; charset=utf-8".to_owned(),
        String::new(),
        markup.to_owned(),
    ];
    for _ in 0..pictures {
        body.extend([
            "--outer".to_owned(),
            "Content-Type: image/png".to_owned(),
            format!("Content-ID: <{named}>"),
            format!("Content-Disposition: {disposition}; filename=\"photo.png\""),
            "Content-Transfer-Encoding: base64".to_owned(),
            String::new(),
            "iVBORw0KGgo=".to_owned(),
        ]);
    }
    body.push("--outer--".to_owned());

    message(
        &[
            "From: zeta@example.com",
            "To: kappa@example.net",
            "Subject: A photograph",
            "MIME-Version: 1.0",
            "Content-Type: multipart/mixed; boundary=\"outer\"",
        ],
        &body.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}

#[test]
fn a_picture_its_markup_does_not_draw_stays_a_file() {
    // An attachment that carries a name for a picture is still an attachment when nothing draws
    // it: taken inside, it would go out unseen, without its name, and could not be taken off.
    let (original, inside) = cut(&carrying_pictures(
        "<p>No chart here</p>",
        "photo@example.com",
        1,
        "attachment",
    ))
    .expect("markup to draw");

    assert!(original.pictures.is_empty());
    assert!(inside.is_empty());
}

#[test]
fn a_name_drawn_twice_takes_the_last_picture_inside() {
    // The reader draws the last part under a name, so that is the one that goes.
    let (original, inside) = cut(&carrying_pictures(
        "<p>The chart:</p><img src=\"cid:photo@example.com\">",
        "photo@example.com",
        2,
        "inline",
    ))
    .expect("markup to draw");

    assert_eq!(original.pictures.len(), 1);
    assert_eq!(
        inside,
        HashSet::from([3]),
        "the second picture, after the root and the markup"
    );
}

#[test]
fn a_picture_a_style_draws_goes_inside() {
    // The reader draws a picture a background names as it draws one an `<img>` names: left out, a
    // logo drawn as a background would go as a file and leave a gap where it was.
    for markup in [
        "<style>p{background-image:url(cid:photo@example.com)}</style><p>T</p>",
        "<p style=\"background-image: url(cid:photo@example.com)\">T</p>",
    ] {
        let (original, inside) = cut(&carrying_pictures(markup, "photo@example.com", 1, "inline"))
            .expect("markup to draw");

        assert_eq!(original.pictures.len(), 1, "{markup}");
        assert_eq!(inside.len(), 1, "{markup}");
    }
}

#[test]
fn a_picture_of_a_kind_this_build_does_not_draw_stays_a_file() {
    let (original, inside) =
        cut(&with_a_picture("image/tiff", "chart@example.com")).expect("markup to draw");

    assert!(original.pictures.is_empty());
    assert!(inside.is_empty());
}

#[test]
fn a_picture_whose_name_cannot_be_written_back_as_it_was_stays_a_file() {
    for named in [
        "two words@example.com",
        &format!("{}@example.com", "a".repeat(250)),
    ] {
        let (original, inside) = cut(&with_a_picture("image/png", named)).expect("markup to draw");

        assert!(original.pictures.is_empty(), "{named:?} went inside");
        assert!(inside.is_empty(), "{named:?}");
    }
}

#[test]
fn what_is_written_goes_above_it_as_text() {
    let (original, _) = cut(&markup_alone(
        "<html><body bgcolor=\"#f4f4f4\"><p>Theirs</p></body></html>",
    ))
    .expect("markup to draw");

    let written = beneath("Look <here> & see\r\n  two  spaces\n\nend", &original);

    // In its own colours: inside a body drawn white on black, black text on its default would
    // not be seen.
    assert!(
        written.contains("<body bgcolor=\"#f4f4f4\"><div dir=\"auto\" style=\"white-space: pre-wrap; color: #000000; background-color: #ffffff\">Look &lt;here&gt; &amp; see<br>  two  spaces<br><br>end</div><p>Theirs</p></body></html>"),
        "{written:?}"
    );
}

#[test]
fn as_a_message_of_its_own_it_is_drawn_with_its_picture_inside() {
    use crate::ports::body::{BodyForm, BodyKind, Pictures, Viewing};

    let (original, _) =
        cut(&with_a_picture("image/png", "chart@example.com")).expect("markup to draw");

    let shown = super::super::body_of(
        &as_message(&original),
        Viewing {
            form: BodyForm::Sent,
            pictures: Pictures::Blocked,
        },
    );

    assert_eq!(shown.kind, BodyKind::Html);
    assert_eq!(shown.parts.len(), 1, "the chart is drawn in its place");
    assert!(shown.html.contains("The chart:"));
    assert!(!shown.html.contains("cid:"), "{:?}", shown.html);
}
