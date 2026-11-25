# FrankenForecast (`fcast`) — Delivery Log

> **Governance Rule (`§0.7` Rule 1 & `§0.2`):** Live execution tracking, milestone state transitions, and signed gate-receipt references MUST reside exclusively in `docs/delivery/`. Never embed sprint burndowns, task checkboxes, or mutable delivery status inside `COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md`. Closing a task or issue is NEVER equivalent to formal acceptance; acceptance requires all eight conditions of `§0.2` plus a verified `receipts/GATE-XX.json` artifact.

## Current Repository Maturity

* **Master Specification Version:** `1.6` (`2026-11-25`)
* **Overall Implementation Status:** `Tier 0 (Plan-Only)` for the 6-crate Rust Cargo workspace (`0 / 6` crates compiled); `Tier 1 (Contract-Proved)` for the canonical immutable `ForecastRevision` 12-field schema + 4-stage lifecycle, Metaculus `201`-point/discrete CDF & FutureEval contract math, 8 SHA-256-locked canonical `.tmpl` prompt templates (`templates/`), `CLAIM-05` (`HYP-05A..05F`) AlphaProof-Code-style Lean 4 specification & audit harness (`lean_experiments/`), full-universe anti-selection-bias `universe_question_ledger` + 10-metric scorecard + `QuestionMetaForecast`, SQLite WAL/backup/migration contracts, and document hygiene (`scripts/verify_*.py`, `lean_experiments/verify_audit_flags.py`).

## Gate Receipt Ledger (`GATE-01` .. `GATE-24`)

| Gate ID | Primary Claim | Subsystem | Tier | Receipt Artifact | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `GATE-01` .. `GATE-24` | `CLAIM-01` .. `CLAIM-08` | `SUB-01` .. `SUB-08` | `Tier 0` | *Pending `M0..M5` execution* | `UNIMPLEMENTED` |

## Chronological Delivery Entries

* **2026-11-25 — `PLAN-v1.6` Metaculus FutureEval Fall 2026 Optimization & Rust `forecasting-tools` Client Port:** Added FutureEval Fall 2026 (`Notebook 45615`) tournament playbook, Google participant credits / Metaculus LLM Proxy / AskNews seasonal renewal onboarding (`§5.2`), pure-Rust port of `Metaculus/forecasting-tools` (`MetaculusQuestion` hierarchy, `ApiFilter`, `MetaculusCdfBuilder` enforcing `201`/discrete CDF + `zero_point` log-scale + `5e-5 <= Δcdf <= 0.59`, `MonetaryCostManager`, `AskNewsSearcher`, `Benchmarker` Peer/Baseline/Spot Peer scoring in `§12.5`), and executable CDF/FutureEval contract tests.

* **2026-11-25 — `PLAN-v1.5` Plan Simplification & Metaculus Notebook `43497` Bot Patterns:** Removed all `franken_wargame` references, NVMe/hardware-sizing tables, and removed `Plan-Only` / `Pre-Cutoff` defect entries; condensed `§0.6`; and integrated five production bot patterns from Metaculus Notebook `43497` (`§5`: multi-vendor median ensembling, disagreement-triggered deep dives, multi-provider parallel search + PDF/CSV parsing, single-knob modular ablations, and `MiniBench` + defensive tail-clipping).

* **2026-11-25 — `PLAN-v1.4` Canonical Variable Alignment & 8-Template Epistemic Prompt Suite (`templates/*.tmpl`):** Revised `bitemporal_event_audit.tmpl` to bind canonical `§7.1`/`§7.2` identifiers (`as_of_utc`, `event_utc`, `published_utc`, `available_utc`, `question_id`, `resolution_status`, `resolved_published_utc`, `adjudication_ev_handle`, `ev_handle`) and added 7 SHA-256-locked companion templates (`rationale_staged_turn_a/b`, `stream_likelihood_tick`, `belief_fold_step`, `seiph_process_critic`, `meta_forecast_triage`, `postmortem_ee_distill`) with automated variable-blinding enforcement in `verify_plan_hygiene.py`.

* **2026-11-25 — `PLAN-v1.3` `CLAIM-05` Subordinate Hypotheses (`HYP-05A..05F`) & AlphaProof-Code Lean 4 Specification Experiment:** Preserved `CLAIM-05` as the umbrella epistemic skill claim decomposed into `HYP-05A..05F`, authored reference pseudocode + Lean 4 specification (`lean_experiments/Claim05Spec.lean`) + critical TCB "Closer Look" audit report (`lean_experiments/LEAN_EXPERIMENTS.md`), and repaired the `HYP-05E` naive-clamp Martingale feasibility bug uncovered during Lean formalization (`lean_experiments/verify_audit_flags.py`).

* **2026-11-25 — `PLAN-v1.2` Meta-Forecasting Pane & Full-Universe Anti-Selection-Bias Telemetry:** Added `QuestionMetaForecast` (*"Forecast the Forecasts"*: cost, latency, popularity, complexity), irreversible `universe_question_ledger` (tracking full eligible universe, exclusion & abandonment rates, and sunk spend), all 10 mandatory dual-denominator system metrics (`§12.2–§12.4`), and `Cockpit` View `[6:Meta]` (`§16`), verified in SQLite WAL by `scripts/verify_math_and_contracts.py` and `scripts/verify_plan_hygiene.py`.

* **2026-11-25 — `PLAN-v1.1` Canonical `ForecastRevision` & Pure-CPU Scope Refinement:** Removed local GPU sidecar component (`0` GPU architecture on macOS + Linux + `4 TB` NVMe SSD), extracted multi-agent swarm simulation into sister project `franken_wargame` (`6`-crate pure-forecasting DAG, `8` canonical subsystems `SUB-01..08`), and established the immutable 12-field `ForecastRevision` central data contract + 4-stage lifecycle separation (`§7`), verified in SQLite WAL by `scripts/verify_math_and_contracts.py`.
* **2026-11-25 — `PLAN-v1.0` Baseline Sealed:** Initialized repository with `COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md`, frozen `templates/bitemporal_event_audit.tmpl`, `scripts/verify_plan_hygiene.py`, and `scripts/verify_math_and_contracts.py`.
