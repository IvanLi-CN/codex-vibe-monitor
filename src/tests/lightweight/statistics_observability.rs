use super::*;
use crate::observability::ObservabilityRuntime;
use crate::observability::contention::{
    QueryOutcome, StatsQuery, SummaryReason, SummaryResult, observe_query, with_performance,
};
use anyhow::anyhow;
use axum::Json;

fn summary_response(degraded: bool) -> Json<StatsResponse> {
    let mut response = StatsTotals::default().into_response();
    if degraded {
        response.data_quality = Some(StatsDataQualityResponse::summary_delta_journal_pending());
    }
    Json(response)
}

fn has_series(text: &str, name: &str, labels: &[&str], value: &str) -> bool {
    text.lines().any(|line| {
        line.starts_with(&format!("{name}{{"))
            && labels.iter().all(|label| line.contains(label))
            && line.ends_with(value)
    })
}

#[tokio::test]
async fn statistics_observation_exports_summary_outcomes_and_repeated_queries() {
    let runtime = ObservabilityRuntime::new(true);
    with_performance(runtime.clone(), async {
        let success = Ok::<_, ApiError>(summary_response(false));
        let mut observer = SummaryResponseObservation::new();
        observer.finish(&success);
        drop(observer);

        let degraded = Ok::<_, ApiError>(summary_response(true));
        let mut observer = SummaryResponseObservation::new();
        observer.finish(&degraded);
        drop(observer);

        let unavailable = Err::<Json<StatsResponse>, _>(ApiError::unavailable(anyhow!(
            "summary projection has not completed hydration"
        )));
        let mut observer = SummaryResponseObservation::new();
        observer.finish(&unavailable);
        drop(observer);

        let bad_request =
            Err::<Json<StatsResponse>, _>(ApiError::bad_request(anyhow!("invalid summary window")));
        let mut observer = SummaryResponseObservation::new();
        observer.finish(&bad_request);
        drop(observer);

        let repeated = Ok::<_, ApiError>(summary_response(false));
        for _ in 0..2 {
            let mut observer = SummaryResponseObservation::new();
            observer.finish(&repeated);
            drop(observer);
        }

        assert_eq!(
            observe_query(StatsQuery::PromptCacheStatistics, async { Ok::<_, ()>(()) }).await,
            Ok(())
        );
        assert_eq!(
            observe_query(StatsQuery::LongTermOverlap, async {
                Err::<(), _>("fixture")
            })
            .await,
            Err("fixture")
        );
    })
    .await;

    let text = runtime.render();
    assert!(has_series(
        &text,
        "cvm_summary_responses_total",
        &["outcome=\"success\"", "reason=\"none\""],
        " 3"
    ));
    assert!(has_series(
        &text,
        "cvm_summary_responses_total",
        &["outcome=\"degraded\"", "reason=\"source_gap\""],
        " 1"
    ));
    assert!(has_series(
        &text,
        "cvm_summary_responses_total",
        &["outcome=\"unavailable\"", "reason=\"hydration\""],
        " 1"
    ));
    assert!(!text.contains("invalid summary window"));
    assert!(has_series(
        &text,
        "cvm_stats_queries_total",
        &["outcome=\"success\"", "query=\"prompt_cache_statistics\""],
        " 1"
    ));
    assert!(has_series(
        &text,
        "cvm_stats_queries_total",
        &["outcome=\"error\"", "query=\"long_term_overlap\""],
        " 1"
    ));
    assert!(has_series(
        &text,
        "cvm_summary_response_duration_seconds_count",
        &["outcome=\"success\""],
        " 3"
    ));
}

#[tokio::test]
async fn statistics_observation_preserves_cancellation_and_disabled_isolation() {
    let runtime = ObservabilityRuntime::new(true);
    let mut pending = Box::pin(with_performance(
        runtime.clone(),
        observe_query(
            StatsQuery::Summary,
            std::future::pending::<std::result::Result<(), ()>>(),
        ),
    ));
    assert!(futures_util::FutureExt::now_or_never(pending.as_mut()).is_none());
    drop(pending);

    let disabled = ObservabilityRuntime::new(false);
    with_performance(disabled.clone(), async {
        assert_eq!(
            observe_query(StatsQuery::Summary, async { Ok::<_, ()>(()) }).await,
            Ok(())
        );
    })
    .await;

    let text = runtime.render();
    assert!(has_series(
        &text,
        "cvm_stats_queries_total",
        &["outcome=\"cancelled\"", "query=\"summary\""],
        " 1"
    ));
    assert!(!disabled.render().contains("cvm_"));
    assert_eq!(SummaryResult::ALL.len(), 3);
    assert_eq!(SummaryReason::ALL.len(), 6);
    assert_eq!(QueryOutcome::ALL.len(), 3);
}
