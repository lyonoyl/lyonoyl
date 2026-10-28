//! What is asked, and in what order.

use super::*;
use crate::ports::account_repository::AccountRepositoryError;
use crate::ports::provider_directory::DirectoryError;
use lyonoyl_domain::account::{
    Encryption, MailAccount, MailAccountSummary, ServerSettings as Server, Submission as Sending,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// A directory that answers what it was told to, and writes down who was asked.
///
/// Written by hand rather than generated: what these tests are about is the order of the asking,
/// and a fake that records the questions is the only way to see one.
struct Directory {
    answer: Result<Published, DirectoryError>,
    asked: Mutex<Vec<String>>,
}

impl Directory {
    fn answering(answer: Result<Published, DirectoryError>) -> Self {
        Self {
            answer,
            asked: Mutex::new(Vec::new()),
        }
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().expect("not poisoned").clone()
    }
}

impl ProviderDirectory for Directory {
    fn published_by(&self, domain: &str) -> Result<Published, DirectoryError> {
        self.asked
            .lock()
            .expect("not poisoned")
            .push(domain.to_owned());
        self.answer.clone()
    }
}

/// The accounts already on this device, in the order the store lists them.
///
/// Only the two questions this use case asks are answered. Anything else is a question it has no
/// business asking — about passwords least of all, which do not pass through this port and which
/// this use case is given no other way to reach.
struct Accounts {
    rows: Vec<MailAccount>,
    /// The store will not be read at all.
    unavailable: bool,
    /// Listed, and then refused when read whole.
    unreadable: Vec<&'static str>,
    /// Listed, and gone by the time it is read whole.
    gone: Vec<&'static str>,
    listed: AtomicUsize,
}

impl Accounts {
    fn holding(rows: Vec<MailAccount>) -> Self {
        Self {
            rows,
            unavailable: false,
            unreadable: Vec::new(),
            gone: Vec::new(),
            listed: AtomicUsize::new(0),
        }
    }

    fn none() -> Self {
        Self::holding(Vec::new())
    }

    fn listed(&self) -> usize {
        self.listed.load(Ordering::SeqCst)
    }
}

impl AccountRepository for &Accounts {
    fn list(&self) -> Result<Vec<MailAccountSummary>, AccountRepositoryError> {
        self.listed.fetch_add(1, Ordering::SeqCst);
        if self.unavailable {
            return Err(AccountRepositoryError::Unavailable);
        }
        Ok(self
            .rows
            .iter()
            .map(|account| account.summary().clone())
            .collect())
    }

    fn find(&self, account_id: &str) -> Result<Option<MailAccount>, AccountRepositoryError> {
        if self.unreadable.contains(&account_id) {
            return Err(AccountRepositoryError::Corrupted);
        }
        if self.gone.contains(&account_id) {
            return Ok(None);
        }
        Ok(self
            .rows
            .iter()
            .find(|account| account.summary().id() == account_id)
            .cloned())
    }

    fn able_to_send(&self) -> Result<Vec<String>, AccountRepositoryError> {
        unreachable!("finding servers does not ask who can send")
    }

    fn find_by_email(&self, _email: &str) -> Result<Option<String>, AccountRepositoryError> {
        unreachable!("finding servers does not look an account up by its address")
    }

    fn save(&self, _account: &MailAccount) -> Result<(), AccountRepositoryError> {
        unreachable!("finding servers stores nothing")
    }

    fn delete(&self, _account_id: &str) -> Result<(), AccountRepositoryError> {
        unreachable!("finding servers removes nothing")
    }

    fn number_of(&self, _account_id: &str) -> Result<Option<i64>, AccountRepositoryError> {
        unreachable!("finding servers does not open anybody's mail")
    }

    fn set_collapsed(
        &self,
        _account_id: &str,
        _collapsed: bool,
    ) -> Result<(), AccountRepositoryError> {
        unreachable!("finding servers changes nothing")
    }
}

/// An account whose reading server is named after its identifier, so a test can tell which one
/// was taken.
fn account(id: &str, email: &str, imap_username: &str) -> MailAccount {
    MailAccount::new(
        MailAccountSummary::new(id, email, "Zeta Quorin", "imap").expect("well-formed"),
        Server::new(format!("imap.{id}.example.net"), 143, Encryption::StartTls)
            .expect("well-formed"),
        imap_username,
    )
    .expect("well-formed")
}

/// The same account, sending through a server named after it too.
fn sending(account: MailAccount, username: &str) -> MailAccount {
    let id = account.summary().id().to_owned();
    account.sending_through(
        Sending::new(
            Server::new(format!("smtp.{id}.example.net"), 587, Encryption::StartTls)
                .expect("well-formed"),
            username,
        )
        .expect("well-formed"),
    )
}

fn published() -> ServerSettings {
    ServerSettings {
        imap_host: "imap.example.com".to_owned(),
        imap_port: 993,
        imap_encryption: Encryption::Tls,
        smtp: None,
    }
}

/// What was found, taken apart: the servers, what to sign in as, and who said so.
fn known(settings: Settings) -> (ServerSettings, SignIn, Found) {
    match settings {
        Settings::Known {
            servers,
            sign_in,
            found,
        } => (*servers, sign_in, found),
        Settings::Unknown => panic!("nothing was found"),
    }
}

#[test]
fn a_provider_this_build_carries_is_answered_without_asking_anybody() {
    // The whole reason the table comes first. Reaching the network here would tell somebody that
    // this person uses Gmail, to learn what was already known.
    let directory = Directory::answering(Ok(Published::Nothing));
    let found = FindServerSettings::new(&Accounts::none(), &directory)
        .execute("zeta@gmail.com", None)
        .expect("an answer");

    assert!(matches!(
        found,
        Settings::Known {
            found: Found::Carried,
            sign_in: SignIn::ADDRESS,
            ..
        }
    ));
    assert!(
        directory.asked().is_empty(),
        "somebody was asked about a domain this build already knows"
    );
}

#[test]
fn a_domain_nobody_carries_is_asked_about_itself() {
    let directory = Directory::answering(Ok(Published::Settings(Box::new(published()))));
    let found = FindServerSettings::new(&Accounts::none(), &directory)
        .execute("zeta@example.com", None)
        .expect("an answer");

    let (servers, sign_in, found) = known(found);
    assert_eq!(found, Found::PublishedByTheDomain);
    assert_eq!(servers.imap_host, "imap.example.com");
    assert_eq!(sign_in, SignIn::ADDRESS);
    assert_eq!(directory.asked(), vec!["example.com".to_owned()]);
}

#[test]
fn the_domain_asked_about_is_the_one_in_the_address_and_lowered() {
    // What is asked is a web address built out of this, so the shape of it matters more than it
    // looks: a second spelling is a second question to somebody else's machine.
    let directory = Directory::answering(Ok(Published::Nothing));
    let _ =
        FindServerSettings::new(&Accounts::none(), &directory).execute("Zeta@Example.COM", None);

    assert_eq!(directory.asked(), vec!["example.com".to_owned()]);
}

#[test]
fn a_domain_that_publishes_nothing_is_an_answer_rather_than_a_failure() {
    let directory = Directory::answering(Ok(Published::Nothing));
    let found = FindServerSettings::new(&Accounts::none(), &directory)
        .execute("zeta@example.com", None)
        .expect("an answer");

    assert_eq!(found, Settings::Unknown);
}

#[test]
fn a_domain_nobody_could_reach_is_the_same_answer_to_the_person() {
    // Different underneath and the same on screen: either way the settings are typed in, and a
    // person who has been shown a network error still has to type them.
    let directory = Directory::answering(Err(DirectoryError::Unreachable));
    let found = FindServerSettings::new(&Accounts::none(), &directory)
        .execute("zeta@example.com", None)
        .expect("an answer");
    assert_eq!(found, Settings::Unknown);
}

#[test]
fn what_is_not_one_address_reaches_nobody() {
    // Half a domain is not this person's domain, and asking about it would tell its owner that
    // somebody is setting up mail there.
    let directory = Directory::answering(Ok(Published::Nothing));
    let accounts = Accounts::holding(vec![account("one", "zeta@example.com", "zeta")]);

    for half in ["zeta", "zeta@", "zeta@exa", "@example.com", "zeta@example"] {
        let found = FindServerSettings::new(&accounts, &directory)
            .execute(half, None)
            .expect("an answer");
        assert_eq!(found, Settings::Unknown, "{half}");
    }
    assert!(
        directory.asked().is_empty(),
        "a half-typed address was sent to somebody: {:?}",
        directory.asked()
    );
    assert_eq!(
        accounts.listed(),
        0,
        "the accounts here were read for an address that is not one"
    );
}

#[test]
fn an_account_here_at_the_same_domain_answers_before_anybody_is_asked() {
    // The second mailbox at a domain: the servers were typed in once,
    // they answered on this machine, and asking the domain again would tell it something for
    // nothing.
    let directory = Directory::answering(Ok(Published::Settings(Box::new(published()))));
    let accounts = Accounts::holding(vec![sending(
        account("one", "zeta@example.com", "zeta@example.com"),
        "zeta@example.com",
    )]);

    let (servers, sign_in, found) = known(
        FindServerSettings::new(&accounts, &directory)
            .execute("kappa@example.com", None)
            .expect("an answer"),
    );

    assert_eq!(found, Found::OnThisDevice);
    assert_eq!(servers.imap_host, "imap.one.example.net");
    assert_eq!(servers.imap_port, 143);
    assert_eq!(servers.imap_encryption, Encryption::StartTls);
    let smtp = servers.smtp.expect("the sending server came with it");
    assert_eq!(smtp.host, "smtp.one.example.net");
    assert_eq!(smtp.port, 587);
    assert_eq!(smtp.encryption, Encryption::StartTls);
    assert_eq!(sign_in, SignIn::ADDRESS);
    assert!(
        directory.asked().is_empty(),
        "the domain was asked about servers this device already had: {:?}",
        directory.asked()
    );
}

#[test]
fn an_account_here_at_another_domain_leaves_the_cascade_as_it_was() {
    let directory = Directory::answering(Ok(Published::Settings(Box::new(published()))));
    let accounts = Accounts::holding(vec![sending(
        account("one", "zeta@example.com", "zeta@example.com"),
        "zeta@example.com",
    )]);

    let (servers, _, found) = known(
        FindServerSettings::new(&accounts, &directory)
            .execute("sigma@example.org", None)
            .expect("an answer"),
    );

    assert_eq!(found, Found::PublishedByTheDomain);
    assert_eq!(servers, published());
    assert_eq!(directory.asked(), vec!["example.org".to_owned()]);
}

#[test]
fn letter_case_does_not_make_a_domain_another_one() {
    let directory = Directory::answering(Ok(Published::Nothing));
    let accounts = Accounts::holding(vec![account("one", "zeta@Example.COM", "zeta")]);

    let (servers, _, found) = known(
        FindServerSettings::new(&accounts, &directory)
            .execute("kappa@EXAMPLE.com", None)
            .expect("an answer"),
    );

    assert_eq!(found, Found::OnThisDevice);
    assert_eq!(servers.imap_host, "imap.one.example.net");
    assert!(directory.asked().is_empty());
}

#[test]
fn a_subdomain_is_another_domain_in_either_direction() {
    // A subdomain is somebody else's servers as often as it is the same ones, and a guess that
    // hands a password to the wrong machine is worse than a question.
    for (theirs, added) in [
        ("zeta@mail.example.com", "kappa@example.com"),
        ("zeta@example.com", "kappa@mail.example.com"),
        // Ending the same way is not being the same domain either.
        ("zeta@myexample.test", "kappa@example.test"),
    ] {
        let directory = Directory::answering(Ok(Published::Nothing));
        let accounts = Accounts::holding(vec![account("one", theirs, theirs)]);

        let found = FindServerSettings::new(&accounts, &directory)
            .execute(added, None)
            .expect("an answer");

        assert_eq!(found, Settings::Unknown, "{theirs} answered for {added}");
        assert_eq!(directory.asked().len(), 1, "{added} was not asked about");
    }
}

#[test]
fn a_user_name_is_its_shape_and_never_the_other_accounts_own() {
    // Signing in as the address, as the part before the `@`, or as something only that account's
    // owner knows. The first two are shapes this address can take too; the third is somebody
    // else's user name, and it stays theirs.
    for (theirs, expected, spelled) in [
        ("ZETA@example.com", SignInAs::Address, "kappa@example.com"),
        ("zeta", SignInAs::LocalPart, "kappa"),
        ("Zeta", SignInAs::LocalPart, "kappa"),
        ("mailbox-0", SignInAs::Address, "kappa@example.com"),
        ("zeta@example.net", SignInAs::Address, "kappa@example.com"),
    ] {
        let directory = Directory::answering(Ok(Published::Nothing));
        let accounts = Accounts::holding(vec![sending(
            account("one", "zeta@example.com", theirs),
            theirs,
        )]);

        let answer = FindServerSettings::new(&accounts, &directory)
            .execute("kappa@example.com", None)
            .expect("an answer");
        assert!(
            !format!("{answer:?}").contains(theirs),
            "{theirs} travelled in the answer: {answer:?}"
        );

        let (_, sign_in, _) = known(answer);
        assert_eq!(sign_in.imap, expected, "reading, signed in as {theirs}");
        assert_eq!(
            sign_in.submission, expected,
            "sending, signed in as {theirs}"
        );
        assert_eq!(sign_in.imap.user_name_for(" kappa@example.com "), spelled);
    }
}

#[test]
fn reading_and_sending_are_shaped_each_on_its_own() {
    // One server that wants the local part and another that wants the whole address is a real
    // arrangement, and copying one answer to both would get one of them wrong.
    let directory = Directory::answering(Ok(Published::Nothing));
    let accounts = Accounts::holding(vec![sending(
        account("one", "zeta@example.com", "zeta"),
        "zeta@example.com",
    )]);

    let (_, sign_in, _) = known(
        FindServerSettings::new(&accounts, &directory)
            .execute("kappa@example.com", None)
            .expect("an answer"),
    );

    assert_eq!(
        sign_in,
        SignIn {
            imap: SignInAs::LocalPart,
            submission: SignInAs::Address,
        }
    );
}

#[test]
fn an_account_here_that_cannot_send_still_answers_for_reading() {
    let directory = Directory::answering(Ok(Published::Nothing));
    let accounts = Accounts::holding(vec![account("one", "zeta@example.com", "zeta")]);

    let (servers, sign_in, found) = known(
        FindServerSettings::new(&accounts, &directory)
            .execute("kappa@example.com", None)
            .expect("an answer"),
    );

    assert_eq!(found, Found::OnThisDevice);
    assert_eq!(
        servers.smtp, None,
        "a sending server nobody gave was made up"
    );
    assert_eq!(sign_in.imap, SignInAs::LocalPart);
}

#[test]
fn the_account_being_corrected_is_no_answer_about_itself() {
    // Its servers are what the person is correcting. Handing them back as the answer would put the
    // mistake straight back into the form.
    let directory = Directory::answering(Ok(Published::Nothing));
    let accounts = Accounts::holding(vec![account("one", "zeta@example.com", "zeta")]);

    let found = FindServerSettings::new(&accounts, &directory)
        .execute("kappa@example.com", Some("one"))
        .expect("an answer");

    assert_eq!(found, Settings::Unknown);
    assert_eq!(directory.asked(), vec!["example.com".to_owned()]);
}

#[test]
fn an_account_with_this_very_address_is_no_other_mailbox() {
    let directory = Directory::answering(Ok(Published::Nothing));
    let accounts = Accounts::holding(vec![account("one", "zeta@example.com", "zeta")]);

    let found = FindServerSettings::new(&accounts, &directory)
        .execute(" Zeta@Example.com ", None)
        .expect("an answer");

    assert_eq!(found, Settings::Unknown);
    assert_eq!(directory.asked(), vec!["example.com".to_owned()]);
}

#[test]
fn of_several_the_first_that_can_send_is_taken_and_otherwise_the_first() {
    let reads = || account("reads", "zeta@example.com", "zeta");
    let sends = || sending(account("sends", "sigma@example.com", "sigma"), "sigma");
    let also_sends = || {
        sending(
            account("also-sends", "kappa@example.com", "kappa@example.com"),
            "kappa@example.com",
        )
    };
    let also_reads = || account("also-reads", "sigma@example.com", "sigma");

    for (listed, taken) in [
        (
            vec![reads(), sends(), also_sends()],
            "imap.sends.example.net",
        ),
        (
            vec![reads(), also_sends(), sends()],
            "imap.also-sends.example.net",
        ),
        (vec![reads(), also_reads()], "imap.reads.example.net"),
        (vec![also_reads(), reads()], "imap.also-reads.example.net"),
    ] {
        let directory = Directory::answering(Ok(Published::Nothing));
        let accounts = Accounts::holding(listed);

        // Asked twice, to be sure the same question gets the same answer.
        let first = FindServerSettings::new(&accounts, &directory)
            .execute("zeta.quorin@example.com", None)
            .expect("an answer");
        let again = FindServerSettings::new(&accounts, &directory)
            .execute("zeta.quorin@example.com", None)
            .expect("an answer");
        assert_eq!(first, again);

        let (servers, _, _) = known(first);
        assert_eq!(servers.imap_host, taken);
    }
}

#[test]
fn a_store_that_cannot_be_read_leaves_the_cascade_to_answer() {
    // Taking servers from an account here saves a question; it is never the only way to answer
    // one, and a store that will not open is no reason to stop a person adding mail.
    let directory = Directory::answering(Ok(Published::Settings(Box::new(published()))));
    let mut accounts = Accounts::holding(vec![account("one", "zeta@example.com", "zeta")]);
    accounts.unavailable = true;

    let (_, _, found) = known(
        FindServerSettings::new(&accounts, &directory)
            .execute("kappa@example.com", None)
            .expect("an answer"),
    );

    assert_eq!(found, Found::PublishedByTheDomain);
}

#[test]
fn an_account_that_will_not_read_or_has_gone_is_passed_over() {
    let directory = Directory::answering(Ok(Published::Nothing));
    let mut accounts = Accounts::holding(vec![
        sending(account("broken", "zeta@example.com", "zeta"), "zeta"),
        sending(account("gone", "sigma@example.com", "sigma"), "sigma"),
        account("whole", "kappa@example.com", "kappa"),
    ]);
    accounts.unreadable = vec!["broken"];
    accounts.gone = vec!["gone"];

    let (servers, _, found) = known(
        FindServerSettings::new(&accounts, &directory)
            .execute("zeta.quorin@example.com", None)
            .expect("an answer"),
    );

    assert_eq!(found, Found::OnThisDevice);
    assert_eq!(servers.imap_host, "imap.whole.example.net");
}
