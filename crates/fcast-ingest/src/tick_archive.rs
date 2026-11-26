//! Pure-Rust `Tier W1` Zstd-compressed partitioned L2 tick archive (`§19`).
//!
//! Layout: `ticks/venue={v}/year={Y}/month={M}/ticks.jsonl.zst`

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Default Zstd compression level (`Zstd-6`, matching `§19` `Tier W1`).
pub const DEFAULT_ZSTD_LEVEL: i32 = 6;

#[derive(Debug, Error)]
pub enum TickArchiveError {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid venue identifier (must match [a-zA-Z0-9_-]+): {0}")]
    InvalidVenue(String),
    #[error("invalid ISO-8601 timestamp (expected YYYY-MM-...): {0}")]
    InvalidTimestamp(String),
}

/// Single L2 orderbook tick record stored in `Tier W1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct L2TickRecord {
    pub venue: String,
    pub question_id: String,
    pub timestamp_utc: String,
    pub bid_price: f64,
    pub bid_size_usd: f64,
    pub ask_price: f64,
    pub ask_size_usd: f64,
    pub last_trade: Option<f64>,
    pub depth_impact_500_prob: f64,
    pub s_tick_bps: f64,
}

/// Partition identifier `(venue, year, month)` for `Tier W1` layout.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PartitionKey {
    pub venue: String,
    pub year: u16,
    pub month: u8,
}

impl PartitionKey {
    /// Extract a validated [`PartitionKey`] from a venue and ISO-8601 UTC timestamp.
    pub fn from_venue_and_timestamp(
        venue: &str,
        timestamp_utc: &str,
    ) -> Result<Self, TickArchiveError> {
        validate_venue(venue)?;
        let (year, month) = parse_year_month(timestamp_utc)?;
        Ok(Self {
            venue: venue.to_string(),
            year,
            month,
        })
    }

    /// Relative path under the archive root:
    /// `ticks/venue={v}/year={Y:04}/month={M:02}/ticks.jsonl.zst`.
    #[must_use]
    pub fn relative_path(&self) -> PathBuf {
        PathBuf::from("ticks")
            .join(format!("venue={}", self.venue))
            .join(format!("year={:04}", self.year))
            .join(format!("month={:02}", self.month))
            .join("ticks.jsonl.zst")
    }
}

/// Pure-Rust Zstd-compressed partitioned L2 tick writer & roundtrip reader (`Tier W1`).
#[derive(Debug, Clone)]
pub struct TickArchive {
    root_dir: PathBuf,
    compression_level: i32,
}

impl TickArchive {
    #[must_use]
    pub fn new(root_dir: impl Into<PathBuf>) -> Self {
        Self {
            root_dir: root_dir.into(),
            compression_level: DEFAULT_ZSTD_LEVEL,
        }
    }

    #[must_use]
    pub const fn with_compression_level(mut self, level: i32) -> Self {
        self.compression_level = level;
        self
    }

    #[must_use]
    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }

    /// Full filesystem path for a [`PartitionKey`].
    #[must_use]
    pub fn partition_path(&self, key: &PartitionKey) -> PathBuf {
        self.root_dir.join(key.relative_path())
    }

    /// Partition and append ticks into Zstd-compressed JSONL frames.
    ///
    /// Returns the sorted list of partition paths written.
    pub fn write_ticks(&self, ticks: &[L2TickRecord]) -> Result<Vec<PathBuf>, TickArchiveError> {
        let mut grouped: BTreeMap<PartitionKey, Vec<&L2TickRecord>> = BTreeMap::new();
        for tick in ticks {
            let key = PartitionKey::from_venue_and_timestamp(&tick.venue, &tick.timestamp_utc)?;
            grouped.entry(key).or_default().push(tick);
        }

        let mut written_paths = Vec::with_capacity(grouped.len());
        for (key, records) in grouped {
            let path = self.partition_path(&key);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let file = OpenOptions::new().create(true).append(true).open(&path)?;
            let mut encoder = zstd::stream::write::Encoder::new(file, self.compression_level)?;
            for rec in records {
                serde_json::to_writer(&mut encoder, rec)?;
                encoder.write_all(b"\n")?;
            }
            let mut file = encoder.finish()?;
            file.flush()?;
            written_paths.push(path);
        }
        Ok(written_paths)
    }

    /// Read all ticks from a single partition (seamlessly decoding concatenated Zstd frames).
    pub fn read_partition(
        &self,
        key: &PartitionKey,
    ) -> Result<Vec<L2TickRecord>, TickArchiveError> {
        let path = self.partition_path(key);
        let file = File::open(path)?;
        let decoder = zstd::stream::read::Decoder::new(file)?;
        let reader = BufReader::new(decoder);
        let mut out = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let rec: L2TickRecord = serde_json::from_str(&line)?;
            out.push(rec);
        }
        Ok(out)
    }

    /// List all partitions currently stored under `root_dir/ticks/`.
    pub fn list_partitions(&self) -> Result<Vec<PartitionKey>, TickArchiveError> {
        let ticks_dir = self.root_dir.join("ticks");
        if !ticks_dir.exists() {
            return Ok(Vec::new());
        }
        let mut keys = Vec::new();
        for v_ent in fs::read_dir(&ticks_dir)? {
            let v_ent = v_ent?;
            let v_name = v_ent.file_name();
            let Some(venue) = v_name.to_str().and_then(|s| s.strip_prefix("venue=")) else {
                continue;
            };
            if !v_ent.file_type()?.is_dir() {
                continue;
            }
            for y_ent in fs::read_dir(v_ent.path())? {
                let y_ent = y_ent?;
                let y_name = y_ent.file_name();
                let Some(year) = y_name
                    .to_str()
                    .and_then(|s| s.strip_prefix("year="))
                    .and_then(|s| s.parse::<u16>().ok())
                else {
                    continue;
                };
                if !y_ent.file_type()?.is_dir() {
                    continue;
                }
                for m_ent in fs::read_dir(y_ent.path())? {
                    let m_ent = m_ent?;
                    let m_name = m_ent.file_name();
                    let Some(month) = m_name
                        .to_str()
                        .and_then(|s| s.strip_prefix("month="))
                        .and_then(|s| s.parse::<u8>().ok())
                    else {
                        continue;
                    };
                    if m_ent.path().join("ticks.jsonl.zst").is_file() {
                        keys.push(PartitionKey {
                            venue: venue.to_string(),
                            year,
                            month,
                        });
                    }
                }
            }
        }
        keys.sort();
        Ok(keys)
    }

    /// Read all ticks for `venue`, optionally enforcing `timestamp_utc <= as_of_utc` (`INV-03`).
    pub fn read_venue_as_of(
        &self,
        venue: &str,
        as_of_utc: Option<&str>,
    ) -> Result<Vec<L2TickRecord>, TickArchiveError> {
        validate_venue(venue)?;
        let mut out = Vec::new();
        for key in self.list_partitions()? {
            if key.venue != venue {
                continue;
            }
            for rec in self.read_partition(&key)? {
                if as_of_utc.is_none_or(|cutoff| rec.timestamp_utc.as_str() <= cutoff) {
                    out.push(rec);
                }
            }
        }
        out.sort_by(|a, b| a.timestamp_utc.cmp(&b.timestamp_utc));
        Ok(out)
    }
}

fn validate_venue(venue: &str) -> Result<(), TickArchiveError> {
    if venue.is_empty()
        || !venue
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(TickArchiveError::InvalidVenue(venue.to_string()));
    }
    Ok(())
}

fn parse_year_month(ts: &str) -> Result<(u16, u8), TickArchiveError> {
    let bytes = ts.as_bytes();
    if bytes.len() < 7 || bytes[4] != b'-' {
        return Err(TickArchiveError::InvalidTimestamp(ts.to_string()));
    }
    let year: u16 = ts[0..4]
        .parse()
        .map_err(|_| TickArchiveError::InvalidTimestamp(ts.to_string()))?;
    let month: u8 = ts[5..7]
        .parse()
        .map_err(|_| TickArchiveError::InvalidTimestamp(ts.to_string()))?;
    if !(1..=12).contains(&month) {
        return Err(TickArchiveError::InvalidTimestamp(ts.to_string()));
    }
    Ok((year, month))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn zstd_partitioned_roundtrip_and_as_of_filter() {
        let dir = TempDir::new().expect("tempdir");
        let archive = TickArchive::new(dir.path());

        let batch1 = vec![
            L2TickRecord {
                venue: "polymarket".into(),
                question_id: "q_baltic".into(),
                timestamp_utc: "2026-11-25T19:45:00Z".into(),
                bid_price: 0.37,
                bid_size_usd: 2000.0,
                ask_price: 0.39,
                ask_size_usd: 100.0,
                last_trade: Some(0.38),
                depth_impact_500_prob: 0.01,
                s_tick_bps: 100.0,
            },
            L2TickRecord {
                venue: "kalshi".into(),
                question_id: "q_fed".into(),
                timestamp_utc: "2026-12-01T10:00:00Z".into(),
                bid_price: 0.49,
                bid_size_usd: 8000.0,
                ask_price: 0.50,
                ask_size_usd: 8000.0,
                last_trade: Some(0.495),
                depth_impact_500_prob: 0.002,
                s_tick_bps: 100.0,
            },
        ];
        let batch2 = vec![L2TickRecord {
            venue: "polymarket".into(),
            question_id: "q_baltic".into(),
            timestamp_utc: "2026-11-26T01:00:00Z".into(),
            bid_price: 0.40,
            bid_size_usd: 3000.0,
            ask_price: 0.41,
            ask_size_usd: 2500.0,
            last_trade: Some(0.405),
            depth_impact_500_prob: 0.005,
            s_tick_bps: 100.0,
        }];

        let paths1 = archive.write_ticks(&batch1).expect("write batch1");
        assert_eq!(paths1.len(), 2);
        let paths2 = archive.write_ticks(&batch2).expect("write batch2");
        assert_eq!(paths2.len(), 1);
        assert!(paths2[0].ends_with("ticks/venue=polymarket/year=2026/month=11/ticks.jsonl.zst"));

        let poly_key = PartitionKey {
            venue: "polymarket".into(),
            year: 2026,
            month: 11,
        };
        let poly_all = archive.read_partition(&poly_key).expect("read poly");
        assert_eq!(poly_all.len(), 2);
        assert_eq!(poly_all[0], batch1[0]);
        assert_eq!(poly_all[1], batch2[0]);

        // INV-03 as_of_utc cutoff excludes the later tick
        let poly_cutoff = archive
            .read_venue_as_of("polymarket", Some("2026-11-25T23:59:59Z"))
            .expect("as_of");
        assert_eq!(poly_cutoff.len(), 1);
        assert_eq!(poly_cutoff[0], batch1[0]);
    }
}
