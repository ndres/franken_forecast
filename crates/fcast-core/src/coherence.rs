//! Cross-question coherence (`PAV`, Fréchet `Combo`), `MetaculusCdfBuilder` (`§12.5`),
//! multimodal probability-space CDF mixture pooling, Empirical-Bayes shrinkage, and `M_eff` (`§11.4`).

use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum CoherenceError {
    #[error("invalid cdf bounds: lower={0}, upper={1}")]
    InvalidBounds(f64, f64),
    #[error("empty percentile anchors")]
    EmptyPercentiles,
    #[error("cdf mixture lengths or weights invalid")]
    InvalidMixture,
}

/// Pure-Rust port of `Metaculus/forecasting-tools` `NumericDistribution` (`§12.5`).
#[allow(clippy::too_many_arguments)]
pub fn build_metaculus_cdf(
    percentiles: &[(f64, f64)],
    lower_bound: f64,
    upper_bound: f64,
    open_lower: bool,
    open_upper: bool,
    zero_point: Option<f64>,
    inbound_outcome_count: Option<usize>,
    min_step: f64,
    max_step: f64,
) -> Result<Vec<f64>, CoherenceError> {
    if percentiles.is_empty() {
        return Err(CoherenceError::EmptyPercentiles);
    }
    if !lower_bound.is_finite() || !upper_bound.is_finite() || upper_bound <= lower_bound {
        return Err(CoherenceError::InvalidBounds(lower_bound, upper_bound));
    }
    if !min_step.is_finite()
        || !max_step.is_finite()
        || min_step < 0.0
        || max_step <= 0.0
        || min_step > max_step
    {
        return Err(CoherenceError::InvalidBounds(min_step, max_step));
    }
    if let Some(z) = zero_point
        && (!z.is_finite() || z >= lower_bound)
    {
        return Err(CoherenceError::InvalidBounds(z, lower_bound));
    }
    if percentiles.iter().any(|&(v, p)| {
        !v.is_finite()
            || !p.is_finite()
            || !(0.0..=1.0).contains(&p)
            || zero_point.is_some_and(|z| v <= z)
    }) {
        return Err(CoherenceError::InvalidBounds(lower_bound, upper_bound));
    }
    let n = match inbound_outcome_count {
        Some(0) => return Err(CoherenceError::InvalidBounds(0.0, upper_bound)),
        Some(c) => c + 1,
        None => 201,
    };

    let to_unit = |x: f64| -> f64 {
        if let Some(z) = zero_point {
            ((x - z) / (lower_bound - z)).ln() / ((upper_bound - z) / (lower_bound - z)).ln()
        } else {
            (x - lower_bound) / (upper_bound - lower_bound)
        }
    };

    let mut pts: Vec<(f64, f64)> = percentiles.iter().map(|&(v, p)| (to_unit(v), p)).collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));

    let lo_val = if open_lower { pts[0].1.max(0.001) } else { 0.0 };
    let hi_val = if open_upper {
        pts[pts.len() - 1].1.min(0.999)
    } else {
        1.0
    };

    let mut knots = Vec::with_capacity(pts.len() + 2);
    knots.push((0.0, lo_val));
    for (x, p) in pts {
        if x > 0.0 && x < 1.0 {
            knots.push((x, p));
        }
    }
    knots.push((1.0, hi_val));

    let mut cdf = Vec::with_capacity(n);
    for i in 0..n {
        let t = (i as f64) / ((n - 1) as f64);
        let mut val = knots[knots.len() - 1].1;
        for win in knots.windows(2) {
            let (x0, y0) = win[0];
            let (x1, y1) = win[1];
            if t >= x0 && t <= x1 {
                let frac = if (x1 - x0).abs() < 1e-15 {
                    0.0
                } else {
                    (t - x0) / (x1 - x0)
                };
                val = y0 + frac * (y1 - y0);
                break;
            }
        }
        cdf.push(val);
    }

    cdf[0] = if open_lower { cdf[0].max(0.001) } else { 0.0 };
    let last_idx = n - 1;
    cdf[last_idx] = if open_upper {
        cdf[last_idx].min(0.999)
    } else {
        1.0
    };

    for i in 1..n {
        cdf[i] = cdf[i].max(cdf[i - 1] + min_step);
        cdf[i] = cdf[i].min(cdf[i - 1] + max_step);
    }

    let span_cur = cdf[last_idx] - cdf[0];
    let target_hi = if open_upper {
        cdf[last_idx].min(0.999)
    } else {
        1.0
    };
    let span_tgt = target_hi - cdf[0];
    if span_cur > 0.0 && (cdf[last_idx] - target_hi).abs() > 1e-12 {
        let c0 = cdf[0];
        for c in &mut cdf {
            *c = c0 + (*c - c0) * (span_tgt / span_cur);
        }
    }
    cdf[0] = if open_lower { cdf[0].max(0.001) } else { 0.0 };
    cdf[last_idx] = target_hi;
    Ok(cdf)
}

/// Probability-space multimodal CDF mixture pooling (`§11.4`).
pub fn mix_cdfs_probability_space(
    cdfs: &[Vec<f64>],
    weights: &[f64],
) -> Result<Vec<f64>, CoherenceError> {
    if cdfs.is_empty() || cdfs.len() != weights.len() {
        return Err(CoherenceError::InvalidMixture);
    }
    let len = cdfs[0].len();
    if len < 2 || cdfs.iter().any(|c| c.len() != len) {
        return Err(CoherenceError::InvalidMixture);
    }
    let w_sum: f64 = weights.iter().sum();
    if !w_sum.is_finite() || w_sum <= 0.0 || weights.iter().any(|w| !w.is_finite() || *w < 0.0) {
        return Err(CoherenceError::InvalidMixture);
    }
    let mut out = vec![0.0; len];
    for (cdf, &w) in cdfs.iter().zip(weights.iter()) {
        let wn = w / w_sum;
        for (o, &v) in out.iter_mut().zip(cdf.iter()) {
            *o += wn * v;
        }
    }
    Ok(out)
}

/// Pool-Adjacent-Violators (PAV) isotonic projection for temporal monotonicity (`T1 < T2 => P(T1) <= P(T2)`).
#[must_use]
pub fn pav_isotonic_nondecreasing(vals: &[f64]) -> Vec<f64> {
    let mut blocks: Vec<(f64, usize)> = vals.iter().map(|&v| (v, 1)).collect();
    let mut i = 0;
    while i + 1 < blocks.len() {
        if blocks[i].0 > blocks[i + 1].0 {
            let w = blocks[i].1 + blocks[i + 1].1;
            let mean = (blocks[i].0 * blocks[i].1 as f64
                + blocks[i + 1].0 * blocks[i + 1].1 as f64)
                / (w as f64);
            blocks[i] = (mean, w);
            blocks.remove(i + 1);
            i = i.saturating_sub(1);
        } else {
            i += 1;
        }
    }
    let mut out = Vec::with_capacity(vals.len());
    for (m, w) in blocks {
        for _ in 0..w {
            out.push(m);
        }
    }
    out
}

/// Signed Fréchet–Hoeffding bounds for a 2-leg `Combo` question (`dir_a, dir_b ∈ {+1, -1}`).
#[must_use]
pub fn frechet_combo_bounds(pa: f64, pb: f64, dir_a: i8, dir_b: i8) -> (f64, f64) {
    let ea = if dir_a >= 0 { pa } else { 1.0 - pa };
    let eb = if dir_b >= 0 { pb } else { 1.0 - pb };
    ((ea + eb - 1.0).max(0.0), ea.min(eb))
}

/// Empirical-Bayes hierarchical outside-view shrinkage (`§11.4`).
#[must_use]
pub fn eb_shrink_base_rate(k_hits: u32, n_obs: u32, mu_parent: f64, m_pseudo: f64) -> f64 {
    (f64::from(k_hits) + m_pseudo * mu_parent) / (f64::from(n_obs) + m_pseudo)
}

/// Correlation-adjusted effective independent model count `M_eff` (`§11.4`).
#[must_use]
pub fn effective_model_count(m_models: usize, mean_pair_corr: f64) -> f64 {
    let m = m_models as f64;
    m / (1.0 + (m - 1.0) * mean_pair_corr.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metaculus_cdf_linear_log_and_discrete() {
        let lin = build_metaculus_cdf(
            &[(20.0, 0.10), (50.0, 0.50), (80.0, 0.90)],
            0.0,
            100.0,
            false,
            false,
            None,
            None,
            5e-5,
            0.59,
        )
        .expect("lin");
        assert_eq!(lin.len(), 201);
        assert_eq!(lin[0], 0.0);
        assert_eq!(lin[200], 1.0);
        assert!(
            lin.windows(2)
                .all(|w| w[1] - w[0] >= 5e-5 - 1e-12 && w[1] - w[0] <= 0.59 + 1e-12)
        );

        let disc = build_metaculus_cdf(
            &[(3.0, 0.20), (8.0, 0.60), (12.0, 0.90)],
            0.0,
            15.0,
            false,
            true,
            None,
            Some(15),
            5e-5,
            0.59,
        )
        .expect("disc");
        assert_eq!(disc.len(), 16);

        // Adversarial inputs: invalid zero_point, zero outcome count, NaN percentiles
        assert!(
            build_metaculus_cdf(
                &[(50.0, 0.5)],
                0.0,
                100.0,
                false,
                false,
                Some(0.0),
                None,
                5e-5,
                0.59
            )
            .is_err()
        );
        assert!(
            build_metaculus_cdf(
                &[(50.0, 0.5)],
                0.0,
                100.0,
                false,
                false,
                None,
                Some(0),
                5e-5,
                0.59
            )
            .is_err()
        );
        assert!(
            build_metaculus_cdf(
                &[(f64::NAN, 0.5)],
                0.0,
                100.0,
                false,
                false,
                None,
                None,
                5e-5,
                0.59
            )
            .is_err()
        );
    }

    #[test]
    fn pav_frechet_and_eb_shrinkage() {
        let mono = pav_isotonic_nondecreasing(&[0.30, 0.24, 0.50, 0.48]);
        assert!(mono.windows(2).all(|w| w[0] <= w[1] + 1e-12));
        let (lo, hi) = frechet_combo_bounds(0.40, 0.50, 1, 1);
        assert!((lo - 0.0).abs() < 1e-12 && (hi - 0.40).abs() < 1e-12);
        let mu = eb_shrink_base_rate(0, 3, 0.20, 5.0);
        assert!(mu > 0.10 && mu < 0.15);
        assert!((effective_model_count(4, 0.5) - 1.6).abs() < 1e-12);
    }
}
