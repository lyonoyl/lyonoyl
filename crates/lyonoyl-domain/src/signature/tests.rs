use super::*;

fn kept(typed: &str) -> String {
    Signature::new(typed)
        .expect("a signature")
        .as_str()
        .to_owned()
}

#[test]
fn a_signature_is_kept_as_it_was_typed() {
    assert_eq!(
        kept("Kappa Vandel\nkappa@example.net"),
        "Kappa Vandel\nkappa@example.net"
    );
}

#[test]
fn every_way_a_line_can_end_becomes_one_newline() {
    assert_eq!(
        kept("Kappa Vandel\r\nExample Office\rkappa@example.net"),
        "Kappa Vandel\nExample Office\nkappa@example.net"
    );
}

#[test]
fn a_character_that_writes_nothing_is_dropped_and_a_tab_kept() {
    assert_eq!(
        kept("Kappa\u{0000} Vandel\u{0007}\u{001B}[31m\tOffice\u{007F}\u{0085}"),
        "Kappa Vandel[31m\tOffice"
    );
}

#[test]
fn the_marks_that_set_a_line_s_direction_are_the_person_s_own_and_kept() {
    assert_eq!(
        kept("\u{200F}Κάππα Βάντελ\u{200E}"),
        "\u{200F}Κάππα Βάντελ\u{200E}"
    );
}

#[test]
fn blank_lines_before_it_and_spaces_after_it_go_and_the_lines_between_stay() {
    assert_eq!(
        kept(" \n\t\n\n  Kappa Vandel\n\n  Example Office  \n \n\t"),
        "  Kappa Vandel\n\n  Example Office"
    );
}

#[test]
fn nothing_but_blank_lines_is_no_signature() {
    for typed in ["", "   ", "\n\n", " \r\n\t\r\n", "\u{0000}\n\u{0007}"] {
        assert!(
            Signature::new(typed).expect("not refused").is_empty(),
            "{typed:?} kept as a signature"
        );
    }
    assert!(Signature::default().is_empty());
}

#[test]
fn the_longest_signature_is_kept_and_one_character_more_is_refused() {
    // Two bytes each, so the bound is in characters and not in bytes.
    let longest = "Σ".repeat(LONGEST_SIGNATURE);
    let past_it = "Σ".repeat(LONGEST_SIGNATURE + 1);

    assert_eq!(kept(&longest), longest);
    assert_eq!(Signature::new(&past_it), Err(SignatureError::TooLong));
}

#[test]
fn what_is_dropped_is_not_counted_against_the_bound() {
    let longest = "k".repeat(LONGEST_SIGNATURE);
    let typed = format!("\r\n\r\n{longest}\u{0000}  \r\n");

    assert_eq!(kept(&typed), longest);
}

#[test]
fn a_signature_never_shows_what_it_says_where_it_is_logged() {
    let signature = Signature::new("Kappa Vandel\nkappa@example.net").expect("a signature");

    let shown = format!("{signature:?}");

    assert_eq!(shown, "Signature(30 characters)");
    assert!(!shown.contains("Kappa"));
}
