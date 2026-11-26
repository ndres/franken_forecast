//! `$FCAST_DATA_ROOT` 6-tier logical storage layout (`§19`), zero-key initialization (`GATE-24`),
//! singleton `DaemonLock` (`GATE-19`), and POSIX `sun_path` validation (`§13`).

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// POSIX `sockaddr_un.sun_path` ceiling on macOS / BSD (`§13`).
pub const MAX_UDS_SUN_PATH_BYTES: usize = 104;

#[derive(Debug, Error)]
pub enum LayoutError {
    #[error("failed to initialize data root layout: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Error)]
pub enum DaemonLockError {
    #[error("daemon lock already held by another fcastd instance: {path}")]
    AlreadyLocked { path: PathBuf },
    #[error("failed to acquire daemon lock: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum UdsPathError {
    #[error("Unix domain socket path must not be empty")]
    EmptyPath,
    #[error(
        "Unix domain socket path exceeds {max_bytes}-byte POSIX sun_path ceiling ({byte_len} bytes): {path}"
    )]
    PathTooLong {
        path: PathBuf,
        byte_len: usize,
        max_bytes: usize,
    },
}

/// Validates that a Unix domain socket path fits within the `<= 104`-byte POSIX `sun_path` ceiling.
pub fn validate_uds_sun_path(path: impl AsRef<Path>) -> Result<(), UdsPathError> {
    let path = path.as_ref();
    let byte_len = path.as_os_str().as_encoded_bytes().len();
    if byte_len == 0 {
        return Err(UdsPathError::EmptyPath);
    }
    if byte_len > MAX_UDS_SUN_PATH_BYTES {
        return Err(UdsPathError::PathTooLong {
            path: path.to_path_buf(),
            byte_len,
            max_bytes: MAX_UDS_SUN_PATH_BYTES,
        });
    }
    Ok(())
}

/// Exclusive non-blocking singleton file lock (`flock(LOCK_EX | LOCK_NB)` + `O_CLOEXEC`)
/// preventing duplicate `fcastd` instances (`GATE-19`, `§13`).
#[derive(Debug)]
pub struct DaemonLock {
    file: File,
    path: PathBuf,
}

impl DaemonLock {
    /// Acquires an exclusive non-blocking lock on `path`.
    ///
    /// Fails with [`DaemonLockError::AlreadyLocked`] if another live `fcastd` instance holds the lock.
    pub fn acquire(path: impl AsRef<Path>) -> Result<Self, DaemonLockError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)?;
        match file.try_lock() {
            Ok(()) => {
                file.set_len(0)?;
                writeln!(file, "{}", std::process::id())?;
                file.flush()?;
                Ok(Self { file, path })
            }
            Err(TryLockError::WouldBlock) => Err(DaemonLockError::AlreadyLocked { path }),
            Err(TryLockError::Error(err)) => Err(DaemonLockError::Io(err)),
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for DaemonLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
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

    #[must_use]
    pub fn daemon_lock_path(&self) -> PathBuf {
        self.root.join("daemon.lock")
    }

    #[must_use]
    pub fn daemon_sock_path(&self) -> PathBuf {
        self.root.join("daemon.sock")
    }
}
