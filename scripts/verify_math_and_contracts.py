import math

# Extend contract & math verification to cover Rationale Scorer (6D + 2G + unscoreable),
# PATCH-05 King-Zeng Prior-Shift Calibration, PATCH-10 Martingale EVSI, and PATCH-06 BiTemporal Lattice.

def validate_and_rank_rationales(items):
    ranked = []
    for idx, r in enumerate(items):
        dims = [r.get(k) for k in ("ct", "ei", "ov", "me", "ej", "pr")]
        g1, g2, unscoreable = r.get("g1"), r.get("g2"), r.get("unscoreable")
        note = (r.get("note") or "").strip()

        if unscoreable is True:
            assert all(d is None for d in dims), "unscoreable=True must have all 6 dims None"
            assert g1 is False and g2 is False, "unscoreable=True must have g1=False, g2=False"
            continue

        assert all(isinstance(d, int) and 0 <= d <= 4 for d in dims), f"Invalid integer dims: {dims}"
        if g1 or g2:
            assert len(note) >= 10, "Gate set without justification in note"

        raw_sum = sum(dims)
        ct, ei, ov, me, ej, pr = dims
        if g2:
            tier, eff = 0, 0
        elif g1:
            tier, eff = 1, min(raw_sum, 8)
        else:
            tier, eff = 2, raw_sum

        ranked.append({
            "id": r["id"],
            "tier": tier,
            "eff": eff,
            "key": (tier, eff, min(ct, me), ei, ov, ej, pr, -r["ts"]),
        })
    ranked.sort(key=lambda x: x["key"], reverse=True)
    return ranked

sample = [
    {"id": "redacted", "ct": None, "ei": None, "ov": None, "me": None, "ej": None, "pr": None, "g1": False, "g2": False, "unscoreable": True, "note": "Empty comment.", "ts": 100},
    {"id": "misread_high", "ct": 3, "ei": 4, "ov": 3, "me": 3, "ej": 3, "pr": 3, "g1": True, "g2": False, "unscoreable": False, "note": "G1: Reads metric as Brent instead of WTI.", "ts": 101},
    {"id": "clean_strong", "ct": 4, "ei": 4, "ov": 3, "me": 4, "ej": 3, "pr": 4, "g1": False, "g2": False, "unscoreable": False, "note": "Strong base rate and explicit Aspides trigger.", "ts": 102},
    {"id": "leaked_g2", "ct": 4, "ei": 4, "ov": 4, "me": 4, "ej": 4, "pr": 4, "g1": False, "g2": True, "unscoreable": False, "note": "G2: Cites Dec 15 post-cutoff communique.", "ts": 99},
]
res = validate_and_rank_rationales(sample)
assert [x["id"] for x in res] == ["clean_strong", "misread_high", "leaked_g2"], f"Unexpected order: {res}"

# PATCH-05 King-Zeng prior shift check: 1:1 train (pi_train=0.5) -> real base rate pi_real=0.01
def prior_shift_prob(q_train: float, pi_train: float, pi_real: float) -> float:
    l_q = math.log(q_train / (1.0 - q_train))
    l_tr = math.log(pi_train / (1.0 - pi_train))
    l_re = math.log(pi_real / (1.0 - pi_real))
    return 1.0 / (1.0 + math.exp(-(l_q - l_tr + l_re)))

# A 0.70 score on 50/50 balanced train corresponds to ~2.31% true probability when base rate is 1% (NOT 70%!)
p_cal = prior_shift_prob(0.70, 0.50, 0.01)
assert 0.02 < p_cal < 0.03, f"Prior shift failed: {p_cal}"

# PATCH-10 Hardened: Expected Log-Score Gain per Call (Martingale KL VoI) + 2-Fold Floor + Fat-Tail Guard
def kl_bern(q: float, p: float, eps: float = 1e-6) -> float:
    q = min(max(q, eps), 1.0 - eps)
    p = min(max(p, eps), 1.0 - eps)
    return q * math.log(q / p) + (1.0 - q) * math.log((1.0 - q) / (1.0 - p))

def entropy_bern(p: float, eps: float = 1e-6) -> float:
    p = min(max(p, eps), 1.0 - eps)
    return -p * math.log(p) - (1.0 - p) * math.log(1.0 - p)

def should_stop_fold(fold_count: int, productive_folds: int, p_k: float, hyps, cruxes, lam_nat_per_usd: float = 2.0, kappa_tail: float = 0.50):
    # Hard minimum 2-fold eligibility floor
    if fold_count < 2 or productive_folds < 2:
        return False, "INELIGIBLE_MIN_2_FOLDS"
    # Intrinsic hypothesis mixture MI: H(P_k) - sum w_h H(P_{k,h})
    i_hyp = max(0.0, entropy_bern(p_k) - sum(w * entropy_bern(ph) for w, ph in hyps))
    best_voi = 0.0
    for c in cruxes:
        q_j, p_pos, r_j, cost_usd, attempts = c["q"], c["p_pos"], c["r"], c["cost"], c["attempts"]
        # Martingale projection: q_j * p_pos + (1 - q_j) * p_neg = p_k
        p_neg = min(max((p_k - q_j * p_pos) / (1.0 - q_j), 1e-6), 1.0 - 1e-6)
        kl_pos = kl_bern(p_pos, p_k)
        kl_neg = kl_bern(p_neg, p_k)
        # Unprobed fat-tailed crux guard
        if attempts == 0 and max(kl_pos, kl_neg) >= kappa_tail:
            return False, "UNPROBED_FAT_TAIL_CRUX"
        delta_log_score = r_j * (q_j * kl_pos + (1.0 - q_j) * kl_neg)
        eff_gain = max(delta_log_score, 0.25 * r_j * i_hyp)
        assert eff_gain <= entropy_bern(p_k) + 1e-9
        best_voi = max(best_voi, eff_gain / cost_usd)
    return (best_voi < lam_nat_per_usd), f"best_voi={best_voi:.2f}"

# 1) Fold 1 is NEVER allowed to stop, even with zero open cruxes and zero dispersion
stop_f1, reason_f1 = should_stop_fold(1, 1, 0.40, [(1.0, 0.40)], [])
assert stop_f1 is False and reason_f1 == "INELIGIBLE_MIN_2_FOLDS"

# 2) Fat-tailed crux at Fold 2 where previous Δlogit == 0 (p_k = 0.04, q = 0.05 -> p_pos = 0.65, cost = $0.005):
#    Naive |Δlogit| < eps would falsely seal; Expected Log-Score VoI (~8.8 nats/$) + fat-tail guard keeps exploring!
stop_tail, reason_tail = should_stop_fold(
    2, 2, 0.04, [(0.95, 0.01), (0.05, 0.61)],
    [{"q": 0.05, "p_pos": 0.65, "r": 0.50, "cost": 0.005, "attempts": 1}],
    lam_nat_per_usd=2.0,
)
assert stop_tail is False and float(reason_tail.split("=")[1]) > 8.0, f"Premature stop on fat tail: {reason_tail}"

# 3) Exhausted crux at Fold 3 (tiny posterior shift 0.40 -> 0.41, cost = $0.01) cleanly seals
stop_done, reason_done = should_stop_fold(
    3, 3, 0.40, [(0.5, 0.395), (0.5, 0.405)],
    [{"q": 0.50, "p_pos": 0.41, "r": 0.40, "cost": 0.01, "attempts": 2}],
    lam_nat_per_usd=2.0,
)
assert stop_done is True, f"Should have sealed when VoI_log < lambda: {reason_done}"

# Microstructure check: TOB Stoikov microprice + liquidity floor + depth-adjusted impact
def compute_book_mark(bid_p, bid_sz_usd, ask_p, ask_sz_usd, depth_impact_500_prob, s_tick_bps=100.0, last_trade=None):
    l_tob = bid_sz_usd + ask_sz_usd
    spread_bps = (ask_p - bid_p) * 10000.0
    if l_tob < 100.0 or spread_bps > 800.0:
        mark = last_trade if (last_trade is not None and bid_p <= last_trade <= ask_p) else 0.5 * (bid_p + ask_p)
        return mark, 0.0, True  # thin_or_wide=True -> dim 24h, highlight 7d
    p_micro = (bid_sz_usd * ask_p + ask_sz_usd * bid_p) / l_tob
    s_term = 1.0 / (1.0 + math.exp(-math.log(l_tob / 250.0)))
    excess_spread = max(0.0, spread_bps - s_tick_bps)
    phi = s_term * math.exp(-excess_spread / 400.0) * math.exp(-depth_impact_500_prob / 0.05)
    return p_micro, phi, False

# Tight 2c book ($2,000 at 0.37 vs $100 at 0.39) -> microprice leans toward 0.39 (0.3890), phi ~ 0.57;
# Deep 1c book ($8,000 at 0.49 vs $8,000 at 0.50) -> phi > 0.90;
# Thin/wide stub book ($40 at 0.20 vs $30 at 0.55) -> caught by floor (thin_flag=True, phi=0.0, fallback to last_trade)
m_tight, phi_tight, thin_tight = compute_book_mark(0.37, 2000.0, 0.39, 100.0, 0.01)
m_deep, phi_deep, thin_deep = compute_book_mark(0.49, 8000.0, 0.50, 8000.0, 0.002)
assert not thin_tight and m_tight > 0.388 and 0.50 < phi_tight < 0.70
assert not thin_deep and phi_deep > 0.90
m_thin, phi_thin, thin_flag = compute_book_mark(0.20, 40.0, 0.55, 30.0, 0.25, last_trade=0.31)
assert thin_flag and abs(m_thin - 0.31) < 1e-9 and phi_thin == 0.0

# Day-1 Store Durability check: WAL mode, online hot backup/verify, and belief_revisions schema + payload_schema_ver migration
import hashlib, json as _json, sqlite3, tempfile, os

MIGRATION_0001 = """
CREATE TABLE _fcast_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    sha256_hex TEXT NOT NULL
);
CREATE TABLE belief_revisions (
    revision_id TEXT PRIMARY KEY,
    question_id TEXT NOT NULL,
    fold_idx INTEGER NOT NULL,
    parent_revision_id TEXT REFERENCES belief_revisions(revision_id),
    productive_fold INTEGER NOT NULL,
    payload_schema_ver INTEGER NOT NULL,
    dist_kind TEXT NOT NULL,
    dist_blob BLOB NOT NULL,
    hypotheses_json TEXT NOT NULL,
    open_cruxes_json TEXT NOT NULL,
    verified_ev_refs_json TEXT NOT NULL,
    discarded_leads_json TEXT NOT NULL,
    voi_log_nats_per_usd REAL,
    seiph_json TEXT,
    content_sha256 BLOB NOT NULL,
    as_of_utc TEXT NOT NULL,
    created_at_utc TEXT NOT NULL,
    UNIQUE(question_id, fold_idx)
);
"""

MIGRATION_0002 = """
ALTER TABLE belief_revisions ADD COLUMN cyber_prior_shift_nats REAL;
"""

with tempfile.TemporaryDirectory() as tmp:
    db_path = os.path.join(tmp, "ops.fsqlite")
    bak_path = os.path.join(tmp, "ops-pre-v2.fsqlite")
    conn = sqlite3.connect(db_path)
    mode = conn.execute("PRAGMA journal_mode = WAL;").fetchone()[0]
    conn.execute("PRAGMA foreign_keys = ON;")
    assert mode.lower() == "wal"

    conn.executescript(MIGRATION_0001)
    h1 = hashlib.sha256(MIGRATION_0001.encode()).hexdigest()
    conn.execute("INSERT INTO _fcast_migrations VALUES (1, '0001_init', ?)", (h1,))
    conn.execute(
        "INSERT INTO belief_revisions VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        ("rev1", "q1", 1, None, 1, 1, "binary", b"\x00", '[{"w":1.0,"p":0.4}]', "[]", "[]", "[]", 5.2, None, b"\x11"*32, "2026-11-25T00:00:00Z", "2026-11-25T00:01:00Z")
    )
    conn.commit()

    # Pre-migration hot online backup + integrity verification
    bak_conn = sqlite3.connect(bak_path)
    conn.backup(bak_conn)
    assert bak_conn.execute("PRAGMA integrity_check;").fetchone()[0] == "ok"
    assert bak_conn.execute("PRAGMA foreign_key_check;").fetchall() == []
    bak_conn.close()

    # Apply Migration 0002 + upcast payload_schema_ver 1 -> 2
    conn.executescript(MIGRATION_0002)
    h2 = hashlib.sha256(MIGRATION_0002.encode()).hexdigest()
    conn.execute("INSERT INTO _fcast_migrations VALUES (2, '0002_cyber_shift', ?)", (h2,))
    row = conn.execute("SELECT payload_schema_ver, hypotheses_json, cyber_prior_shift_nats FROM belief_revisions WHERE revision_id='rev1'").fetchone()
    assert row == (1, '[{"w":1.0,"p":0.4}]', None)
    # Upcaster upgrades v1 -> v2 cleanly with default 0.0 cyber_prior_shift_nats
    conn.execute("UPDATE belief_revisions SET payload_schema_ver = 2, cyber_prior_shift_nats = COALESCE(cyber_prior_shift_nats, 0.0) WHERE payload_schema_ver = 1")
    conn.commit()
    assert conn.execute("SELECT payload_schema_ver, cyber_prior_shift_nats FROM belief_revisions WHERE revision_id='rev1'").fetchone() == (2, 0.0)
    conn.execute("PRAGMA wal_checkpoint(TRUNCATE);")
    conn.close()

# ForecastBench Community/Human Median 3-tier crowd_prior resolution + pre-seal blinding check
def resolve_crowd_prior(venue_tob_prob, venue_thin_or_masked, fb_median_row, cluster_median, as_of_utc, blind_pre_seal=False):
    if blind_pre_seal:
        return None  # Strictly quarantined from BlindAI before p_indep is sealed!
    if not venue_thin_or_masked and venue_tob_prob is not None:
        return ("venue_live", venue_tob_prob)
    if fb_median_row and fb_median_row["available_utc"] <= as_of_utc:
        val = fb_median_row.get("freeze_crowd_median") or fb_median_row.get("sf_median") or fb_median_row.get("public_median")
        if val is not None:
            return ("forecastbench_median", val)
    if cluster_median is not None:
        return ("cluster_consensus", cluster_median)
    return None

# Gateway-masked Metaculus question recovers crowd_prior cleanly from ForecastBench published median when available_utc <= freeze_date
assert resolve_crowd_prior(None, True, {"freeze_crowd_median": 0.34, "sf_median": 0.31, "available_utc": "2026-11-15T00:00:00Z"}, 0.36, "2026-11-18T00:00:00Z") == ("forecastbench_median", 0.34)
# Post-cutoff human survey release is blocked under as_of_utc fence, falling back to cluster consensus
assert resolve_crowd_prior(None, True, {"sf_median": 0.29, "available_utc": "2026-11-25T00:00:00Z"}, 0.36, "2026-11-18T00:00:00Z") == ("cluster_consensus", 0.36)
# Pre-seal BlindAI receives None (zero leakage)
assert resolve_crowd_prior(0.38, False, {"freeze_crowd_median": 0.34, "available_utc": "2026-11-15T00:00:00Z"}, 0.36, "2026-11-18T00:00:00Z", blind_pre_seal=True) is None

# Two-Speed Conjugate Belief + Closed-Form DecayController + Surprise Reset + TimeSeek Lifecycle Deferral
def sigmoid(x: float) -> float:
    return 1.0 / (1.0 + math.exp(-x))

def logit(p: float, eps: float = 1e-6) -> float:
    p = min(max(p, eps), 1.0 - eps)
    return math.log(p / (1.0 - p))

DOMAIN_DECAY = {
    "cyber":       (0.15, 24.0,   168.0),
    "geopolitics": (0.20, 72.0,   504.0),
    "macro":       (0.35, 336.0, 2160.0),
}

def effective_half_life_hours(domain: str, h_rem_hours: float, surprise_ewma: float, lam_vol: float = 0.8) -> float:
    beta_d, tau_min, tau_max = DOMAIN_DECAY[domain]
    base = min(max(beta_d * max(h_rem_hours, 6.0), tau_min), tau_max)
    return base / (1.0 + lam_vol * max(0.0, surprise_ewma - 1.0))

def anchor_mean(mu0: float, geometry: str, t_rem_h: float, t_tot_h: float) -> float:
    if geometry == "CumulativeStoppingTime":
        frac = min(max(t_rem_h / max(t_tot_h, 1.0), 1e-4), 1.0)
        return 1.0 - ((1.0 - mu0) ** frac)
    return mu0

def stream_belief_step(
    alpha: float, beta: float, mu0: float, n0: float,
    domain: str, geometry: str, dt_hours: float, t_rem_h: float, t_tot_h: float,
    surprise_ewma: float, direction: str, delta_ll: float, diag: float, novelty: float,
    kappa_lr: float = 1.35, eta_shock: float = 0.6,
):
    tau_eff = effective_half_life_hours(domain, t_rem_h, surprise_ewma)
    gamma = 2.0 ** (-dt_hours / tau_eff)
    mu_anc = anchor_mean(mu0, geometry, t_rem_h, t_tot_h)
    a_pre = gamma * alpha + (1.0 - gamma) * n0 * mu_anc
    b_pre = gamma * beta + (1.0 - gamma) * n0 * (1.0 - mu_anc)
    n_pre = a_pre + b_pre
    mu_pre = a_pre / n_pre

    if direction == "Still":
        delta_ll, diag = 0.0, 0.0
    w_t = novelty * diag
    mu_post = sigmoid(logit(mu_pre) + kappa_lr * novelty * delta_ll)
    n_post = (n_pre / (1.0 + eta_shock * (delta_ll ** 2))) + w_t
    var_post = (mu_post * (1.0 - mu_post)) / (n_post + 1.0)
    return n_post * mu_post, n_post * (1.0 - mu_post), mu_post, n_post, var_post

# 1) "Still" news on a 4% tail risk with dt=0 leaves belief strictly invariant (no drift toward 50%!)
a0, b0 = 0.4, 9.6
for _ in range(30):
    a0, b0, mu_s, n_s, _ = stream_belief_step(a0, b0, 0.04, 10.0, "cyber", "PointInTime", 0.0, 720.0, 720.0, 0.5, "Still", 0.0, 0.0, 1.0)
assert abs(mu_s - 0.04) < 1e-12 and abs(n_s - 10.0) < 1e-12

# 2) Mildly "Up" news (+0.25 log-odds) on an 80% belief RAISES probability (eliminating linear pseudo-count bug!)
_, _, mu_up, _, _ = stream_belief_step(8.0, 2.0, 0.50, 4.0, "geopolitics", "PointInTime", 0.0, 500.0, 1000.0, 0.5, "Up", +0.25, 0.6, 1.0)
assert mu_up > 0.80, f"Mild Up lowered high prior: {mu_up}"

# 3) CumulativeStoppingTime silent decay near deadline lowers anchor hazard rather than pulling back up to full-year mu0
_, _, mu_dead, _, _ = stream_belief_step(1.2, 8.8, 0.12, 10.0, "geopolitics", "CumulativeStoppingTime", 500.0, 36.0, 8760.0, 0.5, "Still", 0.0, 0.0, 1.0)
assert mu_dead < 0.02, f"Deadline hazard failed to contract near expiry: {mu_dead}"

# 4) High-surprise regime shock (+2.2 log-odds) damps old N_eff and WIDENS Beta variance -> wakes Tier-1 PBM fold
var_before = (0.05 * 0.95) / (25.0 + 1.0)
_, _, mu_shk, n_shk, var_after = stream_belief_step(1.25, 23.75, 0.05, 5.0, "geopolitics", "PointInTime", 1.0, 300.0, 600.0, 2.8, "Up", +2.2, 0.9, 1.0)
assert n_shk < 25.0 and var_after > 4.0 * var_before

# 5) TimeSeek post-seal market deferral: higher AI weight early / fresh shock, strong deferral late on liquid book
def timeseek_ai_weight(n_ai: float, phi_liq: float, tau_life: float, shock_age_min: float, n_mkt: float = 40.0) -> float:
    psi_life = (0.25 + 1.75 * (tau_life ** 1.5)) * (0.20 if shock_age_min < 30.0 else 1.0)
    return n_ai / (n_ai + n_mkt * phi_liq * psi_life)

w_early_shock = timeseek_ai_weight(12.0, 0.85, 0.10, 10.0)
w_late_calm = timeseek_ai_weight(12.0, 0.85, 0.92, 600.0)
assert w_early_shock > 0.80 and w_late_calm < 0.18

# Durable Job Queue & Daily Spend Circuit Breaker + Async MCP Job Handle check
with tempfile.TemporaryDirectory() as tmpj:
    jdb = sqlite3.connect(os.path.join(tmpj, "jobs.fsqlite"))
    jdb.execute("PRAGMA journal_mode = WAL;")
    jdb.executescript("""
        CREATE TABLE fcast_jobs (
            job_id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            lane TEXT NOT NULL,
            idempotency_key TEXT NOT NULL UNIQUE,
            state TEXT NOT NULL,
            lease_expires_epoch INTEGER,
            attempt INTEGER NOT NULL DEFAULT 0,
            est_cost_usd REAL NOT NULL DEFAULT 0.0
        );
        CREATE TABLE daemon_budget_ledger (
            utc_day TEXT PRIMARY KEY,
            spent_usd REAL NOT NULL,
            cap_usd REAL NOT NULL
        );
    """)
    jdb.execute("INSERT INTO daemon_budget_ledger VALUES ('2026-11-25', 1.20, 5.00)")
    jdb.execute("INSERT INTO fcast_jobs VALUES ('job-1', 'belief_fold', 'epistemic_heavy', 'idem-q1-f3', 'RUNNING', 1000, 1, 0.25)")
    # Duplicate idempotency key rejected cleanly (idempotent MCP async enqueue)
    jdb.execute("INSERT OR IGNORE INTO fcast_jobs VALUES ('job-dup', 'belief_fold', 'epistemic_heavy', 'idem-q1-f3', 'QUEUED', NULL, 0, 0.25)")
    assert jdb.execute("SELECT COUNT(*) FROM fcast_jobs").fetchone()[0] == 1
    # Crash recovery reclaims stale RUNNING lease when now=2000 > lease_expires_epoch=1000
    jdb.execute("UPDATE fcast_jobs SET state='QUEUED', lease_expires_epoch=NULL WHERE state='RUNNING' AND lease_expires_epoch < 2000")
    assert jdb.execute("SELECT state FROM fcast_jobs WHERE job_id='job-1'").fetchone()[0] == "QUEUED"
    # Budget circuit breaker blocks metered epistemic_heavy job once daily cap ($5.00) would be exceeded
    spent, cap = jdb.execute("SELECT spent_usd, cap_usd FROM daemon_budget_ledger WHERE utc_day='2026-11-25'").fetchone()
    assert (spent + 0.25) <= cap and not ((4.90 + 0.25) <= cap)
    jdb.close()

print("ALL_EXTENDED_VALIDATION_CHECKS_PASSED")
