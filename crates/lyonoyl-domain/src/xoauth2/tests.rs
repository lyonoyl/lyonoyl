use super::*;

/// Google's own published example, decoded. The expectation is their bytes, not ours.
///
/// Written out rather than base64-decoded in the test, because this crate has no base64 and
/// because the decoded form is the thing being asserted: a reader can see the two `\x01` at the
/// end and the single space after `Bearer`, which is what a reader of the encoded blob cannot.
#[test]
fn matches_the_example_google_publishes() {
    let built = initial_response(
        "someuser@example.com",
        "ya29.vF9dft4qmTc2Nvb3RlckBhdHRhdmlzdGEuY29tCg",
    )
    .expect("a plain user and token");

    let mut expected = Vec::new();
    expected.extend_from_slice(b"user=someuser@example.com");
    expected.push(0x01);
    expected.extend_from_slice(b"auth=Bearer ya29.vF9dft4qmTc2Nvb3RlckBhdHRhdmlzdGEuY29tCg");
    expected.push(0x01);
    expected.push(0x01);

    assert_eq!(built, expected);
}

/// Microsoft's example decodes to the same shape, so one builder serves both.
#[test]
fn matches_the_example_microsoft_publishes() {
    let built = initial_response(
        "test@contoso.onmicrosoft.com",
        "EwBAAl3BAAUFFpUAo7J3Ve0bjLBWZWCclRC3EoAA",
    )
    .expect("a plain user and token");

    let text = String::from_utf8(built).expect("the parts are text");

    assert_eq!(
        text,
        "user=test@contoso.onmicrosoft.com\u{1}auth=Bearer EwBAAl3BAAUFFpUAo7J3Ve0bjLBWZWCclRC3EoAA\u{1}\u{1}"
    );
}

#[test]
fn there_is_exactly_one_space_after_bearer() {
    // The single most likely thing to get wrong, and the one a server rejects with a message that
    // reads like a wrong password.
    let built = initial_response("a@b.c", "T").expect("valid");
    let text = String::from_utf8(built).expect("text");

    assert!(text.contains("auth=Bearer T"));
    assert!(!text.contains("auth=Bearer  T"), "two spaces");
    assert!(!text.contains("auth=BearerT"), "no space");
}

#[test]
fn there_are_two_separators_at_the_end_and_one_in_the_middle() {
    let built = initial_response("a@b.c", "T").expect("valid");

    assert_eq!(
        built.iter().filter(|byte| **byte == 0x01).count(),
        3,
        "one between the fields and two closing"
    );
    assert_eq!(&built[built.len() - 2..], &[0x01, 0x01]);
}

#[test]
fn a_token_carrying_the_separator_is_refused_rather_than_escaped() {
    // The mechanism has no escape. A token holding "\x01auth=Bearer …" would append a second
    // field of somebody else's choosing to a string the server reads as one credential.
    assert_eq!(
        initial_response("a@b.c", "good\u{1}auth=Bearer evil"),
        Err(Xoauth2Error::HoldsTheSeparator)
    );
    assert_eq!(
        initial_response("a\u{1}b@c.d", "T"),
        Err(Xoauth2Error::HoldsTheSeparator)
    );
}

#[test]
fn neither_half_may_be_empty() {
    // Sent, this authenticates as nobody with nothing, and the answer reads like a wrong password.
    assert_eq!(initial_response("", "T"), Err(Xoauth2Error::Empty));
    assert_eq!(initial_response("a@b.c", ""), Err(Xoauth2Error::Empty));
}

#[test]
fn a_user_name_that_is_not_the_account_is_still_built() {
    // Microsoft's shared mailboxes: the token belongs to the person, the user field names the
    // mailbox. The mechanism does not care that they differ, and neither does this.
    let built = initial_response("shared@example.com", "T").expect("valid");

    assert!(String::from_utf8(built)
        .expect("text")
        .starts_with("user=shared@example.com"));
}
