use std::{future::Future, time::Instant};

pub(crate) const STATEFUL_TEST_TIMING_ENV: &str = "CODEX_VIBE_MONITOR_STATEFUL_TEST_TIMING";

fn timing_enabled() -> bool {
    std::env::var(STATEFUL_TEST_TIMING_ENV)
        .ok()
        .is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
}

pub(crate) fn test_timing_span(phase: &'static str) -> Option<TestTimingSpan> {
    timing_enabled().then(|| TestTimingSpan {
        phase,
        started_at: Instant::now(),
    })
}

pub(crate) fn test_timing_target(target: &'static str) -> Option<TestTimingTarget> {
    timing_enabled().then(|| {
        eprintln!("stateful-test-timing target_start={target}");
        TestTimingTarget { target }
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

pub(crate) struct TestTimingTarget {
    target: &'static str,
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

impl Drop for TestTimingTarget {
    fn drop(&mut self) {
        eprintln!("stateful-test-timing target_end={}", self.target);
    }
}
