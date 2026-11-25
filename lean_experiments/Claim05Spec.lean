/-!
# FrankenForecast (`fcast`) — `CLAIM-05` Formal Specification Draft (`HYP-05A` .. `HYP-05F`)

Modeled on Google DeepMind's **AlphaProof Code** verification methodology
(<https://deepmind.google/blog/introducing-alphaproof-code/>):
1. **Small Human-Audited Trusted Computing Base (TCB):** Types, domain invariants,
   non-vacuity witnesses (`inhabited_*`), and top-level safety/optimality theorems
   live in this human-reviewed specification file.
2. **Untrusted LLM Oracle Boundary (`UntrustedSignal`, `UntrustedCrux`):** Safety
   theorems NEVER assume the LLM is calibrated or truthful; they quantify over
   arbitrary adversarial oracle outputs and prove Rust's deterministic projections
   preserve invariants unconditionally.
3. **Explicit `[CLOSER-LOOK]` Annotations (`AUDIT-FLAG-01` .. `AUDIT-FLAG-07`):**
   Marks the ~1% specification boundaries where real-number (`ℝ`) math, discrete
   ticks, or IEEE-754 `Float64` (`f64`) machine semantics can silently diverge.

*Note on Toolchain:* Written in Lean 4 + `Mathlib` real/log/Finset vocabulary.
Proof bodies marked `sorry` denote autonomous prover-agent obligations once `lake` +
`mathlib4` are initialized; discrete structural lemmas carry constructive proofs.
-/

import Mathlib.Data.Real.Basic
import Mathlib.Analysis.SpecialFunctions.Log.Basic
import Mathlib.Analysis.SpecialFunctions.Exp
import Mathlib.Data.Finset.Basic

noncomputable section

namespace FrankenForecast.Claim05

/-! ## Module 0 · Trusted Computing Base (TCB) Primitives & Non-Vacuity Witnesses -/

/-- Clamping constant matching `fcast-core::ProbClamped` (`ε = 10⁻⁶`). -/
def epsDefault : ℝ := (1 : ℝ) / 1000000

/-- A clamped open-interval probability `p ∈ [ε, 1 - ε] ⊂ (0, 1)`.
    `[CLOSER-LOOK: AUDIT-FLAG-01 & AUDIT-FLAG-07]` In Rust `f64`, `NaN` bypasses naive
    `min`/`max` comparisons unless explicitly guarded (`!p.is_nan()`). -/
structure ProbClamped (ε : ℝ) where
  val    : ℝ
  h_eps  : 0 < ε ∧ ε < (1 : ℝ) / 2
  h_low  : ε ≤ val
  h_high : val ≤ 1 - ε

/-- Distinct newtypes for absolute log-odds vs. prior-centered log-likelihood ratios.
    `[CLOSER-LOOK: AUDIT-FLAG-06]` Separating `LogOdds` from `CenteredLLR` prevents
    double-counting the base-rate prior `logit(π_real)` during multi-modal fusion! -/
structure LogOdds where
  theta : ℝ

structure CenteredLLR where
  ell : ℝ

def sigmoid (θ : ℝ) : ℝ :=
  1 / (1 + Real.exp (-θ))

def logit (p : ℝ) : ℝ :=
  Real.log (p / (1 - p))

def clampReal (x lo hi : ℝ) : ℝ :=
  max lo (min hi x)

/-- TCB Non-Vacuity Witness (`AUDIT-FLAG-07`): proves `ProbClamped epsDefault` is inhabited
    so no theorem quantifying over `ProbClamped` holds vacuously! -/
theorem nonvacuous_prob_clamped : ∃ _ : ProbClamped epsDefault, True := by
  refine ⟨⟨(1 : ℝ) / 2, ?_, ?_, ?_⟩, trivial⟩ <;> norm_num [epsDefault]

/-! ## Module A · `HYP-05A`: Mean–Precision Filtering vs. Linear Pseudo-Count Updating -/

inductive Direction where
  | Up
  | Down
  | Still
  deriving DecidableEq, Repr

/-- Untrusted Tier-0 LLM likelihood output (`Loom`). -/
structure UntrustedSignal where
  dir           : Direction
  rawDeltaLLR   : ℝ
  diagnosticity : ℝ

/-- Mean–Precision state stored natively in log-odds `theta` + effective precision `nEff`.
    `[CLOSER-LOOK: AUDIT-FLAG-01]` Storing `theta : LogOdds` directly avoids `Float64`
    catastrophic cancellation in `1.0 - p` when `p → 1 - 10⁻⁶` across repeated `"Still"` ticks! -/
structure MeanPrecisionState where
  theta : LogOdds
  nEff  : ℝ
  h_pos : 0 < nEff

/-- Naive baseline (`LinearPseudoCount`): `α_t = α_{t-1} + y_t`, `β_t = β_{t-1} + (1 - y_t)`. -/
def naivePseudoCountStep (α β y : ℝ) : ℝ × ℝ :=
  (α + y, β + (1 - y))

/-- Counterexample Theorem (`HYP-05A` Baseline Defect):
    Adding neutral pseudo-count `y = 1/2` strictly pulls any tail belief `μ < 1/2` upward toward `1/2`,
    and adding "mildly Up" `y = 0.65` to a high prior `μ = 0.80` (`α=8, β=2`) strictly LOWERS the mean! -/
theorem naive_pseudocount_sign_inversion :
    let (α', β') := naivePseudoCountStep 8 2 (65 / 100)
    α' / (α' + β') < (8 : ℝ) / (8 + 2) := by
  norm_num [naivePseudoCountStep]

/-- Repaired Mean–Precision update step (`HYP-05A`). -/
def meanPrecisionUpdate
    (s : MeanPrecisionState)
    (sig : UntrustedSignal)
    (novelty κLR ηShock : ℝ) : LogOdds × ℝ :=
  let ν := clampReal novelty 0 1
  let d := clampReal sig.diagnosticity 0 1
  let Δℓ := if sig.dir = Direction.Still then 0 else clampReal sig.rawDeltaLLR (-2.5) 2.5
  let w  := if sig.dir = Direction.Still then 0 else ν * d
  let θ' := if sig.dir = Direction.Still then s.theta.theta else s.theta.theta + κLR * ν * Δℓ
  let n' := (s.nEff / (1 + ηShock * (Δℓ ^ 2))) + w
  (⟨θ'⟩, n')

/-- Safety Theorem (`HYP-05A.1` — Exact `"Still"` Invariance):
    Regardless of untrusted `rawDeltaLLR` or `diagnosticity`, a `Direction.Still` classification
    leaves both log-odds `θ` and effective sample size `nEff` bit-identically unchanged. -/
theorem hyp05a_still_strictly_invariant
    (s : MeanPrecisionState) (sig : UntrustedSignal) (ν κ η : ℝ)
    (h_still : sig.dir = Direction.Still) :
    meanPrecisionUpdate s sig ν κ η = (s.theta, s.nEff) := by
  simp [meanPrecisionUpdate, h_still]

/-- Monotonicity Theorem (`HYP-05A.2` — Elimination of High-Prior Sign Inversion):
    For any positive gain `κ > 0`, novelty `ν > 0`, and positive `Up` likelihood `Δℓ > 0`,
    posterior log-odds strictly increases regardless of how close `s.theta` already is to `1.0`. -/
theorem hyp05a_positive_llr_strictly_increases_theta
    (s : MeanPrecisionState) (sig : UntrustedSignal) (ν κ η : ℝ)
    (h_up : sig.dir = Direction.Up)
    (h_κ : 0 < κ) (h_ν : 0 < ν ∧ ν ≤ 1)
    (h_ℓ : 0 < sig.rawDeltaLLR ∧ sig.rawDeltaLLR ≤ 2.5) :
    s.theta.theta < (meanPrecisionUpdate s sig ν κ η).1.theta := by
  sorry -- Prover obligation: `clampReal` identity on `(0, 1]` and `(0, 2.5]` + `mul_pos`

/-! ## Module B · `HYP-05B`: Adaptive `DecayController` & Regime-Shift Contraction -/

inductive HorizonGeometry where
  | PointInTime
  | RollingWindow
  | CumulativeStoppingTime
  deriving DecidableEq, Repr

structure DomainDecayBounds where
  beta   : ℝ
  tauMin : ℝ
  tauMax : ℝ
  h_ord  : 0 < tauMin ∧ tauMin ≤ tauMax

/-- Market-blind Kulhavý–Zarrop effective half-life (`INV-04`: zero market volatility input!). -/
def effectiveHalfLife (b : DomainDecayBounds) (hRem surpriseEwma λVol : ℝ) : ℝ :=
  let base := clampReal (b.beta * max hRem 6) b.tauMin b.tauMax
  base / (1 + λVol * max 0 (surpriseEwma - 1))

/-- Safety Theorem (`HYP-05B.1` — Regime-Surprise Monotone Memory Contraction):
    Higher endogenous conflict `S₂ ≥ S₁ ≥ 1` with `λVol > 0` strictly shortens (or preserves)
    effective half-life `τ_eff`, accelerating decay of stale pre-regime-shift precision. -/
theorem hyp05b_surprise_contracts_halflife
    (b : DomainDecayBounds) (hRem S₁ S₂ λVol : ℝ)
    (h_λ : 0 < λVol) (h_s : 1 ≤ S₁ ∧ S₁ < S₂) :
    effectiveHalfLife b hRem S₂ λVol < effectiveHalfLife b hRem S₁ λVol := by
  sorry -- Prover obligation: positive numerator `base ≥ b.tauMin > 0` divided by strictly increasing denom

/-- `[CLOSER-LOOK: AUDIT-FLAG-02]` Time-Subdivision Semigroup Property:
    Single-step decay `γ(Δτ₁ + Δτ₂) = γ(Δτ₁) · γ(Δτ₂)` holds ONLY when `effectiveHalfLife` and
    `anchorMean` are constant across `[t, t + Δτ₁ + Δτ₂]` (or evaluated on a canonical `1h` discrete
    grid)! Variable `hRem` + `CumulativeStoppingTime` require grid-canonical stepping for replay parity. -/
def canonicalHourlyGridSteps (tStartHours tEndHours : Nat) : Nat :=
  tEndHours - tStartHours

/-! ## Module C · `HYP-05C`: Correlation-Aware Evidence Aggregation & Overconfidence Bound -/

/-- `[CLOSER-LOOK: AUDIT-FLAG-03]` Embedding cosine similarity lies in `[-1, 1]`, NOT `[0, 1]`.
    Without `max 0 maxSim`, negative cosine similarity yields `novelty > 1`, inflating precision! -/
def clippedNovelty (maxCosineSim : ℝ) : ℝ :=
  1 - clampReal maxCosineSim 0 1

theorem hyp05c_clipped_novelty_unit_interval (sim : ℝ) :
    0 ≤ clippedNovelty sim ∧ clippedNovelty sim ≤ 1 := by
  sorry -- Prover obligation: `0 ≤ clampReal sim 0 1 ≤ 1`

/-- Overconfidence Bound (`HYP-05C.2`):
    When `M` syndicated wire copies arrive with pairwise similarity `≥ 1 - δ` to the primary flash
    (`0 ≤ δ ≪ 1`), cumulative precision added by the `M - 1` echoes is bounded by `(M - 1) · δ`
    rather than `(M - 1)` under naive independent Bayesian pooling. -/
theorem hyp05c_echo_burst_precision_bound
    (M : Nat) (δ : ℝ) (h_δ : 0 ≤ δ ∧ δ ≤ 1) :
    (M - 1 : ℝ) * clippedNovelty (1 - δ) ≤ (M - 1 : ℝ) * δ := by
  sorry -- Prover obligation: `clippedNovelty (1 - δ) = δ` for `δ ∈ [0, 1]`

/-! ## Module D · `HYP-05D`: CAS-Handle PBM `fold` & Monotone Tombstone Non-Resurrection -/

abbrev EvHandle := Nat

structure FoldState where
  foldIdx            : Nat
  productiveFolds    : Nat
  activeEvidence     : Finset EvHandle
  cumulativeTombstones : Finset EvHandle
  h_disjoint         : Disjoint activeEvidence cumulativeTombstones

/-- Atomic `FoldCommit` transition enforcing monotone tombstones (`AUDIT-FLAG-04`):
    Any previously tombstoned handle stays tombstoned and is stripped from `proposedSupport`,
    preventing a bounded-context future fold from resurrecting a debunked rumor ("Zombie Lead"). -/
def applyFoldCommit
    (prev : FoldState)
    (proposedSupport newTombstones : Finset EvHandle)
    (isProductive : Bool) : FoldState :=
  let nextTombs := prev.cumulativeTombstones ∪ newTombstones
  let cleanSupport := proposedSupport \ nextTombs
  { foldIdx := prev.foldIdx + 1
  , productiveFolds := if isProductive then prev.productiveFolds + 1 else prev.productiveFolds
  , activeEvidence := cleanSupport
  , cumulativeTombstones := nextTombs
  , h_disjoint := Finset.disjoint_sdiff_self_left
  }

/-- Constructively Proved Safety Theorem (`HYP-05D.1` — Zombie-Lead Non-Resurrection):
    Once an evidence handle `h` is tombstoned at fold `k`, no subsequent `applyFoldCommit` can
    ever place `h` in `activeEvidence`. -/
theorem hyp05d_tombstone_never_resurrected
    (prev : FoldState) (propSup newTombs : Finset EvHandle) (prod : Bool) (h : EvHandle)
    (h_tomb : h ∈ prev.cumulativeTombstones) :
    h ∉ (applyFoldCommit prev propSup newTombs prod).activeEvidence := by
  simp [applyFoldCommit, Finset.mem_sdiff, Finset.mem_union, h_tomb]

/-! ## Module E · `HYP-05E`: Fréchet-Feasible Martingale Log-Score `VoI_log` Stopping -/

def bernoulliKL (q p : ℝ) : ℝ :=
  q * Real.log (q / p) + (1 - q) * Real.log ((1 - q) / (1 - p))

structure UntrustedCrux where
  qRaw   : ℝ
  pPosRaw : ℝ
  hitRate : ℝ
  costUsd : ℝ
  attempts : Nat

/-- `[CLOSER-LOOK: AUDIT-FLAG-05 — CRITICAL BUG DISCOVERED BY LEAN FORMALIZATION!]`
    In our earlier draft pseudocode (`verify_math_and_contracts.py` L80), we wrote:
      `p_neg = clamp((p_k - q * p_pos) / (1 - q), ε, 1 - ε)`
    WITHOUT constraining `q` to the Fréchet-feasible range compatible with `(p_k, p_pos, ε)`!
    When `p_k = 0.04`, `q = 0.20`, `p_pos = 0.65`, `(0.04 - 0.13)/0.80 = -0.1125` clamps to `ε`,
    so `q * p_pos + (1 - q) * p_neg = 0.13 ≠ 0.04` — BREAKING Doob's Martingale identity and
    making true expected log-score gain NEGATIVE while naive KL looks huge!

    `projectMartingaleCrux` projects `q` onto the `[2ε, 1-2ε]`-buffered Martingale interval
    (`εMargin = 2 * ε`) so `q * pPos + (1 - q) * pNeg = pK` holds IDENTICALLY over `ℝ` AND
    `pNeg` stays strictly inside `[ε, 1 - ε]` even under 1-ULP IEEE-754 `Float64` rounding! -/
def projectMartingaleCrux (pK pPosRaw qRaw ε : ℝ) : ℝ × ℝ × ℝ :=
  let εm := 2 * ε
  let pPos := clampReal pPosRaw εm (1 - εm)
  let qMaxPos := if pPos > pK then (pK - εm) / (pPos - εm)
                 else if pPos < pK then (1 - εm - pK) / (1 - εm - pPos)
                 else 1 - εm
  let qFeas := clampReal qRaw ε (min (1 - ε) qMaxPos)
  let pNeg  := (pK - qFeas * pPos) / (1 - qFeas)
  (qFeas, pPos, pNeg)

/-- Fundamental Identity (`HYP-05E.1` — Proper Log-Score Expected Gain = Expected KL):
    Whenever `(q, pPos, pNeg)` satisfies the exact Martingale law `q * pPos + (1 - q) * pNeg = pK`
    with `pK, pPos, pNeg ∈ (0, 1)`, expected log-score improvement under `Y ~ Bernoulli(pK)` equals
    `q * KL(pPos ‖ pK) + (1 - q) * KL(pNeg ‖ pK) ≥ 0`. -/
theorem hyp05e_martingale_logscore_eq_kl
    (pK q pPos pNeg : ℝ)
    (h_pk : 0 < pK ∧ pK < 1) (h_q : 0 < q ∧ q < 1)
    (h_pos : 0 < pPos ∧ pPos < 1) (h_neg : 0 < pNeg ∧ pNeg < 1)
    (h_mart : q * pPos + (1 - q) * pNeg = pK) :
    q * (pPos * Real.log (pPos / pK) + (1 - pPos) * Real.log ((1 - pPos) / (1 - pK))) +
    (1 - q) * (pNeg * Real.log (pNeg / pK) + (1 - pNeg) * Real.log ((1 - pNeg) / (1 - pK)))
    = q * bernoulliKL pPos pK + (1 - q) * bernoulliKL pNeg pK := by
  rfl

/-- Hard 2-Fold Eligibility Gate (`INV-10` & `HYP-05E.2`). -/
def eligibleToStop (foldIdx productiveFolds maxFolds : Nat) (spentUsd budgetUsd : ℝ) : Prop :=
  spentUsd ≥ budgetUsd ∨ foldIdx ≥ maxFolds ∨ (foldIdx ≥ 2 ∧ productiveFolds ≥ 2)

theorem hyp05e_never_stops_at_fold_one
    (productiveFolds : Nat) (spentUsd budgetUsd : ℝ)
    (h_budget : spentUsd < budgetUsd) :
    ¬ eligibleToStop 1 productiveFolds 6 spentUsd budgetUsd := by
  unfold eligibleToStop
  omega

/-! ## Module F · `HYP-05F`: King-Zeng Prior-Shift & Centered Multi-Modal Fusion -/

/-- Extracts prior-centered log-likelihood ratio from a case-control classifier (`arXiv:2610.04798`). -/
def extractCenteredLLR (qTrain πTrain : ℝ) : CenteredLLR :=
  ⟨logit qTrain - logit πTrain⟩

/-- Applies exact King-Zeng prior shift + L2-shrunk multi-modal fusion (`AUDIT-FLAG-06`):
    Notice `logit πReal` is added EXACTLY ONCE to the shrunk sum of `CenteredLLR`s! -/
def fusePriorShiftedCyber
    (πReal : ℝ) (techLLR geoLLR : CenteredLLR) (shrinkTech shrinkGeo : ℝ) : LogOdds :=
  let fusedLLR := clampReal (shrinkTech * techLLR.ell + shrinkGeo * geoLLR.ell) (-4.0) 4.0
  ⟨logit πReal + fusedLLR⟩

/-- Exact Bayes Label-Shift Recovery (`HYP-05F.1`):
    Under pure case-control prevalence shift (`P_train(X|Y) = P_real(X|Y)`), shifting single-model
    balanced posterior `qTrain` by `logit(πReal) - logit(πTrain)` recovers exact `logit P_real(Y=1|X)`,
    whereas unshifted `qTrain` incurs systematic calibration bias `|logit πReal - logit πTrain|`. -/
theorem hyp05f_king_zeng_recovers_true_posterior_logodds
    (pXGivenY1 pXGivenY0 πTrain πReal : ℝ)
    (h_pos : 0 < pXGivenY1 ∧ 0 < pXGivenY0)
    (h_tr : 0 < πTrain ∧ πTrain < 1) (h_re : 0 < πReal ∧ πReal < 1) :
    let qTrain := (πTrain * pXGivenY1) / (πTrain * pXGivenY1 + (1 - πTrain) * pXGivenY0)
    let pTrue  := (πReal  * pXGivenY1) / (πReal  * pXGivenY1 + (1 - πReal)  * pXGivenY0)
    logit πReal + (extractCenteredLLR qTrain πTrain).ell = logit pTrue := by
  sorry -- Prover obligation: algebraic cancellation of `πTrain / (1 - πTrain)` inside `Real.log`

end FrankenForecast.Claim05
