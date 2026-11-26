//! `GATE-10` integration tests: `BookMarkPrice` microstructure, Stoikov microprice,
//! liquidity/spread floors, `phi_liq` credibility, fallback flags, and `Tier W1` tick replay.

use fcast_ingest::{
    BookMarkPrice, BookState, L2TickRecord, MicrostructureError, OrderbookSnapshot, PartitionKey,
    TickArchive,
};
use proptest::prelude::*;
use tempfile::TempDir;

#[test]
fn gate10_tight_and_deep_liquid_books() {
    // Tight 2c book ($2,000 at 0.37 vs $100 at 0.39) -> microprice leans toward 0.39 (~0.3890), phi ~ 0.57
    let tight = OrderbookSnapshot::new(0.37, 2_000.0, 0.39, 100.0, 0.01)
        .compute_mark()
        .expect("tight liquid book");
    assert_eq!(tight.state, BookState::Liquid);
    assert!(!tight.dim_24h_use_7d);
    assert!(!tight.mute_short_shocks);
    assert!((tight.l_tob_usd - 2_100.0).abs() < 1e-9);
    assert!((tight.spread_bps - 200.0).abs() < 1e-6);
    assert!(tight.p_micro.get() > 0.388 && tight.p_micro.get() < 0.390);
    assert!(tight.phi_liq > 0.50 && tight.phi_liq < 0.70);

    // Deep 1c book ($8,000 at 0.49 vs $8,000 at 0.50) -> unpenalized at 1c tick, phi > 0.90
    let deep = OrderbookSnapshot::new(0.49, 8_000.0, 0.50, 8_000.0, 0.002)
        .compute_mark()
        .expect("deep liquid book");
    assert_eq!(deep.state, BookState::Liquid);
    assert!(deep.phi_liq > 0.90);
    assert!((deep.p_micro.get() - 0.495).abs() < 1e-9);
}

#[test]
fn gate10_thin_and_wide_books_fallback_and_mute_flags() {
    // Thin stub book ($40 at 0.20 vs $30 at 0.55) with valid last_trade inside [bid, ask]
    let thin = OrderbookSnapshot::new(0.20, 40.0, 0.55, 30.0, 0.25)
        .with_last_trade(0.31)
        .compute_mark()
        .expect("thin stub book");
    assert_eq!(thin.state, BookState::ThinOrWide);
    assert_eq!(thin.phi_liq, 0.0);
    assert!(thin.dim_24h_use_7d);
    assert!(thin.mute_short_shocks);
    assert!((thin.p_micro.get() - 0.31).abs() < 1e-9);

    // Wide spread (> 800 bps) even with deep size, and stale last_trade outside [bid, ask] -> mid fallback
    let wide = OrderbookSnapshot::new(0.30, 5_000.0, 0.42, 5_000.0, 0.01)
        .with_last_trade(0.55)
        .compute_mark()
        .expect("wide spread book");
    assert_eq!(wide.state, BookState::ThinOrWide);
    assert_eq!(wide.phi_liq, 0.0);
    assert!(wide.dim_24h_use_7d);
    assert!(wide.mute_short_shocks);
    assert!((wide.p_micro.get() - 0.36).abs() < 1e-9);
}

#[test]
fn gate10_exact_floor_boundaries_and_validation_errors() {
    // Exact $100 TOB and exact 800 bps spread -> Liquid
    let edge_ok = OrderbookSnapshot::new(0.40, 50.0, 0.48, 50.0, 0.0)
        .compute_mark()
        .expect("exact floor boundary");
    assert_eq!(edge_ok.state, BookState::Liquid);
    assert!(edge_ok.phi_liq > 0.0);

    // Just below $100 TOB -> ThinOrWide
    let under_depth = OrderbookSnapshot::new(0.40, 49.99, 0.41, 50.0, 0.0)
        .compute_mark()
        .expect("under depth");
    assert_eq!(under_depth.state, BookState::ThinOrWide);

    // Just above 800 bps spread -> ThinOrWide
    let over_spread = OrderbookSnapshot::new(0.40, 500.0, 0.4801, 500.0, 0.0)
        .compute_mark()
        .expect("over spread");
    assert_eq!(over_spread.state, BookState::ThinOrWide);

    // Invalid crossed book and negative depth
    assert!(matches!(
        OrderbookSnapshot::new(0.60, 500.0, 0.50, 500.0, 0.01).compute_mark(),
        Err(MicrostructureError::InvalidPrices { .. })
    ));
    assert!(matches!(
        OrderbookSnapshot::new(0.40, -10.0, 0.45, 500.0, 0.01).compute_mark(),
        Err(MicrostructureError::InvalidSizes { .. })
    ));
}

#[test]
fn tier_w1_tick_archive_roundtrip_feeds_mark_estimator() {
    let tmp = TempDir::new().expect("tempdir");
    let archive = TickArchive::new(tmp.path());

    let ticks = vec![
        L2TickRecord {
            venue: "polymarket".into(),
            question_id: "q_hormuz".into(),
            timestamp_utc: "2026-11-25T18:00:00Z".into(),
            bid_price: 0.24,
            bid_size_usd: 1_500.0,
            ask_price: 0.25,
            ask_size_usd: 1_500.0,
            last_trade: Some(0.245),
            depth_impact_500_prob: 0.004,
            s_tick_bps: 100.0,
        },
        L2TickRecord {
            venue: "polymarket".into(),
            question_id: "q_hormuz".into(),
            timestamp_utc: "2026-11-25T18:05:00Z".into(),
            bid_price: 0.20,
            bid_size_usd: 30.0,
            ask_price: 0.35,
            ask_size_usd: 25.0,
            last_trade: Some(0.26),
            depth_impact_500_prob: 0.20,
            s_tick_bps: 100.0,
        },
    ];

    let paths = archive.write_ticks(&ticks).expect("write ticks");
    assert_eq!(paths.len(), 1);
    assert!(paths[0].ends_with("ticks/venue=polymarket/year=2026/month=11/ticks.jsonl.zst"));

    let key = PartitionKey::from_venue_and_timestamp("polymarket", "2026-11-25T18:00:00Z")
        .expect("partition key");
    let loaded = archive.read_partition(&key).expect("read partition");
    assert_eq!(loaded, ticks);

    let m0 = BookMarkPrice::from_snapshot(&OrderbookSnapshot {
        bid_price: loaded[0].bid_price,
        bid_size_usd: loaded[0].bid_size_usd,
        ask_price: loaded[0].ask_price,
        ask_size_usd: loaded[0].ask_size_usd,
        depth_impact_500_prob: loaded[0].depth_impact_500_prob,
        s_tick_bps: loaded[0].s_tick_bps,
        last_trade: loaded[0].last_trade,
    })
    .expect("mark 0");
    assert_eq!(m0.state, BookState::Liquid);
    assert!(m0.phi_liq > 0.80);

    let m1 = BookMarkPrice::from_snapshot(&OrderbookSnapshot {
        bid_price: loaded[1].bid_price,
        bid_size_usd: loaded[1].bid_size_usd,
        ask_price: loaded[1].ask_price,
        ask_size_usd: loaded[1].ask_size_usd,
        depth_impact_500_prob: loaded[1].depth_impact_500_prob,
        s_tick_bps: loaded[1].s_tick_bps,
        last_trade: loaded[1].last_trade,
    })
    .expect("mark 1");
    assert_eq!(m1.state, BookState::ThinOrWide);
    assert_eq!(m1.phi_liq, 0.0);
    assert!((m1.p_micro.get() - 0.26).abs() < 1e-9);
}

proptest! {
    #[test]
    fn prop_phi_liq_bounded_and_monotone_in_impact(
        bid in 0.05_f64..0.85,
        spread in 0.001_f64..0.075,
        bid_sz in 60.0_f64..20_000.0,
        ask_sz in 60.0_f64..20_000.0,
        impact_lo in 0.0_f64..0.05,
        impact_delta in 0.001_f64..0.20,
    ) {
        let ask = (bid + spread).min(0.99);
        let s1 = OrderbookSnapshot::new(bid, bid_sz, ask, ask_sz, impact_lo);
        let s2 = OrderbookSnapshot::new(bid, bid_sz, ask, ask_sz, impact_lo + impact_delta);
        let m1 = s1.compute_mark().expect("m1");
        let m2 = s2.compute_mark().expect("m2");
        prop_assert!(m1.phi_liq >= 0.0 && m1.phi_liq <= 1.0);
        prop_assert!(m2.phi_liq >= 0.0 && m2.phi_liq <= 1.0);
        prop_assert!(m2.phi_liq <= m1.phi_liq + 1e-12);
        prop_assert!(m1.p_micro.get() >= bid - 1e-6 && m1.p_micro.get() <= ask + 1e-6);
    }
}
