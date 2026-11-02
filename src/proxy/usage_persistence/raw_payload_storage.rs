use super::*;

#[derive(Default)]
pub(crate) struct RawResponsePreviewBuffer {
    bytes: Vec<u8>,
}

impl RawResponsePreviewBuffer {
    pub(crate) fn append(&mut self, chunk: &[u8]) {
        let remaining = RAW_RESPONSE_PREVIEW_LIMIT.saturating_sub(self.bytes.len());
        if remaining == 0 || chunk.is_empty() {
            return;
        }
        self.bytes
            .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }

    pub(crate) fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn into_preview(self) -> String {
        build_raw_response_preview(&self.bytes)
    }
}

pub(crate) struct BoundedResponseParseBuffer {
    bytes: Vec<u8>,
    limit: usize,
    exceeded_limit: bool,
}

impl BoundedResponseParseBuffer {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            exceeded_limit: false,
        }
    }

    pub(crate) fn append(&mut self, chunk: &[u8]) {
        if self.exceeded_limit || chunk.is_empty() {
            return;
        }

        let remaining = self.limit.saturating_sub(self.bytes.len());
        let take_len = remaining.min(chunk.len());
        if take_len > 0 {
            self.bytes.extend_from_slice(&chunk[..take_len]);
        }
        if take_len < chunk.len() {
            self.exceeded_limit = true;
        }
    }

    pub(crate) fn into_response_info(
        self,
        target: ProxyCaptureTarget,
        content_encoding: Option<&str>,
    ) -> ResponseCaptureInfo {
        let mut response_info =
            parse_target_response_payload(target, &self.bytes, false, content_encoding);
        if self.exceeded_limit {
            merge_response_capture_reason(
                &mut response_info,
                PROXY_USAGE_MISSING_NON_STREAM_PARSE_SKIPPED,
            );
        }
        response_info
    }
}

pub(crate) enum PendingRawPayloadWrite {
    Ready(RawPayloadMeta),
    Task(JoinHandle<RawPayloadMeta>),
}

impl PendingRawPayloadWrite {
    pub(crate) async fn finish(self) -> RawPayloadMeta {
        match self {
            Self::Ready(meta) => meta,
            Self::Task(handle) => match handle.await {
                Ok(meta) => meta,
                Err(err) => RawPayloadMeta {
                    path: None,
                    size_bytes: 0,
                    truncated: true,
                    truncated_reason: Some(format!("write_failed:{err}")),
                    write_fence: None,
                },
            },
        }
    }
}

pub(crate) fn spawn_raw_payload_file_write(
    state: &AppState,
    invoke_id: &str,
    kind: &'static str,
    bytes: Bytes,
    enabled: bool,
) -> PendingRawPayloadWrite {
    if bytes.is_empty() {
        return PendingRawPayloadWrite::Ready(RawPayloadMeta::default());
    }
    if !enabled {
        return PendingRawPayloadWrite::Ready(RawPayloadMeta {
            path: None,
            size_bytes: bytes.len() as i64,
            truncated: false,
            truncated_reason: None,
            write_fence: None,
        });
    }
    let reservation = match state.raw_capture_circuit.admit(bytes.len() as u64) {
        Ok(reservation) => reservation,
        Err(_) => {
            return PendingRawPayloadWrite::Ready(storage_suppressed_raw_payload_meta(
                bytes.len() as i64
            ));
        }
    };

    let semaphore = state.proxy_raw_async_semaphore.clone();
    let invoke_id = invoke_id.to_string();
    let kind_for_spool = kind;
    let bytes_for_spool = bytes.clone();
    if semaphore.available_permits() == 0 {
        let codec = state.config.proxy_raw_compression;
        let spool = match RawOverflowSpool::create(state, &invoke_id, kind_for_spool, codec) {
            Ok(spool) => spool,
            Err(err) => {
                warn!(
                    capture_path = "capture_unavailable",
                    capture_unavailable_reason = "spool_capacity",
                    error = %err,
                    "raw capture unavailable because the durable spool cannot accept it"
                );
                return PendingRawPayloadWrite::Ready(RawPayloadMeta {
                    path: None,
                    size_bytes: bytes_for_spool.len() as i64,
                    truncated: true,
                    truncated_reason: Some("capture_unavailable:spool_capacity".to_string()),
                    write_fence: None,
                });
            }
        };
        return PendingRawPayloadWrite::Task(crate::observability::diagnostics::spawn(
            async move {
                let mut spool = spool;
                if let Err(err) = spool.append(&bytes_for_spool) {
                    reservation.finish(0);
                    return RawPayloadMeta {
                        path: None,
                        size_bytes: bytes_for_spool.len() as i64,
                        truncated: true,
                        truncated_reason: Some(if err.to_string().contains("capacity") {
                            "capture_unavailable:spool_capacity".to_string()
                        } else {
                            "capture_unavailable:spool_write_failed".to_string()
                        }),
                        write_fence: None,
                    };
                }
                let meta = spool.finish(bytes_for_spool.len() as i64).await;
                reservation.finish(raw_payload_stored_bytes(&meta));
                meta
            },
        ));
    }

    let config = state.config.clone();
    PendingRawPayloadWrite::Task(crate::observability::diagnostics::spawn(async move {
        // Queue behind the bounded CPU writer pool instead of dropping an enabled capture.
        let permit = semaphore
            .acquire_owned()
            .await
            .expect("raw writer semaphore is live");
        let _permit = permit;
        let meta = store_raw_payload_file(&config, &invoke_id, kind, bytes).await;
        reservation.finish(raw_payload_stored_bytes(&meta));
        meta
    }))
}

pub(crate) fn spawn_raw_payload_snapshot_write(
    state: Arc<AppState>,
    invoke_id: &str,
    kind: &'static str,
    snapshot: PoolReplayBodySnapshot,
    enabled: bool,
) -> PendingRawPayloadWrite {
    match snapshot {
        PoolReplayBodySnapshot::Empty => PendingRawPayloadWrite::Ready(RawPayloadMeta::default()),
        PoolReplayBodySnapshot::Memory(bytes) => {
            spawn_raw_payload_file_write(state.as_ref(), invoke_id, kind, bytes, enabled)
        }
        PoolReplayBodySnapshot::File { size, .. } if !enabled => {
            PendingRawPayloadWrite::Ready(RawPayloadMeta {
                path: None,
                size_bytes: size as i64,
                truncated: false,
                truncated_reason: None,
                write_fence: None,
            })
        }
        PoolReplayBodySnapshot::File { temp_file, size } => {
            let reservation = match state.raw_capture_circuit.admit(size as u64) {
                Ok(reservation) => reservation,
                Err(_) => {
                    return PendingRawPayloadWrite::Ready(storage_suppressed_raw_payload_meta(
                        size as i64,
                    ));
                }
            };
            let config = state.config.clone();
            let semaphore = state.proxy_raw_async_semaphore.clone();
            let invoke_id = invoke_id.to_string();
            let source_path = temp_file.path.clone();
            PendingRawPayloadWrite::Task(crate::observability::diagnostics::spawn(async move {
                let _temp_file_guard = temp_file;
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .expect("raw writer semaphore is live");
                let meta =
                    store_raw_payload_snapshot_file(&config, &invoke_id, kind, source_path, size)
                        .await;
                reservation.finish(raw_payload_stored_bytes(&meta));
                meta
            }))
        }
    }
}

pub(crate) fn raw_payload_stored_bytes(meta: &RawPayloadMeta) -> u64 {
    meta.path
        .as_deref()
        .and_then(|path| fs::metadata(path).ok())
        .map(|metadata| metadata.len())
        .unwrap_or_default()
}

pub(crate) fn raw_payload_path_for_kind(
    raw_dir: &Path,
    invoke_id: &str,
    kind: &str,
    gzip: bool,
) -> PathBuf {
    let filename = if gzip {
        format!("{invoke_id}-{kind}.bin.gz")
    } else {
        format!("{invoke_id}-{kind}.bin")
    };
    raw_dir.join(filename)
}

pub(crate) fn raw_payload_zstd_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.zst", path.display()))
}

pub(crate) fn raw_payload_path_is_gzip(path: Option<&str>) -> bool {
    path.is_some_and(|value| value.ends_with(".gz"))
}

pub(crate) fn raw_payload_meta_codec(meta: &RawPayloadMeta) -> &'static str {
    if raw_payload_path_is_gzip(meta.path.as_deref()) {
        RAW_CODEC_GZIP
    } else if meta
        .path
        .as_deref()
        .is_some_and(|path| path.ends_with(".zst"))
    {
        RAW_CODEC_ZSTD
    } else {
        RAW_CODEC_IDENTITY
    }
}

fn storage_suppressed_raw_payload_meta(size_bytes: i64) -> RawPayloadMeta {
    RawPayloadMeta {
        path: None,
        size_bytes,
        truncated: false,
        truncated_reason: Some("storage_suppressed".to_string()),
        write_fence: None,
    }
}

pub(crate) fn compress_raw_payload_bytes_to_gzip(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    encoder.finish()
}

pub(crate) fn raw_payload_gzip_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.gz", path.display()))
}

pub(crate) const RAW_OVERFLOW_SPOOL_DIR: &str = ".spool";
const RAW_OVERFLOW_SPOOL_MAGIC: &[u8] = b"CVM_RAW_SPOOL_V1\n";
pub(crate) const RAW_OVERFLOW_SPOOL_SEGMENT_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const RAW_OVERFLOW_SPOOL_MAX_BYTES: u64 = 512 * 1024 * 1024;
const RAW_OVERFLOW_SPOOL_FRAME_OVERHEAD_BYTES: u64 = 8;
const RAW_OVERFLOW_SPOOL_RECOVERY_BATCH_SIZE: usize = 32;
const RAW_OVERFLOW_SPOOL_RECOVERY_SCAN_LIMIT: usize = RAW_OVERFLOW_SPOOL_RECOVERY_BATCH_SIZE * 4;

#[derive(Default)]
struct RawOverflowSpoolAccounting {
    durable_bytes: std::collections::HashMap<PathBuf, u64>,
    reserved_bytes: std::collections::HashMap<PathBuf, u64>,
}

static RAW_OVERFLOW_SPOOL_ACCOUNTING: std::sync::LazyLock<
    std::sync::Mutex<RawOverflowSpoolAccounting>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(RawOverflowSpoolAccounting::default()));
static RAW_OVERFLOW_SPOOL_ACTIVE_CAPTURES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashSet<String>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashSet::new()));

struct StreamingRawPayloadChunk {
    bytes: Bytes,
}

impl StreamingRawPayloadChunk {
    fn unreserved(bytes: Bytes) -> Self {
        Self { bytes }
    }
}

impl std::ops::Deref for StreamingRawPayloadChunk {
    type Target = Bytes;

    fn deref(&self) -> &Self::Target {
        &self.bytes
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RawOverflowSpoolHeader {
    invoke_id: String,
    kind: String,
    codec: RawCompressionCodec,
    #[serde(default)]
    capture_id: Option<String>,
    #[serde(default)]
    segment_index: u32,
}

pub(crate) struct RawOverflowSpool {
    directory: PathBuf,
    capture_id: String,
    segment_index: u32,
    segment_payload_bytes: u64,
    paths: Vec<PathBuf>,
    file: fs::File,
    config: AppConfig,
    invoke_id: String,
    kind: &'static str,
    semaphore: Arc<Semaphore>,
    pending_records: u64,
    pending_bytes: u64,
    payload_limit_bytes: u64,
    payload_bytes: u64,
    exceeded_payload_limit: bool,
    truncation_reason: Option<&'static str>,
}

impl RawOverflowSpool {
    pub(crate) fn create(
        state: &AppState,
        invoke_id: &str,
        kind: &'static str,
        codec: RawCompressionCodec,
    ) -> io::Result<Self> {
        let directory = state
            .config
            .resolved_proxy_raw_dir()
            .join(RAW_OVERFLOW_SPOOL_DIR);
        fs::create_dir_all(&directory)?;
        let payload_limit_bytes = state
            .config
            .proxy_raw_max_bytes
            .map(|limit| u64::try_from(limit).unwrap_or(u64::MAX))
            .unwrap_or(u64::MAX);
        let capture_id = nanoid::nanoid!();
        RAW_OVERFLOW_SPOOL_ACTIVE_CAPTURES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(capture_id.clone());
        let (path, file) = match create_raw_overflow_spool_segment(
            &directory,
            invoke_id,
            kind,
            codec,
            &capture_id,
            0,
        ) {
            Ok(result) => result,
            Err(error) => {
                RAW_OVERFLOW_SPOOL_ACTIVE_CAPTURES
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&capture_id);
                return Err(error);
            }
        };
        let mut config = state.config.clone();
        config.proxy_raw_compression = codec;
        Ok(Self {
            directory,
            capture_id,
            segment_index: 0,
            segment_payload_bytes: 0,
            paths: vec![path],
            file,
            config,
            invoke_id: invoke_id.to_string(),
            kind,
            semaphore: state.proxy_raw_async_semaphore.clone(),
            pending_records: 0,
            pending_bytes: 0,
            payload_limit_bytes,
            payload_bytes: 0,
            exceeded_payload_limit: false,
            truncation_reason: None,
        })
    }

    pub(crate) fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        let remaining_capacity = self.payload_limit_bytes.saturating_sub(self.payload_bytes);
        let accepted_len = remaining_capacity.min(bytes.len() as u64) as usize;
        if accepted_len < bytes.len() {
            self.exceeded_payload_limit = true;
            self.truncation_reason = Some("max_bytes_exceeded");
        }
        let mut remaining = &bytes[..accepted_len];
        while !remaining.is_empty() {
            if self.segment_payload_bytes == RAW_OVERFLOW_SPOOL_SEGMENT_BYTES
                && let Err(error) = self.rotate_segment()
            {
                if error.kind() == io::ErrorKind::WouldBlock {
                    self.mark_spool_capacity_exceeded();
                    break;
                }
                return Err(error);
            }
            let writable = (RAW_OVERFLOW_SPOOL_SEGMENT_BYTES - self.segment_payload_bytes) as usize;
            let frame = &remaining[..remaining.len().min(writable)];
            let reservation_bytes =
                (frame.len() as u64).saturating_add(RAW_OVERFLOW_SPOOL_FRAME_OVERHEAD_BYTES);
            if let Err(error) = reserve_raw_overflow_spool_bytes(&self.directory, reservation_bytes)
            {
                if error.kind() == io::ErrorKind::WouldBlock {
                    self.mark_spool_capacity_exceeded();
                    break;
                }
                return Err(error);
            }
            let write_result = write_raw_overflow_spool_frame(&mut self.file, frame);
            release_raw_overflow_spool_bytes(&self.directory, reservation_bytes);
            write_result?;
            record_raw_overflow_spool_bytes(
                &self.directory,
                reservation_bytes.min(i64::MAX as u64) as i64,
            );
            self.segment_payload_bytes = self
                .segment_payload_bytes
                .saturating_add(frame.len() as u64);
            self.pending_records = self.pending_records.saturating_add(1);
            self.pending_bytes = self.pending_bytes.saturating_add(frame.len() as u64);
            self.payload_bytes = self.payload_bytes.saturating_add(frame.len() as u64);
            remaining = &remaining[frame.len()..];
        }
        Ok(())
    }

    fn mark_spool_capacity_exceeded(&mut self) {
        self.exceeded_payload_limit = true;
        self.truncation_reason = Some("spool_capacity_exceeded");
        // A raw capture is useful only when it is complete. Leave an invalid trailing
        // frame marker so restart recovery retains this evidence instead of publishing
        // the durable prefix as if it were a complete response.
        let _ = self.file.write_all(&[0]);
        let _ = self.file.flush();
    }

    fn rotate_segment(&mut self) -> io::Result<()> {
        self.file.flush()?;
        self.segment_index = self.segment_index.saturating_add(1);
        let (path, file) = create_raw_overflow_spool_segment(
            &self.directory,
            &self.invoke_id,
            self.kind,
            self.config.proxy_raw_compression,
            &self.capture_id,
            self.segment_index,
        )?;
        self.paths.push(path);
        self.file = file;
        self.segment_payload_bytes = 0;
        Ok(())
    }

    pub(crate) async fn finish(mut self, observed_size_bytes: i64) -> RawPayloadMeta {
        if self.truncation_reason == Some("spool_capacity_exceeded") {
            warn!(
                capture_path = "overflow_spool",
                durability_mode = "rejected_at_capacity",
                spool_pending_bytes = self.pending_bytes,
                spool_segment_count = self.paths.len(),
                "raw capture exceeded the durable overflow spool budget; retaining evidence"
            );
            return RawPayloadMeta {
                path: None,
                size_bytes: observed_size_bytes,
                truncated: true,
                truncated_reason: Some("spool_capacity_exceeded".to_string()),
                write_fence: None,
            };
        }
        if let Err(err) = self.file.flush() {
            return RawPayloadMeta {
                path: None,
                size_bytes: observed_size_bytes,
                truncated: true,
                truncated_reason: Some(format!("spool_write_failed:{err}")),
                write_fence: None,
            };
        }
        let completion_marker =
            raw_overflow_spool_completion_marker(&self.directory, &self.capture_id);
        let completion_marker_written =
            fs::write(&completion_marker, self.segment_index.to_string()).is_ok();
        let mut meta = replay_raw_overflow_spool_segments(
            &self.config,
            self.semaphore.clone(),
            self.paths.clone(),
            None,
        )
        .await;
        debug!(
            capture_path = "overflow_spool",
            storage_codec = ?self.config.proxy_raw_compression,
            spool_pending_records = self.pending_records,
            spool_pending_bytes = self.pending_bytes,
            spool_segment_count = self.paths.len(),
            "raw capture overflow spool finished"
        );
        if meta.path.is_some() || meta.truncated_reason.as_deref() == Some("max_bytes_exceeded") {
            match remove_raw_overflow_spool_segments(&self.paths) {
                Ok(()) => {
                    let _ = fs::remove_file(&completion_marker);
                }
                Err(error) => {
                    warn!(
                        error_kind = "spool_cleanup_deferred",
                        error = %error,
                        spool_segment_count = self.paths.len(),
                        "raw overflow spool cleanup deferred; retaining completion marker"
                    );
                }
            }
        } else if !completion_marker_written {
            warn!(
                error_kind = "completion_marker_write_failed",
                "raw overflow spool completion marker could not be written"
            );
        }
        if self.exceeded_payload_limit {
            meta.truncated = true;
            meta.truncated_reason.get_or_insert_with(|| {
                self.truncation_reason
                    .unwrap_or_else(|| {
                        raw_overflow_payload_limit_reason(
                            self.config.proxy_raw_max_bytes,
                            self.payload_limit_bytes,
                        )
                    })
                    .to_string()
            });
        }
        meta
    }
}

fn raw_overflow_spool_completion_marker(directory: &Path, capture_id: &str) -> PathBuf {
    directory.join(format!("{capture_id}.done"))
}

impl Drop for RawOverflowSpool {
    fn drop(&mut self) {
        RAW_OVERFLOW_SPOOL_ACTIVE_CAPTURES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.capture_id);
    }
}

fn raw_overflow_payload_limit_reason(
    configured_max_bytes: Option<usize>,
    payload_limit_bytes: u64,
) -> &'static str {
    if configured_max_bytes
        .is_some_and(|limit| u64::try_from(limit).unwrap_or(u64::MAX) <= payload_limit_bytes)
    {
        "max_bytes_exceeded"
    } else {
        "spool_capacity_exceeded"
    }
}

pub(crate) fn bounded_raw_overflow_spool_directory_bytes(
    directory: &Path,
    max_entries: usize,
) -> io::Result<(u64, bool)> {
    let mut total = 0_u64;
    let mut matching_entries = 0_usize;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("frames")
        {
            if matching_entries >= max_entries {
                return Ok((total, true));
            }
            matching_entries += 1;
            total = total.saturating_add(entry.metadata()?.len());
        }
    }
    Ok((total, false))
}

fn reserve_raw_overflow_spool_bytes(directory: &Path, bytes: u64) -> io::Result<()> {
    let mut accounting = RAW_OVERFLOW_SPOOL_ACCOUNTING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let durable_bytes = accounting
        .durable_bytes
        .get(directory)
        .copied()
        .unwrap_or_default();
    let reserved_bytes = accounting
        .reserved_bytes
        .get(directory)
        .copied()
        .unwrap_or_default();
    if durable_bytes
        .saturating_add(reserved_bytes)
        .saturating_add(bytes)
        > RAW_OVERFLOW_SPOOL_MAX_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "raw overflow spool capacity reached",
        ));
    }
    accounting.reserved_bytes.insert(
        directory.to_path_buf(),
        reserved_bytes.saturating_add(bytes),
    );
    Ok(())
}

fn release_raw_overflow_spool_bytes(directory: &Path, bytes: u64) {
    if bytes == 0 {
        return;
    }
    let mut accounting = RAW_OVERFLOW_SPOOL_ACCOUNTING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(reserved_bytes) = accounting.reserved_bytes.get_mut(directory) else {
        return;
    };
    *reserved_bytes = reserved_bytes.saturating_sub(bytes);
    if *reserved_bytes == 0 {
        accounting.reserved_bytes.remove(directory);
    }
}

pub(crate) fn set_raw_overflow_spool_accounted_bytes(directory: &Path, bytes: u64) {
    let active = RAW_OVERFLOW_SPOOL_ACTIVE_CAPTURES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut accounting = RAW_OVERFLOW_SPOOL_ACCOUNTING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let current = accounting
        .durable_bytes
        .get(directory)
        .copied()
        .unwrap_or_default();
    let reconciled = if active.is_empty() {
        bytes
    } else {
        current.max(bytes)
    };
    accounting
        .durable_bytes
        .insert(directory.to_path_buf(), reconciled);
}

fn record_raw_overflow_spool_bytes(directory: &Path, delta: i64) {
    if delta == 0 {
        return;
    }
    let mut accounting = RAW_OVERFLOW_SPOOL_ACCOUNTING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let current = accounting
        .durable_bytes
        .get(directory)
        .copied()
        .unwrap_or_default();
    let next = if delta >= 0 {
        current.saturating_add(delta as u64)
    } else {
        current.saturating_sub(delta.unsigned_abs())
    };
    accounting
        .durable_bytes
        .insert(directory.to_path_buf(), next);
}

fn create_raw_overflow_spool_segment(
    directory: &Path,
    invoke_id: &str,
    kind: &str,
    codec: RawCompressionCodec,
    capture_id: &str,
    segment_index: u32,
) -> io::Result<(PathBuf, fs::File)> {
    let path = directory.join(format!("{capture_id}-{segment_index:06}.frames"));
    let header = RawOverflowSpoolHeader {
        invoke_id: invoke_id.to_string(),
        kind: kind.to_string(),
        codec,
        capture_id: Some(capture_id.to_string()),
        segment_index,
    };
    let header_bytes = raw_overflow_spool_header_bytes(&header)?;
    let reserved_bytes = (RAW_OVERFLOW_SPOOL_MAGIC.len() as u64)
        .saturating_add(4)
        .saturating_add(header_bytes.len() as u64);
    reserve_raw_overflow_spool_bytes(directory, reserved_bytes)?;
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path);
    let write_result = match &mut file {
        Ok(file) => write_raw_overflow_spool_header_bytes(file, &header_bytes),
        Err(error) => Err(io::Error::new(error.kind(), error.to_string())),
    };
    release_raw_overflow_spool_bytes(directory, reserved_bytes);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&path);
        return Err(error);
    }
    record_raw_overflow_spool_bytes(
        directory,
        fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(reserved_bytes) as i64,
    );
    let file = file?;
    Ok((path, file))
}

fn raw_overflow_spool_header_bytes(header: &RawOverflowSpoolHeader) -> io::Result<Vec<u8>> {
    serde_json::to_vec(header).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn write_raw_overflow_spool_header_bytes(file: &mut fs::File, header: &[u8]) -> io::Result<()> {
    let header_len = u32::try_from(header.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "raw spool header too large"))?;
    file.write_all(RAW_OVERFLOW_SPOOL_MAGIC)?;
    file.write_all(&header_len.to_le_bytes())?;
    file.write_all(header)
}

fn write_raw_overflow_spool_frame(file: &mut fs::File, bytes: &[u8]) -> io::Result<()> {
    let length = u32::try_from(bytes.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "raw spool frame too large"))?;
    let mut crc = Crc32Hasher::new();
    crc.update(bytes);
    file.write_all(&length.to_le_bytes())?;
    file.write_all(&crc.finalize().to_le_bytes())?;
    file.write_all(bytes)
}

fn read_raw_overflow_spool_segment(path: &Path) -> io::Result<(RawOverflowSpoolHeader, Vec<u8>)> {
    let bytes = fs::read(path)?;
    if !bytes.starts_with(RAW_OVERFLOW_SPOOL_MAGIC) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "raw spool has invalid header magic",
        ));
    }
    let mut offset = RAW_OVERFLOW_SPOOL_MAGIC.len();
    if bytes.len().saturating_sub(offset) < 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "raw spool has partial header length",
        ));
    }
    let header_len =
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("header length")) as usize;
    offset += 4;
    let header_end = offset.saturating_add(header_len);
    if header_end > bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "raw spool has partial header",
        ));
    }
    let header = serde_json::from_slice(&bytes[offset..header_end])
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    offset = header_end;
    let mut payload = Vec::new();
    while offset < bytes.len() {
        if bytes.len().saturating_sub(offset) < 8 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "raw spool has partial frame header",
            ));
        }
        let length = u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("frame length"))
            as usize;
        let expected_crc =
            u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().expect("frame crc"));
        offset += 8;
        let end = offset.saturating_add(length);
        if end > bytes.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "raw spool has partial frame payload",
            ));
        }
        let frame = &bytes[offset..end];
        let mut crc = Crc32Hasher::new();
        crc.update(frame);
        if crc.finalize() != expected_crc {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "raw spool frame checksum mismatch",
            ));
        }
        payload.extend_from_slice(frame);
        offset = end;
    }
    Ok((header, payload))
}

fn raw_overflow_spool_capture_key(path: &Path, header: &RawOverflowSpoolHeader) -> String {
    header.capture_id.clone().unwrap_or_else(|| {
        format!(
            "legacy:{}",
            short_sha256_fingerprint(&path.to_string_lossy())
        )
    })
}

fn raw_overflow_spool_capture_key_from_path(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    let (capture_id, segment_index) = stem.rsplit_once('-')?;
    (!capture_id.is_empty()
        && segment_index.len() == 6
        && segment_index.bytes().all(|byte| byte.is_ascii_digit()))
    .then(|| capture_id.to_string())
}

fn validate_raw_overflow_spool_segments(
    segments: &[(PathBuf, RawOverflowSpoolHeader)],
) -> io::Result<RawOverflowSpoolHeader> {
    let Some((_, first)) = segments.first() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "raw spool capture has no segments",
        ));
    };
    let capture_id = raw_overflow_spool_capture_key(&segments[0].0, first);
    for (expected_index, (path, header)) in segments.iter().enumerate() {
        if header.invoke_id != first.invoke_id
            || header.kind != first.kind
            || header.codec != first.codec
            || raw_overflow_spool_capture_key(path, header) != capture_id
            || header
                .capture_id
                .as_ref()
                .is_some_and(|_| header.segment_index != expected_index as u32)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "raw spool capture segments are inconsistent",
            ));
        }
    }
    Ok(first.clone())
}

async fn replay_raw_overflow_spool_segments(
    config: &AppConfig,
    semaphore: Arc<Semaphore>,
    paths: Vec<PathBuf>,
    circuit: Option<Arc<RawCaptureCircuitBreaker>>,
) -> RawPayloadMeta {
    let inspected = match run_blocking_raw_writer_io({
        let paths = paths.clone();
        move || {
            let mut segments = Vec::with_capacity(paths.len());
            for path in paths {
                let (header, _) = read_raw_overflow_spool_segment(&path)?;
                segments.push((path, header));
            }
            validate_raw_overflow_spool_segments(&segments)
        }
    })
    .await
    {
        Ok(header) => header,
        Err(_err) => {
            return RawPayloadMeta {
                path: None,
                size_bytes: 0,
                truncated: true,
                truncated_reason: Some("spool_replay_failed".to_string()),
                write_fence: None,
            };
        }
    };
    let reservation = if let Some(circuit) = circuit {
        let requested_bytes = paths
            .iter()
            .filter_map(|path| fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .sum();
        match circuit.admit_recovery(requested_bytes) {
            Ok(reservation) => Some(reservation),
            Err(_) => {
                return storage_suppressed_raw_payload_meta(requested_bytes as i64);
            }
        }
    } else {
        None
    };
    let permit = semaphore
        .acquire_owned()
        .await
        .expect("raw writer semaphore is live");
    let _permit = permit;
    let path = raw_payload_path_for_kind(
        &config.resolved_proxy_raw_dir(),
        &inspected.invoke_id,
        &inspected.kind,
        false,
    );
    // One segment at a time is enough to keep disk replay busy. A bounded channel
    // prevents recovery from copying the entire durable spool back into RAM.
    let (tx, mut rx) = mpsc::channel::<Bytes>(1);
    let mut replay_config = config.clone();
    replay_config.proxy_raw_compression = inspected.codec;
    if let Some(reservation) = reservation.as_ref() {
        let reservation_limit = usize::try_from(reservation.reserved_bytes()).unwrap_or(usize::MAX);
        replay_config.proxy_raw_max_bytes = Some(
            replay_config
                .proxy_raw_max_bytes
                .map_or(reservation_limit, |configured| {
                    configured.min(reservation_limit)
                }),
        );
    }
    let writer = crate::observability::diagnostics::spawn(async move {
        write_bounded_streaming_raw_payload_to_file(
            path,
            replay_config.proxy_raw_max_bytes,
            replay_config.proxy_raw_immediate_compression_threshold(),
            replay_config.proxy_raw_compression,
            &mut rx,
        )
        .await
    });
    for spool_path in paths {
        let payload = match run_blocking_raw_writer_io(move || {
            read_raw_overflow_spool_segment(&spool_path).map(|(_, payload)| payload)
        })
        .await
        {
            Ok(payload) => payload,
            Err(_err) => {
                drop(tx);
                let _ = writer.await;
                if let Some(reservation) = reservation {
                    reservation.finish(0);
                }
                return RawPayloadMeta {
                    path: None,
                    size_bytes: 0,
                    truncated: true,
                    truncated_reason: Some("spool_replay_failed".to_string()),
                    write_fence: None,
                };
            }
        };
        if tx.send(Bytes::from(payload)).await.is_err() {
            if let Some(reservation) = reservation {
                reservation.finish(0);
            }
            return RawPayloadMeta {
                path: None,
                size_bytes: 0,
                truncated: true,
                truncated_reason: Some("spool_replay_failed".to_string()),
                write_fence: None,
            };
        }
    }
    drop(tx);
    let meta = match writer.await {
        Ok(meta) => meta,
        Err(_err) => RawPayloadMeta {
            path: None,
            size_bytes: 0,
            truncated: true,
            truncated_reason: Some("spool_replay_failed".to_string()),
            write_fence: None,
        },
    };
    if let Some(reservation) = reservation {
        reservation.finish(raw_payload_stored_bytes(&meta));
    }
    meta
}

#[cfg(test)]
tokio::task_local! {
    static RAW_OVERFLOW_SPOOL_TEST_CLEANUP_BUSY: ();
}

fn remove_raw_overflow_spool_segments(paths: &[PathBuf]) -> io::Result<()> {
    // Acquire a competing permit only at cleanup so tests can finish replay first.
    #[cfg(test)]
    let _competing_file_phase = RAW_OVERFLOW_SPOOL_TEST_CLEANUP_BUSY
        .try_with(|()| crate::maintenance::try_acquire_file_phase_permit())
        .ok()
        .flatten();
    let _file_phase = crate::maintenance::try_acquire_file_phase_permit()
        .ok_or_else(|| io::Error::new(io::ErrorKind::WouldBlock, "raw spool file phase is busy"))?;
    for path in paths {
        let bytes = fs::metadata(path)
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        match fs::remove_file(path) {
            Ok(()) => {
                if let Some(directory) = path.parent() {
                    record_raw_overflow_spool_bytes(
                        directory,
                        -(bytes.min(i64::MAX as u64) as i64),
                    );
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub(crate) async fn recover_raw_overflow_spools(config: &AppConfig) {
    recover_raw_overflow_spools_inner(config, None, None).await;
}

pub(crate) async fn recover_raw_overflow_spools_with_circuit(state: &AppState) {
    recover_raw_overflow_spools_inner(
        &state.config,
        Some(state.raw_capture_circuit.clone()),
        Some(state.proxy_raw_async_semaphore.clone()),
    )
    .await;
}

async fn recover_raw_overflow_spools_inner(
    config: &AppConfig,
    circuit: Option<Arc<RawCaptureCircuitBreaker>>,
    writer_semaphore: Option<Arc<Semaphore>>,
) {
    let directory = config.resolved_proxy_raw_dir().join(RAW_OVERFLOW_SPOOL_DIR);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return,
        Err(_) => {
            warn!(
                error_kind = "read_dir_failed",
                "failed to scan raw overflow spool directory"
            );
            return;
        }
    };

    let mut captures = HashMap::<String, Vec<(PathBuf, RawOverflowSpoolHeader)>>::new();
    let mut corrupt_captures = std::collections::HashSet::new();
    let mut active_captures_seen = std::collections::HashSet::new();
    let mut inspected_segments = 0_usize;
    let mut batch_truncated = false;
    for (scanned_entries, entry) in entries.enumerate() {
        if scanned_entries >= RAW_OVERFLOW_SPOOL_RECOVERY_SCAN_LIMIT {
            batch_truncated = true;
            break;
        }
        if inspected_segments >= RAW_OVERFLOW_SPOOL_RECOVERY_BATCH_SIZE {
            batch_truncated = true;
            break;
        }
        let Ok(entry) = entry else {
            inspected_segments += 1;
            continue;
        };
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("frames") {
            inspected_segments += 1;
            continue;
        }
        if let Some(active_capture) = raw_overflow_spool_capture_key_from_path(&path)
            && RAW_OVERFLOW_SPOOL_ACTIVE_CAPTURES
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains(&active_capture)
        {
            // Count one live capture once so a long stream cannot monopolize the
            // bounded recovery budget with its rotated segments.
            if active_captures_seen.insert(active_capture) {
                inspected_segments += 1;
            }
            continue;
        }
        inspected_segments += 1;
        let header = match run_blocking_raw_writer_io({
            let path = path.clone();
            move || read_raw_overflow_spool_segment(&path).map(|(header, _)| header)
        })
        .await
        {
            Ok(header) => header,
            Err(_) => {
                warn!(
                    error_kind = "segment_read_failed",
                    "raw overflow spool is incomplete or corrupt; retaining for inspection"
                );
                if let Some(capture_key) = raw_overflow_spool_capture_key_from_path(&path) {
                    corrupt_captures.insert(capture_key);
                }
                continue;
            }
        };
        let capture_key = raw_overflow_spool_capture_key(&path, &header);
        captures
            .entry(capture_key)
            .or_default()
            .push((path, header));
    }
    let semaphore = writer_semaphore
        .unwrap_or_else(|| Arc::new(Semaphore::new(proxy_raw_async_writer_limit(config))));
    for (capture_key, mut segments) in captures {
        if corrupt_captures.contains(&capture_key) {
            warn!(
                capture_key,
                spool_segment_count = segments.len(),
                "raw overflow spool capture has a corrupt segment; retaining all segments"
            );
            continue;
        }
        segments.sort_by_key(|(_, header)| header.segment_index);
        let header = match validate_raw_overflow_spool_segments(&segments) {
            Ok(header) => header,
            Err(_) => {
                warn!(
                    error_kind = "segment_sequence_invalid",
                    spool_segment_count = segments.len(),
                    "raw overflow spool capture is incomplete or inconsistent; retaining for inspection"
                );
                continue;
            }
        };
        let selected_last_segment = segments
            .last()
            .map(|(_, segment_header)| segment_header.segment_index)
            .unwrap_or(header.segment_index);
        let paths = segments
            .into_iter()
            .map(|(path, _)| path)
            .collect::<Vec<_>>();
        let completion_marker = raw_overflow_spool_completion_marker(&directory, &capture_key);
        let expected_last_segment = fs::read_to_string(&completion_marker)
            .ok()
            .and_then(|value| value.trim().parse::<u32>().ok());
        if expected_last_segment.is_none() && header.capture_id.is_some() {
            continue;
        }
        if (batch_truncated && expected_last_segment.is_none())
            || expected_last_segment.is_some_and(|expected| expected != selected_last_segment)
        {
            continue;
        }
        let meta = replay_raw_overflow_spool_segments(
            config,
            semaphore.clone(),
            paths.clone(),
            circuit.clone(),
        )
        .await;
        if meta.path.is_some() || meta.truncated_reason.as_deref() == Some("max_bytes_exceeded") {
            if let Err(error) = remove_raw_overflow_spool_segments(&paths) {
                warn!(
                    error_kind = "spool_cleanup_deferred",
                    error = %error,
                    kind = %header.kind,
                    spool_segment_count = paths.len(),
                    "raw overflow spool recovery cleanup deferred; retaining completion marker"
                );
                continue;
            }
            let _ = fs::remove_file(completion_marker);
            info!(
                kind = %header.kind,
                replay_count = 1,
                spool_segment_count = paths.len(),
                "recovered raw overflow spool"
            );
        } else {
            warn!(
                kind = %header.kind,
                spool_segment_count = paths.len(),
                reason = ?meta.truncated_reason,
                "raw overflow spool recovery did not publish a payload; retaining spool"
            );
        }
    }
}

pub(crate) fn build_raw_response_preview(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "{}".to_string();
    }
    let preview = if bytes.len() > RAW_RESPONSE_PREVIEW_LIMIT {
        &bytes[..RAW_RESPONSE_PREVIEW_LIMIT]
    } else {
        bytes
    };
    String::from_utf8_lossy(preview).to_string()
}

pub(crate) fn extract_error_message_from_response(bytes: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(bytes).ok()?;
    value
        .pointer("/error/message")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .or_else(|| {
            value
                .get("message")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        })
}

pub(crate) fn summarize_plaintext_upstream_error(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    if text.is_empty() {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    if text.starts_with('<')
        || lower.starts_with("<!doctype")
        || lower.starts_with("<html")
        || lower.starts_with("<body")
    {
        return None;
    }
    Some(text.chars().take(240).collect())
}

pub(crate) fn extract_error_message_from_response_preview(bytes: &[u8]) -> Option<String> {
    extract_error_message_from_response(bytes).or_else(|| summarize_plaintext_upstream_error(bytes))
}

pub(crate) fn merge_response_capture_reason(
    response_info: &mut ResponseCaptureInfo,
    reason: impl Into<String>,
) {
    let reason = reason.into();
    let combined_reason = if let Some(existing) = response_info.usage_missing_reason.take() {
        format!("{reason};{existing}")
    } else {
        reason
    };
    response_info.usage_missing_reason = Some(combined_reason);
}

#[cfg(test)]
mod raw_overflow_spool_tests {
    use super::*;

    async fn spool_cleanup_fixture(invoke_id: &str) -> (Arc<AppState>, RawOverflowSpool) {
        let mut config = crate::tests::test_config();
        config.proxy_raw_dir =
            std::env::temp_dir().join(format!("cvm-raw-spool-cleanup-{}", nanoid::nanoid!()));
        config.proxy_raw_compression = RawCompressionCodec::None;
        config.proxy_raw_max_bytes = None;
        let state = crate::tests::test_state_from_config(config, true).await;
        let mut spool =
            RawOverflowSpool::create(&state, invoke_id, "response", RawCompressionCodec::None)
                .expect("create cleanup test spool");
        spool.append(b"first-").expect("append first segment");
        spool.rotate_segment().expect("rotate cleanup test spool");
        spool.append(b"second").expect("append second segment");
        (state, spool)
    }

    #[tokio::test]
    async fn overflow_spool_cleanup_reports_file_phase_busy_without_removing_segments() {
        let (state, spool) = spool_cleanup_fixture("cleanup-file-phase-busy").await;
        let file_phase = crate::maintenance::acquire_file_phase_permit("raw_spool_cleanup_test")
            .await
            .expect("hold competing file phase permit");

        let error = remove_raw_overflow_spool_segments(&spool.paths)
            .expect_err("busy cleanup must report deferral");

        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(spool.paths.iter().all(|path| path.is_file()));
        crate::maintenance::with_file_phase_permit(file_phase, async {
            remove_raw_overflow_spool_segments(&spool.paths).expect("cleanup with owned permit");
            remove_raw_overflow_spool_segments(&spool.paths).expect("cleanup is idempotent");
        })
        .await;
        assert!(spool.paths.iter().all(|path| !path.exists()));
        drop(spool);
        fs::remove_dir_all(state.config.resolved_proxy_raw_dir()).expect("remove cleanup fixture");
    }

    #[tokio::test]
    async fn overflow_spool_finish_cleanup_deferred_retains_segments_and_marker_for_recovery() {
        let (state, spool) = spool_cleanup_fixture("finish-cleanup-deferred").await;
        let paths = spool.paths.clone();
        let marker = raw_overflow_spool_completion_marker(&spool.directory, &spool.capture_id);
        let original_segments = paths
            .iter()
            .map(|path| fs::read(path).expect("read original segment"))
            .collect::<Vec<_>>();

        let meta = RAW_OVERFLOW_SPOOL_TEST_CLEANUP_BUSY
            .scope((), spool.finish(12))
            .await;

        assert!(!meta.truncated);
        let payload_path = PathBuf::from(meta.path.expect("published payload path"));
        assert_eq!(
            fs::read(&payload_path).expect("read published payload"),
            b"first-second"
        );
        for (path, original) in paths.iter().zip(&original_segments) {
            assert_eq!(&fs::read(path).expect("retained segment"), original);
        }
        assert_eq!(
            fs::read_to_string(&marker).expect("completion marker retained after deferred finish"),
            "1"
        );

        fs::remove_file(&payload_path).expect("remove payload to prove recovery replays");
        recover_raw_overflow_spools(&state.config).await;

        assert_eq!(
            fs::read(payload_path).expect("read recovered payload"),
            b"first-second"
        );
        assert!(paths.iter().all(|path| !path.exists()));
        assert!(!marker.exists());
        fs::remove_dir_all(state.config.resolved_proxy_raw_dir()).expect("remove cleanup fixture");
    }

    #[tokio::test]
    async fn overflow_spool_recovery_cleanup_deferred_retains_segments_and_marker_for_retry() {
        let (state, mut spool) = spool_cleanup_fixture("recovery-cleanup-deferred").await;
        spool.file.flush().expect("flush recovery spool");
        let paths = spool.paths.clone();
        let marker = raw_overflow_spool_completion_marker(&spool.directory, &spool.capture_id);
        fs::write(&marker, spool.segment_index.to_string()).expect("write completion marker");
        drop(spool);
        let payload_path = raw_payload_path_for_kind(
            &state.config.resolved_proxy_raw_dir(),
            "recovery-cleanup-deferred",
            "response",
            false,
        );

        RAW_OVERFLOW_SPOOL_TEST_CLEANUP_BUSY
            .scope((), recover_raw_overflow_spools(&state.config))
            .await;

        assert_eq!(
            fs::read(&payload_path).expect("read first recovered payload"),
            b"first-second"
        );
        assert!(paths.iter().all(|path| path.is_file()));
        assert_eq!(
            fs::read_to_string(&marker)
                .expect("completion marker retained after deferred recovery"),
            "1"
        );

        fs::remove_file(&payload_path).expect("remove payload to prove retry replays");
        recover_raw_overflow_spools(&state.config).await;

        assert_eq!(
            fs::read(payload_path).expect("read retried payload"),
            b"first-second"
        );
        assert!(paths.iter().all(|path| !path.exists()));
        assert!(!marker.exists());
        fs::remove_dir_all(state.config.resolved_proxy_raw_dir()).expect("remove cleanup fixture");
    }

    #[test]
    fn overflow_spool_preserves_configured_max_bytes_reason() {
        assert_eq!(
            raw_overflow_payload_limit_reason(Some(1024), 1024),
            "max_bytes_exceeded"
        );
        assert_eq!(
            raw_overflow_payload_limit_reason(Some(2048), 1024),
            "spool_capacity_exceeded"
        );
    }

    #[test]
    fn storage_suppression_is_not_payload_truncation() {
        let meta = storage_suppressed_raw_payload_meta(128);
        assert!(!meta.truncated);
        assert_eq!(meta.truncated_reason.as_deref(), Some("storage_suppressed"));
        assert!(meta.path.is_none());
    }
}
