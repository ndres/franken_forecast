//! Content-addressed `EvidenceVault` (`Tier H1`, `§19`): SHA-256 blobs with `Zstd-3` spill `> 64 KiB`.

use fcast_core::blinding::VerifiedEvidenceRef;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const ZSTD_SPILL_THRESHOLD_BYTES: usize = 64 * 1024;

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("vault I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("evidence handle error: {0}")]
    Handle(#[from] fcast_core::blinding::BlindingError),
    #[error("SHA-256 integrity mismatch for {handle}: expected {expected}, got {actual}")]
    IntegrityMismatch {
        handle: String,
        expected: String,
        actual: String,
    },
}

#[derive(Debug, Clone)]
pub struct VaultedArtifact {
    pub ev_handle: VerifiedEvidenceRef,
    pub sha256_hex: String,
    pub span_id: String,
    pub byte_len: usize,
    pub zstd_compressed: bool,
    pub blob_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct EvidenceVault {
    root: PathBuf,
}

impl EvidenceVault {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, VaultError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn put_bytes(&self, span_id: &str, payload: &[u8]) -> Result<VaultedArtifact, VaultError> {
        let digest = Sha256::digest(payload);
        let sha256_hex = format!("{digest:x}");
        let shard = &sha256_hex[0..2];
        let shard_dir = self.root.join(shard);
        fs::create_dir_all(&shard_dir)?;

        let zstd_compressed = payload.len() > ZSTD_SPILL_THRESHOLD_BYTES;
        let ext = if zstd_compressed { "blob.zst" } else { "blob" };
        let blob_path = shard_dir.join(format!("{sha256_hex}.{ext}"));

        if !blob_path.exists() {
            if zstd_compressed {
                let compressed = zstd::bulk::compress(payload, 3)?;
                fs::write(&blob_path, compressed)?;
            } else {
                fs::write(&blob_path, payload)?;
            }
        }

        let handle_str = format!("[ev:{}#{}]", &sha256_hex[0..16], span_id);
        let ev_handle = VerifiedEvidenceRef::parse(&handle_str)?;
        Ok(VaultedArtifact {
            ev_handle,
            sha256_hex,
            span_id: span_id.to_owned(),
            byte_len: payload.len(),
            zstd_compressed,
            blob_path,
        })
    }

    pub fn read_verified(
        &self,
        sha256_hex: &str,
        zstd_compressed: bool,
    ) -> Result<Vec<u8>, VaultError> {
        let shard = &sha256_hex[0..2];
        let ext = if zstd_compressed { "blob.zst" } else { "blob" };
        let blob_path = self.root.join(shard).join(format!("{sha256_hex}.{ext}"));
        let raw = fs::read(&blob_path)?;
        let bytes = if zstd_compressed {
            zstd::stream::decode_all(raw.as_slice())?
        } else {
            raw
        };
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if actual != sha256_hex {
            return Err(VaultError::IntegrityMismatch {
                handle: sha256_hex.to_owned(),
                expected: sha256_hex.to_owned(),
                actual,
            });
        }
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn roundtrips_small_and_zstd_spilled_blobs() {
        let dir = TempDir::new().expect("tempdir");
        let vault = EvidenceVault::open(dir.path()).expect("open");

        let small = b"CERT-EU advisory span";
        let a1 = vault.put_bytes("span1", small).expect("put small");
        assert!(!a1.zstd_compressed);
        assert_eq!(
            vault.read_verified(&a1.sha256_hex, false).expect("read"),
            small
        );

        let big = vec![b'A'; ZSTD_SPILL_THRESHOLD_BYTES + 4096];
        let a2 = vault.put_bytes("pdf_tbl", &big).expect("put big");
        assert!(a2.zstd_compressed);
        assert_eq!(
            vault.read_verified(&a2.sha256_hex, true).expect("read"),
            big
        );
    }
}
