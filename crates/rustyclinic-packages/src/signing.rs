//! Ed25519 signing infrastructure for package integrity.

use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use rustyclinic_core::error::{AppError, AppResult};

/// An Ed25519 key pair for signing and verifying packages.
pub struct KeyPair {
    pub signing_key: [u8; 32],
    pub verifying_key: [u8; 32],
}

/// Generate a new random Ed25519 key pair.
pub fn generate_keypair() -> KeyPair {
    let mut csprng = rand_core::OsRng;
    let signing = SigningKey::generate(&mut csprng);
    let verifying = signing.verifying_key();
    KeyPair {
        signing_key: signing.to_bytes(),
        verifying_key: verifying.to_bytes(),
    }
}

/// Sign `data` with the given 32-byte Ed25519 signing key.
pub fn sign(data: &[u8], signing_key: &[u8; 32]) -> AppResult<[u8; 64]> {
    let key = SigningKey::from_bytes(signing_key);
    let sig = key.sign(data);
    Ok(sig.to_bytes())
}

/// Verify an Ed25519 signature over `data`.
pub fn verify(data: &[u8], signature: &[u8; 64], verifying_key: &[u8; 32]) -> AppResult<bool> {
    let key = VerifyingKey::from_bytes(verifying_key).map_err(|e| AppError::Validation {
        message: format!("invalid verifying key: {e}"),
    })?;
    let sig = ed25519_dalek::Signature::from_bytes(signature);
    Ok(key.verify(data, &sig).is_ok())
}
