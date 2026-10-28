use super::*;

#[test]
fn a_name_that_reaches_out_of_a_directory_stops_being_one() {
    // The whole reason this function exists. A save panel handed this is a save panel doing what
    // it was told.
    for reaching in [
        "../../.ssh/authorized_keys",
        "..\\..\\Windows\\System32\\drivers\\etc\\hosts",
        "/etc/passwd",
        "C:\\Windows\\notepad.exe",
    ] {
        let made = safe_name(reaching);

        assert!(!made.contains('/'), "{reaching} -> {made}");
        assert!(!made.contains('\\'), "{reaching} -> {made}");
        assert!(!made.contains(':'), "{reaching} -> {made}");
        assert!(!made.contains(".."), "{reaching} -> {made}");
    }
}

#[test]
fn the_separator_a_mac_once_used_is_a_separator_here_too() {
    // Named on its own because the drive letter above hid it: `C:\Windows\notepad.exe` loses its
    // backslashes whatever happens to the colon, so every assertion there passed with the colon
    // left in. On a system where `:` divides one directory from another, that is a name reaching
    // out of wherever the dialog started.
    assert_eq!(
        safe_name("Macintosh HD:Users:notes.txt"),
        "Macintosh HD_Users_notes.txt"
    );
}

#[test]
fn a_name_that_ends_early_in_one_layer_and_not_another_is_refused_its_trick() {
    // A null ends a C string. A name that is "report.pdf" to one layer and "report.pdf\0.exe" to
    // the next is two names, and the second is the sender's.
    let made = safe_name("report.pdf\u{0}.exe");

    assert!(!made.contains('\u{0}'));
    assert_eq!(made, "report.pdf.exe");
}

#[test]
fn a_name_that_would_hide_the_file_does_not() {
    // Hidden on two of the three systems this ships on. Somebody who saved a file and cannot find
    // it has lost it.
    assert_eq!(safe_name(".bashrc"), "bashrc");
    assert_eq!(safe_name("...hidden"), "hidden");
}

#[test]
fn a_space_between_the_dots_does_not_keep_one_of_them() {
    // Dots taken off first and spaces after them uncover a dot and leave it: `. .zshrc` would be
    // offered as `.zshrc`, hidden after all, with a space of any kind standing in the gap.
    assert_eq!(safe_name(". .zshrc"), "zshrc");
    assert_eq!(safe_name(" . .zshrc"), "zshrc");
    assert_eq!(safe_name(". . .zshrc"), "zshrc");
    assert_eq!(safe_name(".\u{00A0}.zshrc"), "zshrc");
    assert_eq!(safe_name("\u{3000}.zshrc"), "zshrc");
}

#[test]
fn a_space_after_the_last_dot_does_not_keep_it() {
    // The same order at the other end: the dot Windows strips, uncovered by a space the first
    // pass did not take, is a name that is two names for one file there.
    assert_eq!(safe_name("report.txt.\u{00A0}"), "report.txt");
    assert_eq!(safe_name("report.txt. \u{3000}"), "report.txt");
    assert_eq!(safe_name("report.txt .\u{2003}."), "report.txt");
}

#[test]
fn a_cut_that_lands_beside_a_space_does_not_keep_the_dot_before_it() {
    // The cut is trimmed by the same rule: landing on a dot and a no-break space, it would
    // otherwise end in the dot.
    let landing = format!(
        "{}.\u{00A0}{}",
        "a".repeat(LONGEST - 2),
        "b".repeat(LONGEST)
    );

    assert_eq!(safe_name(&landing), "a".repeat(LONGEST - 2));
}

#[test]
fn windows_stripping_a_trailing_dot_does_not_make_two_files_one() {
    // `report.txt.` and `report.txt` are one file on Windows and two here. Settled before the
    // dialog sees either.
    assert_eq!(safe_name("report.txt."), "report.txt");
    assert_eq!(safe_name("report.txt  "), "report.txt");
}

#[test]
fn a_name_in_another_alphabet_is_a_name() {
    // Replacing it would be this application deciding which languages deserve filenames.
    assert_eq!(safe_name("ζήτα.pdf"), "ζήτα.pdf");
    assert_eq!(safe_name("報告書.xlsx"), "報告書.xlsx");
}

#[test]
fn nothing_at_all_is_offered_as_something() {
    // A dialog opening with an empty name is a dialog a person has to name a file in before they
    // know what it is.
    assert_eq!(safe_name(""), UNNAMED);
    assert_eq!(safe_name("   "), UNNAMED);
    assert_eq!(safe_name("/"), UNNAMED);
    assert_eq!(safe_name("..."), UNNAMED);
}

#[test]
fn a_very_long_name_is_cut_on_a_letter_rather_than_through_one() {
    // Cut by bytes, a name in Greek ends mid-letter and stops being text.
    let long = "ζ".repeat(LONGEST * 2);
    let made = safe_name(&long);

    assert_eq!(made.chars().count(), LONGEST);
    assert!(made.chars().all(|character| character == 'ζ'));
}

#[test]
fn an_ordinary_name_is_left_alone() {
    // The commonest case, and the one a change to this function would break silently.
    for ordinary in ["report.pdf", "photo 2026-09-01.jpeg", "notes-v2.final.docx"] {
        assert_eq!(safe_name(ordinary), ordinary);
    }
}

#[test]
fn a_name_written_to_read_backwards_stops_reading_backwards() {
    // The whole of a known attack: `holiday<U+202E>gnp.exe` is drawn as `holidayexe.png` and runs
    // as an executable. Rust's `is_control` does not cover the override — it is formatting, not
    // control — so a filter written against control characters alone lets it straight through.
    let made = safe_name("holiday\u{202E}gnp.exe");

    assert_eq!(made, "holidaygnp.exe");
    assert!(!made.contains('\u{202E}'));
}

#[test]
fn a_name_windows_keeps_for_a_device_is_not_offered() {
    // Creating `COM1.pdf` on Windows does not create a file — it opens a serial port. What the
    // person sees is a save that failed for no reason they can act on, and the rule for the saved
    // message already knew that while the rule for a file it carried did not.
    for device in [
        "CON",
        "con.txt",
        "PRN.pdf",
        "aux",
        "NUL.eml",
        "COM1.pdf",
        "lpt9.doc",
        "CoM4",
        // The forms Windows also answers to and a plain comparison walks past: a space between
        // the stem and the extension, and the superscript digit its own device list uses.
        "CON .txt",
        "COM\u{00B9}.pdf",
        "LPT\u{00B3}",
    ] {
        assert_eq!(safe_name(device), UNNAMED, "{device} was offered as itself");
    }

    // The rule is the stem, not the letters: an ordinary name that begins the same way is ordinary.
    for ordinary in ["console.log", "connections.csv", "com10.txt", "lpt.txt"] {
        assert_eq!(safe_name(ordinary), ordinary);
    }
}

#[test]
fn a_cut_that_leaves_a_name_windows_keeps_for_a_device_does_not_offer_it() {
    // Asked before the cut, each of these is an ordinary name: the stem runs on past the spaces to
    // the letters behind them. The cut takes the letters, the trim takes the spaces, and what is
    // left opens a device on Windows instead of creating a file.
    for leaving_a_device in [
        format!("CON{}x.pdf", " ".repeat(LONGEST)),
        format!("COM1{}report.pdf", "\u{3000}".repeat(LONGEST)),
    ] {
        assert_eq!(
            safe_name(&leaving_a_device),
            UNNAMED,
            "{leaving_a_device:?} was offered as a device"
        );
    }
}

#[test]
fn a_cut_that_leaves_only_marks_does_not_offer_them() {
    // The letters that made it a name were past the cut. A row of the marks this function writes
    // is the name nobody chose, and it looks like one somebody did.
    for leaving_marks in [
        format!("{}abc", "_".repeat(LONGEST)),
        format!("{}abc", "_ ".repeat(LONGEST)),
    ] {
        assert_eq!(
            safe_name(&leaving_marks),
            UNNAMED,
            "{leaving_marks:?} was offered as its marks"
        );
    }
}

#[test]
fn a_name_too_long_is_not_left_ending_where_windows_would_cut_it() {
    // The cut can land on a dot or a space and put back exactly what the trim took out. A name
    // ending in either is the one Windows silently shortens, so `report….` asked for and `report…`
    // created are two names for one file — which is what the trim exists to prevent.
    let ending_in_a_dot = format!("{}.{}", "a".repeat(LONGEST - 1), "b".repeat(LONGEST));
    let made = safe_name(&ending_in_a_dot);

    assert!(!made.ends_with('.'), "{made}");
    assert!(!made.ends_with(' '), "{made}");
    assert!(made.chars().count() <= LONGEST);
}

#[test]
fn the_mark_the_obvious_list_leaves_out_is_on_this_one() {
    // U+061C is a bidirectional control by the standard's own property and sits nowhere near the
    // others in the code chart, so a list written from memory stops at U+200E. It reorders text
    // exactly as the override does.
    let made = safe_name("holiday\u{061C}gnp.exe");

    assert_eq!(made, "holidaygnp.exe");
    assert!(!made.contains('\u{061C}'));
}

#[test]
fn every_character_that_reorders_text_is_removed() {
    for reordering in [
        '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}',
        '\u{202E}', '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
    ] {
        let made = safe_name(&format!("a{reordering}b.txt"));

        assert_eq!(made, "ab.txt", "{reordering:?} survived");
    }
}

/// The name a message goes by as a file.
mod a_message_as_a_file {
    use super::*;

    #[test]
    fn a_subject_that_points_somewhere_else_names_a_file_and_nothing_more() {
        // A subject is written by whoever sent the message. Every separator on every platform
        // leaves, so the name the dialog is offered cannot walk out of the directory it opens in.
        for pointing in [
            "../../etc/passwd",
            "..\\..\\Windows\\System32\\drivers",
            "/etc/shadow",
            "C:\\Users\\Public\\startup",
        ] {
            let name = message_file_name(Some(pointing));

            assert!(
                !name.contains('/') && !name.contains('\\'),
                "{pointing:?} became {name:?}"
            );
            assert!(!name.starts_with('.'), "{pointing:?} became {name:?}");
            assert!(name.ends_with(".eml"), "{pointing:?} became {name:?}");
        }
    }

    #[test]
    fn a_subject_carrying_control_characters_does_not_carry_them_into_a_name() {
        // A newline or an escape passes through a filesystem and then does as it likes in a
        // terminal or a file manager that prints the name.
        let name = message_file_name(Some("Invoice\u{0}\u{1b}[2J\r\nattached"));

        assert_eq!(name, "Invoice [2J attached.eml");
        assert!(!name.chars().any(char::is_control));
    }

    #[test]
    fn characters_windows_refuses_are_gone_before_windows_sees_them() {
        let name = message_file_name(Some("Re: <urgent> \"quarterly\" report | Q3? *now*"));

        for refused in ['<', '>', ':', '"', '|', '?', '*'] {
            assert!(!name.contains(refused), "{name:?} still holds {refused:?}");
        }
        assert_eq!(name, "Re urgent quarterly report Q3 now.eml");
    }

    #[test]
    fn a_name_windows_keeps_for_a_device_is_not_offered() {
        // Creating CON, NUL or COM1 does not create a file: it opens a device, whatever extension
        // follows the reserved stem.
        for reserved in ["CON", "con", "NUL", "nul.txt", "COM1", "lpt9", "AUX", "PRN"] {
            assert_eq!(
                message_file_name(Some(reserved)),
                "message.eml",
                "{reserved:?} is a device on Windows"
            );
        }

        // And a name that merely starts the same way is an ordinary name.
        assert_eq!(message_file_name(Some("Console")), "Console.eml");
        assert_eq!(message_file_name(Some("COM0")), "COM0.eml");
        assert_eq!(message_file_name(Some("COM10")), "COM10.eml");
    }

    #[test]
    fn a_name_ending_in_a_dot_or_a_space_is_trimmed_here_rather_than_by_windows() {
        // Windows removes them itself and then the file that exists is not the one that was asked
        // for, which turns "did it save?" into a question.
        assert_eq!(message_file_name(Some("Release v1.0.")), "Release v1.0.eml");
        assert_eq!(message_file_name(Some("Notes   ")), "Notes.eml");
        assert_eq!(message_file_name(Some(".hidden")), "hidden.eml");
    }

    #[test]
    fn a_message_with_nothing_to_name_it_by_is_still_saveable() {
        assert_eq!(message_file_name(None), "message.eml");
        assert_eq!(message_file_name(Some("")), "message.eml");
        assert_eq!(message_file_name(Some("   \t\r\n ")), "message.eml");
        assert_eq!(message_file_name(Some("///")), "message.eml");
    }

    #[test]
    fn a_very_long_subject_is_cut_where_a_character_ends() {
        // Filesystems count bytes, and a subject can be entirely multi-byte characters. Cutting
        // by characters keeps the name well inside the limit and never splits one in half.
        let subject = "Ζήτα".repeat(60);
        let name = message_file_name(Some(&subject));

        assert!(name.is_char_boundary(name.len()));
        assert_eq!(name.chars().count(), LONGEST_MESSAGE_STEM + ".eml".len());
        assert!(name.len() < 255, "{} bytes", name.len());
        assert!(name.ends_with(".eml"));
    }

    #[test]
    fn a_name_that_reads_backwards_is_not_offered() {
        // U+202E turns everything after it round on screen: `invoice\u{202E}gnp.eml` is read as
        // `invoice`, then `lme.png` — a name whose extension is not the extension. The threat model
        // names this one by number (§5), the corpus has carried a message for it since before
        // there was anything to assert, and `char::is_control` never touched it: that is the Cc
        // category and these are Cf.
        let named = message_file_name(Some("invoice\u{202E}gnp"));

        assert!(
            !named.chars().any(reorders_text),
            "the name still turns text round: {named:?}"
        );
        // Removed rather than replaced: they occupy no width, so a space in their place is a gap
        // nobody wrote.
        assert_eq!(named, "invoicegnp.eml");
    }

    #[test]
    fn every_character_that_changes_direction_is_taken_out() {
        // The whole range, not the one that is famous. An embedding or an isolate does the same
        // thing to the same name.
        for turning in [
            '\u{200E}', '\u{200F}', '\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}',
            '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}',
        ] {
            let named = message_file_name(Some(&format!("report{turning}fdp")));
            assert_eq!(named, "reportfdp.eml", "U+{:04X} survived", turning as u32);
        }
    }

    #[test]
    fn an_ordinary_subject_is_offered_as_the_person_would_recognise_it() {
        assert_eq!(
            message_file_name(Some("Lunch on Thursday")),
            "Lunch on Thursday.eml"
        );
        assert_eq!(message_file_name(Some("Ζήτα Κάππα")), "Ζήτα Κάππα.eml");
    }
}
