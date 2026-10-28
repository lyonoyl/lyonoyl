//! The project's own site, `lyonoyl.com`, over HTTPS.
//!
//! It is asked for the manifest and its seal under `updates/`, and for a release's files under
//! `releases/<version>/`. The site answers with each file or sends a copy on to wherever it keeps
//! it, which is its own business, and a copy follows it for a chain of [`CHAIN`] addresses.
//!
//! One service over the client beside it, on the same road as pictures and server discovery: one
//! address policy, one set of deadlines, and an attempt towards every address a name answers with.
//! Every request names the product alone — not its version, which the question a person answers
//! before the first look does not mention.

use tokio::runtime::Handle;

use lyonoyl_domain::version::Version;

use super::{Sealed, UpdateProvider};
use crate::infrastructure::http::HttpClient;
use crate::infrastructure::updates::the_manifest::LONGEST;
use crate::ports::updates::{UpdateError, Watching};

/// The project's own site.
///
/// The product's own address, which names the product and nobody else. A literal rather than
/// something read from the environment: a build that had to be told where to look is a build whose
/// check is dead in every copy nobody configured.
pub(super) const SITE: &str = "https://lyonoyl.com/";

/// How many addresses a chain of redirects may ask, the first one included.
///
/// The site chooses where each file is served from, and may send a copy on rather than answer
/// itself — to a release host whose address for its latest release is three requests on its own,
/// measured, and four with the site in front. What a copy takes is decided by the seal and the
/// digest and never by where a file came from, so a longer chain costs a copy nothing but time, and
/// the limit is what ends a loop.
pub(super) const CHAIN: usize = 10;

pub struct SiteProvider {
    client: HttpClient,
    runtime: Handle,
    /// Where the paths start: [`SITE`], or a host on this machine standing in for it in a test.
    root: String,
}

impl SiteProvider {
    pub fn new(client: HttpClient, runtime: Handle) -> Self {
        Self::at(SITE, client, runtime)
    }

    /// The same provider asking `root` instead of the site, so a test can be the site.
    pub(super) fn at(root: &str, client: HttpClient, runtime: Handle) -> Self {
        Self {
            client: client.following_at_most(CHAIN),
            runtime,
            root: root.to_owned(),
        }
    }

    /// Where the manifest for this build is: the shape of manifest it reads, then the system it runs
    /// on. A manifest of another shape, or for another system, is a file beside this one rather than
    /// a change to it.
    fn the_manifest_at(&self) -> String {
        format!(
            "{}updates/v1/{}/latest.json",
            self.root,
            std::env::consts::OS
        )
    }

    /// Where a release keeps its files: under its version.
    fn the_folder_of(&self, version: Version) -> String {
        format!("{}releases/{version}/", self.root)
    }

    /// One small document, whole.
    fn small(&self, from: &str) -> Result<String, UpdateError> {
        let got = self
            .runtime
            .block_on(
                self.client
                    .get(from, "application/json, text/plain", LONGEST),
            )
            .map_err(|_| UpdateError::CouldNotAsk)?;

        String::from_utf8(got.bytes).map_err(|_| UpdateError::CouldNotAsk)
    }
}

/// Where the seal over a file is: beside it, under its whole name with `.minisig` added, as the
/// format names a seal.
fn the_seal_over(file: &str) -> String {
    format!("{file}.minisig")
}

impl UpdateProvider for SiteProvider {
    fn name(&self) -> &str {
        "lyonoyl.com"
    }

    fn the_manifest(&self) -> Result<Sealed, UpdateError> {
        let at = self.the_manifest_at();

        Ok(Sealed {
            manifest: self.small(&at)?,
            seal: self.small(&the_seal_over(&at))?,
        })
    }

    fn a_file(
        &self,
        version: Version,
        name: &str,
        at_most: usize,
        watching: Watching<'_>,
    ) -> Result<Vec<u8>, UpdateError> {
        // Whether the person said stop. The client answers a read its caller refused the way it
        // answers a connection that ended early, and only the first is the person's own doing.
        let stopped = std::cell::Cell::new(false);
        let fetched = self.runtime.block_on(self.client.get_watching(
            &format!("{}{name}", self.the_folder_of(version)),
            "application/octet-stream",
            at_most,
            &mut |so_far, whole| {
                let going_on = watching(so_far as u64, whole.map(|whole| whole as u64));
                if !going_on {
                    stopped.set(true);
                }
                going_on
            },
        ));

        match fetched {
            Ok(file) => Ok(file.bytes),
            Err(_) if stopped.get() => Err(UpdateError::Stopped),
            Err(_) => Err(UpdateError::CouldNotAsk),
        }
    }
}

#[cfg(test)]
mod tests;
