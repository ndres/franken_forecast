# Whole-System Refactoring Proposal — FrankenForecast (`fcast`)

* **Scope:** High-horizon, simplicity-first evaluation of six candidate architectural additions against the existing `6`-crate `fcast` specification ([`COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md`](../../COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md) `v1.8`).
* **Executable Mathematical Witnesses:** [`verify_refactoring_math.py`](./verify_refactoring_math.py) (`W1`–`W4`).
* **Guiding Filter:** Add a mechanism **only** if it closes a real epistemic gap in `ForecastRevision` quality *without* creating parallel databases, runaway token bills, or un-attributable complexity.

---

## 1. Executive Decision Matrix (Need × Complexity × Cost)

| Proposal | Need | Complexity | Marginal LLM Cost | Whole-System Verdict |
| :--- | :--- | :--- | :--- | :--- |
| **A. Three-Document Question Memory** (Immutable Gold Dossier + Episodic Event Log + Incremental State Doc) | **High** | **Low** *(overlays existing `EvidenceVault` + `Chronicle`)* | Low (`\$0.01–\$0.03` once for Gold; `\$0.001` incremental patch) | **ADOPT (Unified)** — Gives `Loom` a clean, auditable narrative/constraint layer without a second database. |
| **B. Pre-Update Red-Team Skill** (consulting the Immutable Gold Dossier) | **High** | **Low** *(1 skill/template)* | High if run every tick; **Low if gated** (`\$0.005–\$0.015` on big moves) | **ADOPT (Gated)** — Trigger *only* when Tier-1 `fold` moves `\|Δlogit\| >= 0.35` or enters tails (`<10%` / `>90%`). |
| **C. Post-Hoc Platt Scaling + Uncertainty-Conditioned Extremizing** | **Medium** | **Low** *(~40 LOC in `fcast-core`)* | `\$0.00` *(pure Rust)* | **DEFER TO TOGGLE** — Wait for `n >= 100` resolved forecasts/domain (`W2`); collapse Platt+extremizing into one ridge-to-identity map + uncertainty-conditioned `d(u)` (`W1`). |
| **D. Market-Anchored Bayesian Blending & Latent Bayesian Surprise** | **High** | **Very Low** *(~20 LOC)* | `\$0.00` *(pure Rust)* | **ADOPT (Unify)** — Precision-weighted logit blending *is* a 1D Kalman filter (`W4`); Gaussian KL surprise (`W3`) cleanly filters deep-book shocks from thin-book noise. |
| **E. Deeper `Eidetic Engine` (`ee`) & `cass --robot` Integration** | **High** | **Low** *(CLI/MCP hooks)* | `\$0.00` *(local Rust indices)* | **ADOPT** — `ee` governs procedural patching/decomposition rules; `cass --robot` supplies zero-bloat episodic agent session recall. |

---

## 2. Three-Document Question Memory (Keeping `Chronicle` Simple)

Right now `Chronicle` stores discrete `ForecastRevision` snapshots, `question_data_packs`, and `EvidenceVault` blobs, which is ideal for replay but underspecifies *how a long-horizon question's narrative context evolves over months* without context rot. We solve this with **three lightweight views over the existing `EvidenceVault` CAS**—zero new storage engines:

1. **Immutable Gold Dossier (`gold_dossier_handle`, written once at `InitialForecast` `seq_no = 0`):**
   * Frozen in `EvidenceVault` and never patched. Captures deep resolution-criteria minutiae, reference-class base rates (`OV`), structural constraints, historical analogues, and key actors. Anchors every future update against gradual goal/criteria drift.
2. **Episodic Event Log (`episodic_events` table, append-only, `available_utc <= as_of_utc`):**
   * Unifies discrete news flashes (`GDELT`/`AskNews`), `question_data_packs` vintage releases, and `BiTemporalAudit` entries into one chronological timeline per `question_id`. Never summarized destructively; every row links to `[ev:sha256#span]`.
3. **Incremental State Representation Document (`state_doc_revisions`, patched by a lightweight Updater):**
   * Compact working brief (`<= 4,000` tokens) holding *current* base rates, active constraints, key indicators, and open cruxes.
   * **Simplicity Rule on Decomposition:** Do **not** prematurely shatter simple binary questions into a web of sub-documents. Split into linked child state docs *only* when **(a)** the question is a multi-leg `Combo`/`GroupOfQuestions` or **(b)** `state_doc` exceeds `4,000` tokens and `ee` signals an active multi-mechanism split.

---

## 3. Gated Red-Team Skill (High-Leverage Safeguard, Bounded Cost)

* **Design:** Before sealing a material probability shift, a dedicated Red-Team skill (`templates/red_team_audit.tmpl`) reads **only three artifacts**: **(1)** the immutable **Gold Dossier**, **(2)** the proposed `ForecastRevision` delta + `evidence_handles`, and **(3)** top `ee` trauma-guard rules for the domain. It checks for contract-rule misreads (`G1`), base-rate neglect, wire-echo overreaction, and ignored contrary cases.
* **Cost Discipline:** Running a red-team pass on every Tier-0 stream tick would triple background LLM spend for negligible gain. Gate it strictly on **material shifts (`|Δlogit| >= 0.35`), tail crossings (`p < 0.10` or `p > 0.90`), or `ForecastBench`/`FutureEval` final freeze issuance**.

---

## 4. Recalibration: Why Platt + Extremizing Must Stay Simple (`W1`, `W2`)

Our numerical witness suite ([`verify_refactoring_math.py`](./verify_refactoring_math.py)) proves two practical constraints that prevent over-engineering:
1. **Algebraic Collapse (`W1`):** Composing Platt scaling `sigmoid(a · logit(p) + b)` with static extremizing `p^d / (p^d + (1 - p)^d)` equals a **single affine logit transform** `sigmoid((d·a) · logit(p) + d·b)`. A separate extremizing step adds value **only** when exponent `d(u)` is conditioned on a per-forecast uncertainty signal `u ∈ [0, 1]` (ensemble spread / Beta variance: `d → 1` under high disagreement, `d > 1` only when independent evidence converges).
2. **Small-Sample Overfit Trap (`W2`):** At `n = 25` resolved forecasts, unregularized 2-parameter Platt beats the identity map only **`12%`** of the time on holdout log-loss (`raw = 0.12`), whereas ridge-to-identity regularization (`a → 1, b → 0`) jumps to **`47%`** (`ridge = 0.47`). Therefore: **keep post-hoc recalibration behind a single-knob flag in `config/ablation.toml`, require `n >= 100` resolved forecasts per domain, and always regularize toward identity**.

---

## 5. Market-Anchored Blending & Latent Bayesian Surprise (`W3`, `W4`)

Rather than adding a second market-blending subsystem alongside TimeSeek (`§11.3`), unify both into one **1D Latent-Logit Kalman Filter** (`~20 LOC` in `fcast-core`, `\$0` LLM cost):
* **Algebraic Equivalence (`W4`):** Two-observation Kalman updating (combining `p_indep` precision with `p_market` precision) is **identically equal** to precision-weighted logit pooling.
* **Liquidity-Scaled Latent Bayesian Surprise (`W3`):** Setting market observation noise $R_t = R_0 / \max(\phi_{\text{liq}}, 10^{-3})$ produces an exact closed-form Gaussian KL **Bayesian Surprise** `KL(posterior ‖ prior)` on every market tick: a `10pp` jump on a deep book (`φ_liq = 0.90`) registers **`0.592 nats`** of surprise (waking a Tier-1 `fold` + Red-Team check), whereas the identical `10pp` jump on a thin book (`φ_liq = 0.05`) registers only **`0.0079 nats`** (`75×` smaller, cleanly ignored!).

---

## 6. Deeper `Eidetic Engine` (`ee`) & `cass` Integration

Keep memory cleanly split across the Franken Suite's three natural cognitive layers—zero duplication:

1. **Episodic Memory (`Chronicle` + `cass --robot`):** `Chronicle` owns *world* episodes (`episodic_events`, `EvidenceVault`), while **`cass` (`Coding Agent Session Search`)** indexes *agent reasoning sessions* across `pi_agent_rust`, `Hermes`, and `Antigravity`. Always invoke **`cass search "<query>" --robot --json`** (never bare `cass`, which opens an interactive TUI) so the Red-Team skill and analysts can recall past domain pitfalls in milliseconds.
2. **Working Memory (`question_state_doc` + `ForecastRevision`):** Active, compact, patched incrementally per `question_id`.
3. **Procedural Memory (`Eidetic Engine` `ee` Playbooks & Rules):** Distills `postmortem_ee_distill.tmpl` outcomes and Red-Team catches into maturity-staged rules (`candidate → established → proven`, with confidence decay) exported via `ee playbook`. `ee` advises the State-Doc updater on *what to append vs. ignore*, *when a question warrants sub-document decomposition*, and which `Trauma Guard` pitfalls to inject before every major update.

---

## 7. Top 5 Recommended Improvements to the Main Plan (`COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md`)

Ranked by **epistemic return per unit of complexity**:

1. **Add the 3-Document Question Memory Overlay (`Gold Dossier` + `Episodic Log` + `State Doc`) in `§7` / `§11`:** Eliminates long-horizon context drift across multi-month `FutureEval` and `ForecastBench` questions using existing `EvidenceVault` handles.
2. **Add Gated Red-Team Pre-Update Auditing (`|Δlogit| >= 0.35` or Tail Crossings) in `§9.2` / `§11`:** Catches `G1` contract misreads and wire-echo overreactions against the frozen Gold Dossier at minimal token cost.
3. **Unify Post-Seal TimeSeek Blending with Latent-Logit Kalman Bayesian Surprise in `§11.3`:** Replaces ad-hoc shock heuristics with a `20`-LOC liquidity-scaled KL surprise detector (`W3`/`W4`) at `\$0` API cost.
4. **Add `cass --robot` Read-Only Recall Alongside `ee` Procedural Playbooks in `§14`:** Gives `Tether` skills instant cross-session recall of past forecasting mistakes without bloating `ops.fsqlite`.
5. **Gate Ridge-Regularized Platt + Uncertainty-Conditioned Extremizing Behind `config/ablation.toml` (`n >= 100`) in `§12`:** Prevents small-sample calibration overfit (`W2`) while letting `MiniBench` / `ForecastBench` ablations prove real log-score gains before default activation.
