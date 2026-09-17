#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerfStageStats {
    pub(crate) stage: String,
    pub(crate) count: i64,
    pub(crate) avg_ms: f64,
    pub(crate) p50_ms: f64,
    pub(crate) p90_ms: f64,
    pub(crate) p99_ms: f64,
    pub(crate) max_ms: f64,
}
