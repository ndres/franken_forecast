#!/usr/bin/env python3
"""
Exports deterministic Python reference math outputs to tests/golden/math_vectors.json
for <= 1e-9 cross-language parity verification by crates/fcast-core/tests/golden_parity.rs (§0.2 Rule 6).
"""

from __future__ import annotations
import json
import math
import pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT_PATH = ROOT / "tests" / "golden" / "math_vectors.json"

# Import exact reference functions from scripts/verify_math_and_contracts.py
import importlib.util

spec = importlib.util.spec_from_file_location("vmc", ROOT / "scripts" / "verify_math_and_contracts.py")
assert spec and spec.loader
vmc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(vmc)


def kalman_blend_ref(p_indep: float, prior_var: float, p_market: float, phi_liq: float, r0: float):
    mu0 = vmc.logit(p_indep)
    v0 = max(prior_var, 1e-6)
    r_t = max(r0, 1e-6) / max(phi_liq, 1e-3)
    k = v0 / (v0 + r_t)
    mu1 = mu0 + k * (vmc.logit(p_market) - mu0)
    v1 = (1.0 - k) * v0
    kl = 0.5 * ((v1 / v0) + ((mu1 - mu0) ** 2 / v0) - 1.0 + math.log(v0 / v1))
    return {"p_post": vmc.sigmoid(mu1), "post_var": v1, "bayesian_surprise_nats": max(0.0, kl)}


def main() -> None:
    OUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    a_up, b_up, mu_up, n_up, var_up = vmc.stream_belief_step(
        8.0, 2.0, 0.50, 4.0, "geopolitics", "PointInTime", 0.0, 500.0, 1000.0, 0.5, "Up", +0.25, 0.6, 1.0
    )
    a_dead, b_dead, mu_dead, n_dead, var_dead = vmc.stream_belief_step(
        1.2, 8.8, 0.12, 10.0, "geopolitics", "CumulativeStoppingTime", 500.0, 36.0, 8760.0, 0.5, "Still", 0.0, 0.0, 1.0
    )
    a_shk, b_shk, mu_shk, n_shk, var_shk = vmc.stream_belief_step(
        1.25, 23.75, 0.05, 5.0, "geopolitics", "PointInTime", 1.0, 300.0, 600.0, 2.8, "Up", +2.2, 0.9, 1.0
    )

    payload = {
        "stream_up": {"alpha": a_up, "beta": b_up, "mean": mu_up, "n_eff": n_up, "var": var_up},
        "stream_deadline": {"alpha": a_dead, "beta": b_dead, "mean": mu_dead, "n_eff": n_dead, "var": var_dead},
        "stream_shock": {"alpha": a_shk, "beta": b_shk, "mean": mu_shk, "n_eff": n_shk, "var": var_shk},
        "king_zeng_070_050_001": vmc.prior_shift_prob(0.70, 0.50, 0.01),
        "kalman_deep": kalman_blend_ref(0.40, 0.25, 0.50, 0.90, 0.10),
        "kalman_thin": kalman_blend_ref(0.40, 0.25, 0.50, 0.01, 0.10),
        "cdf_linear_201": vmc.build_metaculus_cdf([(20, 0.10), (50, 0.50), (80, 0.90)], 0.0, 100.0, False, False),
        "cdf_log_201": vmc.build_metaculus_cdf([(10, 0.15), (100, 0.55), (1000, 0.85)], 1.0, 10000.0, True, True, zero_point=0.0),
        "cdf_discrete_16": vmc.build_metaculus_cdf([(3, 0.20), (8, 0.60), (12, 0.90)], 0.0, 15.0, False, True, inbound_outcome_count=15),
        "pav_mono": vmc.pav_isotonic_nondecreasing([0.30, 0.24, 0.50, 0.48]),
        "eb_shrink_0_3_020_5": vmc.eb_shrink_base_rate(0, 3, 0.20, 5.0),
        "m_eff_4_05": vmc.effective_model_count(4, 0.5),
    }
    OUT_PATH.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    print(f"GOLDEN_VECTORS_WRITTEN: {OUT_PATH}")


if __name__ == "__main__":
    main()
