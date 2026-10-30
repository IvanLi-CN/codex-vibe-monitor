use super::*;

#[test]
fn app_config_defaults_to_sqlite_analytics_without_new_environment() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();
    let _env = EnvVarGuard::set(&[
        (ENV_ANALYTICS_BACKEND, None),
        (ENV_POSTGRES_DATABASE_URL, None),
        (ENV_POSTGRES_MAX_CONNECTIONS, None),
    ]);

    let config = AppConfig::from_sources(&CliArgs::default())
        .expect("analytics defaults should preserve SQLite configuration");

    assert_eq!(config.analytics_backend, AnalyticsBackend::Sqlite);
    assert_eq!(config.postgres_database_url, None);
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
        (ENV_POSTGRES_MAX_CONNECTIONS, Some("7")),
    ]);

    let config = AppConfig::from_sources(&CliArgs::default()).expect("postgres settings parse");
    let debug = format!("{config:?}");
    let json = serde_json::to_string(&config).expect("config serializes");

    assert_eq!(config.analytics_backend, AnalyticsBackend::Postgres);
    assert_eq!(config.postgres_max_connections, 7);
    assert_eq!(
        config
            .postgres_database_url
            .as_ref()
            .expect("postgres URL")
            .as_str(),
        "postgresql://analytics:secret@example.test/codex"
    );
    assert!(!debug.contains("secret"));
    assert!(!json.contains("secret"));
}

#[test]
fn app_config_rejects_postgres_without_url_and_zero_pool_size() {
    let _guard = APP_CONFIG_ENV_LOCK.blocking_lock();

    let _env = EnvVarGuard::set(&[
        (ENV_ANALYTICS_BACKEND, Some("postgres")),
        (ENV_POSTGRES_DATABASE_URL, None),
        (ENV_POSTGRES_MAX_CONNECTIONS, None),
    ]);
    let missing_url = AppConfig::from_sources(&CliArgs::default())
        .expect_err("postgres backend requires a database URL");
    assert!(missing_url.to_string().contains(ENV_POSTGRES_DATABASE_URL));
    drop(_env);

    let _env = EnvVarGuard::set(&[
        (ENV_ANALYTICS_BACKEND, Some("sqlite")),
        (ENV_POSTGRES_DATABASE_URL, None),
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
