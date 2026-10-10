use super::*;

pub(crate) async fn read_request_body_with_limit(
    body: Body,
    body_limit: usize,
    request_read_timeout: Duration,
    proxy_request_id: u64,
) -> Result<Vec<u8>, RequestBodyReadError> {
    let mut data = Vec::new();
    let mut stream = body.into_data_stream();
    let read_deadline = Instant::now() + request_read_timeout;

    loop {
        let remaining = read_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            warn!(
                proxy_request_id,
                timeout_ms = request_read_timeout.as_millis(),
                read_bytes = data.len(),
                "openai proxy request body read timed out"
            );
            return Err(RequestBodyReadError {
                status: StatusCode::REQUEST_TIMEOUT,
                message: format!(
                    "request body read timed out after {}ms",
                    request_read_timeout.as_millis()
                ),
                failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                partial_body: data,
            });
        }

        let next_chunk = match timeout(remaining, stream.next()).await {
            Ok(chunk) => chunk,
            Err(_) => {
                warn!(
                    proxy_request_id,
                    timeout_ms = request_read_timeout.as_millis(),
                    read_bytes = data.len(),
                    "openai proxy request body read timed out"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::REQUEST_TIMEOUT,
                    message: format!(
                        "request body read timed out after {}ms",
                        request_read_timeout.as_millis()
                    ),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                    partial_body: data,
                });
            }
        };

        let Some(chunk) = next_chunk else {
            break;
        };
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(err) => {
                warn!(
                    proxy_request_id,
                    error = %err,
                    read_bytes = data.len(),
                    "openai proxy request body stream error"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::BAD_REQUEST,
                    message: format!("failed to read request body stream: {err}"),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED,
                    partial_body: data,
                });
            }
        };

        if data.len().saturating_add(chunk.len()) > body_limit {
            let allowed = body_limit.saturating_sub(data.len());
            if allowed > 0 {
                data.extend_from_slice(&chunk[..allowed.min(chunk.len())]);
            }
            return Err(RequestBodyReadError {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: format!("request body exceeds {body_limit} bytes"),
                failure_kind: PROXY_FAILURE_BODY_TOO_LARGE,
                partial_body: data,
            });
        }

        data.extend_from_slice(&chunk);
    }

    Ok(data)
}

pub(crate) async fn read_request_body_snapshot_with_limit(
    body: Body,
    body_limit: usize,
    request_read_timeout: Duration,
    proxy_request_id: u64,
) -> Result<PoolReplayBodySnapshot, RequestBodyReadError> {
    let mut buffer = PoolReplayBodyBuffer::new(proxy_request_id);
    let mut stream = body.into_data_stream();
    let read_deadline = Instant::now() + request_read_timeout;
    let mut data_len = 0usize;

    loop {
        let remaining = read_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            warn!(
                proxy_request_id,
                timeout_ms = request_read_timeout.as_millis(),
                read_bytes = data_len,
                "openai proxy request body read timed out"
            );
            return Err(RequestBodyReadError {
                status: StatusCode::REQUEST_TIMEOUT,
                message: format!(
                    "request body read timed out after {}ms",
                    request_read_timeout.as_millis()
                ),
                failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                partial_body: Vec::new(),
            });
        }

        let next_chunk = match timeout(remaining, stream.next()).await {
            Ok(chunk) => chunk,
            Err(_) => {
                warn!(
                    proxy_request_id,
                    timeout_ms = request_read_timeout.as_millis(),
                    read_bytes = data_len,
                    "openai proxy request body read timed out"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::REQUEST_TIMEOUT,
                    message: format!(
                        "request body read timed out after {}ms",
                        request_read_timeout.as_millis()
                    ),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                    partial_body: Vec::new(),
                });
            }
        };

        let Some(chunk) = next_chunk else {
            return buffer.finish().await.map_err(|err| RequestBodyReadError {
                status: StatusCode::BAD_GATEWAY,
                message: format!("failed to cache request body for oauth replay: {err}"),
                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                partial_body: Vec::new(),
            });
        };

        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(err) => {
                warn!(
                    proxy_request_id,
                    error = %err,
                    read_bytes = data_len,
                    "openai proxy request body stream error"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::BAD_REQUEST,
                    message: format!("failed to read request body stream: {err}"),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED,
                    partial_body: Vec::new(),
                });
            }
        };

        if data_len.saturating_add(chunk.len()) > body_limit {
            return Err(RequestBodyReadError {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: format!("request body exceeds {body_limit} bytes"),
                failure_kind: PROXY_FAILURE_BODY_TOO_LARGE,
                partial_body: Vec::new(),
            });
        }
        data_len = data_len.saturating_add(chunk.len());

        buffer
            .append(&chunk)
            .await
            .map_err(|err| RequestBodyReadError {
                status: StatusCode::BAD_GATEWAY,
                message: format!("failed to cache request body for oauth replay: {err}"),
                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                partial_body: Vec::new(),
            })?;
    }
}

pub(crate) async fn read_request_body_snapshot_with_partial_limit(
    body: Body,
    body_limit: usize,
    request_read_timeout: Duration,
    proxy_request_id: u64,
) -> Result<PoolReplayBodySnapshot, RequestBodyReadError> {
    const ERROR_PARTIAL_BODY_LIMIT_BYTES: usize = 64 * 1024;

    let mut buffer = PoolReplayBodyBuffer::new(proxy_request_id);
    let mut partial_body = Vec::new();
    let mut stream = body.into_data_stream();
    let read_deadline = Instant::now() + request_read_timeout;
    let mut data_len = 0usize;

    loop {
        let remaining = read_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            warn!(
                proxy_request_id,
                timeout_ms = request_read_timeout.as_millis(),
                read_bytes = data_len,
                "openai proxy request body read timed out"
            );
            return Err(RequestBodyReadError {
                status: StatusCode::REQUEST_TIMEOUT,
                message: format!(
                    "request body read timed out after {}ms",
                    request_read_timeout.as_millis()
                ),
                failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                partial_body,
            });
        }

        let next_chunk = match timeout(remaining, stream.next()).await {
            Ok(chunk) => chunk,
            Err(_) => {
                warn!(
                    proxy_request_id,
                    timeout_ms = request_read_timeout.as_millis(),
                    read_bytes = data_len,
                    "openai proxy request body read timed out"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::REQUEST_TIMEOUT,
                    message: format!(
                        "request body read timed out after {}ms",
                        request_read_timeout.as_millis()
                    ),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_READ_TIMEOUT,
                    partial_body,
                });
            }
        };

        let Some(chunk) = next_chunk else {
            return buffer.finish().await.map_err(|err| RequestBodyReadError {
                status: StatusCode::BAD_GATEWAY,
                message: format!("failed to cache request body for replay: {err}"),
                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                partial_body,
            });
        };

        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(err) => {
                warn!(
                    proxy_request_id,
                    error = %err,
                    read_bytes = data_len,
                    "openai proxy request body stream error"
                );
                return Err(RequestBodyReadError {
                    status: StatusCode::BAD_REQUEST,
                    message: format!("failed to read request body stream: {err}"),
                    failure_kind: PROXY_FAILURE_REQUEST_BODY_STREAM_ERROR_CLIENT_CLOSED,
                    partial_body,
                });
            }
        };

        if data_len.saturating_add(chunk.len()) > body_limit {
            let allowed = body_limit.saturating_sub(data_len);
            if allowed > 0 {
                append_bounded_partial_body(
                    &mut partial_body,
                    &chunk[..allowed.min(chunk.len())],
                    ERROR_PARTIAL_BODY_LIMIT_BYTES,
                );
            }
            return Err(RequestBodyReadError {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: format!("request body exceeds {body_limit} bytes"),
                failure_kind: PROXY_FAILURE_BODY_TOO_LARGE,
                partial_body,
            });
        }
        data_len = data_len.saturating_add(chunk.len());
        append_bounded_partial_body(&mut partial_body, &chunk, ERROR_PARTIAL_BODY_LIMIT_BYTES);

        buffer
            .append(&chunk)
            .await
            .map_err(|err| RequestBodyReadError {
                status: StatusCode::BAD_GATEWAY,
                message: format!("failed to cache request body for replay: {err}"),
                failure_kind: PROXY_FAILURE_FAILED_CONTACT_UPSTREAM,
                partial_body: partial_body.clone(),
            })?;
    }
}

pub(crate) fn append_bounded_partial_body(partial_body: &mut Vec<u8>, chunk: &[u8], limit: usize) {
    let remaining = limit.saturating_sub(partial_body.len());
    if remaining > 0 {
        partial_body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }
}
