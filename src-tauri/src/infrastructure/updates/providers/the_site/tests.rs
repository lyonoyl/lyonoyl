use std::sync::mpsc::Receiver;
use std::sync::Arc;

use super::*;
use crate::infrastructure::http::testing::{host_answering, nobody_there};
use crate::infrastructure::updates::the_seal::made_elsewhere::{SEALED, SEALED_FILE};

/// A runtime of its own, as the window hands one to the service it builds.
fn a_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("a runtime")
}

/// The provider asking a host on this machine instead of the site.
fn a_site_at(root: &str, runtime: &tokio::runtime::Runtime) -> SiteProvider {
    let road = Arc::new(crate::infrastructure::net::TransportPolicy::new().expect("a policy"));

    SiteProvider::at(
        root,
        HttpClient::reaching_anywhere(road),
        runtime.handle().clone(),
    )
}

/// The provider as every copy builds it, asking nothing in these tests.
fn the_site(runtime: &tokio::runtime::Runtime) -> SiteProvider {
    let road = Arc::new(crate::infrastructure::net::TransportPolicy::new().expect("a policy"));

    SiteProvider::new(HttpClient::new(road), runtime.handle().clone())
}

/// An answer carrying `body` whole.
fn answering(body: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

/// An answer saying there is nothing at that address.
fn not_found() -> String {
    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_owned()
}

/// An answer sending the request on to `url`.
fn sending_on_to(url: &str) -> String {
    format!("HTTP/1.1 302 Found\r\nLocation: {url}\r\nContent-Length: 0\r\n\r\n")
}

/// A site holding a manifest and the seal over it.
fn a_site_with_a_manifest() -> (String, Receiver<String>) {
    let manifest = std::str::from_utf8(SEALED_FILE).expect("text");

    host_answering(vec![answering(manifest), answering(SEALED)])
}

/// The line a request says who is asking on.
fn the_agent_in(request: &str) -> Option<&str> {
    request
        .lines()
        .find(|line| line.to_ascii_lowercase().starts_with("user-agent:"))
}

/// What a request asked for, as its first line names it.
fn the_path_in(request: &str) -> &str {
    request
        .lines()
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .unwrap_or("")
}

fn version() -> Version {
    Version::parse("1.1.0").expect("a version")
}

/// The site every copy asks, and where on it — written out here rather than built the way the code
/// builds it.
#[test]
fn the_site_is_the_products_own() {
    let runtime = a_runtime();
    let site = the_site(&runtime);

    assert_eq!(SITE, "https://lyonoyl.com/");
    assert_eq!(site.name(), "lyonoyl.com");
    assert_eq!(
        site.the_folder_of(Version::parse("1.2.3").expect("a version")),
        "https://lyonoyl.com/releases/1.2.3/"
    );
    #[cfg(target_os = "macos")]
    assert_eq!(
        site.the_manifest_at(),
        "https://lyonoyl.com/updates/v1/macos/latest.json"
    );
}

/// The manifest of the shape this build reads and the system it runs on, then the seal beside it,
/// each asked for naming the product alone.
#[test]
fn the_manifest_and_its_seal_are_asked_for_naming_the_product_alone() {
    let runtime = a_runtime();
    let (root, asked) = a_site_with_a_manifest();

    let sealed = a_site_at(&root, &runtime)
        .the_manifest()
        .expect("the site answered");

    assert_eq!(sealed.manifest.as_bytes(), SEALED_FILE);
    assert_eq!(sealed.seal, SEALED);

    let requests: Vec<String> = asked.try_iter().collect();
    let shape_and_system = format!("/updates/v1/{}/latest.json", std::env::consts::OS);
    assert_eq!(
        requests
            .iter()
            .map(|request| the_path_in(request))
            .collect::<Vec<_>>(),
        [
            shape_and_system.clone(),
            format!("{shape_and_system}.minisig")
        ],
        "not the manifest and then the seal beside it"
    );
    for request in &requests {
        assert_eq!(
            the_agent_in(request),
            Some("User-Agent: LyonoylMail"),
            "a look said more than the product's name:\n{request}"
        );
    }
}

/// The site may send a copy on to wherever it keeps the manifest and the seal.
#[test]
fn a_site_that_sends_the_copy_on_is_followed_to_the_manifest() {
    let runtime = a_runtime();
    let (kept, kept_asked) = a_site_with_a_manifest();
    let (root, _) = host_answering(vec![
        sending_on_to(&format!("{kept}kept/latest.json")),
        sending_on_to(&format!("{kept}kept/latest.json.minisig")),
    ]);

    let sealed = a_site_at(&root, &runtime)
        .the_manifest()
        .expect("the place the site sent the copy on to answered");

    assert_eq!(sealed.manifest.as_bytes(), SEALED_FILE);
    assert_eq!(
        kept_asked
            .try_iter()
            .map(|request| the_path_in(&request).to_owned())
            .collect::<Vec<_>>(),
        ["/kept/latest.json", "/kept/latest.json.minisig"]
    );
}

#[test]
fn a_site_that_does_not_answer_hands_over_no_manifest() {
    let runtime = a_runtime();

    assert_eq!(
        a_site_at(&nobody_there(), &runtime).the_manifest(),
        Err(UpdateError::CouldNotAsk)
    );
}

#[test]
fn a_manifest_without_its_seal_is_not_handed_over() {
    let runtime = a_runtime();
    let manifest = std::str::from_utf8(SEALED_FILE).expect("text");
    let (root, _) = host_answering(vec![answering(manifest), not_found()]);

    assert_eq!(
        a_site_at(&root, &runtime).the_manifest(),
        Err(UpdateError::CouldNotAsk)
    );
}

/// A manifest carries what changed in every language a catalogue exists for, every letter past
/// ASCII written as an escape, and every copy holds the manifests it ever reads to the ceiling it
/// shipped with: one of a few hundred kilobytes is read whole, and one past the ceiling is not.
#[test]
fn a_manifest_with_notes_in_every_language_is_read_and_one_past_the_ceiling_is_not() {
    let runtime = a_runtime();
    let long = "x".repeat(200 * 1024);
    let (root, _) = host_answering(vec![answering(&long), answering(SEALED)]);

    let sealed = a_site_at(&root, &runtime)
        .the_manifest()
        .expect("a manifest of two hundred kilobytes is read");
    assert_eq!(sealed.manifest.len(), long.len());

    let past = "x".repeat(LONGEST + 1);
    let (root, _) = host_answering(vec![answering(&past), answering(SEALED)]);
    assert_eq!(
        a_site_at(&root, &runtime).the_manifest(),
        Err(UpdateError::CouldNotAsk),
        "a manifest past the ceiling was read"
    );
}

/// A file of a release is asked for in that release's folder, naming the product alone.
#[test]
fn a_file_is_asked_for_in_its_release_naming_the_product_alone() {
    let runtime = a_runtime();
    let (root, asked) = host_answering(vec![answering("abc")]);

    let file = a_site_at(&root, &runtime)
        .a_file(version(), "a.app.tar.gz", 3, &mut |_, _| true)
        .expect("the site handed it over");
    assert_eq!(file, b"abc");

    let request = asked.recv().expect("the file was asked for");
    assert_eq!(the_path_in(&request), "/releases/1.1.0/a.app.tar.gz");
    assert_eq!(
        the_agent_in(&request),
        Some("User-Agent: LyonoylMail"),
        "a download said more than the name:\n{request}"
    );
}

/// Stop is the person's own doing, and it is said to be that rather than a failure.
#[test]
fn a_person_saying_stop_stops_a_file() {
    let runtime = a_runtime();
    let (root, _) = host_answering(vec![answering("abc")]);

    assert_eq!(
        a_site_at(&root, &runtime).a_file(version(), "a.app.tar.gz", 3, &mut |_, _| false),
        Err(UpdateError::Stopped)
    );
}

#[test]
fn a_file_that_is_not_handed_over_whole_is_a_question_that_could_not_be_asked() {
    let runtime = a_runtime();
    let (short, _) = host_answering(vec![
        "HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nab".to_owned()
    ]);
    let (refusing, _) = host_answering(vec![not_found()]);

    for root in [short, refusing, nobody_there()] {
        assert_eq!(
            a_site_at(&root, &runtime).a_file(version(), "a.app.tar.gz", 3, &mut |_, _| true),
            Err(UpdateError::CouldNotAsk),
            "{root}"
        );
    }
}

/// A chain of `addresses`, each sending the request on to the next and the last holding `body`:
/// the first address's root, and what the last one heard.
fn a_chain_of(addresses: usize, body: &str) -> (String, Receiver<String>) {
    let (end, end_asked) = host_answering(vec![answering(body)]);
    let mut next = format!("{end}releases/1.1.0/a.app.tar.gz");
    let mut root = end;
    for _ in 1..addresses {
        let (here, _) = host_answering(vec![sending_on_to(&next)]);
        next = format!("{here}releases/1.1.0/a.app.tar.gz");
        root = here;
    }

    (root, end_asked)
}

/// The site may send a download on as far as the chain allows, and a chain past that is never
/// followed to its end. Two chains, because a limit is two claims: the longest allowed still
/// arrives, and the one past it does not.
#[test]
fn a_file_is_followed_through_the_whole_chain_allowed_and_no_further() {
    let runtime = a_runtime();

    let (root, end_asked) = a_chain_of(CHAIN, "abc");
    assert_eq!(
        a_site_at(&root, &runtime).a_file(version(), "a.app.tar.gz", 3, &mut |_, _| true),
        Ok(b"abc".to_vec()),
        "a chain of {CHAIN} addresses did not arrive"
    );
    assert!(end_asked.try_recv().is_ok(), "the end was never asked");

    let (root, end_asked) = a_chain_of(CHAIN + 1, "abc");
    assert_eq!(
        a_site_at(&root, &runtime).a_file(version(), "a.app.tar.gz", 3, &mut |_, _| true),
        Err(UpdateError::CouldNotAsk)
    );
    assert!(
        end_asked.try_recv().is_err(),
        "the chain was followed one address too far"
    );
}
