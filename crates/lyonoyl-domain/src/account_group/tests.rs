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

fn member(id: &str, label: &str, added: &str) -> Member {
    Member::new(
        id.to_owned(),
        label,
        "",
        added.to_owned(),
        str::to_lowercase,
    )
}

#[test]
fn by_hand_is_the_order_the_mailboxes_were_put_in() {
    let members = vec![
        member("c", "gamma", "2026-01-03 00:00:00"),
        member("a", "alpha", "2026-01-01 00:00:00"),
        member("b", "beta", "2026-01-02 00:00:00"),
    ];

    assert_eq!(sorted(members, Sorting::Manual), ["c", "a", "b"]);
}

#[test]
fn by_date_added_puts_the_first_mailbox_added_first() {
    // Whatever order a hand left them in.
    let members = vec![
        member("late", "alpha", "2026-03-01 09:00:00"),
        member("early", "beta", "2025-12-31 23:59:59"),
        member("middle", "gamma", "2026-01-15 12:00:00"),
    ];

    assert_eq!(
        sorted(members, Sorting::DateAdded),
        ["early", "middle", "late"]
    );
}

#[test]
fn by_name_reads_the_name_whatever_its_case() {
    // A capital is not a reason to stand first: "beta" belongs after "Alpha" and before "Gamma".
    let members = vec![
        member("g", "Gamma", "2026-01-01 00:00:00"),
        member("b", "beta", "2026-01-02 00:00:00"),
        member("a", "Alpha", "2026-01-03 00:00:00"),
    ];

    assert_eq!(sorted(members, Sorting::Name), ["a", "b", "g"]);
}

#[test]
fn by_name_compares_the_name_as_the_fold_reads_it() {
    // The fold says which spellings are one name. This one drops a leading mark, as the real one
    // drops accents: compared as typed, "~alpha" would stand after "zulu".
    let fold = |name: &str| name.trim_start_matches('~').to_lowercase();
    let members = vec![
        Member::new("z".to_owned(), "zulu", "", String::new(), fold),
        Member::new("a".to_owned(), "~alpha", "", String::new(), fold),
    ];

    assert_eq!(members[1].label, "~alpha");
    assert_eq!(sorted(members, Sorting::Name), ["a", "z"]);
}

#[test]
fn a_mailbox_with_no_name_of_its_own_is_sorted_by_its_address() {
    // The sidebar draws it under its address, and the sort reads the word the person reads.
    let unnamed = Member::new(
        "k".to_owned(),
        "   ",
        "kappa@example.net",
        "2026-01-01 00:00:00".to_owned(),
        str::to_lowercase,
    );
    let named = Member::new(
        "z".to_owned(),
        "Zeta Quorin",
        "zeta@example.com",
        "2026-01-02 00:00:00".to_owned(),
        str::to_lowercase,
    );

    assert_eq!(unnamed.label, "kappa@example.net");
    assert_eq!(named.label, "Zeta Quorin");
    assert_eq!(sorted(vec![named, unnamed], Sorting::Name), ["k", "z"]);
}

#[test]
fn two_mailboxes_that_sort_alike_stand_as_a_hand_left_them() {
    // One name twice, or two added in the same second: the sort has no answer, and the order the
    // person left is the one that stays.
    let members = vec![
        member("second", "Work", "2026-01-01 00:00:00"),
        member("first", "work", "2026-01-01 00:00:00"),
    ];

    assert_eq!(sorted(members.clone(), Sorting::Name), ["second", "first"]);
    assert_eq!(sorted(members, Sorting::DateAdded), ["second", "first"]);
}

#[test]
fn a_group_is_sorted_by_hand_until_somebody_says_otherwise() {
    assert_eq!(Sorting::default(), Sorting::Manual);
}
