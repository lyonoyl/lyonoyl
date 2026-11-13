use super::*;

/// What the text's box holds, without the document around it.
fn lines_of(text: &str) -> String {
    let whole = markup(text);
    let opens = format!("<div style=\"{LETTERS}; {BREAKS}\">");
    let closes = "</div></body></html>";
    let start = whole.find(&opens).expect("the text's own box") + opens.len();
    let end = whole.len() - closes.len();
    assert!(whole.ends_with(closes), "{whole}");
    whole[start..end].to_owned()
}

/// One line as its block.
fn line(said: &str) -> String {
    format!("<div dir=\"auto\">{said}</div>")
}

fn quote_opens() -> String {
    format!("<blockquote type=\"cite\" style=\"{QUOTED}\">")
}

#[test]
fn what_means_something_to_markup_is_written_as_itself() {
    // A person typing a tag is writing about one. Read as markup, it would be a tag in
    // somebody else's mail.
    assert_eq!(
        lines_of("<b>bold</b> & \"quoted\" 'too'"),
        line("&lt;b&gt;bold&lt;/b&gt; &amp; &quot;quoted&quot; &#39;too&#39;")
    );
    assert_eq!(
        lines_of("<script>alert(1)</script>"),
        line("&lt;script&gt;alert(1)&lt;/script&gt;")
    );
}

#[test]
fn every_line_is_a_block_of_its_own_and_a_blank_one_holds_a_break() {
    assert_eq!(
        lines_of("Hello,\r\n\r\nsee below\nZeta"),
        [
            line("Hello,"),
            line("<br>"),
            line("see below"),
            line("Zeta")
        ]
        .concat()
    );
}

#[test]
fn spaces_markup_would_fold_are_kept_and_a_tab_is_four() {
    // A renderer that ignores white-space keeps a space that does not fold, and only that.
    assert_eq!(lines_of("  a   b"), line("&nbsp;&nbsp;a &nbsp;&nbsp;b"));
    assert_eq!(lines_of("a\tb"), line("a&nbsp;&nbsp;&nbsp;&nbsp;b"));
}

#[test]
fn every_line_finds_its_own_direction() {
    // One block for the whole message would take its direction from its first letter, and an
    // English line under a Hebrew one would be set right to left.
    assert_eq!(
        lines_of("\u{05e9}\u{05dc}\u{05d5}\u{05dd}\nHello"),
        [line("\u{05e9}\u{05dc}\u{05d5}\u{05dd}"), line("Hello")].concat()
    );
}

#[test]
fn a_reply_s_quoted_lines_are_quoted_at_their_level_without_their_marks() {
    let quoted = lines_of("Agreed.\n> On Monday:\n>> Shall we?\n>\n> Yes\nZeta");

    assert_eq!(
        quoted,
        [
            line("Agreed."),
            quote_opens(),
            line("On Monday:"),
            quote_opens(),
            line("Shall we?"),
            "</blockquote>".to_owned(),
            line("<br>"),
            line("Yes"),
            "</blockquote>".to_owned(),
            line("Zeta"),
        ]
        .concat()
    );
    // A mark that does not open a line is the character it is.
    assert_eq!(lines_of("5 > 3"), line("5 &gt; 3"));
}

#[test]
fn a_quotation_the_text_ends_in_is_closed() {
    assert_eq!(
        lines_of("> > last"),
        [
            quote_opens(),
            quote_opens(),
            line("last"),
            "</blockquote></blockquote>".to_owned()
        ]
        .concat()
    );
}

#[test]
fn an_address_the_window_marks_is_a_link_to_what_it_opens() {
    assert_eq!(
        lines_of("See https://example.com/a?b=1&c=2, or www.example.org."),
        line(
            "See <a href=\"https://example.com/a?b=1&amp;c=2\">https://example.com/a?b=1&amp;c=2</a>, \
             or <a href=\"https://www.example.org/\">www.example.org</a>."
        )
    );
    assert_eq!(
        lines_of("Write to mailto:zeta@example.com"),
        line("Write to <a href=\"mailto:zeta@example.com\">mailto:zeta@example.com</a>")
    );
}

#[test]
fn what_only_looks_like_an_address_stays_text() {
    // Only what the window would mark: a scheme it does not open is the words they are.
    assert_eq!(lines_of("javascript:alert(1)"), line("javascript:alert(1)"));
    assert_eq!(lines_of("file:///etc/hosts"), line("file:///etc/hosts"));
}

#[test]
fn a_line_that_turns_text_around_carries_no_link() {
    // Drawn as written, the override shows one address and the link opens another.
    let turned = "\u{202e}https://example.com/moc.elpmaxe";

    assert_eq!(lines_of(turned), line(turned));
    assert_eq!(
        lines_of(&format!("{turned}\nhttps://example.org")),
        [
            line(turned),
            line("<a href=\"https://example.org/\">https://example.org</a>")
        ]
        .concat(),
        "a line beside it, holding nothing that turns, keeps its link"
    );
}

#[test]
fn the_markup_names_its_letters_and_gives_the_text_no_colour() {
    let whole = markup("Hi");

    assert!(whole.contains("font-family: 'Segoe UI'"), "{whole}");
    assert!(
        !whole.contains("color:") && !whole.contains("background"),
        "a colour would fix the message to one theme: {whole}"
    );
}

#[test]
fn the_text_and_the_markup_go_as_one_alternative_cut_by_a_random_boundary() {
    let wire = mail_builder::MessageBuilder::new()
        .from("zeta@example.com")
        .body(written("Shall we?\nhttps://example.com", usize::MAX))
        .write_to_string()
        .expect("written");
    let parsed = mail_parser::MessageParser::default()
        .parse(wire.as_bytes())
        .expect("a message");

    assert_eq!(
        parsed.body_text(0).as_deref(),
        Some("Shall we?\r\nhttps://example.com")
    );
    assert_eq!(
        parsed.body_html(0).as_deref(),
        Some(markup("Shall we?\nhttps://example.com").as_str())
    );
    let boundary = wire
        .split("boundary=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a boundary");
    assert!(
        boundary.len() == 32 && boundary.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "not random: {boundary}"
    );
}

#[test]
fn markup_with_no_room_for_it_is_left_out_and_the_text_goes_alone() {
    // A message with files close to the most a message may weigh has room for its text and not
    // for its markup; the markup is the half every reader can do without.
    let text = "Shall we?";
    let room = markup(text).len() - 1;
    let wire = mail_builder::MessageBuilder::new()
        .from("zeta@example.com")
        .body(written(text, room))
        .write_to_string()
        .expect("written");

    assert!(!wire.contains("multipart/alternative"), "{wire}");
    assert!(!wire.contains("text/html"), "{wire}");
    assert!(wire.contains("text/plain"), "{wire}");
}

#[test]
fn markup_this_application_would_not_draw_whole_is_not_sent() {
    // Past what the reader draws, the copy kept in Sent would be shown cut short of a text that
    // is whole beside it.
    let text = "\n".repeat(LARGEST_BODY / 20);
    assert!(markup(&text).len() > LARGEST_BODY);

    let wire = mail_builder::MessageBuilder::new()
        .from("zeta@example.com")
        .body(written(&text, usize::MAX))
        .write_to_string()
        .expect("written");

    assert!(!wire.contains("text/html"));
}

#[test]
fn markup_is_known_as_the_markup_of_its_text_and_of_nothing_else() {
    let text = "Hello\n> quoted";

    assert!(is_the_markup_of(&markup(text), text));
    // The text as a text box holds it, its last line end gone.
    assert!(is_the_markup_of(&markup("Hello\n"), "Hello"));
    assert!(!is_the_markup_of(&markup("Hello"), "Hello, Zeta"));
    assert!(!is_the_markup_of("<p>Hello</p>", "Hello"));
}
