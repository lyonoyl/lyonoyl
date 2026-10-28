use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use crate::ports::drafts::DraftFile;
use lyonoyl_domain::outgoing::{fit_in_one_message, OutgoingFile};

/// Files chosen for a message that has not been sent yet.
///
/// Held here rather than handed to the window, and that is the whole design: a photograph is
/// megabytes, and megabytes crossing the boundary twice — out to be listed and back to be sent —
/// is a copy of somebody's file in a webview for no reason at all. What the window gets is a name,
/// a size and a token; what it sends back is the token.
///
/// In memory, and in the draft the message is kept as. Every keep names the tokens of what is
/// attached at that moment, and the draft takes the bytes from here once; until the message is
/// sent the bytes stay here as well, which is what sending takes them from. A draft opened again
/// has its files held here under fresh tokens, each standing for the name the draft already
/// knows it by.
#[derive(Clone, Default)]
pub struct FilesBeingAttached {
    held: Arc<Mutex<Held>>,
}

#[derive(Default)]
struct Held {
    /// The files, by token. Shared rather than copied with a draft being kept.
    files: HashMap<String, Arc<OutgoingFile>>,
    /// What a draft knows a file by, for a token that is not that name itself: one a draft opened
    /// again was given. Outlives the file being taken for sending, so a message refused and given
    /// its files back is kept under the same names, and goes when the token is forgotten.
    kept_as: HashMap<String, String>,
}

impl FilesBeingAttached {
    /// Keeps one, and answers with the token it is known by.
    pub fn hold(&self, token: String, file: OutgoingFile) {
        if let Ok(mut held) = self.held.lock() {
            held.files.insert(token, Arc::new(file));
        }
    }

    /// Keeps one chosen to go beside the files `beside` names, and answers whether it was kept.
    ///
    /// Not kept when together they would weigh more than one message may. A file that makes them
    /// so could never go with the message, and a draft carrying it is one no keep writes down,
    /// with everything typed after it: refused here, it is refused as it is chosen, while somebody
    /// can choose another. A token named twice is one file, and one that is not here names none.
    #[must_use]
    pub fn hold_beside(&self, token: String, file: OutgoingFile, beside: &[String]) -> bool {
        let Ok(mut held) = self.held.lock() else {
            return false;
        };

        let named: HashSet<&String> = beside.iter().collect();
        let fits = fit_in_one_message(
            named
                .into_iter()
                .filter_map(|one| held.files.get(one))
                .map(|one| &**one)
                .chain([&file]),
        );
        if fits {
            held.files.insert(token, Arc::new(file));
        }

        fits
    }

    /// Keeps a file a draft already carries, under a fresh token standing for the name the draft
    /// knows it by.
    pub fn hold_kept(&self, token: String, kept_as: String, file: Arc<OutgoingFile>) {
        if let Ok(mut held) = self.held.lock() {
            held.kept_as.insert(token.clone(), kept_as);
            held.files.insert(token, file);
        }
    }

    /// The ones named, as a draft keeps them, in the order they were named — all of them, or none.
    ///
    /// Not taken: the message is still being written, and sending takes them from here. Nothing is
    /// copied either, so naming a photograph at every keep costs a count and not its bytes.
    ///
    /// `None` when any token named is not here, or when a file would be named twice — by its token,
    /// or by the name a draft knows it by — and then the draft is written nothing.
    pub fn to_keep(&self, tokens: &[String]) -> Option<Vec<DraftFile>> {
        let held = self.held.lock().ok()?;

        let mut named = HashSet::with_capacity(tokens.len());
        let mut kept_as = HashSet::with_capacity(tokens.len());
        tokens
            .iter()
            .map(|token| {
                let file = held.files.get(token)?;
                let known_as = held.kept_as.get(token).unwrap_or(token);

                (named.insert(token) && kept_as.insert(known_as)).then(|| DraftFile {
                    kept_as: known_as.clone(),
                    file: Arc::clone(file),
                })
            })
            .collect()
    }

    /// Takes the ones named, in the order they were named, and forgets them — all of them, or none.
    ///
    /// Taken rather than copied: a message is sent once, and a token that survived its message
    /// would be a file the next one could attach without anybody choosing it.
    ///
    /// `None` when any token named is not here, or is named twice, and then nothing is taken: one
    /// file gone is no reason to lose the others, which the window still shows and will send once
    /// it is told.
    pub fn take(&self, tokens: &[String]) -> Option<Vec<OutgoingFile>> {
        let mut held = self.held.lock().ok()?;

        // Each named once: the second `remove` of a token finds nothing, after the first has
        // already taken its file.
        let mut named = HashSet::with_capacity(tokens.len());
        if !tokens
            .iter()
            .all(|token| held.files.contains_key(token) && named.insert(token))
        {
            return None;
        }

        // The bytes themselves rather than a copy of them, unless a keep still has them in hand.
        tokens
            .iter()
            .map(|token| held.files.remove(token).map(Arc::unwrap_or_clone))
            .collect()
    }

    /// Puts files taken for a message back under the tokens they were taken by, for a message that
    /// was refused before it was written down: the window still shows them, and sending again
    /// should find them where they were.
    pub fn give_back(&self, tokens: &[String], files: Vec<OutgoingFile>) {
        for (token, file) in tokens.iter().zip(files) {
            self.hold(token.clone(), file);
        }
    }

    /// Forgets one, for somebody who attached a file and changed their mind, and for a composer
    /// that closed.
    pub fn forget(&self, token: &str) {
        if let Ok(mut held) = self.held.lock() {
            held.files.remove(token);
            held.kept_as.remove(token);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_file(name: &str) -> OutgoingFile {
        OutgoingFile {
            name: name.to_owned(),
            content_type: "application/octet-stream".to_owned(),
            bytes: name.as_bytes().to_vec(),
        }
    }

    #[test]
    fn files_come_back_in_the_order_they_were_named() {
        // The order they appear in the message is the order somebody attached them in, and the
        // window is what remembers that — so what comes back must follow what was asked for
        // rather than whatever a map happened to hold.
        let held = FilesBeingAttached::default();
        held.hold("t1".to_owned(), a_file("a.txt"));
        held.hold("t2".to_owned(), a_file("b.txt"));
        held.hold("t3".to_owned(), a_file("c.txt"));

        let taken = held
            .take(&["t3".to_owned(), "t1".to_owned(), "t2".to_owned()])
            .expect("all three are here");

        assert_eq!(
            taken
                .iter()
                .map(|file| file.name.as_str())
                .collect::<Vec<_>>(),
            vec!["c.txt", "a.txt", "b.txt"]
        );
    }

    #[test]
    fn a_file_is_taken_once_and_cannot_be_attached_to_a_second_message() {
        // A token that survived its message would be a file the next one could carry without
        // anybody choosing it — somebody's invoice attached to a message they never meant it for.
        let held = FilesBeingAttached::default();
        held.hold("t1".to_owned(), a_file("a.txt"));

        assert_eq!(
            held.take(&["t1".to_owned()]).map(|files| files.len()),
            Some(1)
        );
        assert_eq!(held.take(&["t1".to_owned()]), None);
    }

    #[test]
    fn nothing_is_taken_when_one_of_the_files_is_gone() {
        // One file gone is a message that cannot be sent as written, and not a reason to lose the
        // others: they stay, and the window that still shows them can send them once told.
        let held = FilesBeingAttached::default();
        held.hold("t1".to_owned(), a_file("a.txt"));

        assert_eq!(held.take(&["t1".to_owned(), "gone".to_owned()]), None);
        assert_eq!(
            held.take(&["t1".to_owned()]).map(|files| files.len()),
            Some(1),
            "the file that was there went with the one that was not"
        );
    }

    #[test]
    fn a_token_named_twice_takes_nothing_and_loses_nothing() {
        // What a window sends is not trusted to be what this one would: a file named twice is a
        // message that cannot be sent as written, and the file stays for one that names it once.
        let held = FilesBeingAttached::default();
        held.hold("t1".to_owned(), a_file("a.txt"));
        held.hold("t2".to_owned(), a_file("b.txt"));

        assert_eq!(
            held.take(&["t1".to_owned(), "t2".to_owned(), "t1".to_owned()]),
            None
        );
        assert_eq!(
            held.take(&["t1".to_owned(), "t2".to_owned()])
                .map(|files| files.len()),
            Some(2),
            "a file named twice took the files with it"
        );
    }

    #[test]
    fn files_given_back_are_there_to_be_sent_again() {
        let held = FilesBeingAttached::default();
        held.hold("t1".to_owned(), a_file("a.txt"));
        held.hold("t2".to_owned(), a_file("b.txt"));
        let tokens = ["t1".to_owned(), "t2".to_owned()];

        let taken = held.take(&tokens).expect("both are here");
        held.give_back(&tokens, taken);

        assert_eq!(
            held.take(&tokens)
                .expect("both are back")
                .iter()
                .map(|file| file.name.as_str())
                .collect::<Vec<_>>(),
            vec!["a.txt", "b.txt"]
        );
    }

    #[test]
    fn changing_your_mind_forgets_the_file() {
        let held = FilesBeingAttached::default();
        held.hold("t1".to_owned(), a_file("a.txt"));

        held.forget("t1");

        assert_eq!(held.take(&["t1".to_owned()]), None);
    }

    /// What a keep is given for these tokens, as (name kept as, file name) pairs.
    fn kept(held: &FilesBeingAttached, tokens: &[&str]) -> Option<Vec<(String, String)>> {
        let tokens: Vec<String> = tokens.iter().map(|token| (*token).to_owned()).collect();
        held.to_keep(&tokens).map(|files| {
            files
                .into_iter()
                .map(|one| (one.kept_as, one.file.name.clone()))
                .collect()
        })
    }

    #[test]
    fn a_keep_names_each_file_by_its_token_and_takes_none_of_them() {
        // Kept, not sent: the message is still being written, and sending takes the files from
        // here — a keep that spent the token would leave the message nothing to send.
        let held = FilesBeingAttached::default();
        held.hold("t1".to_owned(), a_file("a.txt"));
        held.hold("t2".to_owned(), a_file("b.txt"));

        assert_eq!(
            kept(&held, &["t2", "t1"]),
            Some(vec![
                ("t2".to_owned(), "b.txt".to_owned()),
                ("t1".to_owned(), "a.txt".to_owned())
            ])
        );
        assert_eq!(
            held.take(&["t1".to_owned(), "t2".to_owned()])
                .map(|files| files.len()),
            Some(2),
            "keeping took the files"
        );
    }

    #[test]
    fn a_keep_is_handed_the_bytes_held_here_rather_than_a_copy() {
        // Named at every keep, a photograph copied each time would be eighteen megabytes of memory
        // a second or so after every pause in the typing.
        let held = FilesBeingAttached::default();
        held.hold("t1".to_owned(), a_file("a.txt"));
        let tokens = ["t1".to_owned()];

        let first = held.to_keep(&tokens).expect("it is here");
        let second = held.to_keep(&tokens).expect("it is still here");

        assert!(Arc::ptr_eq(&first[0].file, &second[0].file));
    }

    #[test]
    fn a_file_of_a_draft_opened_again_is_kept_under_the_name_the_draft_knows_it_by() {
        // A fresh token, so a composer is never handed one another composer had; and the draft's
        // own name for the file behind it, so the draft does not take it for a new one and write
        // it all over again.
        let held = FilesBeingAttached::default();
        held.hold_kept(
            "fresh".to_owned(),
            "t1".to_owned(),
            Arc::new(a_file("a.txt")),
        );

        assert_eq!(
            kept(&held, &["fresh"]),
            Some(vec![("t1".to_owned(), "a.txt".to_owned())])
        );
    }

    #[test]
    fn a_keep_naming_a_file_that_is_not_here_or_naming_one_twice_is_given_nothing() {
        // Nothing rather than the rest: a draft written without one of the files on screen is a
        // draft that comes back without it.
        let held = FilesBeingAttached::default();
        held.hold("t1".to_owned(), a_file("a.txt"));
        held.hold_kept(
            "fresh".to_owned(),
            "t1".to_owned(),
            Arc::new(a_file("a.txt")),
        );

        assert_eq!(kept(&held, &["t1", "gone"]), None, "a file that is gone");
        assert_eq!(
            kept(&held, &["fresh", "fresh"]),
            None,
            "a token named twice"
        );
        assert_eq!(
            kept(&held, &["t1", "fresh"]),
            None,
            "two tokens standing for one file of the draft"
        );
    }

    #[test]
    fn a_file_given_back_to_a_message_refused_keeps_the_name_the_draft_knows_it_by() {
        // Refused before it was written down, the message is still being written, and the draft
        // it was kept as still holds the file under that name.
        let held = FilesBeingAttached::default();
        held.hold_kept(
            "fresh".to_owned(),
            "t1".to_owned(),
            Arc::new(a_file("a.txt")),
        );
        let tokens = ["fresh".to_owned()];

        let taken = held.take(&tokens).expect("it is here");
        held.give_back(&tokens, taken);

        assert_eq!(
            kept(&held, &["fresh"]),
            Some(vec![("t1".to_owned(), "a.txt".to_owned())])
        );
    }

    #[test]
    fn a_file_taken_while_a_keep_has_it_in_hand_is_taken_whole() {
        let held = FilesBeingAttached::default();
        held.hold("t1".to_owned(), a_file("a.txt"));
        let tokens = ["t1".to_owned()];
        let in_hand = held.to_keep(&tokens).expect("it is here");

        let taken = held.take(&tokens).expect("it is here");

        assert_eq!(taken, vec![a_file("a.txt")]);
        assert_eq!(*in_hand[0].file, a_file("a.txt"));
    }

    /// A file of `size` bytes.
    fn weighing(name: &str, size: usize) -> OutgoingFile {
        OutgoingFile {
            bytes: vec![0; size],
            ..a_file(name)
        }
    }

    #[test]
    fn a_file_too_heavy_to_go_beside_the_others_is_refused_as_it_is_chosen() {
        // Held, it is a message that can never be sent and a draft no keep writes down, with
        // everything typed after it. Refused now, somebody can still choose another.
        use lyonoyl_domain::outgoing::HEAVIEST_MESSAGE;
        let held = FilesBeingAttached::default();
        held.hold_kept(
            "fresh".to_owned(),
            "k1".to_owned(),
            Arc::new(weighing("a.bin", HEAVIEST_MESSAGE / 2)),
        );
        let rest = HEAVIEST_MESSAGE - HEAVIEST_MESSAGE / 2;
        let beside = ["fresh".to_owned()];

        assert!(!held.hold_beside("t1".to_owned(), weighing("b.bin", rest + 1), &beside));
        assert_eq!(kept(&held, &["t1"]), None, "the file refused is held");
        assert!(
            held.hold_beside("t2".to_owned(), weighing("c.bin", rest), &beside),
            "a file that fits exactly was refused"
        );
        assert!(kept(&held, &["fresh", "t2"]).is_some());
    }

    #[test]
    fn only_the_files_named_weigh_and_each_of_them_once() {
        // Somebody else's message is not this one, a file let go of is not on its way, and a file
        // named twice is one file.
        use lyonoyl_domain::outgoing::HEAVIEST_MESSAGE;
        let held = FilesBeingAttached::default();
        held.hold("elsewhere".to_owned(), weighing("a.bin", HEAVIEST_MESSAGE));
        held.hold("t1".to_owned(), weighing("b.bin", HEAVIEST_MESSAGE / 2));
        let rest = HEAVIEST_MESSAGE - HEAVIEST_MESSAGE / 2;

        assert!(held.hold_beside(
            "t2".to_owned(),
            weighing("c.bin", rest),
            &["t1".to_owned(), "t1".to_owned(), "gone".to_owned()],
        ));
    }

    #[test]
    fn forgetting_a_file_of_a_draft_opened_again_forgets_it_whole() {
        let held = FilesBeingAttached::default();
        held.hold_kept(
            "fresh".to_owned(),
            "t1".to_owned(),
            Arc::new(a_file("a.txt")),
        );

        held.forget("fresh");

        assert_eq!(kept(&held, &["fresh"]), None);
        assert!(
            held.held.lock().expect("test lock").kept_as.is_empty(),
            "what the draft knows it by outlived it"
        );
    }
}
