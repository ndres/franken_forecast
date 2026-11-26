# FrankenForecast (fcast) Developer Task Runner
set shell := ["bash", "-euo", "pipefail", "-c"]

export PATH := "/opt/homebrew/opt/rustup/bin:" + env_var("PATH")

default:
    @just --list

# Run all 4 Python specification, math, Lean-audit & refactoring verifiers
spec:
    python3 scripts/verify_plan_hygiene.py
    python3 scripts/verify_math_and_contracts.py
    python3 lean_experiments/verify_audit_flags.py
    python3 docs/proposals/verify_refactoring_math.py

# Check formatting and strict Clippy (-D warnings) across all 6 crates
lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings

# Run all Rust unit, 10k-case proptest, golden-parity & gate integration tests
test:
    cargo test --workspace

# Regenerate cross-language Python -> Rust golden math vectors
golden:
    python3 scripts/export_golden_vectors.py

# Emit per-gate offline receipts under receipts/
receipts:
    cargo run -p fcast-app -- verify-receipts --write-dir receipts

# Full pre-commit verification gauntlet
verify: spec lint test receipts
