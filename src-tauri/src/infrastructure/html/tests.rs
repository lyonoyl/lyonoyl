use super::*;

/// The corpus: every message that ever broke something, kept as a file.
fn corpus() -> Vec<(String, Vec<u8>)> {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("eml");

    let mut messages: Vec<(String, Vec<u8>)> = std::fs::read_dir(&directory)
        .expect("the corpus is in the repository")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|kind| kind == "eml"))
        .map(|path| {
            let name = path
                .file_name()
                .expect("a file has a name")
                .to_string_lossy()
                .into_owned();
            (name, std::fs::read(&path).expect("the file is readable"))
        })
        .collect();

    messages.sort_by(|left, right| left.0.cmp(&right.0));
    messages
}

fn named(name: &str) -> Vec<u8> {
    corpus()
        .into_iter()
        .find(|(file, _)| file == name)
        .unwrap_or_else(|| panic!("{name} is in the corpus"))
        .1
}

/// A message built here rather than kept as a file: the lines are joined explicitly, so no
/// formatter can fold them into one and quietly change what is being tested.
fn message(headers: &[&str], body: &[&str]) -> Vec<u8> {
    let mut lines: Vec<&str> = headers.to_vec();
    lines.push("");
    lines.extend_from_slice(body);

    lines.join("\n").into_bytes()
}

fn html_message(body: &[&str]) -> Vec<u8> {
    message(
        &[
            "From: someone@example.com",
            "To: you@example.com",
            "Subject: A message",
            "MIME-Version: 1.0",
            "Content-Type: text/html; charset=utf-8",
        ],
        body,
    )
}

/// What must be true of anything this module hands out, whatever went in.
///
/// Not a list of vectors — a list of the shapes a vector has to take to do anything at all, and
/// all of them are markup. Only what sits between `<` and `>` is looked at: a message that
/// spells `onerror=` in its text is spelling it, and a document where every `<` was escaped is
/// nothing but text however alarming the words in it are.
fn nothing_that_runs(html: &str, from: &str) {
    let lowered = html.to_lowercase();
    let tags = tags_in(&lowered);
    let markup = tags.join(" ");

    // Elements are matched by name rather than by the letters appearing anywhere: `link` is
    // also how the address of an offered link is written, and a rule that cannot tell the two
    // apart is a rule that fires on the thing it is meant to allow.
    for tag in &tags {
        let name = tag
            .trim_start_matches('/')
            .split(|character: char| character.is_whitespace() || character == '/')
            .next()
            .unwrap_or_default();

        assert!(
            !matches!(
                name,
                "script"
                    | "iframe"
                    | "object"
                    | "embed"
                    | "form"
                    | "input"
                    | "link"
                    | "base"
                    | "template"
                    | "noscript"
                    | "svg"
                    | "math"
            ) && !name.starts_with("!--"),
            "{from}: <{name}> survived sanitizing:\n{html}"
        );
    }

    // `style` used to be on that list and is deliberately no longer: a message's own stylesheet is
    // kept, because dropping it showed the message as something other than what was written. What
    // it may not do is stop being a stylesheet -- the element holds raw text, and text that closes
    // it early would put whatever follows into the document. The parser decides where it ends, so
    // this says out loud that nothing between those tags ever looks like markup.
    for block in html.to_lowercase().split("<style").skip(1) {
        let inside = block.split_once('>').map(|(_, rest)| rest).unwrap_or(block);
        let inside = inside.split("</style").next().unwrap_or(inside);

        assert!(
            !inside.contains('<'),
            "{from}: a stylesheet carries markup:\n{html}"
        );
    }

    for forbidden in ["javascript:", "srcdoc"] {
        assert!(
            !markup.contains(forbidden),
            "{from}: {forbidden} survived sanitizing:\n{html}"
        );
    }

    // Any handler at all, rather than the three everyone remembers.
    for (at, _) in markup.match_indices(" on") {
        let rest = &markup[at + 3..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        let follows = rest[name.len()..].chars().next();
        assert!(
            name.is_empty() || follows != Some('='),
            "{from}: an event handler survived: on{name}=\n{html}"
        );
    }

    // Nothing may be fetched from somewhere else, and nothing may be navigated to. What is
    // forbidden is an attribute that does either — not an address written as words: a sender
    // who typed a link into their message typed it for a person to read, and taking it out of
    // the text would lose what they wrote. The frame has no network at all, so text is text.
    for fetching in [
        "srcset=",
        "background=",
        "poster=",
        "action=",
        "data=",
        "url(",
    ] {
        assert!(
            !markup.contains(fetching),
            "{from}: {fetching} survived, and it is how a message reaches somewhere else:\n{html}"
        );
    }

    // Two attributes may survive, and only as names on this origin: a picture that travelled
    // inside the message, and a link that leads back to this application rather than out.
    // No scheme, no host, nothing that leaves here on its own.
    for (attribute, prefix) in [("src=\"", "/"), ("href=\"", "/link/")] {
        for value in attribute_values(&markup, attribute) {
            let name = value.strip_prefix(prefix);
            assert!(
                name.is_some_and(|name| {
                    name.len() == 64 && name.chars().all(|c| c.is_ascii_hexdigit())
                }),
                "{from}: {attribute}{value}\" points somewhere other than here:\n{html}"
            );
        }
    }
}

/// What sits between `<` and `>`, which is the only part of a document that does anything.
fn tags_in(html: &str) -> Vec<&str> {
    html.match_indices('<')
        .filter_map(|(at, _)| {
            let rest = &html[at + 1..];
            rest.find('>').map(|end| &rest[..end])
        })
        .collect()
}

/// Every value of one attribute, as written.
fn attribute_values<'a>(html: &'a str, attribute: &str) -> Vec<&'a str> {
    html.match_indices(attribute)
        .filter_map(|(at, _)| {
            let rest = &html[at + attribute.len()..];
            rest.find('"').map(|end| &rest[..end])
        })
        .collect()
}

#[test]
fn no_message_in_the_corpus_produces_anything_that_runs() {
    // The golden test the stage is about. Every message that ever broke something is in the
    // corpus, and none of them may come out of here able to do anything.
    let corpus = corpus();
    assert!(
        corpus.len() >= 12,
        "the corpus has shrunk: {}",
        corpus.len()
    );

    for (name, raw) in corpus {
        let rendered = body_of(&raw, Viewing::plainly());
        nothing_that_runs(&rendered.html, &name);
    }
}

#[test]
fn the_ways_of_running_code_are_all_gone_and_the_words_are_kept() {
    let rendered = body_of(
        &named("hostile-script-and-handlers.eml"),
        Viewing::plainly(),
    );

    nothing_that_runs(&rendered.html, "script-and-handlers");
    assert!(
        rendered.html.contains("Ordinary text before it.")
            && rendered.html.contains("Ordinary text after it."),
        "the message was emptied rather than cleaned:\n{}",
        rendered.html
    );
}

#[test]
fn markup_that_changes_meaning_when_it_is_read_again_is_gone_too() {
    // The class ammonia itself was bypassed by three times in eleven months. The sanitizer is
    // one layer of four for exactly this reason; what this pins is that the layer does its part.
    let rendered = body_of(&named("hostile-mxss.eml"), Viewing::plainly());

    nothing_that_runs(&rendered.html, "mxss");
    assert!(
        !rendered.html.contains("alert("),
        "a payload survived, even as text:\n{}",
        rendered.html
    );
    // What is not asserted: that the words around the vectors survive. An mXSS vector works by
    // being re-parsed into something else, and the re-parse loses the markup around it too — a
    // message shaped like this renders as almost nothing, which is the right outcome for it and
    // the reason the stage also owes a "show the source" way out.
}

#[test]
fn a_picture_that_travelled_inside_the_message_is_shown() {
    // Nothing is fetched to show it: the bytes arrived in the message, and they are served
    // from beside the body under a name that means nothing outside this moment.
    let rendered = body_of(&named("inline-image.eml"), Viewing::plainly());

    assert_eq!(rendered.parts.len(), 1, "the picture was not kept");
    assert_eq!(rendered.parts[0].content_type, "image/png");
    assert!(rendered.parts[0].bytes.starts_with(b"\x89PNG"));
    assert!(
        rendered
            .html
            .contains(&format!("src=\"/{}\"", rendered.parts[0].name)),
        "the body does not refer to the picture:\n{}",
        rendered.html
    );
    assert_eq!(rendered.remote_blocked, 0, "nothing was blocked here");
    nothing_that_runs(&rendered.html, "inline-image");
}

#[test]
fn a_picture_the_message_does_not_carry_is_counted_rather_than_shown() {
    // A `cid:` naming a part that is not there is a picture that cannot be shown, and it is
    // counted with the ones that live somewhere else.
    let raw = html_message(&["<p><img src=\"cid:missing@example.com\" alt=\"gone\"></p>"]);

    let rendered = body_of(&raw, Viewing::plainly());

    assert!(rendered.parts.is_empty());
    assert_eq!(rendered.remote_blocked, 1);
    nothing_that_runs(&rendered.html, "missing cid");
}

#[test]
fn a_drawing_is_shown_and_can_still_do_nothing() {
    // An SVG is a document that can carry script, and this build refused it for that. It is shown
    // now, and what makes that safe is that it can only ever be reached as an image: the sanitizer
    // keeps `src` on `img` and drops every element that could point at one as a document. In an
    // `<img>` an engine renders it in a mode where no script runs and no external reference is
    // followed -- and the part is served under a policy that permits nothing besides.
    let raw = message(
        &[
            "From: someone@example.com",
            "Subject: A drawing",
            "MIME-Version: 1.0",
            "Content-Type: multipart/related; boundary=\"rel\"",
        ],
        &[
            "--rel",
            "Content-Type: text/html; charset=utf-8",
            "",
            "<p><img src=\"cid:drawing@example.com\" alt=\"a drawing\"></p>",
            "",
            "--rel",
            "Content-Type: image/svg+xml",
            "Content-ID: <drawing@example.com>",
            "",
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><script>alert(1)</script></svg>",
            "--rel--",
        ],
    );

    let rendered = body_of(&raw, Viewing::plainly());

    assert_eq!(rendered.parts.len(), 1, "the drawing is not shown");
    assert_eq!(rendered.parts[0].content_type, "image/svg+xml");
    assert_eq!(rendered.remote_not_shown, 0);
    assert_eq!(rendered.remote_blocked, 0);

    // Reachable as an image and in no other way: the script it carries is inert in that mode, and
    // there is no element left in the body that could ask for it as a document instead.
    assert!(
        rendered.html.contains("<img"),
        "the body does not point at it as an image"
    );
    for way_in in ["<object", "<embed", "<iframe", "<svg", "<use"] {
        assert!(
            !rendered.html.contains(way_in),
            "{way_in} survived, and it can ask for a picture as a document"
        );
    }
    nothing_that_runs(&rendered.html, "svg part");
}

#[test]
fn a_picture_the_body_never_mentions_is_not_held() {
    // A part can be a picture, carry an identifier, and still be something the body never
    // points at. Holding it for the length of a read is a mailbox of pictures nobody asked for.
    let raw = message(
            &[
                "From: someone@example.com",
                "Subject: One shown, one not",
                "MIME-Version: 1.0",
                "Content-Type: multipart/related; boundary=\"rel\"",
            ],
            &[
                "--rel",
                "Content-Type: text/html; charset=utf-8",
                "",
                "<p><img src=\"cid:shown@example.com\" alt=\"shown\"></p>",
                "",
                "--rel",
                "Content-Type: image/png",
                "Content-Transfer-Encoding: base64",
                "Content-ID: <shown@example.com>",
                "",
                "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAIAAACQkWg2AAAAF0lEQVQoz2P8z8DwHwAEjCoGYBSMKgZgFAAA970K/2tqKx4AAAAASUVORK5CYII=",
                "--rel",
                "Content-Type: image/png",
                "Content-Transfer-Encoding: base64",
                "Content-ID: <never-mentioned@example.com>",
                "",
                "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAIAAACQkWg2AAAAF0lEQVQoz2P8z8DwHwAEjCoGYBSMKgZgFAAA970K/2tqKx4AAAAASUVORK5CYII=",
                "--rel--",
            ],
        );

    let rendered = body_of(&raw, Viewing::plainly());

    assert_eq!(
        rendered.parts.len(),
        1,
        "a picture nobody points at was kept"
    );
    assert!(rendered
        .html
        .contains(&format!("src=\"/{}\"", rendered.parts[0].name)));
}

#[test]
fn a_message_that_wants_to_report_back_is_counted_and_stopped() {
    let rendered = body_of(&named("hostile-remote-content.eml"), Viewing::plainly());

    nothing_that_runs(&rendered.html, "remote-content");
    assert_eq!(rendered.remote_blocked, 2, "both pictures are counted");
    // The link is offered rather than removed: it leads to this application, which asks
    // before anything leaves the machine.
    assert_eq!(rendered.links.len(), 1);
    assert_eq!(
        rendered.links[0].url,
        "https://tracker.example.net/click?who=you"
    );
    assert!(rendered.html.contains("Read more"));
}

#[test]
fn a_link_leads_back_here_and_the_address_stays_behind() {
    let rendered = body_of(&named("newsletter-heavy-layout.eml"), Viewing::plainly());

    assert_eq!(rendered.links.len(), 3, "every link is offered");
    for link in &rendered.links {
        assert!(
            rendered
                .html
                .contains(&format!("href=\"/link/{}\"", link.name)),
            "the body does not carry the name of {}",
            link.url
        );
        assert!(
            !rendered.html.contains(&link.url),
            "the address itself is in the body: {}",
            link.url
        );
    }
}

#[test]
fn an_address_this_build_will_not_open_is_not_offered_at_all() {
    // A `file:` link would open something on this machine; a scheme some other application
    // registered would hand a message whatever that application does with it.
    let raw = html_message(&[
        "<p><a href=\"file:///etc/passwd\">a file</a></p>",
        "<p><a href=\"tel:+15550100\">a number</a></p>",
        "<p><a href=\"mailto:someone@example.com\">an address</a></p>",
    ]);

    let rendered = body_of(&raw, Viewing::plainly());

    assert_eq!(rendered.links.len(), 1, "only the mail address is offered");
    assert_eq!(rendered.links[0].url, "mailto:someone@example.com");
    nothing_that_runs(&rendered.html, "schemes");
}

#[test]
fn which_addresses_may_be_offered_is_a_short_list() {
    assert!(openable("https://example.com/a").is_some());
    assert!(openable("HTTP://example.com").is_some());
    assert!(openable("mailto:someone@example.com").is_some());

    assert!(openable("file:///etc/passwd").is_none());
    assert!(openable("javascript:alert(1)").is_none());
    assert!(openable("data:text/html,<script>alert(1)</script>").is_none());
    assert!(openable("tel:+15550100").is_none());
    assert!(openable("nothing-at-all").is_none());
}

#[test]
fn a_link_is_kept_as_the_parser_writes_it_out() {
    // The standard reads a backslash as a slash, so this is a path on `good.example` to the parser
    // that names the host in the panel — and a sign-in at `evil.example` to the one the system
    // opens links with. Kept as written, the panel names one host and the other opens.
    let raw = html_message(&["<p><a href=\"https://good.example\\@evil.example/\">pay</a></p>"]);

    let rendered = body_of(&raw, Viewing::plainly());

    assert_eq!(rendered.links.len(), 1, "the link was not offered");
    let kept = &rendered.links[0].url;
    assert!(!kept.contains('\\'), "kept as the message wrote it: {kept}");
    assert_eq!(kept, "https://good.example/@evil.example/");
}

#[test]
fn a_link_is_handed_over_as_it_was_kept() {
    // Read again when the person says yes, and what is handed to the system is what that reading
    // writes out. For an address kept the way it is offered, that is the same string, so the
    // panel shows exactly what opens.
    for written in [
        "https://good.example\\@evil.example/",
        "HTTPS://Example.COM/Path?q=1#part",
        "https:\\\\example.com\\a",
        "mailto:someone@example.com",
    ] {
        let kept = openable(written).expect("offered");

        assert_eq!(
            openable(kept.as_str()).map(String::from),
            Some(kept.to_string()),
            "{written} is opened as something other than what was shown"
        );
    }
}

#[test]
fn an_address_nothing_can_read_is_not_offered() {
    // An `http` link with no host, or a host with a space in it, is nothing the standard reads,
    // and nothing a person could be told the host of. A no-break space in front is what carries
    // one this far: the sanitizing library reads such an address as a relative one and lets it
    // through, and the folding here takes the space off.
    let raw = html_message(&[
        "<p><a href=\"&#160;http://\">nowhere</a></p>",
        "<p><a href=\"&#160;https://exa mple.com/\">a gap</a></p>",
    ]);

    let rendered = body_of(&raw, Viewing::plainly());

    assert!(
        rendered.links.is_empty(),
        "offered: {:?}",
        rendered
            .links
            .iter()
            .map(|link| link.url.as_str())
            .collect::<Vec<_>>()
    );
    nothing_that_runs(&rendered.html, "unreadable addresses");
}

#[test]
fn a_picture_somewhere_else_is_named_rather_than_fetched_by_the_frame() {
    // What the reader asked for is still not fetched by the frame: the body gets a name on
    // this origin, and the bytes are put under it by the application.
    let raw = named("hostile-remote-content.eml");

    let shown = body_of(
        &raw,
        Viewing {
            form: BodyForm::Sent,
            pictures: Pictures::Shown,
        },
    );

    assert_eq!(shown.remote.len(), 2, "both pictures are named");
    assert_eq!(shown.remote_blocked, 0, "nothing was blocked this time");
    for pointed_at in &shown.remote {
        assert!(pointed_at.url.starts_with("http"), "{}", pointed_at.url);
        assert!(shown
            .html
            .contains(&format!("src=\"/{}\"", pointed_at.name)));
        assert!(
            !shown.html.contains(&pointed_at.url),
            "the address is in the body"
        );
    }
    nothing_that_runs(&shown.html, "pictures shown");
}

#[test]
fn nothing_is_named_when_the_reader_did_not_ask() {
    let raw = named("hostile-remote-content.eml");

    let blocked = body_of(&raw, Viewing::plainly());

    assert!(blocked.remote.is_empty());
    assert_eq!(blocked.remote_blocked, 2);
}

#[test]
fn a_message_may_say_how_it_looks() {
    // A newsletter is written in inline styles. Cutting them shows the reader something other
    // than what was written — and inside a frame with no network and no script, the reasons
    // CSS is usually cut do not apply.
    let raw = html_message(&[
        "<div style=\"background-color:#ffffff;padding:24px;border-radius:12px\">",
        "<p style=\"color:#111318;font-size:22px;font-weight:600\">A heading</p>",
        "</div>",
    ]);

    let rendered = body_of(&raw, Viewing::plainly());

    assert!(rendered.html.contains("background-color:#ffffff"));
    assert!(rendered.html.contains("padding:24px"));
    assert!(rendered.html.contains("font-weight:600"));
    nothing_that_runs(&rendered.html, "styled");
}

#[test]
fn what_the_sender_hid_stays_hidden() {
    // A preheader is written to be read by the list and not by the reader. Dropping the
    // declaration that hides it puts a line on screen that nobody wrote to be seen.
    let raw = html_message(&[
        "<div style=\"display:none;max-height:0;overflow:hidden\">The preheader.</div>",
        "<p>The message.</p>",
    ]);

    let rendered = body_of(&raw, Viewing::plainly());

    assert!(rendered.html.contains("display:none"), "{}", rendered.html);
}

#[test]
fn a_declaration_that_fetches_or_runs_is_dropped_and_the_rest_is_kept() {
    let raw = html_message(&[
            "<p style=\"color:#111;background:url('https://tracker.example.net/p.png');padding:8px\">Text</p>",
        ]);

    let rendered = body_of(&raw, Viewing::plainly());

    assert!(rendered.html.contains("color:#111"));
    assert!(rendered.html.contains("padding:8px"));
    assert!(!rendered.html.contains("tracker"), "{}", rendered.html);
    nothing_that_runs(&rendered.html, "url in style");
}

/// What `filtered_style` keeps, with nothing to serve.
///
/// Every address is answered "never arriving", so what comes back is the filter's own decision
/// rather than what a fetch would have put there.
fn kept_style(style: &str) -> Option<String> {
    super::styling::filtered_style(style, |_| super::styling::ThePicture::NeverArriving).0
}

#[test]
fn the_shapes_that_make_a_declaration_something_else_are_refused() {
    for value in [
        "width:expression(alert(1))",
        "behavior:url(#default#time2)",
        "-moz-binding:url(http://example.net/x.xml)",
        "background:image-set('https://example.net/a.png')",
        "color:\\75rl(https://example.net/a.png)",
        "font-family:'</style><script>alert(1)</script>'",
    ] {
        assert_eq!(kept_style(value), None, "kept: {value}");
    }
}

#[test]
fn a_fragment_that_is_not_a_declaration_is_dropped_and_carries_nothing_with_it() {
    // A comment in a declaration list hides what follows it from whoever reads the list. What
    // is written out here is rebuilt from declarations this build recognised, so the comment
    // is not among them — and what follows it is judged on its own, not on where it sat.
    let kept = kept_style("color:red;/*;padding:8px").expect("something is kept");

    assert_eq!(kept, "color:red;padding:8px");
    assert!(!kept.contains("/*"), "a comment reached the output: {kept}");
}

#[test]
fn a_property_this_build_does_not_know_is_not_passed_on() {
    // Everything kept describes text, spacing, colour or the size of a box. What is left out
    // is what makes a frame behave unlike a document.
    for value in [
        "position:fixed;top:0",
        "animation:spin 1s infinite",
        "content:'x'",
        "pointer-events:none",
        "z-index:99999",
        "transform:scale(40)",
    ] {
        assert_eq!(kept_style(value), None, "kept: {value}");
    }
}

#[test]
fn a_declaration_list_longer_than_anyone_writes_is_dropped_whole() {
    let long = "color:red;".repeat(400);

    assert!(long.len() > super::styling::LONGEST_STYLE);
    assert_eq!(kept_style(&long), None);
}

#[test]
fn what_is_kept_is_written_out_again_rather_than_trimmed() {
    // Whatever shape the message wrote it in is gone: the property is lowercased, the spacing
    // is this build's, and nothing between the declarations survives.
    let kept = kept_style("  COLOR : #Fff ;  padding:8px  ").expect("both are kept");

    assert_eq!(kept, "color:#Fff;padding:8px");
}

#[test]
fn the_attributes_that_predate_css_carry_numbers_and_nothing_else() {
    assert_eq!(plain_value("600"), Some("600".to_owned()));
    assert_eq!(plain_value("100%"), Some("100%".to_owned()));
    assert_eq!(plain_value("#f2f3f5"), Some("#f2f3f5".to_owned()));
    assert_eq!(plain_value("center"), Some("center".to_owned()));

    assert_eq!(plain_value("600\" onload=\"alert(1)"), None);
    assert_eq!(plain_value("url(https://example.net/a.png)"), None);
    assert_eq!(plain_value(&"9".repeat(64)), None);
    assert_eq!(plain_value(""), None);
}

#[test]
fn a_presentational_attribute_carrying_something_else_is_dropped() {
    // The value is escaped on the way out, so this is not the boundary — it is the layer that
    // means the boundary never has to hold. What a `width` may carry is a number.
    let raw = html_message(&[
        "<table width=\"url('https://tracker.example.net/a.png')\">",
        "<tr><td colspan=\"2\">A cell</td></tr>",
        "</table>",
    ]);

    let rendered = body_of(&raw, Viewing::plainly());

    assert!(!rendered.html.contains("tracker"), "{}", rendered.html);
    assert!(
        rendered.html.contains("colspan=\"2\""),
        "the plain one was dropped too"
    );
    nothing_that_runs(&rendered.html, "junk width");
}

#[test]
fn a_table_written_the_old_way_keeps_its_shape() {
    let raw = html_message(&[
        "<table width=\"600\" cellpadding=\"0\" bgcolor=\"#ffffff\">",
        "<tr valign=\"top\"><td width=\"50%\" align=\"center\">One</td></tr>",
        "</table>",
    ]);

    let rendered = body_of(&raw, Viewing::plainly());

    assert!(rendered.html.contains("width=\"600\""));
    assert!(rendered.html.contains("bgcolor=\"#ffffff\""));
    assert!(rendered.html.contains("align=\"center\""));
    nothing_that_runs(&rendered.html, "old table");
}

#[test]
fn a_document_that_is_not_one_is_shown_as_text_rather_than_parsed() {
    // Five thousand nested tags, nearly ten times deeper than an engine builds a document, and a
    // tree whose building costs the square of its depth. The refusal happens before a parser is
    // handed anything.
    let rendered = body_of(&named("hostile-deep-nesting.eml"), Viewing::plainly());

    assert_eq!(rendered.kind, BodyKind::PlainText);
    nothing_that_runs(&rendered.html, "deep-nesting");
}

#[test]
fn a_message_that_stops_in_the_middle_still_gives_back_what_it_had() {
    // Naming the words rather than the shape. A kind other than `Nothing` is satisfied by an empty
    // body, because an empty string renders as a paragraph like any other — so the assertion said
    // "something came back" while the test was named for what came back.
    for (name, words) in [
        // Its HTML alternative is the part that never closes, so what survives is the text one —
        // which is the right answer and not the obvious one, and worth naming for that reason.
        ("hostile-broken-boundary.eml", "The text part."),
        (
            "hostile-truncated-base64.eml",
            "This is a paragraph that was cut off in the m",
        ),
    ] {
        let rendered = body_of(&named(name), Viewing::plainly());

        assert_ne!(rendered.kind, BodyKind::Nothing, "{name} gave back nothing");
        assert!(
            rendered.html.contains(words),
            "{name} lost the part it had:\n{}",
            rendered.html
        );
        nothing_that_runs(&rendered.html, name);
    }
}

#[test]
fn a_body_written_in_another_charset_arrives_as_words() {
    let rendered = body_of(&named("hostile-koi8r-body.eml"), Viewing::plainly());

    assert_eq!(rendered.kind, BodyKind::PlainText);
    assert!(
        rendered.html.contains("письмо"),
        "KOI8-R was not decoded:\n{}",
        rendered.html
    );
}

#[test]
fn a_message_that_renders_as_nothing_can_still_be_looked_at() {
    // The mXSS message comes out almost empty, and that is the right outcome for it. What
    // would not be right is leaving the reader with no way to see what arrived.
    let raw = named("hostile-mxss.eml");

    let sent = body_of(&raw, Viewing::plainly());
    let source = body_of(
        &raw,
        Viewing {
            form: BodyForm::Source,
            pictures: Pictures::Blocked,
        },
    );

    assert!(
        sent.html.len() < 200,
        "the message rendered as more than nothing"
    );
    assert!(
        source.html.contains("Subject:") && source.html.contains("&lt;svg&gt;"),
        "the source does not show the message:\n{}",
        source.html
    );
    nothing_that_runs(&source.html, "source");
}

#[test]
fn the_source_is_shown_as_text_and_never_parsed() {
    // Nothing is parsed to show it — which is the point of offering it at all: it is what to
    // look at when parsing produced nothing worth seeing.
    let raw = html_message(&["<script>alert('x')</script><p>Hello.</p>"]);

    let source = body_of(
        &raw,
        Viewing {
            form: BodyForm::Source,
            pictures: Pictures::Blocked,
        },
    );

    assert_eq!(source.kind, BodyKind::PlainText);
    assert!(source.html.contains("&lt;script&gt;"));
    nothing_that_runs(&source.html, "source of a script");
}

#[test]
fn the_text_of_a_message_can_be_read_on_its_own() {
    // A message that carries both wrote the text for a client that shows no HTML. A reader
    // whose HTML came out empty is in the same position.
    let raw = named("html-and-text.eml");

    let text = body_of(
        &raw,
        Viewing {
            form: BodyForm::Text,
            pictures: Pictures::Blocked,
        },
    );

    assert_eq!(text.kind, BodyKind::PlainText);
    assert!(text.parts.is_empty() && text.links.is_empty());
    assert_ne!(body_of(&raw, Viewing::plainly()).kind, BodyKind::PlainText);
    nothing_that_runs(&text.html, "text form");
}

/// Three answers reach the reader as three, not as two.
///
/// "There was nowhere to read it" used to arrive as "there is nothing in it to show". The person
/// was then told their message was empty when the machine had simply been busy with other
/// messages, and went to argue with a message that was fine.
#[test]
fn a_message_nobody_could_look_at_is_not_a_message_with_nothing_in_it() {
    use super::guards::Guarded;
    use crate::ports::body::BodyKind;

    assert_eq!(super::shown(Guarded::Busy).kind, BodyKind::Busy);
    assert_eq!(super::shown(Guarded::Refused).kind, BodyKind::Nothing);
    assert_eq!(
        super::shown(Guarded::Answered(super::RenderedBody::nothing())).kind,
        BodyKind::Nothing,
        "what the reading itself said is what the reader is told"
    );
}

/// A message written only in markup is still something an answer can quote.
///
/// Most of what a person is sent carries no text part, so refusing here opened an answer on a
/// blank page with nothing on it to say why — indistinguishable from a window that had failed.
/// The conversion happens behind the guards every whole message goes through, and what comes back
/// is words rather than markup.
#[test]
fn a_message_written_only_in_markup_can_still_be_quoted() {
    let raw = b"From: a@x.test\r\n\
                To: b@x.test\r\n\
                Subject: Tuesday\r\n\
                Content-Type: text/html; charset=utf-8\r\n\
                \r\n\
                <html><body><p>The first line.</p><p>And the second.</p></body></html>";

    let quoted = super::text_of(raw).expect("a message with words in it has words to quote");

    assert!(quoted.contains("The first line."), "{quoted}");
    assert!(quoted.contains("And the second."), "{quoted}");
    assert!(
        !quoted.contains("<p>"),
        "markup reached the answer: {quoted}"
    );
    assert!(
        quoted
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count()
            > 1,
        "two paragraphs arrived as one run of words: {quoted}"
    );
}

/// A message with a text part is quoted from it, and never from a conversion.
#[test]
fn a_message_that_wrote_text_is_quoted_from_what_it_wrote() {
    let raw = named("html-and-text.eml");
    let quoted = super::text_of(&raw).expect("it wrote a text part");

    assert!(!quoted.contains('<'), "{quoted}");
}

#[test]
fn a_message_with_no_text_part_says_so_rather_than_showing_markup_as_words() {
    let raw = html_message(&["<p>Only HTML here.</p>"]);

    assert_eq!(
        body_of(
            &raw,
            Viewing {
                form: BodyForm::Text,
                pictures: Pictures::Blocked
            }
        )
        .kind,
        BodyKind::Nothing
    );
}

#[test]
fn text_is_shown_as_text_and_not_as_the_markup_it_spells() {
    let raw = message(
        &[
            "From: someone@example.com",
            "Subject: A message",
            "Content-Type: text/plain; charset=utf-8",
        ],
        &["I wrote <b>bold</b> and <script>alert(1)</script> on purpose."],
    );

    let rendered = body_of(&raw, Viewing::plainly());

    assert_eq!(rendered.kind, BodyKind::PlainText);
    assert!(rendered.html.contains("&lt;b&gt;bold&lt;/b&gt;"));
    nothing_that_runs(&rendered.html, "plain text");
}

#[test]
fn an_address_in_plain_text_is_a_link_behind_a_name() {
    // The way an address in markup is: what is in the page is a name, and following it reaches
    // this application, which asks the person before anything leaves the machine.
    let raw = message(
        &[
            "From: zeta@example.com",
            "Subject: Three addresses",
            "Content-Type: text/plain; charset=utf-8",
        ],
        &["Read https://example.com/q?a=1&b=2 and www.example.org, then mailto:kappa@example.net."],
    );

    let rendered = body_of(&raw, Viewing::plainly());

    let opened: Vec<&str> = rendered
        .links
        .iter()
        .map(|link| link.url.as_str())
        .collect();
    assert_eq!(
        opened,
        [
            "https://example.com/q?a=1&b=2",
            "https://www.example.org/",
            "mailto:kappa@example.net"
        ]
    );
    for link in &rendered.links {
        assert!(
            rendered
                .html
                .contains(&format!("<a href=\"/{LINKS}/{}\">", link.name)),
            "{}",
            rendered.html
        );
    }
    // The address as it was written, escaped as the text around it is, and nothing between the
    // links changed.
    assert!(
        rendered
            .html
            .contains(">https://example.com/q?a=1&amp;b=2</a> and <a"),
        "{}",
        rendered.html
    );
    assert!(rendered.html.contains("</a>, then <a"), "{}", rendered.html);
    assert!(
        !rendered.html.contains("href=\"https:"),
        "{}",
        rendered.html
    );
    nothing_that_runs(&rendered.html, "plain text with links");
}

#[test]
fn the_text_of_a_message_offers_its_addresses_as_links_too() {
    let raw = message(
        &[
            "From: zeta@example.com",
            "Subject: Both",
            "MIME-Version: 1.0",
            "Content-Type: multipart/alternative; boundary=\"between\"",
        ],
        &[
            "--between",
            "Content-Type: text/plain; charset=utf-8",
            "",
            "See https://example.com/written.",
            "--between",
            "Content-Type: text/html; charset=utf-8",
            "",
            "<p>See <a href=\"https://example.com/marked\">this</a>.</p>",
            "--between--",
        ],
    );

    let text = body_of(
        &raw,
        Viewing {
            form: BodyForm::Text,
            pictures: Pictures::Blocked,
        },
    );

    assert_eq!(text.kind, BodyKind::PlainText);
    assert_eq!(
        text.links
            .iter()
            .map(|link| link.url.as_str())
            .collect::<Vec<_>>(),
        ["https://example.com/written"]
    );
}

#[test]
fn the_source_shows_its_addresses_as_the_text_they_are() {
    // The source is the message as it arrived, and an address in it is part of what arrived.
    let raw = message(
        &[
            "From: zeta@example.com",
            "Subject: A link",
            "Content-Type: text/plain; charset=utf-8",
        ],
        &["Read https://example.com/a now."],
    );

    let source = body_of(
        &raw,
        Viewing {
            form: BodyForm::Source,
            pictures: Pictures::Blocked,
        },
    );

    assert!(source.links.is_empty());
    assert!(!source.html.contains("<a "), "{}", source.html);
    assert!(source.html.contains("https://example.com/a"));
}

#[test]
fn the_html_part_is_preferred_over_the_text_one() {
    let raw = named("html-and-text.eml");

    assert_eq!(body_of(&raw, Viewing::plainly()).kind, BodyKind::Html);
}

#[test]
fn a_size_the_index_recorded_decides_before_the_message_is_fetched() {
    assert!(!too_large_to_read(Some(0)));
    assert!(!too_large_to_read(Some(LARGEST_MESSAGE as u64)));
    assert!(too_large_to_read(Some(LARGEST_MESSAGE as u64 + 1)));
    // A message the index never measured is fetched and meets the limit one step later,
    // rather than being refused on a number nobody has.
    assert!(!too_large_to_read(None));
}

#[test]
fn a_message_larger_than_this_build_reads_is_refused_rather_than_parsed() {
    let mut raw = html_message(&["<p>The beginning.</p>"]);
    raw.resize(LARGEST_MESSAGE + 1, b'.');

    assert_eq!(body_of(&raw, Viewing::plainly()).kind, BodyKind::Nothing);
}

#[test]
fn nothing_at_all_is_nothing_rather_than_an_empty_document() {
    assert_eq!(body_of(b"", Viewing::plainly()).kind, BodyKind::Nothing);
    assert_eq!(
        body_of(b"not a message", Viewing::plainly()).kind,
        BodyKind::Nothing
    );
}

#[test]
fn a_body_longer_than_this_build_shows_is_cut_and_says_so() {
    let long = "a".repeat(LARGEST_BODY + 1000);
    let raw = html_message(&[&format!("<p>{long}</p>")]);

    let rendered = body_of(&raw, Viewing::plainly());

    assert!(rendered.truncated);
    assert!(rendered.html.len() <= LARGEST_BODY + 64);
}

#[test]
fn cutting_a_body_never_cuts_a_character_in_half() {
    // Every multi-byte character would otherwise be a chance to produce something that is not
    // text at all, at a length an attacker picks.
    let text = "ζ".repeat(100);

    for at in 0..text.len() {
        let cut = cut_at_character(&text, at);
        assert!(text.starts_with(cut));
    }
}

#[test]
fn a_closing_tag_that_matches_nothing_does_not_undo_the_count() {
    // The bypass this closes: `<div></b>` repeated walks a naive counter between one and zero
    // for ever, while the tree really does get one level deeper each time. A parser drops the
    // `</b>`; so does this.
    let walked = "<div></b>".repeat(200);

    assert!(nesting_beyond(&walked, 100));
}

#[test]
fn a_comment_that_ends_the_other_way_does_not_hide_the_document_behind_it() {
    // A parser ends a comment at `--!>` as well as at `-->`. A scan that knew only the first
    // would read the whole document as a comment and count nothing in it.
    let hidden = format!(
        "<!-- --!>{}deep{}",
        "<div>".repeat(200),
        "</div>".repeat(200)
    );

    assert!(nesting_beyond(&hidden, 100));
}

#[test]
fn a_comment_nobody_closed_is_refused_rather_than_read_past() {
    // This once said the opposite, and said it about the guard rather than about a
    // parser: a comment nobody closed swallows the rest of the document, so counting nothing
    // after it looked like agreeing with what a parser would do. It is not the same thing. A
    // parser reading the rest as a comment renders nothing of it; this scan reading it that way
    // hands the whole document to a sanitizer unexamined, and five bytes in front of a body
    // were enough to arrange that.
    let swallowed = format!("<!--{}", "<div>".repeat(500));

    assert!(nesting_beyond(&swallowed, 100));
}

#[test]
fn ordinary_mail_is_not_mistaken_for_a_document_that_is_not_one() {
    // The mistake that costs a reader something: a newsletter shown as plain text because a
    // scan counted its paragraphs as depth.
    for name in [
        "newsletter-heavy-layout.eml",
        "html-and-text.eml",
        "hostile-remote-content.eml",
    ] {
        let rendered = body_of(&named(name), Viewing::plainly());

        assert_eq!(rendered.kind, BodyKind::Html, "{name} was shown as text");
    }

    // The shapes real mail is full of: paragraphs and line breaks that never close.
    assert!(!nesting_beyond(&"<p>a line".repeat(400), 100));
    assert!(!nesting_beyond(&"<br>".repeat(400), 100));
    assert!(!nesting_beyond(&"<li>an item".repeat(400), 100));
    assert!(!nesting_beyond(
        &"<tr><td>a cell</td></tr>".repeat(200),
        100
    ));
}

#[test]
fn nesting_is_counted_without_parsing_anything() {
    assert!(!nesting_beyond("<p>one</p><p>two</p>", 2));
    assert!(!nesting_beyond("<div><p>two deep</p></div>", 2));
    assert!(nesting_beyond("<div><div><div>three</div></div></div>", 2));
    // A comment and a closing tag are not an opening one.
    assert!(!nesting_beyond("<!-- <div><div><div> --></p>", 2));
}

/// What a message carries beside its body, listed from real messages in the corpus.
mod files_a_message_carries {
    use super::*;

    #[test]
    fn a_message_with_an_invoice_lists_it_by_name_and_size() {
        let rendered = body_of(&named("with-attachment.eml"), Viewing::plainly());

        assert_eq!(rendered.attachments.len(), 1);
        let file = &rendered.attachments[0];
        assert_eq!(file.name, "invoice.pdf");
        assert!(file.bytes > 0, "a file with no bytes is not a file");
    }

    #[test]
    fn a_picture_a_style_meant_to_draw_is_counted_rather_than_dropped_in_silence() {
        // The one way a picture used to leave without a trace. The refusal itself is right — a
        // background fetched from a host tells its owner the message was opened, exactly as a
        // picture does — but a reader was shown an empty box where a logo belongs and told
        // nothing, with no number and nothing to press.
        let raw = message(
            &[
                "From: zeta@example.com",
                "To: kappa@example.net",
                "Subject: A signature drawn by a style",
                "MIME-Version: 1.0",
                "Content-Type: text/html; charset=utf-8",
            ],
            &[
                "<p>Best,</p>",
                "<div style=\"width:160px;height:60px;background-image:url(https://example.net/logo.png)\"></div>",
            ],
        );

        let rendered = body_of(&raw, Viewing::plainly());

        assert_eq!(
            rendered.remote_blocked, 1,
            "the picture the style draws was not counted, so nothing on screen names it"
        );
        // Counted with the ones a press can still get, because now it can: the address was taken
        // out and this application will fetch it the moment the reader asks.
        assert_eq!(rendered.remote_not_shown, 0);
        assert!(
            !rendered.html.contains("example.net"),
            "an address a message wrote is still in the document"
        );
    }

    #[test]
    fn a_message_with_a_document_and_a_photograph_lists_both() {
        // A corpus of messages carrying one file says nothing about the second. A listing that
        // stopped after the first, or kept one entry per name, or passed over a part because its
        // kind was a picture, satisfied every other test here.
        let rendered = body_of(&named("two-files.eml"), Viewing::plainly());

        assert_eq!(rendered.attachments.len(), 2);

        let invoice = &rendered.attachments[0];
        assert_eq!(invoice.name, "invoice.pdf");
        assert_eq!(invoice.bytes, 78);

        let photograph = &rendered.attachments[1];
        assert_eq!(photograph.name, "photo.jpg");
        assert_eq!(photograph.bytes, 331);

        // Each is asked for by where it sits, so two files sharing a name would still be two.
        assert_ne!(invoice.at, photograph.at);
    }

    #[test]
    fn either_of_the_two_comes_back_by_where_it_sits() {
        // The adapter itself, not the fake the use case is tested against: what a person saves is
        // the part they pressed, and a build that answered the first one to every question would
        // pass every test that only ever asks for one.
        use crate::ports::message_parts::MessageParts;

        let raw = named("two-files.eml");
        let rendered = body_of(&raw, Viewing::plainly());
        let listed = &rendered.attachments;

        let (name, bytes) = MimeParts
            .file_at(&raw, listed[1].at)
            .expect("the photograph is where the listing said");
        assert_eq!(name, "photo.jpg");
        assert_eq!(bytes.len(), 331);
        assert_eq!(&bytes[..2], b"\xff\xd8", "and it is the photograph itself");

        let (name, bytes) = MimeParts
            .file_at(&raw, listed[0].at)
            .expect("the invoice is where the listing said");
        assert_eq!(name, "invoice.pdf");
        assert_eq!(bytes.len(), 78);
        assert!(bytes.starts_with(b"%PDF"));
    }

    #[test]
    fn the_bytes_are_not_carried_with_the_listing() {
        // A message with four photographs in it is four photographs nobody has asked for yet. The
        // listing is a name and a size; the bytes are fetched when somebody asks to save one.
        let rendered = body_of(&named("with-attachment.eml"), Viewing::plainly());

        // The whole rendered body must not have grown by the size of what it lists.
        let listed: u64 = rendered.attachments.iter().map(|file| file.bytes).sum();
        assert!(listed > 0);
        assert!(
            rendered.html.len() < 4096,
            "the body is the body, not the body plus its attachments"
        );
    }

    #[test]
    fn a_name_written_to_read_backwards_is_offered_as_what_it_is() {
        // The corpus message is named with a right-to-left override so `holiday<RLO>gnp.exe` reads
        // as `holidayexe.png`. The override is a control character, and a control character is not
        // part of a name this application hands to a save panel.
        let rendered = body_of(&named("hostile-rtlo-filename.eml"), Viewing::plainly());

        assert_eq!(rendered.attachments.len(), 1);
        let offered = &rendered.attachments[0].name;

        // Named exactly. Asserting only that it ends `.exe` passed while the override was still
        // in the middle of the name — which is the whole attack, and which a raw filename would
        // also have satisfied.
        assert_eq!(offered, "holidaygnp.exe");
        assert!(!offered.contains('\u{202E}'));
    }

    #[test]
    fn a_picture_the_body_shows_is_not_also_a_file_beside_it() {
        // Listed twice, it would be a photograph somebody saves and then finds already in the
        // message. A part the body points at belongs to the body.
        let rendered = body_of(&named("inline-image.eml"), Viewing::plainly());

        assert!(
            rendered.attachments.is_empty(),
            "an inline picture is part of the body: {:?}",
            rendered.attachments
        );
        assert!(!rendered.parts.is_empty(), "and it is still shown");
    }

    #[test]
    fn a_message_that_is_only_text_carries_nothing() {
        let rendered = body_of(&named("plain-text.eml"), Viewing::plainly());

        assert!(rendered.attachments.is_empty());
    }

    #[test]
    fn a_table_a_note_and_a_page_written_in_text_are_listed_as_the_files_they_are() {
        // The parser reads every text part as text, whatever it is for, and a listing that took
        // only bytes passed a spreadsheet and a calendar invitation over as though the message
        // carried nothing: not offered to save, and not passed on with it.
        use crate::ports::message_parts::MessageParts;

        let raw = named("text-files.eml");
        let rendered = body_of(&raw, Viewing::plainly());

        assert_eq!(
            rendered
                .attachments
                .iter()
                .map(|file| (file.name.as_str(), file.bytes))
                .collect::<Vec<_>>(),
            vec![("figures.csv", 35), ("notes.txt", 60), ("summary.html", 25)]
        );
        assert!(
            !rendered.html.contains("Totals"),
            "the page attached to the message was shown as the message"
        );
        let (name, bytes) = MimeParts
            .file_at(&raw, rendered.attachments[0].at)
            .expect("the table is where the listing said");
        assert_eq!(name, "figures.csv");
        assert_eq!(bytes, b"month,total\nOctober,12\nNovember,15\n");
    }

    /// A note with a message carried inside it, as a mail app passes one on as a file, and the
    /// lines of the message it carries.
    fn carrying_a_message(disposition: &str) -> (Vec<u8>, Vec<u8>) {
        let carried = [
            "From: Kappa Vandel <kappa@example.net>",
            "To: zeta@example.com",
            "Subject: About Tuesday",
            "",
            "Shall we meet at noon?",
        ]
        .join("\r\n");
        let raw = [
            "From: zeta@example.com",
            "To: sigma@example.org",
            "Subject: Fwd: About Tuesday",
            "MIME-Version: 1.0",
            "Content-Type: multipart/mixed; boundary=\"m4\"",
            "",
            "--m4",
            "Content-Type: text/plain; charset=utf-8",
            "",
            "See below.",
            "--m4",
            "Content-Type: message/rfc822",
            disposition,
            "",
            &carried,
            "--m4--",
            "",
        ]
        .join("\r\n");
        (raw.into_bytes(), carried.into_bytes())
    }

    #[test]
    fn a_message_carried_inside_is_a_file_named_by_its_subject() {
        // What a mail app sends when it passes a message on as a file. Read as nothing, it was a
        // note saying "see below" with nothing below it.
        use crate::ports::message_parts::MessageParts;

        let (raw, carried) = carrying_a_message("Content-Disposition: attachment");
        let rendered = body_of(&raw, Viewing::plainly());

        assert_eq!(
            rendered
                .attachments
                .iter()
                .map(|file| file.name.as_str())
                .collect::<Vec<_>>(),
            vec!["About Tuesday.eml"]
        );
        let (_, bytes) = MimeParts
            .file_at(&raw, rendered.attachments[0].at)
            .expect("the message is where the listing said");
        assert_eq!(bytes, carried);
        assert_eq!(rendered.attachments[0].bytes, carried.len() as u64);
    }

    #[test]
    fn a_message_carried_inside_keeps_the_name_it_was_given_and_ends_as_a_message() {
        let named = |disposition: &str| {
            let (raw, _) = carrying_a_message(disposition);
            body_of(&raw, Viewing::plainly())
                .attachments
                .into_iter()
                .map(|file| file.name)
                .collect::<Vec<_>>()
        };

        assert_eq!(
            named("Content-Disposition: attachment; filename=\"Lunch.EML\""),
            vec!["Lunch.EML"]
        );
        assert_eq!(
            named("Content-Disposition: attachment; filename=\"Lunch\""),
            vec!["Lunch.eml"]
        );
        assert_eq!(
            named("Content-Disposition: inline; filename=\"../../Lunch\""),
            vec!["Lunch.eml"],
            "a name that points somewhere else names a file and nothing more"
        );
    }

    #[test]
    fn a_message_carried_encoded_is_the_message_it_decodes_to() {
        // The standard says a carried message goes as written; some mail apps encode it all the
        // same. Read in place, its bytes would be the encoding rather than the message.
        use crate::ports::message_parts::MessageParts;

        let raw = [
            "From: zeta@example.com",
            "Subject: Fwd: About Tuesday",
            "MIME-Version: 1.0",
            "Content-Type: multipart/mixed; boundary=\"m5\"",
            "",
            "--m5",
            "Content-Type: text/plain; charset=utf-8",
            "",
            "See below.",
            "--m5",
            "Content-Type: message/rfc822",
            "Content-Transfer-Encoding: base64",
            "",
            "RnJvbTogS2FwcGEgVmFuZGVsIDxrYXBwYUBleGFtcGxlLm5ldD4NClN1YmplY3Q6IEFib3V0IFR1ZXNkYXkNCg0KU2hhbGwgd2U/",
            "--m5--",
            "",
        ]
        .join("\r\n")
        .into_bytes();
        let rendered = body_of(&raw, Viewing::plainly());

        assert_eq!(rendered.attachments.len(), 1);
        assert_eq!(rendered.attachments[0].name, "About Tuesday.eml");
        let (_, bytes) = MimeParts
            .file_at(&raw, rendered.attachments[0].at)
            .expect("the message is where the listing said");
        assert_eq!(
            bytes,
            b"From: Kappa Vandel <kappa@example.net>\r\nSubject: About Tuesday\r\n\r\nShall we?"
                .to_vec()
        );
    }

    #[test]
    fn a_message_carried_inside_goes_with_one_passed_on() {
        use crate::ports::message_parts::MessageParts;

        let (raw, carried) = carrying_a_message("Content-Disposition: attachment");

        let passed = MimeParts.passed_on(&raw).expect("nothing else is reading");

        assert_eq!(
            passed.files,
            vec![("About Tuesday.eml".to_owned(), carried)]
        );
        assert_eq!(passed.text, "See below.");
    }

    #[test]
    fn a_message_whose_text_is_named_like_a_file_is_still_read_as_its_text() {
        // Some clients name the one part a message has. It is what the message says, and
        // listing it would offer it as a file beside itself.
        let raw = message(
            &[
                "From: zeta@example.com",
                "To: kappa@example.net",
                "Subject: A note",
                "MIME-Version: 1.0",
                "Content-Type: text/plain; charset=utf-8; name=\"note.txt\"",
                "Content-Disposition: inline; filename=\"note.txt\"",
            ],
            &["Back on Monday."],
        );

        assert!(body_of(&raw, Viewing::plainly()).attachments.is_empty());
    }

    #[test]
    fn a_file_the_parser_could_not_decode_is_not_listed_as_text() {
        // What it reads such a part as is the encoding it could not undo, which is not what was
        // sent: saved or passed on, it would be a file of the wrong bytes under the right name.
        let raw = message(
            &[
                "From: zeta@example.com",
                "To: kappa@example.net",
                "Subject: A broken file",
                "MIME-Version: 1.0",
                "Content-Type: multipart/mixed; boundary=\"m3\"",
            ],
            &[
                "--m3",
                "Content-Type: text/plain; charset=utf-8",
                "",
                "The chart is attached.",
                "--m3",
                "Content-Type: image/png; name=\"chart.png\"",
                "Content-Disposition: attachment; filename=\"chart.png\"",
                "Content-Transfer-Encoding: base64",
                "",
                "iVBORw0KGgo=====\u{2603}\u{2603}%%%%",
                "--m3--",
            ],
        );
        let parsed = mail_parser::MessageParser::default()
            .parse(&raw)
            .expect("a message");
        assert!(
            parsed.parts.iter().any(|part| part.is_encoding_problem),
            "the fixture no longer makes the parser give up on a part"
        );

        assert!(body_of(&raw, Viewing::plainly())
            .attachments
            .iter()
            .all(|file| !matches!(
                parsed.parts.get(file.at as usize).map(|part| &part.body),
                Some(PartType::Text(_))
            )));
    }

    #[test]
    fn the_text_a_message_reads_as_is_not_also_a_file() {
        // The other side of the rule above: a text part the message is made of is the message,
        // and listing it would offer somebody what they are reading as a file beside it.
        for name in ["plain-text.eml", "html-and-text.eml", "text-files.eml"] {
            let rendered = body_of(&named(name), Viewing::plainly());

            assert!(
                rendered
                    .attachments
                    .iter()
                    .all(|file| file.name != "attachment"),
                "{name} listed its own text as a file: {:?}",
                rendered.attachments
            );
        }
        assert!(body_of(&named("html-and-text.eml"), Viewing::plainly())
            .attachments
            .is_empty());
    }
}

/// A message read to be passed on to somebody else: what it says, and every file it carries.
mod passing_a_message_on {
    use super::*;
    use crate::ports::message_parts::{MessageParts, PassedOn};

    fn passed(raw: &[u8]) -> PassedOn {
        MimeParts
            .passed_on(raw)
            .expect("nothing else is reading a message")
    }

    fn names(passed: &PassedOn) -> Vec<&str> {
        passed.files.iter().map(|(name, _)| name.as_str()).collect()
    }

    #[test]
    fn every_file_beside_its_text_goes_with_it_in_the_order_it_carries_them() {
        let passed = passed(&named("two-files.eml"));

        assert_eq!(passed.text, "Both of them are attached.");
        assert_eq!(names(&passed), vec!["invoice.pdf", "photo.jpg"]);
        assert!(passed.files[0].1.starts_with(b"%PDF"));
        assert_eq!(passed.files[0].1.len(), 78);
        assert_eq!(
            &passed.files[1].1[..2],
            b"\xff\xd8",
            "and the photograph itself"
        );
        assert_eq!(passed.files[1].1.len(), 331);
    }

    #[test]
    fn a_picture_drawn_inside_the_message_goes_as_a_file() {
        // Passed on as text, the message draws nothing: a chart somebody saw in it would be lost
        // on the way rather than go beside it.
        let passed = passed(&named("inline-image.eml"));

        assert_eq!(names(&passed), vec!["chart.png"]);
        assert!(passed.files[0].1.starts_with(b"\x89PNG"));
        assert!(
            passed.text.contains("Here is the chart we talked about"),
            "the words of the markup were not passed on: {:?}",
            passed.text
        );
        assert!(!passed.text.contains('<'), "markup was passed on as text");
    }

    #[test]
    fn files_written_in_text_go_too_and_the_text_is_only_the_message() {
        let passed = passed(&named("text-files.eml"));

        assert_eq!(
            passed.text,
            "The figures are in the table, and the notes beside them."
        );
        assert_eq!(
            passed.files,
            vec![
                (
                    "figures.csv".to_owned(),
                    b"month,total\nOctober,12\nNovember,15\n".to_vec()
                ),
                (
                    "notes.txt".to_owned(),
                    "Ask Kappa about the totals \u{2014} the November one looks high.\n"
                        .as_bytes()
                        .to_vec()
                ),
                (
                    "summary.html".to_owned(),
                    b"<p>Totals: <b>27</b></p>\n".to_vec()
                ),
            ]
        );
    }

    #[test]
    fn its_pictures_and_files_go_in_the_order_it_carries_them() {
        // A chart drawn in the text and an invoice after it go as the chart and then the invoice,
        // not as every file first and every picture after.
        let raw = message(
            &[
                "From: zeta@example.com",
                "To: kappa@example.net",
                "Subject: The chart and the invoice",
                "MIME-Version: 1.0",
                "Content-Type: multipart/mixed; boundary=\"outer\"",
            ],
            &[
                "--outer",
                "Content-Type: multipart/related; boundary=\"inner\"",
                "",
                "--inner",
                "Content-Type: text/html; charset=utf-8",
                "",
                "<p>The chart:</p><img src=\"cid:chart@example.com\">",
                "--inner",
                "Content-Type: image/png",
                "Content-ID: <chart@example.com>",
                "Content-Disposition: inline; filename=\"chart.png\"",
                "Content-Transfer-Encoding: base64",
                "",
                "iVBORw0KGgo=",
                "--inner--",
                "--outer",
                "Content-Type: application/pdf; name=\"invoice.pdf\"",
                "Content-Disposition: attachment; filename=\"invoice.pdf\"",
                "Content-Transfer-Encoding: base64",
                "",
                "JVBERi0xLjQK",
                "--outer--",
            ],
        );

        assert_eq!(names(&passed(&raw)), vec!["chart.png", "invoice.pdf"]);
    }

    #[test]
    fn its_lines_end_as_a_text_box_ends_them() {
        // A line ending in a return as well is one a window rewrites the moment it shows it, and
        // the text it was handed is then not the text on screen.
        let raw = [
            "From: zeta@example.com",
            "To: kappa@example.net",
            "Subject: Three lines",
            "",
            "One,",
            "two,",
            "three.",
        ]
        .join("\r\n");

        assert_eq!(passed(raw.as_bytes()).text, "One,\ntwo,\nthree.");
    }

    #[test]
    fn a_message_of_markup_alone_goes_as_its_words() {
        let passed = passed(&html_message(&[
            "<p>The <b>first</b> line.</p>",
            "<p>The second.</p>",
        ]));

        assert!(passed.text.contains("first"), "{:?}", passed.text);
        assert!(passed.text.contains("The second."), "{:?}", passed.text);
        assert!(!passed.text.contains('<'), "markup was passed on as text");
        assert!(passed.files.is_empty());
    }

    #[test]
    fn a_name_written_to_read_backwards_goes_as_what_it_is() {
        let passed = passed(&named("hostile-rtlo-filename.eml"));

        assert_eq!(names(&passed), vec!["holidaygnp.exe"]);
    }

    #[test]
    fn a_message_nowhere_could_read_is_not_passed_on_as_one_that_says_nothing() {
        // "Could not look" is not said as "there is nothing to see": passed on empty, it would go
        // to somebody as a message saying nothing.
        use super::super::files::passed_as;
        use super::super::guards::Guarded;

        let read = PassedOn {
            text: "Shall we?".to_owned(),
            files: vec![("invoice.pdf".to_owned(), b"%PDF".to_vec())],
        };

        assert_eq!(passed_as(Guarded::Busy), None);
        assert_eq!(passed_as(Guarded::Refused), Some(PassedOn::default()));
        assert_eq!(passed_as(Guarded::Answered(read.clone())), Some(read));
    }

    #[test]
    fn nothing_at_all_is_passed_on_as_nothing() {
        // What the reader shows of it too. Refused rather than read, which is not the machine
        // being busy and is not said as that.
        assert_eq!(MimeParts.passed_on(b""), Some(PassedOn::default()));
    }
}

#[test]
fn an_address_folded_across_lines_is_read_as_the_address_that_was_written() {
    // Mail folds. A long address arrives wrapped, and what the parser hands over has the break and
    // the next line's indent inside it. Sent as it stands, no host answers -- and the reader was
    // told the picture could not be fetched, about one nobody had ever asked for correctly.
    let raw = html_message(&[
        "<p>Look</p>",
        "<img src=\"https://pictures.example.com/very/long/path/",
        "picture.png\">",
        "<a href=\"https://example.com/very/long/path/",
        "page\">read on</a>",
    ]);

    let shown = body_of(
        &raw,
        Viewing {
            form: BodyForm::Sent,
            pictures: Pictures::Shown,
        },
    );

    assert_eq!(shown.remote.len(), 1, "the picture was not even recognised");
    assert_eq!(
        shown.remote[0].url, "https://pictures.example.com/very/long/path/picture.png",
        "the address still carries what the wrapping put in it"
    );
    assert_eq!(shown.links.len(), 1, "the link was not recognised");
    assert_eq!(
        shown.links[0].url, "https://example.com/very/long/path/page",
        "a person would be asked about an address nobody wrote"
    );
}

#[test]
fn a_real_message_of_inline_drawings_says_what_is_true_about_them() {
    // The message this came from showed a person "10 pictures are not where the message says they
    // are" and a button to fetch them again. Every one of the ten was inside the message, of a
    // kind this build refused to draw -- so the button could never do anything, and the sentence
    // was about a fault that was not there. They are drawn now.
    let raw = named("inline-pictures-not-drawn-here.eml");

    let shown = body_of(
        &raw,
        Viewing {
            form: BodyForm::Sent,
            pictures: Pictures::Shown,
        },
    );

    assert_eq!(shown.parts.len(), 3, "the drawings are not shown");
    assert_eq!(shown.remote_not_shown, 0);
    assert_eq!(shown.remote_blocked, 0, "nothing here is somewhere else");
    assert!(
        shown.remote.is_empty(),
        "nothing to fetch: they are carried"
    );
}

#[test]
fn a_picture_written_into_the_body_is_served_from_the_body() {
    // Messages carry small pictures this way -- the stars of a rating, an icon in a signature.
    // The sanitizer dropped the whole element before anything could count it: no picture, no
    // notice, and nothing on screen to say why. It is the safest kind there is -- the bytes are
    // already in hand, nothing is fetched, and nobody learns the message was opened.
    let raw = html_message(&[
        "<p>Look</p>",
        "<img src=\"data:image/png;base64,iVBORw0KGgo=\" width=\"20\" height=\"20\">",
    ]);

    let shown = body_of(
        &raw,
        Viewing {
            form: BodyForm::Sent,
            pictures: Pictures::Blocked,
        },
    );

    assert_eq!(shown.parts.len(), 1, "the picture was thrown away");
    assert_eq!(shown.parts[0].content_type, "image/png");
    assert_eq!(shown.parts[0].bytes, b"\x89PNG\r\n\x1a\n");
    assert!(
        shown
            .html
            .contains(&format!("src=\"/{}\"", shown.parts[0].name)),
        "the body does not point at it: {}",
        shown.html
    );
    // Nothing was fetched and nothing is owed: this is not a picture somewhere else.
    assert_eq!(shown.remote_blocked, 0);
    assert_eq!(shown.remote_not_had, 0);
    assert!(shown.remote.is_empty());
}

#[test]
fn a_picture_written_into_the_body_is_served_as_what_it_is() {
    // The kind is decided by the allowlist, never by what the message wrote -- and anything that
    // is not a picture this build serves goes, taking its element with it.
    for written in [
        "data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==",
        "data:application/octet-stream;base64,AAAA",
        "data:image/png;base64,",
        "data:image/png;base64,!!!!",
        "data:nonsense",
    ] {
        let raw = html_message(&[&format!("<img src=\"{written}\">")]);

        let shown = body_of(
            &raw,
            Viewing {
                form: BodyForm::Sent,
                pictures: Pictures::Shown,
            },
        );

        assert!(shown.parts.is_empty(), "{written} was served as a picture");
        nothing_that_runs(&shown.html, written);
    }
}

#[test]
fn a_picture_written_into_the_body_survives_the_folding_a_message_puts_in_it() {
    // Base64 in a message arrives wrapped. Read as it stands it is not the picture anybody wrote.
    let raw = html_message(&["<img src=\"data:image/png;base64,iVBO", "Rw0K", "Ggo=\">"]);

    let shown = body_of(
        &raw,
        Viewing {
            form: BodyForm::Sent,
            pictures: Pictures::Blocked,
        },
    );

    assert_eq!(shown.parts.len(), 1, "the folding was read as the picture");
    assert_eq!(shown.parts[0].bytes, b"\x89PNG\r\n\x1a\n");
}

#[test]
fn probe_three_real_messages() {
    for name in [
        "rating-stars-deep-in-tables",
        "signature-picture-inside-the-message",
        "newsletter-pictures-behind-conditionals",
    ] {
        let raw = std::fs::read(format!("../tests/fixtures/eml/{name}.eml"))
            .expect("the corpus holds it");
        for how in [Pictures::Blocked, Pictures::Shown] {
            let shown = body_of(
                &raw,
                Viewing {
                    form: BodyForm::Sent,
                    pictures: how,
                },
            );
            eprintln!(
                "PROBE {name} {how:?}: blocked={} not_had={} not_shown={} parts={} remote={} imgs={}",
                shown.remote_blocked,
                shown.remote_not_had,
                shown.remote_not_shown,
                shown.parts.len(),
                shown.remote.len(),
                shown.html.matches("<img").count()
            );
        }
    }
}

/// Three real messages, kept because each carried a picture that did not appear on screen.
///
/// What they have in common is where the pictures are: five levels of table down, inside a link,
/// behind an Outlook conditional, inside a `<figure>`. What a message wraps a picture in is the
/// message's business, and none of it may decide whether the picture is on screen.
#[test]
fn the_pictures_of_a_real_message_survive_being_read() {
    // (fixture, pictures the message points at somewhere else, pictures it carries in itself)
    for (name, elsewhere, carried) in [
        ("rating-stars-deep-in-tables", 10usize, 0usize),
        ("signature-picture-inside-the-message", 1, 1),
        ("newsletter-pictures-behind-conditionals", 5, 0),
    ] {
        let raw = std::fs::read(format!("../tests/fixtures/eml/{name}.eml"))
            .expect("the corpus holds it");

        // Nobody asked for them: every one is counted, so the reader is told how many are not on
        // screen and offered the press that changes it.
        let refused = body_of(
            &raw,
            Viewing {
                form: BodyForm::Sent,
                pictures: Pictures::Blocked,
            },
        );
        assert_eq!(
            refused.remote_blocked as usize, elsewhere,
            "{name}: the reader is told nothing about {elsewhere} pictures"
        );
        assert!(
            refused.remote.is_empty(),
            "{name}: fetched without being asked"
        );
        assert_eq!(
            refused.parts.len(),
            carried,
            "{name}: what it carries is not held"
        );

        // Asked for: every one is listed to be fetched, under a name the body points at.
        let asked = body_of(
            &raw,
            Viewing {
                form: BodyForm::Sent,
                pictures: Pictures::Shown,
            },
        );
        assert_eq!(
            asked.remote.len(),
            elsewhere,
            "{name}: not all of them are fetched"
        );
        assert_eq!(
            asked.remote_blocked, 0,
            "{name}: counted as refused after being asked for"
        );

        for pointed_at in &asked.remote {
            assert!(
                asked
                    .html
                    .contains(&format!("src=\"/{}\"", pointed_at.name)),
                "{name}: nothing in the body points at a picture it asked for"
            );
        }
        for part in &asked.parts {
            assert!(
                asked.html.contains(&format!("src=\"/{}\"", part.name)),
                "{name}: nothing in the body points at a picture it carries"
            );
        }

        // And the whole way to the frame: held, then served under the name the body wrote.
        let mut ready = asked;
        for pointed_at in &ready.remote.clone() {
            ready.parts.push(crate::ports::body::InlinePart {
                name: pointed_at.name.clone(),
                content_type: "image/png".to_owned(),
                bytes: b"\x89PNG\r\n\x1a\n".to_vec(),
            });
        }

        let held = super::serving::BodyBeingRead::default();
        let token = held.hold(&ready, None).expect("a name");
        let (page, _) = held.page(&token).expect("the page");

        assert_eq!(
            ready.parts.len(),
            elsewhere + carried,
            "{name}: not every picture reached the frame"
        );
        for part in &ready.parts {
            assert!(page.contains(&part.name), "{name}: the page lost a picture");
            assert!(
                held.part(&part.name).is_some(),
                "{name}: a picture the page points at is served with nothing"
            );
        }
    }
}

#[test]
fn a_message_keeps_the_stylesheet_it_was_written_with() {
    // Dropped it was shown as something other than what was written: a newsletter is a table whose
    // every rule lives in that one block, and without it the columns collapse.
    for (where_it_is, raw) in [
        (
            "in the head, where a message puts it",
            "<html><head><style>.card{border-radius:5px}</style></head><body><p class=\"card\">Text</p></body></html>",
        ),
        (
            "in the body",
            "<html><body><style>.card{border-radius:5px}</style><p class=\"card\">Text</p></body></html>",
        ),
    ] {
        let shown = body_of(
            &html_message(&[raw]),
            Viewing {
                form: BodyForm::Sent,
                pictures: Pictures::Blocked,
            },
        );

        assert!(
            shown.html.contains("border-radius:5px"),
            "{where_it_is}: the stylesheet was thrown away: {}",
            shown.html
        );
    }
}

#[test]
fn a_stylesheet_cannot_carry_anything_that_runs() {
    // The element holds raw text, so the question is whether a message can close it early and put
    // something else after. It cannot: the parser decides where it ends, and what follows is
    // cleaned like anything else.
    for raw in [
        "<style>a{}</style ><script>alert(1)</script><p>T</p>",
        "<style>a{content:'</style><script>alert(1)</script>'}</style><p>T</p>",
        "<style>@import 'x';</style><script>alert(1)</script><p>T</p>",
        "<template><style>x{}</style></template><p>T</p>",
        "<style onload=\"alert(1)\">a{}</style><p>T</p>",
    ] {
        let shown = body_of(
            &html_message(&[raw]),
            Viewing {
                form: BodyForm::Sent,
                pictures: Pictures::Blocked,
            },
        );

        nothing_that_runs(&shown.html, raw);
        assert!(
            !shown.html.contains("alert"),
            "something that runs survived: {}",
            shown.html
        );
    }
}

#[test]
fn the_only_url_a_style_may_carry_is_a_name_this_build_minted() {
    // The rewriting takes every address out and puts a name in its place. This is the rule that
    // holds when the rewriting did not happen — a shape it did not recognise, or a property that
    // draws nothing and is therefore never rewritten — and it is the whole of what stops a style
    // fetching anything: the one form allowed is `url(/` and sixty-four hex characters, which a
    // message cannot spell.
    let ours = "0".repeat(64);
    assert!(
        kept_style(&format!("background-image:url(/{ours})")).is_some(),
        "a name this build minted was refused"
    );

    for spelled in [
        // A property that draws nothing is never rewritten, so the address arrives here as the
        // message wrote it and this is what refuses it.
        "list-style:url(https://elsewhere.invalid/dot.png)",
        // A drawing property whose value this build cannot read as a token: nothing was taken out,
        // so what is left has to be refused whole.
        "background-image:url(https://elsewhere.invalid/logo.png",
        "background-image:url(\"https://elsewhere.invalid/logo.png",
        "background-image:url(\\68 ttps://elsewhere.invalid/logo.png)",
        // A name of the right shape is still not a name of ours. This one reaches the filter with
        // nothing rewritten because the property is not one that draws.
        "list-style:url(/00000000000000000000000000000000000000000000000000000000000000000)",
        "list-style:url(/not-hex-and-far-too-short)",
    ] {
        assert!(kept_style(spelled).is_none(), "{spelled} was let through");
    }

    // And where the rewriting did happen, what it wrote is a name or nothing — never the address.
    for spelled in [
        "background-image:url(https://elsewhere.invalid/logo.png)",
        "background-image:url(//elsewhere.invalid/logo.png)",
        "background:#fff url(https://elsewhere.invalid/x.png) no-repeat",
    ] {
        let kept = kept_style(spelled).unwrap_or_default();
        assert!(
            !kept.contains("elsewhere.invalid"),
            "{spelled} kept its address: {kept}"
        );
    }
}

#[test]
fn an_address_in_brackets_is_one_address_and_not_the_first_half_of_one() {
    // A bracket in a path is ordinary in real mail, and ending the token at the first `)` did two
    // things at once: this application went and fetched an address nobody had written, and the
    // half left behind re-opened a string that swallowed every rule after it.
    let mut asked: Vec<String> = Vec::new();
    let (served, _, _) = super::styling::pictures_pointed_at(
        "p{background-image:url(\"https://elsewhere.invalid/logo(1).png\")}td{padding:8px}",
        |address| {
            asked.push(address.to_owned());
            super::styling::ThePicture::NeverArriving
        },
    );

    assert_eq!(asked, ["https://elsewhere.invalid/logo(1).png"]);
    assert_eq!(served, "p{background-image:none}td{padding:8px}");
}

#[test]
fn a_stylesheet_that_says_it_ends_and_does_not_is_not_a_way_into_the_document() {
    // `</stylex` is CSS text and not an end tag — html5ever ends the element only where the name
    // does. A scan that stopped at the first `</style` came out of step with the document and went
    // on to rewrite a paragraph's own words, and in one shape turned an address written only as
    // prose into something this application fetched.
    // Three blocks, and the middle of the first one holds both halves of a tag. Ending the first
    // block at `</stylex` leaves the walk one tag out of step, and from there what it takes for a
    // stylesheet is the document between two real tags — a paragraph, whose words are then read as
    // declarations and whose address is fetched.
    let body = "<style>a{}</stylex b{}<style c{}</style>\
                <p>p{background-image:url(https://elsewhere.invalid/x.png)}</p>\
                <style>d{}</style>";
    let rendered = body_of(
        &html_message(&[body]),
        Viewing {
            form: BodyForm::Sent,
            pictures: Pictures::Shown,
        },
    );

    assert!(
        rendered
            .html
            .contains("<p>p{background-image:url(https://elsewhere.invalid/x.png)}</p>"),
        "text outside a stylesheet was rewritten: {}",
        rendered.html
    );
    assert!(
        rendered.remote.is_empty(),
        "an address written as prose was asked for"
    );
}

#[test]
fn a_style_naming_a_picture_the_message_carries_finds_it() {
    // A message names what it carries with `cid:`, and a background may name it the same way. The
    // `<img>` road has always found these; the style road looked for an address, found none, and
    // counted a picture that could never arrive — a logo the message was carrying all along, drawn
    // as an empty box with a sentence beside it saying nothing could be done.
    let raw = message(
        &[
            "From: someone@example.com",
            "Subject: A logo in a style",
            "MIME-Version: 1.0",
            "Content-Type: multipart/related; boundary=\"rel\"",
        ],
        &[
            "--rel",
            "Content-Type: text/html; charset=utf-8",
            "",
            "<style>p{background-image:url(cid:logo@example.com)}</style><p>T</p>",
            "",
            "--rel",
            "Content-Type: image/gif",
            "Content-ID: <logo@example.com>",
            "Content-Transfer-Encoding: base64",
            "",
            "R0lGODlhAQABAAAAACw=",
            "--rel--",
        ],
    );

    let rendered = body_of(&raw, Viewing::plainly());

    assert_eq!(
        rendered.parts.len(),
        1,
        "the picture it carries is not held"
    );
    assert_eq!(
        (rendered.remote_blocked, rendered.remote_not_shown),
        (0, 0),
        "a picture the message carried was counted as one that did not arrive"
    );
    assert!(
        rendered
            .html
            .contains(&format!("url(/{})", rendered.parts[0].name)),
        "the style does not point at what the message carries: {}",
        rendered.html
    );
}

#[test]
fn a_picture_named_in_a_way_this_build_cannot_read_is_fetched_by_nobody() {
    // The boundary, written down rather than left to be discovered. CSS names a picture in more
    // ways than this build reads: an escape spelling `url(` in another alphabet, a token that never
    // closes, `image-set` with a bare string. None of them is rewritten, and none of them is
    // guessed at either — a build that read an address one way and handed the engine another would
    // be counting one picture and serving a different one.
    //
    // What holds them is the policy the document is served under. `default-src 'none'` with
    // `img-src lyomsg:` means the frame reaches this application or nothing, so an address left in a
    // stylesheet is an address nothing goes to. That is the layer, and this is what says it is
    // still the layer for everything the rewriting does not recognise.
    for spelled in [
        "p{background-image:\\75 rl(https://elsewhere.invalid/a.png)}",
        "p{background-image:url(https://elsewhere.invalid/b.png}",
        "p{background-image:image-set(\"https://elsewhere.invalid/c.png\" 1x)}",
    ] {
        let rendered = body_of(
            &html_message(&[&format!("<style>{spelled}</style><p>T</p>")]),
            Viewing {
                form: BodyForm::Sent,
                pictures: Pictures::Shown,
            },
        );

        assert!(
            rendered.remote.is_empty(),
            "{spelled} became something this build goes and fetches"
        );
        assert_eq!(
            (rendered.remote_blocked, rendered.remote_not_shown),
            (0, 0),
            "{spelled} was counted as a picture"
        );
        assert!(
            !rendered.html.contains("url(/"),
            "{spelled} was rewritten into a name: {}",
            rendered.html
        );
    }
}

#[test]
fn a_url_in_a_property_that_draws_no_picture_is_nobody_s_picture() {
    // `@import`, a font and a cursor are all `url(` and none of them is a picture. Taking their
    // addresses out would send this application to fetch a stylesheet, a typeface or a pointer,
    // hold the answer as a picture, and tell the reader a picture did not arrive.
    let sheet = "@import url(https://elsewhere.invalid/more.css);\
                 @font-face{src:url(https://elsewhere.invalid/f.woff2)}\
                 a{cursor:url(https://elsewhere.invalid/c.png),auto}";
    let mut asked: Vec<String> = Vec::new();
    let (served, blocked, never) = super::styling::pictures_pointed_at(sheet, |address| {
        asked.push(address.to_owned());
        super::styling::ThePicture::NotAskedFor
    });

    assert!(asked.is_empty(), "asked about {asked:?}");
    assert_eq!((blocked, never), (0, 0));
    assert_eq!(served, sheet, "a value that draws nothing was rewritten");
}

#[test]
fn a_stylesheet_longer_than_this_build_walks_is_handed_on_as_it_came() {
    // The bound is on how much of a message's writing is read at all. Two megabytes of `url(` used
    // to be minutes of one render thread, and a thread that has begun cannot be stopped: four such
    // messages ended every message opened after them for the life of the process.
    let sheet = format!("p{{background-image:{}}}", "url(a),".repeat(20_000));
    assert!(sheet.len() > 64 * 1024);

    let began = std::time::Instant::now();
    let (served, blocked, never) =
        super::styling::pictures_pointed_at(&sheet, |_| super::styling::ThePicture::NotAskedFor);

    assert_eq!(served, sheet, "a stylesheet past the bound was walked");
    assert_eq!((blocked, never), (0, 0));
    assert!(began.elapsed() < std::time::Duration::from_secs(1));
}

#[test]
fn the_largest_stylesheet_this_build_walks_is_walked_in_a_moment() {
    // The bound is what makes the cost safe, so this is a test about the bound rather than about
    // the loop: whatever the walk does per token, the most of it a message can buy has to finish
    // inside a fraction of what `guards` allows a whole body. Raise the bound and this is what
    // says the arithmetic no longer holds.
    let sheet = format!("p{{background-image:{}}}", "url(a),".repeat(9_000));
    assert!(sheet.len() < super::styling::LONGEST_STYLESHEET);

    let began = std::time::Instant::now();
    let (_, blocked, _) =
        super::styling::pictures_pointed_at(&sheet, |_| super::styling::ThePicture::NotAskedFor);

    assert_eq!(blocked, 9_000);
    assert!(
        began.elapsed() < std::time::Duration::from_secs(1),
        "the largest stylesheet this build walks took {:?}",
        began.elapsed()
    );
}

#[test]
fn a_style_attribute_past_its_bound_is_dropped_before_anything_walks_it() {
    let long = format!("background-image:{}", "url(a),".repeat(1_000));
    assert!(long.len() > super::styling::LONGEST_STYLE);

    let mut asked = 0;
    let (kept, blocked, never) = super::styling::filtered_style(&long, |_| {
        asked += 1;
        super::styling::ThePicture::NotAskedFor
    });

    assert_eq!(kept, None);
    assert_eq!((asked, blocked, never), (0, 0, 0));
}

#[test]
fn what_is_counted_is_what_survived_the_filter() {
    // A picture in a declaration the filter then dropped was a number in front of somebody with
    // nothing behind it, and a press that could not help.
    let (kept, blocked, never) =
        super::styling::filtered_style("background-image:url(a)/*", |_| {
            super::styling::ThePicture::NotAskedFor
        });

    assert_eq!(kept, None, "a comment reached the output");
    assert_eq!(
        (blocked, never),
        (0, 0),
        "a dropped declaration was counted"
    );
}

#[test]
fn a_style_pointing_at_a_scheme_this_build_does_not_fetch_asks_nobody() {
    // The same list an `<img>` is held to. A style is not a second door into the network.
    for pointing in [
        "<div style=\"background-image:url(file:///etc/passwd)\"></div>",
        "<div style=\"background-image:url(javascript:alert(1))\"></div>",
        "<div style=\"background-image:url(ftp://elsewhere.invalid/x.png)\"></div>",
        "<style>.a{background-image:url(file:///etc/passwd)}</style><div class=\"a\"></div>",
    ] {
        let rendered = body_of(
            &html_message(&[pointing]),
            Viewing {
                form: BodyForm::Sent,
                pictures: Pictures::Shown,
            },
        );

        assert!(
            rendered.remote.is_empty(),
            "{pointing} became something this build goes and fetches"
        );
        assert!(
            !rendered.html.contains("passwd")
                && !rendered.html.contains("javascript")
                && !rendered.html.contains("ftp:"),
            "{pointing} survived into the document: {}",
            rendered.html
        );
    }
}

#[test]
fn a_picture_is_the_same_picture_whether_a_tag_or_a_style_draws_it() {
    // Three ways a message draws the same logo, and until now only the first of them worked: the
    // other two were refused where nothing counted them, so a reader saw an empty box, no number
    // and nothing to press. One road for all three.
    let asked_for = body_of(
        &html_message(&[
            "<style>.hero{background-image:url(https://elsewhere.invalid/logo.png)}</style>",
            "<div class=\"hero\"></div>",
            "<div style=\"background:#fff url(https://elsewhere.invalid/logo.png) no-repeat\"></div>",
            "<img src=\"https://elsewhere.invalid/logo.png\">",
        ]),
        Viewing {
            form: BodyForm::Sent,
            pictures: Pictures::Shown,
        },
    );

    assert_eq!(
        asked_for.remote.len(),
        3,
        "the three are not all asked for: {}",
        asked_for.html
    );

    // Each has its own name, and the document points at each of them.
    for pointed_at in &asked_for.remote {
        assert!(
            asked_for.html.contains(&format!("/{}", pointed_at.name)),
            "the document does not point at {}",
            pointed_at.name
        );
    }
    let names: std::collections::HashSet<&str> = asked_for
        .remote
        .iter()
        .map(|one| one.name.as_str())
        .collect();
    assert_eq!(names.len(), 3, "a name was reused between two pictures");

    // And not one of them is an address any more.
    assert!(
        !asked_for.html.contains("elsewhere.invalid"),
        "an address a message wrote is still in the document: {}",
        asked_for.html
    );
}

#[test]
fn a_picture_a_stylesheet_carries_in_itself_needs_nobody_asked() {
    // Written into the message, so nothing is fetched and no host learns it was opened — the same
    // answer an `<img>` carrying its own picture already gets, now given to a style as well.
    let carried = body_of(
        &html_message(&[
            "<div style=\"background-image:url(data:image/gif;base64,R0lGODlhAQABAAAAACw=)\"></div>",
        ]),
        Viewing::plainly(),
    );

    assert_eq!(carried.parts.len(), 1, "the picture it carries is held");
    assert_eq!(carried.remote_blocked, 0, "nobody was asked about it");
    assert!(
        carried
            .html
            .contains(&format!("url(/{})", carried.parts[0].name)),
        "the style does not point at what it carries: {}",
        carried.html
    );
}

#[test]
fn a_stylesheet_asks_this_build_for_its_pictures_and_never_the_network() {
    // A stylesheet used to go out as written, with `default-src 'none'` the single thing between
    // it and a host — so a message's background was refused by the policy, counted by nobody, and
    // drawn as an empty box the reader could not ask about. Its pictures now take the road every
    // other picture takes: the address leaves here, and what stays is a name served from here.
    let asked_nothing = body_of(
        &html_message(&[
            "<style>@import url(http://elsewhere.invalid/x.css);             p{background-image:url(http://elsewhere.invalid/y.png)}</style><p>T</p>",
        ]),
        Viewing {
            form: BodyForm::Sent,
            pictures: Pictures::Blocked,
        },
    );

    // The stylesheet is still the message's own — only the picture's address is gone.
    assert!(
        asked_nothing.html.contains("background-image"),
        "the stylesheet lost more than its addresses: {}",
        asked_nothing.html
    );
    assert!(
        !asked_nothing.html.contains("elsewhere.invalid/y.png"),
        "an address a message wrote is still in the document: {}",
        asked_nothing.html
    );
    // The `@import` is left exactly as it came, and deliberately: it is not a picture, and what
    // refuses it is the policy the frame reads the document under rather than anything here. Taking
    // its address out would send this application to fetch a stylesheet and hold it as a picture.
    assert!(
        asked_nothing
            .html
            .contains("@import url(http://elsewhere.invalid/x.css)"),
        "an at-rule was rewritten as though it named a picture: {}",
        asked_nothing.html
    );
    // Counted, so the reader is told and offered the press that can change it — one picture, not
    // two: a number that counted the `@import` was a press that could never bring anything back.
    assert_eq!(asked_nothing.remote_blocked, 1);
    assert!(asked_nothing.remote.is_empty(), "and nothing was fetched");

    let asked_for = body_of(
        &html_message(&[
            "<style>p{background-image:url(http://elsewhere.invalid/y.png)}</style><p>T</p>",
        ]),
        Viewing {
            form: BodyForm::Sent,
            pictures: Pictures::Shown,
        },
    );

    assert_eq!(
        asked_for.remote.len(),
        1,
        "the picture is one this build fetches"
    );
    assert!(
        asked_for
            .html
            .contains(&format!("url(/{})", asked_for.remote[0].name)),
        "the stylesheet does not point at the name it was given: {}",
        asked_for.html
    );
    assert!(!asked_for.html.contains("elsewhere.invalid"));
}

#[test]
fn a_message_of_tens_of_thousands_of_elements_is_shown_as_text() {
    // Bytes were bounded and depth was bounded; what it costs to lay out was not. Measured on
    // WKWebView with the frame served under the production policy inside the window that holds it:
    // 46,000 elements stopped the window answering for as long as the message was open -- no
    // sidebar, no list, no way to leave. The bytes were inside the cap and the depth was one.
    let flat = format!(
        "<html><body>{}</body></html>",
        "<i style=\"padding:3000px;display:table\">x</i>".repeat(super::nesting::MOST_ELEMENTS + 1)
    );

    let shown = body_of(
        &html_message(&[&flat]),
        Viewing {
            form: BodyForm::Sent,
            pictures: Pictures::Blocked,
        },
    );

    assert_eq!(
        shown.kind,
        crate::ports::body::BodyKind::PlainText,
        "a message that costs the window its thread was drawn anyway"
    );
    assert!(shown.html.contains('x'), "and its words were lost with it");
}

#[test]
fn a_message_that_is_merely_long_is_still_drawn_as_it_was_written() {
    // The limit is not a limit on ordinary mail. The ordinary messages in the corpus run from 5 to
    // 552 elements and the one written to be hostile about depth has 5,002; a newsletter with a
    // thousand rows is still a newsletter.
    let ordinary = format!(
        "<html><body><table>{}</table></body></html>",
        "<tr><td>Row</td></tr>".repeat(1_000)
    );

    let shown = body_of(
        &html_message(&[&ordinary]),
        Viewing {
            form: BodyForm::Sent,
            pictures: Pictures::Blocked,
        },
    );

    assert_eq!(shown.kind, crate::ports::body::BodyKind::Html);
    assert!(shown.html.contains("<table>"));
}

#[test]
fn nothing_a_message_wrote_reaches_the_journal() {
    // The whole corpus, rendered with the journal turned all the way up, and then the file read
    // back looking for anything the messages themselves said. The type barrier is what makes this
    // true and this is what says the barrier is still there — a `Value` variant taking a string
    // added next month compiles, and fails here.
    //
    // Searched by what a message *contains* rather than by a list of things to look for: a grep
    // for one known address proves nothing about the next one. Every run of letters that looks
    // like a host or an address in any of these messages has to be absent, and the messages are
    // searched through their encodings, because base64 hides a hostname from a naive scan.
    let at = std::env::temp_dir().join(format!("lyonoyl-journal-leak-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&at);
    std::fs::create_dir_all(&at).expect("a directory to write into");

    let writing_to =
        crate::infrastructure::journal::install(&at, crate::ports::journal::Note::Detail)
            .expect("a journal");

    let messages = corpus();
    assert!(messages.len() > 5, "the corpus is what makes this a test");

    let mut said: Vec<String> = Vec::new();

    for (name, raw) in &messages {
        for pictures in [Pictures::Blocked, Pictures::Shown] {
            let _ = body_of(
                raw,
                Viewing {
                    form: BodyForm::Sent,
                    pictures,
                },
            );
        }

        // What this message could leak: every word of it long enough to be a name, taken from the
        // raw bytes and from anything base64 inside it decodes to.
        said.extend(words_worth_hiding(raw));
        let _ = name;
    }

    // The corpus crosses no bound and draws no picture where a style may not, and nothing above
    // hands a body to the frame, so each of those is made to happen here: an event that never
    // reaches the file is an event the level check below says nothing about.
    let crossing = [
        format!("<p style=\"{}\">Long.</p>", "margin: 0; ".repeat(300)),
        format!(
            "<style>{}</style><p>Sheet.</p>",
            "p {{ margin: 0; }} ".repeat(5_000)
        ),
        "<p style=\"quux: url(https://example.com/a.png)\">Drawn.</p>".to_owned(),
    ];
    let held = super::serving::BodyBeingRead::default();
    for html in &crossing {
        let shown = body_of(
            &html_message(&[html]),
            Viewing {
                form: BodyForm::Sent,
                pictures: Pictures::Blocked,
            },
        );
        let _ = held.hold(&shown, None);
    }

    crate::infrastructure::journal::flush();

    let written = std::fs::read_to_string(&writing_to).unwrap_or_default();
    let lowered = written.to_ascii_lowercase();

    for word in &said {
        assert!(
            !lowered.contains(word.as_str()),
            "a message's own word reached the journal: {word:?}"
        );
    }

    // And the journal did write something, or the assertions above are about an empty file.
    assert!(
        written.contains("body.rendered"),
        "the journal recorded nothing at all"
    );

    // And nothing about one message is kept at a level a build writes by default. A line there for
    // every message opened is a record of when somebody read their mail, in a file the databases'
    // key does not cover. Read from the level each line carries, since this journal keeps them all.
    let kept_by_default: Vec<&str> = written
        .lines()
        .filter(|line| is_about_one_message(line) && !is_at_detail(line))
        .collect();
    assert!(
        kept_by_default.is_empty(),
        "an event about one message is written at a level a build keeps: {kept_by_default:#?}"
    );

    let _ = std::fs::remove_dir_all(&at);
}

/// Whether a journal line is about one message, read from the event it names.
fn is_about_one_message(line: &str) -> bool {
    let event = line.split_whitespace().nth(3).unwrap_or_default();
    ["body.", "picture.", "store.", "style.", "stylesheet."]
        .iter()
        .any(|about| event.starts_with(about))
}

/// Whether a journal line was written at `detail`, which nobody gets without asking for it.
fn is_at_detail(line: &str) -> bool {
    line.split_whitespace().nth(2) == Some("detail")
}

/// Every run of characters in a message that would identify somebody if it were written down.
///
/// Hosts, addresses and any long word, from the bytes as they are and from what the base64 in them
/// decodes to. Short words are left out: `to`, `png` and `image` appear in a message and in this
/// build's own vocabulary alike, and a test that failed on those would be a test nobody could keep.
fn words_worth_hiding(raw: &[u8]) -> Vec<String> {
    let mut found = Vec::new();

    for text in [
        String::from_utf8_lossy(raw).to_string(),
        decoded_base64(raw),
    ] {
        for word in text
            .to_ascii_lowercase()
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '@'))
        {
            let looks_like_a_name = word.len() >= 12
                && (word.contains('.') || word.contains('@'))
                && word.bytes().any(|byte| byte.is_ascii_alphabetic());

            if looks_like_a_name {
                found.push(word.to_owned());
            }
        }
    }

    found.sort_unstable();
    found.dedup();
    found
}

/// What the base64 in a message decodes to, so a hostname folded into an attachment is searched too.
fn decoded_base64(raw: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut bytes = Vec::new();
    let mut held = 0u32;
    let mut bits = 0u32;

    for character in raw {
        match ALPHABET.iter().position(|allowed| allowed == character) {
            Some(value) => {
                held = (held << 6) | value as u32;
                bits += 6;
                if bits >= 8 {
                    bits -= 8;
                    bytes.push((held >> bits) as u8);
                }
            }
            // Anything outside the alphabet ends the run rather than being skipped, so two
            // unrelated stretches are not spliced into a word that was never there.
            None => {
                held = 0;
                bits = 0;
            }
        }
    }

    String::from_utf8_lossy(&bytes).to_string()
}

#[test]
fn what_the_frame_is_actually_served() {
    // Not an assertion — a look at the document that leaves this build, for a message carrying a
    // picture of its own. Run with --nocapture.
    let raw = message(
        &[
            "From: someone@example.com",
            "Subject: A logo",
            "MIME-Version: 1.0",
            "Content-Type: multipart/related; boundary=\"rel\"",
        ],
        &[
            "--rel",
            "Content-Type: text/html; charset=utf-8",
            "",
            "<p>Before</p><img src=\"cid:logo@example.com\" width=\"40\" height=\"40\"><p>After</p>",
            "",
            "--rel",
            "Content-Type: image/gif",
            "Content-ID: <logo@example.com>",
            "Content-Transfer-Encoding: base64",
            "",
            "R0lGODlhAQABAAAAACw=",
            "--rel--",
        ],
    );

    let rendered = body_of(&raw, Viewing::plainly());
    eprintln!("--- parts: {} ---", rendered.parts.len());
    for part in &rendered.parts {
        eprintln!("part name={} type={}", part.name, part.content_type);
    }
    eprintln!("--- body html ---\n{}", rendered.html);

    let held = crate::infrastructure::html::serving::BodyBeingRead::default();
    let token = held.hold(&rendered, None).expect("a name");
    let (page, policy) = held.page(&token).expect("a page");
    eprintln!("--- served under: {policy} ---\n{page}");
}
