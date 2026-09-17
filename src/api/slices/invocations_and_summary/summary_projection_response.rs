impl SummaryProjection {
    pub(crate) fn all_time_terminal_account_scopes_cover(&self, terminal_sequence: u64) -> bool {
        self.all_time_account_ids_with_projection_data
            .iter()
            .all(|account_id| {
                self.all_time_terminal_scope_covers(Some(*account_id), "", "", terminal_sequence)
            })
    }

    fn needs_cadence_refresh(&self, has_all_time_owner: bool) -> bool {
        let refresh_due = |refreshed_at: Option<Instant>| {
            refreshed_at.is_none_or(|refreshed_at| {
                refreshed_at.elapsed() >= SUMMARY_SNAPSHOT_MIN_REFRESH_INTERVAL
            })
        };
        let now = Instant::now();
        let all_time_refresh_due = if self.all_time_manifest_admission_blocked_at.is_some() {
            summary_projection_manifest_admission_retry_is_due(
                self.all_time_manifest_admission_blocked_at,
                now,
            )
        } else {
            refresh_due(self.all_time_refreshed_at)
        };
        let account_refresh_due = if self.all_time_manifest_admission_blocked_at.is_some() {
            false
        } else if self
            .all_time_account_manifest_admission_blocked_at
            .is_some()
        {
            summary_projection_manifest_admission_retry_is_due(
                self.all_time_account_manifest_admission_blocked_at,
                now,
            )
        } else {
            refresh_due(self.all_time_oldest_account_refreshed_at)
        };
        refresh_due(self.rolling_refreshed_at())
            || (has_all_time_owner && (all_time_refresh_due || account_refresh_due))
    }

    fn empty_all_time_account_response(&self, upstream_account_id: Option<i64>) -> StatsResponse {
        let in_progress = self
            .in_progress_by_account
            .get(&upstream_account_id)
            .copied()
            .unwrap_or_default();
        let mut response = StatsTotals::default().into_response();
        response.non_success_cost = Some(0.0);
        response.in_progress_conversation_count = Some(in_progress.in_progress_count);
        response.in_progress_retry_conversation_count = Some(in_progress.retry_count);
        response.in_progress_avg_wait_ms = in_progress.avg_wait_ms;
        response.in_progress_phase_counts = Some(in_progress.phase_counts);
        response.maintenance = self.maintenance.clone();
        response
    }

    pub(crate) fn response_for_query(
        &self,
        params: &SummaryQuery,
        default_limit: i64,
    ) -> Result<StatsResponse, ApiError> {
        self.response_for_query_with_rolling_delta(params, default_limit, false)
    }

    pub(crate) fn response_for_query_with_rolling_delta(
        &self,
        params: &SummaryQuery,
        default_limit: i64,
        rolling_delta_is_exact: bool,
    ) -> Result<StatsResponse, ApiError> {
        let query = parse_summary_projection_query(self, params, default_limit)?;
        ensure_summary_projection_archive_sources(self, &query)?;
        ensure_summary_projection_current_sources(self, &query)?;
        ensure_summary_projection_freshness(self, &query, rolling_delta_is_exact)?;
        if let Some(response) = response_for_summary_all_time_query(self, &query)? {
            return Ok(response);
        }
        calculate_summary_projection_response(self, &query)
    }
}
