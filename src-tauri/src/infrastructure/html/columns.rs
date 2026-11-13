//! Where the columns of a text somebody wrote matter: a patch, as a whole, and a table or a drawing
//! in box characters where it stands, a table ruled with dashes and bars or aligned with runs of
//! spaces. There the text is read in the letters of code, where every character is as wide as the
//! next and a column stays a column; everywhere else in the letters every message is read in.

use std::ops::Range;

/// How many rows in a row make a table rather than a line that happened to hold a wide space.
const ROWS: usize = 3;

/// How many lines in a row drawn in box characters make a drawing rather than a line that holds
/// one.
const DRAWN: usize = 2;

/// How many spaces in a row part two columns; a tab parts them by itself.
const GAP: usize = 3;

/// What a line drawn across the page is drawn with: dashes, equals signs, underscores, and the box
/// characters' strokes from side to side, whole, heavy, dashed and double.
const ACROSS: [char; 12] = [
    '-', '=', '_', '\u{2500}', '\u{2501}', '\u{2504}', '\u{2505}', '\u{2508}', '\u{2509}',
    '\u{254C}', '\u{254D}', '\u{2550}',
];

/// Where the columns of `text` matter, as byte ranges of whole lines in order, none of them
/// touching the next: all of it for a patch, and every table and drawing where it stands.
pub(super) fn where_they_matter(text: &str) -> Vec<Range<usize>> {
    let mut found = Vec::new();
    let mut table = Run::of(ROWS);
    let mut drawing = Run::of(DRAWN);
    let mut takes_from = false;
    let mut at = 0;

    for whole in text.split_inclusive('\n') {
        let line = whole.trim_end();
        let here = at..at + whole.len();
        at = here.end;

        // A patch: its header, a hunk, or the two lines naming what it changes, one after the
        // other.
        if line.starts_with("diff --git ")
            || line.starts_with("@@ -")
            || (takes_from && line.starts_with("+++ "))
        {
            return all_of(text);
        }
        takes_from = line.starts_with("--- ");

        drawing.goes_on(Line::counted(is_drawn(line)), &here, &mut found);
        let row = if is_across(line) {
            Line::Between
        } else if !line.is_empty() && is_a_row(line) {
            Line::Counted
        } else {
            Line::Other
        };
        table.goes_on(row, &here, &mut found);
    }
    drawing.ends(&mut found);
    table.ends(&mut found);

    joined(found)
}

/// The whole text, as the one stretch where its columns matter.
fn all_of(text: &str) -> Vec<Range<usize>> {
    std::iter::once(0..text.len()).collect()
}

/// What a line is to a run of lines.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Line {
    /// One of the lines the run is counted in.
    Counted,
    /// A line a run goes on across, counted or not: a rule between a table's rows.
    Between,
    /// A line that ends the run.
    Other,
}

impl Line {
    fn counted(is: bool) -> Self {
        match is {
            true => Line::Counted,
            false => Line::Other,
        }
    }
}

/// Lines one after another that make a table or a drawing once enough of them are counted.
struct Run {
    /// How many counted lines make one.
    enough: usize,
    /// How many are counted so far.
    counted: usize,
    /// Where the run is, from its first line to its last, if it has begun.
    taken: Option<Range<usize>>,
}

impl Run {
    fn of(enough: usize) -> Self {
        Run {
            enough,
            counted: 0,
            taken: None,
        }
    }

    /// The run goes on with `line`, at `here`, or ends before it.
    fn goes_on(&mut self, line: Line, here: &Range<usize>, found: &mut Vec<Range<usize>>) {
        match line {
            Line::Other => self.ends(found),
            Line::Counted | Line::Between => {
                self.counted += usize::from(line == Line::Counted);
                self.taken = Some(
                    self.taken
                        .take()
                        .map_or(here.clone(), |taken| taken.start..here.end),
                );
            }
        }
    }

    /// The run ends, and is kept if enough of its lines were counted.
    fn ends(&mut self, found: &mut Vec<Range<usize>>) {
        if let Some(taken) = self.taken.take() {
            if self.counted >= self.enough {
                found.push(taken);
            }
        }
        self.counted = 0;
    }
}

/// The stretches in order, those that overlap or touch made one: a drawing that is a table too,
/// or a table right under a drawing, is one stretch.
fn joined(mut found: Vec<Range<usize>>) -> Vec<Range<usize>> {
    found.sort_by_key(|stretch| stretch.start);
    let mut joined: Vec<Range<usize>> = Vec::with_capacity(found.len());
    for stretch in found {
        match joined.last_mut() {
            Some(last) if stretch.start <= last.end => last.end = last.end.max(stretch.end),
            _ => joined.push(stretch),
        }
    }
    joined
}

/// A line that reads as a row of a table: ruled, cut by bars, or with a gap between its columns.
fn is_a_row(line: &str) -> bool {
    is_a_rule(line) || line.matches('|').count() >= 2 || has_a_gap(line)
}

/// A line of dashes or equals signs crossed by a bar or a plus, as a table is ruled.
fn is_a_rule(line: &str) -> bool {
    let rule = line.trim();
    rule.chars()
        .all(|character| matches!(character, '-' | '=' | '+' | '|' | ' '))
        && rule
            .chars()
            .filter(|character| matches!(character, '-' | '='))
            .count()
            >= 3
        && rule.chars().any(|character| matches!(character, '+' | '|'))
}

/// A line drawn across the page and nothing more, quoted or not, which a letter puts between its
/// parts and a table between its rows.
fn is_across(line: &str) -> bool {
    let (_, said) = super::typed::quoted(line);
    said.chars()
        .filter(|character| ACROSS.contains(character))
        .count()
        >= 3
        && said
            .chars()
            .all(|character| character.is_whitespace() || ACROSS.contains(&character))
}

/// Whether, after its first word, what a line says holds a tab or a gap of spaces before more
/// words.
fn has_a_gap(line: &str) -> bool {
    let mut spaces = 0;
    for character in said_in(line).chars() {
        match character {
            ' ' => spaces += 1,
            '\t' => spaces += GAP,
            _ if spaces >= GAP => return true,
            _ => spaces = 0,
        }
    }
    false
}

/// What a line says, under the marks that quote it and the mark of a list it may be in: neither is
/// a column, and the space after them is not a gap between two.
fn said_in(line: &str) -> &str {
    let (_, said) = super::typed::quoted(line);
    without_a_mark(said.trim_start()).trim_start()
}

/// `line` without the mark a line of a list may start with: a dash, a star, a dot or a small
/// letter o, or a number with a full stop or a bracket after it. Taken off the start of a word that
/// is not a mark, it only makes the word shorter, which holds no gap.
fn without_a_mark(line: &str) -> &str {
    let digits = line.bytes().take_while(u8::is_ascii_digit).count();
    match digits {
        0 => ['-', '*', '\u{2022}', '\u{00B7}', 'o']
            .into_iter()
            .find_map(|mark| line.strip_prefix(mark))
            .unwrap_or(line),
        _ => line[digits..].strip_prefix(['.', ')']).unwrap_or(line),
    }
}

/// A line of a drawing: one holding a box character other than a stroke from side to side, which
/// a line drawn across the page is made of.
fn is_drawn(line: &str) -> bool {
    line.chars()
        .any(|character| is_drawing(character) && !ACROSS.contains(&character))
}

/// A character of the box drawings: a line, a corner, a joint.
fn is_drawing(character: char) -> bool {
    ('\u{2500}'..='\u{257F}').contains(&character)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matters_in(text: &str) -> Vec<&str> {
        where_they_matter(text)
            .into_iter()
            .map(|range| &text[range])
            .collect()
    }

    #[test]
    fn a_patch_is_read_in_the_letters_of_code_as_a_whole() {
        let patch =
            "Here is the fix:\n\ndiff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new";
        assert_eq!(matters_in(patch), [patch]);

        // Each of the ways a patch shows itself, alone.
        for patch in [
            "diff --git a/logo.png b/logo.png\nBinary files differ",
            "Hunk:\n@@ -1 +1 @@\n-old\n+new",
            "--- a/notes\n+++ b/notes\n-one\n+two",
        ] {
            assert_eq!(matters_in(patch), [patch], "{patch:?}");
        }
    }

    #[test]
    fn a_drawing_in_box_characters_is_read_in_them_where_it_stands() {
        assert_eq!(
            matters_in("A box:\n┌────┐\n│ hi │\n└────┘\nThat is all."),
            ["┌────┐\n│ hi │\n└────┘\n"]
        );
        assert_eq!(
            matters_in("The files:\n├── src\n└── main.rs"),
            ["├── src\n└── main.rs"]
        );
    }

    #[test]
    fn a_table_is_read_in_them_where_it_stands() {
        let letter = "Here is the order:\n\
                      | Name | Qty |\n\
                      |------|-----|\n\
                      | Pens | 12  |\n\
                      Thank you,\nZeta";
        assert_eq!(
            matters_in(letter),
            ["| Name | Qty |\n|------|-----|\n| Pens | 12  |\n"]
        );

        assert_eq!(
            matters_in("+------+-----+\n| Pens | 12  |\n+------+-----+"),
            ["+------+-----+\n| Pens | 12  |\n+------+-----+"]
        );
        assert_eq!(
            matters_in("Item      Qty   Price\nPens      12    3.50\nPaper     500   9.00\n"),
            ["Item      Qty   Price\nPens      12    3.50\nPaper     500   9.00\n"]
        );
        assert_eq!(
            matters_in("Item\tQty\nPens\t12\nPaper\t500"),
            ["Item\tQty\nPens\t12\nPaper\t500"]
        );
        // Quoted, it is the same table.
        assert_eq!(
            matters_in("> Item      Qty\n> Pens      12\n> Paper     500"),
            ["> Item      Qty\n> Pens      12\n> Paper     500"]
        );
    }

    #[test]
    fn a_table_goes_on_across_the_rules_between_its_rows() {
        // A rule under a table's head, or drawn above and below it, is the table's.
        assert_eq!(
            matters_in("Totals:\nItem      Qty\n-------------\nPens      12\nPaper     500\nZeta"),
            ["Item      Qty\n-------------\nPens      12\nPaper     500\n"]
        );
        assert_eq!(
            matters_in("────────────\n Item     Qty\n────────────\n Pens     12\n Paper    500\n────────────"),
            ["────────────\n Item     Qty\n────────────\n Pens     12\n Paper    500\n────────────"]
        );
        let quoted = "> Item      Qty\n> -------------\n> Pens      12\n> Paper     500";
        assert_eq!(matters_in(quoted), [quoted]);
    }

    #[test]
    fn a_drawing_and_a_table_that_meet_are_one_stretch() {
        // A tree with words aligned beside it is both, and a table right under a drawing goes on
        // from it.
        let tree = "├── src       the code\n├── docs      the words\n└── tests     the checks";
        assert_eq!(matters_in(&format!("Files:\n{tree}")), [tree]);

        let both = "┌──┐\n└──┘\nItem      Qty\nPens      12\nPaper     500\n";
        assert_eq!(matters_in(&format!("{both}\nZeta")), [both]);
    }

    #[test]
    fn an_aligned_signature_keeps_its_columns_and_the_letter_above_it_its_letters() {
        let letter = "Hello Kappa,\n\nThank you for the papers.\n\nZeta Quorin\n\
                      Phone:     +1 555 0100\n\
                      Office:    Room 3\n\
                      Hours:     9 to 5";

        assert_eq!(
            matters_in(letter),
            ["Phone:     +1 555 0100\nOffice:    Room 3\nHours:     9 to 5"]
        );
    }

    #[test]
    fn a_letter_is_read_in_the_letters_of_mail() {
        for letter in [
            "Hello Kappa,\n\nShall we meet on Tuesday? I can bring the papers.\n\nZeta",
            // One wide gap, or one rule, is a sentence and a separator, not a table.
            "See you   soon.\n\n-----\nZeta",
            "--- Original Message ---\nFrom: Kappa\nSaid: hello",
            // A line taken from and a line given to are a patch only one right after the other.
            "--- Original Message ---\nFrom: Kappa\n+++ Urgent +++\nSaid: hello",
            // Two rows are not yet a table.
            "Item      Qty\nPens      12\n\nThanks",
            // One line drawn in box characters is not yet a drawing, and a line drawn across a
            // letter is none, quoted or not.
            "The plan:\n└ ship it\nZeta",
            "────────\nHello Kappa,\n────────\nZeta",
            "> Thanks,\n> ────\n> Kappa\n> ════",
            "Thanks,\n────────────\n════════════\nZeta",
            // Neither a quote's marks nor a list's are a column of their own.
            ">    - First\n>    - Second\n>    - Third",
            ">   indented prose\n>   that goes on\n>   and ends",
            "1.\tBuy milk\n2.\tBread\n3.\tEggs",
            "-   Pens\n*   Paper\n•   Ink",
        ] {
            assert!(matters_in(letter).is_empty(), "{letter:?}");
        }
    }

    #[test]
    fn finding_where_columns_matter_costs_what_the_text_s_length_costs() {
        // Every line is looked at once, however the text is made: two megabytes of rows, of
        // tables, of drawing, of quote marks, of one long line or of almost patches.
        for text in [
            "| a | b |\n".repeat(200_000),
            "| a | b |\n| c | d |\n| e | f |\nprose\n".repeat(50_000),
            "│ a │\n".repeat(250_000),
            ">".repeat(2 << 20),
            format!("a{}b", " ".repeat(2 << 20)),
            "--- a\n".repeat(300_000),
        ] {
            let began = std::time::Instant::now();

            let _ = where_they_matter(&text);
            assert!(
                began.elapsed() < std::time::Duration::from_secs(2),
                "{:?}: {:?}",
                text.chars().take(8).collect::<String>(),
                began.elapsed()
            );
        }
    }
}
