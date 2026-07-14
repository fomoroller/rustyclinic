//! Package reader — parses and verifies `.rcpkg` bytes.

use sha2::{Digest, Sha256};
use std::io::Read;

use rustyclinic_core::error::{AppError, AppResult};

use crate::format::{
    CHECKSUM_LEN, FORMAT_VERSION, MAGIC, PackageEntry, PackageHeader, SIGNATURE_LEN,
};
use crate::signing;

/// A parsed `.rcpkg` package.
pub struct PackageReader {
    header: PackageHeader,
    header_json: Vec<u8>,
    signature: [u8; SIGNATURE_LEN],
    compressed_payload: Vec<u8>,
    stored_checksum: [u8; CHECKSUM_LEN],
    payload: Vec<u8>,
}

impl PackageReader {
    /// Parse an `.rcpkg` from raw bytes.
    pub fn open(data: &[u8]) -> AppResult<Self> {
        let min_size = MAGIC.len() + 1 + 4 + SIGNATURE_LEN + CHECKSUM_LEN;
        if data.len() < min_size {
            return Err(AppError::Validation {
                message: "package too small to be a valid .rcpkg".to_string(),
            });
        }

        // Magic.
        if &data[0..4] != MAGIC.as_slice() {
            return Err(AppError::Validation {
                message: "invalid magic bytes — not an .rcpkg file".to_string(),
            });
        }

        // Version.
        let version = data[4];
        if version != FORMAT_VERSION {
            return Err(AppError::Validation {
                message: format!("unsupported format version: {version}"),
            });
        }

        // Header length.
        let header_len = u32::from_le_bytes([data[5], data[6], data[7], data[8]]) as usize;
        let header_start = 9;
        let header_end = header_start + header_len;

        if data.len() < header_end + SIGNATURE_LEN + CHECKSUM_LEN {
            return Err(AppError::Validation {
                message: "package truncated: not enough bytes for header + signature + checksum"
                    .to_string(),
            });
        }

        let header_json = data[header_start..header_end].to_vec();
        let header: PackageHeader =
            serde_json::from_slice(&header_json).map_err(|e| AppError::Validation {
                message: format!("invalid package header JSON: {e}"),
            })?;

        // Signature.
        let sig_start = header_end;
        let sig_end = sig_start + SIGNATURE_LEN;
        let mut signature = [0u8; SIGNATURE_LEN];
        signature.copy_from_slice(&data[sig_start..sig_end]);

        // Payload (compressed) and trailing checksum.
        let payload_start = sig_end;
        let checksum_start = data.len() - CHECKSUM_LEN;
        if payload_start > checksum_start {
            return Err(AppError::Validation {
                message: "package truncated: no room for payload".to_string(),
            });
        }
        let compressed_payload = data[payload_start..checksum_start].to_vec();
        let mut stored_checksum = [0u8; CHECKSUM_LEN];
        stored_checksum.copy_from_slice(&data[checksum_start..]);

        // Decompress payload.
        let payload =
            zstd::decode_all(compressed_payload.as_slice()).map_err(|e| AppError::Validation {
                message: format!("zstd decompression error: {e}"),
            })?;

        Ok(Self {
            header,
            header_json,
            signature,
            compressed_payload,
            stored_checksum,
            payload,
        })
    }

    /// Access the parsed header.
    pub fn header(&self) -> &PackageHeader {
        &self.header
    }

    /// Verify the Ed25519 signature against the given public key.
    /// Returns `Ok(true)` if valid, `Ok(false)` if the signature doesn't match.
    pub fn verify_signature(&self, public_key: &[u8; 32]) -> AppResult<bool> {
        signing::verify(&self.header_json, &self.signature, public_key)
    }

    /// Verify payload checksums:
    /// 1. SHA-256 of compressed payload matches the trailing checksum.
    /// 2. SHA-256 of uncompressed payload matches `header.content_hash`.
    /// 3. Per-file hashes match.
    pub fn verify_checksums(&self) -> AppResult<bool> {
        // Check compressed payload checksum.
        let mut hasher = Sha256::new();
        hasher.update(&self.compressed_payload);
        let computed_checksum = hasher.finalize();
        if computed_checksum.as_slice() != self.stored_checksum.as_slice() {
            return Ok(false);
        }

        // Check uncompressed content hash.
        let mut hasher = Sha256::new();
        hasher.update(&self.payload);
        let computed_content = hex_encode(hasher.finalize().as_slice());
        if computed_content != self.header.content_hash {
            return Ok(false);
        }

        // Check per-file hashes.
        let files = self.extract_all_files()?;
        for entry in &self.header.entries {
            let file_data = files.iter().find(|(p, _)| p == &entry.path).map(|(_, d)| d);
            match file_data {
                Some(data) => {
                    let mut hasher = Sha256::new();
                    hasher.update(data);
                    let hash = hex_encode(hasher.finalize().as_slice());
                    if hash != entry.content_hash {
                        return Ok(false);
                    }
                }
                None => return Ok(false),
            }
        }

        Ok(true)
    }

    /// Read a single file from the decompressed tar payload by path.
    pub fn read_file(&self, path: &str) -> AppResult<Vec<u8>> {
        let mut archive = tar::Archive::new(self.payload.as_slice());
        let entries = archive.entries().map_err(|e| AppError::Validation {
            message: format!("tar entries error: {e}"),
        })?;

        for entry_result in entries {
            let mut entry = entry_result.map_err(|e| AppError::Validation {
                message: format!("tar entry error: {e}"),
            })?;
            let entry_path = entry
                .path()
                .map_err(|e| AppError::Validation {
                    message: format!("tar path error: {e}"),
                })?
                .to_string_lossy()
                .to_string();
            if entry_path == path {
                let mut buf = Vec::new();
                entry
                    .read_to_end(&mut buf)
                    .map_err(|e| AppError::Validation {
                        message: format!("tar read error: {e}"),
                    })?;
                return Ok(buf);
            }
        }

        Err(AppError::Validation {
            message: format!("file not found in package: {path}"),
        })
    }

    /// List all file entries from the header.
    pub fn list_files(&self) -> Vec<&PackageEntry> {
        self.header.entries.iter().collect()
    }

    /// Extract all form JSON files (those under `forms/`).
    /// Returns `(form_id, json_content)` pairs.
    pub fn extract_forms(&self) -> AppResult<Vec<(String, String)>> {
        self.extract_named_json_files("forms/", "form", true)
    }

    pub fn extract_reports(&self) -> AppResult<Vec<(String, String)>> {
        self.extract_named_json_files("reports/", "report", true)
    }

    pub fn extract_terminology_artifacts(&self) -> AppResult<Vec<(String, String)>> {
        self.extract_named_json_files("terminology/", "terminology artifact", false)
    }

    pub fn extract_deployment_settings(&self) -> AppResult<Option<String>> {
        let files = self.extract_all_files()?;
        let mut settings: Option<String> = None;

        for (path, data) in files {
            if let Some(_rest) = path.strip_prefix("deployment/") {
                if path != "deployment/settings.json" {
                    return Err(AppError::Validation {
                        message: format!(
                            "unsupported deployment payload path: {path} (expected deployment/settings.json)",
                        ),
                    });
                }

                if settings.is_some() {
                    return Err(AppError::Validation {
                        message: "duplicate deployment settings file found".to_string(),
                    });
                }

                let content = Self::decode_utf8_json_object(&data, "deployment settings")?;
                settings = Some(content);
            }
        }

        Ok(settings)
    }

    fn extract_named_json_files(
        &self,
        prefix: &str,
        artifact_kind: &str,
        require_id_match: bool,
    ) -> AppResult<Vec<(String, String)>> {
        let mut extracted = Vec::new();
        let files = self.extract_all_files()?;

        for (path, data) in files {
            let Some(rest) = path.strip_prefix(prefix) else {
                continue;
            };

            if rest.contains('/') {
                return Err(AppError::Validation {
                    message: format!(
                        "invalid {artifact_kind} payload path: {path} (nested directories are not supported)",
                    ),
                });
            }

            let Some(name) = rest.strip_suffix(".json") else {
                return Err(AppError::Validation {
                    message: format!(
                        "invalid {artifact_kind} payload path: {path} (expected .json extension)",
                    ),
                });
            };

            if name.is_empty() {
                return Err(AppError::Validation {
                    message: format!("invalid {artifact_kind} payload path: {path}"),
                });
            }

            let content = Self::decode_utf8_json_object(&data, artifact_kind)?;

            if require_id_match {
                let value: serde_json::Value =
                    serde_json::from_str(&content).map_err(|e| AppError::Validation {
                        message: format!("invalid {artifact_kind} JSON: {e}"),
                    })?;
                let id = value.get("id").and_then(|v| v.as_str()).ok_or_else(|| {
                    AppError::Validation {
                        message: format!(
                            "{artifact_kind} JSON at {path} must contain a string \"id\" field",
                        ),
                    }
                })?;
                if id != name {
                    return Err(AppError::Validation {
                        message: format!(
                            "{artifact_kind} id mismatch for {path}: file id is {name}, JSON id is {id}",
                        ),
                    });
                }
            }

            extracted.push((name.to_string(), content));
        }

        extracted.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(extracted)
    }

    fn decode_utf8_json_object(data: &[u8], artifact_kind: &str) -> AppResult<String> {
        let content = String::from_utf8(data.to_vec()).map_err(|e| AppError::Validation {
            message: format!("{artifact_kind} file is not valid UTF-8: {e}"),
        })?;

        let value: serde_json::Value =
            serde_json::from_str(&content).map_err(|e| AppError::Validation {
                message: format!("invalid {artifact_kind} JSON: {e}"),
            })?;

        if !value.is_object() {
            return Err(AppError::Validation {
                message: format!("{artifact_kind} JSON must be an object"),
            });
        }

        Ok(content)
    }

    /// Internal helper: extract all files from the tar payload.
    fn extract_all_files(&self) -> AppResult<Vec<(String, Vec<u8>)>> {
        let mut archive = tar::Archive::new(self.payload.as_slice());
        let entries = archive.entries().map_err(|e| AppError::Validation {
            message: format!("tar entries error: {e}"),
        })?;

        let mut files = Vec::new();
        for entry_result in entries {
            let mut entry = entry_result.map_err(|e| AppError::Validation {
                message: format!("tar entry error: {e}"),
            })?;
            let path = entry
                .path()
                .map_err(|e| AppError::Validation {
                    message: format!("tar path error: {e}"),
                })?
                .to_string_lossy()
                .to_string();
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| AppError::Validation {
                    message: format!("tar read error: {e}"),
                })?;
            files.push((path, buf));
        }
        Ok(files)
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
