//! `GATE-24` (CLAIM-08): Zero-key `fcast init` + offline `ForecastBench` seed + `verify-receipts`.

use std::process::Command;
use tempfile::TempDir;

#[test]
fn gate_24_zero_key_quickstart_and_verify_receipts() {
    let dir = TempDir::new().expect("tempdir");
    let bin = env!("CARGO_BIN_EXE_fcast");

    let init_out = Command::new(bin)
        .args(["init", "--data-root"])
        .arg(dir.path())
        .output()
        .expect("run fcast init");
    assert!(
        init_out.status.success(),
        "fcast init failed: {}",
        String::from_utf8_lossy(&init_out.stderr)
    );
    let init_json: serde_json::Value =
        serde_json::from_slice(&init_out.stdout).expect("valid init json");
    assert_eq!(init_json["status"], "ok");
    assert!(init_json["zero_key_seed_revision_id"]
        .as_str()
        .unwrap_or_default()
        .starts_with("rev_"));

    let repo_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(std::path::Path::parent)
        .expect("workspace root");
    let verify_out = Command::new(bin)
        .arg("verify-receipts")
        .current_dir(repo_root)
        .output()
        .expect("run fcast verify-receipts");
    assert!(verify_out.status.success());
    let rec_json: serde_json::Value =
        serde_json::from_slice(&verify_out.stdout).expect("valid receipt json");
    assert_eq!(rec_json["status"], "VERIFIED");
    assert_eq!(rec_json["templates_verified"], 9);
}
