use thiserror::Error;

pub use crate::ports::mail_provider::{WatchEnded, Watched, Watermark};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AccountSyncError {
    #[error("the account could not be synchronised")]
    Failed,
}

/// What one pass over one account changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AccountChanged {
    pub arrived: u32,
    pub updated: u32,
    pub removed: u32,
    /// Folders the pass found. Counted because a first pass over an account brings folders and
    /// often no mail at all, and an account whose folders arrived without anybody being told is an
    /// account that sits empty in the sidebar until something else happens to it.
    pub folders: u32,
    /// Messages read whole. Counted because what they say goes into the index as they are read,
    /// so a round that brought no mail at all still changed what a search answers — and an open
    /// search that is never told goes on showing the matches there were fifty messages ago.
    pub bodies_read: u32,
    /// Mail nobody has seen, that arrived in this account's inbox during this pass.
    ///
    /// Not the same as `arrived`, and the difference is the whole of what makes a notification
    /// bearable: a first pass over an account brings its whole mailbox and none of it is news, and
    /// mail read on another device arrives here already seen.
    pub unseen_in_inbox: u32,
}

impl AccountChanged {
    /// Whether anything a person would notice happened.
    ///
    /// What decides if the interface is told. A pass over a quiet mailbox is the common case, and
    /// announcing it would redraw the list under the reader for nothing.
    pub fn is_anything(self) -> bool {
        self.arrived + self.updated + self.removed + self.folders + self.bodies_read > 0
    }
}

/// Waiting for one account's server to say that something arrived.
///
/// Separate from reading, and optional: a server that cannot push says so, and the timer above
/// covers it. What a watch gives back is the fact that something happened, and whether mail arrived
/// with it — the pass that follows is the same pass a timer would have run.
pub trait AccountWatch: Send + Sync + 'static {
    /// `since` is where the account's last watch ended: mail that arrived after it is news to this
    /// one, however long nothing was listening in between.
    fn watch(
        &self,
        account_id: &str,
        since: Option<&Watermark>,
    ) -> Result<WatchEnded, AccountSyncError>;
}

/// Reading one account, whole: its mailboxes and what is in them.
///
/// The scheduler works in accounts rather than in folders. How an account is read — how many
/// folders, in what order, how far back — is decided below this port.
pub trait AccountSync: Send + Sync + 'static {
    fn sync(&self, account_id: &str) -> Result<AccountChanged, AccountSyncError>;

    /// The accounts on this device, in the order they should be visited.
    fn accounts(&self) -> Result<Vec<String>, AccountSyncError>;
}

/// What one push did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Pushed {
    /// Whether the server moved or took away a message, which only reading the account brings
    /// into the index. A flag the server confirmed is already written where every view reads it.
    pub moved: bool,
    /// Whether there may be more waiting than one push carries.
    pub more: bool,
    /// Whether a message went out, so its copy is to be filed once the server has had its moment.
    pub sent: bool,
}

/// What reading an account back after pushes found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ReadBack {
    pub changed: AccountChanged,
    /// Instructions that were waiting for exactly this read — a way back asked of where a message
    /// had just gone — and can be carried out now.
    pub let_go: u32,
}

/// Telling one account's server what a person has done, and nothing else.
///
/// Separate from reading, and much smaller: the queue and the outbox. Reading back what that moved
/// is its own step, taken once when a burst of pushes is over rather than once per push — a trash
/// of a thousand emptied twenty at a time would otherwise read the whole account fifty times.
pub trait AccountPush: Send + Sync + 'static {
    fn push(&self, account_id: &str) -> Result<Pushed, AccountSyncError>;

    /// Files the copies of what went out, and answers whether any copy is now in the folder for
    /// sent mail — which only reading the account brings into the index.
    fn file_copies(&self, account_id: &str) -> Result<bool, AccountSyncError>;

    fn read_back(&self, account_id: &str) -> Result<ReadBack, AccountSyncError>;
}
