//! `GATE-12` integration tests: 300-question deterministic Geo/Macro/Cyber `DomainGate`
//! benchmark (`precision >= 0.92`, `recall >= 0.90`) and cross-venue `EquivalenceCluster` matcher.

use fcast_ingest::{
    ClusterMatcher, DomainCandidate, DomainDecision, DomainGate, MarketContractRef, RejectReason,
    TargetDomain,
};

fn build_300_question_benchmark() -> Vec<(DomainCandidate, bool)> {
    let mut dataset = Vec::with_capacity(300);

    // 50 Geopolitics (in-scope = true)
    let geo_regions = [
        "Taiwan Strait",
        "Strait of Hormuz",
        "Black Sea",
        "Baltic",
        "Red Sea",
        "Ukraine",
        "South China Sea",
        "Gaza",
        "Lebanon",
        "Arctic",
    ];
    let geo_events = [
        "ceasefire agreement signed before Q4 2026",
        "NATO defense pact invocation or border clash",
        "new multilateral sanctions and maritime embargo",
        "ballistic missile test or military incursion",
        "parliamentary election and coalition government shift",
    ];
    let mut idx = 0usize;
    for region in geo_regions {
        for event in geo_events {
            idx += 1;
            dataset.push((
                DomainCandidate::new(
                    format!("geo_{idx:03}"),
                    format!("Will {region} see a {event}?"),
                    "polymarket",
                ),
                true,
            ));
        }
    }

    // 50 Macroeconomics (in-scope = true)
    let macro_entities = [
        "Federal Reserve",
        "ECB",
        "Bank of Japan",
        "Bank of England",
        "PBOC",
        "US Treasury",
        "IMF",
        "OPEC",
        "Eurozone",
        "US",
    ];
    let macro_events = [
        "announce a 25bps interest rate cut at the next FOMC or central bank meeting",
        "report core CPI inflation above 3.0% YoY",
        "enter technical recession with two quarters of negative GDP",
        "see 10-year yield exceed 5.0% amid sovereign debt deficit",
        "impose a new bilateral tariff escalating the trade war",
    ];
    idx = 0;
    for entity in macro_entities {
        for event in macro_events {
            idx += 1;
            dataset.push((
                DomainCandidate::new(
                    format!("macro_{idx:03}"),
                    format!("Will {entity} {event}?"),
                    "kalshi",
                ),
                true,
            ));
        }
    }

    // 50 Cybersecurity (in-scope = true)
    let cyber_actors = [
        "Volt Typhoon",
        "Salt Typhoon",
        "APT28",
        "APT29",
        "Lazarus Group",
        "Sandworm",
        "CISA KEV",
        "Cloudflare Radar",
        "IODA",
        "EuRepoC",
    ];
    let cyber_events = [
        "attributed ransomware attack against critical infrastructure power grid",
        "exploited zero-day CVE vulnerability added to CISA KEV",
        "major BGP hijack or national internet shutdown detected",
        "supply chain attack and malware wiper targeting telecom intrusion",
        "large-scale DDoS or undersea cable sabotage incident",
    ];
    idx = 0;
    for actor in cyber_actors {
        for event in cyber_events {
            idx += 1;
            dataset.push((
                DomainCandidate::new(
                    format!("cyber_{idx:03}"),
                    format!("Will {actor} trigger an {event}?"),
                    "metaculus",
                ),
                true,
            ));
        }
    }

    // 50 Sports (in-scope = false)
    let sports_leagues = [
        "NBA Finals",
        "Super Bowl",
        "Premier League",
        "Champions League",
        "Wimbledon",
        "Formula 1",
        "UFC",
        "MLB World Series",
        "NHL Stanley Cup",
        "FIFA World Cup",
    ];
    let sports_outcomes = [
        "winner decided in overtime playoffs",
        "top scorer wins MVP award this season",
        "heavyweight bout or championship match ends in upset",
        "team wins the grand prix or tournament title",
        "quarterback throws 4 touchdowns in the football game",
    ];
    idx = 0;
    for league in sports_leagues {
        for outcome in sports_outcomes {
            idx += 1;
            dataset.push((
                DomainCandidate::new(
                    format!("sports_{idx:03}"),
                    format!("{league}: Will the {outcome}?"),
                    "polymarket",
                ),
                false,
            ));
        }
    }

    // 50 Pop Culture (in-scope = false)
    let pop_subjects = [
        "Oscars",
        "Grammys",
        "Taylor Swift",
        "Netflix top",
        "Billboard Hot 100",
        "Eurovision",
        "Met Gala",
        "Coachella",
        "Rotten Tomatoes",
        "Reality TV",
    ];
    let pop_outcomes = [
        "album release breaks Spotify streams record",
        "movie sequel tops weekend box office",
        "wins Academy Award or Emmy for best drama",
        "celebrity wedding trends as TikTok viral clip",
        "pop star announces superstar tour at fashion week",
    ];
    idx = 0;
    for subj in pop_subjects {
        for outcome in pop_outcomes {
            idx += 1;
            dataset.push((
                DomainCandidate::new(
                    format!("pop_{idx:03}"),
                    format!("{subj}: Will the {outcome}?"),
                    "manifold",
                ),
                false,
            ));
        }
    }

    // 50 Sub-Hour Crypto (in-scope = false)
    let assets = [
        "BTC", "ETH", "SOL", "DOGE", "XRP", "Bitcoin", "Ethereum", "Solana", "BNB", "AVAX",
    ];
    let horizons = [
        "close up or down in the next 5 minutes",
        "trade higher at the 15-minute candle",
        "finish green in the 15m sub-hour window",
        "beat strike at the 30-minute hourly close",
        "spike in the next 15 minutes on 5m chart",
    ];
    idx = 0;
    for asset in assets {
        for horizon in horizons {
            idx += 1;
            dataset.push((
                DomainCandidate::new(
                    format!("crypto_{idx:03}"),
                    format!("Will {asset} {horizon}?"),
                    "polymarket",
                ),
                false,
            ));
        }
    }

    assert_eq!(dataset.len(), 300);
    dataset
}

#[test]
fn gate12_300_question_benchmark_precision_and_recall() {
    let gate = DomainGate;
    let dataset = build_300_question_benchmark();
    let metrics = gate.evaluate_benchmark(&dataset);

    assert_eq!(metrics.tp + metrics.fp + metrics.tn + metrics.fn_count, 300);
    assert!(
        metrics.precision >= 0.92,
        "precision {:.4} < 0.92 (tp={}, fp={})",
        metrics.precision,
        metrics.tp,
        metrics.fp
    );
    assert!(
        metrics.recall >= 0.90,
        "recall {:.4} < 0.90 (tp={}, fn={})",
        metrics.recall,
        metrics.tp,
        metrics.fn_count
    );
}

#[test]
fn gate12_nuanced_cyber_crypto_vs_sub_hour_crypto() {
    let gate = DomainGate;

    // Nation-state cyber heist involving crypto is Cybersecurity, NOT SubHourCrypto
    let heist = DomainCandidate::new(
        "q_lazarus",
        "Will North Korea Lazarus Group execute a >$100M crypto exchange supply chain attack in 2026?",
        "metaculus",
    );
    match gate.classify(&heist) {
        DomainDecision::Accept { domain, .. } => assert_eq!(domain, TargetDomain::Cybersecurity),
        other => panic!("expected Cyber accept, got {other:?}"),
    }

    // 15-minute BTC price flip is rejected as SubHourCrypto
    let flip = DomainCandidate::new(
        "q_btc_15m",
        "Will BTC finish higher in the next 15 minutes?",
        "polymarket",
    );
    assert!(matches!(
        gate.classify(&flip),
        DomainDecision::Reject {
            reason: RejectReason::SubHourCrypto,
            ..
        }
    ));
}

#[test]
fn gate12_cross_venue_equivalence_cluster_alignment() {
    let matcher = ClusterMatcher::default();
    let contracts = vec![
        MarketContractRef {
            venue: "polymarket".into(),
            question_id: "pm_fed_dec".into(),
            title: "Will the Federal Reserve cut interest rates by 25 bps in December 2026?".into(),
            resolution_date_utc: Some("2026-12-18T19:00:00Z".into()),
            domain: TargetDomain::Macroeconomics,
        },
        MarketContractRef {
            venue: "kalshi".into(),
            question_id: "kx_fomc_dec".into(),
            title: "FOMC cuts interest rate 25 bps December 2026".into(),
            resolution_date_utc: Some("2026-12-16T19:00:00Z".into()),
            domain: TargetDomain::Macroeconomics,
        },
        MarketContractRef {
            venue: "metaculus".into(),
            question_id: "mc_fed_2027".into(),
            title: "Will the Federal Reserve cut interest rates by 25 bps in June 2027?".into(),
            resolution_date_utc: Some("2027-06-18T19:00:00Z".into()),
            domain: TargetDomain::Macroeconomics,
        },
        MarketContractRef {
            venue: "polymarket".into(),
            question_id: "pm_baltic".into(),
            title: "Will a subsea cable sabotage outage hit the Baltic Sea in Q4 2026?".into(),
            resolution_date_utc: Some("2026-12-31T23:59:59Z".into()),
            domain: TargetDomain::Cybersecurity,
        },
    ];

    let clusters = matcher.cluster(&contracts);
    // pm_fed_dec + kx_fomc_dec cluster together; mc_fed_2027 is separated by date window; pm_baltic is separate domain
    assert_eq!(clusters.len(), 3);
    let fed_dec_cluster = clusters
        .iter()
        .find(|c| c.members.len() == 2)
        .expect("2-member cross-venue cluster");
    assert_eq!(fed_dec_cluster.domain, TargetDomain::Macroeconomics);
    assert!(fed_dec_cluster.min_pair_similarity >= matcher.min_jaccard);
    let venues: Vec<&str> = fed_dec_cluster
        .members
        .iter()
        .map(|m| m.venue.as_str())
        .collect();
    assert_eq!(venues, vec!["kalshi", "polymarket"]);
}
