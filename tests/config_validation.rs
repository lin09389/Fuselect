use std::collections::BTreeSet;

use fuselect::{
    config::{FuselectConfig, GatewayConfig},
    domain::{
        BudgetLimits, Capability, FusionPolicy, PricePerMillion, QualityTier, WorkerCapabilities,
        WorkerConfig,
    },
};

fn worker(id: &str) -> WorkerConfig {
    WorkerConfig {
        id: id.to_owned(),
        display_name: format!("Worker {id}"),
        base_url: "https://api.example.test/v1".to_owned(),
        model_id: "example-coder".to_owned(),
        pricing: PricePerMillion {
            input_microusd: 1_000_000,
            output_microusd: 2_000_000,
            cached_input_microusd: None,
        },
        capabilities: WorkerCapabilities {
            tags: BTreeSet::from([Capability::Coding, Capability::Reasoning, Capability::Tools]),
            supports_streaming: true,
        },
        context_window_tokens: 128_000,
        provider_policy_url: "https://example.test/privacy".to_owned(),
        secret_ref: format!("fuselect/worker/{id}"),
        compatibility_profile: "openai-chat-completions".to_owned(),
        enabled: true,
    }
}

fn config(workers: Vec<WorkerConfig>) -> FuselectConfig {
    FuselectConfig {
        schema_version: FuselectConfig::CURRENT_SCHEMA_VERSION,
        gateway: GatewayConfig::default(),
        fusion_policy: FusionPolicy {
            default_tier: QualityTier::High,
            budgets: BudgetLimits {
                per_task_microusd: 100_000,
                daily_microusd: 1_000_000,
            },
        },
        workers,
    }
}

#[test]
fn rejects_an_eleventh_worker() {
    let workers = (0..11)
        .map(|index| worker(&format!("coder-{index}")))
        .collect();
    assert!(config(workers).validate().is_err());
}

#[test]
fn requires_tools_and_streaming_for_every_worker() {
    let mut without_tools = worker("coder-a");
    without_tools.capabilities.tags.remove(&Capability::Tools);
    assert!(config(vec![without_tools]).validate().is_err());

    let mut without_streaming = worker("coder-b");
    without_streaming.capabilities.supports_streaming = false;
    assert!(config(vec![without_streaming]).validate().is_err());
}

#[test]
fn rejects_duplicate_worker_ids() {
    let workers = vec![worker("coder-a"), worker("coder-a")];
    assert!(config(workers).validate().is_err());
}

#[test]
fn rejects_invalid_worker_ids() {
    let mut invalid = worker("coder-a");
    invalid.id = "Coder-A".to_owned();
    assert!(config(vec![invalid]).validate().is_err());
}

#[test]
fn worker_config_uses_keyring_reference_instead_of_api_key() {
    let worker = worker("coder-a");
    assert!(worker.secret_ref.starts_with("fuselect/"));

    let persisted_fields = [
        worker.id.as_str(),
        worker.display_name.as_str(),
        worker.base_url.as_str(),
        worker.model_id.as_str(),
        worker.secret_ref.as_str(),
        worker.provider_policy_url.as_str(),
    ];
    for field in persisted_fields {
        assert!(
            !field.starts_with("sk-") && !field.starts_with("Bearer "),
            "raw API key material must not appear in persisted WorkerConfig fields"
        );
    }
}

#[test]
fn rejects_non_https_non_loopback_worker_urls() {
    let mut invalid = worker("coder-a");
    invalid.base_url = "http://provider.example.test/v1".to_owned();
    assert!(config(vec![invalid]).validate().is_err());
}

#[test]
fn accepts_a_valid_minimal_worker_config() {
    assert!(config(vec![worker("coder-a")]).validate().is_ok());
}

#[test]
fn accepts_loopback_test_worker_urls() {
    let mut loopback = worker("coder-a");
    loopback.base_url = "http://127.0.0.1:8080/v1".to_owned();
    assert!(config(vec![loopback]).validate().is_ok());
}
