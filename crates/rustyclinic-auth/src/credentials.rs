//! Password and PIN hashing with Argon2id.

use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rustyclinic_core::error::{AppError, AppResult};

/// Hash a password or PIN with Argon2id.
pub fn hash_credential(credential: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    let hash = argon2
        .hash_password(credential.as_bytes(), &salt)
        .map_err(|e| AppError::Database(format!("hashing failed: {e}")))?;
    Ok(hash.to_string())
}

/// Verify a credential against its Argon2id hash.
pub fn verify_credential(credential: &str, hash: &str) -> AppResult<bool> {
    let parsed = PasswordHash::new(hash)
        .map_err(|e| AppError::Database(format!("invalid hash format: {e}")))?;
    Ok(Argon2::default()
        .verify_password(credential.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_and_verify_password() {
        let password = "secureP@ssw0rd";
        let hash = hash_credential(password).expect("hash should succeed");

        assert!(verify_credential(password, &hash).expect("verify should succeed"));
        assert!(!verify_credential("wrong_password", &hash).expect("verify should succeed"));
    }

    #[test]
    fn test_hash_and_verify_pin() {
        let pin = "1234";
        let hash = hash_credential(pin).expect("hash should succeed");

        assert!(verify_credential(pin, &hash).expect("verify should succeed"));
        assert!(!verify_credential("5678", &hash).expect("verify should succeed"));
    }

    #[test]
    fn test_different_salts_produce_different_hashes() {
        let password = "samePassword";
        let hash1 = hash_credential(password).expect("hash1");
        let hash2 = hash_credential(password).expect("hash2");

        assert_ne!(hash1, hash2, "each hash should use a unique salt");
        assert!(verify_credential(password, &hash1).expect("verify1"));
        assert!(verify_credential(password, &hash2).expect("verify2"));
    }
}
