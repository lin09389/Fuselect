//! Local non-secret metadata persistence (SQLite).

mod migrations;
mod models;
mod paths;
mod sqlite;

pub use migrations::{CURRENT_DB_VERSION, MIGRATION_001_NAME};
pub use models::{
    CompleteRunUpdate, DailySpendRecord, NewRequestRun, NewRunParticipant, NewRunStage,
    PersistedFusionPreset, PersistedGateway, PersistedWorker, RequestRunRecord, RunParticipantRole,
    RunStageRecord,
};
pub use paths::{database_path, resolve_data_dir};
pub use sqlite::SqliteStore;

use crate::domain::ConfigError;

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("记录不存在：{0}")]
    NotFound(String),
    #[error("记录冲突：{0}")]
    Conflict(String),
    #[error("配置无效：{0}")]
    InvalidConfig(#[from] ConfigError),
    #[error("数据库损坏或数据无法解析")]
    CorruptData,
    #[error("数据库版本过高：找到 {found}，当前程序支持 {supported}；拒绝降级打开")]
    UnsupportedDatabaseVersion { found: i64, supported: i64 },
    #[error("数据库迁移失败，事务已回滚")]
    Migration,
    #[error("数值超出 SQLite 可安全保存的范围")]
    ValueOutOfRange,
    #[error("不允许检查未知的数据表")]
    InvalidTable,
    #[error("密钥清理未完成：{0}")]
    KeyringCleanupPending(String),
    #[error("数据库操作失败")]
    Database,
}

impl StorageError {
    pub(crate) fn from_sqlx(error: sqlx::Error) -> Self {
        match error {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                Self::Conflict("unique constraint".to_owned())
            }
            sqlx::Error::RowNotFound => Self::NotFound("row".to_owned()),
            _ => Self::Database,
        }
    }
}

/// Hidden helpers used by integration tests to verify transactional migration rollback.
#[doc(hidden)]
pub mod migrations_test {
    use sqlx::sqlite::SqliteConnection;

    use super::StorageError;
    use super::migrations;

    pub async fn apply_failing_migration(
        connection: &mut SqliteConnection,
    ) -> Result<(), StorageError> {
        migrations::apply_failing_migration_for_test(connection).await
    }

    pub async fn probe_table_exists(
        connection: &mut SqliteConnection,
        name: &str,
    ) -> Result<bool, StorageError> {
        migrations::probe_table_exists(connection, name).await
    }
}
