//! Integration & contract tests for `fcast_infra::llm::GeminiCliClient` (`google-gemini/gemini-api-cli`).

use fcast_infra::llm::{
    GEMINI_4_HYDROGEN, GeminiCliClient, GeminiCliConfig, GeminiCliError, GeminiGenerateRequest,
    extract_interaction_text,
};
use serde_json::json;
use std::path::PathBuf;
use tempfile::TempDir;

#[test]
fn builds_structured_multiturn_generate_args_cleanly() {
    let client = GeminiCliClient::from_env();
    let mut req = GeminiGenerateRequest::new(GEMINI_4_HYDROGEN, "Forecast EU grid cyber risk.");
    req.system_instruction = Some("Return strict JSON only.".into());
    req.previous_interaction_id = Some("v1_turn_a_123".into());
    req.inputs.push("files/abc123".into());
    req.body_json = Some(json!({
        "response_format": {
            "type": "text",
            "mime_type": "application/json",
            "schema": {"type": "object", "properties": {"p": {"type": "number"}}}
        },
        "store": true
    }));
    req.dry_run = true;

    let args = client.build_generate_args(&req);
    assert!(args.contains(&"generate".to_owned()));
    assert!(args.contains(&GEMINI_4_HYDROGEN.to_owned()));
    assert!(args.contains(&"--stdin".to_owned()));
    assert!(args.contains(&"--stream=false".to_owned()));
    assert!(args.contains(&"--no-interactive".to_owned()));
    assert!(args.contains(&"--previous-interaction-id".to_owned()));
    assert!(args.contains(&"v1_turn_a_123".to_owned()));
    assert!(args.contains(&"--system".to_owned()));
    assert!(args.contains(&"--body".to_owned()));
    assert!(args.contains(&"--dry-run".to_owned()));
}

#[test]
fn extracts_text_from_interactions_steps_and_handles_missing_binary() {
    let sample = json!({
        "id": "v1_inter_01",
        "status": "completed",
        "steps": [
            {"type": "thought", "content": [{"type": "text", "text": "hidden"}]},
            {"type": "model_output", "content": [
                {"type": "text", "text": "{\"p_indep\": "},
                {"type": "text", "text": "0.24}"}
            ]}
        ]
    });
    assert_eq!(extract_interaction_text(&sample), "{\"p_indep\": 0.24}");

    let missing = GeminiCliClient::new(GeminiCliConfig {
        binary: PathBuf::from("/nonexistent/gemini-api-does-not-exist"),
        ..GeminiCliConfig::default()
    });
    let err = missing.whoami();
    assert!(matches!(err, Err(GeminiCliError::BinaryNotFound { .. })));
}

#[test]
fn live_installed_gemini_api_whoami_and_offline_dry_run() {
    // Locate installed `gemini-api` binary if present on this workstation.
    let candidates = [
        std::env::var_os("GEMINI_API_BIN").map(PathBuf::from),
        Some(PathBuf::from("/Users/macbookpro/.local/bin/gemini-api")),
    ];
    let Some(bin_path) = candidates.into_iter().flatten().find(|p| p.is_file()) else {
        return;
    };

    let tmp_home = TempDir::new().expect("temp home");
    std::fs::create_dir_all(tmp_home.path().join(".config/gemini-api")).expect("cfg dir");

    let client = GeminiCliClient::new(GeminiCliConfig {
        binary: bin_path,
        api_key: Some("dummy-dry-run-key".into()),
        server_url: None,
        api_revision: None,
        home_override: Some(tmp_home.path().to_path_buf()),
        no_keyring: true,
        ..GeminiCliConfig::default()
    });

    let who = client.whoami().expect("whoami json");
    assert_eq!(who.api_key.source, "env");
    assert_eq!(who.api_version.as_deref(), Some("v1beta"));

    let mut req = GeminiGenerateRequest::new(GEMINI_4_HYDROGEN, "Forecast dry-run ping.");
    req.system_instruction = Some("Be concise.".into());
    req.dry_run = true;

    let resp = client.generate(&req).expect("dry-run generate");
    assert!(resp.dry_run);
    assert_eq!(resp.model, GEMINI_4_HYDROGEN);
    assert_eq!(resp.raw_json["request"]["method"], "POST");
    assert!(
        resp.raw_json["request"]["url"]
            .as_str()
            .unwrap_or_default()
            .contains("/interactions")
    );
    assert_eq!(resp.raw_json["request"]["body"]["stream"], false);
}
