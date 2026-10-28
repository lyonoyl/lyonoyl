//! The mail of a scene, held the way the accounts' servers would hold it, and asked the same way.
//!
//! Each message is written whole, by the library outgoing mail is written with, and described by
//! the code that describes a message a server hands over: what the list shows of it is read out of
//! the message, never copied from the scene beside it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use mail_builder::headers::address::Address;
use mail_builder::MessageBuilder;

use super::{sender_of, Account, Message, Scene, SceneError, MAILBOXES};
use crate::infrastructure::imap::described_whole;
use crate::ports::credential::Credential;
use crate::ports::mail_provider::{
    DiscoveredMailbox, MailFetch, MailProviderError, MailSync, MailWatch, MailboxDiscovery,
    ProviderFactory, SyncIntent, SyncRequest, WatchEnded, Watched, Watermark,
};
use lyonoyl_domain::account::MailAccount;
use lyonoyl_domain::folder::FolderRole;
use lyonoyl_domain::message::{FlagScope, Flags, Uid, UidValidity};
use lyonoyl_domain::sync::{
    ArrivedMessage, FolderDelta, MessageEnvelope, SyncBatch, SyncCursor, SyncScope, VanishedReport,
};

/// The generation every mailbox here counts in. Nothing is ever renumbered.
const GENERATION: u32 = 1;

/// Where a pass has read to. One place for every mailbox, because nothing arrives once the scene is
/// written.
const READ_TO: &str = "scene";

/// The mail of every account in a scene, written at one moment.
///
/// Cloned rather than written again: a message read whole after the pass has to be the message the
/// pass described, to the byte, and the same scene written a minute later is dated a minute later.
#[derive(Clone, Default)]
pub struct Servers {
    /// Each account's mailboxes, by its address in lower case.
    accounts: Arc<HashMap<String, Arc<[Mailbox]>>>,
}

struct Mailbox {
    name: &'static str,
    role: FolderRole,
    /// Oldest first, numbered from one, the order a server numbers mail in as it arrives.
    messages: Vec<Held>,
}

struct Held {
    uid: Uid,
    flags: Flags,
    raw: Vec<u8>,
}

impl Servers {
    /// The scene's mail, each message dated back from `now`, in Unix seconds.
    pub fn holding(scene: &Scene, now: i64) -> Result<Self, SceneError> {
        let mut accounts = HashMap::new();
        for (at, account) in scene.accounts.iter().enumerate() {
            let place = format!("accounts[{at}]");
            accounts.insert(
                account.address.trim().to_ascii_lowercase(),
                Arc::from(mailboxes_of(account, &place, at, now)?),
            );
        }

        Ok(Self {
            accounts: Arc::new(accounts),
        })
    }

    /// A message as it was written, by the name it carries.
    #[cfg(test)]
    pub(crate) fn written_named(&self, name: &str) -> Option<Vec<u8>> {
        let carried = format!("<{name}>");
        self.accounts
            .values()
            .flat_map(|mailboxes| mailboxes.iter())
            .flat_map(|mailbox| mailbox.messages.iter())
            .find(|held| {
                String::from_utf8_lossy(&held.raw)
                    .lines()
                    .any(|line| line.trim_start_matches("Message-ID:").trim() == carried)
            })
            .map(|held| held.raw.clone())
    }

    /// The server of `account`, or no connection for an account the scene does not have: a build
    /// that plays a scene reads mail from nowhere else.
    fn of(&self, account: &MailAccount) -> Result<AccountMail, MailProviderError> {
        self.accounts
            .get(&account.summary().email().trim().to_ascii_lowercase())
            .map(|mailboxes| AccountMail {
                mailboxes: Arc::clone(mailboxes),
            })
            .ok_or(MailProviderError::Offline)
    }
}

impl ProviderFactory for Servers {
    fn discovery(
        &self,
        account: &MailAccount,
        _credential: &Credential,
    ) -> Result<Box<dyn MailboxDiscovery>, MailProviderError> {
        Ok(Box::new(self.of(account)?))
    }

    fn sync(
        &self,
        account: &MailAccount,
        _credential: &Credential,
    ) -> Result<Box<dyn MailSync>, MailProviderError> {
        Ok(Box::new(self.of(account)?))
    }

    fn watch(
        &self,
        _account: &MailAccount,
        _credential: &Credential,
    ) -> Result<Box<dyn MailWatch>, MailProviderError> {
        Ok(Box::new(SaysNothing))
    }

    fn reading(
        &self,
        account: &MailAccount,
        _credential: &Credential,
    ) -> Result<Box<dyn MailFetch>, MailProviderError> {
        Ok(Box::new(self.of(account)?))
    }
}

/// One account's mailboxes, asked the way a pass asks its server.
struct AccountMail {
    mailboxes: Arc<[Mailbox]>,
}

impl AccountMail {
    fn mailbox(&self, name: &str) -> Result<&Mailbox, MailProviderError> {
        self.mailboxes
            .iter()
            .find(|mailbox| mailbox.name == name)
            .ok_or(MailProviderError::Protocol)
    }
}

impl MailboxDiscovery for AccountMail {
    fn list_mailboxes(&self) -> Result<Vec<DiscoveredMailbox>, MailProviderError> {
        self.mailboxes
            .iter()
            .map(|mailbox| {
                Ok(DiscoveredMailbox {
                    name: mailbox.name.to_owned(),
                    role: mailbox.role,
                    uid_validity: Some(generation()?),
                    uid_next: Uid::new(mailbox.messages.len() as u32 + 1).ok(),
                })
            })
            .collect()
    }
}

impl MailSync for AccountMail {
    /// As an IMAP server keeps them: on each copy of a message rather than on the message.
    fn flag_scope(&self) -> FlagScope {
        FlagScope::Placement
    }

    /// The first reading of a mailbox finds everything in it, and every reading after it finds
    /// nothing new. Nothing is said about anything gone, because nothing goes.
    fn fetch(&self, request: &SyncRequest) -> Result<SyncBatch, MailProviderError> {
        let mailbox = self.mailbox(&request.mailbox)?;

        let (arrived, covered_from) = match &request.intent {
            SyncIntent::Forward => {
                let arrived = match request.cursor {
                    None => mailbox.messages.iter().map(Held::arrival).collect(),
                    Some(_) => Vec::new(),
                };
                (arrived, Uid::new(1).ok())
            }
            SyncIntent::Older(range) => (
                mailbox
                    .messages
                    .iter()
                    .filter(|held| range.contains(&held.uid))
                    .map(Held::arrival)
                    .collect(),
                Some(*range.start()),
            ),
        };

        Ok(SyncBatch {
            scope: SyncScope::Mailbox,
            cursor_before: request.cursor.clone(),
            cursor_after: SyncCursor::new(READ_TO),
            per_mailbox: vec![FolderDelta {
                folder_id: request.folder_id,
                uid_validity: generation()?,
                covered_from,
                stopped_short: false,
                arrived,
                flag_updates: Vec::new(),
                vanished: VanishedReport::Unknown,
            }],
        })
    }
}

impl MailFetch for AccountMail {
    fn whole_message(
        &self,
        mailbox: &str,
        uid_validity: UidValidity,
        uid: Uid,
    ) -> Result<Vec<u8>, MailProviderError> {
        if uid_validity != generation()? {
            return Err(MailProviderError::Protocol);
        }

        self.mailbox(mailbox)?
            .messages
            .iter()
            .find(|held| held.uid == uid)
            .map(|held| held.raw.clone())
            .ok_or(MailProviderError::Protocol)
    }
}

/// A server that tells nobody anything until it is asked.
struct SaysNothing;

impl MailWatch for SaysNothing {
    fn watch(
        &self,
        _mailbox: &str,
        _since: Option<&Watermark>,
        _at_most: Duration,
    ) -> Result<WatchEnded, MailProviderError> {
        Ok(Watched::CannotPush.into())
    }
}

impl Held {
    /// The message as a pass describes one arriving.
    fn arrival(&self) -> ArrivedMessage {
        ArrivedMessage {
            uid: self.uid,
            modseq: None,
            envelope: MessageEnvelope {
                rfc822_size: u32::try_from(self.raw.len()).ok(),
                flags: self.flags,
                ..described_whole(&self.raw)
            },
        }
    }
}

fn generation() -> Result<UidValidity, MailProviderError> {
    UidValidity::new(GENERATION).map_err(|_| MailProviderError::Protocol)
}

/// One account's mail, written and put in its mailboxes.
fn mailboxes_of(
    account: &Account,
    place: &str,
    position: usize,
    now: i64,
) -> Result<Vec<Mailbox>, SceneError> {
    let mut mailboxes: Vec<Mailbox> = MAILBOXES
        .iter()
        .map(|&(name, role)| Mailbox {
            name,
            role,
            messages: Vec::new(),
        })
        .collect();

    // What each message is called, so an answer can name what it answers.
    let names: Vec<String> = (0..account.mail.len())
        .map(|at| name_of(position, at))
        .collect();
    let named: HashMap<&str, usize> = account
        .mail
        .iter()
        .enumerate()
        .filter_map(|(at, message)| Some((message.id.as_deref()?, at)))
        .collect();

    let mut arriving = Vec::with_capacity(account.mail.len());
    for (at, message) in account.mail.iter().enumerate() {
        let place = format!("{place}.mail[{at}]");
        arriving.push((message.seconds_ago(&place)?, at, place));
    }
    // Oldest first.
    arriving.sort_by_key(|&(ago, at, _)| (std::cmp::Reverse(ago), at));

    for (ago, at, place) in arriving {
        let message = &account.mail[at];
        let chain = chain_of(account, at, &names, &named);
        let written_at = now.saturating_sub(i64::try_from(ago).unwrap_or(i64::MAX));
        let raw = written(account, message, &names[at], &chain, written_at, &place)?;

        let name = message.mailbox(&place)?;
        let Some(mailbox) = mailboxes.iter_mut().find(|mailbox| mailbox.name == name) else {
            return Err(SceneError(format!(
                "{place}.folder: no mailbox is called {name}"
            )));
        };
        let uid = Uid::new(mailbox.messages.len() as u32 + 1)
            .map_err(|_| SceneError(format!("{place}: no number is left for it")))?;

        let mut flags = match message.unread {
            true => Flags::NONE,
            false => Flags::SEEN,
        };
        if message.flagged {
            flags = flags.with(Flags::FLAGGED);
        }

        mailbox.messages.push(Held { uid, flags, raw });
    }

    Ok(mailboxes)
}

/// The name a scene's message carries, by the places of its account and of it in the scene. Under
/// `.invalid`, which nothing answers for.
pub fn name_of(account: usize, message: usize) -> String {
    format!("{}.{}@scene.invalid", account + 1, message + 1)
}

/// The names of what a message answers, oldest first: the conversation it belongs to.
///
/// Each answer arrives after what it answers, which the scene was checked for, so the walk ends; it
/// is bounded by the account's mail as well, so a scene nobody checked cannot keep it walking.
fn chain_of(
    account: &Account,
    at: usize,
    names: &[String],
    named: &HashMap<&str, usize>,
) -> Vec<String> {
    let mut chain = Vec::new();
    let mut next = account.mail[at].answers.as_deref();

    while let Some(answered) = next.and_then(|id| named.get(id).copied()) {
        if chain.len() == account.mail.len() {
            break;
        }
        chain.push(names[answered].clone());
        next = account.mail[answered].answers.as_deref();
    }

    chain.reverse();
    chain
}

/// One message as it would arrive: headers, then the text.
fn written(
    account: &Account,
    message: &Message,
    name: &str,
    chain: &[String],
    at: i64,
    place: &str,
) -> Result<Vec<u8>, SceneError> {
    let (from_name, from) = sender_of(&message.from, &format!("{place}.from"))?;
    let (to_name, to) = match &message.to {
        Some(to) => sender_of(to, &format!("{place}.to"))?,
        None => (account.name.trim(), account.address.trim()),
    };

    let mut builder = MessageBuilder::new()
        .date(at)
        .message_id(name)
        .from(called(from_name, from))
        .to(called(to_name, to))
        .subject(message.subject.as_str());

    if let Some(answered) = chain.last() {
        builder = builder
            .in_reply_to(answered.as_str())
            .references(chain.to_vec());
    }

    builder = builder.text_body(message.text.as_str());
    if let Some(html) = &message.written {
        builder = builder.html_body(html.text.as_str());
        for picture in &html.pictures {
            builder = builder.inline(
                picture.content_type,
                picture.content_id.as_str(),
                picture.bytes.as_slice(),
            );
        }
    }

    builder
        .write_to_vec()
        .map_err(|why| SceneError(format!("{place}: {why}")))
}

/// An address with the name it goes by, or the address alone where it has none.
fn called<'a>(name: &'a str, address: &'a str) -> Address<'a> {
    match name.is_empty() {
        true => Address::from(address),
        false => Address::from((name, address)),
    }
}
