/// Telling the window how far a pass has got, while it is still getting there.
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
}
