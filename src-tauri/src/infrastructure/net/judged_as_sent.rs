//! The certificate check, on macOS, for every connection but mail: a picture, server discovery, the
//! update check.
//!
//! The same evaluation mail gets from `rustls-platform-verifier` — the system's own trust store and
//! its SSL policy, held to the name that was dialled, at the moment rustls gives, with the
//! revocation answer the host stapled — and one setting more: nothing is fetched on the way to the
//! verdict. Left to itself, the system fetches an issuer a chain leaves out from the address the
//! certificate names for it. That is a request this application did not make, to an address the
//! address policy never sees, written into a certificate by whoever answered — and for a host a
//! message names, whoever answered is whoever sent the message.
//!
//! A chain sent without its intermediates is therefore refused here, as Linux refuses it
//! everywhere: the verifier there reads what the host sent and what this machine holds, and asks
//! nobody.

use std::sync::Arc;

use core_foundation::date::CFDate;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, Error, OtherError, SignatureScheme};
use security_framework::certificate::SecCertificate;
use security_framework::policy::SecPolicy;
use security_framework::secure_transport::SslProtocolSide;
use security_framework::trust::SecTrust;

/// How far the system's clock starts after the Unix one, in seconds: Core Foundation counts time
/// from an epoch of its own.
const SYSTEM_EPOCH_AFTER_UNIX: u64 = 978_307_200;

/// The refusals worth telling apart, as the system numbers them.
///
/// `errSecHostNameMismatch`: the certificate is for some other name.
const NOT_FOR_THIS_NAME: isize = -67_602;
/// `errSecCreateChainFailed`: no chain reaches anything this machine trusts.
const NO_CHAIN: isize = -25_318;
/// `errSecInvalidExtendedKeyUsage`: the certificate is not one for a server.
const NOT_FOR_A_SERVER: isize = -67_609;
/// `errSecCertificateRevoked`.
const REVOKED: isize = -67_820;

/// A verifier that judges a chain as the host sent it, against what this machine trusts.
#[derive(Debug)]
pub(super) struct JudgedAsSent {
    /// Where the handshake's own signatures are checked: the provider every connection uses.
    crypto: Arc<CryptoProvider>,
}

impl JudgedAsSent {
    pub(super) fn new(crypto: Arc<CryptoProvider>) -> Self {
        Self { crypto }
    }
}

/// The evaluation of one chain, every setting made and nothing evaluated yet.
///
/// Its own function so the settings can be read back before anything is asked of the system: the
/// one setting this file exists for is one nothing else would show the absence of.
fn evaluation(
    end_entity: &CertificateDer<'_>,
    intermediates: &[CertificateDer<'_>],
    server_name: &str,
    ocsp_response: &[u8],
    now: UnixTime,
) -> Result<SecTrust, Error> {
    // The certificate the host is, first — the system requires it there — and whatever the host
    // sent to vouch for it after.
    let certificates = std::iter::once(end_entity)
        .chain(intermediates)
        .map(|certificate| {
            SecCertificate::from_der(certificate)
                .map_err(|_| Error::InvalidCertificate(CertificateError::BadEncoding))
        })
        .collect::<Result<Vec<_>, _>>()?;

    // A server's certificate, for the name that was dialled.
    let policy = SecPolicy::create_ssl(SslProtocolSide::SERVER, Some(server_name));
    let mut trust = SecTrust::create_with_certificates(&certificates, &[policy])
        .map_err(|_| Error::General("the system would not take the chain".to_owned()))?;

    // What this file is for. Refused rather than evaluated without it: a verdict reached with the
    // fetching left on is the request this exists to stop.
    trust
        .set_network_fetch_allowed(false)
        .map_err(|_| Error::General("the system would not stop fetching".to_owned()))?;

    // At the moment rustls gives, rather than whenever the system reads its clock.
    trust
        .set_trust_verify_date(&at(now)?)
        .map_err(|_| Error::FailedToGetCurrentTime)?;

    // The revocation answer the host stapled, when it stapled one. It came in the handshake, so
    // using it asks nobody.
    if !ocsp_response.is_empty() {
        trust
            .set_trust_ocsp_response(std::iter::once(ocsp_response))
            .map_err(|_| Error::InvalidCertificate(CertificateError::BadEncoding))?;
    }

    Ok(trust)
}

/// `now` as the system counts time.
fn at(now: UnixTime) -> Result<CFDate, Error> {
    now.as_secs()
        .checked_sub(SYSTEM_EPOCH_AFTER_UNIX)
        .map(|since| CFDate::new(since as f64))
        .ok_or(Error::FailedToGetCurrentTime)
}

/// What the system's refusal means, in the words rustls has for it.
fn why(code: isize) -> CertificateError {
    match code {
        NOT_FOR_THIS_NAME => CertificateError::NotValidForName,
        NO_CHAIN => CertificateError::UnknownIssuer,
        NOT_FOR_A_SERVER => CertificateError::InvalidPurpose,
        REVOKED => CertificateError::Revoked,
        other => CertificateError::Other(OtherError(Arc::new(Refused(other)))),
    }
}

/// A refusal the system gave a number this file has no name for.
#[derive(Debug, thiserror::Error)]
#[error("the system refused the chain with code {0}")]
struct Refused(isize);

impl ServerCertVerifier for JudgedAsSent {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        // An address dialled as a number is held to the addresses the certificate names, the way a
        // name is held to its names.
        let trust = evaluation(
            end_entity,
            intermediates,
            &server_name.to_str(),
            ocsp_response,
            now,
        )?;

        match trust.evaluate_with_error() {
            Ok(()) => Ok(ServerCertVerified::assertion()),
            Err(refusal) => Err(Error::InvalidCertificate(why(refusal.code()))),
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls12_signature(
            message,
            certificate,
            signed,
            &self.crypto.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signed: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        verify_tls13_signature(
            message,
            certificate,
            signed,
            &self.crypto.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.crypto
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::{Read, Write};

    /// One DER element: its tag, the length of what follows, and what follows.
    fn element(tag: u8, content: &[u8]) -> Vec<u8> {
        let mut written = vec![tag];
        match u8::try_from(content.len()) {
            Ok(short) if short < 0x80 => written.push(short),
            _ => {
                let length: Vec<u8> = content
                    .len()
                    .to_be_bytes()
                    .into_iter()
                    .skip_while(|byte| *byte == 0)
                    .collect();
                written.push(0x80 | length.len() as u8);
                written.extend(length);
            }
        }
        written.extend_from_slice(content);
        written
    }

    fn sequence(parts: &[Vec<u8>]) -> Vec<u8> {
        element(0x30, &parts.concat())
    }

    fn identifier(arcs: &[u8]) -> Vec<u8> {
        element(0x06, arcs)
    }

    /// A distinguished name of one common name.
    fn named(name: &str) -> Vec<u8> {
        sequence(&[element(
            0x31,
            &sequence(&[
                identifier(&[0x55, 0x04, 0x03]),
                element(0x0c, name.as_bytes()),
            ]),
        )])
    }

    /// A certificate for `name`, signed by an issuer nobody holds, naming `address` as the place
    /// that issuer's own certificate is kept — and the key it was made for, so a host can present
    /// it.
    ///
    /// Well formed and properly signed, so the only thing wrong with it is the thing under test:
    /// the chain stops short, and the certificate says where the rest of it is.
    fn a_certificate_whose_issuer_is_kept_at(name: &str, address: &str) -> (Vec<u8>, Vec<u8>) {
        use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};

        let random = ring::rand::SystemRandom::new();
        let pkcs8 =
            EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &random).expect("a key");
        let key =
            EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &random)
                .expect("the key just made");

        // ecdsa-with-SHA256.
        let signed_with = sequence(&[identifier(&[
            0x2a, 0x86, 0x48, 0xce, 0x3d, 0x04, 0x03, 0x02,
        ])]);
        // id-ecPublicKey on prime256v1, and the point.
        let public = sequence(&[
            sequence(&[
                identifier(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01]),
                identifier(&[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07]),
            ]),
            element(0x03, &[&[0u8][..], key.public_key().as_ref()].concat()),
        ]);
        // Authority information access: the issuer's certificate is kept at this address.
        let issuer_kept_at = sequence(&[
            identifier(&[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x01, 0x01]),
            element(
                0x04,
                &sequence(&[sequence(&[
                    identifier(&[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x30, 0x02]),
                    element(0x86, address.as_bytes()),
                ])]),
            ),
        ]);
        // The name it is for.
        let for_the_name = sequence(&[
            identifier(&[0x55, 0x1d, 0x11]),
            element(0x04, &sequence(&[element(0x82, name.as_bytes())])),
        ]);

        let to_be_signed = sequence(&[
            element(0xa0, &element(0x02, &[2])),
            element(0x02, &[1]),
            signed_with.clone(),
            named("an issuer nobody holds"),
            sequence(&[
                element(0x17, b"200101000000Z"),
                element(0x17, b"491231235959Z"),
            ]),
            named(name),
            public,
            element(0xa3, &sequence(&[issuer_kept_at, for_the_name])),
        ]);
        let signature = key.sign(&random, &to_be_signed).expect("a signature");

        let certificate = sequence(&[
            to_be_signed,
            signed_with,
            element(0x03, &[&[0u8][..], signature.as_ref()].concat()),
        ]);

        (certificate, pkcs8.as_ref().to_vec())
    }

    /// A host on this machine that answers every request with nothing and says it was asked.
    fn a_host_that_keeps_nothing() -> (String, std::sync::mpsc::Receiver<String>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let (asked, heard) = std::sync::mpsc::channel();

        std::thread::spawn(move || {
            for arriving in listener.incoming() {
                let Ok(mut stream) = arriving else {
                    return;
                };
                let mut request = [0u8; 2048];
                let read = stream.read(&mut request).unwrap_or(0);
                let _ = asked.send(String::from_utf8_lossy(&request[..read]).into_owned());
                let _ = stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });

        (format!("http://127.0.0.1:{port}/issuer.cer"), heard)
    }

    /// A host on this machine that answers TLS with `certificate` and says nothing after it.
    async fn a_host_that_presents(certificate: Vec<u8>, key: Vec<u8>) -> u16 {
        let presenting = rustls::ServerConfig::builder_with_provider(the_ring())
            .with_safe_default_protocol_versions()
            .expect("the protocol versions")
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(certificate)],
                rustls::pki_types::PrivateKeyDer::Pkcs8(key.into()),
            )
            .expect("the certificate and its key");
        let accepting = tokio_rustls::TlsAcceptor::from(Arc::new(presenting));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let port = listener.local_addr().expect("an address").port();

        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let _ = accepting.accept(stream).await;
            }
        });

        port
    }

    fn the_ring() -> Arc<CryptoProvider> {
        Arc::new(rustls::crypto::ring::default_provider())
    }

    #[test]
    fn a_certificate_saying_where_its_issuer_is_kept_sends_nobody_there() {
        // The request this file exists to stop: a chain that stops short, from a certificate that
        // names an address for the rest of it. Left to itself the system asks that address — one
        // the address policy never saw, written by whoever answered — and here it is a port on
        // this very machine.
        let (kept_at, asked) = a_host_that_keeps_nothing();
        let (certificate, _) = a_certificate_whose_issuer_is_kept_at("pictures.example", &kept_at);
        let certificate = CertificateDer::from(certificate);

        let verdict = JudgedAsSent::new(the_ring()).verify_server_cert(
            &certificate,
            &[],
            &ServerName::try_from("pictures.example").expect("a name"),
            &[],
            UnixTime::now(),
        );

        assert!(
            verdict.is_err(),
            "a chain that reaches nothing this machine trusts was accepted"
        );
        assert!(
            asked
                .recv_timeout(std::time::Duration::from_millis(500))
                .is_err(),
            "the system fetched the address the certificate named"
        );
    }

    #[tokio::test]
    async fn a_connection_to_a_host_a_message_named_sends_nobody_where_its_certificate_points() {
        // The same certificate, met the way a picture meets it: over a connection the policy
        // opens. What is under test is which check that connection is handed, since the
        // platform's own asks the address the certificate names. The policy reaches this machine
        // here only because a test is the one place it may.
        use crate::infrastructure::net::{Encryption, Endpoint, TransportError, TransportPolicy};

        let (kept_at, asked) = a_host_that_keeps_nothing();
        let (certificate, key) = a_certificate_whose_issuer_is_kept_at("localhost", &kept_at);
        let port = a_host_that_presents(certificate, key).await;

        let outcome = TransportPolicy::new()
            .expect("policy builds")
            .connect_anywhere_at_all(&Endpoint {
                host: "localhost".to_owned(),
                port,
                encryption: Encryption::Tls,
            })
            .await;

        assert_eq!(
            outcome.err(),
            Some(TransportError::Tls),
            "a chain that reaches nothing this machine trusts was accepted"
        );
        assert!(
            asked
                .recv_timeout(std::time::Duration::from_millis(500))
                .is_err(),
            "the connection fetched the address its certificate named"
        );
    }

    #[test]
    fn the_evaluation_is_made_to_fetch_nothing_before_it_runs() {
        // The setting read back, so a run on a machine whose system would not have fetched the
        // address above anyway still says whether it was made.
        let (certificate, _) = a_certificate_whose_issuer_is_kept_at(
            "pictures.example",
            "http://127.0.0.1:9/issuer.cer",
        );
        let certificate = CertificateDer::from(certificate);

        let mut judged = evaluation(&certificate, &[], "pictures.example", &[], UnixTime::now())
            .expect("an evaluation");

        assert_eq!(
            judged.get_network_fetch_allowed().ok(),
            Some(false),
            "the evaluation is left free to fetch what a certificate names"
        );
    }

    #[test]
    fn a_refusal_keeps_its_meaning() {
        assert_eq!(why(NOT_FOR_THIS_NAME), CertificateError::NotValidForName);
        assert_eq!(why(NO_CHAIN), CertificateError::UnknownIssuer);
        assert_eq!(why(NOT_FOR_A_SERVER), CertificateError::InvalidPurpose);
        assert_eq!(why(REVOKED), CertificateError::Revoked);
        assert!(matches!(why(-1), CertificateError::Other(_)));
    }
}
