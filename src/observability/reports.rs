use crate::*;

const REPORT_MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub(super) struct ReportLimiter {
    started: Instant,
    count: u32,
}
impl Default for ReportLimiter {
    fn default() -> Self {
        Self {
            started: Instant::now(),
            count: 0,
        }
    }
}
impl ReportLimiter {
    fn allow(&mut self) -> bool {
        if self.started.elapsed() >= Duration::from_secs(60) {
            self.started = Instant::now();
            self.count = 0;
        }
        if self.count >= 30 {
            return false;
        }
        self.count += 1;
        true
    }
}
pub(crate) async fn observability_capabilities(
    State(state): State<Arc<AppState>>,
) -> Json<serde_json::Value> {
    Json(
        json!({"enabled":state.observability.enabled,"state":state.observability.state(),"grafanaPublicUrl":state.config.observability.grafana_public_url,"grafanaConnectivity":"unknown","hotpath":cfg!(feature="hotpath"),"dashboards":["cvm-overview","cvm-proxy","cvm-sqlite","cvm-runtime","cvm-web"],"datasourceUid":"cvm-prometheus","variables":["service","environment","instance","task_key"]}),
    )
}
pub(crate) async fn hotpath_report(
    State(state): State<Arc<AppState>>,
    AxumPath(report): AxumPath<String>,
    headers: HeaderMap,
) -> Response {
    if !super::config::authorized(&headers, state.config.observability.read_token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let path = match report.as_str() {
        "server" => "server",
        "sql" => "sql",
        "functions" => "functions_timing",
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    if !state
        .observability
        .report_limiter
        .lock()
        .map(|mut limiter| limiter.allow())
        .unwrap_or(false)
    {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    if !state.observability.enabled || !cfg!(feature = "hotpath") {
        return unavailable();
    }
    match tokio::time::timeout(Duration::from_secs(2), read_report(path)).await {
        Ok(Ok(rows)) => {
            let response = json!({"report":report,"collectedAt":format_utc_iso_millis(Utc::now()),"processStartedAt":format_utc_iso_millis(state.observability.started_at),"revision":option_env!("APP_GIT_REVISION").unwrap_or("unknown"),"functionSamplingRate":0.1,"rows":rows});
            match serde_json::to_vec(&response) {
                Ok(bytes) if bytes.len() <= REPORT_MAX_BYTES => {
                    ([(header::CONTENT_TYPE, "application/json")], bytes).into_response()
                }
                _ => unavailable(),
            }
        }
        Ok(Err(error)) => {
            tracing::warn!(report, error = %error, "hotpath report unavailable");
            unavailable()
        }
        Err(error) => {
            tracing::warn!(report, error = %error, "hotpath report unavailable");
            unavailable()
        }
    }
}
fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"code":"profiler_unavailable"})),
    )
        .into_response()
}
async fn read_report(path: &str) -> Result<Vec<serde_json::Value>> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()?;
    let token = env::var("HOTPATH_METRICS_AUTH_TOKEN").ok();
    let request = report_request(&client, path, token.as_deref());
    let response = request.send().await?.error_for_status()?;
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if bytes.len() + chunk.len() > REPORT_MAX_BYTES {
            bail!("profiler response exceeds limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)?;
    sanitize_report(path, &value)
}

fn report_request(
    client: &reqwest::Client,
    path: &str,
    token: Option<&str>,
) -> reqwest::RequestBuilder {
    let request = client.get(format!("http://127.0.0.1:6770/{path}"));
    // Unlike its Prometheus exporter, hotpath's report server compares the raw header.
    match token {
        Some(token) => request.header(header::AUTHORIZATION, token),
        None => request,
    }
}

fn sanitize_report(path: &str, value: &serde_json::Value) -> Result<Vec<serde_json::Value>> {
    let (fields, required): (&[&str], &[&str]) = match path {
        "functions_timing" => (
            &[
                "name",
                "calls",
                "sampled_calls",
                "avg",
                "total",
                "percentiles",
            ],
            &["name", "calls", "sampled_calls", "avg", "total"],
        ),
        "sql" => (
            &[
                "query",
                "source",
                "route",
                "count",
                "avg",
                "total",
                "percentiles",
            ],
            &["query", "count", "avg", "total", "percentiles"],
        ),
        "server" => (
            &[
                "route",
                "count",
                "status_4xx",
                "status_5xx",
                "avg",
                "total",
                "percentiles",
                "sql_per_request",
            ],
            &[
                "route",
                "count",
                "status_4xx",
                "status_5xx",
                "avg",
                "total",
                "percentiles",
            ],
        ),
        _ => bail!("unsupported profiler report"),
    };
    let data = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .context("invalid profiler report")?;
    let mut rows = Vec::new();
    for row in data.iter().take(100) {
        let object = row.as_object().context("invalid profiler row")?;
        if required.iter().any(|field| !object.contains_key(*field)) {
            bail!("missing required profiler field");
        }
        let mut safe = serde_json::Map::new();
        for &field in fields {
            // JsonFunctionEntry flattens its percentile map; the other reports nest it.
            let v = if field == "percentiles" && path == "functions_timing" {
                row
            } else {
                let Some(v) = object.get(field) else { continue };
                v
            };
            match field {
                "percentiles" => {
                    let object = v.as_object().context("invalid profiler percentiles")?;
                    let mut percentiles = serde_json::Map::new();
                    for key in ["p50", "p95", "p99", "p99.9"] {
                        if let Some(value) = object.get(key) {
                            let text = value
                                .as_str()
                                .filter(|text| text.len() <= 64)
                                .context("invalid profiler percentile")?;
                            percentiles.insert(key.into(), json!(text));
                        }
                    }
                    safe.insert(field.into(), json!(percentiles));
                }
                "count" | "calls" | "sampled_calls" | "status_4xx" | "status_5xx" => {
                    let count = v.as_u64().context("invalid profiler count")?;
                    safe.insert(field.into(), json!(count));
                }
                "sql_per_request" => {
                    if !v.is_null()
                        && !v
                            .as_f64()
                            .is_some_and(|number| number.is_finite() && number >= 0.0)
                    {
                        bail!("invalid profiler per-request count");
                    }
                    safe.insert(field.into(), v.clone());
                }
                "source" | "route" if v.is_null() && path == "sql" => {
                    safe.insert(field.into(), serde_json::Value::Null);
                }
                _ => {
                    let limit = match field {
                        "avg" | "total" => 64,
                        // Generated triggers combine multiple long statements. Bound
                        // them by the whole report, rather than rejecting valid SQL.
                        "query" => REPORT_MAX_BYTES,
                        _ => 4096,
                    };
                    let text = v
                        .as_str()
                        .with_context(|| format!("invalid profiler text type: {field}"))?;
                    if text.len() > limit {
                        bail!("profiler text limit exceeded: {field}");
                    }
                    safe.insert(field.into(), json!(text));
                }
            }
        }
        rows.push(json!(safe));
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_contract_preserves_sample_counts_and_unknown_timing() {
        // Use the dependency's serialized model, including its flattened percentiles.
        let mut row = serde_json::to_value(hotpath::json::JsonFunctionEntry {
            id: 1,
            name: "batch_flush".into(),
            calls: 40,
            sampled_calls: 0,
            avg: "-".into(),
            total: "-".into(),
            percent_total: "-".into(),
            percentiles: HashMap::from([("p95".into(), "-".into())]),
            location: None,
        })
        .unwrap();
        row["arbitrary"] = json!("secret");
        row["location"] = json!("private path");
        row["raw"] = json!("secret");
        let rows = sanitize_report("functions_timing", &json!({"data":vec![row;101]})).unwrap();
        assert_eq!(rows.len(), 100);
        assert_eq!(rows[0]["calls"], 40);
        assert_eq!(rows[0]["sampled_calls"], 0);
        assert_eq!(rows[0]["percentiles"], json!({"p95":"-"}));
        assert!(rows[0].get("location").is_none());
        assert!(rows[0].get("raw").is_none());
    }

    #[test]
    fn report_requests_match_hotpath_raw_header_authentication() {
        let client = reqwest::Client::new();
        let request = report_request(&client, "server", Some("internal-token"))
            .build()
            .unwrap();
        assert_eq!(request.url().as_str(), "http://127.0.0.1:6770/server");
        assert_eq!(request.headers()[header::AUTHORIZATION], "internal-token");
        let request = report_request(&client, "sql", None).build().unwrap();
        assert!(!request.headers().contains_key(header::AUTHORIZATION));
    }

    #[test]
    fn sql_report_contract_uses_hotpath_serializer_shape() {
        let statement =
            prompt_cache_working_set_live_refresh_sql_for_key("NEW.prompt_cache_key", "NEW.id");
        assert!(
            statement.contains("INDEXED BY idx_codex_invocations_prompt_cache_key_occurred_at")
        );
        assert!(statement.contains("FROM codex_invocations WHERE id IN"));
        assert!(!statement.contains("NOT INDEXED"));
        // Trigger refreshes compose several of the real generated statements.
        let query = crate::observability::hotpath_sql_normalization::normalize(&format!(
            "CREATE TRIGGER fixture AFTER UPDATE ON codex_invocations BEGIN {statement}; {statement}; {statement}; END"
        ));
        assert!(
            query.len() > 16 * 1024,
            "exercise the actual long startup statement"
        );
        let row = serde_json::to_value(hotpath::json::JsonSqlEntry {
            id: 1,
            query: query.clone(),
            source: None,
            route: None,
            count: 3,
            avg: "1ms".into(),
            total: "3ms".into(),
            percent_total: "100%".into(),
            percentiles: HashMap::from([("p95".into(), "1ms".into())]),
            location: None,
        })
        .unwrap();
        let rows = sanitize_report("sql", &json!({"data": [row]})).unwrap();
        assert_eq!(rows[0]["query"], query);
        assert_eq!(rows[0]["count"], 3);
        assert_eq!(rows[0]["percentiles"]["p95"], "1ms");
    }

    #[test]
    fn sql_report_serializer_preserves_only_redacted_sqlite_templates() {
        let query = crate::observability::hotpath_sql_normalization::normalize(
            "SELECT \"private-value\", 'private-value', 0xCAFE, .125e+2 FROM t /* private-value */",
        );
        let row = serde_json::to_value(hotpath::json::JsonSqlEntry {
            id: 1,
            query,
            source: None,
            route: None,
            count: 3,
            avg: "1ms".into(),
            total: "3ms".into(),
            percent_total: "100%".into(),
            percentiles: HashMap::from([("p95".into(), "1ms".into())]),
            location: None,
        })
        .unwrap();
        let rows = sanitize_report("sql", &json!({"data": [row]})).unwrap();
        assert_eq!(rows[0]["query"], "SELECT ?, ?, ?, ? FROM t");
        assert_eq!(rows[0]["count"], 3);
        assert!(
            !serde_json::to_string(&rows)
                .unwrap()
                .contains("private-value")
        );
    }

    #[test]
    fn reports_reject_malformed_or_unbounded_fields() {
        let row = json!({"query":"SELECT ?","source":null,"route":null,"count":1,"avg":"1ms","total":"1ms","percentiles":{"p95":"1ms"}});
        assert!(sanitize_report("sql", &json!({"data":[row.clone()]})).is_ok());
        for (field, value) in [
            ("count", json!("1")),
            ("avg", json!(2)),
            ("query", json!("x".repeat(REPORT_MAX_BYTES + 1))),
            ("percentiles", json!({"p95":{"secret":1}})),
        ] {
            let mut invalid = row.clone();
            invalid[field] = value;
            assert!(
                sanitize_report("sql", &json!({"data":[invalid]})).is_err(),
                "{field}"
            );
        }
        assert!(sanitize_report("sql", &json!({"data":[{}]})).is_err());
        assert!(sanitize_report("arbitrary", &json!({"data":[]})).is_err());
    }
}
