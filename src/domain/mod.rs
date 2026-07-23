//! Domain types and validation that do not depend on HTTP, storage, or CLI I/O.

mod budget;
mod error;
mod fusion;
mod worker;

pub use budget::{BudgetLimits, PricePerMillion};
pub use error::ConfigError;
pub use fusion::{FusionPolicy, FusionPreset, MAX_ADVISORS, MIN_ADVISORS, QualityTier};
pub use worker::{Capability, WorkerCapabilities, WorkerConfig, WorkerHealthStatus};

pub const MAX_WORKERS: usize = 10;
