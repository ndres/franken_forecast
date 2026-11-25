# FrankenForecast (`franken_forecast`)

> **Honest Repository Status (`Plan v1.3` · `2026-11-25`):** This is currently a **planning, formal-spec, and contract repository** (`Tier 0: Plan-Only` for Rust Cargo crates; `Tier 1: Contract-Proved` for the canonical `ForecastRevision` lifecycle, full-universe anti-selection-bias 10-metric scorecard & meta-forecaster, `CLAIM-05` (`HYP-05A..05F`) AlphaProof-Code-style Lean 4 specification draft in [`lean_experiments/`](./lean_experiments/), SQLite immutability/WAL/backup/migration schemas, and document hygiene). **Zero Rust workspace crates are compiled in this repository yet.** See `§0.2` of [`COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md`](./COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md) for the eight mandatory conditions required before any subsystem may be marked *Implemented*.

**FrankenForecast (`fcast`)** is a CPU-complete, pure-Rust geopolitical, macroeconomic, and cyber-risk probabilistic forecasting workstation, canonical `ForecastRevision` ledger, vintage-safe telemetry warehouse, hindsight-blind community-rationale judge, background job daemon, Model Context Protocol (MCP) server, and **ForecastBench** tournament competitor built on **Eidetic Engine (`ee`)** and the **Franken Suite** (`FrankenSQLite`, `FrankenTUI`, `Frankensearch`, `FrankenNetworkX`, `Asupersync`).

> **Companion Project Boundary:** Multi-agent geopolitical swarm wargaming (`franken_wargame`) is maintained as a **separate sister project** (`§6.2` `NGOAL-06`, `§14.3` `INT-06`) that consumes `franken_forecast`'s immutable `ForecastRevision` snapshots and `EvidenceVault` handles strictly over read-only MCP/Parquet interfaces—never sharing write access to `ops.fsqlite`.

---

## Core Instruments (`§0.3`)

| Instrument | Invocation | Role |
| :--- | :--- | :--- |
| **`fcast`** | `fcast <verb> --json` | Deterministic CLI surface for ingestion, canonical `ForecastRevision` issuance/inspection, horizon projection, bi-temporal verification, rationale scoring, two-speed belief folding, FutureSearch-class epistemic ops, and ForecastBench execution. |
| **`fcastd`** | `fcast daemon run` | Persistent `Asupersync` 4-lane background job daemon (`launchd` on macOS, `systemd --user` on Linux) with crash-safe SQLite visibility leases, `0600` Unix Domain Socket bus, and daily USD spend circuit breakers. |
| **`fcast-mcp`** | `fcast mcp serve` | First-class Model Context Protocol server (`stdio` with `dup`/`dup2` FD quarantine + loopback `Streamable HTTP`) exposing synchronous epistemic tools, async `fcastd` job handles, `fcast://` resource subscriptions, and parameterized prompts. |
| **`fcast-verify`** | `fcast verify-receipts` | Offline cryptographic auditor verifying `ForecastRevision` hash-chains & immutability, WAL backup chains, `_fcast_migrations` SHA-256 locks, `templates/bitemporal_event_audit.tmpl` byte parity, `TemporalFence` cassettes, and `GATE-*` receipts. |
| **`fcast-tui`** | `fcast tui` | Sub-16ms `FrankenTUI` 7-view terminal cockpit featuring 3-curve Braille crosshairs, 6-strip synchronized cross-venue comparison, coexisting `24h`/`7d` cadence deltas, a dedicated `[7:Revisions]` horizon-audit ledger, and atomic `[H]` PTY harness handoff (`pi_agent_rust`, `Hermes`, `Antigravity`). |

---

## Central Data Contract (`§7`) & Canonical Subsystems (`§0.5`)

At the center of every subsystem sits the immutable **`ForecastRevision`** record (`forecast_id`, `revision_id`, `question_id`, `as_of_utc`, `p_indep`, `p_market`, `probability_distribution`, `evidence_handles`, `model_version`, `calibration_version`, `rationale_handle`, `resolution_status`), strictly separating **(1) original issuance (`InitialForecast`)**, **(2) longitudinal updates (`SubsequentRevision`)**, **(3) point-in-time evaluation slices (`HorizonForecastView`)**, and **(4) disjoint adjudication (`QuestionResolution`)**.

1. **`Chronicle` (`SUB-01`)** — WAL/MVCC `OperationalStore`, append-only `forecast_revisions` + disjoint `question_resolutions`, online `zstd` hot backups, two-layer migrations, content-addressed `EvidenceVault`, Parquet tick archiver, and `ee_outbox` bridge.
2. **`Harvester` (`SUB-02`)** — Stoikov top-of-book microprice + tick-aware sweep impact (`φ_liq`), ForecastBench human/community median backfill, `available_utc` data packs with Horvitz-Thompson lag correction, `symphonia` 24-minute verbatim audio chunking, and SSRF-hardened `SafeExternalFetch`.
3. **`Notary` (`SUB-03`)** — Verbatim bi-temporal `event_utc` vs. `published_utc` union-envelope verifier, mechanical 2-turn staged-reveal `6D+2G` (`CT/EI/OV/ME/EJ/PR` + `G1/G2/unscoreable`) rationale scorer, and `MarketBlindingScrubber`.
4. **`Loom` (`SUB-04`)** — 4-layer `fnx-*` causal Knowledge Graph and Two-Speed Dynamic Belief Engine (Tier-0 `ConjugateBelief` + Kulhavý–Zarrop `DecayController` + EvolveCast `κ_LR` + KL-gated cache escalating to Tier-1 CAS-handle PBM `fold`, Martingale KL `FoldStoppingController`, `0..5` `SEIPH`, TimeSeek lifecycle deferral, King-Zeng `arXiv:2610.04798` cyber calibrator, and `map`/`decision`/`scenario`/`decompose`) sealing every output as an immutable `ForecastRevision`.
5. **`Crucible` (`SUB-05`)** — Pre-dispatch Question Meta-Forecaster (*"Forecast the Forecasts"*: cost, latency, popularity, complexity), irreversible `universe_question_ledger` (tracking the entire eligible universe plus exclusion & abandonment rates), selection-bias-free 10-metric scorecard (`HorizonForecastView ⋈ QuestionResolution`), ForecastBench solver, and leak-free `dspy.GEPA` Pareto optimizer.
6. **`Metronome` (`SUB-06`)** — `Asupersync` background job scheduler/worker (`fcastd`), `O_CLOEXEC` singleton lease, `fcast_jobs` visibility recovery, `daemon.sock` push IPC, and `daemon_budget_ledger` spend governor.
7. **`Tether` (`SUB-07`)** — Hardened MCP server (`fcast-mcp`), async job handles, `fcast://` live resource subscriptions (including `franken_wargame` read-only snapshot feeds), and static-linted compiled skill packs for `pi_agent_rust`, `Hermes`, and `Antigravity`.
8. **`Cockpit` (`SUB-08`)** — `FrankenTUI` 7-view visual triage dashboard (featuring `[6:Meta]` *"Forecast the Forecasts"* & Full-Universe Scorecard and `[7:Revisions]` longitudinal audit ledger), progressive `fcast init` / `DoctorProbe` onboarding, and atomic 4-phase PTY harness handoff.

---

## Target Hardware & OS Envelope (`§5` & `§19`)

* **Operating Systems:** First-class native CPU execution on **macOS 15+ (`aarch64-apple-darwin` Apple Silicon)** and **Linux Kernel 6.1+ (`x86_64-unknown-linux-gnu` / `aarch64-unknown-linux-gnu`)** (`0` GPUs required or assumed).
* **Storage Baseline:** **4 TB (`4,000 GB`) NVMe SSD** partitioned across six explicit tiers: hot MVCC operational state (`ops.fsqlite` + `forecast_revisions`), content-addressed `EvidenceVault`, multi-year columnar Parquet L2 orderbook ticks, point-in-time `available_utc` macro/conflict/cyber vintages, 25,000-hour Opus audio + `ReplayAsOf` cassettes, and rolling `zstd` WAL backups (`10%` wear-leveling reserve preserved).

---

## Verification

```bash
python3 scripts/verify_plan_hygiene.py
python3 scripts/verify_math_and_contracts.py
python3 lean_experiments/verify_audit_flags.py
```
