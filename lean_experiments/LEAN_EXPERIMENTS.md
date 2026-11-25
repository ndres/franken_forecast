# Lean 4 Formal Specification Experiment for `CLAIM-05` (`HYP-05A` .. `HYP-05F`)

* **Reference Paradigm:** Google DeepMind, *Introducing AlphaProof Code* (<https://deepmind.google/blog/introducing-alphaproof-code/>)
* **Artifacts in This Folder:**
  * [`Claim05Spec.lean`](./Claim05Spec.lean) — Human-audited Lean 4 Trusted Computing Base (TCB) specification covering `HYP-05A` .. `HYP-05F`.
  * [`verify_audit_flags.py`](./verify_audit_flags.py) — Executable numerical witness script reproducing every edge case and counterexample discovered during Lean formalization.
  * [`LEAN_EXPERIMENTS.md`](./LEAN_EXPERIMENTS.md) — Reference pseudocode draft, pseudocode-to-Lean translation architecture, and the **7 "Closer Look" Audit Findings (`AUDIT-FLAG-01` .. `AUDIT-FLAG-07`)**.

---

## 1. Why Apply the *AlphaProof Code* Workflow Before Writing Rust?

In DeepMind's *AlphaProof Code* architecture (demonstrated on `libsignal`), formal verification succeeds at scale through a strict three-part division of labor:

1. **Small, Human-Reviewed Trusted Specification (TCB):** Humans and spec-drafting models translate domain protocols and pseudocode into ~`10³–10⁴` lines of Lean 4 definitions and top-level theorems. Because an autonomous prover will happily exploit a flawed or vacuous specification (`False → anything`), **every line of the specification must be human-readable and human-audited**.
2. **Untrusted Oracle & Machine-Level Realism:** Rather than assuming external inputs or floating-point instructions behave ideally, specifications quantify over **arbitrary adversarial inputs** (`UntrustedSignal`, `UntrustedCrux`) and confront concrete machine semantics (`IEEE-754 f64`, clamping, discrete clocks).
3. **Targeted "Closer Look" Human Audit (~1% High-Leverage Sites):** Translating informal pseudocode into total Lean 4 functions forces hidden preconditions to the surface—flagging the exact 1% of boundary equations where real-number identities silently fail in code **before** a single crate is implemented.

---

## 2. Draft Reference Pseudocode for `CLAIM-05` (`HYP-05A` .. `HYP-05F`)

Below is the consolidated Rust-like reference pseudocode drafted for the six subordinate experimental hypotheses of `CLAIM-05` (`Loom` & `Crucible`), incorporating the repairs forced by [`Claim05Spec.lean`](./Claim05Spec.lean):

```rust
// --- HYP-05A: Native Log-Odds Mean-Precision Filter ---
fn mean_precision_step(
    state: MeanPrecisionState,          // stores (theta: LogOdds, n_eff: f64 > 0)
    sig: UntrustedSignal,               // (dir: Up|Down|Still, raw_delta_llr, diagnosticity)
    novelty_raw: f64,                   // cosine-derived novelty
    kappa_lr: f64,
    eta_shock: f64,
) -> MeanPrecisionState {
    assert!(!sig.raw_delta_llr.is_nan() && !novelty_raw.is_nan());
    if sig.dir == Direction::Still {
        return state;                   // Exact bit-identity short-circuit (AUDIT-FLAG-01)
    }
    let nu = novelty_raw.clamp(0.0, 1.0);                  // Clipped novelty (AUDIT-FLAG-03)
    let diag = sig.diagnosticity.clamp(0.0, 1.0);
    let dl = sig.raw_delta_llr.clamp(-2.5, 2.5);
    let theta_next = (state.theta + kappa_lr * nu * dl).clamp(-13.8155, 13.8155);
    let n_next = (state.n_eff / (1.0 + eta_shock * dl * dl)) + nu * diag;
    MeanPrecisionState { theta: theta_next, n_eff: n_next }
}

// --- HYP-05B: Market-Blind Kulhavy-Zarrop DecayController (Canonical 1h Grid) ---
fn decay_on_hourly_grid(
    alpha: f64, beta: f64, mu0: f64, n0: f64,
    bounds: DomainDecayBounds, geometry: HorizonGeometry,
    t_prev_hour: u64, t_now_hour: u64, t_open_hour: u64, t_close_hour: u64,
    surprise_ewma: f64, lambda_vol: f64,
) -> (f64, f64) {
    let (mut a, mut b) = (alpha, beta);
    for h in (t_prev_hour + 1)..=t_now_hour {              // Semigroup-safe 1h grid (AUDIT-FLAG-02)
        let h_rem = (t_close_hour.saturating_sub(h) as f64).max(6.0);
        let tau_0 = (bounds.beta * h_rem).clamp(bounds.tau_min, bounds.tau_max);
        let tau_eff = tau_0 / (1.0 + lambda_vol * (surprise_ewma - 1.0).max(0.0));
        let gamma = 2.0_f64.powf(-1.0 / tau_eff);
        let mu_anc = match geometry {
            HorizonGeometry::CumulativeStoppingTime => {
                let frac = (h_rem / ((t_close_hour - t_open_hour).max(1) as f64)).clamp(1e-4, 1.0);
                1.0 - (1.0 - mu0).powf(frac)
            }
            _ => mu0,
        };
        a = gamma * a + (1.0 - gamma) * n0 * mu_anc;
        b = gamma * b + (1.0 - gamma) * n0 * (1.0 - mu_anc);
    }
    (a, b)
}

// --- HYP-05C: Correlation-Aware Novelty Discounting ---
fn clipped_novelty(max_cosine_sim: f64) -> f64 {
    1.0 - max_cosine_sim.clamp(0.0, 1.0)                   // Blocks <0 cosine inflation (AUDIT-FLAG-03)
}

// --- HYP-05D: CAS-Handle PBM Fold with Monotone Cumulative Tombstones ---
fn apply_fold_commit(prev: FoldState, prop_support: Set<EvHandle>, new_tombs: Set<EvHandle>, prod: bool) -> FoldState {
    let tombs = prev.cumulative_tombstones.union(&new_tombs);
    let active = prop_support.difference(&tombs);          // Blocks Zombie Lead revival (AUDIT-FLAG-04)
    FoldState {
        fold_idx: prev.fold_idx + 1,
        productive_folds: prev.productive_folds + usize::from(prod),
        active_evidence: active,
        cumulative_tombstones: tombs,
    }
}

// --- HYP-05E: Frechet-Feasible Martingale Projection & Log-Score VoI Stopping ---
fn project_martingale_crux(p_k: f64, p_pos_raw: f64, q_raw: f64, eps: f64) -> (f64, f64, f64) {
    let eps_m = 2.0 * eps;                                 // 2*eps IEEE-754 ULP guard (AUDIT-FLAG-05b!)
    let p_pos = p_pos_raw.clamp(eps_m, 1.0 - eps_m);
    let q_max = if p_pos > p_k {
        (p_k - eps_m) / (p_pos - eps_m)                    // Frechet upper bound (AUDIT-FLAG-05!)
    } else if p_pos < p_k {
        (1.0 - eps_m - p_k) / (1.0 - eps_m - p_pos)
    } else {
        1.0 - eps_m
    };
    let q_feas = q_raw.clamp(eps, (1.0 - eps).min(q_max));
    let p_neg = (p_k - q_feas * p_pos) / (1.0 - q_feas);   // Strictly in [eps, 1-eps] even after f64 ULP rounding!
    (q_feas, p_pos, p_neg)
}

// --- HYP-05F: Centered-LLR King-Zeng Prior-Shifted Cyber Calibration ---
fn fuse_prior_shifted_cyber(pi_real: f64, q_tech: f64, q_geo: f64, pi_tr: f64, w_t: f64, w_g: f64) -> LogOdds {
    let llr_tech = CenteredLLR(logit(q_tech) - logit(pi_tr));
    let llr_geo  = CenteredLLR(logit(q_geo)  - logit(pi_tr));
    let fused    = (w_t * llr_tech.0 + w_g * llr_geo.0).clamp(-4.0, 4.0);
    LogOdds(logit(pi_real) + fused)                        // Base rate added ONCE (AUDIT-FLAG-06)
}
```

---

## 3. Structure of [`Claim05Spec.lean`](./Claim05Spec.lean): Safety Theorems vs. Statistical Theorems

Translating `CLAIM-05` into Lean forced a crisp separation between two fundamentally different proof classes:

1. **Deterministic Safety & Structural Theorems (Hold Unconditionally Against Any Untrusted LLM Output):**
   * `hyp05a_still_strictly_invariant` *(proved constructively)*: `Direction.Still` preserves `(θ, nEff)` identically even if the LLM emits garbage non-zero `rawDeltaLLR`.
   * `hyp05a_positive_llr_strictly_increases_theta`: Positive `Up` evidence strictly raises log-odds even at `p = 0.80` or `0.99` (accompanied by `naive_pseudocount_sign_inversion`, constructively proving that linear pseudo-counts fail!).
   * `hyp05b_surprise_contracts_halflife`: Higher endogenous surprise `S_t > 1` strictly contracts memory half-life without reading market volatility.
   * `hyp05c_clipped_novelty_unit_interval` & `hyp05c_echo_burst_precision_bound`: Bounds syndicated wire precision growth by `(M - 1) · δ`.
   * `hyp05d_tombstone_never_resurrected` *(proved constructively)*: Debunked `EvidenceVault` handles cannot re-enter `activeEvidence` in later bounded-context folds.
   * `hyp05e_martingale_logscore_eq_kl` *(proved constructively)* & `hyp05e_never_stops_at_fold_one` *(proved via `omega`)*: Proves expected log-score gain equals expected KL *under the exact Martingale equality*, and mechanically blocks fold-1 stopping.
2. **Conditional Statistical Optimality Theorems (Require Explicit Generative Assumptions in the TCB):**
   * `hyp05f_king_zeng_recovers_true_posterior_logodds`: Recovers exact posterior log-odds **iff** the label-shift invariance assumption `P_train(X | Y) = P_real(X | Y)` holds—making clear where empirical `ForecastBench` / `EuRepoC` validation (`GATE-14`) must take over from deductive proof!

---

## 4. The Seven Critical "Closer Look" Audit Sites (`AUDIT-FLAG-01` .. `AUDIT-FLAG-07`)

Drafting `Claim05Spec.lean` exposed **seven concrete places** where human engineers and Rust implementers MUST look closely—including **one genuine mathematical bug (`AUDIT-FLAG-05`)** in our earlier Python prototype (`scripts/verify_math_and_contracts.py` line 80), all verified in [`verify_audit_flags.py`](./verify_audit_flags.py):

### `AUDIT-FLAG-01` (`HYP-05A`) — `Float64` Catastrophic Cancellation in `p → logit(p) → sigmoid` Round-Trips
* **What Lean Flagged:** Over `ℝ`, `sigmoid (logit p + 0) = p`. Over IEEE-754 `f64`, when `p = 1 - 10⁻⁶`, `1.0 - p` loses ~`20` bits of mantissa precision; repeated `"Still"` ticks that evaluate `sigmoid(logit(p) + 0.0)` drift numerically instead of staying invariant.
* **Required Rust Fix (`fcast-core`):** Store `theta: f64` (`LogOdds`) as the primary field inside `ConjugateBelief` (computing `p = sigmoid(theta)` on read) AND branch-short-circuit `Direction::Still` before touching transcendental math.

### `AUDIT-FLAG-02` (`HYP-05B`) — Non-Autonomous Decay Breaks Time-Subdivision Semigroup Consistency
* **What Lean Flagged:** When `effectiveHalfLife` depends on remaining horizon `H_rem = T_close - t` and `CumulativeStoppingTime` shifts `mu_anc(t)` over time, stepping `1 × 24h` vs. `24 × 1h` using right-endpoint evaluation diverges by `> 10⁻⁴` (`verify_audit_flags.py` L49), which would make `HorizonForecastView` depend on `fcastd` wakeup jitter!
* **Required Rust Fix (`fcast-core`):** Define `DecayController` over a **canonical discrete `1-hour` UTC grid** (`canonicalHourlyGridSteps`) between persisted revisions so replay at any `eval_as_of_utc` is 100% path-independent and deterministic.

### `AUDIT-FLAG-03` (`HYP-05C`) — Negative Cosine Similarity Turns `1 - max_sim` Into Precision Inflation (`> 1.0`)
* **What Lean Flagged:** Typing `sim : Evidence → Evidence → ℝ` forced asking its codomain. Embedding cosine similarity (`model2vec`) spans **`[-1, 1]`**, not `[0, 1]`! When a first-in-subtopic article has `max_sim = -0.35`, `1 - max_sim = 1.35 > 1.0`, unintentionally amplifying `κ_LR` by `+35%` and adding `> 1` pseudo-observation.
* **Required Rust Fix (`fcast-epistemic`):** Clamp similarity to `[0, 1]` (`1.0 - max_sim.clamp(0.0, 1.0)`), and note in TCB review that arrival-order pairwise max-sim is non-commutative when a summary wire arrives before detailed primary reports.

### `AUDIT-FLAG-04` (`HYP-05D`) — Bounded-Context "Zombie Lead" Resurrection Across Folds
* **What Lean Flagged:** Proving `hyp05d_tombstone_never_resurrected` by induction across folds failed if `discarded_leads` only filtered the *current* fold's delta: because PBM `fold` discards raw prior search transcripts to bound context, Fold `k+2` can re-fetch a debunked URL, obtain the identical content-addressed `[ev:hash#s1]` handle from `EvidenceVault`, and re-insert it into `evidence_handles`!
* **Required Rust Fix (`fcast-epistemic`):** Persist `cumulative_tombstones` monotonically across the entire `forecast_id` stream and subtract it unconditionally inside `FoldCommit` (unless an explicit `ReviveWithCounterproof` witness is provided).

### `AUDIT-FLAG-05` (`HYP-05E`) — **CRITICAL BUG CAUGHT:** Clamping `p_neg` Alone Breaks Doob's Martingale Law & Fakes Positive VoI on Value-Destroying Calls!
* **What Lean Flagged:** In our earlier prototype (`scripts/verify_math_and_contracts.py` L80), we projected candidate cruxes via `p_neg = clamp((p_k - q * p_pos) / (1 - q), 1e-6, 1 - 1e-6)`. When attempting to prove `hyp05e_martingale_logscore_eq_kl`, Lean required the exact hypothesis `q * pPos + (1 - q) * pNeg = pK`.
* **Concrete Counterexample (`verify_audit_flags.py` L69):** Suppose `p_k = 0.04` and the LLM proposes `q = 0.25, p_pos = 0.80` (violating the Fréchet probability bound `q * p_pos ≤ p_k` since `0.20 > 0.04`!). Naive clamping sets `p_neg = 1e-6`, leaving `q * p_pos + (1 - q) * p_neg = 0.20 ≠ 0.04`. Plugging `(0.25, 0.80, 1e-6)` into the KL formula reports a huge fake gain of **`+0.476 nats`**, whereas the **true expected log-score change** under `Y ~ Bernoulli(0.04)` is **`-0.105 nats` (actively destroying log score and burning budget)**!
* **Required Rust Fix (`fcast-core`, Now Patched in `verify_math_and_contracts.py`):** Jointly clamp `q` to the **buffered Fréchet-feasible Martingale bound** `q ≤ (p_k - 2ε) / (p_pos - 2ε)` when `p_pos > p_k` (and `q ≤ (1 - 2ε - p_k) / (1 - 2ε - p_pos)` when `p_pos < p_k`). *Bonus `ℝ` vs. `f64` catch:* using unbuffered `ε` gives `p_neg = ε` analytically over `ℝ`, which underflows `EPS` by `~3.2 × 10⁻¹⁸` (`1–2` ULPs) in IEEE-754 `f64` division `(p_k - q_max * p_pos) / (1 - q_max)`! Buffering `q_max` with `2ε` guarantees strict `EPS <= p_neg <= 1 - EPS` in `f64` while keeping `q * p_pos + (1 - q) * p_neg == p_k` exact to `< 10⁻¹⁵`!

### `AUDIT-FLAG-06` (`HYP-05F`) — Base-Rate Prior Double-Counting & PAV Non-Commutativity in Cyber Fusion
* **What Lean Flagged:** If Tech and Geopolitical classifiers are each prior-shifted to `logit(π_real)` first and then summed, `logit(π_real)` (`≈ -4.60` nats at `1%` hazard) is counted **twice** (`-9.20` nats), crushing two positive cyber alerts (`0.70` and `0.65`) down to **`0.043%`** (`1/20th` of the base rate!). In addition, non-parametric PAV/Venn-Abers calibration must be fit *before* applying the parametric King-Zeng shift, because step-function binning does not commute with affine log-odds shifts.
* **Required Rust Fix (`fcast-core`):** Enforce distinct Rust newtypes `CenteredLLR(f64)` (`logit(q_m) - logit(π_train)`) vs. `LogOdds(f64)` (`logit(π_real) + fused_llr`) so `LogOdds + LogOdds` is a compile-time type error.

### `AUDIT-FLAG-07` (Cross-Cutting TCB) — `IEEE-754 NaN` Bypass & Vacuous Specification Guards
* **What Lean Flagged:** In AlphaProof Code, two classic TCB pitfalls are **(1)** subtly inconsistent struct invariants (`∃ s, Valid s` false $\implies$ every theorem holds vacuously!) and **(2)** Rust `f64::min`/`max`/`clamp` edge behavior if `NaN` arises from `0.0 / 0.0` or `-∞ + ∞` (`f64::clamp` panics on `min > max` and propagates `NaN`!).
* **Required Rust & Lean Fix:** Keep constructive `nonvacuous_*` witnesses in `Claim05Spec.lean` for every state struct, and wrap every `ProbClamped::new` / `LogOdds::new` constructor in `fcast-core` with `val.is_finite()` returning a typed `Result<_, NumericInvariantError>` rather than bare `f64::clamp`.
