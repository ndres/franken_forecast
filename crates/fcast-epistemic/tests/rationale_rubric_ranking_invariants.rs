//! `GATE-17` integration test: `10,000` `proptest` cases verifying `unscoreable`,
//! `G2 -> 0`, `G1 -> min(sum, 8)`, and exact deterministic tie-breaking (`INV-06`).

use fcast_epistemic::{
    RankTier, RationaleError, RationaleRecord, rank_rationales, score_rationale,
};
use proptest::prelude::*;

fn arb_record(idx: usize) -> impl Strategy<Value = RationaleRecord> {
    (
        any::<bool>(),
        0_u8..=4,
        0_u8..=4,
        0_u8..=4,
        0_u8..=4,
        0_u8..=4,
        0_u8..=4,
        any::<bool>(),
        any::<bool>(),
        0_u32..=50,
    )
        .prop_map(move |(unscoreable, ct, ei, ov, me, ej, pr, g1, g2, sec)| {
            let created_at_utc = format!("2026-11-18T12:00:{sec:02}Z");
            if unscoreable {
                RationaleRecord {
                    id: format!("r_{idx}"),
                    ct: None,
                    ei: None,
                    ov: None,
                    me: None,
                    ej: None,
                    pr: None,
                    g1: false,
                    g2: false,
                    unscoreable: true,
                    note: "Empty or redacted rationale.".to_owned(),
                    created_at_utc,
                }
            } else {
                RationaleRecord {
                    id: format!("r_{idx}"),
                    ct: Some(ct),
                    ei: Some(ei),
                    ov: Some(ov),
                    me: Some(me),
                    ej: Some(ej),
                    pr: Some(pr),
                    g1,
                    g2,
                    unscoreable: false,
                    note: "Gate justification text meeting minimum length.".to_owned(),
                    created_at_utc,
                }
            }
        })
}

fn arb_cohort() -> impl Strategy<Value = Vec<RationaleRecord>> {
    (1_usize..=10).prop_flat_map(|n| (0..n).map(arb_record).collect::<Vec<_>>())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn rubric_scoring_and_ranking_invariants_hold_over_10k_cases(cohort in arb_cohort()) {
        let expected_scoreable = cohort.iter().filter(|r| !r.unscoreable).count();
        let ranked = rank_rationales(&cohort).expect("valid cohort must score and rank");
        prop_assert_eq!(ranked.len(), expected_scoreable);

        for rec in &cohort {
            let scored = score_rationale(rec).expect("valid record");
            if rec.unscoreable {
                prop_assert!(scored.is_none());
                continue;
            }
            let s = scored.expect("scoreable record");
            let raw = s.ct + s.ei + s.ov + s.me + s.ej + s.pr;
            prop_assert_eq!(s.raw_sum, raw);
            if rec.g2 {
                prop_assert_eq!(s.tier, RankTier::DisqualifiedG2);
                prop_assert_eq!(s.effective_score, 0);
            } else if rec.g1 {
                prop_assert_eq!(s.tier, RankTier::CappedG1);
                prop_assert_eq!(s.effective_score, raw.min(8));
            } else {
                prop_assert_eq!(s.tier, RankTier::Clean);
                prop_assert_eq!(s.effective_score, raw);
                prop_assert!(s.effective_score <= 24);
            }
        }

        // Verify descending order on exact tie-break tuple (tier, eff, min(ct, me), ei, ov, ej, pr, Reverse(created_at_utc)).
        for win in ranked.windows(2) {
            let a = &win[0];
            let b = &win[1];
            prop_assert!(
                a.rank_key() >= b.rank_key(),
                "ordering violated: {:?} before {:?}",
                a,
                b
            );
            if a.rank_key() == b.rank_key() {
                prop_assert!(a.id <= b.id);
            }
        }

        // Reversed input order must yield identical deterministic output order.
        let mut reversed = cohort;
        reversed.reverse();
        let ranked_rev = rank_rationales(&reversed).expect("reversed cohort");
        prop_assert_eq!(ranked, ranked_rev);
    }
}

#[test]
fn rejects_invalid_gate_notes_and_malformed_unscoreable_records() {
    let missing_note = RationaleRecord {
        id: "bad_g1".to_owned(),
        ct: Some(3),
        ei: Some(3),
        ov: Some(3),
        me: Some(3),
        ej: Some(3),
        pr: Some(3),
        g1: true,
        g2: false,
        unscoreable: false,
        note: "too short".to_owned(),
        created_at_utc: "2026-11-18T00:00:00Z".to_owned(),
    };
    assert_eq!(
        score_rationale(&missing_note),
        Err(RationaleError::MissingGateJustification { gate: "g1" })
    );

    let bad_unscoreable = RationaleRecord {
        id: "bad_u".to_owned(),
        ct: Some(1),
        ei: None,
        ov: None,
        me: None,
        ej: None,
        pr: None,
        g1: false,
        g2: false,
        unscoreable: true,
        note: String::new(),
        created_at_utc: "2026-11-18T00:00:00Z".to_owned(),
    };
    assert_eq!(
        score_rationale(&bad_unscoreable),
        Err(RationaleError::InvalidUnscoreablePayload)
    );

    let out_of_range = RationaleRecord {
        id: "bad_dim".to_owned(),
        ct: Some(5),
        ei: Some(2),
        ov: Some(2),
        me: Some(2),
        ej: Some(2),
        pr: Some(2),
        g1: false,
        g2: false,
        unscoreable: false,
        note: String::new(),
        created_at_utc: "2026-11-18T00:00:00Z".to_owned(),
    };
    assert_eq!(
        score_rationale(&out_of_range),
        Err(RationaleError::DimensionOutOfRange {
            dim: "ct",
            value: 5
        })
    );
}
