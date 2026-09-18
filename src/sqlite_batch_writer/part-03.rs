#[cfg(test)]
macro_rules! flush_pending_batch_accounted {
    ($accounting:expr, $pool:expr, $pricing_catalog:expr, $batch:expr, $reason:expr,
     $prompt_cache_conversation_cache:expr, $terminal_runtime_store:expr,
     $dashboard_activity_snapshot_cache:expr, $summary_delta_hub:expr,
     $terminal_projection_hub:expr, $dashboard_reconcile_gate:expr, $terminal_journal:expr $(,)?) => {
        flush_pending_batch_accounted(
            $accounting,
            $pool,
            $pricing_catalog,
            $batch,
            $reason,
            flush_dependencies!(
                $prompt_cache_conversation_cache,
                $terminal_runtime_store,
                $dashboard_activity_snapshot_cache,
                $summary_delta_hub,
                $terminal_projection_hub,
                $dashboard_reconcile_gate,
                $terminal_journal,
            ),
        )
    };
}

struct SqliteBatchWriterConfig {
    pool: Pool<Sqlite>,
    database_path: std::path::PathBuf,
    write_receiver: mpsc::Receiver<SqliteBatchWrite>,
    control_receiver: mpsc::Receiver<SqliteBatchWriterControl>,
    accounting: Arc<PendingQueueAccounting>,
    prompt_cache_conversation_cache: Option<Arc<Mutex<PromptCacheConversationsCacheState>>>,
    pricing_catalog: Option<Arc<RwLock<PricingCatalog>>>,
    terminal_runtime_store: Arc<std::sync::Mutex<Option<Arc<ProxyRuntimeInvocationStore>>>>,
    dashboard_activity_snapshot_cache:
        Arc<std::sync::Mutex<Option<Arc<Mutex<DashboardActivitySnapshotCacheState>>>>>,
    summary_delta_hub: Arc<std::sync::Mutex<Option<Arc<SubscriptionHub>>>>,
    terminal_projection_hub: Arc<std::sync::Mutex<Option<Arc<TerminalProjectionHub>>>>,
    dashboard_reconcile_gate: Arc<Mutex<()>>,
    terminal_journal: Arc<std::sync::Mutex<Option<TerminalJournal>>>,
    queued_p1_count: Arc<AtomicUsize>,
    p1_priority_gate: Arc<std::sync::Mutex<()>>,
}

async fn run_sqlite_batch_writer(config: SqliteBatchWriterConfig) {
    SqliteBatchWriterLoop::from_config(config).run().await;
}
