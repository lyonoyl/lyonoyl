//! Whether the development mail server is answering, and what a run does when it is not.
//!
//! Some tests ask a real mail server what it did — including the two that prove this application
//! removes the message it named and nothing else, and the four that hand a message to its
//! submission port and then look in the recipient's inbox. On a machine where nobody started one,
//! those tests skipping themselves is right: somebody running `cargo test` to check an unrelated
//! change should not be told to install Docker first.
//!
//! Everywhere the server is part of the run, it is wrong, and quietly so. A suite that reports
//! green about mail it never touched says the same word as one that touched all of it, and the
//! difference is invisible in a log nobody reads.
//!
//! `LYONOYL_MAIL_SERVER_REQUIRED` is what tells the two situations apart. Where it is set — the
//! preflight script — a server that is not answering fails the run instead of silently reducing
//! it.

/// Where the development mail server is read over IMAP. Loopback and nowhere else: it is a fixture.
const IMAP: &str = "127.0.0.1:1143";

/// Where it takes mail to send, as an account is given it: a host and a port.
///
/// Loopback for the same reason as IMAP, and it is also the one kind of address the submission
/// adapter will sign in to without encryption. It carries mail for the fixture domains and relays
/// nothing anywhere else.
pub const SUBMISSION: (&str, u16) = ("127.0.0.1", 1587);

/// The variable that turns a skipped test into a failed run.
const REQUIRED: &str = "LYONOYL_MAIL_SERVER_REQUIRED";

/// How long a connection to either port is given to complete.
const PATIENCE: std::time::Duration = std::time::Duration::from_millis(300);

/// Whether the development mail server is answering on its IMAP port.
pub fn answering() -> bool {
    may_skip(reachable(IMAP), demanded(std::env::var_os(REQUIRED)), IMAP)
}

/// Whether it is answering on its submission port as well.
///
/// Asked by the tests that send: each of them also looks in the recipient's inbox, so a run that
/// demanded a server is failed by whichever of the two is missing, under the address that is.
pub fn submission_answering() -> bool {
    let submission = submission_address();

    answering()
        && may_skip(
            reachable(&submission),
            demanded(std::env::var_os(REQUIRED)),
            &submission,
        )
}

/// The submission address written the way a socket is given one.
fn submission_address() -> String {
    let (host, port) = SUBMISSION;
    format!("{host}:{port}")
}

/// Whether the run declared it needs a server.
///
/// An empty value is not a demand. It reads like a pedantic distinction and is not: a caller that
/// sets the variable from something it did not find — a shell writing `VAR=$missing`, a script
/// deciding per platform — writes a defined variable holding nothing. Read as a demand, that turns
/// every machine with no server into a machine whose every run fails.
fn demanded(value: Option<std::ffi::OsString>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}

/// Whether something is listening at an address, and what the connection met when it is not. The
/// only part of this that touches the world.
fn reachable(address: &str) -> Result<(), String> {
    std::net::TcpStream::connect_timeout(&address.parse().expect("a literal address"), PATIENCE)
        .map(drop)
        .map_err(|error| unanswered(&error))
}

/// What a connection that was not made met. A refusal is a port nothing listens on; a connection
/// that does not complete in time was held on the way.
fn unanswered(error: &std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::ConnectionRefused => "the connection was refused".to_owned(),
        std::io::ErrorKind::TimedOut => {
            format!("no connection within {} ms", PATIENCE.as_millis())
        }
        _ => error.to_string(),
    }
}

/// What a run does about a server that is not there.
///
/// Separate from reaching for the socket so the decision can be checked without one, which is the
/// only way to prove the failing half: a test that needed a machine with no mail server on it to
/// demonstrate what happens without a mail server would never run anywhere it mattered.
///
/// Panics rather than returning `false` where the run declared it needs a server. The failure has
/// to happen here, in the test that was about to skip, rather than as a green tick somebody reads
/// as coverage.
fn may_skip(reachable: Result<(), String>, required: bool, address: &str) -> bool {
    if let Err(why) = &reachable {
        assert!(
            !required,
            "{REQUIRED} is set and nothing is answering on {address}: {why}. This run was told the \
             development mail server is part of it, so a test that asks a real server must not \
             skip itself and pass. Start it with: docker compose -f docker/mail-server/compose.yml \
             up -d --build --wait"
        );
    }

    reachable.is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused() -> Result<(), String> {
        Err(unanswered(&std::io::ErrorKind::ConnectionRefused.into()))
    }

    #[test]
    fn a_server_that_is_answering_is_used_whether_or_not_it_was_demanded() {
        assert!(may_skip(Ok(()), false, IMAP));
        assert!(may_skip(Ok(()), true, IMAP));
    }

    #[test]
    fn a_missing_server_lets_a_test_skip_itself_where_nobody_asked_for_one() {
        // Somebody checking an unrelated change should not be told to install Docker first.
        assert!(!may_skip(refused(), false, IMAP));
        assert!(!may_skip(refused(), false, &submission_address()));
    }

    #[test]
    fn a_variable_set_to_nothing_is_not_a_demand() {
        // What a caller writes when it sets the variable from something it did not find. Read as a
        // demand, it fails every run on a machine that has no server on purpose.
        assert!(!demanded(Some(std::ffi::OsString::from(""))));
        assert!(!demanded(None));
        assert!(demanded(Some(std::ffi::OsString::from("1"))));
    }

    #[test]
    #[should_panic(expected = "LYONOYL_MAIL_SERVER_REQUIRED")]
    fn a_missing_server_fails_a_run_that_said_it_needs_one() {
        // The whole reason this module exists. The tests that ask a real server would otherwise
        // skip themselves and report green about mail nothing touched.
        may_skip(refused(), true, IMAP);
    }

    #[test]
    #[should_panic(expected = "nothing is answering on 127.0.0.1:1587")]
    fn a_missing_submission_port_fails_the_run_under_its_own_address() {
        // An IMAP server that answers and a submission port that does not is the state an old
        // image leaves behind, and the failure has to say which of the two is missing rather than
        // send somebody to look at the one that works.
        may_skip(refused(), true, &submission_address());
    }

    #[test]
    #[should_panic(
        expected = "nothing is answering on 127.0.0.1:1143: the connection was refused."
    )]
    fn a_failed_run_says_nothing_was_listening_where_the_connection_was_refused() {
        may_skip(refused(), true, IMAP);
    }

    #[test]
    #[should_panic(
        expected = "nothing is answering on 127.0.0.1:1143: no connection within 300 ms."
    )]
    fn a_failed_run_says_the_connection_did_not_complete_where_it_timed_out() {
        may_skip(
            Err(unanswered(&std::io::ErrorKind::TimedOut.into())),
            true,
            IMAP,
        );
    }

    #[test]
    fn any_other_failure_is_given_in_the_systems_own_words() {
        let error = std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, "no port left");

        assert_eq!(unanswered(&error), "no port left");
    }

    #[test]
    fn both_ports_are_on_this_machine_and_nowhere_else() {
        for address in [IMAP.to_owned(), submission_address()] {
            let parsed: std::net::SocketAddr = address.parse().expect("a literal address");
            assert!(parsed.ip().is_loopback(), "{address} is not loopback");
        }
    }

    #[test]
    fn the_address_checked_is_the_address_the_imap_tests_connect_to() {
        // Two places name the IMAP port: this one, which decides whether to skip, and the adapter's
        // test endpoint, which is connected to. Apart, a moved port would have every test skip
        // itself against a server that is answering somewhere else.
        let endpoint = crate::infrastructure::imap::development_endpoint();

        assert_eq!(format!("{}:{}", endpoint.host, endpoint.port), IMAP);
    }
}
