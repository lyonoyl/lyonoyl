use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use super::connection_limiter::{
    outcome_of, Change, ConnectionLimiter, Decision, Outcome, Server, Waiting, FIRST_PAUSE,
    LONGEST_PAUSE, PROBE_AT_MOST,
};
use super::{Encryption, Endpoint, TransportError, TransportPolicy, CONNECT_TIMEOUT};

fn address(text: &str) -> SocketAddr {
    text.parse().expect("an address")
}

fn server() -> Server {
    Server::at(&[address("192.0.2.1:993")])
}

const MILLI: Duration = Duration::from_millis(1);
const NOBODY: Waiting = Waiting::Nobody;

/// A limiter whose server stopped answering at `now`.
fn paused_at(now: SystemTime) -> ConnectionLimiter {
    let limiter = ConnectionLimiter::default();
    let first = limiter.decide(&server(), NOBODY, now);
    limiter.record(&server(), first, Outcome::FailedThere, now);
    limiter
}

#[test]
fn a_server_that_answers_is_never_held_back() {
    let limiter = ConnectionLimiter::default();
    let now = SystemTime::now();

    for _ in 0..3 {
        let decision = limiter.decide(&server(), NOBODY, now);
        assert_eq!(decision, Decision::Open(now));
        assert_eq!(
            limiter.record(&server(), decision, Outcome::Answered, now),
            None
        );
    }

    assert_eq!(limiter.paused_servers(), 0);
}

#[test]
fn a_paused_server_gets_one_probe_and_the_others_wait_for_its_answer() {
    let now = SystemTime::now();
    let limiter = paused_at(now);

    assert_eq!(limiter.decide(&server(), NOBODY, now), Decision::Refuse);
    assert_eq!(
        limiter.decide(&server(), NOBODY, now + FIRST_PAUSE - MILLI),
        Decision::Refuse
    );

    let ended = now + FIRST_PAUSE;
    assert_eq!(
        limiter.decide(&server(), NOBODY, ended),
        Decision::Probe(ended)
    );
    // While the probe is on its way, the rest wait for what it hears rather than knock.
    assert_eq!(limiter.decide(&server(), NOBODY, ended), Decision::Wait);
    assert_eq!(
        limiter.decide(&server(), NOBODY, ended + PROBE_AT_MOST - MILLI),
        Decision::Wait
    );
    // And once a probe has had longer than any can take, the next connection probes instead.
    let given_up = ended + PROBE_AT_MOST;
    assert_eq!(
        limiter.decide(&server(), NOBODY, given_up),
        Decision::Probe(given_up)
    );
}

#[test]
fn every_failed_probe_doubles_the_pause_and_the_pause_stops_growing_at_its_longest() {
    let mut now = SystemTime::now();
    let limiter = paused_at(now);

    let mut length = FIRST_PAUSE;
    for _ in 0..5 {
        // Refused until the pause is over, probed the moment it is.
        assert_eq!(
            limiter.decide(&server(), NOBODY, now + length - MILLI),
            Decision::Refuse
        );
        now += length;
        let probe = limiter.decide(&server(), NOBODY, now);
        assert_eq!(probe, Decision::Probe(now));

        let next = limiter.record(&server(), probe, Outcome::FailedThere, now);

        length = (length * 2).min(LONGEST_PAUSE);
        assert_eq!(next, Some(Change::Paused { length }));
    }

    // Thirty seconds, a minute, then two minutes for as long as it lasts.
    assert_eq!(length, LONGEST_PAUSE);
    assert_eq!(LONGEST_PAUSE, FIRST_PAUSE * 4);
}

#[test]
fn connections_opened_before_the_silence_do_not_lengthen_the_pause() {
    // A round opens several before the server stops answering, and each of them fails after, a
    // moment apart: one silence heard late, not a pause each.
    let limiter = ConnectionLimiter::default();
    let now = SystemTime::now();
    let started: Vec<Decision> = (0..8)
        .map(|_| limiter.decide(&server(), NOBODY, now))
        .collect();

    let failed = now + CONNECT_TIMEOUT;
    let heard: Vec<Option<Change>> = started
        .iter()
        .enumerate()
        .map(|(at, decision)| {
            limiter.record(
                &server(),
                *decision,
                Outcome::FailedThere,
                failed + Duration::from_millis(400) * at as u32,
            )
        })
        .collect();

    assert_eq!(
        heard[0],
        Some(Change::Paused {
            length: FIRST_PAUSE
        })
    );
    assert!(heard[1..].iter().all(Option::is_none), "{heard:?}");
    // Counted from the first failure: the later ones did not start it again.
    let ended = failed + FIRST_PAUSE;
    assert_eq!(
        limiter.decide(&server(), NOBODY, ended),
        Decision::Probe(ended)
    );
}

#[test]
fn one_connection_failing_while_the_server_answers_the_others_pauses_nothing() {
    // A busy server lets one handshake of a round's eight run out of time and takes the other
    // seven: one connection failed, and the server did not stop answering.
    let limiter = ConnectionLimiter::default();
    let now = SystemTime::now();
    let started: Vec<Decision> = (0..8)
        .map(|_| limiter.decide(&server(), NOBODY, now))
        .collect();
    for decision in &started[..7] {
        limiter.record(
            &server(),
            *decision,
            Outcome::Answered,
            now + Duration::from_secs(1),
        );
    }

    let failed = now + CONNECT_TIMEOUT;
    assert_eq!(
        limiter.record(&server(), started[7], Outcome::FailedThere, failed),
        None
    );
    assert_eq!(
        limiter.decide(&server(), NOBODY, failed),
        Decision::Open(failed)
    );

    // One that set out after the server last answered, and failed, is a server gone quiet.
    let later = limiter.decide(&server(), NOBODY, failed);
    assert_eq!(
        limiter.record(
            &server(),
            later,
            Outcome::FailedThere,
            failed + CONNECT_TIMEOUT
        ),
        Some(Change::Paused {
            length: FIRST_PAUSE
        })
    );
}

#[test]
fn only_the_current_probe_can_lengthen_the_pause() {
    // A probe that took longer than it may is replaced by another; when the first finally fails,
    // that is not news about the second.
    let now = SystemTime::now();
    let limiter = paused_at(now);
    let first = limiter.decide(&server(), NOBODY, now + FIRST_PAUSE);
    let second_at = now + FIRST_PAUSE + PROBE_AT_MOST;
    let second = limiter.decide(&server(), NOBODY, second_at);
    assert_eq!(second, Decision::Probe(second_at));

    assert_eq!(
        limiter.record(&server(), first, Outcome::FailedThere, second_at),
        None
    );

    // The second is still the probe: everybody else waits for it.
    assert_eq!(
        limiter.decide(&server(), NOBODY, second_at + Duration::from_secs(1)),
        Decision::Wait
    );
    // And its own failure doubles the pause from when it failed.
    let failed = second_at + Duration::from_secs(2);
    limiter.record(&server(), second, Outcome::FailedThere, failed);
    assert_eq!(
        limiter.decide(&server(), NOBODY, failed + FIRST_PAUSE * 2 - MILLI),
        Decision::Refuse
    );
    assert!(matches!(
        limiter.decide(&server(), NOBODY, failed + FIRST_PAUSE * 2),
        Decision::Probe(_)
    ));
}

#[test]
fn a_successful_probe_ends_the_pause_for_every_connection() {
    let now = SystemTime::now();
    let limiter = paused_at(now);

    let ended = now + FIRST_PAUSE;
    let probe = limiter.decide(&server(), NOBODY, ended);

    assert_eq!(
        limiter.record(&server(), probe, Outcome::Answered, ended),
        Some(Change::Resumed { after: FIRST_PAUSE })
    );
    assert_eq!(
        limiter.decide(&server(), NOBODY, ended),
        Decision::Open(ended)
    );
    assert_eq!(limiter.paused_servers(), 0);
}

#[test]
fn the_answer_after_a_long_silence_is_counted_from_its_first_failure() {
    let start = SystemTime::now();
    let limiter = paused_at(start);
    let mut now = start;
    for _ in 0..3 {
        now += LONGEST_PAUSE;
        let probe = limiter.decide(&server(), NOBODY, now);
        limiter.record(&server(), probe, Outcome::FailedThere, now);
    }

    now += LONGEST_PAUSE;
    let probe = limiter.decide(&server(), NOBODY, now);

    assert_eq!(
        limiter.record(&server(), probe, Outcome::Answered, now),
        Some(Change::Resumed {
            after: LONGEST_PAUSE * 4
        })
    );
}

#[test]
fn any_connection_the_server_takes_ends_the_pause() {
    // The server answered somebody: it is answering, whichever connection heard it.
    let limiter = ConnectionLimiter::default();
    let now = SystemTime::now();
    let early = limiter.decide(&server(), NOBODY, now);
    let late = limiter.decide(&server(), NOBODY, now);
    limiter.record(&server(), early, Outcome::FailedThere, now);

    limiter.record(&server(), late, Outcome::Answered, now);

    assert_eq!(limiter.decide(&server(), NOBODY, now), Decision::Open(now));
}

#[test]
fn a_failure_on_this_machine_does_not_pause_the_server() {
    // No network here is not a server that stopped answering, and mail goes on being tried as
    // soon as the network is back rather than when a pause it never earned is over.
    let limiter = ConnectionLimiter::default();
    let now = SystemTime::now();
    let decision = limiter.decide(&server(), NOBODY, now);

    assert_eq!(
        limiter.record(&server(), decision, Outcome::FailedHere, now),
        None
    );
    assert_eq!(limiter.decide(&server(), NOBODY, now), Decision::Open(now));
}

#[test]
fn a_probe_that_failed_on_this_machine_lets_the_next_connection_probe_at_once() {
    let now = SystemTime::now();
    let limiter = paused_at(now);
    let ended = now + FIRST_PAUSE;
    let probe = limiter.decide(&server(), NOBODY, ended);
    limiter.record(&server(), probe, Outcome::FailedHere, ended);

    let next = ended + MILLI;
    assert_eq!(
        limiter.decide(&server(), NOBODY, next),
        Decision::Probe(next)
    );
}

#[test]
fn a_person_s_connection_to_a_paused_server_is_its_probe_at_once() {
    // A refresh, a message to open: the server is asked now rather than when the pause ends, and
    // one at a time, so going through unread mail is not a flood either.
    let now = SystemTime::now();
    let limiter = paused_at(now);

    let asked = limiter.decide(&server(), Waiting::Person, now);
    assert_eq!(asked, Decision::Probe(now));
    assert_eq!(
        limiter.decide(&server(), Waiting::Person, now),
        Decision::Wait,
        "a second while the first is on its way"
    );

    // Answered: everything goes again.
    assert_eq!(
        limiter.record(&server(), asked, Outcome::Answered, now),
        Some(Change::Resumed {
            after: Duration::ZERO
        })
    );
    assert_eq!(limiter.decide(&server(), NOBODY, now), Decision::Open(now));
}

#[test]
fn a_clock_set_back_ends_a_pause_rather_than_stretching_it() {
    let now = SystemTime::now();
    let limiter = paused_at(now);

    let set_back = now - Duration::from_secs(3600);

    assert_eq!(
        limiter.decide(&server(), NOBODY, set_back),
        Decision::Probe(set_back)
    );
}

#[test]
fn a_clock_set_back_after_hours_of_silence_ends_the_pause_too() {
    // The pause in force is the one the last failed probe began, not the first silence hours ago:
    // a clock set back an hour after that is before the pause, and the next connection probes.
    let mut now = SystemTime::now();
    let limiter = paused_at(now);
    for _ in 0..90 {
        now += LONGEST_PAUSE;
        let probe = limiter.decide(&server(), NOBODY, now);
        limiter.record(&server(), probe, Outcome::FailedThere, now);
    }

    let set_back = now - Duration::from_secs(3600);

    assert_eq!(
        limiter.decide(&server(), NOBODY, set_back),
        Decision::Probe(set_back)
    );
}

#[test]
fn a_clock_that_jumps_ahead_and_back_leaves_no_pause_behind() {
    let now = SystemTime::now();
    let limiter = paused_at(now);
    let ahead = now + Duration::from_secs(24 * 3600);
    let probe = limiter.decide(&server(), NOBODY, ahead);
    limiter.record(&server(), probe, Outcome::FailedThere, ahead);

    let back = now + FIRST_PAUSE + Duration::from_secs(5);

    assert_eq!(
        limiter.decide(&server(), NOBODY, back),
        Decision::Probe(back)
    );
}

#[test]
fn a_server_is_its_addresses_whatever_name_or_order_they_came_by() {
    // Two of a provider's customers' domains, each a name for the same machine: pausing one pauses
    // the other.
    let one = address("192.0.2.1:993");
    let six = address("[2001:db8::1]:993");
    assert_eq!(Server::at(&[six, one, one]), Server::at(&[one, six]));

    let now = SystemTime::now();
    let limiter = ConnectionLimiter::default();
    let first = limiter.decide(&Server::at(&[one, six]), NOBODY, now);
    limiter.record(&Server::at(&[one, six]), first, Outcome::FailedThere, now);

    assert_eq!(
        limiter.decide(&Server::at(&[six, one]), NOBODY, now),
        Decision::Refuse
    );
    // Another machine is not held back by it.
    assert_eq!(
        limiter.decide(&Server::at(&[address("198.51.100.7:993")]), NOBODY, now),
        Decision::Open(now)
    );
    // Nor is the same address on another port: one service down is not the machine down.
    assert_eq!(
        limiter.decide(&Server::at(&[address("192.0.2.1:143")]), NOBODY, now),
        Decision::Open(now)
    );
}

#[test]
fn the_outcome_is_read_off_how_the_connection_ended() {
    assert_eq!(outcome_of(&Ok::<(), _>(())), Outcome::Answered);
    // A certificate refused here is a server that took the connection: one mailbox with a wrong
    // name or an expired certificate pauses nothing for the others at the same address.
    assert_eq!(
        outcome_of::<()>(&Err(TransportError::Tls)),
        Outcome::Answered
    );
    assert_eq!(
        outcome_of::<()>(&Err(TransportError::Timeout)),
        Outcome::FailedThere
    );
    assert_eq!(
        outcome_of::<()>(&Err(TransportError::CutOff)),
        Outcome::FailedThere
    );
    for here in [
        TransportError::NoRoute,
        TransportError::Unreachable,
        TransportError::PlaintextRefused,
        TransportError::UnsupportedEncryption,
    ] {
        assert_eq!(
            outcome_of::<()>(&Err(here)),
            Outcome::FailedHere,
            "{here:?}"
        );
    }
}

#[test]
fn a_pause_is_written_down_by_every_build_and_the_answer_after_it_too() {
    use crate::ports::journal::{Note, Value};

    assert_eq!(
        super::connection_limiter::line_for(Change::Paused {
            length: FIRST_PAUSE
        }),
        (
            Note::Trouble,
            "server.paused",
            ("for", Value::Millis(30_000))
        )
    );
    assert_eq!(
        super::connection_limiter::line_for(Change::Resumed {
            after: Duration::from_secs(90)
        }),
        (
            Note::Worth,
            "server.resumed",
            ("after", Value::Millis(90_000))
        )
    );
}

#[test]
fn a_pause_is_held_at_most_until_two_minutes_after_the_last_failed_probe() {
    assert_eq!(FIRST_PAUSE, Duration::from_secs(30));
    assert_eq!(LONGEST_PAUSE, Duration::from_secs(120));
    // A probe is waited for longer than a connection and its handshake can take, so only a probe
    // dropped half way ever runs out of it.
    assert!(PROBE_AT_MOST > CONNECT_TIMEOUT * 2);
}

#[tokio::test]
async fn a_refused_connection_sends_nothing_and_counts_against_its_server() {
    let limiter = paused_at(SystemTime::now());
    let started = AtomicBool::new(false);

    let opened = limiter
        .open(&server(), NOBODY, async {
            started.store(true, Ordering::SeqCst);
            Ok::<(), TransportError>(())
        })
        .await;

    assert_eq!(opened, Err(TransportError::Unreachable));
    assert!(
        !started.load(Ordering::SeqCst),
        "a refused connection was opened"
    );
    assert_eq!(limiter.refusals_of(&server()), 1);
}

#[tokio::test]
async fn the_limiter_records_how_each_connection_it_opened_ended() {
    // No answer pauses the server.
    let limiter = ConnectionLimiter::default();
    let _ = limiter
        .open(&server(), NOBODY, async {
            Err::<(), _>(TransportError::Timeout)
        })
        .await;
    assert_eq!(
        limiter.decide(&server(), NOBODY, SystemTime::now()),
        Decision::Refuse
    );

    // A probe that is not answered doubles the pause.
    let limiter = paused_at(SystemTime::now() - FIRST_PAUSE);
    let _ = limiter
        .open(&server(), NOBODY, async {
            Err::<(), _>(TransportError::Timeout)
        })
        .await;
    let now = SystemTime::now();
    assert_eq!(
        limiter.decide(&server(), NOBODY, now + FIRST_PAUSE),
        Decision::Refuse,
        "still paused half way through a doubled pause"
    );

    // A probe that failed on this machine hands probing to the next connection.
    let limiter = paused_at(SystemTime::now() - FIRST_PAUSE);
    let _ = limiter
        .open(&server(), NOBODY, async {
            Err::<(), _>(TransportError::NoRoute)
        })
        .await;
    assert!(matches!(
        limiter.decide(&server(), NOBODY, SystemTime::now()),
        Decision::Probe(_)
    ));

    // A probe the server takes lets everything through.
    let limiter = paused_at(SystemTime::now() - FIRST_PAUSE);
    let probed = limiter
        .open(&server(), NOBODY, async { Ok::<(), TransportError>(()) })
        .await;
    assert!(probed.is_ok());
    assert!(matches!(
        limiter.decide(&server(), NOBODY, SystemTime::now()),
        Decision::Open(_)
    ));
}

/// The probe of a server paused a whole pause ago, held until `answer` is sent, and a connection
/// that comes while it is on its way: answers with whether that one opened, and how.
async fn behind_a_probe_that(
    answer: Result<(), TransportError>,
) -> (bool, Result<(), TransportError>) {
    let limiter = paused_at(SystemTime::now() - FIRST_PAUSE);
    let (tell, told) = tokio::sync::oneshot::channel::<()>();
    let started = AtomicBool::new(false);
    let server = server();

    let probe = limiter.open(&server, NOBODY, async move {
        let _ = told.await;
        answer
    });
    let behind = async {
        // Asked once the probe is on its way.
        tokio::task::yield_now().await;
        limiter
            .open(&server, NOBODY, async {
                started.store(true, Ordering::SeqCst);
                Ok::<(), TransportError>(())
            })
            .await
    };
    let release = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = tell.send(());
    };

    let (_, behind, ()) = tokio::join!(probe, behind, release);
    (started.load(Ordering::SeqCst), behind)
}

#[tokio::test]
async fn a_connection_that_comes_while_the_probe_is_on_its_way_opens_once_it_is_answered() {
    let (started, opened) = behind_a_probe_that(Ok(())).await;

    assert!(started, "it waited for the probe and never went");
    assert!(opened.is_ok());
}

#[tokio::test]
async fn a_connection_that_comes_while_the_probe_is_on_its_way_sends_nothing_once_it_fails() {
    let (started, opened) = behind_a_probe_that(Err(TransportError::Timeout)).await;

    assert!(!started, "it went to a server its probe found silent");
    assert_eq!(opened, Err(TransportError::Unreachable));
}

#[tokio::test]
async fn a_mail_connection_to_a_paused_server_is_refused_before_anything_is_sent() {
    // Through the transport every mail connection uses, at an address no other test pauses: the
    // limiter is one for the whole process.
    let paused = address("192.0.2.77:993");
    let limiter = super::connection_limiter::connection_limiter();
    let first = limiter.decide(&Server::at(&[paused]), NOBODY, SystemTime::now());
    limiter.record(
        &Server::at(&[paused]),
        first,
        Outcome::FailedThere,
        SystemTime::now(),
    );
    let before = limiter.refusals_of(&Server::at(&[paused]));

    let opened = TransportPolicy::new()
        .expect("a policy")
        .connect_retrying(&Endpoint {
            host: "192.0.2.77".to_owned(),
            port: 993,
            encryption: Encryption::Tls,
        })
        .await;

    assert!(matches!(opened, Err(TransportError::Unreachable)));
    // Every try refused here, the retries included: nothing reached the server.
    assert_eq!(
        limiter.refusals_of(&Server::at(&[paused])) - before,
        3,
        "a try went past the limiter"
    );
}

#[tokio::test(start_paused = true)]
async fn a_mail_connection_a_person_waits_for_is_not_refused_by_a_pause() {
    // The same, for a person: the connection goes as the probe. Paused time ends the attempt at
    // its deadline at once, at a documentation address nothing answers.
    let paused = address("192.0.2.78:993");
    let limiter = super::connection_limiter::connection_limiter();
    let first = limiter.decide(&Server::at(&[paused]), NOBODY, SystemTime::now());
    limiter.record(
        &Server::at(&[paused]),
        first,
        Outcome::FailedThere,
        SystemTime::now(),
    );
    let before = limiter.refusals_of(&Server::at(&[paused]));

    let _ = TransportPolicy::new()
        .expect("a policy")
        .connect_retrying_for(
            &Endpoint {
                host: "192.0.2.78".to_owned(),
                port: 993,
                encryption: Encryption::Tls,
            },
            Waiting::Person,
        )
        .await;

    assert_eq!(limiter.refusals_of(&Server::at(&[paused])), before);
}

#[test]
fn only_mail_to_another_machine_is_limited() {
    let elsewhere = [address("192.0.2.1:993")];
    let here = [address("127.0.0.1:1143"), address("[::1]:1143")];

    let mail = super::limiter_for(true, &elsewhere).expect("mail to another machine");
    // One limiter for the whole process: a provider builds a transport of its own for every pass.
    assert!(std::ptr::eq(
        mail,
        super::connection_limiter::connection_limiter()
    ));

    assert!(
        super::limiter_for(true, &here).is_none(),
        "a server on this machine needs no protecting"
    );
    assert!(
        super::limiter_for(false, &elsewhere).is_none(),
        "a picture's host is whoever wrote the message, not a provider"
    );
}

#[test]
fn where_one_attempt_failed_is_read_off_its_error() {
    use super::Failed;
    use std::io::{Error, ErrorKind};

    for here in [
        ErrorKind::NetworkUnreachable,
        ErrorKind::NetworkDown,
        ErrorKind::AddrNotAvailable,
        ErrorKind::AddrInUse,
        ErrorKind::PermissionDenied,
    ] {
        assert!(
            matches!(
                super::where_it_failed(Some(&Error::from(here))),
                Failed::Here
            ),
            "{here:?}"
        );
    }
    for there in [
        ErrorKind::ConnectionRefused,
        ErrorKind::ConnectionReset,
        ErrorKind::HostUnreachable,
        ErrorKind::TimedOut,
        ErrorKind::Other,
    ] {
        assert!(
            matches!(
                super::where_it_failed(Some(&Error::from(there))),
                Failed::There
            ),
            "{there:?}"
        );
    }
    // An attempt given up on after its deadline sent something and heard nothing.
    assert!(matches!(super::where_it_failed(None), Failed::There));
}

#[test]
fn a_connection_no_address_took_says_whether_anything_went_out() {
    assert_eq!(super::none_connected(true), TransportError::Timeout);
    assert_eq!(super::none_connected(false), TransportError::NoRoute);
}

#[test]
fn a_handshake_cut_off_is_told_apart_from_a_certificate_refused() {
    use std::io::{Error, ErrorKind};

    for cut in [
        ErrorKind::UnexpectedEof,
        ErrorKind::ConnectionReset,
        ErrorKind::ConnectionAborted,
        ErrorKind::BrokenPipe,
        ErrorKind::NotConnected,
        ErrorKind::Other,
    ] {
        assert_eq!(
            super::handshake_failed(&Error::from(cut)),
            TransportError::CutOff,
            "{cut:?}"
        );
    }
    // What rustls refuses — a certificate, a name, an alert — comes as invalid data, and is the
    // one kind that is a server that answered.
    assert_eq!(
        super::handshake_failed(&Error::from(ErrorKind::InvalidData)),
        TransportError::Tls
    );
}

#[tokio::test]
async fn a_closed_port_is_the_other_end_turning_the_connection_away() {
    // A port nothing listens on answers with a refusal, which is a server turning the connection
    // away rather than this machine failing to send it.
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listening| listening.local_addr())
        .expect("a port to close");

    let opened = super::whichever_answers_first(&[closed]).await;

    assert_eq!(opened.err(), Some(TransportError::Timeout));
}
