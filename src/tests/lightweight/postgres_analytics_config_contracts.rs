use super::*;

#[test]
fn app_config_defaults_to_sqlite_analytics_without_new_environment() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let _env = EnvVarGuard::set(&[
        (ENV_ANALYTICS_BACKEND, None),
        (ENV_POSTGRES_DATABASE_URL, None),
        (ENV_POSTGRES_MIGRATION_URL, None),
        (ENV_POSTGRES_ANALYTICS_MAINTENANCE_URL, None),
        (ENV_POSTGRES_MAX_CONNECTIONS, None),
    ]);

    let config = AppConfig::from_sources(&CliArgs::default())
        .expect("analytics defaults should preserve SQLite configuration");

    assert_eq!(config.analytics_backend, AnalyticsBackend::Sqlite);
    assert_eq!(config.postgres_database_url, None);
    assert_eq!(config.postgres_analytics_maintenance_url, None);
    assert_eq!(config.postgres_max_connections, 5);
    assert!(config.database_url().starts_with("sqlite://"));
}

#[test]
fn app_config_accepts_postgres_settings_without_leaking_credentials() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let _env = EnvVarGuard::set(&[
        (ENV_ANALYTICS_BACKEND, Some("postgres")),
        (
            ENV_POSTGRES_DATABASE_URL,
            Some("postgresql://analytics:secret@example.test/codex"),
        ),
        (
            ENV_POSTGRES_MIGRATION_URL,
            Some("postgresql://migration:migration-secret@example.test/codex"),
        ),
        (
            ENV_POSTGRES_ANALYTICS_MAINTENANCE_URL,
            Some("postgresql://maintenance:other-secret@example.test/codex"),
        ),
        (ENV_POSTGRES_MAX_CONNECTIONS, Some("7")),
    ]);

    let config = AppConfig::from_sources(&CliArgs::default()).expect("postgres settings parse");
    let debug = format!("{config:?}");
    let json = serde_json::to_string(&config).expect("config serializes");

    assert_eq!(config.analytics_backend, AnalyticsBackend::Postgres);
    assert_eq!(config.postgres_max_connections, 7);
    assert_eq!(
        config
            .postgres_analytics_maintenance_url
            .as_ref()
            .expect("maintenance URL")
            .as_str(),
        "postgresql://maintenance:other-secret@example.test/codex"
    );
    assert_eq!(
        config
            .postgres_database_url
            .as_ref()
            .expect("postgres URL")
            .as_str(),
        "postgresql://analytics:secret@example.test/codex"
    );
    assert!(!debug.contains("secret"));
    assert!(!debug.contains("other-secret"));
    assert!(!json.contains("secret"));
    assert!(!json.contains("other-secret"));
}

#[test]
fn app_config_rejects_postgres_without_url_and_zero_pool_size() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();

    let _env = EnvVarGuard::set(&[
        (ENV_ANALYTICS_BACKEND, Some("postgres")),
        (ENV_POSTGRES_DATABASE_URL, None),
        (ENV_POSTGRES_ANALYTICS_MAINTENANCE_URL, None),
        (ENV_POSTGRES_MAX_CONNECTIONS, None),
    ]);
    let missing_url = AppConfig::from_sources(&CliArgs::default())
        .expect_err("postgres backend requires a database URL");
    assert!(missing_url.to_string().contains(ENV_POSTGRES_DATABASE_URL));
    drop(_env);

    let _env = EnvVarGuard::set(&[
        (ENV_ANALYTICS_BACKEND, Some("sqlite")),
        (ENV_POSTGRES_DATABASE_URL, None),
        (ENV_POSTGRES_ANALYTICS_MAINTENANCE_URL, None),
        (ENV_POSTGRES_MAX_CONNECTIONS, Some("0")),
    ]);
    let zero_connections = AppConfig::from_sources(&CliArgs::default())
        .expect_err("zero postgres connections should be rejected");
    assert!(
        zero_connections
            .to_string()
            .contains(ENV_POSTGRES_MAX_CONNECTIONS)
    );
}

#[test]
fn app_config_rejects_invalid_maintenance_url_with_its_own_name() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let _env = EnvVarGuard::set(&[
        (ENV_ANALYTICS_BACKEND, Some("sqlite")),
        (ENV_POSTGRES_DATABASE_URL, None),
        (ENV_POSTGRES_ANALYTICS_MAINTENANCE_URL, Some("not-a-url")),
        (ENV_POSTGRES_MAX_CONNECTIONS, None),
    ]);

    let error = AppConfig::from_sources(&CliArgs::default())
        .expect_err("invalid maintenance URL should be rejected");
    assert!(
        error
            .to_string()
            .contains(ENV_POSTGRES_ANALYTICS_MAINTENANCE_URL)
    );
    assert!(!error.to_string().contains(ENV_POSTGRES_DATABASE_URL));
}

#[test]
fn app_config_rejects_shared_postgres_login_roles() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let _env = EnvVarGuard::set(&[
        (ENV_ANALYTICS_BACKEND, Some("postgres")),
        (
            ENV_POSTGRES_DATABASE_URL,
            Some("postgresql://analytics:secret@example.test/codex"),
        ),
        (
            ENV_POSTGRES_ANALYTICS_MAINTENANCE_URL,
            Some("postgresql://analytics:other-secret@example.test/codex"),
        ),
        (ENV_POSTGRES_MAX_CONNECTIONS, None),
    ]);

    let error = AppConfig::from_sources(&CliArgs::default())
        .expect_err("shared PostgreSQL login roles should be rejected");
    assert!(
        error
            .to_string()
            .contains("different PostgreSQL login roles")
    );
    assert!(!error.to_string().contains("secret"));
    assert!(!error.to_string().contains("other-secret"));
}

#[test]
fn app_config_rejects_shared_postgres_login_roles_from_user_query_override() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let _env = EnvVarGuard::set(&[
        (ENV_ANALYTICS_BACKEND, Some("postgres")),
        (
            ENV_POSTGRES_DATABASE_URL,
            Some("postgresql://analytics:secret@example.test/codex"),
        ),
        (
            ENV_POSTGRES_ANALYTICS_MAINTENANCE_URL,
            Some("postgresql://maintenance:other-secret@example.test/codex?user=analytics"),
        ),
        (ENV_POSTGRES_MAX_CONNECTIONS, None),
    ]);

    let error = AppConfig::from_sources(&CliArgs::default())
        .expect_err("the SQLx user query override should not bypass role separation");
    assert!(
        error
            .to_string()
            .contains("must use different PostgreSQL login roles")
    );
    assert!(!error.to_string().contains("secret"));
    assert!(!error.to_string().contains("other-secret"));
}

#[test]
fn analytics_ownership_switch_is_captured_at_process_configuration() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let _env = EnvVarGuard::set(&[(ENV_ANALYTICS_OWNERSHIP_V2, Some("disabled"))]);
    let config = AppConfig::from_sources(&CliArgs::default()).expect("parse startup configuration");
    let _changed = EnvVarGuard::set(&[(ENV_ANALYTICS_OWNERSHIP_V2, Some("enabled"))]);
    assert_eq!(
        config.analytics_ownership_v2().expect("captured switch"),
        AnalyticsOwnershipV2::Disabled
    );
    let restarted =
        AppConfig::from_sources(&CliArgs::default()).expect("parse restart configuration");
    assert_eq!(
        restarted.analytics_ownership_v2().expect("restart switch"),
        AnalyticsOwnershipV2::Enabled
    );
}
