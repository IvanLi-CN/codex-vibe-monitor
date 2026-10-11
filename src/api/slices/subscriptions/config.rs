use super::*;

pub(crate) const SUBSCRIPTION_REPLAY_WINDOW_SECS: i64 = 60;
pub(crate) const SUBSCRIPTION_REPLAY_MAX_EVENTS_PER_TOPIC: usize = 512;
pub(crate) const SUBSCRIPTION_REPLAY_MAX_BYTES_PER_TOPIC: usize = 1024 * 1024;
pub(crate) const SUBSCRIPTION_REPLAY_MAX_GAP_EVENTS: usize = 128;
pub(crate) const SUBSCRIPTION_REPLAY_MAX_GAP_BYTES: usize = 256 * 1024;
pub(crate) const SUBSCRIPTION_DEFAULT_TIME_ZONE: &str = "Asia/Shanghai";
pub(crate) const SUBSCRIPTION_DEFAULT_DASHBOARD_RECENT_LIMIT: i64 = 16;
pub(crate) const SUBSCRIPTION_DEFAULT_PROMPT_CACHE_RECENT_LIMIT: i64 = 16;
pub(crate) const SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES: i64 = 5;
pub(crate) const SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_PAGE_SIZE: i64 = 20;
pub(crate) const SUBSCRIPTION_DEFAULT_INVOCATION_LIMIT: i64 = 20;
pub(crate) const SUBSCRIPTION_CONVERSATION_HISTORY_LIMIT: i64 = 50;
pub(crate) const SUBSCRIPTION_CONVERSATION_OPERATION_LIMIT: usize = 20;
pub(crate) const SUBSCRIPTION_CONVERSATION_OVERVIEW_MAX_RECORDS: usize = 1_000;
pub(crate) const MANAGED_TASK_TIMELINE_TOPIC_SEGMENT_LIMIT: usize = 10_000;
pub(crate) const UPSTREAM_ACCOUNT_ATTEMPTS_TOPIC_REFRESH_DEBOUNCE: Duration =
    Duration::from_millis(250);
#[cfg(not(test))]
pub(crate) const DASHBOARD_NETWORK_RECENT_TOPIC_PUSH_INTERVAL: Duration = Duration::from_secs(1);
#[cfg(test)]
pub(crate) const DASHBOARD_NETWORK_RECENT_TOPIC_PUSH_INTERVAL: Duration = Duration::from_millis(50);

#[cfg(not(test))]
pub(crate) const DASHBOARD_ACTIVITY_TOPIC_REFRESH_TTL: Duration = Duration::from_secs(5);
#[cfg(test)]
pub(crate) const DASHBOARD_ACTIVITY_TOPIC_REFRESH_TTL: Duration = Duration::from_millis(500);
pub(crate) const SUMMARY_TOPIC_REFRESH_DEBOUNCE: Duration = Duration::from_millis(500);
pub(crate) const PROMPT_CACHE_TOPIC_REFRESH_DEBOUNCE: Duration = Duration::from_millis(500);
#[cfg(not(test))]
pub(crate) const PARALLEL_WORK_TOPIC_MATERIALIZATION_DEBOUNCE: Duration = Duration::from_secs(1);
#[cfg(test)]
pub(crate) const PARALLEL_WORK_TOPIC_MATERIALIZATION_DEBOUNCE: Duration = Duration::from_millis(50);
pub(crate) const PROMPT_CACHE_TOPIC_RECONCILE_INTERVAL: Duration = Duration::from_secs(60);
pub(crate) const RUNTIME_TOPIC_RECOVERY_QUEUE_CAPACITY: usize = 64;
pub(crate) const RUNTIME_TOPIC_RECOVERY_BATCH_SIZE: usize = 8;
pub(crate) const RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF: Duration = Duration::from_secs(1);
pub(crate) const SUBSCRIPTION_INITIAL_TOPIC_BUILD_ATTEMPTS: usize = 3;
pub(crate) const SUMMARY_TERMINAL_OVERLAY_MAX_DELTAS: usize = 10_000;
pub(crate) const SUMMARY_TERMINAL_OVERLAY_MAX_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const SUMMARY_TERMINAL_OVERLAY_MAX_ACCOUNT_OVERFLOW_MARKERS: usize = 1_024;
pub(crate) const SUMMARY_DELTA_JOURNAL_MAX_GAP_PROOFS: usize = 4_096;
#[cfg(test)]
pub(crate) const DASHBOARD_RUNTIME_TOPOLOGY_CONTRACT_REASON: &str =
    "dashboard-runtime-topology-contract";
#[cfg(not(test))]
pub(crate) const CONVERSATION_OVERVIEW_TOPIC_REFRESH_DEBOUNCE: Duration = Duration::from_secs(2);
#[cfg(test)]
pub(crate) const CONVERSATION_OVERVIEW_TOPIC_REFRESH_DEBOUNCE: Duration = Duration::from_millis(50);

pub(crate) fn subscription_calendar_anchor(topic: &SubscriptionTopic) -> Option<String> {
    let SubscriptionTopic::SummaryCurrent {
        window, time_zone, ..
    } = topic
    else {
        return None;
    };
    if !matches!(window.as_str(), "yesterday" | "previous7d") {
        return None;
    }
    let reporting_tz = parse_reporting_tz(Some(time_zone.as_str())).ok()?;
    Some(
        Utc::now()
            .with_timezone(&reporting_tz)
            .date_naive()
            .to_string(),
    )
}

pub(crate) fn subscription_calendar_rollover_delay(topic: &SubscriptionTopic) -> Duration {
    let SubscriptionTopic::SummaryCurrent { time_zone, .. } = topic else {
        return Duration::from_secs(1);
    };
    let reporting_tz = parse_reporting_tz(Some(time_zone.as_str()))
        .unwrap_or_else(|_| "Asia/Shanghai".parse().expect("default timezone is valid"));
    subscription_calendar_rollover_delay_at(topic, Utc::now(), reporting_tz)
}

pub(crate) fn subscription_calendar_rollover_delay_at(
    topic: &SubscriptionTopic,
    now: DateTime<Utc>,
    reporting_tz: Tz,
) -> Duration {
    let SubscriptionTopic::SummaryCurrent { .. } = topic else {
        return Duration::from_secs(1);
    };
    let tomorrow = now.with_timezone(&reporting_tz).date_naive() + ChronoDuration::days(1);
    let next_midnight = local_midnight_utc(tomorrow, reporting_tz);
    (next_midnight - now)
        .to_std()
        .unwrap_or_else(|_| Duration::from_millis(1))
        .max(Duration::from_millis(1))
}
