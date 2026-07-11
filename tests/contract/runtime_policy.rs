use std::time::Duration;

use cockpit_agent_runtime::{AgentRuntimePolicy, FallbackPolicy, TurnDisposition};

#[tokio::test(flavor = "current_thread")]
async fn timed_out_agent_turn_returns_explicit_rule_fallback() {
    let policy = AgentRuntimePolicy::new(5, 1, FallbackPolicy::RuleAgent);
    let turn = policy
        .execute(
            async {
                tokio::time::sleep(Duration::from_millis(30)).await;
                Ok::<_, &'static str>("external-agent")
            },
            || "rule-agent",
        )
        .await;

    assert_eq!(turn.value, "rule-agent");
    assert!(matches!(turn.disposition, TurnDisposition::Fallback { .. }));
    assert!(turn.elapsed_ms < 1000);
}
