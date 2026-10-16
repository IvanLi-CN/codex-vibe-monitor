use super::*;
use crate::proxy_sqlite_write_coordinator::ProxySqliteWriteCoordinator;
use serde::Deserialize;

pub(crate) const PERFORMANCE_INDEX_TASK: &str = "prepare_performance_indexes";

#[cfg(test)]
tokio::task_local! {
    pub(crate) static PERFORMANCE_INDEX_TEST_PROBE: Arc<PerformanceIndexNativeProbe>;
}

#[cfg(test)]
#[derive(Default)]
pub(crate) struct PerformanceIndexNativeProbe {
    pub(crate) entered: tokio::sync::Notify,
    pub(crate) closed: tokio::sync::Notify,
    released: std::sync::Mutex<bool>,
    release: std::sync::Condvar,
}

#[cfg(test)]
impl PerformanceIndexNativeProbe {
    pub(crate) fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.release.notify_all();
    }

    fn observe_native_ddl(&self) {
        let released = self.released.lock().unwrap();
        if !*released {
            self.entered.notify_one();
            drop(
                self.release
                    .wait_while(released, |released| !*released)
                    .unwrap(),
            );
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PerformanceIndexPreparationRequest {
    pub(crate) dry_run: bool,
    pub(crate) budget_ms: u64,
}

impl Default for PerformanceIndexPreparationRequest {
    fn default() -> Self {
        Self {
            dry_run: false,
            budget_ms: 2_000,
        }
    }
}

impl PerformanceIndexPreparationRequest {
    fn deadline(self) -> Result<Instant> {
        anyhow::ensure!(
            self.budget_ms != 0,
            "index preparation budget must be positive"
        );
        Instant::now()
            .checked_add(Duration::from_millis(self.budget_ms))
            .context("index preparation budget is too large")
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerformanceIndexPreparationReport {
    pub(crate) dry_run: bool,
    pub(crate) budget_ms: u64,
    pub(crate) elapsed_ms: u64,
    pub(crate) outcome: &'static str,
    pub(crate) indexes: Vec<PerformanceIndexResult>,
}

impl PerformanceIndexPreparationReport {
    pub(crate) fn is_complete(&self) -> bool {
        matches!(self.outcome, "complete" | "dry_run")
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerformanceIndexResult {
    pub(crate) name: &'static str,
    pub(crate) outcome: &'static str,
    pub(crate) elapsed_ms: u64,
}

pub(crate) fn performance_index_definitions() -> [(&'static str, String); 2] {
    [
        (
            "idx_proxy_raw_payload_blob_links_owner",
            "CREATE INDEX idx_proxy_raw_payload_blob_links_owner ON proxy_raw_payload_blob_links(owner_kind, owner_id)".to_string(),
        ),
        (
            "idx_codex_invocations_prompt_cache_key_id",
            format!("CREATE INDEX idx_codex_invocations_prompt_cache_key_id ON codex_invocations({INVOCATION_PROMPT_CACHE_KEY_EXPR_SQL}, id DESC)"),
        ),
    ]
}

// Normalize only SQL outside quoted tokens. Case/space inside the JSON path must remain
// significant; accepting a differently cased path would silently accept the wrong index.
fn normalize_index_definition(sql: &str) -> String {
    let mut normalized = String::new();
    let mut quote = None;
    let mut chars = sql.trim_end_matches(';').chars().peekable();
    while let Some(ch) = chars.next() {
        if let Some(delimiter) = quote {
            normalized.push(ch);
            if ch == delimiter {
                if chars.peek() == Some(&delimiter) {
                    normalized.push(chars.next().expect("peeked quote"));
                } else {
                    quote = None;
                }
            }
        } else if matches!(ch, '\'' | '"' | '`' | '[') {
            quote = Some(if ch == '[' { ']' } else { ch });
            normalized.push(ch);
        } else if !ch.is_whitespace() {
            normalized.push(ch.to_ascii_lowercase());
        }
    }
    normalized
}

fn preparation_stop(deadline: Instant, shutdown: &CancellationToken) -> Option<&'static str> {
    if shutdown.is_cancelled() {
        Some("cancelled")
    } else if Instant::now() >= deadline {
        Some("budget_exhausted")
    } else {
        None
    }
}

fn preparation_error_stop(
    error: &anyhow::Error,
    deadline: Instant,
    shutdown: &CancellationToken,
) -> Option<&'static str> {
    preparation_stop(deadline, shutdown).or_else(|| {
        error
            .downcast_ref::<sqlx::Error>()
            .and_then(sqlx::Error::as_database_error)
            .and_then(|error| error.code())
            .and_then(|code| code.parse::<i32>().ok())
            .filter(|code| matches!(code & 0xff, 5 | 6))
            .map(|_| "sqlite_busy")
    })
}

/// The owned worker keeps its connection and write permit even if a caller drops its wait.
/// Cancellation requests interrupt native work; they never substitute dropping a SQL future
/// for rollback/close. Service dispatch also drains this owner before releasing runtime locks.
pub(crate) async fn prepare_performance_indexes(
    options: SqliteConnectOptions,
    request: PerformanceIndexPreparationRequest,
    coordinator: Arc<ProxySqliteWriteCoordinator>,
    shutdown: CancellationToken,
) -> Result<PerformanceIndexPreparationReport> {
    let deadline = request.deadline()?;
    tokio::spawn(prepare_performance_indexes_owned(
        options,
        request,
        deadline,
        coordinator,
        shutdown,
        #[cfg(test)]
        PERFORMANCE_INDEX_TEST_PROBE.try_with(Arc::clone).ok(),
    ))
    .await
    .context("index preparation owner failed")?
}

async fn prepare_performance_indexes_owned(
    options: SqliteConnectOptions,
    request: PerformanceIndexPreparationRequest,
    deadline: Instant,
    coordinator: Arc<ProxySqliteWriteCoordinator>,
    shutdown: CancellationToken,
    #[cfg(test)] probe: Option<Arc<PerformanceIndexNativeProbe>>,
) -> Result<PerformanceIndexPreparationReport> {
    let started = Instant::now();
    let mut report = PerformanceIndexPreparationReport {
        dry_run: request.dry_run,
        budget_ms: request.budget_ms,
        elapsed_ms: 0,
        outcome: if request.dry_run {
            "dry_run"
        } else {
            "complete"
        },
        indexes: Vec::with_capacity(2),
    };
    for (name, definition) in performance_index_definitions() {
        if let Some(stop) = preparation_stop(deadline, &shutdown) {
            report.outcome = stop;
            break;
        }
        let operation_started = Instant::now();
        // Waiting for admission owns neither a connection nor a SQLite transaction.
        let permit = if request.dry_run {
            None
        } else {
            tokio::select! {
                biased;
                _ = shutdown.cancelled() => { report.outcome = "cancelled"; break; }
                _ = tokio::time::sleep_until(deadline.into()) => { report.outcome = "budget_exhausted"; break; }
                permit = coordinator.acquire_maintenance(Duration::from_secs(1)) => Some(permit),
            }
        };
        let result = prepare_one_performance_index(
            &options,
            request.dry_run,
            name,
            &definition,
            deadline,
            &shutdown,
            #[cfg(test)]
            probe.clone(),
        )
        .await;
        // prepare_one has awaited native completion and closed the dedicated connection.
        drop(permit);
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                // IO/schema errors are surfaced without changing the application's old path.
                // They cannot be converted to a successful preparation result.
                warn!(index = name, error = %error, "performance index preparation failed; existing queries remain available");
                return Err(error);
            }
        };
        report.indexes.push(PerformanceIndexResult {
            name,
            outcome,
            elapsed_ms: operation_started
                .elapsed()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
        });
        if !matches!(outcome, "present" | "created" | "would_create") {
            report.outcome = outcome;
            break;
        }
    }
    report.elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
    if !report.is_complete() {
        warn!(
            outcome = report.outcome,
            elapsed_ms = report.elapsed_ms,
            "performance index preparation stopped; re-entry is safe and existing queries remain available"
        );
    }
    Ok(report)
}

async fn prepare_one_performance_index(
    options: &SqliteConnectOptions,
    dry_run: bool,
    name: &'static str,
    definition: &str,
    deadline: Instant,
    shutdown: &CancellationToken,
    #[cfg(test)] probe: Option<Arc<PerformanceIndexNativeProbe>>,
) -> Result<&'static str> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if let Some(stop) = preparation_stop(deadline, shutdown) {
        return Ok(stop);
    }
    let options = options
        .clone()
        .create_if_missing(false)
        .busy_timeout(remaining.min(Duration::from_millis(i32::MAX as u64)));
    let options = if dry_run {
        options.read_only(true)
    } else {
        options
    };
    // Do not timeout/drop connect: its native setup must drain under the same ownership.
    let mut connection = match SqliteConnection::connect_with(&options).await {
        Ok(connection) => connection,
        Err(error) => {
            let error = anyhow::Error::from(error);
            return preparation_error_stop(&error, deadline, shutdown).map_or(Err(error), Ok);
        }
    };
    let mut transaction_open = false;
    let outcome: Result<&'static str> = async {
        if let Some(stop) = preparation_stop(deadline, shutdown) {
            return Ok(stop);
        }
        {
            let shutdown = shutdown.clone();
            connection
                .lock_handle()
                .await?
                .set_progress_handler(1_000, move || {
                    preparation_stop(deadline, &shutdown).is_none()
                });
        }
        if !dry_run {
            // Connection setup consumed part of the total budget. Refresh native lock wait
            // before BEGIN so it cannot start another full-budget wait after setup.
            let remaining_ms = deadline
                .saturating_duration_since(Instant::now())
                .as_millis();
            sqlx::query(&format!(
                "PRAGMA busy_timeout = {}",
                remaining_ms.min(i32::MAX as u128)
            ))
            .execute(&mut connection)
            .await?;
            sqlx::query("BEGIN IMMEDIATE")
                .execute(&mut connection)
                .await?;
            transaction_open = true;
        }
        if let Some(stop) = preparation_stop(deadline, shutdown) {
            return Ok(stop);
        }
        let existing: Option<String> =
            sqlx::query_scalar("SELECT sql FROM sqlite_schema WHERE type='index' AND name=?")
                .bind(name)
                .fetch_optional(&mut connection)
                .await?;
        if let Some(existing) = existing {
            if normalize_index_definition(&existing) != normalize_index_definition(definition) {
                return Ok("definition_conflict");
            }
            return Ok("present");
        }
        if dry_run {
            return Ok("would_create");
        }
        #[cfg(test)]
        if let Some(probe) = &probe {
            let probe = probe.clone();
            let shutdown = shutdown.clone();
            connection
                .lock_handle()
                .await?
                .set_progress_handler(1, move || {
                    probe.observe_native_ddl();
                    preparation_stop(deadline, &shutdown).is_none()
                });
        }
        sqlx::query(definition).execute(&mut connection).await?;
        if let Some(stop) = preparation_stop(deadline, shutdown) {
            return Ok(stop);
        }
        sqlx::query("COMMIT").execute(&mut connection).await?;
        transaction_open = false;
        Ok("created")
    }
    .await;
    // Remove the deadline handler before rollback. An expired handler must not interrupt
    // the cleanup that proves SQLite has stopped using the write lock.
    let cleanup = connection.lock_handle().await.map(|mut handle| {
        handle.remove_progress_handler();
        // SQLITE_INTERRUPT may already have rolled the explicit transaction back.
        // Inspect the native handle rather than misreporting that successful cleanup.
        unsafe { libsqlite3_sys::sqlite3_get_autocommit(handle.as_raw_handle().as_ptr()) == 0 }
    });
    let rollback = if transaction_open && cleanup.as_ref().is_ok_and(|active| *active) {
        sqlx::query("ROLLBACK")
            .execute(&mut connection)
            .await
            .map(|_| ())
    } else {
        Ok(())
    };
    let closed = connection.close().await;
    #[cfg(test)]
    if let Some(probe) = probe {
        probe.closed.notify_one();
    }
    cleanup?;
    rollback?;
    closed?;
    match outcome {
        Ok(outcome) => Ok(outcome),
        Err(error) => preparation_error_stop(&error, deadline, shutdown).map_or(Err(error), Ok),
    }
}

impl crate::maintenance_store::MaintenanceStore {
    pub(crate) async fn request_performance_indexes(
        &self,
        request: PerformanceIndexPreparationRequest,
    ) -> Result<i64> {
        request.deadline()?;
        crate::maintenance_store::observe_store_write(
            crate::observability::contention::StoreOperation::Run,
            async {
                sqlx::query_scalar(
                    "INSERT INTO managed_task_runs(task_key, trigger_kind, started_at, status, summary, details)
                     SELECT task_key, 'manual', ?, 'requested', '手动索引准备请求', ?
                     FROM managed_tasks WHERE task_key=? AND is_manual!=0 RETURNING id",
                )
                .bind(format_utc_iso_millis(Utc::now()))
                .bind(serde_json::to_string(&request)?)
                .bind(PERFORMANCE_INDEX_TASK)
                .fetch_optional(&self.pool)
                .await?
                .context("performance index task is unavailable or already active")
            },
        )
        .await
    }
}

pub(crate) async fn run_performance_index_preparation_cli(
    config: &AppConfig,
    options: &PerformanceIndexPreparationArgs,
) -> Result<()> {
    let request = PerformanceIndexPreparationRequest {
        dry_run: options.dry_run,
        budget_ms: options.budget_ms,
    };
    request.deadline()?;
    match MaintenanceRuntimeLock::route(config, true)? {
        MaintenanceRuntimeRoute::Online(mut runtime) => {
            let store = crate::maintenance_store::connect_existing(config, &mut runtime).await?;
            let result = async {
                runtime.validate()?;
                let id = store.request_performance_indexes(request).await?;
                let deadline = Instant::now() + Duration::from_secs(15 * 60);
                loop {
                    runtime.validate()?;
                    let (status, details, error): (String, Option<String>, Option<String>) = sqlx::query_as(
                        "SELECT status, details, error_detail FROM managed_task_runs WHERE id=?",
                    ).bind(id).fetch_one(&store.pool).await?;
                    if !matches!(status.as_str(), "requested" | "running") {
                        println!("{}", details.unwrap_or_else(|| "{}".to_string()));
                        anyhow::ensure!(status == "success", "index preparation {status}: {}", error.as_deref().unwrap_or("see task history"));
                        return Ok(());
                    }
                    anyhow::ensure!(Instant::now() < deadline, "index preparation request remains owned by the service; inspect task history");
                    sleep(Duration::from_millis(100)).await;
                }
            }.await;
            store.pool.close().await;
            result
        }
        MaintenanceRuntimeRoute::Offline(mut runtime) => {
            runtime.publish_role("cli:ownership-v1")?;
            let options = runtime.sqlite_connect_options(
                &config.database_path,
                build_sqlite_connect_options(
                    &config.database_url(),
                    Duration::from_millis(request.budget_ms),
                )?
                .create_if_missing(false),
            )?;
            // The spawned owner retains the exclusive runtime proof through worker drain,
            // even when a client cancels its join wait.
            tokio::spawn(async move {
                let result = prepare_performance_indexes(
                    options,
                    request,
                    crate::proxy_sqlite_write_coordinator::proxy_sqlite_write_coordinator(),
                    CancellationToken::new(),
                )
                .await;
                runtime.refresh_inode_pair_lock()?;
                let report = result?;
                println!("{}", serde_json::to_string(&report)?);
                anyhow::ensure!(
                    report.is_complete(),
                    "index preparation stopped: {}",
                    report.outcome
                );
                Ok(())
            })
            .await
            .context("offline index preparation owner failed")?
        }
    }
}
