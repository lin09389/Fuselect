use crate::domain::{ConfigError, FusionPolicy, MAX_WORKERS, WorkerConfig};

/// The schema stored in local metadata. API secrets are held in OS Keyring only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuselectConfig {
    pub schema_version: u32,
    pub gateway: GatewayConfig,
    pub fusion_policy: FusionPolicy,
    pub workers: Vec<WorkerConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayConfig {
    pub port: u16,
    pub metadata_retention_days: u16,
    /// Keyring entry name for the Gateway Key — never the key value.
    pub gateway_key_ref: String,
    pub durable_session_enabled: bool,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            port: 8787,
            metadata_retention_days: 30,
            gateway_key_ref: "fuselect/gateway/default".to_owned(),
            durable_session_enabled: false,
        }
    }
}

impl FuselectConfig {
    pub const CURRENT_SCHEMA_VERSION: u32 = 1;

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != Self::CURRENT_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedSchemaVersion {
                found: self.schema_version,
                supported: Self::CURRENT_SCHEMA_VERSION,
            });
        }

        if self.workers.len() > MAX_WORKERS {
            return Err(ConfigError::TooManyWorkers {
                maximum: MAX_WORKERS,
                found: self.workers.len(),
            });
        }

        if self.gateway.port == 0 {
            return Err(ConfigError::InvalidGatewayPort);
        }
        if self.gateway.gateway_key_ref.trim().is_empty() {
            return Err(ConfigError::EmptyField("gateway_key_ref"));
        }
        if self.gateway.metadata_retention_days == 0 {
            return Err(ConfigError::EmptyField("metadata_retention_days"));
        }

        let mut ids = std::collections::BTreeSet::new();
        for worker in &self.workers {
            worker.validate()?;
            if !ids.insert(worker.id.as_str()) {
                return Err(ConfigError::DuplicateWorkerId(worker.id.clone()));
            }
        }

        self.fusion_policy.validate(&self.workers)
    }
}
