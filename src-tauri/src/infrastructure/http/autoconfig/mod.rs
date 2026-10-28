//! Asking a domain where it keeps its mail, over the client beside it.
//!
//! One service over that client. What it adds is the part that is about mail rather than about
//! HTTP: which two addresses a domain publishes at, what the document looks like, and which of the
//! servers in it this build can use.
//!
//! Two addresses and no more. Both belong to the domain in the address a person typed, so what
//! either of them learns is that somebody is setting up mail there — and the machine that learns
//! it already holds that mail. Nothing here asks anybody else.
//!
//! The client refuses an address that is not on the public internet, at every hop and after every
//! redirect, because a name resolves wherever its owner likes. That check is what makes it safe to
//! build a web address out of something a person typed.

use std::sync::Arc;

use tokio::runtime::Handle;

use crate::infrastructure::http::{HttpClient, HttpError};
use crate::ports::provider_directory::{DirectoryError, ProviderDirectory, Published};
use lyonoyl_domain::account::Encryption;
use lyonoyl_domain::provider::{ServerSettings, Submission};

/// The largest document this build will read for one domain.
///
/// The answer is a handful of hostnames. Anything past this is somebody's idea of a joke or
/// somebody's website, and reading either costs the same memory.
const LARGEST: usize = 64 * 1024;

pub struct PublishedByTheDomain {
    client: Arc<HttpClient>,
    /// The one runtime, borrowed rather than made — the same reason as everywhere else here.
    runtime: Handle,
}

impl PublishedByTheDomain {
    pub fn new(client: Arc<HttpClient>, runtime: Handle) -> Self {
        Self { client, runtime }
    }

    /// The two addresses a domain may publish at, in the order they are asked.
    ///
    /// The subdomain first because it is what almost every provider that publishes anything uses,
    /// and the well-known path second because it is where the standard says to look. Asking the
    /// second first would mean a request to the domain's own website for nearly everybody.
    fn addresses(domain: &str) -> [String; 2] {
        [
            format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress="),
            format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
        ]
    }
}

impl ProviderDirectory for PublishedByTheDomain {
    fn published_by(&self, domain: &str) -> Result<Published, DirectoryError> {
        let client = Arc::clone(&self.client);
        let addresses = Self::addresses(domain);

        self.runtime.block_on(async move {
            let mut reached_nobody = true;

            for address in addresses {
                match client
                    .get(&address, "application/xml, text/xml", LARGEST)
                    .await
                {
                    Ok(fetched) => {
                        reached_nobody = false;
                        if let Some(settings) = settings_in(&fetched.bytes, domain) {
                            return Ok(Published::Settings(Box::new(settings)));
                        }
                    }
                    // Answered, and the answer was no. That is a domain saying it publishes
                    // nothing here, which is different from one nobody could reach.
                    Err(HttpError::Refused | HttpError::TooLarge) => reached_nobody = false,
                    Err(HttpError::Offline | HttpError::Address) => {}
                }
            }

            match reached_nobody {
                true => Err(DirectoryError::Unreachable),
                false => Ok(Published::Nothing),
            }
        })
    }
}

/// The servers in one of these documents, if there are any this build can use.
///
/// A server with no encryption is not offered: this build refuses an unencrypted connection to
/// anywhere but this machine, so offering one would be an answer that cannot connect.
fn settings_in(document: &[u8], domain: &str) -> Option<ServerSettings> {
    let text = std::str::from_utf8(document).ok()?;

    // A document that is about somebody else's domain is not an answer about this one. A host that
    // answers at `autoconfig.<domain>` is not necessarily under the control of whoever owns the
    // mail, so what it says about other domains is none of its business here.
    if !mentions(text, domain) {
        return None;
    }

    let imap = server_in(text, "incomingServer", "imap", domain)?;
    let smtp = server_in(text, "outgoingServer", "smtp", domain);

    Some(ServerSettings {
        imap_host: imap.0,
        imap_port: imap.1,
        imap_encryption: imap.2,
        smtp: smtp.map(|(host, port, encryption)| Submission {
            host,
            port,
            encryption,
        }),
    })
}

/// Whether the document says it is about this domain.
fn mentions(text: &str, domain: &str) -> bool {
    let wanted = format!("<domain>{domain}</domain>");
    text.to_ascii_lowercase()
        .contains(&wanted.to_ascii_lowercase())
}

/// One server block, as host, port and encryption.
fn server_in(
    text: &str,
    element: &str,
    kind: &str,
    domain: &str,
) -> Option<(String, u16, Encryption)> {
    let opens = format!("<{element} type=\"{kind}\">");
    let closes = format!("</{element}>");
    let from = text.find(&opens)? + opens.len();
    let to = from + text[from..].find(&closes)?;
    let block = &text[from..to];

    let host = between(block, "hostname")?.replace("%EMAILDOMAIN%", domain);
    // Anything else the document substitutes is a placeholder this build does not understand, and
    // a host name with a percent sign in it is not one.
    if host.contains('%') || !host.contains('.') {
        return None;
    }

    let port: u16 = between(block, "port")?.parse().ok()?;
    if port == 0 {
        return None;
    }

    let encryption = match between(block, "socketType")?.as_str() {
        "SSL" => Encryption::Tls,
        "STARTTLS" => Encryption::StartTls,
        // `plain`, or something this build has never heard of. Neither is offered.
        _ => return None,
    };

    Some((host, port, encryption))
}

/// What one element holds, trimmed.
fn between(block: &str, element: &str) -> Option<String> {
    let opens = format!("<{element}>");
    let closes = format!("</{element}>");
    let from = block.find(&opens)? + opens.len();
    let to = from + block[from..].find(&closes)?;
    Some(block[from..to].trim().to_owned())
}

#[cfg(test)]
mod tests;
