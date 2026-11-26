# Competition & Reference Codebase Analysis — `franken_forecast` (`fcast`)

> **Living Reference Document (`v1.0` · `2026-11-26`):** Designed to accumulate deep, code-verified dissections of top-performing AI forecasting bots and competition platforms, mapping concrete ideas, operational boilerplate, empirical falsifications, and architectural gaps directly onto `COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md` (`Plan v2.0`) and our Rust crate milestones (`M0`–`M5`).

## Master Competitor & Platform Matrix

| Competitor / Repo | Branch / Snapshot | Verified Rank / Track Record | Competitions Targeted | Core Stack | Primary Edge vs. `fcast` | Primary Weakness vs. `fcast` |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **[01. `No-Stream/nostreambot-metaculus-bot`](#competitor-01--no-streamnostreambot-metaculus-bot)** | `gemini-4-hydrogen-research` (`317k` LOC, `389` test files) | **15th / 277** Summer 2026 FutureEval (`~3,240` spot peer pts / `256` Qs, above all-bots ensemble) | FutureEval, MiniBench, Metaculus Cup, **Mantic Crucible** | Python 3.12 (`forecasting-tools`), GitHub Actions + `cron-job.org` | Battle-tested Mantic support, close-aware time budgeting, 4-rung fidelity-checked extraction ladder, shared-brief verification gap-fill, 389 offline tests w/ socket egress guard | Single-shot (zero longitudinal updating), `~\$2.60/Q` flat spend (no ForecastBench), no pre-seal market blinding, no bi-temporal vintage discipline, heavy Python framework monkeypatching |
| **[02. `Schokolade12138/metac-bot-template`](#competitor-02--schokolade12138metac-bot-template-sakuraqkl)** | `main` (`+1,450` LOC over `metac-bot-template`, `3` test files) | Fall 2026 FutureEval entrant (`sakuraqkl`, lightweight fork) | FutureEval + MiniBench (`bot-testing-area` smoke test) | Python (`forecasting-tools`, OpenRouter `paid`/`free` profiles), GitHub Actions | Zero-key free research (Google News RSS + Polymarket/Manifold public search), iterative fixed-point MC simplex flooring, 2-tier credit auto-downgrade (`< \$8` cheap, `< \$0.50` stop), cross-vendor failover retry, type-stratified smoke test | Silent exit-0 crash swallowing, zero numeric/date/Mantic guards, blind `[:40_000]` research truncation, no market blinding, no resolution-source fetch or gap-fill, no longitudinal updating |
| **[03. `Poke1997/metac-firefly26710`](#competitor-03--poke1997metac-firefly26710)** | `main` @ `4e675a5` (`4,322` LOC, `0` test files) | Fall 2026 FutureEval + Metaculus Cup entrant | FutureEval, Metaculus Cup, `test_bot` | Python (`forecasting-tools`, Anthropic-only + `bot-data` git branch + Claude `review-bot` skill) | **Control/Control/Challenger** bounded-risk live A/B slots, auto-expiring **shadow forecasts** (`SHADOWS_UNTIL`), shared-context prompt-cache splitting, 4-tier billing-cycle budget pacing (`70%/90%/100%`), daily `model_watch`, cheapest-first read-only `review-bot` skill | Single-vendor (Anthropic-only) monoculture, `0` automated tests, GitHub Actions cache budget ledger (eviction resets spend), no numeric/Mantic/ForecastBench guards, no longitudinal updating or market blinding |
| *(Reserved: Competitor 04)* | *Pending next analysis* | — | — | — | — | — |

---

## Competitor 01 · `No-Stream/nostreambot-metaculus-bot`

* **Repository:** `https://github.com/No-Stream/nostreambot-metaculus-bot` (branch `gemini-4-hydrogen-research`)
* **Scale:** `317,149` total lines across Python modules, `15` detailed operational docs (`docs/`), `10` GitHub Actions workflows, and **`389` test files** (`tests/`).
* **Operating Model:** Unattended hourly GitHub Actions runs (backed by `cron-job.org` workflow-dispatch because GitHub cron delivered only `~22%` of scheduled firings) executing a single-shot pipeline per open question: parallel research → dual concurrent gap-fill (`v1` targeted + `v2` agentic loop) → multi-vendor forecaster fan-out → median aggregation → publish + full-trace comment.

### 1. Competitions Optimized For (Including Gaps in `franken_forecast`)

| Competition / Mode | Supported in `nostreambot`? | Supported in `fcast` Plan `v2.0`? | Critical Platform-Specific Findings |
| :--- | :---: | :---: | :--- |
| **Metaculus FutureEval Bot Tournament** (`--mode tournament`) | Yes | Yes (`§5.2`) | Hourly polling + `skip_previously_forecasted_questions` fail-shut guard. |
| **Metaculus MiniBench** (`--mode minibench`) | Yes | Yes (`§5.1–§5.2`) | Biweekly fast-feedback track; `nostreambot` uses it for era-bucketed residual analysis. |
| **Metaculus Cup / Quarterly Cup** (`--mode metaculus_cup`) | Yes | **Not yet** | Regular open-question calibration stream; comments stay **private**, so `my_forecasts` (not public comments) is the sole reliable coverage probe. |
| **Mantic Crucible** (`competitions.mantic.com`, `--mode mantic`) | **Yes (deeply hardened)** | **Not yet** | Runs on a fork of Metaculus's open-source backend, so our Rust `MetaculusClient` can support it cheaply **if** we implement 6 verified wire & scoring quirks (see §2.1 below). |
| **ForecastBench** (`forecastingresearch/forecastbench`) | No (*"Study-only, not planned"*) | **Yes (`§5.3`, `§12.6`)** | `nostreambot`'s `~\$2.60/Q` flat spend makes `1,000`-Q biweekly rounds prohibitively expensive (`~\$2,600/round`). |

---

### 2. High-Value Ideas `nostreambot` Implements That We Don't Have Yet

#### 2.1 Mantic Crucible Platform Quirks & Scoring Math (`mantic.py`, `numeric/pmf_cdf.py`, `numeric/out_of_range_floor.py`)
1. **Wire-Type & Filter Divergence:** Mantic's list filter uses `forecast_type=quantitative` (not `numeric,discrete`) and emits `question.type == "quantitative"`. A standard Metaculus client returns **zero** quantitative questions unless it omits `allowed_types` on the GET and rewrites `quantitative → discrete` locally before parsing.
2. **Broken Paginator `next` Link:** Mantic advertises `next` past the final page; pagination MUST walk offsets until an empty page up to a ceiling (`MANTIC_FETCH_QUESTION_CEILING`).
3. **Per-Bin PMF Elicitation (`N <= 31` Coarse Grids) + Pointwise Mean Pooling:** On coarse discrete/date grids (e.g., 12 one-day bins where 3 are weekend non-trading days), 13 percentiles cannot express `0%` on excluded bins and leak `~25%` mass onto impossible days (`-14.4` baseline points). `nostreambot` switches to direct per-bin PMF elicitation (`bin_probs` + `below_range`/`above_range`) and aggregates coarse per-bin members via **pointwise MEAN (linear opinion pool)** rather than median (because median of 3 sharp PMFs crushes bins believed by only 1 member to the floor).
4. **Mandatory `>= 5%` Open Out-of-Range Tail Floor (`MANTIC_OUT_OF_RANGE_TAIL_FLOOR = 0.05`):** Applied last to Mantic open-bound numeric/date aggregates because Mantic scores out-of-range resolutions against a fixed `0.05` reference (a `1%` tail suffers `-80.5` points).

#### 2.2 Close-Derived Dynamic Time Budget (`time_budget.py`)
* Computes `total_s = min(WALL_CLOCK_CAP, close_time - now - PUBLISH_RESERVE)` **before spending a single token**:
  * **Intake Skip:** Arithmetically unpublishable questions (e.g., 22s to close) abort at intake before any research/LLM spend.
  * **Automatic Fast Path:** Below `TIME_BUDGET_FAST_PATH_THRESHOLD`, drops slow optional search providers, both gap-fill passes, and expensive browser/paid fetch rungs while keeping fast direct fetch + ensemble.
  * **Tightest-Close-First Queueing:** Sorts open questions by `close_time ASC` so urgent contracts win semaphore slots first.

#### 2.3 Four-Rung Deterministic Value Extraction Ladder + Anti-Infidelity Repair (`value_extraction.py`, `structured_output_schema.py`)
* Strictly separates **shape validation** from **fidelity validation** across 4 rungs (`strict JSON block` → `deterministic repair` → `constrained LLM salvage` → `drop member`):
  * **Duplicate-Key Refusal:** Rejects JSON objects with repeated keys (default `json.loads` silently takes last-write-wins, turning `{"0": 0.9, "0": 0.4}` into `0.4`).
  * **Boolean-as-Float Refusal:** Refuses JSON `true`/`false` inside probability fields (preventing lax parsers from turning `true` into `1.0`).
  * **Bidirectional Multiset Numeric-Token Check on Repair:** Refuses any JSON repair that **adds** or **drops** a numeric literal or completes a truncated literal (`0.`, `1e-`, mid-date `"2027-06-1`)—ensuring a repairer never invents a probability!
  * **Candidate-Major Ordering:** Tries strict+repair on the *last* fenced JSON block before falling back to earlier blocks, preventing an early superseded draft from beating a slightly malformed final answer.

#### 2.4 Shared-Brief Verification Gap-Fill (`research/agentic/`, `docs/agentic_gap_fill.md`)
* Residual postmortems proved that **`nostreambot`'s worst misses occurred when all 3 models agreed on one stale, misread, or hallucinated briefing claim.**
* Their `gap-fill v2` driver performs a **private dry-run forecast against the exact panel prompt skeleton** and prioritizes **VERIFY** targets (checking the 2–3 load-bearing claims against primary sources) alongside **RESOLUTION** (fetching the exact named resolution-source URL via a multi-rung fetch ladder: HTTP → TLS-impersonated fetch → Chromium → paid reader), **FILL**, **TIMING**, and **BASE-RATE**, keeping the user brief frozen as a prompt-cache prefix.

#### 2.5 Survivor-Conditional `THIN_PUBLISH_FLOOR` & Numeric Pipeline Hardening
* **`THIN_PUBLISH_FLOOR` (`[0.05, 0.95]` when `survivors == 1`):** When timeouts/errors degrade an ensemble to a single surviving forecaster on a binary question, clamp tighter (`[0.05, 0.95]` vs `[0.02, 0.98]`) because median-of-1 has zero variance reduction.
* **Numeric Edge Cases (`docs/numeric_pipeline.md`):**
  * `13` canonical percentiles (`1, 2.5, 5, 10, 20, 40, 50, 60, 80, 90, 95, 97.5, 99`) + guarded **PCHIP** CDF interpolation.
  * `DateQuestion` mapped onto epoch seconds with **date-only strings parsed as noon UTC** (landing safely inside right-closed day bins).
  * `zero_point` log-scale preserved on **non-201 discrete grids** (not just 201-point continuous grids!).
  * **Whole-Set Collapse Rule:** Never invent artificial width on a 201-pt continuous grid when a model outputs 13 identical values (withhold the member instead); on an outcome-space discrete grid, cap plateau spread strictly to `<= 1` grid bin width.

---

### 3. Production Boilerplate We Haven't Incorporated Yet

1. **Fail-Shut Re-Spend Guard (`SKIP_GUARD_UNREADABLE`):** If `my_forecasts` is missing or unreadable due to an API/auth glitch, **refuse to forecast** rather than defaulting `already_forecasted = False` and re-spending on the entire tournament.
2. **API Identity Preflight (`api_preflight.py`) & Pre-Spend Permission Check:** One unauthenticated identity check before transmitting any bearer token (prevents leaking credentials to hijacked/parked DNS), plus checking `user_permission ∈ {forecaster, curator, admin, creator}` on the tournament detail route before paying for an ensemble.
3. **Strict Donated-Credit Isolation (`_assert_personal_keys_only`) + Runway Floor Alert (`\$100`):** Refuses to start a non-Metaculus run (Mantic / ForecastBench) if donated Metaculus OpenRouter/Proxy keys are enabled, and alerts while credit runway still exists.
4. **Money-Safety Network-Egress Guard in Tests (`tests/conftest.py`):** Autouse socket + native-transport (`libcurl`, Chromium) egress block allowing only loopback (`127.0.0.1`, `::1`), paired with `20+` dedicated `*_fakes.py` double modules and **branch coverage (`branch = true`)** so untaken fail-shut/fallback arms cannot hide behind line coverage.
5. **Contract-Tested Telemetry Markers (`scripts/telemetry/markers.py`):** Machine-harvested structured lines (`TIME_BUDGET`, `FORECASTERS_SURVIVED`, `FORECASTER_DROPS`, `EXTRACTION_RUNG`, `MEMBER_FORECAST` carrying `raw` vs `published`, `EXTREME_CALL`, `THIN_PUBLISH_FLOOR`, `MANTIC_POST_DROPPED`) that survive CI log retention expiry.
6. **Structure-Preserving Comment Trimming (`149,999`-char budget) & Flush-in-`finally` Persistence:** Trims research prose before rationales, preserves per-model attribution + structured JSON blocks, and flushes research archives inside `finally` so crashes/timeouts never lose partial-batch data.
7. **External Cron-Dispatch Backup:** Never trust GitHub Actions `schedule:` cron alone (`~22%` delivery reliability); pair with `fcastd` local daemon or an external webhook dispatcher.

---

### 4. Empirical Lessons from `nostreambot`'s `FUTURE.md` (Direct Implications for `franken_forecast`)

`nostreambot`'s `FUTURE.md` is unusually honest—it documents every idea they built, benchmarked, and **killed**. Five findings directly pressure-test our `Plan v2.0`:

| `nostreambot` Empirical Finding (`FUTURE.md`) | Impact on `franken_forecast` (`Plan v2.0`) |
| :--- | :--- |
| **Post-hoc calibration (Platt, isotonic, shrinkage) killed:** Binary calibration slope flipped sign across eras; statistical power `~0.25` even at `N = 400`. | **Validates keeping `enable_ridge_platt_extremize = false` in `config/ablation.toml`**, and warrants tightening our `§12.7` gate from `n >= 100` to **multi-era out-of-sample sign stability**. |
| **Advisory, non-binding critic passes killed (`2026-07-08`):** Critic diagnosed the exact defect and the forecast never moved. | **Critical upgrade for our Red-Team skill (`templates/red_team_audit.tmpl`):** It MUST have **binding** authority (dampen `Δlogit`, block `G1` contract misread, or trigger a verification fold)—never advisory-only text. |
| **LLM stacker & always-on crux rewrite disabled in prod (`88`-Q test = null vs. median); quantile-averaging lost `-7.4` log points vs. CDF-space pooling.** | **Directly validates** our `§5.1` robust median default and our `v2.0` `§11.4` probability-space CDF mixture pooling! |
| **Kalshi `volume_24h` is `0` on long-horizon markets**, mislabeling deep books as thin. | **Directly validates** our `§8` `BookMarkPrice` using resting top-of-book depth (`L_tob`) and `\$500` sweep impact rather than `volume_24h`. |
| **Worst misses = all 3 models agreeing on one wrong briefing claim** (`consensus-fragility`). | **Motivates adding a `load_bearing_claims` verification step** inside `Loom` Tier-1 `fold` / Red-Team even when ensemble spread is low! |

---

### 5. Where `nostreambot` Falls Short (And Where `franken_forecast` Is Structurally Stronger)

1. **Single-Shot Amnesia (Zero Longitudinal Belief Updating):** `nostreambot` permanently skips any question once forecast (`skip_previously_forecasted_questions = True`) and has no immutable revision chain or horizon projection. `fcast`'s 12-field `ForecastRevision` + Two-Speed `(μ_t, N_t)` + 3-document question memory (`Gold Dossier` + `Episodic Log` + `State Doc`) + `\$0` Kalman Bayesian Surprise directly solves multi-month belief maintenance.
2. **Flat `~\$2.60/Question` Compute Spend:** Runs the full heavy pipeline on every open question regardless of VoI, pricing itself out of `1,000`-question `ForecastBench` rounds. `fcast`'s `QuestionMetaForecast` + Two-Speed `VoI_log` allocation targets `<= \$15.00/round`.
3. **Zero Pre-Seal Market Blinding (`INV-04`):** Injects Polymarket/Kalshi/Manifold snapshots straight into the shared briefing before models forecast, making independent `p_indep` measurement and true CLV / information-gain attribution impossible.
4. **No Bi-Temporal (`available_utc <= as_of_utc`) Vintage Discipline:** Lacks vintage-locked macro/conflict/cyber `DataPacks`, Horvitz-Thompson lag adjustment, and point-in-time replay guarantees.
5. **No Full-Universe Anti-Selection-Bias Ledger (`INV-13`) or Interactive Cockpit (`fcast-tui`):** Relies on post-hoc comment parsing and offline scripts rather than an irreversible eligible-universe ledger and a sub-16ms interactive workstation (`[d]` Why-Moved waterfall, `[w]` What-If sandbox).
6. **Heavy Python Framework Coupling:** `317k` lines of Python wrapping monkeypatched `forecasting-tools` internals with process-global counters (`_post_drop_count`) and `22` brittle monkeypatch seams vs. our clean, zero-`unsafe`, 6-crate pure-Rust DAG over `FrankenSQLite` WAL/MVCC.

---

### 6. Verdict: Should We Redo `M0`?

**No — do NOT redo `M0`.** Our `M0` foundation (`fcast-core` canonical `ForecastRevision` + Two-Speed math + `fcast-infra` `fsqlite` WAL/MVCC + `DurableJobStore` + `domain_events` + `SafeExternalFetch`) is already architecturally cleaner than `nostreambot`'s persistence layer and avoids every Python/monkeypatch liability.

Instead, **fold six high-ROI upgrades incrementally into `Plan v2.1` and `M1`** (zero rewrite):

1. **Add Mantic Crucible & Metaculus Cup Support (`M1` `fcast-ingest` + `fcast-core::coherence`):**
   * Add `PlatformKind::{Metaculus, Mantic, ForecastBench}` and absorb Mantic's 3 wire quirks (`quantitative`, empty `allowed_types`, offset ceiling), per-bin PMF elicitation (`N <= 31` + pointwise mean), and `>= 5%` open-tail floor.
2. **Add `TimeBudget` & `ThinPublishFloor` (`M1` `fcast-core`):**
   * Close-derived intake skip + fast-path degradation + `[0.05, 0.95]` binary clamp when `survivors == 1`.
3. **Add Fidelity-Checked 4-Rung `ValueExtractionLadder` (`M1` `fcast-core` / `fcast-epistemic`):**
   * Strict JSON block → bidirectional multiset-checked repair (rejecting duplicate keys, booleans-as-floats, truncated literals) → constrained LLM salvage → member drop, logging `raw` vs `published`.
4. **Add Fail-Shut Re-Spend Guard, API Identity Preflight & Credit-Pool Platform Lock (`M1` `fcast-infra` / `fcast-ingest`):**
   * Prevent re-spending on unreadable `my_forecasts`, prevent DNS-parking token leaks, and forbid spending Metaculus-donated credits on Mantic/ForecastBench.
5. **Upgrade Red-Team & Consensus-Fragility Verification (`M3` `Loom`):**
   * Keep `red_team_audit.tmpl` **binding** (never advisory-only), add a `load_bearing_claims` primary-source **VERIFY** check even when ensemble spread is low, and require **multi-era sign stability** before enabling `enable_ridge_platt_extremize`.
6. **Upgrade Rust Test Harness Discipline (`M1+`):**
   * Enforce an explicit offline network-egress guard in `M1` HTTP transport tests, build clean trait-based `*_fakes` modules, and track branch coverage on fail-shut/fallback arms.

---

## Competitor 02 · `Schokolade12138/metac-bot-template` (`sakuraqkl`)

* **Repository Diff:** `https://github.com/Metaculus/metac-bot-template/compare/main...Schokolade12138:metac-bot-template:main` (`4` commits, `16` files, `+1,450` LOC over `Metaculus/metac-bot-template`)
* **Scale:** Minimal, clean single-bot fork (`bot/{forecaster,free_research,guards,settings}.py`, `run_bot.py`, `3` `unittest` files totaling `282` LOC).
* **Competitions Targeted:** **Metaculus FutureEval + MiniBench** (combined in one `--mode tournament` pass) and `bot-testing-area` smoke tests (`--mode test_questions`). Does **not** target Mantic Crucible, Metaculus Cup (kept manual/unscheduled), or ForecastBench.

### 1. Clever Lightweight Ideas Worth Extracting

1. **Zero-Key Free-Research Layer (`bot/free_research.py`):**
   * Combines **Google News RSS** (`https://news.google.com/rss/search?q=<query>+when:30d`) with public **Polymarket** (`gamma-api.polymarket.com/public-search`, filtered to `volume >= \$5,000` active events) and **Manifold** (`api.manifold.markets/v0/search-markets`, filtered to `uniqueBettorCount >= 8` open markets)—requiring **zero API keys**.
   * Uses a tiny model to generate 3 news + 2 market keyword queries with a **deterministic zero-LLM keyword fallback (`fallback_queries`)**.
   * *Adoption in `fcast`:* Ideal for `M1` `Harvester`'s zero-key `fcast sync` (`GATE-24`) and as an automatic `\$0` fallback when AskNews/Perplexity/Exa quotas are exhausted—while keeping Polymarket/Manifold prices strictly quarantined behind `MarketBlindingScrubber` pre-seal (`INV-04`).
2. **Exact Fixed-Point Multiple-Choice Simplex Flooring (`bot/guards.py::floor_option_probabilities`):**
   * Raising low options to `floor = 0.01` and rescaling the remaining options proportionally can push another option below `floor`; their `while True` fixed-point loop iterates until the floored set stabilizes, guaranteeing exact sum-to-1 and `p_i >= floor` for all options.
   * *Adoption in `fcast`:* Add directly to `fcast-core::coherence` (`<= 25` LOC + `10,000`-case `proptest`).
3. **Graceful Two-Tier Credit Auto-Degradation & Free-Profile Tournament Guard (`bot/settings.py`):**
   * Checks remaining OpenRouter credits (`/api/v1/key`) before forecasting: below **`\$8.00`** (`LOW_CREDIT_USD`), automatically swaps to cheap models and disables paid web search; below **`\$0.50`** (`MIN_CREDIT_USD`), halts forecasting cleanly.
   * **`free` Profile Safety Rail:** In `--mode tournament`, the `free` profile *only lists open questions* and refuses to publish weak free-model predictions into prize tournaments (preventing accidental leaderboard poisoning during setup).
   * *Adoption in `fcast`:* Add a `low_credit_degrade_usd` tier to `daemon_budget_ledger` / `LlmRouter` so `fcast` gracefully downgrades to Tier-0 + cheap models instead of hard-stopping mid-tournament.
4. **Cross-Vendor Failover Retry (`bot/forecaster.py::_guarded`):**
   * On forecast failure, retries on the **next vendor in the cycle** rather than hammering the same failing/rate-limited model.
5. **Type-Stratified Smoke Test Sampling (`run_bot.py::pick_test_questions`) & `check_setup.yaml`:**
   * Round-robins `bot-testing-area` questions by question type so a 4-question smoke test covers `Binary`, `MultipleChoice`, `Numeric`, and `Date` together; `check_setup.yaml` verifies secret presence without leaking values.

### 2. Where `Schokolade12138` Falls Short (Pitfalls to Avoid)

1. **Silent Exit-`0` Crash Swallowing (`run_bot.py:213-223`):** Catches `BaseException` and returns exit code `0` to avoid GitHub failure emails—meaning broken tokens, API changes, or parser regressions can fail silently for weeks. (`nostreambot`'s alertable exit ladder + `fcast`'s `DEAD_LETTER` HUD badge are far safer.)
2. **Zero Numeric/Date Safeguards & Blind Research Truncation:** Clamps binary `[0.02, 0.98]` and floors MC `>= 0.01`, but applies **no guards** on numeric/date distributions (no PCHIP, no discrete snap, no Mantic tail floor, no extraction fidelity ladder), and truncates combined research via a blind `[:40_000]` character slice that can chop mid-market or mid-sentence.
3. **No Market Blinding, No Resolution-Source Fetching, No Gap-Fill, No Fail-Shut Re-Spend Guard:** Injects Polymarket/Manifold prices straight into the prompt pre-seal, relies only on headlines + one web-search summary, and filters `not q.already_forecasted` without failing shut if `my_forecasts` is missing.
4. **Single-Shot Only & Minimal Test Coverage:** `3` small `unittest` files (`282` LOC), no network-egress guard, no longitudinal revision tracking, and no calibration/residual postmortem tooling.

---

## Competitor 03 · `Poke1997/metac-firefly26710`

* **Repository & Commit:** `https://github.com/Poke1997/metac-firefly26710/commit/4e675a5ce88e77e40ccdac8037e7a9c310663210` (`4,322` total LOC, `+821` lines in commit `4e675a5`)
* **Competitions Targeted:** **Metaculus FutureEval** and **Metaculus Cup** (`housekeeping.yaml` + `review_bot.yaml` automation). Does **not** target Mantic Crucible, MiniBench explicitly, or ForecastBench.

### 1. High-Leverage Ideas Unique to `Poke1997` (Not in Competitors 01 or 02)

1. **Bounded-Risk Live A/B Testing via `Control / Control / Challenger` Median Slots (`experiment.py`):**
   * Submits the median of `[control_1, control_2, challenger]`. Because the median of three *always* lies between `control_1` and `control_2` (or on one of them), **a flawed challenger prompt can never drag the submitted tournament forecast outside the controls' range**, while simultaneously measuring live prospective A/B skill *and* pure same-prompt sampling variance (`|control_1 - control_2|`).
   * *Adoption in `fcast`:* Add an optional `challenger_slot` mode in `Loom`'s ensemble runner (`config/ablation.toml`) so new prompt variants or `GEPA` candidates can accrue real prospective evidence safely inside live tournaments.
2. **Auto-Expiring Shadow Forecasts (`SHADOW_FORECASTS` + `SHADOWS_UNTIL`):**
   * Runs cheaper/faster models (`economy:2, helper:1, forecaster_medium:1`) on the exact same cached research bundle, logs their predictions without submitting them, and **auto-terminates on `SHADOWS_UNTIL`** so experimental spend never runs indefinitely.
   * *Adoption in `fcast`:* Maps cleanly to `forecast_streams.stream_kind = 'shadow_experiment'` in `Chronicle` with a mandatory `expires_utc` cutoff.
3. **Explicit Prompt-Cache Prefix Splitting (`split_prompt`):**
   * Splits every forecast prompt into `(shared_context [question + criteria + research], slot_instructions)` so all submitted and shadow slots share a byte-identical cached prefix—slashing input token costs across multi-slot ensembles.
   * *Adoption in `fcast`:* Enforce a strict cache-stable `(system + Gold Dossier + State Doc + EvidenceVault brief)` prefix across all ensemble members and `red_team_audit.tmpl` calls in `LlmRouter` (`§15`).
4. **4-Tier Billing-Cycle Pacing Budget (`budget.py`):**
   * Tracks spend against a monthly credit renewal day (`BUDGET_CYCLE_DAY`) plus a `BUDGET_PER_RUN_USD` cap, stepping smoothly across `full` (`< 70%`) → `economy` (`70–90%`) → `minimal` (`90–100%`, 1 forecast) → `stop` (`>= 100%`).
   * *Adoption in `fcast`:* Extend `daemon_budget_ledger` (`§13`) with cycle-day renewal and `70%`/`90%` graceful tier step-downs alongside the daily hard cap.
5. **Report-Only Daily Model Watcher (`model_watch.py`) & Leaderboard Snapshots (`track_record.py`):**
   * Queries vendor `/v1/models` daily to report newly released models (`released_after_forecaster`) and alert if a configured model is deprecated/missing—never auto-switching without a test pass.
   * *Adoption in `fcast`:* Add `fcast doctor --check-models` across Gemini/Anthropic/OpenAI/OpenRouter.
6. **Cheapest-First Read-Only Postmortem Skill (`.claude/skills/review-bot/SKILL.md`):**
   * Enforces strict read-only triage (`mechanical trace flags → grep research for deciding fact → 1–2 rationales → full research last`), explicitly guards against hindsight bias (*"Well forecast, unlucky outcome is a real finding"*), and requires every proposed fix to cite the exact resolved questions it would have improved.
   * *Adoption in `fcast`:* Directly informs our `Tether` compiled postmortem skill (`§14`) and `postmortem_ee_distill.tmpl` workflow.

### 2. Where `Poke1997` Falls Short

1. **Single-Vendor Monoculture (Anthropic-Only):** All slots (`FORECASTER`, `ECONOMY`, `HELPER`) are Claude models—zero cross-vendor diversity against correlated lab blind spots.
2. **Zero Automated Tests (`0` test files):** No `pytest`/`unittest` suite, no extraction ladder, no numeric/date/Mantic guards, and a live bug in `track_record.py:61` (`open(..., "w")` overwrites the history CSV instead of appending).
3. **Fragile Cache-Backed Budget State & Single-Shot Pipeline:** Storing `.budget/ledger.json` in GitHub Actions cache risks silent spend resets on cache eviction (`fcast`'s `fsqlite` WAL `daemon_budget_ledger` is durable), and there is no longitudinal `ForecastRevision` updating, no pre-seal market blinding, and no bi-temporal vintage discipline.
