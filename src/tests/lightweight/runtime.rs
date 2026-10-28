use crate::runtime::configure_primary_sqlite_pool_options;
use sqlx::sqlite::SqlitePoolOptions;

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
