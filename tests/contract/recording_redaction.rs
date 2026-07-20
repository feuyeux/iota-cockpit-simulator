use std::fs;

use cockpit_agent::{HumanDecision, HumanTurnEvidence};
use cockpit_recording::{RecordingStore, run_rule_agent_recording};
use cockpit_scenario::load_scenario;
use serde_json::{Value, json};

#[test]
fn recording_payloads_redact_nested_secrets_before_writing_to_disk() {
    let database =
        std::env::temp_dir().join(format!("cockpit-redaction-{}.sqlite", uuid::Uuid::new_v4()));
    let database_path = database.to_string_lossy().to_string();
    let mut recording = run_rule_agent_recording(
        "redaction-run",
        load_scenario("scenarios/smoke-in-cockpit.yaml").expect("scenario"),
        4,
    )
    .expect("recording");
    let trace = recording
        .ticks
        .iter_mut()
        .find_map(|tick| tick.tool_calls.first_mut())
        .expect("tool trace");
    trace.arguments = json!({
        "nested": {
            "apiKey": "api-key-must-not-persist",
            "auth_token": "token-must-not-persist",
            "prompt": "complete-private-prompt",
            "credential": "credential-must-not-persist"
        }
    });
    trace.result = json!({ "secret": "secret-must-not-persist" });
    recording.push_human_turns(vec![HumanTurnEvidence {
        human_id: "pilot-1".to_string(),
        decision: HumanDecision {
            narrative: "prompt-must-not-persist".to_string(),
            utterance: Some("token-must-not-persist".to_string()),
            ..HumanDecision::default()
        },
        tool_calls: Vec::new(),
        latency_ms: None,
    }]);

    let exported = String::from_utf8(
        cockpit_recording::serialize_redacted_recording(&recording)
            .expect("external evaluator recording serializes"),
    )
    .expect("recording JSON is UTF-8");
    for secret in [
        "api-key-must-not-persist",
        "complete-private-prompt",
        "prompt-must-not-persist",
        "token-must-not-persist",
    ] {
        assert!(
            !exported.contains(secret),
            "exported recording contains {secret}"
        );
    }
    assert!(exported.contains("[REDACTED]"));

    let mut store = RecordingStore::open(&database_path).expect("store");
    store.save(&recording).expect("recording saves");
    let loaded = store.load("redaction-run").expect("recording loads");
    let trace = loaded
        .ticks
        .iter()
        .find_map(|tick| tick.tool_calls.first())
        .expect("stored tool trace");
    assert_eq!(
        trace.arguments["nested"]["apiKey"],
        Value::String("[REDACTED]".to_string())
    );
    assert_eq!(
        trace.arguments["nested"]["auth_token"],
        Value::String("[REDACTED]".to_string())
    );
    assert_eq!(
        trace.arguments["nested"]["prompt"],
        Value::String("[REDACTED]".to_string())
    );
    assert_eq!(
        trace.arguments["nested"]["credential"],
        Value::String("[REDACTED]".to_string())
    );
    assert_eq!(
        trace.result["secret"],
        Value::String("[REDACTED]".to_string())
    );

    let payload_root = database.with_extension("payloads");
    let stored = fs::read_dir(&payload_root)
        .expect("payload fanout")
        .flat_map(|entry| fs::read_dir(entry.expect("fanout entry").path()).expect("payload files"))
        .map(|entry| {
            fs::read_to_string(entry.expect("payload file").path()).expect("payload contents")
        })
        .collect::<String>();
    for secret in [
        "api-key-must-not-persist",
        "token-must-not-persist",
        "complete-private-prompt",
        "secret-must-not-persist",
        "prompt-must-not-persist",
        "token-must-not-persist",
        "credential-must-not-persist",
    ] {
        assert!(!stored.contains(secret), "recording contains {secret}");
    }

    let _ = fs::remove_file(&database);
    let _ = fs::remove_dir_all(payload_root);
}
