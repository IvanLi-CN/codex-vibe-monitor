use super::*;

pub(super) fn run_routing_failover_future_with_large_stack<T, Fut>(future: Fut) -> T
where
    T: Send + 'static,
    Fut: std::future::Future<Output = T> + Send + 'static,
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
        .expect("join large-stack test worker")
}

#[path = "routing_failover_retry_budget/failure_persistence_and_capacity.rs"]
mod failure_persistence_and_capacity;
#[path = "routing_failover_retry_budget/failure_timing_and_recovery.rs"]
mod failure_timing_and_recovery;
#[path = "routing_failover_retry_budget/reservation_and_selection.rs"]
mod reservation_and_selection;
#[path = "routing_failover_retry_budget/retry_and_capture.rs"]
mod retry_and_capture;
