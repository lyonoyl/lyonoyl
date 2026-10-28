use crate::application::error::UseCaseError;
use crate::ports::mail_index::{FolderRecord, MailIndex};
use crate::ports::mail_provider::MailboxDiscovery;
use lyonoyl_domain::folder::{is_inbox, role_of_mailbox, FolderRole};
use lyonoyl_domain::message::FolderId;

/// What the account's mailboxes turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedMailboxes {
    /// Every mailbox, in the order the server listed them, with the identifier it is stored under.
    pub folders: Vec<(String, FolderId)>,
    /// The inbox, which is the folder the interface opens on. Absent only from a broken server.
    pub inbox: Option<FolderId>,
    /// Where the account keeps deleted mail, where it has somewhere.
    pub trash: Option<FolderId>,
}

/// Asks an account's server which mailboxes it has and records them.
///
/// Reading only: no mailbox is created, renamed or removed on the server, and none is removed
/// locally either.
pub struct SyncMailboxes<D: MailboxDiscovery, I: MailIndex> {
    discovery: D,
    index: I,
}

impl<D: MailboxDiscovery, I: MailIndex> SyncMailboxes<D, I> {
    pub fn new(discovery: D, index: I) -> Self {
        Self { discovery, index }
    }

    /// `account` is the number the index knows the account by, not the identifier the rest of the
    /// application uses.
    pub fn execute(&self, account: i64) -> Result<RecordedMailboxes, UseCaseError> {
        let mailboxes = self.discovery.list_mailboxes()?;

        let records: Vec<FolderRecord> = mailboxes
            .iter()
            .map(|mailbox| FolderRecord {
                name: mailbox.name.clone(),
                role: role_of_mailbox(&mailbox.name, mailbox.role),
            })
            .collect();

        let folders = self.index.record_folders(account, &records)?;

        let inbox = folders
            .iter()
            .find(|(name, _)| is_inbox(name))
            .map(|(_, id)| *id);

        let trash = records
            .iter()
            .find(|record| record.role == FolderRole::Trash)
            .and_then(|record| folders.iter().find(|(name, _)| *name == record.name))
            .map(|(_, id)| *id);

        Ok(RecordedMailboxes {
            folders,
            inbox,
            trash,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::ports::mail_index::{AppliedSummary, FolderSyncState, MailIndexError, UidScope};
    use crate::ports::mail_provider::{DiscoveredMailbox, MailProviderError};
    use lyonoyl_domain::folder::FolderRole;
    use lyonoyl_domain::message::{FlagScope, Uid, UidValidity};
    use lyonoyl_domain::sync::{FolderDelta, SyncCursor};

    fn mailbox(name: &str, role: FolderRole) -> DiscoveredMailbox {
        DiscoveredMailbox {
            name: name.to_owned(),
            role,
            uid_validity: None,
            uid_next: None,
        }
    }

    struct FakeDiscovery {
        answer: Result<Vec<DiscoveredMailbox>, MailProviderError>,
    }

    impl MailboxDiscovery for FakeDiscovery {
        fn list_mailboxes(&self) -> Result<Vec<DiscoveredMailbox>, MailProviderError> {
            match &self.answer {
                Ok(mailboxes) => Ok(mailboxes.clone()),
                Err(error) => Err(*error),
            }
        }
    }

    #[derive(Default)]
    struct FakeIndex {
        recorded: Mutex<Vec<(i64, Vec<FolderRecord>)>>,
        unavailable: bool,
    }

    impl MailIndex for FakeIndex {
        fn not_read_whole(
            &self,
            _folder_id: FolderId,
            _at_most: u32,
        ) -> Result<Vec<(lyonoyl_domain::message::MessageId, Uid)>, MailIndexError> {
            Ok(Vec::new())
        }
        fn record_folders(
            &self,
            account: i64,
            folders: &[FolderRecord],
        ) -> Result<Vec<(String, FolderId)>, MailIndexError> {
            if self.unavailable {
                return Err(MailIndexError::Unavailable);
            }

            self.recorded
                .lock()
                .expect("test lock")
                .push((account, folders.to_vec()));

            Ok(folders
                .iter()
                .enumerate()
                .map(|(position, folder)| {
                    (
                        folder.name.clone(),
                        FolderId::new(position as u64 + 1).expect("non-zero"),
                    )
                })
                .collect())
        }

        fn record_window_floor(
            &self,
            _folder_id: FolderId,
            _floor: Uid,
            _complete: bool,
        ) -> Result<(), MailIndexError> {
            unreachable!("recording mailboxes reads no message")
        }

        fn folder_state(&self, _folder_id: FolderId) -> Result<FolderSyncState, MailIndexError> {
            unreachable!("recording mailboxes does not read folder state")
        }

        fn inbox_of(&self, _account: i64) -> Result<Option<String>, MailIndexError> {
            unreachable!("recording mailboxes does not look for the inbox")
        }

        fn known_uids(
            &self,
            _folder_id: FolderId,
            _scope: &UidScope,
        ) -> Result<Vec<Uid>, MailIndexError> {
            unreachable!("recording mailboxes does not read UIDs")
        }

        fn placement_count(&self, _folder_id: FolderId) -> Result<usize, MailIndexError> {
            unreachable!("recording mailboxes does not count placements")
        }

        fn apply(
            &self,
            _delta: &FolderDelta,
            _deletions: &[Uid],
            _cursor: &SyncCursor,
            _flag_scope: FlagScope,
        ) -> Result<AppliedSummary, MailIndexError> {
            unreachable!("recording mailboxes applies no delta")
        }

        fn reset_folder(
            &self,
            _folder_id: FolderId,
            _uid_validity: UidValidity,
        ) -> Result<(), MailIndexError> {
            unreachable!("recording mailboxes resets nothing")
        }

        fn forget_it_stopped_short(&self, _folder_id: FolderId) -> Result<(), MailIndexError> {
            unreachable!("recording mailboxes reads no folder")
        }

        fn refused_for_now(
            &self,
            _folder_id: FolderId,
            _reading: crate::ports::mail_index::Reading,
            _refused: bool,
        ) -> Result<(), MailIndexError> {
            unreachable!("recording mailboxes reads no folder")
        }

        fn holds(
            &self,
            _folder_id: FolderId,
            _message: lyonoyl_domain::message::MessageId,
        ) -> Result<bool, MailIndexError> {
            unreachable!("recording mailboxes asks after no copy")
        }

        fn unplaced(
            &self,
            _account: i64,
        ) -> Result<Vec<lyonoyl_domain::message::MessageId>, MailIndexError> {
            unreachable!("recording mailboxes asks after no message")
        }

        fn forget_the_unplaced(
            &self,
            _account: i64,
            _named: &[lyonoyl_domain::message::MessageId],
        ) -> Result<(), MailIndexError> {
            unreachable!("recording mailboxes forgets nothing")
        }
    }

    fn run(
        mailboxes: Vec<DiscoveredMailbox>,
    ) -> (Result<RecordedMailboxes, UseCaseError>, FakeIndex) {
        let index = FakeIndex::default();
        let outcome = SyncMailboxes::new(
            FakeDiscovery {
                answer: Ok(mailboxes),
            },
            &index,
        )
        .execute(4);
        (outcome, index)
    }

    impl MailIndex for &FakeIndex {
        fn not_read_whole(
            &self,
            _folder_id: FolderId,
            _at_most: u32,
        ) -> Result<Vec<(lyonoyl_domain::message::MessageId, Uid)>, MailIndexError> {
            Ok(Vec::new())
        }
        fn record_folders(
            &self,
            account: i64,
            folders: &[FolderRecord],
        ) -> Result<Vec<(String, FolderId)>, MailIndexError> {
            (*self).record_folders(account, folders)
        }

        fn record_window_floor(
            &self,
            folder_id: FolderId,
            floor: Uid,
            complete: bool,
        ) -> Result<(), MailIndexError> {
            (*self).record_window_floor(folder_id, floor, complete)
        }

        fn folder_state(&self, folder_id: FolderId) -> Result<FolderSyncState, MailIndexError> {
            (*self).folder_state(folder_id)
        }

        fn inbox_of(&self, account: i64) -> Result<Option<String>, MailIndexError> {
            (*self).inbox_of(account)
        }

        fn known_uids(
            &self,
            folder_id: FolderId,
            scope: &UidScope,
        ) -> Result<Vec<Uid>, MailIndexError> {
            (*self).known_uids(folder_id, scope)
        }

        fn placement_count(&self, folder_id: FolderId) -> Result<usize, MailIndexError> {
            (*self).placement_count(folder_id)
        }

        fn apply(
            &self,
            delta: &FolderDelta,
            deletions: &[Uid],
            cursor: &SyncCursor,
            flag_scope: FlagScope,
        ) -> Result<AppliedSummary, MailIndexError> {
            (*self).apply(delta, deletions, cursor, flag_scope)
        }

        fn reset_folder(
            &self,
            folder_id: FolderId,
            uid_validity: UidValidity,
        ) -> Result<(), MailIndexError> {
            (*self).reset_folder(folder_id, uid_validity)
        }

        fn forget_it_stopped_short(&self, folder_id: FolderId) -> Result<(), MailIndexError> {
            (*self).forget_it_stopped_short(folder_id)
        }

        fn refused_for_now(
            &self,
            folder_id: FolderId,
            reading: crate::ports::mail_index::Reading,
            refused: bool,
        ) -> Result<(), MailIndexError> {
            (*self).refused_for_now(folder_id, reading, refused)
        }

        fn holds(
            &self,
            folder_id: FolderId,
            message: lyonoyl_domain::message::MessageId,
        ) -> Result<bool, MailIndexError> {
            (*self).holds(folder_id, message)
        }

        fn unplaced(
            &self,
            account: i64,
        ) -> Result<Vec<lyonoyl_domain::message::MessageId>, MailIndexError> {
            (*self).unplaced(account)
        }

        fn forget_the_unplaced(
            &self,
            account: i64,
            named: &[lyonoyl_domain::message::MessageId],
        ) -> Result<(), MailIndexError> {
            (*self).forget_the_unplaced(account, named)
        }
    }

    #[test]
    fn every_mailbox_the_server_listed_is_recorded_against_the_account() {
        let (outcome, index) = run(vec![
            mailbox("INBOX", FolderRole::Custom),
            mailbox("Sent", FolderRole::Sent),
            mailbox("Projects 2026", FolderRole::Custom),
        ]);

        let recorded = outcome.expect("the mailboxes are recorded");
        assert_eq!(recorded.folders.len(), 3);

        let calls = index.recorded.lock().expect("test lock");
        assert_eq!(calls.len(), 1, "one listing is one write");
        assert_eq!(calls[0].0, 4, "recorded against the account that was asked");
    }

    #[test]
    fn the_inbox_is_named_the_inbox_even_though_no_server_marks_it() {
        // Without this the folder a person opens first is listed as an ordinary folder, under the
        // name the protocol uses, at the bottom of the sidebar.
        let (outcome, index) = run(vec![
            mailbox("INBOX", FolderRole::Custom),
            mailbox("Archive", FolderRole::Archive),
        ]);

        let recorded = outcome.expect("the mailboxes are recorded");
        assert_eq!(recorded.inbox, Some(FolderId::new(1).expect("non-zero")));

        let calls = index.recorded.lock().expect("test lock");
        assert_eq!(calls[0].1[0].role, FolderRole::Inbox);
        assert_eq!(calls[0].1[1].role, FolderRole::Archive);
    }

    #[test]
    fn a_translated_name_is_not_guessed_at() {
        let (outcome, index) = run(vec![
            mailbox("INBOX", FolderRole::Custom),
            mailbox("Отправленные", FolderRole::Custom),
        ]);

        outcome.expect("the mailboxes are recorded");
        let calls = index.recorded.lock().expect("test lock");
        assert_eq!(
            calls[0].1[1].role,
            FolderRole::Custom,
            "a name is not evidence of what a folder is for"
        );
    }

    #[test]
    fn an_account_without_an_inbox_is_reported_rather_than_invented() {
        let (outcome, _) = run(vec![mailbox("Archive", FolderRole::Archive)]);

        assert_eq!(outcome.expect("the mailboxes are recorded").inbox, None);
    }

    #[test]
    fn a_server_that_cannot_be_reached_records_nothing() {
        let index = FakeIndex::default();
        let error = SyncMailboxes::new(
            FakeDiscovery {
                answer: Err(MailProviderError::Offline),
            },
            &index,
        )
        .execute(4)
        .expect_err("the server cannot be reached");

        assert!(matches!(error, UseCaseError::Offline));
        assert!(
            index.recorded.lock().expect("test lock").is_empty(),
            "a failed listing must not touch the folder tree"
        );
    }

    #[test]
    fn a_listing_that_cannot_be_stored_is_reported_as_storage() {
        let index = FakeIndex {
            unavailable: true,
            ..Default::default()
        };
        let error = SyncMailboxes::new(
            FakeDiscovery {
                answer: Ok(vec![mailbox("INBOX", FolderRole::Custom)]),
            },
            &index,
        )
        .execute(4)
        .expect_err("the index is unavailable");

        assert!(matches!(error, UseCaseError::StorageUnavailable));
    }
}
