impl DashboardWorkingConversationsMaterializerState {
    fn new(
        response: PromptCacheConversationsResponse,
        page_size: i64,
        recent_invocation_limit: i64,
        blocked_binding_filter: Option<PromptCacheConversationBlockedBindingFilter>,
    ) -> Self {
        let baseline_has_more = response.has_more;
        let in_flight_phase_counts_by_key = response
            .conversations
            .iter()
            .map(|conversation| {
                (
                    conversation.prompt_cache_key.clone(),
                    conversation.in_flight_phase_counts,
                )
            })
            .collect();
        Self {
            response,
            page_size: page_size.max(0) as usize,
            recent_invocation_limit: recent_invocation_limit.max(0) as usize,
            blocked_binding_filter,
            baseline_has_more,
            in_flight_by_identity: HashMap::new(),
            in_flight_phase_counts_by_key,
        }
    }

    fn install_in_flight_phase_records(
        &mut self,
        records: impl IntoIterator<Item = PromptCacheInFlightPhaseRecord>,
    ) {
        self.in_flight_by_identity.clear();
        self.in_flight_phase_counts_by_key.clear();
        for record in records {
            self.in_flight_phase_counts_by_key
                .entry(record.prompt_cache_key.clone())
                .or_default()
                .increment_phase_name(record.phase.as_deref());
            self.in_flight_by_identity
                .insert(record.identity.clone(), record);
        }
        self.sync_in_flight_phase_counts_to_response();
    }

    fn sync_in_flight_phase_counts_to_response(&mut self) {
        for conversation in &mut self.response.conversations {
            conversation.in_flight_phase_counts = self
                .in_flight_phase_counts_by_key
                .get(&conversation.prompt_cache_key)
                .copied()
                .unwrap_or_default();
        }
    }

    fn apply_in_flight_phase_delta(&mut self, record: &PromptCacheTopicDelta) -> bool {
        let mut affected_keys = BTreeSet::new();
        if let Some(previous) = self.in_flight_by_identity.remove(&record.identity) {
            self.in_flight_phase_counts_by_key
                .entry(previous.prompt_cache_key.clone())
                .or_default()
                .decrement_phase_name(previous.phase.as_deref());
            affected_keys.insert(previous.prompt_cache_key);
        }
        let is_in_flight = !record.is_runtime_removed
            && !record.is_terminal
            && matches!(
                record.status.trim().to_ascii_lowercase().as_str(),
                "running" | "pending"
            );
        if is_in_flight
            && let Some(prompt_cache_key) = record
                .prompt_cache_key
                .as_deref()
                .filter(|key| !key.is_empty())
        {
            let phase = record
                .preview
                .as_ref()
                .and_then(|preview| preview.live_phase.clone());
            let entry = PromptCacheInFlightPhaseRecord {
                identity: record.identity.clone(),
                prompt_cache_key: prompt_cache_key.to_string(),
                phase,
            };
            self.in_flight_phase_counts_by_key
                .entry(entry.prompt_cache_key.clone())
                .or_default()
                .increment_phase_name(entry.phase.as_deref());
            affected_keys.insert(entry.prompt_cache_key.clone());
            self.in_flight_by_identity
                .insert(entry.identity.clone(), entry);
        }
        let mut changed = false;
        for key in affected_keys {
            if let Some(conversation) = self
                .response
                .conversations
                .iter_mut()
                .find(|conversation| conversation.prompt_cache_key == key)
            {
                let next = self
                    .in_flight_phase_counts_by_key
                    .get(&key)
                    .copied()
                    .unwrap_or_default();
                if conversation.in_flight_phase_counts != next {
                    conversation.in_flight_phase_counts = next;
                    changed = true;
                }
            }
        }
        changed
    }

    fn apply_deltas(
        &mut self,
        records: &[PromptCacheTopicDelta],
        applied_terminal_ids: &mut HashSet<String>,
        baseline_row_id: i64,
    ) -> Result<WorkingConversationsProjectionUpdate, ApiError> {
        let now = Utc::now();
        if let Some(update) =
            self.preflight_delta_update(records, applied_terminal_ids, baseline_row_id, now)
        {
            return Ok(update);
        }

        let mut changed = false;

        for record in records {
            changed |= self.apply_in_flight_phase_delta(record);
        }

        for record in records {
            let Some(prompt_cache_key) = record.prompt_cache_key.as_deref() else {
                continue;
            };
            let conversation_index = self
                .response
                .conversations
                .iter()
                .position(|conversation| conversation.prompt_cache_key == prompt_cache_key);

            if record.is_runtime_removed {
                if let Some(index) = conversation_index {
                    changed |= self.remove_runtime_preview(index, record);
                }
                continue;
            }

            let Some(preview) = record.preview.as_ref() else {
                unreachable!("bounded hydration preflight must handle incomplete deltas");
            };
            if !self.matches_blocked_binding_filter(record, preview)
                || !Self::is_in_working_window(record, now)
            {
                continue;
            }
            let conversation_index = conversation_index.expect(
                "bounded hydration preflight must hydrate a missing working conversation key",
            );

            let conversation = &mut self.response.conversations[conversation_index];
            changed |= apply_working_conversation_delta(
                conversation,
                record,
                preview,
                self.recent_invocation_limit,
                applied_terminal_ids,
                baseline_row_id,
            );
        }

        if changed {
            self.refresh_pagination();
        }

        Ok(if changed {
            WorkingConversationsProjectionUpdate::Changed
        } else {
            WorkingConversationsProjectionUpdate::Unchanged
        })
    }

    fn preflight_delta_update(
        &self,
        records: &[PromptCacheTopicDelta],
        applied_terminal_ids: &HashSet<String>,
        baseline_row_id: i64,
        now: DateTime<Utc>,
    ) -> Option<WorkingConversationsProjectionUpdate> {
        let mut bounded_hydration_keys = BTreeSet::new();
        let mut reconcile_required = false;
        for record in records {
            let Some(prompt_cache_key) = record.prompt_cache_key.as_deref() else {
                continue;
            };
            if record.is_runtime_removed {
                continue;
            }
            let conversation = self
                .response
                .conversations
                .iter()
                .find(|conversation| conversation.prompt_cache_key == prompt_cache_key);
            let conversation_is_visible = conversation.is_some();
            let Some(preview) = record.preview.as_ref() else {
                reconcile_required = true;
                continue;
            };
            if !self.matches_blocked_binding_filter(record, preview)
                || !Self::is_in_working_window(record, now)
            {
                if conversation_is_visible {
                    bounded_hydration_keys.insert(prompt_cache_key.to_string());
                }
                continue;
            }
            let needs_hydration = !conversation_is_visible
                || conversation.is_some_and(|conversation| {
                    Self::delta_requires_account_hydration(
                        conversation,
                        record,
                        applied_terminal_ids,
                        baseline_row_id,
                    )
                });
            if needs_hydration {
                bounded_hydration_keys.insert(prompt_cache_key.to_string());
            }
        }
        if reconcile_required {
            Some(WorkingConversationsProjectionUpdate::NeedsReconcile)
        } else if !bounded_hydration_keys.is_empty() {
            Some(
                WorkingConversationsProjectionUpdate::NeedsBoundedKeyHydration(
                    bounded_hydration_keys,
                ),
            )
        } else {
            None
        }
    }

    fn apply_binding(
        &mut self,
        prompt_cache_key: &str,
        binding: &PromptCacheConversationBindingResponse,
    ) -> Option<bool> {
        let conversation = self
            .response
            .conversations
            .iter_mut()
            .find(|conversation| conversation.prompt_cache_key == prompt_cache_key)?;
        let manual_binding = (binding.binding_kind != "none").then(|| {
            PromptCacheConversationManualBindingResponse {
                binding_kind: binding.binding_kind.clone(),
                group_name: binding.group_name.clone(),
                upstream_account_id: binding.upstream_account_id,
                upstream_account_name: binding.upstream_account_name.clone(),
            }
        });
        let changed = conversation.has_encrypted_session_owner
            != binding.has_encrypted_session_owner
            || conversation.encrypted_owner_account_id != binding.encrypted_owner_account_id
            || conversation.encrypted_owner_account_name != binding.encrypted_owner_account_name
            || conversation.encrypted_owner_group_name != binding.encrypted_owner_group_name
            || conversation.manual_binding != manual_binding;
        if changed {
            conversation.has_encrypted_session_owner = binding.has_encrypted_session_owner;
            conversation.encrypted_owner_account_id = binding.encrypted_owner_account_id;
            conversation.encrypted_owner_account_name =
                binding.encrypted_owner_account_name.clone();
            conversation.encrypted_owner_group_name = binding.encrypted_owner_group_name.clone();
            conversation.manual_binding = manual_binding;
        }
        Some(changed)
    }

    fn serialize(&self) -> Result<Vec<u8>, ApiError> {
        serde_json::to_vec(&self.response).map_err(ApiError::from)
    }

    fn matches_blocked_binding_filter(
        &self,
        record: &PromptCacheTopicDelta,
        preview: &PromptCacheConversationInvocationPreviewResponse,
    ) -> bool {
        let Some(filter) = self.blocked_binding_filter.as_ref() else {
            return true;
        };
        if !filter.is_active() || !record.is_terminal {
            return !filter.is_active();
        }
        let Some(blocked_binding) = preview.blocked_binding.as_ref() else {
            return false;
        };
        filter
            .upstream_account_id
            .is_none_or(|account_id| account_id == blocked_binding.upstream_account_id)
            && filter
                .constraint_source
                .is_none_or(|source| source == blocked_binding.constraint_source)
    }

    fn is_in_working_window(record: &PromptCacheTopicDelta, now: DateTime<Utc>) -> bool {
        !record.is_terminal
            || parse_to_utc_datetime(&record.occurred_at).is_some_and(|occurred_at| {
                occurred_at
                    >= now
                        - ChronoDuration::minutes(
                            SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES,
                        )
            })
    }

    fn delta_requires_account_hydration(
        conversation: &PromptCacheConversationResponse,
        record: &PromptCacheTopicDelta,
        applied_terminal_ids: &HashSet<String>,
        baseline_row_id: i64,
    ) -> bool {
        let already_terminal = applied_terminal_ids.contains(&record.identity)
            || (record.row_id > 0 && record.row_id <= baseline_row_id);
        if !record.is_terminal
            || already_terminal
            || conversation.upstream_accounts.len()
                < PROMPT_CACHE_CONVERSATION_UPSTREAM_ACCOUNT_LIMIT
        {
            return false;
        }
        let account_group_key = resolve_prompt_cache_upstream_account_group_key(
            record.upstream_account_id,
            record.upstream_account_name.as_deref(),
        );
        !conversation.upstream_accounts.iter().any(|account| {
            resolve_prompt_cache_upstream_account_group_key(
                account.upstream_account_id,
                account.upstream_account_name.as_deref(),
            ) == account_group_key
        })
    }

    fn delta_is_eligible(&self, record: &PromptCacheTopicDelta, now: DateTime<Utc>) -> bool {
        !record.is_runtime_removed
            && record.preview.as_ref().is_some_and(|preview| {
                self.matches_blocked_binding_filter(record, preview)
                    && Self::is_in_working_window(record, now)
            })
    }

    fn replace_hydrated_conversation(
        &mut self,
        prompt_cache_key: &str,
        hydrated: Option<PromptCacheConversationResponse>,
    ) -> bool {
        let existing_index = self
            .response
            .conversations
            .iter()
            .position(|conversation| conversation.prompt_cache_key == prompt_cache_key);
        let changed = match (existing_index, hydrated) {
            (Some(index), Some(hydrated)) => {
                if self.response.conversations[index] == hydrated {
                    false
                } else {
                    self.response.conversations[index] = hydrated;
                    true
                }
            }
            (Some(index), None) => {
                self.response.conversations.remove(index);
                true
            }
            (None, Some(hydrated)) => {
                self.response.conversations.push(hydrated);
                true
            }
            (None, None) => false,
        };
        if changed {
            self.refresh_pagination();
        }
        changed
    }

    fn set_total_matched(&mut self, total_matched: i64) -> bool {
        if self.response.total_matched == Some(total_matched) {
            return false;
        }
        self.response.total_matched = Some(total_matched);
        self.refresh_pagination();
        true
    }

    fn visible_keys(&self) -> HashSet<String> {
        self.response
            .conversations
            .iter()
            .map(|conversation| conversation.prompt_cache_key.clone())
            .collect()
    }

    fn refresh_pagination(&mut self) {
        // The cold baseline owns continuation cursor semantics. Live deltas may reorder the
        // active display page, but they never manufacture a cursor for that changed ordering:
        // such a cursor would be applied to the original database snapshot and skip or repeat
        // rows. Fresh page-size subscriptions establish a new baseline when needed.
        sort_working_conversation_responses(&mut self.response.conversations);
        let overflowed = self.response.conversations.len() > self.page_size;
        self.response.conversations.truncate(self.page_size);
        self.response.has_more = self.baseline_has_more
            || overflowed
            || self.response.total_matched.is_some_and(|total_matched| {
                total_matched > self.response.conversations.len() as i64
            });
        for conversation in &mut self.response.conversations {
            conversation.cursor = None;
        }
        self.response.next_cursor = None;
    }

    fn remove_runtime_preview(
        &mut self,
        conversation_index: usize,
        record: &PromptCacheTopicDelta,
    ) -> bool {
        let conversation = &mut self.response.conversations[conversation_index];
        let before = conversation.recent_invocations.len();
        conversation.recent_invocations.retain(|preview| {
            preview.invoke_id != record.invoke_id
                || !working_timestamp_cmp(&preview.occurred_at, &record.occurred_at).is_eq()
        });
        if conversation.recent_invocations.len() == before {
            return false;
        }

        conversation.last_in_flight_at = working_conversation_latest_in_flight_at(conversation);
        conversation.blocked_binding = conversation
            .recent_invocations
            .iter()
            .find_map(|preview| preview.blocked_binding.clone());
        if let Some(last_activity_at) = working_conversation_latest_activity_at(conversation) {
            conversation.last_activity_at = last_activity_at;
        }

        if conversation.request_count == 0
            && let Some(first_preview) = conversation.recent_invocations.last()
        {
            conversation.created_at = first_preview.occurred_at.clone();
        }

        if conversation.request_count == 0 && conversation.recent_invocations.is_empty() {
            self.response.conversations.remove(conversation_index);
            if let Some(total_matched) = self.response.total_matched.as_mut() {
                *total_matched = total_matched.saturating_sub(1);
            }
        }
        true
    }

    fn expire(&mut self, now: DateTime<Utc>) -> bool {
        let activity_cutoff = now
            - ChronoDuration::minutes(SUBSCRIPTION_DEFAULT_WORKING_CONVERSATIONS_ACTIVITY_MINUTES);
        let request_cutoff = now - ChronoDuration::hours(24);
        let before = self.response.conversations.len();
        self.response.conversations.retain(|conversation| {
            conversation.last_in_flight_at.is_some()
                || parse_to_utc_datetime(&conversation.last_activity_at)
                    .is_some_and(|last_activity_at| last_activity_at >= activity_cutoff)
        });
        let removed = before.saturating_sub(self.response.conversations.len());
        if removed > 0
            && let Some(total_matched) = self.response.total_matched.as_mut()
        {
            *total_matched = total_matched.saturating_sub(removed as i64);
        }
        let mut changed = removed > 0;
        for conversation in &mut self.response.conversations {
            let before = conversation.last24h_requests.len();
            conversation.last24h_requests.retain(|point| {
                parse_to_utc_datetime(&point.occurred_at)
                    .is_some_and(|occurred_at| occurred_at >= request_cutoff)
            });
            changed |= conversation.last24h_requests.len() != before;
            let mut cumulative_tokens = 0_i64;
            for point in &mut conversation.last24h_requests {
                cumulative_tokens = cumulative_tokens.saturating_add(point.request_tokens);
                point.cumulative_tokens = cumulative_tokens;
            }
        }
        if changed {
            self.refresh_pagination();
        }
        changed
    }
}

fn apply_working_conversation_delta(
    conversation: &mut PromptCacheConversationResponse,
    record: &PromptCacheTopicDelta,
    preview: &PromptCacheConversationInvocationPreviewResponse,
    recent_invocation_limit: usize,
    applied_terminal_ids: &mut HashSet<String>,
    baseline_row_id: i64,
) -> bool {
    let mut changed = false;
    if let Some(index) = conversation.recent_invocations.iter().position(|existing| {
        existing.invoke_id == preview.invoke_id && existing.occurred_at == preview.occurred_at
    }) {
        if conversation.recent_invocations[index] != *preview {
            conversation.recent_invocations[index] = preview.clone();
            changed = true;
        }
    } else {
        conversation.recent_invocations.push(preview.clone());
        changed = true;
    }
    conversation.recent_invocations.sort_by(|left, right| {
        working_timestamp_cmp(&right.occurred_at, &left.occurred_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    if conversation.recent_invocations.len() > recent_invocation_limit {
        conversation
            .recent_invocations
            .truncate(recent_invocation_limit);
    }

    changed |= update_string_if_newer(&mut conversation.last_activity_at, &record.occurred_at);
    changed |= update_string_if_earlier(&mut conversation.created_at, &record.occurred_at);

    let already_terminal = applied_terminal_ids.contains(&record.identity)
        || (record.row_id > 0 && record.row_id <= baseline_row_id);
    if record.is_terminal && !already_terminal {
        applied_terminal_ids.insert(record.identity.clone());
        apply_working_conversation_terminal_delta(conversation, record, preview);
        changed = true;
    }
    if !record.is_terminal {
        changed |= update_option_if_newer(&mut conversation.last_in_flight_at, &record.occurred_at);
    } else {
        let latest_in_flight = working_conversation_latest_in_flight_at(conversation);
        if conversation.last_in_flight_at != latest_in_flight {
            conversation.last_in_flight_at = latest_in_flight;
            changed = true;
        }
    }

    let blocked_binding = conversation
        .recent_invocations
        .iter()
        .find_map(|candidate| candidate.blocked_binding.clone());
    if conversation.blocked_binding != blocked_binding {
        conversation.blocked_binding = blocked_binding;
        changed = true;
    }
    changed
}

fn working_conversation_latest_in_flight_at(
    conversation: &PromptCacheConversationResponse,
) -> Option<String> {
    conversation
        .recent_invocations
        .iter()
        .filter(|preview| {
            !prompt_invocation_status_counts_toward_terminal_totals(Some(&preview.status))
        })
        .map(|preview| preview.occurred_at.clone())
        .max_by(|left, right| working_timestamp_cmp(left, right))
}

fn working_conversation_latest_activity_at(
    conversation: &PromptCacheConversationResponse,
) -> Option<String> {
    conversation.recent_invocations.iter().fold(
        conversation.last_terminal_at.clone(),
        |latest, preview| match latest {
            Some(latest) if !working_timestamp_cmp(&preview.occurred_at, &latest).is_gt() => {
                Some(latest)
            }
            _ => Some(preview.occurred_at.clone()),
        },
    )
}

fn apply_working_conversation_terminal_delta(
    conversation: &mut PromptCacheConversationResponse,
    record: &PromptCacheTopicDelta,
    preview: &PromptCacheConversationInvocationPreviewResponse,
) {
    conversation.request_count = conversation.request_count.saturating_add(1);
    conversation.total_tokens = conversation
        .total_tokens
        .saturating_add(record.request_tokens.max(0));
    conversation.total_cost += record.cost;
    update_option_if_newer(&mut conversation.last_terminal_at, &record.occurred_at);

    let account_group_key = resolve_prompt_cache_upstream_account_group_key(
        record.upstream_account_id,
        record.upstream_account_name.as_deref(),
    );
    let account_index = conversation
        .upstream_accounts
        .iter()
        .position(|account| {
            resolve_prompt_cache_upstream_account_group_key(
                account.upstream_account_id,
                account.upstream_account_name.as_deref(),
            ) == account_group_key
        })
        .unwrap_or_else(|| {
            conversation
                .upstream_accounts
                .push(PromptCacheConversationUpstreamAccountResponse {
                    upstream_account_id: record.upstream_account_id,
                    upstream_account_name: record.upstream_account_name.clone(),
                    request_count: 0,
                    total_tokens: 0,
                    total_cost: 0.0,
                    last_activity_at: record.occurred_at.clone(),
                });
            conversation.upstream_accounts.len() - 1
        });
    let account = &mut conversation.upstream_accounts[account_index];
    if account.upstream_account_id.is_none() && record.upstream_account_id.is_some() {
        account.upstream_account_id = record.upstream_account_id;
    }
    if account.upstream_account_name.is_none() && record.upstream_account_name.is_some() {
        account.upstream_account_name = record.upstream_account_name.clone();
    }
    account.request_count = account.request_count.saturating_add(1);
    account.total_tokens = account
        .total_tokens
        .saturating_add(record.request_tokens.max(0));
    account.total_cost += record.cost;
    update_string_if_newer(&mut account.last_activity_at, &record.occurred_at);
    conversation.upstream_accounts.sort_by(|left, right| {
        working_timestamp_cmp(&right.last_activity_at, &left.last_activity_at)
            .then_with(|| {
                resolve_prompt_cache_upstream_account_label(
                    right.upstream_account_name.as_deref(),
                    right.upstream_account_id,
                )
                .cmp(&resolve_prompt_cache_upstream_account_label(
                    left.upstream_account_name.as_deref(),
                    left.upstream_account_id,
                ))
            })
            .then_with(|| {
                right
                    .upstream_account_id
                    .unwrap_or(i64::MIN)
                    .cmp(&left.upstream_account_id.unwrap_or(i64::MIN))
            })
            .then_with(|| right.total_tokens.cmp(&left.total_tokens))
            .then_with(|| right.request_count.cmp(&left.request_count))
    });
    conversation
        .upstream_accounts
        .truncate(PROMPT_CACHE_CONVERSATION_UPSTREAM_ACCOUNT_LIMIT);

    append_working_conversation_request_point(conversation, record, preview);
}

fn append_working_conversation_request_point(
    conversation: &mut PromptCacheConversationResponse,
    record: &PromptCacheTopicDelta,
    preview: &PromptCacheConversationInvocationPreviewResponse,
) {
    let outcome = invocation_point_outcome(
        Some(&record.status),
        preview.error_message.as_deref(),
        preview.downstream_error_message.as_deref(),
        preview.failure_kind.as_deref(),
        preview.failure_class.as_deref(),
    )
    .to_string();
    conversation
        .last24h_requests
        .push(PromptCacheConversationRequestPointResponse {
            occurred_at: record.occurred_at.clone(),
            status: if record.status.trim().is_empty() {
                "unknown".to_string()
            } else {
                record.status.clone()
            },
            is_success: outcome == "success",
            outcome,
            request_tokens: record.request_tokens.max(0),
            cumulative_tokens: 0,
        });
    conversation
        .last24h_requests
        .sort_by(|left, right| working_timestamp_cmp(&left.occurred_at, &right.occurred_at));
    let mut cumulative_tokens = 0_i64;
    for point in &mut conversation.last24h_requests {
        cumulative_tokens = cumulative_tokens.saturating_add(point.request_tokens);
        point.cumulative_tokens = cumulative_tokens;
    }
}

fn sort_working_conversation_responses(conversations: &mut [PromptCacheConversationResponse]) {
    conversations.sort_by(|left, right| {
        working_timestamp_cmp(
            working_conversation_sort_anchor(right),
            working_conversation_sort_anchor(left),
        )
        .then_with(|| working_timestamp_cmp(&right.created_at, &left.created_at))
        .then_with(|| right.prompt_cache_key.cmp(&left.prompt_cache_key))
    });
}

fn working_conversation_sort_anchor(conversation: &PromptCacheConversationResponse) -> &str {
    resolve_working_conversation_sort_anchor(
        conversation.last_terminal_at.as_deref(),
        conversation.last_in_flight_at.as_deref(),
        &conversation.created_at,
    )
}

fn working_timestamp_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    match (parse_to_utc_datetime(left), parse_to_utc_datetime(right)) {
        (Some(left), Some(right)) => left.cmp(&right),
        _ => left.cmp(right),
    }
}

fn update_string_if_newer(current: &mut String, candidate: &str) -> bool {
    if working_timestamp_cmp(candidate, current).is_gt() {
        *current = candidate.to_string();
        true
    } else {
        false
    }
}

fn update_string_if_earlier(current: &mut String, candidate: &str) -> bool {
    if working_timestamp_cmp(candidate, current).is_lt() {
        *current = candidate.to_string();
        true
    } else {
        false
    }
}

fn update_option_if_newer(current: &mut Option<String>, candidate: &str) -> bool {
    if current
        .as_deref()
        .is_none_or(|existing| working_timestamp_cmp(candidate, existing).is_gt())
    {
        *current = Some(candidate.to_string());
        true
    } else {
        false
    }
}

#[derive(Debug, Clone)]
enum DashboardTopicMaterializer {
    Activity {
        base: Arc<StdMutex<DashboardActivityMaterializerState>>,
        reporting_tz: Tz,
        source_scope: InvocationSourceScope,
    },
    Summary {
        base: Arc<StdMutex<DashboardSummaryMaterializerState>>,
        window: SummaryWindow,
        reporting_tz: Tz,
        source_scope: InvocationSourceScope,
        upstream_account_id: Option<i64>,
    },
    NetworkTimeseries {
        base: Arc<DashboardNetworkTimeseriesResponse>,
        upstream_account_id: Option<i64>,
    },
    NetworkRecent {
        base: Arc<DashboardRecentNetworkWindowResponse>,
    },
    Timeseries {
        base: Arc<StdMutex<TimeseriesTopicMaterializedBase>>,
        runtime: Arc<RuntimeProjectionHub>,
    },
    ParallelWork {
        base: Arc<StdMutex<DashboardParallelWorkMaterializerState>>,
    },
    WorkingConversations {
        state: Arc<StdMutex<DashboardWorkingConversationsMaterializerState>>,
    },
}
