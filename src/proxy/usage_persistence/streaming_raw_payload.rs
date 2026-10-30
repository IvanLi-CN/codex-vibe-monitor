use super::*;

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
    let file_phase = crate::maintenance::acquire_file_phase_permit("proxy_raw_file_io")
        .await
        .map_err(|error| io::Error::other(error.to_string()))?;
    tokio::task::spawn_blocking(move || {
        let _file_phase = file_phase;
        op()
    })
    .await
    .map_err(|err| io::Error::other(format!("raw writer task join failed: {err}")))?
}

pub(crate) struct StreamingRawPayloadChunk {
    bytes: Bytes,
}

impl StreamingRawPayloadChunk {
    pub(crate) fn unreserved(bytes: Bytes) -> Self {
        Self { bytes }
    }
}

impl std::ops::Deref for StreamingRawPayloadChunk {
    type Target = Bytes;

    fn deref(&self) -> &Self::Target {
        &self.bytes
    }
}

pub(crate) struct AsyncStreamingRawPayloadWriter {
    tx: Option<std::sync::mpsc::SyncSender<TrackedRawPayloadChunk>>,
    meta_rx: Option<oneshot::Receiver<RawPayloadMeta>>,
    observed_size_bytes: i64,
    local_truncated_reason: Option<String>,
    local_truncated: bool,
    spool: Option<RawOverflowSpool>,
    reservation: Option<RawCaptureReservation>,
    reservation_shared: Option<Arc<std::sync::Mutex<Option<RawCaptureReservation>>>>,
    reserved_payload_bytes: u64,
    reservation_circuit: Option<Arc<RawCaptureCircuitBreaker>>,
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

impl AsyncStreamingRawPayloadWriter {
    pub(crate) fn new(
        state: &AppState,
        invoke_id: &str,
        kind: &'static str,
        enabled: bool,
        wire_content_encoding: Option<&str>,
    ) -> Self {
        Self::new_with_expected_size(state, invoke_id, kind, enabled, wire_content_encoding, None)
    }

    pub(crate) fn new_with_expected_size(
        state: &AppState,
        invoke_id: &str,
        kind: &'static str,
        enabled: bool,
        wire_content_encoding: Option<&str>,
        expected_size_bytes: Option<u64>,
    ) -> Self {
        if !enabled {
            return Self {
                tx: None,
                meta_rx: None,
                observed_size_bytes: 0,
                local_truncated_reason: None,
                local_truncated: false,
                spool: None,
                reservation: None,
                reservation_shared: None,
                reserved_payload_bytes: 0,
                reservation_circuit: None,
            };
        }

        let reservation = match state
            .raw_capture_circuit
            .admit(expected_size_bytes.unwrap_or_default())
        {
            Ok(reservation) => reservation,
            Err(_) => {
                return Self {
                    tx: None,
                    meta_rx: None,
                    observed_size_bytes: 0,
                    local_truncated_reason: Some("storage_suppressed".to_string()),
                    local_truncated: false,
                    spool: None,
                    reservation: None,
                    reservation_shared: None,
                    reserved_payload_bytes: 0,
                    reservation_circuit: None,
                };
            }
        };

        let path = raw_payload_path_for_kind(
            &state.config.resolved_proxy_raw_dir(),
            invoke_id,
            kind,
            false,
        );
        let max_bytes = state.config.proxy_raw_max_bytes;
        let immediate_gzip_bytes = state.config.proxy_raw_immediate_compression_threshold();
        // A wire-compressed response is already compressed. Preserve its bytes so the
        // capture path never spends CPU decoding and recompressing gzip/deflate/zstd.
        let codec = wire_content_encoding
            .map(str::trim)
            .filter(|encoding| !encoding.is_empty() && !encoding.eq_ignore_ascii_case("identity"))
            .map(|_| RawCompressionCodec::None)
            .unwrap_or(state.config.proxy_raw_compression);
        let semaphore = state.proxy_raw_async_semaphore.clone();
        let writer_max = proxy_raw_async_writer_limit(&state.config);
        let writer_active = writer_max.saturating_sub(semaphore.available_permits());
        let permit = semaphore.clone().try_acquire_owned();
        if permit.is_err() {
            return match RawOverflowSpool::create(state, invoke_id, kind, codec) {
                Ok(spool) => {
                    debug!(
                        capture_path = "overflow_spool",
                        storage_codec = ?codec,
                        writer_active,
                        writer_max,
                        spool_pending_records = 0,
                        spool_pending_bytes = 0,
                        "raw capture queued to durable overflow spool"
                    );
                    Self {
                        tx: None,
                        meta_rx: None,
                        observed_size_bytes: 0,
                        local_truncated_reason: None,
                        local_truncated: false,
                        spool: Some(spool),
                        reservation: Some(reservation),
                        reservation_shared: None,
                        reserved_payload_bytes: expected_size_bytes.unwrap_or_default(),
                        reservation_circuit: Some(state.raw_capture_circuit.clone()),
                    }
                }
                Err(err) => {
                    reservation.finish(0);
                    warn!(
                        capture_path = "capture_unavailable",
                        capture_unavailable_reason = "spool_capacity",
                        storage_codec = ?codec,
                        writer_active,
                        writer_max,
                        error = %err,
                        "raw streaming capture unavailable because the durable spool cannot accept it"
                    );
                    Self {
                        tx: None,
                        meta_rx: None,
                        observed_size_bytes: 0,
                        local_truncated_reason: Some(
                            "capture_unavailable:spool_capacity".to_string(),
                        ),
                        local_truncated: true,
                        spool: None,
                        reservation: None,
                        reservation_shared: None,
                        reserved_payload_bytes: 0,
                        reservation_circuit: None,
                    }
                }
            };
        }
        let permit = permit.expect("checked above");
        let (tx, rx) = std::sync::mpsc::sync_channel::<TrackedRawPayloadChunk>(64);
        let (meta_tx, meta_rx) = oneshot::channel();
        debug!(
            capture_path = "direct_writer",
            storage_codec = ?codec,
            writer_active,
            writer_max,
            "raw capture assigned to compression writer"
        );
        let reservation_shared = Arc::new(std::sync::Mutex::new(Some(reservation)));
        let reservation_shared_for_task = reservation_shared.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let meta = write_direct_streaming_raw_payload_to_file_tracked(
                path,
                max_bytes,
                immediate_gzip_bytes,
                codec,
                rx,
            );
            if let Ok(mut reservation) = reservation_shared_for_task.lock()
                && let Some(reservation) = reservation.take()
            {
                reservation.finish(raw_payload_stored_bytes(&meta));
            }
            let _ = meta_tx.send(meta);
        });

        Self {
            tx: Some(tx),
            meta_rx: Some(meta_rx),
            observed_size_bytes: 0,
            local_truncated_reason: None,
            local_truncated: false,
            spool: None,
            reservation: None,
            reservation_shared: Some(reservation_shared),
            reserved_payload_bytes: expected_size_bytes.unwrap_or_default(),
            reservation_circuit: Some(state.raw_capture_circuit.clone()),
        }
    }

    fn mark_writer_closed(&mut self, message: String) {
        self.local_truncated = message != "storage_suppressed";
        self.local_truncated_reason.get_or_insert_with(|| {
            if message.starts_with("capture_unavailable:") {
                message
            } else {
                format!("write_failed:{message}")
            }
        });
        self.tx = None;
    }

    pub(crate) fn append(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.observed_size_bytes = self.observed_size_bytes.saturating_add(bytes.len() as i64);
        let required_bytes = self.observed_size_bytes.max(0) as u64;
        let additional_bytes = required_bytes.saturating_sub(self.reserved_payload_bytes);
        if additional_bytes > 0 {
            let result = if let Some(reservation) = self.reservation.as_mut() {
                reservation.extend(additional_bytes)
            } else if let Some(reservation_shared) = self.reservation_shared.as_ref() {
                match reservation_shared.lock() {
                    Ok(mut reservation) => reservation
                        .as_mut()
                        .map_or(Ok(()), |reservation| reservation.extend(additional_bytes)),
                    Err(_) => Err(RawCaptureAdmissionError {
                        reason: "inventory_unready",
                    }),
                }
            } else {
                Ok(())
            };
            if result.is_err() {
                self.mark_writer_closed("storage_suppressed".to_string());
                return;
            }
            self.reserved_payload_bytes = required_bytes;
        }
        if let Some(spool) = self.spool.as_mut() {
            if let Err(err) = spool.append(bytes) {
                self.spool = None;
                let reason = if err.to_string().contains("capacity") {
                    "capture_unavailable:spool_capacity"
                } else {
                    "capture_unavailable:spool_write_failed"
                };
                self.mark_writer_closed(reason.to_string());
            }
            return;
        }
        let Some(tx) = self.tx.as_ref() else {
            return;
        };
        RAW_ASYNC_WRITER_QUEUED_BYTES.fetch_add(bytes.len(), std::sync::atomic::Ordering::Relaxed);
        match tx.try_send(TrackedRawPayloadChunk::new(Bytes::copy_from_slice(bytes))) {
            Ok(()) => {}
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                self.mark_writer_closed("capture_unavailable:ingress_queue_full".to_string());
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                self.mark_writer_closed("capture_unavailable:writer_closed".to_string());
            }
        }
    }

    pub(crate) async fn finish(mut self) -> RawPayloadMeta {
        self.tx.take();
        let mut meta = if let Some(spool) = self.spool.take() {
            let meta = spool.finish(self.observed_size_bytes).await;
            if let Some(reservation) = self.reservation.take() {
                reservation.finish(raw_payload_stored_bytes(&meta));
            }
            meta
        } else {
            match self.meta_rx.take() {
                Some(meta_rx) => match meta_rx.await {
                    Ok(meta) => meta,
                    Err(err) => RawPayloadMeta {
                        path: None,
                        size_bytes: self.observed_size_bytes,
                        truncated: true,
                        truncated_reason: Some(format!("write_failed:{err}")),
                        write_fence: None,
                    },
                },
                None => RawPayloadMeta::default(),
            }
        };
        meta.size_bytes = self.observed_size_bytes;
        if self.local_truncated {
            meta.truncated = true;
        }
        if let Some(reason) = self.local_truncated_reason.as_deref() {
            if meta.truncated_reason.is_none() {
                meta.truncated_reason = Some(reason.to_string());
            }
            if (reason.starts_with("capture_unavailable:") || reason == "storage_suppressed")
                && let Some(path) = meta.path.take()
            {
                let stored_bytes = fs::metadata(&path)
                    .map(|metadata| metadata.len())
                    .unwrap_or_default();
                let _ = fs::remove_file(path);
                if let Some(circuit) = self.reservation_circuit.as_ref() {
                    circuit.record_deleted_bytes(stored_bytes);
                }
            }
        }
        meta
    }
}

pub(crate) async fn write_streaming_raw_payload_to_file(
    path: PathBuf,
    max_bytes: Option<usize>,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    rx: &mut mpsc::UnboundedReceiver<Bytes>,
) -> RawPayloadMeta {
    write_streaming_raw_payload_to_file_from_receiver(
        path,
        max_bytes,
        immediate_gzip_bytes,
        codec,
        StreamingRawPayloadReceiver::Unbounded(rx),
    )
    .await
}

async fn write_reserved_streaming_raw_payload_to_file(
    path: PathBuf,
    max_bytes: Option<usize>,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    rx: &mut mpsc::UnboundedReceiver<StreamingRawPayloadChunk>,
) -> RawPayloadMeta {
    write_streaming_raw_payload_to_file_from_receiver(
        path,
        max_bytes,
        immediate_gzip_bytes,
        codec,
        StreamingRawPayloadReceiver::ReservedUnbounded(rx),
    )
    .await
}

pub(crate) fn write_direct_streaming_raw_payload_to_file(
    path: PathBuf,
    max_bytes: Option<usize>,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    rx: std::sync::mpsc::Receiver<Bytes>,
) -> RawPayloadMeta {
    write_direct_streaming_raw_payload_to_file_inner(
        path,
        max_bytes,
        immediate_gzip_bytes,
        codec,
        rx,
    )
}

fn write_direct_streaming_raw_payload_to_file_tracked(
    path: PathBuf,
    max_bytes: Option<usize>,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    rx: std::sync::mpsc::Receiver<TrackedRawPayloadChunk>,
) -> RawPayloadMeta {
    write_direct_streaming_raw_payload_to_file_inner(
        path,
        max_bytes,
        immediate_gzip_bytes,
        codec,
        rx,
    )
}

fn release_raw_async_writer_queued_bytes(bytes: usize) {
    let _ = RAW_ASYNC_WRITER_QUEUED_BYTES.try_update(
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
        |queued| Some(queued.saturating_sub(bytes)),
    );
}

fn write_direct_streaming_raw_payload_to_file_inner(
    path: PathBuf,
    max_bytes: Option<usize>,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    rx: std::sync::mpsc::Receiver<impl RawPayloadChunk>,
) -> RawPayloadMeta {
    enum DirectWriter {
        Buffer(Vec<u8>),
        Plain(fs::File),
        Gzip(flate2::write::GzEncoder<io::BufWriter<fs::File>>),
        Zstd(zstd::stream::write::Encoder<'static, io::BufWriter<fs::File>>),
    }

    let gzip_path = raw_payload_gzip_path(&path);
    let zstd_path = raw_payload_zstd_path(&path);
    let write_fence = match crate::maintenance::acquire_retention_raw_write_fence(&path) {
        Ok(write_fence) => write_fence,
        Err(error) => {
            return RawPayloadMeta {
                truncated: true,
                truncated_reason: Some(format!("write_failed:{error}")),
                ..RawPayloadMeta::default()
            };
        }
    };
    let mut writer = match codec {
        // Gzip retains the existing hot-plaintext threshold. Zstd intentionally starts at
        // the first byte: it is the current identity-response storage format.
        RawCompressionCodec::Gzip if immediate_gzip_bytes.is_some() => {
            Ok(DirectWriter::Buffer(Vec::new()))
        }
        RawCompressionCodec::Gzip => {
            create_plain_streaming_raw_file(&path).map(DirectWriter::Plain)
        }
        RawCompressionCodec::Zstd => {
            create_zstd_streaming_raw_encoder(&zstd_path).map(DirectWriter::Zstd)
        }
        RawCompressionCodec::None => {
            create_plain_streaming_raw_file(&path).map(DirectWriter::Plain)
        }
    };
    let mut meta = RawPayloadMeta::default();
    let mut written_bytes = 0usize;
    let mut active_path = match codec {
        RawCompressionCodec::Zstd => Some(zstd_path.clone()),
        RawCompressionCodec::Gzip if immediate_gzip_bytes.is_none() => Some(path.clone()),
        RawCompressionCodec::None => Some(path.clone()),
        RawCompressionCodec::Gzip => None,
    };
    while let Ok(chunk) = rx.recv() {
        let bytes = chunk.into_bytes();
        meta.size_bytes = meta.size_bytes.saturating_add(bytes.len() as i64);
        let write_len = max_bytes
            .map(|limit| limit.saturating_sub(written_bytes).min(bytes.len()))
            .unwrap_or(bytes.len());
        if write_len < bytes.len() {
            meta.truncated = true;
            meta.truncated_reason
                .get_or_insert_with(|| "max_bytes_exceeded".to_string());
        }
        if write_len == 0 {
            continue;
        }
        let current_writer = std::mem::replace(&mut writer, Ok(DirectWriter::Buffer(Vec::new())));
        let result = match current_writer {
            Ok(DirectWriter::Buffer(mut buffer)) => {
                buffer.extend_from_slice(&bytes[..write_len]);
                if buffer.len()
                    >= immediate_gzip_bytes.expect("buffer only used for gzip threshold")
                {
                    match create_gzip_streaming_raw_encoder(&gzip_path).and_then(|mut encoder| {
                        encoder.write_all(&buffer)?;
                        Ok(encoder)
                    }) {
                        Ok(encoder) => {
                            active_path = Some(gzip_path.clone());
                            writer = Ok(DirectWriter::Gzip(encoder));
                            Ok(())
                        }
                        Err(error) => Err(error),
                    }
                } else {
                    writer = Ok(DirectWriter::Buffer(buffer));
                    Ok(())
                }
            }
            Ok(DirectWriter::Plain(mut file)) => match file.write_all(&bytes[..write_len]) {
                Ok(()) => {
                    writer = Ok(DirectWriter::Plain(file));
                    Ok(())
                }
                Err(error) => Err(error),
            },
            Ok(DirectWriter::Gzip(mut encoder)) => match encoder.write_all(&bytes[..write_len]) {
                Ok(()) => {
                    writer = Ok(DirectWriter::Gzip(encoder));
                    Ok(())
                }
                Err(error) => Err(error),
            },
            Ok(DirectWriter::Zstd(mut encoder)) => match encoder.write_all(&bytes[..write_len]) {
                Ok(()) => {
                    writer = Ok(DirectWriter::Zstd(encoder));
                    Ok(())
                }
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            if let Some(active_path) = active_path.as_deref() {
                let _ = fs::remove_file(active_path);
            }
            meta.truncated = true;
            meta.truncated_reason = Some(format!("write_failed:{error}"));
            return meta;
        }
        written_bytes = written_bytes.saturating_add(write_len);
    }
    if written_bytes == 0 {
        if let Some(active_path) = active_path.as_deref() {
            let _ = fs::remove_file(active_path);
        }
        return meta;
    }
    let result = match writer {
        Ok(DirectWriter::Buffer(buffer)) => {
            let mut file = match create_plain_streaming_raw_file(&path) {
                Ok(file) => file,
                Err(error) => {
                    return RawPayloadMeta {
                        path: None,
                        size_bytes: meta.size_bytes,
                        truncated: true,
                        truncated_reason: Some(format!("write_failed:{error}")),
                        write_fence: None,
                    };
                }
            };
            active_path = Some(path.clone());
            file.write_all(&buffer).and_then(|()| file.flush())
        }
        Ok(DirectWriter::Plain(mut file)) => file.flush(),
        Ok(DirectWriter::Gzip(encoder)) => encoder.finish().and_then(|mut file| file.flush()),
        Ok(DirectWriter::Zstd(encoder)) => encoder.finish().and_then(|mut file| file.flush()),
        Err(error) => Err(error),
    };
    match result {
        Ok(()) => {
            if let Some(active_path) = active_path {
                meta.path = Some(active_path.to_string_lossy().to_string());
                meta.write_fence = Some(write_fence);
            }
        }
        Err(error) => {
            if let Some(active_path) = active_path.as_deref() {
                let _ = fs::remove_file(active_path);
            }
            meta.truncated = true;
            meta.truncated_reason = Some(format!("write_failed:{error}"));
        }
    }
    meta
}

pub(crate) async fn write_bounded_streaming_raw_payload_to_file(
    path: PathBuf,
    max_bytes: Option<usize>,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    rx: &mut mpsc::Receiver<Bytes>,
) -> RawPayloadMeta {
    write_streaming_raw_payload_to_file_from_receiver(
        path,
        max_bytes,
        immediate_gzip_bytes,
        codec,
        StreamingRawPayloadReceiver::Bounded(rx),
    )
    .await
}

enum StreamingRawPayloadReceiver<'a> {
    Bounded(&'a mut mpsc::Receiver<Bytes>),
    Unbounded(&'a mut mpsc::UnboundedReceiver<Bytes>),
    ReservedUnbounded(&'a mut mpsc::UnboundedReceiver<StreamingRawPayloadChunk>),
}

impl StreamingRawPayloadReceiver<'_> {
    async fn recv(&mut self) -> Option<StreamingRawPayloadChunk> {
        match self {
            Self::Bounded(receiver) => receiver
                .recv()
                .await
                .map(StreamingRawPayloadChunk::unreserved),
            Self::Unbounded(receiver) => receiver
                .recv()
                .await
                .map(StreamingRawPayloadChunk::unreserved),
            Self::ReservedUnbounded(receiver) => receiver.recv().await,
        }
    }
}

async fn write_streaming_raw_payload_to_file_from_receiver(
    path: PathBuf,
    max_bytes: Option<usize>,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    mut rx: StreamingRawPayloadReceiver<'_>,
) -> RawPayloadMeta {
    let mut meta = RawPayloadMeta::default();
    let gzip_path = raw_payload_gzip_path(&path);
    let zstd_path = raw_payload_zstd_path(&path);
    let write_fence = match run_blocking_raw_writer_io({
        let write_path = path.clone();
        move || crate::maintenance::acquire_retention_raw_write_fence(&write_path)
    })
    .await
    {
        Ok(write_fence) => write_fence,
        Err(error) => {
            meta.truncated = true;
            meta.truncated_reason = Some(format!("write_failed:{error}"));
            return meta;
        }
    };
    let mut writer = StreamingRawPayloadWriterState::Buffer(Vec::new());
    let mut written_bytes = 0usize;
    while let Some(bytes) = rx.recv().await {
        if bytes.is_empty() {
            continue;
        }
        meta.size_bytes = meta.size_bytes.saturating_add(bytes.len() as i64);

        let write_len = if let Some(limit) = max_bytes {
            let remaining = limit.saturating_sub(written_bytes);
            if remaining == 0 {
                meta.truncated = true;
                meta.truncated_reason
                    .get_or_insert_with(|| "max_bytes_exceeded".to_string());
                continue;
            }
            let write_len = remaining.min(bytes.len());
            if write_len < bytes.len() {
                meta.truncated = true;
                meta.truncated_reason
                    .get_or_insert_with(|| "max_bytes_exceeded".to_string());
            }
            write_len
        } else {
            bytes.len()
        };

        if write_len == 0 {
            continue;
        }

        let current_writer = std::mem::replace(
            &mut writer,
            StreamingRawPayloadWriterState::Buffer(Vec::new()),
        );
        let mut failed_path: Option<PathBuf> = None;
        let result = match current_writer {
            StreamingRawPayloadWriterState::Buffer(mut buffer) => {
                buffer.extend_from_slice(&bytes[..write_len]);
                match immediate_gzip_bytes {
                    Some(threshold)
                        if buffer.len() >= threshold && codec == RawCompressionCodec::Gzip =>
                    {
                        let write_path = gzip_path.clone();
                        match run_blocking_raw_writer_io(move || {
                            let mut encoder = create_gzip_streaming_raw_encoder(&write_path)?;
                            encoder.write_all(&buffer)?;
                            Ok(encoder)
                        })
                        .await
                        {
                            Ok(encoder) => {
                                meta.path = Some(gzip_path.to_string_lossy().to_string());
                                writer = StreamingRawPayloadWriterState::Gzip {
                                    path: gzip_path.clone(),
                                    encoder,
                                };
                                Ok(())
                            }
                            Err(err) => {
                                failed_path = Some(gzip_path.clone());
                                Err(err)
                            }
                        }
                    }
                    Some(threshold)
                        if buffer.len() >= threshold && codec == RawCompressionCodec::Zstd =>
                    {
                        let write_path = zstd_path.clone();
                        match run_blocking_raw_writer_io(move || {
                            let mut encoder = create_zstd_streaming_raw_encoder(&write_path)?;
                            encoder.write_all(&buffer)?;
                            Ok(encoder)
                        })
                        .await
                        {
                            Ok(encoder) => {
                                meta.path = Some(zstd_path.to_string_lossy().to_string());
                                writer = StreamingRawPayloadWriterState::Zstd {
                                    path: zstd_path.clone(),
                                    encoder,
                                };
                                Ok(())
                            }
                            Err(err) => {
                                failed_path = Some(zstd_path.clone());
                                Err(err)
                            }
                        }
                    }
                    Some(_) => {
                        writer = StreamingRawPayloadWriterState::Buffer(buffer);
                        Ok(())
                    }
                    None => {
                        let write_path = path.clone();
                        match run_blocking_raw_writer_io(move || {
                            let mut file = create_plain_streaming_raw_file(&write_path)?;
                            file.write_all(&buffer)?;
                            Ok(file)
                        })
                        .await
                        {
                            Ok(file) => {
                                meta.path = Some(path.to_string_lossy().to_string());
                                writer = StreamingRawPayloadWriterState::Plain {
                                    path: path.clone(),
                                    file,
                                };
                                Ok(())
                            }
                            Err(err) => {
                                failed_path = Some(path.clone());
                                Err(err)
                            }
                        }
                    }
                }
            }
            StreamingRawPayloadWriterState::Plain { path, mut file } => {
                let chunk = bytes.slice(..write_len);
                match run_blocking_raw_writer_io(move || {
                    file.write_all(chunk.as_ref())?;
                    Ok(file)
                })
                .await
                {
                    Ok(file) => {
                        writer = StreamingRawPayloadWriterState::Plain { path, file };
                        Ok(())
                    }
                    Err(err) => {
                        failed_path = Some(path.clone());
                        Err(err)
                    }
                }
            }
            StreamingRawPayloadWriterState::Gzip { path, mut encoder } => {
                let chunk = bytes.slice(..write_len);
                match run_blocking_raw_writer_io(move || {
                    encoder.write_all(chunk.as_ref())?;
                    Ok(encoder)
                })
                .await
                {
                    Ok(encoder) => {
                        writer = StreamingRawPayloadWriterState::Gzip { path, encoder };
                        Ok(())
                    }
                    Err(err) => {
                        failed_path = Some(path.clone());
                        Err(err)
                    }
                }
            }
            StreamingRawPayloadWriterState::Zstd { path, mut encoder } => {
                let chunk = bytes.slice(..write_len);
                match run_blocking_raw_writer_io(move || {
                    encoder.write_all(chunk.as_ref())?;
                    Ok(encoder)
                })
                .await
                {
                    Ok(encoder) => {
                        writer = StreamingRawPayloadWriterState::Zstd { path, encoder };
                        Ok(())
                    }
                    Err(err) => {
                        failed_path = Some(path.clone());
                        Err(err)
                    }
                }
            }
        };

        if let Err(err) = result {
            meta.truncated = true;
            meta.truncated_reason = Some(format!("write_failed:{err}"));
            if let Some(current_path) = failed_path.as_deref() {
                let _ = fs::remove_file(current_path);
            }
            meta.path = None;
            return meta;
        }
        written_bytes = written_bytes.saturating_add(write_len);
    }

    let final_path = writer
        .current_path()
        .map(|value| value.to_path_buf())
        .or_else(|| meta.path.as_ref().map(PathBuf::from));
    let finish_result = match writer {
        StreamingRawPayloadWriterState::Buffer(buffer) => {
            if buffer.is_empty() {
                Ok(())
            } else if codec == RawCompressionCodec::Zstd {
                let final_zstd_path = zstd_path.clone();
                let write_result = run_blocking_raw_writer_io(move || {
                    let mut encoder = create_zstd_streaming_raw_encoder(&final_zstd_path)?;
                    encoder.write_all(&buffer)?;
                    let mut writer = encoder.finish()?;
                    writer.flush()
                })
                .await;
                if write_result.is_ok() {
                    meta.path = Some(zstd_path.to_string_lossy().to_string());
                }
                write_result
            } else {
                let final_plain_path = path.clone();
                let write_result = run_blocking_raw_writer_io(move || {
                    let mut file = create_plain_streaming_raw_file(&final_plain_path)?;
                    file.write_all(&buffer)?;
                    file.flush()
                })
                .await;
                if write_result.is_ok() {
                    meta.path = Some(path.to_string_lossy().to_string());
                }
                write_result
            }
        }
        StreamingRawPayloadWriterState::Plain { path, mut file } => {
            let flush_result = run_blocking_raw_writer_io(move || file.flush()).await;
            if flush_result.is_ok() {
                meta.path = Some(path.to_string_lossy().to_string());
            }
            flush_result
        }
        StreamingRawPayloadWriterState::Gzip { path, encoder } => {
            let finish_result = run_blocking_raw_writer_io(move || {
                let mut writer = encoder.finish()?;
                writer.flush()
            })
            .await;
            if finish_result.is_ok() {
                meta.path = Some(path.to_string_lossy().to_string());
            }
            finish_result
        }
        StreamingRawPayloadWriterState::Zstd { path, encoder } => {
            let finish_result = run_blocking_raw_writer_io(move || {
                let mut writer = encoder.finish()?;
                writer.flush()
            })
            .await;
            if finish_result.is_ok() {
                meta.path = Some(path.to_string_lossy().to_string());
            }
            finish_result
        }
    };

    if let Err(err) = finish_result {
        meta.truncated = true;
        meta.truncated_reason = Some(format!("write_failed:{err}"));
        if let Some(path) = final_path.as_deref() {
            let _ = fs::remove_file(path);
        }
        meta.path = None;
    } else if meta.path.is_some() {
        meta.write_fence = Some(write_fence);
    }

    meta
}

#[cfg(test)]
mod streaming_raw_payload_tests {
    use super::*;

    #[test]
    fn bounded_streaming_ingress_marks_capture_unavailable_without_blocking() {
        let (tx, _rx) = std::sync::mpsc::sync_channel(1);
        let mut writer = AsyncStreamingRawPayloadWriter {
            tx: Some(tx),
            meta_rx: None,
            observed_size_bytes: 0,
            local_truncated_reason: None,
            local_truncated: false,
            spool: None,
            reservation: None,
            reservation_shared: None,
            reserved_payload_bytes: 0,
            reservation_circuit: None,
        };

        writer.append(b"first");
        writer.append(b"second");

        assert!(writer.local_truncated);
        assert_eq!(
            writer.local_truncated_reason.as_deref(),
            Some("capture_unavailable:ingress_queue_full")
        );
    }
}
