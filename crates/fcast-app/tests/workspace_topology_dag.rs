//! `GATE-01` (CLAIM-01): Full `cargo metadata` DAG rank check + `cargo tree` compiled transitive banned-crate audit.

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

#[test]
fn gate_01_workspace_dag_and_transitive_banned_crates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");

    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());

    // 1. Verify strict 6-crate acyclic DAG via offline `cargo metadata --no-deps`
    let output = Command::new(&cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .current_dir(root)
        .output()
        .expect("run cargo metadata");
    assert!(
        output.status.success(),
        "cargo metadata failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let meta: Value = serde_json::from_slice(&output.stdout).expect("parse cargo metadata");

    let ranks: BTreeMap<&str, usize> = BTreeMap::from([
        ("fcast-core", 0),
        ("fcast-infra", 1),
        ("fcast-ingest", 2),
        ("fcast-epistemic", 3),
        ("fcast-bench", 4),
        ("fcast-app", 5),
    ]);
    let packages = meta["packages"].as_array().expect("packages");
    assert_eq!(packages.len(), 6, "Expected exactly 6 workspace crates");
    for pkg in packages {
        let name = pkg["name"].as_str().expect("name");
        let &my_rank = ranks.get(name).expect("known workspace crate");
        for dep in pkg["dependencies"].as_array().expect("deps") {
            let dep_name = dep["name"].as_str().expect("dep name");
            if let Some(&dep_rank) = ranks.get(dep_name) {
                assert!(
                    dep_rank < my_rank,
                    "DAG rank violation: {name} (L{my_rank}) depends on {dep_name} (L{dep_rank})"
                );
            }
        }
    }

    // 2. Verify zero banned crates in the actual compiled transitive dependency graph (`cargo tree`)
    let banned = [
        "tokio",
        "reqwest",
        "hyper",
        "ort",
        "openssl-sys",
        "rusqlite",
        "sqlx",
        "petgraph",
    ];
    let tree_out = Command::new(&cargo)
        .args([
            "tree",
            "--workspace",
            "--edges",
            "normal",
            "--offline",
            "--prefix",
            "none",
            "--format",
            "{p}",
        ])
        .current_dir(root)
        .output()
        .expect("run cargo tree");
    assert!(
        tree_out.status.success(),
        "cargo tree failed: {}",
        String::from_utf8_lossy(&tree_out.stderr)
    );
    let tree_text = String::from_utf8_lossy(&tree_out.stdout);
    assert!(
        tree_text.contains("fsqlite "),
        "Expected upstream fsqlite in compiled dependency graph"
    );
    for line in tree_text.lines() {
        if let Some(pkg_name) = line.split_whitespace().next() {
            assert!(
                !banned.contains(&pkg_name),
                "Banned crate `{pkg_name}` found in compiled transitive dependency graph!"
            );
        }
    }
}
