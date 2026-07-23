//! Testable local configuration application services.  This module never
//! constructs a platform keyring; callers inject that boundary explicitly.

use std::collections::BTreeSet;
use std::sync::Arc;

use base64::Engine as _;
use getrandom::fill;
use serde_json::{Value, json};

use crate::config::GatewayConfig;
use crate::domain::{
    BudgetLimits, Capability, FusionPreset, PricePerMillion, QualityTier, WorkerCapabilities,
    WorkerConfig,
};
use crate::secrets::{SecretError, SecretRef, SecretStore, SecretString};
use crate::storage::{PersistedFusionPreset, PersistedWorker, SqliteStore, StorageError};

#[derive(Debug, Clone, Copy)]
pub enum OutputMode {
    Human,
    Json,
}

pub struct AppContext {
    pub store: SqliteStore,
    pub secrets: Arc<dyn SecretStore>,
    pub output_mode: OutputMode,
}

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Validation(String),
    #[error("记录不存在：{0}")]
    NotFound(String),
    #[error("记录冲突：{0}")]
    Conflict(String),
    #[error("系统密钥库操作失败")]
    SecretStore,
    #[error("本地存储操作失败")]
    Storage,
    #[error("非交互环境缺少必填输入：{0}")]
    NonInteractiveInputRequired(String),
    #[error("此操作需要明确确认；请在终端确认或传入 --yes")]
    ConfirmationRequired,
    #[error("当前构建尚未启用 Provider 网络兼容性探测")]
    NetworkProbeUnavailable,
    #[error("密钥清理未完成；请重试清理")]
    CleanupPending,
    #[error("内部错误")]
    Internal,
}

impl AppError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::SecretStore | Self::CleanupPending => 3,
            Self::NetworkProbeUnavailable => 4,
            _ => 2,
        }
    }
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Validation(_) => "validation",
            Self::NotFound(_) => "not_found",
            Self::Conflict(_) => "conflict",
            Self::SecretStore => "secret_store",
            Self::Storage => "storage",
            Self::NonInteractiveInputRequired(_) => "non_interactive_input_required",
            Self::ConfirmationRequired => "confirmation_required",
            Self::NetworkProbeUnavailable => "network_probe_unavailable",
            Self::CleanupPending => "cleanup_pending",
            Self::Internal => "internal",
        }
    }
}

impl From<SecretError> for AppError {
    fn from(value: SecretError) -> Self {
        match value {
            SecretError::NotFound => Self::NotFound("密钥".into()),
            SecretError::CleanupPending => Self::CleanupPending,
            _ => Self::SecretStore,
        }
    }
}
impl From<StorageError> for AppError {
    fn from(value: StorageError) -> Self {
        match value {
            StorageError::NotFound(v) => Self::NotFound(v),
            StorageError::Conflict(v) => Self::Conflict(v),
            StorageError::InvalidConfig(v) => Self::Validation(v.to_string()),
            StorageError::KeyringCleanupPending(_) => Self::CleanupPending,
            _ => Self::Storage,
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkerInput {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub input_price_microusd: u64,
    pub output_price_microusd: u64,
    pub cached_input_price_microusd: Option<u64>,
    pub context_window: u32,
    pub capabilities: Vec<String>,
    pub provider_policy_url: String,
    pub compatibility_profile: String,
    pub secret_ref: Option<String>,
}
#[derive(Debug, Clone)]
pub struct PresetInput {
    pub name: String,
    pub quality_tier: String,
    pub outer_worker_policy: String,
    pub advisors: Vec<String>,
    pub judge: String,
    pub max_completion_tokens: u32,
    pub task_budget_microusd: u64,
    pub daily_budget_microusd: u64,
}

impl AppContext {
    pub async fn init(&self) -> Result<Value, AppError> {
        let reference = SecretRef::gateway_default();
        let existing_config = match self.store.get_gateway_config().await {
            Ok(config) => Some(config),
            Err(StorageError::NotFound(_)) => None,
            Err(error) => return Err(error.into()),
        };
        let existed = self.secrets.exists(&reference)?;
        let mut created = false;
        if !existed {
            let mut bytes = [0u8; 32];
            fill(&mut bytes).map_err(|_| AppError::Internal)?;
            let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
            self.secrets.set(&reference, SecretString::from(token))?;
            created = true;
        }
        if existing_config.is_none() {
            if let Err(error) = self
                .store
                .save_gateway_config(&GatewayConfig::default())
                .await
            {
                if created && self.secrets.delete(&reference).is_err() {
                    return Err(AppError::CleanupPending);
                }
                return Err(error.into());
            }
        }
        Ok(
            json!({"status":"initialized", "database_version":self.store.database_version().await?, "gateway_key_configured":true, "next_command":"fuselect worker add"}),
        )
    }

    pub async fn add_worker(
        &self,
        input: WorkerInput,
        new_secret: Option<SecretString>,
    ) -> Result<Value, AppError> {
        let tags = input
            .capabilities
            .iter()
            .map(|tag| {
                Capability::parse(tag)
                    .ok_or_else(|| AppError::Validation(format!("未知 capability：{tag}")))
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        let reference = match input.secret_ref {
            Some(value) => SecretRef::new(value)?,
            None => SecretRef::worker(&input.id)?,
        };
        let worker = WorkerConfig {
            id: input.id,
            display_name: input.name,
            base_url: input.base_url,
            model_id: input.model,
            pricing: PricePerMillion {
                input_microusd: input.input_price_microusd,
                output_microusd: input.output_price_microusd,
                cached_input_microusd: input.cached_input_price_microusd,
            },
            capabilities: WorkerCapabilities {
                tags,
                supports_streaming: true,
            },
            context_window_tokens: input.context_window,
            provider_policy_url: input.provider_policy_url,
            secret_ref: reference.as_str().to_owned(),
            compatibility_profile: input.compatibility_profile,
            enabled: false,
        };
        worker
            .validate()
            .map_err(|v| AppError::Validation(v.to_string()))?;
        match self.store.get_worker(&worker.id).await {
            Ok(_) => return Err(AppError::Conflict(worker.id)),
            Err(StorageError::NotFound(_)) => {}
            Err(error) => return Err(error.into()),
        }
        let created = if let Some(secret) = new_secret {
            if self.secrets.exists(&reference)? {
                return Err(AppError::Conflict("密钥引用已存在".into()));
            }
            self.secrets.set(&reference, secret)?;
            true
        } else if !self.secrets.exists(&reference)? {
            return Err(AppError::NotFound("指定的密钥引用".into()));
        } else {
            false
        };
        if let Err(error) = self.store.save_worker(&worker).await {
            if created && self.secrets.delete(&reference).is_err() {
                return Err(AppError::CleanupPending);
            }
            return Err(error.into());
        }
        Ok(
            json!({"status":"added", "id":worker.id, "api_key_configured":true, "next_command":format!("fuselect worker test {}", worker.id)}),
        )
    }

    pub async fn workers(&self) -> Result<Value, AppError> {
        Ok(Value::Array(
            self.store
                .list_workers()
                .await?
                .iter()
                .map(|w| self.worker_json(w))
                .collect(),
        ))
    }
    pub async fn worker(&self, id: &str) -> Result<Value, AppError> {
        Ok(self.worker_json(&self.store.get_worker(id).await?))
    }
    fn worker_json(&self, worker: &PersistedWorker) -> Value {
        let state = SecretRef::new(worker.config.secret_ref.clone())
            .ok()
            .and_then(|r| self.secrets.exists(&r).ok())
            .map(|yes| if yes { "configured" } else { "missing" })
            .unwrap_or("error");
        json!({"id":worker.config.id,"display_name":worker.config.display_name,"base_url":worker.config.base_url,"model":worker.config.model_id,"enabled":worker.config.enabled,"health_status":worker.health_status.as_str(),"compatibility_profile":worker.config.compatibility_profile,"secret_status":state,"capabilities":worker.config.capabilities.tags.iter().map(Capability::as_str).collect::<Vec<_>>(),"supports_streaming":true,"input_price_microusd":worker.config.pricing.input_microusd,"output_price_microusd":worker.config.pricing.output_microusd,"cached_input_price_microusd":worker.config.pricing.cached_input_microusd,"context_window":worker.config.context_window_tokens,"provider_policy_url":worker.config.provider_policy_url})
    }
    pub async fn remove_worker(&self, id: &str) -> Result<Value, AppError> {
        let linked: Vec<String> = self
            .store
            .list_fusion_presets()
            .await?
            .into_iter()
            .filter(|p| {
                p.preset.judge_worker_id == id
                    || p.preset.advisor_worker_ids.iter().any(|a| a == id)
            })
            .map(|p| p.preset.name)
            .collect();
        if !linked.is_empty() {
            return Err(AppError::Conflict(format!(
                "Worker 被预设引用：{}",
                linked.join(", ")
            )));
        }
        self.store.remove_worker(self.secrets.as_ref(), id).await?;
        Ok(json!({"status":"removed","id":id}))
    }
    pub async fn worker_test(&self, id: &str) -> Result<Value, AppError> {
        self.store.get_worker(id).await?;
        Err(AppError::NetworkProbeUnavailable)
    }
    pub async fn add_preset(&self, input: PresetInput) -> Result<Value, AppError> {
        let tier = QualityTier::parse(&input.quality_tier)
            .ok_or_else(|| AppError::Validation("quality-tier 必须为 budget 或 high".into()))?;
        let preset = FusionPreset {
            name: input.name,
            quality_tier: tier,
            outer_worker_policy: input.outer_worker_policy,
            advisor_worker_ids: input.advisors,
            judge_worker_id: input.judge,
            max_completion_tokens: input.max_completion_tokens,
            budgets: BudgetLimits {
                per_task_microusd: input.task_budget_microusd,
                daily_microusd: input.daily_budget_microusd,
            },
            enabled: true,
        };
        let workers = self
            .store
            .list_workers()
            .await?
            .into_iter()
            .map(|w| w.config)
            .collect::<Vec<_>>();
        preset
            .validate(&workers)
            .map_err(|e| AppError::Validation(e.to_string()))?;
        self.store.save_fusion_preset(&preset).await?;
        Ok(
            json!({"status":"added","name":preset.name,"maximum_output_cost_note":"不包含请求输入 Token；实际执行前仍会进行完整预算预留。"}),
        )
    }
    pub async fn presets(&self) -> Result<Value, AppError> {
        let configured = self.store.list_fusion_presets().await?;
        let configured_names: BTreeSet<&str> = configured
            .iter()
            .map(|value| value.preset.name.as_str())
            .collect();
        let mut output: Vec<Value> = configured.iter().map(Self::preset_json).collect();
        for name in ["coding-budget", "coding-high"] {
            if !configured_names.contains(name) {
                output.push(Self::builtin_template_json(name).expect("known built-in template"));
            }
        }
        Ok(Value::Array(output))
    }
    pub async fn preset(&self, name: &str) -> Result<Value, AppError> {
        match self.store.get_fusion_preset(name).await {
            Ok(value) => Ok(Self::preset_json(&value)),
            Err(StorageError::NotFound(_)) => {
                Self::builtin_template_json(name).ok_or_else(|| AppError::NotFound(name.to_owned()))
            }
            Err(error) => Err(error.into()),
        }
    }
    fn preset_json(value: &PersistedFusionPreset) -> Value {
        let p = &value.preset;
        json!({"name":p.name,"quality_tier":p.quality_tier.as_str(),"outer_worker_policy":p.outer_worker_policy,"advisors":p.advisor_worker_ids,"judge":p.judge_worker_id,"max_completion_tokens":p.max_completion_tokens,"task_budget_microusd":p.budgets.per_task_microusd,"daily_budget_microusd":p.budgets.daily_microusd,"enabled":p.enabled,"maximum_output_cost_note":"不包含请求输入 Token；实际执行前仍会进行完整预算预留。"})
    }
    fn builtin_template_json(name: &str) -> Option<Value> {
        let (quality_tier, outer_worker_policy) = match name {
            "coding-budget" => ("budget", "quality-first-cost-second"),
            "coding-high" => ("high", "quality-first"),
            _ => return None,
        };
        Some(json!({
            "name": name,
            "status": "template_unavailable",
            "quality_tier": quality_tier,
            "outer_worker_policy": outer_worker_policy,
            "advisor_range": {"minimum": 1, "maximum": 8},
            "reason": "内置策略模板需要本机 Worker 角色绑定，不会伪造 Worker ID。请添加 Worker 后创建同名预设。"
        }))
    }
    pub async fn remove_preset(&self, name: &str) -> Result<Value, AppError> {
        self.store.delete_fusion_preset(name).await?;
        Ok(json!({"status":"removed","name":name}))
    }
}
