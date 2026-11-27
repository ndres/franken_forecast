//! Elicitation-measurement & Kalshi statistical inference tests in `fcast-bench`:
//! - Release-date clustered bootstrap 95% CI
//! - Exact `2^n` sign-flip randomization test (`n <= 16`) & Monte Carlo (`n > 16`)
//! - Counterfactual entity-redacted dose-response monotonicity probe

use fcast_bench::{
    CounterfactualDoseProbe, exact_sign_flip_p_value, release_date_clustered_bootstrap_ci,
};

#[test]
fn clustered_bootstrap_exact_sign_flip_and_counterfactual_dose_probe() {
    // 8 distinct macro release dates, 2 correlated series per release date (all negative = improvement)
    let items = vec![
        ("2026-03-12", -0.08),
        ("2026-03-12", -0.06),
        ("2026-04-10", -0.09),
        ("2026-04-10", -0.07),
        ("2026-05-14", -0.05),
        ("2026-05-14", -0.04),
        ("2026-06-11", -0.10),
        ("2026-06-11", -0.08),
        ("2026-07-10", -0.07),
        ("2026-07-10", -0.06),
        ("2026-08-13", -0.09),
        ("2026-08-13", -0.08),
        ("2026-09-10", -0.05),
        ("2026-09-10", -0.06),
        ("2026-10-15", -0.11),
        ("2026-10-15", -0.09),
    ];

    let ci_a = release_date_clustered_bootstrap_ci(&items, 2_000, 42).expect("ci a");
    let ci_b = release_date_clustered_bootstrap_ci(&items, 2_000, 42).expect("ci b");
    assert_eq!(ci_a, ci_b, "seeded bootstrap must be deterministic");
    assert_eq!(ci_a.n_clusters, 8);
    assert!(ci_a.ci_lo_95 <= ci_a.mean && ci_a.mean <= ci_a.ci_hi_95);
    assert!(
        ci_a.ci_hi_95 < 0.0,
        "entire 95% CI should confirm improvement"
    );

    // Exact 2^8 = 256 sign-flip test: all 8 negative clusters => p = 2 / 256 = 0.0078125
    let cluster_deltas = [-0.07, -0.08, -0.045, -0.09, -0.065, -0.085, -0.055, -0.10];
    let p_exact = exact_sign_flip_p_value(&cluster_deltas).expect("exact p");
    assert!((p_exact - (2.0 / 256.0)).abs() < 1e-12);

    // Symmetric zero-sum null => p = 1.0
    let p_null = exact_sign_flip_p_value(&[-0.05, 0.05, -0.05, 0.05]).expect("null p");
    assert!((p_null - 1.0).abs() < 1e-12);

    // Monte Carlo branch for n > 16
    let large_neg = vec![-0.05; 20];
    let p_mc = exact_sign_flip_p_value(&large_neg).expect("mc p");
    assert!(p_mc < 0.005);

    // Counterfactual entity redaction & dose monotonicity probe
    let redacted = CounterfactualDoseProbe::redact_entity_names(
        "Under the American Rescue Plan Act (FPUC $300/wk), UI outlays rose.",
        &["American Rescue Plan Act", "FPUC"],
    );
    assert!(!redacted.contains("American Rescue Plan Act"));
    assert!(!redacted.contains("FPUC"));
    assert!(redacted.contains("[ENTITY_1]"));
    assert!(redacted.contains("[ENTITY_2]"));

    // Memorized flat output & duplicate-dose inputs fail; genuine counterfactual response passes
    assert!(!CounterfactualDoseProbe::is_dose_monotone(
        &[(0.0, 210.0), (150.0, 210.0), (300.0, 210.0), (600.0, 210.0)],
        true
    ));
    assert!(!CounterfactualDoseProbe::is_dose_monotone(
        &[(1.0, 0.2), (1.0, 0.8)],
        true
    ));
    assert!(CounterfactualDoseProbe::is_dose_monotone(
        &[(0.0, 120.0), (150.0, 165.0), (300.0, 210.0), (600.0, 300.0)],
        true
    ));

    // Longest-first case-insensitive redaction prevents prefix corruption ("Fed" vs "Federal Reserve")
    assert_eq!(
        CounterfactualDoseProbe::redact_entity_names(
            "The FEDERAL RESERVE cut rates",
            &["Fed", "Federal Reserve"]
        ),
        "The [ENTITY_2] cut rates"
    );

    // Small-scale sign-flip test must not collapse to p=1.0; MC p-value > 0; NaN sigma rejected
    assert!(exact_sign_flip_p_value(&[1e-13; 10]).expect("small") < 0.01);
    assert!(exact_sign_flip_p_value(&[1.0; 20]).expect("mc") > 0.0);
    assert_eq!(
        fcast_bench::paired_normalized_crps_delta(2.0, f64::NAN, 2.0, &[1.0, 2.0, 4.0]),
        None
    );
}
