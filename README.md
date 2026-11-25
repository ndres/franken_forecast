# FrankenForecast (`franken_forecast`)

> **Honest Repository Status (`Plan v1.0` · `2026-11-25`):** This is currently a **planning-and-contract repository** (`Tier 0: Plan-Only` for Rust Cargo crates; `Tier 1: Contract-Proved` for mathematical invariants, SQLite WAL/backup/migration schemas, rationale staging, log-score VoI stopping, conjugate decay, and document hygiene). **Zero Rust workspace crates are compiled in this repository yet.** See `§0.2` of [`COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md`](./COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md) for the eight mandatory conditions required before any subsystem may be marked *Implemented*.

**FrankenForecast (`fcast`)** is a pure-Rust geopolitical, macroeconomic, and cyber-risk probabilistic forecasting workstation, vintage-safe telemetry warehouse, hindsight-blind community-rationale judge, background job daemon, Model Context Protocol (MCP) server, and **ForecastBench** tournament competitor built on **Eidetic Engine (`ee`)** and the **Franken Suite** (`FrankenSQLite`, `FrankenTUI`, `Frankensearch`, `FrankenNetworkX`, `Asupersync`).

---

## Core Instruments (`§0.3`)

| Instrument | Invocation | Role |
| :--- | :--- | :--- |
| **`fcast`** | `fcast <verb> --json` | Deterministic CLI surface for ingestion, bi-temporal verification, rationale scoring, two-speed belief folding, FutureSearch-class epistemic ops, and ForecastBench execution. |
| **`fcastd`** | `fcast daemon run` | Persistent `Asupersync` 4-lane background job daemon (`launchd` on macOS, `systemd --user` on Linux) with crash-safe SQLite visibility leases, `0600` Unix Domain Socket bus, and daily USD spend circuit breakers. |
| **`fcast-mcp`** | `fcast mcp serve` | First-class Model Context Protocol server (`stdio` with `dup`/`dup2` FD quarantine + loopback `Streamable HTTP`) exposing synchronous epistemic tools, async `fcastd` job handles, `fcast://` resource subscriptions, and parameterized prompts. |
| **`fcast-verify`** | `fcast verify-receipts` | Offline cryptographic auditor verifying WAL backup chains, `_fcast_migrations` SHA-256 locks, `templates/bitemporal_event_audit.tmpl` byte parity, `TemporalFence` hindsight-blinding cassettes, and `GATE-*` receipts. |
| **`fcast-tui`** | `fcast tui` | Sub-16ms `FrankenTUI` 7-view terminal cockpit featuring 3-curve Braille crosshairs, 6-strip synchronized cross-venue comparison, coexisting `24h`/`7d` cadence deltas, and atomic `[H]` PTY harness handoff (`pi_agent_rust`, `Hermes`, `Antigravity`). |

---

## Canonical Subsystems (`§0.5`)

1. **`Chronicle` (`SUB-01`)** — WAL/MVCC `OperationalStore`, online `zstd` hot backups, two-layer `belief_revisions` migrations, content-addressed `EvidenceVault`, and `ee_outbox` institutional memory bridge.
2. **`Harvester` (`SUB-02`)** — Stoikov top-of-book microprice + tick-aware sweep impact (`φ_liq`), ForecastBench human/community median backfill, `available_utc` data packs with Horvitz-Thompson lag correction, `symphonia` 24-minute verbatim audio chunking, and SSRF-hardened `SafeExternalFetch`.
3. **`Notary` (`SUB-03`)** — Verbatim bi-temporal `event_utc` vs. `published_utc` union-envelope verifier, mechanical 2-turn staged-reveal `6D+2G` (`CT/EI/OV/ME/EJ/PR` + `G1/G2/unscoreable`) rationale scorer, and `MarketBlindingScrubber`.
4. **`Loom` (`SUB-04`)** — 4-layer `fnx-*` causal Knowledge Graph and Two-Speed Dynamic Belief Engine (Tier-0 `ConjugateBelief` + Kulhavý–Zarrop `DecayController` + EvolveCast `κ_LR` + KL-gated cache escalating to Tier-1 CAS-handle PBM `fold`, Martingale KL `FoldStoppingController`, `0..5` `SEIPH`, TimeSeek lifecycle deferral, King-Zeng `arXiv:2610.04798` cyber calibrator, and `map`/`decision`/`scenario`/`decompose`).
5. **`Crucible` (`SUB-05`)** — ForecastBench solver (`Combo` signed Fréchet bounds, geometry-aware horizons, VoI Batch router, TimeSeek lifecycle checkpoint scoring) and leak-free `PurgedChronoSplitter` + `ReplayAsOf` `dspy.GEPA` Pareto optimizer.
6. **`Wargame` (`SUB-06`)** — Compile-time `ReadOnlyStore`-isolated (`L3_HYPOTHETICAL`) 4-phase commit-reveal `Referee`, Hegselmann-Krause `CrowdField`, `8×5×5` stratified branching, and 5-step `ExternalMiroFishAdapter`.
7. **`Metronome` (`SUB-07`)** — `Asupersync` background job scheduler/worker (`fcastd`), `O_CLOEXEC` singleton lease, `fcast_jobs` visibility recovery, `daemon.sock` push IPC, and `daemon_budget_ledger` spend governor.
8. **`Tether` (`SUB-08`)** — Hardened MCP server (`fcast-mcp`), async job handles, `fcast://` live resource subscriptions, and static-linted compiled skill packs for `pi_agent_rust`, `Hermes`, and `Antigravity`.
9. **`Cockpit` (`SUB-09`)** — `FrankenTUI` 7-view visual triage dashboard, progressive `fcast init` / `DoctorProbe` onboarding, and atomic 4-phase PTY harness handoff.

---

## Target Hardware & OS Envelope (`§5` & `§19`)

* **Operating Systems:** First-class native support for **macOS 15+ (`aarch64-apple-darwin` Apple Silicon)** and **Linux Kernel 6.1+ (`x86_64-unknown-linux-gnu` / `aarch64-unknown-linux-gnu`)**.
* **Storage Baseline:** **4 TB (`4,000 GB`) NVMe SSD** partitioned into hot MVCC operational state (`ops.fsqlite`), content-addressed `EvidenceVault`, multi-year columnar Parquet L2 orderbook ticks, point-in-time `available_utc` macro/conflict/cyber vintages, 25,000-hour Opus audio + `ReplayAsOf` cassettes, and rolling `zstd` WAL backups (`10%` wear-leveling reserve preserved).
* **Optional Accelerator Annex:** **0 GPU required** for full core compilation and tournament play; optional process-isolated **1× NVIDIA RTX 3090 (`24 GiB GDDR6X`, Compute Capability `8.6`)** compute annex unlocks \$0-API local `24B–32B` Tier-0 likelihood sweeps across 2,000+ contracts, multilingual cross-encoder reranking, local Whisper/Parakeet podcast pre-screening, and `10,000`-episode offline `GEPA` calibration runs.

---

## Repository Layout & Verification

* [`COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md`](./COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md) — Master engineering specification (`Parts 0–V`, `§0–§25`).
* [`docs/delivery/DELIVERY_LOG.md`](./docs/delivery/DELIVERY_LOG.md) — Externalized delivery ledger (`§0.7` Rule 1).
* [`templates/bitemporal_event_audit.tmpl`](./templates/bitemporal_event_audit.tmpl) — Byte-frozen bi-temporal event verification prompt (`SHA-256` locked in `§9`).
* [`scripts/verify_plan_hygiene.py`](./scripts/verify_plan_hygiene.py) — Automated `§0.7` plan-hygiene auditor.
* [`scripts/verify_math_and_contracts.py`](./scripts/verify_math_and_contracts.py) — Reference contract tests (`[MEASURED]` evidence binding).

```bash
python3 scripts/verify_plan_hygiene.py
python3 scripts/verify_math_and_contracts.py
```
