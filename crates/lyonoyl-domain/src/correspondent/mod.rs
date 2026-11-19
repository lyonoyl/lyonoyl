//! Who the composer offers as an address is typed: the people a mailbox has written to.
//!
//! Only the mailbox's own sent mail says who those are. Somebody who wrote to it and was never
//! written back to is not offered, however often they wrote: a newsletter, a stranger, or a message
//! made to put an address under somebody's fingers stays where it arrived. What their own mail adds
//! is the name they give themselves, shown beside an address that is already offered.

use std::collections::{HashMap, HashSet};

use crate::attachment::reorders_text;
use crate::outgoing::{EmailAddress, LONGEST_ADDRESS};

/// How many people are offered at once.
pub const MOST_OFFERED: usize = 6;

/// How many of the addresses one header names count as people written to.
///
/// As many as the reader shows of a header. A message to ten thousand people is a mailing, and a
/// header made that long would otherwise grow what is gathered by everybody it named.
pub const MOST_COUNTED_OF_A_HEADER: usize = 100;

/// The longest name shown beside an address, in characters. Past it, it is not somebody's name.
pub const LONGEST_NAME: usize = 100;

/// How many people one mailbox's sent mail is gathered into at most.
///
/// Far past what anybody writes to. A server that fills the folder for sent mail with messages to a
/// hundred new addresses each would otherwise grow what is kept, for as long as the application
/// runs, by everybody those messages named. Past it nobody new is counted, and everybody already
/// counted still is.
pub const MOST_KEPT: usize = 50_000;

/// Somebody a mailbox has written to, as the composer offers them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correspondent {
    /// Their address, as the newest message to them wrote it.
    pub address: String,
    /// The name the newest message from them gave them, when one did.
    pub name: Option<String>,
    /// How many of the mailbox's messages went to them.
    pub times: u32,
    /// When the newest of those went, in Unix seconds.
    pub last: i64,
    /// The address as addresses compare, to tell it from one already written out.
    compared: EmailAddress,
    /// The address with case and accents taken off, as typing is matched against it.
    folded_address: String,
    /// The words of the name with case and accents taken off and one space between them, as
    /// typing is matched against it.
    folded_name: Option<String>,
}

/// Everybody a mailbox's sent mail names, gathered one message at a time.
#[derive(Debug, Default)]
pub struct Gathering {
    /// By the address with the case of its ASCII letters taken off, which is how two addresses
    /// compare: what [`Gathering::named`] looks a sender up by for every message there is, without
    /// reading the address again.
    people: HashMap<String, Tally>,
}

#[derive(Debug)]
struct Tally {
    compared: EmailAddress,
    written: String,
    times: u32,
    last: i64,
    name: Option<(String, i64)>,
}

/// An address as two addresses compare: the case of its ASCII letters off, the domain's and the
/// local part's alike, as [`EmailAddress`] compares them.
fn compared_as(written: &str) -> String {
    written.trim().to_ascii_lowercase()
}

impl Gathering {
    /// One message the mailbox sent: everybody its `To` and `Cc` named, and when it went.
    ///
    /// Somebody named twice in one message was written to once. An address that could not be
    /// handed back to the composer as it is written is passed over, since offering it would put
    /// into the line something that line splits apart, refuses, or shows as another address.
    pub fn sent<'a>(
        &mut self,
        to: impl IntoIterator<Item = &'a str>,
        cc: impl IntoIterator<Item = &'a str>,
        at: i64,
    ) {
        let mut named = HashSet::new();
        let headers = to
            .into_iter()
            .take(MOST_COUNTED_OF_A_HEADER)
            .chain(cc.into_iter().take(MOST_COUNTED_OF_A_HEADER));

        for written in headers {
            let Some(address) = offerable(written) else {
                continue;
            };
            let key = compared_as(written);
            if !named.insert(key.clone()) {
                continue;
            }
            if self.people.len() >= MOST_KEPT && !self.people.contains_key(&key) {
                continue;
            }

            let tally = self.people.entry(key).or_insert_with(|| Tally {
                compared: address,
                written: String::new(),
                times: 0,
                last: i64::MIN,
                name: None,
            });
            tally.times = tally.times.saturating_add(1);
            if at >= tally.last {
                tally.last = at;
                tally.written = written.trim().to_owned();
            }
        }
    }

    /// The name a message from somebody gave them, and when it came.
    ///
    /// Kept only for somebody the mailbox has written to, so every message it sent is gathered
    /// first. The newest name wins, because a name is changed by using another one.
    pub fn named(&mut self, address: &str, name: &str, at: i64) {
        let Some(tally) = self.people.get_mut(&compared_as(address)) else {
            return;
        };
        let Some(name) = shown_name(name) else {
            return;
        };

        if tally.name.as_ref().is_none_or(|(_, when)| at >= *when) {
            tally.name = Some((name, at));
        }
    }

    /// Everybody gathered, with what typing is matched against worked out once rather than at
    /// every key.
    ///
    /// `fold` takes the case and the accents off text, as typing is compared. It is handed in,
    /// because the tables that say what an accent is, in every alphabet, are not this crate's to
    /// carry.
    pub fn people(self, fold: impl Fn(&str) -> String) -> Vec<Correspondent> {
        self.people
            .into_values()
            .map(|tally| {
                let name = tally.name.map(|(name, _)| name);

                Correspondent {
                    folded_address: fold(&tally.written),
                    folded_name: name
                        .as_deref()
                        .map(|name| words(&fold(name)).join(" "))
                        .filter(|joined| !joined.is_empty()),
                    address: tally.written,
                    name,
                    times: tally.times,
                    last: tally.last,
                    compared: tally.compared,
                }
            })
            .collect()
    }
}

/// Who of `people` the composer offers for what is being typed: everybody whose address or name
/// holds it anywhere, at most [`MOST_OFFERED`] of them. Those whose address, a part of the address
/// after a `.`, `_`, `-` or `+` before the `@`, or a word of the name begins with it come first,
/// since a few letters are most often the start of who is meant, and then the rest. Among either,
/// the most written to come first, and then the most recently written to.
///
/// Nothing is offered for nothing typed, nor for more than an address can hold. Once what is typed
/// holds an `@`, it is an address being written and only addresses are matched: a name is matched
/// by its words, and `kappa@ex` is not the words "kappa ex". An address already written out in full
/// is not offered again, which leaves only the others that hold it.
///
/// `fold` is the one [`Gathering::people`] was given, so typing and what it is matched against
/// lose their case and accents the same way.
pub fn offered<'a>(
    people: &'a [Correspondent],
    typed: &str,
    fold: impl Fn(&str) -> String,
) -> Vec<&'a Correspondent> {
    let typed = typed.trim();
    if typed.is_empty() || typed.chars().count() > LONGEST_ADDRESS {
        return Vec::new();
    }

    let as_address = fold(typed);
    if as_address.is_empty() {
        return Vec::new();
    }
    let as_words = (!typed.contains('@'))
        .then(|| words(&as_address).join(" "))
        .filter(|joined| !joined.is_empty());
    let written_out = EmailAddress::parse(typed).ok();

    let mut found: Vec<(Found, &Correspondent)> = people
        .iter()
        .filter(|person| written_out.as_ref() != Some(&person.compared))
        .filter_map(|person| {
            let in_the_address = found_in_address(&person.folded_address, &as_address);
            let in_the_name = as_words
                .as_deref()
                .zip(person.folded_name.as_deref())
                .and_then(|(typed, name)| found_in_name(name, typed));

            in_the_address
                .into_iter()
                .chain(in_the_name)
                .min()
                .map(|found| (found, person))
        })
        .collect();

    found.sort_by(|(one_found, one), (other_found, other)| {
        one_found
            .cmp(other_found)
            .then(other.times.cmp(&one.times))
            .then(other.last.cmp(&one.last))
            .then_with(|| one.address.cmp(&other.address))
    });
    found.truncate(MOST_OFFERED);
    found.into_iter().map(|(_, person)| person).collect()
}

/// Where what is typed stands in an address or a name. A start comes before the inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Found {
    Start,
    Inside,
}

/// Where `typed` stands in a folded address, if anywhere. At a start when the address begins with
/// it, or a part of the address after a `.`, `_`, `-` or `+` before the `@` does, so `vandel` is at
/// a start in `k.vandel@example.net`. The domain's own parts are no start, or everybody at it would
/// come first.
fn found_in_address(address: &str, typed: &str) -> Option<Found> {
    if !address.contains(typed) {
        return None;
    }

    let before_the_at = &address[..address.rfind('@').unwrap_or(address.len())];
    let begins = address.starts_with(typed)
        || before_the_at
            .char_indices()
            .filter(|(_, character)| matches!(character, '.' | '_' | '-' | '+'))
            .any(|(at, character)| address[at + character.len_utf8()..].starts_with(typed));

    Some(if begins { Found::Start } else { Found::Inside })
}

/// Where `typed`, as words, stands in the folded words of a name, if anywhere. At a start when a
/// word of the name begins with it, so `vandel` and `kappa va` are both at a start in Kappa Vandel.
fn found_in_name(name: &str, typed: &str) -> Option<Found> {
    if !name.contains(typed) {
        return None;
    }

    let begins = name.starts_with(typed)
        || name
            .match_indices(' ')
            .any(|(at, _)| name[at + 1..].starts_with(typed));

    Some(if begins { Found::Start } else { Found::Inside })
}

/// An address that can be handed back to the composer as it is written: one this build sends to,
/// holding a single `@` and none of the characters the composer's line of addresses splits on or
/// refuses, and nothing that turns text around or takes up no room, either of which makes it read
/// as another address.
fn offerable(written: &str) -> Option<EmailAddress> {
    let written = written.trim();
    let unfit = |character: char| {
        matches!(character, ',' | ';' | '<' | '>' | '"')
            || reorders_text(character)
            || takes_no_room(character)
    };

    if written.matches('@').count() != 1 || written.chars().any(unfit) {
        return None;
    }

    EmailAddress::parse(written).ok()
}

/// A character drawn as nothing at all: a soft hyphen, a zero-width space, joiner or non-joiner, a
/// word joiner or an invisible operator, or a byte order mark. Two addresses that differ by one of
/// these look the same.
fn takes_no_room(character: char) -> bool {
    matches!(
        character,
        '\u{00AD}' | '\u{200B}'..='\u{200D}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}'
    )
}

/// A name as it may be shown beside an address: on one line, without the characters that turn
/// text around, and not at all when it holds an `@`, since a name written as an address shows one
/// address over another.
fn shown_name(name: &str) -> Option<String> {
    let kept: String = name
        .chars()
        .filter(|character| !character.is_control() && !reorders_text(*character))
        .collect();
    let kept = kept.split_whitespace().collect::<Vec<_>>().join(" ");

    let fits = !kept.is_empty() && !kept.contains('@') && kept.chars().count() <= LONGEST_NAME;
    fits.then_some(kept)
}

/// The words of a text: what is left between everything that is not a letter or a digit.
fn words(text: &str) -> Vec<&str> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect()
}

#[cfg(test)]
mod tests;
