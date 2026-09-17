/// The raw-free, versioned Summary input retained for an archive page. This is deliberately
/// shared by the retention writer and off-request projection reader so cleanup and recovery use
/// the same semantic field set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SummaryArchiveSnapshotV2Record {
    pub(crate) id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) source: String,
    pub(crate) model: Option<String>,
    pub(crate) response_model: Option<String>,
    pub(crate) input_tokens: i64,
    pub(crate) output_tokens: i64,
    pub(crate) cache_input_tokens: i64,
    pub(crate) reasoning_tokens: i64,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) total_tokens: i64,
    pub(crate) cost: Option<f64>,
    pub(crate) cost_input: Option<f64>,
    pub(crate) cost_cache_write: Option<f64>,
    pub(crate) cost_cache_read: Option<f64>,
    pub(crate) cost_output: Option<f64>,
    pub(crate) cost_reasoning: Option<f64>,
    pub(crate) status: String,
    pub(crate) error_message: Option<String>,
    pub(crate) failure_kind: Option<String>,
    pub(crate) failure_class: Option<String>,
    pub(crate) is_actionable: bool,
    pub(crate) upstream_account_id: Option<i64>,
}

pub(crate) fn decode_summary_archive_snapshot_v2_payload(
    payload: &[u8],
) -> Result<Vec<SummaryArchiveSnapshotV2Record>> {
    let mut decoder =
        zstd::stream::read::Decoder::new(payload).context("open V2 Summary Snapshot decoder")?;
    let mut decoded = Vec::new();
    decoder
        .by_ref()
        .take((SUMMARY_SOURCE_CHANGE_JOURNAL_MAX_BYTES + 1) as u64)
        .read_to_end(&mut decoded)
        .context("decode V2 Summary Snapshot page")?;
    if decoded.len() > SUMMARY_SOURCE_CHANGE_JOURNAL_MAX_BYTES {
        bail!("V2 Summary Snapshot decoded payload exceeds byte budget");
    }
    let records = serde_json::from_slice::<Vec<SummaryArchiveSnapshotV2Record>>(&decoded)
        .context("parse V2 Summary Snapshot records")?;
    if records.is_empty() {
        bail!("V2 Summary Snapshot page cannot be empty");
    }
    if records.len() > SUMMARY_ARCHIVE_SNAPSHOT_MAX_RECORDS {
        bail!("V2 Summary Snapshot page record budget exceeded");
    }
    if records.iter().any(|record| {
        record.invoke_id.trim().is_empty()
            || record.occurred_at.trim().is_empty()
            || record.source.trim().is_empty()
            || record.status.trim().is_empty()
    }) {
        bail!("V2 Summary Snapshot page contains incomplete semantic record");
    }
    Ok(records)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SummarySourceChangeEntry {
    pub(crate) row_id: i64,
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) current_rank: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SummarySourceChangeDescriptor {
    pub(crate) version: i64,
    pub(crate) source_kind: String,
    pub(crate) source_revision: u64,
    pub(crate) first_row_id: i64,
    pub(crate) last_row_id: i64,
    pub(crate) occurred_start: String,
    pub(crate) occurred_end: String,
    pub(crate) entries: Vec<SummarySourceChangeEntry>,
}

impl SummarySourceChangeDescriptor {
    pub(crate) fn source_change(
        source_kind: impl Into<String>,
        source_revision: u64,
        entries: Vec<SummarySourceChangeEntry>,
    ) -> Result<Self> {
        let first = entries
            .first()
            .ok_or_else(|| anyhow::anyhow!("summary source descriptor cannot be empty"))?;
        let last = entries.last().unwrap_or(first);
        let occurred_start = entries
            .iter()
            .map(|entry| entry.occurred_at.as_str())
            .min()
            .unwrap_or_default()
            .to_string();
        let occurred_end = entries
            .iter()
            .map(|entry| entry.occurred_at.as_str())
            .max()
            .unwrap_or_default()
            .to_string();
        Ok(Self {
            version: SUMMARY_SOURCE_CHANGE_DESCRIPTOR_VERSION,
            source_kind: source_kind.into(),
            source_revision,
            first_row_id: first.row_id,
            last_row_id: last.row_id,
            occurred_start,
            occurred_end,
            entries,
        })
    }

    pub(crate) fn terminal_batch(
        source_revision: u64,
        entries: Vec<SummarySourceChangeEntry>,
    ) -> Result<Self> {
        Self::source_change("terminal_batch", source_revision, entries)
    }

    pub(crate) fn coverage_change(
        source_revision: u64,
        entries: Vec<SummarySourceChangeEntry>,
    ) -> Result<Self> {
        Self::source_change("coverage_change", source_revision, entries)
    }

    pub(crate) fn encoded_len(&self) -> Result<usize> {
        Ok(serde_json::to_vec(self)?.len())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SummarySourceChangeRecord {
    pub(crate) cursor: u64,
    pub(crate) descriptor: SummarySourceChangeDescriptor,
}

/// A compressed, normalized Summary page for one authoritative archive. The caller is
/// responsible for encoding only fields needed by the Summary reducers; raw payloads and preview
/// text are intentionally rejected by convention and bounded by the page byte limit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SummaryArchiveSnapshotPage {
    pub(crate) archive_batch_id: i64,
    pub(crate) manifest_sha256: String,
    pub(crate) page_index: u32,
    pub(crate) coverage_start: String,
    pub(crate) coverage_end: String,
    pub(crate) row_count: u32,
    pub(crate) payload: Vec<u8>,
}

impl SummaryArchiveSnapshotPage {
    pub(crate) fn snapshot_sha256(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(&self.payload);
        format!("{:x}", hasher.finalize())
    }
}
