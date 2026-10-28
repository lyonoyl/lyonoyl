//! Opening one message: reading it whole from the server and making it safe to show.
//!
//! Read when it is opened rather than when it arrives. A pass stores what a list needs — who wrote,
//! when, what about — and a body is the part nobody may need: fetching every one of them would
//! trade a mailbox of metadata for a mailbox of MIME.
//!
//! What comes back is already sanitized. This use case never sees HTML that has not been through
//! the renderer, and what it hands back does not cross IPC as a string.

use crate::application::error::UseCaseError;
use crate::application::whole_message::WholeMessage;
use crate::ports::account_repository::AccountRepository;
use crate::ports::body::{BodyRenderer, InlinePart, RenderedBody, Viewing};
use crate::ports::credential_store::CredentialStore;
use crate::ports::mail_provider::ProviderFactory;
use crate::ports::message_bodies::MessageBodies;
use crate::ports::pictures::{PictureOutcome, PictureSource};
use lyonoyl_domain::message::{MessageId, Uid, UidValidity};

pub struct OpenMessage<R, C, F, B, P, S>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    B: BodyRenderer,
    P: PictureSource,
    S: MessageBodies,
{
    accounts: R,
    credentials: C,
    providers: F,
    renderer: B,
    pictures: P,
    kept: S,
}

impl<R, C, F, B, P, S> OpenMessage<R, C, F, B, P, S>
where
    R: AccountRepository,
    C: CredentialStore,
    F: ProviderFactory,
    B: BodyRenderer,
    P: PictureSource,
    S: MessageBodies,
{
    pub fn new(
        accounts: R,
        credentials: C,
        providers: F,
        renderer: B,
        pictures: P,
        kept: S,
    ) -> Self {
        Self {
            accounts,
            credentials,
            providers,
            renderer,
            pictures,
            kept,
        }
    }

    /// `mailbox`, `uid_validity` and `uid` say where the message sits on the server; all three
    /// come from the index, never from the interface, which knows a message only by its own
    /// identifier. `uid_validity` is absent where the index has no generation stored for the
    /// folder, and then the message is read only from this device.
    pub fn execute(
        &self,
        account_id: &str,
        message: MessageId,
        mailbox: &str,
        uid_validity: Option<UidValidity>,
        uid: Uid,
        viewing: Viewing,
    ) -> Result<RenderedBody, UseCaseError> {
        let raw = WholeMessage {
            accounts: &self.accounts,
            credentials: &self.credentials,
            providers: &self.providers,
            kept: &self.kept,
        }
        .read(account_id, message, mailbox, uid_validity, uid)?;

        // What the index never wrote down about this message, written down now. The message is
        // here either way, so it costs a read of what is already in hand and never a server. It is
        // done for one that was already kept as well as one just fetched: the messages this is for
        // are precisely the ones that have been on the device the longest.
        //
        // A repair that fails changes nothing about reading the message, which is what somebody
        // asked for.
        let _ = self.kept.fill_in_missing(message, &raw);

        self.show(raw, viewing)
    }

    /// The message as something to look at.
    fn show(&self, raw: Vec<u8>, viewing: Viewing) -> Result<RenderedBody, UseCaseError> {
        // Handed over whole and unread: what turns bytes into something showable is the renderer,
        // and nothing here looks at them on the way.
        let mut body = self.renderer.render(&raw, viewing);

        // The pictures the message points at, fetched because the reader asked for this message.
        // They arrive here rather than at the frame: the frame has no network, and the host learns
        // nothing about it beyond that something asked once.
        if !body.remote.is_empty() {
            let urls: Vec<String> = body.remote.iter().map(|at| at.url.clone()).collect();
            let fetched = self.pictures.fetch(&urls);

            put_the_pictures_in(&mut body, fetched);
        }

        Ok(body)
    }
}

/// Puts what came back where the body points, and takes back the pointing where nothing came.
///
/// Public to the crate because the engine probe renders a message the same way and must reach the
/// same answer: an instrument that does not share the code it measures is measuring something else.
pub(crate) fn put_the_pictures_in(body: &mut RenderedBody, fetched: Vec<PictureOutcome>) {
    let mut missing = 0;
    let mut not_a_kind_shown_here = 0;
    let mut nothing_behind_them: Vec<String> = Vec::new();

    for (pointed_at, got) in body.remote.clone().iter().zip(fetched) {
        match got {
            PictureOutcome::Shown(picture) => body.parts.push(InlinePart {
                name: pointed_at.name.clone(),
                content_type: picture.content_type,
                bytes: picture.bytes,
            }),
            // A host that is down is not a message that cannot be read, and a minute later it may
            // be up.
            PictureOutcome::CouldNotBeHad => {
                missing += 1;
                nothing_behind_them.push(pointed_at.name.clone());
            }
            // This one arrived. Counting it with the others would send a reader back to ask again
            // for something that will not become showable by being asked twice.
            PictureOutcome::NotAKindShownHere => {
                not_a_kind_shown_here += 1;
                nothing_behind_them.push(pointed_at.name.clone());
            }
        }
    }

    // The body was written before anybody knew which of these would arrive, so it points at all of
    // them. The ones that did not arrive are taken back out: the frame has nowhere to ask but the
    // store, and a name the store cannot answer is a request that comes back empty — drawn as a
    // broken picture where the message meant one, and counted a second time as a picture gone
    // missing when the reader has already been told the number.
    //
    // The attribute and nothing else: what the message wrote around it — its size, its alt text,
    // the cell it sits in — is the message's own and stays. An `img` with no source asks for
    // nothing, which is the whole point. A name is 32 bytes of randomness minted for one render and
    // written by the sanitizer in exactly this form, so there is one place it can appear and no
    // message can spell it by accident.
    for name in &nothing_behind_them {
        body.html = body.html.replace(&format!(" src=\"/{name}\""), "");
        // The same taking-back for a picture a style draws, which is written another way. Left in,
        // it is the identical fault one shape along: the frame asks the store for a name it does
        // not hold, and draws nothing where the message meant a background.
        body.html = body.html.replace(&format!("url(/{name})"), "none");
    }

    // Counted with the ones nobody asked for: what the number means to a reader is "this many
    // pictures are not on screen", and why is a separate sentence.
    //
    // Kept apart from the ones nobody asked about. To a reader they are one sentence — this many
    // pictures are not on screen — but "the host said no" and "we never asked" are different
    // faults, and a reader who is told to try again at the second is being sent back to something
    // that cannot succeed.
    body.remote_not_had += missing;
    body.remote_not_shown += not_a_kind_shown_here;
}

#[cfg(test)]
mod tests {
    use crate::ports::message_bodies::MessageBodyError;
    use std::sync::Mutex;

    use secrecy::SecretString;

    use super::*;
    use crate::ports::account_repository::AccountRepositoryError;
    use crate::ports::body::{BodyKind, Link};
    use crate::ports::credential::Credential;
    use crate::ports::credential_store::{CredentialError, CredentialRef};
    use crate::ports::mail_provider::{
        MailFetch, MailProviderError, MailSync, MailWatch, MailboxDiscovery,
    };
    use crate::ports::pictures::FetchedPicture;
    use lyonoyl_domain::account::{Encryption, MailAccount, MailAccountSummary, ServerSettings};

    fn account() -> MailAccount {
        MailAccount::new(
            MailAccountSummary::new("acc-1", "user@example.com", "User", "imap")
                .expect("valid summary"),
            ServerSettings::new("imap.example.com", 993, Encryption::Tls).expect("valid server"),
            "user@example.com",
        )
        .expect("valid account")
    }

    fn uid(value: u32) -> Uid {
        Uid::new(value).expect("non-zero")
    }

    /// The generation the index holds the numbers in these tests under.
    fn stored_generation() -> Option<UidValidity> {
        UidValidity::new(7).ok()
    }

    /// Messages this device already has, and what it was asked to keep.
    #[derive(Default)]
    struct Kept {
        held: Mutex<std::collections::HashMap<u64, Vec<u8>>>,
        keeps: Mutex<Vec<u64>>,
        /// Every message asked to have what is missing about it written down.
        repaired: Mutex<Vec<MessageId>>,
        refuses: bool,
    }

    impl Kept {
        fn holding(message: u64, raw: &[u8]) -> Self {
            Self {
                held: Mutex::new(std::collections::HashMap::from([(message, raw.to_vec())])),
                ..Self::default()
            }
        }
    }

    impl MessageBodies for &Kept {
        fn held(&self, message: MessageId) -> Result<Option<Vec<u8>>, MessageBodyError> {
            if self.refuses {
                return Err(MessageBodyError::Unavailable);
            }
            Ok(self
                .held
                .lock()
                .expect("test lock")
                .get(&message.get())
                .cloned())
        }

        fn fill_in_missing(&self, message: MessageId, _raw: &[u8]) -> Result<(), MessageBodyError> {
            self.repaired.lock().expect("test lock").push(message);

            Ok(())
        }

        fn keep(&self, message: MessageId, raw: &[u8]) -> Result<(), MessageBodyError> {
            if self.refuses {
                return Err(MessageBodyError::Unavailable);
            }
            self.keeps.lock().expect("test lock").push(message.get());
            self.held
                .lock()
                .expect("test lock")
                .insert(message.get(), raw.to_vec());
            Ok(())
        }
    }

    fn message() -> MessageId {
        MessageId::new(7).expect("non-zero")
    }

    #[derive(Default)]
    struct FakeAccounts {
        held: Option<MailAccount>,
    }

    impl FakeAccounts {
        fn holding() -> Self {
            Self {
                held: Some(account()),
            }
        }
    }

    impl AccountRepository for FakeAccounts {
        fn find(&self, _account_id: &str) -> Result<Option<MailAccount>, AccountRepositoryError> {
            Ok(self.held.clone())
        }

        fn able_to_send(&self) -> Result<Vec<String>, AccountRepositoryError> {
            unreachable!("nothing here asks who can send")
        }

        fn list(&self) -> Result<Vec<MailAccountSummary>, AccountRepositoryError> {
            unreachable!("opening a message lists no accounts")
        }

        fn find_by_email(&self, _email: &str) -> Result<Option<String>, AccountRepositoryError> {
            unreachable!("opening a message does not look an account up by address")
        }

        fn save(&self, _account: &MailAccount) -> Result<(), AccountRepositoryError> {
            unreachable!("opening a message stores nothing")
        }

        fn delete(&self, _account_id: &str) -> Result<(), AccountRepositoryError> {
            unreachable!("opening a message removes nothing")
        }

        fn set_collapsed(
            &self,
            _account_id: &str,
            _collapsed: bool,
        ) -> Result<(), AccountRepositoryError> {
            unreachable!("opening a message does not touch the sidebar")
        }

        fn number_of(&self, _: &str) -> Result<Option<i64>, AccountRepositoryError> {
            unreachable!("this fake is not asked which file the mail is in")
        }
    }

    struct FakeCredentials {
        stored: bool,
    }

    impl CredentialStore for FakeCredentials {
        fn read(&self, _reference: &CredentialRef) -> Result<SecretString, CredentialError> {
            match self.stored {
                true => Ok(SecretString::from("correct horse".to_owned())),
                false => Err(CredentialError::NotFound),
            }
        }

        fn save(
            &self,
            _reference: &CredentialRef,
            _secret: &SecretString,
        ) -> Result<(), CredentialError> {
            unreachable!("opening a message stores no secret")
        }

        fn delete(&self, _reference: &CredentialRef) -> Result<(), CredentialError> {
            unreachable!("opening a message removes no secret")
        }
    }

    /// What a server was asked for: the mailbox, the generation, the number.
    type Asked = std::sync::Arc<Mutex<Vec<(String, u32, u32)>>>;

    /// A server holding one message, which remembers what it was asked for.
    struct FakeServer {
        answer: Result<Vec<u8>, MailProviderError>,
        asked: Asked,
    }

    impl MailFetch for FakeServer {
        fn whole_message(
            &self,
            mailbox: &str,
            uid_validity: UidValidity,
            uid: Uid,
        ) -> Result<Vec<u8>, MailProviderError> {
            self.asked.lock().expect("test lock").push((
                mailbox.to_owned(),
                uid_validity.get(),
                uid.get(),
            ));

            self.answer.clone()
        }
    }

    struct FakeFactory {
        answer: Result<Vec<u8>, MailProviderError>,
        asked: Asked,
    }

    impl FakeFactory {
        fn holding(raw: &[u8]) -> Self {
            Self {
                answer: Ok(raw.to_vec()),
                asked: std::sync::Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn failing(error: MailProviderError) -> Self {
            Self {
                answer: Err(error),
                asked: std::sync::Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl ProviderFactory for FakeFactory {
        fn reading(
            &self,
            _account: &MailAccount,
            _credential: &Credential,
        ) -> Result<Box<dyn MailFetch>, MailProviderError> {
            Ok(Box::new(FakeServer {
                answer: self.answer.clone(),
                asked: std::sync::Arc::clone(&self.asked),
            }))
        }

        fn discovery(
            &self,
            _account: &MailAccount,
            _credential: &Credential,
        ) -> Result<Box<dyn MailboxDiscovery>, MailProviderError> {
            unreachable!("opening a message lists no mailboxes")
        }

        fn sync(
            &self,
            _account: &MailAccount,
            _credential: &Credential,
        ) -> Result<Box<dyn MailSync>, MailProviderError> {
            unreachable!("opening a message runs no pass")
        }

        fn watch(
            &self,
            _account: &MailAccount,
            _credential: &Credential,
        ) -> Result<Box<dyn MailWatch>, MailProviderError> {
            unreachable!("opening a message watches nothing")
        }
    }

    /// A renderer that reports what it was handed, so the use case can be watched rather than
    /// trusted not to look at the bytes on the way.
    #[derive(Default)]
    struct FakeRenderer {
        given: Mutex<Vec<Vec<u8>>>,
    }

    impl BodyRenderer for FakeRenderer {
        fn render(&self, raw: &[u8], _viewing: Viewing) -> RenderedBody {
            self.given.lock().expect("test lock").push(raw.to_vec());

            RenderedBody {
                attachments: Vec::new(),
                html: "<p>rendered</p>".to_owned(),
                parts: Vec::new(),
                links: Vec::new(),
                remote: Vec::new(),
                kind: BodyKind::Html,
                remote_blocked: 2,
                remote_not_shown: 0,
                remote_not_had: 0,
                truncated: false,
                declares_colours: false,
            }
        }
    }

    /// A message that points at nothing, which is every message in these tests unless one says
    /// otherwise.
    struct NoPictures;

    impl PictureSource for NoPictures {
        fn fetch(&self, urls: &[String]) -> Vec<PictureOutcome> {
            assert!(urls.is_empty(), "nothing was asked for here");

            Vec::new()
        }
    }

    #[test]
    fn what_the_index_never_wrote_down_is_written_down_when_the_message_is_opened() {
        // The messages this is for are the ones that have been on the device longest, so the one
        // already kept is exactly the case that matters. It costs a read of what is in hand.
        let kept = Kept::holding(message().get(), b"From: someone\r\n\r\nsaid");
        let renderer = FakeRenderer::default();

        OpenMessage::new(
            FakeAccounts::holding(),
            FakeCredentials { stored: true },
            FakeFactory::holding(b"anything"),
            &renderer,
            NoPictures,
            &kept,
        )
        .execute(
            "acc-1",
            message(),
            "INBOX",
            stored_generation(),
            uid(1),
            Viewing::plainly(),
        )
        .expect("the message is read");

        assert_eq!(
            *kept.repaired.lock().expect("test lock"),
            vec![message()],
            "nothing filled in what the index was missing"
        );
    }

    #[test]
    fn a_message_this_device_already_has_is_not_asked_for_again() {
        // The whole point of keeping it. A provider several hundred milliseconds away is several
        // hundred milliseconds of staring at a message that was read yesterday.
        let factory = FakeFactory::holding(b"From: someone\r\n\r\nFrom the server.");
        let asked = std::sync::Arc::clone(&factory.asked);
        let kept = Kept::holding(message().get(), b"From: someone\r\n\r\nFrom this device.");
        let renderer = FakeRenderer::default();

        let body = OpenMessage::new(
            FakeAccounts::holding(),
            FakeCredentials { stored: true },
            factory,
            &renderer,
            NoPictures,
            &kept,
        )
        .execute(
            "acc-1",
            message(),
            "INBOX.Mail",
            stored_generation(),
            uid(42),
            Viewing::plainly(),
        )
        .expect("the message is read");

        assert!(
            asked.lock().expect("test lock").is_empty(),
            "a message already on this device was fetched from the server anyway"
        );
        // The renderer records what it was handed, which is where the difference shows.
        let given = renderer.given.lock().expect("test lock");
        assert_eq!(
            given.len(),
            1,
            "the message was rendered from something other than the copy on this device"
        );
        assert!(String::from_utf8_lossy(&given[0]).contains("From this device."));
        let _ = &body;
    }

    #[test]
    fn a_message_read_for_the_first_time_is_kept() {
        let factory = FakeFactory::holding(b"From: someone\r\n\r\nHello.");
        let kept = Kept::default();

        OpenMessage::new(
            FakeAccounts::holding(),
            FakeCredentials { stored: true },
            factory,
            FakeRenderer::default(),
            NoPictures,
            &kept,
        )
        .execute(
            "acc-1",
            message(),
            "INBOX.Mail",
            stored_generation(),
            uid(42),
            Viewing::plainly(),
        )
        .expect("the message is read");

        assert_eq!(
            *kept.keeps.lock().expect("test lock"),
            vec![message().get()],
            "the message was read and then thrown away"
        );
    }

    #[test]
    fn a_store_that_will_not_answer_is_a_slow_message_rather_than_no_message() {
        // Reading mail must not depend on being able to keep a copy of it.
        let factory = FakeFactory::holding(b"From: someone\r\n\r\nHello.");
        let asked = std::sync::Arc::clone(&factory.asked);
        let kept = Kept {
            refuses: true,
            ..Kept::default()
        };

        let body = OpenMessage::new(
            FakeAccounts::holding(),
            FakeCredentials { stored: true },
            factory,
            FakeRenderer::default(),
            NoPictures,
            &kept,
        )
        .execute(
            "acc-1",
            message(),
            "INBOX.Mail",
            stored_generation(),
            uid(42),
            Viewing::plainly(),
        );

        assert!(
            body.is_ok(),
            "a store that will not open stopped the message being read"
        );
        assert_eq!(asked.lock().expect("test lock").len(), 1);
    }

    #[test]
    fn the_message_is_read_from_where_the_index_says_it_is() {
        let factory = FakeFactory::holding(b"From: someone\r\n\r\nHello.");
        let asked = std::sync::Arc::clone(&factory.asked);

        let body = OpenMessage::new(
            FakeAccounts::holding(),
            FakeCredentials { stored: true },
            factory,
            FakeRenderer::default(),
            NoPictures,
            &Kept::default(),
        )
        .execute(
            "acc-1",
            message(),
            "INBOX.Mail",
            stored_generation(),
            uid(42),
            Viewing::plainly(),
        )
        .expect("the message is read");

        assert_eq!(body.kind, BodyKind::Html);
        assert_eq!(body.remote_blocked, 2);
        // The generation goes with the number: the number means a message in that generation and
        // nowhere else, and the server is asked to still be counting in it.
        assert_eq!(
            *asked.lock().expect("test lock"),
            vec![("INBOX.Mail".to_owned(), 7, 42)]
        );
    }

    #[test]
    fn a_number_with_no_generation_beside_it_is_not_read_by() {
        // Nothing can check such a number against the mailbox, and a mailbox that renumbered has
        // given it to somebody else's message — which would then be kept as this one.
        let factory = FakeFactory::holding(b"From: someone\r\n\r\nNot this message.");
        let asked = std::sync::Arc::clone(&factory.asked);
        let kept = Kept::default();

        let outcome = OpenMessage::new(
            FakeAccounts::holding(),
            FakeCredentials { stored: true },
            factory,
            FakeRenderer::default(),
            NoPictures,
            &kept,
        )
        .execute(
            "acc-1",
            message(),
            "INBOX",
            None,
            uid(1),
            Viewing::plainly(),
        );

        assert!(matches!(outcome, Err(UseCaseError::StorageCorrupted)));
        assert!(
            asked.lock().expect("test lock").is_empty(),
            "the server was asked by a number nothing could check"
        );
        assert!(kept.keeps.lock().expect("test lock").is_empty());
    }

    #[test]
    fn a_message_already_here_opens_without_a_generation() {
        // The generation is what makes asking the server safe. A message this device already holds
        // is not asked for, so it does not need one.
        let kept = Kept::holding(message().get(), b"From: someone\r\n\r\nHeld here.");

        let outcome = OpenMessage::new(
            FakeAccounts::holding(),
            FakeCredentials { stored: true },
            FakeFactory::holding(b"anything"),
            FakeRenderer::default(),
            NoPictures,
            &kept,
        )
        .execute(
            "acc-1",
            message(),
            "INBOX",
            None,
            uid(1),
            Viewing::plainly(),
        );

        assert!(outcome.is_ok(), "a message held here would not open");
    }

    #[test]
    fn what_the_server_sent_is_what_the_renderer_is_given() {
        // Nothing on the way may look at a message, decide what it is, or cut it: the one place
        // that reads hostile bytes is the renderer, on its own thread, behind its own limits.
        let raw = b"From: someone\r\nContent-Type: text/html\r\n\r\n<p>Hello.</p>";
        let renderer = FakeRenderer::default();

        let _ = OpenMessage::new(
            FakeAccounts::holding(),
            FakeCredentials { stored: true },
            FakeFactory::holding(raw),
            &renderer,
            NoPictures,
            &Kept::default(),
        )
        .execute(
            "acc-1",
            message(),
            "INBOX",
            stored_generation(),
            uid(1),
            Viewing::plainly(),
        );

        assert_eq!(
            *renderer.given.lock().expect("test lock"),
            vec![raw.to_vec()]
        );
    }

    /// A renderer that says the message points at two pictures somewhere else.
    struct PointsAtPictures;

    impl BodyRenderer for PointsAtPictures {
        fn render(&self, _raw: &[u8], _viewing: Viewing) -> RenderedBody {
            RenderedBody {
                attachments: Vec::new(),
                html: "<p><img src=\"/one\"><img src=\"/two\"></p>".to_owned(),
                parts: Vec::new(),
                links: Vec::new(),
                remote: vec![
                    Link {
                        name: "one".to_owned(),
                        url: "https://example.net/one.png".to_owned(),
                    },
                    Link {
                        name: "two".to_owned(),
                        url: "https://example.net/two.png".to_owned(),
                    },
                ],
                kind: BodyKind::Html,
                remote_blocked: 0,
                remote_not_shown: 0,
                remote_not_had: 0,
                truncated: false,
                declares_colours: false,
            }
        }
    }

    /// A source that answers for the first address and not for the second.
    struct OneOfThem {
        asked: Mutex<Vec<String>>,
    }

    /// One that arrived and is not shown here, one that never arrived.
    struct OneRefusedOneMissing;

    impl PictureSource for OneRefusedOneMissing {
        fn fetch(&self, urls: &[String]) -> Vec<PictureOutcome> {
            assert_eq!(urls.len(), 2);

            vec![
                PictureOutcome::NotAKindShownHere,
                PictureOutcome::CouldNotBeHad,
            ]
        }
    }

    impl PictureSource for OneOfThem {
        fn fetch(&self, urls: &[String]) -> Vec<PictureOutcome> {
            *self.asked.lock().expect("test lock") = urls.to_vec();

            vec![
                PictureOutcome::Shown(FetchedPicture {
                    content_type: "image/png".to_owned(),
                    bytes: b"the first".to_vec(),
                }),
                PictureOutcome::CouldNotBeHad,
            ]
        }
    }

    #[test]
    fn a_picture_the_message_points_at_is_fetched_and_named_the_way_the_body_names_it() {
        // The frame has no network. What the reader asked for is fetched here and served from
        // beside the body, under the name the body already refers to.
        let pictures = OneOfThem {
            asked: Mutex::new(Vec::new()),
        };

        let body = OpenMessage::new(
            FakeAccounts::holding(),
            FakeCredentials { stored: true },
            FakeFactory::holding(b"anything"),
            PointsAtPictures,
            &pictures,
            &Kept::default(),
        )
        .execute(
            "acc-1",
            message(),
            "INBOX",
            stored_generation(),
            uid(1),
            Viewing::plainly(),
        )
        .expect("the message is read");

        assert_eq!(
            *pictures.asked.lock().expect("test lock"),
            vec![
                "https://example.net/one.png".to_owned(),
                "https://example.net/two.png".to_owned()
            ]
        );
        // One arrived and one did not: a host that is down is not a message that cannot be read.
        assert_eq!(body.parts.len(), 1);
        assert_eq!(body.parts[0].name, "one");
        assert_eq!(body.parts[0].content_type, "image/png");
        assert_eq!(body.parts[0].bytes, b"the first");
    }

    #[test]
    fn a_picture_that_could_not_be_had_is_counted_as_one_that_is_not_shown() {
        // The reader is told what is on screen, not what was attempted. A message that says its
        // pictures were fetched when the host never answered is a message telling someone what was
        // meant to happen.
        let pictures = OneOfThem {
            asked: Mutex::new(Vec::new()),
        };

        let body = OpenMessage::new(
            FakeAccounts::holding(),
            FakeCredentials { stored: true },
            FakeFactory::holding(b"anything"),
            PointsAtPictures,
            &pictures,
            &Kept::default(),
        )
        .execute(
            "acc-1",
            message(),
            "INBOX",
            stored_generation(),
            uid(1),
            Viewing::plainly(),
        )
        .expect("the message is read");

        assert_eq!(body.parts.len(), 1, "the one that arrived is held");
        // Counted where a second press can still change it, apart from the ones nobody asked
        // about: a host that was down a moment ago may not be.
        assert_eq!(body.remote_not_had, 1, "the one that did not is counted");
        assert_eq!(body.remote_blocked, 0, "both were asked about");
    }

    #[test]
    fn a_picture_a_style_drew_and_never_got_stops_being_pointed_at_too() {
        // The same taking-back, one shape along. A picture a style draws is written `url(/name)`
        // and nothing that strips the `<img>` form touches it, so the frame went to the store for
        // a name it does not hold and drew nothing where the message meant a background.
        let mut body = RenderedBody {
            html: "<style>p{background-image:url(/gone)}h1{background-image:url(/here)}</style>"
                .to_owned(),
            remote: vec![
                Link {
                    name: "gone".to_owned(),
                    url: "https://elsewhere.invalid/a.png".to_owned(),
                },
                Link {
                    name: "here".to_owned(),
                    url: "https://elsewhere.invalid/b.png".to_owned(),
                },
            ],
            ..RenderedBody::nothing()
        };

        put_the_pictures_in(
            &mut body,
            vec![
                PictureOutcome::CouldNotBeHad,
                PictureOutcome::Shown(FetchedPicture {
                    content_type: "image/png".to_owned(),
                    bytes: vec![1, 2, 3],
                }),
            ],
        );

        assert!(
            !body.html.contains("url(/gone)"),
            "a picture that never came is still pointed at: {}",
            body.html
        );
        assert!(
            body.html.contains("background-image:none"),
            "the declaration around it was left as something the engine cannot read: {}",
            body.html
        );
        assert!(
            body.html.contains("url(/here)"),
            "the one that arrived stopped being drawn: {}",
            body.html
        );
    }

    #[test]
    fn one_that_arrived_and_is_not_shown_is_counted_apart_from_one_that_never_came() {
        // Both end with nothing on screen, and they are not the same thing to tell somebody. A
        // host that is down may be up in a minute; a drawing this build refuses to render will not
        // become one by being asked for again, and a notice that invites a retry wastes the time
        // of the person reading it.
        let body = OpenMessage::new(
            FakeAccounts::holding(),
            FakeCredentials { stored: true },
            FakeFactory::holding(b"anything"),
            PointsAtPictures,
            &OneRefusedOneMissing,
            &Kept::default(),
        )
        .execute(
            "acc-1",
            message(),
            "INBOX",
            stored_generation(),
            uid(1),
            Viewing::plainly(),
        )
        .expect("the message is read");

        assert!(body.parts.is_empty(), "neither is on screen");
        assert_eq!(body.remote_not_had, 1, "the one that never came");
        assert_eq!(body.remote_not_shown, 1, "the one that came and is refused");
        assert_eq!(body.remote_blocked, 0, "both were asked about");
    }

    #[test]
    fn an_account_that_is_not_on_this_device_is_refused() {
        let outcome = OpenMessage::new(
            FakeAccounts::default(),
            FakeCredentials { stored: true },
            FakeFactory::holding(b"anything"),
            FakeRenderer::default(),
            NoPictures,
            &Kept::default(),
        )
        .execute(
            "acc-1",
            message(),
            "INBOX",
            stored_generation(),
            uid(1),
            Viewing::plainly(),
        );

        assert!(matches!(outcome, Err(UseCaseError::UnknownAccount)));
    }

    #[test]
    fn a_missing_password_is_reported_as_itself() {
        let outcome = OpenMessage::new(
            FakeAccounts::holding(),
            FakeCredentials { stored: false },
            FakeFactory::holding(b"anything"),
            FakeRenderer::default(),
            NoPictures,
            &Kept::default(),
        )
        .execute(
            "acc-1",
            message(),
            "INBOX",
            stored_generation(),
            uid(1),
            Viewing::plainly(),
        );

        assert!(matches!(outcome, Err(UseCaseError::CredentialsMissing)));
    }

    #[test]
    fn a_server_that_cannot_be_reached_keeps_its_meaning() {
        // "Offline" sends the reader somewhere different from "this message is not there".
        let outcome = OpenMessage::new(
            FakeAccounts::holding(),
            FakeCredentials { stored: true },
            FakeFactory::failing(MailProviderError::Offline),
            FakeRenderer::default(),
            NoPictures,
            &Kept::default(),
        )
        .execute(
            "acc-1",
            message(),
            "INBOX",
            stored_generation(),
            uid(1),
            Viewing::plainly(),
        );

        assert!(matches!(outcome, Err(UseCaseError::Offline)));
    }
}
