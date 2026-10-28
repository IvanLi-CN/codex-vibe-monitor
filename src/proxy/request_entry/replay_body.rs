use super::*;

pub(crate) const ENV_PROXY_REQUEST_SEMANTIC_PIPELINE_MODE: &str =
    "PROXY_REQUEST_SEMANTIC_PIPELINE_MODE";

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum RequestSemanticPipelineMode {
    Projection,
    Legacy,
}

impl RequestSemanticPipelineMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Projection => "projection",
            Self::Legacy => "legacy",
        }
    }
}

pub(crate) fn request_semantic_pipeline_mode() -> RequestSemanticPipelineMode {
    match std::env::var(ENV_PROXY_REQUEST_SEMANTIC_PIPELINE_MODE) {
        Ok(value) if value.trim().eq_ignore_ascii_case("legacy") => {
            RequestSemanticPipelineMode::Legacy
        }
        _ => RequestSemanticPipelineMode::Projection,
    }
}

#[derive(Debug)]
pub(crate) struct PoolReplayTempFile {
    pub(crate) path: PathBuf,
}

impl Drop for PoolReplayTempFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[derive(Debug, Clone)]
pub(crate) enum PoolReplayBodySnapshot {
    Empty,
    Memory(Bytes),
    File {
        temp_file: Arc<PoolReplayTempFile>,
        size: usize,
    },
}

#[derive(Debug, Clone)]
pub(crate) enum PoolReplayBodyStatus {
    Reading,
    Complete(PoolReplayBodySnapshot),
    ReadError(RequestBodyReadError),
    InternalError(String),
    Incomplete,
}

pub(crate) struct PoolReplayBodyBuffer {
    proxy_request_id: u64,
    len: usize,
    memory: Vec<u8>,
    file: Option<(Arc<PoolReplayTempFile>, tokio::fs::File)>,
}

pub(crate) const OAUTH_RESPONSES_MAX_REWRITE_BODY_BYTES: usize = 8 * 1024 * 1024;
pub(crate) static NEXT_POOL_REPLAY_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(1);

impl PoolReplayBodyBuffer {
    pub(crate) fn new(proxy_request_id: u64) -> Self {
        Self {
            proxy_request_id,
            len: 0,
            memory: Vec::new(),
            file: None,
        }
    }

    pub(crate) async fn append(&mut self, chunk: &[u8]) -> io::Result<()> {
        self.len = self.len.saturating_add(chunk.len());
        if let Some((_, file)) = self.file.as_mut() {
            file.write_all(chunk).await?;
            return Ok(());
        }

        if self.memory.len().saturating_add(chunk.len())
            <= POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES
        {
            self.memory.extend_from_slice(chunk);
            return Ok(());
        }

        let temp_file = Arc::new(PoolReplayTempFile {
            path: build_pool_replay_temp_path(self.proxy_request_id),
        });
        let mut file = tokio::fs::File::create(&temp_file.path).await?;
        if !self.memory.is_empty() {
            file.write_all(&self.memory).await?;
            self.memory.clear();
        }
        file.write_all(chunk).await?;
        self.file = Some((temp_file, file));
        Ok(())
    }

    pub(crate) async fn finish(mut self) -> io::Result<PoolReplayBodySnapshot> {
        if let Some((temp_file, mut file)) = self.file.take() {
            file.flush().await?;
            return Ok(PoolReplayBodySnapshot::File {
                temp_file,
                size: self.len,
            });
        }

        if self.memory.is_empty() {
            Ok(PoolReplayBodySnapshot::Empty)
        } else {
            Ok(PoolReplayBodySnapshot::Memory(Bytes::from(self.memory)))
        }
    }
}

pub(crate) async fn pool_replay_snapshot_from_bytes(
    proxy_request_id: u64,
    bytes: Bytes,
) -> io::Result<PoolReplayBodySnapshot> {
    pool_replay_snapshot_from_bytes_with_memory_threshold(
        proxy_request_id,
        bytes,
        POOL_REQUEST_REPLAY_MEMORY_THRESHOLD_BYTES,
    )
    .await
}

pub(crate) async fn pool_replay_snapshot_from_bytes_with_memory_threshold(
    proxy_request_id: u64,
    bytes: Bytes,
    memory_threshold_bytes: usize,
) -> io::Result<PoolReplayBodySnapshot> {
    if bytes.is_empty() {
        return Ok(PoolReplayBodySnapshot::Empty);
    }
    if bytes.len() <= memory_threshold_bytes {
        return Ok(PoolReplayBodySnapshot::Memory(bytes));
    }

    let temp_file = Arc::new(PoolReplayTempFile {
        path: build_pool_replay_temp_path(proxy_request_id),
    });
    match tokio::fs::File::create(&temp_file.path).await {
        Ok(mut file) => {
            if let Err(err) = file.write_all(&bytes).await {
                warn!(
                    proxy_request_id,
                    bytes = bytes.len(),
                    error = %err,
                    "failed to write large replay snapshot"
                );
                return Err(err);
            }
            if let Err(err) = file.flush().await {
                warn!(
                    proxy_request_id,
                    bytes = bytes.len(),
                    error = %err,
                    "failed to flush large replay snapshot"
                );
                return Err(err);
            }
            Ok(PoolReplayBodySnapshot::File {
                temp_file,
                size: bytes.len(),
            })
        }
        Err(err) => {
            warn!(
                proxy_request_id,
                bytes = bytes.len(),
                error = %err,
                "failed to create large replay snapshot"
            );
            Err(err)
        }
    }
}

pub(crate) async fn pool_replay_snapshot_from_vec(
    proxy_request_id: u64,
    bytes: Vec<u8>,
) -> io::Result<PoolReplayBodySnapshot> {
    pool_replay_snapshot_from_bytes(proxy_request_id, Bytes::from(bytes)).await
}

impl PoolReplayBodySnapshot {
    pub(crate) fn to_http_body(&self) -> Body {
        match self {
            Self::Empty => Body::from(Bytes::new()),
            Self::Memory(bytes) => Body::from(bytes.clone()),
            Self::File { temp_file, size } => {
                let temp_file = temp_file.clone();
                let expected_size = *size;
                let stream = stream::unfold(
                    Some((temp_file, expected_size, None::<tokio::fs::File>)),
                    |state| async move {
                        let (temp_file, remaining, file) = state?;
                        if remaining == 0 {
                            return None;
                        }
                        let mut file = match file {
                            Some(file) => file,
                            None => match tokio::fs::File::open(&temp_file.path).await {
                                Ok(file) => file,
                                Err(err) => {
                                    return Some((Err(io::Error::other(err.to_string())), None));
                                }
                            },
                        };
                        let mut buf = vec![0_u8; remaining.min(64 * 1024)];
                        match file.read(&mut buf).await {
                            Ok(0) => None,
                            Ok(read_len) => {
                                buf.truncate(read_len);
                                Some((
                                    Ok(Bytes::from(buf)),
                                    Some((temp_file, remaining - read_len, Some(file))),
                                ))
                            }
                            Err(err) => Some((Err(io::Error::other(err.to_string())), None)),
                        }
                    },
                );
                Body::from_stream(stream)
            }
        }
    }

    pub(crate) async fn to_bytes(&self) -> io::Result<Bytes> {
        match self {
            Self::Empty => Ok(Bytes::new()),
            Self::Memory(bytes) => Ok(bytes.clone()),
            Self::File { temp_file, .. } => tokio::fs::read(&temp_file.path).await.map(Bytes::from),
        }
    }

    pub(crate) async fn into_vec(self) -> io::Result<Vec<u8>> {
        match self {
            Self::Empty => Ok(Vec::new()),
            Self::Memory(bytes) => Ok(bytes.to_vec()),
            Self::File { temp_file, .. } => tokio::fs::read(&temp_file.path).await,
        }
    }

    pub(crate) async fn to_prefix_bytes(&self, limit: usize) -> io::Result<Bytes> {
        match self {
            Self::Empty => Ok(Bytes::new()),
            Self::Memory(bytes) => Ok(bytes.slice(..bytes.len().min(limit))),
            Self::File { temp_file, .. } => {
                let mut file = tokio::fs::File::open(&temp_file.path).await?;
                let mut buf = vec![0_u8; limit];
                let read_len = file.read(&mut buf).await?;
                buf.truncate(read_len);
                Ok(Bytes::from(buf))
            }
        }
    }

    pub(crate) async fn extract_request_stream_flag(
        &self,
        content_encoding: Option<&str>,
    ) -> Option<bool> {
        #[derive(serde::Deserialize)]
        struct StreamFlagProjection {
            #[serde(default)]
            stream: Option<bool>,
        }

        fn parse_stream_flag_from_bytes(bytes: &[u8]) -> Option<bool> {
            serde_json::from_slice::<StreamFlagProjection>(bytes)
                .ok()
                .and_then(|projection| projection.stream)
        }

        fn parse_stream_flag_from_reader<R: std::io::Read>(reader: R) -> Option<bool> {
            serde_json::from_reader::<R, StreamFlagProjection>(reader)
                .ok()
                .and_then(|projection| projection.stream)
        }

        match self {
            Self::Empty => None,
            Self::Memory(bytes) => {
                let (decoded, _) = decode_response_payload(bytes.as_ref(), content_encoding, true);
                parse_stream_flag_from_bytes(decoded.as_ref())
            }
            Self::File { temp_file, .. } => {
                let path = temp_file.path.clone();
                let content_encoding = content_encoding.map(str::to_string);
                tokio::task::spawn_blocking(move || {
                    let reader =
                        open_decoded_response_reader(&path, content_encoding.as_deref()).ok()?;
                    parse_stream_flag_from_reader(std::io::BufReader::new(reader))
                })
                .await
                .ok()
                .flatten()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedPoolRequestBody {
    pub(crate) snapshot: PoolReplayBodySnapshot,
    pub(crate) request_body_for_capture: Option<Bytes>,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) requested_image_intent: ImageIntent,
    pub(crate) requested_hosted_image_intent: ImageIntent,
    pub(crate) codex_imagegen_rewrite: Option<Value>,
    pub(crate) snapshot_is_decoded: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct PoolRequestBodyPreparationError {
    pub(crate) status: StatusCode,
    pub(crate) message: String,
}

impl PoolRequestBodyPreparationError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn bad_gateway(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_GATEWAY,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PoolRequestBodyCompressionMode {
    Identity,
    Passthrough,
    Recompressed,
}

impl PoolRequestBodyCompressionMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Passthrough => "passthrough",
            Self::Recompressed => "recompressed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RequestBodyContentEncoding {
    Identity,
    Gzip,
    Deflate { zlib_wrapper: bool },
    Zstd,
}

impl RequestBodyContentEncoding {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Gzip => "gzip",
            Self::Deflate { .. } => "deflate",
            Self::Zstd => "zstd",
        }
    }

    pub(crate) fn header_value(self) -> Option<&'static str> {
        match self {
            Self::Identity => None,
            Self::Gzip => Some("gzip"),
            Self::Deflate { .. } => Some("deflate"),
            Self::Zstd => Some("zstd"),
        }
    }

    pub(crate) fn algorithm(self) -> RequestCompressionAlgorithm {
        match self {
            Self::Identity => RequestCompressionAlgorithm::Identity,
            Self::Gzip => RequestCompressionAlgorithm::Gzip,
            Self::Deflate { .. } => RequestCompressionAlgorithm::Deflate,
            Self::Zstd => RequestCompressionAlgorithm::Zstd,
        }
    }
}

#[derive(Debug)]
pub(crate) struct PreparedPoolUpstreamRequestBody {
    pub(crate) body: Body,
    pub(crate) content_length: Option<usize>,
    pub(crate) content_encoding: RequestBodyContentEncoding,
    pub(crate) compression_mode: PoolRequestBodyCompressionMode,
    pub(crate) byte_observation: PreparedPoolUpstreamRequestBodyObservation,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ObservedByteCounter {
    inner: Arc<AtomicU64>,
}

impl ObservedByteCounter {
    pub(crate) fn add(&self, bytes: usize) {
        let Ok(bytes) = u64::try_from(bytes) else {
            return;
        };
        self.inner.fetch_add(bytes, Ordering::Relaxed);
    }

    pub(crate) fn load(&self) -> usize {
        usize::try_from(self.inner.load(Ordering::Relaxed)).unwrap_or(usize::MAX)
    }
}

#[derive(Debug, Clone)]
pub(crate) enum ObservedBodyBytes {
    Fixed(usize),
    Counter(ObservedByteCounter),
}

impl ObservedBodyBytes {
    pub(crate) fn load(&self) -> usize {
        match self {
            Self::Fixed(bytes) => *bytes,
            Self::Counter(counter) => counter.load(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedPoolUpstreamRequestBodyObservation {
    pub(crate) logical_body_bytes: ObservedBodyBytes,
    pub(crate) transmitted_body_bytes: ObservedByteCounter,
}

#[derive(Debug, Clone)]
pub(crate) struct RequestCompressionObservation {
    pub(crate) algorithm: String,
    pub(crate) mode: String,
    pub(crate) logical_body_bytes: usize,
    pub(crate) transmitted_body_bytes: usize,
}

#[derive(Debug)]
struct CountingAsyncRead<R> {
    inner: R,
    counter: ObservedByteCounter,
}

impl<R> CountingAsyncRead<R> {
    fn new(inner: R, counter: ObservedByteCounter) -> Self {
        Self { inner, counter }
    }
}

impl<R> AsyncRead for CountingAsyncRead<R>
where
    R: AsyncRead + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        let filled_before = buf.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let std::task::Poll::Ready(Ok(())) = &result {
            let filled_after = buf.filled().len();
            if filled_after > filled_before {
                self.counter.add(filled_after - filled_before);
            }
        }
        result
    }
}

pub(crate) fn http_visible_header_bytes_approx(headers: &HeaderMap) -> usize {
    headers
        .iter()
        .map(|(name, value)| name.as_str().len() + 2 + value.as_bytes().len() + 2)
        .sum()
}

pub(crate) fn counted_http_body_from_bytes(bytes: Bytes, counter: ObservedByteCounter) -> Body {
    if bytes.is_empty() {
        return Body::from(Bytes::new());
    }
    let stream = stream::once(async move {
        counter.add(bytes.len());
        Ok::<Bytes, io::Error>(bytes)
    });
    Body::from_stream(stream)
}

fn counted_http_body_from_reader<R>(reader: R, counter: ObservedByteCounter) -> Body
where
    R: AsyncRead + Send + 'static,
{
    let stream = ReaderStream::new(reader).map(move |chunk| {
        if let Ok(bytes) = &chunk {
            counter.add(bytes.len());
        }
        chunk
    });
    Body::from_stream(stream)
}

pub(crate) fn counted_http_body_from_snapshot(
    snapshot: &PoolReplayBodySnapshot,
    counter: ObservedByteCounter,
) -> Body {
    match snapshot {
        PoolReplayBodySnapshot::Empty => Body::from(Bytes::new()),
        PoolReplayBodySnapshot::Memory(bytes) => {
            counted_http_body_from_bytes(bytes.clone(), counter)
        }
        PoolReplayBodySnapshot::File { temp_file, size } => {
            let temp_file = temp_file.clone();
            let expected_size = *size;
            let stream = stream::unfold(
                Some((temp_file, expected_size, None::<tokio::fs::File>, counter)),
                |state| async move {
                    let (temp_file, remaining, file, counter) = state?;
                    if remaining == 0 {
                        return None;
                    }
                    let mut file = match file {
                        Some(file) => file,
                        None => match tokio::fs::File::open(&temp_file.path).await {
                            Ok(file) => file,
                            Err(err) => {
                                return Some((Err(io::Error::other(err.to_string())), None));
                            }
                        },
                    };
                    let mut buf = vec![0_u8; remaining.min(64 * 1024)];
                    match file.read(&mut buf).await {
                        Ok(0) => None,
                        Ok(read_len) => {
                            buf.truncate(read_len);
                            counter.add(read_len);
                            Some((
                                Ok(Bytes::from(buf)),
                                Some((temp_file, remaining - read_len, Some(file), counter)),
                            ))
                        }
                        Err(err) => Some((Err(io::Error::other(err.to_string())), None)),
                    }
                },
            );
            Body::from_stream(stream)
        }
    }
}

pub(crate) type BoxedPoolRequestReader = Pin<Box<dyn AsyncRead + Send>>;

pub(crate) fn request_compression_preset_to_async_level(
    preset: RequestCompressionLevelPreset,
) -> AsyncCompressionLevel {
    match preset {
        RequestCompressionLevelPreset::Fast => AsyncCompressionLevel::Fastest,
        RequestCompressionLevelPreset::Balanced => AsyncCompressionLevel::Default,
        RequestCompressionLevelPreset::Best => AsyncCompressionLevel::Best,
    }
}

pub(crate) fn resolve_request_body_content_encoding_from_prefix(
    prefix: Option<&[u8]>,
    content_encoding: Option<&str>,
) -> Result<RequestBodyContentEncoding, PoolRequestBodyPreparationError> {
    let encodings = parse_content_encodings(content_encoding);
    if encodings.is_empty() || encodings.iter().all(|encoding| encoding == "identity") {
        return Ok(RequestBodyContentEncoding::Identity);
    }
    if encodings.len() != 1 {
        return Err(PoolRequestBodyPreparationError::bad_request(format!(
            "unsupported request Content-Encoding chain: {}",
            encodings.join(", ")
        )));
    }

    match encodings[0].as_str() {
        "gzip" | "x-gzip" => Ok(RequestBodyContentEncoding::Gzip),
        "deflate" => Ok(RequestBodyContentEncoding::Deflate {
            zlib_wrapper: deflate_stream_uses_zlib_wrapper(prefix.unwrap_or_default()),
        }),
        "zstd" => Ok(RequestBodyContentEncoding::Zstd),
        other => Err(PoolRequestBodyPreparationError::bad_request(format!(
            "unsupported request Content-Encoding: {other}"
        ))),
    }
}

pub(crate) async fn resolve_request_body_content_encoding(
    snapshot: &PoolReplayBodySnapshot,
    content_encoding: Option<&str>,
) -> Result<RequestBodyContentEncoding, PoolRequestBodyPreparationError> {
    let prefix = if parse_content_encodings(content_encoding)
        .iter()
        .any(|encoding| encoding == "deflate")
    {
        Some(snapshot.to_prefix_bytes(2).await.map_err(|err| {
            PoolRequestBodyPreparationError::bad_gateway(format!(
                "failed to inspect deflate request body header: {err}"
            ))
        })?)
    } else {
        None
    };
    resolve_request_body_content_encoding_from_prefix(
        prefix.as_ref().map(Bytes::as_ref),
        content_encoding,
    )
}

pub(crate) fn observe_request_compression_from_bytes(
    bytes: &[u8],
    content_encoding: Option<&str>,
) -> Option<RequestCompressionObservation> {
    let needs_prefix = parse_content_encodings(content_encoding)
        .iter()
        .any(|encoding| encoding == "deflate");
    let prefix = needs_prefix.then(|| &bytes[..bytes.len().min(2)]);
    let encoding =
        resolve_request_body_content_encoding_from_prefix(prefix, content_encoding).ok()?;
    let logical_body_bytes = decode_request_payload_bytes(bytes, encoding).ok()?.len();
    Some(RequestCompressionObservation {
        algorithm: encoding.algorithm().as_str().to_string(),
        mode: if matches!(encoding, RequestBodyContentEncoding::Identity) {
            PoolRequestBodyCompressionMode::Identity
                .as_str()
                .to_string()
        } else {
            PoolRequestBodyCompressionMode::Passthrough
                .as_str()
                .to_string()
        },
        logical_body_bytes,
        transmitted_body_bytes: bytes.len(),
    })
}

async fn count_decoded_request_snapshot_bytes(
    snapshot: &PoolReplayBodySnapshot,
    encoding: RequestBodyContentEncoding,
) -> Result<usize, PoolRequestBodyPreparationError> {
    if matches!(encoding, RequestBodyContentEncoding::Identity) {
        return Ok(pool_request_snapshot_body_bytes(snapshot));
    }
    match snapshot {
        PoolReplayBodySnapshot::Empty => Ok(0),
        PoolReplayBodySnapshot::Memory(bytes) => {
            Ok(decode_request_payload_bytes(bytes, encoding)?.len())
        }
        PoolReplayBodySnapshot::File { .. } => {
            let raw_reader = open_pool_request_snapshot_reader(snapshot).await?;
            let mut decoded_reader = decode_pool_request_reader(raw_reader, encoding).await?;
            let mut total = 0usize;
            let mut buf = [0_u8; 64 * 1024];
            loop {
                let read_len = decoded_reader.read(&mut buf).await.map_err(|err| {
                    PoolRequestBodyPreparationError::bad_gateway(format!(
                        "failed to count decoded request body bytes: {err}"
                    ))
                })?;
                if read_len == 0 {
                    break;
                }
                total = total.saturating_add(read_len);
            }
            Ok(total)
        }
    }
}

pub(crate) fn decode_request_payload_bytes(
    bytes: &[u8],
    encoding: RequestBodyContentEncoding,
) -> Result<Bytes, PoolRequestBodyPreparationError> {
    match encoding {
        RequestBodyContentEncoding::Identity => Ok(Bytes::copy_from_slice(bytes)),
        RequestBodyContentEncoding::Gzip => {
            let mut decoder = GzDecoder::new(bytes);
            let mut decoded = Vec::new();
            decoder.read_to_end(&mut decoded).map_err(|err| {
                PoolRequestBodyPreparationError::bad_request(format!(
                    "failed to decode gzip request body: {err}"
                ))
            })?;
            Ok(Bytes::from(decoded))
        }
        RequestBodyContentEncoding::Deflate { zlib_wrapper } => {
            let mut decoded = Vec::new();
            if zlib_wrapper {
                let mut decoder = ZlibDecoder::new(bytes);
                decoder.read_to_end(&mut decoded).map_err(|err| {
                    PoolRequestBodyPreparationError::bad_request(format!(
                        "failed to decode deflate request body: {err}"
                    ))
                })?;
            } else {
                let mut decoder = DeflateDecoder::new(bytes);
                decoder.read_to_end(&mut decoded).map_err(|err| {
                    PoolRequestBodyPreparationError::bad_request(format!(
                        "failed to decode deflate request body: {err}"
                    ))
                })?;
            }
            Ok(Bytes::from(decoded))
        }
        RequestBodyContentEncoding::Zstd => {
            zstd::decode_all(bytes).map(Bytes::from).map_err(|err| {
                PoolRequestBodyPreparationError::bad_request(format!(
                    "failed to decode zstd request body: {err}"
                ))
            })
        }
    }
}

async fn open_pool_request_snapshot_reader(
    snapshot: &PoolReplayBodySnapshot,
) -> Result<BoxedPoolRequestReader, PoolRequestBodyPreparationError> {
    match snapshot {
        PoolReplayBodySnapshot::Empty => Ok(Box::pin(tokio::io::empty())),
        PoolReplayBodySnapshot::Memory(bytes) => {
            let bytes = bytes.clone();
            let stream = stream::once(async move { Ok::<Bytes, io::Error>(bytes) });
            Ok(Box::pin(StreamReader::new(stream)))
        }
        PoolReplayBodySnapshot::File { temp_file, .. } => {
            let file = tokio::fs::File::open(&temp_file.path)
                .await
                .map_err(|err| {
                    PoolRequestBodyPreparationError::bad_gateway(format!(
                        "failed to open request replay body: {err}"
                    ))
                })?;
            Ok(Box::pin(file))
        }
    }
}

pub(crate) async fn decode_pool_request_reader(
    reader: BoxedPoolRequestReader,
    encoding: RequestBodyContentEncoding,
) -> Result<BoxedPoolRequestReader, PoolRequestBodyPreparationError> {
    match encoding {
        RequestBodyContentEncoding::Identity => Ok(reader),
        RequestBodyContentEncoding::Gzip => Ok(Box::pin(AsyncGzipDecoder::new(
            tokio::io::BufReader::new(reader),
        ))),
        RequestBodyContentEncoding::Deflate { zlib_wrapper } => {
            let mut buffered = tokio::io::BufReader::new(reader);
            let _ = buffered.fill_buf().await.map_err(|err| {
                PoolRequestBodyPreparationError::bad_request(format!(
                    "failed to read deflate request body header: {err}"
                ))
            })?;
            if zlib_wrapper {
                Ok(Box::pin(AsyncZlibDecoder::new(buffered)))
            } else {
                Ok(Box::pin(AsyncDeflateDecoder::new(buffered)))
            }
        }
        RequestBodyContentEncoding::Zstd => Ok(Box::pin(AsyncZstdDecoder::new(
            tokio::io::BufReader::new(reader),
        ))),
    }
}

pub(crate) fn encode_pool_request_reader(
    reader: BoxedPoolRequestReader,
    encoding: RequestBodyContentEncoding,
    level: AsyncCompressionLevel,
) -> BoxedPoolRequestReader {
    let buffered = tokio::io::BufReader::new(reader);
    match encoding {
        RequestBodyContentEncoding::Identity => Box::pin(buffered),
        RequestBodyContentEncoding::Gzip => {
            Box::pin(AsyncGzipEncoder::with_quality(buffered, level))
        }
        RequestBodyContentEncoding::Deflate { zlib_wrapper } => {
            if zlib_wrapper {
                Box::pin(AsyncZlibEncoder::with_quality(buffered, level))
            } else {
                Box::pin(
                    async_compression::tokio::bufread::DeflateEncoder::with_quality(
                        buffered, level,
                    ),
                )
            }
        }
        RequestBodyContentEncoding::Zstd => {
            Box::pin(AsyncZstdEncoder::with_quality(buffered, level))
        }
    }
}

pub(crate) fn request_compression_algorithm_encoding(
    algorithm: RequestCompressionAlgorithm,
    downstream_encoding: RequestBodyContentEncoding,
) -> RequestBodyContentEncoding {
    match algorithm {
        RequestCompressionAlgorithm::Follow => downstream_encoding,
        RequestCompressionAlgorithm::Identity => RequestBodyContentEncoding::Identity,
        RequestCompressionAlgorithm::Gzip => RequestBodyContentEncoding::Gzip,
        RequestCompressionAlgorithm::Deflate => {
            RequestBodyContentEncoding::Deflate { zlib_wrapper: true }
        }
        RequestCompressionAlgorithm::Zstd => RequestBodyContentEncoding::Zstd,
    }
}

pub(crate) async fn build_pool_upstream_request_body(
    prepared: &PreparedPoolRequestBody,
    request_compression_algorithm: RequestCompressionAlgorithm,
    request_compression_level_preset: RequestCompressionLevelPreset,
    downstream_content_encoding: Option<&str>,
) -> Result<PreparedPoolUpstreamRequestBody, PoolRequestBodyPreparationError> {
    if matches!(prepared.snapshot, PoolReplayBodySnapshot::Empty) {
        let transmitted_body_bytes = ObservedByteCounter::default();
        return Ok(PreparedPoolUpstreamRequestBody {
            body: Body::from(Bytes::new()),
            content_length: Some(0),
            content_encoding: RequestBodyContentEncoding::Identity,
            compression_mode: PoolRequestBodyCompressionMode::Identity,
            byte_observation: PreparedPoolUpstreamRequestBodyObservation {
                logical_body_bytes: ObservedBodyBytes::Fixed(0),
                transmitted_body_bytes,
            },
        });
    }

    let downstream_encoding =
        resolve_request_body_content_encoding(&prepared.snapshot, downstream_content_encoding)
            .await?;
    let target_encoding = match request_compression_algorithm {
        RequestCompressionAlgorithm::Follow => downstream_encoding,
        RequestCompressionAlgorithm::Identity => RequestBodyContentEncoding::Identity,
        RequestCompressionAlgorithm::Gzip => RequestBodyContentEncoding::Gzip,
        RequestCompressionAlgorithm::Deflate => {
            RequestBodyContentEncoding::Deflate { zlib_wrapper: true }
        }
        RequestCompressionAlgorithm::Zstd => RequestBodyContentEncoding::Zstd,
    };

    if prepared.snapshot_is_decoded
        && matches!(target_encoding, RequestBodyContentEncoding::Identity)
    {
        let transmitted_body_bytes = ObservedByteCounter::default();
        return Ok(PreparedPoolUpstreamRequestBody {
            body: counted_http_body_from_snapshot(
                &prepared.snapshot,
                transmitted_body_bytes.clone(),
            ),
            content_length: Some(pool_request_snapshot_body_bytes(&prepared.snapshot)),
            content_encoding: RequestBodyContentEncoding::Identity,
            compression_mode: PoolRequestBodyCompressionMode::Identity,
            byte_observation: PreparedPoolUpstreamRequestBodyObservation {
                logical_body_bytes: ObservedBodyBytes::Fixed(pool_request_snapshot_body_bytes(
                    &prepared.snapshot,
                )),
                transmitted_body_bytes,
            },
        });
    }

    if !prepared.snapshot_is_decoded && target_encoding == downstream_encoding {
        let compression_mode = if matches!(target_encoding, RequestBodyContentEncoding::Identity) {
            PoolRequestBodyCompressionMode::Identity
        } else {
            PoolRequestBodyCompressionMode::Passthrough
        };
        let transmitted_body_bytes = ObservedByteCounter::default();
        let logical_body_bytes = if matches!(target_encoding, RequestBodyContentEncoding::Identity)
        {
            ObservedBodyBytes::Fixed(pool_request_snapshot_body_bytes(&prepared.snapshot))
        } else {
            ObservedBodyBytes::Fixed(
                count_decoded_request_snapshot_bytes(&prepared.snapshot, target_encoding).await?,
            )
        };
        return Ok(PreparedPoolUpstreamRequestBody {
            body: counted_http_body_from_snapshot(
                &prepared.snapshot,
                transmitted_body_bytes.clone(),
            ),
            content_length: Some(pool_request_snapshot_body_bytes(&prepared.snapshot)),
            content_encoding: target_encoding,
            compression_mode,
            byte_observation: PreparedPoolUpstreamRequestBodyObservation {
                logical_body_bytes,
                transmitted_body_bytes,
            },
        });
    }

    let raw_reader = open_pool_request_snapshot_reader(&prepared.snapshot).await?;
    let decoded_reader = if prepared.snapshot_is_decoded {
        raw_reader
    } else {
        decode_pool_request_reader(raw_reader, downstream_encoding).await?
    };

    if matches!(target_encoding, RequestBodyContentEncoding::Identity) {
        let transmitted_body_bytes = ObservedByteCounter::default();
        return Ok(PreparedPoolUpstreamRequestBody {
            body: counted_http_body_from_reader(decoded_reader, transmitted_body_bytes.clone()),
            content_length: None,
            content_encoding: RequestBodyContentEncoding::Identity,
            compression_mode: PoolRequestBodyCompressionMode::Identity,
            byte_observation: PreparedPoolUpstreamRequestBodyObservation {
                logical_body_bytes: if prepared.snapshot_is_decoded {
                    ObservedBodyBytes::Fixed(pool_request_snapshot_body_bytes(&prepared.snapshot))
                } else {
                    ObservedBodyBytes::Counter(transmitted_body_bytes.clone())
                },
                transmitted_body_bytes,
            },
        });
    }

    let level = request_compression_preset_to_async_level(request_compression_level_preset);
    let logical_body_bytes = if prepared.snapshot_is_decoded {
        ObservedBodyBytes::Fixed(pool_request_snapshot_body_bytes(&prepared.snapshot))
    } else {
        ObservedBodyBytes::Counter(ObservedByteCounter::default())
    };
    let decoded_reader = match &logical_body_bytes {
        ObservedBodyBytes::Fixed(_) => decoded_reader,
        ObservedBodyBytes::Counter(counter) => {
            Box::pin(CountingAsyncRead::new(decoded_reader, counter.clone()))
        }
    };
    let encoded_reader = encode_pool_request_reader(decoded_reader, target_encoding, level);
    let transmitted_body_bytes = ObservedByteCounter::default();
    Ok(PreparedPoolUpstreamRequestBody {
        body: counted_http_body_from_reader(encoded_reader, transmitted_body_bytes.clone()),
        content_length: None,
        content_encoding: target_encoding,
        compression_mode: PoolRequestBodyCompressionMode::Recompressed,
        byte_observation: PreparedPoolUpstreamRequestBodyObservation {
            logical_body_bytes,
            transmitted_body_bytes,
        },
    })
}

pub(crate) fn pool_request_snapshot_preserves_content_length(
    snapshot: &PoolReplayBodySnapshot,
) -> bool {
    matches!(snapshot, PoolReplayBodySnapshot::File { .. })
}

pub(crate) fn pool_request_snapshot_kind(snapshot: &PoolReplayBodySnapshot) -> &'static str {
    match snapshot {
        PoolReplayBodySnapshot::Empty => "empty",
        PoolReplayBodySnapshot::Memory(_) => "memory",
        PoolReplayBodySnapshot::File { .. } => "file",
    }
}

pub(crate) fn pool_request_snapshot_body_bytes(snapshot: &PoolReplayBodySnapshot) -> usize {
    match snapshot {
        PoolReplayBodySnapshot::Empty => 0,
        PoolReplayBodySnapshot::Memory(bytes) => bytes.len(),
        PoolReplayBodySnapshot::File { size, .. } => *size,
    }
}

pub(crate) async fn prepare_pool_request_body_for_account(
    proxy_request_id: u64,
    body: Option<&PoolReplayBodySnapshot>,
    original_uri: &Uri,
    method: &Method,
    content_encoding: Option<&str>,
    fast_mode_rewrite_mode: TagFastModeRewriteMode,
    image_tool_rewrite_mode: crate::ImageToolRewriteMode,
    codex_imagegen_rewrite_mode: crate::CodexImagegenRewriteMode,
    codex_imagegen_protocol: Option<CodexImagegenProtocol>,
    projected_request_info: Option<&RequestCaptureInfo>,
    projected_hosted_image_intent: Option<ImageIntent>,
    model_mapping: Option<&ResolvedModelMapping>,
) -> Result<PreparedPoolRequestBody, PoolRequestBodyPreparationError> {
    let capture_target = capture_target_for_request(original_uri.path(), method);
    let default_image_intent = match capture_target {
        Some(ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits) => {
            ImageIntent::DirectImage
        }
        _ => ImageIntent::Unknown,
    };
    let fast_mode_rewrite_required = capture_target
        .is_some_and(|target| target.allows_fast_mode_rewrite())
        && fast_mode_rewrite_mode != TagFastModeRewriteMode::KeepOriginal;
    let codex_imagegen_rewrite_required = capture_target.is_some_and(|target| {
        matches!(
            target,
            ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact
        )
    }) && codex_imagegen_protocol.is_some()
        && codex_imagegen_rewrite_mode != crate::CodexImagegenRewriteMode::KeepOriginal;
    let image_tool_rewrite_required = capture_target.is_some_and(|target| {
        matches!(
            target,
            ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact
        )
    }) && codex_imagegen_protocol.is_none()
        && image_tool_rewrite_mode != crate::ImageToolRewriteMode::KeepOriginal;
    let model_mapping_required = model_mapping.is_some();
    let rewrite_required = model_mapping_required
        || fast_mode_rewrite_required
        || image_tool_rewrite_required
        || codex_imagegen_rewrite_required;

    let Some(snapshot) = body.cloned() else {
        if model_mapping_required {
            return Err(PoolRequestBodyPreparationError::bad_request(
                "model mapping requires a JSON request body with a top-level model field",
            ));
        }
        return Ok(PreparedPoolRequestBody {
            snapshot: PoolReplayBodySnapshot::Empty,
            request_body_for_capture: Some(Bytes::new()),
            requested_service_tier: None,
            requested_image_intent: default_image_intent,
            requested_hosted_image_intent: default_image_intent,
            codex_imagegen_rewrite: codex_imagegen_protocol
                .filter(|_| {
                    codex_imagegen_rewrite_mode == crate::CodexImagegenRewriteMode::KeepOriginal
                })
                .map(codex_imagegen_keep_original_audit),
            snapshot_is_decoded: false,
        });
    };

    if !rewrite_required {
        let (
            request_body_for_capture,
            requested_service_tier,
            requested_image_intent,
            requested_hosted_image_intent,
        ) = match &snapshot {
            PoolReplayBodySnapshot::Empty => (
                Some(Bytes::new()),
                None,
                default_image_intent,
                default_image_intent,
            ),
            PoolReplayBodySnapshot::Memory(bytes) => {
                let (requested_service_tier, requested_image_intent, requested_hosted_image_intent) =
                    projected_request_info
                        .zip(projected_hosted_image_intent)
                        .map(|(info, hosted_image_intent)| {
                            let image_intent = info
                                .image_intent
                                .as_deref()
                                .map(ImageIntent::from_str)
                                .unwrap_or(default_image_intent);
                            (
                                info.requested_service_tier.clone(),
                                image_intent,
                                hosted_image_intent,
                            )
                        })
                        .or_else(|| {
                            serde_json::from_slice::<Value>(bytes).ok().map(|value| {
                                (
                                    extract_requested_service_tier_from_request_body(&value),
                                    capture_target
                                        .map(|target| {
                                            infer_image_intent_from_request_body(target, &value)
                                        })
                                        .unwrap_or(ImageIntent::Unknown),
                                    capture_target
                                        .map(|target| {
                                            infer_hosted_image_intent_from_request_body(
                                                target, &value,
                                            )
                                        })
                                        .unwrap_or(ImageIntent::Unknown),
                                )
                            })
                        })
                        .unwrap_or((None, default_image_intent, default_image_intent));
                (
                    Some(bytes.clone()),
                    requested_service_tier,
                    requested_image_intent,
                    requested_hosted_image_intent,
                )
            }
            PoolReplayBodySnapshot::File { .. } => {
                let requested_service_tier =
                    projected_request_info.and_then(|info| info.requested_service_tier.clone());
                let image_intent = projected_request_info
                    .and_then(|info| info.image_intent.as_deref())
                    .map(ImageIntent::from_str)
                    .unwrap_or(default_image_intent);
                (
                    None,
                    requested_service_tier,
                    image_intent,
                    projected_hosted_image_intent.unwrap_or(default_image_intent),
                )
            }
        };
        let codex_imagegen_rewrite = codex_imagegen_protocol
            .filter(|_| {
                codex_imagegen_rewrite_mode == crate::CodexImagegenRewriteMode::KeepOriginal
            })
            .map(codex_imagegen_keep_original_audit);
        return Ok(PreparedPoolRequestBody {
            snapshot,
            request_body_for_capture,
            requested_service_tier,
            requested_image_intent,
            requested_hosted_image_intent,
            codex_imagegen_rewrite,
            snapshot_is_decoded: false,
        });
    }

    let original_bytes = snapshot.to_bytes().await.map_err(|err| {
        PoolRequestBodyPreparationError::bad_gateway(format!(
            "failed to materialize pool request body for rewrite: {err}"
        ))
    })?;
    info!(
        proxy_request_id,
        json_parse_count = 1_u8,
        whole_body_materialization_count = 1_u8,
        materialization_bytes = original_bytes.len(),
        purpose = "account_specific_request_rewrite",
        "pool request preparation materialized account-specific rewrite body"
    );
    let downstream_encoding =
        resolve_request_body_content_encoding(&snapshot, content_encoding).await?;
    let decoded_original_bytes =
        decode_request_payload_bytes(&original_bytes, downstream_encoding)?;
    let Some(target) = capture_target else {
        if model_mapping_required {
            return Err(PoolRequestBodyPreparationError::bad_request(
                "model mapping is not supported for this request endpoint",
            ));
        }
        return Ok(PreparedPoolRequestBody {
            snapshot,
            request_body_for_capture: Some(original_bytes),
            requested_service_tier: None,
            requested_image_intent: default_image_intent,
            requested_hosted_image_intent: default_image_intent,
            codex_imagegen_rewrite: None,
            snapshot_is_decoded: false,
        });
    };
    let mut value = match serde_json::from_slice::<Value>(&decoded_original_bytes) {
        Ok(value) => value,
        Err(_) if model_mapping_required => {
            return Err(PoolRequestBodyPreparationError::bad_request(
                "model mapping requires a valid JSON request body",
            ));
        }
        Err(_) => {
            return Ok(PreparedPoolRequestBody {
                snapshot,
                request_body_for_capture: Some(original_bytes),
                requested_service_tier: None,
                requested_image_intent: default_image_intent,
                requested_hosted_image_intent: default_image_intent,
                codex_imagegen_rewrite: None,
                snapshot_is_decoded: false,
            });
        }
    };

    let model_mapping_rewritten = if let Some(mapping) = model_mapping {
        let Some(object) = value.as_object_mut() else {
            return Err(PoolRequestBodyPreparationError::bad_request(
                "model mapping requires a JSON object with a top-level model field",
            ));
        };
        if object.get("model").and_then(Value::as_str).is_none() {
            return Err(PoolRequestBodyPreparationError::bad_request(
                "model mapping requires a top-level string model field",
            ));
        }
        object.insert(
            "model".to_string(),
            Value::String(mapping.target_model.clone()),
        );
        true
    } else {
        false
    };
    let rewritten = model_mapping_rewritten
        || (if target.allows_fast_mode_rewrite() {
            rewrite_request_service_tier_for_fast_mode(&mut value, fast_mode_rewrite_mode)
        } else {
            false
        });
    let original_image_intent = infer_image_intent_from_request_body(target, &value);
    let (codex_image_rewritten, codex_imagegen_rewrite) = if let Some(protocol) =
        codex_imagegen_protocol
        && matches!(
            target,
            ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact
        ) {
        let (rewritten, audit) = rewrite_codex_imagegen_tools(
            &mut value,
            protocol,
            codex_imagegen_rewrite_mode,
            original_image_intent,
        );
        (rewritten, Some(audit))
    } else {
        (false, None)
    };
    let image_rewritten = if codex_imagegen_protocol.is_none()
        && matches!(
            target,
            ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact
        ) {
        rewrite_openai_responses_image_tools(
            &mut value,
            image_tool_rewrite_mode,
            original_image_intent,
        )
    } else {
        false
    };
    let requested_service_tier = extract_requested_service_tier_from_request_body(&value);
    let upstream_image_intent = infer_image_intent_from_request_body(target, &value);
    let upstream_hosted_image_intent = infer_hosted_image_intent_from_request_body(target, &value);
    if !rewritten && !image_rewritten && !codex_image_rewritten {
        return Ok(PreparedPoolRequestBody {
            snapshot,
            request_body_for_capture: Some(original_bytes),
            requested_service_tier,
            requested_image_intent: upstream_image_intent,
            requested_hosted_image_intent: upstream_hosted_image_intent,
            codex_imagegen_rewrite,
            snapshot_is_decoded: false,
        });
    }

    let rewritten_bytes = serde_json::to_vec(&value).map(Bytes::from).map_err(|err| {
        PoolRequestBodyPreparationError::bad_gateway(format!(
            "failed to serialize rewritten pool request body: {err}"
        ))
    })?;
    let rewritten_snapshot =
        pool_replay_snapshot_from_bytes(proxy_request_id, rewritten_bytes.clone())
            .await
            .map_err(|err| {
                PoolRequestBodyPreparationError::bad_gateway(format!(
                    "failed to persist rewritten pool request body: {err}"
                ))
            })?;
    Ok(PreparedPoolRequestBody {
        snapshot: rewritten_snapshot,
        request_body_for_capture: Some(rewritten_bytes.clone()),
        requested_service_tier,
        requested_image_intent: upstream_image_intent,
        requested_hosted_image_intent: upstream_hosted_image_intent,
        codex_imagegen_rewrite,
        snapshot_is_decoded: true,
    })
}

pub(crate) fn build_pool_replay_temp_path(proxy_request_id: u64) -> PathBuf {
    let mut path = env::temp_dir();
    let unique_id = NEXT_POOL_REPLAY_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
    path.push(format!(
        "cvm-pool-replay-{proxy_request_id}-{}-{unique_id}.bin",
        Utc::now().timestamp_nanos_opt().unwrap_or_default(),
    ));
    path
}
