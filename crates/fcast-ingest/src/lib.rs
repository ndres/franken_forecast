//! `fcast-ingest` (DAG Layer L2 — `SUB-02` Harvester).
//!
//! Hosts market orderbook microstructure (`BookMarkPrice`), `Metaculus` / `AskNews` /
//! `ForecastBench` connectors, and `available_utc`-locked `question_data_packs` (`M1`–`M2`).

use fcast_core::forecast::ProbClamped;
use serde::{Deserialize, Serialize};

/// Orderbook health classification (`§8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BookState {
    Liquid,
    ThinOrWide,
}

/// Top-of-book Stoikov microprice and liquidity credibility (`§8`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BookMarkPrice {
    pub p_micro: ProbClamped,
    pub phi_liq: f64,
    pub state: BookState,
}
