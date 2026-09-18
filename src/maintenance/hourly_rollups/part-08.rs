pub(crate) async fn migrate_codex_invocations_drop_raw_expires_at(
    pool: &Pool<Sqlite>,
    existing: &HashSet<String>,
) -> Result<()> {
    const TEMP_TABLE: &str = "codex_invocations_drop_raw_expires_at_new";

    let mut tx = pool.begin().await?;
    let drop_temp_sql = format!("DROP TABLE IF EXISTS {TEMP_TABLE}");
    sqlx::query(&drop_temp_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to clear stale codex_invocations migration temp table")?;
    let create_temp_sql = codex_invocations_create_sql(TEMP_TABLE);
    sqlx::query(&create_temp_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to create codex_invocations migration temp table")?;

    let temp_pragma_sql = format!("PRAGMA table_info('{TEMP_TABLE}')");
    let new_columns: Vec<String> = sqlx::query(&temp_pragma_sql)
        .fetch_all(tx.as_mut())
        .await
        .context("failed to inspect codex_invocations migration temp schema")?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect();
    let copy_columns: Vec<String> = new_columns
        .into_iter()
        .filter(|column| existing.contains(column))
        .collect();
    if copy_columns.is_empty() {
        bail!("codex_invocations migration found no shared columns to copy");
    }

    let copy_columns_csv = copy_columns.join(", ");
    let copy_sql = format!(
        "INSERT INTO {TEMP_TABLE} ({copy_columns_csv}) SELECT {copy_columns_csv} FROM codex_invocations"
    );
    sqlx::query(&copy_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to copy codex_invocations rows into migration temp table")?;
    sqlx::query("DROP TABLE codex_invocations")
        .execute(tx.as_mut())
        .await
        .context("failed to drop legacy codex_invocations table during migration")?;
    let rename_sql = format!("ALTER TABLE {TEMP_TABLE} RENAME TO codex_invocations");
    sqlx::query(&rename_sql)
        .execute(tx.as_mut())
        .await
        .context("failed to swap migrated codex_invocations table into place")?;
    tx.commit().await?;
    Ok(())
}
