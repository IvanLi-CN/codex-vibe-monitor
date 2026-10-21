use super::*;

pub(crate) const REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES: usize = 64 * 1024;

/// Immutable request semantics derived from the single replay snapshot.
///
/// The snapshot remains the source of truth for forwarding and raw capture. The
/// projection only owns bounded business metadata and an optional rewritten
/// snapshot, so large request bodies never need to be copied into a `Vec`.
#[derive(Debug, Clone)]
pub(crate) struct RequestSemanticProjection {
    pub(crate) snapshot: PoolReplayBodySnapshot,
    pub(crate) request_info: RequestCaptureInfo,
    pub(crate) hosted_image_intent: ImageIntent,
    pub(crate) upstream_snapshot: PoolReplayBodySnapshot,
    pub(crate) request_body_for_capture: Option<Bytes>,
    pub(crate) body_rewritten: bool,
    pub(crate) parse_elapsed_ms: u64,
    pub(crate) materialization_bytes: usize,
    pub(crate) buffer_bytes: usize,
    pub(crate) json_parse_count: u8,
    pub(crate) whole_body_materialization_count: u8,
    pub(crate) peak_business_buffer_bytes: usize,
    pub(crate) fallback_reason: Option<&'static str>,
}

const INCLUDE_USAGE_ROOT_INSERTION: &[u8] = br#","stream_options":{"include_usage":true}"#;
const INCLUDE_USAGE_OBJECT_INSERTION: &[u8] = br#","include_usage":true"#;
const INCLUDE_USAGE_EMPTY_OBJECT_INSERTION: &[u8] = br#""include_usage":true"#;
const INCLUDE_USAGE_REPLACEMENT: &[u8] = br#"{"include_usage":true}"#;
const INCLUDE_USAGE_COPY_BUFFER_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Copy)]
enum IncludeUsageRewritePlan {
    InsertRoot {
        offset: usize,
    },
    InsertObject {
        start: usize,
        offset: usize,
        empty: bool,
    },
    ReplaceValue {
        start: usize,
        end: usize,
    },
    ReplaceIncludeUsageValue {
        start: usize,
        end: usize,
    },
}

#[derive(Debug, Clone, Copy)]
enum ActiveStreamOptionsValue {
    Object {
        start: usize,
        depth: usize,
        has_content: bool,
    },
    Composite {
        start: usize,
        depth: usize,
    },
    Primitive {
        start: usize,
        last_non_whitespace: usize,
    },
    String {
        start: usize,
    },
}

fn locate_root_field_rewrite(
    reader: impl Read,
    target_key: &[u8],
) -> io::Result<Option<IncludeUsageRewritePlan>> {
    let mut reader = std::io::BufReader::with_capacity(INCLUDE_USAGE_COPY_BUFFER_BYTES, reader);
    let mut depth = 0_usize;
    let mut position = 0_usize;
    let mut root_close = None;
    let mut expect_root_key = false;
    let mut pending_root_key_is_stream_options = false;
    let mut awaiting_stream_options_value = false;
    let mut in_string = false;
    let mut escaped = false;
    let mut string_is_root_key = false;
    let mut string_is_target_value = false;
    let mut root_key = Vec::with_capacity("stream_options".len());
    let mut active = None;
    let mut plan = None;
    let mut buffer = [0_u8; 1];

    while reader.read(&mut buffer)? != 0 {
        let byte = buffer[0];
        let current = position;
        position += 1;

        if in_string {
            if escaped {
                if string_is_root_key {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "escaped semantic key requires fail-open rewrite",
                    ));
                }
                escaped = false;
                continue;
            }
            if byte == b'\\' {
                escaped = true;
                continue;
            }
            if byte == b'"' {
                in_string = false;
                if string_is_root_key {
                    pending_root_key_is_stream_options = root_key == target_key;
                    string_is_root_key = false;
                } else if string_is_target_value {
                    if let Some(ActiveStreamOptionsValue::String { start }) = active.take() {
                        plan = Some(IncludeUsageRewritePlan::ReplaceValue {
                            start,
                            end: current + 1,
                        });
                    }
                    string_is_target_value = false;
                }
                continue;
            }
            if string_is_root_key && root_key.len() <= "stream_options".len() {
                root_key.push(byte);
            }
            continue;
        }

        if awaiting_stream_options_value {
            if byte.is_ascii_whitespace() {
                continue;
            }
            awaiting_stream_options_value = false;
            active = Some(match byte {
                b'{' => ActiveStreamOptionsValue::Object {
                    start: current,
                    depth: depth + 1,
                    has_content: false,
                },
                b'[' => ActiveStreamOptionsValue::Composite {
                    start: current,
                    depth: depth + 1,
                },
                b'"' => {
                    in_string = true;
                    string_is_target_value = true;
                    ActiveStreamOptionsValue::String { start: current }
                }
                _ => ActiveStreamOptionsValue::Primitive {
                    start: current,
                    last_non_whitespace: current,
                },
            });
            if byte == b'"' {
                continue;
            }
        }

        if let Some(ActiveStreamOptionsValue::Primitive {
            start,
            last_non_whitespace,
        }) = active
        {
            if depth == 1 && matches!(byte, b',' | b'}') {
                plan = Some(IncludeUsageRewritePlan::ReplaceValue {
                    start,
                    end: last_non_whitespace + 1,
                });
                active = None;
            } else if !byte.is_ascii_whitespace() {
                active = Some(ActiveStreamOptionsValue::Primitive {
                    start,
                    last_non_whitespace: current,
                });
            }
        }

        if byte == b'"' {
            if let Some(ActiveStreamOptionsValue::Object {
                start,
                depth: target_depth,
                ..
            }) = active
                && depth >= target_depth
            {
                active = Some(ActiveStreamOptionsValue::Object {
                    start,
                    depth: target_depth,
                    has_content: true,
                });
            }
            in_string = true;
            string_is_root_key = depth == 1 && expect_root_key;
            if string_is_root_key {
                root_key.clear();
                expect_root_key = false;
            }
            continue;
        }

        if let Some(ActiveStreamOptionsValue::Object {
            start,
            depth: target_depth,
            has_content,
        }) = active
            && depth >= target_depth
            && !byte.is_ascii_whitespace()
            && !(byte == b'}' && depth == target_depth)
        {
            active = Some(ActiveStreamOptionsValue::Object {
                start,
                depth: target_depth,
                has_content: has_content || byte != b'{',
            });
        }

        match byte {
            b':' if depth == 1 => {
                awaiting_stream_options_value = pending_root_key_is_stream_options;
                pending_root_key_is_stream_options = false;
            }
            b'{' | b'[' => {
                depth += 1;
                if depth == 1 {
                    expect_root_key = byte == b'{';
                }
            }
            b'}' | b']' => {
                match active {
                    Some(ActiveStreamOptionsValue::Object {
                        start,
                        depth: target_depth,
                        has_content,
                    }) if byte == b'}' && depth == target_depth => {
                        plan = Some(IncludeUsageRewritePlan::InsertObject {
                            start,
                            offset: current,
                            empty: !has_content,
                        });
                        active = None;
                    }
                    Some(ActiveStreamOptionsValue::Composite {
                        start,
                        depth: target_depth,
                    }) if depth == target_depth => {
                        plan = Some(IncludeUsageRewritePlan::ReplaceValue {
                            start,
                            end: current + 1,
                        });
                        active = None;
                    }
                    _ => {}
                }
                if depth == 0 {
                    return Ok(None);
                }
                depth -= 1;
                if depth == 0 && byte == b'}' {
                    root_close = Some(current);
                }
            }
            b',' if depth == 1 => expect_root_key = true,
            _ => {}
        }
    }

    Ok(plan.or_else(|| root_close.map(|offset| IncludeUsageRewritePlan::InsertRoot { offset })))
}

fn locate_include_usage_rewrite<R>(mut reader: R) -> io::Result<Option<IncludeUsageRewritePlan>>
where
    R: Read + Seek,
{
    let plan = locate_root_field_rewrite(&mut reader, b"stream_options")?;
    let Some(IncludeUsageRewritePlan::InsertObject {
        start,
        offset,
        empty,
    }) = plan
    else {
        return Ok(plan);
    };
    reader.seek(SeekFrom::Start(start as u64))?;
    let nested = locate_root_field_rewrite(
        (&mut reader).take((offset - start + 1) as u64),
        b"include_usage",
    )?;
    Ok(match nested {
        Some(IncludeUsageRewritePlan::ReplaceValue {
            start: nested_start,
            end,
        }) => Some(IncludeUsageRewritePlan::ReplaceIncludeUsageValue {
            start: start + nested_start,
            end: start + end,
        }),
        Some(IncludeUsageRewritePlan::InsertObject {
            start: nested_start,
            offset: nested_offset,
            ..
        }) => Some(IncludeUsageRewritePlan::ReplaceIncludeUsageValue {
            start: start + nested_start,
            end: start + nested_offset + 1,
        }),
        Some(IncludeUsageRewritePlan::ReplaceIncludeUsageValue { .. }) => {
            unreachable!("nested locator only emits generic replacement plans")
        }
        Some(IncludeUsageRewritePlan::InsertRoot { .. }) | None => {
            Some(IncludeUsageRewritePlan::InsertObject {
                start,
                offset,
                empty,
            })
        }
    })
}

fn include_usage_rewrite_segments(plan: IncludeUsageRewritePlan) -> (usize, usize, &'static [u8]) {
    match plan {
        IncludeUsageRewritePlan::InsertRoot { offset } => (offset, 0, INCLUDE_USAGE_ROOT_INSERTION),
        IncludeUsageRewritePlan::InsertObject { offset, empty, .. } => (
            offset,
            0,
            if empty {
                INCLUDE_USAGE_EMPTY_OBJECT_INSERTION
            } else {
                INCLUDE_USAGE_OBJECT_INSERTION
            },
        ),
        IncludeUsageRewritePlan::ReplaceValue { start, end } => {
            (start, end - start, INCLUDE_USAGE_REPLACEMENT)
        }
        IncludeUsageRewritePlan::ReplaceIncludeUsageValue { start, end } => {
            (start, end - start, b"true".as_slice())
        }
    }
}

fn copy_exact_bounded(
    reader: &mut std::fs::File,
    writer: &mut std::fs::File,
    mut bytes: usize,
) -> io::Result<()> {
    let mut buffer = [0_u8; INCLUDE_USAGE_COPY_BUFFER_BYTES];
    while bytes > 0 {
        let capacity = buffer.len();
        let read = reader.read(&mut buffer[..bytes.min(capacity)])?;
        if read == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "request snapshot ended during include_usage rewrite",
            ));
        }
        writer.write_all(&buffer[..read])?;
        bytes -= read;
    }
    Ok(())
}

fn copy_to_end_bounded(reader: &mut std::fs::File, writer: &mut std::fs::File) -> io::Result<()> {
    let mut buffer = [0_u8; INCLUDE_USAGE_COPY_BUFFER_BYTES];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        writer.write_all(&buffer[..read])?;
    }
}

async fn rewrite_snapshot_include_usage(
    proxy_request_id: u64,
    snapshot: &PoolReplayBodySnapshot,
) -> io::Result<Option<PoolReplayBodySnapshot>> {
    match snapshot {
        PoolReplayBodySnapshot::Empty => Ok(None),
        PoolReplayBodySnapshot::Memory(bytes) => {
            if bytes.len() > REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES {
                let temp_file = Arc::new(PoolReplayTempFile {
                    path: build_pool_replay_temp_path(proxy_request_id),
                });
                tokio::fs::write(&temp_file.path, bytes).await?;
                let file_snapshot = PoolReplayBodySnapshot::File {
                    temp_file,
                    size: bytes.len(),
                };
                return Box::pin(rewrite_snapshot_include_usage(
                    proxy_request_id,
                    &file_snapshot,
                ))
                .await;
            }
            let Some(plan) = locate_include_usage_rewrite(std::io::Cursor::new(bytes.as_ref()))?
            else {
                return Ok(None);
            };
            let (prefix_bytes, skipped_bytes, insertion) = include_usage_rewrite_segments(plan);
            let mut rewritten = Vec::with_capacity(
                bytes
                    .len()
                    .saturating_sub(skipped_bytes)
                    .saturating_add(insertion.len()),
            );
            rewritten.extend_from_slice(&bytes[..prefix_bytes]);
            rewritten.extend_from_slice(insertion);
            rewritten.extend_from_slice(&bytes[prefix_bytes + skipped_bytes..]);
            Ok(Some(
                pool_replay_snapshot_from_bytes(proxy_request_id, Bytes::from(rewritten)).await?,
            ))
        }
        PoolReplayBodySnapshot::File { temp_file, size } => {
            let source = temp_file.path.clone();
            let destination = Arc::new(PoolReplayTempFile {
                path: build_pool_replay_temp_path(proxy_request_id),
            });
            let destination_for_worker = destination.clone();
            let source_size = *size;
            let rewritten_size =
                tokio::task::spawn_blocking(move || -> io::Result<Option<usize>> {
                    let plan = locate_include_usage_rewrite(std::fs::File::open(&source)?)?;
                    let Some(plan) = plan else {
                        return Ok(None);
                    };
                    let mut reader = std::fs::File::open(source)?;
                    let mut writer = std::fs::File::create(&destination_for_worker.path)?;
                    let (prefix_bytes, skipped_bytes, insertion) =
                        include_usage_rewrite_segments(plan);
                    copy_exact_bounded(&mut reader, &mut writer, prefix_bytes)?;
                    if skipped_bytes > 0 {
                        reader.seek(SeekFrom::Current(skipped_bytes as i64))?;
                    }
                    writer.write_all(insertion)?;
                    copy_to_end_bounded(&mut reader, &mut writer)?;
                    writer.flush()?;
                    Ok(Some(source_size - skipped_bytes + insertion.len()))
                })
                .await
                .map_err(|err| io::Error::other(err.to_string()))??;
            Ok(rewritten_size.map(|size| PoolReplayBodySnapshot::File {
                temp_file: destination,
                size,
            }))
        }
    }
}

/// Build the semantic projection once, reusing the replay snapshot for routing,
/// capture, and upstream preparation.
pub(crate) async fn project_request_semantics(
    proxy_request_id: u64,
    snapshot: PoolReplayBodySnapshot,
    target: ProxyCaptureTarget,
    auto_include_usage: bool,
) -> RequestSemanticProjection {
    let started = Instant::now();
    let body_len = pool_request_snapshot_body_bytes(&snapshot);
    if request_semantic_pipeline_mode() == RequestSemanticPipelineMode::Legacy {
        let Ok(original) = snapshot.to_bytes().await else {
            return RequestSemanticProjection {
                snapshot: snapshot.clone(),
                request_info: RequestCaptureInfo::default(),
                hosted_image_intent: ImageIntent::Unknown,
                upstream_snapshot: snapshot,
                request_body_for_capture: None,
                body_rewritten: false,
                parse_elapsed_ms: started.elapsed().as_millis() as u64,
                materialization_bytes: 0,
                buffer_bytes: 0,
                json_parse_count: 0,
                whole_body_materialization_count: 0,
                peak_business_buffer_bytes: 0,
                fallback_reason: Some("legacy_snapshot_materialization_failed"),
            };
        };
        let (upstream, request_info, body_rewritten, hosted_image_intent) =
            prepare_target_request_body_with_hosted_intent(
                target,
                original.to_vec(),
                auto_include_usage,
            );
        let json_parse_count = u8::from(!original.is_empty());
        let (upstream_snapshot, body_rewritten, fallback_reason) =
            match pool_replay_snapshot_from_bytes(proxy_request_id, Bytes::from(upstream)).await {
                Ok(snapshot) => (snapshot, body_rewritten, None),
                Err(error) => {
                    warn!(
                        proxy_request_id,
                        error = %error,
                        fallback_reason = "rewritten_snapshot_persist_failed",
                        "request semantic projection kept the original snapshot"
                    );
                    (
                        snapshot.clone(),
                        false,
                        Some("rewritten_snapshot_persist_failed"),
                    )
                }
            };
        return RequestSemanticProjection {
            snapshot,
            request_info,
            hosted_image_intent,
            upstream_snapshot,
            request_body_for_capture: Some(original),
            body_rewritten,
            parse_elapsed_ms: started.elapsed().as_millis() as u64,
            materialization_bytes: body_len,
            buffer_bytes: body_len,
            json_parse_count,
            whole_body_materialization_count: u8::from(body_len > 0),
            peak_business_buffer_bytes: body_len,
            fallback_reason,
        };
    }
    let mut projected_json_parse_count = u8::from(body_len > 0);
    let mut semantic_parse_buffer_bytes = 0_usize;
    let mut fallback_reason = None;
    let (
        request_info,
        hosted_image_intent,
        upstream_snapshot,
        request_body_for_capture,
        body_rewritten,
        rewrite_buffer_bytes,
    ) = match &snapshot {
        PoolReplayBodySnapshot::Empty => (
            RequestCaptureInfo::default(),
            ImageIntent::Unknown,
            PoolReplayBodySnapshot::Empty,
            Some(Bytes::new()),
            false,
            0,
        ),
        PoolReplayBodySnapshot::Memory(bytes)
            if bytes.len() <= REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES =>
        {
            let (upstream, info, rewritten, hosted_image_intent) =
                prepare_target_request_body_with_hosted_intent(
                    target,
                    bytes.to_vec(),
                    auto_include_usage,
                );
            let capture = Some(bytes.clone());
            let (upstream_snapshot, body_rewritten) = if rewritten {
                match pool_replay_snapshot_from_bytes(proxy_request_id, Bytes::from(upstream)).await
                {
                    Ok(snapshot) => (snapshot, true),
                    Err(error) => {
                        warn!(
                            proxy_request_id,
                            error = %error,
                            fallback_reason = "rewritten_snapshot_persist_failed",
                            "request semantic projection kept the original snapshot"
                        );
                        (snapshot.clone(), false)
                    }
                }
            } else {
                (snapshot.clone(), false)
            };
            (
                info,
                hosted_image_intent,
                upstream_snapshot,
                capture,
                body_rewritten,
                bytes.len(),
            )
        }
        _ => {
            let analysis = analyze_replay_snapshot_for_pool_routing(
                &snapshot,
                Some(target),
                proxy_request_id,
                "request_semantic_projection",
            )
            .await;
            projected_json_parse_count = analysis.json_parse_count;
            semantic_parse_buffer_bytes = REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES.min(body_len);
            let request_info = RequestCaptureInfo {
                model: analysis.requested_model,
                sticky_key: analysis.sticky_key,
                prompt_cache_key: analysis.prompt_cache_key,
                prompt_cache_key_attribution_source: None,
                contains_encrypted_content: analysis.contains_encrypted_content,
                image_intent: Some(analysis.image_intent.as_str().to_string()),
                requested_service_tier: analysis.requested_service_tier,
                reasoning_effort: analysis.reasoning_effort,
                compaction_request_kind: analysis.compaction_kind,
                is_stream: analysis.is_stream,
                parse_error: (analysis.parse_outcome != "parsed")
                    .then(|| format!("request_json_{}", analysis.parse_outcome)),
            };
            let should_rewrite =
                target.should_auto_include_usage() && auto_include_usage && request_info.is_stream;
            let rewritten_snapshot = if should_rewrite {
                match rewrite_snapshot_include_usage(proxy_request_id, &snapshot).await {
                    Ok(rewritten) => rewritten,
                    Err(error) => {
                        warn!(
                            proxy_request_id,
                            error = %error,
                            fallback_reason = "include_usage_rewrite_failed",
                            "request semantic projection kept the original snapshot"
                        );
                        fallback_reason = Some("include_usage_rewrite_failed");
                        None
                    }
                }
            } else {
                None
            };
            let body_rewritten = rewritten_snapshot.is_some();
            (
                request_info,
                analysis.hosted_image_intent,
                rewritten_snapshot.unwrap_or_else(|| snapshot.clone()),
                None,
                body_rewritten,
                if should_rewrite {
                    INCLUDE_USAGE_COPY_BUFFER_BYTES.min(body_len)
                } else {
                    0
                },
            )
        }
    };
    let buffer_bytes = rewrite_buffer_bytes.max(semantic_parse_buffer_bytes);

    let whole_body_materialization_count = u8::from(matches!(
        &snapshot,
        PoolReplayBodySnapshot::Memory(bytes)
            if !bytes.is_empty() && bytes.len() <= REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES
    ));
    RequestSemanticProjection {
        snapshot,
        request_info,
        hosted_image_intent,
        upstream_snapshot,
        request_body_for_capture,
        body_rewritten,
        parse_elapsed_ms: started.elapsed().as_millis() as u64,
        materialization_bytes: if whole_body_materialization_count > 0 {
            body_len
        } else {
            0
        },
        buffer_bytes,
        json_parse_count: projected_json_parse_count,
        whole_body_materialization_count,
        peak_business_buffer_bytes: buffer_bytes,
        fallback_reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn request_semantic_projection_rewrites_small_stream_body_without_changing_capture() {
        let original = Bytes::from_static(br#"{"model":"gpt-5","stream":true}"#);
        let snapshot = pool_replay_snapshot_from_bytes(90_001, original.clone())
            .await
            .expect("build replay snapshot");

        let projection =
            project_request_semantics(90_001, snapshot, ProxyCaptureTarget::ChatCompletions, true)
                .await;

        assert!(projection.body_rewritten);
        assert_eq!(projection.request_body_for_capture, Some(original));
        let rewritten = projection
            .upstream_snapshot
            .to_bytes()
            .await
            .expect("read rewritten body");
        let value: Value = serde_json::from_slice(&rewritten).expect("parse rewritten body");
        assert_eq!(value["stream_options"]["include_usage"], true);
        assert!(projection.buffer_bytes <= REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES);
    }

    #[tokio::test]
    async fn request_semantic_projection_rewrites_medium_memory_snapshot_via_file() {
        let prefix = br#"{"model":"gpt-5","stream":true,"stream_options":{"include_usage":false,"other_option":"preserved"},"padding":""#;
        let mut body = Vec::with_capacity(512 * 1024);
        body.extend_from_slice(prefix);
        body.resize(512 * 1024 - 2, b'x');
        body.extend_from_slice(br#""}"#);
        let snapshot = pool_replay_snapshot_from_bytes(90_005, Bytes::from(body))
            .await
            .expect("build replay snapshot");
        assert!(matches!(&snapshot, PoolReplayBodySnapshot::Memory(_)));

        let projection =
            project_request_semantics(90_005, snapshot, ProxyCaptureTarget::ChatCompletions, true)
                .await;

        assert!(projection.body_rewritten);
        assert!(matches!(
            &projection.upstream_snapshot,
            PoolReplayBodySnapshot::File { .. }
        ));
        let forwarded = projection
            .upstream_snapshot
            .to_bytes()
            .await
            .expect("read medium rewritten body");
        assert_eq!(
            forwarded
                .windows(b"\"include_usage\"".len())
                .filter(|window| *window == b"\"include_usage\"")
                .count(),
            1
        );
        let value: Value = serde_json::from_slice(&forwarded).expect("parse medium rewritten body");
        assert_eq!(value["stream_options"]["include_usage"], true);
        assert_eq!(value["stream_options"]["other_option"], "preserved");
        assert_eq!(projection.whole_body_materialization_count, 0);
        assert_eq!(projection.materialization_bytes, 0);
    }

    #[tokio::test]
    async fn request_semantic_projection_merges_existing_in_memory_stream_options() {
        let prefix = br#"{"model":"gpt-5","stream":true,"stream_options":{"include_usage":false,"other_option":"preserved"},"padding":""#;
        let mut body = Vec::with_capacity(128 * 1024);
        body.extend_from_slice(prefix);
        body.resize(128 * 1024 - 2, b'x');
        body.extend_from_slice(br#""}"#);
        let snapshot = pool_replay_snapshot_from_bytes(90_008, Bytes::from(body))
            .await
            .expect("build replay snapshot");
        assert!(matches!(&snapshot, PoolReplayBodySnapshot::Memory(_)));

        let projection =
            project_request_semantics(90_008, snapshot, ProxyCaptureTarget::ChatCompletions, true)
                .await;
        let forwarded = projection
            .upstream_snapshot
            .to_bytes()
            .await
            .expect("read rewritten body");
        let value: Value = serde_json::from_slice(&forwarded).expect("parse rewritten body");

        assert!(projection.body_rewritten);
        assert_eq!(value["stream_options"]["include_usage"], true);
        assert_eq!(value["stream_options"]["other_option"], "preserved");
        assert_eq!(
            forwarded
                .windows(b"\"stream_options\"".len())
                .filter(|window| *window == b"\"stream_options\"")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn request_semantic_projection_keeps_codex_image_intent_out_of_hosted_routing() {
        let prefix = br#"{"model":"gpt-5","input":[{"type":"additional_tools","tools":[{"type":"image_gen.imagegen"}]}],"padding":""#;
        let mut body = Vec::with_capacity(512 * 1024);
        body.extend_from_slice(prefix);
        body.resize(512 * 1024 - 2, b'x');
        body.extend_from_slice(br#""}"#);
        let snapshot = pool_replay_snapshot_from_bytes(90_007, Bytes::from(body))
            .await
            .expect("build replay snapshot");
        let projection =
            project_request_semantics(90_007, snapshot, ProxyCaptureTarget::Responses, true).await;

        assert_eq!(projection.request_info.image_intent.as_deref(), Some("yes"));
        assert_eq!(projection.hosted_image_intent, ImageIntent::No);
        let prepared = prepare_pool_request_body_for_account(
            90_007,
            Some(&projection.upstream_snapshot),
            &"/v1/responses".parse().expect("valid responses uri"),
            &Method::POST,
            None,
            TagFastModeRewriteMode::KeepOriginal,
            crate::ImageToolRewriteMode::KeepOriginal,
            crate::CodexImagegenRewriteMode::KeepOriginal,
            None,
            Some(&projection.request_info),
            Some(projection.hosted_image_intent),
            None,
        )
        .await
        .expect("prepare projected Codex image request");
        assert_eq!(prepared.requested_image_intent, ImageIntent::Yes);
        assert_eq!(prepared.requested_hosted_image_intent, ImageIntent::No);
    }

    #[tokio::test]
    async fn request_semantic_projection_keeps_file_backed_bodies_file_backed() {
        for (request_id, size, target, case_suffix) in [
            (
                90_016,
                256 * 1024,
                ProxyCaptureTarget::ChatCompletions,
                "16",
            ),
            (90_064, 512 * 1024, ProxyCaptureTarget::Responses, "64"),
        ] {
            let prefix = match target {
                    ProxyCaptureTarget::ChatCompletions => br#"{"model":"gpt-5","stream":true,"stream_options":{"include_usage":false,"other_option":"preserved"},"metadata":{"sticky_key":"sticky-16","prompt_cache_key":"cache-16"},"service_tier":"priority","reasoning_effort":"high","input":[{"encrypted_content":"cipher"}],"padding":""#.as_slice(),
                    ProxyCaptureTarget::Responses => br#"{"model":"gpt-5","stream":true,"metadata":{"sticky_key":"sticky-64","prompt_cache_key":"cache-64"},"serviceTier":"priority","reasoning":{"effort":"medium"},"nested":{"type":"encrypted\u005fcontent"},"tools":[{"type":"image_generation"}],"context_management":[{"type":"compaction","compact_threshold":1000}],"padding":""#.as_slice(),
                    _ => unreachable!(),
                };
            let suffix = br#""}"#;
            let mut body = Vec::with_capacity(size);
            body.extend_from_slice(prefix);
            body.resize(size - suffix.len(), b'x');
            body.extend_from_slice(suffix);
            let original_digest = Sha256::digest(&body);
            let snapshot = pool_replay_snapshot_from_bytes_with_memory_threshold(
                request_id,
                Bytes::from(body),
                64 * 1024,
            )
            .await
            .expect("build replay snapshot");
            assert!(matches!(&snapshot, PoolReplayBodySnapshot::File { .. }));

            let projection = project_request_semantics(request_id, snapshot, target, true).await;

            assert!(matches!(
                &projection.snapshot,
                PoolReplayBodySnapshot::File { .. }
            ));
            assert!(matches!(
                &projection.upstream_snapshot,
                PoolReplayBodySnapshot::File { .. }
            ));
            assert_eq!(projection.request_info.model.as_deref(), Some("gpt-5"));
            assert!(projection.request_info.is_stream);
            assert!(projection.request_info.contains_encrypted_content);
            assert_eq!(
                projection.request_info.requested_service_tier.as_deref(),
                Some("priority")
            );
            assert_eq!(
                projection.request_info.sticky_key.as_deref(),
                Some(format!("sticky-{case_suffix}").as_str())
            );
            assert_eq!(
                projection.request_info.prompt_cache_key.as_deref(),
                Some(format!("cache-{case_suffix}").as_str())
            );
            assert_eq!(projection.request_body_for_capture, None);
            assert_eq!(
                Sha256::digest(
                    projection
                        .snapshot
                        .to_bytes()
                        .await
                        .expect("read original replay snapshot")
                ),
                original_digest
            );
            if target == ProxyCaptureTarget::ChatCompletions {
                assert!(projection.body_rewritten);
                assert!(projection.buffer_bytes <= REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES);
                assert_eq!(
                    projection.request_info.reasoning_effort.as_deref(),
                    Some("high")
                );
                let forwarded_bytes = projection
                    .upstream_snapshot
                    .to_bytes()
                    .await
                    .expect("read rewritten replay snapshot");
                assert_eq!(
                    forwarded_bytes
                        .windows(b"\"include_usage\"".len())
                        .filter(|window| *window == b"\"include_usage\"")
                        .count(),
                    1
                );
                let forwarded: Value = serde_json::from_slice(&forwarded_bytes)
                    .expect("parse rewritten replay snapshot");
                assert_eq!(forwarded["stream_options"]["include_usage"], true);
                assert_eq!(forwarded["stream_options"]["other_option"], "preserved");
                let PoolReplayBodySnapshot::File { temp_file, size } =
                    &projection.upstream_snapshot
                else {
                    panic!("rewritten body must remain file-backed");
                };
                let mut config = crate::tests::test_config();
                config.proxy_raw_dir = std::env::temp_dir().join(format!(
                    "cvm-request-semantic-raw-{}",
                    Utc::now().timestamp_nanos_opt().unwrap_or_default()
                ));
                config.proxy_raw_compression = RawCompressionCodec::None;
                config.proxy_raw_max_bytes = None;
                let raw = store_raw_payload_snapshot_file(
                    &config,
                    "request-semantic-file-backed",
                    "request",
                    temp_file.path.clone(),
                    *size,
                )
                .await;
                let raw_path = raw.path.expect("raw capture path");
                assert_eq!(
                    Sha256::digest(tokio::fs::read(&raw_path).await.expect("read raw capture")),
                    Sha256::digest(&forwarded_bytes)
                );
                let _ = tokio::fs::remove_dir_all(&config.proxy_raw_dir).await;
            } else {
                assert!(!projection.body_rewritten);
                assert!(projection.buffer_bytes <= REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES);
                assert_eq!(
                    projection.request_info.reasoning_effort.as_deref(),
                    Some("medium")
                );
                assert_eq!(projection.request_info.image_intent.as_deref(), Some("yes"));
                assert_eq!(
                    projection.request_info.compaction_request_kind,
                    Some(CompactionKind::RemoteV2)
                );
                assert_eq!(
                    Sha256::digest(
                        projection
                            .upstream_snapshot
                            .to_bytes()
                            .await
                            .expect("read forwarded replay snapshot")
                    ),
                    original_digest
                );
                for _ in 0..2 {
                    let replay = counted_http_body_from_snapshot(
                        &projection.upstream_snapshot,
                        ObservedByteCounter::default(),
                    );
                    assert_eq!(
                        Sha256::digest(
                            axum::body::to_bytes(replay, size + 1)
                                .await
                                .expect("read direct retry replay body")
                        ),
                        original_digest
                    );
                }
            }
            assert_eq!(projection.json_parse_count, 1);
            assert_eq!(projection.whole_body_materialization_count, 0);
            assert_eq!(projection.materialization_bytes, 0);
            assert!(
                projection.peak_business_buffer_bytes <= REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES
            );
        }
    }

    #[tokio::test]
    async fn request_semantic_projection_fails_open_for_invalid_large_json() {
        let original = Bytes::from(vec![b'!'; 2 * 1024 * 1024]);
        let snapshot = pool_replay_snapshot_from_bytes(90_002, original.clone())
            .await
            .expect("build replay snapshot");
        let projection =
            project_request_semantics(90_002, snapshot, ProxyCaptureTarget::ChatCompletions, true)
                .await;

        assert!(!projection.body_rewritten);
        assert!(projection.request_info.parse_error.is_some());
        assert_eq!(
            projection
                .upstream_snapshot
                .to_bytes()
                .await
                .expect("read fail-open body"),
            original
        );
        assert_eq!(projection.whole_body_materialization_count, 0);
        assert_eq!(projection.materialization_bytes, 0);
    }

    #[tokio::test]
    async fn request_semantic_projection_bounds_large_escaped_selected_string() {
        for (request_id, size) in [(90_003, 512 * 1024), (90_004, 2 * 1024 * 1024)] {
            let mut body = Vec::with_capacity(size);
            body.extend_from_slice(br#"{"model":""#);
            while body.len() < size {
                body.extend_from_slice(br#"\u0061"#);
            }
            body.extend_from_slice(br#"","stream":true}"#);
            let original = Bytes::from(body);
            let snapshot = pool_replay_snapshot_from_bytes(request_id, original.clone())
                .await
                .expect("build replay snapshot");

            let projection = project_request_semantics(
                request_id,
                snapshot,
                ProxyCaptureTarget::ChatCompletions,
                true,
            )
            .await;

            assert!(projection.request_info.parse_error.is_some());
            assert!(!projection.body_rewritten);
            assert_eq!(
                projection
                    .upstream_snapshot
                    .to_bytes()
                    .await
                    .expect("read bounded fail-open body"),
                original
            );
            assert_eq!(projection.whole_body_materialization_count, 0);
            assert_eq!(projection.materialization_bytes, 0);
            assert!(
                projection.peak_business_buffer_bytes <= REQUEST_SEMANTIC_BUSINESS_BUFFER_BYTES
            );
        }
    }

    #[tokio::test]
    async fn request_semantic_projection_fails_open_for_escaped_rewrite_key() {
        let prefix =
                br#"{"model":"gpt-5","stream":true,"stream\u005foptions":{"include_usage":false},"padding":""#;
        let mut body = Vec::with_capacity(512 * 1024);
        body.extend_from_slice(prefix);
        body.resize(512 * 1024 - 2, b'x');
        body.extend_from_slice(br#""}"#);
        let original = Bytes::from(body);
        let snapshot = pool_replay_snapshot_from_bytes(90_006, original.clone())
            .await
            .expect("build replay snapshot");

        let projection =
            project_request_semantics(90_006, snapshot, ProxyCaptureTarget::ChatCompletions, true)
                .await;

        assert!(!projection.body_rewritten);
        assert_eq!(
            projection
                .upstream_snapshot
                .to_bytes()
                .await
                .expect("read escaped-key fail-open body"),
            original
        );
        assert_eq!(projection.whole_body_materialization_count, 0);
    }
}
