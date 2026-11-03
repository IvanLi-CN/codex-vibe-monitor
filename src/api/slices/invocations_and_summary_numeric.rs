use super::*;

pub(crate) fn summary_live_numeric_overflow(field: &str) -> ApiError {
    ApiError::unavailable(anyhow::anyhow!(
        "summary live augmentation exceeded its numeric budget while adding {field}"
    ))
}

pub(crate) fn checked_summary_live_add(
    target: &mut f64,
    value: f64,
    field: &str,
) -> Result<(), ApiError> {
    let next = *target + value;
    if !next.is_finite() || next < 0.0 {
        return Err(summary_live_numeric_overflow(field));
    }
    *target = next;
    Ok(())
}

pub(crate) fn checked_summary_live_subtract(
    target: &mut f64,
    value: f64,
    field: &str,
) -> Result<(), ApiError> {
    if !target.is_finite() || *target < 0.0 || !value.is_finite() || value < 0.0 {
        return Err(summary_live_numeric_overflow(field));
    }
    let next = *target - value;
    let rounding_tolerance = f64::EPSILON * target.abs().max(value.abs()).max(1.0) * 8.0;
    if !next.is_finite() || next < -rounding_tolerance {
        return Err(summary_live_numeric_overflow(field));
    }
    *target = next.max(0.0);
    Ok(())
}

pub(crate) fn checked_summary_live_add_i64(
    target: &mut i64,
    value: i64,
    field: &str,
) -> Result<(), ApiError> {
    *target = target
        .checked_add(value)
        .ok_or_else(|| summary_live_numeric_overflow(field))?;
    Ok(())
}

pub(crate) fn merge_summary_live_i64(base: i64, delta: i64, field: &str) -> Result<i64, ApiError> {
    Ok(base
        .checked_add(delta)
        .ok_or_else(|| summary_live_numeric_overflow(field))?
        .max(0))
}

pub(crate) fn merge_summary_live_f64(base: f64, delta: f64, field: &str) -> Result<f64, ApiError> {
    let next = base + delta;
    if !next.is_finite() || next < 0.0 {
        return Err(summary_live_numeric_overflow(field));
    }
    Ok(next)
}

pub(crate) struct MergedInvocationSummary {
    pub(crate) total_count: i64,
    pub(crate) success_count: i64,
    pub(crate) failure_count: i64,
    pub(crate) total_tokens: i64,
    pub(crate) total_cost: f64,
    pub(crate) cache_write_tokens: i64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) exception: InvocationExceptionSummary,
}

pub(crate) fn merge_invocation_summary_totals(
    totals: &InvocationSummaryAggRow,
    exception: &InvocationExceptionSummary,
    delta: &RuntimeSummaryOverlayDelta,
) -> Result<MergedInvocationSummary, ApiError> {
    Ok(MergedInvocationSummary {
        total_count: merge_summary_live_i64(totals.total_count, delta.total_count, "total count")?,
        success_count: merge_summary_live_i64(
            totals.success_count,
            delta.success_count,
            "success count",
        )?,
        failure_count: merge_summary_live_i64(
            totals.failure_count,
            delta.failure_count,
            "failure count",
        )?,
        total_tokens: merge_summary_live_i64(
            totals.total_tokens,
            delta.total_tokens,
            "total tokens",
        )?,
        total_cost: merge_summary_live_f64(totals.total_cost, delta.total_cost, "total cost")?,
        cache_write_tokens: merge_summary_live_i64(
            totals.cache_write_tokens,
            delta.cache_write_tokens,
            "cache write tokens",
        )?,
        cache_input_tokens: merge_summary_live_i64(
            totals.cache_input_tokens,
            delta.cache_input_tokens,
            "cache input tokens",
        )?,
        output_tokens: merge_summary_live_i64(
            totals.output_tokens,
            delta.output_tokens,
            "output tokens",
        )?,
        exception: InvocationExceptionSummary {
            failure_count: merge_summary_live_i64(
                exception.failure_count,
                delta.failure_count,
                "exception failure count",
            )?,
            service_failure_count: merge_summary_live_i64(
                exception.service_failure_count,
                delta.service_failure_count,
                "service failure count",
            )?,
            client_failure_count: merge_summary_live_i64(
                exception.client_failure_count,
                delta.client_failure_count,
                "client failure count",
            )?,
            client_abort_count: merge_summary_live_i64(
                exception.client_abort_count,
                delta.client_abort_count,
                "client abort count",
            )?,
            actionable_failure_count: merge_summary_live_i64(
                exception.actionable_failure_count,
                delta.service_failure_count,
                "actionable failure count",
            )?,
        },
    })
}

pub(crate) fn summary_live_sum_from_average(
    average: Option<f64>,
    sample_count: i64,
) -> Result<f64, ApiError> {
    if sample_count < 0 {
        return Err(summary_live_numeric_overflow("database wait sample count"));
    }
    match (average, sample_count) {
        (None, 0) => Ok(0.0),
        (Some(average), sample_count)
            if average.is_finite() && average >= 0.0 && sample_count > 0 =>
        {
            let sum = average * sample_count as f64;
            if sum.is_finite() {
                Ok(sum)
            } else {
                Err(summary_live_numeric_overflow("database wait sum"))
            }
        }
        _ => Err(summary_live_numeric_overflow("database wait average")),
    }
}
