//! The sidebar: every account on this device and the folders under it.
//!
//! The only read model that opens both databases. The profile is authoritative for which accounts
//! exist, what they are called and what is being written from them; the index holds their folders
//! and the counts. They are joined here, in Rust, because SQLite cannot join across two files and
//! because the two answer different questions: one survives losing the index, the other is rebuilt
//! from a server.

use std::collections::HashMap;

use crate::infrastructure::sqlite::connection::SqlitePool;
use crate::read_model::as_asked;
use crate::read_model::list::ReadModelError;
use lyonoyl_domain::as_asked::{moved_by, AsAsked, Held};
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::mailbox_name::readable;
use lyonoyl_domain::message::{FolderId, MessageId};

/// Folders in the order a mail client shows them: by what a folder is for, then by name.
///
/// Providers translate folder names, so an alphabetical list puts the inbox in a different place
/// in every language.
const BY_PURPOSE: &str = "CASE f.role
                              WHEN 1 THEN 0   -- Inbox
                              WHEN 3 THEN 1   -- Drafts
                              WHEN 2 THEN 2   -- Sent
                              WHEN 8 THEN 3   -- Flagged
                              WHEN 6 THEN 4   -- Archive
                              WHEN 7 THEN 5   -- All mail
                              WHEN 5 THEN 6   -- Junk
                              WHEN 4 THEN 7   -- Trash
                              ELSE 8
                          END,
                          f.name";

/// One folder under an account.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderNode {
    pub folder_id: FolderId,
    /// The name the provider gave. Shown only for folders without a role.
    /// The name a person reads. Never the one the server answers to: that stays in the database
    /// and in the adapter, because decoding what is sent addresses a mailbox nobody has.
    pub name: String,
    pub role: FolderRole,
    pub total: u32,
    pub unread: u32,
    /// Messages being written from this account and kept on this device, on its drafts folder.
    ///
    /// Nought on every other folder. Its own number rather than part of `total`: `total` is what
    /// the folder holds on the server, and the drafts folder shows both — so the badge adds the
    /// two, and neither half is mistaken for the other.
    pub drafts_here: u32,
}

/// One account, with everything the sidebar draws for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountNode {
    /// The identifier the rest of the application uses. Opaque, and stable across a rename.
    pub account_id: String,
    pub email: String,
    pub display_name: String,
    /// Whether its folders are folded away. Remembered with the account.
    pub collapsed: bool,
    /// Where it stands: at the top of the sidebar among the groups and the other accounts in no
    /// group, or, in a group, among the accounts in that group.
    pub place: i64,
    /// Unread across the whole account, which is what its own row counts.
    pub unread: u32,
    pub folders: Vec<FolderNode>,
    /// Whether this account's mail could be read at all.
    ///
    /// Its mail is in a file of its own, and a file that will not open leaves the account here with
    /// nothing under it. Saying so is the difference between "this account has no folders" and
    /// "this account's mail could not be read" — which look the same and mean opposite things.
    pub answered: bool,
}

pub struct MailboxReadModel {
    profile: SqlitePool,
    /// The files the mail is kept in, one per account.
    ///
    /// The sidebar spans them: an account lives in exactly one file, so what each answers about
    /// its own accounts is joined rather than merged — there is no order to keep and no key two
    /// files can both claim.
    indexes: Vec<(i64, SqlitePool)>,
}

impl MailboxReadModel {
    pub fn new(profile: SqlitePool, index: SqlitePool) -> Self {
        Self::spanning(profile, vec![(1, index)])
    }

    pub fn spanning(profile: SqlitePool, indexes: Vec<(i64, SqlitePool)>) -> Self {
        Self { profile, indexes }
    }

    /// The one file, for a question about a single account that cannot yet name which.
    ///
    /// Asking whose folder this is means reading the file being chosen between, and that circle is
    /// closed by an identifier carrying its account rather than here. Until it is, these refuse out
    /// loud with more than one file rather than answering from whichever came first.
    fn the_one(&self) -> Result<&SqlitePool, ReadModelError> {
        match self.indexes.as_slice() {
            [(_, only)] => Ok(only),
            _ => Err(ReadModelError::Unavailable),
        }
    }

    /// The whole sidebar, without how far the queue had got: what a test asks about.
    #[cfg(test)]
    pub fn tree(&self) -> Result<Vec<AccountNode>, ReadModelError> {
        self.tree_as_of().map(|(tree, _)| tree)
    }

    /// Reads the whole sidebar, with how many instructions a person had asked for when the queue
    /// it lays over the counts was read: a change asked for later than that is not in them.
    ///
    /// Three statements for the profile — the accounts, the queue, and what is being written from
    /// them — then two per account file: its folders and its unread counts, and a third where its
    /// accounts have something outstanding. Per file rather than in total, because the mail is one
    /// file per account and no statement reaches across files — the cost of the split, paid here
    /// and nowhere else. What must never happen is two per *folder*, which is what a hundred
    /// accounts would turn into a thousand queries.
    pub fn tree_as_of(&self) -> Result<(Vec<AccountNode>, u64), ReadModelError> {
        self.tree_read(|| {})
    }

    /// The same read, with `between` run after each statement and before the next.
    ///
    /// Nothing runs there in the product. It is where a test lands a push in the middle of a read,
    /// at every place one could land.
    fn tree_read(
        &self,
        mut between: impl FnMut(),
    ) -> Result<(Vec<AccountNode>, u64), ReadModelError> {
        let accounts = self.accounts()?;
        if accounts.is_empty() {
            return Ok((Vec::new(), 0));
        }
        between();

        let (queued, asked_through) = as_asked::everywhere_as_of(&self.profile)?;
        between();

        let mut folders = HashMap::new();
        let mut unread = HashMap::new();
        let mut quiet = Vec::new();
        for (number, index) in &self.indexes {
            match self.counted_in(*number, index, &queued, Wanted::Folders, &mut between) {
                Ok(counted) => {
                    folders.extend(counted.folders);
                    unread.extend(counted.unread);
                }
                // An account whose file will not answer is left in the sidebar rather than taken
                // out of a list the profile says it belongs in — but named, not silently empty.
                // An account with no folders and an account whose mail could not be read look
                // identical and mean the opposite.
                Err(_) => quiet.push(*number),
            }
        }

        let written_here = self.drafts_by_account()?;

        let tree = accounts
            .into_iter()
            .map(|(index_id, mut node)| {
                // Empty either way, and `answered` is what tells the two apart: a mailbox with
                // nothing in it yet, and one whose file was busy when this was asked. The window
                // acts on the difference — read as the first, the second made it ask the server
                // about every account, which kept every file busy, which emptied this again.
                node.folders = folders.remove(&index_id).unwrap_or_default();
                node.unread = unread.get(&index_id).copied().unwrap_or(0);
                node.answered = !quiet.contains(&index_id);
                carry_drafts(
                    &mut node.folders,
                    written_here.get(&node.account_id).copied().unwrap_or(0),
                );
                node
            })
            .collect();

        Ok((tree, asked_through))
    }

    /// Resolves an account to the number the index knows it by.
    ///
    /// `None` means the account is not on this device, which is a stale selection rather than a
    /// failure: the sidebar may have been read before it was removed.
    pub fn index_id_of(&self, account_id: &str) -> Result<Option<i64>, ReadModelError> {
        index_id_of(&self.profile, account_id)
    }

    /// The other direction of `index_id_of`: the index addresses an account by a number, and
    /// everything above it — the keychain, the operation log, the interface — by its identifier.
    pub fn account_of(&self, index_id: i64) -> Result<Option<String>, ReadModelError> {
        self.profile
            .get()
            .map_err(|_| ReadModelError::Unavailable)?
            .query_row(
                "SELECT id FROM mail_accounts WHERE index_id = ?1",
                [index_id],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                _ => Err(ReadModelError::Unavailable),
            })
    }

    /// What a folder is for, and which account number owns it.
    ///
    /// Both come from one row because a move needs both: the account to record the operation
    /// against, and the role to refuse a destination that means something else.
    /// The number of the account a folder belongs to, or nothing when no folder has that number.
    pub fn account_number_of_folder(
        &self,
        folder: FolderId,
    ) -> Result<Option<i64>, ReadModelError> {
        Ok(account_that_minted(folder.get()))
    }

    /// What a folder **of this account** is for, or nothing when this account has no such folder.
    ///
    /// The account is an argument rather than part of the answer, and that is the whole point. This
    /// replaced a call that returned the owning account beside the role, which every caller then
    /// destructured — `let Some((_, role))` — and one of them threw the account away. What it did
    /// with it was tell account A's server to file a message into a mailbox named after account B's
    /// folder; `INBOX`, `Archive` and `Sent` collide across accounts constantly, so it succeeded
    /// against the wrong mailbox rather than failing. A folder that cannot be asked about without
    /// naming an account cannot be asked about across accounts.
    pub fn folder_purpose_in(
        &self,
        account: i64,
        folder: FolderId,
    ) -> Result<Option<FolderRole>, ReadModelError> {
        self.the_one()?
            .get()
            .map_err(|_| ReadModelError::Unavailable)?
            .query_row(
                "SELECT role FROM folders WHERE id = ?1 AND account_id = ?2",
                [folder.get() as i64, account],
                |row| row.get::<_, i64>(0),
            )
            .map(|role| Some(FolderRole::from_stored(role)))
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                _ => Err(ReadModelError::Unavailable),
            })
    }

    /// The folder an account keeps a given kind of mail in, where it has one.
    ///
    /// The oldest one, so an account that somehow grew two answers the same way every time rather
    /// than filing mail in whichever the query happened to see first.
    pub fn folder_for_role(
        &self,
        account: i64,
        role: FolderRole,
    ) -> Result<Option<FolderId>, ReadModelError> {
        let found: Option<i64> = self
            .the_one()?
            .get()
            .map_err(|_| ReadModelError::Unavailable)?
            .query_row(
                "SELECT id FROM folders
                 WHERE account_id = ?1 AND role = ?2
                 ORDER BY id ASC
                 LIMIT 1",
                rusqlite::params![account, role.stored()],
                |row| row.get(0),
            )
            .map(Some)
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                _ => Err(ReadModelError::Unavailable),
            })?;

        Ok(found
            .and_then(|id| u64::try_from(id).ok())
            .and_then(|id| FolderId::new(id).ok()))
    }

    /// Accounts in the order they were added, each with the number the index knows it by.
    fn accounts(&self) -> Result<Vec<(i64, AccountNode)>, ReadModelError> {
        let connection = self
            .profile
            .get()
            .map_err(|_| ReadModelError::Unavailable)?;

        let mut statement = connection
            .prepare(
                // The order somebody arranged them into, and the order they arrived in where two
                // places agree. Each account is put last as it is added, so the tie-breakers order
                // only what a profile written before that holds.
                "SELECT id, email, display_name, index_id, sidebar_collapsed, position_in_group
                 FROM mail_accounts
                 ORDER BY position_in_group ASC, created_at ASC, id ASC",
            )
            .map_err(|_| ReadModelError::Unavailable)?;

        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })
            .map_err(|_| ReadModelError::Unavailable)?;

        let mut accounts = Vec::new();
        for row in rows {
            let (account_id, email, display_name, index_id, collapsed, place) =
                row.map_err(|_| ReadModelError::Unavailable)?;
            accounts.push((
                index_id,
                AccountNode {
                    account_id,
                    email,
                    display_name,
                    collapsed: collapsed != 0,
                    place,
                    unread: 0,
                    folders: Vec::new(),
                    // Until its file has been asked. Filled in by `tree`, which is the only thing
                    // that asks.
                    answered: true,
                },
            ));
        }

        Ok(accounts)
    }

    /// What one account file counts, with what its accounts have asked for laid over it.
    ///
    /// Every statement here reads one snapshot of the file, and the queue is read before it. That
    /// order is what makes the two agree: a push writes what the server confirmed onto the index
    /// first and writes the instruction off after, and a pass settles a move only once it has
    /// written where the message went. So an instruction missing from the queue is already in the
    /// snapshot, and one still in it is folded over copies read from the same snapshot as the
    /// counts — which moves nothing where the snapshot already has it. Read apart, a push landing
    /// between two statements leaves a count without the change beside a number saying it is in,
    /// and the window, trusting the number, drops its own; nothing reads the sidebar again after a
    /// flag, so the count stays wrong.
    ///
    /// An account whose file will not answer is left as its file counts it — which is nothing, and
    /// already said beside it.
    fn counted_in(
        &self,
        number: i64,
        index: &SqlitePool,
        queued: &[as_asked::Queued],
        wanted: Wanted,
        between: &mut impl FnMut(),
    ) -> Result<Counts, ReadModelError> {
        let mut connection = index.get().map_err(|_| ReadModelError::Unavailable)?;
        // Deferred: nothing here writes, and a reader's snapshot is taken at its first statement
        // and held until the transaction ends, whatever is written beside it meanwhile.
        let snapshot = connection
            .transaction()
            .map_err(|_| ReadModelError::Unavailable)?;

        let mut folders = match wanted {
            Wanted::Folders => {
                let kept = folders_kept_in(&snapshot)?;
                between();
                kept
            }
            Wanted::Unread => HashMap::new(),
        };
        let mut unread = unread_kept_in(&snapshot).unwrap_or_default();
        between();

        let mut by_account: HashMap<i64, Vec<as_asked::Queued>> = HashMap::new();
        for one in queued.iter().filter(|one| self.keeps(number, one.account)) {
            by_account.entry(one.account).or_default().push(*one);
        }

        // What a person has asked for, laid over what the index counts. The same fold the list
        // reads its rows through, so a folder cannot count a message unread that its own list
        // shows as read.
        for (account, queued) in by_account {
            let folded =
                as_asked::folded(&queued, |account| as_asked::trash_on(&snapshot, account));
            let Ok((held, inboxes)) = held_in(&snapshot, account, &folded) else {
                continue;
            };
            between();
            let moved = folded.counted(&held, |folder| inboxes.contains(&folder));

            for folder in folders.get_mut(&account).into_iter().flatten() {
                if let Some((total, unread)) = moved.folders.get(&folder.folder_id) {
                    folder.total = moved_by(folder.total, *total);
                    folder.unread = moved_by(folder.unread, *unread);
                }
            }
            let unread = unread.entry(account).or_insert(0);
            *unread = moved_by(*unread, moved.inbox_unread);
        }

        Ok(Counts { folders, unread })
    }

    /// Whether the file numbered `number` is the one an account's mail is kept in: the one there
    /// is, or the one numbered for it.
    fn keeps(&self, number: i64, account: i64) -> bool {
        self.indexes.len() == 1 || number == account
    }

    /// How many messages each account has being written and kept on this device.
    ///
    /// From the profile, because that is where a draft is kept; nothing in any mail file knows it
    /// exists. Accounts with none are absent rather than nought.
    fn drafts_by_account(&self) -> Result<HashMap<String, u32>, ReadModelError> {
        let connection = self
            .profile
            .get()
            .map_err(|_| ReadModelError::Unavailable)?;

        let mut statement = connection
            .prepare("SELECT account_id, count(*) FROM drafts GROUP BY account_id")
            .map_err(|_| ReadModelError::Unavailable)?;

        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .map_err(|_| ReadModelError::Unavailable)?;

        let mut counts = HashMap::new();
        for row in rows {
            let (account_id, kept) = row.map_err(|_| ReadModelError::Unavailable)?;
            counts.insert(
                account_id,
                u32::try_from(kept).map_err(|_| ReadModelError::Corrupted)?,
            );
        }

        Ok(counts)
    }

    /// Everything nobody has read, across every account this device holds.
    ///
    /// The number on the application's own icon, and the same mail the sidebar counts per account
    /// — summed here rather than in whoever asks, and read in the same order as the sidebar, so
    /// the icon and the rows under it can never answer differently.
    pub fn unread_everywhere(&self) -> Result<u32, ReadModelError> {
        self.unread_read(|| {})
    }

    /// The same count, with `between` run after each statement, for the reason `tree_read` has it.
    fn unread_read(&self, mut between: impl FnMut()) -> Result<u32, ReadModelError> {
        let (queued, _) = as_asked::everywhere_as_of(&self.profile)?;
        between();

        let mut unread = 0;
        for (number, index) in &self.indexes {
            if let Ok(counted) =
                self.counted_in(*number, index, &queued, Wanted::Unread, &mut between)
            {
                unread += counted.unread.values().sum::<u32>();
            }
        }
        Ok(unread)
    }
}

/// How much of a file the question needs: the whole sidebar, or only what the icon counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wanted {
    Folders,
    Unread,
}

/// What one account file counts, keyed by the number of each account it keeps.
struct Counts {
    folders: HashMap<i64, Vec<FolderNode>>,
    /// What is unread in each account's inbox, which is what its own row counts.
    unread: HashMap<i64, u32>,
}

/// Every account's folders in one file, with what each holds on the server.
fn folders_kept_in(
    connection: &rusqlite::Connection,
) -> Result<HashMap<i64, Vec<FolderNode>>, ReadModelError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT f.account_id,
                    f.id,
                    f.name,
                    f.role,
                    count(DISTINCT p.message_id),
                    count(DISTINCT p.message_id) FILTER (
                        WHERE (p.flags & 1) = 0
                    )
             -- No join to `messages`: not one of its columns is read here, and a placement
             -- without a message cannot exist -- the foreign key says so and the cascade
             -- keeps it true. The join cost a lookup by rowid for every placement in every
             -- folder of every account, on a panel rebuilt whenever a synchronisation
             -- reports a change.
             FROM folders f
             LEFT JOIN message_placements p ON p.folder_id = f.id
             GROUP BY f.id
             ORDER BY f.account_id, {BY_PURPOSE}"
        ))
        .map_err(|_| ReadModelError::Unavailable)?;

    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })
        .map_err(|_| ReadModelError::Unavailable)?;

    let mut by_account: HashMap<i64, Vec<FolderNode>> = HashMap::new();
    for row in rows {
        let (account_id, id, wire, role, total, unread) =
            row.map_err(|_| ReadModelError::Unavailable)?;

        // Decoded here and nowhere earlier. The name in the database is the one the server
        // answers to — `SELECT` takes it and nothing else — and the Russian for "Sent" arrives on
        // the wire as `&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-`. Showing that is showing
        // line noise; storing the readable one would address a mailbox nobody has.
        let name = readable(&wire);

        by_account.entry(account_id).or_default().push(FolderNode {
            folder_id: u64::try_from(id)
                .ok()
                .and_then(|value| FolderId::new(value).ok())
                .ok_or(ReadModelError::Corrupted)?,
            name,
            role: FolderRole::from_stored(role),
            total: u32::try_from(total).map_err(|_| ReadModelError::Corrupted)?,
            unread: u32::try_from(unread).map_err(|_| ReadModelError::Corrupted)?,
            drafts_here: 0,
        });
    }

    Ok(by_account)
}

/// Unread per account in one file: what is unread in its inbox.
///
/// The same mail the account's own row shows when it is selected, counted the same way — from
/// the placement, which is where a flag lives when the provider keeps flags per mailbox. A count
/// taken over every folder would include drafts and the bin, and would not match the list it sits
/// above.
fn unread_kept_in(connection: &rusqlite::Connection) -> Result<HashMap<i64, u32>, ReadModelError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT f.account_id, count(DISTINCT p.message_id)
             -- The same dead join, and the same reason it changed nothing: an inner one
             -- here could only have dropped a placement whose message is missing, which the
             -- foreign key forbids.
             FROM message_placements p
             JOIN folders f ON f.id = p.folder_id
             WHERE f.role = {} AND (p.flags & 1) = 0
             GROUP BY f.account_id",
            FolderRole::Inbox.stored()
        ))
        .map_err(|_| ReadModelError::Unavailable)?;

    let rows = statement
        .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
        .map_err(|_| ReadModelError::Unavailable)?;

    let mut counts = HashMap::new();
    for row in rows {
        let (account_id, unread) = row.map_err(|_| ReadModelError::Unavailable)?;
        counts.insert(
            account_id,
            u32::try_from(unread).map_err(|_| ReadModelError::Corrupted)?,
        );
    }

    Ok(counts)
}

/// Every copy an account's file holds of the messages something was asked about, and which of its
/// folders are inboxes.
fn held_in(
    connection: &rusqlite::Connection,
    account: i64,
    folded: &AsAsked,
) -> Result<(Vec<Held>, Vec<FolderId>), ReadModelError> {
    let folder_id = |id: i64| FolderId::new(id as u64).map_err(|_| ReadModelError::Corrupted);

    let messages: Vec<i64> = folded
        .messages()
        .into_iter()
        .map(|message| message.get() as i64)
        .collect();

    let mut held = Vec::new();
    // In slices, for the reason the queue is read in slices: emptying a large trash is one
    // instruction per message in it.
    for slice in messages.chunks(500) {
        let places = std::iter::repeat_n("?", slice.len())
            .collect::<Vec<_>>()
            .join(", ");
        let mut statement = connection
            .prepare(&format!(
                "SELECT message_id, folder_id, (flags & 1) = 0
                 FROM message_placements
                 WHERE message_id IN ({places})"
            ))
            .map_err(|_| ReadModelError::Unavailable)?;
        let copies = statement
            .query_map(rusqlite::params_from_iter(slice), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, bool>(2)?,
                ))
            })
            .map_err(|_| ReadModelError::Unavailable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ReadModelError::Unavailable)?;

        for (message, folder, unread) in copies {
            held.push(Held {
                message: MessageId::new(message as u64).map_err(|_| ReadModelError::Corrupted)?,
                folder: folder_id(folder)?,
                unread,
            });
        }
    }

    let inboxes = connection
        .prepare("SELECT id FROM folders WHERE account_id = ?1 AND role = ?2")
        .and_then(|mut statement| {
            statement
                .query_map([account, FolderRole::Inbox.stored()], |row| {
                    row.get::<_, i64>(0)
                })
                .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        })
        .map_err(|_| ReadModelError::Unavailable)?
        .into_iter()
        .map(folder_id)
        .collect::<Result<Vec<_>, _>>()?;

    Ok((held, inboxes))
}

/// Puts what an account is writing on this device onto its drafts folder.
///
/// Onto every folder the server calls its drafts, because every one of them shows these when it
/// is opened, and a number beside a folder counts what that folder shows. An account whose server
/// keeps no drafts folder has nowhere to show the number, and it is put nowhere else: on the inbox
/// it would count what that list does not hold.
fn carry_drafts(folders: &mut [FolderNode], written_here: u32) {
    for drafts in folders
        .iter_mut()
        .filter(|folder| folder.role == FolderRole::Drafts)
    {
        drafts.drafts_here = written_here;
    }
}

/// What an account is called, given the number the index knows it by.
pub fn account_named(
    profile: &SqlitePool,
    index_id: i64,
) -> Result<Option<String>, ReadModelError> {
    profile
        .get()
        .map_err(|_| ReadModelError::Unavailable)?
        .query_row(
            "SELECT id FROM mail_accounts WHERE index_id = ?1",
            [index_id],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            _ => Err(ReadModelError::Unavailable),
        })
}

/// Which account's file a number was minted in, read out of the number itself.
///
/// Asking the index would be a circle: the folder row says which account owns it, and that row is in
/// the very file the question is asked to choose. Each file seeds its counter at `account << 40`, so
/// the account is in the number and no file has to be opened to learn it.
///
/// Nothing means an untagged number: zero is nobody, because the counter that names accounts starts
/// at one.
pub fn account_that_minted(identifier: u64) -> Option<i64> {
    match (identifier >> 40) as i64 {
        0 => None,
        account => Some(account),
    }
}

/// The number the index knows an account by.
///
/// Kept in the profile and nowhere else, which is why this asks for the profile alone. It used to
/// be reached through a reader holding both databases, and every caller handed over an index that
/// was never touched — a dependency nobody had, carried until choosing which index became a
/// question worth asking.
pub fn index_id_of(profile: &SqlitePool, account_id: &str) -> Result<Option<i64>, ReadModelError> {
    profile
        .get()
        .map_err(|_| ReadModelError::Unavailable)?
        .query_row(
            "SELECT index_id FROM mail_accounts WHERE id = ?1",
            [account_id],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            _ => Err(ReadModelError::Unavailable),
        })
}

/// The index numbers of the accounts in one group, in the order they are drawn.
///
/// Numbers rather than identifiers, because what the list does with them is choose which mail
/// files to open. A group that is not here answers with nothing rather than with an error: it was
/// removed in another window, which is not something the person did wrong.
pub fn accounts_in_group(profile: &SqlitePool, group_id: &str) -> Result<Vec<i64>, ReadModelError> {
    let connection = profile.get().map_err(|_| ReadModelError::Unavailable)?;

    let mut statement = connection
        .prepare(
            "SELECT index_id FROM mail_accounts
                 WHERE group_id = ?1
                 ORDER BY position_in_group, created_at, id",
        )
        .map_err(|_| ReadModelError::Unavailable)?;

    let numbers = statement
        .query_map([group_id], |row| row.get(0))
        .map_err(|_| ReadModelError::Unavailable)?
        .collect::<Result<Vec<i64>, _>>()
        .map_err(|_| ReadModelError::Unavailable)?;

    Ok(numbers)
}

#[cfg(test)]
mod tests {

    #[test]
    fn a_folder_is_only_found_in_the_account_that_owns_it() {
        // What the old shape allowed: a folder resolved without an account, then used as the
        // destination of a move in a different one. `INBOX`, `Archive` and `Sent` collide across
        // accounts constantly, so the move succeeded against the wrong mailbox rather than
        // failing — and nobody saw it, because the message was filed somewhere plausible.
        let db = SeededIndex::with("folder-in-account", FolderId::new(1).expect("non-zero"), 0);

        // Numbered the way a file numbers its own: account two's counter starts at `2 << 40`, so a
        // folder of theirs carries a two in its top bits. A hand-picked nine would be a number no
        // file on this device could hand out, and the test would be about numbers the product never
        // produces.
        let mine = (2u64 << 40) | 9;
        db.add_folder_of_account(2, mine as i64, "Archive", FolderRole::Archive);

        let mailboxes = MailboxReadModel::new(db.profile.clone(), db.pool.clone());
        let theirs = FolderId::new(mine).expect("non-zero");

        // It exists, and it belongs to account 2.
        assert_eq!(
            mailboxes
                .account_number_of_folder(theirs)
                .expect("readable"),
            Some(2)
        );
        // Asked about inside account 1, it is not there — which is the answer that matters.
        assert_eq!(
            mailboxes.folder_purpose_in(1, theirs).expect("readable"),
            None
        );
        assert_eq!(
            mailboxes.folder_purpose_in(2, theirs).expect("readable"),
            Some(FolderRole::Archive)
        );
    }
    use super::*;
    use crate::infrastructure::fake_provider::{SeededIndex, SeededProfile};

    fn folder(id: u64) -> FolderId {
        FolderId::new(id).expect("non-zero")
    }

    /// A profile beside a seeded index: what the sidebar joins together.
    struct Workspace {
        profile: SeededProfile,
        index: SeededIndex,
    }

    impl Workspace {
        fn new(name: &str, messages: u32) -> Self {
            Self {
                profile: SeededProfile::new(name),
                index: SeededIndex::with(name, folder(1), messages),
            }
        }

        fn add_account(&self, id: &str, email: &str, index_id: i64) {
            self.profile.add_account(id, email, index_id);
        }

        fn fold_away(&self, id: &str) {
            self.profile.fold_away(id);
        }

        fn rename_folder(&self, id: i64, wire: &str) {
            self.index.name_folder(id, wire);
        }

        fn folder_name(&self, id: i64) -> String {
            self.index.folder_name(id)
        }

        fn tree(&self) -> Vec<AccountNode> {
            MailboxReadModel::new(self.profile.pool.clone(), self.index.pool.clone())
                .tree()
                .expect("the sidebar is readable")
        }

        fn on_the_icon(&self) -> u32 {
            MailboxReadModel::new(self.profile.pool.clone(), self.index.pool.clone())
                .unread_everywhere()
                .expect("the count is readable")
        }
    }

    #[test]
    fn the_sidebar_is_drawn_in_the_order_somebody_dragged_into() {
        // Nothing else reads this order, so a column written and never looked at would be a drag
        // that appears to work and is gone at the next start.
        use crate::ports::account_groups::AccountGroups;

        let workspace = Workspace::new("sidebar-order", 2);
        workspace.add_account("acc-1", "one@example.com", 1);
        workspace.add_account("acc-2", "two@example.com", 2);
        workspace.add_account("acc-3", "three@example.com", 3);

        let named = |tree: Vec<AccountNode>| {
            tree.into_iter()
                .map(|account| account.email)
                .collect::<Vec<_>>()
        };

        assert_eq!(
            named(workspace.tree()),
            vec![
                "one@example.com".to_owned(),
                "two@example.com".to_owned(),
                "three@example.com".to_owned()
            ],
            "the order they arrived in, until somebody says otherwise"
        );

        crate::infrastructure::sqlite::account_groups::SqliteAccountGroups::new(
            workspace.profile.pool.clone(),
        )
        .reorder(&["acc-3".to_owned(), "acc-1".to_owned(), "acc-2".to_owned()])
        .expect("written");

        assert_eq!(
            named(workspace.tree()),
            vec![
                "three@example.com".to_owned(),
                "one@example.com".to_owned(),
                "two@example.com".to_owned()
            ]
        );
    }

    #[test]
    fn the_icon_carries_the_sidebar_added_up() {
        // The number on the application's own icon and the numbers beside the accounts answer the
        // same question, so they are read from one place. Two readings would be an icon saying
        // four over a sidebar showing three, and there would be no way to tell which was right.
        let workspace = Workspace::new("icon-sum", 10);
        workspace.add_account("acc-1", "user@example.com", 1);

        let summed: u32 = workspace.tree().iter().map(|account| account.unread).sum();

        assert_eq!(workspace.on_the_icon(), summed);
        assert!(summed > 0, "the fixture has unread mail to count");
    }

    #[test]
    fn the_icon_leaves_out_what_the_sidebar_leaves_out() {
        // Drafts and the bin are not mail somebody has yet to read, and a number on an icon that
        // counted them would send a person looking for mail that is not there.
        let workspace = Workspace::new("icon-scope", 10);
        workspace.add_account("acc-1", "user@example.com", 1);
        let inbox_only = workspace.on_the_icon();

        workspace.index.add_folder(2, "Trash", FolderRole::Trash);
        workspace.index.fill_folder(folder(2), 100, 5);
        workspace.index.add_folder(3, "Drafts", FolderRole::Drafts);
        workspace.index.fill_folder(folder(3), 200, 4);

        assert_eq!(
            workspace.on_the_icon(),
            inbox_only,
            "nine more messages, and none of them unread mail"
        );
    }

    #[test]
    fn a_message_a_folder_holds_twice_is_counted_once_in_the_sidebar() {
        // The number beside a folder is how much mail is there, and the number beside the account
        // is the sum of those. Counting the rows that place a message rather than the messages
        // says there is more than a person can open — which is what "22 unread, 15 messages" was.
        //
        // The copy is made of a message that is *unread* on purpose: a read one would leave the
        // unread count alone whether it is counted properly or not, and the unread count is the
        // one people see.
        let workspace = Workspace::new("sidebar-two-copies", 10);
        workspace.add_account("acc-1", "user@example.com", 1);

        let before = workspace.tree();
        let folder_total = before[0].folders[0].total;
        let folder_unread = before[0].folders[0].unread;
        let account_unread = before[0].unread;
        assert!(
            folder_unread > 0,
            "the fixture left nothing unread to double"
        );

        let unread_one: i64 = workspace
            .index
            .pool
            .get()
            .expect("connection")
            .query_row(
                "SELECT p.message_id
                 FROM message_placements p
                 JOIN messages m ON m.id = p.message_id
                 WHERE p.folder_id = 1 AND (p.flags & 1) = 0
                 LIMIT 1",
                [],
                |row| row.get(0),
            )
            .expect("an unread message is there");

        workspace.index.a_second_copy_of(1, unread_one as u64);

        let after = workspace.tree();

        assert_eq!(after[0].folders[0].total, folder_total, "counted twice");
        assert_eq!(
            after[0].folders[0].unread, folder_unread,
            "counted twice as unread"
        );
        assert_eq!(
            after[0].unread, account_unread,
            "the account's number counted it twice"
        );
    }

    /// An account whose file will not open stays in the sidebar, and says why.
    ///
    /// It is in the profile, so it is a mailbox this person has. Leaving it out would say it is
    /// gone; leaving it in with no folders would say it is empty. Neither is true, and the two
    /// look identical on screen.
    #[test]
    fn an_account_whose_file_will_not_open_is_shown_and_marked() {
        let profile = SeededProfile::new("sidebar-one-quiet");
        profile.add_account("acc-one", "one@example.com", 1);
        profile.add_account("acc-seven", "seven@example.com", 7);

        let working = SeededIndex::with("quiet-file-one", folder(1), 3);
        let quiet = SeededIndex::with("quiet-file-seven", folder(1), 0);
        quiet.stops_answering();

        let sidebar = MailboxReadModel::spanning(
            profile.pool.clone(),
            vec![(1, working.pool.clone()), (7, quiet.pool.clone())],
        )
        .tree()
        .expect("the sidebar is readable");

        assert_eq!(
            sidebar.len(),
            2,
            "the account whose file is unreadable was left out"
        );

        let seven = sidebar
            .iter()
            .find(|node| node.account_id == "acc-seven")
            .expect("the account is still there");
        assert!(
            !seven.answered,
            "an account whose mail could not be read looks like one with no mail"
        );

        let one = sidebar
            .iter()
            .find(|node| node.account_id == "acc-one")
            .expect("the account that is fine");
        assert!(
            one.answered,
            "an account that answered was marked as silent"
        );
        assert!(!one.folders.is_empty());
    }

    /// The sidebar shows accounts kept in different files, not only the first file's.
    ///
    /// Written with two real files rather than one, because "it spans them" is untestable while
    /// there is only one to span: everything passes, and would go on passing if the second file
    /// were never opened. The numbers are 1 and 7, never 1 and 2, so a prefix or an off-by-one
    /// cannot pass by looking right.
    #[test]
    fn the_sidebar_shows_accounts_kept_in_different_files() {
        let profile = SeededProfile::new("sidebar-across-files");
        profile.add_account("acc-one", "one@example.com", 1);
        profile.add_account("acc-seven", "seven@example.com", 7);

        let first = SeededIndex::with("sidebar-file-one", folder(1), 3);
        let second = SeededIndex::with("sidebar-file-seven", folder(1), 0);
        second.add_folder_of_another_account(4, 7);
        second.fill_folder(folder(4), 500, 5);

        let sidebar = MailboxReadModel::spanning(
            profile.pool.clone(),
            vec![(1, first.pool.clone()), (7, second.pool.clone())],
        )
        .tree()
        .expect("the sidebar is readable");

        assert_eq!(
            sidebar.len(),
            2,
            "an account kept in another file is missing"
        );

        let seven = sidebar
            .iter()
            .find(|node| node.account_id == "acc-seven")
            .expect("the account kept in the second file");
        assert!(
            !seven.folders.is_empty(),
            "the second file's account came back without the folders that file holds"
        );

        let one = sidebar
            .iter()
            .find(|node| node.account_id == "acc-one")
            .expect("the account kept in the first file");
        assert!(
            !one.folders.is_empty(),
            "the first file's account lost its folders once a second file was read"
        );
    }

    #[test]
    fn a_device_with_no_accounts_has_an_empty_sidebar() {
        let workspace = Workspace::new("empty", 0);
        assert!(workspace.tree().is_empty());
    }

    #[test]
    fn an_account_carries_its_own_folders_and_counts() {
        let workspace = Workspace::new("one-account", 10);
        workspace.add_account("acc-1", "user@example.com", 1);
        workspace
            .index
            .add_folder(2, "Archive", FolderRole::Archive);

        let tree = workspace.tree();

        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].email, "user@example.com");
        assert_eq!(tree[0].folders.len(), 2);
        assert_eq!(tree[0].folders[0].role, FolderRole::Inbox);
        assert_eq!(tree[0].folders[0].total, 10);
        // The generator marks every fifth message unread.
        assert_eq!(tree[0].folders[0].unread, 2);
        assert_eq!(tree[0].unread, 2, "the account counts what its folders do");
    }

    #[test]
    fn folders_come_in_the_order_a_mail_client_shows_them() {
        let workspace = Workspace::new("order", 0);
        workspace.add_account("acc-1", "user@example.com", 1);
        workspace.index.add_folder(2, "Trash", FolderRole::Trash);
        workspace.index.add_folder(3, "Sent", FolderRole::Sent);
        workspace
            .index
            .add_folder(4, "Projects", FolderRole::Custom);
        workspace.index.add_folder(5, "Drafts", FolderRole::Drafts);

        let roles: Vec<FolderRole> = workspace.tree()[0]
            .folders
            .iter()
            .map(|folder| folder.role)
            .collect();

        assert_eq!(
            roles,
            vec![
                FolderRole::Inbox,
                FolderRole::Drafts,
                FolderRole::Sent,
                FolderRole::Trash,
                FolderRole::Custom,
            ]
        );
    }

    #[test]
    fn each_account_gets_its_own_folders_and_not_another_accounts() {
        let workspace = Workspace::new("two-accounts", 10);
        workspace.add_account("acc-1", "one@example.com", 1);
        workspace.add_account("acc-2", "two@example.com", 2);
        workspace.index.add_folder_of_another_account(9, 2);
        workspace.index.fill_folder(folder(9), 200, 3);

        let tree = workspace.tree();

        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].email, "one@example.com");
        assert_eq!(tree[0].folders.len(), 1);
        assert_eq!(tree[0].folders[0].folder_id, folder(1));
        assert_eq!(tree[1].email, "two@example.com");
        assert_eq!(tree[1].folders.len(), 1);
        assert_eq!(tree[1].folders[0].folder_id, folder(9));
        assert_eq!(tree[1].unread, 1, "three messages, every fifth unread");
    }

    #[test]
    fn accounts_come_in_the_order_they_were_added() {
        let workspace = Workspace::new("account-order", 0);
        workspace.add_account("acc-2", "second@example.com", 2);
        workspace.add_account("acc-1", "first@example.com", 1);

        let emails: Vec<String> = workspace
            .tree()
            .into_iter()
            .map(|account| account.email)
            .collect();

        assert_eq!(emails, vec!["first@example.com", "second@example.com"]);
    }

    #[test]
    fn an_account_that_was_folded_away_reads_as_folded_away() {
        let workspace = Workspace::new("collapsed", 0);
        workspace.add_account("acc-1", "user@example.com", 1);
        assert!(!workspace.tree()[0].collapsed, "accounts start unfolded");

        workspace.fold_away("acc-1");

        assert!(workspace.tree()[0].collapsed);
    }

    #[test]
    fn an_account_added_but_never_synced_is_listed_with_no_folders() {
        // Adding an account is one action and filling it is another; between them the sidebar
        // still has to show that the account is there.
        let workspace = Workspace::new("unsynced", 0);
        workspace.add_account("acc-2", "fresh@example.com", 7);

        let fresh = workspace
            .tree()
            .into_iter()
            .find(|account| account.email == "fresh@example.com")
            .expect("the account is listed");

        assert!(fresh.folders.is_empty());
        assert_eq!(fresh.unread, 0);
    }

    #[test]
    fn the_badge_counts_the_same_mail_the_account_row_shows() {
        // A badge over a list has one job: to describe that list. Counting every folder puts
        // drafts and the bin into a number sitting above the inbox.
        let workspace = Workspace::new("badge-scope", 10);
        workspace.add_account("acc-1", "user@example.com", 1);
        workspace.index.add_folder(2, "Trash", FolderRole::Trash);
        workspace.index.fill_folder(folder(2), 100, 5);

        let tree = workspace.tree();
        let unread_of = |role: FolderRole| {
            tree[0]
                .folders
                .iter()
                .find(|listed| listed.role == role)
                .unwrap_or_else(|| panic!("no folder for {role:?}"))
                .unread
        };

        assert!(
            unread_of(FolderRole::Trash) > 0,
            "the fixture needs unread mail outside the inbox, or it proves nothing"
        );
        assert_eq!(
            tree[0].unread,
            unread_of(FolderRole::Inbox),
            "the account's number is the inbox's number, not every folder's"
        );
        assert_ne!(
            tree[0].unread,
            unread_of(FolderRole::Inbox) + unread_of(FolderRole::Trash),
            "and it is plainly not the sum of them"
        );
    }

    #[test]
    fn one_message_in_two_folders_is_one_unread_for_the_account() {
        let workspace = Workspace::new("labels", 5);
        workspace.add_account("acc-1", "user@example.com", 1);
        workspace
            .index
            .add_folder(2, "Archive", FolderRole::Archive);
        // Message 5 is the unread one the generator makes.
        workspace.index.place_existing_message(2, 500, 5);

        let tree = workspace.tree();

        assert_eq!(
            tree[0].unread, 1,
            "the account counts messages, not the folders they are filed in"
        );
        assert_eq!(
            tree[0].folders[1].unread, 1,
            "the folder still counts what it holds"
        );
    }

    #[test]
    fn an_account_not_on_this_device_resolves_to_nothing_rather_than_failing() {
        let workspace = Workspace::new("unknown", 0);
        let model =
            MailboxReadModel::new(workspace.profile.pool.clone(), workspace.index.pool.clone());

        assert_eq!(
            model.index_id_of("acc-gone").expect("lookup succeeds"),
            None
        );
    }

    #[test]
    fn an_account_resolves_to_the_number_the_index_knows_it_by() {
        let workspace = Workspace::new("resolve", 0);
        workspace.add_account("acc-1", "user@example.com", 4);
        let model =
            MailboxReadModel::new(workspace.profile.pool.clone(), workspace.index.pool.clone());

        assert_eq!(
            model.index_id_of("acc-1").expect("lookup succeeds"),
            Some(4)
        );
    }

    #[test]
    fn a_folder_named_in_another_alphabet_is_shown_in_that_alphabet() {
        // What the wire carries is not text anybody wrote. Shown as it arrives, a folder named in
        // Greek reads as line noise.
        let workspace = Workspace::new("wire-names", 0);
        workspace.add_account("acc-1", "zeta@example.com", 1);
        workspace.rename_folder(1, "&A5YDrgPEA7E-");

        let shown: Vec<String> = workspace
            .tree()
            .iter()
            .flat_map(|account| account.folders.iter().map(|folder| folder.name.clone()))
            .collect();

        assert!(
            shown.contains(&"Ζήτα".to_owned()),
            "the sidebar shows what the wire carries rather than what it means: {shown:?}"
        );
    }

    #[test]
    fn the_name_the_server_answers_to_is_left_in_the_database() {
        // The one thing that must not change. Decoding what is sent addresses a mailbox nobody
        // has, and the failure is a folder that cannot be opened.
        let workspace = Workspace::new("wire-kept", 0);
        workspace.add_account("acc-1", "zeta@example.com", 1);
        let wire = "&BB4EQgQ,BEAEMAQyBDsENQQ9BD0ESwQ1-";
        workspace.rename_folder(1, wire);

        let _ = workspace.tree();

        assert_eq!(workspace.folder_name(1), wire);
    }

    /// What an account's drafts folder counts: what the server holds, and what is kept here.
    fn drafts_of(tree: &[AccountNode], account_id: &str) -> (u32, u32) {
        let drafts = tree
            .iter()
            .find(|account| account.account_id == account_id)
            .expect("the account is listed")
            .folders
            .iter()
            .find(|folder| folder.role == FolderRole::Drafts)
            .expect("the account has a drafts folder");
        (drafts.total, drafts.drafts_here)
    }

    #[test]
    fn a_draft_on_the_server_is_counted_as_the_servers() {
        // Saved by another client or by webmail, and brought in by an ordinary sync. It is in the
        // folder, so the folder counts it -- and not as something written on this device.
        let workspace = Workspace::new("drafts-server-only", 0);
        workspace.add_account("acc-1", "zeta@example.com", 1);
        workspace.index.add_folder(2, "Drafts", FolderRole::Drafts);
        workspace.index.fill_folder(folder(2), 100, 1);

        assert_eq!(drafts_of(&workspace.tree(), "acc-1"), (1, 0));
    }

    #[test]
    fn a_draft_written_here_is_counted_on_the_drafts_folder() {
        // Kept in the profile and nowhere in the index, so a count taken from the index alone
        // would say nothing while the folder shows it.
        let workspace = Workspace::new("drafts-here-only", 0);
        workspace.add_account("acc-1", "zeta@example.com", 1);
        workspace.index.add_folder(2, "Drafts", FolderRole::Drafts);
        workspace.profile.kept_draft("acc-1");

        assert_eq!(drafts_of(&workspace.tree(), "acc-1"), (0, 1));
    }

    #[test]
    fn drafts_in_both_places_are_counted_in_both() {
        let workspace = Workspace::new("drafts-both", 0);
        workspace.add_account("acc-1", "zeta@example.com", 1);
        workspace.index.add_folder(2, "Drafts", FolderRole::Drafts);
        workspace.index.fill_folder(folder(2), 100, 1);
        workspace.profile.kept_draft("acc-1");

        assert_eq!(drafts_of(&workspace.tree(), "acc-1"), (1, 1));
    }

    #[test]
    fn each_account_counts_only_the_drafts_written_from_it() {
        // Two and three rather than one and one, so a count handed to the wrong account cannot
        // pass by looking the same.
        let workspace = Workspace::new("drafts-per-account", 0);
        workspace.add_account("acc-1", "zeta@example.com", 1);
        workspace.add_account("acc-2", "kappa@example.net", 2);
        workspace.index.add_folder(2, "Drafts", FolderRole::Drafts);
        workspace
            .index
            .add_folder_of_account(2, 3, "Drafts", FolderRole::Drafts);
        workspace.profile.kept_draft("acc-1");
        workspace.profile.kept_draft("acc-1");
        for _ in 0..3 {
            workspace.profile.kept_draft("acc-2");
        }

        let tree = workspace.tree();

        assert_eq!(drafts_of(&tree, "acc-1"), (0, 2));
        assert_eq!(drafts_of(&tree, "acc-2"), (0, 3));
    }

    #[test]
    fn a_draft_written_here_is_counted_on_no_other_folder() {
        let workspace = Workspace::new("drafts-nowhere-else", 4);
        workspace.add_account("acc-1", "zeta@example.com", 1);
        workspace.index.add_folder(2, "Drafts", FolderRole::Drafts);
        workspace.index.add_folder(3, "Sent", FolderRole::Sent);
        workspace.profile.kept_draft("acc-1");

        let elsewhere: Vec<(FolderRole, u32)> = workspace.tree()[0]
            .folders
            .iter()
            .filter(|folder| folder.role != FolderRole::Drafts)
            .map(|folder| (folder.role, folder.drafts_here))
            .collect();

        assert_eq!(
            elsewhere,
            vec![(FolderRole::Inbox, 0), (FolderRole::Sent, 0)]
        );
    }

    #[test]
    fn every_drafts_folder_counts_what_it_shows() {
        // A server may call two mailboxes its drafts, and opening either shows the drafts kept
        // here. Counted on one only, the other would show drafts beside a number that says none.
        let workspace = Workspace::new("drafts-two-folders", 0);
        workspace.add_account("acc-1", "zeta@example.com", 1);
        workspace.index.add_folder(5, "Drafts", FolderRole::Drafts);
        workspace
            .index
            .add_folder(2, "Brouillons", FolderRole::Drafts);
        workspace.profile.kept_draft("acc-1");

        let carried: Vec<(FolderId, u32)> = workspace.tree()[0]
            .folders
            .iter()
            .filter(|folder| folder.role == FolderRole::Drafts)
            .map(|folder| (folder.folder_id, folder.drafts_here))
            .collect();

        assert_eq!(carried, vec![(folder(2), 1), (folder(5), 1)]);
    }

    #[test]
    fn an_account_whose_server_keeps_no_drafts_folder_puts_the_count_nowhere() {
        // There is no folder for it to stand beside, and putting it on the inbox would be a number
        // over a list that does not hold what it counts.
        let workspace = Workspace::new("drafts-no-folder", 2);
        workspace.add_account("acc-1", "zeta@example.com", 1);
        workspace.profile.kept_draft("acc-1");

        let tree = workspace.tree();

        assert!(tree[0].folders.iter().all(|folder| folder.drafts_here == 0));
    }

    /// The sidebar read over the index and the queue beside it, the way the window reads it.
    fn laid_over(db: &SeededIndex) -> (Vec<AccountNode>, u32) {
        let mailboxes = MailboxReadModel::new(db.profile.clone(), db.pool.clone());
        (
            mailboxes.tree().expect("the sidebar is readable"),
            mailboxes.unread_everywhere().expect("the icon is readable"),
        )
    }

    fn unread_in(tree: &[AccountNode], folder_id: u64) -> (u32, u32) {
        tree[0]
            .folders
            .iter()
            .find(|listed| listed.folder_id.get() == folder_id)
            .map(|listed| (listed.total, listed.unread))
            .unwrap_or_else(|| panic!("no folder {folder_id}"))
    }

    #[test]
    fn a_message_opened_is_counted_read_everywhere_before_any_server_hears() {
        // What the person did is what every number on the screen says, from the moment it is
        // written down. Counted from the index alone, the folder, the account, every inbox
        // together and the icon would go on saying "1" until a server had heard, over a message
        // that had visibly been read.
        let db = SeededIndex::with("counts-read-at-once", folder(1), 10);
        let (before, icon_before) = laid_over(&db);

        // Message 5 is one the generator leaves unread.
        db.asked_to_mark(5, 1, true);
        let (after, icon_after) = laid_over(&db);

        assert_eq!(unread_in(&after, 1).1, unread_in(&before, 1).1 - 1);
        assert_eq!(unread_in(&after, 1).0, unread_in(&before, 1).0);
        assert_eq!(after[0].unread, before[0].unread - 1);
        assert_eq!(icon_after, icon_before - 1);
    }

    #[test]
    fn the_sidebar_says_how_far_the_queue_had_got_when_it_was_read() {
        // The window lays a change over counts read before the change was written and over none
        // read after, and this is how it tells which: the write answers with where it stands, and
        // a read of the sidebar with how far the queue had got. Ordered by the window's own clock
        // instead, a read already on its way when the change was written holds it, and has it
        // laid over again.
        use crate::infrastructure::sqlite::op_log::SqliteOperationLog;
        use crate::ports::op_log::OperationLog;
        use lyonoyl_domain::message::Flags;
        use lyonoyl_domain::operation::{MailOperation, OperationKind, OperationTarget};

        let db = SeededIndex::with("counts-as-of", folder(1), 10);
        let mailboxes = MailboxReadModel::new(db.profile.clone(), db.pool.clone());
        let (before, through_before) = mailboxes.tree_as_of().expect("readable");

        // Message 5 is one the generator leaves unread.
        let asked = SqliteOperationLog::new(db.profile.clone())
            .record(
                "acc-1",
                &[],
                MailOperation::asked(
                    OperationTarget::new(MessageId::new(5).expect("non-zero"), folder(1)),
                    OperationKind::SetFlags {
                        add: Flags::SEEN,
                        remove: Flags::NONE,
                    },
                ),
            )
            .expect("it is written");
        let (after, through_after) = mailboxes.tree_as_of().expect("readable");

        assert!(
            asked > through_before,
            "a read from before the write holds it"
        );
        assert_eq!(through_after, asked, "a read from after the write lacks it");
        assert_eq!(unread_in(&after, 1).1, unread_in(&before, 1).1 - 1);
    }

    #[test]
    fn a_message_marked_unread_is_counted_unread_at_once() {
        let db = SeededIndex::with("counts-unread-at-once", folder(1), 10);
        let (before, icon_before) = laid_over(&db);

        // Message 1 is one the generator leaves read.
        db.asked_to_mark(1, 1, false);
        let (after, icon_after) = laid_over(&db);

        assert_eq!(unread_in(&after, 1).1, unread_in(&before, 1).1 + 1);
        assert_eq!(after[0].unread, before[0].unread + 1);
        assert_eq!(icon_after, icon_before + 1);
    }

    #[test]
    fn a_message_filed_away_is_counted_where_it_went_at_once() {
        let db = SeededIndex::with("counts-filed", folder(1), 10);
        db.add_folder(2, "Archive", FolderRole::Archive);
        let (before, _) = laid_over(&db);

        db.asked_to_file(5, 1, 2);
        let (after, icon) = laid_over(&db);

        assert_eq!(
            unread_in(&after, 1),
            (unread_in(&before, 1).0 - 1, unread_in(&before, 1).1 - 1)
        );
        assert_eq!(unread_in(&after, 2), (1, 1));
        assert_eq!(after[0].unread, before[0].unread - 1, "out of the inbox");
        assert_eq!(icon, after[0].unread);
    }

    #[test]
    fn a_message_thrown_away_is_counted_in_the_trash_and_one_thrown_out_nowhere() {
        let db = SeededIndex::with("counts-thrown", folder(1), 10);
        db.add_folder(4, "Trash", FolderRole::Trash);
        let (before, _) = laid_over(&db);

        db.asked_to_throw_away(5, 1);
        db.asked_to_throw_away_for_good(10, 1);
        let (after, _) = laid_over(&db);

        assert_eq!(unread_in(&after, 1).0, unread_in(&before, 1).0 - 2);
        assert_eq!(unread_in(&after, 4), (1, 1), "the one that can be had back");
    }

    #[test]
    fn what_stopped_moves_no_count() {
        let db = SeededIndex::with("counts-stopped", folder(1), 10);
        let (before, icon_before) = laid_over(&db);

        db.stopped_asking(5, 1, "messageIsNotThere");
        let (after, icon_after) = laid_over(&db);

        assert_eq!(unread_in(&after, 1), unread_in(&before, 1));
        assert_eq!(icon_after, icon_before);
    }

    #[test]
    fn a_move_the_server_carried_out_is_counted_as_done_until_it_is_read_back() {
        // Between the push and the pass that reads it back, the queue says "done" and the index
        // still has the message where it was. Counted from the index, the inbox would go back up
        // for that second, and down again when the pass came.
        let db = SeededIndex::with("counts-owed", folder(1), 10);
        db.add_folder(2, "Archive", FolderRole::Archive);
        let (before, _) = laid_over(&db);

        db.asked_to_file(5, 1, 2);
        db.carried_out_and_not_read_back();
        let (after, _) = laid_over(&db);

        assert_eq!(unread_in(&after, 1).1, unread_in(&before, 1).1 - 1);
        assert_eq!(unread_in(&after, 2), (1, 1));
    }

    #[test]
    fn a_message_asked_back_is_counted_where_it_was() {
        let db = SeededIndex::with("counts-back", folder(1), 10);
        db.add_folder(2, "Archive", FolderRole::Archive);
        let (before, icon_before) = laid_over(&db);

        db.asked_to_file(5, 1, 2);
        db.carried_out_and_not_read_back();
        db.asked_back(5, 2, 1);
        let (after, icon_after) = laid_over(&db);

        assert_eq!(unread_in(&after, 1), unread_in(&before, 1));
        assert_eq!(unread_in(&after, 2), (0, 0));
        assert_eq!(icon_after, icon_before);
    }

    #[test]
    fn a_flag_the_server_confirmed_is_counted_once() {
        // Written onto the index when the server said yes, and no longer in the queue as waiting:
        // counted from both it would be two fewer for one message.
        let db = SeededIndex::with("counts-confirmed", folder(1), 10);
        let (before, _) = laid_over(&db);

        db.asked_to_mark(5, 1, true);
        db.everything_reached_the_server();
        db.every_copy_read(1, 5);
        let (after, _) = laid_over(&db);

        assert_eq!(unread_in(&after, 1).1, unread_in(&before, 1).1 - 1);
    }

    /// Writes down that a person asked for a message to be marked read, the way the window does,
    /// and answers with where that stands among everything asked.
    fn asked_to_read(db: &SeededIndex, message: u64) -> u64 {
        use crate::infrastructure::sqlite::op_log::SqliteOperationLog;
        use crate::ports::op_log::OperationLog;
        use lyonoyl_domain::message::Flags;
        use lyonoyl_domain::operation::{MailOperation, OperationKind, OperationTarget};

        SqliteOperationLog::new(db.profile.clone())
            .record(
                "acc-1",
                &[],
                MailOperation::asked(
                    OperationTarget::new(MessageId::new(message).expect("non-zero"), folder(1)),
                    OperationKind::SetFlags {
                        add: Flags::SEEN,
                        remove: Flags::NONE,
                    },
                ),
            )
            .expect("it is written")
    }

    /// The same for filing a message away, carried out on the server and not read back yet.
    fn filed_and_carried_out(db: &SeededIndex, message: u64, into: u64) -> u64 {
        use crate::infrastructure::sqlite::op_log::SqliteOperationLog;
        use crate::ports::op_log::OperationLog;
        use lyonoyl_domain::operation::{MailOperation, OperationKind, OperationTarget};

        let asked = SqliteOperationLog::new(db.profile.clone())
            .record(
                "acc-1",
                &[],
                MailOperation::asked(
                    OperationTarget::new(MessageId::new(message).expect("non-zero"), folder(1)),
                    OperationKind::MoveTo {
                        folder: folder(into),
                    },
                ),
            )
            .expect("it is written");
        db.carried_out_and_not_read_back();
        asked
    }

    /// What a push does once the server has said yes to a flag: the index first, the queue after.
    fn a_flag_push_lands(db: &SeededIndex, message: u64) {
        db.every_copy_read(1, message);
        db.everything_reached_the_server();
    }

    /// What a pass does once it has read a move back: the index first, the queue after.
    fn a_move_is_read_back(db: &SeededIndex, message: u64, into: i64) {
        db.only_in(message, into);
        db.read_back_everything();
    }

    /// Runs `read` with `lands` done after its `at`-th statement, answering with what it read, or
    /// with nothing once the read has fewer statements than that.
    fn landing_after<T>(
        at: usize,
        read: impl FnOnce(&mut dyn FnMut()) -> T,
        lands: impl FnOnce(),
    ) -> Option<T> {
        let mut lands = Some(lands);
        let mut statement = 0;
        let read = read(&mut || {
            if statement == at {
                if let Some(lands) = lands.take() {
                    lands();
                }
            }
            statement += 1;
        });
        lands.is_none().then_some(read)
    }

    #[test]
    fn a_flag_confirmed_in_the_middle_of_a_read_is_counted_once_and_said_to_be_in() {
        // A push writes what the server confirmed onto the index and then writes the instruction
        // off. However that lands against the statements of a sidebar already on its way, the
        // counts it answers hold the change exactly once, and the number it answers says they do:
        // the window drops its own copy of every change that number covers, and a flag push
        // announces nothing that would read the sidebar again. A count left without the change is
        // a count wrong until something unrelated happens.
        for at in 0.. {
            let db = SeededIndex::with(&format!("counts-mid-flag-{at}"), folder(1), 10);
            let mailboxes = MailboxReadModel::new(db.profile.clone(), db.pool.clone());
            let (before, icon_before) = laid_over(&db);

            // Message 5 is one the generator leaves unread.
            let asked = asked_to_read(&db, 5);
            let Some(read) = landing_after(
                at,
                |between| mailboxes.tree_read(between),
                || a_flag_push_lands(&db, 5),
            ) else {
                break;
            };
            let (after, through) = read.expect("the sidebar is readable");

            assert!(
                through >= asked,
                "a read begun after the change says it is in"
            );
            assert_eq!(
                unread_in(&after, 1).1,
                unread_in(&before, 1).1 - 1,
                "the inbox, with the push landing after statement {at}"
            );
            assert_eq!(
                after[0].unread,
                before[0].unread - 1,
                "the account, with the push landing after statement {at}"
            );

            let db = SeededIndex::with(&format!("icon-mid-flag-{at}"), folder(1), 10);
            let mailboxes = MailboxReadModel::new(db.profile.clone(), db.pool.clone());
            asked_to_read(&db, 5);
            if let Some(icon) = landing_after(
                at,
                |between| mailboxes.unread_read(between),
                || a_flag_push_lands(&db, 5),
            ) {
                assert_eq!(
                    icon.expect("the icon is readable"),
                    icon_before - 1,
                    "the icon, with the push landing after statement {at}"
                );
            }
        }
    }

    #[test]
    fn a_move_read_back_in_the_middle_of_a_read_is_counted_once() {
        // The same order holds for a move: a pass writes where the message now is onto the index,
        // and only then stops laying the move over it.
        for at in 0.. {
            let db = SeededIndex::with(&format!("counts-mid-move-{at}"), folder(1), 10);
            db.add_folder(2, "Archive", FolderRole::Archive);
            let mailboxes = MailboxReadModel::new(db.profile.clone(), db.pool.clone());
            let (before, _) = laid_over(&db);

            let asked = filed_and_carried_out(&db, 5, 2);
            let Some(read) = landing_after(
                at,
                |between| mailboxes.tree_read(between),
                || a_move_is_read_back(&db, 5, 2),
            ) else {
                break;
            };
            let (after, through) = read.expect("the sidebar is readable");

            assert!(through >= asked);
            assert_eq!(
                unread_in(&after, 1),
                (unread_in(&before, 1).0 - 1, unread_in(&before, 1).1 - 1),
                "the inbox, with the read back landing after statement {at}"
            );
            assert_eq!(
                unread_in(&after, 2),
                (1, 1),
                "the archive, with the read back landing after statement {at}"
            );
            assert_eq!(after[0].unread, before[0].unread - 1);
        }
    }
}
