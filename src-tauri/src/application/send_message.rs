//! Writing a message down so it can be sent.
//!
//! Nothing here talks to a server, and that is the point. A message somebody has written exists in
//! one place and nowhere else until a server has taken it, so it is committed first and handed
//! over afterwards — the same shape as acting on mail, for a stronger reason: a folder can be read
//! again, a message that was never written down cannot.
//!
//! The name it will carry is decided here too, before the first attempt. A server that takes a
//! message and then loses the answer leaves nobody knowing whether it arrived; a message that
//! already has its name can be recognised when the mailbox is next read, and one that is given a
//! new name on every attempt cannot.

use crate::application::error::UseCaseError;
use crate::ports::account_repository::AccountRepository;
use crate::ports::outbox::Outbox;
use lyonoyl_domain::outgoing::{message_id_for, Outgoing};

pub struct SendMessage<O: Outbox, A: AccountRepository> {
    outbox: O,
    accounts: A,
}

impl<O: Outbox, A: AccountRepository> SendMessage<O, A> {
    pub fn new(outbox: O, accounts: A) -> Self {
        Self { outbox, accounts }
    }

    /// Writes the message down and answers with the name it will carry.
    ///
    /// `unique` is the part of that name nothing can derive: supplied from outside so this stays a
    /// rule rather than a thing that behaves differently every time it is asked.
    pub fn execute(
        &self,
        account_id: &str,
        message: &Outgoing,
        unique: &str,
    ) -> Result<String, UseCaseError> {
        // A message with nothing written in it is a message somebody meant to send; a message with
        // nobody to send it to is not a message.
        if !message.is_addressed() {
            return Err(UseCaseError::MessageHasNoRecipients);
        }
        // Before anything is committed. Everything below this line writes into the authoritative
        // database, which is never dropped and never re-derived, so a caller with no bound is a
        // caller that can fill a disk in a way nothing here clears.
        if !message.is_within_bounds() {
            return Err(UseCaseError::MessageIsTooLarge);
        }

        // Who it says it is from has to be somebody this device can be. Both halves matter and
        // neither was checked: an account identifier that is not here would queue a message under
        // nothing, and an address that is not that account's would put a name on it the person
        // does not own — most submission servers refuse that, a permissive relay does not, and
        // either way it sits in their own outbox as something they appear to have written.
        let Some(sending) = self.accounts.find(account_id)? else {
            return Err(UseCaseError::UnknownAccount);
        };
        if sending.summary().email() != message.from.as_str() {
            return Err(UseCaseError::NotThisAccountsAddress);
        }

        let message_id = message_id_for(&message.from, unique);
        self.outbox.queue(account_id, &message_id, message)?;

        Ok(message_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::account_repository::AccountRepositoryError;
    use crate::ports::outbox::{Held, OutboxError, WhyHeld};
    use lyonoyl_domain::account::{Encryption, MailAccount, MailAccountSummary, ServerSettings};
    use lyonoyl_domain::outgoing::{EmailAddress, Subject};
    use std::sync::Mutex;

    #[derive(Default)]
    struct Written {
        queued: Mutex<Vec<(String, String, Outgoing)>>,
        refuses: Option<OutboxError>,
    }

    impl Outbox for Written {
        fn queue(
            &self,
            account_id: &str,
            message_id: &str,
            message: &Outgoing,
        ) -> Result<(), OutboxError> {
            if let Some(refusal) = self.refuses.as_ref() {
                return Err(refusal.clone());
            }

            self.queued.lock().expect("test lock").push((
                account_id.to_owned(),
                message_id.to_owned(),
                message.clone(),
            ));

            Ok(())
        }

        fn ready(&self, _account_id: &str, _at_most: usize) -> Result<Vec<Held>, OutboxError> {
            unreachable!("writing a message down reads nothing")
        }

        fn claim(&self, _id: i64) -> Result<bool, OutboxError> {
            unreachable!("writing a message down hands nothing over")
        }

        fn interrupted(&self, _account_id: &str) -> Result<Vec<Held>, OutboxError> {
            unreachable!("writing a message down reads nothing")
        }

        fn awaiting_filing(
            &self,

            _account_id: &str,

            _settled: std::time::Duration,

            _at_most: usize,
        ) -> Result<Vec<Held>, OutboxError> {
            Ok(Vec::new())
        }

        fn filed(&self, _id: i64) -> Result<(), OutboxError> {
            Ok(())
        }

        fn refused_for_now(&self, _id: i64, _retry_after: u64) -> Result<(), OutboxError> {
            unreachable!("writing a message down files nothing")
        }

        fn forget_filed(&self, _account_id: &str) -> Result<(), OutboxError> {
            unreachable!("writing a message down files nothing")
        }

        fn forget_given_back(&self, _account_id: &str) -> Result<(), OutboxError> {
            unreachable!("writing a message down gives nothing back")
        }

        fn accepted(&self, _id: i64) -> Result<(), OutboxError> {
            unreachable!("nothing has been sent")
        }

        fn stopped(
            &self,
            _id: i64,
            _why: WhyHeld,
            _retry_after: Option<u64>,
        ) -> Result<(), OutboxError> {
            unreachable!("nothing has been sent")
        }
    }

    /// The accounts on this device, as this test has them.
    struct TheirAccounts {
        here: Vec<(String, String)>,
    }

    impl TheirAccounts {
        fn with(accounts: &[(&str, &str)]) -> Self {
            Self {
                here: accounts
                    .iter()
                    .map(|(id, email)| ((*id).to_owned(), (*email).to_owned()))
                    .collect(),
            }
        }
    }

    impl AccountRepository for TheirAccounts {
        fn list(&self) -> Result<Vec<MailAccountSummary>, AccountRepositoryError> {
            Ok(Vec::new())
        }

        fn able_to_send(&self) -> Result<Vec<String>, AccountRepositoryError> {
            Ok(Vec::new())
        }

        fn find(&self, account_id: &str) -> Result<Option<MailAccount>, AccountRepositoryError> {
            let Some((id, email)) = self.here.iter().find(|(id, _)| id == account_id) else {
                return Ok(None);
            };

            let summary = MailAccountSummary::new(id, email, "", "imap")
                .map_err(|_| AccountRepositoryError::Corrupted)?;
            MailAccount::new(
                summary,
                ServerSettings::new("imap.example.com", 993, Encryption::Tls)
                    .map_err(|_| AccountRepositoryError::Corrupted)?,
                email,
            )
            .map(Some)
            .map_err(|_| AccountRepositoryError::Corrupted)
        }

        fn find_by_email(&self, _email: &str) -> Result<Option<String>, AccountRepositoryError> {
            Ok(None)
        }

        fn save(&self, _account: &MailAccount) -> Result<(), AccountRepositoryError> {
            Ok(())
        }

        fn delete(&self, _account_id: &str) -> Result<(), AccountRepositoryError> {
            Ok(())
        }

        fn set_collapsed(
            &self,
            _account_id: &str,
            _collapsed: bool,
        ) -> Result<(), AccountRepositoryError> {
            Ok(())
        }

        fn number_of(&self, _: &str) -> Result<Option<i64>, AccountRepositoryError> {
            unreachable!("this fake is not asked which file the mail is in")
        }
    }

    /// The account these tests send from.
    fn zeta() -> TheirAccounts {
        TheirAccounts::with(&[
            ("acc-1", "zeta@example.com"),
            ("acc-2", "zeta@first-domain.test"),
        ])
    }

    fn address(value: &str) -> EmailAddress {
        EmailAddress::parse(value).expect("a valid address")
    }

    fn written_by(who: &str) -> Outgoing {
        Outgoing {
            files: Vec::new(),
            from: address(who),
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: Subject::parse("").expect("empty is a subject"),
            body: String::new(),
            thread: Default::default(),
        }
    }

    #[test]
    fn a_message_is_written_down_under_the_name_it_will_keep() {
        let outbox = Written::default();
        let mut message = written_by("zeta@example.com");
        message.to.push(address("kappa@example.net"));

        let named = SendMessage::new(&outbox, zeta())
            .execute("acc-1", &message, "abc123")
            .expect("it is written down");

        assert_eq!(named, "<abc123@example.com>");

        let queued = outbox.queued.lock().expect("test lock");
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].0, "acc-1");
        assert_eq!(
            queued[0].1, named,
            "the name is written with it, not after it"
        );
        assert_eq!(queued[0].2, message);
    }

    #[test]
    fn a_message_named_after_the_sender_never_names_the_machine() {
        let outbox = Written::default();
        let mut message = written_by("zeta@first-domain.test");
        message.to.push(address("kappa@example.net"));

        let named = SendMessage::new(&outbox, zeta())
            .execute("acc-2", &message, "abc123")
            .expect("it is written down");

        assert!(
            named.ends_with("@first-domain.test>"),
            "a name built from the hostname tells every recipient what somebody's laptop is called"
        );
    }

    #[test]
    fn a_message_claiming_an_address_the_account_does_not_own_is_refused() {
        // Nothing checked this. A message bearing an address the person does not own is one they
        // appear to have written: most submission servers refuse it, a permissive relay does not,
        // and either way it is in their own outbox looking like theirs.
        let outbox = Written::default();
        let mut message = written_by("somebody@else.example");
        message.to.push(address("kappa@example.net"));

        assert!(matches!(
            SendMessage::new(&outbox, zeta()).execute("acc-1", &message, "abc123"),
            Err(UseCaseError::NotThisAccountsAddress)
        ));
        assert!(
            outbox.queued.lock().expect("test lock").is_empty(),
            "it was written down before it was refused"
        );
    }

    #[test]
    fn a_message_from_one_account_cannot_be_sent_under_another() {
        // Both accounts are this person's, and it is still wrong: the message would go out over
        // the wrong identity, and the copy filed in Sent would be in the wrong mailbox.
        let outbox = Written::default();
        let mut message = written_by("zeta@first-domain.test");
        message.to.push(address("kappa@example.net"));

        assert!(matches!(
            SendMessage::new(&outbox, zeta()).execute("acc-1", &message, "abc123"),
            Err(UseCaseError::NotThisAccountsAddress)
        ));
    }

    #[test]
    fn a_message_under_an_account_that_is_not_here_is_refused() {
        let outbox = Written::default();
        let mut message = written_by("zeta@example.com");
        message.to.push(address("kappa@example.net"));

        assert!(matches!(
            SendMessage::new(&outbox, zeta()).execute("acc-gone", &message, "abc123"),
            Err(UseCaseError::UnknownAccount)
        ));
        assert!(outbox.queued.lock().expect("test lock").is_empty());
    }

    #[test]
    fn a_message_too_large_to_write_down_is_refused_before_anything_is_committed() {
        // The outbox is the only unbounded writer this application had into the one database it
        // never drops and never re-derives. What that buys an attacker is not a breach — it is a
        // disk somebody's own mail client filled and nothing here clears.
        let outbox = Written::default();
        let mut message = written_by("zeta@example.com");
        message.to.push(address("kappa@example.net"));
        message.body = "a".repeat(16 * 1024 * 1024 + 1);

        assert!(matches!(
            SendMessage::new(&outbox, zeta()).execute("acc-1", &message, "abc123"),
            Err(UseCaseError::MessageIsTooLarge)
        ));
        assert!(outbox.queued.lock().expect("test lock").is_empty());
    }

    #[test]
    fn a_message_addressed_to_more_people_than_this_build_writes_is_refused() {
        // Counted across all three, because all three are rows.
        let outbox = Written::default();
        let mut message = written_by("zeta@example.com");
        for at in 0..=500 {
            message.to.push(address(&format!("person{at}@example.net")));
        }

        assert!(matches!(
            SendMessage::new(&outbox, zeta()).execute("acc-1", &message, "abc123"),
            Err(UseCaseError::MessageIsTooLarge)
        ));
        assert!(outbox.queued.lock().expect("test lock").is_empty());
    }

    #[test]
    fn a_message_a_person_would_actually_write_is_not_refused() {
        // The bound has to be one nobody meets. A long message to a large list still goes.
        let outbox = Written::default();
        let mut message = written_by("zeta@example.com");
        for at in 0..400 {
            message.to.push(address(&format!("person{at}@example.net")));
        }
        message.body = "a".repeat(1024 * 1024);

        assert!(SendMessage::new(&outbox, zeta())
            .execute("acc-1", &message, "abc123")
            .is_ok());
    }

    #[test]
    fn a_message_with_nobody_to_send_it_to_is_refused_and_not_written() {
        let outbox = Written::default();
        let message = written_by("zeta@example.com");

        assert!(matches!(
            SendMessage::new(&outbox, zeta()).execute("acc-1", &message, "abc123"),
            Err(UseCaseError::MessageHasNoRecipients)
        ));
        assert!(outbox.queued.lock().expect("test lock").is_empty());
    }

    #[test]
    fn a_message_addressed_only_in_blind_copy_is_a_message() {
        let outbox = Written::default();
        let mut message = written_by("zeta@example.com");
        message.bcc.push(address("kappa@example.net"));

        SendMessage::new(&outbox, zeta())
            .execute("acc-1", &message, "abc123")
            .expect("somebody is going to receive it");

        assert_eq!(outbox.queued.lock().expect("test lock").len(), 1);
    }

    #[test]
    fn a_message_with_nothing_written_in_it_is_still_written_down() {
        // Refusing it would be arguing with somebody about what they meant to send.
        let outbox = Written::default();
        let mut message = written_by("zeta@example.com");
        message.to.push(address("kappa@example.net"));

        SendMessage::new(&outbox, zeta())
            .execute("acc-1", &message, "abc123")
            .expect("an empty message is a message");
    }

    #[test]
    fn the_same_message_twice_is_said_rather_than_written_twice() {
        let outbox = Written {
            refuses: Some(OutboxError::AlreadyWaiting),
            ..Written::default()
        };
        let mut message = written_by("zeta@example.com");
        message.to.push(address("kappa@example.net"));

        assert!(matches!(
            SendMessage::new(&outbox, zeta()).execute("acc-1", &message, "abc123"),
            Err(UseCaseError::MessageAlreadyWaiting)
        ));
    }

    #[test]
    fn an_outbox_that_will_not_open_is_a_message_not_sent_rather_than_a_message_lost() {
        let outbox = Written {
            refuses: Some(OutboxError::Unavailable),
            ..Written::default()
        };
        let mut message = written_by("zeta@example.com");
        message.to.push(address("kappa@example.net"));

        assert!(matches!(
            SendMessage::new(&outbox, zeta()).execute("acc-1", &message, "abc123"),
            Err(UseCaseError::StorageUnavailable)
        ));
    }
}
