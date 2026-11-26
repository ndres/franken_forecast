//! Two-Speed `(μ_t, N_t)` belief engine, `DecayController`, Dirichlet–Categorical filter,
//! Fréchet-feasible Martingale `VoI_log` stopping (`AUDIT-FLAG-05`), King-Zeng prior shift,
//! 1D Latent-Logit Kalman blending + Bayesian Surprise, and gated Platt/extremizing (`§11`, `§12.7`).

use crate::PROB_EPS;
use crate::forecast::{HorizonGeometry, ProbClamped, entropy_bern, kl_bern, logit, sigmoid};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainKind {
    Cyber,
    Geopolitics,
    Macro,
}

impl DomainKind {
    #[must_use]
    pub const fn decay_bounds(self) -> (f64, f64, f64) {
        match self {
            Self::Cyber => (0.15, 24.0, 168.0),
            Self::Geopolitics => (0.20, 72.0, 504.0),
            Self::Macro => (0.35, 336.0, 2160.0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Up,
    Down,
    Still,
}

#[must_use]
pub fn effective_half_life_hours(
    domain: DomainKind,
    h_rem_hours: f64,
    surprise_ewma: f64,
    lam_vol: f64,
) -> f64 {
    let (beta_d, tau_min, tau_max) = domain.decay_bounds();
    let base = (beta_d * h_rem_hours.max(6.0)).clamp(tau_min, tau_max);
    base / (1.0 + lam_vol * (surprise_ewma - 1.0).max(0.0))
}

#[must_use]
pub fn anchor_mean(mu0: f64, geometry: HorizonGeometry, t_rem_h: f64, t_tot_h: f64) -> f64 {
    match geometry {
        HorizonGeometry::CumulativeStoppingTime => {
            let frac = (t_rem_h / t_tot_h.max(1.0)).clamp(1e-4, 1.0);
            1.0 - (1.0 - mu0).powf(frac)
        }
        HorizonGeometry::PointInTime | HorizonGeometry::RollingWindow => mu0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ConjugateBelief {
    pub alpha: f64,
    pub beta: f64,
}

impl ConjugateBelief {
    #[must_use]
    pub fn from_mean_precision(mu: ProbClamped, n_eff: f64) -> Self {
        let n = n_eff.max(PROB_EPS);
        Self {
            alpha: n * mu.get(),
            beta: n * (1.0 - mu.get()),
        }
    }

    #[must_use]
    pub fn n_eff(self) -> f64 {
        self.alpha + self.beta
    }

    #[must_use]
    pub fn mean(self) -> f64 {
        self.alpha / self.n_eff().max(PROB_EPS)
    }

    #[must_use]
    pub fn variance(self) -> f64 {
        let mu = self.mean();
        (mu * (1.0 - mu)) / (self.n_eff() + 1.0)
    }

    /// Two-Speed Tier-0 mean–precision step (`HYP-05A..05C`, `§11.1`).
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn stream_step(
        self,
        mu0: f64,
        n0: f64,
        domain: DomainKind,
        geometry: HorizonGeometry,
        dt_hours: f64,
        t_rem_h: f64,
        t_tot_h: f64,
        surprise_ewma: f64,
        direction: Direction,
        delta_ll: f64,
        diagnosticity: f64,
        novelty: f64,
        kappa_lr: f64,
        eta_shock: f64,
    ) -> Self {
        let tau_eff = effective_half_life_hours(domain, t_rem_h, surprise_ewma, 0.8);
        let gamma = 2.0_f64.powf(-dt_hours / tau_eff);
        let mu_anc = anchor_mean(mu0, geometry, t_rem_h, t_tot_h);
        let a_pre = gamma * self.alpha + (1.0 - gamma) * n0 * mu_anc;
        let b_pre = gamma * self.beta + (1.0 - gamma) * n0 * (1.0 - mu_anc);
        let n_pre = a_pre + b_pre;
        let mu_pre = a_pre / n_pre;

        let (dl, diag) = match direction {
            Direction::Still => (0.0, 0.0),
            Direction::Up | Direction::Down => (delta_ll, diagnosticity),
        };
        let w_t = novelty * diag;
        let mu_post = sigmoid(logit(mu_pre) + kappa_lr * novelty * dl);
        let n_post = (n_pre / (1.0 + eta_shock * dl * dl)) + w_t;
        Self {
            alpha: n_post * mu_post,
            beta: n_post * (1.0 - mu_post),
        }
    }
}

/// Dirichlet–Categorical filter for `MultipleChoiceQuestion` (`§11.4`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DirichletCategoricalBelief {
    pub probs: Vec<f64>,
    pub n_eff: f64,
}

impl DirichletCategoricalBelief {
    #[must_use]
    pub fn step(&self, delta_lls: &[f64], novelty: f64, kappa_lr: f64, still: bool) -> Self {
        if still || delta_lls.len() != self.probs.len() {
            return self.clone();
        }
        let logs: Vec<f64> = self
            .probs
            .iter()
            .zip(delta_lls.iter())
            .map(|(p, dl)| p.max(1e-12).ln() + kappa_lr * novelty * dl)
            .collect();
        let max_log = logs.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let exps: Vec<f64> = logs.iter().map(|x| (x - max_log).exp()).collect();
        let sum: f64 = exps.iter().sum();
        let probs = exps.into_iter().map(|e| e / sum).collect();
        Self {
            probs,
            n_eff: self.n_eff + novelty.max(0.0),
        }
    }
}

/// Candidate crux for Martingale `VoI_log` stopping (`HYP-05E`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CruxCandidate {
    pub q_raw: f64,
    pub p_pos_raw: f64,
    pub r_hit: f64,
    pub cost_usd: f64,
    pub attempts: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopDecision {
    IneligibleMin2Folds,
    ContinueUnprobedFatTail,
    ContinueHighVoi,
    SealBelief,
}

/// `AUDIT-FLAG-05` (`lean_experiments/Claim05Spec.lean`): Fréchet-feasible Martingale projection
/// using `2ε` interior buffer so `p_neg ∈ [ε, 1-ε]` AND `q * p_pos + (1-q) * p_neg == p_k`.
#[must_use]
pub fn project_martingale_crux(p_k: f64, q_raw: f64, p_pos_raw: f64) -> (f64, f64, f64) {
    let eps = PROB_EPS;
    let eps_m = 2.0 * eps;
    let p_k = p_k.clamp(eps_m, 1.0 - eps_m);
    let p_pos = p_pos_raw.clamp(eps_m, 1.0 - eps_m);
    let q_max = if p_pos > p_k {
        (p_k - eps_m) / (p_pos - eps_m)
    } else if p_pos < p_k {
        (1.0 - eps_m - p_k) / (1.0 - eps_m - p_pos)
    } else {
        1.0 - eps_m
    };
    let q_j = q_raw.clamp(eps, (1.0 - eps).min(q_max));
    let p_neg = (p_k - q_j * p_pos) / (1.0 - q_j);
    (q_j, p_pos, p_neg)
}

#[must_use]
pub fn should_stop_fold(
    fold_count: u32,
    productive_folds: u32,
    p_k: f64,
    hyps: &[(f64, f64)],
    cruxes: &[CruxCandidate],
    lam_nat_per_usd: f64,
    kappa_tail: f64,
) -> (StopDecision, f64) {
    if fold_count < 2 || productive_folds < 2 {
        return (StopDecision::IneligibleMin2Folds, 0.0);
    }
    let hyp_ent: f64 = hyps.iter().map(|(w, ph)| w * entropy_bern(*ph)).sum();
    let i_hyp = (entropy_bern(p_k) - hyp_ent).max(0.0);
    let mut best_voi = 0.0_f64;

    for c in cruxes {
        let (q_j, p_pos, p_neg) = project_martingale_crux(p_k, c.q_raw, c.p_pos_raw);
        let kl_pos = kl_bern(p_pos, p_k);
        let kl_neg = kl_bern(p_neg, p_k);
        if c.attempts == 0 && kl_pos.max(kl_neg) >= kappa_tail {
            return (StopDecision::ContinueUnprobedFatTail, f64::INFINITY);
        }
        let delta_log = c.r_hit * (q_j * kl_pos + (1.0 - q_j) * kl_neg);
        let eff_gain = delta_log.max(0.25 * c.r_hit * i_hyp);
        let voi = eff_gain / c.cost_usd.max(1e-6);
        best_voi = best_voi.max(voi);
    }
    if best_voi < lam_nat_per_usd {
        (StopDecision::SealBelief, best_voi)
    } else {
        (StopDecision::ContinueHighVoi, best_voi)
    }
}

/// King-Zeng prior-shift calibration (`arXiv:2610.04798`, `HYP-05F`).
#[must_use]
pub fn prior_shift_prob(q_train: f64, pi_train: f64, pi_real: f64) -> f64 {
    sigmoid(logit(q_train) - logit(pi_train) + logit(pi_real))
}

/// Unified 1D Latent-Logit Kalman market blend + closed-form Gaussian KL Bayesian Surprise (`§11.3`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct KalmanBlendResult {
    pub p_post: ProbClamped,
    pub post_var: f64,
    pub bayesian_surprise_nats: f64,
}

#[must_use]
pub fn kalman_market_blend(
    p_indep: ProbClamped,
    prior_var: f64,
    p_market: ProbClamped,
    phi_liq: f64,
    r0: f64,
) -> KalmanBlendResult {
    let mu0 = p_indep.logit();
    let v0 = prior_var.max(1e-6);
    let r_t = r0.max(1e-6) / phi_liq.max(1e-3);
    let k_gain = v0 / (v0 + r_t);
    let mu1 = mu0 + k_gain * (p_market.logit() - mu0);
    let v1 = (1.0 - k_gain) * v0;
    // Closed-form KL(N(mu1, v1) || N(mu0, v0))
    let kl = 0.5 * ((v1 / v0) + ((mu1 - mu0).powi(2) / v0) - 1.0 + (v0 / v1).ln());
    KalmanBlendResult {
        p_post: ProbClamped::clamp_lossy(sigmoid(mu1)),
        post_var: v1,
        bayesian_surprise_nats: kl.max(0.0),
    }
}

/// Gated ridge-to-identity Platt + uncertainty-conditioned extremizing (`§12.7`).
#[must_use]
pub fn recalibrate_gated(
    p: ProbClamped,
    resolved_n: usize,
    a_ridge: f64,
    b_ridge: f64,
    uncertainty_u: f64,
    d_max: f64,
) -> ProbClamped {
    if resolved_n < 100 {
        return p;
    }
    let u = uncertainty_u.clamp(0.0, 1.0);
    let d_u = 1.0 + (d_max.max(1.0) - 1.0) * (1.0 - u);
    let z = d_u * (a_ridge * p.logit() + b_ridge);
    ProbClamped::clamp_lossy(sigmoid(z))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn still_invariance_and_up_monotonicity() {
        let mut b = ConjugateBelief { alpha: 0.4, beta: 9.6 };
        for _ in 0..30 {
            b = b.stream_step(
                0.04, 10.0, DomainKind::Cyber, HorizonGeometry::PointInTime,
                0.0, 720.0, 720.0, 0.5, Direction::Still, 0.0, 0.0, 1.0, 1.35, 0.6,
            );
        }
        assert!((b.mean() - 0.04).abs() < 1e-12);
        assert!((b.n_eff() - 10.0).abs() < 1e-12);

        let hi = ConjugateBelief { alpha: 8.0, beta: 2.0 }.stream_step(
            0.50, 4.0, DomainKind::Geopolitics, HorizonGeometry::PointInTime,
            0.0, 500.0, 1000.0, 0.5, Direction::Up, 0.25, 0.6, 1.0, 1.35, 0.6,
        );
        assert!(hi.mean() > 0.80);
    }

    #[test]
    fn king_zeng_and_kalman_surprise_parity() {
        let p_cal = prior_shift_prob(0.70, 0.50, 0.01);
        assert!(p_cal > 0.02 && p_cal < 0.03);

        let deep = kalman_market_blend(
            ProbClamped::new(0.40).expect("p"),
            0.25,
            ProbClamped::new(0.50).expect("p"),
            0.90,
            0.10,
        );
        let thin = kalman_market_blend(
            ProbClamped::new(0.40).expect("p"),
            0.25,
            ProbClamped::new(0.50).expect("p"),
            0.01,
            0.10,
        );
        assert!(deep.bayesian_surprise_nats > 10.0 * thin.bayesian_surprise_nats);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(10_000))]

        #[test]
        fn prop_martingale_projection_preserves_doob_law(
            p_k in 1e-5_f64..(1.0 - 1e-5),
            q_raw in 0.0_f64..1.0,
            p_pos_raw in 0.0_f64..1.0,
        ) {
            let (q, p_pos, p_neg) = project_martingale_crux(p_k, q_raw, p_pos_raw);
            prop_assert!((PROB_EPS..=1.0 - PROB_EPS).contains(&p_neg));
            let mean = q * p_pos + (1.0 - q) * p_neg;
            prop_assert!((mean - p_k).abs() < 1e-10);
        }
    }
}
