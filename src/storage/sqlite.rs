use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use chrono::Utc;
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow, SqliteSynchronous,
};
use sqlx::{Row, SqlitePool};

use crate::config::GatewayConfig;
use crate::domain::{FusionPreset, MAX_WORKERS, WorkerConfig, WorkerHealthStatus};
use crate::secrets::{SecretError, SecretRef, SecretStore};

use super::migrations::{self, CURRENT_DB_VERSION};
use super::models::{
    CompleteRunUpdate, DailySpendRecord, NewRequestRun, NewRunParticipant, NewRunStage,
    PersistedFusionPreset, PersistedGateway, PersistedWorker, PresetRow, RequestRunRecord,
    RunStageRecord, WorkerRow, encode_advisor_ids, encode_capabilities, format_time, parse_time,
    preset_from_row, worker_from_row,
};
use super::{StorageError, database_path, resolve_data_dir};

const WORKER_SCHEMA_VERSION: u32 = 1;
const PRESET_SCHEMA_VERSION: u32 = 1;
const GATEWAY_SCHEMA_VERSION: u32 = 1;
const BUSY_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, Clone)]
pub struct SqliteStore {
    pool: SqlitePool,
    path: PathBuf,
}

impl SqliteStore {
    pub async fn open_default() -> Result<Self, StorageError> {
        Self::open_path(database_path()).await
    }

    /// Close all database connections. Primarily used by lifecycle code and
    /// failure-path tests to prove callers do not swallow backend errors.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub async fn open_path(path: impl AsRef<Path>) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| StorageError::Database)?;
        }

        let options = SqliteConnectOptions::from_str(&format!("sqlite:{}", path.display()))
            .map_err(|_| StorageError::Database)?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_millis(BUSY_TIMEOUT_MS));

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .map_err(StorageError::from_sqlx)?;

        let store = Self { pool, path };
        store.migrate().await?;
        store.verify_pragmas().await?;
        Ok(store)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn data_dir_hint() -> PathBuf {
        resolve_data_dir()
    }

    pub async fn migrate(&self) -> Result<(), StorageError> {
        let mut connection = self.pool.acquire().await.map_err(StorageError::from_sqlx)?;
        migrations::migrate(&mut connection).await
    }

    pub async fn database_version(&self) -> Result<i64, StorageError> {
        let mut connection = self.pool.acquire().await.map_err(StorageError::from_sqlx)?;
        migrations::current_version(&mut connection).await
    }

    pub async fn foreign_keys_enabled(&self) -> Result<bool, StorageError> {
        let row: (i64,) = sqlx::query_as("PRAGMA foreign_keys")
            .fetch_one(&self.pool)
            .await
            .map_err(StorageError::from_sqlx)?;
        Ok(row.0 == 1)
    }

    pub async fn journal_mode(&self) -> Result<String, StorageError> {
        let row: (String,) = sqlx::query_as("PRAGMA journal_mode")
            .fetch_one(&self.pool)
            .await
            .map_err(StorageError::from_sqlx)?;
        Ok(row.0.to_lowercase())
    }

    pub async fn busy_timeout_ms(&self) -> Result<i64, StorageError> {
        let row: (i64,) = sqlx::query_as("PRAGMA busy_timeout")
            .fetch_one(&self.pool)
            .await
            .map_err(StorageError::from_sqlx)?;
        Ok(row.0)
    }

    async fn verify_pragmas(&self) -> Result<(), StorageError> {
        if !self.foreign_keys_enabled().await? {
            return Err(StorageError::Database);
        }
        Ok(())
    }

    pub async fn save_worker(&self, worker: &WorkerConfig) -> Result<(), StorageError> {
        worker.validate()?;
        let existing = self.list_workers().await?;
        if existing.iter().all(|item| item.config.id != worker.id) && existing.len() >= MAX_WORKERS
        {
            return Err(StorageError::InvalidConfig(
                crate::domain::ConfigError::TooManyWorkers {
                    maximum: MAX_WORKERS,
                    found: existing.len() + 1,
                },
            ));
        }

        let now = format_time(Utc::now());
        let capabilities = encode_capabilities(&worker.capabilities)?;
        sqlx::query(
            "INSERT INTO workers (
                id, display_name, base_url, model_id,
                input_price_microusd, output_price_microusd, cached_input_price_microusd,
                context_window_tokens, capabilities, supports_streaming,
                provider_policy_url, secret_ref, compatibility_profile,
                enabled, health_status, created_at, updated_at, schema_version
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&worker.id)
        .bind(&worker.display_name)
        .bind(&worker.base_url)
        .bind(&worker.model_id)
        .bind(to_sqlite_i64(worker.pricing.input_microusd)?)
        .bind(to_sqlite_i64(worker.pricing.output_microusd)?)
        .bind(
            worker
                .pricing
                .cached_input_microusd
                .map(to_sqlite_i64)
                .transpose()?,
        )
        .bind(worker.context_window_tokens as i64)
        .bind(capabilities)
        .bind(i64::from(worker.capabilities.supports_streaming))
        .bind(&worker.provider_policy_url)
        .bind(&worker.secret_ref)
        .bind(&worker.compatibility_profile)
        .bind(i64::from(worker.enabled))
        .bind(WorkerHealthStatus::Unknown.as_str())
        .bind(&now)
        .bind(&now)
        .bind(WORKER_SCHEMA_VERSION as i64)
        .execute(&self.pool)
        .await
        .map_err(StorageError::from_sqlx)?;
        Ok(())
    }

    pub async fn get_worker(&self, id: &str) -> Result<PersistedWorker, StorageError> {
        let row = sqlx::query(
            "SELECT id, display_name, base_url, model_id,
                    input_price_microusd, output_price_microusd, cached_input_price_microusd,
                    context_window_tokens, capabilities, supports_streaming,
                    provider_policy_url, secret_ref, compatibility_profile,
                    enabled, health_status, created_at, updated_at, schema_version
             FROM workers WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::from_sqlx)?
        .ok_or_else(|| StorageError::NotFound(id.to_owned()))?;
        map_worker_row(&row)
    }

    pub async fn list_workers(&self) -> Result<Vec<PersistedWorker>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, display_name, base_url, model_id,
                    input_price_microusd, output_price_microusd, cached_input_price_microusd,
                    context_window_tokens, capabilities, supports_streaming,
                    provider_policy_url, secret_ref, compatibility_profile,
                    enabled, health_status, created_at, updated_at, schema_version
             FROM workers
             ORDER BY id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::from_sqlx)?;

        let mut workers = Vec::with_capacity(rows.len());
        for row in rows {
            workers.push(map_worker_row(&row)?);
        }
        Ok(workers)
    }

    pub async fn update_worker(&self, worker: &WorkerConfig) -> Result<(), StorageError> {
        worker.validate()?;
        let _existing = self.get_worker(&worker.id).await?;
        let now = format_time(Utc::now());
        let capabilities = encode_capabilities(&worker.capabilities)?;
        let result = sqlx::query(
            "UPDATE workers SET
                display_name = ?,
                base_url = ?,
                model_id = ?,
                input_price_microusd = ?,
                output_price_microusd = ?,
                cached_input_price_microusd = ?,
                context_window_tokens = ?,
                capabilities = ?,
                supports_streaming = ?,
                provider_policy_url = ?,
                secret_ref = ?,
                compatibility_profile = ?,
                enabled = ?,
                updated_at = ?,
                schema_version = ?
             WHERE id = ?",
        )
        .bind(&worker.display_name)
        .bind(&worker.base_url)
        .bind(&worker.model_id)
        .bind(to_sqlite_i64(worker.pricing.input_microusd)?)
        .bind(to_sqlite_i64(worker.pricing.output_microusd)?)
        .bind(
            worker
                .pricing
                .cached_input_microusd
                .map(to_sqlite_i64)
                .transpose()?,
        )
        .bind(worker.context_window_tokens as i64)
        .bind(capabilities)
        .bind(i64::from(worker.capabilities.supports_streaming))
        .bind(&worker.provider_policy_url)
        .bind(&worker.secret_ref)
        .bind(&worker.compatibility_profile)
        .bind(i64::from(worker.enabled))
        .bind(&now)
        .bind(WORKER_SCHEMA_VERSION as i64)
        .bind(&worker.id)
        .execute(&self.pool)
        .await
        .map_err(StorageError::from_sqlx)?;

        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound(worker.id.clone()));
        }
        Ok(())
    }

    pub async fn disable_worker(&self, id: &str) -> Result<(), StorageError> {
        let now = format_time(Utc::now());
        let result = sqlx::query("UPDATE workers SET enabled = 0, updated_at = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(StorageError::from_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound(id.to_owned()));
        }
        Ok(())
    }

    pub async fn delete_worker_metadata(&self, id: &str) -> Result<(), StorageError> {
        let result = sqlx::query("DELETE FROM workers WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(StorageError::from_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound(id.to_owned()));
        }
        Ok(())
    }

    /// Disable → delete Keyring secret → delete metadata.
    pub async fn remove_worker(
        &self,
        secrets: &dyn SecretStore,
        id: &str,
    ) -> Result<(), StorageError> {
        let worker = self.get_worker(id).await?;
        let secret_ref = SecretRef::new(worker.config.secret_ref.clone())
            .map_err(|_| StorageError::CorruptData)?;

        self.disable_worker(id).await?;

        match secrets.delete(&secret_ref) {
            Ok(()) | Err(SecretError::NotFound) => {}
            Err(_) => {
                return Err(StorageError::KeyringCleanupPending(id.to_owned()));
            }
        }

        self.delete_worker_metadata(id).await?;
        Ok(())
    }

    pub async fn save_fusion_preset(&self, preset: &FusionPreset) -> Result<(), StorageError> {
        let workers: Vec<WorkerConfig> = self
            .list_workers()
            .await?
            .into_iter()
            .map(|item| item.config)
            .collect();
        preset.validate(&workers)?;

        let now = format_time(Utc::now());
        let advisors = encode_advisor_ids(&preset.advisor_worker_ids)?;
        sqlx::query(
            "INSERT INTO fusion_presets (
                name, quality_tier, outer_worker_policy, advisor_worker_ids,
                judge_worker_id, max_completion_tokens, task_budget_microusd,
                daily_budget_microusd, enabled, created_at, updated_at, schema_version
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(name) DO UPDATE SET
                quality_tier = excluded.quality_tier,
                outer_worker_policy = excluded.outer_worker_policy,
                advisor_worker_ids = excluded.advisor_worker_ids,
                judge_worker_id = excluded.judge_worker_id,
                max_completion_tokens = excluded.max_completion_tokens,
                task_budget_microusd = excluded.task_budget_microusd,
                daily_budget_microusd = excluded.daily_budget_microusd,
                enabled = excluded.enabled,
                updated_at = excluded.updated_at,
                schema_version = excluded.schema_version",
        )
        .bind(&preset.name)
        .bind(preset.quality_tier.as_str())
        .bind(&preset.outer_worker_policy)
        .bind(advisors)
        .bind(&preset.judge_worker_id)
        .bind(preset.max_completion_tokens as i64)
        .bind(to_sqlite_i64(preset.budgets.per_task_microusd)?)
        .bind(to_sqlite_i64(preset.budgets.daily_microusd)?)
        .bind(i64::from(preset.enabled))
        .bind(&now)
        .bind(&now)
        .bind(PRESET_SCHEMA_VERSION as i64)
        .execute(&self.pool)
        .await
        .map_err(StorageError::from_sqlx)?;
        Ok(())
    }

    pub async fn get_fusion_preset(
        &self,
        name: &str,
    ) -> Result<PersistedFusionPreset, StorageError> {
        let row = sqlx::query(
            "SELECT name, quality_tier, outer_worker_policy, advisor_worker_ids,
                    judge_worker_id, max_completion_tokens, task_budget_microusd,
                    daily_budget_microusd, enabled, created_at, updated_at, schema_version
             FROM fusion_presets WHERE name = ?",
        )
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::from_sqlx)?
        .ok_or_else(|| StorageError::NotFound(name.to_owned()))?;
        map_preset_row(&row)
    }

    pub async fn list_fusion_presets(&self) -> Result<Vec<PersistedFusionPreset>, StorageError> {
        let rows = sqlx::query(
            "SELECT name, quality_tier, outer_worker_policy, advisor_worker_ids,
                    judge_worker_id, max_completion_tokens, task_budget_microusd,
                    daily_budget_microusd, enabled, created_at, updated_at, schema_version
             FROM fusion_presets
             ORDER BY name ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::from_sqlx)?;

        let mut presets = Vec::with_capacity(rows.len());
        for row in rows {
            presets.push(map_preset_row(&row)?);
        }
        Ok(presets)
    }

    pub async fn delete_fusion_preset(&self, name: &str) -> Result<(), StorageError> {
        let result = sqlx::query("DELETE FROM fusion_presets WHERE name = ?")
            .bind(name)
            .execute(&self.pool)
            .await
            .map_err(StorageError::from_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound(name.to_owned()));
        }
        Ok(())
    }

    pub async fn save_gateway_config(&self, config: &GatewayConfig) -> Result<(), StorageError> {
        if config.port == 0 {
            return Err(StorageError::InvalidConfig(
                crate::domain::ConfigError::InvalidGatewayPort,
            ));
        }
        if config.gateway_key_ref.trim().is_empty() {
            return Err(StorageError::InvalidConfig(
                crate::domain::ConfigError::EmptyField("gateway_key_ref"),
            ));
        }
        if config.metadata_retention_days == 0 {
            return Err(StorageError::InvalidConfig(
                crate::domain::ConfigError::EmptyField("metadata_retention_days"),
            ));
        }

        let now = format_time(Utc::now());
        sqlx::query(
            "INSERT INTO gateway (
                id, port, metadata_retention_days, gateway_key_ref,
                durable_session_enabled, created_at, updated_at, schema_version
            ) VALUES (1, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(id) DO UPDATE SET
                port = excluded.port,
                metadata_retention_days = excluded.metadata_retention_days,
                gateway_key_ref = excluded.gateway_key_ref,
                durable_session_enabled = excluded.durable_session_enabled,
                updated_at = excluded.updated_at,
                schema_version = excluded.schema_version",
        )
        .bind(config.port as i64)
        .bind(config.metadata_retention_days as i64)
        .bind(&config.gateway_key_ref)
        .bind(i64::from(config.durable_session_enabled))
        .bind(&now)
        .bind(&now)
        .bind(GATEWAY_SCHEMA_VERSION as i64)
        .execute(&self.pool)
        .await
        .map_err(StorageError::from_sqlx)?;
        Ok(())
    }

    pub async fn get_gateway_config(&self) -> Result<PersistedGateway, StorageError> {
        let row = sqlx::query(
            "SELECT port, metadata_retention_days, gateway_key_ref,
                    durable_session_enabled, created_at, updated_at, schema_version
             FROM gateway WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::from_sqlx)?
        .ok_or_else(|| StorageError::NotFound("gateway".to_owned()))?;

        Ok(PersistedGateway {
            config: GatewayConfig {
                port: row
                    .try_get::<i64, _>("port")
                    .map_err(|_| StorageError::CorruptData)? as u16,
                metadata_retention_days: row
                    .try_get::<i64, _>("metadata_retention_days")
                    .map_err(|_| StorageError::CorruptData)?
                    as u16,
                gateway_key_ref: row
                    .try_get("gateway_key_ref")
                    .map_err(|_| StorageError::CorruptData)?,
                durable_session_enabled: row
                    .try_get::<i64, _>("durable_session_enabled")
                    .map_err(|_| StorageError::CorruptData)?
                    != 0,
            },
            created_at: parse_time(
                &row.try_get::<String, _>("created_at")
                    .map_err(|_| StorageError::CorruptData)?,
            )?,
            updated_at: parse_time(
                &row.try_get::<String, _>("updated_at")
                    .map_err(|_| StorageError::CorruptData)?,
            )?,
            schema_version: row
                .try_get::<i64, _>("schema_version")
                .map_err(|_| StorageError::CorruptData)? as u32,
        })
    }

    pub async fn begin_request_run(&self, run: &NewRequestRun) -> Result<i64, StorageError> {
        if run.request_id_hash.trim().is_empty() {
            return Err(StorageError::CorruptData);
        }
        let result = sqlx::query(
            "INSERT INTO request_runs (
                request_id_hash, public_model, route_mode, selected_outer_worker_id,
                preset_name, started_at, cost_unknown
            ) VALUES (?, ?, ?, ?, ?, ?, 0)",
        )
        .bind(&run.request_id_hash)
        .bind(&run.public_model)
        .bind(&run.route_mode)
        .bind(&run.selected_outer_worker_id)
        .bind(&run.preset_name)
        .bind(format_time(run.started_at))
        .execute(&self.pool)
        .await
        .map_err(StorageError::from_sqlx)?;
        Ok(result.last_insert_rowid())
    }

    pub async fn add_run_participants(
        &self,
        run_id: i64,
        participants: &[NewRunParticipant],
    ) -> Result<(), StorageError> {
        let mut tx = self.pool.begin().await.map_err(StorageError::from_sqlx)?;
        for participant in participants {
            sqlx::query(
                "INSERT INTO run_participants (run_id, worker_id, role, stage_order)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(run_id)
            .bind(&participant.worker_id)
            .bind(participant.role.as_str())
            .bind(participant.stage_order)
            .execute(&mut *tx)
            .await
            .map_err(StorageError::from_sqlx)?;
        }
        tx.commit().await.map_err(StorageError::from_sqlx)?;
        Ok(())
    }

    pub async fn append_run_stage(
        &self,
        run_id: i64,
        stage: &NewRunStage,
    ) -> Result<i64, StorageError> {
        let result = sqlx::query(
            "INSERT INTO run_stages (
                run_id, stage_type, worker_id, attempt_number, started_at, cost_unknown
            ) VALUES (?, ?, ?, ?, ?, 0)",
        )
        .bind(run_id)
        .bind(&stage.stage_type)
        .bind(&stage.worker_id)
        .bind(stage.attempt_number)
        .bind(format_time(stage.started_at))
        .execute(&self.pool)
        .await
        .map_err(StorageError::from_sqlx)?;
        Ok(result.last_insert_rowid())
    }

    pub async fn complete_request_run(
        &self,
        run_id: i64,
        update: &CompleteRunUpdate,
    ) -> Result<(), StorageError> {
        let completed_at = format_time(Utc::now());
        let result = sqlx::query(
            "UPDATE request_runs SET
                completed_at = ?,
                latency_ms = ?,
                input_tokens = ?,
                output_tokens = ?,
                known_cost_microusd = ?,
                cost_unknown = ?,
                outcome = ?,
                error_category = ?
             WHERE id = ?",
        )
        .bind(completed_at)
        .bind(update.latency_ms)
        .bind(update.input_tokens)
        .bind(update.output_tokens)
        .bind(update.known_cost_microusd)
        .bind(i64::from(update.cost_unknown))
        .bind(&update.outcome)
        .bind(&update.error_category)
        .bind(run_id)
        .execute(&self.pool)
        .await
        .map_err(StorageError::from_sqlx)?;
        if result.rows_affected() == 0 {
            return Err(StorageError::NotFound(run_id.to_string()));
        }
        Ok(())
    }

    pub async fn get_request_run(&self, run_id: i64) -> Result<RequestRunRecord, StorageError> {
        let row = sqlx::query(
            "SELECT id, request_id_hash, public_model, route_mode, selected_outer_worker_id,
                    preset_name, started_at, completed_at, latency_ms, input_tokens,
                    output_tokens, known_cost_microusd, cost_unknown, outcome, error_category
             FROM request_runs WHERE id = ?",
        )
        .bind(run_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(StorageError::from_sqlx)?
        .ok_or_else(|| StorageError::NotFound(run_id.to_string()))?;

        Ok(RequestRunRecord {
            id: row.try_get("id").map_err(|_| StorageError::CorruptData)?,
            request_id_hash: row
                .try_get("request_id_hash")
                .map_err(|_| StorageError::CorruptData)?,
            public_model: row
                .try_get("public_model")
                .map_err(|_| StorageError::CorruptData)?,
            route_mode: row
                .try_get("route_mode")
                .map_err(|_| StorageError::CorruptData)?,
            selected_outer_worker_id: row
                .try_get("selected_outer_worker_id")
                .map_err(|_| StorageError::CorruptData)?,
            preset_name: row
                .try_get("preset_name")
                .map_err(|_| StorageError::CorruptData)?,
            started_at: parse_time(
                &row.try_get::<String, _>("started_at")
                    .map_err(|_| StorageError::CorruptData)?,
            )?,
            completed_at: row
                .try_get::<Option<String>, _>("completed_at")
                .map_err(|_| StorageError::CorruptData)?
                .map(|value| parse_time(&value))
                .transpose()?,
            latency_ms: row
                .try_get("latency_ms")
                .map_err(|_| StorageError::CorruptData)?,
            input_tokens: row
                .try_get("input_tokens")
                .map_err(|_| StorageError::CorruptData)?,
            output_tokens: row
                .try_get("output_tokens")
                .map_err(|_| StorageError::CorruptData)?,
            known_cost_microusd: row
                .try_get("known_cost_microusd")
                .map_err(|_| StorageError::CorruptData)?,
            cost_unknown: row
                .try_get::<i64, _>("cost_unknown")
                .map_err(|_| StorageError::CorruptData)?
                != 0,
            outcome: row
                .try_get("outcome")
                .map_err(|_| StorageError::CorruptData)?,
            error_category: row
                .try_get("error_category")
                .map_err(|_| StorageError::CorruptData)?,
        })
    }

    pub async fn list_run_stages(&self, run_id: i64) -> Result<Vec<RunStageRecord>, StorageError> {
        let rows = sqlx::query(
            "SELECT id, run_id, stage_type, worker_id, attempt_number, started_at,
                    completed_at, latency_ms, input_tokens, output_tokens,
                    known_cost_microusd, cost_unknown, outcome, error_category
             FROM run_stages WHERE run_id = ? ORDER BY id ASC",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .map_err(StorageError::from_sqlx)?;

        let mut stages = Vec::with_capacity(rows.len());
        for row in rows {
            stages.push(RunStageRecord {
                id: row.try_get("id").map_err(|_| StorageError::CorruptData)?,
                run_id: row
                    .try_get("run_id")
                    .map_err(|_| StorageError::CorruptData)?,
                stage_type: row
                    .try_get("stage_type")
                    .map_err(|_| StorageError::CorruptData)?,
                worker_id: row
                    .try_get("worker_id")
                    .map_err(|_| StorageError::CorruptData)?,
                attempt_number: row
                    .try_get("attempt_number")
                    .map_err(|_| StorageError::CorruptData)?,
                started_at: parse_time(
                    &row.try_get::<String, _>("started_at")
                        .map_err(|_| StorageError::CorruptData)?,
                )?,
                completed_at: row
                    .try_get::<Option<String>, _>("completed_at")
                    .map_err(|_| StorageError::CorruptData)?
                    .map(|value| parse_time(&value))
                    .transpose()?,
                latency_ms: row
                    .try_get("latency_ms")
                    .map_err(|_| StorageError::CorruptData)?,
                input_tokens: row
                    .try_get("input_tokens")
                    .map_err(|_| StorageError::CorruptData)?,
                output_tokens: row
                    .try_get("output_tokens")
                    .map_err(|_| StorageError::CorruptData)?,
                known_cost_microusd: row
                    .try_get("known_cost_microusd")
                    .map_err(|_| StorageError::CorruptData)?,
                cost_unknown: row
                    .try_get::<i64, _>("cost_unknown")
                    .map_err(|_| StorageError::CorruptData)?
                    != 0,
                outcome: row
                    .try_get("outcome")
                    .map_err(|_| StorageError::CorruptData)?,
                error_category: row
                    .try_get("error_category")
                    .map_err(|_| StorageError::CorruptData)?,
            });
        }
        Ok(stages)
    }

    pub async fn reserve_daily_budget(
        &self,
        utc_date: &str,
        amount_microusd: u64,
    ) -> Result<DailySpendRecord, StorageError> {
        let now = format_time(Utc::now());
        let mut tx = self.pool.begin().await.map_err(StorageError::from_sqlx)?;
        sqlx::query(
            "INSERT INTO daily_spend (utc_date, reserved_microusd, settled_microusd, updated_at)
             VALUES (?, ?, 0, ?)
             ON CONFLICT(utc_date) DO UPDATE SET
                reserved_microusd = reserved_microusd + excluded.reserved_microusd,
                updated_at = excluded.updated_at",
        )
        .bind(utc_date)
        .bind(to_sqlite_i64(amount_microusd)?)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;

        let row = sqlx::query(
            "SELECT utc_date, reserved_microusd, settled_microusd, updated_at
             FROM daily_spend WHERE utc_date = ?",
        )
        .bind(utc_date)
        .fetch_one(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;
        tx.commit().await.map_err(StorageError::from_sqlx)?;
        daily_spend_from_row(&row)
    }

    pub async fn settle_daily_budget(
        &self,
        utc_date: &str,
        release_reserved_microusd: u64,
        settle_microusd: u64,
    ) -> Result<DailySpendRecord, StorageError> {
        let now = format_time(Utc::now());
        let mut tx = self.pool.begin().await.map_err(StorageError::from_sqlx)?;
        let existing = sqlx::query(
            "SELECT reserved_microusd, settled_microusd FROM daily_spend WHERE utc_date = ?",
        )
        .bind(utc_date)
        .fetch_optional(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?
        .ok_or_else(|| StorageError::NotFound(utc_date.to_owned()))?;

        let reserved: i64 = existing
            .try_get("reserved_microusd")
            .map_err(|_| StorageError::CorruptData)?;
        let settled: i64 = existing
            .try_get("settled_microusd")
            .map_err(|_| StorageError::CorruptData)?;
        let release = to_sqlite_i64(release_reserved_microusd)?;
        if release > reserved {
            return Err(StorageError::Conflict(
                "cannot release more than reserved".to_owned(),
            ));
        }

        let settled_increment = to_sqlite_i64(settle_microusd)?;
        let new_settled = settled
            .checked_add(settled_increment)
            .ok_or(StorageError::ValueOutOfRange)?;

        sqlx::query(
            "UPDATE daily_spend SET
                reserved_microusd = ?,
                settled_microusd = ?,
                updated_at = ?
             WHERE utc_date = ?",
        )
        .bind(reserved - release)
        .bind(new_settled)
        .bind(&now)
        .bind(utc_date)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;

        let row = sqlx::query(
            "SELECT utc_date, reserved_microusd, settled_microusd, updated_at
             FROM daily_spend WHERE utc_date = ?",
        )
        .bind(utc_date)
        .fetch_one(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;
        tx.commit().await.map_err(StorageError::from_sqlx)?;
        daily_spend_from_row(&row)
    }

    pub async fn reconcile_orphaned_reservations(&self) -> Result<u64, StorageError> {
        let incomplete: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM request_runs WHERE completed_at IS NULL")
                .fetch_one(&self.pool)
                .await
                .map_err(StorageError::from_sqlx)?;
        if incomplete.0 > 0 {
            return Ok(0);
        }

        let now = format_time(Utc::now());
        let result = sqlx::query(
            "UPDATE daily_spend
             SET reserved_microusd = 0, updated_at = ?
             WHERE reserved_microusd > 0",
        )
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(StorageError::from_sqlx)?;
        Ok(result.rows_affected())
    }

    pub async fn table_column_names(&self, table: &str) -> Result<Vec<String>, StorageError> {
        let query = match table {
            "workers" => "PRAGMA table_info(workers)",
            "fusion_presets" => "PRAGMA table_info(fusion_presets)",
            "gateway" => "PRAGMA table_info(gateway)",
            "request_runs" => "PRAGMA table_info(request_runs)",
            "run_participants" => "PRAGMA table_info(run_participants)",
            "run_stages" => "PRAGMA table_info(run_stages)",
            "daily_spend" => "PRAGMA table_info(daily_spend)",
            "schema_migrations" => "PRAGMA table_info(schema_migrations)",
            _ => return Err(StorageError::InvalidTable),
        };
        let rows = sqlx::query(query)
            .fetch_all(&self.pool)
            .await
            .map_err(StorageError::from_sqlx)?;
        let mut names = Vec::new();
        for row in rows {
            names.push(
                row.try_get::<String, _>("name")
                    .map_err(|_| StorageError::CorruptData)?,
            );
        }
        Ok(names)
    }

    pub fn supported_db_version() -> i64 {
        CURRENT_DB_VERSION
    }
}

fn to_sqlite_i64(value: u64) -> Result<i64, StorageError> {
    i64::try_from(value).map_err(|_| StorageError::ValueOutOfRange)
}

fn map_worker_row(row: &SqliteRow) -> Result<PersistedWorker, StorageError> {
    worker_from_row(WorkerRow {
        id: row.try_get("id").map_err(|_| StorageError::CorruptData)?,
        display_name: row
            .try_get("display_name")
            .map_err(|_| StorageError::CorruptData)?,
        base_url: row
            .try_get("base_url")
            .map_err(|_| StorageError::CorruptData)?,
        model_id: row
            .try_get("model_id")
            .map_err(|_| StorageError::CorruptData)?,
        input_price_microusd: row
            .try_get("input_price_microusd")
            .map_err(|_| StorageError::CorruptData)?,
        output_price_microusd: row
            .try_get("output_price_microusd")
            .map_err(|_| StorageError::CorruptData)?,
        cached_input_price_microusd: row
            .try_get("cached_input_price_microusd")
            .map_err(|_| StorageError::CorruptData)?,
        context_window_tokens: row
            .try_get("context_window_tokens")
            .map_err(|_| StorageError::CorruptData)?,
        capabilities: row
            .try_get("capabilities")
            .map_err(|_| StorageError::CorruptData)?,
        supports_streaming: row
            .try_get("supports_streaming")
            .map_err(|_| StorageError::CorruptData)?,
        provider_policy_url: row
            .try_get("provider_policy_url")
            .map_err(|_| StorageError::CorruptData)?,
        secret_ref: row
            .try_get("secret_ref")
            .map_err(|_| StorageError::CorruptData)?,
        compatibility_profile: row
            .try_get("compatibility_profile")
            .map_err(|_| StorageError::CorruptData)?,
        enabled: row
            .try_get("enabled")
            .map_err(|_| StorageError::CorruptData)?,
        health_status: row
            .try_get("health_status")
            .map_err(|_| StorageError::CorruptData)?,
        created_at: row
            .try_get("created_at")
            .map_err(|_| StorageError::CorruptData)?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|_| StorageError::CorruptData)?,
        schema_version: row
            .try_get("schema_version")
            .map_err(|_| StorageError::CorruptData)?,
    })
}

fn map_preset_row(row: &SqliteRow) -> Result<PersistedFusionPreset, StorageError> {
    preset_from_row(PresetRow {
        name: row.try_get("name").map_err(|_| StorageError::CorruptData)?,
        quality_tier: row
            .try_get("quality_tier")
            .map_err(|_| StorageError::CorruptData)?,
        outer_worker_policy: row
            .try_get("outer_worker_policy")
            .map_err(|_| StorageError::CorruptData)?,
        advisor_worker_ids: row
            .try_get("advisor_worker_ids")
            .map_err(|_| StorageError::CorruptData)?,
        judge_worker_id: row
            .try_get("judge_worker_id")
            .map_err(|_| StorageError::CorruptData)?,
        max_completion_tokens: row
            .try_get("max_completion_tokens")
            .map_err(|_| StorageError::CorruptData)?,
        task_budget_microusd: row
            .try_get("task_budget_microusd")
            .map_err(|_| StorageError::CorruptData)?,
        daily_budget_microusd: row
            .try_get("daily_budget_microusd")
            .map_err(|_| StorageError::CorruptData)?,
        enabled: row
            .try_get("enabled")
            .map_err(|_| StorageError::CorruptData)?,
        created_at: row
            .try_get("created_at")
            .map_err(|_| StorageError::CorruptData)?,
        updated_at: row
            .try_get("updated_at")
            .map_err(|_| StorageError::CorruptData)?,
        schema_version: row
            .try_get("schema_version")
            .map_err(|_| StorageError::CorruptData)?,
    })
}

fn daily_spend_from_row(row: &SqliteRow) -> Result<DailySpendRecord, StorageError> {
    Ok(DailySpendRecord {
        utc_date: row
            .try_get("utc_date")
            .map_err(|_| StorageError::CorruptData)?,
        reserved_microusd: row
            .try_get::<i64, _>("reserved_microusd")
            .map_err(|_| StorageError::CorruptData)? as u64,
        settled_microusd: row
            .try_get::<i64, _>("settled_microusd")
            .map_err(|_| StorageError::CorruptData)? as u64,
        updated_at: parse_time(
            &row.try_get::<String, _>("updated_at")
                .map_err(|_| StorageError::CorruptData)?,
        )?,
    })
}
