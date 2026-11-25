# FrankenForecast (`fcast`) — Delivery Log

> **Governance Rule (`§0.7` Rule 1 & `§0.2`):** Live execution tracking, milestone state transitions, and signed gate-receipt references MUST reside exclusively in `docs/delivery/`. Never embed sprint burndowns, task checkboxes, or mutable delivery status inside `COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md`. Closing a task or issue is NEVER equivalent to formal acceptance; acceptance requires all eight conditions of `§0.2` plus a verified `receipts/GATE-XX.json` artifact.

## Current Repository Maturity

* **Master Specification Version:** `1.2` (`2026-11-25`)
* **Overall Implementation Status:** `Tier 0 (Plan-Only)` for the 6-crate Rust Cargo workspace (`0 / 6` crates compiled); `Tier 1 (Contract-Proved)` for the canonical immutable `ForecastRevision` 12-field schema + 4-stage lifecycle, full-universe anti-selection-bias `universe_question_ledger` + 10-metric scorecard + `QuestionMetaForecast` estimator, SQLite WAL/backup/migration contracts, rationale staging, log-score VoI stopping, conjugate decay, and document hygiene (`scripts/verify_math_and_contracts.py`, `scripts/verify_plan_hygiene.py`).

## Gate Receipt Ledger (`GATE-01` .. `GATE-24`)

| Gate ID | Primary Claim | Subsystem | Tier | Receipt Artifact | Status |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `GATE-01` .. `GATE-24` | `CLAIM-01` .. `CLAIM-08` | `SUB-01` .. `SUB-08` | `Tier 0` | *Pending `M0..M5` execution* | `UNIMPLEMENTED` |

## Chronological Delivery Entries

* **2026-11-25 — `PLAN-v1.2` Meta-Forecasting Pane & Full-Universe Anti-Selection-Bias Telemetry:** Added `QuestionMetaForecast` (*"Forecast the Forecasts"*: cost, latency, popularity, complexity), irreversible `universe_question_ledger` (tracking full eligible universe, exclusion & abandonment rates, and sunk spend), all 10 mandatory dual-denominator system metrics (`§12.2–§12.4`), and `Cockpit` View `[6:Meta]` (`§16`), verified in SQLite WAL by `scripts/verify_math_and_contracts.py` and `scripts/verify_plan_hygiene.py`.

* **2026-11-25 — `PLAN-v1.1` Canonical `ForecastRevision` & Pure-CPU Scope Refinement:** Removed local GPU sidecar component (`0` GPU architecture on macOS + Linux + `4 TB` NVMe SSD), extracted multi-agent swarm simulation into sister project `franken_wargame` (`6`-crate pure-forecasting DAG, `8` canonical subsystems `SUB-01..08`), and established the immutable 12-field `ForecastRevision` central data contract + 4-stage lifecycle separation (`§7`), verified in SQLite WAL by `scripts/verify_math_and_contracts.py`.
* **2026-11-25 — `PLAN-v1.0` Baseline Sealed:** Initialized repository with `COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md`, frozen `templates/bitemporal_event_audit.tmpl`, `scripts/verify_plan_hygiene.py`, and `scripts/verify_math_and_contracts.py`.
