use super::*;

// The only buffered copy is one client batch. PostgreSQL stores native business
// columns; checkpoint rows contain counts and hashes, never source payloads.
pub(super) fn append_typed_csv(
    payload: &serde_json::Value,
    table: &SourceTable,
    output: &mut String,
) -> Result<()> {
    for (index, column) in table.columns.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        let value = &payload[&column.name];
        if value.is_null() {
            continue;
        }
        let text = match value {
            serde_json::Value::String(value)
                if postgres_type_for_source_column(&column.declared_type) == "BYTEA" =>
            {
                let bytes = base64::engine::general_purpose::STANDARD.decode(value)?;
                let mut hex = String::from("\\x");
                use std::fmt::Write;
                for byte in bytes {
                    write!(&mut hex, "{byte:02x}")?;
                }
                hex
            }
            serde_json::Value::String(value) => value.clone(),
            value => value.to_string(),
        };
        copy_csv_field(&text, output);
    }
    output.push('\n');
    Ok(())
}

// A stopped source starts a new cursor pass. Reset only metadata and defer foreign
// keys; reconcile keyed business rows in bounded ranges without a full data copy.
// Checkpoint reset is atomic, so a failed pass resumes committed batches.
pub(super) async fn reset_direct_import(pool: &sqlx::PgPool, run_id: &str) -> Result<()> {
    let mut tx = pool.begin().await?;
    let reset: bool =
        sqlx::query_scalar("SELECT needs_reset FROM cvm_migration.runs WHERE run_id=$1 FOR UPDATE")
            .bind(run_id)
            .fetch_one(&mut *tx)
            .await?;
    if !reset {
        tx.commit().await?;
        return Ok(());
    }
    let targets: Vec<(String, String)> = sqlx::query_as(
        "SELECT target_schema,target_table FROM cvm_migration.object_targets WHERE run_id=$1",
    )
    .bind(run_id)
    .fetch_all(&mut *tx)
    .await?;
    for (schema, table) in targets {
        let qualified = format!("{}.{}", quote_identifier(&schema), quote_identifier(&table));
        let constraints: Vec<String> = sqlx::query_scalar("SELECT c.conname FROM pg_constraint c WHERE c.conrelid=$1::regclass AND c.contype='f' AND c.conname LIKE 'cvm_fk_%'")
            .bind(&qualified).fetch_all(&mut *tx).await?;
        for constraint in constraints {
            sqlx::query(&format!(
                "ALTER TABLE {qualified} DROP CONSTRAINT {}",
                quote_identifier(&constraint)
            ))
            .execute(&mut *tx)
            .await?;
        }
    }
    sqlx::query("DELETE FROM cvm_migration.batches WHERE run_id=$1")
        .bind(run_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE cvm_migration.objects SET imported_rows=0,checkpoint=NULL,checksum=NULL WHERE run_id=$1").bind(run_id).execute(&mut *tx).await?;
    sqlx::query(
        "UPDATE cvm_migration.runs SET needs_reset=FALSE,snapshot_sha256=NULL WHERE run_id=$1",
    )
    .bind(run_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn verify_direct_payloads(
    pool: &sqlx::PgPool,
    run_id: &str,
    source_root: &Path,
    existing_snapshot: Option<&Path>,
) -> Result<()> {
    let (snapshot, cleanup) = if let Some(snapshot) = existing_snapshot {
        (snapshot.to_path_buf(), false)
    } else {
        migration_snapshot(source_root).await?
    };
    let _guard = SnapshotGuard::new(snapshot.clone(), cleanup);
    bind_snapshot(pool, run_id, &snapshot).await?;
    let source = source_pool(&snapshot).await?;
    let manifest = inspect_source_database(&source, source_root).await?;
    for table in &manifest.tables {
        let qualified = format!(
            "{}.{}",
            quote_identifier(target_schema_for_object(&table.name)),
            quote_identifier(&target_table_for_object(&table.name))
        );
        let columns = table
            .columns
            .iter()
            .map(|c| quote_identifier(&c.name))
            .collect::<Vec<_>>()
            .join(", ");
        let target_columns = table
            .columns
            .iter()
            .map(|c| format!("actual.{}", quote_identifier(&c.name)))
            .collect::<Vec<_>>()
            .join(", ");
        let mut tx = pool.begin().await?;
        sqlx::query(&format!(
            "CREATE TEMP TABLE cvm_verify_batch (LIKE {qualified}) ON COMMIT DROP"
        ))
        .execute(&mut *tx)
        .await?;
        let mut offset = 0_i64;
        let mut cursor = None;
        let keyed = if table.primary_key.is_empty() {
            false
        } else {
            let nulls = table
                .primary_key
                .iter()
                .map(|c| format!("{} IS NULL", quote_identifier(c)))
                .collect::<Vec<_>>()
                .join(" OR ");
            !sqlx::query_scalar::<_, bool>(&format!(
                "SELECT EXISTS(SELECT 1 FROM {} WHERE {nulls})",
                quote_identifier(&table.name)
            ))
            .fetch_one(&source)
            .await?
        };
        let mut rowid = keyed;
        loop {
            let query = if rowid {
                let predicate = cursor
                    .map(|c: i64| format!("WHERE rowid>{c}"))
                    .unwrap_or_default();
                format!(
                    "SELECT rowid AS _cvm_rowid, * FROM {} {predicate} ORDER BY rowid LIMIT 20000",
                    quote_identifier(&table.name)
                )
            } else {
                format!(
                    "SELECT * FROM {} ORDER BY {} LIMIT 20000 OFFSET {offset}",
                    quote_identifier(&table.name),
                    if !keyed {
                        table
                            .columns
                            .iter()
                            .map(|c| quote_identifier(&c.name))
                            .collect::<Vec<_>>()
                            .join(", ")
                    } else {
                        source_order_by(table)
                    }
                )
            };
            let rows = match sqlx::query(&query).fetch_all(&source).await {
                Ok(rows) => rows,
                Err(error) if rowid && error.to_string().contains("no such column: rowid") => {
                    rowid = false;
                    continue;
                }
                Err(error) => return Err(error.into()),
            };
            if rows.is_empty() {
                break;
            }
            let mut data = String::new();
            let mut null_key = false;
            for row in &rows {
                let payload = encode_row(row, table).await?;
                null_key |= table.primary_key.iter().any(|key| payload[key].is_null());
                append_typed_csv(&payload, table, &mut data)?;
            }
            if rowid {
                cursor = rows
                    .last()
                    .and_then(|r| r.try_get::<i64, _>("_cvm_rowid").ok());
            }
            sqlx::query("TRUNCATE cvm_verify_batch")
                .execute(&mut *tx)
                .await?;
            let mut copy = tx
                .as_mut()
                .copy_in_raw(&format!(
                    "COPY cvm_verify_batch ({columns}) FROM STDIN WITH (FORMAT csv)"
                ))
                .await?;
            copy.send(data.as_bytes()).await?;
            copy.finish().await?;
            sqlx::query("ANALYZE cvm_verify_batch")
                .execute(&mut *tx)
                .await?;
            let selection = if !keyed {
                // No source key exists. Compare a deterministic, bounded multiset slice.
                let order = table
                    .columns
                    .iter()
                    .map(|c| {
                        let column = quote_identifier(&c.name);
                        if postgres_type_for_source_column(&c.declared_type) == "TEXT" {
                            format!("{column} COLLATE \"C\" NULLS FIRST")
                        } else {
                            format!("{column} NULLS FIRST")
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "SELECT {columns} FROM {qualified} ORDER BY {order} LIMIT 20000 OFFSET {offset}"
                )
            } else {
                let predicate = table
                    .primary_key
                    .iter()
                    .map(|c| {
                        format!(
                            "actual.{0} {1} expected.{0}",
                            quote_identifier(c),
                            if null_key {
                                "IS NOT DISTINCT FROM"
                            } else {
                                "="
                            }
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" AND ");
                format!(
                    "SELECT {target_columns} FROM {qualified} actual WHERE EXISTS (SELECT 1 FROM cvm_verify_batch expected WHERE {predicate})"
                )
            };
            let mismatch: bool = sqlx::query_scalar(&format!(
                "WITH actual AS ({selection}), expected AS (SELECT {columns} FROM cvm_verify_batch) SELECT EXISTS((TABLE expected EXCEPT ALL TABLE actual) UNION ALL (TABLE actual EXCEPT ALL TABLE expected))"
            )).fetch_one(&mut *tx).await?;
            if mismatch {
                bail!(
                    "direct payload verification failed for {} at row {offset}",
                    table.name
                );
            }
            offset += rows.len() as i64;
        }
        tx.commit().await?;
    }
    source.close().await;
    Ok(())
}

// A batch cursor belongs to one immutable snapshot. A new online snapshot cannot
// silently reuse committed batches from a previous snapshot, even at equal counts.
pub(super) async fn bind_snapshot(
    pool: &sqlx::PgPool,
    run_id: &str,
    snapshot: &Path,
) -> Result<()> {
    let snapshot = snapshot.to_path_buf();
    let digest = tokio::task::spawn_blocking(move || -> Result<String> {
        use std::io::Read;
        let mut file = fs::File::open(snapshot)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 1024 * 1024];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        Ok(format!("{:x}", hasher.finalize()))
    })
    .await??;
    let changed = sqlx::query("UPDATE cvm_migration.runs SET snapshot_sha256=$2 WHERE run_id=$1 AND (snapshot_sha256 IS NULL OR snapshot_sha256=$2)").bind(run_id).bind(digest).execute(pool).await?;
    if changed.rows_affected() != 1 {
        bail!("source snapshot changed; resume requires the original immutable snapshot");
    }
    Ok(())
}
pub(super) async fn import_snapshot(
    pool: &sqlx::PgPool,
    run_id: &str,
    tables: &[SourceTable],
    batch_rows: u64,
    snapshot: &Path,
) -> Result<()> {
    bind_snapshot(pool, run_id, snapshot).await?;
    ensure_domain_tables(pool, run_id).await?;
    let source = source_pool(snapshot).await?;
    let catch_up: bool =
        sqlx::query_scalar("SELECT state='catching_up' FROM cvm_migration.runs WHERE run_id=$1")
            .bind(run_id)
            .fetch_one(pool)
            .await?;
    for table in tables {
        let started = std::time::Instant::now();
        let (mut offset, mut batch_no, mut cursor): (i64,i64,Option<String>) = sqlx::query_as("SELECT COALESCE(SUM(row_count),0)::bigint,COALESCE(MAX(batch_no)+1,0)::bigint,(SELECT source_cursor FROM cvm_migration.batches WHERE run_id=$1 AND object_name=$2 ORDER BY batch_no DESC LIMIT 1) FROM cvm_migration.batches WHERE run_id=$1 AND object_name=$2")
            .bind(run_id).bind(&table.name).fetch_one(pool).await?;
        let qualified = format!(
            "{}.{}",
            quote_identifier(target_schema_for_object(&table.name)),
            quote_identifier(&target_table_for_object(&table.name))
        );
        let columns = table
            .columns
            .iter()
            .map(|c| quote_identifier(&c.name))
            .collect::<Vec<_>>()
            .join(", ");
        // SQLite permits NULLs in some non-integer primary keys. Such tables need
        // a replay rather than a key cursor, as do tables without a declared key.
        let keyed = if table.primary_key.is_empty() {
            false
        } else {
            let nulls = table
                .primary_key
                .iter()
                .map(|c| format!("{} IS NULL", quote_identifier(c)))
                .collect::<Vec<_>>()
                .join(" OR ");
            !sqlx::query_scalar::<_, bool>(&format!(
                "SELECT EXISTS(SELECT 1 FROM {} WHERE {nulls})",
                quote_identifier(&table.name)
            ))
            .fetch_one(&source)
            .await?
        };
        if catch_up && !keyed && offset == 0 {
            sqlx::query(&format!("TRUNCATE TABLE {qualified}"))
                .execute(pool)
                .await?;
        }
        loop {
            let order = if keyed {
                source_order_by(table)
            } else {
                table
                    .columns
                    .iter()
                    .map(|c| quote_identifier(&c.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            let predicate = if keyed {
                cursor
                    .as_deref()
                    .map(|cursor| sqlite_key_after(table, cursor))
                    .transpose()?
                    .map(|p| format!("WHERE {p}"))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let pagination = if keyed {
                String::new()
            } else {
                format!("OFFSET {offset}")
            };
            let rows = sqlx::query(&format!(
                "SELECT * FROM {} {predicate} ORDER BY {order} LIMIT {batch_rows} {pagination}",
                quote_identifier(&table.name)
            ))
            .fetch_all(&source)
            .await?;
            if rows.is_empty() {
                break;
            }
            let mut data = String::new();
            let mut hasher = Sha256::new();
            let mut first_key = None;
            let mut last_key = String::new();
            for (index, row) in rows.iter().enumerate() {
                let payload = encode_row(row, table).await?;
                hasher.update(serde_json::to_string(&payload)?.as_bytes());
                let key = source_key(&payload, table, offset + index as i64)?;
                first_key.get_or_insert_with(|| key.clone());
                last_key = key;
                append_typed_csv(&payload, table, &mut data)?;
            }
            let mut tx = pool.begin().await?;
            if catch_up && keyed {
                sqlx::query(&format!(
                    "CREATE TEMP TABLE cvm_import_batch (LIKE {qualified}) ON COMMIT DROP"
                ))
                .execute(&mut *tx)
                .await?;
                let mut copy = tx
                    .as_mut()
                    .copy_in_raw(&format!(
                        "COPY cvm_import_batch ({columns}) FROM STDIN WITH (FORMAT csv)"
                    ))
                    .await?;
                copy.send(data.as_bytes()).await?;
                copy.finish().await?;
                sqlx::query("ANALYZE cvm_import_batch")
                    .execute(&mut *tx)
                    .await?;
                let lower = cursor
                    .as_deref()
                    .map(|c| pg_key_bound(table, c, "actual", ">"))
                    .transpose()?
                    .unwrap_or_else(|| "TRUE".to_owned());
                let upper = pg_key_bound(table, &last_key, "actual", "<=")?;
                let matches = table
                    .primary_key
                    .iter()
                    .map(|c| format!("actual.{0}=expected.{0}", quote_identifier(c)))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                // This cursor range is complete in the final snapshot. Remove only
                // missing source keys, never an unscanned range of the target.
                sqlx::query(&format!("DELETE FROM {qualified} actual WHERE {lower} AND {upper} AND NOT EXISTS(SELECT 1 FROM cvm_import_batch expected WHERE {matches})")).execute(&mut *tx).await?;
                let keys = table
                    .primary_key
                    .iter()
                    .map(|c| quote_identifier(c))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mutable = table
                    .columns
                    .iter()
                    .filter(|c| !table.primary_key.contains(&c.name))
                    .map(|c| quote_identifier(&c.name))
                    .collect::<Vec<_>>();
                let conflict = if mutable.is_empty() {
                    "DO NOTHING".to_owned()
                } else {
                    let assignments = mutable
                        .iter()
                        .map(|c| format!("{c}=EXCLUDED.{c}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let existing = mutable
                        .iter()
                        .map(|c| format!("actual.{c}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let excluded = mutable
                        .iter()
                        .map(|c| format!("EXCLUDED.{c}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!(
                        "DO UPDATE SET {assignments} WHERE ROW({existing}) IS DISTINCT FROM ROW({excluded})"
                    )
                };
                sqlx::query(&format!("INSERT INTO {qualified} AS actual ({columns}) SELECT {columns} FROM cvm_import_batch WHERE TRUE ON CONFLICT ({keys}) {conflict}")).execute(&mut *tx).await?;
            } else {
                let mut copy = tx
                    .as_mut()
                    .copy_in_raw(&format!(
                        "COPY {qualified} ({columns}) FROM STDIN WITH (FORMAT csv)"
                    ))
                    .await?;
                copy.send(data.as_bytes()).await?;
                copy.finish().await?;
            }
            sqlx::query("INSERT INTO cvm_migration.batches(run_id,object_name,batch_no,first_key,last_key,row_count,checksum,source_cursor) VALUES($1,$2,$3,$4,$5,$6,$7,$5)")
                .bind(run_id).bind(&table.name).bind(batch_no).bind(first_key).bind(&last_key).bind(rows.len() as i64).bind(format!("{:x}",hasher.finalize())).execute(&mut *tx).await?;
            tx.commit().await?;
            enforce_capacity(pool).await?;
            offset += rows.len() as i64;
            batch_no += 1;
            cursor = Some(last_key);
        }
        // Catch deletions beyond the last source key, including a now-empty table.
        if catch_up && keyed {
            let predicate = cursor
                .as_deref()
                .map(|c| pg_key_bound(table, c, "actual", ">"))
                .transpose()?
                .unwrap_or_else(|| "TRUE".to_owned());
            sqlx::query(&format!("DELETE FROM {qualified} actual WHERE {predicate}"))
                .execute(pool)
                .await?;
        }
        sqlx::query("UPDATE cvm_migration.objects SET source_rows=$3,imported_rows=$3,checkpoint=$4,updated_at=now() WHERE run_id=$1 AND object_name=$2").bind(run_id).bind(&table.name).bind(offset).bind(cursor).execute(pool).await?;
        info!(object=%table.name,rows=offset,seconds=started.elapsed().as_secs_f64(),"direct table import complete");
    }
    source.close().await;
    Ok(())
}

fn sqlite_key_after(table: &SourceTable, key: &str) -> Result<String> {
    let key: serde_json::Value = serde_json::from_str(key)?;
    let columns = table
        .primary_key
        .iter()
        .map(|c| quote_identifier(c))
        .collect::<Vec<_>>()
        .join(", ");
    let values = table
        .primary_key
        .iter()
        .map(|c| -> Result<String> {
            let value = &key[c];
            if value.is_null() {
                bail!("NULL source key cannot be used as a cursor");
            }
            if let Some(value) = value.as_str() {
                if table.columns.iter().any(|col| {
                    col.name == *c && postgres_type_for_source_column(&col.declared_type) == "BYTEA"
                }) {
                    let bytes = base64::engine::general_purpose::STANDARD.decode(value)?;
                    return Ok(format!(
                        "X'{}'",
                        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
                    ));
                }
                Ok(quote_literal(value))
            } else {
                Ok(value.to_string())
            }
        })
        .collect::<Result<Vec<_>>>()?
        .join(", ");
    if table.primary_key.len() == 1 {
        Ok(format!("{columns} > {values}"))
    } else {
        Ok(format!("({columns}) > ({values})"))
    }
}

fn pg_key_bound(table: &SourceTable, key: &str, alias: &str, operator: &str) -> Result<String> {
    let key: serde_json::Value = serde_json::from_str(key)?;
    let columns = table
        .primary_key
        .iter()
        .map(|c| format!("{alias}.{}", quote_identifier(c)))
        .collect::<Vec<_>>()
        .join(", ");
    let values = table
        .primary_key
        .iter()
        .map(|c| -> Result<String> {
            let column = table
                .columns
                .iter()
                .find(|col| col.name == *c)
                .context("missing key column")?;
            let pg_type = postgres_type_for_source_column(&column.declared_type);
            let value = &key[c];
            if value.is_null() {
                bail!("NULL key cannot be used as a cursor");
            }
            let text = value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| value.to_string());
            if pg_type == "BYTEA" {
                Ok(format!("decode({},'base64')", quote_literal(&text)))
            } else {
                Ok(format!("{}::{pg_type}", quote_literal(&text)))
            }
        })
        .collect::<Result<Vec<_>>>()?
        .join(", ");
    if table.primary_key.len() == 1 {
        Ok(format!("{columns} {operator} {values}"))
    } else {
        Ok(format!("({columns}) {operator} ({values})"))
    }
}

pub(super) async fn enforce_capacity(pool: &sqlx::PgPool) -> Result<()> {
    // Decimal GB: the deployment's hard ceiling is 30 GB, not 30 GiB and not
    // the compressed dump size. A lower test/deployment budget is allowed.
    let ceiling = env::var("MIGRATION_MAX_TARGET_BYTES")
        .ok()
        .map(|v| v.parse::<i64>())
        .transpose()?
        .unwrap_or(30_000_000_000);
    if !(1..=30_000_000_000).contains(&ceiling) {
        bail!("migration target budget must be between 1 and 30000000000 bytes");
    }
    let size: i64 = sqlx::query_scalar("SELECT pg_database_size(current_database())")
        .fetch_one(pool)
        .await?;
    if size > ceiling {
        bail!(
            "migration target exceeds capacity: {size} bytes > {ceiling} bytes; import is not accepted"
        );
    }
    Ok(())
}
