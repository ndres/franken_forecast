# Current Project Evaluation — `franken_forecast` (`fcast`)

* **Date:** `2026-11-25`
* **Evaluated Commit:** `fbfd4fc` (`main`, `Plan v2.0`, `M0` Rust workspace bootstrapped)
* **Purpose:** An unvarnished, evidence-based audit of the current repository structure, what is genuinely working vs. what is missing or over-claimed against `§0.2`, how to tighten the engineering process, and a concrete multi-layer testing blueprint before advancing to `M1`.

---

## 1. Honest Snapshot of Where the Repo Stands Today

| Dimension | Measured Current State | Assessment |
| :--- | :--- | :--- |
| **Specification & Formal Contracts** | `COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md` (`v2.0`, 780+ lines), `REFACTORING_PROPOSAL.md`, `lean_experiments/Claim05Spec.lean`, `9` SHA-256-locked `templates/*.tmpl`, `4` passing Python verifiers | **Very Strong** — unusually rigorous data contracts (`ForecastRevision`), anti-selection-bias math, and formal auditability. |
| **Rust Implementation Footprint** | **`2,115` total Rust LOC** across 6 crates: `fcast-core` (`1,018`), `fcast-infra` (`653`), `fcast-app` (`391`), `fcast-ingest` (`22`), `fcast-epistemic` (`14`), `fcast-bench` (`17`) | **Early Foundation (`M0`)** — compiles cleanly (`0` Clippy warnings, `#![forbid(unsafe_code)]`) on `nightly-2026-08-31` against upstream `fsqlite::blocking`, but `L2`–`L4` crates are still stubs. |
| **Automated Rust Test Suite** | **`13` passing tests** (`8` in `fcast-core` incl. 1 `10,000`-case `proptest`, `2` in `fcast-infra`, `3` integration tests in `fcast-app`) | **Thin** — proves the happy path & key invariants, but falls short of full `§0.2` rigor on several claimed gates (see §2). |
| **CI / Automation / Dev Ergonomics** | **No `.github/workflows/ci.yml`**, no `justfile`/`Makefile`, no `cargo-deny` or `cargo-nextest` run yet, no `config/ablation.toml`, `.DS_Store` present | **Weak** — all verification is currently manual; nothing enforces regressions automatically on push/PR. |

---

## 2. Brutally Honest Rundown: What Is Missing or Over-Claimed

Applying our own **`§0.2` (*What "Implemented" Means*)** rules strictly against the current repo reveals **eight concrete gaps** that should be closed before moving to `M1`:

### A. `§0.2` Strictness Gaps on `M0` Gates
1. **Transitive Dependency Hygiene Not Yet Audited (`§0.2` Rule 2 / `GATE-01`):**
   * `deny.toml` is committed, and `workspace_topology_dag.rs` checks direct `Cargo.toml` text for banned crates—**but `cargo deny check` has not actually been installed or executed yet.** A transitive dependency inside `fsqlite` or another crate could silently pull in a banned crate without `workspace_topology_dag.rs` noticing.
   * *Fix:* Replace string-matching in `workspace_topology_dag.rs` with `cargo metadata` JSON graph traversal, and run `cargo deny check bans advisories licenses sources` in CI.
2. **JSON Schema Exists, But Zero Tests Validate Against It (`§0.2` Rule 3):**
   * `schemas/v1/forecast_revision.schema.json` was committed, yet neither Rust tests nor Python verifiers currently validate serialized `ForecastRevision` outputs against it.
3. **Receipts Are Bundled & Static Rather Than Generated Per-Gate (`§0.2` Rule 7):**
   * `receipts/M0-BUNDLE.json` was written manually as a summary file instead of being emitted programmatically per gate (`receipts/GATE-01.json`, `receipts/GATE-02.json`, ...) by `fcast verify-receipts` after executing real checks.
4. **Incomplete Coverage Inside `M0` Gates:**
   * **`GATE-02` (Migrations & Upcaster):** Currently tests `0001_init.sql` + `Zstd-9` backup, but does **not** yet test a real `0001 -> 0002` forward migration or lossless `V1 -> V2` `ForecastRevision` payload upcast.
   * **`GATE-03` (MVCC Concurrency & Parquet Offload):** Still unimplemented (`Tier 0/1`); `Tier W1` Parquet writer does not exist yet.
   * **`GATE-09` (`SafeExternalFetch`):** Tests pure URL/IP/XML/cap policy functions in isolation, not an actual HTTP transport performing connect-time `SocketAddr` pinning (`ureq` + `rustls`).
   * **`fcast-infra::jobs` (`DurableJobStore`):** The SQL tables (`fcast_jobs`, `daemon_budget_ledger`, `domain_events`) exist in `0001_init.sql`, but the Rust methods to enqueue, coalesce, dequeue by `P0..P4` priority, recover expired leases, and quarantine `DEAD_LETTER` jobs are not written yet.

### B. Operational, Reproducibility & Repo Hygiene Gaps
5. **No CI Pipeline (`.github/workflows/ci.yml`):**
   * Neither the 4 Python spec/math/Lean verifiers nor `cargo clippy -- -D warnings` / `cargo test` run automatically on GitHub pushes.
6. **Unpinned Upstream Git Revision in `Cargo.toml`:**
   * `fsqlite = { git = "https://github.com/Dicklesworthstone/frankensqlite" }` tracks floating `main` (pinned only in `Cargo.lock`). Adding an explicit `rev = "80083505..."` prevents accidental breakage on `cargo update`.
7. **Missing `config/ablation.toml`:**
   * Referenced throughout `§5.1` and `§12.7` as the single-knob feature toggle file, but not yet created in the repository.
8. **Missing Cross-Language Golden-Vector Parity (`§0.2` Rule 6):**
   * Python reference math (`scripts/verify_math_and_contracts.py`, `docs/proposals/verify_refactoring_math.py`) and Rust math (`crates/fcast-core`) both pass independently, **but no automated test proves they agree to `<= 1e-9` on identical inputs.**
9. **Minor Repo Hygiene:**
   * `.DS_Store` files sit in root and `docs/` (`.gitignore` should exclude `.DS_Store`), and there is no `justfile`/`Makefile` for single-command verification.

---

## 3. How to Improve the Engineering Process

1. **Freeze Spec Expansion — Shift to Code-First, Falsification-Driven Updates:**
   * `COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md` (`v2.0`) is now mature enough to build from for months. Stop expanding plan prose unless a real implementation benchmark or test falsifies an assumption (`§0.8`).
2. **Enforce Tight "One-Gate, One-PR" Vertical Slices:**
   * Never land 5 subsystems or 6 gates in one giant commit. Each slice should follow a strict loop:
     1. Write/extend the failing gate test (`crates/fcast-app/tests/gate_XX_*.rs`).
     2. Implement the minimal pure-Rust module to pass it cleanly (`0` Clippy warnings, `0` `unwrap()`).
     3. Run `just verify` (all Rust + Python checks).
     4. Emit `receipts/GATE-XX.json` via `fcast verify-receipts` and update `docs/delivery/DELIVERY_LOG.md` *only* after CI is green.
3. **Add a `justfile` + GitHub Actions CI Immediately (Before `M1`):**
   * Standardize local and CI commands so every check runs identically everywhere:
     * `just lint` → `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`
     * `just test` → `cargo nextest run --workspace` (or `cargo test --workspace`)
     * `just deny` → `cargo deny check bans advisories licenses sources`
     * `just spec` → all 4 Python verifiers (`verify_plan_hygiene`, `verify_math_and_contracts`, `verify_audit_flags`, `verify_refactoring_math`)
     * `just verify` → runs all of the above + `fcast verify-receipts`
4. **Pin Exact Upstream Git SHAs (`rev = "..."`) for Franken Suite Crates:**
   * Keep `.cargo/config.toml` HEAD-only shallow `gitoxide` fetching active, and pin every Franken Suite git dep (`frankensqlite`, `asupersync`, `franken_numpy`, etc.) to an explicit commit SHA updated deliberately alongside `rust-toolchain.toml`.
5. **Use `beads_rust` (`br`) or Keep `DELIVERY_LOG.md` Minimal & Automated:**
   * Either install `br` and initialize `.beads/` so task tracking matches `§0.7` Rule 1, or keep `docs/delivery/DELIVERY_LOG.md` as the sole ledger and remove unexercised `br` ceremony until needed.

---

## 4. Concrete Testing Blueprint (How to Implement Tests Across All 6 Layers)

To reach genuine `Tier 2 (Crate-Verified)` and `Tier 3 (Tournament-Calibrated)` confidence without flaky network tests or slow feedback loops, structure the test suite into **six crisp layers**:

### Layer 1: High-Volume Property Tests (`proptest`, `10,000` cases/module in `fcast-core`)
Currently only `project_martingale_crux` has a `10,000`-case `proptest`. Add `10,000`-case property suites for every numerical invariant:
* **`MetaculusCdfBuilder`:** For any sorted finite quantile anchors, open/closed bounds, linear or `zero_point` log scale, assert `len == 201` (or `inbound_outcome_count + 1`), boundary rules (`0.0`/`>=0.001`, `1.0`/`<=0.999`), and `5e-5 <= cdf[i] - cdf[i-1] <= 0.59`.
* **`ConjugateBelief::stream_step`:** Assert `"Still"` invariance (`Δμ == 0`, `ΔN == 0` when `dt == 0`), strict `Up` monotonicity (`μ_post > μ_pre`), and `μ_post ∈ (0, 1)`, `N_post > 0`.
* **`DirichletCategoricalBelief::step`:** Assert simplex sum `|∑ p_k - 1| < 1e-12` and strict `"Still"` invariance.
* **`pav_isotonic_nondecreasing`:** Assert non-decreasing output, idempotence (`pav(pav(x)) == pav(x)`), and mean preservation (`mean(pav(x)) == mean(x)`).
* **`kalman_market_blend`:** Assert `bayesian_surprise_nats >= 0.0` and strict monotonicity in `φ_liq` for fixed `|p_market - p_indep| > 0`.
* **`AttributionWaterfall`:** Assert `|logit(p_next) - logit(p_prev) - sum(components)| < 1e-10` on interior probabilities.

### Layer 2: Cross-Language Golden-Vector Parity (`tests/golden/math_vectors.json`)
* Add a small generator mode in `scripts/verify_math_and_contracts.py` (`--emit-golden tests/golden/math_vectors.json`) that dumps exact `(inputs, expected_outputs)` for:
  * `stream_belief_step`, `should_stop_fold`, `prior_shift_prob`, `build_metaculus_cdf`, `pav_isotonic_nondecreasing`, `eb_shrink_base_rate`, `effective_model_count`, `kalman_market_blend`, and `solve_two_way_fe_brier`.
* Add a Rust integration test `crates/fcast-core/tests/golden_parity.rs` that loads `tests/golden/math_vectors.json` and asserts every single Rust output matches Python to **`<= 1e-9`** (`§0.2` Rule 6).

### Layer 3: JSON Schema & Wire-Contract Validation (`schemas/v1/*.schema.json`)
* Add a test (in Rust via `jsonschema` dev-dep or in Python CI) asserting that:
  * Serialized `ForecastRevision` JSON from `fcast-core` validates against `schemas/v1/forecast_revision.schema.json`.
  * `fcast init --json` and `fcast verify-receipts` outputs match committed CLI output schemas.
  * Every `templates/*.tmpl` file passes `MarketBlindingScrubber::assert_blind_payload` for market-blind templates (`rationale_staged_turn_a`, `stream_likelihood_tick`, `belief_fold_step`, `red_team_audit`).

### Layer 4: Storage, Migration, Outbox & Crash-Recovery Tests (`crates/fcast-infra/tests/`)
* **`migration_and_upcast.rs` (`GATE-02`):** Open DB at `0001`, insert `V1` `ForecastRevision`, take verified `Zstd-9` backup, apply `0002` migration, verify `V1 -> V2` lossless read upcast without mutating `forecast_revisions` rows.
* **`jobs_and_outbox.rs` (`§13.2`, `GATE-19`):** Verify `BLAKE3` idempotency deduplication, `domain_events` burst coalescing, `P0 > P1 > P2 > P3` priority ordering, stale `RUNNING` lease reclamation after simulated crash, `attempts >= 3 -> DEAD_LETTER`, and `\$5.00/day` spend circuit breaker rejection.
* **`mvcc_concurrency.rs` (`GATE-03`):** Spawn `4` concurrent reader/writer threads against `OperationalStore` (each with its own thread-local `fsqlite::blocking::Connection`) and assert `0` corrupt pages and `PRAGMA integrity_check == "ok"`.

### Layer 5: Offline Deterministic Cassettes (`TemporalFence` / `ReplayAsOf` for `M1`–`M3`)
* **Rule:** `cargo test` MUST NEVER hit live Metaculus, AskNews, Polymarket, or Gemini APIs.
* Store sanitized JSON HTTP/LLM fixtures under `tests/cassettes/` with explicit `available_utc` timestamps.
* Wire `Harvester`, `Notary`, and `Loom` against a `HttpTransport` / `LlmTransport` trait so unit & integration tests replay cassettes deterministically in milliseconds, proving `INV-03` (`available_utc <= as_of_utc`), `INV-04` (`0` market tokens in Turn A / pre-seal `Loom`), and `GATE-07..18` offline.

### Layer 6: Headless TUI & PTY Handoff Tests (`M4` — `GATE-22`, `GATE-23`)
* Render `FrankenTUI` Views `[1..7]` into an in-memory terminal buffer across `2,000` synthetic contracts, asserting `p99 < 8.0 ms`, `0` Braille color-cell collisions, and responsive layout integrity at `140`, `100`, and `80` columns.
* Run `50` consecutive PTY handoff cycles verifying `0` leaked file descriptors and `< 50 ms` repaint.

---

## 5. Recommended Next Action (Tight "M0.5 Hardening" Sprint Before `M1`)

Before opening `M1`, spend **one focused hardening pass** closing the `M0` gaps above:
1. Add `.github/workflows/ci.yml`, `justfile`, `config/ablation.toml`, and `.DS_Store` to `.gitignore`.
2. Pin `frankensqlite` to its exact commit `rev` in `Cargo.toml` and run `cargo deny check`.
3. Implement `fcast-infra::jobs` (`DurableJobStore` + `domain_events` consumer) and `tests/golden/math_vectors.json` + `golden_parity.rs` (`<= 1e-9` Python↔Rust parity) + JSON Schema validation + `proptest` expansion across `fcast-core`.
4. Upgrade `fcast verify-receipts` to emit genuine per-gate receipts (`receipts/GATE-01.json`, `GATE-02.json`, ...).

Once those 4 items are green in CI, the foundation is genuinely bulletproof for **`M1`** (`Harvester` + `Metronome` + `Notary`).
