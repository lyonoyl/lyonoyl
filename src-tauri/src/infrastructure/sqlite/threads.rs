//! Putting messages into conversations, as they are stored.
//!
//! The rule for what joins what is in the domain ([`lyonoyl_domain::thread`]); everything here is
//! the part that needs a database. Mail does not arrive in the order it was written — a reply is
//! often stored before the message it answers — so a conversation cannot be found by looking for
//! messages that name this one. Instead every name a message mentions is written down against the
//! conversation it belongs to, and a later message finds itself already named.

use rusqlite::{OptionalExtension, Transaction};

use lyonoyl_domain::message::{MessageId, Rfc822MessageId};
use lyonoyl_domain::thread::{joining, names_that_decide, Joining, ThreadKey};

use crate::ports::mail_index::MailIndexError;

/// How many already-stored messages one synchronisation puts into conversations.
///
/// Mail stored before this build had none, and threading all of it at once would hold a write
/// transaction for as long as it takes — on a mailbox of the size this is built for, seconds. It
/// converges over a few passes instead, and a folder that is already caught up pays one indexed
/// lookup to find that out.
const CAUGHT_UP_AT_A_TIME: usize = 2_000;

/// Puts one stored message into a conversation, merging any it turns out to tie together.
///
/// `own` is its `Message-ID` where it has a usable one, and `chain` what it said about the
/// conversation it is in. Runs inside the transaction that stored the message, because a message
/// visible in a list without a conversation is a row the thread view cannot place.
pub fn thread_the_message(
    transaction: &Transaction<'_>,
    account: i64,
    message: i64,
    own: Option<&Rfc822MessageId>,
    chain: &[Rfc822MessageId],
) -> Result<(), MailIndexError> {
    let consulted = names_that_decide(own, chain);

    // Asked once per name and used twice: to find the conversation, and to decide whether the name
    // needs writing down at all.
    let mut already: Vec<Option<Option<i64>>> = Vec::with_capacity(consulted.len());
    let mut named = Vec::new();

    for name in &consulted {
        let stored = message_called(transaction, account, name)?;
        already.push(stored);

        let key = match stored {
            Some(Some(thread)) => Some(thread),
            _ => noted(transaction, account, name)?,
        };

        if let Some(thread) = key {
            named.push(ThreadKey::of(
                MessageId::new(thread as u64).map_err(|_| MailIndexError::Unavailable)?,
            ));
        }
    }

    let key = match joining(&named) {
        Joining::Begins => {
            ThreadKey::of(MessageId::new(message as u64).map_err(|_| MailIndexError::Unavailable)?)
        }
        Joining::Joins(one) => one,
        Joining::Ties { into, absorbing } => {
            for absorbed in absorbing {
                tie(transaction, account, into, absorbed)?;
            }
            into
        }
    };

    transaction
        .execute(
            "UPDATE messages SET thread_id = ?2 WHERE id = ?1",
            [message, key.root().get() as i64],
        )
        .map_err(|_| MailIndexError::Unavailable)?;

    // Only what is not written down anywhere else. A name belonging to a message this device holds
    // is answered by that message, and writing it here again would keep a second copy of every
    // `Message-ID` in the mailbox — measured at thirty-six megabytes on four hundred thousand
    // messages, for nothing.
    for (name, stored) in consulted.iter().zip(already) {
        let answered_by_a_message = match stored {
            Some(Some(_)) => true,
            // The message being threaded right now: the line above just gave it a conversation.
            Some(None) => Some(*name) == own,
            None => false,
        };

        if answered_by_a_message {
            continue;
        }

        transaction
            .execute(
                "INSERT INTO thread_names (account_id, name, thread_id)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(account_id, name) DO NOTHING",
                (account, name.as_str(), key.root().get() as i64),
            )
            .map_err(|_| MailIndexError::Unavailable)?;
    }

    Ok(())
}

/// The conversation of a message this device holds under that name, and whether it holds one at all.
///
/// `None` means no such message; `Some(None)` one that has not been put into a conversation yet,
/// which is the state every message is in for the moment between being stored and being threaded,
/// and the state all of them are in while an older mailbox is being caught up.
fn message_called(
    transaction: &Transaction<'_>,
    account: i64,
    name: &Rfc822MessageId,
) -> Result<Option<Option<i64>>, MailIndexError> {
    transaction
        .query_row(
            "SELECT thread_id FROM messages
             WHERE account_id = ?1 AND rfc822_message_id = ?2",
            (account, name.as_str()),
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| MailIndexError::Unavailable)
}

/// Which conversation a name leads to when no message of that name is held here.
fn noted(
    transaction: &Transaction<'_>,
    account: i64,
    name: &Rfc822MessageId,
) -> Result<Option<i64>, MailIndexError> {
    transaction
        .query_row(
            "SELECT thread_id FROM thread_names WHERE account_id = ?1 AND name = ?2",
            (account, name.as_str()),
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| MailIndexError::Unavailable)
}

/// Hands everything of one conversation to another. The survivor keeps its name.
fn tie(
    transaction: &Transaction<'_>,
    account: i64,
    into: ThreadKey,
    absorbed: ThreadKey,
) -> Result<(), MailIndexError> {
    let survivor = into.root().get() as i64;
    let gone = absorbed.root().get() as i64;

    transaction
        .execute(
            "UPDATE messages SET thread_id = ?1 WHERE account_id = ?3 AND thread_id = ?2",
            [survivor, gone, account],
        )
        .map_err(|_| MailIndexError::Unavailable)?;

    transaction
        .execute(
            "UPDATE thread_names SET thread_id = ?1 WHERE account_id = ?3 AND thread_id = ?2",
            [survivor, gone, account],
        )
        .map_err(|_| MailIndexError::Unavailable)?;

    Ok(())
}

/// Forgets the names of a conversation that has no messages left.
///
/// Called after messages are removed. Without it the table grows with every conversation that ever
/// passed through and never shrinks — and a `messages.id` freed by a deletion can be handed to a
/// new message, which would then find itself named by a conversation it has nothing to do with.
pub fn forget_empty_conversations(
    transaction: &Transaction<'_>,
    account: i64,
) -> Result<(), MailIndexError> {
    transaction
        .execute(
            "DELETE FROM thread_names
             WHERE account_id = ?1
               AND thread_id NOT IN (
                   SELECT thread_id FROM messages
                   WHERE account_id = ?1 AND thread_id IS NOT NULL)",
            [account],
        )
        .map(|_| ())
        .map_err(|_| MailIndexError::Unavailable)
}

/// Puts a bounded number of already-stored messages into conversations.
///
/// Returns how many it placed, which is zero once an account is caught up — the state it is in
/// after the first few synchronisations following an upgrade, and the state a new account is in
/// from the start.
pub fn catch_up(transaction: &Transaction<'_>, account: i64) -> Result<usize, MailIndexError> {
    let waiting: Vec<(i64, Option<String>, Option<String>)> = {
        let mut statement = transaction
            .prepare(
                "SELECT id, rfc822_message_id, references_chain FROM messages
                 WHERE account_id = ?1 AND thread_id IS NULL
                 ORDER BY id ASC LIMIT ?2",
            )
            .map_err(|_| MailIndexError::Unavailable)?;

        let rows = statement
            .query_map((account, CAUGHT_UP_AT_A_TIME as i64), |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .map_err(|_| MailIndexError::Unavailable)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| MailIndexError::Unavailable)?;
        rows
    };

    let placed = waiting.len();

    for (id, own, chain) in waiting {
        let own = own.as_deref().and_then(parse);
        let chain = chain.as_deref().map(split).unwrap_or_default();
        thread_the_message(transaction, account, id, own.as_ref(), &chain)?;
    }

    Ok(placed)
}

/// A stored identifier, if this build can still read it.
///
/// A name that no longer parses is not an error to report: it was written by an earlier build or
/// by a message whose header this one judges differently, and the message it belongs to is stored
/// either way. It simply names no conversation.
fn parse(value: &str) -> Option<Rfc822MessageId> {
    Rfc822MessageId::parse(value).ok()
}

/// The stored chain, split the way it was joined.
fn split(chain: &str) -> Vec<Rfc822MessageId> {
    chain.split('\n').filter_map(parse).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::sqlite::migrations::migrate_index;
    use rusqlite::{Connection, TransactionBehavior};

    fn an_account() -> Connection {
        let mut connection = Connection::open_in_memory().expect("in-memory database opens");
        migrate_index(&mut connection, ":memory:").expect("schema applies");
        connection
            .execute(
                "INSERT INTO folders (id, account_id, name, role) VALUES (1, 1, 'INBOX', 1)",
                [],
            )
            .expect("a folder");
        connection
    }

    fn name(at: &str) -> Rfc822MessageId {
        Rfc822MessageId::parse(&format!("<{at}@example.invalid>")).expect("well formed")
    }

    /// Stores a message and puts it into a conversation, the way a synchronisation would.
    fn store(connection: &mut Connection, own: &str, answering: &[&str]) -> i64 {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("a transaction");
        transaction
            .execute(
                "INSERT INTO messages (account_id, rfc822_message_id, received_at)
                 VALUES (1, ?1, 0)",
                [name(own).as_str()],
            )
            .expect("a message");
        let id = transaction.last_insert_rowid();

        let chain: Vec<Rfc822MessageId> = answering.iter().map(|at| name(at)).collect();
        thread_the_message(&transaction, 1, id, Some(&name(own)), &chain).expect("it is threaded");
        transaction.commit().expect("committed");
        id
    }

    fn conversation_of(connection: &Connection, message: i64) -> i64 {
        connection
            .query_row(
                "SELECT thread_id FROM messages WHERE id = ?1",
                [message],
                |row| row.get(0),
            )
            .expect("the message has a conversation")
    }

    #[test]
    fn a_message_naming_nothing_known_is_a_conversation_of_its_own() {
        let mut connection = an_account();
        let alone = store(&mut connection, "alone", &[]);

        assert_eq!(conversation_of(&connection, alone), alone);
    }

    #[test]
    fn a_reply_joins_the_message_it_answers() {
        let mut connection = an_account();
        let first = store(&mut connection, "first", &[]);
        let reply = store(&mut connection, "reply", &["first"]);

        assert_eq!(conversation_of(&connection, reply), first);
    }

    #[test]
    fn a_reply_stored_first_still_holds_the_conversation_open_for_what_it_answers() {
        // The case the whole design exists for. Mail arrives in the order a server hands it over,
        // which is not the order it was written: a reply is often stored days before the message
        // it answers is backfilled. Looking for messages that name this one would find nothing at
        // the moment it is stored, and there is no second look.
        let mut connection = an_account();
        let reply = store(&mut connection, "reply", &["first"]);
        let first = store(&mut connection, "first", &[]);

        assert_eq!(
            conversation_of(&connection, first),
            conversation_of(&connection, reply),
            "the two are one conversation whichever arrived first"
        );
    }

    #[test]
    fn a_message_naming_two_conversations_ties_them_into_the_older() {
        let mut connection = an_account();
        let one = store(&mut connection, "one", &[]);
        let other = store(&mut connection, "other", &[]);
        assert_ne!(
            conversation_of(&connection, one),
            conversation_of(&connection, other)
        );

        let both = store(&mut connection, "both", &["one", "other"]);

        assert_eq!(conversation_of(&connection, one), one);
        assert_eq!(
            conversation_of(&connection, other),
            one,
            "the older conversation keeps its name and the newer is absorbed"
        );
        assert_eq!(conversation_of(&connection, both), one);
    }

    #[test]
    fn a_conversation_already_merged_does_not_change_its_name_again() {
        // What a row on screen depends on. A name that survived one merge survives every later
        // one, so a thread being looked at is not renamed under the reader.
        let mut connection = an_account();
        let one = store(&mut connection, "one", &[]);
        store(&mut connection, "other", &[]);
        store(&mut connection, "both", &["one", "other"]);

        store(&mut connection, "third", &[]);
        store(&mut connection, "all", &["both", "third"]);

        assert_eq!(
            conversation_of(&connection, one),
            one,
            "the oldest message still names the conversation"
        );
    }

    #[test]
    fn mail_stored_before_conversations_existed_is_given_one() {
        // An upgrade finds messages with no conversation. They are threaded from what they already
        // said about each other, which is the same rule applied in the order they arrived.
        let mut connection = an_account();
        connection
            .execute_batch(
                "INSERT INTO messages (id, account_id, rfc822_message_id, received_at)
                 VALUES (1, 1, '<first@example.invalid>', 0);
                 INSERT INTO messages (id, account_id, rfc822_message_id, references_chain,
                                       received_at)
                 VALUES (2, 1, '<reply@example.invalid>', '<first@example.invalid>', 0);",
            )
            .expect("two messages from an older build");

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("a transaction");
        let placed = catch_up(&transaction, 1).expect("they are threaded");
        transaction.commit().expect("committed");

        assert_eq!(placed, 2);
        assert_eq!(conversation_of(&connection, 1), 1);
        assert_eq!(conversation_of(&connection, 2), 1);

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("a transaction");
        let again = catch_up(&transaction, 1).expect("readable");
        transaction.commit().expect("committed");
        assert_eq!(again, 0, "an account already caught up does no work");
    }

    #[test]
    fn a_conversation_whose_last_message_is_gone_is_forgotten() {
        // Otherwise the table grows with every conversation that ever passed through, and a
        // `messages.id` freed by a deletion can be handed to a new message, which would then find
        // itself named by a conversation it has nothing to do with.
        let mut connection = an_account();
        let alone = store(&mut connection, "alone", &["mentioned"]);
        let names: i64 = connection
            .query_row("SELECT count(*) FROM thread_names", [], |row| row.get(0))
            .expect("readable");
        assert_eq!(
            names, 1,
            "only the name it mentions; its own is answered by the message itself"
        );

        connection
            .execute("DELETE FROM messages WHERE id = ?1", [alone])
            .expect("the message goes");

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("a transaction");
        forget_empty_conversations(&transaction, 1).expect("readable");
        transaction.commit().expect("committed");

        let left: i64 = connection
            .query_row("SELECT count(*) FROM thread_names", [], |row| row.get(0))
            .expect("readable");
        assert_eq!(left, 0);
    }

    #[test]
    fn a_conversation_outlives_the_message_it_is_named_after() {
        // The name is a name, not a pointer. A message removed from every folder is dropped from
        // the index, and the conversation it was the oldest of carries on under the same name --
        // which is what keeps a thread from being renamed on screen because somebody deleted its
        // first message.
        let mut connection = an_account();
        let first = store(&mut connection, "first", &[]);
        let reply = store(&mut connection, "reply", &["first"]);
        assert_eq!(conversation_of(&connection, reply), first);

        connection
            .execute("DELETE FROM messages WHERE id = ?1", [first])
            .expect("the first message goes");

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("a transaction");
        forget_empty_conversations(&transaction, 1).expect("readable");
        transaction.commit().expect("committed");

        assert_eq!(
            conversation_of(&connection, reply),
            first,
            "the reply is still in the conversation it was in"
        );
    }

    #[test]
    fn catching_up_ties_a_message_to_one_that_named_it_earlier_in_the_run() {
        // Inside one catch-up every message starts without a conversation, so a message mentioned
        // by an earlier one cannot be recognised by looking it up among the messages. The name
        // written down by the message that mentioned it is what ties them.
        let mut connection = an_account();
        connection
            .execute_batch(
                "INSERT INTO messages (id, account_id, rfc822_message_id, references_chain,
                                       received_at)
                 VALUES (1, 1, '<reply@example.invalid>', '<later@example.invalid>', 0);
                 INSERT INTO messages (id, account_id, rfc822_message_id, received_at)
                 VALUES (2, 1, '<later@example.invalid>', 0);",
            )
            .expect("two messages from an older build, the answer stored first");

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("a transaction");
        catch_up(&transaction, 1).expect("they are threaded");
        transaction.commit().expect("committed");

        assert_eq!(
            conversation_of(&connection, 2),
            conversation_of(&connection, 1),
            "the two are one conversation"
        );
    }

    #[test]
    fn a_reference_chain_written_to_be_hostile_costs_a_bounded_number_of_lookups() {
        // The header has no length limit and is written by whoever sent the message. Without a
        // bound, one message costs a lookup per identifier in it, on a connection holding a write
        // transaction open.
        let mut connection = an_account();
        let chain: Vec<String> = (0..50_000)
            .map(|at| format!("<{at}@example.invalid>"))
            .collect();

        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .expect("a transaction");
        transaction
            .execute(
                "INSERT INTO messages (account_id, rfc822_message_id, received_at)
                 VALUES (1, '<hostile@example.invalid>', 0)",
                [],
            )
            .expect("a message");
        let id = transaction.last_insert_rowid();

        let parsed: Vec<Rfc822MessageId> = chain.iter().filter_map(|at| parse(at)).collect();
        thread_the_message(&transaction, 1, id, Some(&name("hostile")), &parsed)
            .expect("it is threaded");
        transaction.commit().expect("committed");

        let written: i64 = connection
            .query_row("SELECT count(*) FROM thread_names", [], |row| row.get(0))
            .expect("readable");
        assert_eq!(
            written,
            lyonoyl_domain::thread::NAMES_CONSULTED as i64,
            "a message writes down a bounded number of names whatever it claims to answer"
        );
    }
}
