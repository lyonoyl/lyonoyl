use super::*;

/// Case off, and the two accents these tests write, as the real fold takes every accent off.
fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|letter| match letter {
            'é' => 'e',
            'ö' => 'o',
            other => other,
        })
        .collect()
}

fn people(gathering: Gathering) -> Vec<Correspondent> {
    gathering.people(fold)
}

/// The addresses offered for `typed`, in the order they are offered.
fn offered_for(people: &[Correspondent], typed: &str) -> Vec<String> {
    offered(people, typed, fold)
        .into_iter()
        .map(|person| person.address.clone())
        .collect()
}

/// One message sent to each address given, at one second each.
fn written_to(addresses: &[&str]) -> Vec<Correspondent> {
    let mut gathering = Gathering::default();
    for (at, address) in addresses.iter().enumerate() {
        gathering.sent([*address], [], at as i64);
    }
    people(gathering)
}

#[test]
fn somebody_written_to_is_offered_by_the_start_of_their_address() {
    let people = written_to(&["kappa@example.net", "sigma@example.org"]);

    assert_eq!(offered_for(&people, "kap"), vec!["kappa@example.net"]);
    assert_eq!(offered_for(&people, "KAPPA@EX"), vec!["kappa@example.net"]);
}

#[test]
fn what_is_typed_finds_whoever_holds_it_anywhere_in_their_address_or_name() {
    let mut gathering = Gathering::default();
    gathering.sent(["kappa@example.net"], [], 10);
    gathering.named("kappa@example.net", "Kappa Vandel", 10);
    gathering.sent(["zq@example.com"], [], 20);
    gathering.named("zq@example.com", "Zéta Quörin", 20);
    let people = people(gathering);

    assert_eq!(offered_for(&people, "appa"), vec!["kappa@example.net"]);
    assert_eq!(offered_for(&people, "ANDE"), vec!["kappa@example.net"]);
    assert_eq!(offered_for(&people, "uör"), vec!["zq@example.com"]);
    assert_eq!(offered_for(&people, "a q"), vec!["zq@example.com"]);
    assert_eq!(
        offered_for(&people, "@example.n"),
        vec!["kappa@example.net"]
    );
    assert_eq!(
        offered_for(&people, "example"),
        vec!["zq@example.com", "kappa@example.net"]
    );
}

#[test]
fn whoever_what_is_typed_begins_comes_before_whoever_only_holds_it_however_often_written_to() {
    let mut gathering = Gathering::default();
    gathering.sent(["ordrin@example.org"], [], 4);
    gathering.sent(["sigma@example.org"], [], 3);
    gathering.named("sigma@example.org", "Sigma Ordrin", 3);
    for _ in 0..5 {
        gathering.sent(["zq@example.com"], [], 2);
    }
    gathering.named("zq@example.com", "Zeta Quorin", 2);
    for _ in 0..6 {
        gathering.sent(["kappazeta@example.net"], [], 1);
    }
    let people = people(gathering);

    // `o` begins the first address and the second word of the second name, and is only inside
    // the third address and name.
    assert_eq!(
        offered_for(&people, "o"),
        vec!["ordrin@example.org", "sigma@example.org", "zq@example.com"]
    );
    // `ze` begins the first word of a name, and is only inside the other address.
    assert_eq!(
        offered_for(&people, "ze"),
        vec!["zq@example.com", "kappazeta@example.net"]
    );
}

#[test]
fn a_part_of_the_address_after_a_dot_a_dash_an_underscore_or_a_plus_is_a_start_too() {
    // Each beside an address that holds the same part only inside it, and was written to more.
    let mut gathering = Gathering::default();
    for (times, address) in [
        (1, "k.vandel@example.net"),
        (2, "kvandel@example.net"),
        (1, "z-quorin@example.com"),
        (2, "zquorin@example.com"),
        (1, "s_ordrin@example.org"),
        (2, "sordrin@example.org"),
        (1, "kappa+mail@example.net"),
        (2, "kappamail@example.net"),
    ] {
        for _ in 0..times {
            gathering.sent([address], [], 1);
        }
    }
    let people = people(gathering);

    assert_eq!(
        offered_for(&people, "vandel"),
        vec!["k.vandel@example.net", "kvandel@example.net"]
    );
    assert_eq!(
        offered_for(&people, "quorin"),
        vec!["z-quorin@example.com", "zquorin@example.com"]
    );
    assert_eq!(
        offered_for(&people, "ordrin"),
        vec!["s_ordrin@example.org", "sordrin@example.org"]
    );
    assert_eq!(
        offered_for(&people, "mail@"),
        vec!["kappa+mail@example.net", "kappamail@example.net"]
    );
}

#[test]
fn the_domain_holds_what_is_typed_and_begins_none_of_it() {
    let mut gathering = Gathering::default();
    for _ in 0..3 {
        gathering.sent(["kappa@example.net"], [], 1);
    }
    gathering.sent(["sigma.net@example.org"], [], 1);
    let people = people(gathering);

    // Were a part of the domain a start, everybody at it would come first.
    assert_eq!(
        offered_for(&people, "net"),
        vec!["sigma.net@example.org", "kappa@example.net"]
    );
}

#[test]
fn a_name_their_own_mail_gives_them_is_matched_by_its_words_in_their_order() {
    let mut gathering = Gathering::default();
    gathering.sent(["kv@example.net"], [], 10);
    gathering.named("kv@example.net", "Kappa Vandel", 5);
    let people = people(gathering);

    assert_eq!(offered_for(&people, "vandel"), vec!["kv@example.net"]);
    assert_eq!(offered_for(&people, "kappa va"), vec!["kv@example.net"]);
    assert_eq!(
        offered_for(&people, "  Kappa   Vandel "),
        vec!["kv@example.net"]
    );
    assert!(offered_for(&people, "vandel kappa").is_empty());
    assert_eq!(people[0].name.as_deref(), Some("Kappa Vandel"));
}

#[test]
fn typing_meets_a_name_without_its_case_or_its_accents() {
    let mut gathering = Gathering::default();
    gathering.sent(["zq@example.com"], [], 10);
    gathering.named("zq@example.com", "Zéta Quörin", 5);
    let people = people(gathering);

    assert_eq!(offered_for(&people, "zeta"), vec!["zq@example.com"]);
    assert_eq!(offered_for(&people, "QUORIN"), vec!["zq@example.com"]);
    assert_eq!(
        people[0].name.as_deref(),
        Some("Zéta Quörin"),
        "shown as written"
    );
}

#[test]
fn once_what_is_typed_holds_an_at_only_addresses_are_matched() {
    let mut gathering = Gathering::default();
    gathering.sent(["kv@example.net"], [], 10);
    gathering.named("kv@example.net", "Kappa Example", 5);
    let people = people(gathering);

    // As words, `kappa@ex` would be "kappa ex", and would find the name.
    assert!(offered_for(&people, "kappa@ex").is_empty());
    assert_eq!(offered_for(&people, "kappa ex"), vec!["kv@example.net"]);
}

#[test]
fn somebody_who_only_wrote_to_the_mailbox_is_not_offered() {
    let mut gathering = Gathering::default();
    gathering.sent(["kappa@example.net"], [], 10);
    gathering.named("sigma@example.org", "Sigma Ordrin", 20);
    let people = people(gathering);

    assert_eq!(people.len(), 1);
    assert!(offered_for(&people, "sigma").is_empty());
    assert!(offered_for(&people, "ordrin").is_empty());
}

#[test]
fn nothing_is_offered_for_nothing_typed() {
    let mut gathering = Gathering::default();
    gathering.sent(["kappa@example.net"], [], 1);
    gathering.named("kappa@example.net", "Kappa Vandel", 1);
    let people = people(gathering);

    assert!(offered_for(&people, "").is_empty());
    assert!(offered_for(&people, "   ").is_empty());
    // A mark that folds away to nothing is nothing typed, and not a beginning every address has.
    assert!(offered(&people, "\u{0301}", |_: &str| String::new()).is_empty());
}

#[test]
fn nothing_is_offered_for_more_than_an_address_can_hold() {
    let mut gathering = Gathering::default();
    gathering.sent(["kappa@example.net"], [], 1);
    gathering.named("kappa@example.net", "Kappa Vandel", 1);
    let people = people(gathering);

    // Spaces between two words make a line of any length that still reads as those two words.
    let spaces = LONGEST_ADDRESS - "kappa".len() - "vandel".len();
    let longest = format!("kappa{}vandel", " ".repeat(spaces));
    let past_it = format!("kappa{}vandel", " ".repeat(spaces + 1));
    assert_eq!(longest.chars().count(), LONGEST_ADDRESS);

    assert_eq!(offered_for(&people, &longest), vec!["kappa@example.net"]);
    assert!(offered_for(&people, &past_it).is_empty());
}

#[test]
fn at_most_six_are_offered_the_most_written_to_first() {
    let mut gathering = Gathering::default();
    for (times, address) in [
        (1, "p1@example.com"),
        (7, "p7@example.com"),
        (3, "p3@example.com"),
        (5, "p5@example.com"),
        (2, "p2@example.com"),
        (6, "p6@example.com"),
        (4, "p4@example.com"),
    ] {
        for _ in 0..times {
            gathering.sent([address], [], 1);
        }
    }
    let people = people(gathering);

    assert_eq!(MOST_OFFERED, 6);
    assert_eq!(
        offered_for(&people, "p"),
        vec![
            "p7@example.com",
            "p6@example.com",
            "p5@example.com",
            "p4@example.com",
            "p3@example.com",
            "p2@example.com",
        ]
    );
}

#[test]
fn among_people_written_to_as_often_the_more_recent_comes_first_and_then_the_address() {
    let mut gathering = Gathering::default();
    gathering.sent(["kappa@example.net"], [], 100);
    gathering.sent(["kappa.two@example.net"], [], 300);
    gathering.sent(["kappa.three@example.net"], [], 200);
    gathering.sent(["kappa.b@example.net"], [], 50);
    gathering.sent(["kappa.a@example.net"], [], 50);
    let people = people(gathering);

    assert_eq!(
        offered_for(&people, "kappa"),
        vec![
            "kappa.two@example.net",
            "kappa.three@example.net",
            "kappa@example.net",
            "kappa.a@example.net",
            "kappa.b@example.net",
        ]
    );
}

#[test]
fn somebody_named_twice_in_one_message_was_written_to_once() {
    let mut gathering = Gathering::default();
    gathering.sent(
        ["kappa@example.net", "Kappa@Example.NET"],
        ["kappa@example.net"],
        10,
    );
    gathering.sent([], ["kappa@example.net"], 20);
    let people = people(gathering);

    assert_eq!(people.len(), 1);
    assert_eq!(people[0].times, 2);
    assert_eq!(people[0].last, 20);
}

#[test]
fn one_address_in_two_cases_is_one_person_shown_as_the_newest_message_wrote_it() {
    let mut gathering = Gathering::default();
    gathering.sent(["kappa@example.net"], [], 10);
    gathering.sent(["Kappa@Example.net"], [], 30);
    gathering.sent(["KAPPA@example.net"], [], 20);
    let people = people(gathering);

    assert_eq!(people.len(), 1);
    assert_eq!(people[0].address, "Kappa@Example.net");
    assert_eq!(people[0].times, 3);
    assert_eq!(people[0].last, 30);
}

#[test]
fn the_first_hundred_of_each_header_are_counted_and_no_more() {
    let to: Vec<String> = (0..=MOST_COUNTED_OF_A_HEADER)
        .map(|n| format!("to{n}@example.com"))
        .collect();
    let cc: Vec<String> = (0..=MOST_COUNTED_OF_A_HEADER)
        .map(|n| format!("cc{n}@example.com"))
        .collect();
    let mut gathering = Gathering::default();
    gathering.sent(
        to.iter().map(String::as_str),
        cc.iter().map(String::as_str),
        1,
    );
    let people = people(gathering);

    assert_eq!(people.len(), 2 * MOST_COUNTED_OF_A_HEADER);
    let last = format!("to{}@example.com", MOST_COUNTED_OF_A_HEADER - 1);
    let past = format!("to{}@example.com", MOST_COUNTED_OF_A_HEADER);
    assert!(people.iter().any(|person| person.address == last));
    assert!(!people.iter().any(|person| person.address == past));
    assert!(!people
        .iter()
        .any(|person| person.address == format!("cc{MOST_COUNTED_OF_A_HEADER}@example.com")));
}

#[test]
fn an_address_already_written_out_is_not_offered_again_and_a_longer_one_still_is() {
    let people = written_to(&["kappa@example.net", "kappa@example.net.example"]);

    assert_eq!(
        offered_for(&people, "Kappa@example.NET"),
        vec!["kappa@example.net.example"]
    );
    assert_eq!(
        offered_for(&people, "kappa@example.net.example"),
        Vec::<String>::new()
    );
}

#[test]
fn an_address_the_line_of_addresses_would_split_refuse_or_misread_is_not_gathered() {
    let unfit = [
        "kappa,zeta@example.net",
        "kappa;zeta@example.net",
        "<kappa@example.net>",
        "\"kappa\"@example.net",
        "kappa@zeta@example.net",
        "kappa@localhost",
        "kappa@example..net",
        "kappa vandel@example.net",
        "\u{202E}kappa@example.net",
        "kappa\u{2066}@example.net",
        "\u{061C}kappa@example.net",
        "kap\u{200B}pa@example.net",
        "kappa\u{00AD}@example.net",
        "kappa@exam\u{FEFF}ple.net",
        "kappa\u{0007}@example.net",
        "",
        "   ",
    ];
    let longest = format!("{}@example.net", "k".repeat(LONGEST_ADDRESS - 12));
    let too_long = format!("k{longest}");
    assert_eq!(longest.len(), LONGEST_ADDRESS);

    let mut gathering = Gathering::default();
    gathering.sent(unfit.iter().copied().chain([too_long.as_str()]), [], 1);
    assert!(people(gathering).is_empty());

    let mut gathering = Gathering::default();
    gathering.sent([longest.as_str(), "  sigma@example.org  "], [], 1);
    let mut kept: Vec<String> = people(gathering)
        .into_iter()
        .map(|person| person.address)
        .collect();
    kept.sort();
    assert_eq!(kept, vec![longest, "sigma@example.org".to_owned()]);
}

#[test]
fn the_newest_name_wins_whatever_order_the_messages_come_in() {
    let mut gathering = Gathering::default();
    gathering.sent(["kappa@example.net"], [], 1);
    gathering.named("kappa@example.net", "Kappa Vandel", 20);
    gathering.named("Kappa@example.net", "Kappa Ordrin", 40);
    gathering.named("kappa@example.net", "Kappa Quorin", 30);
    let people = people(gathering);

    assert_eq!(people[0].name.as_deref(), Some("Kappa Ordrin"));
    assert_eq!(offered_for(&people, "ordrin"), vec!["kappa@example.net"]);
    assert!(
        offered_for(&people, "vandel").is_empty(),
        "the old name no longer matches"
    );
}

#[test]
fn a_name_is_shown_on_one_line_without_the_characters_that_turn_text_around() {
    let mut gathering = Gathering::default();
    gathering.sent(["kappa@example.net"], [], 1);
    gathering.named(
        "kappa@example.net",
        "\u{202E}Kappa\u{2069}\n\t Vandel\u{200F}\u{0000}",
        1,
    );
    let people = people(gathering);

    assert_eq!(people[0].name.as_deref(), Some("Kappa Vandel"));
}

#[test]
fn a_name_holding_an_address_or_too_long_to_be_a_name_or_empty_is_not_shown() {
    for name in [
        "sigma@example.org",
        "Kappa (sigma@example.org)",
        &"K".repeat(LONGEST_NAME + 1),
        "\u{202E}\u{2066}",
        "   ",
    ] {
        let mut gathering = Gathering::default();
        gathering.sent(["kappa@example.net"], [], 1);
        gathering.named("kappa@example.net", name, 1);
        assert_eq!(people(gathering)[0].name, None, "{name:?} was shown");
    }

    let mut gathering = Gathering::default();
    gathering.sent(["kappa@example.net"], [], 1);
    gathering.named("kappa@example.net", &"K".repeat(LONGEST_NAME), 1);
    assert_eq!(
        people(gathering)[0].name.as_deref(),
        Some("K".repeat(LONGEST_NAME).as_str())
    );
}

#[test]
fn a_name_that_cannot_be_shown_leaves_the_one_before_it() {
    let mut gathering = Gathering::default();
    gathering.sent(["kappa@example.net"], [], 1);
    gathering.named("kappa@example.net", "Kappa Vandel", 10);
    gathering.named("kappa@example.net", "sigma@example.org", 20);
    let people = people(gathering);

    assert_eq!(people[0].name.as_deref(), Some("Kappa Vandel"));
}

#[test]
fn past_the_most_kept_nobody_new_is_counted_and_everybody_counted_still_is() {
    let addresses: Vec<String> = (0..MOST_KEPT)
        .map(|n| format!("p{n}@example.com"))
        .collect();
    let mut gathering = Gathering::default();
    for header in addresses.chunks(MOST_COUNTED_OF_A_HEADER) {
        gathering.sent(header.iter().map(String::as_str), [], 1);
    }
    gathering.sent(["kappa@example.net", "p0@example.com"], [], 2);
    let people = people(gathering);

    assert_eq!(people.len(), MOST_KEPT);
    assert!(!people
        .iter()
        .any(|person| person.address == "kappa@example.net"));
    let first = people
        .iter()
        .find(|person| person.address == "p0@example.com")
        .expect("counted before the bound");
    assert_eq!((first.times, first.last), (2, 2));
}
