pub(crate) async fn probe_subscription_endpoint_with_retries(
    state: &AppState,
    endpoint: &ForwardProxyEndpoint,
    attempts: usize,
    attempt_timeout: Duration,
    validation_timeout: Duration,
    validation_started: Instant,
    cancellation: &CancellationToken,
) -> Result<f64> {
    let mut last_error: Option<anyhow::Error> = None;
    for attempt in 1..=attempts {
        if cancellation.is_cancelled() {
            return Err(shutdown_cancelled_forward_proxy_probe());
        }
        let Some(remaining_timeout) =
            remaining_timeout_budget(validation_timeout, validation_started.elapsed())
        else {
            return Err(timeout_error_for_duration(validation_timeout));
        };
        if remaining_timeout.is_zero() {
            return Err(timeout_error_for_duration(validation_timeout));
        }

        let probe_result = tokio::select! {
            _ = cancellation.cancelled() => {
                return Err(shutdown_cancelled_forward_proxy_probe());
            }
            _ = tokio::time::sleep(remaining_timeout) => {
                return Err(timeout_error_for_duration(validation_timeout));
            }
            result = probe_forward_proxy_endpoint(state, endpoint, attempt_timeout, Some(cancellation)) => {
                result
            }
        };

        match probe_result {
            Ok(Some(latency_ms)) => return Ok(latency_ms),
            Ok(None) => return Err(shutdown_cancelled_forward_proxy_probe()),
            Err(err) => {
                last_error = Some(err.context(format!(
                    "attempt {attempt}/{attempts} failed for {}",
                    endpoint.display_name
                )));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("subscription proxy probe did not run")))
}
