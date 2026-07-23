use super::{BudgetLimits, ConfigError, WorkerConfig};

pub const MIN_ADVISORS: usize = 1;
pub const MAX_ADVISORS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityTier {
    Budget,
    High,
}

impl QualityTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Budget => "budget",
            Self::High => "high",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "budget" => Some(Self::Budget),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FusionPolicy {
    pub default_tier: QualityTier,
    pub budgets: BudgetLimits,
}

impl FusionPolicy {
    pub fn validate(&self, workers: &[WorkerConfig]) -> Result<(), ConfigError> {
        self.budgets.validate()?;
        for worker in workers {
            worker.validate()?;
        }
        Ok(())
    }
}

/// Local Fusion preset: advisor/Judge Worker IDs only — never prompts or model output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FusionPreset {
    pub name: String,
    pub quality_tier: QualityTier,
    pub outer_worker_policy: String,
    pub advisor_worker_ids: Vec<String>,
    pub judge_worker_id: String,
    pub max_completion_tokens: u32,
    pub budgets: BudgetLimits,
    pub enabled: bool,
}

impl FusionPreset {
    pub fn validate(&self, workers: &[WorkerConfig]) -> Result<(), ConfigError> {
        require_preset_name(&self.name)?;
        require_non_empty(&self.outer_worker_policy, "outer_worker_policy")?;
        self.budgets.validate()?;

        if self.max_completion_tokens == 0 {
            return Err(ConfigError::EmptyField("max_completion_tokens"));
        }

        let advisor_count = self.advisor_worker_ids.len();
        if !(MIN_ADVISORS..=MAX_ADVISORS).contains(&advisor_count) {
            return Err(ConfigError::InvalidAdvisorCount {
                minimum: MIN_ADVISORS,
                maximum: MAX_ADVISORS,
                found: advisor_count,
            });
        }

        for advisor_id in &self.advisor_worker_ids {
            require_known_worker(workers, advisor_id)?;
        }
        require_known_worker(workers, &self.judge_worker_id)?;
        Ok(())
    }
}

fn require_preset_name(name: &str) -> Result<(), ConfigError> {
    if name.is_empty()
        || !name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
    {
        return Err(ConfigError::InvalidIdentifier(name.to_owned()));
    }
    Ok(())
}

fn require_non_empty(value: &str, field: &'static str) -> Result<(), ConfigError> {
    if value.trim().is_empty() {
        return Err(ConfigError::EmptyField(field));
    }
    Ok(())
}

fn require_known_worker(workers: &[WorkerConfig], id: &str) -> Result<(), ConfigError> {
    if workers.iter().any(|worker| worker.id == id) {
        Ok(())
    } else {
        Err(ConfigError::MissingWorker(id.to_owned()))
    }
}
