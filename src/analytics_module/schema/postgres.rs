use anyhow::{Context, Result, bail, ensure};
use sqlx::{Connection, PgConnection};
use std::env;
use url::Url;

const OWNER_URL_ENV: &str = "PG_ANALYTICS_TEST_OWNER_URL";
const APP_URL_ENV: &str = "PG_ANALYTICS_TEST_APP_URL";
const MAINTENANCE_URL_ENV: &str = "PG_ANALYTICS_TEST_MAINTENANCE_URL";
const TERMINATOR_URL_ENV: &str = "PG_ANALYTICS_TEST_TERMINATOR_URL";
const RESET_AUTH_ENV: &str = "PG_ANALYTICS_TEST_ALLOW_SCHEMA_RESET";
const TERMINATE_AUTH_ENV: &str = "PG_ANALYTICS_TEST_ALLOW_TERMINATE";
const PG18_TERMINATE_BACKEND_SIGNATURE: &str = "pg_catalog.pg_terminate_backend(integer,bigint)";

struct FixtureInputs {
    owner_url: String,
    app_url: String,
    maintenance_url: String,
    terminator_url: String,
}

impl FixtureInputs {
    fn load() -> Result<Self> {
        ensure_flag(RESET_AUTH_ENV)?;
        ensure_flag(TERMINATE_AUTH_ENV)?;

        let owner_url = required_url(OWNER_URL_ENV)?;
        let app_url = required_url(APP_URL_ENV)?;
        let maintenance_url = required_url(MAINTENANCE_URL_ENV)?;
        let terminator_url = required_url(TERMINATOR_URL_ENV)?;
        let database = fixture_database_name(&owner_url, OWNER_URL_ENV)?;

        for (url, variable) in [
            (&app_url, APP_URL_ENV),
            (&maintenance_url, MAINTENANCE_URL_ENV),
            (&terminator_url, TERMINATOR_URL_ENV),
        ] {
            ensure!(
                fixture_database_name(url, variable)? == database,
                "all PostgreSQL fixture URLs must target the same *_fixture database"
            );
        }

        Ok(Self {
            owner_url,
            app_url,
            maintenance_url,
            terminator_url,
        })
    }
}

fn required_url(variable: &str) -> Result<String> {
    env::var(variable)
        .with_context(|| format!("{variable} is required for the ignored PostgreSQL fixture"))
}

fn ensure_flag(variable: &str) -> Result<()> {
    let value = env::var(variable).with_context(|| format!("{variable}=1 is required"))?;
    ensure!(value == "1", "{variable} must equal 1");
    Ok(())
}

fn fixture_database_name(raw: &str, variable: &str) -> Result<String> {
    let parsed =
        Url::parse(raw).with_context(|| format!("{variable} must be a valid PostgreSQL URL"))?;
    ensure!(
        matches!(parsed.scheme(), "postgres" | "postgresql"),
        "{variable} must use a PostgreSQL URL"
    );
    let database = parsed.path().trim_start_matches('/').trim();
    ensure!(
        !database.is_empty() && !database.contains('/') && database.ends_with("_fixture"),
        "{variable} must target a disposable database whose name ends with _fixture"
    );
    Ok(database.to_owned())
}

async fn connect(url: &str, label: &str) -> Result<PgConnection> {
    PgConnection::connect(url)
        .await
        .with_context(|| format!("connect PostgreSQL fixture {label}"))
}

async fn reset_legacy_schema(owner: &mut PgConnection) -> Result<()> {
    sqlx::query("DROP SCHEMA IF EXISTS cvm_analytics CASCADE")
        .execute(&mut *owner)
        .await
        .context("reset disposable PostgreSQL fixture schema")?;
    sqlx::raw_sql(super::super::super::PG_ANALYTICS_SCHEMA_SQL)
        .execute(&mut *owner)
        .await
        .context("apply legacy PostgreSQL schema fixture")?;
    Ok(())
}

async fn cleanup_schema(owner: &mut PgConnection) -> Result<()> {
    sqlx::query("DROP SCHEMA IF EXISTS cvm_analytics CASCADE")
        .execute(&mut *owner)
        .await
        .context("clean up disposable PostgreSQL fixture schema")?;
    Ok(())
}

async fn bootstrap_module_ledger(owner: &mut PgConnection) -> Result<()> {
    sqlx::raw_sql(super::super::ANALYTICS_MODULE_LEDGER_SCHEMA_SQL)
        .execute(&mut *owner)
        .await
        .context("bootstrap module ledger fixture")?;
    Ok(())
}

async fn module_consumers_exists(owner: &mut PgConnection) -> Result<bool> {
    sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1
             FROM pg_class AS relation
             JOIN pg_namespace AS namespace ON namespace.oid = relation.relnamespace
             WHERE namespace.nspname = 'cvm_analytics'
               AND relation.relname = 'analytics_module_consumers'
         )",
    )
    .fetch_one(&mut *owner)
    .await
    .context("inspect module relation fixture state")
}

fn ensure_insufficient_privilege(error: sqlx::Error, operation: &str) -> Result<()> {
    let code = match &error {
        sqlx::Error::Database(database_error) => {
            database_error.code().map(|code| code.into_owned())
        }
        _ => None,
    };
    ensure!(
        code.as_deref() == Some("42501"),
        "{operation} returned SQLSTATE {code:?} instead of insufficient_privilege (42501): {error}"
    );
    Ok(())
}

async fn insert_ledger_fixture_row(
    owner: &mut PgConnection,
    schema_version: i64,
    checksum: &str,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO cvm_analytics.analytics_module_ddl_ledger
            (component, schema_version, step_order, step_id, checksum, state,
             min_reader_version, requires_bootstrap)
         VALUES ($1, $2, 1, $3, $4, 'applied', 1, TRUE)",
    )
    .bind(super::super::ANALYTICS_MODULE_DDL_COMPONENT)
    .bind(schema_version)
    .bind(super::super::ANALYTICS_MODULE_DDL_STEP)
    .bind(checksum)
    .execute(&mut *owner)
    .await
    .context("insert module ledger fixture row")?;
    Ok(())
}

async fn test_idempotent_schema(owner: &mut PgConnection) -> Result<()> {
    reset_legacy_schema(owner).await?;

    let first = super::apply_analytics_module_schema(owner)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let second = super::apply_analytics_module_schema(owner)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    ensure!(
        first.recovered_steps == 1,
        "first schema apply did not record one step"
    );
    ensure!(
        second.recovered_steps == 0,
        "second schema apply was not idempotent"
    );
    ensure!(
        first.checksum == second.checksum,
        "schema checksum changed between retries"
    );
    ensure!(
        module_consumers_exists(owner).await?,
        "module DDL did not create consumers"
    );

    let applied_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM cvm_analytics.analytics_module_ddl_ledger
         WHERE component = $1 AND state = 'applied'",
    )
    .bind(super::super::ANALYTICS_MODULE_DDL_COMPONENT)
    .fetch_one(&mut *owner)
    .await
    .context("inspect applied module ledger fixture row")?;
    ensure!(
        applied_count == 1,
        "module ledger has an unexpected applied-row count"
    );
    let (step_id, min_reader_version): (String, i64) = sqlx::query_as(
        "SELECT step_id, min_reader_version
         FROM cvm_analytics.analytics_module_ddl_ledger
         WHERE component = $1 AND step_order = 1",
    )
    .bind(super::super::ANALYTICS_MODULE_DDL_COMPONENT)
    .fetch_one(&mut *owner)
    .await
    .context("inspect applied module ledger marker")?;
    ensure!(
        step_id == super::super::ANALYTICS_MODULE_DDL_STEP,
        "module ledger step identity changed"
    );
    ensure!(
        min_reader_version == super::super::ANALYTICS_MODULE_DDL_MIN_READER_VERSION,
        "module ledger minimum reader marker changed"
    );
    Ok(())
}

async fn test_checksum_fail_closed(owner: &mut PgConnection) -> Result<()> {
    reset_legacy_schema(owner).await?;
    bootstrap_module_ledger(owner).await?;
    insert_ledger_fixture_row(owner, 1, "intentionally-wrong-checksum").await?;

    let error = super::apply_analytics_module_schema(owner)
        .await
        .expect_err("wrong module checksum must fail closed");
    ensure!(
        error == super::AnalyticsModuleMigrationError::LedgerChecksumMismatch,
        "wrong module checksum returned an unexpected error: {error}"
    );
    ensure!(
        !module_consumers_exists(owner).await?,
        "wrong checksum path applied module DDL"
    );
    Ok(())
}

async fn test_newer_version_fail_closed(owner: &mut PgConnection) -> Result<()> {
    reset_legacy_schema(owner).await?;
    bootstrap_module_ledger(owner).await?;
    insert_ledger_fixture_row(owner, 2, "future-module-checksum").await?;

    let error = super::apply_analytics_module_schema(owner)
        .await
        .expect_err("newer module version must fail closed");
    ensure!(
        error == super::AnalyticsModuleMigrationError::LedgerNewerVersion,
        "newer module version returned an unexpected error: {error}"
    );
    ensure!(
        !module_consumers_exists(owner).await?,
        "newer module version path applied module DDL"
    );
    Ok(())
}

async fn wait_for_lock(observer: &mut PgConnection, pid: i32) -> Result<()> {
    for _ in 0..500 {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                 SELECT 1
                 FROM pg_stat_activity
                 WHERE pid = $1 AND wait_event_type = 'Lock'
             )",
        )
        .bind(pid)
        .fetch_one(&mut *observer)
        .await
        .context("observe interrupted module runner lock")?;
        if waiting {
            return Ok(());
        }
        tokio::task::yield_now().await;
    }
    bail!("module runner did not reach the disposable fixture lock wait");
}

async fn ensure_terminator_permission(observer: &mut PgConnection) -> Result<()> {
    let terminator_role: String = sqlx::query_scalar("SELECT current_user")
        .fetch_one(&mut *observer)
        .await
        .context("inspect fixture terminator role")?;
    let can_terminate: bool = sqlx::query_scalar(
        "SELECT has_function_privilege(
             current_user,
             $1::regprocedure,
             'EXECUTE'
         )",
    )
    .bind(PG18_TERMINATE_BACKEND_SIGNATURE)
    .fetch_one(&mut *observer)
    .await
    .context("check fixture termination permission")?;
    ensure!(
        can_terminate,
        "fixture terminator role {terminator_role} lacks pg_terminate_backend permission"
    );
    Ok(())
}

async fn terminate_when_locked(observer: &mut PgConnection, pid: i32) -> Result<()> {
    let lock_result = wait_for_lock(observer, pid).await;
    let terminated: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1)")
        .bind(pid)
        .fetch_one(&mut *observer)
        .await
        .context("terminate disposable module runner backend")?;
    ensure!(
        terminated,
        "fixture terminator did not terminate module runner backend"
    );
    lock_result?;
    Ok(())
}

async fn test_rollback_and_reentry(inputs: &FixtureInputs, owner: &mut PgConnection) -> Result<()> {
    reset_legacy_schema(owner).await?;
    sqlx::query("CREATE TABLE cvm_analytics.analytics_module_consumers (fixture_sentinel TEXT)")
        .execute(&mut *owner)
        .await
        .context("create disposable lock fixture relation")?;

    let mut blocker = connect(&inputs.owner_url, "DDL lock blocker").await?;
    let mut blocker_transaction = blocker.begin().await.context("begin DDL lock fixture")?;
    sqlx::query("LOCK TABLE cvm_analytics.analytics_module_consumers IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker_transaction)
        .await
        .context("lock disposable module relation")?;

    let mut victim = connect(&inputs.owner_url, "interrupted module runner").await?;
    let victim_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut victim)
        .await
        .context("inspect interrupted module runner backend")?;
    let mut observer = connect(&inputs.terminator_url, "fixture terminator").await?;
    ensure_terminator_permission(&mut observer).await?;

    let (interrupted, termination) = tokio::join!(
        super::apply_analytics_module_schema(&mut victim),
        terminate_when_locked(&mut observer, victim_pid)
    );
    termination?;
    ensure!(
        interrupted.is_err(),
        "terminated module runner unexpectedly succeeded"
    );

    observer.close().await.context("close fixture observer")?;
    blocker_transaction
        .rollback()
        .await
        .context("release disposable DDL lock")?;
    blocker.close().await.context("close DDL lock blocker")?;
    sqlx::query("DROP TABLE cvm_analytics.analytics_module_consumers")
        .execute(&mut *owner)
        .await
        .context("remove malformed rollback fixture relation")?;

    let rerun = super::apply_analytics_module_schema(owner)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    ensure!(
        rerun.recovered_steps == 1,
        "module runner did not re-enter after rollback"
    );
    ensure!(
        module_consumers_exists(owner).await?,
        "module runner re-entry missed DDL"
    );
    Ok(())
}

async fn test_role_boundaries(inputs: &FixtureInputs, owner: &mut PgConnection) -> Result<()> {
    reset_legacy_schema(owner).await?;
    super::apply_analytics_module_schema(owner)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let first_roles = super::apply_analytics_module_roles(owner)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let second_roles = super::apply_analytics_module_roles(owner)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    ensure!(
        first_roles.checksum == second_roles.checksum,
        "repeated role reconciliation changed its checksum"
    );

    sqlx::query(
        "INSERT INTO cvm_analytics.analytics_module_owners
            (module_id, owner_id, source_instance_id, owner_epoch, status)
         VALUES ('invocation', 'fixture-owner', 'fixture-source', 1, 'active')",
    )
    .execute(&mut *owner)
    .await
    .context("insert owner role-boundary fixture row")?;

    let mut app = connect(&inputs.app_url, "application role").await?;
    let app_inherits_reader: bool =
        sqlx::query_scalar("SELECT pg_has_role(current_user, 'cvm_analytics_reader', 'USAGE')")
            .fetch_one(&mut app)
            .await
            .context("inspect pre-provisioned reader membership")?;
    ensure!(
        app_inherits_reader,
        "fixture provisioning must grant reader membership before object-grant testing"
    );
    sqlx::query("SELECT consumer_id, module_id FROM cvm_analytics.analytics_module_consumers")
        .fetch_optional(&mut app)
        .await
        .context("reader SELECT contract")?;
    let source_instance_id: String = sqlx::query_scalar(
        "SELECT source_instance_id
         FROM cvm_analytics.analytics_module_owners
         WHERE module_id = 'invocation'",
    )
    .fetch_one(&mut app)
    .await
    .context("reader owner source-instance SELECT contract")?;
    ensure!(
        source_instance_id == "fixture-source",
        "reader owner source-instance SELECT returned an unexpected value"
    );
    let error = sqlx::query(
        "SELECT owner_id
         FROM cvm_analytics.analytics_module_owners
         WHERE module_id = 'invocation'",
    )
    .fetch_one(&mut app)
    .await
    .expect_err("reader application read of owner_id unexpectedly succeeded");
    ensure_insufficient_privilege(error, "reader unauthorized owners column contract")?;
    let error = sqlx::query(
        "INSERT INTO cvm_analytics.analytics_module_owners
            (module_id, owner_id, source_instance_id, owner_epoch, status)
         VALUES ('app-owner-write', 'fixture-owner', 'fixture-source', 1, 'active')",
    )
    .execute(&mut app)
    .await
    .expect_err("reader application owners INSERT unexpectedly succeeded");
    ensure_insufficient_privilege(error, "reader owners INSERT contract")?;
    let error = sqlx::query(
        "UPDATE cvm_analytics.analytics_module_owners
         SET status = 'released'
         WHERE module_id = 'invocation'",
    )
    .execute(&mut app)
    .await
    .expect_err("reader application owners UPDATE unexpectedly succeeded");
    ensure_insufficient_privilege(error, "reader owners UPDATE contract")?;
    let error = sqlx::query(
        "INSERT INTO cvm_analytics.analytics_module_consumers
            (consumer_id, module_id, consumer_version, state, source_instance_id)
         VALUES ('app-write', 'invocation', 1, 'building', 'fixture')",
    )
    .execute(&mut app)
    .await
    .expect_err("reader application INSERT unexpectedly succeeded");
    ensure_insufficient_privilege(error, "reader application INSERT contract")?;
    let error = sqlx::query("CREATE TABLE cvm_analytics.app_must_not_ddl (id INTEGER)")
        .execute(&mut app)
        .await
        .expect_err("application DDL unexpectedly succeeded");
    ensure_insufficient_privilege(error, "application DDL contract")?;
    app.close()
        .await
        .context("close application fixture connection")?;

    let mut maintenance = connect(&inputs.maintenance_url, "maintenance role").await?;
    sqlx::query(
        "INSERT INTO cvm_analytics.analytics_module_builds
            (build_id, module_id, consumer_id, source_instance_id, source_generation,
             source_high_watermark, source_coverage_revision, owner_epoch, state, projection_version)
         VALUES ('maintenance-build', 'invocation', 'maintenance-consumer', 'fixture', 0, 0, 0, 1, 'pending', 1)",
    )
    .execute(&mut maintenance)
    .await
    .context("maintenance INSERT contract")?;
    sqlx::query(
        "UPDATE cvm_analytics.analytics_module_builds
         SET state = 'ready'
         WHERE build_id = 'maintenance-build'",
    )
    .execute(&mut maintenance)
    .await
    .context("maintenance UPDATE contract")?;
    let error = sqlx::query(
        "INSERT INTO cvm_analytics.analytics_module_consumers
            (consumer_id, module_id, consumer_version, state, source_instance_id)
         VALUES ('maintenance-write', 'invocation', 1, 'building', 'fixture')",
    )
    .execute(&mut maintenance)
    .await
    .expect_err("maintenance consumer INSERT unexpectedly succeeded");
    ensure_insufficient_privilege(error, "maintenance consumer INSERT contract")?;
    let error = sqlx::query("CREATE TABLE cvm_analytics.maintenance_must_not_ddl (id INTEGER)")
        .execute(&mut maintenance)
        .await
        .expect_err("maintenance DDL unexpectedly succeeded");
    ensure_insufficient_privilege(error, "maintenance DDL contract")?;
    maintenance
        .close()
        .await
        .context("close maintenance fixture connection")?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires an explicitly authorized disposable PostgreSQL 18 *_fixture database and pre-provisioned roles"]
async fn postgres_module_api_fixture_behaviors() -> Result<()> {
    let inputs = FixtureInputs::load()?;
    let mut owner = connect(&inputs.owner_url, "schema owner").await?;
    let result = async {
        test_idempotent_schema(&mut owner).await?;
        test_checksum_fail_closed(&mut owner).await?;
        test_newer_version_fail_closed(&mut owner).await?;
        test_rollback_and_reentry(&inputs, &mut owner).await?;
        test_role_boundaries(&inputs, &mut owner).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    let cleanup = cleanup_schema(&mut owner).await;
    let close = owner.close().await;
    result?;
    cleanup?;
    close.context("close schema owner fixture connection")?;
    Ok(())
}
