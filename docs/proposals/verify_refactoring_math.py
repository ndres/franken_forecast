#!/usr/bin/env python3
"""
Numerical witnesses for REFACTORING_PROPOSAL.md (§5 Recalibration, §6 Market-Anchored Blending & Latent Surprise).

W1  Platt scaling with slope a and extremizing exponent d compose into ONE affine logit map:
    sigmoid(d * (a * logit p + b)) == sigmoid((d*a) * logit p + d*b)  -> a separate extremizer is redundant
    unless d is conditioned on a per-forecast uncertainty signal u (not available to Platt).
W2  Small-sample Platt overfits: with n = 25 resolved questions an unregularised 2-parameter fit loses to
    the identity map on holdout log-loss more often than a ridge-to-identity fit does.
W3  Latent-logit Kalman blending: market observation noise scales with 1/phi_liq, so the same 10pp price
    jump yields far larger Bayesian surprise (KL posterior||prior) on a deep book than on a thin one.
W4  Precision-weighted logit blending == Kalman update with two observations (closed-form identity).
"""
from __future__ import annotations
import math
import random


def sigmoid(x: float) -> float:
    return 1.0 / (1.0 + math.exp(-x))


def logit(p: float) -> float:
    p = min(max(p, 1e-6), 1 - 1e-6)
    return math.log(p / (1 - p))


# ---------------------------------------------------------------- W1
def extremize(p: float, d: float) -> float:
    return p ** d / (p ** d + (1 - p) ** d)


a, b, d = 1.3, -0.1, 1.5
for p in (0.03, 0.2, 0.5, 0.71, 0.97):
    two_step = extremize(sigmoid(a * logit(p) + b), d)
    one_step = sigmoid((d * a) * logit(p) + d * b)
    assert abs(two_step - one_step) < 1e-12


def uncertainty_conditioned_extremize(p: float, u: float, d_max: float = 1.6) -> float:
    """u in [0,1]: 0 = confident/independent evidence, 1 = high disagreement. d -> 1 as u -> 1."""
    dd = 1.0 + (d_max - 1.0) * (1.0 - min(max(u, 0.0), 1.0))
    return sigmoid(dd * logit(p))


assert abs(uncertainty_conditioned_extremize(0.7, 1.0) - 0.7) < 1e-12
assert uncertainty_conditioned_extremize(0.7, 0.0) > 0.75


# ---------------------------------------------------------------- W2
def fit_platt(xs, ys, ridge: float, iters: int = 50):
    """Newton-Raphson logistic regression on logit(p) with Gaussian prior centred at identity (a=1, b=0)."""
    A, B = 1.0, 0.0
    for _ in range(iters):
        ga = gb = 0.0
        haa, hab, hbb = ridge, 0.0, ridge
        ga += ridge * (A - 1.0)
        gb += ridge * B
        for x, y in zip(xs, ys):
            q = sigmoid(A * x + B)
            r = q - y
            w = q * (1 - q)
            ga += r * x
            gb += r
            haa += w * x * x
            hab += w * x
            hbb += w
        det = haa * hbb - hab * hab
        if abs(det) < 1e-12:
            break
        A -= (hbb * ga - hab * gb) / det
        B -= (-hab * ga + haa * gb) / det
        A, B = max(min(A, 10.0), -10.0), max(min(B, 10.0), -10.0)
    return A, B


def logloss(xs, ys, A, B):
    s = 0.0
    for x, y in zip(xs, ys):
        q = min(max(sigmoid(A * x + B), 1e-6), 1 - 1e-6)
        s -= y * math.log(q) + (1 - y) * math.log(1 - q)
    return s / len(xs)


def synth(rng, n, true_slope=1.25):
    """Raw forecasts are mildly under-confident (truth = sigmoid(1.25 * logit p_raw))."""
    xs, ys = [], []
    for _ in range(n):
        z = rng.gauss(0.0, 1.4)
        xs.append(z)
        ys.append(1 if rng.random() < sigmoid(true_slope * z) else 0)
    return xs, ys


rng = random.Random(20261125)
wins_raw = wins_ridge = 0
trials = 300
for _ in range(trials):
    xtr, ytr = synth(rng, 25)
    xte, yte = synth(rng, 2000)
    ident = logloss(xte, yte, 1.0, 0.0)
    a0, b0 = fit_platt(xtr, ytr, ridge=1e-6)
    a1, b1 = fit_platt(xtr, ytr, ridge=8.0)
    wins_raw += logloss(xte, yte, a0, b0) < ident
    wins_ridge += logloss(xte, yte, a1, b1) < ident
rate_raw, rate_ridge = wins_raw / trials, wins_ridge / trials
assert rate_ridge > rate_raw, (rate_raw, rate_ridge)


# ---------------------------------------------------------------- W3 / W4
def kl_gauss(m1, v1, m0, v0):
    return 0.5 * (v1 / v0 + (m1 - m0) ** 2 / v0 - 1.0 + math.log(v0 / v1))


def kalman_market_step(m, v, q_drift, y_market_logit, phi_liq, r0=0.04):
    v_prior = v + q_drift
    r = r0 / max(phi_liq, 1e-3)
    k = v_prior / (v_prior + r)
    m_post = m + k * (y_market_logit - m)
    v_post = (1 - k) * v_prior
    return m_post, v_post, kl_gauss(m_post, v_post, m, v_prior)


m0, v0 = logit(0.40), 0.05
_, _, surprise_deep = kalman_market_step(m0, v0, 0.01, logit(0.50), phi_liq=0.90)
_, _, surprise_thin = kalman_market_step(m0, v0, 0.01, logit(0.50), phi_liq=0.05)
assert surprise_deep > 5 * surprise_thin, (surprise_deep, surprise_thin)

# W4: two-observation Kalman == precision-weighted logit average
prior_m, prior_v = logit(0.30), 0.5
ai_m, ai_v = logit(0.42), 0.20
mk_m, mk_v = logit(0.36), 0.08
prec = 1 / prior_v + 1 / ai_v + 1 / mk_v
blend = (prior_m / prior_v + ai_m / ai_v + mk_m / mk_v) / prec
k1 = prior_v / (prior_v + ai_v)
m1, v1 = prior_m + k1 * (ai_m - prior_m), (1 - k1) * prior_v
k2 = v1 / (v1 + mk_v)
m2 = m1 + k2 * (mk_m - m1)
assert abs(m2 - blend) < 1e-12

print(
    "REFACTORING_WITNESSES_OK: "
    f"W1 platt∘extremize collapses; W2 n=25 beat-identity rate raw={rate_raw:.2f} ridge={rate_ridge:.2f}; "
    f"W3 surprise deep={surprise_deep:.3f} thin={surprise_thin:.4f} nats; W4 blend==kalman"
)
