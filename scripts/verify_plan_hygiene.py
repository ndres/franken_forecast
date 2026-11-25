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
        "## Part III ·",
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

    # 3. Pure-CPU & External Sister-Project Boundary (§0.7 Rule 8)
    for forbidden in ("RTX 3090", "GDDR6X", "VRAM", "Slot A:", "`SUB-09`"):
        if forbidden in text:
            fail(f"§0.7 Rule 8 boundary violation: forbidden GPU/in-repo-sim token {forbidden!r} present")

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

    # 6. Claims & Master Gate Register 1-to-1 Claim Bijection (§0.7 Rule 6)
    claims_defined = sorted(set(re.findall(r"`(CLAIM-0[1-8])`", text)))
    if len(claims_defined) != 8:
        fail(f"Expected CLAIM-01..CLAIM-08 (8 claims), found {claims_defined}")

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

    # 8. 6 Storage Tier rows in §19.2 carrying explicit §0.1 evidence tags and summing to 4,000 GB
    sec19_match = re.search(r"## §19\..*?(?=\n## §20\.|\Z)", text, flags=re.DOTALL)
    if not sec19_match:
        fail("Could not locate '## §19.' section")
    tier_rows = [
        line for line in sec19_match.group(0).splitlines()
        if re.match(r"^\|\s*`Tier ", line.strip())
    ]
    if len(tier_rows) != 6:
        fail(f"Expected 6 NVMe storage tier rows in §19.2, found {len(tier_rows)}")
    total_gb = 0
    for row in tier_rows:
        if not any(tag in row for tag in EVIDENCE_TAGS):
            fail(f"§0.1 violation in §19 storage row (missing evidence tag): {row}")
        cols = [c.strip() for c in row.strip().strip("|").split("|")]
        m_gb = re.search(r"`([\d,]+)\s*GB`", cols[3])
        if not m_gb:
            fail(f"Could not parse GB quota in §19.2 row: {row}")
        total_gb += int(m_gb.group(1).replace(",", ""))
    if total_gb != 4000:
        fail(f"Expected 6 storage tiers to sum to 4,000 GB, got {total_gb} GB")

    # 9. Verbatim SHA-256 check on bitemporal_event_audit.tmpl vs §9 fenced block
    tmpl_bytes = TMPL_PATH.read_bytes()
    tmpl_sha = hashlib.sha256(tmpl_bytes).hexdigest()
    if tmpl_sha not in text:
        fail(f"Master plan does not record exact template SHA-256 ({tmpl_sha})")

    block_match = re.search(
        r"```text\n(Event \(description \): \{event\}.*?least strict market\)\.\n)```",
        text,
        flags=re.DOTALL,
    )
    if not block_match:
        fail("Could not locate verbatim bitemporal_event_audit.tmpl code block in §9")
    embedded_sha = hashlib.sha256(block_match.group(1).encode("utf-8")).hexdigest()
    if embedded_sha != tmpl_sha:
        fail(f"Embedded §9 template SHA-256 ({embedded_sha}) != file SHA-256 ({tmpl_sha})")

    print(
        f"PLAN_HYGIENE_OK: 8 subsystems, 12 ForecastRevision fields, 4 lifecycle stages, "
        f"8 claims, {len(seen_gates)} 1-to-1 gates, 6 crates, {total_gb}GB NVMe, "
        f"0 GPU/sim drift, template_sha256={tmpl_sha[:16]}..."
    )


if __name__ == "__main__":
    main()
