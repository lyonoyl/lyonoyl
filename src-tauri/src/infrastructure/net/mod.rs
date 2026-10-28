//! Connection policy shared by the adapters that reach it.
//!
//! Timeouts, TLS configuration and the rule about unencrypted connections live here rather than in
//! each adapter, so the ones that use it cannot drift apart on the settings that decide whether a
//! password crosses the network in the clear. **SMTP shares the rule and nothing else**: it asks
//! `is_this_machine` before it signs in without encryption, its TLS is the submission library's
//! own, and so are its timeouts but two, both on the plain-text path: the bound its adapter puts
//! around looking the server's name up, which it does itself to see where the name leads, and the
//! one around opening the connection, greeting included, which the library leaves unbounded. What
//! would put the rest behind this is the submission adapter reaching for `TransportPolicy` the way
//! the others do — a sentence here claiming it already does is how nobody notices that it does not.

// The encrypted path and the read timeout get their callers with the synchronising adapter.
#![allow(dead_code)]

use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use thiserror::Error;
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;
use tokio_rustls::TlsConnector;

/// The certificate check for every connection but mail: a picture, server discovery, the update
/// check. Only macOS has one of its own: the policy's constructor says why.
#[cfg(target_os = "macos")]
mod judged_as_sent;

/// How long a connection attempt may take before it is treated as unreachable.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a server may stay silent mid-conversation.
///
/// Short, because what follows a silence this long is a second attempt on a fresh connection
/// rather than a wait for the next round: a server that has not answered in ten seconds has, in
/// practice, stopped answering on that connection.
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a server may stay silent once it has already failed to answer in time.
///
/// A server slow to answer one command is often slow to answer the next, and a reading tried
/// again allows it longer rather than failing the same way twice.
const PATIENT_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// When a connection with nothing on it is asked whether the other side is still there, and how
/// often after that.
///
/// A connection a router or a firewall dropped without a word looks open for ever: a watch on it
/// hears nothing, mail arrives, and nobody is told. Asked at these intervals, the system finds a
/// connection gone in under a minute, and the traffic keeps the router's record of it alive.
const KEEP_ALIVE: Duration = Duration::from_secs(30);
const KEEP_ALIVE_EVERY: Duration = Duration::from_secs(10);

/// How long one address is given before the attempt to it is abandoned.
///
/// Bounded per address rather than over the list: a host with eight addresses, seven of them on a
/// family this machine cannot reach, used to spend the whole budget before trying the one that
/// answers.
const ONE_ADDRESS: Duration = Duration::from_secs(8);

/// How long the next address waits before it stops waiting for the one before it.
///
/// RFC 8305 calls this the connection attempt delay and puts it at 250 ms. Nothing is cancelled
/// when it expires — the earlier attempt keeps running, and whichever answers first is the one
/// that is used.
const BEFORE_THE_NEXT: Duration = Duration::from_millis(250);

/// How many times an attempt that failed for a passing reason is made in total.
///
/// Three, not more: what is behind a temporary failure is usually a network coming back, and a
/// round comes again in two minutes anyway. Trying for longer than that turns one unreachable
/// server into an account that never finishes a pass.
const ATTEMPTS: u32 = 3;

/// The ceiling of the first wait, doubled for each attempt after it.
const FIRST_BACKOFF: Duration = Duration::from_millis(500);

/// The longest a wait may grow to, however many attempts have failed.
const MAX_BACKOFF: Duration = Duration::from_secs(8);

#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
pub enum TransportError {
    #[error("the server could not be reached")]
    Unreachable,
    #[error("the connection timed out")]
    Timeout,
    #[error("the secure connection could not be established")]
    Tls,
    #[error("an unencrypted connection is only allowed to this machine")]
    PlaintextRefused,
    #[error("the encryption mode is not supported yet")]
    UnsupportedEncryption,
}

impl TransportError {
    /// Whether trying again in a moment could plausibly work.
    ///
    /// A network that is down comes back; a certificate that does not verify does not become valid
    /// by asking twice, and an unencrypted connection off this machine is a refusal, not a failure.
    /// Retrying either of those quietly is how a downgrade ends up looking normal.
    pub fn is_temporary(&self) -> bool {
        match self {
            Self::Unreachable | Self::Timeout => true,
            Self::Tls | Self::PlaintextRefused | Self::UnsupportedEncryption => false,
        }
    }
}

/// How long to wait before attempt number `attempt`, counting the first failure as attempt 0.
///
/// Full jitter: anywhere between nothing and a ceiling that doubles each time. The jitter is the
/// point, not the backoff. A hundred accounts live in one process here, and a network that comes
/// back hands all of them their failure in the same instant; without jitter they would all wait the
/// same 500 ms and hit the server together, which is the shape of a self-inflicted outage.
fn backoff(attempt: u32, entropy: u64) -> Duration {
    let ceiling = FIRST_BACKOFF
        .checked_mul(2u32.saturating_pow(attempt.min(16)))
        .unwrap_or(MAX_BACKOFF)
        .min(MAX_BACKOFF);

    let span = ceiling.as_nanos() as u64 + 1;
    Duration::from_nanos(entropy % span)
}

/// Enough randomness to break a lockstep, taken from what the process already has.
///
/// Not cryptographic and does not need to be: all it decides is how long to wait. The counter is
/// what keeps two accounts failing in the same nanosecond from waiting the same amount.
fn entropy() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let counter = NEXT.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| u64::from(since.subsec_nanos()))
        .unwrap_or(0);

    now.wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(counter.wrapping_mul(1_442_695_040_888_963_407))
}

/// Runs an attempt, and runs it again while it fails for a reason that passes.
///
/// Here rather than in each adapter: IMAP and SMTP disagreeing about what is worth retrying is how
/// one of them ends up hammering a server that is already struggling.
pub async fn retrying<T, F, Fut>(mut attempt: F) -> Result<T, TransportError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, TransportError>>,
{
    let mut failed = 0;

    loop {
        match attempt().await {
            Ok(value) => return Ok(value),
            Err(error) => {
                failed += 1;
                if failed >= ATTEMPTS || !error.is_temporary() {
                    return Err(error);
                }
                tokio::time::sleep(backoff(failed - 1, entropy())).await;
            }
        }
    }
}

/// How a connection is protected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encryption {
    /// TLS from the first byte, the usual choice for IMAP on port 993.
    Tls,
    /// A plain connection upgraded by a protocol command. Not implemented yet.
    StartTls,
    /// No encryption. Accepted only when the address is on this machine.
    None,
}

/// Where to connect and how to protect it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub encryption: Encryption,
}

/// A connected stream, encrypted or not.
///
/// `Debug` prints the kind and nothing else: a stream carries credentials and message contents.
pub enum Stream {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl std::fmt::Debug for Stream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Plain(_) => "Stream::Plain",
            Self::Tls(_) => "Stream::Tls",
        })
    }
}

/// How long a server is allowed to stay silent, as the one thing that can change while a
/// conversation is in progress.
///
/// What is measured is silence, not how long an operation takes — which is what makes one number
/// serve every command. Listing mailboxes and fetching a message with a video attached are not
/// alike in duration, but they are alike in this: while either is working, bytes keep arriving,
/// and each one starts the count again. Only a server that has stopped saying anything runs out.
#[derive(Clone, Debug)]
pub struct Silence(Arc<std::sync::atomic::AtomicU64>);

impl Silence {
    pub fn of(allowed: Duration) -> Self {
        Self(Arc::new(std::sync::atomic::AtomicU64::new(
            allowed.as_nanos().min(u128::from(u64::MAX)) as u64,
        )))
    }

    /// What is allowed right now.
    pub(crate) fn allowed(&self) -> Duration {
        Duration::from_nanos(self.0.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Allows `allowed` until the answer is dropped, and what was allowed before after that.
    ///
    /// For the one stretch of a conversation in which saying nothing is the answer expected — an
    /// `IDLE` — and for nothing around it: the allowance goes back however the stretch ends.
    pub(crate) fn for_a_while(&self, allowed: Duration) -> Meanwhile<'_> {
        let before = self.0.swap(
            allowed.as_nanos().min(u128::from(u64::MAX)) as u64,
            std::sync::atomic::Ordering::Relaxed,
        );

        Meanwhile {
            silence: self,
            before,
        }
    }
}

/// A different silence allowed for as long as this is held.
pub(crate) struct Meanwhile<'a> {
    silence: &'a Silence,
    before: u64,
}

impl Drop for Meanwhile<'_> {
    fn drop(&mut self) {
        self.silence
            .0
            .store(self.before, std::sync::atomic::Ordering::Relaxed);
    }
}

/// A stream that gives up when the other end goes quiet.
///
/// At the stream rather than around each command, and deliberately: every command a protocol
/// adapter sends goes through here, including the ones nobody has written yet, and a deadline that
/// has to be remembered per command is a deadline that will be forgotten.
///
/// Reading and writing stall independently. Writing needs guarding as much as reading, and flushing
/// most of all: TLS buffers, so a write can answer "done" while nothing has left the machine, and
/// a peer that has stopped reading shows up as a flush that never finishes. Both protocol adapters
/// here flush after every command, so that is where such a peer is caught.
pub struct Deadline<S> {
    inner: S,
    silence: Silence,
    reading: Option<Stall>,
    writing: Option<Stall>,
}

/// A wait that has begun, and the silence it was allowed when it began.
type Stall = (std::pin::Pin<Box<tokio::time::Sleep>>, Duration);

impl<S> Deadline<S> {
    pub fn over(inner: S, silence: Silence) -> Self {
        Self {
            inner,
            silence,
            reading: None,
            writing: None,
        }
    }

    /// Whether the stall that was already running has run out.
    ///
    /// Armed once and left alone: a poll that returns pending again must not push the deadline
    /// further away, or a server dribbling one byte a minute would be waited on for ever.
    ///
    /// Except when what is allowed has changed since it was armed. It then counts again from now,
    /// under what is allowed now — so a wait begun during an `IDLE` is held to the ordinary
    /// silence the moment the idle is over, rather than to the half hour the idle was allowed.
    fn ran_out(
        stall: &mut Option<Stall>,
        allowed: Duration,
        context: &mut std::task::Context<'_>,
    ) -> bool {
        if stall
            .as_ref()
            .is_some_and(|(_, armed_under)| *armed_under != allowed)
        {
            *stall = None;
        }
        let (sleeping, _) =
            stall.get_or_insert_with(|| (Box::pin(tokio::time::sleep(allowed)), allowed));

        matches!(sleeping.as_mut().poll(context), std::task::Poll::Ready(()))
    }
}

impl<S: std::fmt::Debug> std::fmt::Debug for Deadline<S> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "Deadline({:?})", self.inner)
    }
}

impl<S: tokio::io::AsyncRead + Unpin> tokio::io::AsyncRead for Deadline<S> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let this = self.get_mut();

        match std::pin::Pin::new(&mut this.inner).poll_read(context, buffer) {
            std::task::Poll::Ready(answer) => {
                this.reading = None;
                std::task::Poll::Ready(answer)
            }
            std::task::Poll::Pending => {
                match Self::ran_out(&mut this.reading, this.silence.allowed(), context) {
                    true => std::task::Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the server stopped answering",
                    ))),
                    false => std::task::Poll::Pending,
                }
            }
        }
    }
}

impl<S: tokio::io::AsyncWrite + Unpin> tokio::io::AsyncWrite for Deadline<S> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        bytes: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        let this = self.get_mut();

        match std::pin::Pin::new(&mut this.inner).poll_write(context, bytes) {
            std::task::Poll::Ready(answer) => {
                this.writing = None;
                std::task::Poll::Ready(answer)
            }
            std::task::Poll::Pending => {
                match Self::ran_out(&mut this.writing, this.silence.allowed(), context) {
                    true => std::task::Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the server stopped listening",
                    ))),
                    false => std::task::Poll::Pending,
                }
            }
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let this = self.get_mut();

        match std::pin::Pin::new(&mut this.inner).poll_flush(context) {
            std::task::Poll::Ready(answer) => {
                this.writing = None;
                std::task::Poll::Ready(answer)
            }
            std::task::Poll::Pending => {
                match Self::ran_out(&mut this.writing, this.silence.allowed(), context) {
                    true => std::task::Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the server stopped listening",
                    ))),
                    false => std::task::Poll::Pending,
                }
            }
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let this = self.get_mut();

        match std::pin::Pin::new(&mut this.inner).poll_shutdown(context) {
            std::task::Poll::Ready(answer) => {
                this.writing = None;
                std::task::Poll::Ready(answer)
            }
            std::task::Poll::Pending => {
                match Self::ran_out(&mut this.writing, this.silence.allowed(), context) {
                    true => std::task::Poll::Ready(Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "the server stopped listening",
                    ))),
                    false => std::task::Poll::Pending,
                }
            }
        }
    }
}

/// Both kinds of stream read and write the same way, so a protocol adapter never learns which one
/// it was given.
impl tokio::io::AsyncRead for Stream {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => std::pin::Pin::new(stream).poll_read(context, buffer),
            Self::Tls(stream) => std::pin::Pin::new(stream.as_mut()).poll_read(context, buffer),
        }
    }
}

impl tokio::io::AsyncWrite for Stream {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &[u8],
    ) -> std::task::Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(stream) => std::pin::Pin::new(stream).poll_write(context, buffer),
            Self::Tls(stream) => std::pin::Pin::new(stream.as_mut()).poll_write(context, buffer),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => std::pin::Pin::new(stream).poll_flush(context),
            Self::Tls(stream) => std::pin::Pin::new(stream.as_mut()).poll_flush(context),
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(stream) => std::pin::Pin::new(stream).poll_shutdown(context),
            Self::Tls(stream) => std::pin::Pin::new(stream.as_mut()).poll_shutdown(context),
        }
    }
}

/// Connection settings every adapter shares.
/// The addresses of an answer that a message may actually be sent to.
///
/// A filter rather than a verdict on the whole answer, and that is the decision worth pinning: a
/// name is free to answer with one address on the public internet and one on this machine, and both
/// other readings are wrong — refusing the whole answer would let any host make itself unreachable
/// to this application by adding a loopback record, and accepting it whole is the attack. The order
/// the lookup gave survives, because the order is what decides which address is tried first.
fn only_what_a_message_may_reach(found: Vec<SocketAddr>) -> Vec<SocketAddr> {
    found
        .into_iter()
        .filter(|address| reachable_by_a_message(address.ip()))
        .collect()
}

pub struct TransportPolicy {
    tls: Arc<rustls::ClientConfig>,
    /// The same trust, for every connection but mail: a picture, server discovery, the update
    /// check. Where the two differ is in what may be fetched to reach a verdict, and
    /// [`TransportPolicy::new`] says where they do.
    tls_for_a_named_host: Arc<rustls::ClientConfig>,
}

impl TransportPolicy {
    /// Builds the policy with the system trust store, so corporate certificate authorities work
    /// without the user importing anything.
    pub fn new() -> Result<Self, TransportError> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = rustls_platform_verifier::BuilderVerifierExt::with_platform_verifier(
            rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
                .with_safe_default_protocol_versions()
                .map_err(|_| TransportError::Tls)?,
        )
        .map_err(|_| TransportError::Tls)?;
        let tls = Arc::new(verifier.with_no_client_auth());

        // On macOS the platform verifier hands the chain to the system, and the system fetches an
        // issuer the chain leaves out from wherever the certificate says it is kept: a request the
        // address policy never sees, to an address whoever answered wrote. Every connection but
        // mail — a picture, server discovery, the update check — gets a verifier there that asks
        // the system the same question with fetching off.
        #[cfg(target_os = "macos")]
        let tls_for_a_named_host = Arc::new(
            rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
                .with_safe_default_protocol_versions()
                .map_err(|_| TransportError::Tls)?
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(judged_as_sent::JudgedAsSent::new(
                    provider,
                )))
                .with_no_client_auth(),
        );
        // Elsewhere one policy serves both. Linux's verifier reads the chain the host sent against
        // this machine's store and asks nobody. Windows's hands the chain to the system, which can
        // fetch what a certificate names as macOS's does, and serves both all the same: nothing
        // here builds or runs Windows.
        #[cfg(not(target_os = "macos"))]
        let tls_for_a_named_host = Arc::clone(&tls);

        Ok(Self {
            tls,
            tls_for_a_named_host,
        })
    }

    /// The trust a connection is held to: the platform's own for mail, and for everything else — a
    /// picture, server discovery, the update check — the one [`TransportPolicy::new`] built for
    /// it, which fetches nothing on macOS and is the platform's own elsewhere.
    fn trust(&self, mail: bool) -> Arc<rustls::ClientConfig> {
        match mail {
            true => Arc::clone(&self.tls),
            false => Arc::clone(&self.tls_for_a_named_host),
        }
    }

    /// The trust policy, for anything else in this application that speaks TLS.
    ///
    /// One policy rather than one per protocol: which certificate authorities this machine trusts
    /// is a property of the machine, and a second answer to that question is a second place to get
    /// it wrong.
    pub fn tls(&self) -> Arc<rustls::ClientConfig> {
        Arc::clone(&self.tls)
    }

    pub fn read_timeout(&self) -> Duration {
        READ_TIMEOUT
    }

    /// How long a server may stay silent on a connection opened after one that went quiet.
    pub fn patient_read_timeout(&self) -> Duration {
        PATIENT_READ_TIMEOUT
    }

    /// Opens a connection, trying again while the reason for failing is one that passes.
    ///
    /// What every caller should use. `connect` itself is the single attempt underneath.
    pub async fn connect_retrying(&self, endpoint: &Endpoint) -> Result<Stream, TransportError> {
        retrying(|| self.connect(endpoint)).await
    }

    /// Opens a connection for everything that is not mail: a picture a message points at, server
    /// discovery, the update check.
    ///
    /// A picture may be asked for in the clear, as its address says. Mail is refused in the clear
    /// because what travels on it is a password and somebody's own writing; a picture's request
    /// carries neither — no credential, no cookie, nothing about this machine — and its scheme was
    /// written by whoever sent the message. Refusing it would drop the picture rather than protect
    /// anybody. Server discovery and the update check ask at https addresses, and a redirect never
    /// leaves encryption.
    ///
    /// Everything else still holds, and that is the point of coming through here at all: the
    /// address policy, the deadline, and an attempt made towards every address the name answers
    /// with rather than towards whichever the resolver happened to put first.
    pub async fn connect_to_a_host_a_message_named(
        &self,
        endpoint: &Endpoint,
    ) -> Result<Stream, TransportError> {
        self.opening(endpoint, false).await
    }

    /// The same, for a test whose host is on this machine.
    ///
    /// Reaching this machine is the whole of what the policy above exists to prevent, so this
    /// exists nowhere but a test build.
    #[cfg(test)]
    pub(crate) async fn connect_anywhere_at_all(
        &self,
        endpoint: &Endpoint,
    ) -> Result<Stream, TransportError> {
        self.opening_and_judging(endpoint, false, false).await
    }

    /// Opens a connection, refusing to send anything in the clear off this machine.
    pub async fn connect(&self, endpoint: &Endpoint) -> Result<Stream, TransportError> {
        self.opening(endpoint, true).await
    }

    async fn opening(&self, endpoint: &Endpoint, mail: bool) -> Result<Stream, TransportError> {
        self.opening_and_judging(endpoint, mail, !mail).await
    }

    /// Where every connection this application makes is actually opened.
    ///
    /// `judging` is the address policy, and it is asked here rather than of a resolver handed to
    /// somebody else's client: a name is free to answer with whatever its owner likes, and the only
    /// place that knows every address a name gave is the place about to connect to one of them.
    async fn opening_and_judging(
        &self,
        endpoint: &Endpoint,
        mail: bool,
        judging: bool,
    ) -> Result<Stream, TransportError> {
        let addresses = resolve(&endpoint.host, endpoint.port).await?;

        // Everything but mail: a picture, server discovery, the update check. What a name resolved
        // to is its owner's choice, and `pictures.attacker.example` answering `127.0.0.1` is the
        // shape this refuses: a message reaching into the machine reading it, or into the network
        // around it.
        let addresses = match judging {
            true => only_what_a_message_may_reach(addresses),
            false => addresses,
        };
        if addresses.is_empty() {
            return Err(TransportError::Unreachable);
        }

        if mail && endpoint.encryption == Encryption::None && !is_this_machine(&addresses) {
            return Err(TransportError::PlaintextRefused);
        }
        if endpoint.encryption == Encryption::StartTls {
            return Err(TransportError::UnsupportedEncryption);
        }

        let stream = tokio::time::timeout(
            CONNECT_TIMEOUT,
            whichever_answers_first(&ordered_for_connecting(addresses)),
        )
        .await
        .map_err(|_| TransportError::Timeout)??;

        // Small writes are the norm in these protocols and waiting to batch them adds a round trip
        // of latency to every command.
        let _ = stream.set_nodelay(true);

        match endpoint.encryption {
            Encryption::None => Ok(Stream::Plain(stream)),
            Encryption::Tls => {
                let name = rustls::pki_types::ServerName::try_from(endpoint.host.clone())
                    .map_err(|_| TransportError::Tls)?;
                // Chosen by what the connection is for rather than by whether its address is
                // judged. The two agree on every connection this application makes, and a test
                // reaching this machine for a host a message names is then checked as that host
                // would be.
                let connector = TlsConnector::from(self.trust(mail));
                let encrypted =
                    tokio::time::timeout(CONNECT_TIMEOUT, connector.connect(name, stream))
                        .await
                        .map_err(|_| TransportError::Timeout)?
                        .map_err(|_| TransportError::Tls)?;
                Ok(Stream::Tls(Box::new(encrypted)))
            }
            Encryption::StartTls => Err(TransportError::UnsupportedEncryption),
        }
    }
}

/// Which family last carried a connection off this machine.
///
/// A machine can advertise addresses of a family it cannot reach — a network with no route for it,
/// a name server answering for both anyway — and every attempt to that family then waits out its
/// own budget. Remembering which one answered costs one byte and puts the next attempt in front.
///
/// 0 is "nothing has connected yet", and then the order the resolver gave is kept.
static FAMILY_THAT_ANSWERED: AtomicU8 = AtomicU8::new(0);

/// Puts it back to knowing nothing, so a test can watch it find out.
#[cfg(test)]
fn forget_the_family() {
    FAMILY_THAT_ANSWERED.store(0, Ordering::Relaxed);
}

/// Records the family an address that answered belongs to.
pub(crate) fn note_the_family_that_answered(address: &SocketAddr) {
    FAMILY_THAT_ANSWERED.store(
        match address.is_ipv6() {
            true => 6,
            false => 4,
        },
        Ordering::Relaxed,
    );
}

/// How long the one-off question "which family answers here" may take.
const TO_LEARN: Duration = Duration::from_secs(4);

/// Establishes once per run which family of addresses this machine can reach.
///
/// The ordering below prefers whatever answered last, and until something has answered it can only
/// keep the order the resolver gave. On a machine advertising a family with no route, that order is
/// wrong and the first connection to every host pays for it. Mail finds out on its own, because it
/// makes its attempts towards every address at once — but a picture may be fetched before any mail
/// has been, and then nothing is known.
///
/// So one address of each family is asked, once, and the first to answer decides. It costs one
/// connection at the start of a run and nothing after that.
pub(crate) async fn learn_which_family_answers(addresses: &[SocketAddr]) {
    if FAMILY_THAT_ANSWERED.load(Ordering::Relaxed) != 0 {
        return;
    }

    // One of each, on a port something listens on. A resolver hands its addresses back without a
    // port of their own, and a connection to port zero answers nothing on either family.
    let asking: Vec<SocketAddr> = [true, false]
        .iter()
        .filter_map(|want_six| {
            addresses
                .iter()
                .find(|address| address.is_ipv6() == *want_six)
                .map(|address| match address.port() {
                    0 => SocketAddr::new(address.ip(), 443),
                    port => SocketAddr::new(address.ip(), port),
                })
        })
        .collect();

    if asking.len() < 2 {
        // Only one family here, so there is nothing to find out and nothing to pay for it.
        if let Some(only) = asking.first() {
            note_the_family_that_answered(only);
        }
        return;
    }

    // The answer is recorded by the attempt itself. Nothing is done with the connection.
    let _ = tokio::time::timeout(TO_LEARN, whichever_answers_first(&asking)).await;
}

/// The order to try a host's addresses in.
///
/// Interleaved by family, so no family can hold the whole list: whatever is wrong with one, the
/// second address tried is of the other. The family that answered last goes first.
pub(crate) fn ordered_for_connecting(addresses: Vec<SocketAddr>) -> Vec<SocketAddr> {
    let remembered = match FAMILY_THAT_ANSWERED.load(Ordering::Relaxed) {
        6 => Some(true),
        4 => Some(false),
        _ => None,
    };
    interleaved(addresses, remembered)
}

/// The ordering itself, with the preference handed in rather than read from anywhere.
fn interleaved(addresses: Vec<SocketAddr>, prefer_v6: Option<bool>) -> Vec<SocketAddr> {
    let (sixes, fours): (Vec<SocketAddr>, Vec<SocketAddr>) =
        addresses.iter().partition(|address| address.is_ipv6());

    // With nothing remembered, the family the resolver put first stays first: it has already
    // applied the machine's own address selection rules, and second-guessing them is not this
    // function's job.
    let six_first = prefer_v6.unwrap_or_else(|| addresses.first().is_some_and(|a| a.is_ipv6()));

    let (first, second) = match six_first {
        true => (sixes, fours),
        false => (fours, sixes),
    };

    let mut ordered = Vec::with_capacity(addresses.len());
    let mut first = first.into_iter();
    let mut second = second.into_iter();
    loop {
        match (first.next(), second.next()) {
            (None, None) => break,
            (one, other) => ordered.extend(one.into_iter().chain(other)),
        }
    }
    ordered
}

/// Opens a connection to whichever address answers first.
///
/// Every address is tried, and no address waits for the one before it to fail: a new attempt
/// starts every `BEFORE_THE_NEXT` and the earlier ones keep running. The first to connect is the
/// one that is used and the rest are dropped where they stand.
async fn whichever_answers_first(addresses: &[SocketAddr]) -> Result<TcpStream, TransportError> {
    use futures::stream::{FuturesUnordered, StreamExt};

    if addresses.is_empty() {
        return Err(TransportError::Unreachable);
    }

    let mut attempts: FuturesUnordered<_> = addresses
        .iter()
        .enumerate()
        .map(|(place, address)| {
            let address = *address;
            async move {
                tokio::time::sleep(BEFORE_THE_NEXT * place as u32).await;
                match tokio::time::timeout(ONE_ADDRESS, TcpStream::connect(address)).await {
                    Ok(Ok(stream)) => Some((address, stream)),
                    _ => None,
                }
            }
        })
        .collect();

    let mut timed_out = false;
    while let Some(answered) = attempts.next().await {
        match answered {
            Some((address, stream)) => {
                note_the_family_that_answered(&address);
                kept_alive(&stream);
                return Ok(stream);
            }
            None => timed_out = true,
        }
    }

    match timed_out {
        true => Err(TransportError::Timeout),
        false => Err(TransportError::Unreachable),
    }
}

/// Has the system ask whether the other end of a quiet connection is still there, at
/// [`KEEP_ALIVE`] and every [`KEEP_ALIVE_EVERY`] after it. A system that refuses leaves the
/// connection as it was, which is what it was before this.
fn kept_alive(stream: &TcpStream) {
    let asking = socket2::TcpKeepalive::new()
        .with_time(KEEP_ALIVE)
        .with_interval(KEEP_ALIVE_EVERY);

    let _ = socket2::SockRef::from(stream).set_tcp_keepalive(&asking);
}

async fn resolve(host: &str, port: u16) -> Result<Vec<SocketAddr>, TransportError> {
    let addresses: Vec<SocketAddr> =
        tokio::time::timeout(CONNECT_TIMEOUT, tokio::net::lookup_host((host, port)))
            .await
            .map_err(|_| TransportError::Timeout)?
            .map_err(|error: io::Error| {
                let _ = error;
                TransportError::Unreachable
            })?
            .collect();

    if addresses.is_empty() {
        return Err(TransportError::Unreachable);
    }
    Ok(addresses)
}

/// Whether an address is one a message is allowed to reach.
///
/// A message is written by a stranger, and a picture it points at is fetched by this application on
/// the reader's word. What the reader agreed to is "load the pictures in this newsletter" — not
/// "make requests to the machine this runs on, or to the network it sits in". Everything that is
/// not the public internet is refused: the loopback, the private ranges, the link-local range that
/// carries cloud metadata services, and the rest of what a host on the internet cannot be.
///
/// Checked against the address a name resolved to rather than against the name, because a name is
/// free to resolve wherever its owner likes, and a redirect never passes through a check written
/// against the first URL.
pub fn reachable_by_a_message(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(address) => reachable_v4(address),
        IpAddr::V6(address) => match address.to_ipv4_mapped() {
            // An address written the long way is the same address.
            Some(mapped) => reachable_v4(mapped),
            None => reachable_v6(address),
        },
    }
}

fn reachable_v4(address: std::net::Ipv4Addr) -> bool {
    let [first, second, ..] = address.octets();

    !(address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_unspecified()
        || address.is_multicast()
        || address.is_broadcast()
        || address.is_documentation()
        // Carrier-grade NAT, 100.64.0.0/10: another network's inside.
        || (first == 100 && (64..128).contains(&second))
        // Benchmarking, 198.18.0.0/15.
        || (first == 198 && (18..20).contains(&second))
        // Reserved, 240.0.0.0/4.
        || first >= 240)
}

fn reachable_v6(address: std::net::Ipv6Addr) -> bool {
    let segments = address.segments();
    let first = segments[0];
    let octets = address.octets();

    // An IPv4 address carried inside an IPv6 one, where the form says where it is, is that IPv4
    // address: something on the way reaches it on this machine's behalf, and reaches a private one
    // as readily as a public one. NAT64's well-known prefix, 64:ff9b::/96, carries it in the last
    // thirty-two bits, for the network's translator.
    if segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
        let [.., a, b, c, d] = octets;
        return reachable_v4(std::net::Ipv4Addr::new(a, b, c, d));
    }
    // 6to4, 2002::/16, carries the address of a site's own router in the thirty-two bits after
    // the prefix.
    if first == 0x2002 {
        return reachable_v4(std::net::Ipv4Addr::new(
            octets[2], octets[3], octets[4], octets[5],
        ));
    }

    !(address.is_loopback()
        || address.is_unspecified()
        || address.is_multicast()
        // The rest of ::/8, which is reserved and holds no host on the internet: an IPv4 address
        // written the withdrawn way, ::/96, or the way a stateless translator writes it; and
        // NAT64's local-use prefix, 64:ff9b:1::/48, whose IPv4 address sits wherever the local
        // network chose to cut the prefix, so nothing here can read which address it is.
        || (first & 0xff00) == 0
        // Unique local, fc00::/7 — the private ranges of IPv6.
        || (first & 0xfe00) == 0xfc00
        // Link local, fe80::/10.
        || (first & 0xffc0) == 0xfe80
        // Site local, fec0::/10: withdrawn, and private wherever it is still in use.
        || (first & 0xffc0) == 0xfec0)
}

/// Whether every address the name resolves to is on this machine.
///
/// Every address, not any: a name that resolves to both a loopback address and a public one would
/// otherwise be enough to have a password sent in the clear.
pub fn is_this_machine(addresses: &[SocketAddr]) -> bool {
    !addresses.is_empty() && addresses.iter().all(|address| address.ip().is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(value: &str) -> IpAddr {
        value.parse().expect("test addresses are well formed")
    }

    #[test]
    fn a_message_may_reach_the_public_internet() {
        for address in ["93.184.216.34", "8.8.8.8", "1.1.1.1", "2606:2800:220:1::1"] {
            assert!(reachable_by_a_message(ip(address)), "{address} was refused");
        }
    }

    #[test]
    fn a_message_may_not_reach_this_machine_or_the_network_it_sits_in() {
        // What the reader agreed to is "load the pictures in this newsletter", not "make requests
        // to the machine this runs on". Each of these is somewhere a message has no business.
        for address in [
            "127.0.0.1",        // the machine itself
            "127.5.5.5",        // the whole loopback range, not just the famous address
            "0.0.0.0",          // this host
            "10.1.2.3",         // private
            "172.16.9.9",       // private
            "192.168.1.1",      // private — the router
            "169.254.169.254",  // link local — where cloud metadata lives
            "100.64.0.1",       // carrier-grade NAT: another network's inside
            "198.18.0.1",       // benchmarking
            "255.255.255.255",  // broadcast
            "224.0.0.1",        // multicast
            "240.0.0.1",        // reserved
            "::1",              // the machine itself, the long way
            "fd00::1",          // unique local
            "fe80::1",          // link local
            "::ffff:127.0.0.1", // the loopback written as IPv6
            "::ffff:192.168.1.1",
        ] {
            assert!(
                !reachable_by_a_message(ip(address)),
                "{address} was allowed"
            );
        }
    }

    #[test]
    fn an_ipv4_address_carried_inside_an_ipv6_one_is_judged_as_what_it_carries() {
        // NAT64 and 6to4 write an IPv4 address into an IPv6 one for something on the way to
        // reach, and it reaches a private address as readily as a public one. Each form is the
        // address it carries, for better and for worse: refused with it, and allowed with it.
        for carried in [
            "127.0.0.1",
            "10.1.2.3",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "93.184.216.34",
        ] {
            let [a, b, c, d] = carried
                .parse::<std::net::Ipv4Addr>()
                .expect("test addresses are well formed")
                .octets();
            let (high, low) = (u16::from_be_bytes([a, b]), u16::from_be_bytes([c, d]));
            let verdict = reachable_by_a_message(ip(carried));

            for carrying in [
                std::net::Ipv6Addr::new(0x64, 0xff9b, 0, 0, 0, 0, high, low),
                std::net::Ipv6Addr::new(0x2002, high, low, 0, 0, 0, 0, 1),
            ] {
                assert_eq!(
                    reachable_by_a_message(IpAddr::V6(carrying)),
                    verdict,
                    "{carrying} was not judged as the {carried} it carries"
                );
            }
        }
    }

    #[test]
    fn a_form_of_ipv6_that_holds_no_host_on_the_internet_is_refused() {
        for address in [
            // An IPv4 address written the withdrawn way, whichever address it is.
            "::127.0.0.1",
            "::93.184.216.34",
            // As a stateless translator writes it.
            "::ffff:0:127.0.0.1",
            // NAT64's local-use prefix, where the IPv4 address cannot be read off.
            "64:ff9b:1::127.0.0.1",
            "64:ff9b:1:7f00:1::",
            // Site local, both ends of it.
            "fec0::1",
            "feff:ffff::1",
        ] {
            assert!(
                !reachable_by_a_message(ip(address)),
                "{address} was allowed"
            );
        }
    }

    fn address(value: &str) -> SocketAddr {
        value.parse().expect("test addresses are well formed")
    }

    #[test]
    fn loopback_addresses_are_this_machine() {
        assert!(is_this_machine(&[address("127.0.0.1:1143")]));
        assert!(is_this_machine(&[address("[::1]:1143")]));
        assert!(is_this_machine(&[
            address("127.0.0.1:1143"),
            address("[::1]:1143")
        ]));
    }

    #[test]
    fn a_public_address_is_not_this_machine() {
        assert!(!is_this_machine(&[address("93.184.216.34:143")]));
        assert!(!is_this_machine(&[address("192.168.1.10:143")]));
    }

    #[test]
    fn one_public_address_is_enough_to_refuse() {
        // A name resolving to both would otherwise be a way to have the password sent in the clear.
        assert!(!is_this_machine(&[
            address("127.0.0.1:143"),
            address("93.184.216.34:143")
        ]));
    }

    #[test]
    fn what_is_worth_trying_again_is_what_passes_on_its_own() {
        assert!(TransportError::Unreachable.is_temporary());
        assert!(TransportError::Timeout.is_temporary());

        // A certificate does not verify on the second ask, and an unencrypted connection off this
        // machine is a refusal. Retrying either would make a downgrade look like a hiccup.
        assert!(!TransportError::Tls.is_temporary());
        assert!(!TransportError::PlaintextRefused.is_temporary());
        assert!(!TransportError::UnsupportedEncryption.is_temporary());
    }

    #[test]
    fn a_wait_is_never_longer_than_its_ceiling_and_the_ceiling_doubles() {
        for entropy in [0, 1, 7, u64::MAX / 3, u64::MAX] {
            assert!(backoff(0, entropy) <= FIRST_BACKOFF);
            assert!(backoff(1, entropy) <= FIRST_BACKOFF * 2);
            assert!(backoff(2, entropy) <= FIRST_BACKOFF * 4);
        }

        // The ceiling is reached: an entropy at the top of the span waits the whole of it.
        let ceiling = FIRST_BACKOFF * 2;
        assert_eq!(backoff(1, ceiling.as_nanos() as u64), ceiling);
    }

    #[test]
    fn a_wait_stops_growing_at_the_maximum() {
        let entropy = u64::MAX;
        for attempt in [8, 20, 1_000, u32::MAX] {
            assert!(
                backoff(attempt, entropy) <= MAX_BACKOFF,
                "attempt {attempt} waited past the maximum"
            );
        }
    }

    #[test]
    fn two_failures_in_the_same_instant_do_not_wait_the_same_amount() {
        // The whole reason for the jitter: a hundred accounts fail together when a network drops,
        // and a fixed backoff would send all of them back at the server in one wave.
        let waits: std::collections::HashSet<Duration> =
            (0..64).map(|_| backoff(2, entropy())).collect();

        assert!(
            waits.len() > 32,
            "only {} distinct waits out of 64, which is not jitter",
            waits.len()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_attempt_that_works_is_made_once() {
        let attempts = std::cell::Cell::new(0);

        let stream = retrying(|| {
            attempts.set(attempts.get() + 1);
            std::future::ready(Ok::<_, TransportError>("connected"))
        })
        .await;

        assert_eq!(stream, Ok("connected"));
        assert_eq!(attempts.get(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_network_that_comes_back_is_waited_for() {
        let attempts = std::cell::Cell::new(0);

        let stream = retrying(|| {
            attempts.set(attempts.get() + 1);
            std::future::ready(if attempts.get() < 3 {
                Err(TransportError::Unreachable)
            } else {
                Ok("connected")
            })
        })
        .await;

        assert_eq!(stream, Ok("connected"));
        assert_eq!(attempts.get(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn a_network_that_does_not_come_back_gives_up_and_says_why() {
        let attempts = std::cell::Cell::new(0);

        let stream = retrying(|| {
            attempts.set(attempts.get() + 1);
            std::future::ready(Err::<&str, _>(TransportError::Timeout))
        })
        .await;

        assert_eq!(stream, Err(TransportError::Timeout));
        assert_eq!(
            attempts.get(),
            ATTEMPTS as i32,
            "an account that cannot be reached must not hold the round open"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_refusal_is_not_tried_again() {
        for refusal in [
            TransportError::Tls,
            TransportError::PlaintextRefused,
            TransportError::UnsupportedEncryption,
        ] {
            let attempts = std::cell::Cell::new(0);

            let stream = retrying(|| {
                attempts.set(attempts.get() + 1);
                std::future::ready(Err::<&str, _>(refusal))
            })
            .await;

            assert_eq!(stream, Err(refusal));
            assert_eq!(attempts.get(), 1, "{refusal} was asked again");
        }
    }

    fn v4(last: u8) -> SocketAddr {
        SocketAddr::from(([192, 0, 2, last], 993))
    }

    fn v6(last: u16) -> SocketAddr {
        SocketAddr::from((
            std::net::Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, last),
            993,
        ))
    }

    #[test]
    fn no_family_holds_the_whole_list() {
        // What this exists to prevent: four addresses of a family this machine cannot reach, tried
        // one after another, and the budget gone before the one that answers is reached.
        let ordered = interleaved(vec![v6(1), v6(2), v6(3), v4(1)], None);

        assert_eq!(
            ordered,
            vec![v6(1), v4(1), v6(2), v6(3)],
            "the second address tried must be of the other family"
        );
    }

    #[test]
    fn the_family_that_answered_last_goes_first() {
        let addresses = vec![v6(1), v6(2), v4(1), v4(2)];

        assert_eq!(
            interleaved(addresses.clone(), Some(false)),
            vec![v4(1), v6(1), v4(2), v6(2)]
        );
        assert_eq!(
            interleaved(addresses, Some(true)),
            vec![v6(1), v4(1), v6(2), v4(2)]
        );
    }

    #[test]
    fn with_nothing_remembered_the_resolver_still_leads() {
        // The machine's own address selection has already run by the time these arrive, and
        // second-guessing it is not this function's job. Only the interleaving is.
        assert_eq!(
            interleaved(vec![v4(1), v6(1), v4(2)], None),
            vec![v4(1), v6(1), v4(2)]
        );
    }

    #[test]
    fn one_family_alone_is_left_as_it_is() {
        assert_eq!(
            interleaved(vec![v4(1), v4(2)], Some(true)),
            vec![v4(1), v4(2)]
        );
        assert!(interleaved(Vec::new(), None).is_empty());
    }

    #[tokio::test]
    async fn an_address_that_never_answers_does_not_hold_up_the_one_that_does() {
        let listening = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let answers = listening.local_addr().expect("its address");

        // 192.0.2.0/24 is reserved for documentation and is routed nowhere: a connection to it
        // hangs rather than being refused, which is exactly the case that used to cost the whole
        // budget.
        let began = std::time::Instant::now();
        let stream = whichever_answers_first(&[v4(1), v4(2), v4(3), answers])
            .await
            .expect("the one that is listening");

        assert_eq!(stream.peer_addr().expect("a peer"), answers);
        assert!(
            began.elapsed() < Duration::from_secs(3),
            "waited {:?}: the attempts are being made one after another again",
            began.elapsed()
        );
    }

    /// A connection that is used is one the system asks after while it is quiet, so a watch on a
    /// connection dropped on the way is found to be gone rather than waited on for half an hour.
    #[tokio::test]
    async fn a_connection_is_asked_after_while_it_is_quiet() {
        let listening = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let answers = listening.local_addr().expect("its address");

        let stream = whichever_answers_first(&[answers])
            .await
            .expect("the one that is listening");

        assert!(
            socket2::SockRef::from(&stream)
                .keepalive()
                .expect("the setting reads"),
            "a quiet connection is never asked after"
        );
    }

    /// The two tests below share one piece of state, so they take turns. Tokio's rather than the
    /// standard one: this is held across an await, and holding the standard one there blocks a
    /// thread that is meant to be running other work.
    static TAKING_TURNS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[tokio::test]
    async fn it_finds_out_which_family_answers_before_anything_needs_to_know() {
        let _turn = TAKING_TURNS.lock().await;
        forget_the_family();

        let listening = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let answers = listening.local_addr().expect("its address");
        let nowhere = SocketAddr::new(
            std::net::IpAddr::V6(std::net::Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
            answers.port(),
        );

        // The order it is handed is the wrong one, which is the case this exists for.
        learn_which_family_answers(&[nowhere, answers]).await;

        let ordered = ordered_for_connecting(vec![nowhere, answers]);
        assert_eq!(
            ordered.first(),
            Some(&answers),
            "the family that answered is not being tried first"
        );
    }

    #[tokio::test]
    async fn asking_again_costs_nothing() {
        let _turn = TAKING_TURNS.lock().await;
        forget_the_family();
        note_the_family_that_answered(&SocketAddr::from(([127, 0, 0, 1], 993)));

        // Nothing is reachable in this list at all. If the question were asked again it would wait
        // for the timeout; it must not, because it already has an answer.
        let began = std::time::Instant::now();
        learn_which_family_answers(&[v4(1), v6(1)]).await;

        assert!(
            began.elapsed() < Duration::from_millis(100),
            "waited {:?}: it is asking again every time",
            began.elapsed()
        );
    }

    #[tokio::test]
    async fn nowhere_to_connect_is_not_a_hang() {
        let began = std::time::Instant::now();
        let outcome = whichever_answers_first(&[]).await;

        assert!(matches!(outcome, Err(TransportError::Unreachable)));
        assert!(began.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn nothing_resolved_is_not_this_machine() {
        assert!(!is_this_machine(&[]));
    }

    #[tokio::test]
    async fn plaintext_off_this_machine_is_refused_before_connecting() {
        let policy = TransportPolicy::new().expect("policy builds");
        // An address rather than a name: the refusal has nothing to do with DNS, and a test that
        // needs a working resolver fails on a machine with no network for the wrong reason.
        let refused = policy
            .connect(&Endpoint {
                host: "93.184.216.34".to_owned(),
                port: 143,
                encryption: Encryption::None,
            })
            .await
            .expect_err("an unencrypted connection off this machine is refused");

        assert_eq!(refused, TransportError::PlaintextRefused);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_host_a_message_named_is_judged_by_the_verifier_that_fetches_nothing() {
        // Which configuration each kind of connection is given, read off the configuration by
        // name. That a connection is actually handed it is held beside the verifier, by a test
        // that connects through the policy.
        let policy = TransportPolicy::new().expect("policy builds");

        assert!(
            format!("{:?}", policy.trust(false)).contains("JudgedAsSent"),
            "a host a message named is judged by a verifier that may fetch what its certificate names"
        );
        assert!(
            !format!("{:?}", policy.trust(true)).contains("JudgedAsSent"),
            "mail lost the platform's verifier"
        );
    }

    #[tokio::test]
    async fn a_name_that_resolves_to_nothing_is_unreachable() {
        let policy = TransportPolicy::new().expect("policy builds");
        let error = policy
            .connect(&Endpoint {
                host: "this-name-does-not-resolve.invalid".to_owned(),
                port: 993,
                encryption: Encryption::Tls,
            })
            .await
            .expect_err("an unknown name cannot be connected to");

        assert_eq!(error, TransportError::Unreachable);
    }
}

#[cfg(test)]
mod a_server_that_goes_quiet {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Far longer than the deadline under test, so what ends the wait is the deadline and not the
    /// end of the script.
    const FOR_EVER: Duration = Duration::from_secs(600);

    /// Short enough that the test does not sit through it, since time is paused.
    const ALLOWED: Duration = Duration::from_millis(50);

    #[tokio::test(start_paused = true)]
    async fn a_read_that_never_answers_ends_rather_than_waiting_for_ever() {
        let quiet = tokio_test::io::Builder::new().wait(FOR_EVER).build();
        let mut stream = Deadline::over(quiet, Silence::of(ALLOWED));

        let mut into = [0u8; 8];
        let answer = stream.read(&mut into).await;

        assert_eq!(
            answer.expect_err("the read went on for ever").kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_server_still_talking_is_waited_for() {
        // The count is of silence, not of how long the work takes: every byte starts it again, so
        // a message that arrives slowly but steadily is never cut off.
        let talking = tokio_test::io::Builder::new()
            .read(b"one ")
            .wait(Duration::from_millis(30))
            .read(b"two ")
            .wait(Duration::from_millis(30))
            .read(b"three")
            .build();
        let mut stream = Deadline::over(talking, Silence::of(ALLOWED));

        let mut said = String::new();
        stream
            .read_to_string(&mut said)
            .await
            .expect("a server that keeps talking was cut off");

        assert_eq!(said, "one two three");
    }

    #[tokio::test(start_paused = true)]
    async fn a_silence_allowed_for_a_while_is_allowed_and_then_is_not() {
        // An idle is a stretch in which saying nothing is the answer, and it is allowed far longer
        // than any other. What it is allowed goes back when it ends, and a wait that began under
        // the longer allowance is held to the ordinary one from that moment.
        let silence = Silence::of(ALLOWED);
        let quiet = tokio_test::io::Builder::new().wait(FOR_EVER).build();
        let mut stream = Deadline::over(quiet, silence.clone());
        let mut into = [0u8; 8];

        let began = tokio::time::Instant::now();
        {
            let _idle = silence.for_a_while(ALLOWED * 100);
            let waited = tokio::time::timeout(ALLOWED * 10, stream.read(&mut into)).await;
            assert!(
                waited.is_err(),
                "a silence allowed for a while ended the read early"
            );
        }
        assert_eq!(silence.allowed(), ALLOWED, "the allowance did not go back");

        let answer = stream.read(&mut into).await;
        assert_eq!(
            answer.expect_err("the read went on for ever").kind(),
            io::ErrorKind::TimedOut
        );
        assert!(
            began.elapsed() <= ALLOWED * 11,
            "the wait after the idle was held to the idle's allowance: {:?}",
            began.elapsed()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_write_nobody_takes_ends_too() {
        // A peer that has stopped reading is as bad as one that has stopped writing, and on TLS it
        // is the more likely of the two.
        let deaf = tokio_test::io::Builder::new().wait(FOR_EVER).build();
        let mut stream = Deadline::over(deaf, Silence::of(ALLOWED));

        let answer = stream.write(b"anybody there?").await;

        assert_eq!(
            answer.expect_err("the write went on for ever").kind(),
            io::ErrorKind::TimedOut
        );
    }

    #[test]
    fn an_answer_holding_both_keeps_the_half_a_message_may_reach() {
        // The dangerous answer, and the one a lookup on this machine cannot be made to give: a
        // name answering with one address on the public internet and one on this machine. Both
        // other readings are wrong — refusing the whole answer lets any host make itself
        // unreachable by adding a loopback record, and taking it whole is the attack.
        // The documentation prefix, which is what a network address in this repository is
        // written from. It has to be the sixth version of the protocol: every documentation
        // range of the fourth is refused by this very rule, so there is no address there that
        // could stand for one a message may reach.
        let public: std::net::SocketAddr = "[2001:db8::1]:0".parse().expect("an address");
        let also_public: std::net::SocketAddr = "[2001:db8::2]:0".parse().expect("an address");
        let loopback: std::net::SocketAddr = "127.0.0.1:0".parse().expect("an address");
        let private: std::net::SocketAddr = "10.0.0.5:0".parse().expect("an address");

        assert_eq!(
            only_what_a_message_may_reach(vec![public, loopback, private]),
            vec![public]
        );
        assert!(
            only_what_a_message_may_reach(vec![loopback, private]).is_empty(),
            "an answer with nothing reachable in it is not an answer"
        );

        // The order the lookup gave survives the filter: it is what decides which address is
        // tried first, and on a machine with no route to one family that is the whole request.
        assert_eq!(
            only_what_a_message_may_reach(vec![also_public, public]),
            vec![also_public, public]
        );
    }
}
