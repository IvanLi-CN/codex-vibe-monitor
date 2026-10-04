//! Durable reservation authority shared by conversation and hourly invocation owners.

use super::*;

const MIGRATION: &str = "invocation_range_ownership_v1";

pub(crate) async fn ensure_schema(pool: &Pool<Sqlite>) -> Result<()> {
    let mut transaction = pool.begin().await?;
    let alphabet = PROXY_INVOKE_ID_ALPHABET.iter().collect::<String>();
    sqlx::query(&format!(
        "CREATE TABLE IF NOT EXISTS hourly_invoke_prefixes (\
            utc_hour INTEGER PRIMARY KEY,\
            prefix TEXT NOT NULL UNIQUE,\
            last_invoke_sequence INTEGER NOT NULL DEFAULT -1,\
            created_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ','now')),\
            updated_at TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ','now')),\
            CHECK (length(prefix)=6 AND prefix NOT GLOB '*[^{alphabet}]*'),\
            CHECK (last_invoke_sequence BETWEEN -1 AND 923520)\
        )"
    ))
    .execute(&mut *transaction)
    .await?;
    sqlx::query("INSERT OR IGNORE INTO schema_refresh_migrations (migration_name) VALUES (?1)")
        .bind(MIGRATION)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    debug!(migration = MIGRATION, "invocation range schema ready");
    Ok(())
}
