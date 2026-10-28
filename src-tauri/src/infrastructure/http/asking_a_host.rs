//! One question to one host, over the road mail already travels.
//!
//! Its own file because it is a protocol and nothing else: what may be asked for, and of whom, is
//! decided next door. What is here is how a request is written and how an answer is read.
//!
//! **Why this exists rather than a client that already knows HTTP.** Mail goes out through
//! `infrastructure/net` — this project's own deadlines, its own address policy, and a connection
//! attempt made towards every address a name answers with, two hundred and fifty milliseconds
//! apart, whichever answers first winning (RFC 8305). A general client decides all of that for
//! itself, and the only lever left is the order the resolver hands addresses back in. On a machine
//! whose IPv6 answers nothing that difference is the whole reason mail arrives and a picture does
//! not: mail tries both families at once and the picture waits out a timeout on the one that is
//! dead.
//!
//! One road, then. What is written here is the small part of HTTP a picture needs: a request with
//! nothing about this machine in it, a status, headers, a body that is either counted or chunked,
//! and a ceiling on every one of those.

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::infrastructure::http::HttpError;
use crate::infrastructure::net::{Deadline, Encryption, Endpoint, Silence, TransportPolicy};

/// The most of a header block this build will read.
///
/// A host that never stops sending headers is a host that would otherwise fill this machine's
/// memory one line at a time. Real answers are a few hundred bytes.
const LONGEST_HEADERS: usize = 32 * 1024;

/// The most a line announcing one piece of a chunked body may be: the length in hexadecimal, and
/// whatever extensions a host hangs after it.
///
/// A length is a handful of digits. Without a ceiling of its own, a line may run as long as the
/// whole answer is allowed to, held until it ends — and a host that never ends one is filling this
/// machine's memory a byte at a time.
const LONGEST_CHUNK_LINE: usize = 1024;

/// How long a host may say nothing before the answer is given up on.
const SILENCE_ALLOWED: std::time::Duration = std::time::Duration::from_secs(20);

/// How long any exchange may take, before the size of its answer is counted in.
///
/// The silence above starts again with every byte, so on its own it lets a host that sends one byte
/// in every nineteen seconds hold a request open for as long as it likes. This is the bound that
/// never starts again.
const TIME_FOR_ANY_ANSWER: std::time::Duration = std::time::Duration::from_secs(30);

/// The slowest an answer may arrive, taken over the whole of it, and still be waited for: thirty-two
/// kilobytes a second, about what a link of a quarter of a megabit carries.
const SLOWEST_WAITED_FOR: u64 = 32 * 1024;

/// What one host said.
pub(super) struct Answered {
    pub status: u16,
    /// As the host stated it, lowercased and without its parameters.
    pub content_type: String,
    /// Where the host is sending this on to, if it is.
    pub sent_on_to: Option<String>,
    pub bytes: Vec<u8>,
}

/// Told as the answer arrives: how much is here, and how much is coming if the host said.
///
/// Answering `false` stops the download and the call fails with [`HttpError::Offline`] — which is
/// what a person pressing *Stop* looks like from here. A caller that does not care hands over a
/// closure that always answers `true`.
///
/// The bytes still come back whole rather than through this. The ceiling already bounds what is
/// held, and a second path that streamed to a file would be a second set of failures to get right
/// for a download that happens once in a while.
pub(super) type Arriving<'a> = &'a mut dyn FnMut(usize, Option<usize>) -> bool;

/// Asks one host for one address, reading no more than `at_most` bytes of the answer.
///
/// Nothing is followed here: a redirect comes back as `sent_on_to` for the caller to judge, because
/// whether an address may be reached is not this file's question.
pub(super) async fn ask(
    policy: &TransportPolicy,
    url: &url::Url,
    agent: &str,
    accepting: &str,
    at_most: usize,
    judging: bool,
    watching: Arriving<'_>,
) -> Result<Answered, HttpError> {
    let host = url.host_str().ok_or(HttpError::Address)?.to_owned();
    let encryption = match url.scheme() {
        "https" => Encryption::Tls,
        // Plain, and it is the sender's choice rather than this application's. Mail is refused in
        // the clear because what travels on it is a password and somebody's own writing; a picture
        // a message points at is neither, the address was written by whoever sent the message, and
        // refusing the scheme would drop the picture rather than protect anything. The request
        // carries no credential, no cookie and nothing about this machine.
        "http" => Encryption::None,
        _ => return Err(HttpError::Address),
    };
    let port = url.port_or_known_default().ok_or(HttpError::Address)?;

    // Asked before anything is looked up, because a lookup is the wrong instrument for both names
    // it catches: `localhost` is answered from a file rather than by a name server, and a `.local`
    // name is answered by whatever is on the local network — asking is itself a packet sent to the
    // neighbours on behalf of a message.
    //
    // **Nothing in this suite fails when this is deleted, and that is the point of it.** Every name
    // it catches is refused a moment later anyway, by the address policy at the connection: both
    // paths end in the same refusal with the same word. What differs is only whether a question
    // about the reader's own network went out on the way, and a test that watched the answer cannot
    // see a question that was never asked. The check stays on that ground rather than on a green
    // tick, and a mutation surviving here is the instrument's limit rather than the code's.
    //
    // Here rather than in the transport, because mail may reach a server on the local network and a
    // picture may not: the same name is an ordinary host for one and the neighbours for the other.
    if judging && super::a_name_that_means_here(&host) {
        return Err(HttpError::Address);
    }

    let endpoint = Endpoint {
        host: host.clone(),
        port,
        encryption,
    };
    let opened = match judging {
        true => policy.connect_to_a_host_a_message_named(&endpoint).await,
        #[cfg(test)]
        false => policy.connect_anywhere_at_all(&endpoint).await,
        #[cfg(not(test))]
        false => policy.connect_to_a_host_a_message_named(&endpoint).await,
    };
    let stream = opened.map_err(|why| match why {
        crate::infrastructure::net::TransportError::Unreachable => HttpError::Address,
        _ => HttpError::Offline,
    })?;

    // What is asked for, and nothing else. No cookie, no referrer, no encoding this build would
    // then have to undo, and a user agent that is a name rather than an inventory of this machine.
    // `close` because one request is one connection: keeping it alive would be a pool to manage for
    // no round trip saved.
    //
    // What is acceptable comes from the caller. One value for every request is wrong for all but
    // one kind of them: a host that takes the header seriously answers a request for a document
    // that accepts only pictures with a refusal, which is a service failing for a reason nothing in
    // it names.
    let asking = format!(
        "GET {path} HTTP/1.1\r\nHost: {host_header}\r\nUser-Agent: {agent}\r\n\
         Accept: {accepting}\r\nAccept-Encoding: identity\r\nConnection: close\r\n\r\n",
        path = what_to_ask_for(url),
        host_header = written_as_a_header(&host, port, encryption),
    );

    the_exchange(
        &mut Deadline::over(stream, Silence::of(SILENCE_ALLOWED)),
        &asking,
        at_most,
        watching,
    )
    .await
}

/// How long one exchange may take, from the first byte of the request to the last of the answer.
///
/// The same time for any answer, and on top of it the time the most the caller will take needs at
/// the slowest rate still waited for. A configuration document is given half a minute and a little
/// more, a picture a few minutes, an update as long as its size takes at that rate — and none of
/// them for ever. Opening the connection is not counted: it is bounded where it is made.
fn time_allowed(at_most: usize) -> std::time::Duration {
    let at_most = u64::try_from(at_most).unwrap_or(u64::MAX);

    TIME_FOR_ANY_ANSWER.saturating_add(std::time::Duration::from_millis(
        at_most.saturating_mul(1000) / SLOWEST_WAITED_FOR,
    ))
}

/// One request written and its answer read, over a connection already open, within the time an
/// answer of `at_most` is allowed.
async fn the_exchange<S>(
    talking: &mut S,
    asking: &str,
    at_most: usize,
    watching: Arriving<'_>,
) -> Result<Answered, HttpError>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let exchanging = async move {
        talking
            .write_all(asking.as_bytes())
            .await
            .map_err(|_| HttpError::Offline)?;
        talking.flush().await.map_err(|_| HttpError::Offline)?;

        let (head, first) = the_head_of_the_answer(talking).await?;
        let (status, headers) = read_the_head(&head)?;

        let content_type = header(&headers, "content-type")
            .map(|value| {
                value
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase()
            })
            .unwrap_or_default();
        let sent_on_to = header(&headers, "location").map(str::to_owned);

        // Refused on what the host says before a byte of it is read, and again on what it actually
        // sends: a host that states one length and sends another is a host, not a mistake.
        if header(&headers, "content-length")
            .and_then(|stated| stated.trim().parse::<u64>().ok())
            .is_some_and(|stated| stated > at_most as u64)
        {
            return Err(HttpError::TooLarge);
        }

        let bytes = match sent_on_to.is_some() && (300..400).contains(&status) {
            // Where it is going matters and what it says on the way does not.
            true => Vec::new(),
            false => {
                let counted = header(&headers, "content-length")
                    .and_then(|stated| stated.trim().parse::<usize>().ok());
                let chunked = header(&headers, "transfer-encoding")
                    .is_some_and(|how| how.to_ascii_lowercase().contains("chunked"));

                the_body(talking, first, counted, chunked, at_most, watching).await?
            }
        };

        Ok(Answered {
            status,
            content_type,
            sent_on_to,
            bytes,
        })
    };

    // An answer that has not finished arriving by then is a host that did not answer in time,
    // however steadily it was still talking.
    tokio::time::timeout(time_allowed(at_most), exchanging)
        .await
        .map_err(|_| HttpError::Offline)?
}

/// The part of the address that goes on the request line. Never the host: that is a header, and a
/// request line carrying the whole address is one a proxy answers.
fn what_to_ask_for(url: &url::Url) -> String {
    match url.query() {
        Some(query) => format!("{}?{}", url.path(), query),
        None => url.path().to_owned(),
    }
}

/// The host as the header spells it, with the port only when it is not the one the scheme implies.
fn written_as_a_header(host: &str, port: u16, encryption: Encryption) -> String {
    let usual = match encryption {
        Encryption::Tls => 443,
        _ => 80,
    };

    match port == usual {
        true => host.to_owned(),
        false => format!("{host}:{port}"),
    }
}

/// Everything up to the blank line, and whatever of the body arrived with it.
async fn the_head_of_the_answer<S>(talking: &mut S) -> Result<(Vec<u8>, Vec<u8>), HttpError>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut held: Vec<u8> = Vec::with_capacity(1024);
    let mut arriving = [0u8; 4096];

    loop {
        let read = talking
            .read(&mut arriving)
            .await
            .map_err(|_| HttpError::Offline)?;

        if read == 0 {
            return Err(HttpError::Offline);
        }

        // Looked for only where it can newly be: in what just arrived, and in the three bytes
        // before it, for a blank line that straddles two reads. Looking from the start every time
        // is a host sending its head a byte at a time and this machine reading all of it again
        // for every byte.
        let from = held.len().saturating_sub(3);
        held.extend_from_slice(&arriving[..read]);

        if let Some(blank) = held[from..].windows(4).position(|four| four == b"\r\n\r\n") {
            let body = held.split_off(from + blank + 4);
            held.truncate(from + blank);
            return Ok((held, body));
        }

        if held.len() > LONGEST_HEADERS {
            return Err(HttpError::TooLarge);
        }
    }
}

/// The status and the headers, as the two things a caller asks about.
fn read_the_head(head: &[u8]) -> Result<(u16, Vec<(String, String)>), HttpError> {
    let text = String::from_utf8_lossy(head);
    let mut lines = text.split("\r\n");

    let status = lines
        .next()
        .and_then(|first| first.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or(HttpError::Offline)?;

    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();

    Ok((status, headers))
}

fn header<'a>(headers: &'a [(String, String)], named: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(name, _)| name == named)
        .map(|(_, value)| value.as_str())
}

/// The body, however the host chose to send it, and never more than `at_most` of it.
///
/// Everything from here on is read through a [`Wire`], and so everything is counted: not only the
/// body, but what came in with the head and whatever a host wraps a body in.
async fn the_body<S>(
    talking: &mut S,
    already: Vec<u8>,
    counted: Option<usize>,
    chunked: bool,
    at_most: usize,
    watching: Arriving<'_>,
) -> Result<Vec<u8>, HttpError>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut wire = Wire::over(talking, already, at_most)?;

    match (chunked, counted) {
        (true, _) => the_body_in_chunks(&mut wire, at_most, watching).await,
        (false, Some(stated)) => the_body_as_stated(&mut wire, stated, watching).await,
        (false, None) => the_body_until_the_host_closes(&mut wire, at_most, watching).await,
    }
}

/// A body whose length the host stated: that many bytes exactly, and an answer that ends before
/// them is not one.
///
/// The length was held to `at_most` before a byte of it was read, so it needs no ceiling of its
/// own here — and the wire holds what is read to its allowance whatever was stated.
async fn the_body_as_stated<S>(
    wire: &mut Wire<'_, S>,
    stated: usize,
    watching: Arriving<'_>,
) -> Result<Vec<u8>, HttpError>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut held = Vec::new();

    loop {
        // Never past what was stated: whatever follows it on the connection is not this answer.
        let left = stated - held.len();
        wire.hand_over(&mut held, left);

        // Said before the first read as well as after each one. A download that is already whole
        // in the first packet would otherwise report nothing at all, and a person would watch a
        // bar that never moved for something that had already arrived.
        if !watching(held.len(), Some(stated)) {
            return Err(HttpError::Offline);
        }
        if held.len() == stated {
            return Ok(held);
        }

        // A connection that closes before the length it stated is a download cut short, and a
        // download cut short is never handed on as if it were whole.
        if !wire.more().await? {
            return Err(HttpError::Offline);
        }
    }
}

/// A body whose host stated no length, which ends when the host closes the connection.
async fn the_body_until_the_host_closes<S>(
    wire: &mut Wire<'_, S>,
    at_most: usize,
    watching: Arriving<'_>,
) -> Result<Vec<u8>, HttpError>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut held = Vec::new();

    loop {
        wire.hand_over(&mut held, usize::MAX);

        // Measured on what has arrived, since nothing was said: this is the ceiling that stops a
        // host that never says how long. It is asked of what came in with the head as well — a
        // host that sends its whole answer in one packet is the ordinary case, not a special one.
        if held.len() > at_most {
            return Err(HttpError::TooLarge);
        }
        if !watching(held.len(), None) {
            return Err(HttpError::Offline);
        }

        // Read until the host closes rather than until the ceiling: an answer that reaches the
        // ceiling exactly may still have more to send, and only the close says it has finished.
        if !wire.more().await? {
            return Ok(held);
        }
    }
}

/// A body a host sends in pieces, each announced by its length in hexadecimal.
async fn the_body_in_chunks<S>(
    wire: &mut Wire<'_, S>,
    at_most: usize,
    watching: Arriving<'_>,
) -> Result<Vec<u8>, HttpError>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut held: Vec<u8> = Vec::new();

    loop {
        // The length on a line of its own, then that many bytes, then the two that end them.
        let size = the_length_announced(&wire.line(LONGEST_CHUNK_LINE).await?)?;

        // The piece of no length is the last, and only it says the body is whole: a connection
        // that closes anywhere before it is a download cut short.
        if size == 0 {
            break;
        }
        // Against what is left rather than added to what is held: a length near the top of what a
        // machine word holds is a sum that does not fit.
        if size > at_most - held.len() {
            return Err(HttpError::TooLarge);
        }

        wire.piece(&mut held, size).await?;

        // No total to report: a host sending in pieces is a host that did not say how many there
        // are, which is exactly when a bar cannot be drawn and a count of what has arrived is all
        // there is to show.
        if !watching(held.len(), None) {
            return Err(HttpError::Offline);
        }
    }

    // After the last piece come its trailers: a head of the body's own, ended by an empty line.
    // Nothing in them is this build's business. They are read so that an answer counts as whole
    // only once its last line has arrived — held to what a head may be, and counted with the rest.
    let mut trailing = 0usize;
    loop {
        let line = wire.line(LONGEST_HEADERS.saturating_sub(trailing)).await?;
        if line.is_empty() {
            return Ok(held);
        }
        trailing += line.len() + 2;
    }
}

/// The length a piece announces, in hexadecimal. Whatever follows a semicolon is an extension, and
/// none of those is this build's business.
///
/// Counted a digit at a time, refusing what does not fit: a length past what a machine word holds
/// is an answer too large to read, rather than a number that wraps round into a small one.
fn the_length_announced(line: &[u8]) -> Result<usize, HttpError> {
    let digits = line
        .split(|&byte| byte == b';')
        .next()
        .unwrap_or_default()
        .trim_ascii();

    if digits.is_empty() {
        return Err(HttpError::Offline);
    }

    digits.iter().try_fold(0usize, |length, &digit| {
        let value = char::from(digit).to_digit(16).ok_or(HttpError::Offline)?;

        length
            .checked_mul(16)
            .and_then(|length| length.checked_add(value as usize))
            .ok_or(HttpError::TooLarge)
    })
}

/// Everything a body may take off the connection, for a caller that allows `at_most` of it.
///
/// `at_most` for the body itself; a sixty-fourth of that again for what a chunked body is wrapped
/// in; and what a head may be, for trailers and for the wrapping of a body cut into a few small
/// pieces. The sixty-fourth is there because a caller that knows the exact size of what it fetches
/// asks for exactly that, and a host is free to cut a body as small as it likes: a piece of half a
/// kilobyte is announced and ended in about seven bytes, so even a body of exactly `at_most` in
/// pieces that small still fits.
fn what_a_body_may_take(at_most: usize) -> usize {
    at_most
        .saturating_add(at_most / 64)
        .saturating_add(LONGEST_HEADERS)
}

/// The connection, as a body is read off it.
///
/// Everything a body takes off the connection is read through here, so all of it is counted
/// against [`what_a_body_may_take`]: what came in with the head, the lines announcing each piece
/// and the two bytes ending each, the trailers after the last. And what has been used is let go
/// before anything more is read, so what is held here at any moment is at most one line and one
/// read, never the answer so far.
struct Wire<'t, S> {
    talking: &'t mut S,
    /// What has arrived: used up to `used`, waiting from there on.
    arrived: Vec<u8>,
    used: usize,
    /// How much more may be read before the answer is larger than this build reads.
    allowed: usize,
}

impl<'t, S> Wire<'t, S>
where
    S: tokio::io::AsyncRead + Unpin,
{
    /// `already` is body that came in with the head: read, and so counted like any other.
    fn over(talking: &'t mut S, already: Vec<u8>, at_most: usize) -> Result<Self, HttpError> {
        let allowed = what_a_body_may_take(at_most)
            .checked_sub(already.len())
            .ok_or(HttpError::TooLarge)?;

        Ok(Self {
            talking,
            arrived: already,
            used: 0,
            allowed,
        })
    }

    /// What has arrived and is not used yet.
    fn waiting(&self) -> &[u8] {
        &self.arrived[self.used..]
    }

    /// Reads once more. `false` is a host that has closed the connection.
    async fn more(&mut self) -> Result<bool, HttpError> {
        // More is needed and nothing more is allowed: whatever the host would send next is past
        // the ceiling, so it is not read at all.
        if self.allowed == 0 {
            return Err(HttpError::TooLarge);
        }

        // What is used is let go before anything new is kept. Kept, it would be the whole answer
        // held twice over by the time it ended: once in the body and again here.
        if self.used > 0 {
            self.arrived.drain(..self.used);
            self.used = 0;
        }

        // Never asked for more than is still allowed, so the ceiling is on what is read rather
        // than on what is noticed after reading it.
        let mut arriving = [0u8; 8192];
        let room = arriving.len().min(self.allowed);
        let read = self
            .talking
            .read(&mut arriving[..room])
            .await
            .map_err(|_| HttpError::Offline)?;

        self.allowed -= read;
        self.arrived.extend_from_slice(&arriving[..read]);

        Ok(read > 0)
    }

    /// Moves what is waiting into `held`, no more than `most` of it, and says how much moved.
    fn hand_over(&mut self, held: &mut Vec<u8>, most: usize) -> usize {
        let moving = self.waiting().len().min(most);
        held.extend_from_slice(&self.arrived[self.used..self.used + moving]);
        self.used += moving;

        moving
    }

    /// One line, without the two bytes that end it.
    ///
    /// Refused once it has run past `longest` without ending, whether or not it would have ended
    /// later: a long line and an endless one look the same until the end arrives, and waiting to
    /// find out is holding whatever the host cares to send.
    async fn line(&mut self, longest: usize) -> Result<Vec<u8>, HttpError> {
        // The line, and the two bytes that end it.
        let most = longest.saturating_add(2);
        // How far the end has been looked for. Each byte is looked at once, however many reads
        // the line takes to arrive: what is waiting moves when what was used is let go, but it
        // moves as a whole, so a place in it stays the same place.
        let mut looked = 0;

        loop {
            let waiting = self.waiting();
            let within = waiting.len().min(most);

            if let Some(found) = waiting[looked..within]
                .iter()
                .position(|&byte| byte == b'\n')
            {
                let end = looked + found;
                // Both bytes end a line. A line feed on its own is not an end this build reads.
                if end == 0 || waiting[end - 1] != b'\r' {
                    return Err(HttpError::Offline);
                }

                let line = waiting[..end - 1].to_vec();
                self.used += end + 1;

                return Ok(line);
            }

            if within == most {
                return Err(HttpError::TooLarge);
            }
            looked = within;

            // A connection that closes inside a line is an answer that did not finish.
            if !self.more().await? {
                return Err(HttpError::Offline);
            }
        }
    }

    /// One piece of a chunked body, into `held`: `size` bytes, then the two that end them.
    async fn piece(&mut self, held: &mut Vec<u8>, size: usize) -> Result<(), HttpError> {
        let mut left = size;

        loop {
            left -= self.hand_over(held, left);
            if left == 0 {
                break;
            }
            // A connection that closes inside a piece is a download cut short.
            if !self.more().await? {
                return Err(HttpError::Offline);
            }
        }

        while self.waiting().len() < 2 {
            if !self.more().await? {
                return Err(HttpError::Offline);
            }
        }
        // The two bytes that end a piece, and nothing in their place: a piece longer than it said
        // is a host sending something other than what it announced.
        if !self.waiting().starts_with(b"\r\n") {
            return Err(HttpError::Offline);
        }
        self.used += 2;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    /// A host that sends `pieces`, waiting `apart` before each, and then closes the connection.
    ///
    /// Over a connection of its own rather than a script: a script has to be read to its end, and
    /// the exchange this is for is one given up on long before that.
    fn a_host_sending(pieces: Vec<Vec<u8>>, apart: Duration) -> tokio::io::DuplexStream {
        let (here, mut there) = tokio::io::duplex(64 * 1024);

        tokio::spawn(async move {
            for piece in pieces {
                tokio::time::sleep(apart).await;
                if there.write_all(&piece).await.is_err() {
                    return;
                }
            }
        });

        here
    }

    #[tokio::test(start_paused = true)]
    async fn a_host_that_keeps_trickling_is_given_up_on_all_the_same() {
        // A byte at a time, each inside the silence allowed: the silence never runs out, and the
        // answer would be whole some hours from now. The bound on the whole exchange ends it.
        let mut pieces = vec![b"HTTP/1.1 200 OK\r\n\r\n".to_vec()];
        pieces.extend(std::iter::repeat_n(b"x".to_vec(), 1024));
        let trickling = a_host_sending(pieces, SILENCE_ALLOWED - Duration::from_secs(1));
        let mut talking = Deadline::over(trickling, Silence::of(SILENCE_ALLOWED));

        let began = tokio::time::Instant::now();
        let outcome = the_exchange(&mut talking, "GET / HTTP/1.1\r\n\r\n", 1024, &mut |_, _| {
            true
        })
        .await;

        assert!(
            matches!(outcome, Err(HttpError::Offline)),
            "a host trickling for hours was waited for to the end"
        );
        assert!(
            began.elapsed() <= time_allowed(1024),
            "given up on after {:?}",
            began.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_answer_arriving_at_the_slowest_rate_waited_for_still_arrives() {
        // A megabyte, thirty-two kilobytes a second: longer than the time any answer is given, and
        // it arrives all the same, because the bound grows with what the caller will take. A
        // download over a slow link is not a host trickling.
        let piece = vec![b'x'; 32 * 1024];
        let mut pieces = vec![b"HTTP/1.1 200 OK\r\nContent-Length: 1048576\r\n\r\n".to_vec()];
        pieces.extend(std::iter::repeat_n(piece, 32));
        let slow = a_host_sending(pieces, Duration::from_secs(1));
        let mut talking = Deadline::over(slow, Silence::of(SILENCE_ALLOWED));

        let began = tokio::time::Instant::now();
        let answered = the_exchange(
            &mut talking,
            "GET / HTTP/1.1\r\n\r\n",
            1024 * 1024,
            &mut |_, _| true,
        )
        .await
        .expect("an answer arriving steadily at the slowest rate is waited for");

        assert_eq!(answered.bytes.len(), 1024 * 1024);
        assert!(
            began.elapsed() > TIME_FOR_ANY_ANSWER,
            "the answer came quickly enough to prove nothing: {:?}",
            began.elapsed()
        );
    }

    /// A host that sends its answer one byte at a time.
    ///
    /// Every line then arrives across reads of its own, the two bytes ending each are split
    /// between two of them, and the blank line ending the head arrives in four. Whatever is put
    /// together across reads is put together here, at every place it can be.
    fn a_byte_at_a_time(answer: &[u8]) -> tokio_test::io::Mock {
        let mut builder = tokio_test::io::Builder::new();
        for byte in answer {
            builder.read(std::slice::from_ref(byte));
        }

        builder.build()
    }

    #[tokio::test]
    async fn an_answer_arrives_whole_however_the_host_splits_it() {
        let mut talking = a_byte_at_a_time(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n\
              5;name=value\r\nhello\r\n7\r\n, world\r\n0\r\nNote: after\r\n\r\n",
        );

        let (_head, first) = the_head_of_the_answer(&mut talking).await.expect("a head");
        let body = the_body(&mut talking, first, None, true, 12, &mut |_, _| true)
            .await
            .expect("a body");

        assert_eq!(body, b"hello, world");
    }
}
