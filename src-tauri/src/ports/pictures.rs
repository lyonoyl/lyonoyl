//! Fetching the pictures a message points at.
//!
//! A port because the use case decides *whether* to fetch — the reader asked — and knows nothing
//! about how. What is on the other side is one service over the application's HTTP client; what
//! would be on the other side in a test is a list of answers.

/// A picture that was fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedPicture {
    /// The type it will be served as. Decided by the build, never by the host.
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// What became of one address.
///
/// Two ways of not being on screen, kept apart because they are different sentences to a reader.
/// A host that is down may be up in a minute; a kind of picture this build does not show will not
/// become one. Telling someone the first when it was the second is telling them to try again at
/// something that cannot succeed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PictureOutcome {
    /// Fetched, and of a kind this build puts in front of a reader.
    Shown(FetchedPicture),
    /// Fetched, and not of a kind this build draws.
    NotAKindShownHere,
    /// Not had at all: the host was unreachable, refused, too slow, answered with something that
    /// was not the picture, or the message asked for more than this build fetches.
    CouldNotBeHad,
}

pub trait PictureSource: Send + Sync {
    /// Fetches every address, together.
    ///
    /// What comes back lines up with what went in, one for one.
    fn fetch(&self, urls: &[String]) -> Vec<PictureOutcome>;
}

impl<S: PictureSource + ?Sized> PictureSource for &S {
    fn fetch(&self, urls: &[String]) -> Vec<PictureOutcome> {
        (**self).fetch(urls)
    }
}
