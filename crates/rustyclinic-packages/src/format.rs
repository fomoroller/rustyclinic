//! Package file format types and constants.
//!
//! An `.rcpkg` file has the following layout:
//!
//! ```text
//! ┌─────────────────────────────────────┐
//! │ MAGIC: "RCPK" (4 bytes)            │
//! │ VERSION: u8 (format version = 1)   │
//! │ HEADER_LEN: u32 LE                 │
//! │ HEADER: JSON PackageHeader         │
//! │ SIGNATURE: Ed25519 (64 bytes)      │
//! │ PAYLOAD: zstd-compressed tar       │
//! │ CHECKSUM: SHA-256 of payload (32)  │
//! └─────────────────────────────────────┘
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::PackageManifest;

/// Magic bytes at the start of every `.rcpkg` file.
pub const MAGIC: &[u8; 4] = b"RCPK";

/// Current format version.
pub const FORMAT_VERSION: u8 = 1;

/// Size of an Ed25519 signature in bytes.
pub const SIGNATURE_LEN: usize = 64;

/// Size of a SHA-256 digest in bytes.
pub const CHECKSUM_LEN: usize = 32;

/// Package header — serialized as JSON inside the `.rcpkg` envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageHeader {
    /// The full package manifest.
    pub manifest: PackageManifest,
    /// When this package was created.
    pub created_at: DateTime<Utc>,
    /// Who (or what tool) created this package.
    pub created_by: String,
    /// SHA-256 hex digest of the *uncompressed* tar payload.
    pub content_hash: String,
    /// Listing of every file inside the tar payload.
    pub entries: Vec<PackageEntry>,
}

/// Metadata for a single file inside the package payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageEntry {
    /// Relative path inside the tar (e.g. `"forms/anc-visit.json"`).
    pub path: String,
    /// Size in bytes.
    pub size: u64,
    /// SHA-256 hex digest of the file contents.
    pub content_hash: String,
}
