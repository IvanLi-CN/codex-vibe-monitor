use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Duration as ChronoDuration, NaiveDateTime, TimeZone, Utc};
use chrono_tz::Asia::Shanghai;
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Sqlite};
use std::{
    collections::HashSet,
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
};
use tracing::warn;

use crate::{
    ARCHIVE_LAYOUT_SEGMENT_V1, AppConfig, ArchiveBatchLayout, ArchiveFileCodec,
    ArchiveSegmentGranularity, format_naive, start_of_local_day,
};

pub(crate) fn resolved_raw_path_candidates(path: &str, fallback_root: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let primary = PathBuf::from(path);
    candidates.push(normalize_path_for_compare(&primary));
    if !primary.is_absolute()
        && let Some(root) = fallback_root
    {
        let fallback = root.join(&primary);
        let normalized = normalize_path_for_compare(&fallback);
        if !candidates.contains(&normalized) {
            candidates.push(normalized);
        }
    }
    candidates
}

pub(crate) fn resolved_raw_path_read_candidates(path: &str, fallback_root: Option<&Path>) -> Vec<PathBuf> {
    let mut candidates = resolved_raw_path_candidates(path, fallback_root);
    if let Some(alternate_path) = raw_payload_alternate_db_path(path) {
        for candidate in resolved_raw_path_candidates(&alternate_path, fallback_root) {
            if !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
        }
    }
    candidates
}

pub(crate) fn raw_payload_alternate_db_path(path: &str) -> Option<String> {
    if path.ends_with(".bin.gz") {
        Some(path.trim_end_matches(".gz").to_string())
    } else if path.ends_with(".bin") {
        Some(format!("{path}.gz"))
    } else {
        None
    }
}

pub(crate) fn normalize_path_for_compare(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    }
}

pub(crate) fn count_existing_proxy_raw_paths(
    raw_paths: &[Option<String>],
    raw_path_fallback_root: Option<&Path>,
) -> usize {
    let mut seen = HashSet::new();
    for raw_path in raw_paths.iter().flatten() {
        for candidate in resolved_raw_path_read_candidates(raw_path, raw_path_fallback_root) {
            if candidate.exists() {
                seen.insert(candidate);
            }
        }
    }
    seen.len()
}

pub(crate) fn delete_proxy_raw_paths(
    raw_paths: &[Option<String>],
    raw_path_fallback_root: Option<&Path>,
) -> Result<usize> {
    let mut removed = 0usize;
    let mut seen = HashSet::new();
    for raw_path in raw_paths.iter().flatten() {
        for candidate in resolved_raw_path_read_candidates(raw_path, raw_path_fallback_root) {
            if !seen.insert(candidate.clone()) {
                continue;
            }
            match fs::remove_file(&candidate) {
                Ok(_) => {
                    removed += 1;
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
                Err(err) => {
                    warn!(path = %candidate.display(), error = %err, "failed to remove raw payload file");
                }
            }
        }
    }
    Ok(removed)
}

pub(crate) fn shanghai_retention_cutoff(days: u64) -> DateTime<Utc> {
    start_of_local_day(Utc::now(), Shanghai) - ChronoDuration::days(days as i64)
}

pub(crate) fn shanghai_local_cutoff_string(days: u64) -> String {
    format_naive(
        shanghai_retention_cutoff(days)
            .with_timezone(&Shanghai)
            .naive_local(),
    )
}

pub(crate) fn shanghai_local_cutoff_for_age_secs_string(age_secs: u64) -> String {
    format_naive(
        Utc::now().with_timezone(&Shanghai).naive_local()
            - ChronoDuration::seconds(age_secs as i64),
    )
}

pub(crate) fn shanghai_utc_cutoff_string(days: u64) -> String {
    format_naive(shanghai_retention_cutoff(days).naive_utc())
}

pub(crate) fn invocation_status_is_success_like(status: Option<&str>, error_message: Option<&str>) -> bool {
    let normalized_status = status.map(str::trim).unwrap_or_default();
    let error_message_empty = error_message.map(str::trim).is_none_or(str::is_empty);

    normalized_status.eq_ignore_ascii_case("success")
        || normalized_status.eq_ignore_ascii_case("completed")
        || (normalized_status.eq_ignore_ascii_case("http_200") && error_message_empty)
}

pub(crate) fn invocation_status_is_success_like_sql(
    status_column: &str,
    error_message_column: &str,
) -> String {
    format!(
        "(LOWER(TRIM(COALESCE({status_column}, ''))) IN ('success', 'completed') OR (LOWER(TRIM(COALESCE({status_column}, ''))) = 'http_200' AND TRIM(COALESCE({error_message_column}, '')) = ''))"
    )
}

pub(crate) fn parse_shanghai_local_naive(value: &str) -> Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .with_context(|| format!("invalid shanghai-local timestamp: {value}"))
}

pub(crate) fn parse_utc_naive(value: &str) -> Result<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .with_context(|| format!("invalid utc timestamp: {value}"))
}

pub(crate) fn shanghai_day_key_from_local_naive(value: &str) -> Result<String> {
    Ok(parse_shanghai_local_naive(value)?
        .format("%Y-%m-%d")
        .to_string())
}

pub(crate) fn shanghai_month_key_from_local_naive(value: &str) -> Result<String> {
    Ok(parse_shanghai_local_naive(value)?
        .format("%Y-%m")
        .to_string())
}

pub(crate) fn shanghai_month_key_from_utc_naive(value: &str) -> Result<String> {
    let utc = Utc.from_utc_datetime(&parse_utc_naive(value)?);
    Ok(utc.with_timezone(&Shanghai).format("%Y-%m").to_string())
}

pub(crate) fn resolved_archive_dir(config: &AppConfig) -> PathBuf {
    resolve_path_from_database_parent(&config.database_path, &config.archive_dir)
}

pub(crate) fn resolve_path_from_database_parent(database_path: &Path, configured_path: &Path) -> PathBuf {
    if configured_path.is_absolute() {
        return configured_path.to_path_buf();
    }

    match database_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(configured_path),
        _ => configured_path.to_path_buf(),
    }
}

pub(crate) fn archive_batch_file_path(config: &AppConfig, dataset: &str, month_key: &str) -> Result<PathBuf> {
    let year = month_key
        .split('-')
        .next()
        .filter(|segment| segment.len() == 4)
        .ok_or_else(|| anyhow!("invalid month key: {month_key}"))?;
    Ok(resolved_archive_dir(config)
        .join(dataset)
        .join(year)
        .join(format!("{dataset}-{month_key}.sqlite.gz")))
}

pub(crate) fn archive_segment_file_path(
    config: &AppConfig,
    dataset: &str,
    day_key: &str,
    part_key: &str,
    codec: ArchiveFileCodec,
) -> Result<PathBuf> {
    let mut segments = day_key.split('-');
    let year = segments
        .next()
        .filter(|segment| segment.len() == 4)
        .ok_or_else(|| anyhow!("invalid day key: {day_key}"))?;
    let month = segments
        .next()
        .filter(|segment| segment.len() == 2)
        .ok_or_else(|| anyhow!("invalid day key: {day_key}"))?;
    let day = segments
        .next()
        .filter(|segment| segment.len() == 2)
        .ok_or_else(|| anyhow!("invalid day key: {day_key}"))?;
    Ok(resolved_archive_dir(config)
        .join(dataset)
        .join(year)
        .join(month)
        .join(day)
        .join(format!("{part_key}.sqlite.{}", codec.file_extension())))
}

pub(crate) fn archive_month_key_from_day_key(day_key: &str) -> Result<String> {
    Ok(day_key
        .get(..7)
        .ok_or_else(|| anyhow!("invalid day key: {day_key}"))?
        .to_string())
}

pub(crate) fn retention_temp_suffix() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    )
}

pub(crate) fn archive_layout_for_dataset(config: &AppConfig, dataset: &str) -> ArchiveBatchLayout {
    if dataset == "codex_invocations" {
        config.codex_invocation_archive_layout
    } else {
        ArchiveBatchLayout::LegacyMonth
    }
}

pub(crate) fn invocation_archive_group_key(config: &AppConfig, occurred_at: &str) -> Result<String> {
    match config.codex_invocation_archive_layout {
        ArchiveBatchLayout::LegacyMonth => shanghai_month_key_from_local_naive(occurred_at),
        ArchiveBatchLayout::SegmentV1 => {
            match config.codex_invocation_archive_segment_granularity {
                ArchiveSegmentGranularity::Day => shanghai_day_key_from_local_naive(occurred_at),
            }
        }
    }
}

pub(crate) async fn next_archive_segment_part_key(
    pool: &Pool<Sqlite>,
    dataset: &str,
    day_key: &str,
) -> Result<String> {
    let latest_part_key = sqlx::query_scalar::<_, String>(
        r#"
        SELECT part_key
        FROM archive_batches
        WHERE dataset = ?1
          AND layout = ?2
          AND day_key = ?3
          AND part_key IS NOT NULL
        ORDER BY part_key DESC, id DESC
        LIMIT 1
        "#,
    )
    .bind(dataset)
    .bind(ARCHIVE_LAYOUT_SEGMENT_V1)
    .bind(day_key)
    .fetch_optional(pool)
    .await?;
    let next_seq = latest_part_key
        .as_deref()
        .and_then(|value| value.strip_prefix("part-"))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default()
        + 1;
    Ok(format!("part-{next_seq:06}"))
}

pub(crate) fn is_archive_temp_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    (lower.contains(".sqlite.gz.") || lower.contains(".sqlite.zst."))
        && (lower.ends_with(".sqlite")
            || lower.ends_with(".tmp")
            || lower.ends_with(".partial")
            || lower.ends_with(".sqlite-wal")
            || lower.ends_with(".sqlite-shm"))
}

pub(crate) fn collect_archive_file_paths(root: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root)
        .with_context(|| format!("failed to read archive directory {}", root.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_archive_file_paths(&path, files)?;
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

pub(crate) fn inflate_gzip_sqlite_file(source: &Path, destination: &Path) -> Result<()> {
    let input = fs::File::open(source)
        .with_context(|| format!("failed to open archive batch {}", source.display()))?;
    let mut decoder = GzDecoder::new(input);
    let output = fs::File::create(destination)
        .with_context(|| format!("failed to create temp archive db {}", destination.display()))?;
    let mut writer = io::BufWriter::new(output);
    io::copy(&mut decoder, &mut writer).with_context(|| {
        format!(
            "failed to decompress archive batch {} into {}",
            source.display(),
            destination.display()
        )
    })?;
    writer.flush()?;
    Ok(())
}

pub(crate) fn deflate_sqlite_file_to_gzip(source: &Path, destination: &Path) -> Result<()> {
    let input = fs::File::open(source)
        .with_context(|| format!("failed to open temp archive db {}", source.display()))?;
    let output = fs::File::create(destination)
        .with_context(|| format!("failed to create archive gzip {}", destination.display()))?;
    let mut encoder = GzEncoder::new(io::BufWriter::new(output), Compression::default());
    let mut reader = io::BufReader::new(input);
    io::copy(&mut reader, &mut encoder).with_context(|| {
        format!(
            "failed to compress temp archive db {} into {}",
            source.display(),
            destination.display()
        )
    })?;
    let mut writer = encoder
        .finish()
        .context("failed to finish archive gzip writer")?;
    writer.flush()?;
    Ok(())
}

pub(crate) fn sha256_hex_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)
        .with_context(|| format!("failed to open file for sha256 {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0_u8; 8192];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

