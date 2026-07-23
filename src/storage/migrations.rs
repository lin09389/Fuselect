use sqlx::Connection;
use sqlx::sqlite::SqliteConnection;
use sqlx::{Sqlite, Transaction};

use super::StorageError;
use super::models::format_time;

pub const CURRENT_DB_VERSION: i64 = 1;
pub const MIGRATION_001_NAME: &str = "0001_initial";

const MIGRATION_001_SQL: &str = include_str!("../../migrations/0001_initial.sql");

pub async fn migrate(connection: &mut SqliteConnection) -> Result<(), StorageError> {
    ensure_migration_table(connection).await?;
    let applied = current_version(connection).await?;
    if applied > CURRENT_DB_VERSION {
        return Err(StorageError::UnsupportedDatabaseVersion {
            found: applied,
            supported: CURRENT_DB_VERSION,
        });
    }

    for version in (applied + 1)..=CURRENT_DB_VERSION {
        apply_version(connection, version).await?;
    }
    Ok(())
}

async fn ensure_migration_table(connection: &mut SqliteConnection) -> Result<(), StorageError> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY NOT NULL,
            name TEXT NOT NULL UNIQUE,
            applied_at TEXT NOT NULL
        )",
    )
    .execute(&mut *connection)
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
}

pub async fn current_version(connection: &mut SqliteConnection) -> Result<i64, StorageError> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT COALESCE(MAX(version), 0) FROM schema_migrations")
            .fetch_optional(&mut *connection)
            .await
            .map_err(StorageError::from_sqlx)?;
    Ok(row.map(|value| value.0).unwrap_or(0))
}

async fn apply_version(
    connection: &mut SqliteConnection,
    version: i64,
) -> Result<(), StorageError> {
    let mut tx: Transaction<'_, Sqlite> =
        connection.begin().await.map_err(StorageError::from_sqlx)?;

    match version {
        1 => apply_migration_sql(&mut tx, MIGRATION_001_SQL).await?,
        _ => return Err(StorageError::Migration),
    }

    let name = migration_name(version)?;
    let applied_at = format_time(chrono::Utc::now());
    sqlx::query("INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, ?)")
        .bind(version)
        .bind(name)
        .bind(applied_at)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;

    sqlx::query(&format!("PRAGMA user_version = {version}"))
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;

    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(())
}

async fn apply_migration_sql(
    tx: &mut Transaction<'_, Sqlite>,
    sql: &str,
) -> Result<(), StorageError> {
    for statement in sql.split(';') {
        let cleaned: String = statement
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        if cleaned.is_empty() {
            continue;
        }
        sqlx::query(&cleaned)
            .execute(&mut **tx)
            .await
            .map_err(|_| StorageError::Migration)?;
    }
    Ok(())
}

fn migration_name(version: i64) -> Result<&'static str, StorageError> {
    match version {
        1 => Ok(MIGRATION_001_NAME),
        _ => Err(StorageError::Migration),
    }
}

/// Run a migration step that intentionally fails inside a transaction.
///
/// Used by integration tests to prove rollback leaves no partial schema.
pub async fn apply_failing_migration_for_test(
    connection: &mut SqliteConnection,
) -> Result<(), StorageError> {
    let mut tx = connection.begin().await.map_err(StorageError::from_sqlx)?;
    sqlx::query("CREATE TABLE migration_probe (id INTEGER PRIMARY KEY)")
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;
    let result = sqlx::query("THIS IS NOT VALID SQL").execute(&mut *tx).await;
    if result.is_ok() {
        return Err(StorageError::Migration);
    }
    // Dropping the transaction without commit rolls back the probe table.
    drop(tx);
    Ok(())
}

pub async fn probe_table_exists(
    connection: &mut SqliteConnection,
    name: &str,
) -> Result<bool, StorageError> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?")
            .bind(name)
            .fetch_optional(&mut *connection)
            .await
            .map_err(StorageError::from_sqlx)?;
    Ok(row.is_some())
}
