use chrono::{DateTime, Utc};

use crate::config::GatewayConfig;
use crate::domain::{
    BudgetLimits, Capability, FusionPreset, PricePerMillion, QualityTier, WorkerCapabilities,
    WorkerConfig, WorkerHealthStatus,
};

use super::StorageError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedWorker {
    pub config: WorkerConfig,
    pub health_status: WorkerHealthStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub schema_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedGateway {
    pub config: GatewayConfig,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub schema_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedFusionPreset {
    pub preset: FusionPreset,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub schema_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRequestRun {
    pub request_id_hash: String,
    pub public_model: String,
    pub route_mode: String,
    pub selected_outer_worker_id: Option<String>,
    pub preset_name: Option<String>,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestRunRecord {
    pub id: i64,
    pub request_id_hash: String,
    pub public_model: String,
    pub route_mode: String,
    pub selected_outer_worker_id: Option<String>,
    pub preset_name: Option<String>,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub latency_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub known_cost_microusd: Option<i64>,
    pub cost_unknown: bool,
    pub outcome: Option<String>,
    pub error_category: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunParticipantRole {
    Outer,
    Advisor,
    Judge,
}

impl RunParticipantRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Outer => "outer",
            Self::Advisor => "advisor",
            Self::Judge => "judge",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "outer" => Some(Self::Outer),
            "advisor" => Some(Self::Advisor),
            "judge" => Some(Self::Judge),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRunParticipant {
    pub worker_id: String,
    pub role: RunParticipantRole,
    pub stage_order: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRunStage {
    pub stage_type: String,
    pub worker_id: Option<String>,
    pub attempt_number: i64,
    pub started_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompleteRunUpdate {
    pub latency_ms: i64,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub known_cost_microusd: Option<i64>,
    pub cost_unknown: bool,
    pub outcome: String,
    pub error_category: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunStageRecord {
    pub id: i64,
    pub run_id: i64,
    pub stage_type: String,
    pub worker_id: Option<String>,
    pub attempt_number: i64,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub latency_ms: Option<i64>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub known_cost_microusd: Option<i64>,
    pub cost_unknown: bool,
    pub outcome: Option<String>,
    pub error_category: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailySpendRecord {
    pub utc_date: String,
    pub reserved_microusd: u64,
    pub settled_microusd: u64,
    pub updated_at: DateTime<Utc>,
}

pub(crate) fn encode_capabilities(
    capabilities: &WorkerCapabilities,
) -> Result<String, StorageError> {
    let tags: Vec<&str> = capabilities.tags.iter().map(Capability::as_str).collect();
    serde_json::to_string(&tags).map_err(|_| StorageError::CorruptData)
}

pub(crate) fn decode_capabilities(
    raw: &str,
    supports_streaming: bool,
) -> Result<WorkerCapabilities, StorageError> {
    let tags: Vec<String> = serde_json::from_str(raw).map_err(|_| StorageError::CorruptData)?;
    let mut parsed = std::collections::BTreeSet::new();
    for tag in tags {
        let capability = Capability::parse(&tag).ok_or(StorageError::CorruptData)?;
        parsed.insert(capability);
    }
    Ok(WorkerCapabilities {
        tags: parsed,
        supports_streaming,
    })
}

pub(crate) fn encode_advisor_ids(ids: &[String]) -> Result<String, StorageError> {
    serde_json::to_string(ids).map_err(|_| StorageError::CorruptData)
}

pub(crate) fn decode_advisor_ids(raw: &str) -> Result<Vec<String>, StorageError> {
    serde_json::from_str(raw).map_err(|_| StorageError::CorruptData)
}

pub(crate) struct WorkerRow {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub model_id: String,
    pub input_price_microusd: i64,
    pub output_price_microusd: i64,
    pub cached_input_price_microusd: Option<i64>,
    pub context_window_tokens: i64,
    pub capabilities: String,
    pub supports_streaming: i64,
    pub provider_policy_url: String,
    pub secret_ref: String,
    pub compatibility_profile: String,
    pub enabled: i64,
    pub health_status: String,
    pub created_at: String,
    pub updated_at: String,
    pub schema_version: i64,
}

pub(crate) struct PresetRow {
    pub name: String,
    pub quality_tier: String,
    pub outer_worker_policy: String,
    pub advisor_worker_ids: String,
    pub judge_worker_id: String,
    pub max_completion_tokens: i64,
    pub task_budget_microusd: i64,
    pub daily_budget_microusd: i64,
    pub enabled: i64,
    pub created_at: String,
    pub updated_at: String,
    pub schema_version: i64,
}

pub(crate) fn worker_from_row(row: WorkerRow) -> Result<PersistedWorker, StorageError> {
    let supports_streaming = row.supports_streaming != 0;
    Ok(PersistedWorker {
        config: WorkerConfig {
            id: row.id,
            display_name: row.display_name,
            base_url: row.base_url,
            model_id: row.model_id,
            pricing: PricePerMillion {
                input_microusd: cast_u64(row.input_price_microusd)?,
                output_microusd: cast_u64(row.output_price_microusd)?,
                cached_input_microusd: row.cached_input_price_microusd.map(cast_u64).transpose()?,
            },
            capabilities: decode_capabilities(&row.capabilities, supports_streaming)?,
            context_window_tokens: cast_u32(row.context_window_tokens)?,
            provider_policy_url: row.provider_policy_url,
            secret_ref: row.secret_ref,
            compatibility_profile: row.compatibility_profile,
            enabled: row.enabled != 0,
        },
        health_status: WorkerHealthStatus::parse(&row.health_status)
            .ok_or(StorageError::CorruptData)?,
        created_at: parse_time(&row.created_at)?,
        updated_at: parse_time(&row.updated_at)?,
        schema_version: cast_u32(row.schema_version)?,
    })
}

pub(crate) fn preset_from_row(row: PresetRow) -> Result<PersistedFusionPreset, StorageError> {
    Ok(PersistedFusionPreset {
        preset: FusionPreset {
            name: row.name,
            quality_tier: QualityTier::parse(&row.quality_tier).ok_or(StorageError::CorruptData)?,
            outer_worker_policy: row.outer_worker_policy,
            advisor_worker_ids: decode_advisor_ids(&row.advisor_worker_ids)?,
            judge_worker_id: row.judge_worker_id,
            max_completion_tokens: cast_u32(row.max_completion_tokens)?,
            budgets: BudgetLimits {
                per_task_microusd: cast_u64(row.task_budget_microusd)?,
                daily_microusd: cast_u64(row.daily_budget_microusd)?,
            },
            enabled: row.enabled != 0,
        },
        created_at: parse_time(&row.created_at)?,
        updated_at: parse_time(&row.updated_at)?,
        schema_version: cast_u32(row.schema_version)?,
    })
}

pub(crate) fn parse_time(value: &str) -> Result<DateTime<Utc>, StorageError> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|_| StorageError::CorruptData)
}

pub(crate) fn format_time(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn cast_u64(value: i64) -> Result<u64, StorageError> {
    u64::try_from(value).map_err(|_| StorageError::CorruptData)
}

fn cast_u32(value: i64) -> Result<u32, StorageError> {
    u32::try_from(value).map_err(|_| StorageError::CorruptData)
}
