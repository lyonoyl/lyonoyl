//! What this build already knows about where a mail domain keeps its servers.
//!
//! A person types an address and nothing about it says which machine holds the mail. There are
//! several places to ask, and they differ in who finds out that this person uses this domain — so
//! the order they are asked in is a decision about privacy rather than about speed, and it starts
//! here, with the one place that tells nobody anything: a table compiled into the application.
//!
//! The table is carried as a snapshot rather than asked for over
//! the network. Asking it live is what the other clients do, and it hands the person's mail domain
//! to somebody else's server on the first screen of adding an account — before any consent, in a
//! product whose thesis is that mail stays with the person. A snapshot costs a quarter of a
//! megabyte and tells nobody anything.
//!
//! What a snapshot costs instead is age: a provider that moves a host is wrong here until the
//! table is corrected. That is why this is the first step of a cascade and not the whole of it —
//! what the table does not know is asked of the domain itself, and typing the settings in by hand
//! is always there underneath.

use crate::account::Encryption;

/// Where one account's mail is read and where its messages are handed over.
///
/// Sending is separate and optional: reading and sending are different servers, and a domain whose
/// provider publishes only the first is a domain this can still answer about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerSettings {
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_encryption: Encryption,
    pub smtp: Option<Submission>,
}

/// Where messages written from an account are handed over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submission {
    pub host: String,
    pub port: u16,
    pub encryption: Encryption,
}

/// The domain of an address, lowercased, or nothing if this is not one address.
///
/// Taken here rather than by the caller because every step of the cascade needs the same answer,
/// and two spellings of it would be two different questions asked of the outside world. An address
/// with no `@`, with several, or with nothing on either side of it, is not one to ask about.
pub fn domain_of(address: &str) -> Option<String> {
    let address = address.trim();
    let (local, domain) = address.split_once('@')?;

    if local.is_empty() || domain.is_empty() || domain.contains('@') {
        return None;
    }

    // A domain is a name, and these are the shapes that are not one. Checked here because what
    // follows builds host names and web addresses out of it.
    if domain.starts_with('.')
        || domain.ends_with('.')
        || domain.contains("..")
        || !domain
            .chars()
            .all(|each| each.is_ascii_alphanumeric() || each == '.' || each == '-')
        || !domain.contains('.')
    {
        return None;
    }

    Some(domain.to_ascii_lowercase())
}

mod table;

/// A provider offered by name on the account screen.
///
/// Recognition, and nothing else. Somebody who reads "Gmail" knows this will work; somebody faced
/// with an empty host field does not. Pressing one fills in the server settings and stops — there
/// is no other flow, no other kind of account, and no code anywhere that branches on which of
/// these was pressed. The day one of them signs in differently, it is a new implementation of
/// `AccountProvider` and this list is still a list of names.
///
/// `hint` names a sentence in the interface catalogue, or nothing. It exists because two of these
/// refuse an ordinary password outright, and a person who is not told that reads "wrong password"
/// and concludes the application is broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    /// The domain its settings are looked up under. Not the person's address.
    pub domain: &'static str,
    pub label: &'static str,
    pub hint: Option<&'static str>,
}

/// The presets offered, in the order they are drawn.
///
/// Short on purpose. A list of forty is a list nobody reads, and every domain not here is answered
/// by the same cascade that answers the ones that are — including Fastmail, which publishes its own
/// settings and is found without anybody writing them down.
pub static PRESETS: &[Preset] = &[
    Preset {
        domain: "gmail.com",
        label: "Gmail",
        hint: Some("account.needsAppPassword"),
    },
    Preset {
        domain: "outlook.com",
        label: "Outlook",
        hint: None,
    },
    Preset {
        domain: "icloud.com",
        label: "iCloud",
        hint: Some("account.needsAppPassword"),
    },
    Preset {
        domain: "yahoo.com",
        label: "Yahoo",
        hint: Some("account.needsAppPassword"),
    },
];

/// What a person needs to know about this provider before they type a password, if anything.
///
/// A catalogue key rather than a sentence: the core does not write words for a person. Answered
/// from the address itself, because the window has the address and does not have the table — and a
/// window that held its own copy of which providers want an app password would be a second table to
/// keep level with this one.
pub fn hint_for(address: &str) -> Option<&'static str> {
    let domain = address.rsplit_once('@')?.1.to_ascii_lowercase();

    PRESETS
        .iter()
        .find(|preset| preset.domain == domain)
        .and_then(|preset| preset.hint)
}

/// What this build knows about a domain without asking anybody.
///
/// A binary search over a list sorted by domain, and the test below holds the table to that order.
/// Nothing here reaches the network, which is the whole reason this step comes first.
pub fn known_for(domain: &str) -> Option<ServerSettings> {
    let domain = domain.to_ascii_lowercase();

    let found = table::ROWS
        .binary_search_by(|row| row.0.cmp(domain.as_str()))
        .ok()
        .map(|at| table::ROWS[at])?;

    let (_, imap_host, imap_port, imap_encryption, smtp_host, smtp_port, smtp_encryption) = found;

    Some(ServerSettings {
        imap_host: imap_host.to_owned(),
        imap_port,
        imap_encryption,
        // A provider with no submission server known. The account is still worth adding:
        // reading and sending are separate servers, and one without the other is an account that
        // receives mail and says so rather than one that cannot be added.
        smtp: (!smtp_host.is_empty()).then(|| Submission {
            host: smtp_host.to_owned(),
            port: smtp_port,
            encryption: smtp_encryption,
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_domain_is_what_follows_the_one_at_sign() {
        assert_eq!(
            domain_of("zeta@example.com").as_deref(),
            Some("example.com")
        );
        assert_eq!(
            domain_of("  zeta@example.com  ").as_deref(),
            Some("example.com"),
            "an address pasted with spaces around it is the same address"
        );
        assert_eq!(
            domain_of("Zeta@Example.COM").as_deref(),
            Some("example.com"),
            "asked in one spelling, or it is two questions to the outside world"
        );
    }

    #[test]
    fn what_is_not_one_address_is_not_a_domain_to_ask_about() {
        // What follows builds host names and web addresses out of this, so a shape that is not a
        // domain must not reach that far.
        for not_one in [
            "",
            "zeta",
            "zeta@",
            "@example.com",
            "zeta@@example.com",
            "zeta@example@com",
            "zeta@example",
            "zeta@.example.com",
            "zeta@example.com.",
            "zeta@example..com",
            "zeta@exa mple.com",
            "zeta@example.com/../evil",
            "zeta@[127.0.0.1]",
            "zeta@ex%41mple.com",
        ] {
            assert_eq!(domain_of(not_one), None, "{not_one} is not one address");
        }
    }

    #[test]
    fn a_provider_this_build_knows_needs_nobody_asked() {
        let found = known_for("gmail.com").expect("a provider in the table");
        assert_eq!(found.imap_host, "imap.gmail.com");
        assert_eq!(found.imap_port, 993);
        assert_eq!(found.imap_encryption, Encryption::Tls);
        let sending = found.smtp.expect("somewhere to hand a message over");
        assert_eq!(sending.host, "smtp.gmail.com");
        assert_eq!(sending.port, 465);
    }

    #[test]
    fn the_other_names_a_provider_answers_to_reach_the_same_servers() {
        // A person on googlemail.com is on Gmail, and being told otherwise is being told to type
        // the settings in by hand for no reason.
        assert_eq!(known_for("googlemail.com"), known_for("gmail.com"));
        assert_eq!(known_for("hotmail.com"), known_for("outlook.com"));
    }

    #[test]
    fn the_table_is_read_whatever_case_the_domain_arrives_in() {
        assert_eq!(known_for("GMAIL.COM"), known_for("gmail.com"));
    }

    #[test]
    fn a_domain_nobody_here_knows_is_answered_with_nothing() {
        // Not a guess. What this build does not know is asked of the domain itself, and answering
        // "imap.<domain>" here would be that guess wearing the table's authority.
        assert_eq!(known_for("example.com"), None);
        assert_eq!(known_for("gmail.com.evil.test"), None);
        assert_eq!(known_for("notgmail.com"), None);
    }

    #[test]
    fn the_table_is_sorted_because_the_lookup_believes_it_is() {
        // A binary search over an unsorted list answers "not here" about mail somebody has.
        assert!(
            table::ROWS.windows(2).all(|pair| pair[0].0 < pair[1].0),
            "the table is out of order, or holds a domain twice"
        );
    }

    #[test]
    fn the_table_is_worth_carrying() {
        // Measured rather than asserted vaguely: the point of a snapshot over a network lookup is
        // that it covers enough to be the first step.
        assert!(
            table::ROWS.len() > 500,
            "{} domains is too few to be the table",
            table::ROWS.len()
        );
    }

    #[test]
    fn every_row_of_the_table_is_a_name_and_not_a_machine_on_this_computer() {
        // A row pointing at this machine would hand an account's password to whatever answers
        // there. The table is compiled from a source this project does not control, so what that
        // source may say is checked rather than trusted.
        for &(domain, imap, imap_port, _, smtp, smtp_port, _) in table::ROWS {
            assert!(domain.contains('.'), "{domain} is not a domain");
            assert_eq!(
                domain,
                domain.to_ascii_lowercase(),
                "{domain} is not lowered"
            );

            for (host, port) in [(imap, imap_port), (smtp, smtp_port)] {
                if host.is_empty() {
                    continue;
                }
                assert!(
                    host.parse::<std::net::IpAddr>().is_err(),
                    "{host} is an address rather than a name"
                );
                assert!(host.contains('.'), "{host} is not a host name");
                assert!(port > 0, "{host} is offered on no port");
            }
            assert!(!imap.is_empty(), "{domain} is carried with nowhere to read");
        }
    }
}

#[cfg(test)]
mod preset_tests {
    use super::*;

    #[test]
    fn every_preset_has_settings_to_fill_in() {
        // A button that fills in nothing is a button that looks broken. This is the only thing a
        // preset does, so a preset the table cannot answer has no reason to be offered.
        for preset in PRESETS {
            assert!(
                known_for(preset.domain).is_some(),
                "{} has no settings to offer",
                preset.label
            );
        }
    }

    #[test]
    fn a_preset_offers_a_reachable_server() {
        // The same rule the carried table is held to: this build refuses an unencrypted connection
        // to anywhere but this machine, so a preset offering one would fill in settings that
        // cannot connect — worse than no preset, which sends the person on to typing their own.
        for preset in PRESETS {
            let settings = known_for(preset.domain).expect("checked above");
            assert_ne!(
                settings.imap_encryption,
                crate::account::Encryption::None,
                "{}",
                preset.label
            );
        }
    }

    #[test]
    fn no_two_presets_are_the_same_provider() {
        // Two buttons filling in the same thing is a list somebody has to read twice to learn it
        // says one thing.
        for (at, preset) in PRESETS.iter().enumerate() {
            assert!(
                !PRESETS[..at]
                    .iter()
                    .any(|earlier| earlier.domain == preset.domain),
                "{} is offered twice",
                preset.label
            );
        }
    }
}
