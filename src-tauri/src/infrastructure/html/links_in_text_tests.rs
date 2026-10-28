use super::*;

/// The examples the window's marks are held to as well.
const SHARED: &str = include_str!("../../../../tests/fixtures/links-in-text.json");

#[derive(serde::Deserialize)]
struct Examples {
    cases: Vec<Example>,
}

#[derive(serde::Deserialize)]
struct Example {
    text: String,
    links: Vec<Expected>,
}

#[derive(serde::Deserialize)]
struct Expected {
    written: String,
    opens: String,
}

#[test]
fn every_shared_example_is_found_as_written_and_opens_what_it_says() {
    let examples: Examples = serde_json::from_str(SHARED).expect("the examples read");
    assert!(examples.cases.len() > 20, "the examples are all there");

    for example in &examples.cases {
        let found = links_in(&example.text);
        let written: Vec<&str> = found
            .iter()
            .map(|link| &example.text[link.at.clone()])
            .collect();
        let opens: Vec<String> = found.iter().map(|link| link.url.to_string()).collect();

        assert_eq!(
            written,
            example
                .links
                .iter()
                .map(|link| link.written.as_str())
                .collect::<Vec<_>>(),
            "{:?}",
            example.text
        );
        assert_eq!(
            opens,
            example
                .links
                .iter()
                .map(|link| link.opens.clone())
                .collect::<Vec<_>>(),
            "{:?}",
            example.text
        );
    }
}

#[test]
fn nothing_in_nothing() {
    assert!(links_in("").is_empty());
    assert!(links_in("   \n\t").is_empty());
}

#[test]
fn an_address_at_the_very_end_of_the_text_is_found_whole() {
    let found = links_in("https://example.com/last");

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].at, 0.."https://example.com/last".len());
}

#[test]
fn only_what_this_build_opens_is_a_link() {
    // The same parser that decides it for a link in markup, so a scheme refused there is refused
    // here, whatever comes after it in the text.
    for text in [
        "javascript:alert(1)",
        "data:text/html,hi",
        "file:///etc/hosts",
        "vbscript:x",
        "ftp://example.com",
    ] {
        assert!(links_in(text).is_empty(), "{text}");
    }
}

#[test]
fn a_backslash_ends_an_address_so_two_parsers_cannot_read_two_hosts() {
    // `https://good.example\@evil.example/` is a path on one host to the URL standard and a
    // sign-in at the other to some parsers. Cut at the backslash, there is one host to read.
    let found = links_in("https://good.example\\@evil.example/");

    assert_eq!(found.len(), 1);
    assert_eq!(found[0].url.host_str(), Some("good.example"));
}

#[test]
fn text_made_of_almost_addresses_costs_what_its_length_costs() {
    // A beginning inside a word is looked at and passed over once. Searched for again from each
    // one, a megabyte of them would take minutes.
    let text = "xwww.".repeat(200_000);
    let began = std::time::Instant::now();

    assert!(links_in(&text).is_empty());
    assert!(
        began.elapsed() < std::time::Duration::from_secs(2),
        "{:?}",
        began.elapsed()
    );
}

#[test]
fn every_address_in_a_long_text_is_found() {
    let text = "see https://example.com/a and ".repeat(1_000);

    let found = links_in(&text);

    assert_eq!(found.len(), 1_000);
    assert!(found
        .iter()
        .all(|link| link.url.as_str() == "https://example.com/a"));
}
