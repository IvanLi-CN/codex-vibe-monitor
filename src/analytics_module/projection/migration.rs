use super::*;

pub(crate) async fn apply_analytics_projection_migration(
    connection: &mut PgConnection,
) -> Result<AnalyticsProjectionMigrationResult> {
    let mut tx = connection.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(ANALYTICS_PROJECTION_MIGRATION_LOCK_KEY)
        .execute(&mut *tx)
        .await?;
    let ready = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM cvm_analytics.analytics_module_ddl_ledger
        WHERE component='analytics-module-ingest' AND step_order=1 AND state='applied'",
    )
    .fetch_one(&mut *tx)
    .await?;
    ensure!(
        ready == 1,
        "C4 projection migration requires applied C3 ingest schema"
    );
    let newer = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM cvm_analytics.analytics_module_ddl_ledger
        WHERE component=$1 AND (schema_version>$2 OR step_order>2 OR min_reader_version>$3)",
    )
    .bind(ANALYTICS_PROJECTION_DDL_COMPONENT)
    .bind(ANALYTICS_PROJECTION_SCHEMA_VERSION)
    .bind(ANALYTICS_PROJECTION_READER_VERSION)
    .fetch_one(&mut *tx)
    .await?;
    ensure!(
        newer == 0,
        "analytics projection migration is newer than this reader"
    );
    let mut recovered_steps = 0;
    let mut final_checksum = String::new();
    for (version, step, ddl) in [
        (
            1i64,
            ANALYTICS_PROJECTION_DDL_STEP,
            ANALYTICS_PROJECTION_SCHEMA_SQL,
        ),
        (2, "certified-baseline-v2", include_str!("baseline.sql")),
    ] {
        let checksum = format!("{:x}", Sha256::digest(ddl.as_bytes()));
        let existing=sqlx::query_as::<_,(i64,String,String,String)>("SELECT schema_version,step_id,checksum,state
            FROM cvm_analytics.analytics_module_ddl_ledger WHERE component=$1 AND step_order=$2 FOR UPDATE")
            .bind(ANALYTICS_PROJECTION_DDL_COMPONENT).bind(version).fetch_optional(&mut *tx).await?;
        let applied = if let Some((stored_version, stored_step, stored_checksum, state)) = existing
        {
            ensure!(
                stored_version == version && stored_step == step && stored_checksum == checksum,
                "analytics projection immutable migration identity/checksum changed"
            );
            ensure!(
                matches!(state.as_str(), "applying" | "applied" | "failed"),
                "invalid projection migration state"
            );
            state == "applied"
        } else {
            false
        };
        if !applied {
            recovered_steps += 1;
            sqlx::query("INSERT INTO cvm_analytics.analytics_module_ddl_ledger
                (component,schema_version,step_order,step_id,checksum,state,min_reader_version,requires_bootstrap)
                VALUES ($1,$2,$2,$3,$4,'applying',$2,TRUE)
                ON CONFLICT (component,step_order) DO UPDATE SET state='applying',error_code=NULL,applied_at=NULL")
                .bind(ANALYTICS_PROJECTION_DDL_COMPONENT).bind(version).bind(step).bind(&checksum).execute(&mut *tx).await?;
        }
        // Reapply idempotent DDL even for an applied marker, retaining the v1 forward-repair
        // contract. Each step retains its original checksum and version forever.
        let mut savepoint = tx.begin().await?;
        if let Err(error) = sqlx::raw_sql(ddl).execute(&mut *savepoint).await {
            savepoint.rollback().await?;
            sqlx::query("UPDATE cvm_analytics.analytics_module_ddl_ledger SET state='failed',error_code='analytics_projection_ddl_failed'
                WHERE component=$1 AND step_order=$2")
                .bind(ANALYTICS_PROJECTION_DDL_COMPONENT).bind(version).execute(&mut *tx).await?;
            tx.commit().await?;
            return Err(error).context("apply analytics projection migration step");
        }
        savepoint.commit().await?;
        sqlx::query("UPDATE cvm_analytics.analytics_module_ddl_ledger SET state='applied',error_code=NULL,applied_at=NOW()
            WHERE component=$1 AND step_order=$2")
            .bind(ANALYTICS_PROJECTION_DDL_COMPONENT).bind(version).execute(&mut *tx).await?;
        final_checksum = checksum;
    }
    tx.commit().await?;
    Ok(AnalyticsProjectionMigrationResult {
        component: ANALYTICS_PROJECTION_DDL_COMPONENT,
        version: ANALYTICS_PROJECTION_SCHEMA_VERSION,
        checksum: final_checksum,
        recovered_steps,
    })
}
