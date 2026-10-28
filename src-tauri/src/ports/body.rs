//! What a message body looks like once it is safe to show.
//!
//! A port rather than a type of the sanitizer: the use case that opens a message decides what to do
//! with a body, and what turns bytes into one is an adapter it never names. The result of rendering
//! never crosses IPC as a string — what crosses is the address it is served at.

/// What a message turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyKind {
    /// HTML the message carried, sanitized.
    Html,
    /// Text the message carried, escaped into a document.
    PlainText,
    /// Nothing that could be shown: no body, or one this build refused.
    Nothing,
    /// Not looked at yet — every place this build reads a message in was taken. Different from
    /// `Nothing` on purpose: that one is about the message and this one is about the moment, and
    /// saying the first when the second happened sends a person to argue with a message that is
    /// fine.
    Busy,
}

/// A picture the message carried inside itself.
///
/// Kept apart from the body and served beside it, under a name that means nothing outside the
/// moment the message is open. Nothing here is fetched from anywhere: these bytes arrived in the
/// message, and that is why they can be shown while remote pictures cannot.
#[derive(Debug, Clone)]
pub struct InlinePart {
    /// What the body refers to it by, and what fetches it.
    pub name: String,
    /// The type it is served as — one this build is willing to serve, never the one the message
    /// claimed.
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// A file the message carried, as it is listed before anybody asks for it.
///
/// Not the type the message declares for it, which nothing reads: the window names a file's kind
/// by its name, which is what the system goes by once it is saved, and nothing is opened here,
/// only saved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attached {
    /// Where it sits in the message, so it can be found again without holding it.
    ///
    /// A position, not a name: two parts may carry the same filename, and a message is free to
    /// give none at all.
    pub at: u32,
    /// What to call it — already made safe to hand to a save panel, never the raw string the
    /// message carried.
    pub name: String,
    pub bytes: u64,
}

/// A link the message carried, kept out of the body and behind a name.
///
/// The body never holds the address: what it holds is a name that means nothing outside the moment
/// the message is open, and following it reaches this application rather than the network.
#[derive(Debug, Clone)]
pub struct Link {
    pub name: String,
    pub url: String,
}

/// A body ready to be served.
#[derive(Debug, Clone)]
pub struct RenderedBody {
    pub html: String,
    /// The pictures the body refers to, which travelled inside the message.
    pub parts: Vec<InlinePart>,
    /// The files the message carried that the body does not refer to.
    ///
    /// Listed rather than held: what is here is a name and a size, and the bytes are fetched only
    /// when somebody asks to save one. A message with four photographs in it is four photographs
    /// nobody has asked for yet.
    pub attachments: Vec<Attached>,
    /// The links the body offers, each behind a name.
    pub links: Vec<Link>,
    /// The pictures the body points at that live somewhere else, each already behind a name the
    /// body refers to. Empty unless the reader asked for them; the bytes are fetched separately
    /// and put under these names.
    pub remote: Vec<Link>,
    pub kind: BodyKind,
    /// How many pictures the message wanted fetched from somewhere else.
    pub remote_blocked: u32,
    /// How many arrived and are not of a kind this build shows. Kept apart from the count above:
    /// one of them may be there next time, and one of these never will.
    pub remote_not_shown: u32,
    /// How many were asked for and did not arrive: the host refused, or did not answer, or
    /// answered with something that was not a picture.
    ///
    /// Kept apart from `remote_blocked`, which counts the ones this build never asked about at
    /// all -- an address it does not fetch, or a picture the message says travels inside it and
    /// then does not carry. The two are one sentence to a reader and two different faults to
    /// anybody trying to find out why a message has no pictures in it, and telling them apart is
    /// the difference between "the host said no" and "we never asked".
    pub remote_not_had: u32,
    /// Whether what is shown is the beginning of the body rather than all of it.
    pub truncated: bool,
    /// Whether the message says anything about colour itself.
    ///
    /// What decides which surface it is shown on. A message that names no colour can wear the
    /// application's, and looks like part of it. A message that names its own was written against
    /// a light background — a newsletter that sets grey text and no background assumes white — so
    /// it is shown on one, and stays readable instead of becoming dark on dark.
    pub declares_colours: bool,
}

impl RenderedBody {
    pub fn nothing() -> Self {
        Self {
            attachments: Vec::new(),
            html: String::new(),
            parts: Vec::new(),
            links: Vec::new(),
            remote: Vec::new(),
            kind: BodyKind::Nothing,
            remote_blocked: 0,
            remote_not_shown: 0,
            remote_not_had: 0,
            truncated: false,
            declares_colours: false,
        }
    }

    /// Nowhere to read it just now. Nothing about the message is known yet, and asking again when
    /// the machine is quieter is worth doing — which is the whole difference from `nothing`.
    pub fn busy() -> Self {
        Self {
            kind: BodyKind::Busy,
            ..Self::nothing()
        }
    }
}

/// How a message is being looked at.
///
/// A message that renders as almost nothing is not a bug to hide: mail is written by people and by
/// programs, and some of it is malformed, and some of it is an attack whose whole shape is markup
/// that means something else when it is read again. The reader is given a way to look anyway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyForm {
    /// As it was written: the HTML part, sanitized, or the text part when there is no HTML.
    Sent,
    /// The text part alone, escaped. What a message looks like to a client that shows no HTML.
    Text,
    /// The message itself, headers and all, as text. The last resort, and the honest one.
    Source,
}

/// Whether the pictures a message points at may be fetched.
///
/// Off unless the reader said so, and said so for this message: a picture fetched from a host is
/// that host being told the message was opened, by whom, and when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pictures {
    Blocked,
    Shown,
}

/// How a message is being looked at, in full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewing {
    pub form: BodyForm,
    pub pictures: Pictures,
}

impl Viewing {
    /// How a message is looked at until the reader asks for anything else.
    ///
    /// Only tests build one this way: what a reader is shown before they ask for anything is
    /// decided where the reader is, and every path into here carries an answer already.
    #[cfg(test)]
    pub fn plainly() -> Self {
        Self {
            form: BodyForm::Sent,
            pictures: Pictures::Blocked,
        }
    }
}

/// Turning the bytes of a message into something that can be shown.
pub trait BodyRenderer: Send + Sync {
    fn render(&self, raw: &[u8], viewing: Viewing) -> RenderedBody;
}

/// A renderer reached through a reference is still a renderer. Lets one be handed to a use case
/// without giving it away.
impl<B: BodyRenderer + ?Sized> BodyRenderer for &B {
    fn render(&self, raw: &[u8], viewing: Viewing) -> RenderedBody {
        (**self).render(raw, viewing)
    }
}
