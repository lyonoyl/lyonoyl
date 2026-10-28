//! Writing down what happened, in a file beside the databases.
//!
//! One writer thread and a channel to it. Nothing a caller does reaches the filesystem on the
//! caller's thread: a record is handed over and the caller carries on. That is not tidiness — a
//! blocking write on a thread holding `BEGIN IMMEDIATE` queues every other writer behind it
//! (invariant 6), and one inside the picture fetcher stalls a runtime worker that six fetches are
//! sharing.
//!
//! **What is written is bounded by the type, not by care.** Everything a record may carry is in
//! [`crate::ports::journal`], and none of it can hold a string, so no address, subject, filename or
//! third-party error text can arrive here to be written down. What this file adds is the frame
//! around those values, and the frame is written by this build alone.
//!
//! **A record is never lost silently.** The channel has a ceiling; past it a record is dropped and
//! counted, and the count is written into the file at the next drain. A log that quietly stops
//! being complete is a log that answers a question wrongly.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, OnceLock};

use crate::ports::journal::{Field, Journal, Note, Value};

pub mod files;

/// How many records may be waiting before one is dropped.
///
/// About a megabyte of them. Past this the writer is not keeping up, and waiting for it is the one
/// thing this design refuses to do.
const WAITING_AT_MOST: usize = 8192;

/// One line, already rendered. Rendering happens on the caller's thread on purpose: it is a few
/// hundred nanoseconds of `write!` into a `String`, and doing it here means the channel carries no
/// borrowed anything and the writer thread does no work that can fail.
type ALine = String;

/// The journal this run writes to. Set once, before the window.
static WRITING: OnceLock<Arc<TheJournal>> = OnceLock::new();

/// The one journal, or one that writes nothing when none was installed — which is every test that
/// is not about the journal, and a run whose folder could not be made.
pub fn journal() -> Arc<TheJournal> {
    WRITING
        .get()
        .cloned()
        .unwrap_or_else(|| Arc::new(TheJournal::nowhere()))
}

/// Starts writing, in a folder beside the databases. Answers with where it is writing, if anywhere.
///
/// Called once. A second call is ignored rather than refused: two journals would be two files
/// interleaving, and a caller has nothing useful to do with the failure.
pub fn install(beside: &std::path::Path, note: Note) -> Option<PathBuf> {
    let made = TheJournal::in_directory(beside, note).ok()?;
    let where_it_writes = made.writing_to.clone();

    match WRITING.set(Arc::new(made)) {
        Ok(()) => where_it_writes,
        // Already installed. The one that is running keeps running.
        Err(_) => journal().writing_to.clone(),
    }
}

/// Hands the writer what is waiting and gives it a moment to finish.
///
/// Called on the way out, where "a moment" is bounded: an application that will not close because
/// it is writing a log is worse than one that loses its last few lines.
pub fn flush() {
    journal().flush_within(std::time::Duration::from_millis(250));
}

/// The journal.
pub struct TheJournal {
    to_the_writer: Option<SyncSender<ALine>>,
    /// Written down and read back at the next drain, so a gap in the file says so out loud.
    dropped: AtomicU64,
    /// How many the writer has taken, so a flush can wait for a number rather than for a marker
    /// that the same pressure would drop.
    handed_over: AtomicU64,
    written: Arc<AtomicU64>,
    note: Note,
    writing_to: Option<PathBuf>,
    began: std::time::Instant,
}

impl TheJournal {
    /// One that writes nothing at all, and costs nothing to ask.
    pub fn nowhere() -> Self {
        Self {
            to_the_writer: None,
            dropped: AtomicU64::new(0),
            handed_over: AtomicU64::new(0),
            written: Arc::new(AtomicU64::new(0)),
            note: Note::Trouble,
            writing_to: None,
            began: std::time::Instant::now(),
        }
    }

    /// One writing into `beside`/`logs`.
    fn in_directory(beside: &std::path::Path, note: Note) -> std::io::Result<Self> {
        let directory = files::ready(beside)?;
        let started = stamp(std::time::SystemTime::now());
        let process = std::process::id();
        let run = format!("{started}-{process}");

        // Before the first line rather than after: a folder already at its ceiling would otherwise
        // spend this run over it.
        let _ = files::make_room(&directory, &run);

        let first = directory.join(files::named(&started, process, 0));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&first)?;

        let (to_the_writer, waiting) = sync_channel::<ALine>(WAITING_AT_MOST);
        let written = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&written);

        std::thread::Builder::new()
            .name("journal".to_owned())
            .spawn(move || the_writer(waiting, file, directory, started, process, counted))?;

        let journal = Self {
            to_the_writer: Some(to_the_writer),
            dropped: AtomicU64::new(0),
            handed_over: AtomicU64::new(0),
            written,
            note,
            writing_to: Some(first),
            began: std::time::Instant::now(),
        };

        // The first line of every file says which run wrote it and how much it was asked to say,
        // so a file that looks empty of what you wanted can be told from one that was never asked.
        journal.write_down(Note::Trouble, "journal.began", &[]);

        Ok(journal)
    }

    /// Waits until the writer has taken everything handed to it, or until the time is up.
    fn flush_within(&self, within: std::time::Duration) {
        let wanted = self.handed_over.load(Ordering::SeqCst);
        let until = std::time::Instant::now() + within;

        while self.written.load(Ordering::SeqCst) < wanted {
            if std::time::Instant::now() >= until {
                return;
            }
            std::thread::yield_now();
        }
    }
}

impl Journal for TheJournal {
    fn keeps(&self, note: Note) -> bool {
        self.to_the_writer.is_some() && note.reaches(self.note)
    }

    fn write_down(&self, note: Note, event: &'static str, fields: &[Field]) {
        let Some(to_the_writer) = &self.to_the_writer else {
            return;
        };
        if !note.reaches(self.note) {
            return;
        }

        let mut line = String::with_capacity(96 + fields.len() * 16);
        let _ = write!(
            line,
            "{} {:>8} {:<7} {event}",
            now(),
            self.began.elapsed().as_millis(),
            note.as_str()
        );

        // Said before anything else this drain carries, so a reader meets the gap before the
        // records that surround it rather than after.
        let lost = self.dropped.swap(0, Ordering::Relaxed);
        if lost > 0 {
            let _ = write!(line, " dropped={lost}");
        }

        for (name, value) in fields {
            let _ = write!(line, " {name}={}", written(*value));
        }
        line.push('\n');

        match to_the_writer.try_send(line) {
            Ok(()) => {
                self.handed_over.fetch_add(1, Ordering::SeqCst);
            }
            // The writer is behind. Counted, and said out loud at the next drain: a log that goes
            // quiet under load and does not say so answers the next question wrongly.
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

/// Written down the first time the window asks the core anything, and never again.
///
/// Without it the journal steps straight from the window appearing to the first mailbox being
/// read, and everything the window spends getting itself ready falls in that gap unmeasured.
pub fn the_window_asked_first_at() {
    use crate::ports::journal::{Journal, Note};
    static ONCE: std::sync::Once = std::sync::Once::new();

    ONCE.call_once(|| {
        let journal = journal();
        if journal.keeps(Note::Detail) {
            journal.write_down(Note::Detail, "start.windowAsked", &[]);
        }
    });
}

/// Writes down what one pass over one account cost.
///
/// Here rather than at either caller, because there are two and they must say the same thing: a
/// pass a person asked for and a pass a round ran are the same work, and only one of them being
/// recorded is a journal that answers "why is this slow" about half the passes there were. That is
/// what it did — the button and the sidebar's own ask wrote nothing at all.
///
/// Counts and a clock. No account, no mailbox, no address: none of them help answer it.
pub fn wrote_down_a_pass(pass: &crate::application::sync::AccountPass, took: std::time::Duration) {
    use crate::ports::journal::{Journal, Note, Value};

    let journal = journal();
    if !journal.keeps(Note::Detail) {
        return;
    }

    journal.write_down(
        Note::Detail,
        "sync.read",
        &[
            ("took", Value::Millis(took.as_millis() as u64)),
            (
                "saidHello",
                Value::Millis(pass.opening_took.as_millis() as u64),
            ),
            ("opened", Value::Count(pass.opened.altogether())),
            ("folders", Value::Count(u64::from(pass.folders))),
            ("arrived", Value::Count(u64::from(pass.arrived))),
            ("bodies", Value::Count(u64::from(pass.bodies_read))),
            ("bodiesFailed", Value::Count(u64::from(pass.bodies_failed))),
            // Folders that could not be read, and folders read a second time because the first
            // reading failed: a pass that found no mail because its inbox would not open looks
            // like a quiet account without them.
            ("failed", Value::Count(u64::from(pass.folders_failed))),
            ("again", Value::Count(u64::from(pass.folders_read_again))),
            // Which phase spent them. `opened` alone says a pass cost so many conversations and
            // not which, and the four below are four different defects.
            // Where the time went, beside where the connections went. A phase that opens nothing
            // of its own still sends commands over the connection it was handed, and every one of
            // those is a wait nothing here could see.
            (
                "inList",
                Value::Millis(pass.spent.discovery.as_millis() as u64),
            ),
            (
                "inFolders",
                Value::Millis(pass.spent.folders.as_millis() as u64),
            ),
            (
                "inBackfill",
                Value::Millis(pass.spent.backfill.as_millis() as u64),
            ),
            (
                "inBodies",
                Value::Millis(pass.spent.bodies.as_millis() as u64),
            ),
            ("onList", Value::Count(pass.opened.discovery)),
            ("onFolders", Value::Count(pass.opened.folders)),
            ("onBackfill", Value::Count(pass.opened.backfill)),
            ("onBodies", Value::Count(pass.opened.bodies)),
        ],
    );
}

fn written(value: Value) -> String {
    match value {
        Value::Count(how_many) => how_many.to_string(),
        // Rounded up to a power of two. An exact length is a fingerprint of which message was
        // opened to anybody holding a copy of the mail, and nothing this log answers needs it.
        Value::Bytes(how_many) => format!("~{}", to_a_power_of_two(how_many)),
        Value::Millis(how_long) => format!("{how_long}ms"),
        Value::Yes(whether) => match whether {
            true => "yes".to_owned(),
            false => "no".to_owned(),
        },
        Value::Word(word) => word.as_str().to_owned(),
    }
}

/// The next power of two at or above this, so a size says its order and not its identity.
fn to_a_power_of_two(how_many: u64) -> u64 {
    match how_many {
        0 => 0,
        more => more.next_power_of_two(),
    }
}

use std::fmt::Write as _;

/// The moment, as a record carries it: the day and the time, in UTC.
///
/// UTC and not the reader's own clock, and the reason is worth a line: the offset a system is in
/// cannot be asked for safely from a thread that is not the first one, and an offset resolved once
/// and then changed by a flight or a season is a log that lies about an hour. A reader correlating
/// "I clicked at noon" does one subtraction, and knows which way.
fn now() -> String {
    stamp_with_time(std::time::SystemTime::now())
}

/// The moment a file name carries: the day and the time, without the punctuation a filesystem
/// argues about.
fn stamp(at: std::time::SystemTime) -> String {
    let seconds = at
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (year, month, day) = civil(seconds / 86_400);
    let day_seconds = seconds % 86_400;

    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        day_seconds / 3600,
        (day_seconds % 3600) / 60,
        day_seconds % 60
    )
}

fn stamp_with_time(at: std::time::SystemTime) -> String {
    let since = at.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let seconds = since.as_secs();
    let (year, month, day) = civil(seconds / 86_400);
    let day_seconds = seconds % 86_400;

    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        day_seconds / 3600,
        (day_seconds % 3600) / 60,
        day_seconds % 60,
        since.subsec_millis()
    )
}

/// The day number since the epoch, as a year, a month and a day.
///
/// Written out rather than taken from a crate: a date is the only thing this file needs from a
/// calendar, the algorithm is a published one, and a dependency here would be several crates in an
/// application that already ships six hundred.
fn civil(days: u64) -> (i64, u32, u32) {
    let days = days as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let of_era = days.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let counted = (5 * of_year + 2) / 153;
    let day = (of_year - (153 * counted + 2) / 5 + 1) as u32;
    let month = match counted < 10 {
        true => counted + 3,
        false => counted - 9,
    } as u32;

    (year + i64::from(month <= 2), month, day)
}

/// The one thread that touches the filesystem.
///
/// Drains what is waiting, joins it, and writes it once. No buffer is kept in this process: eight
/// kilobytes of the most interesting records sitting in a `BufWriter` when the process dies is
/// exactly the log nobody can use. What a returned `write` hands over belongs to the kernel, so
/// this survives a panic, an abort and a kill — and not a power cut, which nobody debugs from a
/// log file anyway.
fn the_writer(
    waiting: std::sync::mpsc::Receiver<ALine>,
    mut file: std::fs::File,
    directory: PathBuf,
    started: String,
    process: u32,
    written: Arc<AtomicU64>,
) {
    let mut in_this_file: u64 = 0;
    let mut which: u32 = 0;

    while let Ok(first) = waiting.recv() {
        let mut batch = first;
        let mut taken = 1u64;

        // Everything else already waiting, in one write.
        while let Ok(next) = waiting.try_recv() {
            batch.push_str(&next);
            taken += 1;
            if batch.len() >= 64 * 1024 {
                break;
            }
        }

        if in_this_file + batch.len() as u64 > files::LONGEST_FILE
            && which + 1 < files::MOST_PER_RUN
        {
            which += 1;
            if let Ok(next) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(directory.join(files::named(&started, process, which)))
            {
                file = next;
                in_this_file = 0;
            }
        }

        // A run that has filled every file it may have stops writing rather than rolling for ever:
        // one run in a loop must not be able to push out every other run's records.
        let room_left = in_this_file < files::LONGEST_FILE || which + 1 < files::MOST_PER_RUN;
        if room_left && file.write_all(batch.as_bytes()).is_ok() {
            in_this_file += batch.len() as u64;
        }

        written.fetch_add(taken, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests;
