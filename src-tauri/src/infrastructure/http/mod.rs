//! The one HTTP client, and the policy every request in this application goes through.
//!
//! Low on purpose: it knows how to make a request and how to refuse one, and nothing about what the
//! request is for. What a request is for lives in a service beside it — `pictures.rs` is the first
//! — and a service is what a use case sees, through a port, so nothing above this module names a
//! URL library or a status code.
//!
//! The policy is here rather than at each call site because the things it decides are the things
//! that go wrong quietly: a redirect chain that ends somewhere else, a body that never ends, a
//! request that says which operating system asked it.

use std::net::IpAddr;
use std::sync::Arc;

use crate::infrastructure::net;

use thiserror::Error;

/// How many times a request may be sent somewhere else before it is refused.
///
/// Redirects are how one address becomes another, and a message is written by someone else. Three
/// is enough for the ordinary content-delivery chain and short enough to be a chain.
const REDIRECTS: usize = 3;

/// What this application says it is.
///
/// The name and nothing else. A user agent that carries the operating system and its version tells
/// every host a message links to what this machine is running — which is a thing worth knowing to
/// somebody, and none of their business. This build's own version is no different: it says which
/// faults the reader's copy still has.
const AGENT: &str = "LyonoylMail";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HttpError {
    /// The host could not be reached, or did not answer in time.
    #[error("the host could not be reached")]
    Offline,
    /// The host answered, and the answer was no.
    #[error("the host refused the request")]
    Refused,
    /// The answer was larger than the caller was willing to hold.
    #[error("the answer is larger than this build reads")]
    TooLarge,
    /// The address is not one this build will fetch.
    #[error("that address is not one this build fetches")]
    Address,
}

/// What came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// As the host stated it, lowercased and without its parameters. What it is used for is a
    /// caller's decision — this module makes none.
    pub content_type: String,
    pub bytes: Vec<u8>,
}

pub struct HttpClient {
    /// The road. The same one mail travels: this project's own deadlines, its own address policy,
    /// and every address a name answers with tried two hundred and fifty milliseconds apart.
    policy: Arc<net::TransportPolicy>,
    /// Whether each address a request is sent on to is held to the policy.
    judging_hops: bool,
    /// Whether an address written out, and every name looked up, is held to the policy. False only
    /// in this module's own tests, where the host answering is on this machine.
    judging_addresses: bool,
    /// How many addresses a chain of redirects may ask, the first one included.
    chain: usize,
}

impl HttpClient {
    /// Builds the client on this application's transport policy.
    ///
    /// The policy is passed in rather than made here: which certificate authorities this machine
    /// trusts is one question with one answer, and the mail transport already asked it. It is the
    /// whole policy and not only its certificates, because the road underneath is now the same
    /// road — see [`asking_a_host`] for what that buys.
    pub fn new(policy: Arc<net::TransportPolicy>) -> Self {
        Self::build(policy, true, true)
    }

    /// The same client without the address policy, so a test can be answered by a host on this
    /// machine. Never built outside a test: reaching this machine is the whole of what the policy
    /// above exists to prevent.
    #[cfg(test)]
    pub(crate) fn reaching_anywhere(policy: Arc<net::TransportPolicy>) -> Self {
        Self::build(policy, false, false)
    }

    /// The same client, following a chain of redirects to at most `addresses` addresses, the first
    /// one included.
    ///
    /// For a service whose host chooses where each file is served from, and whose answers are held
    /// to a seal and a digest rather than to where they came from. Every hop is judged as before.
    pub fn following_at_most(self, addresses: usize) -> Self {
        Self {
            chain: addresses,
            ..self
        }
    }

    /// Reaches this machine on the first address and judges every hop after it.
    ///
    /// The two questions were one flag, and a redirect is the half that had nothing on it: a test
    /// that turned the policy off to be answered at all turned off the check it was there to
    /// prove. This turns off only the first — the shipped client
    /// ([`new`](HttpClient::new)) still asks both.
    #[cfg(test)]
    pub(crate) fn judging_only_the_hops(policy: Arc<net::TransportPolicy>) -> Self {
        Self::build(policy, true, false)
    }

    fn build(
        policy: Arc<net::TransportPolicy>,
        judging_hops: bool,
        judging_addresses: bool,
    ) -> Self {
        Self {
            policy,
            judging_hops,
            judging_addresses,
            chain: REDIRECTS,
        }
    }

    /// Fetches one address, reading no more than `at_most` bytes of the answer.
    ///
    /// The limit is enforced twice: once on what the host says the length is, and again on what it
    /// actually sends — a host that states one length and sends another is a host, not a mistake.
    ///
    /// Redirects are followed here rather than below, because following one is a decision about
    /// where a request may go and the file below decides nothing: every hop is judged, not just
    /// counted, since a redirect is how one address becomes another and the address it becomes is
    /// chosen by whoever sent the message.
    /// Fetches one address, saying what kind of answer is wanted and reading no more than `at_most`
    /// bytes of it.
    ///
    /// `accepting` is not a courtesy. A host is entitled to refuse a request for a document that
    /// asks only for pictures, and one that does refuses with a status nothing downstream can
    /// explain — so the caller says what it is for, and there is no default to forget to change.
    pub async fn get(
        &self,
        url: &str,
        accepting: &str,
        at_most: usize,
    ) -> Result<Fetched, HttpError> {
        self.get_watching(url, accepting, at_most, &mut |_, _| true)
            .await
    }

    /// The same, told how much has arrived as it arrives.
    ///
    /// For the one thing here big enough that somebody waits on it — an update. A picture and a
    /// configuration file are fetched and either appear or do not; a download of tens of megabytes
    /// with nothing on screen reads as a dead control, and this project has a rule about that.
    ///
    /// Answering `false` stops it.
    pub async fn get_watching(
        &self,
        url: &str,
        accepting: &str,
        at_most: usize,
        watching: asking_a_host::Arriving<'_>,
    ) -> Result<Fetched, HttpError> {
        self.asking_as(AGENT, url, accepting, at_most, watching)
            .await
    }

    /// Where every request is made, saying it comes from `agent`.
    async fn asking_as(
        &self,
        agent: &str,
        url: &str,
        accepting: &str,
        at_most: usize,
        watching: asking_a_host::Arriving<'_>,
    ) -> Result<Fetched, HttpError> {
        if !fetchable(url) || (self.judging_addresses && !host_is_reachable(url)) {
            return Err(HttpError::Address);
        }

        let mut at = url::Url::parse(url).map_err(|_| HttpError::Address)?;
        // Addresses asked, the first one included. The limit is on how long a chain may be rather
        // than on how many times it may bend, so the first request is already one of them.
        let mut asked = 1usize;

        loop {
            let answered = asking_a_host::ask(
                &self.policy,
                &at,
                agent,
                accepting,
                at_most,
                self.judging_addresses,
                watching,
            )
            .await?;

            let Some(elsewhere) = answered.sent_on_to else {
                return match (200..300).contains(&answered.status) {
                    true => Ok(Fetched {
                        content_type: answered.content_type,
                        bytes: answered.bytes,
                    }),
                    false => Err(HttpError::Refused),
                };
            };

            // A host that answers 200 and a `Location` is answering, not sending on.
            if !(300..400).contains(&answered.status) {
                return match (200..300).contains(&answered.status) {
                    true => Ok(Fetched {
                        content_type: answered.content_type,
                        bytes: answered.bytes,
                    }),
                    false => Err(HttpError::Refused),
                };
            }

            if asked >= self.chain {
                return Err(HttpError::Offline);
            }

            at = the_next_hop(&at, &elsewhere, self.judging_hops)?;
            asked += 1;
        }
    }
}

/// Where a redirect sends a request next, if it may be sent there at all.
fn the_next_hop(at: &url::Url, location: &str, judging: bool) -> Result<url::Url, HttpError> {
    // Resolved against where the request actually went, because a `Location` is allowed to be a
    // path, and a path resolved against the first address in a chain points somewhere nobody named.
    let next = at.join(location).map_err(|_| HttpError::Offline)?;

    match may_follow(at, &next)
        && fetchable(next.as_str())
        && (!judging || host_is_reachable(next.as_str()))
    {
        true => Ok(next),
        false => Err(HttpError::Offline),
    }
}

/// Whether a request may be sent on from one address to the next.
///
/// Never out of encryption. A request that began encrypted reached the host it named and nobody
/// else, and a hop to a plain address hands the rest of the exchange to whoever is on the path —
/// for a configuration document, the rest of the exchange decides where a password goes. Into
/// encryption, and on within either, is the host's business.
fn may_follow(from: &url::Url, to: &url::Url) -> bool {
    from.scheme() != "https" || to.scheme() == "https"
}

/// Whether a name means this machine or the segment it is on.
///
/// Refused before any lookup, because a lookup is the wrong instrument for both: `localhost` is
/// answered from a file rather than by a name server, and a `.local` name is answered by whatever
/// is on the local network — asking is itself a packet sent to the neighbours on behalf of a
/// message.
///
/// Its own function because nothing downstream can tell it apart: every name it catches would be
/// refused anyway by the two rules below, so a test written against `resolve` alone stays green
/// with this deleted, and the query it was there to prevent goes out.
///
/// The suffixes carry their dot, so `localhostings.example` and `notlocalhost.example` are ordinary
/// names and stay ordinary.
fn a_name_that_means_here(host: &str) -> bool {
    // The root dot comes off first. A name is allowed to carry it — `printer.local.` is the same
    // name written absolutely, and the URL parser keeps it — so a comparison that did not would
    // let the one form through that a person never types and an attacker would.
    let lowered = host.to_ascii_lowercase();
    let lowered = lowered.strip_suffix('.').unwrap_or(&lowered);

    lowered == "localhost" || lowered.ends_with(".local") || lowered.ends_with(".localhost")
}

/// Whether an address written as a number is one a message may reach.
///
/// A name goes through the resolver above; an address written out does not go through a resolver at
/// all, so it is checked where it is read.
fn host_is_reachable(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };

    match parsed.host() {
        Some(url::Host::Ipv4(address)) => net::reachable_by_a_message(IpAddr::V4(address)),
        Some(url::Host::Ipv6(address)) => net::reachable_by_a_message(IpAddr::V6(address)),
        // A name is the resolver's business, and it is checked there.
        Some(url::Host::Domain(_)) => true,
        None => false,
    }
}

/// Whether an address is one this build fetches at all.
///
/// Two schemes. Not `file:`, which would read this machine; not a scheme some other application
/// registered, which would hand it whatever that application does.
pub fn fetchable(url: &str) -> bool {
    url.split_once(':').is_some_and(|(scheme, rest)| {
        (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
            && rest.starts_with("//")
    })
}

mod asking_a_host;
pub mod autoconfig;
pub mod pictures;

/// A host that exists only while a test runs.
#[cfg(test)]
pub(crate) mod testing {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// A host that answers once, on a port the operating system picks, and remembers what it was
    /// asked. No network leaves this machine, and no test waits on one.
    pub(crate) fn host(answer: &'static str) -> (String, mpsc::Receiver<String>) {
        let (root, heard) = host_answering(vec![answer.to_owned()]);

        (format!("{root}thing"), heard)
    }

    /// A host that answers one request after another with the answers given, in order, and then
    /// stops listening — so a test can be a place asked for one file and then for the seal beside
    /// it, and a request past the last answer finds nobody there. Answers with the address of its
    /// root, and remembers every request.
    pub(crate) fn host_answering(answers: Vec<String>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let (asked, heard) = mpsc::channel();

        std::thread::spawn(move || {
            for answer in answers {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };

                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let _ = asked.send(String::from_utf8_lossy(&request[..read]).into_owned());

                let _ = stream.write_all(answer.as_bytes());
                let _ = stream.flush();
            }
        });

        (format!("http://127.0.0.1:{port}/"), heard)
    }

    /// The address of a place where nothing listens: a port the operating system handed out and
    /// took back, so a connection to it is refused at once rather than waited out.
    pub(crate) fn nobody_there() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().expect("an address").port();
        drop(listener);

        format!("http://127.0.0.1:{port}/")
    }
}

#[cfg(test)]
mod tests {
    use super::testing::host;
    use super::*;

    fn client() -> HttpClient {
        let road = Arc::new(net::TransportPolicy::new().expect("a policy"));

        HttpClient::reaching_anywhere(road)
    }

    #[tokio::test]
    async fn what_the_host_sent_is_what_comes_back() {
        let (url, _asked) = host(
            "HTTP/1.1 200 OK\r\nContent-Type: image/PNG; charset=binary\r\nContent-Length: 5\r\n\r\nhello",
        );

        let fetched = client()
            .get(&url, "*/*", 1024)
            .await
            .expect("the host answered");

        assert_eq!(fetched.bytes, b"hello");
        // Lowercased and without its parameters: what it is used for is the caller's decision.
        assert_eq!(fetched.content_type, "image/png");
    }

    #[tokio::test]
    async fn the_request_says_nothing_about_this_machine() {
        // A user agent carrying the operating system and its version tells every host a message
        // links to what this machine is running, and one carrying this build's version tells it
        // which faults the copy still has.
        let (url, asked) = host("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi");

        let _ = client().get(&url, "text/plain", 1024).await;
        let request = asked.recv().expect("the host was asked");

        assert_eq!(
            request
                .lines()
                .find(|line| line.to_ascii_lowercase().starts_with("user-agent:")),
            Some("User-Agent: LyonoylMail"),
            "the request did not carry the name alone:\n{request}"
        );
        for telling in [
            "Mac OS", "Darwin", "Windows", "Linux", "x86", "arm", "Referer", "Cookie",
        ] {
            assert!(
                !request.contains(telling),
                "the request carries {telling}:\n{request}"
            );
        }

        // What the caller asked for is what goes out. One value for every request, whoever made
        // it, is a request for a document that accepts only pictures — and a host entitled to take
        // that seriously answers it with a refusal, a service failing for a reason nothing in it
        // names.
        assert!(
            request.contains("Accept: text/plain\r\n"),
            "the request did not carry what the caller accepts:\n{request}"
        );
    }

    #[tokio::test]
    async fn an_answer_the_host_says_is_too_large_is_refused_before_it_is_read() {
        let (url, _asked) = host("HTTP/1.1 200 OK\r\nContent-Length: 4096\r\n\r\n");

        let outcome = client().get(&url, "*/*", 1024).await;

        assert_eq!(outcome.expect_err("refused"), HttpError::TooLarge);
    }

    #[tokio::test]
    async fn a_host_that_never_says_how_long_is_stopped_by_what_arrives() {
        // A host that states a length is held to it by the protocol itself. A host that states
        // none can send until the connection closes, and this is the limit that stops it: the one
        // on what has actually arrived.
        let body = "x".repeat(4096);
        let answer: &'static str =
            Box::leak(format!("HTTP/1.1 200 OK\r\n\r\n{body}").into_boxed_str());
        let (url, _asked) = host(answer);

        let outcome = client().get(&url, "*/*", 1024).await;

        assert_eq!(outcome.expect_err("refused"), HttpError::TooLarge);
    }

    #[tokio::test]
    async fn a_host_that_says_no_is_a_refusal_rather_than_an_empty_answer() {
        let (url, _asked) = host("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");

        let outcome = client().get(&url, "*/*", 1024).await;

        assert_eq!(outcome.expect_err("refused"), HttpError::Refused);
    }

    #[tokio::test]
    async fn the_client_this_application_builds_will_not_reach_this_machine() {
        // The finding this closes: a message pointing a picture at a port on the reader's own
        // machine, or at their router, had nothing standing in its way. One message could then map
        // the network it was opened on and report the map home by timing.
        let (url, asked) = host("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi");
        let refusing = {
            let road = Arc::new(net::TransportPolicy::new().expect("a policy"));

            HttpClient::new(road)
        };

        let outcome = refusing.get(&url, "*/*", 1024).await;

        assert_eq!(outcome.expect_err("refused"), HttpError::Address);
        assert!(
            asked.try_recv().is_err(),
            "the host was reached anyway: nothing may be sent before the address is judged"
        );
    }

    #[tokio::test]
    async fn every_address_that_is_not_the_public_internet_is_refused_by_the_shipped_client() {
        let refusing = {
            let road = Arc::new(net::TransportPolicy::new().expect("a policy"));

            HttpClient::new(road)
        };

        for address in [
            "http://127.0.0.1:9200/",
            "http://192.168.1.1/cgi-bin/luci",
            "http://10.0.0.5:8080/",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]:5000/",
            "http://0.0.0.0:11211/",
        ] {
            assert_eq!(
                refusing
                    .get(address, "*/*", 1024)
                    .await
                    .expect_err("refused"),
                HttpError::Address,
                "{address} was not refused"
            );
        }
    }

    #[test]
    fn an_address_written_as_a_number_is_judged_where_it_is_read() {
        assert!(host_is_reachable("https://example.com/a"));
        assert!(host_is_reachable("https://[2001:db8::1]/a"));

        assert!(!host_is_reachable("http://127.0.0.1:9200/"));
        assert!(!host_is_reachable("http://192.168.1.1/"));
        assert!(!host_is_reachable("http://[::1]/"));
        assert!(!host_is_reachable("http://[::ffff:127.0.0.1]/"));
        assert!(!host_is_reachable("not a url"));
    }

    #[tokio::test]
    async fn an_address_this_build_does_not_fetch_is_refused_without_asking_anyone() {
        for address in [
            "file:///etc/passwd",
            "ftp://example.com/thing",
            "javascript:alert(1)",
            "data:image/png;base64,AAAA",
            "http:example.com",
            "not an address",
        ] {
            let outcome = client().get(address, "*/*", 1024).await;

            assert_eq!(
                outcome.expect_err("refused"),
                HttpError::Address,
                "{address} was not refused"
            );
        }
    }

    /// A download says how much has arrived, and how much is coming where the host said.
    #[tokio::test]
    async fn how_much_has_arrived_is_told_as_it_arrives() {
        let body = "x".repeat(300);
        let answer: &'static str = Box::leak(
            format!("HTTP/1.1 200 OK\r\nContent-Length: 300\r\n\r\n{body}").into_boxed_str(),
        );
        let (url, _asked) = host(answer);

        let mut said: Vec<(usize, Option<usize>)> = Vec::new();
        let got = client()
            .get_watching(&url, "*/*", 4096, &mut |so_far, total| {
                said.push((so_far, total));
                true
            })
            .await
            .expect("the body arrives");

        assert_eq!(got.bytes.len(), 300);
        assert!(
            !said.is_empty(),
            "nothing was said about the download at all"
        );
        assert_eq!(
            said.last().expect("a last word"),
            &(300, Some(300)),
            "the last word was not that all of it had arrived"
        );
        assert!(
            said.iter().all(|(_, total)| *total == Some(300)),
            "the size the host stated was not carried through"
        );
    }

    /// Saying stop stops it, which is what a person pressing the button looks like from here.
    #[tokio::test]
    async fn a_download_somebody_stops_does_not_come_back_with_the_file() {
        let body = "x".repeat(300);
        let answer: &'static str = Box::leak(
            format!("HTTP/1.1 200 OK\r\nContent-Length: 300\r\n\r\n{body}").into_boxed_str(),
        );
        let (url, _asked) = host(answer);

        let outcome = client()
            .get_watching(&url, "*/*", 4096, &mut |_, _| false)
            .await;

        assert_eq!(outcome.expect_err("stopped"), HttpError::Offline);
    }

    #[test]
    fn which_addresses_are_fetched_at_all() {
        assert!(fetchable("http://example.com/a"));
        assert!(fetchable("HTTPS://example.com"));

        assert!(!fetchable("file:///etc/passwd"));
        assert!(!fetchable("ftp://example.com"));
        assert!(!fetchable("cid:part@example.com"));
        assert!(!fetchable("example.com"));
    }

    /// The address policy at each of the three places it applies.
    ///
    /// The check on the first address was tested; the two that a redirect and a name go through
    /// were not, and they are the two the policy exists for — the first address of a picture is
    /// written where a reader can see it, and the address it becomes is not.
    mod the_policy_holds_after_the_first_address {
        use super::*;

        /// The client the application ships, built here so a test can hold it.
        fn shipped() -> HttpClient {
            let road = Arc::new(net::TransportPolicy::new().expect("a policy"));

            HttpClient::new(road)
        }

        /// Reaches this machine on the first address, and judges every one after it.
        fn judging_the_hops() -> HttpClient {
            let road = Arc::new(net::TransportPolicy::new().expect("a policy"));

            HttpClient::judging_only_the_hops(road)
        }

        /// One answer redirecting to another address, as a host may send.
        fn sending_on_to(url: &str) -> &'static str {
            Box::leak(
                format!("HTTP/1.1 302 Found\r\nLocation: {url}\r\nContent-Length: 0\r\n\r\n")
                    .into_boxed_str(),
            )
        }

        #[tokio::test]
        async fn a_name_for_this_machine_gets_past_the_first_check_and_is_stopped_before_the_lookup(
        ) {
            // The first check reads the address as written, and `localhost` is written as a name —
            // so it passes, and what stops it is the check on the name. Without that, a picture
            // addressed by name reaches the reader's own machine while the address check says
            // nothing.
            //
            // Refused as an address rather than as a host that could not be reached, because that
            // is what it is: nothing was dialled, nothing was looked up, and no name server heard
            // the question.
            let (url, asked) = host("HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nsecret");
            let by_name = url.replace("127.0.0.1", "localhost");

            let outcome = shipped().get(&by_name, "*/*", 1024).await;

            assert_eq!(outcome.expect_err("refused"), HttpError::Address);
            assert!(
                asked.try_recv().is_err(),
                "the host on this machine was asked for something"
            );
        }

        #[tokio::test]
        async fn a_redirect_to_this_machine_is_refused_and_nothing_is_sent_there() {
            // The address a message names is one a reader could look at. The address it is sent on
            // to is chosen after that, by whoever answered — which is why every hop is judged and
            // not only the first.
            let (secret, secret_asked) = host("HTTP/1.1 200 OK\r\nContent-Length: 6\r\n\r\nsecret");
            let (first, _asked) = host(sending_on_to(&secret));

            let outcome = judging_the_hops().get(&first, "*/*", 1024).await;

            assert_eq!(outcome.expect_err("refused"), HttpError::Offline);
            assert!(
                secret_asked.try_recv().is_err(),
                "the request followed the redirect onto this machine"
            );
        }

        #[tokio::test]
        async fn a_chain_is_followed_to_the_third_address_and_no_further() {
            // Two chains, because a limit is two claims: the ordinary chain still arrives, and the
            // one past the limit never reaches its end. Only the pair catches an off-by-one that
            // merely widened the limit.
            let (third, third_asked) = host("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi");
            let (second, _) = host(sending_on_to(&third));
            let (first, _) = host(sending_on_to(&second));

            let arrived = client()
                .get(&first, "*/*", 1024)
                .await
                .expect("three is allowed");

            assert_eq!(arrived.bytes, b"hi");
            assert!(third_asked.try_recv().is_ok(), "the end was never asked");

            let (fourth, fourth_asked) = host("HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nhi");
            let (third, _) = host(sending_on_to(&fourth));
            let (second, _) = host(sending_on_to(&third));
            let (first, _) = host(sending_on_to(&second));

            let outcome = client().get(&first, "*/*", 1024).await;

            assert_eq!(outcome.expect_err("refused"), HttpError::Offline);
            assert!(
                fourth_asked.try_recv().is_err(),
                "the chain was followed one address too far"
            );
        }

        /// The policy where it is actually applied: at the connection, after the name answered.
        ///
        /// Asked of the transport rather than of a resolver handed to somebody else's client,
        /// because that is where it now lives — a name is free to answer with whatever its owner
        /// likes, and the only place that knows every address a name gave is the place about to
        /// connect to one of them. Every one of these is written as a number, so no name server is
        /// asked and no packet leaves this machine.
        #[tokio::test]
        async fn an_address_a_message_may_not_reach_is_refused_at_the_connection() {
            let road = net::TransportPolicy::new().expect("a policy");

            for written in [
                "127.0.0.1",
                "192.168.1.1",
                "10.0.0.5",
                "169.254.169.254",
                "::1",
                "fd00::1",
            ] {
                let refused = road
                    .connect_to_a_host_a_message_named(&net::Endpoint {
                        host: written.to_owned(),
                        port: 80,
                        encryption: net::Encryption::None,
                    })
                    .await;

                assert!(
                    matches!(refused, Err(net::TransportError::Unreachable)),
                    "{written} was not refused"
                );
            }
        }

        #[test]
        fn a_redirect_never_leaves_encryption() {
            // A configuration document asked for encrypted and sent on to a plain address is one
            // whoever is on the path may answer, and its answer decides where a password goes.
            let address = |written: &str| url::Url::parse(written).expect("an address");

            assert!(
                !may_follow(
                    &address("https://example.com/a"),
                    &address("http://example.com/a")
                ),
                "a hop out of encryption was followed"
            );
            assert!(may_follow(
                &address("http://example.com/a"),
                &address("http://example.net/b")
            ));
            assert!(may_follow(
                &address("http://example.com/a"),
                &address("https://example.com/a")
            ));
            assert!(may_follow(
                &address("https://example.com/a"),
                &address("https://example.org/b")
            ));

            // Asked of the hop a redirect actually makes, whether it names a whole address or only
            // a path, which stays encrypted because the address it is resolved against is.
            assert_eq!(
                the_next_hop(
                    &address("https://example.com/a"),
                    "http://example.com/b",
                    true
                ),
                Err(HttpError::Offline)
            );
            assert_eq!(
                the_next_hop(&address("https://example.com/a"), "/b", true),
                Ok(address("https://example.com/b"))
            );
        }

        #[test]
        fn a_name_that_means_here_is_known_by_its_name() {
            for here in [
                "localhost",
                "LOCALHOST",
                "printer.local",
                "Printer.Local",
                "pictures.localhost",
                // Written absolutely, which is the form the parser keeps and a person never types.
                "localhost.",
                "printer.local.",
            ] {
                assert!(a_name_that_means_here(here), "{here} was not recognised");
            }

            // The suffixes carry their dot, so an ordinary name that merely starts or ends with
            // the same letters stays ordinary.
            for elsewhere in [
                "example.com",
                "localhostings.example",
                "notlocalhost.example",
                "local.example.com",
            ] {
                assert!(
                    !a_name_that_means_here(elsewhere),
                    "{elsewhere} was refused"
                );
            }
        }
    }

    /// An answer held to its limits, however the host chooses to send it.
    ///
    /// A host a message names writes every byte of its answer, and an update comes down the same
    /// road. The first five are a host trying it on: wrapping a few bytes in a great deal,
    /// announcing a piece no sum can hold, never ending a line, stopping short of what it promised
    /// whether it stated a length or sent pieces. The last two are the ordinary answer, which has
    /// to keep arriving whole.
    mod an_answer_is_held_to_its_limits {
        use super::*;

        /// An answer sent in pieces, made of whatever `pieces` says.
        fn in_pieces(pieces: &str) -> &'static str {
            Box::leak(
                format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n{pieces}")
                    .into_boxed_str(),
            )
        }

        #[tokio::test]
        async fn a_chunked_answer_that_outgrows_the_limit_in_its_wrapping_is_refused() {
            // A hundred bytes of body, each byte a piece of its own, each piece announced on a line
            // padded to half a kilobyte with an extension nobody reads. The pieces stay far under
            // the limit and the answer is fifty kilobytes all the same. Unless the lines count
            // as well as the pieces, a host can wrap a few bytes in as much as it likes.
            let padding = "a".repeat(500);
            let pieces = format!("1;e={padding}\r\nx\r\n").repeat(100) + "0\r\n\r\n";
            let (url, _asked) = host(in_pieces(&pieces));

            let outcome = client().get(&url, "*/*", 1024).await;

            assert_eq!(outcome.expect_err("refused"), HttpError::TooLarge);
        }

        #[tokio::test]
        async fn a_chunk_length_no_sum_can_hold_is_refused_without_a_panic() {
            // One real piece, then a length at the very top of what a machine word holds. Added to
            // what is already held it does not fit: a debug build panics on the sum, and a release
            // build wraps it to something small and panics on the slice taken with it instead.
            let (url, _asked) = host(in_pieces(&format!("1\r\nx\r\n{:x}\r\nxx", usize::MAX)));

            let outcome = client().get(&url, "*/*", 1024).await;

            assert_eq!(outcome.expect_err("refused"), HttpError::TooLarge);
        }

        #[tokio::test]
        async fn a_chunk_length_line_that_never_ends_is_refused_for_its_length() {
            // The line announcing the first piece never ends: the host sends sixty-four kilobytes
            // of it and gives up. A line has a ceiling of its own, far under the megabyte the
            // caller allows, and that ceiling refuses this at the first kilobyte — for its length.
            // Without one, all of it is read and held until the host stops, and the answer is
            // reported as a host that went away rather than as one that sent too much.
            let endless = format!("1;{}", "a".repeat(64 * 1024));
            let (url, _asked) = host(in_pieces(&endless));

            let outcome = client().get(&url, "*/*", 1024 * 1024).await;

            assert_eq!(outcome.expect_err("refused"), HttpError::TooLarge);
        }

        #[tokio::test]
        async fn an_answer_that_ends_before_its_stated_length_is_not_handed_on_as_whole() {
            // Ten bytes promised, five sent, and the connection closes. Five bytes of a picture is
            // a broken picture and five of an update is an archive cut short: either way, a host
            // that did not finish answering.
            let (url, _asked) = host("HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nhello");

            let outcome = client().get(&url, "*/*", 1024).await;

            assert_eq!(outcome.expect_err("cut short"), HttpError::Offline);
        }

        #[tokio::test]
        async fn a_chunked_answer_that_ends_before_its_last_piece_is_not_handed_on_as_whole() {
            // One whole piece, and then the connection closes. Every piece that arrived is
            // complete, so nothing about them says anything is missing: only the piece of no
            // length says the body is whole, and it never came.
            let (url, _asked) = host(in_pieces("5\r\nhello\r\n"));

            let outcome = client().get(&url, "*/*", 1024).await;

            assert_eq!(outcome.expect_err("cut short"), HttpError::Offline);
        }

        #[tokio::test]
        async fn a_chunked_answer_with_extensions_and_trailers_arrives_whole() {
            // Everything the protocol allows around the pieces, in an answer exactly as large as
            // the caller allows: an extension on every line, one of them with a semicolon inside
            // its quotes, and trailers after the last piece. None of it is body, and none of it
            // may cost the body a byte.
            let (url, _asked) = host(in_pieces(
                "5;name=value\r\nhello\r\n7;quoted=\"a;b\"\r\n, world\r\n0;last\r\n\
                 Checksum: 0\r\nNote: after\r\n\r\n",
            ));

            let fetched = client()
                .get(&url, "*/*", 12)
                .await
                .expect("the answer arrives");

            assert_eq!(fetched.bytes, b"hello, world");
        }

        #[tokio::test]
        async fn an_answer_of_its_stated_length_arrives_whole() {
            // Exactly as long as it says, exactly as long as the caller allows, and longer than any
            // one read, so the body is put together across several of them and ends on the limit.
            let body = "x".repeat(20_000);
            let answer: &'static str = Box::leak(
                format!("HTTP/1.1 200 OK\r\nContent-Length: 20000\r\n\r\n{body}").into_boxed_str(),
            );
            let (url, _asked) = host(answer);

            let fetched = client()
                .get(&url, "*/*", 20_000)
                .await
                .expect("the answer arrives");

            assert_eq!(fetched.bytes, body.as_bytes());
        }
    }
}
