//! Who an answer goes to.
//!
//! Two questions, and only the second is interesting. Answering the writer is one address. Answering
//! everybody is every address the message named, in the order it named them, minus every address
//! the person has — not only the one they happen to be writing from. Somebody with a work address
//! and an alias, answering a thread that carries both, would otherwise send themselves a copy and,
//! to everyone else in the thread, look as though they had meant to.
//!
//! The rule itself is in the domain and is the same rule wherever it is asked. What is here is the
//! part that needs to know things: which addresses are the person's, which is a question about this
//! installation rather than about mail.

use crate::application::error::UseCaseError;
use crate::ports::account_repository::AccountRepository;
use lyonoyl_domain::outgoing::{reply_all_to, EmailAddress};

/// Who an answer is addressed to, and who is kept informed of it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Audience {
    pub to: Vec<String>,
    pub cc: Vec<String>,
}

/// What the message being answered said about who it was for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Answered {
    /// The address it came from, when the index has one.
    pub from: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
}

pub struct AnswerMessage<A: AccountRepository> {
    accounts: A,
}

impl<A: AccountRepository> AnswerMessage<A> {
    pub fn new(accounts: A) -> Self {
        Self { accounts }
    }

    /// Who an answer to this message goes to.
    ///
    /// `everybody` is the difference between answering the person who wrote and answering the room.
    /// Answering the writer keeps nobody informed: a `Cc` a person did not ask for is a message sent
    /// to somebody who was not meant to receive it, and that cannot be taken back.
    ///
    /// An address the index holds that this build cannot parse is left out rather than passed on. It
    /// arrived in a header written by somebody else, and putting it in a message going out would
    /// mean sending to an address nothing here understands.
    pub fn execute(&self, answered: &Answered, everybody: bool) -> Result<Audience, UseCaseError> {
        let writer = answered.from.as_deref().and_then(parsed);

        if !everybody {
            return Ok(Audience {
                to: writer
                    .map(|address| vec![address.as_str()])
                    .unwrap_or_default(),
                cc: Vec::new(),
            });
        }

        let mine: Vec<EmailAddress> = self
            .accounts
            .list()?
            .iter()
            .filter_map(|account| EmailAddress::parse(account.email()).ok())
            .collect();

        // The writer first, then everyone the message was addressed to: an answer goes to the
        // person who wrote it before it goes to the people they wrote to.
        let mut named: Vec<EmailAddress> = Vec::new();
        named.extend(writer);
        named.extend(answered.to.iter().filter_map(|value| parsed(value)));

        Ok(Audience {
            to: written(reply_all_to(&named, &mine)),
            cc: written(reply_all_to(
                &answered
                    .cc
                    .iter()
                    .filter_map(|value| parsed(value))
                    .collect::<Vec<_>>(),
                &mine,
            )),
        })
    }
}

fn parsed(value: &str) -> Option<EmailAddress> {
    EmailAddress::parse(value).ok()
}

fn written(addresses: Vec<EmailAddress>) -> Vec<String> {
    addresses.iter().map(EmailAddress::as_str).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::account_repository::AccountRepositoryError;
    use lyonoyl_domain::account::{MailAccount, MailAccountSummary};

    /// The accounts this person has, and nothing else this port can do.
    struct TheirAccounts {
        emails: Vec<String>,
    }

    impl TheirAccounts {
        fn with(emails: &[&str]) -> Self {
            Self {
                emails: emails.iter().map(|value| (*value).to_owned()).collect(),
            }
        }
    }

    impl AccountRepository for TheirAccounts {
        fn list(&self) -> Result<Vec<MailAccountSummary>, AccountRepositoryError> {
            self.emails
                .iter()
                .enumerate()
                .map(|(index, email)| {
                    MailAccountSummary::new(format!("acc-{index}"), email, "", "imap")
                        .map_err(|_| AccountRepositoryError::Corrupted)
                })
                .collect()
        }

        fn able_to_send(&self) -> Result<Vec<String>, AccountRepositoryError> {
            Ok(Vec::new())
        }

        fn find(&self, _account_id: &str) -> Result<Option<MailAccount>, AccountRepositoryError> {
            Ok(None)
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

    fn answered() -> Answered {
        Answered {
            from: Some("zeta@example.com".to_owned()),
            to: vec!["you@example.com".to_owned(), "kappa@example.net".to_owned()],
            cc: vec!["sigma@example.org".to_owned()],
        }
    }

    fn asking(emails: &[&str]) -> AnswerMessage<TheirAccounts> {
        AnswerMessage::new(TheirAccounts::with(emails))
    }

    #[test]
    fn answering_the_writer_goes_to_the_writer_and_nobody_else() {
        // Everybody else on the message did not ask to hear the answer, and a copy sent to them
        // cannot be taken back.
        let audience = asking(&["you@example.com"])
            .execute(&answered(), false)
            .expect("the accounts are readable");

        assert_eq!(audience.to, vec!["zeta@example.com".to_owned()]);
        assert!(audience.cc.is_empty());
    }

    #[test]
    fn answering_everybody_keeps_the_room_and_leaves_me_out() {
        let audience = asking(&["you@example.com"])
            .execute(&answered(), true)
            .expect("the accounts are readable");

        // The writer first, then the people they wrote to. Cc stays Cc: promoting somebody who was
        // being kept informed to somebody being written to says something that was not said.
        assert_eq!(
            audience.to,
            vec![
                "zeta@example.com".to_owned(),
                "kappa@example.net".to_owned()
            ]
        );
        assert_eq!(audience.cc, vec!["sigma@example.org".to_owned()]);
    }

    #[test]
    fn every_address_of_mine_is_left_out_and_not_only_the_one_i_am_writing_from() {
        // The case the rule exists for: an alias in the same thread. Answering from one address
        // while the other stays in the room sends the person a copy of their own answer.
        let audience = asking(&["you@example.com", "kappa@example.net"])
            .execute(&answered(), true)
            .expect("the accounts are readable");

        assert_eq!(audience.to, vec!["zeta@example.com".to_owned()]);
        assert_eq!(audience.cc, vec!["sigma@example.org".to_owned()]);
    }

    #[test]
    fn a_message_the_index_has_no_address_for_is_answered_to_nobody() {
        // Better an answer somebody finishes addressing than one that goes somewhere unintended.
        // Messages read before the address had a column of its own are the ordinary case.
        let audience = asking(&["you@example.com"])
            .execute(
                &Answered {
                    from: None,
                    ..answered()
                },
                false,
            )
            .expect("the accounts are readable");

        assert!(audience.to.is_empty());
    }

    #[test]
    fn answering_my_own_message_to_everybody_still_reaches_the_room() {
        // A message in Sent: the writer is me, so the writer drops out and the people it went to
        // are the answer's audience.
        let audience = asking(&["zeta@example.com"])
            .execute(&answered(), true)
            .expect("the accounts are readable");

        assert_eq!(
            audience.to,
            vec!["you@example.com".to_owned(), "kappa@example.net".to_owned()]
        );
    }

    #[test]
    fn an_address_this_build_cannot_read_is_left_out_rather_than_passed_on() {
        // It arrived in somebody else's header. Sending to it would mean writing to an address
        // nothing here understands, and the person cannot see that is what happened.
        let audience = asking(&["you@example.com"])
            .execute(
                &Answered {
                    from: Some("zeta@example.com".to_owned()),
                    to: vec!["not an address".to_owned(), "kappa@example.net".to_owned()],
                    cc: Vec::new(),
                },
                true,
            )
            .expect("the accounts are readable");

        assert_eq!(
            audience.to,
            vec![
                "zeta@example.com".to_owned(),
                "kappa@example.net".to_owned()
            ]
        );
    }

    #[test]
    fn the_same_person_named_twice_is_written_to_once() {
        // A thread that has gone round a few times collects the same people several times over.
        let audience = asking(&["you@example.com"])
            .execute(
                &Answered {
                    from: Some("zeta@example.com".to_owned()),
                    to: vec![
                        "zeta@example.com".to_owned(),
                        "kappa@example.net".to_owned(),
                        "KAPPA@Example.NET".to_owned(),
                    ],
                    cc: Vec::new(),
                },
                true,
            )
            .expect("the accounts are readable");

        assert_eq!(
            audience.to,
            vec![
                "zeta@example.com".to_owned(),
                "kappa@example.net".to_owned()
            ]
        );
    }
}
