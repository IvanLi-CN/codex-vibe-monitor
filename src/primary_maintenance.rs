use crate::primary_store::PrimaryStateStore;
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use flate2::{Compression, write::GzEncoder};
use serde_json::{Value, json};
use sha2::Digest;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

#[derive(Debug, Clone)]
pub(crate) struct PrimaryMaintenanceResult {
    pub(crate) task_key: String,
    pub(crate) status: &'static str,
    pub(crate) summary: Value,
}

#[async_trait]
pub(crate) trait PrimaryMaintenanceExecutor: Send + Sync {
    async fn execute(&self, task_key: &str) -> Result<PrimaryMaintenanceResult>;
}

#[derive(Debug, Clone)]
pub(crate) struct PostgresPrimaryMaintenanceExecutor {
    store: Arc<dyn PrimaryStateStore>,
    asset_root: PathBuf,
}

fn camel_case_key(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut uppercase = false;
    for ch in value.chars() {
        if ch == '_' {
            uppercase = true;
        } else if uppercase {
            output.extend(ch.to_uppercase());
            uppercase = false;
        } else {
            output.push(ch);
        }
    }
    output
}

fn find_usage_object(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    match value {
        Value::Object(object) => {
            if let Some(Value::Object(usage)) = object.get("usage") {
                return Some(usage);
            }
            object.values().find_map(find_usage_object)
        }
        Value::Array(values) => values.iter().find_map(find_usage_object),
        _ => None,
    }
}

fn usage_i64(object: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        object.get(*key).and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        })
    })
}

fn row_i64(row: &Value, key: &str) -> Option<i64> {
    row.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
    })
}

fn row_f64(row: &Value, key: &str) -> Option<f64> {
    row.get(key).and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_i64().map(|value| value as f64))
    })
}

impl PostgresPrimaryMaintenanceExecutor {
    pub(crate) fn from_environment(store: Arc<dyn PrimaryStateStore>) -> Self {
        let asset_root = std::env::var_os("DATABASE_PATH")
            .map(PathBuf::from)
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        Self { store, asset_root }
    }

    async fn verify_archive_storage(&self) -> Result<Value> {
        let assets = self.store.latest_materialized_rows("asset_index").await?;
        let mut checked_assets = 0_u64;
        let mut checked_bytes = 0_u64;
        for asset in assets {
            let relative = asset
                .get("relativePath")
                .or_else(|| asset.get("relative_path"))
                .and_then(Value::as_str)
                .context("asset index path is missing")?;
            let relative_path = Path::new(relative);
            if relative_path.is_absolute()
                || relative_path
                    .components()
                    .any(|component| component == std::path::Component::ParentDir)
            {
                bail!("asset path escapes storage root: {relative}");
            }
            let path = self.asset_root.join(relative_path);
            let metadata = tokio::fs::metadata(&path)
                .await
                .with_context(|| format!("asset is unavailable: {relative}"))?;
            let expected_size = asset
                .get("sizeBytes")
                .or_else(|| asset.get("size_bytes"))
                .and_then(Value::as_i64)
                .context("asset size is missing")?;
            if expected_size < 0 || metadata.len() != expected_size as u64 {
                bail!("asset size mismatch: {relative}");
            }
            let expected_sha = asset
                .get("sha256")
                .and_then(Value::as_str)
                .context("asset checksum is missing")?;
            let bytes = tokio::fs::read(&path)
                .await
                .with_context(|| format!("asset cannot be read: {relative}"))?;
            let actual_sha = format!("{:x}", sha2::Sha256::digest(&bytes));
            if actual_sha != expected_sha {
                bail!("asset checksum mismatch: {relative}");
            }
            checked_assets += 1;
            checked_bytes = checked_bytes.saturating_add(metadata.len());
        }
        Ok(json!({
            "status": "succeeded",
            "checkedAssets": checked_assets,
            "checkedBytes": checked_bytes,
            "completedAt": chrono::Utc::now().to_rfc3339()
        }))
    }

    async fn refresh_upstream_activity_manifest(&self) -> Result<Value> {
        let attempts = self
            .store
            .latest_materialized_rows("pool_upstream_request_attempts")
            .await?;
        let invocations = self
            .store
            .latest_materialized_rows("codex_invocations")
            .await?;
        let latest_attempt_at = attempts
            .iter()
            .filter_map(|row| {
                row.get("occurredAt")
                    .or_else(|| row.get("occurred_at"))
                    .and_then(Value::as_str)
            })
            .max()
            .map(str::to_owned);
        let summary = json!({
            "status": "succeeded",
            "attemptCount": attempts.len(),
            "invocationCount": invocations.len(),
            "latestAttemptAt": latest_attempt_at,
            "completedAt": chrono::Utc::now().to_rfc3339()
        });
        self.store
            .put_json("maintenance.archive_upstream_activity_manifest", &summary)
            .await?;
        Ok(summary)
    }

    async fn materialize_historical_rollups(&self) -> Result<Value> {
        let archive_batches = self
            .store
            .latest_materialized_rows("archive_batches")
            .await?;
        let rollups = self
            .store
            .latest_materialized_rows("invocation_hourly_rollups")
            .await?;
        let summary = json!({
            "status": "succeeded",
            "archiveBatchCount": archive_batches.len(),
            "rollupCount": rollups.len(),
            "materializedAt": chrono::Utc::now().to_rfc3339()
        });
        self.store
            .put_json("maintenance.materialize_historical_rollups", &summary)
            .await?;
        Ok(summary)
    }

    async fn refresh_projection_snapshot(
        &self,
        task_key: &str,
        object_names: &[&str],
    ) -> Result<Value> {
        let mut counts = serde_json::Map::new();
        let mut total_rows = 0_usize;
        for object_name in object_names {
            let count = self
                .store
                .latest_materialized_rows(object_name)
                .await?
                .len();
            total_rows = total_rows.saturating_add(count);
            counts.insert((*object_name).to_owned(), json!(count));
        }
        let summary = json!({
            "status": "succeeded",
            "taskKey": task_key,
            "sourceRowCounts": counts,
            "sourceRowCount": total_rows,
            "refreshedAt": chrono::Utc::now().to_rfc3339()
        });
        self.store
            .put_json(&format!("maintenance.{task_key}"), &summary)
            .await?;
        Ok(summary)
    }

    async fn refresh_startup_backfill(&self, task_key: &str) -> Result<Value> {
        let suffix = task_key
            .strip_prefix("startup_backfill.")
            .unwrap_or(task_key);
        let objects: &[&str] = match suffix {
            "proxy_usage" | "proxy_cost" => &["proxy_requests"],
            "prompt_cache_key" | "prompt_cache_conversations_materialization" => {
                &["prompt_cache_conversations"]
            }
            "requested_service_tier"
            | "invocation_service_tier"
            | "reasoning_effort"
            | "failure_classification"
            | "legacy_detail_mirrors" => &["codex_invocations", "invocation_details"],
            "pool_attempt_public_id_live" | "pool_attempt_public_id_archives" => {
                &["pool_upstream_request_attempts"]
            }
            "upstream_activity_live" | "upstream_activity_archives" => {
                &["pool_upstream_request_attempts", "codex_invocations"]
            }
            "pool_upstream_node_health_archives" => &["pool_upstream_node_health"],
            "account_activity_v2_coverage" => &["pool_upstream_accounts"],
            "historical_rollups" => &["archive_batches", "invocation_hourly_rollups"],
            _ => &["codex_invocations"],
        };
        let text_fields = match suffix {
            "prompt_cache_key" => {
                Some(("prompt_cache_key", "request_raw_path", "prompt_cache_key"))
            }
            "requested_service_tier" => Some((
                "requested_service_tier",
                "request_raw_path",
                "requested_service_tier",
            )),
            "invocation_service_tier" => {
                Some(("service_tier", "response_raw_path", "service_tier"))
            }
            "reasoning_effort" => {
                Some(("reasoning_effort", "request_raw_path", "reasoning_effort"))
            }
            _ => None,
        };
        if let Some((field, path_field, json_key)) = text_fields {
            return self
                .backfill_invocation_text_field(task_key, field, path_field, json_key)
                .await;
        }
        if suffix == "failure_classification" {
            return self.backfill_failure_classification().await;
        }
        if suffix == "proxy_usage" {
            return self.backfill_invocation_usage().await;
        }
        if suffix == "proxy_cost" {
            return self.backfill_invocation_cost().await;
        }
        self.refresh_projection_snapshot(task_key, objects).await
    }

    async fn backfill_invocation_cost(&self) -> Result<Value> {
        let rows = self
            .store
            .latest_materialized_rows("codex_invocations")
            .await?;
        let pricing = self
            .store
            .latest_materialized_rows("pricing_settings_models")
            .await?;
        let mut scanned = 0_u64;
        let mut updated = 0_u64;
        let mut skipped = 0_u64;
        for row in rows {
            if row.get("cost").is_some_and(|value| !value.is_null()) {
                continue;
            }
            let Some(model) = row.get("model").and_then(Value::as_str) else {
                continue;
            };
            scanned += 1;
            let Some(price) = pricing
                .iter()
                .find(|entry| entry.get("model").and_then(Value::as_str) == Some(model))
            else {
                skipped += 1;
                continue;
            };
            let input = row_i64(&row, "input_tokens").unwrap_or_default() as f64;
            let output = row_i64(&row, "output_tokens").unwrap_or_default() as f64;
            let cache_input = row_i64(&row, "cache_input_tokens").unwrap_or_default() as f64;
            let cache_write =
                row_i64(&row, "reported_cache_write_tokens").unwrap_or_default() as f64;
            let reasoning = row_i64(&row, "reasoning_tokens").unwrap_or_default() as f64;
            let cost = input * row_f64(price, "input_per_1m").unwrap_or_default()
                + output * row_f64(price, "output_per_1m").unwrap_or_default()
                + cache_input * row_f64(price, "cache_read_per_1m").unwrap_or_default()
                + cache_write * row_f64(price, "cache_write_per_1m").unwrap_or_default()
                + reasoning * row_f64(price, "reasoning_per_1m").unwrap_or_default();
            let cost = cost / 1_000_000.0;
            let Some(source_key) = row
                .get("sourceKey")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    row.get("id")
                        .map(|value| serde_json::json!({"id": value}).to_string())
                })
            else {
                skipped += 1;
                continue;
            };
            self.store
                .update_materialized_float_fields(
                    "codex_invocations",
                    &source_key,
                    &[("cost", cost)],
                )
                .await?;
            updated += 1;
        }
        let summary = json!({
            "status": "succeeded",
            "taskKey": "startup_backfill.proxy_cost",
            "scanned": scanned,
            "updated": updated,
            "skipped": skipped,
            "pricingRows": pricing.len(),
            "coverage": "pg_materialized_pricing",
            "completedAt": Utc::now().to_rfc3339()
        });
        self.store
            .put_json("maintenance.startup_backfill.proxy_cost", &summary)
            .await?;
        Ok(summary)
    }

    async fn backfill_invocation_usage(&self) -> Result<Value> {
        let rows = self
            .store
            .latest_materialized_rows("codex_invocations")
            .await?;
        if rows.is_empty() {
            return self
                .refresh_projection_snapshot("startup_backfill.proxy_usage", &["proxy_requests"])
                .await;
        }
        let mut scanned = 0_u64;
        let mut updated = 0_u64;
        let mut skipped = 0_u64;
        for row in rows {
            let Some(relative) = row
                .get("response_raw_path")
                .or_else(|| row.get("responseRawPath"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let needs_usage = [
                "input_tokens",
                "output_tokens",
                "cache_input_tokens",
                "reasoning_tokens",
                "total_tokens",
            ]
            .iter()
            .any(|field| row.get(*field).is_none_or(Value::is_null));
            if !needs_usage {
                continue;
            }
            scanned += 1;
            let Some(payload) = self.read_asset_json(relative).await? else {
                skipped += 1;
                continue;
            };
            let Some(usage) = find_usage_object(&payload) else {
                skipped += 1;
                continue;
            };
            let input = usage_i64(usage, &["input_tokens", "inputTokens"]);
            let output = usage_i64(usage, &["output_tokens", "outputTokens"]);
            let cache_input = usage_i64(
                usage,
                &[
                    "cache_read_input_tokens",
                    "cache_input_tokens",
                    "cacheInputTokens",
                ],
            );
            let reasoning = usage_i64(usage, &["reasoning_tokens", "reasoningTokens"]);
            let total = usage_i64(usage, &["total_tokens", "totalTokens"])
                .or_else(|| input.zip(output).map(|(left, right)| left + right));
            let mut fields = Vec::new();
            if let Some(value) = input {
                fields.push(("input_tokens", value));
            }
            if let Some(value) = output {
                fields.push(("output_tokens", value));
            }
            if let Some(value) = cache_input {
                fields.push(("cache_input_tokens", value));
            }
            if let Some(value) = reasoning {
                fields.push(("reasoning_tokens", value));
            }
            if let Some(value) = total {
                fields.push(("total_tokens", value));
            }
            if fields.is_empty() {
                skipped += 1;
                continue;
            }
            let Some(source_key) = row
                .get("sourceKey")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    row.get("id")
                        .map(|value| serde_json::json!({"id": value}).to_string())
                })
            else {
                skipped += 1;
                continue;
            };
            self.store
                .update_materialized_integer_fields("codex_invocations", &source_key, &fields)
                .await?;
            updated += 1;
        }
        let summary = json!({
            "status": "succeeded",
            "taskKey": "startup_backfill.proxy_usage",
            "scanned": scanned,
            "updated": updated,
            "skipped": skipped,
            "coverage": "pg_materialized_invocations",
            "completedAt": Utc::now().to_rfc3339()
        });
        self.store
            .put_json("maintenance.startup_backfill.proxy_usage", &summary)
            .await?;
        Ok(summary)
    }

    async fn backfill_failure_classification(&self) -> Result<Value> {
        let rows = self
            .store
            .latest_materialized_rows("codex_invocations")
            .await?;
        let mut scanned = 0_u64;
        let mut updated = 0_u64;
        for row in rows {
            let status = row.get("status").and_then(Value::as_str);
            let failure_kind = row.get("failure_kind").and_then(Value::as_str);
            let failure_class = row.get("failure_class").and_then(Value::as_str);
            let actionable = row.get("is_actionable").and_then(Value::as_i64);
            let classification = crate::api::resolve_failure_classification(
                status,
                row.get("error_message").and_then(Value::as_str),
                failure_kind,
                failure_class,
                actionable,
            );
            let needs_kind = failure_kind.is_none_or(|value| value.trim().is_empty())
                && classification.failure_kind.is_some();
            let needs_class = failure_class.is_none_or(|value| value.trim().is_empty())
                && classification.failure_class != crate::api::FailureClass::None;
            let expected_actionable = i64::from(classification.is_actionable);
            let needs_actionable = actionable != Some(expected_actionable);
            if !needs_kind && !needs_class && !needs_actionable {
                continue;
            }
            scanned += 1;
            let Some(source_key) = row
                .get("sourceKey")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    row.get("id")
                        .map(|value| serde_json::json!({"id": value}).to_string())
                })
            else {
                continue;
            };
            let mut fields = Vec::new();
            if needs_kind && let Some(value) = classification.failure_kind.as_deref() {
                fields.push(("failure_kind", value));
            }
            if needs_class {
                fields.push(("failure_class", classification.failure_class.as_str()));
            }
            self.store
                .update_materialized_text_fields("codex_invocations", &source_key, &fields)
                .await?;
            if needs_actionable {
                self.store
                    .update_materialized_integer_fields(
                        "codex_invocations",
                        &source_key,
                        &[("is_actionable", expected_actionable)],
                    )
                    .await?;
            }
            updated += 1;
        }
        let summary = json!({
            "status": "succeeded",
            "taskKey": "startup_backfill.failure_classification",
            "scanned": scanned,
            "updated": updated,
            "coverage": "pg_materialized_invocations",
            "completedAt": Utc::now().to_rfc3339()
        });
        self.store
            .put_json(
                "maintenance.startup_backfill.failure_classification",
                &summary,
            )
            .await?;
        Ok(summary)
    }

    async fn read_asset_json(&self, relative: &str) -> Result<Option<Value>> {
        let path = Path::new(relative);
        if path.is_absolute()
            || path
                .components()
                .any(|component| component == std::path::Component::ParentDir)
        {
            return Ok(None);
        }
        let full_path = self.asset_root.join(path);
        let bytes = match tokio::fs::read(&full_path).await {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let bytes = if relative.ends_with(".gz") {
            let mut decoder = flate2::read::GzDecoder::new(bytes.as_slice());
            let mut decoded = Vec::new();
            std::io::Read::read_to_end(&mut decoder, &mut decoded)?;
            decoded
        } else {
            bytes
        };
        Ok(serde_json::from_slice(&bytes).ok())
    }

    async fn backfill_invocation_text_field(
        &self,
        task_key: &str,
        field: &str,
        path_field: &str,
        json_key: &str,
    ) -> Result<Value> {
        let rows = self
            .store
            .latest_materialized_rows("codex_invocations")
            .await?;
        let mut scanned = 0_u64;
        let mut updated = 0_u64;
        let mut skipped_missing_file = 0_u64;
        let mut skipped_missing_value = 0_u64;
        for row in rows {
            let current = row
                .get(field)
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty());
            if current.is_some() {
                continue;
            }
            scanned += 1;
            let Some(relative) = row
                .get(path_field)
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            else {
                skipped_missing_file += 1;
                continue;
            };
            let Some(payload) = self.read_asset_json(relative).await? else {
                skipped_missing_file += 1;
                continue;
            };
            let value = payload
                .get(json_key)
                .or_else(|| payload.get(camel_case_key(json_key)))
                .and_then(Value::as_str)
                .filter(|value| !value.trim().is_empty());
            let Some(value) = value else {
                skipped_missing_value += 1;
                continue;
            };
            let Some(source_key) = row
                .get("sourceKey")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| {
                    row.get("id")
                        .map(|value| serde_json::json!({"id": value}).to_string())
                })
            else {
                skipped_missing_value += 1;
                continue;
            };
            self.store
                .update_materialized_text_fields(
                    "codex_invocations",
                    &source_key,
                    &[(field, value)],
                )
                .await?;
            updated += 1;
        }
        let summary = json!({
            "status": "succeeded",
            "taskKey": task_key,
            "field": field,
            "scanned": scanned,
            "updated": updated,
            "skippedMissingFile": skipped_missing_file,
            "skippedMissingValue": skipped_missing_value,
            "completedAt": Utc::now().to_rfc3339()
        });
        self.store
            .put_json(&format!("maintenance.{task_key}"), &summary)
            .await?;
        Ok(summary)
    }

    async fn cleanup_invocation_identities(&self) -> Result<Value> {
        const GRACE_MINUTES: i64 = 5;
        let conversations = self
            .store
            .latest_materialized_rows("prompt_cache_conversations")
            .await?;
        let invocations = self
            .store
            .latest_materialized_rows("codex_invocations")
            .await?;
        let referenced_keys = invocations
            .iter()
            .filter_map(|row| {
                row.get("promptCacheKey")
                    .or_else(|| row.get("prompt_cache_key"))
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
            .collect::<std::collections::HashSet<_>>();
        let cutoff = Utc::now() - chrono::Duration::minutes(GRACE_MINUTES);
        let mut checked = 0_u64;
        let mut eligible = Vec::new();
        for row in conversations {
            let Some(key) = row
                .get("promptCacheKey")
                .or_else(|| row.get("prompt_cache_key"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            checked += 1;
            if referenced_keys.contains(key) {
                continue;
            }
            let last = row
                .get("updatedAt")
                .or_else(|| row.get("updated_at"))
                .or_else(|| row.get("createdAt"))
                .or_else(|| row.get("created_at"))
                .and_then(Value::as_str)
                .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&Utc));
            if last.is_some_and(|value| value < cutoff) {
                eligible.push(key.to_owned());
            }
        }
        eligible.sort();
        eligible.dedup();
        let released = self
            .store
            .delete_materialized_rows_by_source_keys("prompt_cache_conversations", &eligible)
            .await?;
        let summary = json!({
            "status": "succeeded",
            "conversationIdentitiesChecked": checked,
            "conversationIdentitiesReleased": released,
            "hoursChecked": Value::Null,
            "hoursReleased": Value::Null,
            "coverage": "pg_materialized_prompt_cache",
            "graceMinutes": GRACE_MINUTES,
            "completedAt": Utc::now().to_rfc3339()
        });
        self.store
            .put_json("maintenance.invocation_identity_cleanup", &summary)
            .await?;
        Ok(summary)
    }

    async fn sweep_raw_orphans(&self) -> Result<Value> {
        let assets = self.store.latest_materialized_rows("asset_index").await?;
        let referenced = assets
            .iter()
            .filter(|asset| {
                asset
                    .get("assetKind")
                    .or_else(|| asset.get("asset_kind"))
                    .and_then(Value::as_str)
                    == Some("proxy_raw")
            })
            .filter_map(|asset| {
                asset
                    .get("relativePath")
                    .or_else(|| asset.get("relative_path"))
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
            })
            .collect::<std::collections::HashSet<_>>();
        if referenced.is_empty() {
            let summary = json!({
                "status": "deferred",
                "coverage": "unknown",
                "reason": "proxy_raw_asset_index_empty"
            });
            self.store
                .put_json("maintenance.raw_orphan_sweep", &summary)
                .await?;
            return Ok(summary);
        }
        let raw_root = std::env::var_os("PROXY_RAW_DIR")
            .map(PathBuf::from)
            .map(|path| {
                if path.is_absolute() {
                    path
                } else {
                    self.asset_root.join(path)
                }
            })
            .unwrap_or_else(|| self.asset_root.join("proxy_raw_payloads"));
        let mut pending = vec![raw_root.clone()];
        let mut checked = 0_u64;
        let mut removed = 0_u64;
        let mut removed_bytes = 0_u64;
        let cutoff = SystemTime::now()
            .checked_sub(Duration::from_secs(5 * 60))
            .unwrap_or(SystemTime::UNIX_EPOCH);
        while let Some(path) = pending.pop() {
            let mut entries = match tokio::fs::read_dir(&path).await {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            while let Some(entry) = entries.next_entry().await? {
                let entry_path = entry.path();
                let file_type = entry.file_type().await?;
                if file_type.is_dir() {
                    pending.push(entry_path);
                    continue;
                }
                if !file_type.is_file() {
                    continue;
                }
                checked += 1;
                let relative = entry_path
                    .strip_prefix(&self.asset_root)
                    .ok()
                    .map(PathBuf::from);
                let metadata = entry.metadata().await?;
                if relative
                    .as_ref()
                    .is_some_and(|relative| referenced.contains(relative))
                    || metadata.modified().unwrap_or(SystemTime::now()) >= cutoff
                {
                    continue;
                }
                let bytes = metadata.len();
                tokio::fs::remove_file(&entry_path).await?;
                removed += 1;
                removed_bytes = removed_bytes.saturating_add(bytes);
            }
        }
        let summary = json!({
            "status": "succeeded",
            "coverage": "pg_asset_index",
            "filesChecked": checked,
            "filesReleased": removed,
            "bytesReleased": removed_bytes,
            "completedAt": Utc::now().to_rfc3339()
        });
        self.store
            .put_json("maintenance.raw_orphan_sweep", &summary)
            .await?;
        Ok(summary)
    }

    async fn prepare_performance_indexes(&self) -> Result<Value> {
        let summary = json!({
            "status": "succeeded",
            "managedBy": "cvm_migration.schema_checksum",
            "ddlApplied": false,
            "reason": "PostgreSQL primary indexes are owned by schema migrations",
            "completedAt": chrono::Utc::now().to_rfc3339()
        });
        self.store
            .put_json("maintenance.prepare_performance_indexes", &summary)
            .await?;
        Ok(summary)
    }

    async fn prune_archive_batches(&self, legacy_only: bool) -> Result<Value> {
        let rows = self
            .store
            .latest_materialized_rows("archive_batches")
            .await?;
        let now = chrono::Utc::now();
        let mut source_keys = Vec::new();
        let mut removed_files = 0_u64;
        let mut skipped_unsafe_paths = 0_u64;
        for row in rows {
            let status = row
                .get("status")
                .or_else(|| row.get("archiveStatus"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            let legacy = row
                .get("summarySourceKind")
                .or_else(|| row.get("summary_source_kind"))
                .and_then(Value::as_str)
                .map(|value| value.eq_ignore_ascii_case("legacy"))
                .unwrap_or(false);
            if legacy_only != legacy {
                continue;
            }
            let expired = row
                .get("expiresAt")
                .or_else(|| row.get("expires_at"))
                .and_then(Value::as_str)
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.with_timezone(&chrono::Utc) <= now)
                .unwrap_or_else(|| matches!(status, "expired" | "pruned"));
            if !expired || !matches!(status, "completed" | "expired" | "pruned") {
                continue;
            }
            if let Some(file_path) = row
                .get("filePath")
                .or_else(|| row.get("file_path"))
                .and_then(Value::as_str)
                && !file_path.is_empty()
            {
                let relative_path = Path::new(file_path);
                if relative_path.is_absolute()
                    || relative_path
                        .components()
                        .any(|component| component == std::path::Component::ParentDir)
                {
                    skipped_unsafe_paths += 1;
                    continue;
                }
                let path = self.asset_root.join(relative_path);
                if path.exists() {
                    tokio::fs::remove_file(&path).await.with_context(|| {
                        format!("failed to remove expired archive {}", path.display())
                    })?;
                    removed_files += 1;
                }
            }
            if let Some(id) = row.get("id").and_then(Value::as_i64) {
                source_keys.push(json!({"id": id}).to_string());
            }
        }
        let deleted_rows = self
            .store
            .delete_materialized_rows_by_source_keys("archive_batches", &source_keys)
            .await?;
        let task_key = if legacy_only {
            "prune_legacy_archive_batches"
        } else {
            "prune_archive_batches"
        };
        let summary = json!({
            "status": "succeeded",
            "deletedRows": deleted_rows,
            "removedFiles": removed_files,
            "skippedUnsafePaths": skipped_unsafe_paths,
            "completedAt": now.to_rfc3339()
        });
        self.store
            .put_json(&format!("maintenance.{task_key}"), &summary)
            .await?;
        Ok(summary)
    }

    async fn compress_raw_payloads(&self) -> Result<Value> {
        let mut compressed = 0_u64;
        let mut skipped = 0_u64;
        for (object_name, path_field, codec_field) in [
            ("codex_invocations", "request_raw_path", "request_raw_codec"),
            (
                "codex_invocations",
                "response_raw_path",
                "response_raw_codec",
            ),
            (
                "pool_upstream_request_attempts",
                "response_raw_path",
                "response_raw_codec",
            ),
        ] {
            let rows = self.store.latest_materialized_rows(object_name).await?;
            for row in rows {
                let path_value = row
                    .get(path_field)
                    .or_else(|| row.get(camel_case_field(path_field)))
                    .and_then(Value::as_str);
                let Some(path_value) = path_value else {
                    continue;
                };
                let codec = row
                    .get(codec_field)
                    .or_else(|| row.get(camel_case_field(codec_field)))
                    .and_then(Value::as_str)
                    .unwrap_or("identity");
                if codec != "identity" {
                    continue;
                }
                let relative = Path::new(path_value);
                if relative.is_absolute()
                    || relative
                        .components()
                        .any(|component| component == std::path::Component::ParentDir)
                {
                    skipped += 1;
                    continue;
                }
                let source_key = materialized_source_key(&row);
                let Some(source_key) = source_key else {
                    skipped += 1;
                    continue;
                };
                let source = self.asset_root.join(relative);
                let bytes = match tokio::fs::read(&source).await {
                    Ok(bytes) => bytes,
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                };
                let compressed_bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
                    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
                    encoder.write_all(&bytes)?;
                    Ok(encoder.finish()?)
                })
                .await
                .context("raw payload compression task failed")??;
                let compressed_relative = PathBuf::from(format!("{path_value}.gz"));
                let compressed_path = self.asset_root.join(&compressed_relative);
                let temporary_path = PathBuf::from(format!("{}.tmp", compressed_path.display()));
                tokio::fs::write(&temporary_path, compressed_bytes)
                    .await
                    .with_context(|| format!("failed to write compressed payload {path_value}"))?;
                tokio::fs::rename(&temporary_path, &compressed_path)
                    .await
                    .with_context(|| {
                        format!("failed to publish compressed payload {path_value}")
                    })?;
                let compressed_path_value = compressed_relative.to_string_lossy().into_owned();
                if let Err(error) = self
                    .store
                    .update_materialized_text_fields(
                        object_name,
                        &source_key,
                        &[(path_field, &compressed_path_value), (codec_field, "gzip")],
                    )
                    .await
                {
                    let _ = tokio::fs::remove_file(&compressed_path).await;
                    return Err(error);
                }
                tokio::fs::remove_file(&source).await.with_context(|| {
                    format!("failed to remove identity payload after compression {path_value}")
                })?;
                compressed += 1;
            }
        }
        let summary = json!({
            "status": "succeeded",
            "compressed": compressed,
            "skipped": skipped,
            "completedAt": chrono::Utc::now().to_rfc3339()
        });
        self.store
            .put_json("maintenance.raw_compression", &summary)
            .await?;
        Ok(summary)
    }

    async fn prune_expired_object(&self, task_key: &str, object_name: &str) -> Result<Value> {
        let rows = self.store.latest_materialized_rows(object_name).await?;
        let now = chrono::Utc::now();
        let retention_days = self
            .store
            .get_json("core.retention_policy.v1")
            .await?
            .and_then(|policy| {
                let key = match task_key {
                    "invocation_detail_prune" => "invocationMaxDays",
                    "quota_snapshot_compaction" => "quotaSnapshotFullDays",
                    "pool_upstream_attempt_archive" => "poolUpstreamRequestAttemptsRetentionDays",
                    "forward_proxy_attempt_archive" => "forwardProxyAttemptsRetentionDays",
                    _ => return None,
                };
                policy.get(key).and_then(Value::as_u64)
            });
        let source_keys = rows
            .iter()
            .filter_map(|row| {
                let explicit_expiry = row
                    .get("expiresAt")
                    .or_else(|| row.get("expires_at"))
                    .and_then(Value::as_str)
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok());
                let expiry = explicit_expiry.or_else(|| {
                    let days = retention_days?;
                    let occurred_at = row
                        .get("occurredAt")
                        .or_else(|| row.get("occurred_at"))
                        .or_else(|| row.get("createdAt"))
                        .or_else(|| row.get("created_at"))
                        .and_then(Value::as_str)
                        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())?;
                    Some(occurred_at + chrono::Duration::days(days as i64))
                });
                if expiry?.with_timezone(&chrono::Utc) > now {
                    return None;
                }
                materialized_source_key(row)
            })
            .collect::<Vec<_>>();
        let deleted_rows = self
            .store
            .delete_materialized_rows_by_source_keys(object_name, &source_keys)
            .await?;
        let summary = json!({
            "status": "succeeded",
            "object": object_name,
            "candidateRows": source_keys.len(),
            "deletedRows": deleted_rows,
            "completedAt": now.to_rfc3339()
        });
        self.store
            .put_json(&format!("maintenance.{task_key}"), &summary)
            .await?;
        Ok(summary)
    }

    async fn refresh_forward_proxy_subscriptions(&self) -> Result<Value> {
        let settings = self
            .store
            .get_json("core.forward_proxy_settings.v1")
            .await?
            .unwrap_or_else(|| json!({}));
        let subscription_urls = settings
            .get("subscriptionUrls")
            .or_else(|| settings.get("subscription_urls"))
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("failed to build PostgreSQL primary subscription client")?;
        let mut proxy_urls = Vec::new();
        for subscription_url in &subscription_urls {
            let fetched = crate::forward_proxy::fetch_subscription_proxy_urls(
                &client,
                subscription_url,
                Duration::from_secs(30),
            )
            .await
            .with_context(|| format!("failed to refresh subscription {subscription_url}"))?;
            proxy_urls.extend(fetched);
        }
        proxy_urls.sort();
        proxy_urls.dedup();
        let summary = json!({
            "status": "succeeded",
            "subscriptionUrls": subscription_urls,
            "proxyUrls": proxy_urls,
            "refreshedAt": chrono::Utc::now().to_rfc3339()
        });
        self.store
            .put_json("core.forward_proxy_subscription_snapshot.v1", &summary)
            .await?;
        Ok(summary)
    }

    async fn archive_expired_invocations(&self) -> Result<Value> {
        let rows = self
            .store
            .latest_materialized_rows("codex_invocations")
            .await?;
        let now = chrono::Utc::now();
        let retention_days = self
            .store
            .get_json("core.retention_policy.v1")
            .await?
            .and_then(|policy| {
                policy
                    .get("invocationArchiveTtlDays")
                    .or_else(|| policy.get("invocationMaxDays"))
                    .and_then(Value::as_u64)
            })
            .unwrap_or(30);
        let cutoff = now - chrono::Duration::days(retention_days as i64);
        let expired = rows
            .into_iter()
            .filter(|row| {
                row.get("occurredAt")
                    .or_else(|| row.get("occurred_at"))
                    .and_then(Value::as_str)
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .is_some_and(|value| value.with_timezone(&chrono::Utc) < cutoff)
            })
            .collect::<Vec<_>>();
        let archive_root = self.asset_root.join("archives").join("pg-primary");
        tokio::fs::create_dir_all(&archive_root)
            .await
            .context("failed to create PostgreSQL primary archive directory")?;
        let archive_name = format!("codex-invocations-{}.json.gz", now.format("%Y%m%dT%H%M%SZ"));
        let archive_path = archive_root.join(archive_name);
        let payload =
            serde_json::to_vec(&expired).context("failed to encode PG invocation archive")?;
        let compressed = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&payload)?;
            Ok(encoder.finish()?)
        })
        .await
        .context("PG invocation archive task failed")??;
        tokio::fs::write(&archive_path, &compressed)
            .await
            .with_context(|| {
                format!(
                    "failed to write PG invocation archive {}",
                    archive_path.display()
                )
            })?;
        let source_keys = expired
            .iter()
            .filter_map(materialized_source_key)
            .collect::<Vec<_>>();
        let deleted_rows = self
            .store
            .delete_materialized_rows_by_source_keys("codex_invocations", &source_keys)
            .await?;
        let sha256 = format!("{:x}", sha2::Sha256::digest(&compressed));
        let summary = json!({
            "status": "succeeded",
            "sourceObject": "codex_invocations",
            "rowCount": expired.len(),
            "cutoff": cutoff.to_rfc3339(),
            "archivePath": archive_path.strip_prefix(&self.asset_root).unwrap_or(&archive_path).to_string_lossy(),
            "sha256": sha256,
            "sourceRowsRetained": deleted_rows < expired.len() as u64,
            "deletedRows": deleted_rows,
            "completedAt": now.to_rfc3339()
        });
        let mut archives = self
            .store
            .get_json("maintenance.retention_archives")
            .await?
            .filter(|value| value.is_array())
            .unwrap_or_else(|| Value::Array(Vec::new()));
        if let Value::Array(items) = &mut archives {
            items.push(summary.clone());
        }
        self.store
            .put_json("maintenance.retention_archive", &summary)
            .await?;
        self.store
            .put_json("maintenance.retention_archives", &archives)
            .await?;
        Ok(summary)
    }
}

fn camel_case_field(field: &str) -> String {
    let mut output = String::new();
    let mut uppercase = false;
    for character in field.chars() {
        if character == '_' {
            uppercase = true;
        } else if uppercase {
            output.extend(character.to_uppercase());
            uppercase = false;
        } else {
            output.push(character);
        }
    }
    output
}

fn materialized_source_key(row: &Value) -> Option<String> {
    row.get("id")
        .and_then(Value::as_i64)
        .map(|id| json!({"id": id}).to_string())
        .or_else(|| {
            row.get("invoke_id")
                .or_else(|| row.get("invokeId"))
                .and_then(Value::as_str)
                .map(|invoke_id| json!({"invoke_id": invoke_id}).to_string())
        })
}

#[async_trait]
impl PrimaryMaintenanceExecutor for PostgresPrimaryMaintenanceExecutor {
    async fn execute(&self, task_key: &str) -> Result<PrimaryMaintenanceResult> {
        let summary = match task_key {
            "prepare_performance_indexes" => self.prepare_performance_indexes().await?,
            "verify_archive_storage" => self.verify_archive_storage().await?,
            "archive_upstream_activity_manifest" => {
                self.refresh_upstream_activity_manifest().await?
            }
            "materialize_historical_rollups" => self.materialize_historical_rollups().await?,
            "system_status_snapshot" => {
                self.refresh_projection_snapshot(
                    task_key,
                    &[
                        "codex_invocations",
                        "pool_upstream_accounts",
                        "proxy_requests",
                    ],
                )
                .await?
            }
            "summary_snapshot" => {
                self.refresh_projection_snapshot(
                    task_key,
                    &[
                        "codex_invocations",
                        "pool_upstream_request_attempts",
                        "proxy_requests",
                    ],
                )
                .await?
            }
            "dashboard_runtime_projection_reconcile" => {
                self.refresh_projection_snapshot(task_key, &["codex_invocations", "proxy_requests"])
                    .await?
            }
            "timeseries_minute_projection" => {
                self.refresh_projection_snapshot(task_key, &["codex_invocations"])
                    .await?
            }
            "long_term_projection" => {
                self.refresh_projection_snapshot(
                    task_key,
                    &["codex_invocations", "invocation_hourly_rollups"],
                )
                .await?
            }
            "invocation_timeline_snapshot" => {
                self.refresh_projection_snapshot(task_key, &["codex_invocations"])
                    .await?
            }
            "raw_payload_metrics_inventory" => {
                self.refresh_projection_snapshot(
                    task_key,
                    &["codex_invocations", "pool_upstream_request_attempts"],
                )
                .await?
            }
            "upstream_account_maintenance" => {
                self.refresh_projection_snapshot(task_key, &["pool_upstream_accounts"])
                    .await?
            }
            "pool_orphan_recovery" => {
                self.refresh_projection_snapshot(
                    task_key,
                    &["pool_upstream_request_attempts", "codex_invocations"],
                )
                .await?
            }
            "summary_coverage_recovery" => {
                self.refresh_projection_snapshot(
                    task_key,
                    &["invocation_hourly_rollups", "codex_invocations"],
                )
                .await?
            }
            "prompt_cache_materialization" => {
                self.refresh_projection_snapshot(task_key, &["prompt_cache_conversations"])
                    .await?
            }
            "startup_hourly_rollup_bootstrap" => {
                self.refresh_projection_snapshot(task_key, &["invocation_hourly_rollups"])
                    .await?
            }
            "prune_archive_batches" => self.prune_archive_batches(false).await?,
            "prune_legacy_archive_batches" => self.prune_archive_batches(true).await?,
            "raw_compression" => self.compress_raw_payloads().await?,
            "invocation_detail_prune" => {
                self.prune_expired_object(task_key, "codex_invocations")
                    .await?
            }
            "quota_snapshot_compaction" => {
                self.prune_expired_object(task_key, "quota_snapshots")
                    .await?
            }
            "model_route_expiry" => self.prune_expired_object(task_key, "model_routes").await?,
            "task_run_history_cleanup" => {
                self.prune_expired_object(task_key, "managed_task_runs")
                    .await?
            }
            "pool_upstream_attempt_archive" => {
                self.prune_expired_object(task_key, "pool_upstream_request_attempts")
                    .await?
            }
            "forward_proxy_attempt_archive" => {
                self.prune_expired_object(task_key, "proxy_requests")
                    .await?
            }
            "forward_proxy_subscription_refresh" => {
                self.refresh_forward_proxy_subscriptions().await?
            }
            "retention_archive" => self.archive_expired_invocations().await?,
            "invocation_identity_cleanup" => self.cleanup_invocation_identities().await?,
            "raw_orphan_sweep" => self.sweep_raw_orphans().await?,
            "startup_backfill" => {
                self.refresh_projection_snapshot(
                    task_key,
                    &[
                        "codex_invocations",
                        "pool_upstream_request_attempts",
                        "invocation_hourly_rollups",
                    ],
                )
                .await?
            }
            "startup_backfill.pool_attempt_public_id_archives" => {
                self.refresh_projection_snapshot(task_key, &["pool_upstream_request_attempts"])
                    .await?
            }
            "startup_backfill.upstream_activity_archives" => {
                self.refresh_projection_snapshot(task_key, &["archive_batch_upstream_activity"])
                    .await?
            }
            "startup_backfill.pool_upstream_node_health_archives" => {
                self.refresh_projection_snapshot(task_key, &["pool_upstream_node_health"])
                    .await?
            }
            "startup_backfill.legacy_detail_mirrors" => {
                self.refresh_projection_snapshot(task_key, &["invocation_details"])
                    .await?
            }
            "startup_backfill.historical_rollups" => {
                self.refresh_projection_snapshot(task_key, &["invocation_hourly_rollups"])
                    .await?
            }
            "startup_backfill.prompt_cache_conversations_materialization" => {
                self.refresh_projection_snapshot(task_key, &["prompt_cache_conversations"])
                    .await?
            }
            key if key.starts_with("startup_backfill.") => {
                self.refresh_startup_backfill(task_key).await?
            }
            other => bail!("PostgreSQL primary maintenance task is not implemented: {other}"),
        };
        Ok(PrimaryMaintenanceResult {
            task_key: task_key.to_owned(),
            status: "succeeded",
            summary,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primary_store::PrimaryStateStore;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct FakeStore {
        rows: HashMap<String, Vec<Value>>,
        state: Mutex<HashMap<String, Value>>,
    }

    #[async_trait]
    impl PrimaryStateStore for FakeStore {
        async fn get_json(&self, key: &str) -> Result<Option<Value>> {
            Ok(self.state.lock().unwrap().get(key).cloned())
        }

        async fn put_json(&self, key: &str, value: &Value) -> Result<()> {
            self.state
                .lock()
                .unwrap()
                .insert(key.to_owned(), value.clone());
            Ok(())
        }

        async fn latest_materialized_rows(&self, object_name: &str) -> Result<Vec<Value>> {
            Ok(self.rows.get(object_name).cloned().unwrap_or_default())
        }

        async fn delete_materialized_rows_by_source_keys(
            &self,
            _object_name: &str,
            source_keys: &[String],
        ) -> Result<u64> {
            Ok(source_keys.len() as u64)
        }

        async fn update_materialized_text_fields(
            &self,
            _object_name: &str,
            _source_key: &str,
            _fields: &[(&str, &str)],
        ) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn archive_manifest_executor_reads_pg_rows_and_persists_summary() {
        let mut rows = HashMap::new();
        rows.insert(
            "pool_upstream_request_attempts".to_owned(),
            vec![json!({"occurredAt": "2026-01-02T03:04:05Z"})],
        );
        rows.insert("codex_invocations".to_owned(), vec![json!({"id": 1})]);
        let store = Arc::new(FakeStore {
            rows,
            ..FakeStore::default()
        });
        let executor = PostgresPrimaryMaintenanceExecutor {
            store: store.clone(),
            asset_root: PathBuf::from("."),
        };

        let result = executor
            .execute("archive_upstream_activity_manifest")
            .await
            .unwrap();

        assert_eq!(result.status, "succeeded");
        assert_eq!(result.summary["attemptCount"], 1);
        assert_eq!(result.summary["invocationCount"], 1);
        assert_eq!(
            store
                .get_json("maintenance.archive_upstream_activity_manifest")
                .await
                .unwrap()
                .unwrap()["latestAttemptAt"],
            "2026-01-02T03:04:05Z"
        );
    }

    #[tokio::test]
    async fn historical_rollup_executor_persists_pg_materialization_summary() {
        let mut rows = HashMap::new();
        rows.insert("archive_batches".to_owned(), vec![json!({"id": 7})]);
        rows.insert(
            "invocation_hourly_rollups".to_owned(),
            vec![
                json!({"hour": "2026-01-02T03:00:00Z"}),
                json!({"hour": "2026-01-02T04:00:00Z"}),
            ],
        );
        let store = Arc::new(FakeStore {
            rows,
            ..FakeStore::default()
        });
        let executor = PostgresPrimaryMaintenanceExecutor {
            store: store.clone(),
            asset_root: PathBuf::from("."),
        };

        let result = executor
            .execute("materialize_historical_rollups")
            .await
            .unwrap();

        assert_eq!(result.summary["archiveBatchCount"], 1);
        assert_eq!(result.summary["rollupCount"], 2);
        assert!(
            store
                .get_json("maintenance.materialize_historical_rollups")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn projection_executor_counts_pg_materialized_sources() {
        let mut rows = HashMap::new();
        rows.insert(
            "codex_invocations".to_owned(),
            vec![json!({"id": 1}), json!({"id": 2})],
        );
        rows.insert("proxy_requests".to_owned(), vec![json!({"id": 3})]);
        let store = Arc::new(FakeStore {
            rows,
            ..FakeStore::default()
        });
        let executor = PostgresPrimaryMaintenanceExecutor {
            store: store.clone(),
            asset_root: PathBuf::from("."),
        };

        let result = executor.execute("summary_snapshot").await.unwrap();

        assert_eq!(result.summary["sourceRowCount"], 3);
        assert_eq!(result.summary["sourceRowCounts"]["codex_invocations"], 2);
        assert!(
            store
                .get_json("maintenance.summary_snapshot")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn invocation_identity_cleanup_releases_unreferenced_stale_pg_rows() {
        let mut rows = HashMap::new();
        rows.insert(
            "prompt_cache_conversations".to_owned(),
            vec![
                json!({"promptCacheKey": "stale", "updatedAt": "2020-01-01T00:00:00Z"}),
                json!({"promptCacheKey": "active", "updatedAt": "2020-01-01T00:00:00Z"}),
            ],
        );
        rows.insert(
            "codex_invocations".to_owned(),
            vec![json!({"promptCacheKey": "active"})],
        );
        let store = Arc::new(FakeStore {
            rows,
            ..FakeStore::default()
        });
        let executor = PostgresPrimaryMaintenanceExecutor {
            store: store.clone(),
            asset_root: PathBuf::from("."),
        };

        let result = executor
            .execute("invocation_identity_cleanup")
            .await
            .unwrap();

        assert_eq!(result.summary["conversationIdentitiesChecked"], 2);
        assert_eq!(result.summary["conversationIdentitiesReleased"], 1);
        assert_eq!(
            store
                .get_json("maintenance.invocation_identity_cleanup")
                .await
                .unwrap()
                .unwrap()["coverage"],
            "pg_materialized_prompt_cache"
        );
    }

    #[tokio::test]
    async fn raw_orphan_sweep_defers_when_pg_asset_index_is_empty() {
        let store = Arc::new(FakeStore::default());
        let executor = PostgresPrimaryMaintenanceExecutor {
            store: store.clone(),
            asset_root: PathBuf::from("."),
        };

        let result = executor.execute("raw_orphan_sweep").await.unwrap();

        assert_eq!(result.summary["status"], "deferred");
        assert_eq!(result.summary["coverage"], "unknown");
        assert_eq!(
            store
                .get_json("maintenance.raw_orphan_sweep")
                .await
                .unwrap()
                .unwrap()["reason"],
            "proxy_raw_asset_index_empty"
        );
    }

    #[tokio::test]
    async fn startup_backfill_children_use_pg_materialized_sources() {
        let mut rows = HashMap::new();
        rows.insert("proxy_requests".to_owned(), vec![json!({"id": 1})]);
        let store = Arc::new(FakeStore {
            rows,
            ..FakeStore::default()
        });
        let executor = PostgresPrimaryMaintenanceExecutor {
            store: store.clone(),
            asset_root: PathBuf::from("."),
        };

        let result = executor
            .execute("startup_backfill.proxy_usage")
            .await
            .unwrap();

        assert_eq!(result.status, "succeeded");
        assert_eq!(result.summary["sourceRowCounts"]["proxy_requests"], 1);
        assert!(
            store
                .get_json("maintenance.startup_backfill.proxy_usage")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn performance_index_task_records_schema_owned_execution() {
        let store = Arc::new(FakeStore::default());
        let executor = PostgresPrimaryMaintenanceExecutor {
            store: store.clone(),
            asset_root: PathBuf::from("."),
        };

        let result = executor
            .execute("prepare_performance_indexes")
            .await
            .unwrap();

        assert_eq!(result.status, "succeeded");
        assert_eq!(result.summary["ddlApplied"], false);
        assert!(
            store
                .get_json("maintenance.prepare_performance_indexes")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn archive_prune_executor_removes_only_relative_expired_files() {
        let root =
            std::env::temp_dir().join(format!("cvm-primary-maintenance-{}", std::process::id()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let archive = root.join("archive.sqlite.gz");
        tokio::fs::write(&archive, b"archive").await.unwrap();
        let mut rows = HashMap::new();
        rows.insert(
            "archive_batches".to_owned(),
            vec![json!({
                "id": 9,
                "status": "completed",
                "expiresAt": "2020-01-01T00:00:00Z",
                "filePath": "archive.sqlite.gz"
            })],
        );
        let store = Arc::new(FakeStore {
            rows,
            ..FakeStore::default()
        });
        let executor = PostgresPrimaryMaintenanceExecutor {
            store: store.clone(),
            asset_root: root.clone(),
        };

        let result = executor.execute("prune_archive_batches").await.unwrap();

        assert_eq!(result.summary["deletedRows"], 1);
        assert_eq!(result.summary["removedFiles"], 1);
        assert!(!archive.exists());
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn raw_compression_executor_updates_pg_payload_file_safely() {
        let root =
            std::env::temp_dir().join(format!("cvm-primary-compression-{}", std::process::id()));
        tokio::fs::create_dir_all(&root).await.unwrap();
        let source = root.join("payload.json");
        tokio::fs::write(&source, b"payload").await.unwrap();
        let mut rows = HashMap::new();
        rows.insert(
            "codex_invocations".to_owned(),
            vec![json!({
                "id": 11,
                "request_raw_path": "payload.json",
                "request_raw_codec": "identity"
            })],
        );
        let store = Arc::new(FakeStore {
            rows,
            ..FakeStore::default()
        });
        let executor = PostgresPrimaryMaintenanceExecutor {
            store,
            asset_root: root.clone(),
        };

        let result = executor.execute("raw_compression").await.unwrap();

        assert_eq!(result.summary["compressed"], 1);
        assert!(!source.exists());
        assert!(root.join("payload.json.gz").exists());
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn subscription_refresh_executor_persists_empty_pg_snapshot() {
        let store = Arc::new(FakeStore {
            state: Mutex::new(HashMap::from([(
                "core.forward_proxy_settings.v1".to_owned(),
                json!({"subscriptionUrls": []}),
            )])),
            ..FakeStore::default()
        });
        let executor = PostgresPrimaryMaintenanceExecutor {
            store: store.clone(),
            asset_root: PathBuf::from("."),
        };

        let result = executor
            .execute("forward_proxy_subscription_refresh")
            .await
            .unwrap();

        assert_eq!(result.summary["status"], "succeeded");
        assert_eq!(result.summary["proxyUrls"], json!([]));
        assert!(
            store
                .get_json("core.forward_proxy_subscription_snapshot.v1")
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn retention_executor_uses_pg_policy_when_expiry_is_absent() {
        let mut rows = HashMap::new();
        rows.insert(
            "codex_invocations".to_owned(),
            vec![json!({"id": 12, "occurredAt": "2020-01-01T00:00:00Z"})],
        );
        let store = Arc::new(FakeStore {
            rows,
            state: Mutex::new(HashMap::from([(
                "core.retention_policy.v1".to_owned(),
                json!({"invocationMaxDays": 30}),
            )])),
        });
        let executor = PostgresPrimaryMaintenanceExecutor {
            store,
            asset_root: PathBuf::from("."),
        };

        let result = executor.execute("invocation_detail_prune").await.unwrap();

        assert_eq!(result.summary["candidateRows"], 1);
        assert_eq!(result.summary["deletedRows"], 1);
    }

    #[tokio::test]
    async fn retention_archive_executor_writes_gzip_manifest_from_pg_rows() {
        let root =
            std::env::temp_dir().join(format!("cvm-primary-retention-{}", std::process::id()));
        let mut rows = HashMap::new();
        rows.insert(
            "codex_invocations".to_owned(),
            vec![json!({"id": 13, "occurredAt": "2020-01-01T00:00:00Z"})],
        );
        let store = Arc::new(FakeStore {
            rows,
            state: Mutex::new(HashMap::from([(
                "core.retention_policy.v1".to_owned(),
                json!({"invocationArchiveTtlDays": 30}),
            )])),
        });
        let executor = PostgresPrimaryMaintenanceExecutor {
            store: store.clone(),
            asset_root: root.clone(),
        };

        let result = executor.execute("retention_archive").await.unwrap();

        assert_eq!(result.summary["rowCount"], 1);
        assert_eq!(result.summary["sourceRowsRetained"], false);
        assert_eq!(result.summary["deletedRows"], 1);
        assert!(root.join("archives/pg-primary").exists());
        assert!(
            store
                .get_json("maintenance.retention_archive")
                .await
                .unwrap()
                .is_some()
        );
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
