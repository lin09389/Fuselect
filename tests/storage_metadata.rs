use std::collections::BTreeSet;
use std::fs;
use std::sync::Arc;

use chrono::Utc;
use fuselect::config::GatewayConfig;
use fuselect::domain::{
    BudgetLimits, Capability, FusionPreset, PricePerMillion, QualityTier, WorkerCapabilities,
    WorkerConfig,
};
use fuselect::secrets::{FakeSecretStore, SecretRef, SecretStore, SecretString};
use fuselect::storage::{
    CompleteRunUpdate, NewRequestRun, NewRunParticipant, NewRunStage, RunParticipantRole,
    SqliteStore, StorageError,
};
use sqlx::Connection;
use sqlx::sqlite::SqliteConnectOptions;
use tempfile::TempDir;

fn sample_worker(id: &str) -> WorkerConfig {
    WorkerConfig {
        id: id.to_owned(),
        display_name: format!("Worker {id}"),
        base_url: "https://api.example.test/v1".to_owned(),
        model_id: "example-coder".to_owned(),
        pricing: PricePerMillion {
            input_microusd: 1_000_000,
            output_microusd: 2_000_000,
            cached_input_microusd: Some(250_000),
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

fn sample_preset(name: &str, advisor: &str, judge: &str) -> FusionPreset {
    FusionPreset {
        name: name.to_owned(),
        quality_tier: QualityTier::High,
        outer_worker_policy: "quality-first".to_owned(),
        advisor_worker_ids: vec![advisor.to_owned()],
        judge_worker_id: judge.to_owned(),
        max_completion_tokens: 4_096,
        budgets: BudgetLimits {
            per_task_microusd: 100_000,
            daily_microusd: 1_000_000,
        },
        enabled: true,
    }
}

async fn open_temp_store() -> (TempDir, SqliteStore) {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("metadata.sqlite");
    let store = SqliteStore::open_path(&path).await.expect("open store");
    (dir, store)
}

#[tokio::test]
async fn persists_worker_metadata_without_api_key() {
    let (_dir, store) = open_temp_store().await;
    let worker = sample_worker("coder-a");
    store.save_worker(&worker).await.unwrap();

    let loaded = store.get_worker("coder-a").await.unwrap();
    assert_eq!(loaded.config.id, "coder-a");
    assert_eq!(loaded.config.secret_ref, "fuselect/worker/coder-a");
    assert!(!format!("{loaded:?}").contains("TOP_SECRET_API_KEY"));

    let columns = store.table_column_names("workers").await.unwrap();
    for forbidden in ["api_key", "authorization", "bearer_token", "secret_value"] {
        assert!(
            !columns.iter().any(|name| name == forbidden),
            "forbidden column present: {forbidden}"
        );
    }
    assert!(columns.iter().any(|name| name == "secret_ref"));
}

#[tokio::test]
async fn lists_workers_in_stable_id_order() {
    let (_dir, store) = open_temp_store().await;
    store.save_worker(&sample_worker("coder-b")).await.unwrap();
    store.save_worker(&sample_worker("coder-a")).await.unwrap();
    let ids: Vec<_> = store
        .list_workers()
        .await
        .unwrap()
        .into_iter()
        .map(|worker| worker.config.id)
        .collect();
    assert_eq!(ids, vec!["coder-a".to_owned(), "coder-b".to_owned()]);
}

#[tokio::test]
async fn duplicate_worker_id_returns_conflict() {
    let (_dir, store) = open_temp_store().await;
    store.save_worker(&sample_worker("coder-a")).await.unwrap();
    let err = store
        .save_worker(&sample_worker("coder-a"))
        .await
        .expect_err("duplicate");
    assert!(matches!(err, StorageError::Conflict(_)));
}

#[tokio::test]
async fn disable_and_delete_worker_behavior_is_explicit() {
    let (_dir, store) = open_temp_store().await;
    let secrets = FakeSecretStore::new();
    let worker = sample_worker("coder-a");
    let secret_ref = SecretRef::worker("coder-a").unwrap();
    secrets
        .set(&secret_ref, SecretString::from("TOP_SECRET_API_KEY"))
        .unwrap();
    store.save_worker(&worker).await.unwrap();

    store.disable_worker("coder-a").await.unwrap();
    assert!(!store.get_worker("coder-a").await.unwrap().config.enabled);

    store.remove_worker(&secrets, "coder-a").await.unwrap();
    assert!(matches!(
        store.get_worker("coder-a").await,
        Err(StorageError::NotFound(_))
    ));
    assert!(secrets.get(&secret_ref).is_err());
}

#[tokio::test]
async fn fusion_preset_round_trip_and_rejects_missing_worker() {
    let (_dir, store) = open_temp_store().await;
    store.save_worker(&sample_worker("coder-a")).await.unwrap();
    store.save_worker(&sample_worker("judge-a")).await.unwrap();

    let preset = sample_preset("coding-high", "coder-a", "judge-a");
    store.save_fusion_preset(&preset).await.unwrap();
    let loaded = store.get_fusion_preset("coding-high").await.unwrap();
    assert_eq!(loaded.preset.advisor_worker_ids, vec!["coder-a".to_owned()]);
    assert_eq!(loaded.preset.judge_worker_id, "judge-a");

    let invalid = sample_preset("broken", "missing-worker", "judge-a");
    let err = store
        .save_fusion_preset(&invalid)
        .await
        .expect_err("missing");
    assert!(matches!(err, StorageError::InvalidConfig(_)));
}

#[tokio::test]
async fn gateway_config_stores_only_key_ref() {
    let (_dir, store) = open_temp_store().await;
    let config = GatewayConfig {
        gateway_key_ref: "fuselect/gateway/default".to_owned(),
        ..GatewayConfig::default()
    };
    store.save_gateway_config(&config).await.unwrap();

    let loaded = store.get_gateway_config().await.unwrap();
    assert_eq!(loaded.config.gateway_key_ref, "fuselect/gateway/default");
    assert!(!format!("{loaded:?}").contains("TOP_SECRET_GATEWAY_KEY"));

    let columns = store.table_column_names("gateway").await.unwrap();
    assert!(columns.iter().any(|name| name == "gateway_key_ref"));
    assert!(!columns.iter().any(|name| name == "gateway_key"));
}

#[tokio::test]
async fn request_runs_store_metadata_only() {
    let (dir, store) = open_temp_store().await;
    let run_id = store
        .begin_request_run(&NewRequestRun {
            request_id_hash: "hash-abc".to_owned(),
            public_model: "fuselect/auto".to_owned(),
            route_mode: "direct".to_owned(),
            selected_outer_worker_id: Some("coder-a".to_owned()),
            preset_name: None,
            started_at: Utc::now(),
        })
        .await
        .unwrap();

    store
        .add_run_participants(
            run_id,
            &[NewRunParticipant {
                worker_id: "coder-a".to_owned(),
                role: RunParticipantRole::Outer,
                stage_order: 0,
            }],
        )
        .await
        .unwrap();

    store
        .append_run_stage(
            run_id,
            &NewRunStage {
                stage_type: "outer".to_owned(),
                worker_id: Some("coder-a".to_owned()),
                attempt_number: 1,
                started_at: Utc::now(),
            },
        )
        .await
        .unwrap();

    store
        .complete_request_run(
            run_id,
            &CompleteRunUpdate {
                latency_ms: 42,
                input_tokens: Some(10),
                output_tokens: Some(20),
                known_cost_microusd: Some(1_000),
                cost_unknown: false,
                outcome: "ok".to_owned(),
                error_category: None,
            },
        )
        .await
        .unwrap();

    let run = store.get_request_run(run_id).await.unwrap();
    assert_eq!(run.outcome.as_deref(), Some("ok"));
    assert_eq!(run.public_model, "fuselect/auto");

    let db_bytes = fs::read(dir.path().join("metadata.sqlite")).unwrap();
    let db_text = String::from_utf8_lossy(&db_bytes);
    for secret in [
        "TOP_SECRET_API_KEY",
        "TOP_SECRET_GATEWAY_KEY",
        "TOP_SECRET_PROMPT",
        "TOP_SECRET_TOOL_OUTPUT",
    ] {
        assert!(
            !db_text.contains(secret),
            "database file leaked marker: {secret}"
        );
    }
    assert!(!db_text.contains("api_key"));
}

#[tokio::test]
async fn migrations_are_idempotent_and_reject_future_versions() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("metadata.sqlite");
    let store = SqliteStore::open_path(&path).await.unwrap();
    assert_eq!(store.database_version().await.unwrap(), 1);
    store.migrate().await.unwrap();
    store.migrate().await.unwrap();
    assert_eq!(store.database_version().await.unwrap(), 1);
    assert!(store.foreign_keys_enabled().await.unwrap());
    assert_eq!(store.journal_mode().await.unwrap(), "wal");
    assert!(store.busy_timeout_ms().await.unwrap() >= 5_000);

    drop(store);

    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(false);
    let pool = sqlx::SqlitePool::connect_with(options).await.unwrap();
    sqlx::query("INSERT INTO schema_migrations (version, name, applied_at) VALUES (99, 'future', '2026-01-01T00:00:00Z')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let err = SqliteStore::open_path(&path).await.expect_err("too new");
    assert!(matches!(
        err,
        StorageError::UnsupportedDatabaseVersion {
            found: 99,
            supported: 1
        }
    ));
}

#[tokio::test]
async fn failed_migration_rolls_back_transaction() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("metadata.sqlite");
    let _store = SqliteStore::open_path(&path).await.unwrap();

    let options = SqliteConnectOptions::new()
        .filename(&path)
        .foreign_keys(true);
    let mut conn = sqlx::SqliteConnection::connect_with(&options)
        .await
        .unwrap();
    fuselect::storage::migrations_test::apply_failing_migration(&mut conn)
        .await
        .unwrap();
    assert!(
        !fuselect::storage::migrations_test::probe_table_exists(&mut conn, "migration_probe")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn database_reopen_preserves_workers() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("metadata.sqlite");
    {
        let store = SqliteStore::open_path(&path).await.unwrap();
        store.save_worker(&sample_worker("coder-a")).await.unwrap();
    }
    let store = SqliteStore::open_path(&path).await.unwrap();
    assert_eq!(
        store.get_worker("coder-a").await.unwrap().config.id,
        "coder-a"
    );
}

#[tokio::test]
async fn concurrent_writes_do_not_corrupt_database() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("metadata.sqlite");
    let store = Arc::new(SqliteStore::open_path(&path).await.unwrap());

    let mut handles = Vec::new();
    for index in 0..8 {
        let store = Arc::clone(&store);
        handles.push(tokio::spawn(async move {
            let id = format!("coder-{index}");
            store.save_worker(&sample_worker(&id)).await.unwrap();
            store.get_worker(&id).await.unwrap();
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }
    assert_eq!(store.list_workers().await.unwrap().len(), 8);
}

#[tokio::test]
async fn daily_budget_reserve_and_settle_are_transactional() {
    let (_dir, store) = open_temp_store().await;
    let reserved = store
        .reserve_daily_budget("2026-07-23", 10_000)
        .await
        .unwrap();
    assert_eq!(reserved.reserved_microusd, 10_000);

    let settled = store
        .settle_daily_budget("2026-07-23", 10_000, 8_000)
        .await
        .unwrap();
    assert_eq!(settled.reserved_microusd, 0);
    assert_eq!(settled.settled_microusd, 8_000);

    store
        .reserve_daily_budget("2026-07-23", 5_000)
        .await
        .unwrap();
    let cleared = store.reconcile_orphaned_reservations().await.unwrap();
    assert_eq!(cleared, 1);
}

#[tokio::test]
async fn rejects_values_that_exceed_sqlites_signed_integer_range() {
    let (_dir, store) = open_temp_store().await;
    let mut worker = sample_worker("coder-a");
    worker.pricing.input_microusd = u64::MAX;
    assert!(matches!(
        store.save_worker(&worker).await,
        Err(StorageError::ValueOutOfRange)
    ));

    assert!(matches!(
        store.reserve_daily_budget("2026-07-23", u64::MAX).await,
        Err(StorageError::ValueOutOfRange)
    ));
}

#[tokio::test]
async fn rejects_unknown_table_names_in_schema_introspection() {
    let (_dir, store) = open_temp_store().await;
    assert!(matches!(
        store
            .table_column_names("workers); DROP TABLE workers; --")
            .await,
        Err(StorageError::InvalidTable)
    ));
    assert_eq!(store.list_workers().await.unwrap().len(), 0);
}
