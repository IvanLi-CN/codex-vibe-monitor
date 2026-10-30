use crate::{
    configure_offline_maintenance_business_pool_options, configure_primary_sqlite_pool_options,
    connect_primary_sqlite_connection,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

#[tokio::test]
async fn primary_sqlite_pool_enables_recursive_triggers_on_each_connection() {
    let pool = configure_primary_sqlite_pool_options(SqlitePoolOptions::new().max_connections(2))
        .connect("sqlite::memory:")
        .await
        .expect("primary SQLite pool should open");
    let mut first = pool.acquire().await.expect("first SQLite connection");
    let mut second = pool.acquire().await.expect("second SQLite connection");

    let first_value = sqlx::query_scalar::<_, i64>("PRAGMA recursive_triggers")
        .fetch_one(&mut *first)
        .await
        .expect("first connection should expose recursive_triggers");
    let second_value = sqlx::query_scalar::<_, i64>("PRAGMA recursive_triggers")
        .fetch_one(&mut *second)
        .await
        .expect("second connection should expose recursive_triggers");

    assert_eq!(first_value, 1);
    assert_eq!(second_value, 1);
}

#[tokio::test]
async fn offline_maintenance_business_pool_enables_recursive_triggers_on_each_connection() {
    let pool = configure_offline_maintenance_business_pool_options()
        .connect("sqlite::memory:")
        .await
        .expect("offline maintenance business pool should open");
    let mut first = pool.acquire().await.expect("first SQLite connection");
    let mut second = pool.acquire().await.expect("second SQLite connection");

    let first_value = sqlx::query_scalar::<_, i64>("PRAGMA recursive_triggers")
        .fetch_one(&mut *first)
        .await
        .expect("first offline maintenance connection should expose recursive_triggers");
    let second_value = sqlx::query_scalar::<_, i64>("PRAGMA recursive_triggers")
        .fetch_one(&mut *second)
        .await
        .expect("second offline maintenance connection should expose recursive_triggers");

    assert_eq!(first_value, 1);
    assert_eq!(second_value, 1);
}

#[tokio::test]
async fn startup_recovery_sqlite_connection_enables_recursive_triggers() {
    let options = "sqlite::memory:"
        .parse::<SqliteConnectOptions>()
        .expect("startup recovery SQLite options should parse");
    let mut connection = connect_primary_sqlite_connection(&options)
        .await
        .expect("startup recovery SQLite connection should open");

    let value = sqlx::query_scalar::<_, i64>("PRAGMA recursive_triggers")
        .fetch_one(&mut connection)
        .await
        .expect("startup recovery connection should expose recursive_triggers");

    assert_eq!(value, 1);
}
