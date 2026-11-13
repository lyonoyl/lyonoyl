use crate::ports::account_sync::AccountChanged;

/// Telling the window what a pass has got to while it is still going: how far, and its inbox once
/// it is written down.
///
/// One mailbox is read at once with every other, so a round is nine things starting together and
/// finishing together: counted by mailbox, "how far" is nought for five seconds and then nine.
/// What finishes steadily is the folder — each account reads its own one after another — so that
/// is the unit the bar moves by, and mailboxes are what the words count.
///
/// Numbers only. What is being read is a person's own mailbox, and its name is not something to
/// carry out to a line that shows a bar.
pub trait Progress: Send + Sync {
    /// A pass over one account is beginning, and has this many folders to read.
    fn reading(&self, account_id: &str, folders: u32);

    /// One more folder of that account is done.
    fn folder_read(&self, account_id: &str);

    /// The account's inbox is read and written down, and this is what reading it changed.
    ///
    /// Said the moment it is so, before the inbox's older mail, its messages read whole and every
    /// other folder: mail a server has just said arrived is in the inbox, and the window can show
    /// it seconds before the pass ends. Nothing is said by default, which is what a pass with
    /// nobody listening for it wants.
    fn inbox_read(&self, account_id: &str, changed: AccountChanged) {
        let _ = (account_id, changed);
    }
}

/// Nothing is told. What a test uses, and what a pass nobody is watching gets.
impl Progress for () {
    fn reading(&self, _account_id: &str, _folders: u32) {}
    fn folder_read(&self, _account_id: &str) {}
}

impl<T: Progress + ?Sized> Progress for &T {
    fn reading(&self, account_id: &str, folders: u32) {
        (**self).reading(account_id, folders);
    }

    fn folder_read(&self, account_id: &str) {
        (**self).folder_read(account_id);
    }

    fn inbox_read(&self, account_id: &str, changed: AccountChanged) {
        (**self).inbox_read(account_id, changed);
    }
}
