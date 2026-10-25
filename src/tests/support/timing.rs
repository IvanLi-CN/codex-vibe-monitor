use std::{future::Future, time::Instant};

pub(crate) const STATEFUL_TEST_TIMING_ENV: &str = "CODEX_VIBE_MONITOR_STATEFUL_TEST_TIMING";

pub(crate) fn test_timing_span(phase: &'static str) -> Option<TestTimingSpan> {
    let enabled = std::env::var(STATEFUL_TEST_TIMING_ENV)
        .ok()
        .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"));
    enabled.then(|| TestTimingSpan {
        phase,
        started_at: Instant::now(),
    })
}

pub(crate) async fn timed_test_phase<Fut>(phase: &'static str, future: Fut) -> Fut::Output
where
    Fut: Future,
{
    let _timing = test_timing_span(phase);
    future.await
}

#[macro_export]
macro_rules! test_timed {
    ($phase:expr, $operation:expr) => {{
        let _timing = $crate::tests::test_timing_span($phase);
        $operation
    }};
}

pub(crate) struct TestTimingSpan {
    phase: &'static str,
    started_at: Instant,
}

impl Drop for TestTimingSpan {
    fn drop(&mut self) {
        eprintln!(
            "stateful-test-timing phase={} elapsed_ms={}",
            self.phase,
            self.started_at.elapsed().as_millis()
        );
    }
}
