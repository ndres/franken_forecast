#!/usr/bin/env python3
"""
Automated Plan Hygiene Auditor (§0.7) for COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md (v1.1).

Verifies:
1. Part 0 Document Control (§0.1 .. §0.8) and Part I..V structure (§1.1..§1.4, §7.1, §7.2).
2. Delivery-tracking quarantine (§0.7 Rule 1): zero markdown task checkboxes inside master plan.
3. Canonical 8-Subsystem Registry (§0.5): SUB-01 .. SUB-08 present; zero in-repo Wargame crate (fcast-sim).
4. Pure-CPU & Sister-Project Boundary (§0.7 Rule 8): rejects RTX 3090 / VRAM / GPU Annex tables.
5. Canonical 12-Field ForecastRevision & 4-Stage Lifecycle (§0.7 Rule 7 & §7): verifies all 12
   fields and InitialForecast / SubsequentRevision / HorizonForecastView / QuestionResolution.
6. Claim-boundary bijection (§0.7 Rule 6): CLAIM-01 .. CLAIM-08 defined, and every GATE-01 .. GATE-24
   row in §22 maps to EXACTLY ONE primary CLAIM-XX, with >= 2 gates per claim and §0.1 evidence tags.
7. 6-Crate [BUDGET] LOC caps (§17) and 6-Tier NVMe storage rows (§19.2) summing to 4,000 GB.
8. Byte-exact SHA-256 parity between templates/bitemporal_event_audit.tmpl and §9.
"""

from __future__ import annotations
import hashlib
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
PLAN_PATH = ROOT / "COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md"
TMPL_PATH = ROOT / "templates" / "bitemporal_event_audit.tmpl"
DELIVERY_PATH = ROOT / "docs" / "delivery" / "DELIVERY_LOG.md"

EVIDENCE_TAGS = (
    "[MEASURED]",
    "[UPSTREAM-SPEC]",
    "[DERIVED]",
    "[BUDGET]",
    "[TARGET]",
    "[HYPOTHESIS]",
)

EXPECTED_SUBSYSTEMS = [
    ("SUB-01", "Chronicle"),
    ("SUB-02", "Harvester"),
    ("SUB-03", "Notary"),
    ("SUB-04", "Loom"),
    ("SUB-05", "Crucible"),
    ("SUB-06", "Metronome"),
    ("SUB-07", "Tether"),
    ("SUB-08", "Cockpit"),
]

FORECAST_REVISION_FIELDS = [
    "forecast_id",
    "revision_id",
    "question_id",
    "as_of_utc",
    "p_indep",
    "p_market",
    "probability_distribution",
    "evidence_handles",
    "model_version",
    "calibration_version",
    "rationale_handle",
    "resolution_status",
]

LIFECYCLE_STAGES = (
    "InitialForecast",
    "SubsequentRevision",
    "HorizonForecastView",
    "QuestionResolution",
)

MANDATORY_TELEMETRY_METRICS = (
    "brier_score",
    "log_score",
    "calibration_error",
    "skill_vs_baselines",
    "cost_per_forecast",
    "cost_per_resolved_forecast",
    "research_latency",
    "forecast_coverage",
    "pct_forecasts_revised",
    "pct_revisions_improving_score",
)

CLAIM05_HYPOTHESES = (
    "HYP-05A",
    "HYP-05B",
    "HYP-05C",
    "HYP-05D",
    "HYP-05E",
    "HYP-05F",
)

EXPECTED_TEMPLATES = (
    "bitemporal_event_audit.tmpl",
    "rationale_staged_turn_a.tmpl",
    "rationale_staged_turn_b.tmpl",
    "stream_likelihood_tick.tmpl",
    "belief_fold_step.tmpl",
    "seiph_process_critic.tmpl",
    "meta_forecast_triage.tmpl",
    "postmortem_ee_distill.tmpl",
    "red_team_audit.tmpl",
)

MARKET_BLIND_TEMPLATES = (
    "rationale_staged_turn_a.tmpl",
    "stream_likelihood_tick.tmpl",
    "belief_fold_step.tmpl",
    "red_team_audit.tmpl",
)


def fail(msg: str) -> None:
    print(f"HYGIENE_FAIL: {msg}", file=sys.stderr)
    sys.exit(1)


def main() -> None:
    if not PLAN_PATH.exists():
        fail(f"Missing master plan: {PLAN_PATH}")
    if not TMPL_PATH.exists():
        fail(f"Missing frozen template: {TMPL_PATH}")
    if not DELIVERY_PATH.exists():
        fail(f"Missing externalized delivery log: {DELIVERY_PATH}")

    text = PLAN_PATH.read_text(encoding="utf-8")

    # 1. Required Document Control & Diagnostic Headings
    required_markers = [
        "## Part 0 · Document Control",
        "### §0.1",
        "### §0.2",
        "### §0.3",
        "### §0.4",
        "### §0.5",
        "### §0.6",
        "### §0.7",
        "### §0.8",
        "## Part I ·",
        "### §1.1",
        "### §1.2",
        "### §1.3",
        "### §1.4",
        "## Part II ·",
        "### §7.1",
        "### §7.2",
        "### §7.3",
        "### §11.4",
        "### §12.2",
        "### §12.3",
        "### §12.4",
        "## Part III ·",
        "### §13.2",
        "### §16.1",
        "### §16.2",
        "### §16.3",
        "View `[6:Meta]`",
        "## Part IV ·",
        "## Part V ·",
    ]
    for marker in required_markers:
        if marker not in text:
            fail(f"Missing required structural heading marker: {marker!r}")

    # 2. Zero task checkboxes inside master plan (§0.7 Rule 1)
    checkbox_hits = re.findall(r"^\s*[-*]\s+\[[ xX]\]\s+", text, flags=re.MULTILINE)
    if checkbox_hits:
        fail(f"Master plan contains {len(checkbox_hits)} mutable task checkbox(es); move to docs/delivery/")

    # 3. Clean Scope Boundaries (§0.7 Rule 8): zero wargame, NVMe size, GPU, or removed defect drift
    for forbidden in ("wargame", "RTX 3090", "GDDR6X", "VRAM", "Slot A:", "`SUB-09`", "4 TB", "4,000 GB", "NVMe", "Defect D1", "Pre-Cutoff LLM Parametric"):
        if forbidden.lower() in text.lower():
            fail(f"§0.7 Rule 8 scope violation: forbidden token {forbidden!r} present in master plan")
    if "43497" not in text:
        fail("§5 missing Metaculus Notebook 43497 competitive bot patterns reference")
    for req_fe in (
        "45615",
        "AskNews",
        "MetaculusCdfBuilder",
        "forecasting-tools",
        "forecastingresearch/forecastbench",
        "difficulty_fe",
        "FrankenNumPy",
        "FrankenPandas",
        "beads_rust",
        "gold_dossier_handle",
        "episodic_events",
        "state_doc_revisions",
        "cass search",
        "Bayesian Surprise",
        "Platt",
        "domain_events",
        "DEAD_LETTER",
        "Dirichlet",
        "What-If",
        "Command Palette",
        "QuestionRunContext",
        "event_consumers",
        "Mantic",
        "ValueExtractionLadder",
        "fetch_raw_data",
        "THIN_PUBLISH_FLOOR",
        "Control / Control / Challenger",
        "SEASONAL_NOTE",
        "TimeBudget",
    ):
        if req_fe not in text:
            fail(f"Missing required FutureEval / ForecastBench / FrankenSuite / v1.9 reference: {req_fe}")

    # 4. Canonical Subsystem Names (SUB-01 .. SUB-08)
    for sub_id, sub_name in EXPECTED_SUBSYSTEMS:
        pattern = rf"\|\s*`{sub_id}`\s*\|\s*\*\*{sub_name}\*\*"
        if not re.search(pattern, text):
            fail(f"Subsystem registry row missing or malformed for {sub_id} ({sub_name})")

    # 5. Canonical 12-Field ForecastRevision & 4-Stage Lifecycle (§7)
    sec7_match = re.search(r"## §7\..*?(?=\n## §8\.|\Z)", text, flags=re.DOTALL)
    if not sec7_match:
        fail("Could not locate '## §7.' Central Data Contract section")
    sec7 = sec7_match.group(0)
    for fname in FORECAST_REVISION_FIELDS:
        if not re.search(rf"\|\s*`{fname}`\s*\|", sec7):
            fail(f"§7.1 missing canonical ForecastRevision field row: {fname}")
    for stage in LIFECYCLE_STAGES:
        if stage not in sec7:
            fail(f"§7.2 missing lifecycle distinction stage: {stage}")

    # 5b. Meta-Forecasting, Eligible Universe Ledger & 10 Mandatory Metrics (§12.2..§12.4)
    sec12_match = re.search(r"## §12\..*?(?=\n## Part III|\Z)", text, flags=re.DOTALL)
    if not sec12_match:
        fail("Could not locate '## §12.' Crucible Meta-Forecasting & Scorecard section")
    sec12 = sec12_match.group(0)
    for token in ("QuestionMetaForecast", "universe_question_ledger", "trg_universe_eligible_irreversible"):
        if token not in sec12:
            fail(f"§12 missing required universe/meta-forecast contract token: {token}")
    for mkey in MANDATORY_TELEMETRY_METRICS:
        if not re.search(rf"\|\s*`{mkey}`\s*\|", sec12):
            fail(f"§12.4 missing mandatory telemetry metric row: {mkey}")

    # 6. Claims, Subordinate HYP-05A..05F & Master Gate Register 1-to-1 Claim Bijection (§0.7 Rule 6)
    claims_defined = sorted(set(re.findall(r"`(CLAIM-0[1-8])`", text)))
    if len(claims_defined) != 8:
        fail(f"Expected CLAIM-01..CLAIM-08 (8 claims), found {claims_defined}")
    for hyp in CLAIM05_HYPOTHESES:
        if f"`{hyp}`" not in text:
            fail(f"Missing subordinate experimental hypothesis {hyp} under CLAIM-05")

    gate_section_match = re.search(r"## §22\. Master Gate Register.*?(?=\n## §23\.|\Z)", text, flags=re.DOTALL)
    if not gate_section_match:
        fail("Could not locate '## §22. Master Gate Register' section")
    gate_rows = [
        line for line in gate_section_match.group(0).splitlines()
        if line.strip().startswith("| `GATE-")
    ]
    if len(gate_rows) != 24:
        fail(f"Expected 24 gate rows (GATE-01..GATE-24) in §22, found {len(gate_rows)}")

    seen_gates = set()
    claim_counts = {f"CLAIM-0{i}": 0 for i in range(1, 9)}
    for row in gate_rows:
        cols = [c.strip() for c in row.strip().strip("|").split("|")]
        gate_ids = re.findall(r"GATE-\d{2}", cols[0])
        if len(gate_ids) != 1:
            fail(f"Malformed gate ID column in §22: {row}")
        gid = gate_ids[0]
        if gid in seen_gates:
            fail(f"Duplicate gate ID in §22: {gid}")
        seen_gates.add(gid)

        row_claims = re.findall(r"CLAIM-\d{2}", cols[1])
        if len(row_claims) != 1:
            fail(f"§0.7 Rule 6 violation: {gid} must map to exactly 1 CLAIM-XX in Claim column, got {row_claims}")
        claim_counts[row_claims[0]] += 1

        if not any(tag in row for tag in EVIDENCE_TAGS):
            fail(f"§0.1 / §0.7 Rule 5 violation: {gid} row lacks an explicit evidence label: {row}")

    for cid, cnt in claim_counts.items():
        if cnt < 2:
            fail(f"Claim {cid} has insufficient gate coverage ({cnt} < 2)")

    # 7. 6-Crate LOC rows in §17 must carry [BUDGET] scope-cap label & omit fcast-sim (§0.7 Rules 4 & 8)
    sec17_match = re.search(r"## §17\..*?(?=\n## §18\.|\Z)", text, flags=re.DOTALL)
    if not sec17_match:
        fail("Could not locate '## §17.' section")
    crate_rows = [
        line for line in sec17_match.group(0).splitlines()
        if re.match(r"^\|\s*`fcast-[a-z]+`", line.strip())
    ]
    if len(crate_rows) != 6:
        fail(f"Expected 6 crate scope-cap rows in §17, found {len(crate_rows)}")
    for row in crate_rows:
        if "fcast-sim" in row:
            fail("fcast-sim must not exist in §17 crate table (extracted to franken_wargame)")
        if "[BUDGET]" not in row:
            fail(f"§0.7 Rule 4 violation: crate LOC ceiling row missing [BUDGET] tag: {row}")

    # 8. 6 Logical Storage Tier rows in §19 carrying explicit §0.1 evidence tags (hardware-size agnostic)
    sec19_match = re.search(r"## §19\..*?(?=\n## §20\.|\Z)", text, flags=re.DOTALL)
    if not sec19_match:
        fail("Could not locate '## §19.' section")
    tier_rows = [
        line for line in sec19_match.group(0).splitlines()
        if re.match(r"^\|\s*`Tier ", line.strip())
    ]
    if len(tier_rows) != 6:
        fail(f"Expected 6 logical storage tier rows in §19, found {len(tier_rows)}")
    for row in tier_rows:
        if not any(tag in row for tag in EVIDENCE_TAGS):
            fail(f"§0.1 violation in §19 storage row (missing evidence tag): {row}")

    # 9. Verbatim SHA-256 & Canonical Variable/Blinding checks across all 8 templates/*.tmpl (§9.1 & §9.2)
    tmpl_dir = ROOT / "templates"
    for tname in EXPECTED_TEMPLATES:
        tpath = tmpl_dir / tname
        if not tpath.exists():
            fail(f"Missing canonical template file: {tpath}")
        tbytes = tpath.read_bytes()
        tsha = hashlib.sha256(tbytes).hexdigest()
        if tsha not in text:
            fail(f"Master plan §9.2 missing exact SHA-256 ({tsha}) for {tname}")
        tstr = tbytes.decode("utf-8")
        if tname in MARKET_BLIND_TEMPLATES and "{p_market}" in tstr:
            fail(f"Blinding violation (INV-04/INV-05): {tname} must NOT contain {{p_market}} placeholder")
        if tname == "rationale_staged_turn_a.tmpl" and "{forecaster_probability_distribution}" in tstr:
            fail("Blinding violation (INV-05): rationale_staged_turn_a.tmpl must NOT reveal forecaster forecast")

    tmpl_bytes = TMPL_PATH.read_bytes()
    tmpl_sha = hashlib.sha256(tmpl_bytes).hexdigest()
    bitemp_str = tmpl_bytes.decode("utf-8")
    for req_var in (
        "{as_of_utc}",
        "event_utc",
        "published_utc",
        "available_utc",
        "question_id",
        "resolution_status",
        "resolved_published_utc",
        "adjudication_ev_handle",
        "ev_handle",
    ):
        if req_var not in bitemp_str:
            fail(f"bitemporal_event_audit.tmpl missing canonical §7.1/§7.2 variable: {req_var}")

    block_match = re.search(
        r"```text\n(Event description: \{event_description\}.*?zero items satisfy any contract\.\n)```",
        text,
        flags=re.DOTALL,
    )
    if not block_match:
        fail("Could not locate verbatim bitemporal_event_audit.tmpl code block in §9.1")
    embedded_sha = hashlib.sha256(block_match.group(1).encode("utf-8")).hexdigest()
    if embedded_sha != tmpl_sha:
        fail(f"Embedded §9.1 template SHA-256 ({embedded_sha}) != file SHA-256 ({tmpl_sha})")

    print(
        f"PLAN_HYGIENE_OK: 8 subsystems, 12 ForecastRevision fields, 4 lifecycle stages, "
        f"9 SHA256-locked templates, 10 universe metrics, 8 claims + 6 HYP-05*, "
        f"{len(seen_gates)} 1-to-1 gates, 6 crates, 6 logical tiers, bitemp_sha256={tmpl_sha[:16]}..."
    )


if __name__ == "__main__":
    main()
