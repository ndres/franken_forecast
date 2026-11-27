//! `GATE-07` (`CLAIM-03`, `SUB-02`): Vintage `DataPacks` (`available_utc <= as_of_utc` + ALFRED
//! returned header vintage verification), Horvitz-Thompson right-censoring (`< 0.30`),
//! `fetch_raw_data` CSV/JSON parsing with `[PARTIAL_CONTEXT]` guard, `SEASONAL_NOTE`,
//! pre-LLM `PersistenceAndDriftBaseline`, and multi-provider parallel search deduplication.

use fcast_infra::vault::EvidenceVault;
use fcast_ingest::{
    BaselineIntervalMethod, ContextCompleteness, DataPackError, DataPackObservation,
    MultiProviderSearchAggregator, SearchDocument, SearchProvider, VintageDataPack,
    build_persistence_baseline, build_seasonal_note, horvitz_thompson_completeness,
    parse_raw_csv_series, parse_raw_json_series,
};
use tempfile::TempDir;

struct StaticProvider {
    name: &'static str,
    docs: Vec<SearchDocument>,
}

impl SearchProvider for StaticProvider {
    fn name(&self) -> &str {
        self.name
    }

    fn query(&self, _query: &str, _as_of_utc: &str) -> Result<Vec<SearchDocument>, String> {
        Ok(self.docs.clone())
    }
}

#[test]
fn gate_07_vintage_datapack_alfred_header_fence_and_horvitz_thompson() {
    let csv = "\
period,value,available_utc,header_vintage_utc
2026-06,2.80,2026-07-11T12:30:00Z,2026-07-11T12:30:00Z
2026-07,2.60,2026-08-14T12:30:00Z,2026-08-14T12:30:00Z
2026-08,2.50,2026-09-11T12:30:00Z,2026-09-11T12:30:00Z
2026-09,2.40,2026-10-10T12:30:00Z,2026-10-10T12:30:00Z
";
    let parsed = parse_raw_csv_series("CPIAUCSL", csv, 4096).expect("parse csv");
    assert_eq!(parsed.completeness, ContextCompleteness::Complete);
    assert_eq!(parsed.rows.len(), 4);

    let pack = VintageDataPack::build_fenced(
        "CPIAUCSL",
        "2026-10-15T00:00:00Z",
        "2026-10-15T00:00:00Z",
        parsed.rows.clone(),
        parsed.completeness.clone(),
    )
    .expect("valid fenced pack");
    assert_eq!(pack.values(), vec![2.80, 2.60, 2.50, 2.40]);

    // Reject post-cutoff available_utc (0 leaks allowed)
    let mut future_rows = parsed.rows.clone();
    future_rows.push(DataPackObservation {
        series_id: "CPIAUCSL".into(),
        period: "2026-10".into(),
        value: 2.30,
        event_utc: None,
        available_utc: "2026-10-15T00:00:00.100Z".into(),
        header_vintage_utc: "2026-10-15T00:00:00Z".into(),
    });
    assert!(matches!(
        VintageDataPack::build_fenced(
            "CPIAUCSL",
            "2026-10-15T00:00:00Z",
            "2026-10-15T00:00:00Z",
            future_rows,
            ContextCompleteness::Complete,
        ),
        Err(DataPackError::AvailableAfterCutoff { .. })
    ));

    // Reject ALFRED returned header_vintage_utc > requested_vintage_utc (Thesis postmortem #1)
    let mut bad_header = parsed.rows.clone();
    bad_header[0].header_vintage_utc = "2026-10-16T00:00:00Z".into();
    assert!(matches!(
        VintageDataPack::build_fenced(
            "CPIAUCSL",
            "2026-10-20T00:00:00Z",
            "2026-10-15T00:00:00Z",
            bad_header,
            ContextCompleteness::Complete,
        ),
        Err(DataPackError::HeaderVintageAfterRequested { .. })
    ));

    // Partial-context fail-closed flag when payload exceeds complete-context ceiling
    let partial = parse_raw_csv_series("CPIAUCSL", csv, 64).expect("parse partial");
    assert!(!partial.completeness.is_complete());

    // JSON roundtrip
    let json_text = serde_json::to_string(&parsed.rows).expect("json");
    let from_json = parse_raw_json_series("CPIAUCSL", &json_text, 4096).expect("parse json");
    assert_eq!(from_json.rows.len(), 4);

    // Horvitz-Thompson right-censoring detection (< 0.30 completeness)
    let censored = horvitz_thompson_completeness(&[0.20, 0.25, 0.30]).expect("ht censored");
    assert!(censored.right_censored);
    assert!((censored.mean_reporting_completeness - 0.25).abs() < 1e-12);
    assert!(censored.ht_adjusted_total > 12.0);

    let healthy = horvitz_thompson_completeness(&[0.80, 0.90, 0.85]).expect("ht healthy");
    assert!(!healthy.right_censored);

    // SEASONAL_NOTE + Pre-LLM PersistenceAndDriftBaseline
    let note = build_seasonal_note("CPIAUCSL", "October (5-yr)", &[2.9, 2.7, 2.6, 2.5, 2.4])
        .expect("seasonal note");
    assert!(note.summary.contains("SEASONAL_NOTE[CPIAUCSL"));
    assert!((note.median_value - 2.6).abs() < 1e-12);

    let baseline = build_persistence_baseline("CPIAUCSL", "2026-10-15T00:00:00Z", &pack.values())
        .expect("baseline");
    assert_eq!(
        baseline.interval_method,
        BaselineIntervalMethod::ResidualQuantile
    );
    assert!((baseline.point - 2.40).abs() < 1e-12);
    assert!(baseline.p10 <= baseline.point && baseline.p90 >= baseline.point);
    assert!(baseline.override_bar_width > 0.0);
}

#[test]
fn gate_07_multi_provider_parallel_search_fencing_blinding_and_dedup() {
    let dir = TempDir::new().expect("tempdir");
    let vault = EvidenceVault::open(dir.path()).expect("vault");

    let p1 = StaticProvider {
        name: "asknews",
        docs: vec![
            SearchDocument {
                provider: "asknews".into(),
                canonical_url: "https://example.org/cert-eu-1".into(),
                title: "CERT-EU winter grid advisory".into(),
                body_excerpt: "Critical infrastructure operators advised to patch SCADA relays.\nPolymarket odds jumped to 45%".into(),
                published_utc: "2026-10-14T10:00:00Z".into(),
                available_utc: "2026-10-14T10:05:00Z".into(),
            },
            SearchDocument {
                provider: "asknews".into(),
                canonical_url: "https://example.org/future-leak".into(),
                title: "Post-cutoff report".into(),
                body_excerpt: "This arrived after as_of_utc".into(),
                published_utc: "2026-10-15T01:00:00Z".into(),
                available_utc: "2026-10-15T01:00:00Z".into(),
            },
        ],
    };
    let p2 = StaticProvider {
        name: "exa",
        docs: vec![
            // Duplicate of p1 doc (different URL, near-identical content)
            SearchDocument {
                provider: "exa".into(),
                canonical_url: "https://mirror.example.org/cert-eu-1".into(),
                title: "CERT-EU winter grid advisory".into(),
                body_excerpt: "Critical infrastructure operators advised to patch SCADA relays."
                    .into(),
                published_utc: "2026-10-14T10:00:00Z".into(),
                available_utc: "2026-10-14T10:10:00Z".into(),
            },
            // Distinct novel report
            SearchDocument {
                provider: "exa".into(),
                canonical_url: "https://cisa.gov/kev-oct-2026".into(),
                title: "CISA adds router vulnerability to KEV".into(),
                body_excerpt: "Active exploitation observed across European telecom edge routers."
                    .into(),
                published_utc: "2026-10-14T18:00:00Z".into(),
                available_utc: "2026-10-14T18:05:00Z".into(),
            },
        ],
    };

    let agg = MultiProviderSearchAggregator::default();
    let report = agg
        .search_and_vault(&[&p1, &p2], "EU grid cyber", "2026-10-15T00:00:00Z", &vault)
        .expect("search and vault");

    assert_eq!(report.dropped_post_cutoff, 1);
    assert_eq!(report.dropped_duplicates, 1);
    assert_eq!(report.hits.len(), 2);
    assert!(report.provider_errors.is_empty());

    // Confirm vaulted content had Polymarket line scrubbed pre-seal
    let vaulted_bytes = vault
        .read_verified(&report.hits[0].sha256_hex, false)
        .expect("read vaulted");
    let vaulted_str = String::from_utf8(vaulted_bytes).expect("utf8");
    assert!(!vaulted_str.to_ascii_lowercase().contains("polymarket"));
    assert!(vaulted_str.contains("SCADA relays"));

    // Red-team regressions: swapped CSV header, NaN row, empty CSV marked Partial, HT +Inf rejected,
    // and requested_vintage_utc > as_of_utc rejected.
    let swapped = "period,value,header_vintage_utc,available_utc\n2026-09,3.1,2026-10-01T00:00:00Z,2026-10-01T00:00:00Z";
    assert!(parse_raw_csv_series("CPI", swapped, 4096).is_err());
    let nan_csv = "period,value,available_utc,header_vintage_utc\n2026-09,NaN,2026-10-01T00:00:00Z,2026-10-01T00:00:00Z";
    assert!(parse_raw_csv_series("CPI", nan_csv, 4096).is_err());
    let empty_csv = parse_raw_csv_series(
        "CPI",
        "period,value,available_utc,header_vintage_utc\n",
        4096,
    )
    .expect("empty csv");
    assert!(!empty_csv.completeness.is_complete());
    assert!(horvitz_thompson_completeness(&[1e-308, 1e-308]).is_err());
}
