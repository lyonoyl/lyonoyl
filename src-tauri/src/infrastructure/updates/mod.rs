//! Whether a newer version is out, bringing it down, and putting it in place.
//!
//! In layers, each knowing only the one under it:
//!
//! - `asking_the_project` — the updater, the one implementation of the `Updates` port, which
//!   composes the parts below and knows no address, host or transport;
//! - `providers` — where releases are to be had, behind one interface, `UpdateProvider`, with the
//!   list of places a copy asks; a place is added or taken away there and nowhere else;
//! - `the_manifest` — what the project says is out, in the shape every copy reads;
//! - `the_seal` — whether a file is the project's own;
//! - `staging` — putting a version that came down in place of the running one;
//! - `what_it_keeps` — what the updater keeps on this machine between runs.

pub mod asking_the_project;
pub mod providers;
mod staging;
mod the_manifest;
mod the_seal;
mod what_it_keeps;

/// A release asked about before it is published, by the code every copy runs. macOS only, because
/// that is the only system a release is made for.
#[cfg(all(test, target_os = "macos"))]
mod before_publishing;
