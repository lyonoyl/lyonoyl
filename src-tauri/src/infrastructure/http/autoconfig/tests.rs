//! Reading what a domain publishes, and refusing what it should not be believed about.

use super::*;

fn document(domain: &str, socket: &str, host: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<clientConfig version="1.1">
  <emailProvider id="{domain}">
<domain>{domain}</domain>
<displayName>Example</displayName>
<incomingServer type="imap">
  <hostname>{host}</hostname>
  <port>993</port>
  <socketType>{socket}</socketType>
  <username>%EMAILADDRESS%</username>
</incomingServer>
<outgoingServer type="smtp">
  <hostname>smtp.{domain}</hostname>
  <port>465</port>
  <socketType>SSL</socketType>
</outgoingServer>
  </emailProvider>
</clientConfig>"#
    )
}

#[test]
fn a_domain_that_publishes_its_servers_is_read() {
    let found = settings_in(
        document("example.com", "SSL", "imap.example.com").as_bytes(),
        "example.com",
    )
    .expect("it published them");

    assert_eq!(found.imap_host, "imap.example.com");
    assert_eq!(found.imap_port, 993);
    assert_eq!(found.imap_encryption, Encryption::Tls);
    let sending = found.smtp.expect("it published one");
    assert_eq!(sending.host, "smtp.example.com");
    assert_eq!(sending.port, 465);
}

#[test]
fn a_document_about_somebody_elses_domain_answers_nothing() {
    // A host answering at `autoconfig.<domain>` is not necessarily the one holding the mail,
    // and what it says about other domains is not an answer about this one.
    assert_eq!(
        settings_in(
            document("elsewhere.test", "SSL", "imap.elsewhere.test").as_bytes(),
            "example.com"
        ),
        None
    );
}

#[test]
fn a_server_offered_without_encryption_is_not_offered_here() {
    // This build refuses an unencrypted connection to anywhere but this machine, so taking
    // one would be an answer that cannot connect — worse than no answer, which sends the
    // person on to typing the settings in.
    assert_eq!(
        settings_in(
            document("example.com", "plain", "imap.example.com").as_bytes(),
            "example.com"
        ),
        None
    );
}

#[test]
fn the_documents_own_placeholder_stands_for_the_domain() {
    let found = settings_in(
        document("example.com", "SSL", "mail.%EMAILDOMAIN%").as_bytes(),
        "example.com",
    )
    .expect("it published them");

    assert_eq!(found.imap_host, "mail.example.com");
}

#[test]
fn a_host_this_build_cannot_make_sense_of_answers_nothing() {
    for host in ["%SOMETHINGELSE%", "notahostname", ""] {
        assert_eq!(
            settings_in(
                document("example.com", "SSL", host).as_bytes(),
                "example.com"
            ),
            None,
            "{host}"
        );
    }
}

#[test]
fn a_document_that_is_not_one_answers_nothing() {
    for not_one in [
        &b"<html><body>Not found</body></html>"[..],
        &b""[..],
        &[0xff, 0xfe, 0x00][..],
    ] {
        assert_eq!(settings_in(not_one, "example.com"), None);
    }
}

#[test]
fn the_two_addresses_asked_belong_to_the_domain_and_to_nobody_else() {
    // The whole privacy claim of this step. A third address here would be a stranger learning
    // the domain of somebody who has consented to nothing.
    let asked = PublishedByTheDomain::addresses("example.com");

    assert_eq!(asked.len(), 2);
    for address in asked {
        let host = address
            .strip_prefix("https://")
            .expect("asked over https")
            .split('/')
            .next()
            .expect("a host");
        assert!(
            host == "example.com" || host == "autoconfig.example.com",
            "asked somebody else: {host}"
        );
    }
}
