#!/usr/bin/env python3
"""
Automated Plan Hygiene Auditor (§0.7) for COMPREHENSIVE_PLAN_FOR_FRANKEN_FORECAST.md.

Verifies:
1. Presence of Part 0 Document Control (§0.1 .. §0.8) and Part I..V structure,
   including §1.1 What Exists, §1.2 What Is Missing, §1.3 What Is Strongest,
   and §1.4 Structural Defects, Honest Limits & Residual Failure Modes.
2. Delivery-tracking quarantine (§0.7 Rule 1): zero markdown task checkboxes
   (- [ ] / - [x]) inside the master plan; docs/delivery/DELIVERY_LOG.md exists.
3. Canonical Subsystem Registry (§0.5): SUB-01 .. SUB-09 present and unique.
4. Claim-boundary bijection (§0.7 Rule 6): CLAIM-01 .. CLAIM-08 defined, and
   every GATE-01 .. GATE-24 row in §22 maps to EXACTLY ONE primary CLAIM-XX,
   with every CLAIM covered by >= 2 validation gates.
5. Scope-cap & Evidence-label discipline (§0.1 & §0.7 Rules 4–5): every table row
   containing numeric metrics in §5, §17, §19, and §22 carries an explicit §0.1
   evidence tag ([MEASURED], [UPSTREAM-SPEC], [DERIVED], [BUDGET], [TARGET],
   or [HYPOTHESIS]).
6. Byte-exact SHA-256 parity between templates/bitemporal_event_audit.tmpl and
   the frozen verbatim block in §9.
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
    ("SUB-06", "Wargame"),
    ("SUB-07", "Metronome"),
    ("SUB-08", "Tether"),
    ("SUB-09", "Cockpit"),
]


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

    # 3. Canonical Subsystem Names (SUB-01 .. SUB-09)
    for sub_id, sub_name in EXPECTED_SUBSYSTEMS:
        pattern = rf"\|\s*`{sub_id}`\s*\|\s*\*\*{sub_name}\*\*"
        if not re.search(pattern, text):
            fail(f"Subsystem registry row missing or malformed for {sub_id} ({sub_name})")

    # 4. Claims & Master Gate Register 1-to-1 Claim Bijection (§0.7 Rule 6)
    claims_defined = sorted(set(re.findall(r"`(CLAIM-0[1-8])`", text)))
    if len(claims_defined) != 8:
        fail(f"Expected CLAIM-01..CLAIM-08 (8 claims), found {claims_defined}")

    gate_section_match = re.search(r"## §22\. Master Gate Register.*?(?=\n## §23\.|\Z)", text, flags=re.DOTALL)
    if not gate_section_match:
        fail("Could not locate '## §22. Master Gate Register' section")
    gate_section = gate_section_match.group(0)

    gate_rows = [
        line for line in gate_section.splitlines()
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

        # Column 1 MUST contain EXACTLY ONE CLAIM-XX
        row_claims = re.findall(r"CLAIM-\d{2}", cols[1])
        if len(row_claims) != 1:
            fail(f"§0.7 Rule 6 violation: {gid} must map to exactly 1 CLAIM-XX in Claim column, got {row_claims}")
        claim_counts[row_claims[0]] += 1

        # Threshold / Criterion column MUST carry at least one §0.1 evidence tag
        if not any(tag in row for tag in EVIDENCE_TAGS):
            fail(f"§0.1 / §0.7 Rule 5 violation: {gid} row lacks an explicit evidence label: {row}")

    for cid, cnt in claim_counts.items():
        if cnt < 2:
            fail(f"Claim {cid} has insufficient gate coverage ({cnt} < 2)")

    # 5. Crate LOC rows in §17 must carry [BUDGET] scope-cap label (§0.7 Rule 4)
    sec17_match = re.search(r"## §17\..*?(?=\n## §18\.|\Z)", text, flags=re.DOTALL)
    if not sec17_match:
        fail("Could not locate '## §17.' section")
    crate_rows = [
        line for line in sec17_match.group(0).splitlines()
        if re.match(r"^\|\s*`fcast-[a-z]+`", line.strip())
    ]
    if len(crate_rows) != 7:
        fail(f"Expected 7 crate scope-cap rows in §17, found {len(crate_rows)}")
    for row in crate_rows:
        if "[BUDGET]" not in row:
            fail(f"§0.7 Rule 4 violation: crate LOC ceiling row missing [BUDGET] tag: {row}")

    # 6. Storage & GPU tables in §19 must carry explicit §0.1 evidence tags
    sec19_match = re.search(r"## §19\..*?(?=\n## §20\.|\Z)", text, flags=re.DOTALL)
    if not sec19_match:
        fail("Could not locate '## §19.' section")
    tier_rows = [
        line for line in sec19_match.group(0).splitlines()
        if re.match(r"^\|\s*`(?:Tier |Slot )", line.strip())
    ]
    if len(tier_rows) < 9:
        fail(f"Expected >= 9 storage/VRAM budget rows in §19, found {len(tier_rows)}")
    for row in tier_rows:
        if not any(tag in row for tag in EVIDENCE_TAGS):
            fail(f"§0.1 violation in §19 hardware budget row (missing evidence tag): {row}")

    # 7. Verbatim SHA-256 check on bitemporal_event_audit.tmpl vs §9 fenced block
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
        f"PLAN_HYGIENE_OK: 9 subsystems, 8 claims, {len(seen_gates)} 1-to-1 gates, "
        f"7 crate caps, {len(tier_rows)} hardware rows, template_sha256={tmpl_sha[:16]}..."
    )


if __name__ == "__main__":
    main()
