impl SummaryProjection {
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn generation_fence(&self) -> SummaryProjectionGenerationFence {
        self.generation_fence
    }

    pub(crate) fn advance_live_tail_fence(&mut self, live_tail: SummaryLiveTailCursor) {
        self.generation_fence.live_high_watermark_id = live_tail.live_high_watermark_id;
        self.generation_fence.rollup_live_cursor = live_tail.rollup_live_cursor;
        self.generation_fence.account_rollup_live_cursor = live_tail.account_rollup_live_cursor;
        self.generation_fence.durable_terminal_sequence_watermark =
            live_tail.durable_terminal_sequence_watermark;
        self.durable_terminal_sequence_watermark = live_tail.durable_terminal_sequence_watermark;
    }

    pub(crate) fn with_revision(mut self, revision: u64) -> Self {
        self.revision = revision;
        self
    }

    pub(crate) fn revoke_stale_all_time_coverage(
        &mut self,
        generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        let coverage_fence = generation_fence.coverage_fence();
        let revoke_global = self.freshness.global_all_time_eligible
            && self
                .global_all_time_coverage_fence
                .is_none_or(|published| !published.global_sources_match(coverage_fence));
        let revoke_account = !self.freshness.account_all_time_eligible.is_empty()
            && self
                .account_all_time_coverage_fence
                .is_none_or(|published| !published.account_sources_match(coverage_fence));
        let revoke_overlay = self
            .coverage_overlay
            .as_ref()
            .is_some_and(|overlay| overlay.coverage_fence != coverage_fence);
        let coverage_changed = self.generation_fence.coverage_fence() != coverage_fence;
        if !revoke_global && !revoke_account && !revoke_overlay {
            if coverage_changed {
                self.generation_fence.completed_manifest_high_watermark_id =
                    generation_fence.completed_manifest_high_watermark_id;
                self.generation_fence.coverage_revision = generation_fence.coverage_revision;
                self.generation_fence.account_coverage_revision =
                    generation_fence.account_coverage_revision;
                return true;
            }
            return false;
        }
        if revoke_global {
            self.all_time_by_account.remove(&None);
            self.freshness.global_all_time_eligible = false;
            self.all_time_refreshed_at = None;
            self.global_all_time_coverage_fence = None;
            self.all_time_terminal_coverage_complete = false;
            self.all_time_terminal_sequence_watermark = 0;
            self.all_time_persisted_live_terminal_invoke_ids.clear();
        }
        if revoke_account {
            self.all_time_by_account.retain(|scope, _| scope.is_none());
            self.freshness.account_all_time_eligible.clear();
            self.all_time_account_refreshed_at.clear();
            self.account_all_time_coverage_fence = None;
            self.all_time_account_terminal_sequence_watermarks.clear();
            self.all_time_account_persisted_live_terminal_invoke_ids
                .clear();
            self.all_time_oldest_account_refreshed_at = None;
        }
        if revoke_overlay && let Some(overlay) = self.coverage_overlay.take() {
            self.unavailable_unmaterialized_archive_ranges.extend(
                summary_projection_unavailable_bucket_ranges(
                    overlay.global_coverage_buckets.into_iter().collect(),
                ),
            );
            self.unavailable_unmaterialized_archive_account_ranges
                .extend(summary_projection_unavailable_bucket_ranges(
                    overlay.account_coverage_buckets.into_iter().collect(),
                ));
            self.unavailable_unmaterialized_archive_ranges = summary_projection_merge_exact_ranges(
                std::mem::take(&mut self.unavailable_unmaterialized_archive_ranges),
            );
            self.unavailable_unmaterialized_archive_account_ranges =
                summary_projection_merge_exact_ranges(std::mem::take(
                    &mut self.unavailable_unmaterialized_archive_account_ranges,
                ));
        }
        self.generation_fence.completed_manifest_high_watermark_id =
            generation_fence.completed_manifest_high_watermark_id;
        self.generation_fence.coverage_revision = generation_fence.coverage_revision;
        self.generation_fence.account_coverage_revision =
            generation_fence.account_coverage_revision;
        true
    }

    fn rolling_refreshed_at(&self) -> Option<Instant> {
        self.freshness_lease
            .latest(self.freshness.rolling_at(self.refreshed_at))
    }

    pub(crate) fn renew_freshness_if_generation_matches(
        &self,
        generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        if self.generation_fence != generation_fence {
            return false;
        }
        self.freshness_lease.renew();
        true
    }

    // Historical coverage revisions revoke the prior immutable overlay until a replacement is
    // published. Even when the live tail is unchanged, do not renew a lease over stale history;
    // the caller must perform the bounded replacement or leave the affected selection unavailable.
    pub(crate) fn renew_freshness_if_live_tail_matches(
        &self,
        generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        if !self
            .generation_fence
            .live_tail_cursor()
            .terminal_sources_match(generation_fence.live_tail_cursor())
            || self.generation_fence.completed_manifest_high_watermark_id
                != generation_fence.completed_manifest_high_watermark_id
            || self.generation_fence.coverage_revision != generation_fence.coverage_revision
            || self.generation_fence.account_coverage_revision
                != generation_fence.account_coverage_revision
        {
            return false;
        }
        self.freshness_lease.renew();
        true
    }

    // All-time reconciliation owns historical coverage only. A newly committed terminal
    // advances the live tail cursor and must be served by the bounded overlay without cancelling
    // an otherwise valid archive/rollup recovery page.
    pub(crate) fn renew_freshness_if_coverage_matches(
        &self,
        generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        if !self
            .generation_fence
            .coverage_sources_match(generation_fence)
            || !self
                .generation_fence
                .live_tail_cursor()
                .rollup_sources_match(generation_fence.live_tail_cursor())
        {
            return false;
        }
        self.freshness_lease.renew();
        true
    }

    pub(crate) fn coverage_sources_match(
        &self,
        generation_fence: SummaryProjectionGenerationFence,
    ) -> bool {
        self.generation_fence
            .coverage_sources_match(generation_fence)
            && self
                .generation_fence
                .live_tail_cursor()
                .rollup_sources_match(generation_fence.live_tail_cursor())
    }

    pub(crate) fn renew_freshness_from_delta_journal(&self) {
        self.freshness_lease.renew();
    }

    pub(crate) fn contains_persisted_live_terminal_by_row_id(&self, row_id: i64) -> bool {
        self.records
            .iter()
            .chain(self.current_records.iter())
            .any(|record| record.is_persisted_live_record && record.row.id == row_id)
    }

    pub(crate) fn contains_persisted_live_terminal_identity(
        &self,
        row_id: i64,
        invoke_id: &str,
        occurred_at: &str,
    ) -> bool {
        self.records
            .iter()
            .chain(self.current_records.iter())
            .any(|record| {
                record.is_persisted_live_record
                    && record.row.id == row_id
                    && record.row.invoke_id == invoke_id
                    && record.row.occurred_at == occurred_at
            })
    }

    pub(crate) fn contains_persisted_live_terminal_delta(
        &self,
        delta: &DashboardActivityTerminalDelta,
    ) -> bool {
        delta.persisted_row_id.map_or_else(
            || self.contains_persisted_live_terminal(&delta.invoke_id, &delta.occurred_at),
            |row_id| {
                self.contains_persisted_live_terminal_identity(
                    row_id,
                    &delta.invoke_id,
                    &delta.occurred_at,
                )
            },
        )
    }

    pub(crate) fn mark_historical_live_recovery_required(&mut self) {
        if let Some(coverage) = self.historical_live_coverage.as_mut() {
            coverage.reconciliation_required = true;
        }
    }
    pub(crate) fn current_selection_cutoff_for_scope(
        &self,
        upstream_account_id: Option<i64>,
        limit: usize,
    ) -> Option<DateTime<Utc>> {
        if limit == 0 {
            return None;
        }
        let indexes = self.recent_indexes.get(&upstream_account_id)?;
        // A shorter resident prefix has no Nth cutoff. Clamping it to the oldest retained row
        // would incorrectly prove that an unrepresented archive cannot supply the missing rank.
        let index = *indexes.get(limit.saturating_sub(1))?;
        self.current_records
            .get(index)
            .map(|record| record.occurred_at)
    }

    fn current_selection_cutoff(&self, limit: usize) -> Option<DateTime<Utc>> {
        self.current_selection_cutoff_for_scope(None, limit)
    }

    pub(crate) fn delta_gap_affects_current_selection(
        &self,
        gap: &DeltaGapProof,
        limit: usize,
        upstream_account_id: Option<i64>,
        deltas: &[DashboardActivityTerminalDelta],
    ) -> bool {
        if limit == 0 {
            return false;
        }
        // A legacy source change may have been reconstructed exactly into the rolling overlay,
        // even though its descriptor was absent.  The current index can then prove its rank from
        // the overlay; do not reject that newest-N selection solely because the historical proof
        // marker is still waiting for background reconciliation.
        if gap.row_id.is_some_and(|row_id| {
            deltas.iter().any(|delta| {
                delta.persisted_row_id == Some(row_id)
                    && gap.invoke_id.as_deref() == Some(delta.invoke_id.as_str())
                    && gap.occurred_at == delta.occurred_at
                    && upstream_account_id
                        .is_none_or(|account_id| delta.upstream_account_id == Some(account_id))
            })
        }) {
            return false;
        }
        let Some(occurred_at) = parse_to_utc_datetime(&gap.occurred_at) else {
            return true;
        };
        let indexes = self
            .recent_indexes
            .get(&upstream_account_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let row_id = gap.row_id.unwrap_or(i64::MAX);
        let newer_base_count = indexes
            .iter()
            .filter_map(|index| self.current_records.get(*index))
            .filter(|record| {
                record.occurred_at > occurred_at
                    || (record.occurred_at == occurred_at && record.row.id > row_id)
            })
            .count();
        let newer_delta_count = deltas
            .iter()
            .filter(|delta| {
                upstream_account_id
                    .is_none_or(|account_id| delta.upstream_account_id == Some(account_id))
            })
            .filter_map(|delta| {
                parse_to_utc_datetime(&delta.occurred_at).map(|delta_occurred_at| {
                    (
                        delta_occurred_at,
                        delta.persisted_row_id.unwrap_or(i64::MAX),
                    )
                })
            })
            .filter(|(delta_occurred_at, delta_row_id)| {
                *delta_occurred_at > occurred_at
                    || (*delta_occurred_at == occurred_at && *delta_row_id > row_id)
            })
            .count();
        let newer_count = newer_base_count.saturating_add(newer_delta_count);
        // A gap outside the resident prefix cannot alter a smaller current selection. If its
        // rank reaches the requested limit, the response cannot be proved exact in memory.
        limit >= newer_count.saturating_add(1)
    }

    pub(crate) fn apply_rolling_delta_to_current_response(
        &self,
        response: &mut StatsResponse,
        limit: i64,
        upstream_account_id: Option<i64>,
        deltas: &[DashboardActivityTerminalDelta],
    ) -> Result<(), ApiError> {
        if deltas.is_empty() {
            return Ok(());
        }
        let limit = limit.max(0) as usize;
        let indexes = self
            .recent_indexes
            .get(&upstream_account_id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let mut candidates = Vec::<(DateTime<Utc>, i64, StatsTotals, i64)>::new();
        for index in indexes {
            let Some(record) = self.current_records.get(*index) else {
                return Err(ApiError::unavailable(anyhow!(
                    "summary current delta base is outside the resident index"
                )));
            };
            if upstream_account_id
                .is_some_and(|account_id| record.row.upstream_account_id != Some(account_id))
            {
                continue;
            }
            let totals = summary_projection_record_totals(record);
            let non_success_tokens = (totals.failure_count > 0).then_some(record.row.total_tokens);
            candidates.push((
                record.occurred_at,
                record.row.id,
                totals,
                non_success_tokens.unwrap_or_default(),
            ));
        }
        for delta in deltas {
            if upstream_account_id
                .is_some_and(|account_id| delta.upstream_account_id != Some(account_id))
            {
                continue;
            }
            let occurred_at = parse_to_utc_datetime(&delta.occurred_at).ok_or_else(|| {
                ApiError::unavailable(anyhow!(
                    "summary delta journal has an invalid current ordering timestamp"
                ))
            })?;
            let totals = StatsTotals {
                total_count: 1,
                success_count: i64::from(delta.success),
                failure_count: i64::from(delta.failure),
                total_tokens: delta.total_tokens,
                total_cost: delta.total_cost,
                non_success_cost: if delta.failure { delta.total_cost } else { 0.0 },
            };
            candidates.push((
                occurred_at,
                delta.persisted_row_id.unwrap_or(i64::MAX),
                totals,
                if delta.failure { delta.total_tokens } else { 0 },
            ));
        }
        candidates.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));
        let mut totals = StatsTotals::default();
        let mut non_success_tokens = 0_i64;
        for (_, _, candidate, candidate_non_success_tokens) in candidates.into_iter().take(limit) {
            totals = totals.add(candidate);
            non_success_tokens += candidate_non_success_tokens;
        }
        response.total_count = totals.total_count;
        response.success_count = totals.success_count;
        response.failure_count = totals.failure_count;
        response.total_tokens = totals.total_tokens;
        response.total_cost = totals.total_cost;
        response.non_success_cost = Some(totals.non_success_cost);
        if response.non_success_tokens.is_some() {
            response.non_success_tokens = Some(non_success_tokens);
        }
        Ok(())
    }

    fn current_archive_may_affect_global_current(&self, limit: usize) -> bool {
        if limit == 0 {
            return false;
        }
        if self.current_archive_has_unknown_coverage {
            return true;
        }
        let Some(latest_coverage_end) = self.current_archive_latest_coverage_end else {
            return false;
        };
        self.current_selection_cutoff(limit)
            .is_none_or(|cutoff| latest_coverage_end > cutoff)
    }

    fn unavailable_archive_may_affect_global_current(&self, limit: usize) -> bool {
        if limit == 0
            || self
                .unavailable_unmaterialized_archive_current_ranges
                .is_empty()
        {
            return false;
        }
        self.current_selection_cutoff(limit).is_none_or(|cutoff| {
            self.unavailable_unmaterialized_archive_current_ranges
                .iter()
                .any(|range| range.end > cutoff)
        })
    }

    fn unavailable_boundary_archive_may_affect_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> bool {
        summary_projection_partial_rollup_ranges(start, end)
            .into_iter()
            .any(|partial| {
                self.unavailable_boundary_archive_ranges
                    .iter()
                    .any(|unavailable| {
                        unavailable.start < partial.end && partial.start < unavailable.end
                    })
            })
    }

    fn unavailable_archive_may_affect_account_current(
        &self,
        account_id: i64,
        limit: usize,
    ) -> bool {
        if limit == 0
            || self
                .unavailable_unmaterialized_archive_account_current_ranges
                .is_empty()
        {
            return false;
        }
        self.current_selection_cutoff_for_scope(Some(account_id), limit)
            .is_none_or(|cutoff| {
                self.unavailable_unmaterialized_archive_account_current_ranges
                    .iter()
                    .any(|range| range.end > cutoff)
            })
    }

    fn unavailable_account_boundary_archive_may_affect_range(
        &self,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> bool {
        summary_projection_partial_rollup_ranges(start, end)
            .into_iter()
            .any(|partial| {
                self.unavailable_boundary_archive_account_ranges
                    .iter()
                    .any(|unavailable| {
                        unavailable.start < partial.end && partial.start < unavailable.end
                    })
            })
    }

    pub(crate) fn contains_persisted_live_terminal(
        &self,
        invoke_id: &str,
        occurred_at: &str,
    ) -> bool {
        self.persisted_live_terminal_invoke_ids
            .contains(&format!("{invoke_id}\0{occurred_at}"))
            || self
                .persisted_live_terminal_invoke_ids
                .iter()
                .any(|identity| {
                    identity
                        .split_once('\0')
                        .and_then(|(_, remainder)| remainder.split_once('\0'))
                        .is_some_and(|(identity_occurred_at, identity_invoke_id)| {
                            identity_occurred_at == occurred_at && identity_invoke_id == invoke_id
                        })
                })
    }

    pub(crate) fn durable_terminal_sequence_watermark(&self) -> u64 {
        self.durable_terminal_sequence_watermark
    }

    pub(crate) fn all_time_terminal_coverage_complete(&self) -> bool {
        self.all_time_terminal_coverage_complete
    }

    pub(crate) fn all_time_terminal_sequence_watermark(&self) -> u64 {
        self.all_time_terminal_sequence_watermark
    }

    pub(crate) fn all_time_terminal_scope_covers(
        &self,
        upstream_account_id: Option<i64>,
        _invoke_id: &str,
        _occurred_at: &str,
        terminal_sequence: u64,
    ) -> bool {
        if let Some(account_id) = upstream_account_id {
            let Some(account_watermark) = self
                .all_time_account_terminal_sequence_watermarks
                .get(&account_id)
                .copied()
            else {
                return false;
            };
            // This legacy API has no durable row ID, so a full identity proof cannot be
            // reconstructed here.  Require the scoped contiguous watermark instead of falling
            // back to the weaker invoke_id+timestamp pair.
            return account_watermark >= terminal_sequence;
        }
        summary_projection_all_time_scope_covered(
            self.all_time_terminal_coverage_complete,
            self.all_time_terminal_sequence_watermark,
            terminal_sequence,
            self.all_time_terminal_coverage_complete,
        )
    }

    pub(crate) fn all_time_terminal_identity_scope_covers(
        &self,
        upstream_account_id: Option<i64>,
        row_id: i64,
        invoke_id: &str,
        occurred_at: &str,
    ) -> bool {
        self.records
            .iter()
            .chain(self.current_records.iter())
            .any(|record| {
                record.is_persisted_live_record
                    && record.row.id == row_id
                    && record.row.invoke_id == invoke_id
                    && record.row.occurred_at == occurred_at
                    && record.row.upstream_account_id == upstream_account_id
            })
    }

    pub(crate) fn contains_global_rollup_covered_live_terminal_identity(
        &self,
        row_id: i64,
        invoke_id: &str,
        occurred_at: &str,
    ) -> bool {
        self.records
            .iter()
            .chain(self.current_records.iter())
            .any(|record| {
                record.is_persisted_live_record
                    && record.global_rollup_covered
                    && record.row.id == row_id
                    && record.row.invoke_id == invoke_id
                    && record.row.occurred_at == occurred_at
            })
    }

    pub(crate) fn contains_global_all_time_covered_live_terminal_identity(
        &self,
        row_id: i64,
        invoke_id: &str,
        occurred_at: &str,
    ) -> bool {
        self.all_time_persisted_live_terminal_invoke_ids.contains(
            &summary_projection_source_identity_key(row_id, invoke_id, occurred_at),
        )
    }

    pub(crate) fn global_rollup_covers_live_terminal_identity(
        &self,
        row_id: i64,
        occurred_at: &str,
    ) -> bool {
        let Some(occurred_at) = parse_to_utc_datetime(occurred_at) else {
            return false;
        };
        let bucket = align_bucket_epoch(occurred_at.timestamp(), 3_600, 0);
        row_id > 0
            && row_id <= self.rollup_live_cursor
            && self.hourly_rollup_totals.contains_key(&(bucket, None))
            && self.hourly_rollup_usage.contains_key(&(bucket, None))
    }

    pub(crate) fn all_time_terminal_delta_scope_covers(
        &self,
        upstream_account_id: Option<i64>,
        persisted_row_id: Option<i64>,
        invoke_id: &str,
        occurred_at: &str,
        terminal_sequence: u64,
    ) -> bool {
        if let Some(row_id) = persisted_row_id {
            let identity = summary_projection_source_identity_key(row_id, invoke_id, occurred_at);
            if let Some(account_id) = upstream_account_id {
                return self
                    .all_time_account_persisted_live_terminal_invoke_ids
                    .get(&account_id)
                    .is_some_and(|identities| identities.contains(&identity))
                    || self.all_time_terminal_identity_scope_covers(
                        Some(account_id),
                        row_id,
                        invoke_id,
                        occurred_at,
                    );
            }
            let persisted_identity = self
                .all_time_persisted_live_terminal_invoke_ids
                .contains(&identity);
            let global_rollup_identity = self
                .contains_global_rollup_covered_live_terminal_identity(
                    row_id,
                    invoke_id,
                    occurred_at,
                );
            return persisted_identity || global_rollup_identity;
        }
        self.all_time_terminal_scope_covers(
            upstream_account_id,
            invoke_id,
            occurred_at,
            terminal_sequence,
        )
    }
}
