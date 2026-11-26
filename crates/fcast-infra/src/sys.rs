//! `$FCAST_DATA_ROOT` 6-tier logical storage layout (`§19`) and zero-key initialization (`GATE-24`).

use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LayoutError {
    #[error("failed to initialize data root layout: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct DataRootLayout {
    pub root: PathBuf,
    pub db_path: PathBuf,
    pub vault_dir: PathBuf,
    pub ticks_dir: PathBuf,
    pub vintages_dir: PathBuf,
    pub media_dir: PathBuf,
    pub backups_dir: PathBuf,
    pub receipts_dir: PathBuf,
}

impl DataRootLayout {
    pub fn init(root: impl AsRef<Path>) -> Result<Self, LayoutError> {
        let root = root.as_ref().to_path_buf();
        let db_dir = root.join("db");
        let vault_dir = root.join("vault").join("sha256");
        let ticks_dir = root.join("ticks");
        let vintages_dir = root.join("vintages");
        let media_dir = root.join("media").join("audio_opus");
        let backups_dir = root.join("backups");
        let receipts_dir = root.join("receipts");

        for dir in [
            &db_dir,
            &vault_dir,
            &ticks_dir,
            &vintages_dir,
            &media_dir,
            &backups_dir,
            &receipts_dir,
        ] {
            fs::create_dir_all(dir)?;
        }

        Ok(Self {
            db_path: db_dir.join("ops.fsqlite"),
            root,
            vault_dir,
            ticks_dir,
            vintages_dir,
            media_dir,
            backups_dir,
            receipts_dir,
        })
    }
}
