use super::*;

#[cfg(test)]
pub(crate) struct ActiveAccountActivityV2ProgressHandlerTestPause {
    pub(crate) installed: tokio::sync::oneshot::Sender<()>,
    pub(crate) resume: tokio::sync::oneshot::Receiver<()>,
}

pub(crate) struct ActiveAccountActivityV2ProgressHandlerOptions {
    pub(super) progress_probe: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    pub(super) progress_handler_ops: i32,
    pub(super) progress_abort_on_probe: bool,
    #[cfg(test)]
    pub(super) test_pause_after_handler_install:
        Option<ActiveAccountActivityV2ProgressHandlerTestPause>,
}

impl ActiveAccountActivityV2ProgressHandlerOptions {
    pub(crate) fn new(
        progress_probe: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
        progress_handler_ops: i32,
        progress_abort_on_probe: bool,
    ) -> Self {
        Self {
            progress_probe,
            progress_handler_ops,
            progress_abort_on_probe,
            #[cfg(test)]
            test_pause_after_handler_install: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn pause_after_handler_install(
        mut self,
        pause: ActiveAccountActivityV2ProgressHandlerTestPause,
    ) -> Self {
        self.test_pause_after_handler_install = Some(pause);
        self
    }
}

pub(super) struct ActiveAccountActivityV2ProgressConnection {
    pub(super) connection: sqlx::pool::PoolConnection<Sqlite>,
    pub(super) progress_handler_installed: bool,
}

impl ActiveAccountActivityV2ProgressConnection {
    pub(super) fn close_on_drop(&mut self) {
        self.connection.close_on_drop();
    }
}

impl Drop for ActiveAccountActivityV2ProgressConnection {
    fn drop(&mut self) {
        if self.progress_handler_installed {
            self.connection.close_on_drop();
        }
    }
}

pub(crate) fn build_active_account_activity_v2_archive_epoch_coverage_query(
    prefix: &'static str,
    oldest_bucket: i64,
    current_bucket: i64,
) -> QueryBuilder<'static, Sqlite> {
    let mut query = QueryBuilder::<Sqlite>::new(prefix);
    query.push(
        "SELECT coverage_start_epoch, coverage_end_epoch \
         FROM archive_batches INDEXED BY idx_archive_batches_invocation_coverage_epoch \
         WHERE dataset = 'codex_invocations' \
           AND status = 'completed' \
           AND coverage_start_epoch IS NOT NULL \
           AND coverage_end_epoch IS NOT NULL \
           AND coverage_start_epoch < ",
    );
    query.push_bind(current_bucket);
    query.push(" AND coverage_end_epoch >= ");
    query.push_bind(oldest_bucket);
    query
}

pub(crate) fn build_active_account_activity_v2_legacy_coverage_query(
    prefix: &'static str,
    active_month_keys: &[String],
) -> QueryBuilder<'static, Sqlite> {
    let mut query = QueryBuilder::<Sqlite>::new(prefix);
    query.push(
        "SELECT month_key \
         FROM archive_batches INDEXED BY idx_archive_batches_invocation_legacy_coverage_month \
         WHERE dataset = 'codex_invocations' \
           AND status = 'completed' \
           AND (coverage_start_at IS NULL OR coverage_end_at IS NULL)",
    );
    if active_month_keys.is_empty() {
        query.push(" AND 0");
        return query;
    }
    query.push(" AND month_key IN (");
    {
        let mut separated = query.separated(", ");
        for month_key in active_month_keys {
            separated.push_bind(month_key.clone());
        }
    }
    query.push(")");
    query
}

pub(super) fn active_account_activity_v2_month_key(bucket_start_epoch: i64) -> Result<String> {
    Utc.timestamp_opt(bucket_start_epoch, 0)
        .single()
        .map(|bucket_start| {
            bucket_start
                .with_timezone(&Shanghai)
                .format("%Y-%m")
                .to_string()
        })
        .ok_or_else(|| anyhow!("invalid account activity v2 priority bucket start"))
}
