/// Taking what a message carries out of one that has already been fetched.
///
/// A port because parsing MIME is somebody else's job: the use case knows it wants the third part
/// of a message, and nothing about boundaries, transfer encodings or the four ways a filename can
/// be spelled.
///
/// A part that is not there is not a failure — a message can be re-fetched between the listing and
/// the asking, and what was third a moment ago may be second now. The caller says so to the person
/// rather than reporting a fault.
pub trait MessageParts: Send + Sync {
    /// The file at `at`, or nothing if that part is not a file.
    ///
    /// The message is read as it is for showing it, within the same bounds. [`NotReadNow`] is the
    /// moment, not the message, and it is an error here where `passed_on` says the same with
    /// nothing: here nothing already means that the message holds no such file, which is a
    /// different thing to tell a person.
    fn file_at(&self, raw: &[u8], at: u32) -> Result<Option<FileTakenOut>, NotReadNow>;

    /// The message as it is passed on to somebody else, or nothing when there was nowhere to read
    /// it: every place a message is read in was taken, and asking again later is a different
    /// question rather than the same one repeated.
    ///
    /// A message read and found to say nothing — one past what this build reads, or one that is not
    /// a message at all — is passed on as nothing, which is what the reader shows of it too.
    fn passed_on(&self, raw: &[u8]) -> Option<PassedOn>;
}

/// One file taken out of a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTakenOut {
    /// The name to offer, already safe to hand to a save panel: a filename in a message is written
    /// by whoever sent it, and this is the last place before a dialog sees it. A message carried
    /// inside this one is named as the listing names it.
    pub name: String,
    pub bytes: Vec<u8>,
    /// Whether it is a message carried inside this one, which is saved as a message and never
    /// opened, whatever its bytes begin with and whatever it is called.
    pub a_message: bool,
}

/// The message was not read this time: every place a message is read in was taken, or reading it
/// ran past what this build allows. Nothing is known about the file, which is not the same as
/// knowing the message holds none: asking again when the machine is quieter is a different
/// question rather than the same one repeated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotReadNow;

/// What a message passed on carries with it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PassedOn {
    /// What goes into the field: what it says, as text, with its lines as it wrote them. Empty when
    /// it says nothing this build reads as text, and for a message passed on in its own form, whose
    /// text `original` holds.
    pub text: String,
    /// Every file it carries, under names already made safe, in the order it carries them: the
    /// ones beside its text, and the pictures inside it that its markup does not draw where it
    /// goes — every one of them, for a message passed on as text, which draws nothing.
    pub files: Vec<(String, Vec<u8>)>,
    /// The message cut down to what draws it, for passing it on in its own form. Nothing for a
    /// message with no markup to draw, or none this build reads into a tree.
    pub original: Option<lyonoyl_domain::outgoing::Original>,
}
