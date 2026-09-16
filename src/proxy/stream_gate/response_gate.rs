use super::*;
pub(crate) enum PoolInitialResponsesSseEventDecision {
    ContinueMetadata,
    Forward,
    RetrySameAccount {
        upstream_error_code: Option<String>,
        upstream_error_message: Option<String>,
        upstream_request_id: Option<String>,
    },
}

pub(crate) fn classify_pool_initial_responses_sse_event(
    status: StatusCode,
    event_bytes: &[u8],
) -> PoolInitialResponsesSseEventDecision {
    let response_info = parse_stream_response_payload(event_bytes);
    if response_info_is_retryable_responses_overload(status, &response_info) {
        return PoolInitialResponsesSseEventDecision::RetrySameAccount {
            upstream_error_code: response_info.upstream_error_code,
            upstream_error_message: response_info.upstream_error_message,
            upstream_request_id: response_info.upstream_request_id,
        };
    }

    match initial_sse_event_kind(event_bytes).as_deref() {
        None | Some("response.created" | "response.in_progress") => {
            PoolInitialResponsesSseEventDecision::ContinueMetadata
        }
        _ => PoolInitialResponsesSseEventDecision::Forward,
    }
}

pub(crate) fn rebuild_proxy_upstream_response_stream(
    status: StatusCode,
    headers: &HeaderMap,
    stream: Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, io::Error>> + Send>>,
) -> Result<ProxyUpstreamResponseBody, String> {
    let mut response_builder = Response::builder().status(status);
    for (name, value) in headers {
        response_builder = response_builder.header(name, value);
    }
    response_builder
        .body(Body::from_stream(stream))
        .map(ProxyUpstreamResponseBody::Axum)
        .map_err(|err| format!("failed to rebuild upstream response stream: {err}"))
}

pub(crate) enum PoolInitialResponseGateOutcome {
    Forward {
        response: ProxyUpstreamResponseBody,
        prefetched_bytes: Option<Bytes>,
        prefetched_bytes_received_at: Option<Instant>,
        replayed_bytes_received_at: Option<Instant>,
    },
    RetrySameAccount {
        message: String,
        upstream_error_code: Option<String>,
        upstream_error_message: Option<String>,
        upstream_request_id: Option<String>,
        raw_body: Bytes,
    },
}

pub(crate) async fn gate_pool_initial_response_stream(
    response: ProxyUpstreamResponseBody,
    prefetched_first_chunk: Option<Bytes>,
    total_timeout: Duration,
    started: Instant,
) -> Result<PoolInitialResponseGateOutcome, String> {
    gate_pool_initial_response_stream_with_timestamp(
        response,
        prefetched_first_chunk,
        None,
        total_timeout,
        started,
    )
    .await
}

pub(crate) async fn gate_pool_initial_response_stream_with_timestamp(
    response: ProxyUpstreamResponseBody,
    prefetched_first_chunk: Option<Bytes>,
    prefetched_first_chunk_received_at: Option<Instant>,
    total_timeout: Duration,
    started: Instant,
) -> Result<PoolInitialResponseGateOutcome, String> {
    let status = response.status();
    let headers = response.headers().clone();
    let scan = match scan_initial_response_stream(
        status,
        response.into_bytes_stream(),
        prefetched_first_chunk,
        prefetched_first_chunk_received_at,
        total_timeout,
        started,
    )
    .await
    {
        Ok(scan) => scan,
        Err(outcome) => return Ok(outcome),
    };
    let stream = scan.stream;
    let mut buffered = scan.buffered;
    let first_forward_event_start = scan.first_forward_event_start;
    let first_forward_event_received_at = scan.first_forward_event_received_at;

    let mut replay_prefix = None;
    if let Some(forward_event_start) = first_forward_event_start {
        let replay_bytes = buffered.split_off(forward_event_start);
        replay_prefix = (!replay_bytes.is_empty()).then_some(Bytes::from(replay_bytes));
    }
    let prefetched_bytes = (!buffered.is_empty()).then_some(Bytes::from(buffered));

    let remaining_stream: InitialResponseStream = if let Some(err) = scan.stream_error {
        if let Some(prefix) = replay_prefix {
            Box::pin(
                stream::once(async move { Ok(prefix) })
                    .chain(stream::once(async move { Err(err) })),
            )
        } else {
            Box::pin(stream::once(async move { Err(err) }))
        }
    } else if let Some(prefix) = replay_prefix {
        Box::pin(stream::once(async move { Ok(prefix) }).chain(stream))
    } else {
        stream
    };
    let rebuilt_response =
        rebuild_proxy_upstream_response_stream(status, &headers, remaining_stream)?;
    Ok(PoolInitialResponseGateOutcome::Forward {
        response: rebuilt_response,
        prefetched_bytes,
        prefetched_bytes_received_at: first_forward_event_received_at,
        replayed_bytes_received_at: first_forward_event_received_at,
    })
}

type InitialResponseStream =
    Pin<Box<dyn futures_util::Stream<Item = Result<Bytes, io::Error>> + Send>>;

struct InitialResponseScan {
    stream: InitialResponseStream,
    buffered: Vec<u8>,
    first_forward_event_start: Option<usize>,
    first_forward_event_received_at: Option<Instant>,
    stream_error: Option<io::Error>,
}

async fn scan_initial_response_stream(
    status: StatusCode,
    mut stream: InitialResponseStream,
    prefetched_first_chunk: Option<Bytes>,
    prefetched_first_chunk_received_at: Option<Instant>,
    total_timeout: Duration,
    started: Instant,
) -> Result<InitialResponseScan, PoolInitialResponseGateOutcome> {
    let mut buffered = Vec::new();
    let mut scanned_bytes = 0;
    let mut saw_non_metadata_event = false;
    let mut first_forward_event_start = None;
    let mut first_forward_event_received_at = None;
    let mut current_chunk_received_at = prefetched_first_chunk_received_at;
    if let Some(chunk) = prefetched_first_chunk {
        buffered.extend_from_slice(&chunk);
    }
    let mut stream_error = None;

    loop {
        while let Some(relative_event_end) =
            find_first_sse_event_boundary(&buffered[scanned_bytes..])
        {
            let event_end = scanned_bytes + relative_event_end;
            match classify_pool_initial_responses_sse_event(
                status,
                &buffered[scanned_bytes..event_end],
            ) {
                PoolInitialResponsesSseEventDecision::ContinueMetadata => scanned_bytes = event_end,
                PoolInitialResponsesSseEventDecision::Forward => {
                    first_forward_event_start = Some(scanned_bytes);
                    first_forward_event_received_at = current_chunk_received_at;
                    scanned_bytes = event_end;
                    saw_non_metadata_event = true;
                    break;
                }
                PoolInitialResponsesSseEventDecision::RetrySameAccount {
                    upstream_error_code,
                    upstream_error_message,
                    upstream_request_id,
                } => {
                    return Err(build_retryable_overload_gate_outcome(
                        upstream_error_code,
                        upstream_error_message,
                        upstream_request_id,
                        Bytes::copy_from_slice(&buffered),
                    ));
                }
            }
        }
        if saw_non_metadata_event || buffered.len() >= POOL_INITIAL_RESPONSE_GATE_BUFFER_LIMIT {
            break;
        }
        let Some(timeout_budget) = remaining_timeout_budget(total_timeout, started.elapsed())
        else {
            break;
        };
        let next_chunk = match timeout(timeout_budget, stream.next()).await {
            Ok(Some(next_chunk)) => next_chunk,
            Ok(None) | Err(_) => break,
        };
        current_chunk_received_at = Some(Instant::now());
        match next_chunk {
            Ok(chunk) => buffered.extend_from_slice(&chunk),
            Err(err) => {
                stream_error = Some(io::Error::other(err.to_string()));
                break;
            }
        }
    }

    Ok(InitialResponseScan {
        stream,
        buffered,
        first_forward_event_start,
        first_forward_event_received_at,
        stream_error,
    })
}

pub(crate) fn gate_pool_initial_compact_response(
    status: StatusCode,
    headers: &HeaderMap,
    prefetched_first_chunk: Option<&Bytes>,
) -> Option<PoolInitialResponseGateOutcome> {
    if status != StatusCode::OK {
        return None;
    }

    let first_chunk = prefetched_first_chunk?;
    if first_chunk.is_empty() {
        return None;
    }

    let content_encoding = headers
        .get(header::CONTENT_ENCODING)
        .and_then(|value| value.to_str().ok());
    let (decoded_first_chunk, _) =
        decode_response_payload_for_preview_parse(first_chunk.as_ref(), content_encoding);
    let value = serde_json::from_slice::<Value>(decoded_first_chunk.as_ref()).ok()?;
    let error_object = extract_upstream_error_object(&value)?;
    let upstream_error_code = error_object
        .get("code")
        .and_then(|entry| entry.as_str())
        .map(str::to_string);
    let upstream_error_message = extract_upstream_error_message(&value);
    if !upstream_error_is_retryable_responses_overload(
        upstream_error_code.as_deref(),
        upstream_error_message.as_deref(),
    ) {
        return None;
    }

    Some(build_retryable_overload_gate_outcome(
        upstream_error_code,
        upstream_error_message,
        extract_upstream_request_id(&value),
        first_chunk.clone(),
    ))
}

pub(crate) fn extract_request_id_from_message(message: &str) -> Option<String> {
    let lower_message = message.to_ascii_lowercase();
    let start = lower_message
        .find("request id ")
        .map(|index| index + "request id ".len())
        .or_else(|| {
            lower_message
                .find("request_id=")
                .map(|index| index + "request_id=".len())
        })
        .or_else(|| {
            lower_message
                .find("x-request-id: ")
                .map(|index| index + "x-request-id: ".len())
        })?;
    let tail = &message[start..];
    let request_id: String = tail
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || matches!(*ch, '-' | '_'))
        .collect();
    if request_id.is_empty() {
        None
    } else {
        Some(request_id)
    }
}
