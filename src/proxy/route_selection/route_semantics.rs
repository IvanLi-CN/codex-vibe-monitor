use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};

use super::*;

#[cfg(test)]
#[derive(Debug, Default, Deserialize)]
pub(crate) struct StickyKeyProjection {
    #[serde(default)]
    metadata: StickyKeyProjectionMetadata,
    #[serde(default)]
    sticky_key: Option<String>,
    #[serde(default, rename = "stickyKey")]
    sticky_key_alias: Option<String>,
    #[serde(default, rename = "prompt_cache_key")]
    prompt_cache_key: Option<String>,
    #[serde(default, rename = "promptCacheKey")]
    prompt_cache_key_alias: Option<String>,
}

#[cfg(test)]
#[derive(Debug, Default, Deserialize)]
pub(crate) struct StickyKeyProjectionMetadata {
    #[serde(default)]
    sticky_key: Option<String>,
    #[serde(default, rename = "stickyKey")]
    sticky_key_alias: Option<String>,
    #[serde(default, rename = "prompt_cache_key")]
    prompt_cache_key: Option<String>,
    #[serde(default, rename = "promptCacheKey")]
    prompt_cache_key_alias: Option<String>,
}

#[cfg(test)]
impl StickyKeyProjection {
    fn into_sticky_key(self) -> Option<String> {
        [
            self.metadata.sticky_key,
            self.metadata.sticky_key_alias,
            self.metadata.prompt_cache_key,
            self.metadata.prompt_cache_key_alias,
            self.sticky_key,
            self.sticky_key_alias,
            self.prompt_cache_key,
            self.prompt_cache_key_alias,
        ]
        .into_iter()
        .flatten()
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
    }
}

#[cfg(test)]
pub(crate) fn extract_sticky_key_from_request_body_projection(bytes: &[u8]) -> Option<String> {
    serde_json::from_slice::<StickyKeyProjection>(bytes)
        .ok()
        .and_then(StickyKeyProjection::into_sticky_key)
}

#[derive(Debug, Default)]
struct ReplaySnapshotRouteValue {
    value: Value,
    sticky_projection_valid: bool,
}

#[derive(Debug, Clone, Copy)]
enum ReplaySnapshotRouteValueScope {
    Root,
    Metadata,
    Nested,
}

impl ReplaySnapshotRouteValueScope {
    fn child(self, key: &str) -> Self {
        if matches!(self, Self::Root) && key == "metadata" {
            Self::Metadata
        } else {
            Self::Nested
        }
    }

    fn sticky_projection_key_bit(self, key: &str) -> Option<u8> {
        match self {
            Self::Root => match key {
                "metadata" => Some(1 << 0),
                "sticky_key" => Some(1 << 1),
                "stickyKey" => Some(1 << 2),
                "prompt_cache_key" => Some(1 << 3),
                "promptCacheKey" => Some(1 << 4),
                _ => None,
            },
            Self::Metadata => match key {
                "sticky_key" => Some(1 << 0),
                "stickyKey" => Some(1 << 1),
                "prompt_cache_key" => Some(1 << 2),
                "promptCacheKey" => Some(1 << 3),
                _ => None,
            },
            Self::Nested => None,
        }
    }
}

struct ReplaySnapshotRouteValueSeed<'a> {
    sticky_projection_valid: &'a mut bool,
    scope: ReplaySnapshotRouteValueScope,
}

struct ReplaySnapshotRouteValueVisitor<'a> {
    sticky_projection_valid: &'a mut bool,
    scope: ReplaySnapshotRouteValueScope,
}

impl<'de> DeserializeSeed<'de> for ReplaySnapshotRouteValueSeed<'_> {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(ReplaySnapshotRouteValueVisitor {
            sticky_projection_valid: self.sticky_projection_valid,
            scope: self.scope,
        })
    }
}

impl<'de> Visitor<'de> for ReplaySnapshotRouteValueVisitor<'_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("JSON number must be finite"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(value.to_string()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(ReplaySnapshotRouteValueSeed {
            sticky_projection_valid: &mut *self.sticky_projection_valid,
            scope: ReplaySnapshotRouteValueScope::Nested,
        })? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut object = serde_json::Map::new();
        let mut seen_sticky_projection_key_bits = 0_u8;
        while let Some(key) = map.next_key::<String>()? {
            if let Some(bit) = self.scope.sticky_projection_key_bit(&key) {
                if seen_sticky_projection_key_bits & bit != 0 {
                    *self.sticky_projection_valid = false;
                }
                seen_sticky_projection_key_bits |= bit;
            }
            let value = map.next_value_seed(ReplaySnapshotRouteValueSeed {
                sticky_projection_valid: &mut *self.sticky_projection_valid,
                scope: self.scope.child(&key),
            })?;
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
}

impl<'de> Deserialize<'de> for ReplaySnapshotRouteValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut sticky_projection_valid = true;
        let value = ReplaySnapshotRouteValueSeed {
            sticky_projection_valid: &mut sticky_projection_valid,
            scope: ReplaySnapshotRouteValueScope::Root,
        }
        .deserialize(deserializer)?;
        Ok(Self {
            value,
            sticky_projection_valid,
        })
    }
}

#[derive(Debug, Default)]
struct SelectiveRequestSemantics {
    root_sticky_key: Option<String>,
    root_sticky_key_alias: Option<String>,
    root_prompt_cache_key: Option<String>,
    root_prompt_cache_key_alias: Option<String>,
    metadata_sticky_key: Option<String>,
    metadata_sticky_key_alias: Option<String>,
    metadata_prompt_cache_key: Option<String>,
    metadata_prompt_cache_key_alias: Option<String>,
    model: Option<String>,
    stream: bool,
    service_tier: Option<String>,
    service_tier_alias: Option<String>,
    reasoning_effort: Option<String>,
    nested_reasoning_effort: Option<String>,
    contains_encrypted_content: bool,
    contains_image_generation: bool,
    contains_codex_image_generation: bool,
    declares_remote_v2_compaction: bool,
    stream_options_present: bool,
    invalid_sticky_projection: bool,
    invalid_prompt_cache_key: bool,
}

#[derive(Default)]
struct SemanticMarkerScanner {
    in_string: bool,
    escaped: bool,
    unicode_remaining: u8,
    unicode_value: u16,
    string_is_value: bool,
    string_len: usize,
    raw_string_bytes: usize,
    string_overflow: bool,
    string: [u8; 32],
    candidate: Option<([u8; 32], usize)>,
    value_key: Option<([u8; 32], usize)>,
    contains_encrypted_content: bool,
    contains_image_generation: bool,
    contains_codex_image_generation: bool,
    semantic_limit_exceeded: bool,
}

impl SemanticMarkerScanner {
    fn scan(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if self.in_string {
                self.raw_string_bytes = self.raw_string_bytes.saturating_add(1);
                let selected_value = self.string_is_value
                    && self.value_key.is_some_and(|(key, len)| {
                        matches!(
                            &key[..len],
                            b"model"
                                | b"sticky_key"
                                | b"stickyKey"
                                | b"prompt_cache_key"
                                | b"promptCacheKey"
                                | b"service_tier"
                                | b"serviceTier"
                                | b"reasoning_effort"
                                | b"effort"
                                | b"type"
                        )
                    });
                if selected_value && self.raw_string_bytes > REQUEST_SEMANTIC_STRING_MAX_BYTES {
                    self.semantic_limit_exceeded = true;
                }
                if self.unicode_remaining > 0 {
                    let Some(digit) = (byte as char).to_digit(16) else {
                        self.string_overflow = true;
                        self.unicode_remaining = 0;
                        continue;
                    };
                    self.unicode_value = (self.unicode_value << 4) | digit as u16;
                    self.unicode_remaining -= 1;
                    if self.unicode_remaining == 0 {
                        if self.unicode_value <= 0x7f && self.string_len < self.string.len() {
                            self.string[self.string_len] = self.unicode_value as u8;
                            self.string_len += 1;
                        } else {
                            self.string_overflow = true;
                        }
                    }
                    continue;
                }
                if self.escaped {
                    self.escaped = false;
                    if byte == b'u' {
                        self.unicode_remaining = 4;
                        self.unicode_value = 0;
                    } else {
                        let decoded = match byte {
                            b'"' | b'\\' | b'/' => byte,
                            b'b' => 0x08,
                            b'f' => 0x0c,
                            b'n' => b'\n',
                            b'r' => b'\r',
                            b't' => b'\t',
                            _ => {
                                self.string_overflow = true;
                                continue;
                            }
                        };
                        if self.string_len < self.string.len() {
                            self.string[self.string_len] = decoded;
                            self.string_len += 1;
                        } else {
                            self.string_overflow = true;
                        }
                    }
                    continue;
                }
                if byte == b'\\' {
                    self.escaped = true;
                    continue;
                }
                if byte == b'"' {
                    self.in_string = false;
                    if !self.string_overflow {
                        let captured = (self.string, self.string_len);
                        if self.string_is_value {
                            if self
                                .value_key
                                .is_some_and(|(key, len)| &key[..len] == b"type")
                            {
                                let value = &captured.0[..captured.1];
                                self.contains_encrypted_content |= value == b"encrypted_content";
                                self.contains_image_generation |= value == b"image_generation";
                                self.contains_codex_image_generation |=
                                    value == b"image_gen.imagegen";
                            }
                            self.value_key = None;
                        } else {
                            self.candidate = Some(captured);
                        }
                    } else if self.string_is_value {
                        self.value_key = None;
                    }
                    continue;
                }
                if self.string_len < self.string.len() {
                    self.string[self.string_len] = byte;
                    self.string_len += 1;
                } else {
                    self.string_overflow = true;
                }
                continue;
            }

            if byte == b'"' {
                self.in_string = true;
                self.escaped = false;
                self.unicode_remaining = 0;
                self.unicode_value = 0;
                self.string_len = 0;
                self.raw_string_bytes = 0;
                self.string_overflow = false;
                self.string_is_value = self.value_key.is_some();
                continue;
            }
            if byte.is_ascii_whitespace() {
                continue;
            }
            if byte == b':' {
                self.value_key = self.candidate.take();
                if self
                    .value_key
                    .is_some_and(|(key, len)| &key[..len] == b"encrypted_content")
                {
                    self.contains_encrypted_content = true;
                }
            } else if self.value_key.is_some() {
                self.value_key = None;
            }
        }
    }
}

struct SemanticMarkerReader<R> {
    inner: R,
    scanner: SemanticMarkerScanner,
}

impl<R> SemanticMarkerReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            scanner: SemanticMarkerScanner::default(),
        }
    }
}

impl<R: Read> Read for SemanticMarkerReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.scanner.scan(&buffer[..read]);
        if self.scanner.semantic_limit_exceeded {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request semantic string exceeds bounded projection limit",
            ));
        }
        Ok(read)
    }
}

fn parse_selective_request_semantics(
    reader: impl Read,
) -> Result<SelectiveRequestSemantics, &'static str> {
    let mut reader = SemanticMarkerReader::new(reader);
    let parsed = serde_json::from_reader::<_, SelectiveRequestSemantics>(std::io::BufReader::new(
        &mut reader,
    ))
    .map_err(|_| "invalid_json")?;
    let mut parsed = parsed;
    parsed.contains_encrypted_content |= reader.scanner.contains_encrypted_content;
    parsed.contains_image_generation |= reader.scanner.contains_image_generation;
    parsed.contains_codex_image_generation |= reader.scanner.contains_codex_image_generation;
    Ok(parsed)
}

#[cfg(target_os = "linux")]
fn current_thread_cpu_time() -> Option<Duration> {
    let mut value = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime writes to the valid timespec pointer and has no ownership effects.
    let result = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut value) };
    (result == 0).then(|| {
        Duration::from_secs(value.tv_sec as u64)
            .saturating_add(Duration::from_nanos(value.tv_nsec as u64))
    })
}

#[cfg(not(target_os = "linux"))]
fn current_thread_cpu_time() -> Option<Duration> {
    None
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RequestSemanticCpuAttributionSnapshot {
    pub(crate) count: u64,
    pub(crate) cpu_ms: u64,
    pub(crate) wall_ms: u64,
    pub(crate) bytes: u64,
}

static REQUEST_SEMANTIC_CPU_COUNT: AtomicU64 = AtomicU64::new(0);
static REQUEST_SEMANTIC_CPU_MS: AtomicU64 = AtomicU64::new(0);
static REQUEST_SEMANTIC_WALL_MS: AtomicU64 = AtomicU64::new(0);
static REQUEST_SEMANTIC_BYTES: AtomicU64 = AtomicU64::new(0);

pub(crate) fn request_semantic_cpu_attribution_snapshot() -> RequestSemanticCpuAttributionSnapshot {
    RequestSemanticCpuAttributionSnapshot {
        count: REQUEST_SEMANTIC_CPU_COUNT.load(Ordering::Relaxed),
        cpu_ms: REQUEST_SEMANTIC_CPU_MS.load(Ordering::Relaxed),
        wall_ms: REQUEST_SEMANTIC_WALL_MS.load(Ordering::Relaxed),
        bytes: REQUEST_SEMANTIC_BYTES.load(Ordering::Relaxed),
    }
}

fn record_request_semantic_cpu_window(cpu_time: Option<Duration>, wall_time: Duration, bytes: u64) {
    use std::sync::{Mutex as StdMutex, OnceLock};

    #[derive(Default)]
    struct Window {
        started_at: Option<Instant>,
        cpu_ns: u128,
        wall_ns: u128,
        bytes: u64,
        count: u64,
    }

    static WINDOW: OnceLock<StdMutex<Window>> = OnceLock::new();
    let Ok(mut window) = WINDOW
        .get_or_init(|| StdMutex::new(Window::default()))
        .lock()
    else {
        return;
    };
    let started_at = *window.started_at.get_or_insert_with(Instant::now);
    window.cpu_ns = window
        .cpu_ns
        .saturating_add(cpu_time.map_or(0, |value| value.as_nanos()));
    window.wall_ns = window.wall_ns.saturating_add(wall_time.as_nanos());
    window.bytes = window.bytes.saturating_add(bytes);
    window.count = window.count.saturating_add(1);
    if started_at.elapsed() < Duration::from_secs(30) {
        return;
    }
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    REQUEST_SEMANTIC_CPU_COUNT.store(window.count, Ordering::Relaxed);
    REQUEST_SEMANTIC_CPU_MS.store((window.cpu_ns / 1_000_000) as u64, Ordering::Relaxed);
    REQUEST_SEMANTIC_WALL_MS.store((window.wall_ns / 1_000_000) as u64, Ordering::Relaxed);
    REQUEST_SEMANTIC_BYTES.store(window.bytes, Ordering::Relaxed);
    debug!(
        component = "request_semantic_projection",
        window_elapsed_ms = elapsed_ms,
        parse_count = window.count,
        parse_cpu_ms = (window.cpu_ns / 1_000_000) as u64,
        parse_wall_ms = (window.wall_ns / 1_000_000) as u64,
        parse_bytes = window.bytes,
        throughput_bytes_per_second = (u128::from(window.bytes) * 1_000_000_000)
            .checked_div(window.wall_ns)
            .unwrap_or_default() as u64,
        "request semantic CPU attribution window"
    );
    *window = Window {
        started_at: Some(Instant::now()),
        ..Window::default()
    };
}

#[derive(Clone, Copy)]
enum SelectiveSemanticScope {
    Root,
    Metadata,
    Reasoning,
    ContextManagement,
    Nested,
}

#[derive(Default)]
struct OptionalString {
    value: Option<String>,
    valid: bool,
}

const REQUEST_SEMANTIC_STRING_MAX_BYTES: usize = PROMPT_CACHE_KEY_MAX_BYTES;

impl<'de> Deserialize<'de> for OptionalString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct OptionalStringVisitor;
        impl<'de> Visitor<'de> for OptionalStringVisitor {
            type Value = OptionalString;
            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a string or ignored JSON value")
            }
            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
                if value.len() > REQUEST_SEMANTIC_STRING_MAX_BYTES {
                    return Ok(OptionalString {
                        value: None,
                        valid: false,
                    });
                }
                Ok(OptionalString {
                    value: Some(value.to_string()),
                    valid: true,
                })
            }
            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                if value.len() > REQUEST_SEMANTIC_STRING_MAX_BYTES {
                    return Ok(OptionalString {
                        value: None,
                        valid: false,
                    });
                }
                Ok(OptionalString {
                    value: Some(value),
                    valid: true,
                })
            }
            fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
                Ok(OptionalString {
                    value: None,
                    valid: false,
                })
            }
            fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
                Ok(OptionalString {
                    value: None,
                    valid: false,
                })
            }
            fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
                Ok(OptionalString {
                    value: None,
                    valid: false,
                })
            }
            fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
                Ok(OptionalString {
                    value: None,
                    valid: false,
                })
            }
            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(OptionalString {
                    value: None,
                    valid: true,
                })
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(OptionalString {
                    value: None,
                    valid: true,
                })
            }
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
                Ok(OptionalString {
                    value: None,
                    valid: false,
                })
            }
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                while map
                    .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                    .is_some()
                {}
                Ok(OptionalString {
                    value: None,
                    valid: false,
                })
            }
        }
        deserializer.deserialize_any(OptionalStringVisitor)
    }
}

struct SelectiveSemanticSeed<'a> {
    projection: &'a mut SelectiveRequestSemantics,
    scope: SelectiveSemanticScope,
}

impl<'de> DeserializeSeed<'de> for SelectiveSemanticSeed<'_> {
    type Value = ();
    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(SelectiveSemanticVisitor {
            projection: self.projection,
            scope: self.scope,
        })
    }
}

struct SelectiveSemanticVisitor<'a> {
    projection: &'a mut SelectiveRequestSemantics,
    scope: SelectiveSemanticScope,
}

impl<'de> Visitor<'de> for SelectiveSemanticVisitor<'_> {
    type Value = ();
    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a request JSON value")
    }
    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
        if matches!(self.scope, SelectiveSemanticScope::Metadata) {
            self.projection.invalid_sticky_projection = true;
        }
        Ok(())
    }
    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        if matches!(self.scope, SelectiveSemanticScope::Metadata) {
            self.projection.invalid_sticky_projection = true;
        }
        Ok(())
    }
    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        if matches!(self.scope, SelectiveSemanticScope::Metadata) {
            self.projection.invalid_sticky_projection = true;
        }
        Ok(())
    }
    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        if matches!(self.scope, SelectiveSemanticScope::Metadata) {
            self.projection.invalid_sticky_projection = true;
        }
        Ok(())
    }
    fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
        if matches!(self.scope, SelectiveSemanticScope::Metadata) {
            self.projection.invalid_sticky_projection = true;
        }
        Ok(())
    }
    fn visit_string<E>(self, _: String) -> Result<Self::Value, E> {
        if matches!(self.scope, SelectiveSemanticScope::Metadata) {
            self.projection.invalid_sticky_projection = true;
        }
        Ok(())
    }
    fn visit_none<E>(self) -> Result<Self::Value, E> {
        if matches!(self.scope, SelectiveSemanticScope::Metadata) {
            self.projection.invalid_sticky_projection = true;
        }
        Ok(())
    }
    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        if matches!(self.scope, SelectiveSemanticScope::Metadata) {
            self.projection.invalid_sticky_projection = true;
        }
        Ok(())
    }
    fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        if matches!(self.scope, SelectiveSemanticScope::Metadata) {
            self.projection.invalid_sticky_projection = true;
        }
        let child_scope = if matches!(self.scope, SelectiveSemanticScope::ContextManagement) {
            SelectiveSemanticScope::ContextManagement
        } else {
            SelectiveSemanticScope::Nested
        };
        while seq
            .next_element_seed(SelectiveSemanticSeed {
                projection: &mut *self.projection,
                scope: child_scope,
            })?
            .is_some()
        {}
        Ok(())
    }
    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut object_type = None;
        let mut compact_threshold_present = false;
        let mut seen_sticky_projection_bits = 0_u16;
        while let Some(key) = map.next_key::<String>()? {
            let is_sticky_projection_field = matches!(
                (self.scope, key.as_str()),
                (
                    SelectiveSemanticScope::Root | SelectiveSemanticScope::Metadata,
                    "sticky_key" | "stickyKey" | "prompt_cache_key" | "promptCacheKey"
                )
            );
            let sticky_projection_bit = match (self.scope, key.as_str()) {
                (SelectiveSemanticScope::Root, "sticky_key") => Some(1 << 0),
                (SelectiveSemanticScope::Root, "stickyKey") => Some(1 << 1),
                (SelectiveSemanticScope::Root, "prompt_cache_key") => Some(1 << 2),
                (SelectiveSemanticScope::Root, "promptCacheKey") => Some(1 << 3),
                (SelectiveSemanticScope::Root, "metadata") => Some(1 << 4),
                (SelectiveSemanticScope::Metadata, "sticky_key") => Some(1 << 5),
                (SelectiveSemanticScope::Metadata, "stickyKey") => Some(1 << 6),
                (SelectiveSemanticScope::Metadata, "prompt_cache_key") => Some(1 << 7),
                (SelectiveSemanticScope::Metadata, "promptCacheKey") => Some(1 << 8),
                _ => None,
            };
            if let Some(bit) = sticky_projection_bit {
                if seen_sticky_projection_bits & bit != 0 {
                    self.projection.invalid_sticky_projection = true;
                }
                seen_sticky_projection_bits |= bit;
            }
            let string_slot = match (self.scope, key.as_str()) {
                (SelectiveSemanticScope::Root, "model") => Some(&mut self.projection.model),
                (SelectiveSemanticScope::Root, "sticky_key") => {
                    Some(&mut self.projection.root_sticky_key)
                }
                (SelectiveSemanticScope::Root, "stickyKey") => {
                    Some(&mut self.projection.root_sticky_key_alias)
                }
                (SelectiveSemanticScope::Root, "prompt_cache_key") => {
                    Some(&mut self.projection.root_prompt_cache_key)
                }
                (SelectiveSemanticScope::Root, "promptCacheKey") => {
                    Some(&mut self.projection.root_prompt_cache_key_alias)
                }
                (SelectiveSemanticScope::Root, "service_tier") => {
                    Some(&mut self.projection.service_tier)
                }
                (SelectiveSemanticScope::Root, "serviceTier") => {
                    Some(&mut self.projection.service_tier_alias)
                }
                (SelectiveSemanticScope::Root, "reasoning_effort") => {
                    Some(&mut self.projection.reasoning_effort)
                }
                (SelectiveSemanticScope::Metadata, "sticky_key") => {
                    Some(&mut self.projection.metadata_sticky_key)
                }
                (SelectiveSemanticScope::Metadata, "stickyKey") => {
                    Some(&mut self.projection.metadata_sticky_key_alias)
                }
                (SelectiveSemanticScope::Metadata, "prompt_cache_key") => {
                    Some(&mut self.projection.metadata_prompt_cache_key)
                }
                (SelectiveSemanticScope::Metadata, "promptCacheKey") => {
                    Some(&mut self.projection.metadata_prompt_cache_key_alias)
                }
                (SelectiveSemanticScope::Reasoning, "effort") => {
                    Some(&mut self.projection.nested_reasoning_effort)
                }
                _ => None,
            };
            if let Some(slot) = string_slot {
                let parsed = map.next_value::<OptionalString>()?;
                if is_sticky_projection_field && !parsed.valid {
                    self.projection.invalid_sticky_projection = true;
                }
                if matches!(key.as_str(), "prompt_cache_key" | "promptCacheKey") && !parsed.valid {
                    self.projection.invalid_prompt_cache_key = true;
                }
                *slot = parsed.value;
                continue;
            }
            match (self.scope, key.as_str()) {
                (SelectiveSemanticScope::Root, "stream") => {
                    self.projection.stream = map.next_value::<bool>().unwrap_or(false);
                }
                (SelectiveSemanticScope::Root, "stream_options") => {
                    self.projection.stream_options_present = true;
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
                (SelectiveSemanticScope::Root, "metadata") => {
                    map.next_value_seed(SelectiveSemanticSeed {
                        projection: &mut *self.projection,
                        scope: SelectiveSemanticScope::Metadata,
                    })?
                }
                (SelectiveSemanticScope::Root, "reasoning") => {
                    map.next_value_seed(SelectiveSemanticSeed {
                        projection: &mut *self.projection,
                        scope: SelectiveSemanticScope::Reasoning,
                    })?
                }
                (SelectiveSemanticScope::Root, "context_management") => {
                    map.next_value_seed(SelectiveSemanticSeed {
                        projection: &mut *self.projection,
                        scope: SelectiveSemanticScope::ContextManagement,
                    })?
                }
                (_, "encrypted_content") => {
                    self.projection.contains_encrypted_content = true;
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
                (_, "type") => object_type = map.next_value::<OptionalString>()?.value,
                (SelectiveSemanticScope::ContextManagement, "compact_threshold") => {
                    compact_threshold_present = true;
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
                (
                    _,
                    "input" | "messages" | "content" | "items" | "tools" | "tool_choice"
                    | "additional_tools" | "context",
                ) => map.next_value_seed(SelectiveSemanticSeed {
                    projection: &mut *self.projection,
                    scope: if matches!(self.scope, SelectiveSemanticScope::ContextManagement) {
                        SelectiveSemanticScope::ContextManagement
                    } else {
                        SelectiveSemanticScope::Nested
                    },
                })?,
                _ => {
                    map.next_value::<serde::de::IgnoredAny>()?;
                }
            }
        }
        if object_type.as_deref() == Some("encrypted_content") {
            self.projection.contains_encrypted_content = true;
        }
        if object_type.as_deref() == Some("image_generation") {
            self.projection.contains_image_generation = true;
        }
        if object_type.as_deref() == Some("image_gen.imagegen") {
            self.projection.contains_codex_image_generation = true;
        }
        if matches!(self.scope, SelectiveSemanticScope::ContextManagement)
            && object_type.as_deref() == Some("compaction")
            && compact_threshold_present
        {
            self.projection.declares_remote_v2_compaction = true;
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for SelectiveRequestSemantics {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut projection = Self::default();
        SelectiveSemanticSeed {
            projection: &mut projection,
            scope: SelectiveSemanticScope::Root,
        }
        .deserialize(deserializer)?;
        Ok(projection)
    }
}

fn extract_sticky_key_from_request_value_projection_semantics(value: &Value) -> Option<String> {
    fn projection_field<'a>(
        object: &'a serde_json::Map<String, Value>,
        key: &str,
    ) -> Result<Option<&'a str>, ()> {
        match object.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(value)) => Ok(Some(value)),
            Some(_) => Err(()),
        }
    }

    let root = value.as_object()?;
    let metadata = match root.get("metadata") {
        None => None,
        Some(Value::Object(metadata)) => Some(metadata),
        Some(_) => return None,
    };
    let metadata_field = |key| match metadata {
        Some(metadata) => projection_field(metadata, key),
        None => Ok(None),
    };
    let candidates = [
        metadata_field("sticky_key"),
        metadata_field("stickyKey"),
        metadata_field("prompt_cache_key"),
        metadata_field("promptCacheKey"),
        projection_field(root, "sticky_key"),
        projection_field(root, "stickyKey"),
        projection_field(root, "prompt_cache_key"),
        projection_field(root, "promptCacheKey"),
    ];

    if candidates.iter().any(Result::is_err) {
        return None;
    }
    for candidate in candidates {
        match candidate {
            Ok(Some(value)) if !value.trim().is_empty() => return Some(value.trim().to_string()),
            Ok(_) => {}
            Err(()) => unreachable!("invalid sticky projection fields were checked above"),
        }
    }
    None
}

const REPLAY_SNAPSHOT_ROUTE_ANALYSIS_SLOW_MS: f64 = 100.0;

#[derive(Debug, Clone)]
pub(crate) struct ReplaySnapshotRouteAnalysis {
    pub(crate) sticky_key: Option<String>,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) requested_model: Option<String>,
    pub(crate) contains_encrypted_content: bool,
    pub(crate) image_intent: ImageIntent,
    pub(crate) hosted_image_intent: ImageIntent,
    pub(crate) compaction_kind: Option<CompactionKind>,
    pub(crate) is_stream: bool,
    pub(crate) requested_service_tier: Option<String>,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) stream_options_present: bool,
    pub(crate) file_read_count: u8,
    pub(crate) json_parse_count: u8,
    pub(crate) parse_outcome: &'static str,
    pub(crate) invalid_prompt_cache_key: bool,
}

fn analyze_replay_snapshot_route_value(
    parsed_value: Option<SelectiveRequestSemantics>,
    capture_target: Option<ProxyCaptureTarget>,
) -> ReplaySnapshotRouteAnalysis {
    let value = parsed_value.unwrap_or_default();
    let normalize = |value: Option<String>| {
        value
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let sticky_key = (!value.invalid_sticky_projection)
        .then(|| {
            [
                value.metadata_sticky_key,
                value.metadata_sticky_key_alias,
                value.metadata_prompt_cache_key.clone(),
                value.metadata_prompt_cache_key_alias.clone(),
                value.root_sticky_key,
                value.root_sticky_key_alias,
                value.root_prompt_cache_key.clone(),
                value.root_prompt_cache_key_alias.clone(),
            ]
            .into_iter()
            .find_map(normalize)
        })
        .flatten();
    let prompt_cache_key = [
        value.metadata_prompt_cache_key,
        value.metadata_prompt_cache_key_alias,
        value.root_prompt_cache_key,
        value.root_prompt_cache_key_alias,
    ]
    .into_iter()
    .find_map(normalize);
    let requested_model = normalize(value.model);
    let image_intent = match capture_target {
        Some(ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits) => {
            ImageIntent::DirectImage
        }
        Some(ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact) => {
            if requested_model
                .as_deref()
                .is_some_and(is_openai_image_generation_model)
                || value.contains_image_generation
                || value.contains_codex_image_generation
            {
                ImageIntent::Yes
            } else {
                ImageIntent::No
            }
        }
        Some(ProxyCaptureTarget::ChatCompletions | ProxyCaptureTarget::StandaloneSearch) | None => {
            ImageIntent::Unknown
        }
    };
    let hosted_image_intent = match capture_target {
        Some(ProxyCaptureTarget::ImageGenerations | ProxyCaptureTarget::ImageEdits) => {
            ImageIntent::DirectImage
        }
        Some(ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact) => {
            if requested_model
                .as_deref()
                .is_some_and(is_openai_image_generation_model)
                || value.contains_image_generation
            {
                ImageIntent::Yes
            } else {
                ImageIntent::No
            }
        }
        Some(ProxyCaptureTarget::ChatCompletions | ProxyCaptureTarget::StandaloneSearch) | None => {
            ImageIntent::Unknown
        }
    };
    let compaction_kind = match capture_target {
        Some(ProxyCaptureTarget::ResponsesCompact) => Some(CompactionKind::Compact),
        Some(ProxyCaptureTarget::Responses) if value.declares_remote_v2_compaction => {
            Some(CompactionKind::RemoteV2)
        }
        _ => None,
    };
    let requested_service_tier = normalize(value.service_tier.or(value.service_tier_alias))
        .and_then(|value| normalize_service_tier(&value));
    let reasoning_effort = normalize(match capture_target {
        Some(ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact) => {
            value.nested_reasoning_effort
        }
        Some(ProxyCaptureTarget::ChatCompletions) => value.reasoning_effort,
        _ => None,
    });

    ReplaySnapshotRouteAnalysis {
        sticky_key,
        prompt_cache_key,
        requested_model,
        contains_encrypted_content: value.contains_encrypted_content,
        image_intent,
        hosted_image_intent,
        compaction_kind,
        is_stream: value.stream,
        requested_service_tier,
        reasoning_effort,
        stream_options_present: value.stream_options_present,
        file_read_count: 0,
        json_parse_count: 0,
        parse_outcome: "empty",
        invalid_prompt_cache_key: value.invalid_prompt_cache_key,
    }
}

/// Derive every routing projection from a replay snapshot in one read and one JSON parse.
///
/// The parsed `Value` is deliberately dropped before returning: callers keep only the small
/// projections needed for routing, while file-backed snapshots remain replayable for failover.
pub(crate) async fn analyze_replay_snapshot_for_pool_routing(
    snapshot: &PoolReplayBodySnapshot,
    capture_target: Option<ProxyCaptureTarget>,
    proxy_request_id: u64,
    purpose: &'static str,
) -> ReplaySnapshotRouteAnalysis {
    let started = Instant::now();
    let snapshot_kind = pool_request_snapshot_kind(snapshot);
    let snapshot_bytes = pool_request_snapshot_body_bytes(snapshot);
    let (value, parse_outcome, file_read_count, json_parse_count, parse_cpu_time) = match snapshot {
        PoolReplayBodySnapshot::Empty => (None, "empty", 0_u8, 0_u8, None),
        PoolReplayBodySnapshot::Memory(bytes) => {
            let cpu_started = current_thread_cpu_time();
            match parse_selective_request_semantics(std::io::Cursor::new(bytes.as_ref())) {
                Ok(value) => (
                    Some(value),
                    "parsed",
                    0,
                    1,
                    cpu_started
                        .zip(current_thread_cpu_time())
                        .map(|(start, end)| end.saturating_sub(start)),
                ),
                Err(_) => (
                    None,
                    "invalid_json",
                    0,
                    1,
                    cpu_started
                        .zip(current_thread_cpu_time())
                        .map(|(start, end)| end.saturating_sub(start)),
                ),
            }
        }
        PoolReplayBodySnapshot::File { temp_file, .. } => {
            let path = temp_file.path.clone();
            match tokio::task::spawn_blocking(move || {
                let cpu_started = current_thread_cpu_time();
                let file = std::fs::File::open(path).map_err(|_| "read_failed")?;
                let parsed = parse_selective_request_semantics(file).map(Some);
                let cpu_time = cpu_started
                    .zip(current_thread_cpu_time())
                    .map(|(start, end)| end.saturating_sub(start));
                parsed.map(|value| (value, cpu_time))
            })
            .await
            {
                Ok(Ok((value, cpu_time))) => (value, "parsed", 1, 1, cpu_time),
                Ok(Err(outcome)) => (None, outcome, 1, 1, None),
                Err(_) => (None, "worker_failed", 1, 0, None),
            }
        }
    };
    let mut analysis = analyze_replay_snapshot_route_value(value, capture_target);
    analysis.file_read_count = file_read_count;
    analysis.json_parse_count = json_parse_count;
    analysis.parse_outcome = parse_outcome;
    let analysis_elapsed_ms = elapsed_ms(started);
    record_request_semantic_cpu_window(
        parse_cpu_time,
        started.elapsed(),
        snapshot_bytes.min(u64::MAX as usize) as u64,
    );

    if snapshot_kind == "file" {
        if analysis_elapsed_ms >= REPLAY_SNAPSHOT_ROUTE_ANALYSIS_SLOW_MS {
            info!(
                proxy_request_id,
                purpose,
                snapshot_kind,
                snapshot_bytes,
                file_read_count = analysis.file_read_count,
                json_parse_count = analysis.json_parse_count,
                parse_outcome = analysis.parse_outcome,
                analysis_elapsed_ms,
                "pool replay snapshot route analysis exceeded slow-path threshold"
            );
        } else {
            debug!(
                proxy_request_id,
                purpose,
                snapshot_kind,
                snapshot_bytes,
                file_read_count = analysis.file_read_count,
                json_parse_count = analysis.json_parse_count,
                parse_outcome = analysis.parse_outcome,
                analysis_elapsed_ms,
                "pool replay snapshot route analysis completed"
            );
        }
    }

    analysis
}
