use super::*;

const LIVE_ROUTE_PREFIX_BUFFER_BYTES: usize = 64 * 1024;
const LIVE_LOGICAL_OUTPUT_BUFFER_BYTES: usize = 16 * 1024;

#[derive(Debug)]
struct InvalidLiveJsonError(String);

impl std::fmt::Display for InvalidLiveJsonError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for InvalidLiveJsonError {}

#[derive(Debug)]
struct LiveDecodedRequestBodyError(io::Error);

impl std::fmt::Display for LiveDecodedRequestBodyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for LiveDecodedRequestBodyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

struct LiveDecodedRequestBodyReader<R> {
    inner: R,
}

impl<R> AsyncRead for LiveDecodedRequestBodyReader<R>
where
    R: AsyncRead + Unpin,
{
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<io::Result<()>> {
        match std::pin::Pin::new(&mut self.inner).poll_read(context, buffer) {
            std::task::Poll::Ready(Err(error)) => {
                std::task::Poll::Ready(Err(live_decoded_request_body_error(error)))
            }
            other => other,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct LiveOauthResponsesTransform {
    pub(crate) account_id: Option<i64>,
    pub(crate) installation_seed: Option<[u8; 32]>,
}

#[derive(Clone, Debug)]
pub(crate) struct LiveResponsesBodyTransformConfig {
    pub(crate) target_encoding: RequestBodyContentEncoding,
    pub(crate) compression_level: RequestCompressionLevelPreset,
    pub(crate) enforce_include_usage: bool,
    pub(crate) oauth: Option<LiveOauthResponsesTransform>,
    pub(crate) fast_mode_rewrite_mode: TagFastModeRewriteMode,
    pub(crate) image_tool_rewrite_mode: ImageToolRewriteMode,
    pub(crate) codex_imagegen_rewrite_mode: CodexImagegenRewriteMode,
    pub(crate) codex_imagegen_protocol: Option<CodexImagegenProtocol>,
    pub(crate) model_mapping_target: Option<String>,
}

pub(crate) struct LiveResponsesRequestBodyPipeline {
    pub(crate) body: Body,
    pub(crate) routing_probe_rx: watch::Receiver<PoolReplayBodyStickyKeyProbeStatus>,
    pub(crate) first_upstream_body_poll_at_rx: watch::Receiver<Option<Instant>>,
    pub(crate) original_request_stream_rx: watch::Receiver<Option<bool>>,
    pub(crate) request_body_error_rx: watch::Receiver<Option<RequestBodyReadError>>,
    pub(crate) resolved_request_content_encoding_rx:
        watch::Receiver<Option<RequestBodyContentEncoding>>,
    pub(crate) finalization_rx: watch::Receiver<bool>,
    pub(crate) oauth_rewrite_rx:
        watch::Receiver<Option<oauth_bridge::OauthResponsesRewriteSummary>>,
    config_tx: Option<oneshot::Sender<LiveResponsesBodyTransformConfig>>,
}

impl LiveResponsesRequestBodyPipeline {
    pub(crate) fn configure(&mut self, config: LiveResponsesBodyTransformConfig) -> bool {
        self.config_tx
            .take()
            .is_some_and(|tx| tx.send(config).is_ok())
    }
}

/// Starts a replay-backed request-body transformer. The routing probe can be
/// published while the root object is still open only when a future-safe route
/// commit is proven; otherwise it remains buffered until the complete root
/// object has been parsed.
pub(crate) fn spawn_live_responses_request_body_pipeline(
    raw_body: Body,
    downstream_content_encoding: Option<String>,
) -> LiveResponsesRequestBodyPipeline {
    let (config_tx, config_rx) = oneshot::channel();
    let (probe_tx, probe_rx) = watch::channel(PoolReplayBodyStickyKeyProbeStatus::Pending);
    let (logical_tx, logical_rx) = mpsc::channel::<Result<Bytes, io::Error>>(8);
    let (output_tx, output_rx) = mpsc::channel::<Result<Bytes, io::Error>>(8);
    let (first_poll_tx, first_poll_rx) = watch::channel(None);
    let (request_stream_tx, request_stream_rx) = watch::channel(None);
    let (request_body_error_tx, request_body_error_rx) = watch::channel(None);
    let (resolved_encoding_tx, resolved_encoding_rx) = watch::channel(None);
    let (finalization_tx, finalization_rx) = watch::channel(false);
    let (oauth_rewrite_tx, oauth_rewrite_rx) = watch::channel(None);

    tokio::spawn(async move {
        let mut logical_tx = Some(logical_tx);
        let mut logical_rx = Some(logical_rx);
        let result = run_live_responses_request_body_pipeline(
            raw_body,
            downstream_content_encoding.as_deref(),
            config_rx,
            &probe_tx,
            &mut logical_tx,
            &mut logical_rx,
            output_tx.clone(),
            request_stream_tx,
            oauth_rewrite_tx,
            resolved_encoding_tx,
        )
        .await;
        if let Err(err) = result {
            if matches!(
                *probe_tx.borrow(),
                PoolReplayBodyStickyKeyProbeStatus::Pending
            ) {
                let _ = probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(
                    PoolReplayBodyKeyProbe::default(),
                ));
            }
            if let Some(request_body_error) = live_request_body_pipeline_error(&err) {
                let _ = request_body_error_tx.send(Some(RequestBodyReadError {
                    status: request_body_error.0,
                    message: request_body_error.1,
                    failure_kind: request_body_error.2,
                    partial_body: Vec::new(),
                }));
            }
            let _ = finalization_tx.send(true);
            let _ = output_tx.send(Err(err)).await;
        } else {
            let _ = finalization_tx.send(true);
        }
    });

    LiveResponsesRequestBodyPipeline {
        body: Body::from_stream(TimestampedReplayBodyStream {
            inner: ReceiverStream::new(output_rx),
            first_polled_at_tx: first_poll_tx,
        }),
        routing_probe_rx: probe_rx,
        first_upstream_body_poll_at_rx: first_poll_rx,
        original_request_stream_rx: request_stream_rx,
        request_body_error_rx,
        resolved_request_content_encoding_rx: resolved_encoding_rx,
        finalization_rx,
        oauth_rewrite_rx,
        config_tx: Some(config_tx),
    }
}

pub(crate) fn live_responses_target_request_content_encoding(
    downstream_content_encoding: Option<&str>,
    requested: RequestCompressionAlgorithm,
) -> Result<RequestBodyContentEncoding, PoolRequestBodyPreparationError> {
    let downstream =
        resolve_request_body_content_encoding_from_prefix(None, downstream_content_encoding)?;
    live_responses_target_request_content_encoding_with_resolved(downstream, requested)
}

pub(crate) fn live_responses_target_request_content_encoding_with_resolved(
    downstream: RequestBodyContentEncoding,
    requested: RequestCompressionAlgorithm,
) -> Result<RequestBodyContentEncoding, PoolRequestBodyPreparationError> {
    Ok(match requested {
        RequestCompressionAlgorithm::Follow => match downstream {
            RequestBodyContentEncoding::Deflate { zlib_wrapper } => {
                RequestBodyContentEncoding::Deflate { zlib_wrapper }
            }
            other => other,
        },
        RequestCompressionAlgorithm::Identity => RequestBodyContentEncoding::Identity,
        RequestCompressionAlgorithm::Gzip => RequestBodyContentEncoding::Gzip,
        RequestCompressionAlgorithm::Deflate => {
            RequestBodyContentEncoding::Deflate { zlib_wrapper: true }
        }
        RequestCompressionAlgorithm::Zstd => RequestBodyContentEncoding::Zstd,
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_live_responses_request_body_pipeline(
    raw_body: Body,
    downstream_content_encoding: Option<&str>,
    config_rx: oneshot::Receiver<LiveResponsesBodyTransformConfig>,
    probe_tx: &watch::Sender<PoolReplayBodyStickyKeyProbeStatus>,
    logical_tx: &mut Option<mpsc::Sender<Result<Bytes, io::Error>>>,
    logical_rx: &mut Option<mpsc::Receiver<Result<Bytes, io::Error>>>,
    output_tx: mpsc::Sender<Result<Bytes, io::Error>>,
    request_stream_tx: watch::Sender<Option<bool>>,
    oauth_rewrite_tx: watch::Sender<Option<oauth_bridge::OauthResponsesRewriteSummary>>,
    resolved_encoding_tx: watch::Sender<Option<RequestBodyContentEncoding>>,
) -> io::Result<()> {
    let (reader, resolved_encoding) =
        live_decoded_request_reader(raw_body, downstream_content_encoding)
            .await
            .map_err(live_decoded_request_body_error)?;
    let _ = resolved_encoding_tx.send(Some(resolved_encoding));
    let mut reader = LiveDecodedRequestBodyReader { inner: reader };
    let Some(first) = read_non_whitespace(&mut reader).await? else {
        let _ = probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(
            PoolReplayBodyKeyProbe::default(),
        ));
        return Ok(());
    };
    if first != b'{' {
        return Err(invalid_live_json("request body must be a JSON object"));
    }

    let mut pending_fields: Vec<(String, Vec<u8>)> = Vec::new();
    let mut pending_bytes = 0usize;
    let mut config_rx = Some(config_rx);
    let mut delimiter = None;
    let mut selected_writer = None;
    let mut selected_transformer = None;

    loop {
        let mut next = match delimiter.take() {
            Some(byte) => byte,
            None => match read_non_whitespace(&mut reader).await? {
                Some(byte) => byte,
                None => return Err(invalid_live_json("request body ended before root object")),
            },
        };
        if next == b',' {
            next = read_non_whitespace(&mut reader)
                .await?
                .ok_or_else(|| invalid_live_json("request object ended after ','"))?;
            if next == b'}' {
                return Err(invalid_live_json("request object has a trailing ','"));
            }
        }
        if next == b'}' {
            if read_non_whitespace(&mut reader).await?.is_some() {
                return Err(invalid_live_json("request body has trailing content"));
            }
            if selected_writer.is_none() {
                let probe = live_routing_probe_from_fields(&pending_fields, true);
                let _ = probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(probe));
                let Ok(selected_config) = config_rx
                    .take()
                    .expect("live configuration is awaited once")
                    .await
                else {
                    return Ok(());
                };
                start_live_encoder(
                    logical_rx
                        .take()
                        .expect("live logical receiver starts exactly once"),
                    output_tx.clone(),
                    selected_config.target_encoding,
                    selected_config.compression_level,
                );
                let mut writer = LiveLogicalJsonWriter::new(
                    logical_tx
                        .as_ref()
                        .expect("live logical sender remains open")
                        .clone(),
                );
                writer.write_raw(b"{").await?;
                let mut transformer =
                    LiveRootFieldTransformer::new(selected_config, request_stream_tx.clone());
                for (pending_key, pending_value) in pending_fields.drain(..) {
                    transformer
                        .write_buffered_field(&mut writer, pending_key.as_str(), &pending_value)
                        .await?;
                }
                selected_writer = Some(writer);
                selected_transformer = Some(transformer);
            }
            let mut writer = selected_writer
                .take()
                .expect("live writer is selected before root completion");
            let mut transformer = selected_transformer
                .take()
                .expect("live transformer is selected before root completion");
            transformer.finish(&mut writer).await?;
            if let Some(summary) = transformer.oauth_rewrite_summary() {
                let _ = oauth_rewrite_tx.send(Some(summary));
            }
            writer.write_raw(b"}").await?;
            writer.finish().await?;
            logical_tx.take();
            return Ok(());
        }
        if next != b'"' {
            return Err(invalid_live_json(
                "request object key must be a JSON string",
            ));
        }
        let raw_key =
            match read_json_string(&mut reader, next, LIVE_ROUTE_PREFIX_BUFFER_BYTES).await {
                Ok(raw_key) => raw_key,
                Err(err) if is_live_route_probe_budget_error(&err) => {
                    let _ = probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(
                        PoolReplayBodyKeyProbe::default(),
                    ));
                    return Ok(());
                }
                Err(err) => return Err(err),
            };
        let key: String = serde_json::from_slice(&raw_key)
            .map_err(|_| invalid_live_json("request object key is invalid"))?;
        let Some(colon) = read_non_whitespace(&mut reader).await? else {
            return Err(invalid_live_json("request object key is missing a value"));
        };
        if colon != b':' {
            return Err(invalid_live_json("request object key is missing ':'"));
        }
        let Some(value_start) = read_non_whitespace(&mut reader).await? else {
            return Err(invalid_live_json("request object value is missing"));
        };

        // Once the required model, all enabled route metadata seen so far, and
        // the potentially route-affecting input field are buffered, commit the
        // route before consuming the next ordinary value. Until `input` has
        // been observed, a later field may still introduce image capability or
        // encrypted-content requirements, so an ordinary field must remain in
        // the prefix buffer even when the model is already known.
        let input_seen = pending_fields
            .iter()
            .any(|(pending_key, _)| pending_key == "input");
        if selected_writer.is_none()
            && pending_fields
                .iter()
                .any(|(pending_key, _)| pending_key == "model")
            && !is_precommit_routing_root_field(&key)
            && input_seen
            && !should_defer_route_commit(&key, value_start)
        {
            let _ = probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(
                live_routing_probe_from_fields(&pending_fields, false),
            ));
            let Ok(selected_config) = config_rx
                .take()
                .expect("live configuration is awaited once")
                .await
            else {
                return Ok(());
            };
            start_live_encoder(
                logical_rx
                    .take()
                    .expect("live logical receiver starts exactly once"),
                output_tx.clone(),
                selected_config.target_encoding,
                selected_config.compression_level,
            );
            let mut writer = LiveLogicalJsonWriter::new(
                logical_tx
                    .as_ref()
                    .expect("live logical sender remains open")
                    .clone(),
            );
            writer.write_raw(b"{").await?;
            let mut transformer =
                LiveRootFieldTransformer::new(selected_config, request_stream_tx.clone());
            for (pending_key, pending_value) in pending_fields.drain(..) {
                transformer
                    .write_buffered_field(&mut writer, pending_key.as_str(), &pending_value)
                    .await?;
            }
            if transformer.buffers_field(&key) {
                let (value, terminal) = read_json_value_to_vec(
                    &mut reader,
                    value_start,
                    LIVE_ROUTE_PREFIX_BUFFER_BYTES,
                )
                .await?;
                transformer
                    .write_buffered_field(&mut writer, &key, &value)
                    .await?;
                selected_writer = Some(writer);
                selected_transformer = Some(transformer);
                delimiter = Some(terminal);
                continue;
            }
            writer.begin_field(&key).await?;
            writer.flush().await?;
            let terminal = forward_json_value(&mut reader, value_start, &mut writer).await?;
            selected_writer = Some(writer);
            selected_transformer = Some(transformer);
            delimiter = Some(terminal);
            continue;
        }

        let (value, terminal) =
            match read_json_value_to_vec(&mut reader, value_start, LIVE_ROUTE_PREFIX_BUFFER_BYTES)
                .await
            {
                Ok(value) => value,
                Err(err) if is_live_route_probe_budget_error(&err) => {
                    let _ = probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(
                        PoolReplayBodyKeyProbe::default(),
                    ));
                    return Ok(());
                }
                Err(err) => return Err(err),
            };
        if std::str::from_utf8(&value).is_err() {
            return Err(invalid_live_json("request string is not valid UTF-8"));
        }
        if serde_json::from_slice::<Value>(&value).is_err() {
            return Err(invalid_live_json("request field value is invalid"));
        }
        if let (Some(mut writer), Some(mut transformer)) =
            (selected_writer.take(), selected_transformer.take())
        {
            transformer
                .write_buffered_field(&mut writer, &key, &value)
                .await?;
            selected_writer = Some(writer);
            selected_transformer = Some(transformer);
            delimiter = Some(terminal);
            continue;
        }
        pending_bytes = pending_bytes
            .saturating_add(key.len())
            .saturating_add(value.len());
        if pending_bytes > LIVE_ROUTE_PREFIX_BUFFER_BYTES {
            let _ = probe_tx.send(PoolReplayBodyStickyKeyProbeStatus::Ready(
                PoolReplayBodyKeyProbe::default(),
            ));
            return Ok(());
        }
        pending_fields.push((key, value));
        delimiter = Some(terminal);
    }
}

fn start_live_encoder(
    logical_rx: mpsc::Receiver<Result<Bytes, io::Error>>,
    output_tx: mpsc::Sender<Result<Bytes, io::Error>>,
    target_encoding: RequestBodyContentEncoding,
    compression_level: RequestCompressionLevelPreset,
) {
    tokio::spawn(async move {
        let reader: BoxedPoolRequestReader =
            Box::pin(StreamReader::new(ReceiverStream::new(logical_rx)));
        let encoded = encode_pool_request_reader(
            reader,
            target_encoding,
            request_compression_preset_to_async_level(compression_level),
        );
        let mut stream = ReaderStream::new(encoded);
        while let Some(chunk) = stream.next().await {
            if output_tx.send(chunk).await.is_err() {
                return;
            }
        }
    });
}

async fn live_decoded_request_reader(
    raw_body: Body,
    content_encoding: Option<&str>,
) -> io::Result<(BoxedPoolRequestReader, RequestBodyContentEncoding)> {
    let needs_deflate_prefix = parse_content_encodings(content_encoding)
        .iter()
        .any(|encoding| encoding == "deflate");
    let minimum_prefix_bytes = if needs_deflate_prefix { 2 } else { 1 };
    let mut source = raw_body
        .into_data_stream()
        .map(|chunk| chunk.map_err(|err| io::Error::other(err.to_string())));
    let mut prefix = Vec::new();
    while prefix.len() < minimum_prefix_bytes {
        let Some(chunk) = source.next().await else {
            break;
        };
        prefix.push(chunk?);
    }
    let prefix_bytes = prefix.concat();
    let encoding = resolve_request_body_content_encoding_from_prefix(
        Some(&prefix_bytes[..prefix_bytes.len().min(2)]),
        content_encoding,
    )
    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.message))?;
    let prefix_stream = stream::iter(prefix.into_iter().map(Ok::<Bytes, io::Error>));
    let raw_reader: BoxedPoolRequestReader =
        Box::pin(StreamReader::new(prefix_stream.chain(source)));
    let decoded = decode_pool_request_reader(raw_reader, encoding)
        .await
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.message))?;
    Ok((decoded, encoding))
}

fn live_routing_probe_from_fields(
    fields: &[(String, Vec<u8>)],
    root_object_complete: bool,
) -> PoolReplayBodyKeyProbe {
    let mut object = serde_json::Map::new();
    for (key, value) in fields {
        let Ok(value) = serde_json::from_slice::<Value>(value) else {
            continue;
        };
        object.insert(key.clone(), value);
    }
    let value = Value::Object(object);
    let image_intent =
        infer_hosted_image_intent_from_request_body(ProxyCaptureTarget::Responses, &value);
    PoolReplayBodyKeyProbe {
        sticky_key: extract_sticky_key_from_request_body(&value),
        prompt_cache_key: extract_prompt_cache_key_from_request_body(&value),
        model: value
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_string),
        contains_encrypted_content: value_contains_encrypted_content(&value),
        // Until the root object ends, a later `tools`, `tool_choice`, or
        // `input` field may still require an image-capable account. The live
        // attempt treats this as provisional and cancels on such a field.
        image_intent: if root_object_complete || image_intent == ImageIntent::Yes {
            image_intent
        } else {
            ImageIntent::Unknown
        },
        root_object_complete,
    }
}

fn is_precommit_routing_root_field(key: &str) -> bool {
    matches!(
        key,
        "model"
            | "tools"
            | "tool_choice"
            | "metadata"
            | "sticky_key"
            | "stickyKey"
            | "prompt_cache_key"
            | "promptCacheKey"
    )
}

fn should_defer_route_commit(_key: &str, _value_start: u8) -> bool {
    // JSON object fields are unordered and optional. After any ordinary field
    // there may still be a later input, tools, metadata, sticky, prompt-cache,
    // or encrypted-content field that changes the route. Without a schema-level
    // end marker, EOF is the only proof that the route-affecting root set is
    // complete, so the live encoder must stay uncommitted until root EOF.
    true
}

struct LiveRootFieldTransformer {
    config: LiveResponsesBodyTransformConfig,
    instructions: Option<Value>,
    store: Option<Value>,
    stream: Option<Value>,
    stream_options: Option<Value>,
    client_metadata: Option<Value>,
    request_stream_tx: watch::Sender<Option<bool>>,
    rewrite_fields: serde_json::Map<String, Value>,
    oauth_rewrite: oauth_bridge::OauthResponsesRewriteSummary,
    model_mapping_applied: bool,
}

impl LiveRootFieldTransformer {
    fn new(
        config: LiveResponsesBodyTransformConfig,
        request_stream_tx: watch::Sender<Option<bool>>,
    ) -> Self {
        Self {
            config,
            instructions: None,
            store: None,
            stream: None,
            stream_options: None,
            client_metadata: None,
            request_stream_tx,
            rewrite_fields: serde_json::Map::new(),
            oauth_rewrite: oauth_bridge::OauthResponsesRewriteSummary::default(),
            model_mapping_applied: false,
        }
    }

    fn buffers_field(&self, key: &str) -> bool {
        self.config.oauth.is_some_and(|_| {
            matches!(
                key,
                "instructions" | "store" | "stream" | "max_output_tokens" | "client_metadata"
            )
        }) || (self.config.enforce_include_usage && matches!(key, "stream" | "stream_options"))
            || self.account_rewrite_required()
                && matches!(
                    key,
                    "service_tier"
                        | "serviceTier"
                        | "tools"
                        | "tool_choice"
                        | "input"
                        | "additional_tools"
                        | "reasoning"
                        | "parallel_tool_calls"
                )
            || (self.config.model_mapping_target.is_some() && key == "model")
    }

    fn account_rewrite_required(&self) -> bool {
        self.config.fast_mode_rewrite_mode != TagFastModeRewriteMode::KeepOriginal
            || self.config.image_tool_rewrite_mode != ImageToolRewriteMode::KeepOriginal
            || self.config.codex_imagegen_rewrite_mode != CodexImagegenRewriteMode::KeepOriginal
    }

    async fn write_buffered_field(
        &mut self,
        writer: &mut LiveLogicalJsonWriter,
        key: &str,
        raw_value: &[u8],
    ) -> io::Result<()> {
        if !self.buffers_field(key) {
            return writer.write_field_raw(key, raw_value).await;
        }
        let value: Value = serde_json::from_slice(raw_value)
            .map_err(|_| invalid_live_json("request field value is invalid"))?;
        match key {
            "model" if self.config.model_mapping_target.is_some() => {
                if !value.is_string() {
                    return Err(invalid_live_json(
                        "model mapping requires a top-level string model field",
                    ));
                }
                self.model_mapping_applied = true;
                writer
                    .write_field_value(
                        "model",
                        &Value::String(
                            self.config
                                .model_mapping_target
                                .as_ref()
                                .expect("model mapping target should be present")
                                .clone(),
                        ),
                    )
                    .await?;
            }
            "instructions" if self.config.oauth.is_some() => self.instructions = Some(value),
            "store" if self.config.oauth.is_some() => self.store = Some(value),
            "stream" => {
                self.stream = Some(value);
                let _ = self
                    .request_stream_tx
                    .send(self.stream.as_ref().and_then(Value::as_bool));
            }
            "stream_options" => self.stream_options = Some(value),
            "client_metadata" if self.config.oauth.is_some() => self.client_metadata = Some(value),
            "max_output_tokens" if self.config.oauth.is_some() => {
                self.oauth_rewrite.removed_max_output_tokens = true;
            }
            key if self.account_rewrite_required()
                && matches!(
                    key,
                    "service_tier"
                        | "serviceTier"
                        | "tools"
                        | "tool_choice"
                        | "input"
                        | "additional_tools"
                        | "reasoning"
                        | "parallel_tool_calls"
                ) =>
            {
                self.rewrite_fields.insert(key.to_string(), value);
            }
            _ => writer.write_field_value(key, &value).await?,
        }
        Ok(())
    }

    async fn finish(&mut self, writer: &mut LiveLogicalJsonWriter) -> io::Result<()> {
        if self.config.model_mapping_target.is_some() && !self.model_mapping_applied {
            return Err(invalid_live_json(
                "model mapping requires a top-level string model field",
            ));
        }
        if self.account_rewrite_required() {
            let mut rewrite_value = Value::Object(std::mem::take(&mut self.rewrite_fields));
            rewrite_request_service_tier_for_fast_mode(
                &mut rewrite_value,
                self.config.fast_mode_rewrite_mode,
            );
            let image_intent =
                infer_image_intent_from_request_body(ProxyCaptureTarget::Responses, &rewrite_value);
            if let Some(protocol) = self.config.codex_imagegen_protocol {
                rewrite_codex_imagegen_tools(
                    &mut rewrite_value,
                    protocol,
                    self.config.codex_imagegen_rewrite_mode,
                    image_intent,
                );
            } else {
                rewrite_openai_responses_image_tools(
                    &mut rewrite_value,
                    self.config.image_tool_rewrite_mode,
                    image_intent,
                );
            }
            if let Value::Object(fields) = rewrite_value {
                for (key, value) in fields {
                    writer.write_field_value(&key, &value).await?;
                }
            }
        }
        let wants_stream = self.config.oauth.is_some()
            || self
                .stream
                .as_ref()
                .and_then(Value::as_bool)
                .unwrap_or(false);
        if let Some(oauth) = self.config.oauth {
            self.oauth_rewrite.added_instructions = self.instructions.is_none();
            self.oauth_rewrite.added_store = self.store.is_none();
            self.oauth_rewrite.forced_stream_true =
                self.stream.as_ref().and_then(Value::as_bool) != Some(true);
            writer
                .write_field_value(
                    "instructions",
                    self.instructions
                        .as_ref()
                        .unwrap_or(&Value::String(String::new())),
                )
                .await?;
            writer
                .write_field_value("store", self.store.as_ref().unwrap_or(&Value::Bool(false)))
                .await?;
            writer
                .write_field_value("stream", &Value::Bool(true))
                .await?;
            if let Some(client_metadata) = self.client_metadata.take() {
                let (client_metadata, rewrite) = oauth_bridge::rewrite_live_oauth_client_metadata(
                    client_metadata,
                    oauth.account_id,
                    oauth.installation_seed.as_ref(),
                );
                self.oauth_rewrite.rewrote_installation_id = rewrite.rewrote_installation_id;
                self.oauth_rewrite.removed_installation_id = rewrite.removed_installation_id;
                writer
                    .write_field_value("client_metadata", &client_metadata)
                    .await?;
            }
            self.oauth_rewrite.applied = self.oauth_rewrite.added_instructions
                || self.oauth_rewrite.added_store
                || self.oauth_rewrite.forced_stream_true
                || self.oauth_rewrite.removed_max_output_tokens
                || self.oauth_rewrite.rewrote_installation_id
                || self.oauth_rewrite.removed_installation_id;
        } else if let Some(stream) = self.stream.take() {
            writer.write_field_value("stream", &stream).await?;
        }

        if self.config.enforce_include_usage && wants_stream {
            let mut stream_options = self
                .stream_options
                .take()
                .unwrap_or_else(|| Value::Object(Default::default()));
            match &mut stream_options {
                Value::Object(object) => {
                    object.insert("include_usage".to_string(), Value::Bool(true));
                }
                _ => stream_options = serde_json::json!({ "include_usage": true }),
            }
            writer
                .write_field_value("stream_options", &stream_options)
                .await?;
        } else if let Some(stream_options) = self.stream_options.take() {
            writer
                .write_field_value("stream_options", &stream_options)
                .await?;
        }
        Ok(())
    }

    fn oauth_rewrite_summary(&self) -> Option<oauth_bridge::OauthResponsesRewriteSummary> {
        self.config.oauth.map(|_| self.oauth_rewrite.clone())
    }
}

struct LiveLogicalJsonWriter {
    tx: mpsc::Sender<Result<Bytes, io::Error>>,
    buffer: Vec<u8>,
    wrote_field: bool,
}

impl LiveLogicalJsonWriter {
    fn new(tx: mpsc::Sender<Result<Bytes, io::Error>>) -> Self {
        Self {
            tx,
            buffer: Vec::with_capacity(LIVE_LOGICAL_OUTPUT_BUFFER_BYTES),
            wrote_field: false,
        }
    }

    async fn begin_field(&mut self, key: &str) -> io::Result<()> {
        if self.wrote_field {
            self.write_raw(b",").await?;
        }
        let key = serde_json::to_vec(key)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        self.write_raw(&key).await?;
        self.write_raw(b":").await?;
        self.wrote_field = true;
        Ok(())
    }

    async fn write_field_raw(&mut self, key: &str, value: &[u8]) -> io::Result<()> {
        self.begin_field(key).await?;
        self.write_raw(value).await
    }

    async fn write_field_value(&mut self, key: &str, value: &Value) -> io::Result<()> {
        let value = serde_json::to_vec(value)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
        self.write_field_raw(key, &value).await
    }

    async fn write_raw(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.buffer.extend_from_slice(bytes);
        if self.buffer.len() >= LIVE_LOGICAL_OUTPUT_BUFFER_BYTES {
            self.flush().await?;
        }
        Ok(())
    }

    async fn flush(&mut self) -> io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let bytes = Bytes::from(std::mem::take(&mut self.buffer));
        self.tx
            .send(Ok(bytes))
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "live upstream body closed"))
    }

    async fn finish(&mut self) -> io::Result<()> {
        self.flush().await
    }
}

async fn forward_json_value<R>(
    reader: &mut R,
    first: u8,
    writer: &mut LiveLogicalJsonWriter,
) -> io::Result<u8>
where
    R: AsyncRead + Unpin,
{
    writer.write_raw(&[first]).await?;
    match first {
        b'"' => forward_json_string_tail(reader, writer).await?,
        b'{' => Box::pin(forward_json_object(reader, writer)).await?,
        b'[' => Box::pin(forward_json_array(reader, writer)).await?,
        b'-' | b'0'..=b'9' | b't' | b'f' | b'n' => {
            return forward_json_primitive(reader, first, writer).await;
        }
        _ => return Err(invalid_live_json("request value has an invalid token")),
    }
    read_value_terminal(reader).await
}

async fn forward_json_object<R>(
    reader: &mut R,
    writer: &mut LiveLogicalJsonWriter,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
{
    let mut next = read_non_whitespace(reader)
        .await?
        .ok_or_else(|| invalid_live_json("request object ended before closing"))?;
    if next == b'}' {
        return writer.write_raw(&[next]).await;
    }
    loop {
        if next != b'"' {
            return Err(invalid_live_json(
                "request object key must be a JSON string",
            ));
        }
        writer.write_raw(&[next]).await?;
        forward_json_string_tail(reader, writer).await?;
        let colon = read_non_whitespace(reader)
            .await?
            .ok_or_else(|| invalid_live_json("request object key is missing ':'"))?;
        if colon != b':' {
            return Err(invalid_live_json("request object key is missing ':'"));
        }
        writer.write_raw(&[colon]).await?;
        let value_start = read_non_whitespace(reader)
            .await?
            .ok_or_else(|| invalid_live_json("request object value is missing"))?;
        let delimiter = forward_json_value(reader, value_start, writer).await?;
        match delimiter {
            b',' => {
                writer.write_raw(&[delimiter]).await?;
                next = read_non_whitespace(reader)
                    .await?
                    .ok_or_else(|| invalid_live_json("request object ended after ','"))?;
                if next == b'}' {
                    return Err(invalid_live_json("request object has a trailing ','"));
                }
            }
            b'}' => return writer.write_raw(&[delimiter]).await,
            _ => {
                return Err(invalid_live_json(
                    "request object value has an invalid delimiter",
                ));
            }
        }
    }
}

async fn forward_json_array<R>(reader: &mut R, writer: &mut LiveLogicalJsonWriter) -> io::Result<()>
where
    R: AsyncRead + Unpin,
{
    let mut value_start = read_non_whitespace(reader)
        .await?
        .ok_or_else(|| invalid_live_json("request array ended before closing"))?;
    if value_start == b']' {
        return writer.write_raw(&[value_start]).await;
    }
    loop {
        let delimiter = forward_json_value(reader, value_start, writer).await?;
        match delimiter {
            b',' => {
                writer.write_raw(&[delimiter]).await?;
                value_start = read_non_whitespace(reader)
                    .await?
                    .ok_or_else(|| invalid_live_json("request array ended after ','"))?;
                if value_start == b']' {
                    return Err(invalid_live_json("request array has a trailing ','"));
                }
            }
            b']' => return writer.write_raw(&[delimiter]).await,
            _ => {
                return Err(invalid_live_json(
                    "request array value has an invalid delimiter",
                ));
            }
        }
    }
}

async fn forward_json_string_tail<R>(
    reader: &mut R,
    writer: &mut LiveLogicalJsonWriter,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
{
    let mut escaped = false;
    let mut unicode_digits_remaining = 0_u8;
    let mut utf8_tail = Vec::new();
    loop {
        let byte = read_json_string_byte(reader).await?;
        writer.write_raw(&[byte]).await?;
        if unicode_digits_remaining > 0 {
            if !byte.is_ascii_hexdigit() {
                return Err(invalid_live_json(
                    "request string has an invalid unicode escape",
                ));
            }
            unicode_digits_remaining -= 1;
            continue;
        }
        if escaped {
            match byte {
                b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => escaped = false,
                b'u' => {
                    escaped = false;
                    unicode_digits_remaining = 4;
                }
                _ => return Err(invalid_live_json("request string has an invalid escape")),
            }
            continue;
        }
        validate_json_string_utf8_byte(&mut utf8_tail, byte)?;
        match byte {
            b'"' => return Ok(()),
            b'\\' => escaped = true,
            0..=0x1f => return Err(invalid_live_json("request string has a control character")),
            _ => {}
        }
    }
}

async fn forward_json_primitive<R>(
    reader: &mut R,
    first: u8,
    writer: &mut LiveLogicalJsonWriter,
) -> io::Result<u8>
where
    R: AsyncRead + Unpin,
{
    let mut token = vec![first];
    loop {
        let next = read_non_whitespace(reader)
            .await?
            .ok_or_else(|| invalid_live_json("request value is missing a delimiter"))?;
        if matches!(next, b',' | b'}' | b']') {
            if !is_valid_json_primitive(&token) {
                return Err(invalid_live_json("request value has an invalid token"));
            }
            return Ok(next);
        }
        token.push(next);
        writer.write_raw(&[next]).await?;
    }
}

fn is_valid_json_primitive(token: &[u8]) -> bool {
    matches!(token, b"true" | b"false" | b"null") || is_valid_json_number(token)
}

fn is_valid_json_number(token: &[u8]) -> bool {
    let mut index = 0;
    if token.get(index) == Some(&b'-') {
        index += 1;
    }
    let Some(&first_digit) = token.get(index) else {
        return false;
    };
    match first_digit {
        b'0' => index += 1,
        b'1'..=b'9' => {
            index += 1;
            while token.get(index).is_some_and(u8::is_ascii_digit) {
                index += 1;
            }
        }
        _ => return false,
    }
    if first_digit == b'0' && token.get(index).is_some_and(u8::is_ascii_digit) {
        return false;
    }
    if token.get(index) == Some(&b'.') {
        index += 1;
        let fraction_start = index;
        while token.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == fraction_start {
            return false;
        }
    }
    if matches!(token.get(index), Some(b'e' | b'E')) {
        index += 1;
        if matches!(token.get(index), Some(b'+' | b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while token.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == exponent_start {
            return false;
        }
    }
    index == token.len()
}

async fn read_non_whitespace<R>(reader: &mut R) -> io::Result<Option<u8>>
where
    R: AsyncRead + Unpin,
{
    loop {
        match reader.read_u8().await {
            Ok(byte) if byte.is_ascii_whitespace() => {}
            Ok(byte) => return Ok(Some(byte)),
            Err(err)
                if err.kind() == io::ErrorKind::UnexpectedEof
                    && !is_live_decoded_request_body_error(&err) =>
            {
                return Ok(None);
            }
            Err(err) => return Err(err),
        }
    }
}

async fn read_json_string_byte<R>(reader: &mut R) -> io::Result<u8>
where
    R: AsyncRead + Unpin,
{
    match reader.read_u8().await {
        Ok(byte) => Ok(byte),
        Err(err)
            if err.kind() == io::ErrorKind::UnexpectedEof
                && !is_live_decoded_request_body_error(&err) =>
        {
            Err(invalid_live_json("request string ended before closing"))
        }
        Err(err) => Err(err),
    }
}

fn validate_json_string_utf8_byte(tail: &mut Vec<u8>, byte: u8) -> io::Result<()> {
    tail.push(byte);
    match std::str::from_utf8(tail) {
        Ok(_) => tail.clear(),
        Err(error) if error.error_len().is_none() => {}
        Err(_) => return Err(invalid_live_json("request string is not valid UTF-8")),
    }
    Ok(())
}

async fn read_json_string<R>(reader: &mut R, first: u8, limit: usize) -> io::Result<Vec<u8>>
where
    R: AsyncRead + Unpin,
{
    let mut value = vec![first];
    let mut escaped = false;
    let mut utf8_tail = Vec::new();
    loop {
        let byte = read_json_string_byte(reader).await?;
        value.push(byte);
        if value.len() > limit {
            return Err(invalid_live_json(
                "routing field exceeds the live prefix budget",
            ));
        }
        if escaped {
            escaped = false;
        } else if byte == b'\\' {
            validate_json_string_utf8_byte(&mut utf8_tail, byte)?;
            escaped = true;
        } else if byte == b'"' {
            validate_json_string_utf8_byte(&mut utf8_tail, byte)?;
            return Ok(value);
        } else {
            validate_json_string_utf8_byte(&mut utf8_tail, byte)?;
        }
    }
}

async fn read_json_value_to_vec<R>(
    reader: &mut R,
    first: u8,
    limit: usize,
) -> io::Result<(Vec<u8>, u8)>
where
    R: AsyncRead + Unpin,
{
    let mut value = Vec::new();
    let terminal = collect_json_value(reader, first, Some(&mut value), limit).await?;
    Ok((value, terminal))
}

async fn collect_json_value<R>(
    reader: &mut R,
    first: u8,
    mut collected: Option<&mut Vec<u8>>,
    limit: usize,
) -> io::Result<u8>
where
    R: AsyncRead + Unpin,
{
    // The streaming caller writes values through a small adapter below. Keeping
    // the lexical state here makes primitive, string and nested JSON values use
    // identical delimiter handling.
    let mut bytes = vec![first];
    let mut stack = match first {
        b'{' => vec![b'}'],
        b'[' => vec![b']'],
        _ => Vec::new(),
    };
    let mut in_string = first == b'"';
    let mut escaped = false;
    let composite_or_string = !stack.is_empty() || in_string;

    loop {
        if !composite_or_string && stack.is_empty() && !in_string {
            let Some(byte) = read_non_whitespace(reader).await? else {
                return Err(invalid_live_json("request value is missing a delimiter"));
            };
            if matches!(byte, b',' | b'}' | b']') {
                append_live_value(&mut collected, &bytes, limit)?;
                return Ok(byte);
            }
            bytes.push(byte);
            continue;
        }

        let byte = match reader.read_u8().await {
            Ok(byte) => byte,
            Err(err)
                if err.kind() == io::ErrorKind::UnexpectedEof
                    && !is_live_decoded_request_body_error(&err) =>
            {
                return Err(invalid_live_json(if in_string {
                    "request string ended before closing"
                } else {
                    "request value ended before closing"
                }));
            }
            Err(err) => return Err(err),
        };
        bytes.push(byte);
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
                if stack.is_empty() {
                    let terminal = read_value_terminal(reader).await?;
                    append_live_value(&mut collected, &bytes, limit)?;
                    return Ok(terminal);
                }
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => stack.push(b'}'),
            b'[' => stack.push(b']'),
            b'}' | b']' => {
                if stack.pop() != Some(byte) {
                    return Err(invalid_live_json("request JSON nesting is invalid"));
                }
                if stack.is_empty() {
                    let terminal = read_value_terminal(reader).await?;
                    append_live_value(&mut collected, &bytes, limit)?;
                    return Ok(terminal);
                }
            }
            _ => {}
        }
    }
}

fn append_live_value(
    collected: &mut Option<&mut Vec<u8>>,
    bytes: &[u8],
    limit: usize,
) -> io::Result<()> {
    let Some(collected) = collected.as_deref_mut() else {
        return Ok(());
    };
    if collected.len().saturating_add(bytes.len()) > limit {
        return Err(invalid_live_json(
            "routing field exceeds the live prefix budget",
        ));
    }
    collected.extend_from_slice(bytes);
    Ok(())
}

async fn read_value_terminal<R>(reader: &mut R) -> io::Result<u8>
where
    R: AsyncRead + Unpin,
{
    match read_non_whitespace(reader).await? {
        Some(byte @ (b',' | b'}' | b']')) => Ok(byte),
        Some(_) => Err(invalid_live_json("request value has an invalid delimiter")),
        None => Err(invalid_live_json("request value is missing a delimiter")),
    }
}

fn invalid_live_json(message: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        InvalidLiveJsonError(message.to_string()),
    )
}

fn live_decoded_request_body_error(error: io::Error) -> io::Error {
    let kind = error.kind();
    io::Error::new(kind, LiveDecodedRequestBodyError(error))
}

fn is_invalid_live_json_error(err: &io::Error) -> bool {
    err.get_ref()
        .is_some_and(|source| source.is::<InvalidLiveJsonError>())
}

fn is_live_decoded_request_body_error(err: &io::Error) -> bool {
    err.get_ref()
        .is_some_and(|source| source.is::<LiveDecodedRequestBodyError>())
}

fn live_request_body_pipeline_error(err: &io::Error) -> Option<(StatusCode, String, &'static str)> {
    if is_invalid_live_json_error(err) {
        return Some((
            StatusCode::BAD_REQUEST,
            format!("request body must be valid JSON: {err}"),
            PROXY_FAILURE_REQUEST_BODY_INVALID_JSON,
        ));
    }
    if is_live_decoded_request_body_error(err) {
        return Some((
            StatusCode::BAD_REQUEST,
            format!("failed to decode request body: {err}"),
            PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED,
        ));
    }
    None
}

fn is_live_route_probe_budget_error(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::InvalidData
        && err.to_string() == "routing field exceeds the live prefix budget"
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn collect_body(body: Body) -> Bytes {
        let mut bytes = Vec::new();
        let mut stream = body.into_data_stream();
        while let Some(chunk) = stream.next().await {
            bytes.extend_from_slice(&chunk.expect("live body chunk"));
        }
        Bytes::from(bytes)
    }

    #[tokio::test]
    async fn final_route_gate_keeps_upstream_body_empty_until_downstream_eof() {
        let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(2);
        tx.send(Ok(Bytes::from_static(
            br#"{"model":"gpt-5.6","input":"delayed "#,
        )))
        .await
        .expect("send request prefix");
        let mut pipeline = spawn_live_responses_request_body_pipeline(
            Body::from_stream(ReceiverStream::new(rx)),
            None,
        );
        assert!(
            timeout(
                Duration::from_millis(50),
                wait_for_replay_body_sticky_key_probe(
                    &pipeline.routing_probe_rx,
                    Duration::from_secs(1)
                ),
            )
            .await
            .is_err()
        );

        tx.send(Ok(Bytes::from_static(br#"tail"}"#)))
            .await
            .expect("send delayed tail");
        drop(tx);
        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(probe.model.as_deref(), Some("gpt-5.6"));
        assert!(pipeline.configure(LiveResponsesBodyTransformConfig {
            target_encoding: RequestBodyContentEncoding::Identity,
            compression_level: RequestCompressionLevelPreset::Balanced,
            enforce_include_usage: false,
            oauth: None,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
            codex_imagegen_rewrite_mode: CodexImagegenRewriteMode::KeepOriginal,
            codex_imagegen_protocol: None,
            model_mapping_target: None,
        }));

        let output = collect_body(pipeline.body).await;
        assert!(pipeline.first_upstream_body_poll_at_rx.borrow().is_some());
        assert_eq!(
            serde_json::from_slice::<Value>(&output).expect("decode live forwarded JSON"),
            serde_json::json!({"model":"gpt-5.6","input":"delayed tail"})
        );
    }

    #[tokio::test]
    async fn live_first_capture_responses_forwards_objects_inside_input_arrays() {
        let source = br#"{"model":"gpt-5.6","input":[{}]}"#;
        let mut pipeline = spawn_live_responses_request_body_pipeline(
            Body::from(Bytes::from_static(source)),
            None,
        );
        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(probe.model.as_deref(), Some("gpt-5.6"));
        assert!(pipeline.configure(LiveResponsesBodyTransformConfig {
            target_encoding: RequestBodyContentEncoding::Identity,
            compression_level: RequestCompressionLevelPreset::Balanced,
            enforce_include_usage: false,
            oauth: None,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
            codex_imagegen_rewrite_mode: CodexImagegenRewriteMode::KeepOriginal,
            codex_imagegen_protocol: None,
            model_mapping_target: None,
        }));

        let output = collect_body(pipeline.body).await;
        assert_eq!(
            serde_json::from_slice::<Value>(&output).expect("decode live forwarded JSON"),
            serde_json::from_slice::<Value>(source).expect("decode source JSON")
        );
    }

    #[tokio::test]
    async fn live_first_capture_responses_forwards_scalar_input_arrays_before_model() {
        let source = br#"{"input":[1],"model":"gpt-5.6"}"#;
        let mut pipeline = spawn_live_responses_request_body_pipeline(
            Body::from(Bytes::from_static(source)),
            None,
        );
        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(probe.model.as_deref(), Some("gpt-5.6"));
        assert!(pipeline.configure(LiveResponsesBodyTransformConfig {
            target_encoding: RequestBodyContentEncoding::Identity,
            compression_level: RequestCompressionLevelPreset::Balanced,
            enforce_include_usage: false,
            oauth: None,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
            codex_imagegen_rewrite_mode: CodexImagegenRewriteMode::KeepOriginal,
            codex_imagegen_protocol: None,
            model_mapping_target: None,
        }));

        let output = collect_body(pipeline.body).await;
        assert_eq!(
            serde_json::from_slice::<Value>(&output).expect("decode live forwarded JSON"),
            serde_json::from_slice::<Value>(source).expect("decode source JSON")
        );
    }

    #[tokio::test]
    async fn live_first_capture_responses_forwards_nested_input_arrays_across_chunks() {
        let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(2);
        tx.send(Ok(Bytes::from_static(
            br#"{"model":"gpt-5.6","input":[{"role":"user","content":["#,
        )))
        .await
        .expect("send nested input prefix");
        let mut pipeline = spawn_live_responses_request_body_pipeline(
            Body::from_stream(ReceiverStream::new(rx)),
            None,
        );
        tx.send(Ok(Bytes::from_static(
            br#"{"type":"input_text","text":"hello"}]}]}"#,
        )))
        .await
        .expect("send nested input tail");
        drop(tx);

        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(probe.model.as_deref(), Some("gpt-5.6"));
        assert!(pipeline.configure(LiveResponsesBodyTransformConfig {
            target_encoding: RequestBodyContentEncoding::Identity,
            compression_level: RequestCompressionLevelPreset::Balanced,
            enforce_include_usage: false,
            oauth: None,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
            codex_imagegen_rewrite_mode: CodexImagegenRewriteMode::KeepOriginal,
            codex_imagegen_protocol: None,
            model_mapping_target: None,
        }));

        let output = collect_body(pipeline.body).await;
        assert_eq!(
            serde_json::from_slice::<Value>(&output).expect("decode live forwarded JSON"),
            serde_json::json!({
                "model": "gpt-5.6",
                "input": [{
                    "role": "user",
                    "content": [{"type": "input_text", "text": "hello"}],
                }],
            })
        );
    }

    #[tokio::test]
    async fn live_first_capture_responses_forwards_utf8_input_across_chunks() {
        let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(2);
        tx.send(Ok(Bytes::from_static(br#"{"model":"gpt-5.6","input":""#)))
            .await
            .expect("send UTF-8 request prefix");
        let mut pipeline = spawn_live_responses_request_body_pipeline(
            Body::from_stream(ReceiverStream::new(rx)),
            None,
        );
        tx.send(Ok(Bytes::from_static(b"\xe4")))
            .await
            .expect("send UTF-8 leading byte");
        tx.send(Ok(Bytes::from_static(b"\xbd\xa0\"}")))
            .await
            .expect("send UTF-8 trailing bytes");
        drop(tx);

        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(probe.model.as_deref(), Some("gpt-5.6"));
        assert!(pipeline.configure(LiveResponsesBodyTransformConfig {
            target_encoding: RequestBodyContentEncoding::Identity,
            compression_level: RequestCompressionLevelPreset::Balanced,
            enforce_include_usage: false,
            oauth: None,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
            codex_imagegen_rewrite_mode: CodexImagegenRewriteMode::KeepOriginal,
            codex_imagegen_protocol: None,
            model_mapping_target: None,
        }));

        let output = collect_body(pipeline.body).await;
        assert_eq!(
            serde_json::from_slice::<Value>(&output).expect("decode live UTF-8 body"),
            serde_json::json!({"model": "gpt-5.6", "input": "你"})
        );
    }

    #[tokio::test]
    async fn final_route_gate_waits_for_late_routing_metadata_before_upstream_body() {
        let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(2);
        tx.send(Ok(Bytes::from_static(
            br#"{"model":"gpt-5.6","input":"hello","instructions":"stream","#,
        )))
        .await
        .expect("send request prefix");
        let mut pipeline = spawn_live_responses_request_body_pipeline(
            Body::from_stream(ReceiverStream::new(rx)),
            None,
        );
        assert!(
            timeout(
                Duration::from_millis(50),
                wait_for_replay_body_sticky_key_probe(
                    &pipeline.routing_probe_rx,
                    Duration::from_secs(1)
                ),
            )
            .await
            .is_err()
        );

        tx.send(Ok(Bytes::from_static(
            br#""metadata":{"sticky_key":"late"}}"#,
        )))
        .await
        .expect("send late routing metadata");
        drop(tx);

        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(probe.model.as_deref(), Some("gpt-5.6"));
        assert_eq!(probe.sticky_key.as_deref(), Some("late"));
        assert!(pipeline.configure(LiveResponsesBodyTransformConfig {
            target_encoding: RequestBodyContentEncoding::Identity,
            compression_level: RequestCompressionLevelPreset::Balanced,
            enforce_include_usage: false,
            oauth: None,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
            codex_imagegen_rewrite_mode: CodexImagegenRewriteMode::KeepOriginal,
            codex_imagegen_protocol: None,
            model_mapping_target: None,
        }));

        let output: Value = serde_json::from_slice(&collect_body(pipeline.body).await)
            .expect("decode body after final route gate");
        assert_eq!(output["metadata"]["sticky_key"], "late");
    }

    #[tokio::test]
    async fn final_route_gate_waits_for_late_root_sticky_key_before_upstream_body() {
        let pipeline = spawn_live_responses_request_body_pipeline(
            Body::from(Bytes::from_static(
                br#"{"model":"gpt-5.6","input":"hello","stickyKey":"late-root"}"#,
            )),
            None,
        );
        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(probe.model.as_deref(), Some("gpt-5.6"));
        assert_eq!(probe.sticky_key.as_deref(), Some("late-root"));
    }

    #[tokio::test]
    async fn responses_route_finalization_transform_applies_oauth_and_include_usage() {
        let mut pipeline = spawn_live_responses_request_body_pipeline(
            Body::from(Bytes::from_static(
                br#"{"model":"gpt-5.6","stream":false,"max_output_tokens":7,"client_metadata":{"x-codex-installation-id":"downstream","keep":true}}"#,
            )),
            None,
        );
        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(probe.model.as_deref(), Some("gpt-5.6"));
        assert!(pipeline.configure(LiveResponsesBodyTransformConfig {
            target_encoding: RequestBodyContentEncoding::Identity,
            compression_level: RequestCompressionLevelPreset::Balanced,
            enforce_include_usage: true,
            oauth: Some(LiveOauthResponsesTransform {
                account_id: Some(11),
                installation_seed: Some([0x11; 32]),
            }),
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
            codex_imagegen_rewrite_mode: CodexImagegenRewriteMode::KeepOriginal,
            codex_imagegen_protocol: None,
            model_mapping_target: None,
        }));
        let oauth_rewrite_rx = pipeline.oauth_rewrite_rx.clone();
        let output = collect_body(pipeline.body).await;
        let value: Value = serde_json::from_slice(&output).expect("decode OAuth live body");
        assert_eq!(value["stream"], true);
        assert_eq!(value["instructions"], "");
        assert_eq!(value["store"], false);
        assert!(value.get("max_output_tokens").is_none());
        assert_eq!(value["client_metadata"]["keep"], true);
        assert_ne!(
            value["client_metadata"]["x-codex-installation-id"],
            Value::String("downstream".to_string())
        );
        assert_eq!(value["stream_options"]["include_usage"], true);
        let rewrite = oauth_rewrite_rx
            .borrow()
            .clone()
            .expect("OAuth rewrite audit should be finalized with the request body");
        assert!(rewrite.applied);
        assert!(rewrite.added_instructions);
        assert!(rewrite.added_store);
        assert!(rewrite.forced_stream_true);
        assert!(rewrite.removed_max_output_tokens);
        assert!(rewrite.rewrote_installation_id);
    }

    #[tokio::test]
    async fn responses_route_finalization_transform_applies_account_rewrites() {
        let mut pipeline = spawn_live_responses_request_body_pipeline(
            Body::from(Bytes::from_static(
                br#"{"model":"gpt-5.6","tools":[],"tool_choice":"auto"}"#,
            )),
            None,
        );
        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(probe.model.as_deref(), Some("gpt-5.6"));
        assert!(pipeline.configure(LiveResponsesBodyTransformConfig {
            target_encoding: RequestBodyContentEncoding::Identity,
            compression_level: RequestCompressionLevelPreset::Balanced,
            enforce_include_usage: false,
            oauth: None,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::ForceAdd,
            image_tool_rewrite_mode: ImageToolRewriteMode::ForceAdd,
            codex_imagegen_rewrite_mode: CodexImagegenRewriteMode::KeepOriginal,
            codex_imagegen_protocol: None,
            model_mapping_target: None,
        }));
        let output = collect_body(pipeline.body).await;
        let value: Value = serde_json::from_slice(&output).expect("decode rewritten body");
        assert_eq!(value["service_tier"], "priority");
        assert_eq!(value["tool_choice"], "auto");
        assert!(
            value["tools"]
                .as_array()
                .expect("rewritten tools")
                .iter()
                .any(|tool| tool["type"] == "image_generation")
        );
    }

    #[tokio::test]
    async fn responses_route_finalization_transform_rewrites_mapped_model_and_fails_closed() {
        let mut mapped = spawn_live_responses_request_body_pipeline(
            Body::from(Bytes::from_static(
                br#"{"model":"client-fast","input":"hello"}"#,
            )),
            None,
        );
        let probe =
            wait_for_replay_body_sticky_key_probe(&mapped.routing_probe_rx, Duration::from_secs(1))
                .await;
        assert_eq!(probe.model.as_deref(), Some("client-fast"));
        assert!(mapped.configure(LiveResponsesBodyTransformConfig {
            target_encoding: RequestBodyContentEncoding::Identity,
            compression_level: RequestCompressionLevelPreset::Balanced,
            enforce_include_usage: false,
            oauth: None,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
            codex_imagegen_rewrite_mode: CodexImagegenRewriteMode::KeepOriginal,
            codex_imagegen_protocol: None,
            model_mapping_target: Some("upstream-model".to_string()),
        }));
        let value: Value = serde_json::from_slice(&collect_body(mapped.body).await)
            .expect("decode mapped live body");
        assert_eq!(value["model"], "upstream-model");

        let mut unsafe_payload = spawn_live_responses_request_body_pipeline(
            Body::from(Bytes::from_static(br#"{"model":7,"input":"hello"}"#)),
            None,
        );
        let _ = wait_for_replay_body_sticky_key_probe(
            &unsafe_payload.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert!(unsafe_payload.configure(LiveResponsesBodyTransformConfig {
            target_encoding: RequestBodyContentEncoding::Identity,
            compression_level: RequestCompressionLevelPreset::Balanced,
            enforce_include_usage: false,
            oauth: None,
            fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
            image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
            codex_imagegen_rewrite_mode: CodexImagegenRewriteMode::KeepOriginal,
            codex_imagegen_protocol: None,
            model_mapping_target: Some("upstream-model".to_string()),
        }));
        let mut upstream = unsafe_payload.body.into_data_stream();
        let error = loop {
            let Some(chunk) = upstream.next().await else {
                panic!("unsafe mapped payload must fail before upstream forwarding");
            };
            if let Err(error) = chunk {
                break error;
            }
        };
        assert!(error.to_string().contains("top-level string model field"));
        assert_eq!(
            unsafe_payload
                .request_body_error_rx
                .borrow()
                .as_ref()
                .map(|error| error.status),
            Some(StatusCode::BAD_REQUEST)
        );
    }

    #[tokio::test]
    async fn live_first_cancellation_and_failover_rejects_malformed_json() {
        let pipeline = spawn_live_responses_request_body_pipeline(
            Body::from(Bytes::from_static(br#"{"model":"gpt-5.6","input":"#)),
            None,
        );
        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert!(probe.sticky_key.is_none());
        assert!(probe.prompt_cache_key.is_none());
        assert!(probe.model.is_none());
        assert!(!probe.contains_encrypted_content);
        assert_eq!(probe.image_intent, ImageIntent::Unknown);
        let mut stream = pipeline.body.into_data_stream();
        let mut observed_error = None;
        while let Some(chunk) = stream.next().await {
            if let Err(err) = chunk {
                observed_error = Some(err);
                break;
            }
        }
        assert!(
            observed_error.is_some(),
            "malformed body should cancel upstream transfer"
        );
        let error = pipeline
            .request_body_error_rx
            .borrow()
            .clone()
            .expect("malformed body should be reported to dispatch");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn final_route_gate_rejects_trailing_content_before_upstream_body_starts() {
        let (request_body_tx, request_body_rx) = mpsc::channel::<Result<Bytes, io::Error>>(2);
        request_body_tx
            .send(Ok(Bytes::from_static(
                br#"{"model":"gpt-5.6","input":"hello"}"#,
            )))
            .await
            .expect("send complete request root before trailing content");
        let pipeline = spawn_live_responses_request_body_pipeline(
            Body::from_stream(ReceiverStream::new(request_body_rx)),
            None,
        );
        let request_body_error_rx = pipeline.request_body_error_rx.clone();
        request_body_tx
            .send(Ok(Bytes::from_static(b" trailing")))
            .await
            .expect("send invalid trailing content after upstream body starts");
        drop(request_body_tx);

        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert!(probe.model.is_none());
        let mut upstream_body = pipeline.body.into_data_stream();
        let error = upstream_body
            .next()
            .await
            .expect("invalid body should produce an error")
            .expect_err("no upstream body bytes may be produced before validation");
        assert!(error.to_string().contains("trailing content"));
        assert_eq!(
            request_body_error_rx
                .borrow()
                .as_ref()
                .map(|error| error.status),
            Some(StatusCode::BAD_REQUEST)
        );
    }

    #[tokio::test]
    async fn live_first_cancellation_and_failover_rejects_truncated_or_non_utf8_strings() {
        let mut non_utf8 = br#"{"model":"gpt-5.6","input":""#.to_vec();
        non_utf8.push(0xff);
        non_utf8.extend_from_slice(br#""}"#);
        for source in [
            Bytes::from_static(br#"{"model":"gpt-5.6","input":"unterminated"#),
            Bytes::from(non_utf8),
        ] {
            let pipeline = spawn_live_responses_request_body_pipeline(Body::from(source), None);
            let probe = wait_for_replay_body_sticky_key_probe(
                &pipeline.routing_probe_rx,
                Duration::from_secs(1),
            )
            .await;
            assert!(probe.model.is_none());
            let mut stream = pipeline.body.into_data_stream();
            let error = loop {
                let Some(chunk) = stream.next().await else {
                    panic!("invalid string must stop the live body");
                };
                if let Err(error) = chunk {
                    break error;
                }
            };
            assert!(error.to_string().contains("request string"));
            assert_eq!(
                pipeline
                    .request_body_error_rx
                    .borrow()
                    .as_ref()
                    .map(|error| error.failure_kind),
                Some(PROXY_FAILURE_REQUEST_BODY_INVALID_JSON)
            );
        }
    }

    #[tokio::test]
    async fn live_first_cancellation_and_failover_rejects_invalid_nested_json() {
        let pipeline = spawn_live_responses_request_body_pipeline(
            Body::from(Bytes::from_static(
                br#"{"model":"gpt-5.6","input":{"nested":truX}}"#,
            )),
            None,
        );
        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert!(probe.sticky_key.is_none());
        assert!(probe.prompt_cache_key.is_none());
        assert!(probe.model.is_none());
        assert!(!probe.contains_encrypted_content);
        assert_eq!(probe.image_intent, ImageIntent::Unknown);
        let mut stream = pipeline.body.into_data_stream();
        let mut observed_error = None;
        while let Some(chunk) = stream.next().await {
            if let Err(err) = chunk {
                observed_error = Some(err);
                break;
            }
        }
        assert!(
            observed_error.is_some(),
            "invalid nested JSON must stop live body"
        );
        assert_eq!(
            pipeline
                .request_body_error_rx
                .borrow()
                .as_ref()
                .map(|error| error.status),
            Some(StatusCode::BAD_REQUEST)
        );
    }

    #[tokio::test]
    async fn final_route_gate_waits_for_late_input_after_ordinary_fields() {
        let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(2);
        tx.send(Ok(Bytes::from_static(
            br#"{"model":"gpt-5.6","stream":true,"#,
        )))
        .await
        .expect("send ordinary prefix before input");
        let pipeline = spawn_live_responses_request_body_pipeline(
            Body::from_stream(ReceiverStream::new(rx)),
            None,
        );

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(matches!(
            *pipeline.routing_probe_rx.borrow(),
            PoolReplayBodyStickyKeyProbeStatus::Pending
        ));

        tx.send(Ok(Bytes::from_static(
            br#""input":[{"type":"input_text","text":"hello"}]}"#,
        )))
        .await
        .expect("send late route-affecting input");
        drop(tx);
        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(probe.model.as_deref(), Some("gpt-5.6"));
    }

    #[tokio::test]
    async fn live_first_cancellation_and_failover_rejects_extra_array_delimiter() {
        let pipeline = spawn_live_responses_request_body_pipeline(
            Body::from(Bytes::from_static(br#"{"model":"gpt-5.6","input":[{}]]}"#)),
            None,
        );
        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert!(probe.model.is_none());

        let mut stream = pipeline.body.into_data_stream();
        let _error = loop {
            let Some(chunk) = stream.next().await else {
                panic!("extra array delimiter must stop the live body");
            };
            if let Err(error) = chunk {
                break error;
            }
        };
        assert_eq!(
            pipeline
                .request_body_error_rx
                .borrow()
                .as_ref()
                .map(|error| error.status),
            Some(StatusCode::BAD_REQUEST)
        );
    }

    #[tokio::test]
    async fn live_first_cancellation_and_failover_rejects_trailing_comma_after_input_array() {
        let pipeline = spawn_live_responses_request_body_pipeline(
            Body::from(Bytes::from_static(br#"{"model":"gpt-5.6","input":[{}],}"#)),
            None,
        );
        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert!(probe.model.is_none());

        let mut stream = pipeline.body.into_data_stream();
        let _error = loop {
            let Some(chunk) = stream.next().await else {
                panic!("trailing object comma must stop the live body");
            };
            if let Err(error) = chunk {
                break error;
            }
        };
        assert_eq!(
            pipeline
                .request_body_error_rx
                .borrow()
                .as_ref()
                .map(|error| error.status),
            Some(StatusCode::BAD_REQUEST)
        );
    }

    #[tokio::test]
    async fn live_first_cancellation_and_failover_rejects_trailing_comma_inside_input_object() {
        let pipeline = spawn_live_responses_request_body_pipeline(
            Body::from(Bytes::from_static(
                br#"{"model":"gpt-5.6","input":[{"content":{"text":"hello",}}]}"#,
            )),
            None,
        );
        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert!(probe.model.is_none());

        let mut stream = pipeline.body.into_data_stream();
        let _error = loop {
            let Some(chunk) = stream.next().await else {
                panic!("trailing nested object comma must stop the live body");
            };
            if let Err(error) = chunk {
                break error;
            }
        };
        assert_eq!(
            pipeline
                .request_body_error_rx
                .borrow()
                .as_ref()
                .map(|error| error.status),
            Some(StatusCode::BAD_REQUEST)
        );
    }

    #[tokio::test]
    async fn final_route_gate_rejects_invalid_json_without_upstream_prefix() {
        let (tx, rx) = mpsc::channel::<Result<Bytes, io::Error>>(2);
        tx.send(Ok(Bytes::from_static(
            br#"{"model":"gpt-5.6","input":"ready"}"#,
        )))
        .await
        .expect("send complete routing object");
        let pipeline = spawn_live_responses_request_body_pipeline(
            Body::from_stream(ReceiverStream::new(rx)),
            None,
        );
        tx.send(Ok(Bytes::from_static(b"x")))
            .await
            .expect("send invalid trailing JSON");
        drop(tx);
        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert!(probe.model.is_none());
        let error = pipeline
            .body
            .into_data_stream()
            .next()
            .await
            .expect("invalid body should produce an error")
            .expect_err("invalid trailing JSON must not have an upstream prefix");
        assert!(error.to_string().contains("trailing content"));
        assert_eq!(
            pipeline
                .request_body_error_rx
                .borrow()
                .as_ref()
                .map(|error| error.status),
            Some(StatusCode::BAD_REQUEST)
        );
    }

    #[tokio::test]
    async fn live_first_capture_responses_falls_back_when_model_exceeds_probe_budget() {
        let mut body = br#"{"padding":""#.to_vec();
        body.resize(LIVE_ROUTE_PREFIX_BUFFER_BYTES + 64, b'x');
        body.extend_from_slice(br#"","model":"gpt-5.6"}"#);
        let pipeline =
            spawn_live_responses_request_body_pipeline(Body::from(Bytes::from(body)), None);
        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert!(probe.model.is_none());
        assert!(pipeline.request_body_error_rx.borrow().is_none());
    }

    #[tokio::test]
    async fn final_route_gate_falls_back_for_large_root_field() {
        let mut input = String::with_capacity(LIVE_ROUTE_PREFIX_BUFFER_BYTES + 64);
        input.extend(std::iter::repeat_n(
            'x',
            LIVE_ROUTE_PREFIX_BUFFER_BYTES + 64,
        ));
        let mut body = br#"{"model":"gpt-5.6","input":""#.to_vec();
        body.extend_from_slice(input.as_bytes());
        body.extend_from_slice(br#"","tools":[]}"#);
        let mut pipeline =
            spawn_live_responses_request_body_pipeline(Body::from(Bytes::from(body)), None);
        let probe = wait_for_replay_body_sticky_key_probe(
            &pipeline.routing_probe_rx,
            Duration::from_secs(1),
        )
        .await;
        assert!(probe.model.is_none());
        assert!(
            !pipeline.configure(LiveResponsesBodyTransformConfig {
                target_encoding: RequestBodyContentEncoding::Identity,
                compression_level: RequestCompressionLevelPreset::Balanced,
                enforce_include_usage: false,
                oauth: None,
                fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
                image_tool_rewrite_mode: ImageToolRewriteMode::ForceAdd,
                codex_imagegen_rewrite_mode: CodexImagegenRewriteMode::KeepOriginal,
                codex_imagegen_protocol: None,
                model_mapping_target: None,
            }),
            "a probe-budget fallback must not create a live upstream body"
        );
        assert!(pipeline.request_body_error_rx.borrow().is_none());
    }

    #[tokio::test]
    async fn responses_route_finalization_transform_decodes_supported_content_encodings() {
        let source = br#"{"model":"gpt-5.6","input":"compressed"}"#;
        let gzip = {
            let mut encoder = flate2::write::GzEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(source).expect("gzip source");
            Bytes::from(encoder.finish().expect("finish gzip"))
        };
        let deflate = {
            let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(source).expect("deflate source");
            Bytes::from(encoder.finish().expect("finish deflate"))
        };
        let zstd = Bytes::from(zstd::encode_all(source.as_slice(), 1).expect("zstd source"));

        for (encoding, body) in [("gzip", gzip), ("deflate", deflate), ("zstd", zstd)] {
            let mut pipeline = spawn_live_responses_request_body_pipeline(
                Body::from(body),
                Some(encoding.to_string()),
            );
            let probe = wait_for_replay_body_sticky_key_probe(
                &pipeline.routing_probe_rx,
                Duration::from_secs(1),
            )
            .await;
            assert_eq!(probe.model.as_deref(), Some("gpt-5.6"), "{encoding}");
            assert!(pipeline.configure(LiveResponsesBodyTransformConfig {
                target_encoding: RequestBodyContentEncoding::Identity,
                compression_level: RequestCompressionLevelPreset::Balanced,
                enforce_include_usage: false,
                oauth: None,
                fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
                image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
                codex_imagegen_rewrite_mode: CodexImagegenRewriteMode::KeepOriginal,
                codex_imagegen_protocol: None,
                model_mapping_target: None,
            }));
            let output = collect_body(pipeline.body).await;
            assert_eq!(
                serde_json::from_slice::<Value>(&output).expect("decode transformed body"),
                serde_json::from_slice::<Value>(source).expect("decode source"),
                "{encoding}"
            );
        }
    }

    #[tokio::test]
    async fn responses_route_finalization_transform_keeps_corrupt_content_encoding_out_of_json_failure()
     {
        let pipeline = spawn_live_responses_request_body_pipeline(
            Body::from(Bytes::from_static(b"not a gzip stream")),
            Some("gzip".to_string()),
        );
        let mut upstream = pipeline.body.into_data_stream();
        let _error = timeout(Duration::from_secs(1), upstream.next())
            .await
            .expect("corrupt gzip should resolve")
            .expect("corrupt gzip should emit an upstream body error")
            .expect_err("corrupt gzip must not produce a logical request body");
        let mut finalization_rx = pipeline.finalization_rx.clone();
        timeout(Duration::from_secs(1), async {
            while !*finalization_rx.borrow() {
                finalization_rx
                    .changed()
                    .await
                    .expect("live pipeline finalization sender should remain available");
            }
        })
        .await
        .expect("corrupt gzip pipeline should finalize");
        let request_body_error = pipeline
            .request_body_error_rx
            .borrow()
            .clone()
            .expect("corrupt gzip should be audited as a body decode failure");
        assert_eq!(
            request_body_error.failure_kind,
            PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED
        );
        assert!(
            !request_body_error
                .message
                .contains("request body must be valid JSON")
        );
    }

    #[tokio::test]
    async fn responses_route_finalization_transform_reencodes_supported_content_encodings() {
        let source = br#"{"model":"gpt-5.6","input":"reencoded"}"#;
        for (encoding, target) in [
            ("gzip", RequestBodyContentEncoding::Gzip),
            (
                "deflate",
                RequestBodyContentEncoding::Deflate { zlib_wrapper: true },
            ),
            ("zstd", RequestBodyContentEncoding::Zstd),
        ] {
            let mut pipeline = spawn_live_responses_request_body_pipeline(
                Body::from(Bytes::copy_from_slice(source)),
                None,
            );
            let probe = wait_for_replay_body_sticky_key_probe(
                &pipeline.routing_probe_rx,
                Duration::from_secs(1),
            )
            .await;
            assert_eq!(probe.model.as_deref(), Some("gpt-5.6"), "{encoding}");
            assert!(pipeline.configure(LiveResponsesBodyTransformConfig {
                target_encoding: target,
                compression_level: RequestCompressionLevelPreset::Balanced,
                enforce_include_usage: false,
                oauth: None,
                fast_mode_rewrite_mode: TagFastModeRewriteMode::KeepOriginal,
                image_tool_rewrite_mode: ImageToolRewriteMode::KeepOriginal,
                codex_imagegen_rewrite_mode: CodexImagegenRewriteMode::KeepOriginal,
                codex_imagegen_protocol: None,
                model_mapping_target: None,
            }));
            let output = collect_body(pipeline.body).await;
            let decoded =
                decode_request_payload_bytes(&output, target).expect("decode re-encoded body");
            assert_eq!(decoded.as_ref(), source, "{encoding}");
        }
    }

    #[test]
    fn responses_route_finalization_transform_supports_existing_request_encodings() {
        assert_eq!(
            live_responses_target_request_content_encoding(
                Some("gzip"),
                RequestCompressionAlgorithm::Follow
            )
            .expect("gzip follow"),
            RequestBodyContentEncoding::Gzip
        );
        assert_eq!(
            live_responses_target_request_content_encoding(
                Some("zstd"),
                RequestCompressionAlgorithm::Identity
            )
            .expect("identity rewrite"),
            RequestBodyContentEncoding::Identity
        );
        assert_eq!(
            live_responses_target_request_content_encoding(
                Some("deflate"),
                RequestCompressionAlgorithm::Follow
            )
            .expect("deflate follow")
            .as_str(),
            "deflate"
        );
        assert_eq!(
            live_responses_target_request_content_encoding_with_resolved(
                RequestBodyContentEncoding::Deflate {
                    zlib_wrapper: false,
                },
                RequestCompressionAlgorithm::Follow,
            )
            .expect("raw deflate follow"),
            RequestBodyContentEncoding::Deflate {
                zlib_wrapper: false,
            }
        );
    }
}
