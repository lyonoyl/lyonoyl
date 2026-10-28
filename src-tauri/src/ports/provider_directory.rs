//! What a mail domain publishes about its own servers.
//!
//! A port because the use case decides *whether* to ask and *in what order*, and knows nothing
//! about how. The order is the point: it is a decision about who finds out that this person uses
//! this domain, so it belongs where it can be read, not inside an adapter.
//!
//! What is on the other side is the domain's own web server, over the application's HTTP client.
//! What is on the other side in a test is a list of answers.

use lyonoyl_domain::provider::ServerSettings;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DirectoryError {
    /// Nothing was reachable, or nothing answered in time. A different sentence from "asked, and
    /// this domain publishes nothing": the first may work in a minute and the second will not.
    #[error("nobody answered for that domain")]
    Unreachable,
}

/// What a domain said about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Published {
    /// It publishes these, and this is where they are.
    Settings(Box<ServerSettings>),
    /// It was asked and publishes nothing. Asking again answers the same, so the cascade moves on
    /// rather than waiting.
    Nothing,
}

pub trait ProviderDirectory: Send + Sync {
    /// Asks one domain what it publishes, and nobody else.
    ///
    /// The domain arrives already checked as one — the shapes that are not a domain are refused
    /// before this, because what happens here is a web address built out of it.
    fn published_by(&self, domain: &str) -> Result<Published, DirectoryError>;
}

impl<D: ProviderDirectory + ?Sized> ProviderDirectory for &D {
    fn published_by(&self, domain: &str) -> Result<Published, DirectoryError> {
        (**self).published_by(domain)
    }
}
