use super::*;
use serde_json::json;

fn fact(key: &str, time: &str, duration: f64) -> ProjectionFact {
    model::normalize("invocation",key,&json!({"factVersion":1,"occurredAt":time,"successLike":true,"tTotalMs":duration,"totalTokens":10,"cost":0.5,"model":"gpt-test"})).unwrap()
}

#[test]
fn interval_union_clips_midnight_and_does_not_double_count_overlap() {
    let facts = vec![
        fact("a", "2026-11-01T23:59:59Z", 3000.0),
        fact("b", "2026-11-02T00:00:00Z", 3000.0),
    ];
    let models = build_models(&facts).unwrap();
    let daily: Vec<_> = models
        .rollups
        .iter()
        .filter(|(key, _)| key.contains(",86400000,") && key.contains("\"all\""))
        .map(|(_, v)| v)
        .collect();
    assert_eq!(daily.len(), 2);
    let union: i64 = daily
        .iter()
        .flat_map(|r| &r.intervals)
        .map(|i| i.end_ms - i.start_ms)
        .sum();
    assert_eq!(union, 4000);
    assert_eq!(daily.iter().map(|r| r.calls).sum::<i64>(), 2);
}

#[test]
fn replacement_rebuild_removes_previous_dimensions_and_contribution() {
    let before = fact("same", "2026-11-01T12:00:00Z", 1000.0);
    let mut after = before.clone();
    after
        .dimensions
        .insert("model".into(), json!("replacement"));
    after.payload["totalTokens"] = json!(3);
    let old = build_models(&[before]).unwrap();
    let new = build_models(&[after]).unwrap();
    assert!(old.rollups.keys().any(|k| k.contains("gpt-test")));
    assert!(!new.rollups.keys().any(|k| k.contains("gpt-test")));
    assert!(new.rollups.values().all(|r| r.totals["totalTokens"] == 3.0));
    assert_eq!(build_models(&[]).unwrap(), ProjectionModels::default());
}

#[test]
fn unsupported_fact_and_unknown_entity_fail_closed() {
    assert!(model::normalize("unknown", "a", &json!({})).is_err());
    assert!(model::normalize("invocation", "a", &json!({"factVersion":2})).is_err());
    assert!(
        model::normalize(
            "pool_attempt",
            "a",
            &json!({"factVersion":1,"occurredAt":"invalid"})
        )
        .is_err()
    );
}

#[test]
fn attempt_and_invocation_rollups_are_disjoint() {
    let invocation = fact("a", "2026-11-01T12:00:00Z", 1000.0);
    let attempt = model::normalize(
        "pool_attempt",
        "b",
        &json!({"factVersion":1,"occurredAt":"2026-11-01T12:00:00Z","connectLatencyMs":12}),
    )
    .unwrap();
    let models = build_models(&[invocation, attempt]).unwrap();
    for (key, rollup) in models.rollups {
        if key.contains("pool_attempt") {
            assert!(!rollup.totals.contains_key("totalTokens"));
            assert!(rollup.intervals.is_empty());
        }
    }
}
