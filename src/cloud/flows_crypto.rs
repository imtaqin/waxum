//! WhatsApp Flows Data Exchange encryption handshake.
//!
//! Every request Meta sends to a Flow's data endpoint carries
//! `encrypted_flow_data` + `encrypted_aes_key` + `initial_vector` (all
//! base64). The AES key is wrapped with RSA-OAEP/SHA-256 under the
//! business's own keypair (the public half is registered with Meta via
//! [`crate::cloud::client::CloudClient::set_flow_encryption_public_key`];
//! the private half never leaves this server). Decrypt order: RSA-OAEP
//! unwrap the AES-128 key, then AES-128-GCM decrypt the flow payload
//! using that key and the given IV (the GCM tag is the trailing 16
//! bytes of the ciphertext, per Meta's documented layout).
//!
//! The IV is 16 bytes and is used whole as the GCM nonce, matching
//! Meta's reference `crypto.createDecipheriv("aes-128-gcm", key, iv)`.
//! Truncating it to the conventional 12-byte nonce would derive a
//! different initial counter block and fail against every real request,
//! so the cipher is `AesGcm<Aes128, U16>`, not the stock `Aes128Gcm`.
//!
//! The response is encrypted with the *same* AES key but a *flipped*
//! IV -- every byte XORed with `0xFF` -- per Meta's "Implementing Your
//! Flow Endpoint" spec; this is not a general AEAD convention, it is
//! specific to this handshake and is the detail most worth double
//! checking against Meta's published docs.
//!
//! RSA runs on `aws-lc-rs` (already linked through rustls), not the pure
//! Rust `rsa` crate: that crate's decryption is not constant-time
//! (RUSTSEC-2023-0071, "Marvin Attack", no fixed release), and this
//! endpoint decrypts ciphertexts supplied from outside. AWS-LC's
//! `EVP_PKEY_decrypt` is constant-time.

use aes_gcm::aead::consts::U16;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::aes::Aes128;
use aes_gcm::{AesGcm, Key, Nonce};
use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::rsa::{
    KeySize, OaepPrivateDecryptingKey, PrivateDecryptingKey, OAEP_SHA256_MGF1SHA256,
};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

/// AES-128-GCM with a 16-byte nonce -- see the module docs for why.
type FlowCipher = AesGcm<Aes128, U16>;

#[derive(Debug, thiserror::Error)]
pub enum FlowCryptoError {
    #[error("invalid private key PEM")]
    InvalidPrivateKey,
    #[error("failed to unwrap the AES key")]
    KeyUnwrap,
    #[error("malformed request: {0}")]
    Malformed(&'static str),
    #[error("failed to decrypt the flow payload")]
    Decrypt,
    #[error("failed to encrypt the response payload")]
    Encrypt,
}

/// A decrypted Flow Data Exchange request, plus everything needed to
/// encrypt the matching response with the same key under the
/// bit-flipped IV.
pub struct DecryptedRequest {
    pub body: serde_json::Value,
    aes_key: [u8; 16],
    iv: [u8; 16],
}

/// DER encoding of `rsaEncryption` (1.2.840.113549.1.1.1) with NULL
/// parameters, the AlgorithmIdentifier a PKCS#8 RSA key carries.
const RSA_ALGORITHM_ID: &[u8] = &[
    0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00,
];

fn der_len(len: usize) -> Vec<u8> {
    if len < 0x80 {
        return vec![len as u8];
    }
    let bytes: Vec<u8> = len
        .to_be_bytes()
        .into_iter()
        .skip_while(|b| *b == 0)
        .collect();
    let mut out = vec![0x80 | bytes.len() as u8];
    out.extend(bytes);
    out
}

fn der_tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend(der_len(content.len()));
    out.extend_from_slice(content);
    out
}

/// Wraps a PKCS#1 `RSAPrivateKey` in a PKCS#8 `PrivateKeyInfo`, since
/// AWS-LC only loads PKCS#8 and `openssl genrsa -traditional` (and older
/// OpenSSL defaults) still emit PKCS#1.
fn pkcs1_to_pkcs8(pkcs1: &[u8]) -> Vec<u8> {
    let mut body = vec![0x02, 0x01, 0x00];
    body.extend_from_slice(RSA_ALGORITHM_ID);
    body.extend(der_tlv(0x04, pkcs1));
    der_tlv(0x30, &body)
}

fn pem_body(pem: &str, label: &str) -> Option<Vec<u8>> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let start = pem.find(&begin)? + begin.len();
    let stop = pem[start..].find(&end)? + start;
    let b64: String = pem[start..stop]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    B64.decode(b64).ok()
}

fn pem_encode(label: &str, der: &[u8]) -> String {
    let b64 = B64.encode(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for line in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).unwrap_or_default());
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

/// Parses a PEM-encoded RSA private key, accepting either PKCS#8
/// (`-----BEGIN PRIVATE KEY-----`) or PKCS#1
/// (`-----BEGIN RSA PRIVATE KEY-----`).
pub fn parse_private_key(pem: &str) -> Result<PrivateDecryptingKey, FlowCryptoError> {
    let pkcs8 = match pem_body(pem, "PRIVATE KEY") {
        Some(der) => der,
        None => pkcs1_to_pkcs8(
            &pem_body(pem, "RSA PRIVATE KEY").ok_or(FlowCryptoError::InvalidPrivateKey)?,
        ),
    };
    PrivateDecryptingKey::from_pkcs8(&pkcs8).map_err(|_| FlowCryptoError::InvalidPrivateKey)
}

/// Generates a fresh 2048-bit RSA key for a Flow endpoint.
pub fn generate_private_key() -> Result<PrivateDecryptingKey, FlowCryptoError> {
    PrivateDecryptingKey::generate(KeySize::Rsa2048).map_err(|_| FlowCryptoError::InvalidPrivateKey)
}

/// PKCS#8 PEM of `key`, the form stored on the session.
pub fn private_key_pem(key: &PrivateDecryptingKey) -> Result<String, FlowCryptoError> {
    let der = key
        .as_der()
        .map_err(|_| FlowCryptoError::InvalidPrivateKey)?;
    Ok(pem_encode("PRIVATE KEY", der.as_ref()))
}

/// SubjectPublicKeyInfo PEM of `key`'s public half, the form Meta's
/// `whatsapp_business_encryption` endpoint accepts.
pub fn public_key_pem(key: &PrivateDecryptingKey) -> Result<String, FlowCryptoError> {
    let der = key
        .public_key()
        .as_der()
        .map_err(|_| FlowCryptoError::InvalidPrivateKey)?;
    Ok(pem_encode("PUBLIC KEY", der.as_ref()))
}

/// Decrypts one Flow Data Exchange request body
/// (`{encrypted_flow_data, encrypted_aes_key, initial_vector}`, all
/// base64) against the session's stored RSA private key.
pub fn decrypt_request(
    private_key_pem: &str,
    request: &serde_json::Value,
) -> Result<DecryptedRequest, FlowCryptoError> {
    let private_key = OaepPrivateDecryptingKey::new(parse_private_key(private_key_pem)?)
        .map_err(|_| FlowCryptoError::InvalidPrivateKey)?;

    let encrypted_aes_key = request
        .get("encrypted_aes_key")
        .and_then(|v| v.as_str())
        .ok_or(FlowCryptoError::Malformed("missing encrypted_aes_key"))?;
    let encrypted_flow_data = request
        .get("encrypted_flow_data")
        .and_then(|v| v.as_str())
        .ok_or(FlowCryptoError::Malformed("missing encrypted_flow_data"))?;
    let initial_vector = request
        .get("initial_vector")
        .and_then(|v| v.as_str())
        .ok_or(FlowCryptoError::Malformed("missing initial_vector"))?;

    let wrapped_key = B64
        .decode(encrypted_aes_key)
        .map_err(|_| FlowCryptoError::Malformed("encrypted_aes_key is not valid base64"))?;
    let mut key_buf = vec![0u8; private_key.min_output_size()];
    let aes_key: [u8; 16] = private_key
        .decrypt(&OAEP_SHA256_MGF1SHA256, &wrapped_key, &mut key_buf, None)
        .map_err(|_| FlowCryptoError::KeyUnwrap)?
        .try_into()
        .map_err(|_| FlowCryptoError::KeyUnwrap)?;

    let iv_bytes = B64
        .decode(initial_vector)
        .map_err(|_| FlowCryptoError::Malformed("initial_vector is not valid base64"))?;
    let iv: [u8; 16] = iv_bytes
        .try_into()
        .map_err(|_| FlowCryptoError::Malformed("initial_vector is not 16 bytes"))?;

    let ciphertext_and_tag = B64
        .decode(encrypted_flow_data)
        .map_err(|_| FlowCryptoError::Malformed("encrypted_flow_data is not valid base64"))?;

    let cipher = FlowCipher::new(Key::<FlowCipher>::from_slice(&aes_key));
    let nonce = Nonce::<U16>::from_slice(&iv);
    let plaintext = cipher
        .decrypt(
            nonce,
            Payload {
                msg: &ciphertext_and_tag,
                aad: &[],
            },
        )
        .map_err(|_| FlowCryptoError::Decrypt)?;

    let body: serde_json::Value =
        serde_json::from_slice(&plaintext).map_err(|_| FlowCryptoError::Decrypt)?;

    Ok(DecryptedRequest { body, aes_key, iv })
}

/// Encrypts a Flow Data Exchange response with the request's AES key
/// under the bit-flipped IV, returning the base64 text Meta expects as
/// the raw `text/plain` response body.
pub fn encrypt_response(
    decrypted: &DecryptedRequest,
    response: &serde_json::Value,
) -> Result<String, FlowCryptoError> {
    let flipped_iv: Vec<u8> = decrypted.iv.iter().map(|b| !b).collect();
    let cipher = FlowCipher::new(Key::<FlowCipher>::from_slice(&decrypted.aes_key));
    let nonce = Nonce::<U16>::from_slice(&flipped_iv);
    let plaintext = serde_json::to_vec(response).map_err(|_| FlowCryptoError::Encrypt)?;
    let ciphertext_and_tag = cipher
        .encrypt(
            nonce,
            Payload {
                msg: &plaintext,
                aad: &[],
            },
        )
        .map_err(|_| FlowCryptoError::Encrypt)?;
    Ok(B64.encode(ciphertext_and_tag))
}

/// `{"data": {"status": "active"}}`, the fixed response body Meta's
/// periodic `{"action": "ping"}` health check expects.
pub fn ping_response() -> serde_json::Value {
    serde_json::json!({ "data": { "status": "active" } })
}

pub fn is_ping(decrypted: &serde_json::Value) -> bool {
    decrypted.get("action").and_then(|v| v.as_str()) == Some("ping")
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::rsa::OaepPublicEncryptingKey;

    fn test_key() -> (PrivateDecryptingKey, String) {
        let key = generate_private_key().unwrap();
        let pem = private_key_pem(&key).unwrap();
        (key, pem)
    }

    fn wrap_key(key: &PrivateDecryptingKey, aes_key: &[u8; 16]) -> Vec<u8> {
        let public = OaepPublicEncryptingKey::new(key.public_key()).unwrap();
        let mut out = vec![0u8; public.ciphertext_size()];
        public
            .encrypt(&OAEP_SHA256_MGF1SHA256, aes_key, &mut out, None)
            .unwrap()
            .to_vec()
    }

    fn encrypt_like_meta(
        key: &PrivateDecryptingKey,
        aes_key: &[u8; 16],
        iv: &[u8; 16],
        payload: &serde_json::Value,
    ) -> serde_json::Value {
        let cipher = FlowCipher::new(Key::<FlowCipher>::from_slice(aes_key));
        let ciphertext_and_tag = cipher
            .encrypt(
                Nonce::<U16>::from_slice(iv),
                Payload {
                    msg: &serde_json::to_vec(payload).unwrap(),
                    aad: &[],
                },
            )
            .unwrap();
        serde_json::json!({
            "encrypted_flow_data": B64.encode(ciphertext_and_tag),
            "encrypted_aes_key": B64.encode(wrap_key(key, aes_key)),
            "initial_vector": B64.encode(iv),
        })
    }

    /// Reads one DER TLV at `at`, returning (tag, content range, end).
    fn tlv(der: &[u8], at: usize) -> (u8, std::ops::Range<usize>, usize) {
        let tag = der[at];
        let first = der[at + 1] as usize;
        let (len, header) = if first < 0x80 {
            (first, 2)
        } else {
            let n = first & 0x7f;
            let len = der[at + 2..at + 2 + n]
                .iter()
                .fold(0usize, |acc, b| (acc << 8) | *b as usize);
            (len, 2 + n)
        };
        let start = at + header;
        (tag, start..start + len, start + len)
    }

    #[test]
    fn round_trips_a_request_and_flips_the_iv_on_response() {
        let (key, pem) = test_key();
        let aes_key = [7u8; 16];
        let iv = [3u8; 16];
        let request_payload = serde_json::json!({"version": "3.0", "action": "data_exchange", "data": {"foo": "bar"}});
        let meta_request = encrypt_like_meta(&key, &aes_key, &iv, &request_payload);

        let decrypted = decrypt_request(&pem, &meta_request).expect("decrypt should succeed");
        assert_eq!(decrypted.body, request_payload);

        let our_response = serde_json::json!({"screen": "SUCCESS", "data": {}});
        let encrypted_response = encrypt_response(&decrypted, &our_response).unwrap();

        let flipped_iv: [u8; 16] = iv.map(|b| !b);
        let cipher = FlowCipher::new(Key::<FlowCipher>::from_slice(&aes_key));
        let plaintext = cipher
            .decrypt(
                Nonce::<U16>::from_slice(&flipped_iv),
                Payload {
                    msg: &B64.decode(&encrypted_response).unwrap(),
                    aad: &[],
                },
            )
            .unwrap();
        let round_tripped: serde_json::Value = serde_json::from_slice(&plaintext).unwrap();
        assert_eq!(round_tripped, our_response);
    }

    /// Known-answer vectors produced by Node's
    /// `crypto.createCipheriv("aes-128-gcm", key, iv16)` -- the exact call
    /// in Meta's reference Flow endpoint -- with key `[7; 16]` and IV
    /// `[3; 16]`. Guards against a self-consistent but non-interoperable
    /// scheme (e.g. truncating the IV to a 12-byte nonce), which the
    /// round-trip test alone cannot catch.
    #[test]
    fn matches_meta_reference_implementation_byte_for_byte() {
        let (key, pem) = test_key();
        let aes_key = [7u8; 16];
        let iv = [3u8; 16];
        let request = serde_json::json!({
            "encrypted_flow_data": "5NCE4Fae2vYxAzFbUsEzlDPDy7Z/1Oe0F8AbcI3p5ySwa2BprWH4ZeXidDkE8h/3yQ==",
            "encrypted_aes_key": B64.encode(wrap_key(&key, &aes_key)),
            "initial_vector": B64.encode(iv),
        });

        let decrypted = decrypt_request(&pem, &request).expect("decrypts Node-produced payload");
        assert_eq!(
            decrypted.body,
            serde_json::json!({"action": "ping", "version": "3.0"})
        );
        assert!(is_ping(&decrypted.body));

        let response = encrypt_response(&decrypted, &ping_response()).unwrap();
        assert_eq!(
            response,
            "srb6MSaUwYvqjusc3lmYnpa1xrQ4K6zn5LykK7K/1OgV6M7j9NauFfih9Hg="
        );
    }

    /// A PKCS#1 PEM (`openssl genrsa -traditional`) must load as the same
    /// key as its PKCS#8 form. The PKCS#1 body is cut out of the PKCS#8
    /// DER here rather than committed as a fixture, and AWS-LC's own
    /// PKCS#8 parser validates the re-wrapped structure.
    #[test]
    fn pkcs1_pem_loads_as_the_same_key_as_pkcs8() {
        let (key, _) = test_key();
        let pkcs8 = key.as_der().unwrap();
        let der = pkcs8.as_ref();
        let (_, outer, _) = tlv(der, 0);
        let (_, _, after_version) = tlv(der, outer.start);
        let (_, _, after_alg) = tlv(der, after_version);
        let (tag, pkcs1, _) = tlv(der, after_alg);
        assert_eq!(tag, 0x04);
        let pkcs1_pem = pem_encode("RSA PRIVATE KEY", &der[pkcs1]);

        let reloaded = parse_private_key(&pkcs1_pem).expect("PKCS#1 PEM loads");
        assert_eq!(
            public_key_pem(&reloaded).unwrap(),
            public_key_pem(&key).unwrap()
        );
        assert_eq!(pkcs1_to_pkcs8(&der[tlv(der, after_alg).1]), der.to_vec());
    }

    #[test]
    fn pem_round_trips_and_public_key_is_spki() {
        let (key, pem) = test_key();
        assert!(pem.starts_with("-----BEGIN PRIVATE KEY-----\n"));
        assert!(pem.lines().all(|l| l.len() <= 64));
        let reloaded = parse_private_key(&pem).unwrap();
        let public = public_key_pem(&key).unwrap();
        assert!(public.starts_with("-----BEGIN PUBLIC KEY-----\n"));
        assert_eq!(public_key_pem(&reloaded).unwrap(), public);
        assert_eq!(key.key_size_bits(), 2048);
    }

    #[test]
    fn der_lengths_use_long_form_above_127_bytes() {
        assert_eq!(der_len(5), vec![5]);
        assert_eq!(der_len(0x80), vec![0x81, 0x80]);
        assert_eq!(der_len(0x04a8), vec![0x82, 0x04, 0xa8]);
    }

    #[test]
    fn ping_action_is_recognized() {
        let ping = serde_json::json!({"version": "3.0", "action": "ping"});
        assert!(is_ping(&ping));
        let data_exchange = serde_json::json!({"version": "3.0", "action": "data_exchange"});
        assert!(!is_ping(&data_exchange));
        assert_eq!(
            ping_response(),
            serde_json::json!({"data": {"status": "active"}})
        );
    }

    #[test]
    fn wrong_key_fails_cleanly_instead_of_panicking() {
        let (right, _) = test_key();
        let (_, wrong_pem) = test_key();
        let meta_request = encrypt_like_meta(
            &right,
            &[9u8; 16],
            &[1u8; 16],
            &serde_json::json!({"action": "ping"}),
        );
        let result = decrypt_request(&wrong_pem, &meta_request);
        assert!(matches!(result, Err(FlowCryptoError::KeyUnwrap)));
    }

    #[test]
    fn malformed_request_fails_cleanly() {
        let (_, pem) = test_key();
        let result = decrypt_request(&pem, &serde_json::json!({}));
        assert!(matches!(result, Err(FlowCryptoError::Malformed(_))));
    }

    #[test]
    fn invalid_pem_fails_cleanly() {
        for bad in [
            "not a pem",
            "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n",
        ] {
            let result = decrypt_request(bad, &serde_json::json!({}));
            assert!(matches!(result, Err(FlowCryptoError::InvalidPrivateKey)));
        }
    }
}
