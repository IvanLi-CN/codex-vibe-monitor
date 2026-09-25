use anyhow::{Context, Result, bail};
use futures_util::TryStreamExt;
use sha2::{Digest, Sha256};
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection};

use super::CODEX_INVOCATIONS_ARCHIVE_COLUMNS;

#[derive(Debug, Clone, Copy)]
pub(crate) enum InvocationArchiveIdentityDatabase {
    Main,
    Archive,
}

#[derive(Clone, Copy)]
enum InvocationArchiveIdentityVersion {
    Current,
    CandidateV2,
    LegacyV2WithoutReportedCacheWrite,
}

impl InvocationArchiveIdentityDatabase {
    fn table_name(self) -> &'static str {
        match self {
            Self::Main => "main.codex_invocations",
            Self::Archive => "archive_db.codex_invocations",
        }
    }
}

fn hash_identity_component(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

pub(crate) async fn invocation_archive_source_identity_sha256(
    connection: &mut SqliteConnection,
    database: InvocationArchiveIdentityDatabase,
    ids: &[i64],
) -> Result<String> {
    invocation_archive_source_identity_sha256_for_version(
        connection,
        database,
        ids,
        InvocationArchiveIdentityVersion::Current,
    )
    .await
}

async fn invocation_archive_source_identity_sha256_for_version(
    connection: &mut SqliteConnection,
    database: InvocationArchiveIdentityDatabase,
    ids: &[i64],
    version: InvocationArchiveIdentityVersion,
) -> Result<String> {
    if ids.is_empty() {
        bail!("retention archive source identity requires at least one row");
    }

    let mut columns = CODEX_INVOCATIONS_ARCHIVE_COLUMNS
        .split(", ")
        .collect::<Vec<_>>();
    if matches!(
        version,
        InvocationArchiveIdentityVersion::LegacyV2WithoutReportedCacheWrite
    ) {
        columns.retain(|column| *column != "reported_cache_write_tokens");
    }
    let table_name = database.table_name();
    let mut query = QueryBuilder::<Sqlite>::new("SELECT ");
    for (index, column) in columns.iter().enumerate() {
        if index > 0 {
            query.push(", ");
        }
        query
            .push("typeof(")
            .push(table_name)
            .push(".")
            .push(*column)
            .push("), CAST(")
            .push(table_name)
            .push(".")
            .push(*column)
            .push(" AS BLOB)");
    }
    query
        .push(" FROM ")
        .push(table_name)
        .push(" WHERE id IN (SELECT value FROM json_each(")
        .push_bind(serde_json::to_string(ids).context("encode retention archive identity ids")?)
        .push(")) ORDER BY id ASC");

    // Length framing and SQLite storage classes keep NULL, text, numeric, and blob values distinct.
    let mut hasher = Sha256::new();
    match version {
        InvocationArchiveIdentityVersion::Current => {
            hasher.update(b"codex-vibe-monitor/retention-source-identity/v3\0");
        }
        InvocationArchiveIdentityVersion::CandidateV2
        | InvocationArchiveIdentityVersion::LegacyV2WithoutReportedCacheWrite => {
            hasher.update(b"codex-vibe-monitor/retention-source-identity/v2\0");
        }
    }
    hasher.update((ids.len() as u64).to_be_bytes());
    let mut row_count = 0usize;
    let mut rows = query.build().fetch(&mut *connection);
    while let Some(row) = rows.try_next().await? {
        row_count += 1;
        for (index, column) in columns.iter().enumerate() {
            let sqlite_type = row.try_get::<String, _>(index * 2)?;
            let value = row.try_get::<Option<Vec<u8>>, _>(index * 2 + 1)?;
            hash_identity_component(&mut hasher, column.as_bytes());
            hash_identity_component(&mut hasher, sqlite_type.as_bytes());
            match value {
                Some(value) => {
                    hasher.update([1]);
                    hash_identity_component(&mut hasher, &value);
                }
                None => hasher.update([0]),
            }
        }
        hasher.update([0xff]);
    }

    if row_count != ids.len() {
        bail!("retention archive source identity verification failed: source row count changed");
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub(super) async fn invocation_archive_source_identity_matches(
    connection: &mut SqliteConnection,
    database: InvocationArchiveIdentityDatabase,
    ids: &[i64],
    expected_sha256: &str,
) -> Result<bool> {
    for version in [
        InvocationArchiveIdentityVersion::Current,
        InvocationArchiveIdentityVersion::CandidateV2,
    ] {
        if invocation_archive_source_identity_sha256_for_version(connection, database, ids, version)
            .await?
            == expected_sha256
        {
            return Ok(true);
        }
    }

    if invocation_archive_source_identity_sha256_for_version(
        connection,
        database,
        ids,
        InvocationArchiveIdentityVersion::LegacyV2WithoutReportedCacheWrite,
    )
    .await?
        != expected_sha256
    {
        return Ok(false);
    }

    let table_name = database.table_name();
    let has_reported_cache_write = sqlx::query_scalar::<_, i64>(&format!(
        "SELECT EXISTS(SELECT 1 FROM {table_name} WHERE id IN (SELECT value FROM json_each(?1)) AND reported_cache_write_tokens IS NOT NULL)"
    ))
    .bind(serde_json::to_string(ids).context("encode retention identity ids")?)
    .fetch_one(&mut *connection)
    .await?;
    Ok(has_reported_cache_write == 0)
}

#[cfg(test)]
pub(crate) async fn invocation_archive_source_identity_sha256_legacy_for_test(
    connection: &mut SqliteConnection,
    database: InvocationArchiveIdentityDatabase,
    ids: &[i64],
) -> Result<String> {
    invocation_archive_source_identity_sha256_for_version(
        connection,
        database,
        ids,
        InvocationArchiveIdentityVersion::LegacyV2WithoutReportedCacheWrite,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn invocation_archive_source_identity_sha256_candidate_v2_for_test(
    connection: &mut SqliteConnection,
    database: InvocationArchiveIdentityDatabase,
    ids: &[i64],
) -> Result<String> {
    invocation_archive_source_identity_sha256_for_version(
        connection,
        database,
        ids,
        InvocationArchiveIdentityVersion::CandidateV2,
    )
    .await
}

#[cfg(test)]
pub(crate) async fn invocation_archive_source_identity_matches_for_test(
    connection: &mut SqliteConnection,
    database: InvocationArchiveIdentityDatabase,
    ids: &[i64],
    expected_sha256: &str,
) -> Result<bool> {
    invocation_archive_source_identity_matches(connection, database, ids, expected_sha256).await
}
