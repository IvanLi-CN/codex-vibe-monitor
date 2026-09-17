fn buffer_parallel_work_prebaseline_mutations(
    state: &mut SubscriptionHubState,
    topic_key: &str,
    mutations: &[SequencedRuntimeMutation],
) {
    let pending_is_empty = {
        let pending = state
            .parallel_work_prebaseline_mutations
            .entry(topic_key.to_string())
            .or_default();
        for mutation in mutations {
            let RuntimeMutation::Invocation(mutation) = &mutation.mutation else {
                continue;
            };
            let identity = format!(
                "{}\0{}",
                mutation.identity.invoke_id, mutation.identity.occurred_at
            );
            if mutation.kind == RuntimeMutationKind::RuntimeRemoved {
                pending.remove(&identity);
            } else {
                pending.insert(identity, mutation.clone());
            }
        }
        pending.is_empty()
    };
    if pending_is_empty {
        state.parallel_work_prebaseline_mutations.remove(topic_key);
    }
}

fn remove_parallel_work_prebaseline_mutations(
    state: &mut SubscriptionHubState,
    topic_key: &str,
    mutations: &[SequencedRuntimeMutation],
) {
    let Some(pending) = state.parallel_work_prebaseline_mutations.get_mut(topic_key) else {
        return;
    };
    for mutation in mutations {
        let RuntimeMutation::Invocation(mutation) = &mutation.mutation else {
            continue;
        };
        if mutation.kind == RuntimeMutationKind::RuntimeRemoved {
            pending.remove(&format!(
                "{}\0{}",
                mutation.identity.invoke_id, mutation.identity.occurred_at
            ));
        }
    }
    if pending.is_empty() {
        state.parallel_work_prebaseline_mutations.remove(topic_key);
    }
}

fn rolling_dashboard_window_requires_rebase(
    base_start: Option<DateTime<Utc>>,
    current_start: Option<DateTime<Utc>>,
) -> bool {
    match (base_start, current_start) {
        (Some(base_start), Some(current_start)) => {
            current_start < base_start
                || current_start - base_start
                    >= ChronoDuration::seconds(DASHBOARD_ACTIVITY_SNAPSHOT_CACHE_TTL_SECS as i64)
        }
        (base_start, current_start) => base_start != current_start,
    }
}

fn mark_dashboard_terminal_window_rebase_topics(guard: &mut SubscriptionHubState) {
    let active_subscribers = guard.active_subscribers.clone();
    for (topic_key, cached) in &mut guard.topics {
        if cached.dirty
            || !cached
                .dashboard_materializer
                .as_ref()
                .is_some_and(DashboardTopicMaterializer::requires_terminal_window_rebase)
        {
            continue;
        }
        cached.dirty = true;
        cached.refresh_scheduled = active_subscribers
            .get(topic_key)
            .copied()
            .unwrap_or_default()
            > 0;
        cached.latest_live_snapshot = None;
    }
}

fn collect_pending_dashboard_topic_materializations(
    guard: &mut SubscriptionHubState,
) -> Vec<PendingDashboardTopicMaterialization> {
    let active_subscribers = guard.active_subscribers.clone();
    let server_push_subscribers = guard.server_push_subscribers.clone();
    let current = guard.dashboard_current_slice.clone();
    let network = guard.dashboard_network_slice.clone();
    let terminal = guard.dashboard_terminal_slice.clone();
    guard
        .topics
        .iter_mut()
        .filter_map(|(topic_key, cached)| {
            let active = active_subscribers
                .get(topic_key)
                .copied()
                .unwrap_or_default()
                > 0
                || server_push_subscribers
                    .get(topic_key)
                    .copied()
                    .unwrap_or_default()
                    > 0;
            if !active {
                cached.dirty = true;
                return None;
            }
            if cached.dirty {
                return None;
            }
            let materializer = cached.dashboard_materializer.as_ref()?;
            let revision = materializer.revision(
                cached.dashboard_base_revision,
                current.as_deref(),
                network.as_deref(),
                terminal.as_deref(),
            )?;
            (cached.dashboard_materialized_revision != Some(revision)).then(|| {
                PendingDashboardTopicMaterialization {
                    topic_key: topic_key.clone(),
                    topic_name: cached.topic.name(),
                    revision,
                    materializer: materializer.clone(),
                }
            })
        })
        .collect()
}

impl Default for SubscriptionHub {
    fn default() -> Self {
        Self::new()
    }
}
