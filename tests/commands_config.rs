use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;

use fuselect::app::{AppContext, AppError, OutputMode, PresetInput, WorkerInput};
use fuselect::secrets::{FakeSecretStore, SecretError, SecretRef, SecretStore, SecretString};
use fuselect::storage::SqliteStore;

async fn app() -> (tempfile::TempDir, AppContext, Arc<FakeSecretStore>) {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(FakeSecretStore::new());
    let store = SqliteStore::open_path(dir.path().join("fuselect.sqlite"))
        .await
        .unwrap();
    (
        dir,
        AppContext {
            store,
            secrets: secrets.clone(),
            output_mode: OutputMode::Json,
        },
        secrets,
    )
}

fn worker(id: &str, secret_ref: &str) -> WorkerInput {
    WorkerInput {
        id: id.into(),
        name: id.into(),
        base_url: "https://example.test".into(),
        model: "model".into(),
        input_price_microusd: 1,
        output_price_microusd: 2,
        cached_input_price_microusd: None,
        context_window: 4096,
        capabilities: vec!["tools".into(), "coding".into()],
        provider_policy_url: "https://example.test/policy".into(),
        compatibility_profile: "openai-chat-completions".into(),
        secret_ref: Some(secret_ref.into()),
    }
}

#[derive(Default)]
struct DeleteFailingSecretStore {
    entries: Mutex<BTreeMap<String, String>>,
}

impl SecretStore for DeleteFailingSecretStore {
    fn exists(&self, reference: &SecretRef) -> Result<bool, SecretError> {
        Ok(self
            .entries
            .lock()
            .map_err(|_| SecretError::Backend)?
            .contains_key(reference.as_str()))
    }

    fn set(&self, reference: &SecretRef, secret: SecretString) -> Result<(), SecretError> {
        self.entries
            .lock()
            .map_err(|_| SecretError::Backend)?
            .insert(reference.as_str().to_owned(), secret.expose().to_owned());
        Ok(())
    }

    fn get(&self, reference: &SecretRef) -> Result<SecretString, SecretError> {
        self.entries
            .lock()
            .map_err(|_| SecretError::Backend)?
            .get(reference.as_str())
            .cloned()
            .map(SecretString::from)
            .ok_or(SecretError::NotFound)
    }

    fn delete(&self, _reference: &SecretRef) -> Result<(), SecretError> {
        Err(SecretError::Backend)
    }
}

#[tokio::test]
async fn init_is_idempotent_and_never_exposes_gateway_key() {
    let (_dir, app, secrets) = app().await;
    let first = app.init().await.unwrap();
    let reference = SecretRef::gateway_default();
    let stored = secrets.get(&reference).unwrap();
    let second = app.init().await.unwrap();
    assert_eq!(first["status"], "initialized");
    assert_eq!(second["status"], "initialized");
    assert_eq!(stored.expose(), secrets.get(&reference).unwrap().expose());
    assert!(!first.to_string().contains(stored.expose()));
}

#[tokio::test]
async fn worker_and_preset_commands_keep_secrets_out_of_output() {
    let (_dir, app, secrets) = app().await;
    let reference = SecretRef::new("test/coder").unwrap();
    secrets
        .set(&reference, SecretString::from("TOP_SECRET_API_KEY"))
        .unwrap();
    app.add_worker(worker("coder-a", reference.as_str()), None)
        .await
        .unwrap();
    let listed = app.workers().await.unwrap();
    assert_eq!(listed[0]["secret_status"], "configured");
    assert!(!listed.to_string().contains("TOP_SECRET_API_KEY"));
    let error = app.worker_test("coder-a").await.unwrap_err();
    assert!(matches!(error, AppError::NetworkProbeUnavailable));
    let rejected = app
        .add_preset(PresetInput {
            name: "bad".into(),
            quality_tier: "high".into(),
            outer_worker_policy: "any".into(),
            advisors: vec!["coder-a".into()],
            judge: "coder-a".into(),
            max_completion_tokens: 1,
            task_budget_microusd: 1,
            daily_budget_microusd: 1,
        })
        .await
        .unwrap_err();
    assert!(matches!(rejected, AppError::Validation(_)));
}

#[tokio::test]
async fn default_preset_templates_are_explicitly_unavailable_without_workers() {
    let (_dir, app, _secrets) = app().await;
    let presets = app.presets().await.unwrap();
    assert_eq!(presets[0]["name"], "coding-budget");
    assert_eq!(presets[0]["status"], "template_unavailable");
    assert!(presets.to_string().contains("不会伪造 Worker ID"));
    assert_eq!(
        app.preset("coding-high").await.unwrap()["quality_tier"],
        "high"
    );
}

#[tokio::test]
async fn worker_secret_paths_and_capacity_are_enforced() {
    let (_dir, app, secrets) = app().await;
    let missing = app
        .add_worker(worker("missing", "test/missing"), None)
        .await
        .unwrap_err();
    assert!(matches!(missing, AppError::NotFound(_)));

    for index in 0..10 {
        let id = format!("worker-{index}");
        let input = WorkerInput {
            secret_ref: None,
            ..worker(&id, "")
        };
        app.add_worker(input, Some(SecretString::from(format!("key-{index}"))))
            .await
            .unwrap();
    }
    let eleventh = WorkerInput {
        secret_ref: None,
        ..worker("worker-10", "")
    };
    let error = app
        .add_worker(eleventh, Some(SecretString::from("key-10")))
        .await
        .unwrap_err();
    assert!(matches!(error, AppError::Validation(_)));
    assert!(
        !secrets
            .exists(&SecretRef::worker("worker-10").unwrap())
            .unwrap()
    );
}

#[tokio::test]
async fn preset_advisor_bounds_and_unknown_workers_are_rejected() {
    let (_dir, app, secrets) = app().await;
    for id in ["a", "b"] {
        let reference = SecretRef::worker(id).unwrap();
        secrets.set(&reference, SecretString::from("key")).unwrap();
        app.add_worker(worker(id, reference.as_str()), None)
            .await
            .unwrap();
    }
    for advisors in [
        vec![],
        vec!["a".into(); 9],
        vec!["unknown".into()],
        vec!["a".into(), "a".into()],
    ] {
        let error = app
            .add_preset(PresetInput {
                name: format!("bad-{}", advisors.len()),
                quality_tier: "high".into(),
                outer_worker_policy: "any".into(),
                advisors,
                judge: "b".into(),
                max_completion_tokens: 1,
                task_budget_microusd: 1,
                daily_budget_microusd: 1,
            })
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Validation(_)));
    }
}

#[tokio::test]
async fn presets_accept_one_through_eight_unique_advisors() {
    let (_dir, app, secrets) = app().await;
    let worker_ids: Vec<String> = (0..8)
        .map(|index| format!("advisor-{index}"))
        .chain(std::iter::once("judge".to_owned()))
        .collect();
    for id in &worker_ids {
        let reference = SecretRef::worker(id).unwrap();
        secrets.set(&reference, SecretString::from("key")).unwrap();
        app.add_worker(worker(id, reference.as_str()), None)
            .await
            .unwrap();
    }

    for (name, advisors) in [
        ("one-advisor", vec!["advisor-0".to_owned()]),
        (
            "eight-advisors",
            (0..8).map(|index| format!("advisor-{index}")).collect(),
        ),
    ] {
        app.add_preset(PresetInput {
            name: name.into(),
            quality_tier: "high".into(),
            outer_worker_policy: "quality-first".into(),
            advisors,
            judge: "judge".into(),
            max_completion_tokens: 1024,
            task_budget_microusd: 10,
            daily_budget_microusd: 100,
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn builtin_templates_remain_visible_alongside_custom_presets() {
    let (_dir, app, secrets) = app().await;
    for id in ["advisor-a", "judge-a"] {
        let reference = SecretRef::worker(id).unwrap();
        secrets.set(&reference, SecretString::from("key")).unwrap();
        app.add_worker(worker(id, reference.as_str()), None)
            .await
            .unwrap();
    }
    app.add_preset(PresetInput {
        name: "custom".into(),
        quality_tier: "high".into(),
        outer_worker_policy: "quality-first".into(),
        advisors: vec!["advisor-a".into()],
        judge: "judge-a".into(),
        max_completion_tokens: 1024,
        task_budget_microusd: 10,
        daily_budget_microusd: 100,
    })
    .await
    .unwrap();

    let presets = app.presets().await.unwrap();
    let names: Vec<&str> = presets
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value["name"].as_str())
        .collect();
    assert_eq!(names, ["custom", "coding-budget", "coding-high"]);
}

#[tokio::test]
async fn closed_database_errors_are_not_treated_as_missing_records() {
    let (_dir, app, secrets) = app().await;
    app.store.close().await;

    assert!(matches!(app.init().await.unwrap_err(), AppError::Storage));
    assert!(
        !secrets
            .exists(&SecretRef::gateway_default())
            .expect("fake store should remain available")
    );

    let input = WorkerInput {
        secret_ref: None,
        ..worker("coder-a", "")
    };
    assert!(matches!(
        app.add_worker(input, Some(SecretString::from("key")))
            .await
            .unwrap_err(),
        AppError::Storage
    ));
    assert!(
        !secrets
            .exists(&SecretRef::worker("coder-a").unwrap())
            .expect("secret must not be written after a database lookup failure")
    );
}

#[tokio::test]
async fn failed_worker_save_reports_cleanup_pending_when_secret_delete_fails() {
    let dir = tempfile::tempdir().unwrap();
    let secrets = Arc::new(DeleteFailingSecretStore::default());
    let app = AppContext {
        store: SqliteStore::open_path(dir.path().join("fuselect.sqlite"))
            .await
            .unwrap(),
        secrets: secrets.clone(),
        output_mode: OutputMode::Json,
    };
    for index in 0..10 {
        let input = WorkerInput {
            secret_ref: None,
            ..worker(&format!("worker-{index}"), "")
        };
        app.add_worker(input, Some(SecretString::from("key")))
            .await
            .unwrap();
    }

    let input = WorkerInput {
        secret_ref: None,
        ..worker("worker-10", "")
    };
    assert!(matches!(
        app.add_worker(input, Some(SecretString::from("orphaned-key")))
            .await
            .unwrap_err(),
        AppError::CleanupPending
    ));
    assert!(
        secrets
            .exists(&SecretRef::worker("worker-10").unwrap())
            .unwrap(),
        "cleanup failure must remain visible for an explicit retry"
    );
}
