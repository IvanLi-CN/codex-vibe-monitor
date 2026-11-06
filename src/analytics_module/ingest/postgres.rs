use super::*;
use anyhow::{Context, Result, ensure};
use sqlx::{Connection, PgConnection, PgPool, Sqlite, sqlite::SqlitePoolOptions};
use std::env;
use url::Url;

const OWNER_URL_ENV: &str = "PG_ANALYTICS_TEST_OWNER_URL";
const APP_URL_ENV: &str = "PG_ANALYTICS_TEST_APP_URL";
const MAINTENANCE_URL_ENV: &str = "PG_ANALYTICS_TEST_MAINTENANCE_URL";
const TERMINATOR_URL_ENV: &str = "PG_ANALYTICS_TEST_TERMINATOR_URL";
const RESET_AUTH_ENV: &str = "PG_ANALYTICS_TEST_ALLOW_SCHEMA_RESET";
const TERMINATE_AUTH_ENV: &str = "PG_ANALYTICS_TEST_ALLOW_TERMINATE";

struct FixtureInputs {
    owner_url: String,
    app_url: String,
    maintenance_url: String,
    terminator_url: String,
}

impl FixtureInputs {
    fn load() -> Result<Self> {
        ensure!(
            env::var(RESET_AUTH_ENV).as_deref() == Ok("1"),
            "{RESET_AUTH_ENV}=1 is required"
        );
        ensure!(
            env::var(TERMINATE_AUTH_ENV).as_deref() == Ok("1"),
            "{TERMINATE_AUTH_ENV}=1 is required"
        );
        let owner_url = required_url(OWNER_URL_ENV)?;
        let app_url = required_url(APP_URL_ENV)?;
        let maintenance_url = required_url(MAINTENANCE_URL_ENV)?;
        let terminator_url = required_url(TERMINATOR_URL_ENV)?;
        let database = fixture_database_name(&owner_url, OWNER_URL_ENV)?;
        for (url, name) in [
            (&app_url, APP_URL_ENV),
            (&maintenance_url, MAINTENANCE_URL_ENV),
            (&terminator_url, TERMINATOR_URL_ENV),
        ] {
            ensure!(
                fixture_database_name(url, name)? == database,
                "fixture URLs must target one database"
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

fn required_url(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("{name} is required for the ignored PostgreSQL fixture"))
}

fn fixture_database_name(raw: &str, name: &str) -> Result<String> {
    let parsed = Url::parse(raw).with_context(|| format!("{name} must be a PostgreSQL URL"))?;
    ensure!(
        matches!(parsed.scheme(), "postgres" | "postgresql"),
        "{name} must use PostgreSQL"
    );
    let database = parsed.path().trim_start_matches('/').trim();
    ensure!(
        !database.is_empty() && database.ends_with("_fixture"),
        "{name} must target *_fixture"
    );
    Ok(database.to_string())
}

async fn connect_pool(url: &str, name: &str) -> Result<PgPool> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(url)
        .await
        .with_context(|| format!("connect PostgreSQL fixture {name}"))
}

async fn reset_and_apply(owner: &mut PgConnection) -> Result<()> {
    sqlx::query("DROP SCHEMA IF EXISTS cvm_analytics CASCADE")
        .execute(&mut *owner)
        .await
        .context("reset disposable analytics schema")?;
    sqlx::raw_sql(super::super::super::PG_ANALYTICS_SCHEMA_SQL)
        .execute(&mut *owner)
        .await
        .context("apply legacy analytics schema")?;
    super::super::apply_analytics_module_schema(owner)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
        .context("apply existing C2 module schema")?;
    super::super::apply_analytics_module_roles(owner)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))
        .context("apply existing C2 module roles")?;
    let first = super::apply_analytics_ingest_migration(owner).await?;
    ensure!(
        first.recovered_steps == 1,
        "first ingest migration did not recover one step"
    );
    let second = super::apply_analytics_ingest_migration(owner).await?;
    ensure!(
        second.recovered_steps == 0,
        "ingest migration was not idempotent"
    );

    sqlx::query(
        "UPDATE cvm_analytics.analytics_module_ddl_ledger
         SET state = 'failed', error_code = 'fixture'
         WHERE component = $1 AND step_order = 1",
    )
    .bind(ANALYTICS_INGEST_DDL_COMPONENT)
    .execute(&mut *owner)
    .await
    .context("mark ingest migration failed for re-entry fixture")?;
    sqlx::query(
        "DROP TABLE cvm_analytics.analytics_module_ingest_work,
                   cvm_analytics.analytics_module_ingest_receipts",
    )
    .execute(&mut *owner)
    .await
    .context("remove ingest tables for forward re-entry fixture")?;
    let recovered = super::apply_analytics_ingest_migration(owner).await?;
    ensure!(
        recovered.recovered_steps == 1,
        "failed ingest migration did not re-enter"
    );
    Ok(())
}

async fn seed_consumer(owner: &mut PgConnection) -> Result<()> {
    sqlx::query(
        "INSERT INTO cvm_analytics.analytics_module_owners
            (module_id, owner_id, source_instance_id, owner_epoch, status)
         VALUES ('invocation', 'fixture-owner', 'source-a', 1, 'active')",
    )
    .execute(&mut *owner)
    .await
    .context("seed ingest owner")?;
    sqlx::query(
        "INSERT INTO cvm_analytics.analytics_module_consumers
            (consumer_id, module_id, consumer_version, state, source_instance_id, owner_epoch)
         VALUES ('invocation', 'invocation', 1, 'active', 'source-a', 1)",
    )
    .execute(&mut *owner)
    .await
    .context("seed ingest consumer")?;
    Ok(())
}

async fn source_fixture() -> Result<(sqlx::Pool<Sqlite>, SourceEnvelope)> {
    let source = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .context("connect source fixture")?;
    crate::analytics_changes::ensure_analytics_changes_schema(&source).await?;
    let draft = super::super::source::SourceEnvelopeDraft {
        source_instance_id: "source-a".to_string(),
        event_id: "fixture-event-1".to_string(),
        entity_kind: "invocation".to_string(),
        entity_key: "fixture-row-1".to_string(),
        operation: SourceOperation::Upsert,
        occurred_at: "2026-11-06T00:00:00Z".to_string(),
        payload: serde_json::json!({"fixture": true, "value": 1}),
    };
    let mut transaction = source.begin().await?;
    let (envelope, _) =
        super::super::append_source_envelope_tx(transaction.as_mut(), &draft).await?;
    transaction.commit().await?;
    Ok((source, envelope))
}

#[tokio::test]
#[ignore = "requires an explicitly authorized disposable PostgreSQL 18 *_fixture database and pre-provisioned roles"]
async fn postgres_ingest_fixture_covers_migration_lock_fence_receive_and_ack_recovery() -> Result<()>
{
    let inputs = FixtureInputs::load()?;
    let mut owner = PgConnection::connect(&inputs.owner_url).await?;
    reset_and_apply(&mut owner).await?;
    seed_consumer(&mut owner).await?;
    let maintenance = connect_pool(&inputs.maintenance_url, "maintenance").await?;
    let app = connect_pool(&inputs.app_url, "application").await?;
    let (source, envelope) = source_fixture().await?;
    let binding = SourceBinding::new(
        AnalyticsModuleId::Invocation,
        "invocation",
        "source-a",
        1,
        1,
        0,
        0,
    )?;

    let mut actor = PgIngestActor::acquire(&maintenance, binding.clone())
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let receipt = actor
        .receive_batch(std::slice::from_ref(&envelope))
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    ensure!(
        receipt.inserted_events == 1 && receipt.through_change_id == 1,
        "first receive did not commit C"
    );
    let duplicate = actor
        .receive_batch(std::slice::from_ref(&envelope))
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    ensure!(
        duplicate.inserted_events == 0 && duplicate.duplicate_events == 1,
        "duplicate receive was not idempotent"
    );

    let mut conflicting = envelope.clone();
    conflicting.payload = serde_json::json!({"fixture": true, "value": 2});
    conflicting.payload_digest = digest_json(&conflicting.payload)?;
    ensure!(
        actor
            .receive_batch(std::slice::from_ref(&conflicting))
            .await
            .is_err(),
        "conflicting event unexpectedly received"
    );

    let second_actor = PgIngestActor::acquire(&maintenance, binding.clone()).await;
    ensure!(
        second_actor.is_err(),
        "second writer unexpectedly acquired the session lock"
    );
    actor
        .close()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    let mut broken = PgIngestActor::acquire(&maintenance, binding.clone())
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let backend_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut broken.connection)
        .await
        .context("read ingest actor backend pid")?;
    let mut terminator = PgConnection::connect(&inputs.terminator_url)
        .await
        .context("connect PostgreSQL terminator fixture role")?;
    let terminated: bool = sqlx::query_scalar("SELECT pg_terminate_backend($1, 0::bigint)")
        .bind(backend_pid)
        .fetch_one(&mut terminator)
        .await
        .context("terminate ingest actor backend")?;
    ensure!(
        terminated,
        "fixture did not terminate the ingest actor backend"
    );
    let connection_error = broken
        .receive_batch(std::slice::from_ref(&envelope))
        .await
        .expect_err("closed writer connection unexpectedly received");
    ensure!(
        connection_error.is_retryable(),
        "closed writer connection was not retryable"
    );
    terminator.close().await?;

    let payload: String = sqlx::query_scalar(
        "SELECT payload::text
         FROM cvm_analytics.analytics_module_ingest_receipts
         WHERE consumer_id = 'invocation' AND event_id = 'fixture-event-1'",
    )
    .fetch_one(&mut owner)
    .await
    .context("read persisted receive payload")?;
    ensure!(
        payload.contains("fixture"),
        "persisted receive payload is not recoverable"
    );
    let work_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM cvm_analytics.analytics_module_ingest_work
         WHERE consumer_id = 'invocation' AND state = 'pending'",
    )
    .fetch_one(&mut owner)
    .await
    .context("read pending projection work")?;
    ensure!(
        work_count == 1,
        "receive did not persist exactly one pending work item"
    );

    sqlx::query("DELETE FROM analytics_change_events")
        .execute(&source)
        .await
        .context("delete source rows after PG receive")?;
    let mut restarted = PgIngestActor::acquire(&maintenance, binding.clone())
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    let ack_status = restarted
        .consume_once(&source)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    ensure!(
        matches!(ack_status, ConsumeStatus::AckPending { .. }),
        "source ack failure was not retained as pending"
    );
    restarted
        .close()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    let app_error = sqlx::query(
        "SELECT payload
         FROM cvm_analytics.analytics_module_ingest_receipts
         WHERE event_id = 'fixture-event-1'",
    )
    .execute(&app)
    .await
    .expect_err("application role unexpectedly read maintenance receipt data");
    let code = match app_error {
        sqlx::Error::Database(error) => error.code().map(|value| value.into_owned()),
        _ => None,
    };
    ensure!(
        code.as_deref() == Some("42501"),
        "application receipt access did not fail with insufficient_privilege"
    );

    let mut fenced = PgIngestActor::acquire(&maintenance, binding)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
    sqlx::query(
        "UPDATE cvm_analytics.analytics_module_owners
         SET owner_epoch = 2
         WHERE module_id = 'invocation'",
    )
    .execute(&mut owner)
    .await
    .context("fence ingest owner")?;
    let fence_error = fenced
        .receive_batch(std::slice::from_ref(&envelope))
        .await
        .expect_err("fenced actor unexpectedly wrote");
    ensure!(
        fence_error.to_string().contains("fenced"),
        "owner epoch fence was not reported"
    );
    fenced
        .close()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;

    owner.close().await?;
    app.close().await;
    maintenance.close().await;
    source.close().await;
    Ok(())
}
