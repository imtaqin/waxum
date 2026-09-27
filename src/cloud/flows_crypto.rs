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

use aes_gcm::aead::consts::U16;
use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::aes::Aes128;
use aes_gcm::{AesGcm, Key, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use rsa::pkcs1::DecodeRsaPrivateKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::{Oaep, RsaPrivateKey};
use sha2::Sha256;

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

/// Parses a PEM-encoded RSA private key, accepting either PKCS#1
/// (`-----BEGIN RSA PRIVATE KEY-----`) or PKCS#8
/// (`-----BEGIN PRIVATE KEY-----`) since either is a plausible output
/// of `openssl genrsa`/`openssl pkcs8`.
pub fn parse_private_key(pem: &str) -> Result<RsaPrivateKey, FlowCryptoError> {
    RsaPrivateKey::from_pkcs8_pem(pem)
        .or_else(|_| RsaPrivateKey::from_pkcs1_pem(pem))
        .map_err(|_| FlowCryptoError::InvalidPrivateKey)
}

/// Decrypts one Flow Data Exchange request body
/// (`{encrypted_flow_data, encrypted_aes_key, initial_vector}`, all
/// base64) against the session's stored RSA private key.
pub fn decrypt_request(
    private_key_pem: &str,
    request: &serde_json::Value,
) -> Result<DecryptedRequest, FlowCryptoError> {
    let private_key = parse_private_key(private_key_pem)?;

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
    let aes_key_bytes = private_key
        .decrypt(Oaep::new::<Sha256>(), &wrapped_key)
        .map_err(|_| FlowCryptoError::KeyUnwrap)?;
    let aes_key: [u8; 16] = aes_key_bytes
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
    use rsa::pkcs8::EncodePrivateKey;

    fn test_keypair() -> (RsaPrivateKey, rsa::RsaPublicKey) {
        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let public_key = rsa::RsaPublicKey::from(&private_key);
        (private_key, public_key)
    }

    fn encrypt_like_meta(
        public_key: &rsa::RsaPublicKey,
        aes_key: &[u8; 16],
        iv: &[u8; 16],
        payload: &serde_json::Value,
    ) -> serde_json::Value {
        let mut rng = rand::thread_rng();
        let wrapped_key = public_key
            .encrypt(&mut rng, Oaep::new::<Sha256>(), aes_key)
            .unwrap();
        let cipher = FlowCipher::new(Key::<FlowCipher>::from_slice(aes_key));
        let nonce = Nonce::<U16>::from_slice(iv);
        let plaintext = serde_json::to_vec(payload).unwrap();
        let ciphertext_and_tag = cipher
            .encrypt(
                nonce,
                Payload {
                    msg: &plaintext,
                    aad: &[],
                },
            )
            .unwrap();
        serde_json::json!({
            "encrypted_flow_data": B64.encode(ciphertext_and_tag),
            "encrypted_aes_key": B64.encode(wrapped_key),
            "initial_vector": B64.encode(iv),
        })
    }

    #[test]
    fn round_trips_a_request_and_flips_the_iv_on_response() {
        let (private_key, public_key) = test_keypair();
        let pem = private_key.to_pkcs8_pem(Default::default()).unwrap();

        let aes_key = [7u8; 16];
        let iv = [3u8; 16];
        let request_payload = serde_json::json!({"version": "3.0", "action": "data_exchange", "data": {"foo": "bar"}});
        let meta_request = encrypt_like_meta(&public_key, &aes_key, &iv, &request_payload);

        let decrypted = decrypt_request(&pem, &meta_request).expect("decrypt should succeed");
        assert_eq!(decrypted.body, request_payload);

        let our_response = serde_json::json!({"screen": "SUCCESS", "data": {}});
        let encrypted_response = encrypt_response(&decrypted, &our_response).unwrap();

        let flipped_iv: [u8; 16] = iv.map(|b| !b);
        let cipher = FlowCipher::new(Key::<FlowCipher>::from_slice(&aes_key));
        let nonce = Nonce::<U16>::from_slice(&flipped_iv);
        let ciphertext_and_tag = B64.decode(&encrypted_response).unwrap();
        let plaintext = cipher
            .decrypt(
                nonce,
                Payload {
                    msg: &ciphertext_and_tag,
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
        let (private_key, public_key) = test_keypair();
        let pem = private_key.to_pkcs8_pem(Default::default()).unwrap();
        let aes_key = [7u8; 16];
        let iv = [3u8; 16];
        let mut rng = rand::thread_rng();
        let wrapped_key = public_key
            .encrypt(&mut rng, Oaep::new::<Sha256>(), &aes_key)
            .unwrap();
        let request = serde_json::json!({
            "encrypted_flow_data": "5NCE4Fae2vYxAzFbUsEzlDPDy7Z/1Oe0F8AbcI3p5ySwa2BprWH4ZeXidDkE8h/3yQ==",
            "encrypted_aes_key": B64.encode(wrapped_key),
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
        let (_correct_key, public_key) = test_keypair();
        let (wrong_key, _wrong_pub) = test_keypair();
        let wrong_pem = wrong_key.to_pkcs8_pem(Default::default()).unwrap();

        let aes_key = [9u8; 16];
        let iv = [1u8; 16];
        let payload = serde_json::json!({"action": "ping"});
        let meta_request = encrypt_like_meta(&public_key, &aes_key, &iv, &payload);

        let result = decrypt_request(&wrong_pem, &meta_request);
        assert!(matches!(result, Err(FlowCryptoError::KeyUnwrap)));
    }

    #[test]
    fn malformed_request_fails_cleanly() {
        let (private_key, _pub) = test_keypair();
        let pem = private_key.to_pkcs8_pem(Default::default()).unwrap();
        let result = decrypt_request(&pem, &serde_json::json!({}));
        assert!(matches!(result, Err(FlowCryptoError::Malformed(_))));
    }

    #[test]
    fn invalid_pem_fails_cleanly() {
        let result = decrypt_request("not a pem", &serde_json::json!({}));
        assert!(matches!(result, Err(FlowCryptoError::InvalidPrivateKey)));
    }
}
