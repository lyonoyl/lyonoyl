/// Taking what a message carries out of one that has already been fetched.
///
/// A port because parsing MIME is somebody else's job: the use case knows it wants the third part
/// of a message, and nothing about boundaries, transfer encodings or the four ways a filename can
/// be spelled.
///
/// No error type. A part that is not there is not a failure — a message can be re-fetched between
/// the listing and the asking, and what was third a moment ago may be second now. The caller says
/// so to the person rather than reporting a fault.
pub trait MessageParts: Send + Sync {
    /// The name to offer and the bytes, or nothing if that part is not a file.
    ///
    /// The name is already safe to hand to a save panel: a filename in a message is written by
    /// whoever sent it, and this is the last place before a dialog sees it.
    fn file_at(&self, raw: &[u8], at: u32) -> Option<(String, Vec<u8>)>;

    /// The message as it is passed on to somebody else, or nothing when there was nowhere to read
    /// it: every place a message is read in was taken, and asking again later is a different
    /// question rather than the same one repeated.
    ///
    /// A message read and found to say nothing — one past what this build reads, or one that is not
    /// a message at all — is passed on as nothing, which is what the reader shows of it too.
    fn passed_on(&self, raw: &[u8]) -> Option<PassedOn>;
}

/// What a message passed on carries with it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassedOn {
    /// What it says, as text, with its lines as it wrote them. Empty when it says nothing this
    /// build reads as text.
    pub text: String,
    /// Every file it carries, under names already made safe, in the order it carries them: the
    /// ones beside its text, and the pictures inside it, which a message passed on as text no
    /// longer draws.
    pub files: Vec<(String, Vec<u8>)>,
}
