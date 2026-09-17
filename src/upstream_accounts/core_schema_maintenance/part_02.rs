pub(crate) const API_KEY_TRANSIT_PROXY_MIGRATION_AUDIT_ACTION: &str =
    "api_key_transit_proxy_binding_migrated";
const API_KEY_TRANSIT_PROXY_MIGRATION_AUDIT_SOURCE: &str = "account_migration";

pub(crate) async fn ensure_api_key_transit_proxy_bindings_migrated(
    pool: &Pool<Sqlite>,
) -> Result<usize> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .context("failed to begin API Key transit proxy binding migration")?;
    let accounts = sqlx::query_as::<_, (i64, String, Option<String>, i64, Option<String>)>(
        r#"
        SELECT id, display_name, group_name, is_mother, bound_proxy_keys_json
        FROM pool_upstream_accounts
        WHERE kind = ?1 AND COALESCE(deleted_at, '') = ''
        ORDER BY id ASC
        "#,
    )
    .bind(UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX)
    .fetch_all(tx.as_mut())
    .await
    .context("failed to load API Key accounts for transit proxy binding migration")?;
    let now = format_utc_iso(Utc::now());
    let migrated_count = migrate_api_key_transit_proxy_accounts(&mut tx, accounts, &now).await?;
    tx.commit()
        .await
        .context("failed to commit API Key transit proxy binding migration")?;
    Ok(migrated_count)
}

async fn migrate_api_key_transit_proxy_accounts(
    tx: &mut sqlx::SqliteConnection,
    accounts: Vec<(i64, String, Option<String>, i64, Option<String>)>,
    now: &str,
) -> Result<usize> {
    let mut migrated_count = 0;
    for (id, display_name, group_name, is_mother, bound_proxy_keys_json) in accounts {
        let account_proxy_keys = bound_proxy_keys_json
            .as_deref()
            .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
            .map(normalize_bound_proxy_keys)
            .unwrap_or_default();
        let normalized_group_name = group_name
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let group_proxy_keys = if account_proxy_keys.is_empty() {
            match normalized_group_name {
                Some(group_name) => sqlx::query_scalar::<_, Option<String>>(
                    "SELECT bound_proxy_keys_json FROM pool_upstream_account_group_notes WHERE group_name = ?1",
                )
                .bind(group_name)
                .fetch_optional(&mut *tx)
                .await
                .context("failed to load legacy API Key group proxy bindings")?
                .flatten()
                .and_then(|value| serde_json::from_str::<Vec<String>>(&value).ok())
                .map(normalize_bound_proxy_keys)
                .unwrap_or_default(),
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };
        let next_proxy_keys = if !account_proxy_keys.is_empty() {
            account_proxy_keys
        } else if !group_proxy_keys.is_empty() {
            group_proxy_keys
        } else {
            vec![FORWARD_PROXY_DIRECT_KEY.to_string()]
        };
        let needs_migration = normalized_group_name.is_some()
            || is_mother != 0
            || bound_proxy_keys_json
                .as_deref()
                .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
                .map(normalize_bound_proxy_keys)
                .is_none_or(|keys| keys.is_empty());
        if !needs_migration {
            continue;
        }
        let next_proxy_keys_json = encode_group_bound_proxy_keys_json(&next_proxy_keys)
            .context("failed to encode API Key transit proxy bindings")?;
        sqlx::query(
            r#"
            UPDATE pool_upstream_accounts
            SET bound_proxy_keys_json = ?2,
                group_name = NULL,
                is_mother = 0,
                policy_concurrency_limit = NULL,
                policy_upstream_429_retry_enabled = NULL,
                policy_upstream_429_max_retries = NULL,
                updated_at = ?3
            WHERE id = ?1
            "#,
        )
        .bind(id)
        .bind(next_proxy_keys_json)
        .bind(now)
        .execute(&mut *tx)
        .await
        .context("failed to migrate API Key transit proxy bindings")?;
        sqlx::query(
            r#"
            INSERT INTO pool_upstream_account_events (
                account_id, occurred_at, action, source, account_display_name, account_group_name,
                result, result_description, reason_code, reason_message, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'success',
                      'API Key account migrated to an explicit transit proxy binding',
                      'upstream_domain_migration',
                      'legacy group strategies detached; account uses an explicit proxy binding', ?2)
            "#,
        )
        .bind(id)
        .bind(now)
        .bind(API_KEY_TRANSIT_PROXY_MIGRATION_AUDIT_ACTION)
        .bind(API_KEY_TRANSIT_PROXY_MIGRATION_AUDIT_SOURCE)
        .bind(display_name)
        .bind(normalized_group_name)
        .execute(&mut *tx)
        .await
        .context("failed to audit API Key transit proxy binding migration")?;
        migrated_count += 1;
    }
    Ok(migrated_count)
}

async fn repair_responses_lite_image_tool_capability_observations(
    pool: &Pool<Sqlite>,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET response_image_tool_capability = 'unknown',
            response_image_tool_capability_observed_at = NULL,
            response_image_tool_capability_reason = NULL,
            updated_at = datetime('now')
        WHERE response_image_tool_capability = 'unsupported'
          AND lower(COALESCE(response_image_tool_capability_reason, '')) LIKE '%responses lite%'
          AND lower(COALESCE(response_image_tool_capability_reason, '')) LIKE '%top-level tool type%'
          AND lower(COALESCE(response_image_tool_capability_reason, '')) LIKE '%image_generation%'
        "#,
    )
    .execute(pool)
    .await
    .context("failed to repair Responses Lite image-tool capability observations")?;

    Ok(())
}

async fn ensure_upstream_account_capability_axis_split_migrated(pool: &Pool<Sqlite>) -> Result<()> {
    let migrated = sqlx::query_scalar::<_, i64>(
        r#"
        SELECT capability_axis_split_migrated
        FROM pool_routing_settings
        WHERE id = ?1
        LIMIT 1
        "#,
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .fetch_optional(pool)
    .await
    .context("failed to check capability axis split migration flag")?
    .unwrap_or(0);
    if migrated != 0 {
        return Ok(());
    }

    sqlx::query(
        r#"
        UPDATE pool_upstream_accounts
        SET response_endpoint_capability = 'unknown',
            response_endpoint_capability_observed_at = NULL,
            response_endpoint_capability_reason = NULL,
            policy_response_endpoint_capability_override = NULL,
            chat_completions_capability = 'unknown',
            chat_completions_capability_observed_at = NULL,
            chat_completions_capability_reason = NULL,
            policy_chat_completions_capability_override = NULL,
            updated_at = datetime('now')
        "#,
    )
    .execute(pool)
    .await
    .context("failed to reset legacy mixed response/chat capability state")?;

    sqlx::query(
        r#"
        UPDATE pool_routing_settings
        SET capability_axis_split_migrated = 1,
            updated_at = datetime('now')
        WHERE id = ?1
        "#,
    )
    .bind(POOL_SETTINGS_SINGLETON_ID)
    .execute(pool)
    .await
    .context("failed to mark capability axis split migration complete")?;

    Ok(())
}

pub(crate) fn spawn_upstream_account_maintenance(
    state: Arc<AppState>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(UPSTREAM_ACCOUNT_MAINTENANCE_TICK_SECS));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("upstream account maintenance stopped");
                    break;
                }
                _ = ticker.tick() => {
                    if let Err(err) = run_upstream_account_maintenance_once(state.clone()).await {
                        warn!(error = %err, "failed to run upstream account maintenance");
                    }
                }
            }
        }
    })
}

pub(crate) async fn ensure_nullable_text_column(
    pool: &Pool<Sqlite>,
    table_name: &str,
    column_name: &str,
) -> Result<()> {
    let pragma = format!("PRAGMA table_info('{table_name}')");
    let columns = sqlx::query(&pragma)
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();

    if columns.contains(column_name) {
        return Ok(());
    }

    let statement = format!("ALTER TABLE {table_name} ADD COLUMN {column_name} TEXT");
    sqlx::query(&statement).execute(pool).await?;
    Ok(())
}

pub(crate) async fn ensure_nullable_integer_column(
    pool: &Pool<Sqlite>,
    table_name: &str,
    column_name: &str,
) -> Result<()> {
    let pragma = format!("PRAGMA table_info('{table_name}')");
    let columns = sqlx::query(&pragma)
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();

    if columns.contains(column_name) {
        return Ok(());
    }

    let statement = format!("ALTER TABLE {table_name} ADD COLUMN {column_name} INTEGER");
    sqlx::query(&statement).execute(pool).await?;
    Ok(())
}

pub(crate) async fn migrate_legacy_no_new_policy(
    pool: &Pool<Sqlite>,
    table_name: &str,
    priority_column_name: &str,
    legacy_block_column_name: &str,
    legacy_allow_column_name: &str,
) -> Result<()> {
    let pragma = format!("PRAGMA table_info('{table_name}')");
    let columns = sqlx::query(&pragma)
        .fetch_all(pool)
        .await?
        .into_iter()
        .filter_map(|row| row.try_get::<String, _>("name").ok())
        .collect::<HashSet<_>>();

    if columns.contains(legacy_allow_column_name) {
        let statement = format!(
            r#"
            UPDATE {table_name}
            SET {priority_column_name} = 'no_new'
            WHERE {legacy_allow_column_name} = 0
            "#
        );
        sqlx::query(&statement).execute(pool).await?;
    }

    if columns.contains(legacy_block_column_name) {
        let statement = format!(
            r#"
            UPDATE {table_name}
            SET {priority_column_name} = 'no_new'
            WHERE {legacy_block_column_name} = 1
            "#
        );
        sqlx::query(&statement).execute(pool).await?;
    }

    Ok(())
}

pub(crate) async fn ensure_text_column_with_default(
    pool: &Pool<Sqlite>,
    table_name: &str,
    column_name: &str,
    default_value: &str,
) -> Result<()> {
    let pragma_statement = format!("PRAGMA table_info({table_name})");
    let columns: Vec<(i64, String, String, i64, Option<String>, i64)> =
        sqlx::query_as(&pragma_statement).fetch_all(pool).await?;
    if columns
        .iter()
        .any(|(_, name, _, _, _, _)| name == column_name)
    {
        return Ok(());
    }

    let statement = format!(
        "ALTER TABLE {table_name} ADD COLUMN {column_name} TEXT NOT NULL DEFAULT {default_value}"
    );
    sqlx::query(&statement).execute(pool).await?;
    Ok(())
}

pub(crate) async fn sqlite_table_exists(pool: &Pool<Sqlite>, table_name: &str) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
    )
    .bind(table_name)
    .fetch_one(pool)
    .await?
        > 0)
}
