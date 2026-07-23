use super::ConfigError;

/// USD micro-units per one million tokens. Integer accounting avoids float drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PricePerMillion {
    pub input_microusd: u64,
    pub output_microusd: u64,
    pub cached_input_microusd: Option<u64>,
}

impl PricePerMillion {
    pub fn validate(self) -> Result<(), ConfigError> {
        if self.input_microusd == 0 || self.output_microusd == 0 {
            return Err(ConfigError::InvalidPrice);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetLimits {
    pub per_task_microusd: u64,
    pub daily_microusd: u64,
}

impl BudgetLimits {
    pub fn validate(self) -> Result<(), ConfigError> {
        if self.per_task_microusd == 0 || self.daily_microusd == 0 {
            return Err(ConfigError::InvalidPrice);
        }
        Ok(())
    }
}
