use std::collections::BTreeSet;

use super::{ConfigError, PricePerMillion};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Capability {
    Coding,
    Reasoning,
    Review,
    Debug,
    LongContext,
    Tools,
    Fast,
    LowCost,
}

impl Capability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Coding => "coding",
            Self::Reasoning => "reasoning",
            Self::Review => "review",
            Self::Debug => "debug",
            Self::LongContext => "long_context",
            Self::Tools => "tools",
            Self::Fast => "fast",
            Self::LowCost => "low_cost",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "coding" => Some(Self::Coding),
            "reasoning" => Some(Self::Reasoning),
            "review" => Some(Self::Review),
            "debug" => Some(Self::Debug),
            "long_context" => Some(Self::LongContext),
            "tools" => Some(Self::Tools),
            "fast" => Some(Self::Fast),
            "low_cost" => Some(Self::LowCost),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerHealthStatus {
    Unknown,
    Healthy,
    Degraded,
    OpenCircuit,
}

impl WorkerHealthStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::OpenCircuit => "open_circuit",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "unknown" => Some(Self::Unknown),
            "healthy" => Some(Self::Healthy),
            "degraded" => Some(Self::Degraded),
            "open_circuit" => Some(Self::OpenCircuit),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerCapabilities {
    pub tags: BTreeSet<Capability>,
    pub supports_streaming: bool,
}

impl WorkerCapabilities {
    pub fn has(&self, capability: Capability) -> bool {
        self.tags.contains(&capability)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerConfig {
    pub id: String,
    pub display_name: String,
    pub base_url: String,
    pub model_id: String,
    pub pricing: PricePerMillion,
    pub capabilities: WorkerCapabilities,
    pub context_window_tokens: u32,
    pub provider_policy_url: String,
    /// Keyring entry name, never an API key.
    pub secret_ref: String,
    pub compatibility_profile: String,
    pub enabled: bool,
}

impl WorkerConfig {
    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_id(&self.id)?;
        require_non_empty(&self.display_name, "display_name")?;
        require_non_empty(&self.model_id, "model_id")?;
        require_non_empty(&self.secret_ref, "secret_ref")?;
        require_non_empty(&self.compatibility_profile, "compatibility_profile")?;
        validate_upstream_url(&self.base_url)?;
        validate_upstream_url(&self.provider_policy_url)?;
        self.pricing.validate()?;

        if self.context_window_tokens == 0 {
            return Err(ConfigError::ZeroContextWindow(self.id.clone()));
        }
        if !self.capabilities.supports_streaming {
            return Err(ConfigError::MissingCapability {
                worker_id: self.id.clone(),
                capability: "streaming",
            });
        }
        if !self.capabilities.has(Capability::Tools) {
            return Err(ConfigError::MissingCapability {
                worker_id: self.id.clone(),
                capability: "tools",
            });
        }

        Ok(())
    }
}

fn require_non_empty(value: &str, field: &'static str) -> Result<(), ConfigError> {
    if value.trim().is_empty() {
        return Err(ConfigError::EmptyField(field));
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<(), ConfigError> {
    if id.is_empty()
        || !id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
    {
        return Err(ConfigError::InvalidIdentifier(id.to_owned()));
    }
    Ok(())
}

fn validate_upstream_url(value: &str) -> Result<(), ConfigError> {
    let is_https = value.starts_with("https://") && value.len() > "https://".len();
    let is_loopback_test_url = value.starts_with("http://127.0.0.1")
        || value.starts_with("http://[::1]")
        || value.starts_with("http://localhost");

    if is_https || is_loopback_test_url {
        Ok(())
    } else {
        Err(ConfigError::InvalidUrl(value.to_owned()))
    }
}
