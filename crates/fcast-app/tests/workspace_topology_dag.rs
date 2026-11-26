//! `GATE-01` (CLAIM-01): 6-crate acyclic DAG & zero banned runtime/FFI crates.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[test]
fn gate_01_workspace_dag_and_banned_crates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");

    let ranks: BTreeMap<&str, usize> = BTreeMap::from([
        ("fcast-core", 0),
        ("fcast-infra", 1),
        ("fcast-ingest", 2),
        ("fcast-epistemic", 3),
        ("fcast-bench", 4),
        ("fcast-app", 5),
    ]);
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

    for (&crate_name, &my_rank) in &ranks {
        let toml_path = root.join("crates").join(crate_name).join("Cargo.toml");
        let text = fs::read_to_string(&toml_path).expect("read crate Cargo.toml");
        for (&other, &other_rank) in &ranks {
            if other != crate_name && text.contains(&format!("{other}.workspace")) {
                assert!(
                    other_rank < my_rank,
                    "DAG cycle or rank violation: {crate_name} (L{my_rank}) depends on {other} (L{other_rank})"
                );
            }
        }
        for b in banned {
            assert!(
                !text.contains(&format!("\n{b} ")) && !text.contains(&format!("\n{b}.")),
                "Banned crate {b} found in {crate_name}/Cargo.toml"
            );
        }
    }
}
