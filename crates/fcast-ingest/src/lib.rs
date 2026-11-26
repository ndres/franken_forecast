//! `fcast-ingest` (DAG Layer L2 — `SUB-02` Harvester).
//!
//! Hosts market orderbook microstructure (`BookMarkPrice`), `Metaculus` / `Mantic` /
//! `ForecastBench` connectors, deterministic Geo/Macro/Cyber `DomainGate` + cross-venue
//! `EquivalenceCluster` alignment, and `Tier W1` Zstd-compressed partitioned L2 tick archives.

pub mod domain_gate;
pub mod metaculus;
pub mod microstructure;
pub mod tick_archive;

pub use domain_gate::{
    BenchmarkMetrics, ClusterMatcher, DomainCandidate, DomainDecision, DomainGate,
    EquivalenceCluster, MarketContractRef, RejectReason, TargetDomain,
};
pub use metaculus::{
    ApiFilter, BinaryQuestion, BotLeaderboardStatus, CANONICAL_PERCENTILES_13, ConditionalQuestion,
    CrowdPriorSource, DEFAULT_MC_SIMPLEX_FLOOR, DateQuestion, DiscreteQuestion,
    ForecastBenchMedianBackfill, ForecastBenchMedianRow, GroupOfQuestions, MANTIC_MAX_BINS,
    MANTIC_OPEN_TAIL_FLOOR, METACULUS_CDF_MAX_STEP, METACULUS_CDF_MIN_STEP, ManticCrucibleAdapter,
    MetaculusError, MetaculusQuestion, MultipleChoiceQuestion, NumericQuestion, QuestionMeta,
    ResolvedCrowdPrior, aggregate_mantic_pmfs_pointwise, enforce_mantic_open_tail_floor,
    fixed_point_simplex_floor, restore_option_names,
};
pub use microstructure::{
    BookMarkPrice, BookState, DEFAULT_TICK_BPS, EXCESS_SPREAD_SCALE_BPS, MAX_SPREAD_BPS,
    MIN_TOB_LIQUIDITY_USD, MicrostructureError, OrderbookSnapshot, REF_TOB_LIQUIDITY_USD,
    SWEEP_IMPACT_SCALE,
};
pub use tick_archive::{
    DEFAULT_ZSTD_LEVEL, L2TickRecord, PartitionKey, TickArchive, TickArchiveError,
};
