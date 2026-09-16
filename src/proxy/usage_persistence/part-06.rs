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

pub(crate) fn compress_raw_payload_bytes_to_gzip(bytes: &[u8]) -> io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    encoder.finish()
}

pub(crate) fn raw_payload_gzip_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.gz", path.display()))
}

pub(crate) enum StreamingRawPayloadWriterState {
    Buffer(Vec<u8>),
    Plain {
        path: PathBuf,
        file: fs::File,
    },
    Gzip {
        path: PathBuf,
        encoder: GzEncoder<io::BufWriter<fs::File>>,
    },
    Zstd {
        path: PathBuf,
        encoder: zstd::stream::write::Encoder<'static, io::BufWriter<fs::File>>,
    },
}

impl StreamingRawPayloadWriterState {
    fn current_path(&self) -> Option<&Path> {
        match self {
            Self::Buffer(_) => None,
            Self::Plain { path, .. } | Self::Gzip { path, .. } | Self::Zstd { path, .. } => {
                Some(path.as_path())
            }
        }
    }
}

pub(crate) fn prepare_streaming_raw_parent(path: &Path) -> io::Result<&Path> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("raw payload path has no parent: {}", path.display()),
        )
    })?;
    fs::create_dir_all(parent)?;
    Ok(parent)
}

pub(crate) fn create_plain_streaming_raw_file(path: &Path) -> io::Result<fs::File> {
    prepare_streaming_raw_parent(path)?;
    fs::File::create(path)
}

pub(crate) fn create_gzip_streaming_raw_encoder(
    path: &Path,
) -> io::Result<GzEncoder<io::BufWriter<fs::File>>> {
    prepare_streaming_raw_parent(path)?;
    let file = fs::File::create(path)?;
    Ok(GzEncoder::new(
        io::BufWriter::new(file),
        Compression::default(),
    ))
}

pub(crate) fn create_zstd_streaming_raw_encoder(
    path: &Path,
) -> io::Result<zstd::stream::write::Encoder<'static, io::BufWriter<fs::File>>> {
    prepare_streaming_raw_parent(path)?;
    let file = fs::File::create(path)?;
    zstd::stream::write::Encoder::new(io::BufWriter::new(file), 0)
}

pub(crate) async fn run_blocking_raw_writer_io<T, F>(op: F) -> io::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> io::Result<T> + Send + 'static,
{
    tokio::task::spawn_blocking(op)
        .await
        .map_err(|err| io::Error::other(format!("raw writer task join failed: {err}")))?
}

const RAW_OVERFLOW_SPOOL_DIR: &str = ".spool";
const RAW_OVERFLOW_SPOOL_MAGIC: &[u8] = b"CVM_RAW_SPOOL_V1\n";
pub(crate) const RAW_OVERFLOW_SPOOL_SEGMENT_BYTES: u64 = 16 * 1024 * 1024;
pub(crate) const RAW_OVERFLOW_SPOOL_MAX_BYTES: u64 = 512 * 1024 * 1024;
const RAW_OVERFLOW_SPOOL_FRAME_OVERHEAD_BYTES: u64 = 8;

static RAW_OVERFLOW_SPOOL_RESERVATIONS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<PathBuf, u64>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

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

struct RawOverflowSpool {
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
    fn create(
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
        let (path, file) =
            create_raw_overflow_spool_segment(&directory, invoke_id, kind, codec, &capture_id, 0)?;
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

    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
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

    async fn finish(mut self, observed_size_bytes: i64) -> RawPayloadMeta {
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
            };
        }
        if let Err(err) = self.file.flush() {
            return RawPayloadMeta {
                path: None,
                size_bytes: observed_size_bytes,
                truncated: true,
                truncated_reason: Some(format!("spool_write_failed:{err}")),
            };
        }
        let mut meta = replay_raw_overflow_spool_segments(
            &self.config,
            self.semaphore.clone(),
            self.paths.clone(),
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
            remove_raw_overflow_spool_segments(&self.paths);
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

fn raw_overflow_spool_directory_bytes(directory: &Path) -> io::Result<u64> {
    let mut total = 0_u64;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("frames")
        {
            total = total.saturating_add(entry.metadata()?.len());
        }
    }
    Ok(total)
}

fn reserve_raw_overflow_spool_bytes(directory: &Path, bytes: u64) -> io::Result<()> {
    let mut reservations = RAW_OVERFLOW_SPOOL_RESERVATIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // Keep the disk sample and the in-process reservation in one critical section. A
    // concurrent writer may otherwise observe a stale directory size after another
    // writer commits and releases its temporary reservation.
    let on_disk_bytes = raw_overflow_spool_directory_bytes(directory)?;
    let reserved_bytes = reservations.get(directory).copied().unwrap_or_default();
    if on_disk_bytes
        .saturating_add(reserved_bytes)
        .saturating_add(bytes)
        > RAW_OVERFLOW_SPOOL_MAX_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::WouldBlock,
            "raw overflow spool capacity reached",
        ));
    }
    reservations.insert(
        directory.to_path_buf(),
        reserved_bytes.saturating_add(bytes),
    );
    Ok(())
}

fn release_raw_overflow_spool_bytes(directory: &Path, bytes: u64) {
    if bytes == 0 {
        return;
    }
    let mut reservations = RAW_OVERFLOW_SPOOL_RESERVATIONS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(reserved_bytes) = reservations.get_mut(directory) else {
        return;
    };
    *reserved_bytes = reserved_bytes.saturating_sub(bytes);
    if *reserved_bytes == 0 {
        reservations.remove(directory);
    }
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
    header
        .capture_id
        .clone()
        .unwrap_or_else(|| format!("legacy:{}", path.display()))
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
        Err(err) => {
            return RawPayloadMeta {
                path: None,
                size_bytes: 0,
                truncated: true,
                truncated_reason: Some(format!("spool_replay_failed:{err}")),
            };
        }
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
    let writer = tokio::spawn(async move {
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
            Err(err) => {
                drop(tx);
                let _ = writer.await;
                return RawPayloadMeta {
                    path: None,
                    size_bytes: 0,
                    truncated: true,
                    truncated_reason: Some(format!("spool_replay_failed:{err}")),
                };
            }
        };
        if tx.send(Bytes::from(payload)).await.is_err() {
            return RawPayloadMeta {
                path: None,
                size_bytes: 0,
                truncated: true,
                truncated_reason: Some("spool_replay_failed:raw writer closed".to_string()),
            };
        }
    }
    drop(tx);
    match writer.await {
        Ok(meta) => meta,
        Err(err) => RawPayloadMeta {
            path: None,
            size_bytes: 0,
            truncated: true,
            truncated_reason: Some(format!("spool_replay_failed:{err}")),
        },
    }
}

fn remove_raw_overflow_spool_segments(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

pub(crate) async fn recover_raw_overflow_spools(config: &AppConfig) {
    let directory = config.resolved_proxy_raw_dir().join(RAW_OVERFLOW_SPOOL_DIR);
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return,
        Err(err) => {
            warn!(path = %directory.display(), error = %err, "failed to scan raw overflow spool directory");
            return;
        }
    };

    let mut captures = HashMap::<String, Vec<(PathBuf, RawOverflowSpoolHeader)>>::new();
    let mut corrupt_captures = std::collections::HashSet::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("frames") {
            continue;
        }
        let header = match run_blocking_raw_writer_io({
            let path = path.clone();
            move || read_raw_overflow_spool_segment(&path).map(|(header, _)| header)
        })
        .await
        {
            Ok(header) => header,
            Err(err) => {
                warn!(path = %path.display(), error = %err, "raw overflow spool is incomplete or corrupt; retaining for inspection");
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

    let semaphore = Arc::new(Semaphore::new(proxy_raw_async_writer_limit(config)));
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
            Err(err) => {
                warn!(error = %err, spool_segment_count = segments.len(), "raw overflow spool capture is incomplete or inconsistent; retaining for inspection");
                continue;
            }
        };
        let paths = segments
            .into_iter()
            .map(|(path, _)| path)
            .collect::<Vec<_>>();
        let meta =
            replay_raw_overflow_spool_segments(config, semaphore.clone(), paths.clone()).await;
        if meta.path.is_some() || meta.truncated_reason.as_deref() == Some("max_bytes_exceeded") {
            remove_raw_overflow_spool_segments(&paths);
            info!(
                invoke_id = %header.invoke_id,
                kind = %header.kind,
                replay_count = 1,
                spool_segment_count = paths.len(),
                "recovered raw overflow spool"
            );
        } else {
            warn!(
                invoke_id = %header.invoke_id,
                kind = %header.kind,
                spool_segment_count = paths.len(),
                reason = ?meta.truncated_reason,
                "raw overflow spool recovery did not publish a payload; retaining spool"
            );
        }
    }
}

pub(crate) struct AsyncStreamingRawPayloadWriter {
    tx: Option<std::sync::mpsc::SyncSender<TrackedRawPayloadChunk>>,
    meta_rx: Option<oneshot::Receiver<RawPayloadMeta>>,
    observed_size_bytes: i64,
    local_truncated_reason: Option<String>,
    local_truncated: bool,
    spool: Option<RawOverflowSpool>,
}

static RAW_ASYNC_WRITER_QUEUED_BYTES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

pub(crate) fn proxy_raw_async_writer_queued_bytes() -> usize {
    RAW_ASYNC_WRITER_QUEUED_BYTES.load(std::sync::atomic::Ordering::Relaxed)
}

struct TrackedRawPayloadChunk {
    bytes: Option<Bytes>,
}

impl TrackedRawPayloadChunk {
    fn new(bytes: Bytes) -> Self {
        Self { bytes: Some(bytes) }
    }

    fn into_bytes(mut self) -> Bytes {
        let bytes = self.bytes.take().unwrap_or_default();
        release_raw_async_writer_queued_bytes(bytes.len());
        bytes
    }
}

impl Drop for TrackedRawPayloadChunk {
    fn drop(&mut self) {
        if let Some(bytes) = self.bytes.as_ref() {
            release_raw_async_writer_queued_bytes(bytes.len());
        }
    }
}

trait RawPayloadChunk {
    fn into_bytes(self) -> Bytes;
}

impl RawPayloadChunk for Bytes {
    fn into_bytes(self) -> Bytes {
        self
    }
}

impl RawPayloadChunk for TrackedRawPayloadChunk {
    fn into_bytes(self) -> Bytes {
        TrackedRawPayloadChunk::into_bytes(self)
    }
}
