pub(crate) fn proxy_capture_invocation_status(
    status: StatusCode,
    has_error_message: bool,
    _pure_downstream_closed: bool,
) -> String {
    if status.is_success() && !has_error_message {
        "success".to_string()
    } else {
        format!("http_{}", status.as_u16())
    }
}

pub(crate) fn pool_capture_attempt_status(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
    pure_downstream_closed: bool,
) -> &'static str {
    if pure_downstream_closed {
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    } else if stream_error {
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_TRANSPORT_FAILURE
    } else if !status.is_success() || logical_stream_failure {
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_HTTP_FAILURE
    } else {
        POOL_UPSTREAM_REQUEST_ATTEMPT_STATUS_SUCCESS
    }
}

pub(crate) fn proxy_forward_response_failure_kind(
    status: StatusCode,
    stream_error: bool,
) -> Option<&'static str> {
    if stream_error {
        Some(FORWARD_PROXY_FAILURE_STREAM_ERROR)
    } else if status == StatusCode::TOO_MANY_REQUESTS {
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429)
    } else if status.is_server_error() {
        Some(FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX)
    } else {
        None
    }
}

pub(crate) fn proxy_capture_response_failure_kind(
    status: StatusCode,
    stream_error: bool,
    logical_stream_failure: bool,
) -> Option<&'static str> {
    if logical_stream_failure {
        Some(PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED)
    } else {
        proxy_forward_response_failure_kind(status, stream_error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpstreamAccountFailureDisposition {
    HardUnavailable,
    RateLimited,
    Retryable,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct UpstreamAccountHttpFailureClassification {
    pub(crate) disposition: UpstreamAccountFailureDisposition,
    pub(crate) failure_kind: &'static str,
    pub(crate) reason_code: &'static str,
    pub(crate) next_account_status: Option<&'static str>,
}

pub(crate) fn upstream_error_indicates_quota_exhausted(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    [
        "insufficient_quota",
        "quota exhausted",
        "quota_exhausted",
        "the usage limit has been reached",
        "usage limit has been reached",
        "usage limit reached",
        "billing",
        "payment required",
        "subscription required",
        "weekly cap",
        "weekly limit",
        "plan limit",
        "plan quota",
        "check your plan",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

pub(crate) fn upstream_error_code_is_server_overloaded(code: Option<&str>) -> bool {
    code.is_some_and(|value| value.eq_ignore_ascii_case(UPSTREAM_ERROR_CODE_SERVER_IS_OVERLOADED))
}

fn upstream_error_message_indicates_concurrency_limit(message: Option<&str>) -> bool {
    let normalized = message.unwrap_or_default().to_ascii_lowercase();
    let has_concurrency_term =
        normalized.contains("concurrency") || normalized.contains("concurrent");
    let has_limit_term = normalized.contains("limit") || normalized.contains("request");
    let has_exhaustion_term = normalized.contains("exceed")
        || normalized.contains("too many")
        || normalized.contains("maximum")
        || normalized.contains("reached")
        || normalized.contains("hit");
    has_concurrency_term && has_limit_term && has_exhaustion_term
}

pub(crate) fn upstream_error_is_retryable_responses_overload(
    code: Option<&str>,
    message: Option<&str>,
) -> bool {
    if upstream_error_code_is_server_overloaded(code) {
        return true;
    }

    code.is_some_and(|value| value.eq_ignore_ascii_case(UPSTREAM_ERROR_CODE_RATE_LIMIT_EXCEEDED))
        && upstream_error_message_indicates_concurrency_limit(message)
}

pub(crate) fn route_http_failure_is_retryable_responses_overload(
    status: StatusCode,
    error_message: &str,
) -> bool {
    if status != StatusCode::OK {
        return false;
    }

    let prefix = format!("[{}] ", PROXY_FAILURE_UPSTREAM_RESPONSE_FAILED);
    let Some(details) = error_message.strip_prefix(prefix.as_str()) else {
        return false;
    };
    let (code, message) = details
        .split_once(": ")
        .map_or((details, None), |(code, message)| (code, Some(message)));
    upstream_error_is_retryable_responses_overload(Some(code.trim()), message.map(str::trim))
}

pub(crate) fn response_info_is_retryable_responses_overload(
    status: StatusCode,
    response_info: &ResponseCaptureInfo,
) -> bool {
    status == StatusCode::OK
        && response_info.stream_terminal_event.is_some()
        && upstream_error_is_retryable_responses_overload(
            response_info.upstream_error_code.as_deref(),
            response_info.upstream_error_message.as_deref(),
        )
}

pub(crate) fn extract_unsupported_model_from_route_error(
    status: StatusCode,
    error_message: &str,
) -> Option<String> {
    static UNSUPPORTED_MODEL_CONTEXT_REGEX: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r#"(?xi)
            unsupported[_\s]+model\s*[:=]\s*['"`]?([a-z0-9][a-z0-9._-]{0,127})['"`]?
            |
            model(?:\s+id)?\s+['"`]?([a-z0-9][a-z0-9._-]{0,127})['"`]?\s+is\s+not\s+supported\b
            |
            model\s+is\s+not\s+supported\s*[:=]\s*['"`]?([a-z0-9][a-z0-9._-]{0,127})['"`]?
            "#,
        )
        .expect("valid unsupported model context regex")
    });
    if status != StatusCode::BAD_REQUEST {
        return None;
    }
    let normalized = error_message.to_ascii_lowercase();
    if !(normalized.contains("unsupported_model")
        || normalized.contains("unsupported model")
        || normalized.contains("model is not supported")
        || normalized.contains("is not supported")
        || normalized.contains("unsupported model"))
    {
        return None;
    }
    if normalized.contains("for model")
        && !normalized.contains("model is not supported")
        && !normalized.contains("unsupported model")
    {
        return None;
    }
    UNSUPPORTED_MODEL_CONTEXT_REGEX
        .captures_iter(error_message)
        .filter_map(|captures| (1..=3).find_map(|index| captures.get(index)))
        .map(|value| value.as_str().trim().to_string())
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .any(|byte| byte.is_ascii_digit() || matches!(byte, b'-' | b'.'))
        })
        .last()
}

pub(crate) fn classify_pool_account_http_failure(
    account_kind: &str,
    status: StatusCode,
    error_message: &str,
) -> UpstreamAccountHttpFailureClassification {
    if status == StatusCode::TOO_MANY_REQUESTS
        && upstream_error_indicates_quota_exhausted(error_message)
    {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::RateLimited,
            failure_kind: FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            reason_code: "upstream_http_429_quota_exhausted",
            next_account_status: None,
        };
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::RateLimited,
            failure_kind: FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_429,
            reason_code: "upstream_http_429_rate_limit",
            next_account_status: None,
        };
    }
    if status == StatusCode::PAYMENT_REQUIRED {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::HardUnavailable,
            failure_kind: PROXY_FAILURE_UPSTREAM_HTTP_402,
            reason_code: "upstream_http_402",
            next_account_status: Some("error"),
        };
    }
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::Retryable,
            failure_kind: PROXY_FAILURE_UPSTREAM_HTTP_413,
            reason_code: "upstream_http_413",
            next_account_status: None,
        };
    }
    if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
        let next_account_status = if account_kind == "oauth_codex"
            && is_explicit_reauth_error_message(error_message)
            && !is_scope_permission_error_message(error_message)
            && !is_bridge_error_message(error_message)
        {
            Some("needs_reauth")
        } else {
            Some("error")
        };
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::HardUnavailable,
            failure_kind: PROXY_FAILURE_UPSTREAM_HTTP_AUTH,
            reason_code: if status == StatusCode::UNAUTHORIZED {
                "upstream_http_401"
            } else {
                "upstream_http_403"
            },
            next_account_status,
        };
    }
    if status.is_server_error() {
        return UpstreamAccountHttpFailureClassification {
            disposition: UpstreamAccountFailureDisposition::Retryable,
            failure_kind: FORWARD_PROXY_FAILURE_UPSTREAM_HTTP_5XX,
            reason_code: "upstream_http_5xx",
            next_account_status: None,
        };
    }
    UpstreamAccountHttpFailureClassification {
        disposition: UpstreamAccountFailureDisposition::Retryable,
        failure_kind: PROXY_FAILURE_POOL_NO_AVAILABLE_ACCOUNT,
        reason_code: "sync_error",
        next_account_status: None,
    }
}

pub(crate) fn compact_support_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    let has_compact_signal = normalized.contains("compact")
        || normalized.contains("responses/compact")
        || normalized.contains("gpt-5.4-openai-compact");
    if normalized.contains("no available channel for model") && has_compact_signal {
        return true;
    }
    has_compact_signal
        && [
            "unsupported model",
            "unsupported endpoint",
            "unsupported path",
            "unsupported route",
            "not support",
            "does not support",
            "is not supported",
            "unknown model",
            "model not found",
            "no channel",
        ]
        .iter()
        .any(|needle| normalized.contains(needle))
}

pub(crate) fn classify_compact_support_observation(
    original_uri: &Uri,
    status: Option<StatusCode>,
    message: Option<&str>,
) -> Option<CompactSupportObservation> {
    if original_uri.path() != "/v1/responses/compact" {
        return None;
    }
    match status {
        Some(code) if code.is_success() => Some(CompactSupportObservation {
            status: COMPACT_SUPPORT_STATUS_SUPPORTED,
            reason: Some("compact request succeeded".to_string()),
        }),
        _ => {
            let message = message
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string());
            if message
                .as_deref()
                .is_some_and(compact_support_negative_signal)
            {
                Some(CompactSupportObservation {
                    status: COMPACT_SUPPORT_STATUS_UNSUPPORTED,
                    reason: message,
                })
            } else {
                Some(CompactSupportObservation {
                    status: COMPACT_SUPPORT_STATUS_UNKNOWN,
                    reason: message,
                })
            }
        }
    }
}

fn capability_support_failure_signal(normalized: &str) -> bool {
    [
        "unsupported endpoint",
        "unsupported path",
        "unsupported route",
        "unsupported tool",
        "unsupported model",
        "does not support",
        "is not supported",
        "not support",
        "unknown model",
        "model not found",
        "no available channel for model",
        "no channel",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

pub(crate) fn response_endpoint_capability_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    capability_support_failure_signal(&normalized)
        && ["/v1/responses", "responses/compact"]
            .iter()
            .any(|needle| normalized.contains(needle))
}

pub(crate) fn chat_completions_capability_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    capability_support_failure_signal(&normalized)
        && ["/v1/chat/completions", "chat/completions"]
            .iter()
            .any(|needle| normalized.contains(needle))
}

pub(crate) fn standalone_search_capability_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    capability_support_failure_signal(&normalized)
        && ["/v1/alpha/search", "alpha/search", "standalone search"]
            .iter()
            .any(|needle| normalized.contains(needle))
}

pub(crate) fn classify_standalone_search_capability_observation(
    status: StatusCode,
    message: Option<&str>,
) -> CapabilitySupport {
    if status.is_success() {
        return CapabilitySupport::Supported;
    }
    if matches!(
        status,
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ) {
        return CapabilitySupport::Unsupported;
    }
    if status == StatusCode::BAD_REQUEST
        && message
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some_and(standalone_search_capability_negative_signal)
    {
        CapabilitySupport::Unsupported
    } else {
        CapabilitySupport::Unknown
    }
}

pub(crate) fn response_image_tool_capability_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    if is_responses_lite_top_level_image_tool_shape_error(&normalized) {
        return false;
    }
    if !capability_support_failure_signal(&normalized) {
        return false;
    }
    normalized.contains("image_generation")
        || normalized.contains("image generation")
        || normalized.contains("gpt-image-")
}

pub(crate) fn is_responses_lite_top_level_image_tool_shape_error(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("responses lite")
        && normalized.contains("top-level tool type")
        && normalized.contains("image_generation")
}

pub(crate) fn image_endpoint_capability_negative_signal(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    if !capability_support_failure_signal(&normalized) {
        return false;
    }
    normalized.contains("gpt-image-")
        || normalized.contains("/v1/images/")
        || normalized.contains("images/generations")
        || normalized.contains("images/edits")
}

pub(crate) fn classify_response_endpoint_capability_observation(
    status: StatusCode,
    message: Option<&str>,
) -> CapabilitySupport {
    if status.is_success() {
        return CapabilitySupport::Supported;
    }
    let normalized_message = message
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if matches!(
        status,
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ) && normalized_message
        .as_deref()
        .is_some_and(response_endpoint_capability_negative_signal)
    {
        CapabilitySupport::Unsupported
    } else {
        CapabilitySupport::Unknown
    }
}

pub(crate) fn classify_chat_completions_capability_observation(
    status: StatusCode,
    message: Option<&str>,
) -> CapabilitySupport {
    if status.is_success() {
        return CapabilitySupport::Supported;
    }
    let normalized_message = message
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if matches!(
        status,
        StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
    ) && normalized_message
        .as_deref()
        .is_some_and(chat_completions_capability_negative_signal)
    {
        CapabilitySupport::Unsupported
    } else {
        CapabilitySupport::Unknown
    }
}

pub(crate) fn classify_response_image_tool_capability_observation(
    status: StatusCode,
    message: Option<&str>,
) -> CapabilitySupport {
    if status.is_success() {
        return CapabilitySupport::Supported;
    }
    let normalized_message = message
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if status == StatusCode::BAD_REQUEST
        && normalized_message
            .as_deref()
            .is_some_and(response_image_tool_capability_negative_signal)
    {
        CapabilitySupport::Unsupported
    } else {
        CapabilitySupport::Unknown
    }
}

pub(crate) fn classify_image_endpoint_capability_observation(
    status: StatusCode,
    message: Option<&str>,
) -> CapabilitySupport {
    if status.is_success() {
        return CapabilitySupport::Supported;
    }
    let normalized_message = message
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if status == StatusCode::BAD_REQUEST
        && normalized_message
            .as_deref()
            .is_some_and(image_endpoint_capability_negative_signal)
    {
        CapabilitySupport::Unsupported
    } else {
        CapabilitySupport::Unknown
    }
}

pub(crate) fn fallback_proxy_429_retry_delay(retry_index: u32) -> Duration {
    let exponent = retry_index.saturating_sub(1).min(16);
    let multiplier = 1_u64 << exponent;
    Duration::from_millis(500_u64.saturating_mul(multiplier)).min(Duration::from_secs(5))
}

pub(crate) fn fallback_proxy_429_retry_delay_for_state(
    state: &AppState,
    retry_index: u32,
) -> Duration {
    #[cfg(not(test))]
    let _ = state;

    #[cfg(test)]
    if let Some(delay) = state.fallback_proxy_429_retry_delay_override {
        return delay;
    }

    fallback_proxy_429_retry_delay(retry_index)
}

pub(crate) fn pool_group_upstream_429_retry_delay(state: &AppState) -> Duration {
    if let Some(delay) = state.pool_group_429_retry_delay_override {
        return delay;
    }
    Duration::from_secs(rand::thread_rng().gen_range(
        MIN_POOL_GROUP_UPSTREAM_429_RETRY_DELAY_SECS..=MAX_POOL_GROUP_UPSTREAM_429_RETRY_DELAY_SECS,
    ))
}

pub(crate) const DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_TIMEOUT_SECS: u64 = 10;
pub(crate) const DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_POLL_INTERVAL_MS: u64 = 250;
pub(crate) const DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS: u64 = 10;
pub(crate) const POOL_NO_AVAILABLE_ACCOUNT_MESSAGE: &str = "no healthy pool account is available";

#[derive(Debug, Clone, Copy)]
pub(crate) struct PoolNoAvailableWaitSettings {
    pub(crate) timeout: Duration,
    pub(crate) poll_interval: Duration,
    pub(crate) retry_after_secs: u64,
}

impl Default for PoolNoAvailableWaitSettings {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_TIMEOUT_SECS),
            poll_interval: Duration::from_millis(
                DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_WAIT_POLL_INTERVAL_MS,
            ),
            retry_after_secs: DEFAULT_POOL_NO_AVAILABLE_ACCOUNT_RETRY_AFTER_SECS,
        }
    }
}

impl PoolNoAvailableWaitSettings {
    pub(crate) fn normalized_poll_interval(self) -> Duration {
        if self.poll_interval.is_zero() {
            Duration::from_millis(1)
        } else {
            self.poll_interval
        }
    }
}

#[derive(Debug)]
pub(crate) enum PoolAccountResolutionWithWait {
    Resolution(PoolAccountResolution),
    TotalTimeoutExpired,
}

pub(crate) const POOL_UPSTREAM_SAME_ACCOUNT_MAX_ATTEMPTS: u8 = 3;
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

async fn pool_replay_snapshot_from_bytes_with_memory_threshold(
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
