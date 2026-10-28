//! Where the project's releases are to be had: the one interface the updater fetches through, and
//! the list of places a copy asks.
//!
//! A provider hands over what it is asked for by what it is — the manifest of the shape this build
//! reads, for the system it runs on, with the seal beside it, and a file of a release by its plain
//! name — and never by an address a manifest names. Everything it hands over is held to the seal and
//! the digest before anything is done with it, so a provider is trusted with nothing but being
//! there, and nothing above this module knows which places there are or how they are reached.
//!
//! A place is added by a provider in a file beside the site's and a line in
//! [`the_projects_providers`], and taken away by removing both. Nothing else changes.

use std::sync::Arc;

use tokio::runtime::Handle;

use lyonoyl_domain::version::Version;

use crate::infrastructure::http::HttpClient;
use crate::infrastructure::net::TransportPolicy;
use crate::ports::updates::{UpdateError, Watching};

pub mod the_site;

/// A manifest as a provider hands it over, with the seal that came beside it, neither of them read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sealed {
    pub manifest: String,
    pub seal: String,
}

/// A place the project publishes its releases to.
pub trait UpdateProvider: Send + Sync {
    /// What a release check calls this place in its report. Never written to the journal.
    ///
    /// Read by the release check alone, which runs as a test of this crate: nothing in a running
    /// copy needs a place's name, and only a build without the tests finds it unread.
    #[cfg_attr(not(test), allow(dead_code))]
    fn name(&self) -> &str;

    /// The manifest of the shape this build reads, for the system it runs on, and the seal over it,
    /// each whole. Anything short of both is [`UpdateError::CouldNotAsk`].
    fn the_manifest(&self) -> Result<Sealed, UpdateError>;

    /// One file of the release of `version`, by its plain name, whole and no larger than `at_most`,
    /// told how much has arrived as it arrives.
    ///
    /// Answering `false` to `watching` stops it, and the answer is then [`UpdateError::Stopped`]; a
    /// file that is not there or does not arrive whole is [`UpdateError::CouldNotAsk`].
    fn a_file(
        &self,
        version: Version,
        name: &str,
        at_most: usize,
        watching: Watching<'_>,
    ) -> Result<Vec<u8>, UpdateError>;
}

/// The places the project publishes to, in the order a copy asks them.
///
/// The one list there is: every copy asks these, and the release check asks the same ones the same
/// way before a manifest goes out.
pub fn the_projects_providers(
    policy: Arc<TransportPolicy>,
    runtime: Handle,
) -> Vec<Box<dyn UpdateProvider>> {
    vec![Box::new(the_site::SiteProvider::new(
        HttpClient::new(policy),
        runtime,
    ))]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The places every copy asks, in order: the project's site, and nothing else.
    #[test]
    fn a_copy_asks_the_projects_site_alone() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("a runtime");
        let policy = Arc::new(TransportPolicy::new().expect("a policy"));

        let names: Vec<String> = the_projects_providers(policy, runtime.handle().clone())
            .iter()
            .map(|provider| provider.name().to_owned())
            .collect();

        assert_eq!(names, ["lyonoyl.com"]);
    }
}
