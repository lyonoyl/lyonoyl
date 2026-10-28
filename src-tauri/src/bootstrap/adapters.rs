//! What a command hands a use case: every adapter behind a port, chosen here.
//!
//! A command asks this module for what a use case takes and never names the type that does the
//! work, so replacing an implementation is a change to this file and nothing else. What holds a
//! pool is made when a command asks rather than once at the start, because on some devices the
//! databases open after it; making one is wrapping a pool.

use std::sync::Arc;
use std::thread::ThreadId;

use tauri::{AppHandle, Runtime};
use tokio::runtime::Handle;

use crate::infrastructure::dialog::{SystemOpenDialog, SystemSaveDialog};
use crate::infrastructure::http::autoconfig::PublishedByTheDomain;
use crate::infrastructure::http::pictures::RemotePictures;
use crate::infrastructure::http::HttpClient;
use crate::infrastructure::net::{TransportError, TransportPolicy};
use crate::infrastructure::secure_storage::message_key::MessageKey;
use crate::infrastructure::sqlite::account_groups::SqliteAccountGroups;
use crate::infrastructure::sqlite::account_repository::SqliteAccountRepository;
use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::infrastructure::sqlite::drafts::SqliteDrafts;
use crate::infrastructure::sqlite::mail_index::SqliteMailIndex;
use crate::infrastructure::sqlite::message_bodies::SqliteMessageBodies;
use crate::infrastructure::sqlite::op_log::SqliteOperationLog;
use crate::infrastructure::sqlite::outbox::SqliteOutbox;
use crate::infrastructure::sqlite::placement::SqlitePlacements;
use crate::infrastructure::sqlite::settings_repository::{
    SqliteDeviceStateRepository, SqlitePreferencesRepository,
};
use crate::ports::credential_store::{CredentialError, CredentialStore};

/// The accounts on this device, and how each reads and sends.
pub fn accounts(profile: SqlitePool) -> SqliteAccountRepository {
    SqliteAccountRepository::new(profile)
}

/// The groups the sidebar arranges accounts into.
pub fn account_groups(profile: SqlitePool) -> SqliteAccountGroups {
    SqliteAccountGroups::new(profile)
}

/// The person's settings.
pub fn preferences(profile: SqlitePool) -> SqlitePreferencesRepository {
    SqlitePreferencesRepository::new(profile)
}

/// What this machine remembers of the window: its panes and what is folded.
pub fn device_state(profile: SqlitePool) -> SqliteDeviceStateRepository {
    SqliteDeviceStateRepository::new(profile)
}

/// What a person did that has still to reach a server.
pub fn operation_log(profile: SqlitePool) -> SqliteOperationLog {
    SqliteOperationLog::new(profile)
}

/// The messages being written.
pub fn drafts(profile: SqlitePool) -> SqliteDrafts {
    SqliteDrafts::new(profile)
}

/// The messages waiting to be sent.
pub fn outbox(profile: SqlitePool) -> SqliteOutbox {
    SqliteOutbox::new(profile)
}

/// An account's mail as this device holds it.
pub fn mail_index(index: SqlitePool) -> SqliteMailIndex {
    SqliteMailIndex::new(index)
}

/// Where each message of an account's mail sits on its server.
pub fn placements(index: SqlitePool) -> SqlitePlacements {
    SqlitePlacements::new(index)
}

/// The bodies of an account's messages, sealed under the key of this device.
pub fn message_bodies(index: SqlitePool, sealing: MessageKey) -> SqliteMessageBodies {
    SqliteMessageBodies::new(index, sealing)
}

/// The key message bodies are sealed under, from the store this device keeps its secrets in.
pub fn message_key(store: &dyn CredentialStore) -> Result<MessageKey, CredentialError> {
    MessageKey::of_this_device(store)
}

/// Which addresses this application may reach, and how.
pub fn transport() -> Result<TransportPolicy, TransportError> {
    TransportPolicy::new()
}

/// The client every request this application makes outside mail goes through.
pub fn http_client(road: Arc<TransportPolicy>) -> HttpClient {
    HttpClient::new(road)
}

/// The pictures a message points at, fetched by this process and never by the frame.
pub fn remote_pictures(client: Arc<HttpClient>, runtime: Handle) -> RemotePictures {
    RemotePictures::new(client, runtime)
}

/// The server settings a domain publishes for its mail.
pub fn published_by_the_domain(client: Arc<HttpClient>, runtime: Handle) -> PublishedByTheDomain {
    PublishedByTheDomain::new(client, runtime)
}

/// The system's own panel for choosing where a file goes.
pub fn save_dialog<R: Runtime>(app: AppHandle<R>, event_loop: ThreadId) -> SystemSaveDialog<R> {
    SystemSaveDialog::new(app, event_loop)
}

/// The system's own panel for choosing files to attach.
pub fn open_dialog<R: Runtime>(app: AppHandle<R>, event_loop: ThreadId) -> SystemOpenDialog<R> {
    SystemOpenDialog::new(app, event_loop)
}
