//! Package builder — produces `.rcpkg` bytes from a manifest and files.

use chrono::Utc;
use sha2::{Digest, Sha256};
use std::io::Write;

use rustyclinic_core::error::{AppError, AppResult};

use crate::PackageManifest;
use crate::format::{
    CHECKSUM_LEN, FORMAT_VERSION, MAGIC, PackageEntry, PackageHeader, SIGNATURE_LEN,
};
use crate::signing;

/// Accumulates files and produces a deterministic `.rcpkg` archive.
pub struct PackageBuilder {
    manifest: PackageManifest,
    files: Vec<(String, Vec<u8>)>,
    created_by: String,
}

impl PackageBuilder {
    /// Create a new builder with the given manifest.
    pub fn new(manifest: PackageManifest) -> Self {
        Self {
            manifest,
            files: Vec::new(),
            created_by: "rustyclinic-packages".to_string(),
        }
    }

    /// Set the `created_by` field in the header.
    pub fn created_by(&mut self, who: &str) -> &mut Self {
        self.created_by = who.to_string();
        self
    }

    /// Add an arbitrary file to the package payload.
    pub fn add_file(&mut self, path: &str, contents: Vec<u8>) -> &mut Self {
        self.files.push((path.to_string(), contents));
        self
    }

    /// Add a form JSON file. Validates that `form_json` is valid JSON before
    /// accepting it. The file is placed under `forms/<id>.json` using the
    /// form's `"id"` field.
    pub fn add_form(&mut self, form_json: &str) -> Result<&mut Self, AppError> {
        // Validate that the JSON parses and has an "id" field.
        let value: serde_json::Value =
            serde_json::from_str(form_json).map_err(|e| AppError::Validation {
                message: format!("invalid form JSON: {e}"),
            })?;
        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| AppError::Validation {
                message: "form JSON must contain a string \"id\" field".to_string(),
            })?;
        let path = format!("forms/{id}.json");
        self.files.push((path, form_json.as_bytes().to_vec()));
        Ok(self)
    }

    /// Build the uncompressed tar payload and compute per-file metadata.
    /// Returns (tar_bytes, entries).
    fn build_tar_payload(&self) -> AppResult<(Vec<u8>, Vec<PackageEntry>)> {
        let mut entries = Vec::new();

        // Always include manifest.json as the first entry.
        let manifest_bytes =
            serde_json::to_vec_pretty(&self.manifest).map_err(|e| AppError::Validation {
                message: format!("failed to serialize manifest: {e}"),
            })?;

        // Collect all files: manifest first, then user-added files sorted for determinism.
        let mut all_files: Vec<(String, &[u8])> = Vec::new();
        all_files.push(("manifest.json".to_string(), &manifest_bytes));

        // Sort user files by path for deterministic output.
        let mut sorted_files: Vec<&(String, Vec<u8>)> = self.files.iter().collect();
        sorted_files.sort_by(|a, b| a.0.cmp(&b.0));

        for (path, contents) in &sorted_files {
            all_files.push((path.clone(), contents.as_slice()));
        }

        // Build tar archive.
        let mut tar_buf = Vec::new();
        {
            let mut tar_builder = tar::Builder::new(&mut tar_buf);
            for (path, contents) in &all_files {
                let mut header = tar::Header::new_gnu();
                header.set_size(contents.len() as u64);
                header.set_mode(0o644);
                // Use a fixed mtime for reproducibility.
                header.set_mtime(0);
                header.set_cksum();
                tar_builder
                    .append_data(&mut header, path, *contents)
                    .map_err(|e| AppError::Validation {
                        message: format!("tar append error: {e}"),
                    })?;

                let mut hasher = Sha256::new();
                hasher.update(contents);
                let hash = hex::encode(hasher.finalize());

                entries.push(PackageEntry {
                    path: path.clone(),
                    size: contents.len() as u64,
                    content_hash: hash,
                });
            }
            tar_builder.finish().map_err(|e| AppError::Validation {
                message: format!("tar finish error: {e}"),
            })?;
        }

        Ok((tar_buf, entries))
    }

    /// Produce the `.rcpkg` bytes without a signature (64 zero bytes in the
    /// signature slot).
    pub fn build(&self) -> AppResult<Vec<u8>> {
        self.build_inner(None)
    }

    /// Produce the `.rcpkg` bytes signed with the given Ed25519 key.
    pub fn build_signed(&self, signing_key: &[u8; 32]) -> AppResult<Vec<u8>> {
        self.build_inner(Some(signing_key))
    }

    fn build_inner(&self, signing_key: Option<&[u8; 32]>) -> AppResult<Vec<u8>> {
        let (tar_bytes, entries) = self.build_tar_payload()?;

        // Content hash = SHA-256 of uncompressed tar.
        let content_hash = {
            let mut hasher = Sha256::new();
            hasher.update(&tar_bytes);
            hex::encode(hasher.finalize())
        };

        let header = PackageHeader {
            manifest: self.manifest.clone(),
            created_at: Utc::now(),
            created_by: self.created_by.clone(),
            content_hash,
            entries,
        };

        let header_json = serde_json::to_vec(&header).map_err(|e| AppError::Validation {
            message: format!("failed to serialize header: {e}"),
        })?;

        // Compress payload with zstd.
        let compressed =
            zstd::encode_all(tar_bytes.as_slice(), 3).map_err(|e| AppError::Validation {
                message: format!("zstd compression error: {e}"),
            })?;

        // Payload checksum = SHA-256 of compressed payload.
        let payload_checksum = {
            let mut hasher = Sha256::new();
            hasher.update(&compressed);
            hasher.finalize()
        };

        // Compute signature over header_json if a key was provided.
        let signature: [u8; SIGNATURE_LEN] = if let Some(key) = signing_key {
            signing::sign(&header_json, key)?
        } else {
            [0u8; SIGNATURE_LEN]
        };

        // Assemble the .rcpkg bytes.
        let header_len = header_json.len() as u32;
        let total_len = MAGIC.len()
            + 1 // version byte
            + 4 // header_len u32
            + header_json.len()
            + SIGNATURE_LEN
            + compressed.len()
            + CHECKSUM_LEN;

        let mut output = Vec::with_capacity(total_len);
        output.write_all(MAGIC).map_err(|e| AppError::Validation {
            message: format!("write error: {e}"),
        })?;
        output
            .write_all(&[FORMAT_VERSION])
            .map_err(|e| AppError::Validation {
                message: format!("write error: {e}"),
            })?;
        output
            .write_all(&header_len.to_le_bytes())
            .map_err(|e| AppError::Validation {
                message: format!("write error: {e}"),
            })?;
        output
            .write_all(&header_json)
            .map_err(|e| AppError::Validation {
                message: format!("write error: {e}"),
            })?;
        output
            .write_all(&signature)
            .map_err(|e| AppError::Validation {
                message: format!("write error: {e}"),
            })?;
        output
            .write_all(&compressed)
            .map_err(|e| AppError::Validation {
                message: format!("write error: {e}"),
            })?;
        output
            .write_all(&payload_checksum)
            .map_err(|e| AppError::Validation {
                message: format!("write error: {e}"),
            })?;

        Ok(output)
    }
}

/// Tiny hex encoding helper to avoid an external crate.
mod hex {
    pub fn encode(bytes: impl AsRef<[u8]>) -> String {
        bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
    }
}
