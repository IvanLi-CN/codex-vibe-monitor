use super::*;

fn run_routing_future_with_large_stack<Fut>(future: Fut)
where
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    std::thread::Builder::new()
        .name("routing-failover-large-stack".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build large-stack test runtime")
                .block_on(future)
        })
        .expect("spawn large-stack test worker")
        .join()
        .expect("join large-stack test worker");
}

pub(crate) async fn wait_for_pool_attempt_status(
    pool: &SqlitePool,
    attempt_index: i64,
    expected: &str,
) {
    for _ in 0..100 {
        let status: Option<String> = sqlx::query_scalar(
            r#"
            SELECT status
            FROM pool_upstream_request_attempts
            WHERE attempt_index = ?1
            "#,
        )
        .bind(attempt_index)
        .fetch_optional(pool)
        .await
        .expect("load pool attempt status");
        if status.as_deref() == Some(expected) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for attempt {attempt_index} to reach status {expected}");
}

async fn wait_for_pool_attempt_terminal_observation(
    pool: &SqlitePool,
    invoke_id: &str,
    attempt_index: i64,
    expected_status: &str,
    expected_http_status: i64,
) {
    for _ in 0..100 {
        let observation: Option<(String, Option<i64>)> = sqlx::query_as(
            r#"
            SELECT status, http_status
            FROM pool_upstream_request_attempts
            WHERE invoke_id = ?1
              AND attempt_index = ?2
            "#,
        )
        .bind(invoke_id)
        .bind(attempt_index)
        .fetch_optional(pool)
        .await
        .expect("load pool attempt terminal observation");
        if observation.as_ref().is_some_and(|(status, http_status)| {
            status == expected_status && *http_status == Some(expected_http_status)
        }) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "timed out waiting for invocation {invoke_id} attempt {attempt_index} to reach {expected_status} with HTTP {expected_http_status}"
    );
}

#[path = "routing_failover_terminal_reasoning/capture_and_persistence.rs"]
mod capture_and_persistence;
#[path = "routing_failover_terminal_reasoning/failure_reasoning_and_selection.rs"]
mod failure_reasoning_and_selection;
#[path = "routing_failover_terminal_reasoning/timeout_route_behavior.rs"]
mod timeout_route_behavior;
#[path = "routing_failover_terminal_reasoning/upstream_status_and_failover.rs"]
mod upstream_status_and_failover;
