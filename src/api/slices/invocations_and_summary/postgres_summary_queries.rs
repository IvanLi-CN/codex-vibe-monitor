use super::*;

#[derive(Debug, Default)]
struct PostgresSummaryUsageBreakdown {
    cache_write_tokens: i64,
    cache_read_tokens: i64,
    output_tokens: i64,
    costs: UsageCostBreakdownResponse,
    has_cost: bool,
}

impl PostgresSummaryUsageBreakdown {
    fn add_cost_row(&mut self, fact: &AnalyticsFact) {
        let Some(total_cost) = fact.cost.filter(|value| value.is_finite()) else {
            return;
        };
        self.has_cost = true;
        let costs = [
            fact.cost_input,
            fact.cost_cache_write,
            fact.cost_cache_read,
            fact.cost_output,
            fact.cost_reasoning,
        ];
        if costs
            .iter()
            .all(|value| value.is_some_and(|value| value.is_finite()))
        {
            self.costs.input += costs[0].unwrap_or_default();
            self.costs.cache_write += costs[1].unwrap_or_default();
            self.costs.cache_read += costs[2].unwrap_or_default();
            self.costs.output += costs[3].unwrap_or_default();
            self.costs.reasoning += costs[4].unwrap_or_default();
        } else {
            self.costs.unknown += total_cost;
        }
    }

    fn add_fact(&mut self, fact: &AnalyticsFact) {
        let cache_read_tokens = fact.cache_input_tokens.unwrap_or_default().max(0);
        let cache_write_tokens = fact
            .input_tokens
            .unwrap_or_default()
            .max(0)
            .saturating_sub(cache_read_tokens);
        self.cache_write_tokens += cache_write_tokens;
        self.cache_read_tokens += cache_read_tokens;
        self.output_tokens += fact.output_tokens.unwrap_or_default().max(0);
        self.add_cost_row(fact);
    }
}

fn postgres_summary_usage_breakdown(facts: &[AnalyticsFact]) -> Option<UsageBreakdownResponse> {
    let mut total = PostgresSummaryUsageBreakdown::default();
    let mut models = BTreeMap::<(String, Option<String>), PostgresSummaryUsageBreakdown>::new();
    for fact in facts {
        if !fact.terminal {
            continue;
        }
        total.add_fact(fact);
        let model = fact
            .response_model
            .as_deref()
            .or(fact.model.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("unknown")
            .to_string();
        let reasoning_effort = fact
            .reasoning_effort
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        models
            .entry((model, reasoning_effort))
            .or_default()
            .add_fact(fact);
    }
    if models.is_empty() {
        return None;
    }
    let models = models
        .into_iter()
        .map(
            |((model, reasoning_effort), usage)| UsageBreakdownModelResponse {
                model,
                reasoning_effort,
                cache_write_tokens: usage.cache_write_tokens,
                cache_read_tokens: usage.cache_read_tokens,
                output_tokens: usage.output_tokens,
                costs: usage.has_cost.then_some(usage.costs),
            },
        )
        .collect();
    Some(UsageBreakdownResponse {
        cache_write_tokens: total.cache_write_tokens,
        cache_read_tokens: total.cache_read_tokens,
        output_tokens: total.output_tokens,
        costs: total.has_cost.then_some(total.costs),
        models,
    })
}

fn postgres_summary_totals(facts: &[AnalyticsFact]) -> (StatsTotals, i64, i64) {
    let mut totals = StatsTotals::default();
    let mut non_success_tokens = 0_i64;
    let mut in_progress_count = 0_i64;
    for fact in facts {
        totals.total_count += 1;
        totals.total_tokens += fact.total_tokens.unwrap_or_default();
        totals.total_cost += fact.cost.unwrap_or_default();
        if !fact.terminal {
            in_progress_count += 1;
        } else if analytics_fact_is_success_like(fact) {
            totals.success_count += 1;
        } else if analytics_fact_is_failure(fact) {
            totals.failure_count += 1;
            let tokens = fact.total_tokens.unwrap_or_default();
            totals.non_success_cost += fact.cost.unwrap_or_default();
            non_success_tokens += tokens;
        }
    }
    (totals, non_success_tokens, in_progress_count)
}

pub(super) async fn fetch_summary_from_postgres(
    state: &AppState,
    params: &SummaryQuery,
) -> Result<Json<StatsResponse>, ApiError> {
    let default_limit = state.config.list_limit_max as i64;
    validate_summary_projection_window(params, default_limit)?;
    let window = parse_summary_window(params, default_limit).map_err(ApiError::bad_request)?;
    let reporting_tz = parse_reporting_tz(params.time_zone.as_deref())?;
    let now = Utc::now();
    let (range, limit, newest_first) = match window {
        SummaryWindow::All => (None, None, false),
        SummaryWindow::Current(limit) => (None, Some(limit), true),
        SummaryWindow::Duration(duration) => (Some((now - duration, now)), None, false),
        SummaryWindow::Calendar(ref spec) => {
            let range =
                resolve_range_window(spec.as_str(), reporting_tz).map_err(ApiError::from)?;
            (Some((range.start, range.end)), None, false)
        }
        SummaryWindow::PreviousFullDays(day_count) => {
            let (start, end) = previous_full_days_range_bounds(day_count, now, reporting_tz)
                .ok_or_else(|| {
                    ApiError::bad_request(anyhow!("invalid previous full days window"))
                })?;
            (Some((start, end)), None, false)
        }
    };
    let fact_window = query_postgres_summary_facts(
        state,
        InvocationSourceScope::All,
        params.upstream_account_id,
        range,
        limit,
        newest_first,
    )
    .await?;
    let (totals, non_success_tokens, in_progress_count) =
        postgres_summary_totals(&fact_window.facts);
    let mut response = totals.into_response();
    response.non_success_cost = Some(totals.non_success_cost);
    response.non_success_tokens = Some(non_success_tokens);
    response.in_progress_conversation_count = Some(in_progress_count);
    response.in_progress_retry_conversation_count = Some(0);
    response.usage_breakdown = summary_window_range(&window, reporting_tz, now)?
        .and_then(|_| postgres_summary_usage_breakdown(&fact_window.facts));
    response.maintenance = None;
    debug!(
        route = "/api/stats/summary",
        backend = "postgres",
        snapshot_id = fact_window.snapshot_id,
        fact_count = fact_window.facts.len(),
        "built summary from PostgreSQL analytics facts"
    );
    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_fact() -> AnalyticsFact {
        AnalyticsFact {
            fact_version: 1,
            source: "proxy".to_string(),
            source_row_id: 1,
            invoke_id: "invocation-1".to_string(),
            occurred_at: "2026-10-31T00:00:00.000Z".to_string(),
            occurred_at_epoch_ms: 1,
            stats_date: "2026-10-31".to_string(),
            model: Some("request-model".to_string()),
            request_model: None,
            response_model: None,
            reasoning_effort: None,
            upstream_account_id: None,
            upstream_account_name: None,
            status: Some("completed".to_string()),
            success_like: true,
            terminal: true,
            failure_kind: None,
            failure_class: None,
            is_actionable: Some(false),
            input_tokens: Some(10),
            output_tokens: Some(3),
            cache_input_tokens: Some(4),
            reasoning_tokens: None,
            total_tokens: Some(13),
            cost: Some(1.25),
            cost_input: Some(0.4),
            cost_cache_write: Some(0.1),
            cost_cache_read: Some(0.2),
            cost_output: Some(0.5),
            cost_reasoning: Some(0.05),
            t_total_ms: None,
            t_req_read_ms: None,
            t_req_parse_ms: None,
            t_upstream_connect_ms: None,
            t_upstream_ttfb_ms: None,
            t_upstream_stream_ms: None,
            first_token_ms: None,
            t_resp_parse_ms: None,
        }
    }

    #[test]
    fn postgres_summary_usage_preserves_token_and_cost_fallbacks() {
        let mut fact = sample_fact();
        fact.response_model = Some("response-model".to_string());
        let mut unknown_cost_fact = sample_fact();
        unknown_cost_fact.source_row_id = 2;
        unknown_cost_fact.cost = Some(2.0);
        unknown_cost_fact.cost_input = None;
        unknown_cost_fact.cost_cache_write = None;
        unknown_cost_fact.cost_cache_read = None;
        unknown_cost_fact.cost_output = None;
        unknown_cost_fact.cost_reasoning = None;
        unknown_cost_fact.input_tokens = Some(5);
        unknown_cost_fact.cache_input_tokens = Some(2);

        let response = postgres_summary_usage_breakdown(&[fact, unknown_cost_fact])
            .expect("usage breakdown should be present");
        assert_eq!(response.cache_write_tokens, 9);
        assert_eq!(response.cache_read_tokens, 6);
        assert_eq!(response.output_tokens, 6);
        let costs = response.costs.expect("costs should be present");
        assert_eq!(costs.input, 0.4);
        assert_eq!(costs.unknown, 2.0);
        let response_model = response
            .models
            .iter()
            .find(|model| model.model == "response-model")
            .expect("response model should be grouped");
        assert_eq!(response_model.reasoning_effort, None);
        assert_eq!(response_model.cache_write_tokens, 6);
    }

    #[test]
    fn postgres_summary_classification_keeps_legacy_success_statuses() {
        let mut warning_success = sample_fact();
        warning_success.success_like = false;
        warning_success.status = Some("warning_success".to_string());
        assert!(analytics_fact_is_success_like(&warning_success));

        let mut http_200 = sample_fact();
        http_200.success_like = false;
        http_200.status = Some("http_200".to_string());
        assert!(analytics_fact_is_success_like(&http_200));

        let mut failure = sample_fact();
        failure.success_like = true;
        failure.failure_class = Some("service_failure".to_string());
        assert!(!analytics_fact_is_success_like(&failure));
        assert!(analytics_fact_is_failure(&failure));
    }

    #[test]
    fn postgres_summary_usage_excludes_non_terminal_facts() {
        let terminal = sample_fact();
        let mut running = sample_fact();
        running.source_row_id = 2;
        running.status = Some("running".to_string());
        running.terminal = false;
        running.input_tokens = Some(100);
        running.cache_input_tokens = Some(40);
        running.output_tokens = Some(30);
        running.cost = Some(10.0);

        let response = postgres_summary_usage_breakdown(std::slice::from_ref(&terminal))
            .expect("usage should exist");
        let with_running = postgres_summary_usage_breakdown(&[terminal, running])
            .expect("usage should remain present");
        assert_eq!(with_running.cache_write_tokens, response.cache_write_tokens);
        assert_eq!(with_running.cache_read_tokens, response.cache_read_tokens);
        assert_eq!(with_running.output_tokens, response.output_tokens);
        assert_eq!(with_running.costs, response.costs);
        assert_eq!(with_running.models, response.models);
    }
}
