//! Turning what a person typed into something the index can be asked.
//!
//! What is typed is not a query — it is a few words, an address, half a name, and whatever the
//! keyboard did on the way. The index speaks a language with operators, quotes and precedence, and
//! handing it the typing directly means `news@x.test` is a syntax error, `NEAR` is an operator and
//! a stray `"` is an unclosed string. So nothing typed is ever passed through: the words are taken
//! out of it and a query is built from them.

/// The most a query is built from.
///
/// A search box accepts a paste as readily as a word. Past a few dozen words the answer cannot be
/// what anybody meant, and every one of them costs the index a lookup.
pub const WORDS_ASKED_FOR: usize = 32;

/// The longest a single word may be before it is cut.
///
/// Longer than any word in any language this is likely to meet, and short enough that a megabyte
/// of one token cannot be pasted into an index lookup.
pub const LONGEST_WORD: usize = 64;

/// What a person typed, as a question the index understands — or nothing, when what was typed
/// holds no word at all.
///
/// Each run of whitespace separates one thing being looked for, and everything inside it that is
/// not a letter or a digit separates one word from the next — the same rule the index tokenizes
/// by, so `news@x.test` is asked for as the three adjacent words it was stored as, and finds the
/// address rather than every message mentioning a test.
///
/// The last thing typed is asked for as a beginning rather than a whole word, because a person
/// still typing has not finished it yet.
///
/// Nothing typed reaches the query as itself: words are built out of letters and digits only, so
/// there is no quote to close, no operator to trip over and nothing to escape.
pub fn asked_for(typed: &str) -> Option<String> {
    let folded = folded_yo(typed);

    let phrases: Vec<String> = folded
        .split_whitespace()
        .map(|chunk| {
            chunk
                .split(|letter: char| !letter.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .map(|word| word.chars().take(LONGEST_WORD).collect::<String>())
                .collect::<Vec<_>>()
        })
        .filter(|words| !words.is_empty())
        .take(WORDS_ASKED_FOR)
        .map(|words| format!("\"{}\"", words.join(" ")))
        .collect();

    let mut phrases = phrases;
    let last = phrases.pop()?;
    phrases.push(format!("{last}*"));

    Some(phrases.join(" "))
}

/// The same text with the Cyrillic yo (U+0451) written as ie (U+0435), which is how the index holds
/// it.
///
/// Nobody types the two dots and plenty of senders do, so the two spellings have to be one word.
/// It is done here because the tokenizer does not do it: `unicode61` takes marks off Latin letters
/// only, and keeps yo and ie apart as it keeps short i (U+0439) and i (U+0438) apart, which are two
/// letters rather than a letter and an accent.
pub fn folded_yo(text: &str) -> String {
    text.replace('\u{0451}', "\u{0435}")
        .replace('\u{0401}', "\u{0415}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_typed_asks_nothing() {
        assert_eq!(asked_for(""), None);
        assert_eq!(asked_for("   "), None);
        assert_eq!(asked_for("!!! ??? ---"), None);
    }

    #[test]
    fn one_word_is_asked_for_as_a_beginning() {
        // Somebody typing "repo" has not finished the word, and a list that waits for them to
        // finish is a list that answers after they have stopped looking.
        assert_eq!(asked_for("repo").as_deref(), Some("\"repo\"*"));
    }

    #[test]
    fn several_words_are_all_required() {
        assert_eq!(
            asked_for("quarterly report").as_deref(),
            Some("\"quarterly\" \"report\"*")
        );
    }

    #[test]
    fn an_address_is_asked_for_as_the_words_it_is_stored_as() {
        // The index splits an address on its punctuation, so asking for it whole finds nothing and
        // asking for its words separately finds every message mentioning a test.
        assert_eq!(
            asked_for("zeta@x.test").as_deref(),
            Some("\"zeta x test\"*")
        );
    }

    #[test]
    fn an_operator_of_the_query_language_is_only_a_word() {
        // `NEAR`, `AND`, `OR` and `NOT` are the index's own words. Typed into a search box they
        // are what somebody is looking for.
        assert_eq!(
            asked_for("near AND or").as_deref(),
            Some("\"near\" \"AND\" \"or\"*")
        );
    }

    #[test]
    fn nothing_typed_can_close_a_quote_or_open_a_group() {
        // The whole reason this exists. Every one of these is a syntax error passed through as
        // itself, and a search that answers with an error is a search that does not work.
        for typed in ["\"", "a\" OR \"b", "(", "*", "^foo", "a:b", "-x", "a\"\"b"] {
            let asked = asked_for(typed);
            if let Some(asked) = asked {
                assert!(
                    asked.chars().filter(|letter| *letter == '"').count() % 2 == 0,
                    "{typed:?} became {asked:?}, which does not close its quotes"
                );
                assert!(
                    !asked.trim_matches(|letter| letter != '(').is_empty() || !asked.contains('('),
                    "{typed:?} became {asked:?}"
                );
            }
        }
    }

    #[test]
    fn the_two_spellings_of_yo_ask_the_same_thing() {
        assert_eq!(asked_for("ёлка"), asked_for("елка"));
        assert_eq!(asked_for("Ёлка"), asked_for("Елка"));
    }

    #[test]
    fn a_word_longer_than_any_word_is_cut() {
        let pasted = "a".repeat(5_000);
        let asked = asked_for(&pasted).expect("a word");

        assert_eq!(asked.len(), LONGEST_WORD + 3, "quotes and the star");
    }

    #[test]
    fn a_paste_is_cut_to_what_somebody_could_have_meant() {
        let pasted = (0..500)
            .map(|at| format!("word{at}"))
            .collect::<Vec<_>>()
            .join(" ");

        let asked = asked_for(&pasted).expect("words");

        assert_eq!(
            asked.matches('*').count(),
            1,
            "only the last thing typed is a beginning"
        );
        assert_eq!(
            asked.split(' ').count(),
            WORDS_ASKED_FOR,
            "a paste asks for no more of the index than somebody could have meant"
        );
    }
}
