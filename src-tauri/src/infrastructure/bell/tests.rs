use super::*;

/// Short enough that a test is instant, long enough that the two are told apart.
fn bell() -> Bell {
    Bell::ringing_after(Duration::from_millis(20), Duration::from_millis(60))
}

#[test]
fn nothing_is_said_before_the_wave_has_had_its_time() {
    let bell = bell();
    let start = Instant::now();

    bell.reports("acc-1", 3, start);

    assert_eq!(bell.what_to_say(start + Duration::from_millis(5)), None);
    assert_eq!(bell.what_to_say(start + Duration::from_millis(25)), Some(3));
}

#[test]
fn a_hundred_accounts_finishing_at_once_ring_once() {
    // The whole reason this exists. Ringing per account is a machine that pings until somebody
    // turns it off, and turning it off is the outcome this feature is trying to avoid.
    let bell = bell();
    let start = Instant::now();

    for at in 0..100 {
        bell.reports(
            &format!("acc-{at}"),
            1,
            start + Duration::from_millis(at % 15),
        );
    }

    assert_eq!(
        bell.what_to_say(start + Duration::from_millis(30)),
        Some(100)
    );
    assert_eq!(
        bell.what_to_say(start + Duration::from_millis(200)),
        None,
        "and the wave is spent"
    );
}

#[test]
fn one_account_reporting_twice_in_a_wave_is_not_added_to_itself() {
    // A pass reports what that pass found. Summing two reports from one account would count the
    // same three messages as six the moment a round overlapped a watch.
    let bell = bell();
    let start = Instant::now();

    bell.reports("acc-1", 3, start);
    bell.reports("acc-1", 5, start + Duration::from_millis(5));

    assert_eq!(bell.what_to_say(start + Duration::from_millis(25)), Some(5));
}

#[test]
fn the_quiet_after_a_ring_is_kept() {
    let bell = bell();
    let start = Instant::now();

    bell.reports("acc-1", 1, start);
    assert_eq!(bell.what_to_say(start + Duration::from_millis(25)), Some(1));

    bell.reports("acc-2", 1, start + Duration::from_millis(30));
    assert_eq!(
        bell.what_to_say(start + Duration::from_millis(60)),
        None,
        "the second interruption in a minute tells nobody anything the first did not"
    );
}

#[test]
fn a_wave_refused_by_the_quiet_is_emptied_rather_than_held() {
    // Held, it would ring about mail from a minute ago the moment the quiet ended — an
    // interruption about something that stopped being news while it waited.
    let bell = bell();
    let start = Instant::now();

    bell.reports("acc-1", 1, start);
    bell.what_to_say(start + Duration::from_millis(25));

    bell.reports("acc-2", 7, start + Duration::from_millis(30));
    assert_eq!(bell.what_to_say(start + Duration::from_millis(60)), None);

    assert_eq!(
        bell.what_to_say(start + Duration::from_millis(500)),
        None,
        "what was refused is gone rather than waiting for the quiet to end"
    );
}

#[test]
fn a_pass_that_found_nothing_opens_no_wave() {
    let bell = bell();
    let start = Instant::now();

    bell.reports("acc-1", 0, start);

    assert_eq!(bell.what_to_say(start + Duration::from_millis(500)), None);
}

#[test]
fn a_wave_that_follows_a_quiet_one_rings_normally() {
    // The bell must not fall silent for ever after refusing once.
    let bell = bell();
    let start = Instant::now();

    bell.reports("acc-1", 1, start);
    assert_eq!(bell.what_to_say(start + Duration::from_millis(25)), Some(1));

    let later = start + Duration::from_millis(200);
    bell.reports("acc-2", 4, later);

    assert_eq!(bell.what_to_say(later + Duration::from_millis(25)), Some(4));
}
