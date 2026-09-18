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
    let _ = RAW_ASYNC_WRITER_QUEUED_BYTES.fetch_update(
        std::sync::atomic::Ordering::Relaxed,
        std::sync::atomic::Ordering::Relaxed,
        |queued| Some(queued.saturating_sub(bytes)),
    );
}

enum DirectRawPayloadWriter {
    Buffer(Vec<u8>),
    Plain(fs::File),
    Gzip(flate2::write::GzEncoder<io::BufWriter<fs::File>>),
    Zstd(zstd::stream::write::Encoder<'static, io::BufWriter<fs::File>>),
}

fn write_direct_streaming_raw_payload_to_file_inner(
    path: PathBuf,
    max_bytes: Option<usize>,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    rx: std::sync::mpsc::Receiver<impl RawPayloadChunk>,
) -> RawPayloadMeta {
    let gzip_path = raw_payload_gzip_path(&path);
    let zstd_path = raw_payload_zstd_path(&path);
    let mut writer =
        initialize_direct_raw_payload_writer(&path, &zstd_path, codec, immediate_gzip_bytes);
    let mut meta = RawPayloadMeta::default();
    let mut written_bytes = 0usize;
    let mut active_path =
        initial_direct_raw_payload_path(&path, &zstd_path, codec, immediate_gzip_bytes);
    while let Ok(chunk) = rx.recv() {
        let bytes = chunk.into_bytes();
        meta.size_bytes = meta.size_bytes.saturating_add(bytes.len() as i64);
        let write_len = bounded_raw_payload_write_len(max_bytes, written_bytes, bytes.len());
        if write_len < bytes.len() {
            meta.truncated = true;
            meta.truncated_reason
                .get_or_insert_with(|| "max_bytes_exceeded".to_string());
        }
        if write_len == 0 {
            continue;
        }
        if let Err(error) = append_direct_raw_payload_chunk(
            &mut writer,
            &bytes[..write_len],
            immediate_gzip_bytes,
            &gzip_path,
            &mut active_path,
        ) {
            if let Some(active_path) = active_path.as_deref() {
                let _ = fs::remove_file(active_path);
            }
            meta.truncated = true;
            meta.truncated_reason = Some(format!("write_failed:{error}"));
            return meta;
        }
        written_bytes = written_bytes.saturating_add(write_len);
    }
    finish_direct_raw_payload_write(meta, writer, written_bytes, &path, active_path)
}

fn initialize_direct_raw_payload_writer(
    path: &Path,
    zstd_path: &Path,
    codec: RawCompressionCodec,
    immediate_gzip_bytes: Option<usize>,
) -> io::Result<DirectRawPayloadWriter> {
    match codec {
        // Gzip retains the existing hot-plaintext threshold. Zstd starts at the first byte.
        RawCompressionCodec::Gzip if immediate_gzip_bytes.is_some() => {
            Ok(DirectRawPayloadWriter::Buffer(Vec::new()))
        }
        RawCompressionCodec::Gzip | RawCompressionCodec::None => {
            create_plain_streaming_raw_file(path).map(DirectRawPayloadWriter::Plain)
        }
        RawCompressionCodec::Zstd => {
            create_zstd_streaming_raw_encoder(zstd_path).map(DirectRawPayloadWriter::Zstd)
        }
    }
}

fn initial_direct_raw_payload_path(
    path: &Path,
    zstd_path: &Path,
    codec: RawCompressionCodec,
    immediate_gzip_bytes: Option<usize>,
) -> Option<PathBuf> {
    match codec {
        RawCompressionCodec::Zstd => Some(zstd_path.to_path_buf()),
        RawCompressionCodec::Gzip if immediate_gzip_bytes.is_some() => None,
        RawCompressionCodec::Gzip | RawCompressionCodec::None => Some(path.to_path_buf()),
    }
}

fn bounded_raw_payload_write_len(
    max_bytes: Option<usize>,
    written_bytes: usize,
    chunk_len: usize,
) -> usize {
    max_bytes
        .map(|limit| limit.saturating_sub(written_bytes).min(chunk_len))
        .unwrap_or(chunk_len)
}

fn append_direct_raw_payload_chunk(
    writer: &mut io::Result<DirectRawPayloadWriter>,
    bytes: &[u8],
    immediate_gzip_bytes: Option<usize>,
    gzip_path: &Path,
    active_path: &mut Option<PathBuf>,
) -> io::Result<()> {
    let current_writer = std::mem::replace(writer, Ok(DirectRawPayloadWriter::Buffer(Vec::new())));
    let next_writer = match current_writer? {
        DirectRawPayloadWriter::Buffer(mut buffer) => {
            buffer.extend_from_slice(bytes);
            if buffer.len() < immediate_gzip_bytes.expect("buffer only used for gzip threshold") {
                Ok(DirectRawPayloadWriter::Buffer(buffer))
            } else {
                let mut encoder = create_gzip_streaming_raw_encoder(gzip_path)?;
                encoder.write_all(&buffer)?;
                *active_path = Some(gzip_path.to_path_buf());
                Ok(DirectRawPayloadWriter::Gzip(encoder))
            }
        }
        DirectRawPayloadWriter::Plain(mut file) => {
            file.write_all(bytes)?;
            Ok(DirectRawPayloadWriter::Plain(file))
        }
        DirectRawPayloadWriter::Gzip(mut encoder) => {
            encoder.write_all(bytes)?;
            Ok(DirectRawPayloadWriter::Gzip(encoder))
        }
        DirectRawPayloadWriter::Zstd(mut encoder) => {
            encoder.write_all(bytes)?;
            Ok(DirectRawPayloadWriter::Zstd(encoder))
        }
    };
    *writer = next_writer;
    Ok(())
}

fn finish_direct_raw_payload_write(
    mut meta: RawPayloadMeta,
    writer: io::Result<DirectRawPayloadWriter>,
    written_bytes: usize,
    path: &Path,
    mut active_path: Option<PathBuf>,
) -> RawPayloadMeta {
    if written_bytes == 0 {
        if let Some(active_path) = active_path.as_deref() {
            let _ = fs::remove_file(active_path);
        }
        return meta;
    }
    let result = finish_direct_raw_payload_writer(writer, path, &mut active_path);
    match result {
        Ok(()) => meta.path = active_path.map(|value| value.to_string_lossy().to_string()),
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

fn finish_direct_raw_payload_writer(
    writer: io::Result<DirectRawPayloadWriter>,
    path: &Path,
    active_path: &mut Option<PathBuf>,
) -> io::Result<()> {
    match writer? {
        DirectRawPayloadWriter::Buffer(buffer) => {
            let mut file = create_plain_streaming_raw_file(path)?;
            *active_path = Some(path.to_path_buf());
            file.write_all(&buffer).and_then(|()| file.flush())
        }
        DirectRawPayloadWriter::Plain(mut file) => file.flush(),
        DirectRawPayloadWriter::Gzip(encoder) => encoder.finish().and_then(|mut file| file.flush()),
        DirectRawPayloadWriter::Zstd(encoder) => encoder.finish().and_then(|mut file| file.flush()),
    }
}

async fn write_bounded_streaming_raw_payload_to_file(
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

struct StreamingRawPayloadWriteState {
    path: PathBuf,
    max_bytes: Option<usize>,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    gzip_path: PathBuf,
    zstd_path: PathBuf,
    writer: StreamingRawPayloadWriterState,
    meta: RawPayloadMeta,
    written_bytes: usize,
}

impl StreamingRawPayloadWriteState {
    fn new(
        path: PathBuf,
        max_bytes: Option<usize>,
        immediate_gzip_bytes: Option<usize>,
        codec: RawCompressionCodec,
    ) -> Self {
        Self {
            gzip_path: raw_payload_gzip_path(&path),
            zstd_path: raw_payload_zstd_path(&path),
            path,
            max_bytes,
            immediate_gzip_bytes,
            codec,
            writer: StreamingRawPayloadWriterState::Buffer(Vec::new()),
            meta: RawPayloadMeta::default(),
            written_bytes: 0,
        }
    }
}

struct RawPayloadWriteFailure {
    error: io::Error,
    path: Option<PathBuf>,
}

async fn write_streaming_buffer_chunk(
    buffer: Vec<u8>,
    chunk: Bytes,
    immediate_gzip_bytes: Option<usize>,
    codec: RawCompressionCodec,
    path: &Path,
    gzip_path: &Path,
    zstd_path: &Path,
) -> Result<StreamingRawPayloadWriterState, RawPayloadWriteFailure> {
    let mut buffer = buffer;
    buffer.extend_from_slice(chunk.as_ref());
    match immediate_gzip_bytes {
        Some(threshold) if buffer.len() >= threshold && codec == RawCompressionCodec::Gzip => {
            let write_path = gzip_path.to_path_buf();
            match run_blocking_raw_writer_io(move || {
                let mut encoder = create_gzip_streaming_raw_encoder(&write_path)?;
                encoder.write_all(&buffer)?;
                Ok(encoder)
            })
            .await
            {
                Ok(encoder) => Ok(StreamingRawPayloadWriterState::Gzip {
                    path: gzip_path.to_path_buf(),
                    encoder,
                }),
                Err(error) => Err(RawPayloadWriteFailure {
                    error,
                    path: Some(gzip_path.to_path_buf()),
                }),
            }
        }
        Some(threshold) if buffer.len() >= threshold && codec == RawCompressionCodec::Zstd => {
            let write_path = zstd_path.to_path_buf();
            match run_blocking_raw_writer_io(move || {
                let mut encoder = create_zstd_streaming_raw_encoder(&write_path)?;
                encoder.write_all(&buffer)?;
                Ok(encoder)
            })
            .await
            {
                Ok(encoder) => Ok(StreamingRawPayloadWriterState::Zstd {
                    path: zstd_path.to_path_buf(),
                    encoder,
                }),
                Err(error) => Err(RawPayloadWriteFailure {
                    error,
                    path: Some(zstd_path.to_path_buf()),
                }),
            }
        }
        Some(_) => Ok(StreamingRawPayloadWriterState::Buffer(buffer)),
        None => {
            let write_path = path.to_path_buf();
            match run_blocking_raw_writer_io(move || {
                let mut file = create_plain_streaming_raw_file(&write_path)?;
                file.write_all(&buffer)?;
                Ok(file)
            })
            .await
            {
                Ok(file) => Ok(StreamingRawPayloadWriterState::Plain {
                    path: path.to_path_buf(),
                    file,
                }),
                Err(error) => Err(RawPayloadWriteFailure {
                    error,
                    path: Some(path.to_path_buf()),
                }),
            }
        }
    }
}

async fn write_streaming_active_chunk(
    writer: StreamingRawPayloadWriterState,
    chunk: Bytes,
) -> Result<StreamingRawPayloadWriterState, RawPayloadWriteFailure> {
    match writer {
        StreamingRawPayloadWriterState::Plain { path, mut file } => {
            let failed_path = path.clone();
            match run_blocking_raw_writer_io(move || {
                file.write_all(chunk.as_ref())?;
                Ok(file)
            })
            .await
            {
                Ok(file) => Ok(StreamingRawPayloadWriterState::Plain { path, file }),
                Err(error) => Err(RawPayloadWriteFailure {
                    error,
                    path: Some(failed_path),
                }),
            }
        }
        StreamingRawPayloadWriterState::Gzip { path, mut encoder } => {
            let failed_path = path.clone();
            match run_blocking_raw_writer_io(move || {
                encoder.write_all(chunk.as_ref())?;
                Ok(encoder)
            })
            .await
            {
                Ok(encoder) => Ok(StreamingRawPayloadWriterState::Gzip { path, encoder }),
                Err(error) => Err(RawPayloadWriteFailure {
                    error,
                    path: Some(failed_path),
                }),
            }
        }
        StreamingRawPayloadWriterState::Zstd { path, mut encoder } => {
            let failed_path = path.clone();
            match run_blocking_raw_writer_io(move || {
                encoder.write_all(chunk.as_ref())?;
                Ok(encoder)
            })
            .await
            {
                Ok(encoder) => Ok(StreamingRawPayloadWriterState::Zstd { path, encoder }),
                Err(error) => Err(RawPayloadWriteFailure {
                    error,
                    path: Some(failed_path),
                }),
            }
        }
        StreamingRawPayloadWriterState::Buffer(_) => unreachable!("buffer handled separately"),
    }
}

async fn write_streaming_raw_payload_chunk(
    state: &mut StreamingRawPayloadWriteState,
    bytes: StreamingRawPayloadChunk,
) -> Result<(), RawPayloadWriteFailure> {
    if bytes.is_empty() {
        return Ok(());
    }
    state.meta.size_bytes = state.meta.size_bytes.saturating_add(bytes.len() as i64);
    let write_len = match state.max_bytes {
        Some(limit) => {
            let remaining = limit.saturating_sub(state.written_bytes);
            if remaining == 0 {
                state.meta.truncated = true;
                state
                    .meta
                    .truncated_reason
                    .get_or_insert_with(|| "max_bytes_exceeded".to_string());
                return Ok(());
            }
            let write_len = remaining.min(bytes.len());
            if write_len < bytes.len() {
                state.meta.truncated = true;
                state
                    .meta
                    .truncated_reason
                    .get_or_insert_with(|| "max_bytes_exceeded".to_string());
            }
            write_len
        }
        None => bytes.len(),
    };
    if write_len == 0 {
        return Ok(());
    }
    let chunk = bytes.slice(..write_len);
    let current = std::mem::replace(
        &mut state.writer,
        StreamingRawPayloadWriterState::Buffer(Vec::new()),
    );
    let next = match current {
        StreamingRawPayloadWriterState::Buffer(buffer) => {
            write_streaming_buffer_chunk(
                buffer,
                chunk,
                state.immediate_gzip_bytes,
                state.codec,
                &state.path,
                &state.gzip_path,
                &state.zstd_path,
            )
            .await
        }
        writer => write_streaming_active_chunk(writer, chunk).await,
    }?;
    state.meta.path = next
        .current_path()
        .map(|path| path.to_string_lossy().to_string());
    state.writer = next;
    state.written_bytes = state.written_bytes.saturating_add(write_len);
    Ok(())
}

async fn finish_streaming_raw_payload_writer(
    mut state: StreamingRawPayloadWriteState,
) -> RawPayloadMeta {
    let final_path = state
        .writer
        .current_path()
        .map(Path::to_path_buf)
        .or_else(|| state.meta.path.as_ref().map(PathBuf::from));
    let finish_result = match state.writer {
        StreamingRawPayloadWriterState::Buffer(buffer) if buffer.is_empty() => Ok(()),
        StreamingRawPayloadWriterState::Buffer(buffer)
            if state.codec == RawCompressionCodec::Zstd =>
        {
            let path = state.zstd_path.clone();
            let result = run_blocking_raw_writer_io(move || {
                let mut encoder = create_zstd_streaming_raw_encoder(&path)?;
                encoder.write_all(&buffer)?;
                let mut file = encoder.finish()?;
                file.flush()
            })
            .await;
            if result.is_ok() {
                state.meta.path = Some(state.zstd_path.to_string_lossy().to_string());
            }
            result
        }
        StreamingRawPayloadWriterState::Buffer(buffer) => {
            let path = state.path.clone();
            let result = run_blocking_raw_writer_io(move || {
                let mut file = create_plain_streaming_raw_file(&path)?;
                file.write_all(&buffer)?;
                file.flush()
            })
            .await;
            if result.is_ok() {
                state.meta.path = Some(state.path.to_string_lossy().to_string());
            }
            result
        }
        StreamingRawPayloadWriterState::Plain { path, mut file } => {
            let result = run_blocking_raw_writer_io(move || file.flush()).await;
            if result.is_ok() {
                state.meta.path = Some(path.to_string_lossy().to_string());
            }
            result
        }
        StreamingRawPayloadWriterState::Gzip { path, encoder } => {
            let result = run_blocking_raw_writer_io(move || {
                let mut file = encoder.finish()?;
                file.flush()
            })
            .await;
            if result.is_ok() {
                state.meta.path = Some(path.to_string_lossy().to_string());
            }
            result
        }
        StreamingRawPayloadWriterState::Zstd { path, encoder } => {
            let result = run_blocking_raw_writer_io(move || {
                let mut file = encoder.finish()?;
                file.flush()
            })
            .await;
            if result.is_ok() {
                state.meta.path = Some(path.to_string_lossy().to_string());
            }
            result
        }
    };
    if let Err(error) = finish_result {
        state.meta.truncated = true;
        state.meta.truncated_reason = Some(format!("write_failed:{error}"));
        if let Some(path) = final_path.as_deref() {
            let _ = fs::remove_file(path);
        }
        state.meta.path = None;
    }
    state.meta
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
    let mut state =
        StreamingRawPayloadWriteState::new(path, max_bytes, immediate_gzip_bytes, codec);
    while let Some(bytes) = rx.recv().await {
        if let Err(failure) = write_streaming_raw_payload_chunk(&mut state, bytes).await {
            state.meta.truncated = true;
            state.meta.truncated_reason = Some(format!("write_failed:{}", failure.error));
            if let Some(path) = failure.path.as_deref() {
                let _ = fs::remove_file(path);
            }
            state.meta.path = None;
            return state.meta;
        }
    }
    finish_streaming_raw_payload_writer(state).await
}
