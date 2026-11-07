use super::*;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ProjectionFact {
    pub(crate) entity_kind: String,
    pub(crate) entity_key: String,
    pub(crate) occurred_at_ms: i64,
    pub(crate) dimensions: BTreeMap<String, Value>,
    pub(crate) interval: Option<CanonicalInterval>,
    pub(crate) payload: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CanonicalInterval {
    pub(crate) start_ms: i64,
    pub(crate) end_ms: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Rollup {
    pub(crate) calls: i64,
    pub(crate) successes: i64,
    pub(crate) totals: BTreeMap<String, f64>,
    pub(crate) samples: BTreeMap<String, i64>,
    pub(crate) intervals: Vec<CanonicalInterval>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct ProjectionModels {
    pub(crate) rollups: BTreeMap<String, Rollup>,
    pub(crate) intervals: BTreeMap<String, CanonicalInterval>,
}

pub(super) fn normalize(kind: &str, key: &str, payload: &Value) -> Result<ProjectionFact> {
    ensure!(
        matches!(kind, "invocation" | "pool_attempt"),
        "unsupported projection entity kind"
    );
    ensure!(
        payload["factVersion"].as_i64() == Some(1),
        "unsupported projection fact version"
    );
    let occurred = payload["occurredAt"]
        .as_str()
        .context("fact occurredAt is missing")?;
    let occurred_at =
        crate::stats::parse_to_utc_datetime(occurred).context("invalid fact occurredAt")?;
    let occurred_at_ms = occurred_at.timestamp_millis();
    let mut dimensions = BTreeMap::new();
    for field in [
        "model",
        "requestModel",
        "responseModel",
        "reasoningEffort",
        "upstreamAccountId",
        "upstreamAccountName",
        "promptCacheKeyDigest",
        "failureClass",
        "failureKind",
        "status",
        "endpoint",
    ] {
        if let Some(value) = payload.get(field).filter(|v| !v.is_null()) {
            dimensions.insert(field.to_owned(), value.clone());
        }
    }
    let interval = if kind == "invocation" && payload["successLike"].as_bool() == Some(true) {
        payload["tTotalMs"]
            .as_f64()
            .filter(|n| n.is_finite() && *n > 0.0 && *n < (i64::MAX as f64))
            .map(|duration| {
                let end_ms = occurred_at_ms
                    .checked_add(
                        (duration
                            + f64::from(occurred_at.timestamp_subsec_nanos() % 1_000_000)
                                / 1_000_000.0)
                            .ceil() as i64,
                    )
                    .context("fact interval overflow")?;
                Ok::<CanonicalInterval, anyhow::Error>(CanonicalInterval {
                    start_ms: occurred_at_ms,
                    end_ms,
                })
            })
            .transpose()?
    } else {
        None
    };
    Ok(ProjectionFact {
        entity_kind: kind.into(),
        entity_key: key.into(),
        occurred_at_ms,
        dimensions,
        interval,
        payload: payload.clone(),
    })
}

/// Rebuild from versions selected at P. Corrections and revocations therefore remove the old
/// contribution even when its date, dimensions, or interval changed. Attempts have separate
/// buckets and cannot inflate invocation token/cost totals.
pub(crate) fn build_models(facts: &[ProjectionFact]) -> Result<ProjectionModels> {
    let mut models = ProjectionModels::default();
    for fact in facts {
        if let Some(interval) = fact.interval {
            models.intervals.insert(fact.entity_key.clone(), interval);
        }
        for width in [3_600_000_i64, 86_400_000] {
            let start = fact.occurred_at_ms.div_euclid(width) * width;
            let mut dimensions = vec![("all".to_string(), Value::Null)];
            dimensions.extend(fact.dimensions.iter().map(|(k, v)| (k.clone(), v.clone())));
            for (dimension, value) in dimensions {
                let key = serde_json::to_string(&(
                    fact.entity_kind.as_str(),
                    width,
                    start,
                    &dimension,
                    &value,
                ))?;
                let rollup = models.rollups.entry(key).or_default();
                rollup.calls += 1;
                rollup.successes += i64::from(fact.payload["successLike"].as_bool() == Some(true));
                for field in [
                    "inputTokens",
                    "outputTokens",
                    "cacheInputTokens",
                    "reasoningTokens",
                    "totalTokens",
                    "cost",
                    "costInput",
                    "costCacheWrite",
                    "costCacheRead",
                    "costOutput",
                    "costReasoning",
                    "tTotalMs",
                    "tReqReadMs",
                    "tReqParseMs",
                    "tUpstreamConnectMs",
                    "tUpstreamTtfbMs",
                    "tUpstreamStreamMs",
                    "firstTokenMs",
                    "tRespParseMs",
                    "connectLatencyMs",
                    "firstByteLatencyMs",
                    "streamLatencyMs",
                ] {
                    if let Some(n) = fact.payload[field].as_f64().filter(|n| n.is_finite()) {
                        *rollup.totals.entry(field.into()).or_default() += n;
                        *rollup.samples.entry(field.into()).or_default() += 1;
                    }
                }
            }
            // Interval occupancy is clipped into every crossed UTC bucket, not just the
            // bucket containing the invocation's start time.
            if let Some(interval) = fact.interval {
                let mut bucket = interval.start_ms.div_euclid(width) * width;
                let count = (interval.end_ms.div_euclid(width) - bucket.div_euclid(width)) + 1;
                ensure!(
                    count <= 366 * 24,
                    "interval exceeds bounded projection span"
                );
                while bucket < interval.end_ms {
                    for (dimension, value) in std::iter::once(("all".to_string(), Value::Null))
                        .chain(
                            fact.dimensions
                                .iter()
                                .filter(|(k, _)| {
                                    matches!(k.as_str(), "model" | "upstreamAccountId")
                                })
                                .map(|(k, v)| (k.clone(), v.clone())),
                        )
                    {
                        let key = serde_json::to_string(&(
                            fact.entity_kind.as_str(),
                            width,
                            bucket,
                            &dimension,
                            &value,
                        ))?;
                        models
                            .rollups
                            .entry(key)
                            .or_default()
                            .intervals
                            .push(CanonicalInterval {
                                start_ms: interval.start_ms.max(bucket),
                                end_ms: interval
                                    .end_ms
                                    .min(bucket.checked_add(width).context("bucket overflow")?),
                            });
                    }
                    bucket = bucket
                        .checked_add(width)
                        .context("interval bucket overflow")?;
                }
            }
        }
    }
    for rollup in models.rollups.values_mut() {
        rollup.intervals.sort_by_key(|i| i.start_ms);
        let mut merged: Vec<CanonicalInterval> = Vec::new();
        for interval in &rollup.intervals {
            if let Some(last) = merged
                .last_mut()
                .filter(|last| last.end_ms >= interval.start_ms)
            {
                last.end_ms = last.end_ms.max(interval.end_ms);
            } else {
                merged.push(*interval);
            }
        }
        rollup.intervals = merged;
    }
    Ok(models)
}
