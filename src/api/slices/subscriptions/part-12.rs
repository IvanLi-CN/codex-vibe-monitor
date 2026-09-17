impl SubscriptionHub {
    async fn mark_topic_dirty(&self, topic: &SubscriptionTopic) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        let mut guard = self.state.lock().await;
        if let Some(cached) = guard.topics.get_mut(&topic_key) {
            cached.dirty = true;
            cached.refresh_scheduled = false;
            cached.conversation_overview_refresh_scheduled = false;
            cached.conversation_overview_refresh_in_flight = false;
            cached.conversation_overview_refresh_pending = false;
            cached.invalidate_upstream_account_attempt_refresh();
            cached.latest_live_snapshot = None;
        }
    }

    async fn mark_prompt_cache_topic_dirty_and_schedule_reconcile(
        &self,
        topic: &SubscriptionTopic,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(&topic_key)
            .copied()
            .unwrap_or_default()
            > 0;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        cached.dirty = true;
        cached.refresh_scheduled = false;
        cached.prompt_cache_refresh_scheduled = false;
        cached.prompt_cache_reconcile_required = true;
        cached.prompt_cache_pressure_deferred = false;
        if active && !cached.prompt_cache_reconcile_scheduled {
            cached.prompt_cache_reconcile_scheduled = true;
            return true;
        }
        false
    }

    async fn prompt_cache_topic_reconcile_delay(
        &self,
        topic: &SubscriptionTopic,
    ) -> Option<Duration> {
        let topic_key = topic.cache_key().ok()?;
        let guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(&topic_key)
            .copied()
            .unwrap_or_default()
            > 0;
        let cached = guard.topics.get(&topic_key)?;
        if !active || (!cached.dirty && !cached.prompt_cache_reconcile_required) {
            return None;
        }
        Some(
            cached
                .prompt_cache_baseline_at
                .map(|baseline_at| {
                    PROMPT_CACHE_TOPIC_RECONCILE_INTERVAL.saturating_sub(baseline_at.elapsed())
                })
                .unwrap_or_default(),
        )
    }

    async fn begin_prompt_cache_topic_reconcile(&self, topic: &SubscriptionTopic) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(&topic_key)
            .copied()
            .unwrap_or_default()
            > 0;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if !active
            || cached.prompt_cache_reconcile_scheduled
            || (!cached.dirty && !cached.prompt_cache_reconcile_required)
        {
            return false;
        }
        cached.prompt_cache_reconcile_scheduled = true;
        true
    }

    async fn begin_conversation_overview_topic_refresh(&self, topic: &SubscriptionTopic) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if !cached.conversation_overview_refresh_scheduled {
            return false;
        }
        cached.conversation_overview_refresh_in_flight = true;
        true
    }

    async fn finish_conversation_overview_topic_refresh(&self, topic: &SubscriptionTopic) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        let rerun = cached.conversation_overview_refresh_pending;
        cached.conversation_overview_refresh_scheduled = false;
        cached.conversation_overview_refresh_in_flight = false;
        cached.conversation_overview_refresh_pending = false;
        rerun
    }

    async fn rearm_conversation_overview_topic_refresh(&self, topic: &SubscriptionTopic) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(&topic_key)
            .copied()
            .unwrap_or_default();
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if active == 0 {
            cached.dirty = true;
            return false;
        }
        cached.conversation_overview_refresh_scheduled = true;
        true
    }

    async fn schedule_conversation_overview_topic_refresh(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
    ) -> Result<(), ApiError> {
        let topic_key = topic.cache_key()?;
        let delay = {
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default();
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            if active == 0 {
                cached.dirty = true;
                return Ok(());
            }
            if cached.conversation_overview_refresh_scheduled {
                if cached.conversation_overview_refresh_in_flight {
                    cached.conversation_overview_refresh_pending = true;
                }
                return Ok(());
            }
            cached.conversation_overview_refresh_scheduled = true;
            CONVERSATION_OVERVIEW_TOPIC_REFRESH_DEBOUNCE
        };
        let hub = state.subscription_hub.clone();
        tokio::spawn(async move {
            let mut delay = delay;
            loop {
                tokio::time::sleep(delay).await;
                if !hub.begin_conversation_overview_topic_refresh(&topic).await {
                    return;
                }
                if !hub.has_active_topic_key(&topic_key).await {
                    tracing::debug!(
                        topic = %topic.name(),
                        refresh_outcome = "marked_dirty",
                        "skipping deferred conversation overview refresh without owner subscribers"
                    );
                    hub.finish_conversation_overview_topic_refresh(&topic).await;
                    hub.mark_topic_dirty(&topic).await;
                    return;
                }
                let result = hub
                    .refresh_topic_if_active(state.clone(), topic.clone(), true)
                    .await;
                let rerun = hub.finish_conversation_overview_topic_refresh(&topic).await;
                match result {
                    Ok(Some(_)) | Ok(None) => {}
                    Err(err) => {
                        warn!(
                            ?err,
                            topic = %topic.name(),
                            refresh_outcome = "retained_last_good",
                            "conversation overview topic refresh failed"
                        );
                    }
                }
                if !rerun || !hub.rearm_conversation_overview_topic_refresh(&topic).await {
                    return;
                }
                delay = CONVERSATION_OVERVIEW_TOPIC_REFRESH_DEBOUNCE;
            }
        });
        Ok(())
    }

    async fn begin_upstream_account_attempt_topic_refresh(
        &self,
        topic: &SubscriptionTopic,
        generation: u64,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if cached.upstream_account_attempt_refresh_generation != generation
            || !cached.upstream_account_attempt_refresh_scheduled
            || cached.upstream_account_attempt_refresh_in_flight
        {
            return false;
        }
        cached.upstream_account_attempt_refresh_in_flight = true;
        true
    }

    async fn finish_upstream_account_attempt_topic_refresh(
        &self,
        topic: &SubscriptionTopic,
        generation: u64,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if cached.upstream_account_attempt_refresh_generation != generation {
            return false;
        }
        let rerun = cached.upstream_account_attempt_refresh_pending;
        cached.upstream_account_attempt_refresh_in_flight = false;
        cached.upstream_account_attempt_refresh_pending = false;
        if !rerun {
            cached.upstream_account_attempt_refresh_scheduled = false;
        }
        rerun
    }

    async fn retain_upstream_account_attempt_topic_last_good_for_retry(
        &self,
        topic: &SubscriptionTopic,
        generation: u64,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if cached.upstream_account_attempt_refresh_generation != generation {
            return false;
        }
        cached.dirty = true;
        cached.refresh_scheduled = false;
        cached.runtime_topic_recovery_retry_at = None;
        true
    }

    async fn rearm_upstream_account_attempt_topic_refresh(
        &self,
        topic: &SubscriptionTopic,
        generation: u64,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(&topic_key)
            .copied()
            .unwrap_or_default();
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if cached.upstream_account_attempt_refresh_generation != generation {
            return false;
        }
        if active == 0 {
            cached.dirty = true;
            cached.invalidate_upstream_account_attempt_refresh();
            return false;
        }
        cached.upstream_account_attempt_refresh_scheduled = true;
        true
    }

    async fn finish_upstream_account_attempt_topic_refresh_without_owner(
        &self,
        topic: &SubscriptionTopic,
        generation: u64,
    ) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        let mut guard = self.state.lock().await;
        let active = guard
            .active_subscribers
            .get(&topic_key)
            .copied()
            .unwrap_or_default()
            > 0;
        let Some(cached) = guard.topics.get_mut(&topic_key) else {
            return false;
        };
        if cached.upstream_account_attempt_refresh_generation != generation {
            return false;
        }
        if active {
            // A reconnect acquired the owner between the task's wake-up and this check. Keep
            // the refresh marked in-flight so matching events during the database build are
            // coalesced into its follow-up snapshot.
            cached.upstream_account_attempt_refresh_scheduled = true;
            return true;
        }
        cached.dirty = true;
        cached.refresh_scheduled = false;
        cached.invalidate_upstream_account_attempt_refresh();
        cached.latest_live_snapshot = None;
        false
    }

    async fn schedule_upstream_account_attempt_topic_refresh(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
    ) -> Result<(), ApiError> {
        let topic_key = topic.cache_key()?;
        let generation = {
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default();
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            if active == 0 {
                cached.dirty = true;
                return Ok(());
            }
            if cached.upstream_account_attempt_refresh_scheduled {
                if cached.upstream_account_attempt_refresh_in_flight {
                    cached.upstream_account_attempt_refresh_pending = true;
                }
                return Ok(());
            }
            cached.upstream_account_attempt_refresh_scheduled = true;
            cached.upstream_account_attempt_refresh_generation = cached
                .upstream_account_attempt_refresh_generation
                .saturating_add(1);
            cached.upstream_account_attempt_refresh_generation
        };

        let hub = state.subscription_hub.clone();
        tokio::spawn(async move {
            let mut delay = UPSTREAM_ACCOUNT_ATTEMPTS_TOPIC_REFRESH_DEBOUNCE;
            loop {
                tokio::time::sleep(delay).await;
                if !hub
                    .begin_upstream_account_attempt_topic_refresh(&topic, generation)
                    .await
                {
                    return;
                }
                if !hub
                    .finish_upstream_account_attempt_topic_refresh_without_owner(&topic, generation)
                    .await
                {
                    return;
                }
                let result = hub
                    .refresh_upstream_account_attempt_topic_if_active(
                        state.clone(),
                        topic.clone(),
                        true,
                        generation,
                    )
                    .await;
                let rerun = hub
                    .finish_upstream_account_attempt_topic_refresh(&topic, generation)
                    .await;
                if let Err(err) = result {
                    if !hub
                        .retain_upstream_account_attempt_topic_last_good_for_retry(
                            &topic, generation,
                        )
                        .await
                    {
                        return;
                    }
                    warn!(
                        ?err,
                        topic = %topic.name(),
                        refresh_outcome = "retained_last_good",
                        "upstream account attempt topic refresh failed"
                    );
                    if !hub
                        .rearm_upstream_account_attempt_topic_refresh(&topic, generation)
                        .await
                    {
                        return;
                    }
                    delay = RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF;
                    continue;
                }
                if !rerun
                    || !hub
                        .rearm_upstream_account_attempt_topic_refresh(&topic, generation)
                        .await
                {
                    return;
                }
                delay = UPSTREAM_ACCOUNT_ATTEMPTS_TOPIC_REFRESH_DEBOUNCE;
            }
        });
        Ok(())
    }

    async fn schedule_parallel_work_topic_projection(
        &self,
        state: Arc<AppState>,
        topic: SubscriptionTopic,
        mutations: &[SequencedRuntimeMutation],
    ) -> Result<(), ApiError> {
        let topic_key = topic.cache_key()?;
        loop {
            let Some((materializer, outcome)) = self
                .apply_parallel_work_mutations(&topic_key, mutations)
                .await?
            else {
                return Ok(());
            };
            let schedule = self
                .schedule_parallel_work_mutation(&topic_key, mutations, &materializer, outcome)
                .await?;
            let Some(schedule) = schedule else {
                tokio::task::yield_now().await;
                continue;
            };
            match schedule {
                ParallelWorkSchedule::None => {}
                ParallelWorkSchedule::Materialize(scheduled_at) => {
                    let hub = state.subscription_hub.clone();
                    tokio::spawn(async move {
                        let deadline = scheduled_at + PARALLEL_WORK_TOPIC_MATERIALIZATION_DEBOUNCE;
                        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                        if Instant::now().saturating_duration_since(deadline)
                            > PARALLEL_WORK_TOPIC_MATERIALIZATION_DEBOUNCE
                        {
                            hub.dashboard_topology_counters
                                .record_cadence_miss("stats.parallel-work.current");
                        }
                        hub.materialize_dashboard_parallel_work().await;
                    });
                }
                ParallelWorkSchedule::Reconcile => {
                    // Account gaps are recovered from a dirty last-good frame, not by the
                    // in-memory hot materializer. Recovery health reports that state directly;
                    // this background rebaseline delay is therefore not a hot cadence miss.
                    let hub = state.subscription_hub.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(PARALLEL_WORK_TOPIC_MATERIALIZATION_DEBOUNCE).await;
                        let result = hub
                            .refresh_topic_if_active(state.clone(), topic.clone(), true)
                            .await;
                        if let Err(err) = result {
                            warn!(
                                ?err,
                                topic = %topic.name(),
                                "failed to reconcile account-scoped parallel-work projection"
                            );
                            hub.schedule_parallel_work_reconcile_retry(state, &topic)
                                .await;
                        } else if hub.parallel_work_reconcile_pending(&topic).await {
                            hub.schedule_parallel_work_reconcile_retry(state, &topic)
                                .await;
                        }
                    });
                }
            }
            return Ok(());
        }
    }

    async fn apply_parallel_work_mutations(
        &self,
        topic_key: &str,
        mutations: &[SequencedRuntimeMutation],
    ) -> Result<
        Option<(
            Arc<StdMutex<DashboardParallelWorkMaterializerState>>,
            ParallelWorkMutationOutcome,
        )>,
        ApiError,
    > {
        let materializer = {
            let mut guard = self.state.lock().await;
            if guard
                .active_subscribers
                .get(topic_key)
                .copied()
                .unwrap_or_default()
                == 0
            {
                if let Some(cached) = guard.topics.get_mut(topic_key) {
                    cached.dirty = true;
                }
                return Ok(None);
            }
            if !guard.topics.contains_key(topic_key) || guard.topics[topic_key].dirty {
                buffer_parallel_work_prebaseline_mutations(&mut guard, topic_key, mutations);
                return Ok(None);
            }
            let cached = guard
                .topics
                .get_mut(topic_key)
                .expect("cached topic exists");
            let Some(DashboardTopicMaterializer::ParallelWork { base }) =
                cached.dashboard_materializer.as_ref()
            else {
                return Ok(None);
            };
            base.clone()
        };
        let mut base = materializer
            .lock()
            .expect("parallel-work materializer state lock");
        let mut outcome = ParallelWorkMutationOutcome::default();
        for mutation in mutations {
            let RuntimeMutation::Invocation(mutation) = &mutation.mutation else {
                continue;
            };
            let mutation_outcome = base.apply_runtime_mutation(mutation);
            outcome.changed |= mutation_outcome.changed;
            outcome.needs_account_reconcile |= mutation_outcome.needs_account_reconcile;
        }
        drop(base);
        Ok(Some((materializer, outcome)))
    }

    async fn schedule_parallel_work_mutation(
        &self,
        topic_key: &str,
        mutations: &[SequencedRuntimeMutation],
        materializer: &Arc<StdMutex<DashboardParallelWorkMaterializerState>>,
        outcome: ParallelWorkMutationOutcome,
    ) -> Result<Option<ParallelWorkSchedule>, ApiError> {
        let mut guard = self.state.lock().await;
        if guard
            .active_subscribers
            .get(topic_key)
            .copied()
            .unwrap_or_default()
            == 0
        {
            if let Some(cached) = guard.topics.get_mut(topic_key) {
                cached.dirty = true;
            }
            return Ok(None);
        }
        remove_parallel_work_prebaseline_mutations(&mut guard, topic_key, mutations);
        if !guard.topics.contains_key(topic_key) || guard.topics[topic_key].dirty {
            buffer_parallel_work_prebaseline_mutations(&mut guard, topic_key, mutations);
            return Ok(None);
        }
        if outcome.needs_account_reconcile {
            buffer_parallel_work_prebaseline_mutations(&mut guard, topic_key, mutations);
        }
        let cached = guard
            .topics
            .get_mut(topic_key)
            .expect("cached topic exists");
        if !matches!(
            cached.dashboard_materializer.as_ref(),
            Some(DashboardTopicMaterializer::ParallelWork { base }) if Arc::ptr_eq(base, materializer)
        ) {
            return Ok(None);
        }
        let schedule = if outcome.needs_account_reconcile {
            if cached.parallel_work_refresh_scheduled {
                ParallelWorkSchedule::None
            } else {
                cached.dirty = true;
                cached.parallel_work_refresh_scheduled = true;
                ParallelWorkSchedule::Reconcile
            }
        } else if !outcome.changed || cached.parallel_work_refresh_scheduled {
            ParallelWorkSchedule::None
        } else {
            cached.parallel_work_refresh_scheduled = true;
            ParallelWorkSchedule::Materialize(Instant::now())
        };
        Ok(Some(schedule))
    }

    async fn schedule_parallel_work_reconcile_retry(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
    ) {
        let Ok(topic_key) = topic.cache_key() else {
            return;
        };
        let recovery_scheduled = {
            let mut guard = self.state.lock().await;
            let active = guard
                .active_subscribers
                .get(&topic_key)
                .copied()
                .unwrap_or_default()
                > 0;
            let has_pending_mutations = guard
                .parallel_work_prebaseline_mutations
                .get(&topic_key)
                .is_some_and(|mutations| !mutations.is_empty());
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return;
            };
            cached.parallel_work_refresh_scheduled = false;
            if has_pending_mutations {
                cached.dirty = true;
            }
            if !active {
                return;
            }
            cached.runtime_topic_recovery_retry_at =
                Some(Instant::now() + RUNTIME_TOPIC_RECOVERY_RETRY_BACKOFF);
            if guard.runtime_topic_recovery_running {
                false
            } else {
                guard.runtime_topic_recovery_running = true;
                true
            }
        };
        if recovery_scheduled {
            let hub = state.subscription_hub.clone();
            tokio::spawn(async move {
                hub.run_runtime_topic_recovery(state).await;
            });
        }
        self.runtime_topic_recovery_notify.notify_one();
    }

    async fn parallel_work_reconcile_pending(&self, topic: &SubscriptionTopic) -> bool {
        let Ok(topic_key) = topic.cache_key() else {
            return false;
        };
        self.state
            .lock()
            .await
            .parallel_work_prebaseline_mutations
            .get(&topic_key)
            .is_some_and(|mutations| !mutations.is_empty())
    }
}
enum ParallelWorkSchedule {
    None,
    Materialize(Instant),
    Reconcile,
}
