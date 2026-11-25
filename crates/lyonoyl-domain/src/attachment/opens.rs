//! Which files a message carries are handed to the system to open, and the name the copy goes by.
//!
//! A file in a message is the part of it that leaves this application: opened, it is read by
//! another program, with whatever that program can do. So a file opens only when its own first
//! bytes say it is a document of pages or a picture, which every system shows in a viewer of its
//! own, and never because of the name or the type the message gave it: both are written by
//! whoever sent it. A name only ever takes opening away, never gives it ([`opens_as`]). Every
//! other file is saved.
//!
//! The copy is named for what its bytes are. The system chooses the program by the name's ending,
//! so a file opened as a PDF ends in `.pdf`, and the program the system opens it with is the one
//! it opens a PDF with, whatever else the bytes after the first few might also read as.

/// The kinds of file this application asks the system to open, read from a file's first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpensAs {
    Pdf,
    Png,
    Jpeg,
    Gif,
    WebP,
    /// The pictures an iPhone takes, and every HEIF picture beside them that does not say it is
    /// AVIF.
    Heic,
    Tiff,
}

/// The brands an HEIF picture names first in its `ftyp` box: the HEVC ones a camera writes, and
/// the two that say only "a picture in HEIF", which a viewer of pictures reads as well.
const HEIF_BRANDS: [&[u8]; 10] = [
    b"heic", b"heix", b"heim", b"heis", b"hevc", b"hevx", b"hevm", b"hevs", b"mif1", b"msf1",
];

/// The brands that make a picture in HEIF an AVIF one, which a viewer of HEIC pictures is not.
const AVIF_BRANDS: [&[u8]; 2] = [b"avif", b"avis"];

/// The endings of the raw pictures cameras write as TIFFs: to their first bytes they are pictures
/// of that kind, and a viewer of TIFFs shows them as no picture their owner meant.
const RAW_ENDINGS: [&str; 18] = [
    "3fr", "arw", "dcr", "dng", "erf", "iiq", "kdc", "mef", "mos", "nef", "nrw", "orf", "pef",
    "rw2", "rwl", "sr2", "srf", "srw",
];

/// Whether an `ftyp` box names AVIF among the brands after its major one, as far as the box and
/// the bytes go — or cannot be read far enough to say it does not.
///
/// A size of nought is the box running to the end of the file, and a size of one says the real
/// size follows in 64 bits, which this does not read: such a box is taken as saying it, and the
/// file is saved rather than opened. The brands are the sender's word, as a name is: a picture in
/// AV1 that does not list them is a picture a viewer of HEIF reads as well.
fn says_avif(bytes: &[u8]) -> bool {
    let Some(size) = bytes
        .get(0..4)
        .map(|size| u32::from_be_bytes([size[0], size[1], size[2], size[3]]) as usize)
    else {
        return true;
    };
    let end = match size {
        0 => bytes.len(),
        1 => return true,
        size => size.min(bytes.len()),
    };

    (16..end)
        .step_by(4)
        .filter_map(|at| bytes.get(at..at + 4))
        .any(|brand| AVIF_BRANDS.contains(&brand))
}

/// What a file opens as, given the name it is offered under and its bytes: what its first bytes
/// say, unless its name says it is something those bytes would misname.
///
/// A camera's raw picture is a TIFF to its first bytes and is named for its camera; a message
/// carried inside another is named `.eml` and is saved as one. The name only ever takes opening
/// away: a file whose bytes are not one of the kinds that open never opens, whatever it is called.
pub fn opens_as(name: &str, bytes: &[u8]) -> Option<OpensAs> {
    let kind = OpensAs::of(bytes)?;
    let ending = name.rsplit_once('.').map_or("", |(_, ending)| ending);

    let misnamed = ending.eq_ignore_ascii_case("eml")
        || (kind == OpensAs::Tiff
            && RAW_ENDINGS
                .iter()
                .any(|raw| raw.eq_ignore_ascii_case(ending)));

    (!misnamed).then_some(kind)
}

/// The longest name a copy is written under, in bytes.
///
/// Filesystems count bytes, 255 on APFS, ext4 and NTFS, and the name offered is counted in
/// characters, any of which may take four. Well under the limit, so no name a message carries is
/// one the copy cannot be written under.
pub const LONGEST_OPENED: usize = 200;

impl OpensAs {
    /// What the bytes are, among the kinds that open, or nothing.
    ///
    /// Each kind is read from its header at the very start, and a picture in HEIF from the brands
    /// its first box lists as well: nothing that starts otherwise is opened. A PDF with something
    /// in front of its header is a PDF some viewers read anyway, and it is saved rather than
    /// opened.
    pub fn of(bytes: &[u8]) -> Option<Self> {
        let starts = |with: &[u8]| bytes.starts_with(with);
        let at = |range: std::ops::Range<usize>| bytes.get(range);

        if starts(b"%PDF-") {
            Some(Self::Pdf)
        } else if starts(b"\x89PNG\r\n\x1a\n") {
            Some(Self::Png)
        } else if starts(b"\xFF\xD8\xFF") {
            Some(Self::Jpeg)
        } else if starts(b"GIF87a") || starts(b"GIF89a") {
            Some(Self::Gif)
        } else if starts(b"RIFF") && at(8..12) == Some(b"WEBP".as_slice()) {
            Some(Self::WebP)
        } else if at(4..8) == Some(b"ftyp".as_slice())
            && at(8..12).is_some_and(|brand| HEIF_BRANDS.contains(&brand))
            && !says_avif(bytes)
        {
            Some(Self::Heic)
        } else if (starts(b"II*\0") || starts(b"MM\0*")) && at(8..10) != Some(b"CR".as_slice()) {
            // A Canon camera's raw picture is a TIFF to its first four bytes and marks itself
            // right after them.
            Some(Self::Tiff)
        } else {
            None
        }
    }

    /// The ending a copy of this kind is written under, which is what the system opens it by.
    pub fn ending(self) -> &'static str {
        self.endings()[0]
    }

    /// Every ending a file of this kind is already called by, the one a copy is given first.
    fn endings(self) -> &'static [&'static str] {
        match self {
            Self::Pdf => &["pdf"],
            Self::Png => &["png"],
            Self::Jpeg => &["jpg", "jpeg", "jpe"],
            Self::Gif => &["gif"],
            Self::WebP => &["webp"],
            Self::Heic => &["heic", "heif", "hif"],
            Self::Tiff => &["tiff", "tif"],
        }
    }
}

/// The name a copy of a file is opened under: the name it is offered under, ending as its bytes
/// say.
///
/// `offered` is a name already made safe, by [`super::safe_name`]. A name that ends as its kind
/// does keeps its ending, in any case; any other is given the ending after its own rather than in
/// its place, so the name a person reads in the viewer's title still says what the message called
/// the file: `scan` opens as `scan.pdf`, and `invoice.exe` holding a PDF as `invoice.exe.pdf`.
///
/// Cut to [`LONGEST_OPENED`] bytes on a character's edge, ending kept, and never ending in a dot
/// or a space before the ending, which Windows takes off.
pub fn opened_name(offered: &str, kind: OpensAs) -> String {
    let (stem, ending) = match offered.rsplit_once('.') {
        Some((stem, ending))
            if !stem.is_empty()
                && kind
                    .endings()
                    .iter()
                    .any(|one| one.eq_ignore_ascii_case(ending)) =>
        {
            (stem, ending)
        }
        _ => (offered, kind.ending()),
    };

    let room = LONGEST_OPENED - ending.len() - 1;
    let mut cut = stem.len().min(room);
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    let stem = stem[..cut]
        .trim_end_matches(|character: char| character == '.' || character.is_whitespace());

    // What `safe_name` asks after its own cut, asked again after this one: a cut and a trim can
    // make a name Windows keeps for a device, or one of nothing but marks, out of a name that was
    // neither — `CON` followed by spaces and then the rest.
    let stem = if stem.is_empty()
        || super::reserved_on_windows(stem)
        || stem
            .chars()
            .all(|character| character == '_' || character.is_whitespace())
    {
        super::UNNAMED
    } else {
        stem
    };

    format!("{stem}.{ending}")
}
