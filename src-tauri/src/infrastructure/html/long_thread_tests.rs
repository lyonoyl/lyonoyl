//! A reply in a long thread, what a message past the depth bound is shown as, and how deep the
//! whole path can go before it gives way.

use super::*;

/// The corpus file this module is about, read the way the golden test reads every other one.
fn corpus_file(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("eml")
        .join(name);

    std::fs::read(&path).unwrap_or_else(|_| panic!("{name} is in the corpus"))
}

/// A reply sent from Apple Mail in a thread thirty-six messages long, every earlier message quoted
/// inside the one after it: Apple's `blockquote type="cite"` and Gmail's `gmail_quote` in turn,
/// both parts quoted-printable. Its HTML nests 123 elements deep and is drawn as HTML, never as its
/// own source, in the view that shows a message as it was sent.
#[test]
fn a_long_thread_quoted_in_full_is_drawn_as_it_was_written() {
    let raw = corpus_file("long-thread-quoted-in-full.eml");
    let rendered = body_of(&raw, Viewing::plainly());

    assert_eq!(
        rendered.kind,
        BodyKind::Html,
        "an ordinary reply was not drawn:\n{}",
        &rendered.html[..rendered.html.len().min(400)]
    );
    assert!(
        rendered.html.contains("Message 1 of the thread."),
        "the oldest quoted message is missing"
    );
    assert!(rendered.html.contains("<blockquote"));
    assert!(
        !rendered.html.contains("&lt;html") && !rendered.html.contains("&lt;div"),
        "the markup was shown as its source"
    );

    // The other two ways of looking at it are what they were: the text part, and the source.
    let text = body_of(
        &raw,
        Viewing {
            form: BodyForm::Text,
            pictures: Pictures::Blocked,
        },
    );
    assert!(text.html.contains("&gt; Message 36 of the thread."));

    let source = body_of(
        &raw,
        Viewing {
            form: BodyForm::Source,
            pictures: Pictures::Blocked,
        },
    );
    assert!(source
        .html
        .contains("&lt;html aria-label=3D&quot;message body&quot;&gt;"));
}

/// The words of one message in a thread, which say where in the thread they are.
fn written(message: usize) -> String {
    format!("<div>Message {message} of the thread.</div><div><br></div><div>Zeta</div>")
}

/// A reply quoting `messages` earlier ones, each inside the next, as Apple Mail and Gmail quote them
/// in turn: four levels for Apple's wrapper and three for Gmail's, so about three and a half per
/// message quoted.
fn thread_of(messages: usize) -> String {
    let mut html = String::from(
        "<html aria-label=\"message body\"><head><meta charset=\"utf-8\"></head><body>",
    );
    let mut closing = String::new();

    for message in (1..=messages + 1).rev() {
        html.push_str(&written(message));
        if message == 1 {
            break;
        }

        if message % 2 == 0 {
            html.push_str(
                "<div><br><blockquote type=\"cite\"><div>On 5 Jan 2026, at 09:00, Kappa Vandel \
                 &lt;kappa@example.net&gt; wrote:</div><br class=\"Apple-interchange-newline\">\
                 <div><div>",
            );
            closing.insert_str(0, "</div></div></blockquote></div>");
        } else {
            html.push_str(
                "<br><div class=\"gmail_quote\"><div dir=\"ltr\" class=\"gmail_attr\">On Mon, 5 \
                 Jan 2026 at 09:00, Sigma Ordrin &lt;<a href=\"mailto:sigma@example.org\">\
                 sigma@example.org</a>&gt; wrote:<br></div><blockquote class=\"gmail_quote\" \
                 style=\"margin:0px 0px 0px 0.8ex;border-left:1px solid rgb(204,204,204);\
                 padding-left:1ex\"><div dir=\"ltr\">",
            );
            closing.insert_str(0, "</div></blockquote></div>");
        }
    }

    html.push_str(&closing);
    html.push_str("</body></html>");
    html
}

/// A reply in the shape Apple Mail sends one: the text part it writes for a client that shows no
/// HTML, where there is one, beside the HTML, and whatever else the message carries after both.
fn apple_mail_reply(text: Option<&str>, html: &str, also: &[&str]) -> Vec<u8> {
    let mut alternative = String::new();
    if let Some(text) = text {
        alternative.push_str(&format!(
            "--alternative\nContent-Transfer-Encoding: 8bit\nContent-Type: text/plain;\n\t\
             charset=utf-8\n\n{text}\n"
        ));
    }
    alternative.push_str(&format!(
        "--alternative\nContent-Transfer-Encoding: 8bit\nContent-Type: text/html;\n\t\
         charset=utf-8\n\n{html}\n--alternative--\n"
    ));

    let mut mixed = format!(
        "--mixed\nContent-Type: multipart/alternative;\n\tboundary=\"alternative\"\n\n\
         {alternative}"
    );
    for part in also {
        mixed.push_str(&format!("--mixed\n{part}\n"));
    }
    mixed.push_str("--mixed--\n");

    format!(
        "From: Zeta Quorin <zeta@example.com>\nTo: Kappa Vandel <kappa@example.net>\n\
         Subject: Re: The quarterly plan\nMime-Version: 1.0 (Mac OS X Mail 16.0 \\(0000.0.0\\))\n\
         Content-Type: multipart/mixed;\n\tboundary=\"mixed\"\n\n{mixed}"
    )
    .into_bytes()
}

/// A file carried beside the body, for the tests that ask whether it is still listed.
const A_FILE: &str = "Content-Type: application/pdf;\n\tname=\"notes.pdf\"\n\
                      Content-Disposition: attachment;\n\tfilename=\"notes.pdf\"\n\
                      Content-Transfer-Encoding: base64\n\nJVBERi0xLjQK";

/// A picture carried inside the message, referred to by the body through its `Content-ID`.
const A_PICTURE: &str = "Content-Type: image/jpeg;\n\tname=\"IMG_0001.jpeg\"\n\
                         Content-Id: <photo1>\n\
                         Content-Disposition: inline;\n\tfilename=\"IMG_0001.jpeg\"\n\
                         Content-Transfer-Encoding: base64\n\n/9j/4AAQSkZJRgABAQAAAQABAAD/2wA=";

/// The deepest an engine builds a document: WebKit and Chromium both stop nesting at 512 and
/// attach anything deeper beside the element at that depth.
const ENGINES_BUILD: usize = 512;

#[test]
fn a_document_as_deep_as_an_engine_builds_is_drawn_and_one_level_deeper_is_not() {
    assert!(!nesting::nesting_beyond(
        &"<div>".repeat(ENGINES_BUILD),
        DEEPEST_NESTING
    ));
    assert!(nesting::nesting_beyond(
        &"<div>".repeat(ENGINES_BUILD + 1),
        DEEPEST_NESTING
    ));
}

#[test]
fn a_thread_a_hundred_messages_long_is_still_drawn() {
    let html = thread_of(100);
    let rendered = body_of(
        &apple_mail_reply(Some("Written as text."), &html, &[]),
        Viewing::plainly(),
    );

    assert_eq!(rendered.kind, BodyKind::Html);
    assert!(rendered.html.contains("Message 1 of the thread."));
    assert!(rendered.html.contains("Message 101 of the thread."));
    assert!(!rendered.html.contains("Written as text."));
}

#[test]
fn a_thread_deeper_than_the_bound_is_shown_as_the_text_its_sender_wrote() {
    let html = thread_of(200);
    assert!(
        nesting::nesting_beyond(&html, DEEPEST_NESTING),
        "the premise: this document is past the bound"
    );

    let rendered = body_of(
        &apple_mail_reply(
            Some("Written as text.\n\n> Quoted as text, & kept <as written>."),
            &html,
            &[],
        ),
        Viewing::plainly(),
    );

    assert_eq!(rendered.kind, BodyKind::PlainText);
    assert!(
        rendered
            .html
            .contains("Written as text.\n\n&gt; Quoted as text, &amp; kept &lt;as written&gt;."),
        "{}",
        &rendered.html[..rendered.html.len().min(400)]
    );
    assert!(
        !rendered.html.contains("&lt;div") && !rendered.html.contains("&lt;blockquote"),
        "the markup was shown as its source"
    );
    assert!(!rendered.truncated);
}

#[test]
fn an_html_only_thread_deeper_than_the_bound_is_shown_as_its_words() {
    let html = thread_of(200);
    let rendered = body_of(&apple_mail_reply(None, &html, &[]), Viewing::plainly());

    assert_eq!(rendered.kind, BodyKind::PlainText);
    assert!(
        rendered
            .html
            .starts_with("<pre>Message 201 of the thread.\n\nZeta\n"),
        "{}",
        &rendered.html[..rendered.html.len().min(400)]
    );
    assert!(rendered.html.contains("\nMessage 1 of the thread.\n"));
    assert!(rendered
        .html
        .contains("On 5 Jan 2026, at 09:00, Kappa Vandel &lt;kappa@example.net&gt; wrote:"));
    for markup in [
        "&lt;div",
        "&lt;blockquote",
        "gmail_quote",
        "Apple-interchange",
        "&lt;/",
    ] {
        assert!(!rendered.html.contains(markup), "{markup} was shown");
    }
    assert!(!rendered.truncated);
}

#[test]
fn a_text_part_with_nothing_in_it_does_not_take_the_place_of_the_words() {
    let rendered = body_of(
        &apple_mail_reply(Some("  \n"), &thread_of(200), &[]),
        Viewing::plainly(),
    );

    assert_eq!(rendered.kind, BodyKind::PlainText);
    assert!(rendered.html.contains("Message 1 of the thread."));
    assert!(!rendered.html.contains("&lt;div"));
}

#[test]
fn the_words_of_a_hostile_document_past_the_bound_carry_nothing_that_runs() {
    let hostile = format!(
        "{}<script>alert(1)</script><style>body {{ background: url(https://example.com/x) }}</style>\
         <img src=\"https://example.com/p.gif\" onerror=\"alert(2)\"><a href=\"javascript:alert(3)\">\
         Click</a> &lt;script&gt;alert(4)&lt;/script&gt;",
        "<div>".repeat(DEEPEST_NESTING + 1)
    );
    let rendered = body_of(&apple_mail_reply(None, &hostile, &[]), Viewing::plainly());

    assert_eq!(rendered.kind, BodyKind::PlainText);
    assert_eq!(
        rendered.html, "<pre>Click &lt;script&gt;alert(4)&lt;/script&gt;</pre>",
        "a script, a stylesheet, a handler or an address survived as words"
    );
    assert!(rendered.links.is_empty() && rendered.remote.is_empty() && rendered.parts.is_empty());
}

#[test]
fn the_files_of_a_message_past_the_bound_are_still_listed() {
    for text in [Some("Written as text."), None] {
        let rendered = body_of(
            &apple_mail_reply(text, &thread_of(200), &[A_FILE]),
            Viewing::plainly(),
        );

        assert_eq!(rendered.kind, BodyKind::PlainText);
        assert_eq!(
            rendered
                .attachments
                .iter()
                .map(|file| file.name.as_str())
                .collect::<Vec<_>>(),
            ["notes.pdf"],
            "with a text part: {}",
            text.is_some()
        );
    }
}

#[test]
fn a_text_part_that_is_not_the_html_one_said_again_is_not_shown_in_its_place() {
    // A list's footer, sent as a part of its own beside an HTML-only message. It is not the body
    // said another way, and showing it instead of the body would lose the body.
    let footer = "Content-Type: text/plain; charset=utf-8\n\nYou are on the list.";
    let raw = format!(
        "From: Zeta Quorin <zeta@example.com>\nSubject: Re: The quarterly plan\n\
         MIME-Version: 1.0\nContent-Type: multipart/mixed; boundary=\"mixed\"\n\n\
         --mixed\nContent-Type: text/html; charset=utf-8\n\n{}\n--mixed\n{footer}\n--mixed--\n",
        thread_of(200)
    );
    let rendered = body_of(raw.as_bytes(), Viewing::plainly());

    assert_eq!(rendered.kind, BodyKind::PlainText);
    assert!(rendered.html.contains("Message 1 of the thread."));
    assert!(!rendered.html.contains("You are on the list."));
    assert!(!rendered.html.contains("&lt;div"));
}

#[test]
fn a_document_past_the_bound_with_no_words_in_it_has_nothing_to_show() {
    let rendered = body_of(
        &apple_mail_reply(None, &"<div>".repeat(DEEPEST_NESTING + 1), &[A_FILE]),
        Viewing::plainly(),
    );

    assert_eq!(rendered.kind, BodyKind::Nothing);
    assert_eq!(
        rendered.attachments.len(),
        1,
        "and what it carries is still listed"
    );
}

/// Scores of formatting elements left open, then blocks that orphan them. A parser keeps its own
/// list of open formatting elements and rebuilds every one of them before each later run of text,
/// so a source of hundreds of tags becomes a tree of millions. The depth is ordinary and the source
/// is small, so neither the depth count nor the element count sees it: only the count of formatting
/// elements held open refuses it, before any tree is built, and the same shape in an element a
/// parser does not rebuild is drawn.
#[test]
fn many_formatting_elements_left_open_are_refused_where_plain_ones_are_drawn() {
    let orphaned = |tag: &str| {
        format!(
            "<div>{}</div>{}",
            format!("<{tag}>").repeat(200),
            "<div>word</div>".repeat(100),
        )
    };

    // Well under the depth bound: the stack never holds more than a couple of hundred at once.
    assert!(
        !nesting::nesting_beyond(&orphaned("span"), DEEPEST_NESTING),
        "the same shape in a plain element is ordinary depth"
    );
    // But the formatting elements are refused, and uppercase is no way around it.
    assert!(nesting::nesting_beyond(&orphaned("b"), DEEPEST_NESTING));
    assert!(nesting::nesting_beyond(&orphaned("B"), DEEPEST_NESTING));
    assert!(nesting::nesting_beyond(&orphaned("font"), DEEPEST_NESTING));

    let rendered = body_of(
        &apple_mail_reply(None, &orphaned("b"), &[]),
        Viewing::plainly(),
    );
    assert_eq!(
        rendered.kind,
        BodyKind::PlainText,
        "the amplifying document was drawn rather than shown as its words"
    );
    assert!(rendered.html.contains("word"));
    assert!(!rendered.html.contains("&lt;b"));

    // The plain-element shape is drawn as it was written.
    let plain = body_of(
        &apple_mail_reply(None, &orphaned("span"), &[]),
        Viewing::plainly(),
    );
    assert_eq!(plain.kind, BodyKind::Html);
}

/// A picture that travelled inside a message too deep to draw is not lost with the body: the body
/// is shown as its words, and the picture is listed as a file to save. Drawn, it would have been
/// shown in place and listed by nobody.
#[test]
fn a_picture_carried_in_a_message_past_the_bound_is_listed_as_a_file() {
    for text in [Some("Written as text."), None] {
        let rendered = body_of(
            &apple_mail_reply(text, &thread_of(200), &[A_PICTURE]),
            Viewing::plainly(),
        );

        assert_eq!(rendered.kind, BodyKind::PlainText);
        assert!(
            rendered
                .attachments
                .iter()
                .any(|file| file.name == "IMG_0001.jpeg"),
            "the inline picture was neither shown nor listed, with a text part: {}",
            text.is_some()
        );
        // Not shown in the body's place, only listed beside it.
        assert!(!rendered.html.contains("IMG_0001"));
    }
}

/// The text part shown in the body's place is the plain-text one the sender wrote, and only that.
/// `mail-parser` files a calendar, and AMP's own HTML, as text too; showing one of those would put
/// its markup or its raw calendar on the page — the very thing the fallback avoids. With no plain
/// part, the words of the markup are shown instead.
#[test]
fn only_the_plain_text_alternative_stands_in_for_a_body_past_the_bound() {
    let raw = format!(
        "From: Zeta Quorin <zeta@example.com>\nSubject: Re: The quarterly plan\n\
         MIME-Version: 1.0\nContent-Type: multipart/alternative; boundary=\"alt\"\n\n\
         --alt\nContent-Type: text/calendar; charset=utf-8\n\n\
         BEGIN:VCALENDAR\nSUMMARY:Not the body\nEND:VCALENDAR\n\
         --alt\nContent-Type: text/html; charset=utf-8\n\n{}\n--alt--\n",
        thread_of(200)
    );
    let rendered = body_of(raw.as_bytes(), Viewing::plainly());

    assert_eq!(rendered.kind, BodyKind::PlainText);
    assert!(
        !rendered.html.contains("BEGIN:VCALENDAR") && !rendered.html.contains("Not the body"),
        "a calendar was shown in the body's place: {}",
        &rendered.html[..rendered.html.len().min(200)]
    );
    // The words of the HTML instead.
    assert!(rendered.html.contains("Message 1 of the thread."));
}

#[test]
fn whether_what_is_shown_was_cut_is_said_of_what_is_shown() {
    // Past the bound and past the body limit: the markup is cut before it is counted.
    let long = format!(
        "{}{}",
        "<div>".repeat(DEEPEST_NESTING + 1),
        "word ".repeat(LARGEST_BODY / 4)
    );

    // The text part is shown whole, so nothing about it was cut.
    let beside = body_of(
        &apple_mail_reply(Some("Written as text."), &long, &[]),
        Viewing::plainly(),
    );
    assert_eq!(beside.kind, BodyKind::PlainText);
    assert!(!beside.truncated, "the text part was said to be cut");

    // The words of the markup are the beginning of them.
    let words = body_of(&apple_mail_reply(None, &long, &[]), Viewing::plainly());
    assert_eq!(words.kind, BodyKind::PlainText);
    assert!(words.truncated, "the words were not said to be cut");
}

/// The hostile corpus file about depth — five thousand nested `<div>` around one word — is nearly
/// ten times past the bound, refused by the count, and never handed to the parser: what comes back
/// is its one word as text, where the sanitizer would have written back its elements.
#[test]
fn the_document_written_to_be_hostile_about_depth_is_still_refused_before_any_parser() {
    let raw = corpus_file("hostile-deep-nesting.eml");
    let parsed = MessageParser::default()
        .parse(&raw)
        .expect("the corpus file parses as a message");
    let html = parsed
        .html_bodies()
        .find_map(|part| match &part.body {
            PartType::Html(html) => Some(html.to_string()),
            _ => None,
        })
        .expect("it is an HTML message");

    assert!(nesting::nesting_beyond(&html, DEEPEST_NESTING));
    // Nearly ten times past it, which the compiler checks rather than the run.
    const { assert!(DEEPEST_NESTING * 9 < 5_000) };

    let rendered = body_of(&raw, Viewing::plainly());
    assert_eq!(rendered.kind, BodyKind::PlainText);
    assert_eq!(rendered.html, "<pre>deep</pre>");
}

/// The count that runs before the parser, on the shapes that make a careless scan search the rest
/// of the document once per tag: two megabytes of empty comments, of tag names that never end, of
/// closing tags that match nothing. Each costs the length of the document and no more.
#[test]
fn the_count_before_the_parser_costs_the_length_of_the_document() {
    for shape in ["<!---->", "<!-- -->x", "<a", "</a", "<textarea></textarea>"] {
        let document = shape.repeat(LARGEST_BODY / shape.len());
        let began = std::time::Instant::now();
        let _ = nesting::nesting_beyond(&document, DEEPEST_NESTING);
        let _ = nesting::elements_beyond(&document, MOST_ELEMENTS);
        let took = began.elapsed();

        assert!(
            took < std::time::Duration::from_secs(2),
            "{shape:?} to {} bytes took {took:?}",
            document.len()
        );
    }
}

/// A document nested `depth` levels deep, in one of the shapes the measurement below asks about.
///
/// `div` is the plain chain, closed. `open` is the same chain never closed — the deepest tree a
/// body of a given size can build, because a parser closes it at the end. `thread` is a quoted
/// reply as Apple Mail writes one, `blockquote > div > div` for every message quoted, closed. `b`
/// is a chain of formatting elements, which the parser keeps a second list of and walks on every
/// piece of text. `amplify` is `depth` formatting elements left open and orphaned by a closing
/// block, with a block per level after them: the shape a parser rebuilds into a tree of `depth`
/// times the blocks, which `MOST_FORMATTING` refuses before any tree is built. `bounded` is `div`
/// chains of this depth, one after another, until the next would pass `MOST_ELEMENTS`: at
/// `DEEPEST_NESTING`, the costliest document of these shapes that both counts let through.
fn nested(shape: &str, depth: usize) -> String {
    match shape {
        "bounded" => nested("div", depth).repeat((MOST_ELEMENTS / depth).max(1)),
        "div" => format!("{}deep{}", "<div>".repeat(depth), "</div>".repeat(depth)),
        "open" => format!("{}deep", "<div>".repeat(depth)),
        "b" => format!("{}deep{}", "<b>".repeat(depth), "</b>".repeat(depth)),
        "amplify" => format!(
            "<div>{}</div>{}",
            "<b>".repeat(depth),
            "<div>x</div>".repeat(depth),
        ),
        "thread" => {
            let messages = depth / 3;
            let mut html = String::new();
            for message in 0..messages {
                html.push_str(
                    "<blockquote type=\"cite\"><div>On 5 Jan 2026, at 09:00, Zeta Quorin \
                     &lt;zeta@example.com&gt; wrote:</div><br class=\"Apple-interchange-newline\">\
                     <div><div>",
                );
                html.push_str(&format!("Words of message {message}."));
            }
            html.push_str(&"</div></div></blockquote>".repeat(messages));
            html
        }
        _ => panic!("no shape called {shape}"),
    }
}

/// How deep a document may nest before the whole path gives way, asked of the path itself.
///
/// Each depth runs in a process of its own, because the one failure a guard cannot catch is a
/// stack running out: that ends the process rather than unwinding it. The child renders through
/// the guard exactly as a message is rendered — its own thread, `STACK`, the time limit, the catch
/// for a panic — and past the depth bound: parsing, the cleaning with every callback this build
/// passes, serialising, and dropping every tree.
///
/// Two passes. The first goes deeper until the time limit refuses. The second takes the deepest
/// document of each shape that fits in `LARGEST_BODY` and lets the work run to its end after the
/// limit has refused it, which is what a production thread does — a stack that ran out then would
/// still end the process. That pass takes minutes, so it runs in a release build only.
///
/// `cargo test -p lyonoyl --lib how_deep_the_whole_path_goes -- --ignored --nocapture`, and with
/// `--release` for what an installed build does and for the second pass.
#[test]
#[ignore = "renders documents hundreds of thousands of levels deep, one process each; started explicitly"]
fn how_deep_the_whole_path_goes() {
    for shape in ["div", "open", "thread", "b", "amplify"] {
        for depth in [
            1_000, 2_000, 5_000, 10_000, 12_000, 14_000, 16_000, 20_000, 50_000, 100_000, 200_000,
            400_000,
        ] {
            let bytes = nested(shape, depth).len();
            if bytes > LARGEST_BODY {
                eprintln!("MEASURED {shape:>6} {depth:>7} deep: {bytes} bytes, past LARGEST_BODY");
                break;
            }

            let outcome = in_a_process_of_its_own(shape, depth, false);
            eprintln!("MEASURED {shape:>6} {depth:>7} deep, {bytes:>7} bytes: {outcome}");
            if !outcome.starts_with("answered") {
                break;
            }
        }
    }

    // What the bound itself lets through, and what that costs.
    for shape in ["div", "thread", "b", "bounded"] {
        let bytes = nested(shape, DEEPEST_NESTING).len();
        let outcome = in_a_process_of_its_own(shape, DEEPEST_NESTING, false);
        eprintln!("MEASURED at the bound {shape:>7}, {bytes:>7} bytes: {outcome}");
    }

    if cfg!(debug_assertions) {
        eprintln!("MEASURED to the end: only in a release build");
        return;
    }

    for shape in ["open", "div", "b", "thread"] {
        // The deepest document of this shape the body limit lets through.
        let (mut fits, mut past) = (1, 1_000_000);
        while past - fits > 1 {
            let middle = (fits + past) / 2;
            match nested(shape, middle).len() <= LARGEST_BODY {
                true => fits = middle,
                false => past = middle,
            }
        }

        let outcome = in_a_process_of_its_own(shape, fits, true);
        eprintln!("MEASURED to the end {shape:>6} {fits:>7} deep: {outcome}");
    }
}

/// Runs one depth in a child process and says what came of it.
fn in_a_process_of_its_own(shape: &str, depth: usize, to_the_end: bool) -> String {
    let child = std::process::Command::new(
        std::env::current_exe().expect("the test binary knows where it is"),
    )
    .args([
        "--exact",
        "infrastructure::html::long_thread_tests::one_depth_behind_the_guards",
        "--ignored",
        "--nocapture",
        "--test-threads=1",
    ])
    .env("LYONOYL_DEPTH_PROBE", format!("{shape}:{depth}"))
    .env(
        "LYONOYL_DEPTH_TO_THE_END",
        if to_the_end { "1" } else { "" },
    )
    .output()
    .expect("a child process starts");

    // The harness prints the test's name on the same line, before what the test says.
    let said = String::from_utf8_lossy(&child.stdout)
        .lines()
        .find_map(|line| line.split_once("PROBE ").map(|(_, said)| said.to_owned()));

    match (child.status.success(), said) {
        (true, Some(said)) => said,
        _ => format!(
            "the process ended: {:?}, {}",
            child.status,
            String::from_utf8_lossy(&child.stderr)
                .lines()
                .find(|line| line.contains("overflow") || line.contains("panicked"))
                .unwrap_or("nothing said")
        ),
    }
}

/// One depth of the measurement above, in the process it started. Does nothing on its own.
#[test]
#[ignore = "one depth of how_deep_the_whole_path_goes, in a process of its own; started by it"]
fn one_depth_behind_the_guards() {
    let Ok(asked) = std::env::var("LYONOYL_DEPTH_PROBE") else {
        return;
    };
    let to_the_end = std::env::var("LYONOYL_DEPTH_TO_THE_END").is_ok_and(|asked| asked == "1");
    let (shape, depth) = asked.split_once(':').expect("shape:depth");
    let html = nested(shape, depth.parse().expect("a depth"));
    let raw = format!(
        "From: Zeta Quorin <zeta@example.com>\nSubject: Deep\nMIME-Version: 1.0\n\
         Content-Type: text/html; charset=utf-8\n\n{html}"
    );

    // When the work itself ended, in milliseconds from the start and one more, so that zero can
    // mean it has not.
    let ended = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let ended_by_the_work = Arc::clone(&ended);

    let began = std::time::Instant::now();
    let read = guards::behind_the_guards(
        raw.as_bytes(),
        "message-body",
        guards::Asking::SomebodyIsWaiting,
        move |message| {
            let parsed = MessageParser::default().parse(message)?;
            let html = parsed.html_bodies().find_map(|part| match &part.body {
                PartType::Html(html) => Some(html.to_string()),
                _ => None,
            })?;
            drop(parsed);

            let body = drawn(
                &html,
                false,
                Vec::new(),
                Default::default(),
                Pictures::Blocked,
                Vec::new(),
            );
            let bytes = body.html.len();
            drop(body);

            ended_by_the_work.store(
                began.elapsed().as_millis() as u64 + 1,
                std::sync::atomic::Ordering::SeqCst,
            );
            Some(bytes)
        },
    );
    let took = began.elapsed().as_millis();

    let said = match read {
        guards::Guarded::Answered(Some(bytes)) => {
            format!("answered in {took} ms, {bytes} bytes out")
        }
        guards::Guarded::Answered(None) => "nothing came back".to_owned(),
        guards::Guarded::Refused if !to_the_end => {
            format!("refused by the time limit after {took} ms")
        }
        guards::Guarded::Refused => {
            // The thread goes on after the refusal, as it does in production. An hour is past
            // anything measured; a stack that runs out before then ends this process instead.
            while ended.load(std::sync::atomic::Ordering::SeqCst) == 0
                && began.elapsed() < std::time::Duration::from_secs(3600)
            {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            match ended.load(std::sync::atomic::Ordering::SeqCst) {
                0 => format!("refused after {took} ms, and still running after an hour"),
                ended => format!(
                    "refused after {took} ms; the work ran to its end at {} ms and the stack held",
                    ended - 1
                ),
            }
        }
        guards::Guarded::Busy => "busy".to_owned(),
    };

    println!("PROBE {said}");
}
