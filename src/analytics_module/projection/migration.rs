use super::*;
pub(crate) async fn apply_analytics_projection_migration(
    connection: &mut PgConnection,
) -> Result<AnalyticsProjectionMigrationResult> {
    let mut transaction = connection
        .begin()
        .await
        .context("begin analytics projection migration transaction")?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(ANALYTICS_PROJECTION_MIGRATION_LOCK_KEY)
        .execute(&mut *transaction)
        .await
        .context("acquire analytics projection migration lock")?;
    let module_schema_ready = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM cvm_analytics.analytics_module_ddl_ledger
         WHERE component = 'analytics-module-ingest'
           AND step_order = 1
           AND state = 'applied'",
    )
    .fetch_one(&mut *transaction)
    .await
    .context("inspect prerequisite analytics module migration")?;
    ensure!(
        module_schema_ready == 1,
        "C4 projection migration requires the applied C3 ingest schema"
    );

    let checksum = format!(
        "{:x}",
        Sha256::digest(ANALYTICS_PROJECTION_SCHEMA_SQL.as_bytes())
    );
    let existing = sqlx::query_as::<_, (i64, String, String, String)>(
        "SELECT schema_version, step_id, checksum, state
         FROM cvm_analytics.analytics_module_ddl_ledger
         WHERE component = $1 AND step_order = 1
         FOR UPDATE",
    )
    .bind(ANALYTICS_PROJECTION_DDL_COMPONENT)
    .fetch_optional(&mut *transaction)
    .await
    .context("read analytics projection migration ledger")?;
    let newer = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM cvm_analytics.analytics_module_ddl_ledger
         WHERE component = $1
           AND (schema_version > $2 OR step_order > 1 OR min_reader_version > $3)",
    )
    .bind(ANALYTICS_PROJECTION_DDL_COMPONENT)
    .bind(ANALYTICS_PROJECTION_SCHEMA_VERSION)
    .bind(ANALYTICS_PROJECTION_READER_VERSION)
    .fetch_one(&mut *transaction)
    .await
    .context("inspect analytics projection migration version")?;
    ensure!(
        newer == 0,
        "analytics projection migration is newer than this reader"
    );

    let mut recovered_steps = 0usize;
    let existing_state = if let Some((schema_version, step_id, stored_checksum, state)) = existing {
        ensure!(
            schema_version <= ANALYTICS_PROJECTION_SCHEMA_VERSION,
            "analytics projection migration schema is newer than this reader"
        );
        ensure!(
            step_id == ANALYTICS_PROJECTION_DDL_STEP,
            "analytics projection migration step identity changed"
        );
        ensure!(
            stored_checksum == checksum,
            "analytics projection migration checksum mismatch"
        );
        ensure!(
            matches!(state.as_str(), "applying" | "applied" | "failed"),
            "analytics projection migration ledger state is invalid"
        );
        Some(state)
    } else {
        None
    };

    if existing_state.as_deref() != Some("applied") {
        sqlx::query(
            "INSERT INTO cvm_analytics.analytics_module_ddl_ledger
                (component, schema_version, step_order, step_id, checksum, state,
                 min_reader_version, requires_bootstrap, error_code, applied_at)
             VALUES ($1, $2, 1, $3, $4, 'applying', $5, TRUE, NULL, NULL)
             ON CONFLICT (component, step_order) DO UPDATE SET
                 schema_version = EXCLUDED.schema_version,
                 step_id = EXCLUDED.step_id,
                 checksum = EXCLUDED.checksum,
                 state = 'applying',
                 min_reader_version = EXCLUDED.min_reader_version,
                 requires_bootstrap = EXCLUDED.requires_bootstrap,
                 error_code = NULL,
                 applied_at = NULL",
        )
        .bind(ANALYTICS_PROJECTION_DDL_COMPONENT)
        .bind(ANALYTICS_PROJECTION_SCHEMA_VERSION)
        .bind(ANALYTICS_PROJECTION_DDL_STEP)
        .bind(&checksum)
        .bind(ANALYTICS_PROJECTION_READER_VERSION)
        .execute(&mut *transaction)
        .await
        .context("write analytics projection migration applying state")?;
        recovered_steps = 1;
    }

    // A savepoint lets us persist a failed state while retaining the outer transaction for a
    // clean retry. An interrupted process rolls the whole transaction back, leaving no false
    // applied marker; a later invocation safely re-enters the applying step.
    let mut ddl = transaction
        .begin()
        .await
        .context("begin analytics projection DDL savepoint")?;
    if let Err(error) = sqlx::raw_sql(ANALYTICS_PROJECTION_SCHEMA_SQL)
        .execute(&mut *ddl)
        .await
    {
        drop(ddl);
        sqlx::query(
            "UPDATE cvm_analytics.analytics_module_ddl_ledger
             SET state = 'failed', error_code = 'analytics_projection_ddl_failed'
             WHERE component = $1 AND step_order = 1",
        )
        .bind(ANALYTICS_PROJECTION_DDL_COMPONENT)
        .execute(&mut *transaction)
        .await
        .context("record analytics projection migration failure")?;
        transaction
            .commit()
            .await
            .context("commit analytics projection migration failure state")?;
        return Err(error).context("apply analytics projection receipt schema");
    }
    ddl.commit()
        .await
        .context("commit analytics projection DDL savepoint")?;

    sqlx::query(
        "UPDATE cvm_analytics.analytics_module_ddl_ledger
         SET schema_version = $1, checksum = $2, state = 'applied',
             error_code = NULL, applied_at = NOW()
         WHERE component = $3 AND step_order = 1",
    )
    .bind(ANALYTICS_PROJECTION_SCHEMA_VERSION)
    .bind(&checksum)
    .bind(ANALYTICS_PROJECTION_DDL_COMPONENT)
    .execute(&mut *transaction)
    .await
    .context("write analytics projection migration applied state")?;
    transaction
        .commit()
        .await
        .context("commit analytics projection migration")?;
    Ok(AnalyticsProjectionMigrationResult {
        component: ANALYTICS_PROJECTION_DDL_COMPONENT,
        version: ANALYTICS_PROJECTION_SCHEMA_VERSION,
        checksum,
        recovered_steps,
    })
}
