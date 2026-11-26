//! Orderbook microstructure mark estimator (`§8`, `GATE-10`).
//!
//! Implements the exact Stoikov top-of-book microprice, liquidity/spread floor
//! (`L_tob >= $100`, `spread <= 800 bps`), tick-aware sweep credibility `phi_liq`,
//! `last_trade`/mid fallback, and `ThinOrWide` UI/alerting flags.

use fcast_core::forecast::{ProbClamped, sigmoid};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Minimum resting top-of-book depth (`$100` `[BUDGET]`).
pub const MIN_TOB_LIQUIDITY_USD: f64 = 100.0;
/// Maximum top-of-book spread (`800 bps` `[BUDGET]`).
pub const MAX_SPREAD_BPS: f64 = 800.0;
/// Reference top-of-book depth for logistic liquidity scaling (`$250` `[BUDGET]`).
pub const REF_TOB_LIQUIDITY_USD: f64 = 250.0;
/// Excess-spread exponential decay scale (`400 bps` `[BUDGET]`).
pub const EXCESS_SPREAD_SCALE_BPS: f64 = 400.0;
/// Reference `$500` sweep impact exponential decay scale (`0.05` `[BUDGET]`).
pub const SWEEP_IMPACT_SCALE: f64 = 0.05;
/// Default venue minimum tick size (`100 bps` = 1¢ `[UPSTREAM-SPEC]`).
pub const DEFAULT_TICK_BPS: f64 = 100.0;

#[derive(Debug, Error, PartialEq)]
pub enum MicrostructureError {
    #[error("invalid orderbook price(s): bid={bid}, ask={ask}")]
    InvalidPrices { bid: f64, ask: f64 },
    #[error("invalid orderbook depth(s): bid_size_usd={bid_size_usd}, ask_size_usd={ask_size_usd}")]
    InvalidSizes {
        bid_size_usd: f64,
        ask_size_usd: f64,
    },
    #[error("invalid sweep impact: {0}")]
    InvalidSweepImpact(f64),
    #[error("invalid minimum tick size in bps: {0}")]
    InvalidTickBps(f64),
}

/// Top-of-book orderbook snapshot used to compute [`BookMarkPrice`] (`§8`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct OrderbookSnapshot {
    pub bid_price: f64,
    pub bid_size_usd: f64,
    pub ask_price: f64,
    pub ask_size_usd: f64,
    /// Probability impact `iota(Q_ref)` of sweeping `Q_ref = $500` into the book.
    pub depth_impact_500_prob: f64,
    /// Venue minimum tick size in basis points (e.g. `100.0` for 1¢ venues).
    pub s_tick_bps: f64,
    /// Optional most recent trade price used as fallback on `ThinOrWide` books.
    pub last_trade: Option<f64>,
}

impl OrderbookSnapshot {
    /// Construct a snapshot with the default 1¢ (`100 bps`) minimum tick.
    #[must_use]
    pub const fn new(
        bid_price: f64,
        bid_size_usd: f64,
        ask_price: f64,
        ask_size_usd: f64,
        depth_impact_500_prob: f64,
    ) -> Self {
        Self {
            bid_price,
            bid_size_usd,
            ask_price,
            ask_size_usd,
            depth_impact_500_prob,
            s_tick_bps: DEFAULT_TICK_BPS,
            last_trade: None,
        }
    }

    /// Attach a `last_trade` price for `ThinOrWide` fallback.
    #[must_use]
    pub const fn with_last_trade(mut self, last_trade: f64) -> Self {
        self.last_trade = Some(last_trade);
        self
    }

    /// Override the venue minimum tick size in basis points.
    #[must_use]
    pub const fn with_tick_bps(mut self, s_tick_bps: f64) -> Self {
        self.s_tick_bps = s_tick_bps;
        self
    }

    /// Compute the [`BookMarkPrice`] from this snapshot (`§8`).
    pub fn compute_mark(&self) -> Result<BookMarkPrice, MicrostructureError> {
        BookMarkPrice::from_snapshot(self)
    }
}

/// Orderbook health classification (`§8`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    pub l_tob_usd: f64,
    pub spread_bps: f64,
    /// When `true`, UI/analytics dim 24h deltas in favor of 7d deltas (`§8`).
    pub dim_24h_use_7d: bool,
    /// When `true`, `15m`/`1h` short-horizon shock alerts are muted (`§8`).
    pub mute_short_shocks: bool,
}

impl BookMarkPrice {
    /// Compute exact `§8` Stoikov microprice, liquidity/spread floor, and `phi_liq`.
    pub fn from_snapshot(snap: &OrderbookSnapshot) -> Result<Self, MicrostructureError> {
        if !snap.bid_price.is_finite()
            || !snap.ask_price.is_finite()
            || snap.bid_price < 0.0
            || snap.ask_price > 1.0
            || snap.bid_price > snap.ask_price
        {
            return Err(MicrostructureError::InvalidPrices {
                bid: snap.bid_price,
                ask: snap.ask_price,
            });
        }
        if !snap.bid_size_usd.is_finite()
            || !snap.ask_size_usd.is_finite()
            || snap.bid_size_usd < 0.0
            || snap.ask_size_usd < 0.0
        {
            return Err(MicrostructureError::InvalidSizes {
                bid_size_usd: snap.bid_size_usd,
                ask_size_usd: snap.ask_size_usd,
            });
        }
        if !snap.depth_impact_500_prob.is_finite() || snap.depth_impact_500_prob < 0.0 {
            return Err(MicrostructureError::InvalidSweepImpact(
                snap.depth_impact_500_prob,
            ));
        }
        if !snap.s_tick_bps.is_finite() || snap.s_tick_bps < 0.0 {
            return Err(MicrostructureError::InvalidTickBps(snap.s_tick_bps));
        }

        let l_tob_usd = snap.bid_size_usd + snap.ask_size_usd;
        let spread_bps = (snap.ask_price - snap.bid_price) * 10_000.0;

        if l_tob_usd < MIN_TOB_LIQUIDITY_USD || spread_bps > MAX_SPREAD_BPS {
            let fallback = match snap.last_trade {
                Some(lt) if lt.is_finite() && lt >= snap.bid_price && lt <= snap.ask_price => lt,
                _ => 0.5 * (snap.bid_price + snap.ask_price),
            };
            return Ok(Self {
                p_micro: ProbClamped::clamp_lossy(fallback),
                phi_liq: 0.0,
                state: BookState::ThinOrWide,
                l_tob_usd,
                spread_bps,
                dim_24h_use_7d: true,
                mute_short_shocks: true,
            });
        }

        let raw_micro =
            (snap.bid_size_usd * snap.ask_price + snap.ask_size_usd * snap.bid_price) / l_tob_usd;
        let s_term = sigmoid((l_tob_usd / REF_TOB_LIQUIDITY_USD).ln());
        let excess_spread = (spread_bps - snap.s_tick_bps).max(0.0);
        let phi_liq = s_term
            * (-excess_spread / EXCESS_SPREAD_SCALE_BPS).exp()
            * (-snap.depth_impact_500_prob / SWEEP_IMPACT_SCALE).exp();

        Ok(Self {
            p_micro: ProbClamped::clamp_lossy(raw_micro),
            phi_liq,
            state: BookState::Liquid,
            l_tob_usd,
            spread_bps,
            dim_24h_use_7d: false,
            mute_short_shocks: false,
        })
    }

    /// Convenience helper indicating whether the book passed the liquidity & spread floor.
    #[must_use]
    pub const fn is_liquid(&self) -> bool {
        matches!(self.state, BookState::Liquid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tight_and_deep_books_match_reference() {
        let tight = OrderbookSnapshot::new(0.37, 2000.0, 0.39, 100.0, 0.01)
            .compute_mark()
            .expect("tight book");
        assert!(tight.is_liquid());
        assert!(!tight.dim_24h_use_7d);
        assert!(!tight.mute_short_shocks);
        assert!(tight.p_micro.get() > 0.388);
        assert!(tight.phi_liq > 0.50 && tight.phi_liq < 0.70);

        let deep = OrderbookSnapshot::new(0.49, 8000.0, 0.50, 8000.0, 0.002)
            .compute_mark()
            .expect("deep book");
        assert!(deep.is_liquid());
        assert!(deep.phi_liq > 0.90);
    }

    #[test]
    fn thin_book_falls_back_and_sets_flags() {
        let thin = OrderbookSnapshot::new(0.20, 40.0, 0.55, 30.0, 0.25)
            .with_last_trade(0.31)
            .compute_mark()
            .expect("thin book");
        assert_eq!(thin.state, BookState::ThinOrWide);
        assert_eq!(thin.phi_liq, 0.0);
        assert!(thin.dim_24h_use_7d);
        assert!(thin.mute_short_shocks);
        assert!((thin.p_micro.get() - 0.31).abs() < 1e-9);
    }
}
