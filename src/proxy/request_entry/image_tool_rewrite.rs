use super::*;

pub(crate) fn request_entry_openai_json_tools_contain_image_generation(value: &Value) -> bool {
    value
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| {
            tools.iter().any(|tool| {
                tool.get("type")
                    .and_then(Value::as_str)
                    .is_some_and(|tool_type| tool_type.trim() == "image_generation")
            })
        })
}

pub(crate) fn is_openai_responses_lite_request(headers: &HeaderMap) -> bool {
    headers
        .get("x-openai-internal-codex-responses-lite")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("true"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CodexImagegenProtocol {
    Full,
    Lite,
}

impl CodexImagegenProtocol {
    fn as_str(self) -> &'static str {
        match self {
            Self::Full => "responses_full",
            Self::Lite => "responses_lite",
        }
    }
}

pub(crate) fn codex_imagegen_protocol_from_headers(
    headers: &HeaderMap,
) -> Option<CodexImagegenProtocol> {
    if is_openai_responses_lite_request(headers) {
        return Some(CodexImagegenProtocol::Lite);
    }

    let originator_matches = headers
        .get("originator")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("Codex Desktop"));
    let user_agent_matches = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.trim().starts_with("Codex Desktop/"));
    (originator_matches || user_agent_matches).then_some(CodexImagegenProtocol::Full)
}

const CODEX_IMAGEGEN_NAMESPACE: &str = "image_gen";
const CODEX_IMAGEGEN_TOOL: &str = "imagegen";
const CODEX_IMAGEGEN_DESCRIPTION: &str = r#"The `image_gen.imagegen` tool enables image generation from descriptions and editing of existing images based on specific instructions. Use it when:

- The user requests an image based on a scene description, such as a diagram, portrait, comic, meme, or any other visual.
- The user wants to modify an attached or previously generated image with specific changes, including adding or removing elements, altering colors, improving quality/resolution, or transforming the style (e.g., cartoon, oil painting).

Guidelines:
- imagegen needs a few minutes to finish. In code-mode, use the first-line @exec directive to give the initial call 120 seconds and the same yield for any waits that follow. Once it finishes, return the image with generatedImage(result).
- Omit both `referenced_image_paths` and `num_last_images_to_include` when generating a brand new image.
- For edits, use `referenced_image_paths` when every target image has a local file path.
- If you have not seen a local image yet, use `view_image` to inspect it before editing.
- Use `num_last_images_to_include` only when at least one target image has no local file path.
- Set `num_last_images_to_include` to the smallest number of recent conversation images that includes every target image, up to 5.
- Never provide both `referenced_image_paths` and `num_last_images_to_include`.
- If neither mechanism can include every target image, ask the user to attach the missing images again.
- Directly generate the image without reconfirmation or clarification unless required images must be attached again.
- Always use this tool for image editing unless the user explicitly requests otherwise. Do not use the `python` tool for image editing unless specifically instructed."#;

fn codex_imagegen_function() -> Value {
    serde_json::json!({
        "type": "function",
        "name": CODEX_IMAGEGEN_TOOL,
        "description": CODEX_IMAGEGEN_DESCRIPTION,
        "strict": false,
        "parameters": {
            "type": "object",
            "properties": {
                "prompt": {"type": "string"},
                "referenced_image_paths": {
                    "type": "array",
                    "items": {"type": "string"},
                    "maxItems": 5
                },
                "num_last_images_to_include": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 5
                }
            },
            "required": ["prompt"],
            "additionalProperties": false
        }
    })
}

fn codex_imagegen_namespace() -> Value {
    serde_json::json!({
        "type": "namespace",
        "name": CODEX_IMAGEGEN_NAMESPACE,
        "description": "Tools in the image_gen namespace.",
        "tools": [codex_imagegen_function()]
    })
}

fn is_codex_imagegen_namespace(tool: &Value) -> bool {
    tool.get("type").and_then(Value::as_str) == Some("namespace")
        && tool.get("name").and_then(Value::as_str) == Some(CODEX_IMAGEGEN_NAMESPACE)
}

fn is_codex_imagegen_function(tool: &Value) -> bool {
    tool.get("type").and_then(Value::as_str) == Some("function")
        && tool.get("name").and_then(Value::as_str) == Some(CODEX_IMAGEGEN_TOOL)
}

fn is_legacy_codex_imagegen_tool(tool: &Value) -> bool {
    tool.get("type").and_then(Value::as_str) == Some("image_gen.imagegen")
}

fn find_codex_imagegen_function(tools: &[Value]) -> Option<Value> {
    tools.iter().find_map(|tool| {
        if is_legacy_codex_imagegen_tool(tool) {
            return Some(tool.clone());
        }
        if is_codex_imagegen_namespace(tool) {
            return tool
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|namespace_tools| {
                    namespace_tools
                        .iter()
                        .find(|tool| is_codex_imagegen_function(tool))
                        .cloned()
                });
        }
        None
    })
}

fn codex_imagegen_schema_fingerprint(tool: &Value) -> String {
    serde_json::to_string(tool)
        .map(|value| short_sha256_fingerprint(&value))
        .unwrap_or_else(|_| "unavailable".to_string())
}

fn codex_imagegen_schema_diff_paths(
    before: &Value,
    after: &Value,
    path: &str,
    paths: &mut Vec<String>,
) {
    if paths.len() >= 16 {
        return;
    }
    match (before, after) {
        (Value::Object(before), Value::Object(after)) => {
            for key in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
                let next = if path.is_empty() {
                    key.to_string()
                } else {
                    format!("{path}.{key}")
                };
                match (before.get(key), after.get(key)) {
                    (Some(before), Some(after)) => {
                        codex_imagegen_schema_diff_paths(before, after, &next, paths)
                    }
                    _ => paths.push(next),
                }
                if paths.len() >= 16 {
                    break;
                }
            }
        }
        _ if before != after => paths.push(path.to_string()),
        _ => {}
    }
}

fn replace_codex_imagegen_in_tool_list(
    tools: &mut Vec<Value>,
    mode: crate::CodexImagegenRewriteMode,
) -> (bool, Option<Value>, &'static str) {
    use crate::CodexImagegenRewriteMode::*;

    let existing = find_codex_imagegen_function(tools);
    match mode {
        KeepOriginal => (false, existing, "no_change"),
        ForceRemove => {
            let mut removed = false;
            for namespace in tools
                .iter_mut()
                .filter(|tool| is_codex_imagegen_namespace(tool))
            {
                if let Some(namespace_tools) =
                    namespace.get_mut("tools").and_then(Value::as_array_mut)
                {
                    let original_len = namespace_tools.len();
                    namespace_tools.retain(|tool| !is_codex_imagegen_function(tool));
                    removed |= namespace_tools.len() != original_len;
                }
            }
            let original_len = tools.len();
            tools.retain(|tool| {
                if is_legacy_codex_imagegen_tool(tool) {
                    return false;
                }
                !is_codex_imagegen_namespace(tool)
                    || tool
                        .get("tools")
                        .and_then(Value::as_array)
                        .is_some_and(|items| !items.is_empty())
            });
            removed |= tools.len() != original_len;
            let outcome = if removed { "removed" } else { "no_change" };
            (removed, existing, outcome)
        }
        FillMissing if existing.is_some() => (false, existing, "no_change"),
        FillMissing => {
            let replacement = codex_imagegen_function();
            if let Some(namespace) = tools
                .iter_mut()
                .find(|tool| is_codex_imagegen_namespace(tool))
            {
                let Some(namespace_tools) =
                    namespace.get_mut("tools").and_then(Value::as_array_mut)
                else {
                    *namespace = codex_imagegen_namespace();
                    return (true, existing, "injected");
                };
                namespace_tools.push(replacement);
            } else {
                tools.push(codex_imagegen_namespace());
            }
            (true, existing, "injected")
        }
        ForceAdd => {
            let replacement = codex_imagegen_function();
            let original_len = tools.len();
            tools.retain(|tool| !is_legacy_codex_imagegen_tool(tool));
            let mut updated = tools.len() != original_len;

            let mut first_function_namespace = None;
            let mut first_function = None;
            let mut function_count = 0;
            for (namespace_index, namespace) in tools.iter().enumerate() {
                if !is_codex_imagegen_namespace(namespace) {
                    continue;
                }
                for function in namespace
                    .get("tools")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|tool| is_codex_imagegen_function(tool))
                {
                    if first_function.is_none() {
                        first_function_namespace = Some(namespace_index);
                        first_function = Some(function);
                    }
                    function_count += 1;
                }
            }
            let target_namespace_index = first_function_namespace
                .or_else(|| tools.iter().position(is_codex_imagegen_namespace));
            let already_canonical = function_count == 1
                && first_function.is_some_and(|function| function == &replacement);

            if let Some(target_namespace_index) = target_namespace_index {
                if !already_canonical {
                    for (namespace_index, namespace) in tools.iter_mut().enumerate() {
                        if !is_codex_imagegen_namespace(namespace) {
                            continue;
                        }
                        let Some(namespace_tools) =
                            namespace.get_mut("tools").and_then(Value::as_array_mut)
                        else {
                            if namespace_index == target_namespace_index {
                                *namespace = codex_imagegen_namespace();
                            }
                            continue;
                        };
                        if namespace_index == target_namespace_index {
                            let mut canonical_inserted = false;
                            let mut normalized_tools =
                                Vec::with_capacity(namespace_tools.len() + 1);
                            for tool in namespace_tools.drain(..) {
                                if is_codex_imagegen_function(&tool) {
                                    if !canonical_inserted {
                                        normalized_tools.push(replacement.clone());
                                        canonical_inserted = true;
                                    }
                                } else {
                                    normalized_tools.push(tool);
                                }
                            }
                            if !canonical_inserted {
                                normalized_tools.push(replacement.clone());
                            }
                            *namespace_tools = normalized_tools;
                        } else {
                            namespace_tools.retain(|tool| !is_codex_imagegen_function(tool));
                        }
                    }
                    updated = true;
                }
            } else {
                tools.push(codex_imagegen_namespace());
                updated = true;
            }

            let outcome = if existing.is_some() && updated {
                "replaced"
            } else if updated {
                "injected"
            } else {
                "no_change"
            };
            (updated, existing, outcome)
        }
    }
}

fn remove_hosted_image_generation(value: &mut Value) -> bool {
    fn remove_from_tools(tools: &mut Vec<Value>) -> bool {
        let original_len = tools.len();
        tools.retain(|tool| {
            tool.get("type")
                .and_then(Value::as_str)
                .is_none_or(|tool_type| tool_type.trim() != "image_generation")
        });
        tools.len() != original_len
    }

    let tool_choice_selects_image_generation =
        request_entry_openai_json_tool_choice_selects_image_generation(value);
    let Some(object) = value.as_object_mut() else {
        return false;
    };
    let mut modified = false;
    if let Some(tools) = object.get_mut("tools").and_then(Value::as_array_mut) {
        modified |= remove_from_tools(tools);
    }
    match object.get_mut("input") {
        Some(Value::Object(input)) => {
            if let Some(tools) = input
                .get_mut("additional_tools")
                .and_then(Value::as_array_mut)
            {
                modified |= remove_from_tools(tools);
            }
        }
        Some(Value::Array(input)) => {
            for item in input {
                if is_lite_developer_additional_tools(item)
                    && let Some(tools) = item.get_mut("tools").and_then(Value::as_array_mut)
                {
                    modified |= remove_from_tools(tools);
                }
            }
        }
        _ => {}
    }
    if tool_choice_selects_image_generation {
        object.remove("tool_choice");
        modified = true;
    }
    modified
}

fn normalise_lite_input_tools(value: &mut Value) -> Option<(&mut Vec<Value>, bool)> {
    let object = value.as_object_mut()?;
    let mut normalized = false;
    if !object.get("input").is_some_and(Value::is_array) {
        let original = object.remove("input").unwrap_or(Value::Null);
        let mut input = Vec::new();
        if let Value::Object(mut original) = original {
            if let Some(tools) = original.remove("additional_tools")
                && let Some(tools) = tools.as_array()
                && !tools.is_empty()
            {
                input.push(serde_json::json!({
                    "type": "additional_tools",
                    "role": "developer",
                    "tools": tools,
                }));
            }
            if !original.is_empty() {
                input.push(serde_json::json!({
                    "type": "message",
                    "role": "user",
                    "content": original,
                }));
            }
        } else if !original.is_null() {
            input.push(serde_json::json!({
                "type": "message",
                "role": "user",
                "content": original,
            }));
        }
        object.insert("input".to_string(), Value::Array(input));
        normalized = true;
    }
    object
        .get_mut("input")
        .and_then(Value::as_array_mut)
        .map(|input| (input, normalized))
}

fn take_lite_top_level_developer_tools(value: &mut Value) -> (Vec<Value>, bool) {
    let Some(tools) = value
        .as_object_mut()
        .and_then(|object| object.get_mut("tools"))
        .and_then(Value::as_array_mut)
    else {
        return (Vec::new(), false);
    };

    let mut developer_tools = Vec::new();
    let mut retained = Vec::with_capacity(tools.len());
    for tool in std::mem::take(tools) {
        if tool.get("type").and_then(Value::as_str) == Some("namespace")
            || is_legacy_codex_imagegen_tool(&tool)
        {
            developer_tools.push(tool);
        } else {
            retained.push(tool);
        }
    }
    let moved = !developer_tools.is_empty();
    *tools = retained;
    (developer_tools, moved)
}

fn is_lite_developer_additional_tools(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some("additional_tools")
        && item
            .get("role")
            .and_then(Value::as_str)
            .is_none_or(|role| role.eq_ignore_ascii_case("developer"))
}

fn ensure_lite_codex_execution_contract(value: &mut Value) -> bool {
    let Some(object) = value.as_object_mut() else {
        return false;
    };
    let mut changed = false;
    let reasoning = object
        .entry("reasoning".to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    if !reasoning.is_object() {
        *reasoning = Value::Object(Default::default());
        changed = true;
    }
    let reasoning = reasoning
        .as_object_mut()
        .expect("reasoning was normalized to an object");
    if reasoning.get("context").and_then(Value::as_str) != Some("all_turns") {
        reasoning.insert(
            "context".to_string(),
            Value::String("all_turns".to_string()),
        );
        changed = true;
    }
    if object.get("parallel_tool_calls").and_then(Value::as_bool) != Some(false) {
        object.insert("parallel_tool_calls".to_string(), Value::Bool(false));
        changed = true;
    }
    changed
}

fn rewrite_lite_codex_imagegen_tools(
    value: &mut Value,
    mode: crate::CodexImagegenRewriteMode,
) -> (bool, Option<Value>, &'static str) {
    use crate::CodexImagegenRewriteMode::*;

    let (top_level_developer_tools, migrated_top_level_developer_tools) =
        take_lite_top_level_developer_tools(value);
    let Some((input, input_normalized)) = normalise_lite_input_tools(value) else {
        return (false, None, "invalid_input");
    };
    let existing = input
        .iter()
        .find_map(|item| {
            is_lite_developer_additional_tools(item)
                .then(|| item.get("tools").and_then(Value::as_array))
                .flatten()
                .and_then(|tools| find_codex_imagegen_function(tools))
        })
        .or_else(|| find_codex_imagegen_function(&top_level_developer_tools));
    let first_developer_tools_position = input
        .iter()
        .position(is_lite_developer_additional_tools)
        .or_else(|| {
            let needs_developer_tools = !top_level_developer_tools.is_empty()
                || matches!(mode, ForceAdd)
                || (matches!(mode, FillMissing) && existing.is_none());
            needs_developer_tools.then(|| {
                input.insert(
                    0,
                    serde_json::json!({
                        "type": "additional_tools",
                        "role": "developer",
                        "tools": [],
                    }),
                );
                0
            })
        });
    let mut changed = input_normalized || migrated_top_level_developer_tools;

    if let Some(position) = first_developer_tools_position {
        let tools = input[position]
            .as_object_mut()
            .expect("additional tools item is an object")
            .entry("tools".to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if !tools.is_array() {
            *tools = Value::Array(Vec::new());
            changed = true;
        }
        if let Some(tools) = tools.as_array_mut() {
            tools.extend(top_level_developer_tools);
        }
    }

    let outcome = match mode {
        KeepOriginal => "no_change",
        FillMissing if existing.is_some() => "no_change",
        FillMissing => {
            let position =
                first_developer_tools_position.expect("fill missing creates developer tools");
            let tools = input[position]
                .get_mut("tools")
                .and_then(Value::as_array_mut)
                .expect("developer tools are normalized");
            let (tool_changed, _, outcome) = replace_codex_imagegen_in_tool_list(tools, mode);
            changed |= tool_changed;
            outcome
        }
        ForceRemove => {
            let mut removed = false;
            for item in input
                .iter_mut()
                .filter(|item| is_lite_developer_additional_tools(item))
            {
                if let Some(tools) = item.get_mut("tools").and_then(Value::as_array_mut) {
                    let (tool_changed, _, _) = replace_codex_imagegen_in_tool_list(tools, mode);
                    removed |= tool_changed;
                }
            }
            changed |= removed;
            if removed { "removed" } else { "no_change" }
        }
        ForceAdd => {
            let target_position =
                first_developer_tools_position.expect("force add creates developer tools");
            let target_already_canonical = input[target_position]
                .get("tools")
                .and_then(Value::as_array)
                .and_then(|tools| find_codex_imagegen_function(tools))
                .is_some_and(|tool| tool == codex_imagegen_function());
            let multiple_definitions = input
                .iter()
                .filter(|item| is_lite_developer_additional_tools(item))
                .filter_map(|item| item.get("tools").and_then(Value::as_array))
                .filter(|tools| find_codex_imagegen_function(tools).is_some())
                .count()
                > 1;
            if !target_already_canonical || multiple_definitions {
                for item in input
                    .iter_mut()
                    .filter(|item| is_lite_developer_additional_tools(item))
                {
                    if let Some(tools) = item.get_mut("tools").and_then(Value::as_array_mut) {
                        let (tool_changed, _, _) =
                            replace_codex_imagegen_in_tool_list(tools, ForceRemove);
                        changed |= tool_changed;
                    }
                }
                let tools = input[target_position]
                    .get_mut("tools")
                    .and_then(Value::as_array_mut)
                    .expect("developer tools are normalized");
                let (tool_changed, _, _) = replace_codex_imagegen_in_tool_list(tools, ForceAdd);
                changed |= tool_changed;
            }
            if existing.is_some() && !target_already_canonical {
                "replaced"
            } else if changed {
                "injected"
            } else {
                "no_change"
            }
        }
    };

    (changed, existing, outcome)
}

fn codex_imagegen_audit(
    protocol: CodexImagegenProtocol,
    mode: crate::CodexImagegenRewriteMode,
    outcome: &str,
    existing: Option<&Value>,
    hosted_removed: bool,
    reason: Option<&str>,
) -> Value {
    let canonical = codex_imagegen_function();
    let mut audit = serde_json::json!({
        "protocol": protocol.as_str(),
        "clientMatch": "codex_desktop",
        "mode": mode.as_str(),
        "outcome": outcome,
        "hostedRemoved": hosted_removed,
        "snapshotCommit": "61a44880a85d2fd0d8770908dea5733495e571c8",
        "injectedSchemaFingerprint": codex_imagegen_schema_fingerprint(&canonical),
    });
    if let Some(existing) = existing {
        audit["existingSchemaFingerprint"] =
            Value::String(codex_imagegen_schema_fingerprint(existing));
        let mut diff_paths = Vec::new();
        codex_imagegen_schema_diff_paths(existing, &canonical, "", &mut diff_paths);
        audit["schemaDiffPaths"] = serde_json::json!(diff_paths);
    }
    if let Some(reason) = reason {
        audit["reason"] = Value::String(reason.to_string());
    }
    audit
}

pub(crate) fn codex_imagegen_keep_original_audit(protocol: CodexImagegenProtocol) -> Value {
    codex_imagegen_audit(
        protocol,
        crate::CodexImagegenRewriteMode::KeepOriginal,
        "no_change",
        None,
        false,
        Some("policy_keep_original"),
    )
}

pub(crate) fn codex_imagegen_upstream_incompatibility(
    status: StatusCode,
    message: &str,
    audit: Option<&Value>,
) -> bool {
    if status != StatusCode::BAD_GATEWAY {
        return false;
    }
    codex_imagegen_audit_has_canonical_namespace(audit)
        && message
            .to_ascii_lowercase()
            .contains("upstream request failed")
}

pub(crate) fn codex_imagegen_audit_was_injected(audit: Option<&Value>) -> bool {
    audit
        .and_then(|value| value.get("outcome"))
        .and_then(Value::as_str)
        .is_some_and(|outcome| matches!(outcome, "injected" | "replaced"))
}

pub(crate) fn codex_imagegen_audit_has_canonical_namespace(audit: Option<&Value>) -> bool {
    if codex_imagegen_audit_was_injected(audit) {
        return true;
    }
    let Some(audit) = audit else {
        return false;
    };
    audit.get("outcome").and_then(Value::as_str) == Some("no_change")
        && matches!(
            audit.get("mode").and_then(Value::as_str),
            Some("force_add" | "fill_missing")
        )
        && audit.get("reason").and_then(Value::as_str) == Some("already_current")
        && audit
            .get("schemaDiffPaths")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
}

pub(crate) fn rewrite_codex_imagegen_tools(
    value: &mut Value,
    protocol: CodexImagegenProtocol,
    mode: crate::CodexImagegenRewriteMode,
    image_intent: crate::ImageIntent,
) -> (bool, Value) {
    use crate::CodexImagegenRewriteMode::*;

    if matches!(mode, KeepOriginal) {
        return (false, codex_imagegen_keep_original_audit(protocol));
    }
    // A non-default Codex policy always supersedes the hosted image tool path.
    let hosted_removed = remove_hosted_image_generation(value);
    if matches!(mode, FillMissing) && image_intent != crate::ImageIntent::Yes {
        return (
            hosted_removed,
            codex_imagegen_audit(
                protocol,
                mode,
                "skipped",
                None,
                hosted_removed,
                Some("image_intent_not_confirmed"),
            ),
        );
    }

    let (changed, existing, outcome) = match protocol {
        CodexImagegenProtocol::Full => {
            let Some(object) = value.as_object_mut() else {
                return (
                    hosted_removed,
                    codex_imagegen_audit(
                        protocol,
                        mode,
                        "invalid_request",
                        None,
                        hosted_removed,
                        None,
                    ),
                );
            };
            if matches!(mode, ForceRemove) {
                match object.get_mut("tools").and_then(Value::as_array_mut) {
                    Some(tools) => replace_codex_imagegen_in_tool_list(tools, mode),
                    None => (false, None, "no_change"),
                }
            } else {
                let tools_value = object
                    .entry("tools".to_string())
                    .or_insert_with(|| Value::Array(Vec::new()));
                if !tools_value.is_array() {
                    *tools_value = Value::Array(Vec::new());
                }
                let tools = tools_value
                    .as_array_mut()
                    .expect("tools was normalized to an array");
                replace_codex_imagegen_in_tool_list(tools, mode)
            }
        }
        CodexImagegenProtocol::Lite => {
            let (changed, existing, outcome) = rewrite_lite_codex_imagegen_tools(value, mode);
            let execution_contract_changed = ensure_lite_codex_execution_contract(value);
            (changed || execution_contract_changed, existing, outcome)
        }
    };
    let reason = (outcome == "no_change").then_some("already_current");
    let audit = codex_imagegen_audit(
        protocol,
        mode,
        outcome,
        existing.as_ref(),
        hosted_removed,
        reason,
    );
    if outcome == "replaced" {
        warn!(
            protocol = protocol.as_str(),
            mode = mode.as_str(),
            outcome,
            hosted_removed,
            existing_schema_fingerprint = ?audit.get("existingSchemaFingerprint"),
            injected_schema_fingerprint = ?audit.get("injectedSchemaFingerprint"),
            schema_diff_paths = ?audit.get("schemaDiffPaths"),
            "replaced conflicting Codex imagegen tool schema"
        );
    } else if changed || hosted_removed {
        info!(
            protocol = protocol.as_str(),
            mode = mode.as_str(),
            outcome,
            hosted_removed,
            existing_schema = existing.is_some(),
            injected_schema_fingerprint = ?audit.get("injectedSchemaFingerprint"),
            reason = ?audit.get("reason"),
            "rewrote Codex imagegen tool contract"
        );
    }
    (changed || hosted_removed, audit)
}

pub(crate) fn image_tool_rewrite_audit(
    target: ProxyCaptureTarget,
    responses_lite: bool,
    mode: crate::ImageToolRewriteMode,
) -> Option<Value> {
    if !matches!(
        target,
        ProxyCaptureTarget::Responses | ProxyCaptureTarget::ResponsesCompact
    ) {
        return None;
    }

    let (protocol, outcome, reason) = if responses_lite {
        (
            "responses_lite",
            "skipped",
            Some("responses_lite_client_owned_tools"),
        )
    } else if mode == crate::ImageToolRewriteMode::KeepOriginal {
        ("responses_full", "no_change", None)
    } else {
        ("responses_full", "applied", None)
    };
    let mut audit = serde_json::json!({
        "protocol": protocol,
        "mode": mode.as_str(),
        "outcome": outcome,
    });
    if let Some(reason) = reason {
        audit["reason"] = Value::String(reason.to_string());
    }
    Some(audit)
}

pub(crate) fn request_entry_openai_json_tool_choice_selects_image_generation(
    value: &Value,
) -> bool {
    let Some(tool_choice) = value.get("tool_choice") else {
        return false;
    };
    match tool_choice {
        Value::String(choice) => choice.trim() == "image_generation",
        Value::Object(choice) => {
            choice
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|tool_type| tool_type.trim() == "image_generation")
                || choice
                    .get("tool")
                    .and_then(Value::as_object)
                    .and_then(|tool| tool.get("type"))
                    .and_then(Value::as_str)
                    .is_some_and(|tool_type| tool_type.trim() == "image_generation")
                || choice
                    .get("function")
                    .and_then(Value::as_object)
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.trim() == "image_generation")
        }
        _ => false,
    }
}

pub(crate) fn rewrite_openai_responses_image_tools(
    value: &mut Value,
    rewrite_mode: crate::ImageToolRewriteMode,
    image_intent: crate::ImageIntent,
) -> bool {
    use crate::ImageToolRewriteMode::*;

    let has_image_tool = request_entry_openai_json_tools_contain_image_generation(value);
    let tool_choice_selects_image_generation =
        request_entry_openai_json_tool_choice_selects_image_generation(value);
    let Some(obj) = value.as_object_mut() else {
        return false;
    };

    match rewrite_mode {
        KeepOriginal => false,
        ForceRemove => {
            let mut modified = false;
            if let Some(tools) = obj.get_mut("tools").and_then(Value::as_array_mut) {
                let original_len = tools.len();
                tools.retain(|tool| {
                    tool.get("type")
                        .and_then(Value::as_str)
                        .is_none_or(|tool_type| tool_type.trim() != "image_generation")
                });
                modified |= tools.len() != original_len;
            }
            if tool_choice_selects_image_generation {
                obj.remove("tool_choice");
                modified = true;
            }
            modified
        }
        FillMissing | ForceAdd => {
            if matches!(rewrite_mode, FillMissing) && image_intent != crate::ImageIntent::Yes {
                return false;
            }

            let mut modified = false;
            if !has_image_tool {
                let tool = serde_json::json!({
                    "type": "image_generation",
                    "output_format": "png",
                });
                match obj.get_mut("tools") {
                    Some(Value::Array(tools)) => {
                        tools.push(tool);
                    }
                    Some(_) => {
                        obj.insert("tools".to_string(), Value::Array(vec![tool]));
                    }
                    None => {
                        obj.insert("tools".to_string(), Value::Array(vec![tool]));
                    }
                }
                modified = true;
            }
            if !obj.contains_key("tool_choice") {
                obj.insert(
                    "tool_choice".to_string(),
                    serde_json::json!({"type": "image_generation"}),
                );
                modified = true;
            }
            modified
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image_tool_rewrite_audit_marks_lite_tools_as_client_owned() {
        let audit = image_tool_rewrite_audit(
            ProxyCaptureTarget::Responses,
            true,
            crate::ImageToolRewriteMode::ForceAdd,
        )
        .expect("Responses requests should have an image-tool rewrite audit");
        assert_eq!(audit["protocol"], "responses_lite");
        assert_eq!(audit["mode"], "force_add");
        assert_eq!(audit["outcome"], "skipped");
        assert_eq!(audit["reason"], "responses_lite_client_owned_tools");
    }

    #[test]
    fn codex_imagegen_full_force_add_uses_top_level_namespace_and_replaces_conflicts() {
        let mut request = serde_json::json!({
            "input": "draw a test image",
            "tools": [
                {"type": "image_generation"},
                {"type": "namespace", "name": "web", "tools": [{"type": "function", "name": "search"}]},
                {"type": "namespace", "name": "image_gen", "tools": [
                    {"type": "function", "name": "imagegen", "parameters": {"type": "object"}},
                    {"type": "function", "name": "imagegen", "parameters": {"type": "object", "properties": {"legacy": {"type": "string"}}}},
                    {"type": "function", "name": "keep_me"}
                ]},
                {"type": "namespace", "name": "image_gen", "tools": [
                    {"type": "function", "name": "imagegen", "parameters": {"type": "object", "properties": {"other": {"type": "boolean"}}}},
                    {"type": "function", "name": "also_keep_me"}
                ]}
            ],
            "tool_choice": {"type": "image_generation"}
        });

        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Full,
            crate::CodexImagegenRewriteMode::ForceAdd,
            crate::ImageIntent::Yes,
        );

        assert!(changed);
        assert_eq!(audit["outcome"], "replaced");
        assert_eq!(audit["hostedRemoved"], true);
        assert!(audit["existingSchemaFingerprint"].is_string());
        assert!(
            audit["schemaDiffPaths"]
                .as_array()
                .is_some_and(|paths| !paths.is_empty())
        );
        assert!(request.get("tool_choice").is_none());
        let tools = request["tools"].as_array().expect("top-level tools array");
        assert!(tools.iter().any(|tool| tool["name"] == "web"));
        let imagegen = tools
            .iter()
            .filter(|tool| is_codex_imagegen_namespace(tool))
            .flat_map(|namespace| {
                namespace
                    .get("tools")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
            })
            .filter(|tool| is_codex_imagegen_function(tool))
            .collect::<Vec<_>>();
        assert_eq!(imagegen, vec![&codex_imagegen_function()]);
        assert!(tools.iter().any(|tool| {
            tool.get("tools")
                .and_then(Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item["name"] == "keep_me"))
        }));
        assert!(tools.iter().any(|tool| {
            tool.get("tools")
                .and_then(Value::as_array)
                .is_some_and(|items| items.iter().any(|item| item["name"] == "also_keep_me"))
        }));
    }

    #[test]
    fn codex_imagegen_lite_normalizes_input_and_preserves_other_tools() {
        let mut request = serde_json::json!({
            "input": "draw a test image",
            "tools": [{"type": "image_generation"}],
            "tool_choice": "image_generation",
            "reasoning": {"effort": "medium"},
            "parallel_tool_calls": true
        });

        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Lite,
            crate::CodexImagegenRewriteMode::ForceAdd,
            crate::ImageIntent::Yes,
        );

        assert!(changed);
        assert_eq!(audit["protocol"], "responses_lite");
        assert_eq!(audit["hostedRemoved"], true);
        assert_eq!(request["reasoning"]["context"], "all_turns");
        assert_eq!(request["parallel_tool_calls"], false);
        assert!(request.get("tool_choice").is_none());
        let input = request["input"].as_array().expect("normalized input array");
        assert_eq!(input[0]["type"], "additional_tools");
        assert_eq!(input[0]["role"], "developer");
        let tools = input[0]["tools"].as_array().expect("additional tools");
        assert_eq!(
            find_codex_imagegen_function(tools),
            Some(codex_imagegen_function())
        );
        assert_eq!(input[1]["content"], "draw a test image");
    }

    #[test]
    fn codex_imagegen_lite_migrates_top_level_namespaces_before_rewriting() {
        let web = serde_json::json!({
            "type": "namespace",
            "name": "web",
            "tools": [{"type": "function", "name": "search"}]
        });
        let legacy_imagegen = serde_json::json!({
            "type": "image_gen.imagegen",
            "version": "legacy"
        });
        let mut request = serde_json::json!({
            "input": [{"type": "message", "role": "user", "content": "draw"}],
            "tools": [
                {"type": "function", "name": "unrelated"},
                web.clone(),
                legacy_imagegen
            ]
        });

        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Lite,
            crate::CodexImagegenRewriteMode::ForceAdd,
            crate::ImageIntent::Yes,
        );

        assert!(changed);
        assert_eq!(audit["outcome"], "replaced");
        assert_eq!(
            request["tools"],
            serde_json::json!([{"type": "function", "name": "unrelated"}])
        );
        let tools = request["input"][0]["tools"]
            .as_array()
            .expect("Lite developer tools");
        assert!(tools.iter().any(|tool| tool == &web));
        assert!(
            tools
                .iter()
                .all(|tool| tool["type"] != "image_gen.imagegen")
        );
        assert_eq!(
            find_codex_imagegen_function(tools),
            Some(codex_imagegen_function())
        );

        let (changed, _) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Lite,
            crate::CodexImagegenRewriteMode::ForceRemove,
            crate::ImageIntent::Yes,
        );
        assert!(changed);
        let tools = request["input"][0]["tools"]
            .as_array()
            .expect("Lite developer tools after removal");
        assert!(tools.iter().any(|tool| tool == &web));
        assert!(find_codex_imagegen_function(tools).is_none());
    }

    #[test]
    fn codex_imagegen_lite_normalizes_object_additional_tools_and_removes_hosted_tools() {
        let web = serde_json::json!({
            "type": "namespace",
            "name": "web",
            "tools": [{"type": "function", "name": "search"}]
        });
        let mut request = serde_json::json!({
            "input": {
                "additional_tools": [
                    {"type": "image_generation"},
                    web.clone(),
                    {"type": "image_gen.imagegen", "version": "legacy"}
                ]
            }
        });

        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Lite,
            crate::CodexImagegenRewriteMode::ForceAdd,
            crate::ImageIntent::Yes,
        );

        assert!(changed);
        assert_eq!(audit["hostedRemoved"], true);
        let input = request["input"].as_array().expect("normalized input array");
        let tools = input[0]["tools"].as_array().expect("Lite developer tools");
        assert!(tools.iter().any(|tool| tool == &web));
        assert!(tools.iter().all(|tool| tool["type"] != "image_generation"));
        assert!(
            tools
                .iter()
                .all(|tool| tool["type"] != "image_gen.imagegen")
        );
        assert_eq!(
            find_codex_imagegen_function(tools),
            Some(codex_imagegen_function())
        );

        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Lite,
            crate::CodexImagegenRewriteMode::ForceRemove,
            crate::ImageIntent::Yes,
        );
        assert!(changed);
        assert_eq!(audit["outcome"], "removed");
        let tools = request["input"][0]["tools"]
            .as_array()
            .expect("Lite developer tools after removal");
        assert!(tools.iter().any(|tool| tool == &web));
        assert!(find_codex_imagegen_function(tools).is_none());
    }

    #[test]
    fn codex_imagegen_lite_preserves_separate_developer_tool_entries() {
        let first_tool = serde_json::json!({
            "type": "namespace",
            "name": "web",
            "tools": [{"type": "function", "name": "search"}]
        });
        let second_tool = serde_json::json!({
            "type": "namespace",
            "name": "mcp",
            "tools": [{"type": "function", "name": "lookup"}]
        });
        let migrated_tool = serde_json::json!({
            "type": "namespace",
            "name": "computer",
            "tools": [{"type": "function", "name": "screenshot"}]
        });
        let mut request = serde_json::json!({
            "input": [
                {"type": "message", "role": "user", "content": "draw"},
                {"type": "additional_tools", "role": "developer", "tools": [first_tool.clone()]},
                {"type": "additional_tools", "role": "developer", "tools": [second_tool.clone()]}
            ],
            "tools": [migrated_tool.clone()]
        });

        let (changed, _) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Lite,
            crate::CodexImagegenRewriteMode::ForceAdd,
            crate::ImageIntent::Yes,
        );

        assert!(changed);
        let input = request["input"].as_array().expect("Lite input array");
        assert_eq!(input.len(), 3);
        assert_eq!(input[0]["content"], "draw");
        let first_tools = input[1]["tools"].as_array().expect("first developer tools");
        assert!(first_tools.iter().any(|tool| tool == &first_tool));
        assert!(first_tools.iter().any(|tool| tool == &migrated_tool));
        assert!(find_codex_imagegen_function(first_tools).is_some());
        assert_eq!(input[2]["tools"], serde_json::json!([second_tool]));
    }

    #[test]
    fn codex_imagegen_force_remove_without_a_tool_reports_no_change() {
        let mut request = serde_json::json!({"input": "summarize this"});

        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Full,
            crate::CodexImagegenRewriteMode::ForceRemove,
            crate::ImageIntent::No,
        );

        assert!(!changed);
        assert_eq!(audit["outcome"], "no_change");
    }

    #[test]
    fn codex_imagegen_fill_missing_preserves_existing_schema_and_force_remove_keeps_other_tools() {
        let existing = serde_json::json!({
            "type": "namespace",
            "name": "image_gen",
            "tools": [{"type": "function", "name": "imagegen", "parameters": {"type": "object", "properties": {"legacy": {"type": "string"}}}}]
        });
        let mut request = serde_json::json!({
            "tools": [
                {"type": "namespace", "name": "web", "tools": [{"type": "function", "name": "search"}]},
                existing.clone()
            ]
        });

        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Full,
            crate::CodexImagegenRewriteMode::FillMissing,
            crate::ImageIntent::Yes,
        );
        assert!(!changed);
        assert_eq!(audit["outcome"], "no_change");
        assert_eq!(request["tools"][1], existing);

        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut request,
            CodexImagegenProtocol::Full,
            crate::CodexImagegenRewriteMode::ForceRemove,
            crate::ImageIntent::Yes,
        );
        assert!(changed);
        assert_eq!(audit["outcome"], "removed");
        let tools = request["tools"].as_array().expect("tools array");
        assert!(tools.iter().any(|tool| tool["name"] == "web"));
        assert!(find_codex_imagegen_function(tools).is_none());
    }

    #[test]
    fn codex_imagegen_keep_original_and_fill_missing_without_intent_do_not_inject() {
        let original = serde_json::json!({
            "input": "summarize this",
            "tools": [{"type": "image_generation"}],
            "tool_choice": {"type": "image_generation"}
        });
        let mut keep_original = original.clone();
        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut keep_original,
            CodexImagegenProtocol::Full,
            crate::CodexImagegenRewriteMode::KeepOriginal,
            crate::ImageIntent::Unknown,
        );
        assert!(!changed);
        assert_eq!(audit["outcome"], "no_change");
        assert_eq!(keep_original, original);

        let mut fill_missing = original;
        let (changed, audit) = rewrite_codex_imagegen_tools(
            &mut fill_missing,
            CodexImagegenProtocol::Full,
            crate::CodexImagegenRewriteMode::FillMissing,
            crate::ImageIntent::Unknown,
        );
        assert!(changed);
        assert_eq!(audit["outcome"], "skipped");
        assert_eq!(audit["hostedRemoved"], true);
        assert!(fill_missing["tools"].as_array().is_some_and(Vec::is_empty));
        assert!(fill_missing.get("tool_choice").is_none());
    }

    #[test]
    fn codex_imagegen_upstream_incompatibility_requires_a_canonical_namespace_and_known_502() {
        let injected = serde_json::json!({"outcome": "injected"});

        assert!(codex_imagegen_upstream_incompatibility(
            StatusCode::BAD_GATEWAY,
            "Upstream request failed",
            Some(&injected),
        ));
        assert!(!codex_imagegen_upstream_incompatibility(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Upstream request failed",
            Some(&injected),
        ));
        assert!(!codex_imagegen_upstream_incompatibility(
            StatusCode::BAD_GATEWAY,
            "Upstream request failed",
            Some(&serde_json::json!({"outcome": "no_change"})),
        ));
        let canonical_namespace_was_retained = serde_json::json!({
            "mode": "force_add",
            "outcome": "no_change",
            "reason": "already_current",
            "schemaDiffPaths": [],
        });
        assert!(codex_imagegen_upstream_incompatibility(
            StatusCode::BAD_GATEWAY,
            "Upstream request failed",
            Some(&canonical_namespace_was_retained),
        ));
        assert!(!codex_imagegen_upstream_incompatibility(
            StatusCode::BAD_GATEWAY,
            "temporary upstream timeout",
            Some(&injected),
        ));
    }

    #[tokio::test]
    async fn codex_imagegen_keep_original_records_audit_without_rewriting_snapshot() {
        let request = serde_json::json!({"input": "summarize this"});
        let request_bytes = serde_json::to_vec(&request).expect("serialize request fixture");
        let uri: Uri = "/v1/responses".parse().expect("responses uri");
        let snapshot = PoolReplayBodySnapshot::Memory(Bytes::from(request_bytes.clone()));

        let prepared = prepare_pool_request_body_for_account(
            79,
            Some(&snapshot),
            &uri,
            &Method::POST,
            Some("unsupported-encoding"),
            TagFastModeRewriteMode::KeepOriginal,
            crate::ImageToolRewriteMode::KeepOriginal,
            crate::CodexImagegenRewriteMode::KeepOriginal,
            Some(CodexImagegenProtocol::Full),
            None,
            None,
            None,
        )
        .await
        .expect("prepare keep-original Codex snapshot");

        assert!(!prepared.snapshot_is_decoded);
        assert_eq!(
            prepared
                .codex_imagegen_rewrite
                .as_ref()
                .and_then(|audit| audit.get("outcome"))
                .and_then(Value::as_str),
            Some("no_change")
        );
        assert_eq!(
            prepared
                .codex_imagegen_rewrite
                .as_ref()
                .and_then(|audit| audit.get("reason"))
                .and_then(Value::as_str),
            Some("policy_keep_original")
        );
        assert_eq!(
            prepared
                .snapshot
                .to_bytes()
                .await
                .expect("read original snapshot"),
            request_bytes
        );
    }

    #[tokio::test]
    async fn codex_imagegen_rewrites_gzip_and_file_replay_snapshots() {
        let request = serde_json::json!({
            "input": [{"type": "message", "role": "user", "content": "draw"}],
            "tools": [{"type": "namespace", "name": "web", "tools": []}]
        });
        let request_bytes = serde_json::to_vec(&request).expect("serialize request fixture");
        let mut gzip_encoder = GzEncoder::new(Vec::new(), Compression::default());
        gzip_encoder
            .write_all(&request_bytes)
            .expect("compress request fixture");
        let compressed = gzip_encoder
            .finish()
            .expect("finish compressed request fixture");
        let uri: Uri = "/v1/responses".parse().expect("responses uri");
        let compressed_snapshot = PoolReplayBodySnapshot::Memory(Bytes::from(compressed));

        let prepared = prepare_pool_request_body_for_account(
            77,
            Some(&compressed_snapshot),
            &uri,
            &Method::POST,
            Some("gzip"),
            TagFastModeRewriteMode::KeepOriginal,
            crate::ImageToolRewriteMode::KeepOriginal,
            crate::CodexImagegenRewriteMode::ForceAdd,
            Some(CodexImagegenProtocol::Full),
            None,
            None,
            None,
        )
        .await
        .expect("rewrite gzip snapshot");
        assert!(prepared.snapshot_is_decoded);
        assert_eq!(
            prepared
                .codex_imagegen_rewrite
                .as_ref()
                .and_then(|audit| audit.get("outcome"))
                .and_then(Value::as_str),
            Some("injected")
        );
        let prepared_json: Value = serde_json::from_slice(
            &prepared
                .snapshot
                .to_bytes()
                .await
                .expect("read rewritten gzip snapshot"),
        )
        .expect("decode rewritten gzip snapshot");
        assert!(
            find_codex_imagegen_function(
                prepared_json["tools"].as_array().expect("rewritten tools")
            )
            .is_some()
        );

        let file_path = build_pool_replay_temp_path(78);
        tokio::fs::write(&file_path, request_bytes)
            .await
            .expect("write file replay fixture");
        let file_snapshot = PoolReplayBodySnapshot::File {
            temp_file: Arc::new(PoolReplayTempFile { path: file_path }),
            size: request.to_string().len(),
        };
        let prepared = prepare_pool_request_body_for_account(
            78,
            Some(&file_snapshot),
            &uri,
            &Method::POST,
            None,
            TagFastModeRewriteMode::KeepOriginal,
            crate::ImageToolRewriteMode::KeepOriginal,
            crate::CodexImagegenRewriteMode::ForceAdd,
            Some(CodexImagegenProtocol::Full),
            None,
            None,
            None,
        )
        .await
        .expect("rewrite file replay snapshot");
        assert!(prepared.snapshot_is_decoded);
        let prepared_json: Value = serde_json::from_slice(
            &prepared
                .snapshot
                .to_bytes()
                .await
                .expect("read rewritten file snapshot"),
        )
        .expect("decode rewritten file snapshot");
        assert!(
            find_codex_imagegen_function(
                prepared_json["tools"].as_array().expect("rewritten tools")
            )
            .is_some()
        );
    }

    #[test]
    fn codex_imagegen_client_identification_does_not_guess_from_session_headers() {
        let mut lite_headers = HeaderMap::new();
        lite_headers.insert(
            "x-openai-internal-codex-responses-lite",
            HeaderValue::from_static("true"),
        );
        assert_eq!(
            codex_imagegen_protocol_from_headers(&lite_headers),
            Some(CodexImagegenProtocol::Lite)
        );

        let mut full_headers = HeaderMap::new();
        full_headers.insert("originator", HeaderValue::from_static("Codex Desktop"));
        assert_eq!(
            codex_imagegen_protocol_from_headers(&full_headers),
            Some(CodexImagegenProtocol::Full)
        );

        let mut user_agent_headers = HeaderMap::new();
        user_agent_headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static("Codex Desktop/5.6"),
        );
        assert_eq!(
            codex_imagegen_protocol_from_headers(&user_agent_headers),
            Some(CodexImagegenProtocol::Full)
        );

        let mut unrelated_headers = HeaderMap::new();
        unrelated_headers.insert("session_id", HeaderValue::from_static("codex-desktop"));
        assert_eq!(
            codex_imagegen_protocol_from_headers(&unrelated_headers),
            None
        );
    }

    #[test]
    fn codex_imagegen_intent_does_not_require_hosted_image_capability() {
        let full = serde_json::json!({
            "tools": [{
                "type": "namespace",
                "name": "image_gen",
                "tools": [codex_imagegen_function()]
            }]
        });
        let lite = serde_json::json!({
            "input": [{
                "type": "additional_tools",
                "tools": [{
                    "type": "namespace",
                    "name": "image_gen",
                    "tools": [codex_imagegen_function()]
                }]
            }]
        });

        let lite_object = serde_json::json!({
            "input": {
                "additional_tools": [{"type": "image_gen.imagegen"}]
            }
        });

        for request in [&full, &lite, &lite_object] {
            assert_eq!(
                infer_image_intent_from_request_body(ProxyCaptureTarget::Responses, request),
                ImageIntent::Yes
            );
            assert_eq!(
                infer_hosted_image_intent_from_request_body(ProxyCaptureTarget::Responses, request),
                ImageIntent::No
            );
        }
    }
}
