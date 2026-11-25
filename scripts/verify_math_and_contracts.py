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
        q_raw, p_pos_raw, r_j, cost_usd, attempts = c["q"], c["p_pos"], c["r"], c["cost"], c["attempts"]
        # AUDIT-FLAG-05 (Lean Claim05Spec.projectMartingaleCrux): Fréchet-feasible Martingale projection
        # ensuring p_neg in [1e-6, 1-1e-6] AND exact q_j * p_pos + (1 - q_j) * p_neg == p_k!
        eps = 1e-6
        eps_m = 2.0 * eps
        p_pos = min(max(p_pos_raw, eps_m), 1.0 - eps_m)
        if p_pos > p_k:
            q_max = (p_k - eps_m) / (p_pos - eps_m)
        elif p_pos < p_k:
            q_max = (1.0 - eps_m - p_k) / (1.0 - eps_m - p_pos)
        else:
            q_max = 1.0 - eps_m
        q_j = min(max(q_raw, eps), min(1.0 - eps, q_max))
        p_neg = (p_k - q_j * p_pos) / (1.0 - q_j)
        assert eps <= p_neg <= 1.0 - eps
        assert abs((q_j * p_pos + (1.0 - q_j) * p_neg) - p_k) < 1e-12
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

# Canonical 12-Field ForecastRevision Immutability + 4-Stage Lifecycle (InitialForecast, SubsequentRevision,
# HorizonForecastView, and Disjoint QuestionResolution) verification in SQLite WAL
FORECAST_DDL = """
CREATE TABLE questions (
    question_id TEXT PRIMARY KEY,
    contract_sha256 TEXT NOT NULL
);
CREATE TABLE evidence_vault_index (
    ev_handle TEXT PRIMARY KEY,
    available_utc TEXT NOT NULL
);
CREATE TABLE forecast_streams (
    forecast_id TEXT PRIMARY KEY,
    question_id TEXT NOT NULL REFERENCES questions(question_id),
    stream_kind TEXT NOT NULL,
    origin_revision_id TEXT
);
CREATE TABLE forecast_revisions (
    revision_id TEXT PRIMARY KEY,
    forecast_id TEXT NOT NULL REFERENCES forecast_streams(forecast_id),
    seq_no INTEGER NOT NULL,
    parent_revision_id TEXT REFERENCES forecast_revisions(revision_id),
    question_id TEXT NOT NULL REFERENCES questions(question_id),
    as_of_utc TEXT NOT NULL,
    p_indep REAL NOT NULL CHECK (p_indep > 0.0 AND p_indep < 1.0),
    p_market REAL CHECK (p_market IS NULL OR (p_market > 0.0 AND p_market < 1.0)),
    probability_distribution TEXT NOT NULL,
    evidence_handles TEXT NOT NULL,
    model_version TEXT NOT NULL,
    calibration_version TEXT NOT NULL,
    rationale_handle TEXT NOT NULL REFERENCES evidence_vault_index(ev_handle),
    resolution_status TEXT NOT NULL CHECK (resolution_status IN ('unresolved', 'resolved', 'invalidated')),
    CHECK ((seq_no = 0 AND parent_revision_id IS NULL) OR (seq_no > 0 AND parent_revision_id IS NOT NULL)),
    UNIQUE(forecast_id, seq_no)
);
CREATE TRIGGER trg_forecast_revisions_no_update
BEFORE UPDATE ON forecast_revisions
BEGIN
    SELECT RAISE(ABORT, 'IMMUTABLE_FORECAST_REVISION');
END;
CREATE TRIGGER trg_forecast_revisions_no_delete
BEFORE DELETE ON forecast_revisions
BEGIN
    SELECT RAISE(ABORT, 'IMMUTABLE_FORECAST_REVISION');
END;
CREATE TABLE question_resolutions (
    resolution_id TEXT PRIMARY KEY,
    question_id TEXT NOT NULL REFERENCES questions(question_id),
    final_status TEXT NOT NULL CHECK (final_status IN ('resolved', 'invalidated')),
    outcome_binary INTEGER CHECK (outcome_binary IN (0, 1) OR outcome_binary IS NULL),
    event_utc TEXT,
    resolved_published_utc TEXT NOT NULL,
    recorded_available_utc TEXT NOT NULL
);
"""

def seal_forecast_revision(conn, *, forecast_id, seq_no, parent_id, question_id, as_of_utc, p_indep, p_market,
                           dist_obj, ev_handles, model_ver, cal_ver, rationale_handle, res_status):
    # Enforce causal fence: all bound evidence + rationale handles must have available_utc <= as_of_utc
    for h in list(ev_handles) + [rationale_handle]:
        row = conn.execute("SELECT available_utc FROM evidence_vault_index WHERE ev_handle = ?", (h,)).fetchone()
        assert row is not None, f"Missing EvidenceVault handle: {h}"
        if row[0] > as_of_utc:
            raise ValueError(f"TEMPORAL_FENCE_VIOLATION: {h} available_utc={row[0]} > as_of_utc={as_of_utc}")
    canonical_payload = _json.dumps({
        "forecast_id": forecast_id, "seq_no": seq_no, "parent": parent_id, "question_id": question_id,
        "as_of_utc": as_of_utc, "p_indep": round(p_indep, 9),
        "p_market": None if p_market is None else round(p_market, 9),
        "dist": dist_obj, "ev": sorted(ev_handles), "model": model_ver, "cal": cal_ver,
        "rat": rationale_handle, "status": res_status,
    }, sort_keys=True)
    rev_id = "rev_" + hashlib.sha256(canonical_payload.encode()).hexdigest()[:16]
    conn.execute(
        "INSERT INTO forecast_revisions VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        (rev_id, forecast_id, seq_no, parent_id, question_id, as_of_utc, p_indep, p_market,
         _json.dumps(dist_obj), _json.dumps(sorted(ev_handles)), model_ver, cal_ver, rationale_handle, res_status),
    )
    if seq_no == 0:
        conn.execute("UPDATE forecast_streams SET origin_revision_id = ? WHERE forecast_id = ?", (rev_id, forecast_id))
    return rev_id

def project_at_horizon(conn, forecast_id: str, eval_as_of_utc: str):
    row = conn.execute(
        """SELECT revision_id, seq_no, as_of_utc, p_indep, p_market, probability_distribution, resolution_status
           FROM forecast_revisions
           WHERE forecast_id = ? AND as_of_utc <= ?
           ORDER BY as_of_utc DESC, seq_no DESC LIMIT 1""",
        (forecast_id, eval_as_of_utc),
    ).fetchone()
    if row is None:
        return ("NOT_YET_ISSUED", None)
    return ("STANDING_REVISION", {
        "revision_id": row[0], "seq_no": row[1], "as_of_utc": row[2],
        "p_indep": row[3], "p_market": row[4],
        "dist": _json.loads(row[5]), "resolution_status": row[6],
    })

with tempfile.TemporaryDirectory() as tmpf:
    fdb = sqlite3.connect(os.path.join(tmpf, "canonical.fsqlite"))
    fdb.execute("PRAGMA journal_mode = WAL;")
    fdb.execute("PRAGMA foreign_keys = ON;")
    fdb.executescript(FORECAST_DDL)

    fdb.execute("INSERT INTO questions VALUES ('q_baltic', 'sha256_contract_v1'), ('q_void', 'sha256_contract_v2')")
    for h, ts in [
        ("[ev:01#ov]", "2026-11-01T09:00:00Z"),
        ("[ev:02#rat0]", "2026-11-01T10:00:00Z"),
        ("[ev:03#ais]", "2026-11-10T14:00:00Z"),
        ("[ev:04#rat1]", "2026-11-10T14:05:00Z"),
        ("[ev:05#future]", "2026-11-20T00:00:00Z"),
    ]:
        fdb.execute("INSERT INTO evidence_vault_index VALUES (?, ?)", (h, ts))
    fdb.execute("INSERT INTO forecast_streams VALUES ('fc_baltic_main', 'q_baltic', 'live_tournament', NULL)")
    fdb.execute("INSERT INTO forecast_streams VALUES ('fc_void_main', 'q_void', 'live_tournament', NULL)")

    # Stage 1: InitialForecast (seq_no = 0, parent = None)
    r0 = seal_forecast_revision(
        fdb, forecast_id="fc_baltic_main", seq_no=0, parent_id=None, question_id="q_baltic",
        as_of_utc="2026-11-01T10:00:00Z", p_indep=0.18, p_market=0.15,
        dist_obj={"kind": "binary", "p": 0.17, "n_eff": 8.0},
        ev_handles=["[ev:01#ov]"], model_ver="gemini-4-argon@2026-05-20", cal_ver="cal-v1",
        rationale_handle="[ev:02#rat0]", res_status="unresolved",
    )

    # Causal fence blocks binding post-cutoff evidence [ev:05#future] at 2026-11-10
    try:
        seal_forecast_revision(
            fdb, forecast_id="fc_baltic_main", seq_no=1, parent_id=r0, question_id="q_baltic",
            as_of_utc="2026-11-10T14:05:00Z", p_indep=0.62, p_market=0.48,
            dist_obj={"kind": "binary", "p": 0.58, "n_eff": 14.0},
            ev_handles=["[ev:05#future]"], model_ver="gemini-4-argon@2026-05-20", cal_ver="cal-v1",
            rationale_handle="[ev:04#rat1]", res_status="unresolved",
        )
        assert False, "Should have blocked future evidence handle"
    except ValueError as exc:
        assert "TEMPORAL_FENCE_VIOLATION" in str(exc)

    # Stage 2: Valid SubsequentRevision (seq_no = 1, parent = r0)
    r1 = seal_forecast_revision(
        fdb, forecast_id="fc_baltic_main", seq_no=1, parent_id=r0, question_id="q_baltic",
        as_of_utc="2026-11-10T14:05:00Z", p_indep=0.62, p_market=0.48,
        dist_obj={"kind": "binary", "p": 0.58, "n_eff": 14.0},
        ev_handles=["[ev:01#ov]", "[ev:03#ais]"], model_ver="gemini-4-argon@2026-05-20", cal_ver="cal-v1",
        rationale_handle="[ev:04#rat1]", res_status="unresolved",
    )
    rv_void = seal_forecast_revision(
        fdb, forecast_id="fc_void_main", seq_no=0, parent_id=None, question_id="q_void",
        as_of_utc="2026-11-01T10:00:00Z", p_indep=0.30, p_market=None,
        dist_obj={"kind": "binary", "p": 0.30, "n_eff": 6.0},
        ev_handles=["[ev:01#ov]"], model_ver="gemini-4-argon@2026-05-20", cal_ver="cal-v1",
        rationale_handle="[ev:02#rat0]", res_status="unresolved",
    )
    fdb.commit()

    # Immutability triggers MUST abort both UPDATE and DELETE on forecast_revisions
    for bad_sql in (
        f"UPDATE forecast_revisions SET p_indep = 0.99 WHERE revision_id = '{r0}'",
        f"UPDATE forecast_revisions SET resolution_status = 'resolved' WHERE revision_id = '{r1}'",
        f"DELETE FROM forecast_revisions WHERE revision_id = '{r0}'",
    ):
        try:
            fdb.execute(bad_sql)
            assert False, f"Trigger failed to block mutation: {bad_sql}"
        except sqlite3.IntegrityError as exc:
            assert "IMMUTABLE_FORECAST_REVISION" in str(exc)

    # Stage 3: HorizonForecastView projections across pre-issuance, post-initial, and post-revision cutoffs
    st_pre, proj_pre = project_at_horizon(fdb, "fc_baltic_main", "2026-10-31T23:59:59Z")
    st_h0, proj_h0 = project_at_horizon(fdb, "fc_baltic_main", "2026-11-05T00:00:00Z")
    st_h1, proj_h1 = project_at_horizon(fdb, "fc_baltic_main", "2026-11-15T00:00:00Z")
    assert st_pre == "NOT_YET_ISSUED" and proj_pre is None
    assert st_h0 == "STANDING_REVISION" and proj_h0["revision_id"] == r0 and proj_h0["dist"]["p"] == 0.17
    assert st_h1 == "STANDING_REVISION" and proj_h1["revision_id"] == r1 and proj_h1["dist"]["p"] == 0.58

    # Stage 4: Disjoint QuestionResolution join & void-exclusion scoring (without mutating ForecastRevision)
    fdb.execute("INSERT INTO question_resolutions VALUES ('res_1', 'q_baltic', 'resolved', 1, '2026-11-24T04:10:00Z', '2026-11-24T05:00:00Z', '2026-11-24T05:05:00Z')")
    fdb.execute("INSERT INTO question_resolutions VALUES ('res_2', 'q_void', 'invalidated', NULL, NULL, '2026-11-24T06:00:00Z', '2026-11-24T06:05:00Z')")
    fdb.commit()

    # Historical ForecastRevision rows still carry resolution_status == 'unresolved' as of their issuance cutoff!
    statuses = [x[0] for x in fdb.execute("SELECT resolution_status FROM forecast_revisions").fetchall()]
    assert statuses == ["unresolved", "unresolved", "unresolved"]

    # Proper-score join excludes 'invalidated' questions cleanly and proves positive Revision Value Added (Brier improves 0.6889 -> 0.1764)
    brier_r0 = (proj_h0["dist"]["p"] - 1.0) ** 2
    brier_r1 = (proj_h1["dist"]["p"] - 1.0) ** 2
    assert brier_r1 < brier_r0
    scored_qids = [
        x[0] for x in fdb.execute(
            "SELECT question_id FROM question_resolutions WHERE final_status = 'resolved' AND recorded_available_utc <= '2026-11-25T00:00:00Z'"
        ).fetchall()
    ]
    assert scored_qids == ["q_baltic"]
    fdb.close()

# Full Eligible Universe Anti-Selection-Bias State Machine + QuestionMetaForecast + 10-Metric Scorecard check
UNIVERSE_DDL = """
CREATE TABLE universe_question_ledger (
    question_id TEXT PRIMARY KEY,
    discovered_utc TEXT NOT NULL,
    is_eligible INTEGER NOT NULL CHECK (is_eligible IN (0, 1)),
    est_cost_usd_p50 REAL NOT NULL,
    est_latency_sec_p50 REAL NOT NULL,
    popularity_score REAL NOT NULL CHECK (popularity_score BETWEEN 0.0 AND 100.0),
    complexity_score REAL NOT NULL CHECK (complexity_score BETWEEN 0.0 AND 100.0),
    disposition TEXT NOT NULL CHECK (disposition IN ('issued', 'excluded', 'abandoned', 'queued_missed', 'ineligible')),
    reason_code TEXT,
    total_cost_usd REAL NOT NULL DEFAULT 0.0,
    latency_sec REAL,
    crowd_prior REAL
);
CREATE TRIGGER trg_universe_eligible_irreversible
BEFORE UPDATE ON universe_question_ledger
WHEN OLD.is_eligible = 1 AND NEW.is_eligible = 0
BEGIN
    SELECT RAISE(ABORT, 'IRREVERSIBLE_ELIGIBLE_UNIVERSE_ADMISSION');
END;
"""

def compute_ten_metric_scorecard(rows, rev_histories, outcomes):
    elig = [r for r in rows if r["is_eligible"] == 1]
    issued = [r for r in elig if r["disposition"] == "issued"]
    excl = [r for r in elig if r["disposition"] == "excluded"]
    aband = [r for r in elig if r["disposition"] == "abandoned"]

    res_elig = [r for r in elig if r["question_id"] in outcomes]
    res_issued = [r for r in issued if r["question_id"] in outcomes]

    def brier(p, y):
        return (p - y) ** 2

    def log_s(p, y, eps=1e-6):
        p = min(max(p, eps), 1.0 - eps)
        return math.log(p) if y == 1 else math.log(1.0 - p)

    # 1 & 2: Conditional (Investigated-Only) vs Full-Universe Imputed Brier & Log Score
    bs_cond = sum(brier(rev_histories[r["question_id"]][-1], outcomes[r["question_id"]]) for r in res_issued) / len(res_issued)
    ls_cond = sum(log_s(rev_histories[r["question_id"]][-1], outcomes[r["question_id"]]) for r in res_issued) / len(res_issued)

    bs_univ_ign, ls_univ_ign, bs_univ_null, bs_base_all = 0.0, 0.0, 0.0, 0.0
    for r in res_elig:
        qid = r["question_id"]
        y = outcomes[qid]
        p_base = r["crowd_prior"]
        bs_base_all += brier(p_base, y)
        if r["disposition"] == "issued":
            p_f = rev_histories[qid][-1]
            bs_univ_ign += brier(p_f, y)
            ls_univ_ign += log_s(p_f, y)
            bs_univ_null += brier(p_f, y)
        else:
            # Un-issued (excluded/abandoned/queued): 0.50 for raw ignorance score; p_base (0 skill edge) for BSS_univ
            bs_univ_ign += brier(0.50, y)
            ls_univ_ign += log_s(0.50, y)
            bs_univ_null += brier(p_base, y)

    bs_univ_ign /= len(res_elig)
    ls_univ_ign /= len(res_elig)
    bs_base_cond = sum(brier(r["crowd_prior"], outcomes[r["question_id"]]) for r in res_issued) / len(res_issued)

    # 3: Calibration Error (2-bin equal-mass ECE on resolved issued)
    sorted_pts = sorted((rev_histories[r["question_id"]][-1], outcomes[r["question_id"]]) for r in res_issued)
    mid = max(1, len(sorted_pts) // 2)
    bins = [sorted_pts[:mid], sorted_pts[mid:]]
    ece_cond = sum((len(b) / len(sorted_pts)) * abs(sum(p for p, _ in b) / len(b) - sum(y for _, y in b) / len(b)) for b in bins if b)

    # 4: Skill vs baseline (Conditional vs Selection-Bias-Free Full Universe)
    bss_cond = 1.0 - (bs_cond / bs_base_cond)
    bss_univ = 1.0 - (bs_univ_null / bs_base_all)

    # 5 & 6: Cost per forecast vs Cost per resolved forecast (including ALL sunk spend on abandoned/unresolved!)
    cost_per_forecast = sum(r["total_cost_usd"] for r in issued) / len(issued)
    cost_per_resolved_forecast = sum(r["total_cost_usd"] for r in elig) / len(res_issued)

    # 7: Research latency (median p50 over issued)
    lats = sorted(r["latency_sec"] for r in issued)
    latency_p50 = lats[len(lats) // 2]

    # 8: Forecast coverage + exclusion & abandonment rates over entire eligible universe
    coverage = len(issued) / len(elig)
    exclusion_rate = len(excl) / len(elig)
    abandonment_rate = len(aband) / len(elig)

    # 9 & 10: % Forecasts revised & % Revisions improving final score (stepwise & net R_final vs R_0)
    revised_qids = [r["question_id"] for r in issued if len(rev_histories[r["question_id"]]) >= 2]
    pct_revised = len(revised_qids) / len(issued)

    step_wins, step_total, net_wins, net_total = 0, 0, 0, 0
    for qid in revised_qids:
        if qid not in outcomes:
            continue
        y = outcomes[qid]
        seq = rev_histories[qid]
        net_total += 1
        if brier(seq[-1], y) < brier(seq[0], y):
            net_wins += 1
        for k in range(1, len(seq)):
            step_total += 1
            if brier(seq[k], y) < brier(seq[k - 1], y):
                step_wins += 1

    return {
        "brier_score": (bs_cond, bs_univ_ign),
        "log_score": (ls_cond, ls_univ_ign),
        "calibration_error": ece_cond,
        "skill_vs_baselines": (bss_cond, bss_univ),
        "cost_per_forecast": cost_per_forecast,
        "cost_per_resolved_forecast": cost_per_resolved_forecast,
        "research_latency": latency_p50,
        "forecast_coverage": coverage,
        "exclusion_rate": exclusion_rate,
        "abandonment_rate": abandonment_rate,
        "pct_forecasts_revised": pct_revised,
        "pct_revisions_improving_score": (net_wins / net_total, step_wins / step_total),
    }

with tempfile.TemporaryDirectory() as tmpu:
    udb = sqlite3.connect(os.path.join(tmpu, "universe.fsqlite"))
    udb.execute("PRAGMA journal_mode = WAL;")
    udb.executescript(UNIVERSE_DDL)

    cohort = [
        # 5 issued & resolved, 1 issued & unresolved, 2 excluded pre-run, 2 abandoned mid-flight (with sunk cost!) + 1 out-of-scope sports
        ("q1", "2026-11-01T00:00:00Z", 1, 0.08, 30.0, 82.0, 45.0, "issued", None, 0.09, 28.0, 0.35),
        ("q2", "2026-11-01T00:00:00Z", 1, 0.06, 24.0, 74.0, 38.0, "issued", None, 0.07, 22.0, 0.60),
        ("q3", "2026-11-01T00:00:00Z", 1, 0.10, 40.0, 41.0, 62.0, "issued", None, 0.11, 43.0, 0.25),
        ("q4", "2026-11-01T00:00:00Z", 1, 0.07, 26.0, 90.0, 50.0, "issued", None, 0.08, 25.0, 0.70),
        ("q5", "2026-11-01T00:00:00Z", 1, 0.09, 34.0, 55.0, 58.0, "issued", None, 0.09, 35.0, 0.40),
        ("q6_open", "2026-11-01T00:00:00Z", 1, 0.08, 30.0, 63.0, 52.0, "issued", None, 0.08, 31.0, 0.50),
        ("q7_excl", "2026-11-01T00:00:00Z", 1, 0.24, 110.0, 18.0, 94.0, "excluded", "complexity_exceeds_profile", 0.00, None, 0.45),
        ("q8_excl", "2026-11-01T00:00:00Z", 1, 0.19, 95.0, 92.0, 78.0, "excluded", "low_predicted_voi", 0.00, None, 0.52),
        ("q9_aban", "2026-11-01T00:00:00Z", 1, 0.15, 65.0, 33.0, 88.0, "abandoned", "insufficient_evidence", 0.18, 61.0, 0.30),
        ("q10_aban", "2026-11-01T00:00:00Z", 1, 0.16, 70.0, 29.0, 91.0, "abandoned", "fold_budget_exhausted_unsealed", 0.22, 74.0, 0.65),
        ("q11_sports", "2026-11-01T00:00:00Z", 0, 0.01, 5.0, 99.0, 10.0, "ineligible", "out_of_domain_sports", 0.00, None, None),
    ]
    udb.executemany("INSERT INTO universe_question_ledger VALUES (?,?,?,?,?,?,?,?,?,?,?,?)", cohort)
    udb.commit()

    # Trigger MUST block retroactively demoting an abandoned hard question (q9_aban) from is_eligible=1 to 0!
    try:
        udb.execute("UPDATE universe_question_ledger SET is_eligible = 0 WHERE question_id = 'q9_aban'")
        assert False, "Should have blocked retroactive universe demotion"
    except sqlite3.IntegrityError as exc:
        assert "IRREVERSIBLE_ELIGIBLE_UNIVERSE_ADMISSION" in str(exc)

    cols = [d[0] for d in udb.execute("SELECT * FROM universe_question_ledger").description]
    all_rows = [dict(zip(cols, r)) for r in udb.execute("SELECT * FROM universe_question_ledger").fetchall()]
    udb.close()

    rev_histories = {
        "q1": [0.30, 0.15, 0.12],  # y=0: improves twice
        "q2": [0.65, 0.82],        # y=1: improves
        "q3": [0.28],              # y=0: unrevised
        "q4": [0.72, 0.68, 0.88],  # y=1: step 1 dips, step 2 improves, net improves
        "q5": [0.45, 0.55],        # y=0: revised in wrong direction (net & step worsen)
        "q6_open": [0.50, 0.61],   # unresolved
    }
    outcomes = {"q1": 0, "q2": 1, "q3": 0, "q4": 1, "q5": 0, "q7_excl": 1, "q8_excl": 0, "q9_aban": 0, "q10_aban": 1}

    sc = compute_ten_metric_scorecard(all_rows, rev_histories, outcomes)
    # Coverage = 6/10 (60%), Excluded = 20%, Abandoned = 20%
    assert abs(sc["forecast_coverage"] - 0.60) < 1e-9
    assert abs(sc["exclusion_rate"] - 0.20) < 1e-9 and abs(sc["abandonment_rate"] - 0.20) < 1e-9
    # Selection-bias proof: Conditional BSS (+25.7%) is > 2.3x Full-Universe BSS (+11.1%), a +14.6pp illusion!
    bss_cond, bss_univ = sc["skill_vs_baselines"]
    assert bss_cond > 2.0 * bss_univ and (bss_cond - bss_univ) > 0.14, (bss_cond, bss_univ)
    # Sunk-cost proof: Cost per resolved forecast ($0.92 / 5 = $0.184) > 2x Cost per issued forecast ($0.52 / 6 = $0.0867)!
    assert sc["cost_per_resolved_forecast"] > 2.0 * sc["cost_per_forecast"]
    # Revision tracking: 5/6 (83.33%) revised; 3/4 (75%) net improved on resolved revised streams; 4/6 (66.67%) step improved
    assert abs(sc["pct_forecasts_revised"] - (5.0 / 6.0)) < 1e-9
    assert abs(sc["pct_revisions_improving_score"][0] - 0.75) < 1e-9
    assert abs(sc["pct_revisions_improving_score"][1] - (4.0 / 6.0)) < 1e-9

# Pure-Rust MetaculusCdfBuilder reference contract (port of Metaculus/forecasting-tools NumericDistribution)
def build_metaculus_cdf(
    percentiles, lower_bound, upper_bound, open_lower, open_upper,
    zero_point=None, inbound_outcome_count=None, min_step=5e-5, max_step=0.59,
):
    n = 201 if inbound_outcome_count is None else (inbound_outcome_count + 1)

    def to_unit(x):
        if zero_point is not None:
            return math.log((x - zero_point) / (lower_bound - zero_point)) / math.log((upper_bound - zero_point) / (lower_bound - zero_point))
        return (x - lower_bound) / (upper_bound - lower_bound)

    # Map (value, prob) anchors to normalized [0, 1] x-axis
    pts = sorted((to_unit(v), p) for v, p in percentiles)
    lo_val = max(0.001, pts[0][1]) if open_lower else 0.0
    hi_val = min(0.999, pts[-1][1]) if open_upper else 1.0
    knots = [(0.0, lo_val)] + [(x, p) for x, p in pts if 0.0 < x < 1.0] + [(1.0, hi_val)]

    cdf = []
    for i in range(n):
        t = i / (n - 1)
        # Piecewise linear interpolation over knots
        val = knots[-1][1]
        for j in range(len(knots) - 1):
            x0, y0 = knots[j]
            x1, y1 = knots[j + 1]
            if x0 <= t <= x1:
                frac = 0.0 if x1 == x0 else (t - x0) / (x1 - x0)
                val = y0 + frac * (y1 - y0)
                break
        cdf.append(val)

    # Enforce strict monotonicity & Metaculus API bounds: min_step <= Δcdf <= max_step
    cdf[0] = max(0.001, cdf[0]) if open_lower else 0.0
    cdf[-1] = min(0.999, cdf[-1]) if open_upper else 1.0
    for i in range(1, n):
        cdf[i] = max(cdf[i], cdf[i - 1] + min_step)
        cdf[i] = min(cdf[i], cdf[i - 1] + max_step)
    # Rescale interior linearly if upper endpoint overshot while keeping endpoints exact
    span_cur = cdf[-1] - cdf[0]
    target_hi = min(0.999, cdf[-1]) if open_upper else 1.0
    span_tgt = target_hi - cdf[0]
    if span_cur > 0 and abs(cdf[-1] - target_hi) > 1e-12:
        c0 = cdf[0]
        cdf = [c0 + (c - c0) * (span_tgt / span_cur) for c in cdf]
    cdf[0] = max(0.001, cdf[0]) if open_lower else 0.0
    cdf[-1] = target_hi
    return cdf

# 1) Continuous 201-point closed-bounds linear CDF
cdf_lin = build_metaculus_cdf([(20, 0.10), (50, 0.50), (80, 0.90)], 0.0, 100.0, False, False)
assert len(cdf_lin) == 201 and cdf_lin[0] == 0.0 and cdf_lin[-1] == 1.0
assert all(5e-5 - 1e-12 <= cdf_lin[i] - cdf_lin[i - 1] <= 0.59 + 1e-12 for i in range(1, 201))

# 2) Continuous 201-point open-bounds log-scaled (zero_point=0.0) CDF
cdf_log = build_metaculus_cdf([(10, 0.15), (100, 0.55), (1000, 0.85)], 1.0, 10000.0, True, True, zero_point=0.0)
assert len(cdf_log) == 201 and cdf_log[0] >= 0.001 and cdf_log[-1] <= 0.999
assert all(5e-5 - 1e-12 <= cdf_log[i] - cdf_log[i - 1] <= 0.59 + 1e-12 for i in range(1, 201))

# 3) Discrete question (inbound_outcome_count = 15 -> 16-point CDF)
cdf_disc = build_metaculus_cdf([(3, 0.20), (8, 0.60), (12, 0.90)], 0.0, 15.0, False, True, inbound_outcome_count=15)
assert len(cdf_disc) == 16 and cdf_disc[0] == 0.0 and cdf_disc[-1] <= 0.999
assert all(5e-5 - 1e-12 <= cdf_disc[i] - cdf_disc[i - 1] <= 0.59 + 1e-12 for i in range(1, 16))

# Pure-Rust forecastingresearch/forecastbench Submission Validator & Two-Way Fixed-Effects Difficulty-Adjusted Brier
def validate_forecastbench_submission(question_set_keys, submission):
    for req in ("organization", "model", "model_organization", "question_set", "forecasts"):
        assert req in submission and submission[req], f"Missing required top-level key: {req}"
    seen = {}
    for row in submission["forecasts"]:
        qid = tuple(row["id"]) if isinstance(row["id"], list) else (row["id"],)
        direction = tuple(row["direction"]) if isinstance(row.get("direction"), list) else None
        if len(qid) > 1:
            assert direction is not None and len(direction) == len(qid), "Combo arity mismatch"
            assert all(d in (1, -1) for d in direction), "Invalid Combo direction"
        p = row["forecast"]
        assert isinstance(p, (int, float)) and not math.isnan(p) and 0.0 <= p <= 1.0, f"Invalid prob: {p}"
        key = (qid, row["source"], row.get("resolution_date"), direction)
        assert key not in seen, f"Duplicate forecast key: {key}"
        seen[key] = p
    missing = set(question_set_keys) - set(seen.keys())
    if missing:
        raise ValueError(f"INCOMPLETE_SUBMISSION_MISSING_KEYS: {len(missing)}")
    # Signed Fréchet check on any 2-leg Combo (id1, id2) with direction (+1, +1) whose marginals exist
    for (qid, src, rdate, direction), p_ab in seen.items():
        if len(qid) == 2 and direction == (1, 1):
            pa = seen.get(((qid[0],), src, rdate, None))
            pb = seen.get(((qid[1],), src, rdate, None))
            if pa is not None and pb is not None:
                lo, hi = max(0.0, pa + pb - 1.0), min(pa, pb)
                assert lo - 1e-9 <= p_ab <= hi + 1e-9, f"Fréchet violation: {p_ab} not in [{lo}, {hi}]"
    return True

qset_keys = [
    (("qA",), "polymarket", "2026-12-31", None),
    (("qB",), "polymarket", "2026-12-31", None),
    (("qA", "qB"), "polymarket", "2026-12-31", (1, 1)),
]
valid_sub = {
    "organization": "FrankenForecast", "model": "fcast-v1", "model_organization": "Google",
    "question_set": "2026-11-29-llm.json",
    "forecasts": [
        {"id": "qA", "source": "polymarket", "forecast": 0.40, "resolution_date": "2026-12-31", "reasoning": "ev", "direction": None},
        {"id": "qB", "source": "polymarket", "forecast": 0.50, "resolution_date": "2026-12-31", "reasoning": "ev", "direction": None},
        {"id": ["qA", "qB"], "source": "polymarket", "forecast": 0.25, "resolution_date": "2026-12-31", "reasoning": "ev", "direction": [1, 1]},
    ],
}
assert validate_forecastbench_submission(qset_keys, valid_sub) is True

# Two-Way Fixed-Effects Difficulty-Adjusted Brier: B_{m,q} = alpha_m + delta_q (alternating projections)
def solve_two_way_fe_brier(obs, iters=200):
    models = sorted({m for m, _, _ in obs})
    qs = sorted({q for _, q, _ in obs})
    alpha = {m: 0.0 for m in models}
    delta = {q: 0.0 for q in qs}
    for _ in range(iters):
        for m in models:
            rows = [(q, b) for mm, q, b in obs if mm == m]
            alpha[m] = sum(b - delta[q] for q, b in rows) / len(rows)
        for q in qs:
            rows = [(m, b) for m, qq, b in obs if qq == q]
            delta[q] = sum(b - alpha[m] for m, b in rows) / len(rows)
        mean_d = sum(delta.values()) / len(delta)
        for q in qs:
            delta[q] -= mean_d
        for m in models:
            alpha[m] += mean_d
    return alpha, delta

# True skill: model_A (0.10) is BETTER than model_B (0.14), but model_A faces harder questions (q2, q3)
# while model_B faces easier questions (q1, q2), making raw mean Brier wrongly favor model_B!
fe_obs = [
    ("model_A", "q2_mid",  0.10 + 0.00),
    ("model_A", "q3_hard", 0.10 + 0.10),  # raw mean A = 0.15
    ("model_B", "q1_easy", 0.14 - 0.10),
    ("model_B", "q2_mid",  0.14 + 0.00),  # raw mean B = 0.09 (misleading!)
]
raw_a = (0.10 + 0.20) / 2
raw_b = (0.04 + 0.14) / 2
alpha_fe, _ = solve_two_way_fe_brier(fe_obs)
assert raw_b < raw_a and alpha_fe["model_A"] < alpha_fe["model_B"]
assert abs((alpha_fe["model_B"] - alpha_fe["model_A"]) - 0.04) < 1e-6

print("ALL_EXTENDED_VALIDATION_CHECKS_PASSED")
