use super::*;

#[test]
fn a_name_of_spaces_is_no_name() {
    // Refused rather than stored: a group whose label draws as nothing is a row in the sidebar
    // that cannot be told from the gap above it.
    assert_eq!(GroupName::new("   "), Err(GroupNameError::Empty));
    assert_eq!(GroupName::new(""), Err(GroupNameError::Empty));
    assert_eq!(GroupName::new("\t\n"), Err(GroupNameError::Empty));
}

#[test]
fn surrounding_space_is_removed_rather_than_refused() {
    // Somebody who pastes a name with a space on the end meant the name. Kept as typed, this
    // would sit beside a group called "Work" as a second group nobody can tell apart.
    let name = GroupName::new("  Work  ").expect("a name with space around it is a name");

    assert_eq!(name.as_str(), "Work");
    assert_eq!(name, GroupName::new("Work").expect("plain"));
}

#[test]
fn the_limit_counts_characters_and_not_bytes() {
    // The same number of letters is the same length whatever alphabet wrote them. Counted in
    // bytes, a name in Greek would be worth half of one in English — a rule nobody could have
    // guessed by looking at the field.
    let greek = "ζ".repeat(LONGEST_NAME);
    assert!(
        greek.len() > LONGEST_NAME,
        "the fixture must exceed the limit in bytes"
    );

    assert!(GroupName::new(&greek).is_ok());
    assert_eq!(
        GroupName::new(&"ζ".repeat(LONGEST_NAME + 1)),
        Err(GroupNameError::TooLong)
    );
}

#[test]
fn the_limit_is_measured_after_the_space_is_taken_off() {
    // Otherwise a name that fits is refused for whitespace that is about to be discarded.
    let padded = format!("  {}  ", "a".repeat(LONGEST_NAME));

    assert!(GroupName::new(&padded).is_ok());
}

#[test]
fn an_empty_group_is_not_worth_reading() {
    // Asking every mail file for a window that must come back empty is work with a known answer.
    assert!(!worth_reading(&[]));
    assert!(worth_reading(&["account-1".to_owned()]));
}
