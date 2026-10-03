use super::*;

pub(super) async fn rebuild_prompt_cache_working_set_live_triggers(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    let refresh_started = Instant::now();
    // Retention deletes rows well outside the short live working-set window. Recomputing a
    // whole prompt key for each such row is redundant and quadratic for a large key. Keep the
    // projection synchronous for recent or in-flight source rows only.
    let live_window_condition = |subject: &str| {
        format!(
            "(LOWER(TRIM(COALESCE({subject}.status, ''))) IN ('running', 'pending') OR {subject}.occurred_at >= datetime('now', '+8 hours', '-{PROMPT_CACHE_WORKING_SET_WINDOW_SECONDS} seconds'))"
        )
    };
    let old_live_window_condition = live_window_condition("OLD");
    let new_live_window_condition = live_window_condition("NEW");
    let prompt_cache_insert_trigger_sql = format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS trg_codex_invocations_prompt_cache_working_set_insert
        AFTER INSERT ON codex_invocations
        WHEN {new_live_window_condition}
        BEGIN
            {refresh_sql};
        END
        "#,
        new_live_window_condition = new_live_window_condition,
        refresh_sql = prompt_cache_working_set_live_refresh_sql_for_key(
            &invocation_in_progress_live_prompt_cache_key_expr("NEW"),
        ),
    );
    let prompt_cache_update_trigger_sql = format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS trg_codex_invocations_prompt_cache_working_set_update
        AFTER UPDATE OF id, invoke_id, payload, source, status, error_message,
            failure_kind, failure_class, occurred_at, total_tokens, cost
        ON codex_invocations
        WHEN {old_live_window_condition} OR {new_live_window_condition}
        BEGIN
            {refresh_old_sql};
            {refresh_new_sql};
        END
        "#,
        old_live_window_condition = old_live_window_condition,
        new_live_window_condition = new_live_window_condition,
        refresh_old_sql = prompt_cache_working_set_live_refresh_sql_for_key(
            &invocation_in_progress_live_prompt_cache_key_expr("OLD"),
        ),
        refresh_new_sql = prompt_cache_working_set_live_refresh_sql_for_key(
            &invocation_in_progress_live_prompt_cache_key_expr("NEW"),
        ),
    );
    let prompt_cache_delete_trigger_sql = format!(
        r#"
        CREATE TRIGGER IF NOT EXISTS trg_codex_invocations_prompt_cache_working_set_delete
        AFTER DELETE ON codex_invocations
        WHEN {old_live_window_condition}
        BEGIN
            {refresh_sql};
        END
        "#,
        old_live_window_condition = old_live_window_condition,
        refresh_sql = prompt_cache_working_set_live_refresh_sql_for_key(
            &invocation_in_progress_live_prompt_cache_key_expr("OLD"),
        ),
    );
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin prompt cache working set trigger refresh")?;
    for (trigger_name, trigger_sql) in [
        (
            "trg_codex_invocations_prompt_cache_working_set_insert",
            prompt_cache_insert_trigger_sql,
        ),
        (
            "trg_codex_invocations_prompt_cache_working_set_update",
            prompt_cache_update_trigger_sql,
        ),
        (
            "trg_codex_invocations_prompt_cache_working_set_delete",
            prompt_cache_delete_trigger_sql,
        ),
    ] {
        sqlx::query(&format!("DROP TRIGGER IF EXISTS {trigger_name}"))
            .execute(tx.as_mut())
            .await
            .with_context(|| format!("failed to drop stale trigger {trigger_name}"))?;
        sqlx::query(&trigger_sql)
            .execute(tx.as_mut())
            .await
            .with_context(|| format!("failed to ensure trigger {trigger_name}"))?;
    }
    // Timing-only terminal follow-ups must not scan the same live key twice. Record the
    // narrower trigger installation with its DDL so interrupted upgrades retry atomically.
    record_schema_refresh_completion_in_transaction(
        &mut tx,
        PROMPT_CACHE_WORKING_SET_TRIGGER_REFRESH_MIGRATION_NAME,
    )
    .await?;
    tx.commit()
        .await
        .context("failed to commit prompt cache working set trigger refresh")?;
    info!(
        migration_name = PROMPT_CACHE_WORKING_SET_TRIGGER_REFRESH_MIGRATION_NAME,
        elapsed_ms = refresh_started.elapsed().as_millis() as u64,
        "refreshed prompt cache working set triggers"
    );

    Ok(())
}
