use super::opens::*;

/// The first bytes each kind is written with, as a program that writes one starts it.
fn starting(kind: OpensAs) -> Vec<u8> {
    match kind {
        OpensAs::Pdf => b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n1 0 obj".to_vec(),
        OpensAs::Png => b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec(),
        OpensAs::Jpeg => b"\xFF\xD8\xFF\xE0\0\x10JFIF\0".to_vec(),
        OpensAs::Gif => b"GIF89a\x01\0\x01\0".to_vec(),
        OpensAs::WebP => b"RIFF\x24\0\0\0WEBPVP8 ".to_vec(),
        OpensAs::Heic => b"\0\0\0\x18ftypheic\0\0\0\0mif1heic".to_vec(),
        OpensAs::Tiff => b"II*\0\x08\0\0\0".to_vec(),
    }
}

const EVERY_KIND: [OpensAs; 7] = [
    OpensAs::Pdf,
    OpensAs::Png,
    OpensAs::Jpeg,
    OpensAs::Gif,
    OpensAs::WebP,
    OpensAs::Heic,
    OpensAs::Tiff,
];

#[test]
fn every_kind_that_opens_is_read_from_its_first_bytes() {
    for kind in EVERY_KIND {
        assert_eq!(OpensAs::of(&starting(kind)), Some(kind), "{kind:?}");
    }

    // The other spellings each kind is written with.
    assert_eq!(OpensAs::of(b"GIF87a\x01\0"), Some(OpensAs::Gif));
    assert_eq!(OpensAs::of(b"MM\0*\0\0\0\x08"), Some(OpensAs::Tiff));
    assert_eq!(
        OpensAs::of(b"\0\0\0\x1cftypmif1\0\0\0\0mif1heic"),
        Some(OpensAs::Heic)
    );
    assert_eq!(OpensAs::of(b"\xFF\xD8\xFF\xDB\0\x43"), Some(OpensAs::Jpeg));
}

#[test]
fn what_runs_or_holds_other_files_does_not_open() {
    // Every one of these is a file a message carries every day, and each is a program, a script,
    // or a box of other files: opened, it is the system deciding what happens next.
    for (what, bytes) in [
        ("a Windows program", b"MZ\x90\0\x03\0\0\0".as_slice()),
        ("a Mac program", b"\xCF\xFA\xED\xFE\x0C\0\0\x01"),
        ("a Linux program", b"\x7FELF\x02\x01\x01\0"),
        ("a script", b"#!/bin/sh\necho\n"),
        (
            "an archive, or a document of Office",
            b"PK\x03\x04\x14\0\x06\0",
        ),
        (
            "an old document of Office",
            b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1",
        ),
        ("a page", b"<!DOCTYPE html><html>"),
        ("a drawing", b"<svg viewBox=\"0 0 1 1\">"),
        ("a disk image", b"\x78\x01\x73\x0D\x62\x62\x60"),
        ("a film", b"\0\0\0\x18ftypisom\0\0\x02\0"),
        ("a film from a camera", b"\0\0\0\x14ftypqt  \0\0\0\0"),
        ("a sound", b"RIFF\x24\0\0\0WAVEfmt "),
        (
            "a picture of the next generation",
            b"\0\0\0\x1cftypavif\0\0\0\0",
        ),
    ] {
        assert_eq!(OpensAs::of(bytes), None, "{what}");
    }
}

#[test]
fn a_header_almost_right_or_not_at_the_start_does_not_open() {
    for (what, bytes) in [
        ("nothing", b"".as_slice()),
        ("one byte of a picture", b"\xFF"),
        ("the start of a picture and no more", b"\xFF\xD8"),
        ("a PDF without its dash", b"%PDF1.7"),
        ("a PDF after a space", b" %PDF-1.7"),
        ("a PDF after a line", b"\n%PDF-1.7"),
        ("a PDF in lower case", b"%pdf-1.7"),
        ("a GIF of a version there never was", b"GIF88a\x01\0"),
        ("a PNG cut short", b"\x89PNG\r\n"),
        ("a RIFF that is not a picture", b"RIFF\x24\0\0\0AVI LIST"),
        ("a RIFF too short to say", b"RIFF\x24\0"),
        ("a box that is not the first one", b"\0\0\0\x18moovheic"),
        ("a brand cut short", b"\0\0\0\x18ftyphei"),
        ("half a TIFF from each byte order", b"II\0*"),
    ] {
        assert_eq!(OpensAs::of(bytes), None, "{what}");
    }
}

#[test]
fn a_name_that_already_ends_as_its_bytes_say_keeps_its_ending() {
    assert_eq!(opened_name("Invoice.pdf", OpensAs::Pdf), "Invoice.pdf");
    assert_eq!(opened_name("Invoice.PDF", OpensAs::Pdf), "Invoice.PDF");
    assert_eq!(opened_name("holiday.jpeg", OpensAs::Jpeg), "holiday.jpeg");
    assert_eq!(opened_name("IMG_0001.HEIC", OpensAs::Heic), "IMG_0001.HEIC");
    assert_eq!(opened_name("scan.tif", OpensAs::Tiff), "scan.tif");
}

#[test]
fn any_other_name_is_given_the_ending_its_bytes_say_after_its_own() {
    // After its own rather than in its place: the title a person reads in the viewer still says
    // what the message called the file.
    assert_eq!(opened_name("scan", OpensAs::Pdf), "scan.pdf");
    assert_eq!(opened_name("invoice.exe", OpensAs::Pdf), "invoice.exe.pdf");
    assert_eq!(opened_name("photo.png", OpensAs::Jpeg), "photo.png.jpg");
    assert_eq!(
        opened_name("Statement.pdf.app", OpensAs::Pdf),
        "Statement.pdf.app.pdf"
    );
    assert_eq!(opened_name("pdf", OpensAs::Pdf), "pdf.pdf");

    for kind in EVERY_KIND {
        assert!(
            opened_name("file", kind).ends_with(&format!(".{}", kind.ending())),
            "{kind:?}"
        );
    }
}

#[test]
fn a_long_name_is_cut_on_a_character_and_keeps_its_ending() {
    // A hundred and twenty characters of four bytes each, which is what the offered name may be.
    let long: String = "🐈".repeat(120);
    let made = opened_name(&long, OpensAs::Pdf);

    assert!(made.len() <= LONGEST_OPENED, "{} bytes", made.len());
    assert!(made.ends_with(".pdf"));
    assert!(made.starts_with("🐈"));

    // Cut just after a space, the space is not left before the ending.
    let spaced = format!("{} {}.pdf", "a".repeat(LONGEST_OPENED - 5), "b".repeat(10));
    let made = opened_name(&spaced, OpensAs::Pdf);
    assert_eq!(made, format!("{}.pdf", "a".repeat(LONGEST_OPENED - 5)));
}

#[test]
fn a_name_with_nothing_left_before_its_ending_is_called_as_an_unnamed_file_is() {
    assert_eq!(opened_name(" .pdf", OpensAs::Pdf), "attachment.pdf");
    assert_eq!(opened_name("...", OpensAs::Png), "attachment.png");
}

#[test]
fn a_cut_that_would_leave_a_name_windows_keeps_for_a_device_leaves_none() {
    // `CON`, then ideographic spaces, three bytes each, then the rest: whole, a name like any other;
    // cut to fit and trimmed, the console device, which Windows opens instead of making a file.
    for device in ["CON", "COM1", "COM\u{00B9}", "NUL"] {
        let offered = format!("{device}{}x", "\u{3000}".repeat(65));
        assert_eq!(
            opened_name(&offered, OpensAs::Pdf),
            "attachment.pdf",
            "{device}"
        );
    }

    // Nor one of nothing but the marks a name is made safe with.
    let marks = format!("_{}x", "\u{3000}".repeat(65));
    assert_eq!(opened_name(&marks, OpensAs::Pdf), "attachment.pdf");
}

#[test]
fn a_picture_in_heif_that_says_it_is_avif_does_not_open_as_heic() {
    // The major brand says only "a picture in HEIF"; the brands after it say what kind.
    let avif = b"\0\0\0\x20ftypmif1\0\0\0\0mif1avifmiaf\0\0\0\0";
    assert_eq!(OpensAs::of(avif), None);

    let heic = b"\0\0\0\x18ftypmif1\0\0\0\0mif1heic";
    assert_eq!(OpensAs::of(heic), Some(OpensAs::Heic));
}

#[test]
fn a_first_box_whose_size_is_nought_or_one_is_read_as_far_as_it_goes() {
    // Nought: the box runs to the end of the file, and the brand that says AVIF is in it.
    assert_eq!(OpensAs::of(b"\0\0\0\0ftypmif1\0\0\0\0mif1avif"), None);
    assert_eq!(
        OpensAs::of(b"\0\0\0\0ftypmif1\0\0\0\0mif1heic"),
        Some(OpensAs::Heic)
    );

    // One: the real size follows in 64 bits, which is not read, and nothing is assumed of it.
    assert_eq!(
        OpensAs::of(b"\0\0\0\x01ftypmif1\0\0\0\0\0\0\0\x20mif1heic"),
        None
    );
}

#[test]
fn a_raw_picture_from_a_camera_does_not_open_as_a_tiff() {
    // Canon's says so in its bytes.
    assert_eq!(OpensAs::of(b"II*\0\x10\0\0\0CR\x02\0"), None);

    // The others are TIFFs to their bytes and say what they are only in their names.
    let tiff = b"II*\0\x08\0\0\0";
    for name in [
        "DSC_0001.NEF",
        "IMG_0001.dng",
        "A7R.arw",
        "P1.orf",
        "K1.pef",
    ] {
        assert_eq!(opens_as(name, tiff), None, "{name}");
    }
    assert_eq!(opens_as("scan.tiff", tiff), Some(OpensAs::Tiff));
    assert_eq!(opens_as("scan", tiff), Some(OpensAs::Tiff));
}

#[test]
fn a_name_takes_opening_away_and_never_gives_it() {
    let pdf = b"%PDF-1.7";

    // A message carried inside another is saved as one, whatever its bytes begin with.
    assert_eq!(opens_as("About Tuesday.eml", pdf), None);
    assert_eq!(opens_as("Statement.pdf", pdf), Some(OpensAs::Pdf));
    assert_eq!(opens_as("Statement.exe", pdf), Some(OpensAs::Pdf));

    // A name that promises a picture opens nothing that is not one.
    assert_eq!(opens_as("photo.jpg", b"MZ\x90\0"), None);
    assert_eq!(opens_as("photo.dng", b"MZ\x90\0"), None);
}
