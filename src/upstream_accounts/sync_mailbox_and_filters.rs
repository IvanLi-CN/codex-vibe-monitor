#[derive(Debug, Clone)]
struct ParsedMailboxCode {
    value: String,
    source: String,
    updated_at: String,
}

#[derive(Debug, Clone)]
struct ParsedMailboxInvite {
    subject: String,
    copy_value: String,
    copy_label: String,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoeMailConfigPayload {
    email_domains: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoeMailGenerateEmailPayload {
    id: String,
    email: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoeMailEmailListPayload {
    emails: Vec<MoeMailEmailSummary>,
    next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoeMailEmailSummary {
    id: String,
    address: String,
    expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoeMailMessageListPayload {
    messages: Vec<MoeMailMessageSummary>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoeMailMessageSummary {
    id: String,
    subject: Option<String>,
    received_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoeMailMessageDetailPayload {
    message: MoeMailMessageDetail,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoeMailMessageDetail {
    id: String,
    subject: Option<String>,
    content: Option<String>,
    html: Option<String>,
    received_at: Option<String>,
}

const MAILBOX_CODE_CONTEXT_WINDOW_BYTES: usize = 64;
const OAUTH_BRAND_MARKERS: &[&str] = &["openai", "chatgpt"];
const OAUTH_STRONG_CODE_MARKERS: &[&str] = &[
    "verification code",
    "temporary verification code",
    "one-time code",
    "one time code",
    "security code",
    "验证码",
    "驗證碼",
    "校验码",
    "校驗碼",
    "验证代码",
    "驗證代碼",
    "認證碼",
    "認証コード",
    "인증 코드",
    "인증번호",
];
const OAUTH_WEAK_CODE_MARKERS: &[&str] = &[
    "your code",
    "code is",
    "code:",
    "temporary code",
    "代码为",
    "代碼為",
    "代码是",
    "代碼是",
    "臨時代碼",
    "临时代码",
];
const OAUTH_INVITE_SUBJECT_MARKERS: &[&str] = &[
    "has invited you",
    "invited you to",
    "invite you to",
    "邀请你",
    "邀請你",
    "邀请您",
    "邀請您",
    "招待",
    "초대",
];
const OAUTH_INVITE_BODY_MARKERS: &[&str] = &[
    "join workspace",
    "join the workspace",
    "accept invitation",
    "accept invite",
    "workspace invite",
    "accept the invitation",
    "加入工作区",
    "加入工作區",
    "加入工作空间",
    "加入工作空間",
    "接受邀请",
    "接受邀請",
    "接受此邀请",
    "接受此邀請",
    "ワークスペース",
    "招待",
    "워크스페이스",
    "초대 수락",
];
static OAUTH_CODE_CANDIDATE_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:^|[^0-9])([0-9]{4,8})(?:[^0-9]|$)").expect("valid oauth code candidate regex")
});
static URL_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"https?://[^\s"'<>)]+"#).expect("valid url regex"));
static HTML_TAG_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<[^>]+>").expect("valid html tag regex"));
static BASIC_EMAIL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^[a-z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$")
        .expect("valid basic email regex")
});

fn oauth_mailbox_status_from_row(row: &OauthMailboxSessionRow) -> OauthMailboxStatus {
    OauthMailboxStatus {
        session_id: row.session_id.clone(),
        email_address: row.email_address.clone(),
        expires_at: row.expires_at.clone(),
        latest_code: match (
            row.latest_code_value.clone(),
            row.latest_code_source.clone(),
            row.latest_code_updated_at.clone(),
        ) {
            (Some(value), Some(source), Some(updated_at)) => Some(OauthMailboxCodeSummary {
                value,
                source,
                updated_at,
            }),
            _ => None,
        },
        invite: match (
            row.invite_subject.clone(),
            row.invite_copy_value.clone(),
            row.invite_copy_label.clone(),
            row.invite_updated_at.clone(),
        ) {
            (Some(subject), Some(copy_value), Some(copy_label), Some(updated_at)) => {
                Some(OauthInviteSummary {
                    subject,
                    copy_value,
                    copy_label,
                    updated_at,
                })
            }
            _ => None,
        },
        invited: row.invited != 0,
        error: None,
    }
}

fn oauth_mailbox_session_supported_response(
    session_id: String,
    email_address: String,
    expires_at: String,
    source: &str,
) -> OauthMailboxSessionResponse {
    OauthMailboxSessionResponse {
        email_address,
        supported: true,
        session_id: Some(session_id),
        expires_at: Some(expires_at),
        source: Some(source.to_string()),
        reason: None,
    }
}

fn oauth_mailbox_session_unsupported_response(
    email_address: String,
    reason: &str,
) -> OauthMailboxSessionResponse {
    OauthMailboxSessionResponse {
        email_address,
        supported: false,
        session_id: None,
        expires_at: None,
        source: None,
        reason: Some(reason.to_string()),
    }
}

fn normalize_mailbox_address(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

fn normalize_mailbox_domain(value: &str) -> Option<String> {
    let trimmed = value
        .trim()
        .trim_matches(|ch: char| ch.is_whitespace() || ch == '"' || ch == '\'');
    if trimmed.is_empty() {
        return None;
    }
    let without_prefix = trimmed.trim_start_matches('@');
    let domain_like = without_prefix
        .rsplit_once('@')
        .map(|(_, domain)| domain)
        .unwrap_or(without_prefix)
        .trim()
        .trim_start_matches('@')
        .trim_end_matches('.');
    if domain_like.is_empty() {
        return None;
    }
    Some(domain_like.to_ascii_lowercase())
}

fn moemail_supported_domains(payload: &MoeMailConfigPayload) -> HashSet<String> {
    payload
        .email_domains
        .as_deref()
        .unwrap_or_default()
        .split(|ch: char| matches!(ch, ',' | ';' | '\n' | '\r'))
        .filter_map(normalize_mailbox_domain)
        .collect()
}

fn mailbox_local_part(value: &str) -> Option<&str> {
    let (local_part, domain) = value.split_once('@')?;
    if local_part.is_empty() || domain.is_empty() {
        return None;
    }
    Some(local_part)
}

#[derive(Debug, PartialEq, Eq)]
enum RequestedManualMailboxAddress {
    Missing,
    Valid(String),
    Invalid(String),
}

fn requested_manual_mailbox_address(
    raw_email_address: Option<&str>,
) -> RequestedManualMailboxAddress {
    match raw_email_address {
        None => RequestedManualMailboxAddress::Missing,
        Some(value) => match normalize_mailbox_address(value) {
            Some(normalized) => RequestedManualMailboxAddress::Valid(normalized),
            None => RequestedManualMailboxAddress::Invalid(value.to_string()),
        },
    }
}

fn mailbox_address_is_valid(value: &str) -> bool {
    BASIC_EMAIL_REGEX.is_match(value.trim())
}

fn upstream_mailbox_config(
    config: &AppConfig,
) -> Result<&UpstreamAccountsMoeMailConfig, (StatusCode, String)> {
    config.upstream_accounts_moemail.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "oauth temp mail requires {}, {}, and {}",
                ENV_UPSTREAM_ACCOUNTS_MOEMAIL_BASE_URL,
                ENV_UPSTREAM_ACCOUNTS_MOEMAIL_API_KEY,
                ENV_UPSTREAM_ACCOUNTS_MOEMAIL_DEFAULT_DOMAIN
            ),
        )
    })
}

fn validate_mailbox_binding_fields(
    mailbox_session_id: Option<&str>,
    mailbox_address: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    match (mailbox_session_id, mailbox_address) {
        (Some(_), Some(_)) | (None, None) => Ok(()),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "mailboxSessionId and mailboxAddress must be provided together".to_string(),
        )),
    }
}

fn mailbox_addresses_match(left: Option<&str>, right: Option<&str>) -> bool {
    normalize_mailbox_address(left.unwrap_or_default())
        == normalize_mailbox_address(right.unwrap_or_default())
}

fn expired_mailbox_session_requires_remote_delete(row: &OauthMailboxSessionRow) -> bool {
    row.mailbox_source.as_deref() != Some(OAUTH_MAILBOX_SOURCE_ATTACHED)
}

fn normalize_mailbox_session_expires_at(value: Option<&str>, fallback: DateTime<Utc>) -> String {
    value
        .and_then(|raw| {
            DateTime::parse_from_rfc3339(raw)
                .ok()
                .map(|parsed| format_utc_iso(parsed.with_timezone(&Utc)))
        })
        .unwrap_or_else(|| format_utc_iso(fallback))
}

async fn validate_mailbox_binding(
    pool: &Pool<Sqlite>,
    mailbox_session_id: Option<&str>,
    mailbox_address: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    validate_mailbox_binding_fields(mailbox_session_id, mailbox_address)?;
    let Some(session_id) = mailbox_session_id else {
        return Ok(());
    };
    let Some(expected_address) = mailbox_address else {
        return Ok(());
    };
    let row = load_oauth_mailbox_session(pool, session_id)
        .await
        .map_err(internal_error_tuple)?
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "mailbox session is missing or expired".to_string(),
            )
        })?;
    if normalize_mailbox_address(&row.email_address) != normalize_mailbox_address(expected_address)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "mailboxAddress no longer matches the mailbox session".to_string(),
        ));
    }
    Ok(())
}

fn strip_html_tags(raw: &str) -> String {
    HTML_TAG_REGEX.replace_all(raw, " ").into_owned()
}

fn normalize_mailbox_text(raw: &str) -> String {
    let mut normalized = String::with_capacity(raw.len());
    let mut previous_was_space = true;

    for ch in raw.chars() {
        let mapped = match ch {
            '\u{00a0}' | '\u{3000}' => ' ',
            '０'..='９' => {
                char::from_u32(u32::from(ch) - u32::from('０') + u32::from('0')).unwrap_or(ch)
            }
            'Ａ'..='Ｚ' => {
                char::from_u32(u32::from(ch) - u32::from('Ａ') + u32::from('a')).unwrap_or(ch)
            }
            'ａ'..='ｚ' => {
                char::from_u32(u32::from(ch) - u32::from('ａ') + u32::from('a')).unwrap_or(ch)
            }
            '：' => ':',
            '－' => '-',
            '／' => '/',
            '．' => '.',
            '，' => ',',
            '（' => '(',
            '）' => ')',
            '【' => '[',
            '】' => ']',
            _ if ch.is_ascii_uppercase() => ch.to_ascii_lowercase(),
            _ => ch,
        };

        if mapped.is_whitespace() {
            if !previous_was_space && !normalized.is_empty() {
                normalized.push(' ');
            }
            previous_was_space = true;
        } else {
            normalized.push(mapped);
            previous_was_space = false;
        }
    }

    normalized.trim().to_string()
}

fn mailbox_text_contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn mailbox_text_has_brand(text: &str) -> bool {
    mailbox_text_contains_any(text, OAUTH_BRAND_MARKERS)
}

fn clamp_mailbox_context_start(raw: &str, index: usize) -> usize {
    let mut candidate = index.min(raw.len());
    while candidate > 0 && !raw.is_char_boundary(candidate) {
        candidate -= 1;
    }
    candidate
}

fn clamp_mailbox_context_end(raw: &str, index: usize) -> usize {
    let mut candidate = index.min(raw.len());
    while candidate < raw.len() && !raw.is_char_boundary(candidate) {
        candidate += 1;
    }
    candidate
}

fn mailbox_context_slice(raw: &str, start: usize, end: usize, radius: usize) -> &str {
    let context_start = clamp_mailbox_context_start(raw, start.saturating_sub(radius));
    let context_end = clamp_mailbox_context_end(raw, end.saturating_add(radius));
    &raw[context_start..context_end]
}

fn mailbox_context_before(raw: &str, index: usize, radius: usize) -> &str {
    let context_start = clamp_mailbox_context_start(raw, index.saturating_sub(radius));
    let context_end = clamp_mailbox_context_end(raw, index);
    &raw[context_start..context_end]
}

fn extract_mailbox_code_candidate(text: &str, message_has_brand: bool) -> Option<String> {
    let mut best_match: Option<(u8, usize, String)> = None;

    for captures in OAUTH_CODE_CANDIDATE_REGEX.captures_iter(text) {
        let whole_match = captures.get(0)?;
        let digit_match = captures.get(1)?;
        let context = mailbox_context_slice(
            text,
            whole_match.start(),
            whole_match.end(),
            MAILBOX_CODE_CONTEXT_WINDOW_BYTES,
        );
        let prefix_context =
            mailbox_context_before(text, digit_match.start(), MAILBOX_CODE_CONTEXT_WINDOW_BYTES);
        let context_has_strong_code =
            mailbox_text_contains_any(prefix_context, OAUTH_STRONG_CODE_MARKERS);
        let context_has_weak_code =
            mailbox_text_contains_any(prefix_context, OAUTH_WEAK_CODE_MARKERS);
        let context_has_brand =
            mailbox_text_has_brand(prefix_context) || mailbox_text_has_brand(context);
        let score = if context_has_strong_code {
            3
        } else if context_has_weak_code && (message_has_brand || context_has_brand) {
            2
        } else {
            0
        };

        if score == 0 {
            continue;
        }

        let candidate = (score, digit_match.start(), digit_match.as_str().to_string());
        if best_match
            .as_ref()
            .map(|existing| {
                candidate.0 > existing.0 || (candidate.0 == existing.0 && candidate.1 < existing.1)
            })
            .unwrap_or(true)
        {
            best_match = Some(candidate);
        }
    }

    best_match.map(|(_, _, value)| value)
}

fn mailbox_url_candidate_urls(url: &str) -> Vec<String> {
    let mut candidates = vec![url.trim_end_matches('.').to_string()];
    let Ok(parsed) = Url::parse(url) else {
        return candidates;
    };

    for value in parsed
        .query_pairs()
        .map(|(_, value)| value.into_owned())
        .chain(parsed.fragment().map(ToOwned::to_owned))
    {
        if let Some(nested) = URL_REGEX.find(&value) {
            let nested = nested.as_str().trim_end_matches('.').to_string();
            if !candidates.iter().any(|existing| existing == &nested) {
                candidates.push(nested);
            }
        }
    }

    candidates
}

fn mailbox_url_looks_like_direct_invite(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };

    let host = host.to_ascii_lowercase();
    let path = parsed.path().to_ascii_lowercase();
    let query = parsed.query().unwrap_or_default().to_ascii_lowercase();
    let combined = if query.is_empty() {
        format!("{host}{path}")
    } else {
        format!("{host}{path}?{query}")
    };
    let has_invite_action = combined.contains("invite")
        || combined.contains("invitation")
        || combined.contains("accept");
    let has_workspace_context =
        combined.contains("workspace") || host.contains("chatgpt") || host.contains("openai");
    let is_help_like = host.starts_with("help.")
        || host.starts_with("docs.")
        || host.contains("support")
        || path.contains("/articles/")
        || path.contains("/hc/")
        || path.contains("/help/")
        || path.contains("/docs/");

    has_invite_action && has_workspace_context && !is_help_like
}

fn mailbox_url_resolve_invite_target(url: &str) -> Option<String> {
    let candidates = mailbox_url_candidate_urls(url);
    candidates
        .iter()
        .skip(1)
        .find(|candidate| mailbox_url_looks_like_direct_invite(candidate))
        .cloned()
        .or_else(|| {
            candidates
                .into_iter()
                .next()
                .filter(|candidate| mailbox_url_looks_like_direct_invite(candidate))
        })
}

fn mailbox_url_has_brand(url: &str) -> bool {
    mailbox_url_candidate_urls(url)
        .into_iter()
        .any(|candidate| {
            let lower = candidate.to_ascii_lowercase();
            lower.contains("openai") || lower.contains("chatgpt")
        })
}

fn parse_mailbox_code(detail: &MoeMailMessageDetail) -> Option<ParsedMailboxCode> {
    let subject = detail.subject.as_deref().unwrap_or_default();
    let content = detail.content.as_deref().unwrap_or_default();
    let html_text = strip_html_tags(detail.html.as_deref().unwrap_or_default());
    let message_context = normalize_mailbox_text(&format!("{subject}\n{content}\n{html_text}"));
    let message_has_brand = mailbox_text_has_brand(&message_context);

    let subject_text = normalize_mailbox_text(subject);
    let subject_has_brand = mailbox_text_has_brand(&subject_text);
    if subject_has_brand {
        if let Some(value) = extract_mailbox_code_candidate(&subject_text, subject_has_brand) {
            return Some(ParsedMailboxCode {
                value,
                source: "subject".to_string(),
                updated_at: detail
                    .received_at
                    .clone()
                    .unwrap_or_else(|| format_utc_iso(Utc::now())),
            });
        }
    }

    for (source, raw) in [
        ("content", content.to_string()),
        ("html", html_text.clone()),
    ] {
        let normalized = normalize_mailbox_text(&raw);
        if let Some(value) = extract_mailbox_code_candidate(&normalized, message_has_brand) {
            return Some(ParsedMailboxCode {
                value,
                source: source.to_string(),
                updated_at: detail
                    .received_at
                    .clone()
                    .unwrap_or_else(|| format_utc_iso(Utc::now())),
            });
        }
    }

    None
}

fn parse_mailbox_invite(detail: &MoeMailMessageDetail) -> Option<ParsedMailboxInvite> {
    let subject = detail.subject.as_deref().unwrap_or_default().trim();
    if subject.is_empty() {
        return None;
    }

    let stripped_html = strip_html_tags(detail.html.as_deref().unwrap_or_default());
    let subject_text = normalize_mailbox_text(subject);
    let body_text = normalize_mailbox_text(&format!(
        "{}\n{}",
        detail.content.as_deref().unwrap_or_default(),
        stripped_html
    ));
    let subject_has_invite_semantics =
        mailbox_text_contains_any(&subject_text, OAUTH_INVITE_SUBJECT_MARKERS);
    let body_has_invite_semantics =
        mailbox_text_contains_any(&body_text, OAUTH_INVITE_BODY_MARKERS);

    let body_with_urls = format!(
        "{}\n{}",
        detail.content.as_deref().unwrap_or_default(),
        stripped_html
    );
    let copy_value = URL_REGEX
        .find_iter(&body_with_urls)
        .find_map(|value| mailbox_url_resolve_invite_target(value.as_str()))?;
    let body_can_drive_invite = body_has_invite_semantics;
    if !subject_has_invite_semantics && !body_can_drive_invite {
        return None;
    }
    if !mailbox_text_has_brand(&format!("{subject_text}\n{body_text}"))
        && !mailbox_url_has_brand(&copy_value)
    {
        return None;
    }

    Some(ParsedMailboxInvite {
        subject: subject.to_string(),
        copy_label: "invite-link".to_string(),
        copy_value,
        updated_at: detail
            .received_at
            .clone()
            .unwrap_or_else(|| format_utc_iso(Utc::now())),
    })
}

fn parsed_code_from_mailbox_row(row: &OauthMailboxSessionRow) -> Option<ParsedMailboxCode> {
    Some(ParsedMailboxCode {
        value: row.latest_code_value.clone()?,
        source: row.latest_code_source.clone()?,
        updated_at: row.latest_code_updated_at.clone()?,
    })
}

fn parsed_invite_from_mailbox_row(row: &OauthMailboxSessionRow) -> Option<ParsedMailboxInvite> {
    Some(ParsedMailboxInvite {
        subject: row.invite_subject.clone()?,
        copy_value: row.invite_copy_value.clone()?,
        copy_label: row.invite_copy_label.clone()?,
        updated_at: row.invite_updated_at.clone()?,
    })
}

fn mailbox_updated_at_is_newer_or_equal(candidate: &str, baseline: &str) -> bool {
    match (parse_rfc3339_utc(candidate), parse_rfc3339_utc(baseline)) {
        (Some(candidate), Some(baseline)) => candidate >= baseline,
        _ => candidate >= baseline,
    }
}

fn merge_mailbox_code(
    fresh: Option<ParsedMailboxCode>,
    stored: Option<ParsedMailboxCode>,
) -> Option<ParsedMailboxCode> {
    match (fresh, stored) {
        (Some(fresh), Some(stored)) => {
            if mailbox_updated_at_is_newer_or_equal(&fresh.updated_at, &stored.updated_at) {
                Some(fresh)
            } else {
                Some(stored)
            }
        }
        (Some(fresh), None) => Some(fresh),
        (None, Some(stored)) => Some(stored),
        (None, None) => None,
    }
}

fn merge_mailbox_invite(
    fresh: Option<ParsedMailboxInvite>,
    stored: Option<ParsedMailboxInvite>,
) -> Option<ParsedMailboxInvite> {
    match (fresh, stored) {
        (Some(fresh), Some(stored)) => {
            if mailbox_updated_at_is_newer_or_equal(&fresh.updated_at, &stored.updated_at) {
                Some(fresh)
            } else {
                Some(stored)
            }
        }
        (Some(fresh), None) => Some(fresh),
        (None, Some(stored)) => Some(stored),
        (None, None) => None,
    }
}

fn sort_mailbox_messages_desc(messages: &mut [MoeMailMessageSummary]) {
    messages.sort_by(|left, right| right.received_at.cmp(&left.received_at));
}

fn latest_mailbox_message_id(messages: &[MoeMailMessageSummary]) -> Option<String> {
    messages.first().map(|message| message.id.clone())
}

fn collect_unseen_mailbox_messages(
    messages: Vec<MoeMailMessageSummary>,
    last_message_id: Option<&str>,
) -> Vec<MoeMailMessageSummary> {
    let Some(last_message_id) = last_message_id.filter(|value| !value.trim().is_empty()) else {
        return messages;
    };

    let mut unseen = Vec::new();
    for message in messages {
        if message.id == last_message_id {
            break;
        }
        unseen.push(message);
    }
    unseen
}

fn next_mailbox_cursor_after_refresh(
    previous_last_message_id: Option<&str>,
    processed_messages: &[MoeMailMessageSummary],
) -> Option<String> {
    processed_messages
        .first()
        .map(|message| message.id.clone())
        .or_else(|| previous_last_message_id.map(ToOwned::to_owned))
}

async fn resolve_mailbox_message_state(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    remote_email_id: &str,
    messages: &[MoeMailMessageSummary],
) -> Result<(Option<ParsedMailboxCode>, Option<ParsedMailboxInvite>)> {
    let mut latest_code = None;
    let mut latest_invite = None;
    for summary in messages.iter() {
        if latest_code.is_some() && latest_invite.is_some() {
            break;
        }
        let detail = moemail_get_message(client, config, remote_email_id, &summary.id).await?;
        if latest_code.is_none() {
            latest_code = parse_mailbox_code(&detail);
        }
        if latest_invite.is_none() {
            latest_invite = parse_mailbox_invite(&detail);
        }
    }

    Ok((latest_code, latest_invite))
}

enum MoeMailAttachReadState<T> {
    Readable(T),
    NotReadable,
}

fn moemail_attach_status_is_not_readable(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::NOT_FOUND
    )
}

async fn resolve_mailbox_message_state_for_attach(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    remote_email_id: &str,
    messages: &[MoeMailMessageSummary],
) -> Result<MoeMailAttachReadState<(Option<ParsedMailboxCode>, Option<ParsedMailboxInvite>)>> {
    let mut latest_code = None;
    let mut latest_invite = None;
    for summary in messages.iter() {
        if latest_code.is_some() && latest_invite.is_some() {
            break;
        }
        let detail =
            match moemail_get_message_for_attach(client, config, remote_email_id, &summary.id)
                .await?
            {
                MoeMailAttachReadState::Readable(detail) => detail,
                MoeMailAttachReadState::NotReadable => {
                    return Ok(MoeMailAttachReadState::NotReadable);
                }
            };
        if latest_code.is_none() {
            latest_code = parse_mailbox_code(&detail);
        }
        if latest_invite.is_none() {
            latest_invite = parse_mailbox_invite(&detail);
        }
    }

    Ok(MoeMailAttachReadState::Readable((
        latest_code,
        latest_invite,
    )))
}

async fn moemail_create_email(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
) -> Result<MoeMailGenerateEmailPayload> {
    let local_name = generate_mailbox_local_name().map_err(|(_, message)| anyhow!(message))?;
    moemail_create_email_with_name_and_domain(client, config, &local_name, &config.default_domain)
        .await
}

async fn moemail_create_email_for_address(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    email_address: &str,
) -> Result<MoeMailGenerateEmailPayload> {
    let requested_email = normalize_mailbox_address(email_address)
        .ok_or_else(|| anyhow!("manual moemail address must not be blank"))?;
    let requested_domain = normalize_mailbox_domain(&requested_email)
        .ok_or_else(|| anyhow!("manual moemail domain is invalid"))?;
    let requested_local = mailbox_local_part(&requested_email)
        .ok_or_else(|| anyhow!("manual moemail local part is invalid"))?;
    let generated = moemail_create_email_with_name_and_domain(
        client,
        config,
        requested_local,
        &requested_domain,
    )
    .await?;
    let generated_email = normalize_mailbox_address(&generated.email)
        .ok_or_else(|| anyhow!("generated moemail address must not be blank"))?;
    if generated_email != requested_email {
        bail!(
            "generated moemail address {} does not match requested {}",
            generated.email,
            requested_email
        );
    }
    Ok(generated)
}

async fn moemail_create_email_with_name_and_domain(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    local_name: &str,
    domain: &str,
) -> Result<MoeMailGenerateEmailPayload> {
    let response = client
        .post(
            config
                .base_url
                .join("/api/emails/generate")
                .context("invalid moemail generate endpoint")?,
        )
        .header("X-API-Key", config.api_key.as_str())
        .json(&json!({
            "name": local_name,
            "expiryTime": DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS * 1000,
            "domain": domain,
        }))
        .send()
        .await
        .context("failed to create moemail mailbox")?
        .error_for_status()
        .context("moemail mailbox creation request failed")?;

    response
        .json::<MoeMailGenerateEmailPayload>()
        .await
        .context("failed to decode moemail create mailbox response")
}

async fn moemail_get_config(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
) -> Result<MoeMailConfigPayload> {
    let response = client
        .get(
            config
                .base_url
                .join("/api/config")
                .context("invalid moemail config endpoint")?,
        )
        .header("X-API-Key", config.api_key.as_str())
        .send()
        .await
        .context("failed to load moemail config")?
        .error_for_status()
        .context("moemail config request failed")?;

    response
        .json::<MoeMailConfigPayload>()
        .await
        .context("failed to decode moemail config response")
}

async fn moemail_list_emails(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
) -> Result<Vec<MoeMailEmailSummary>> {
    let mut items = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut url = config
            .base_url
            .join("/api/emails")
            .context("invalid moemail email list endpoint")?;
        if let Some(current_cursor) = cursor.as_deref() {
            url.query_pairs_mut().append_pair("cursor", current_cursor);
        }
        let response = client
            .get(url)
            .header("X-API-Key", config.api_key.as_str())
            .send()
            .await
            .context("failed to list moemail mailboxes")?
            .error_for_status()
            .context("moemail email list request failed")?;
        let payload = response
            .json::<MoeMailEmailListPayload>()
            .await
            .context("failed to decode moemail email list response")?;
        items.extend(payload.emails);
        match payload.next_cursor {
            Some(next_cursor) if !next_cursor.trim().is_empty() => cursor = Some(next_cursor),
            _ => break,
        }
    }
    Ok(items)
}

async fn moemail_list_messages(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    remote_email_id: &str,
) -> Result<Vec<MoeMailMessageSummary>> {
    let response = client
        .get(
            config
                .base_url
                .join(&format!("/api/emails/{remote_email_id}"))
                .context("invalid moemail email detail endpoint")?,
        )
        .header("X-API-Key", config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to list moemail messages for {remote_email_id}"))?
        .error_for_status()
        .with_context(|| format!("moemail list messages request failed for {remote_email_id}"))?;

    let payload = response
        .json::<MoeMailMessageListPayload>()
        .await
        .context("failed to decode moemail message list response")?;
    Ok(payload.messages)
}

async fn moemail_list_messages_for_attach(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    remote_email_id: &str,
) -> Result<MoeMailAttachReadState<Vec<MoeMailMessageSummary>>> {
    let response = client
        .get(
            config
                .base_url
                .join(&format!("/api/emails/{remote_email_id}"))
                .context("invalid moemail email detail endpoint")?,
        )
        .header("X-API-Key", config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to list moemail messages for {remote_email_id}"))?;
    if moemail_attach_status_is_not_readable(response.status()) {
        return Ok(MoeMailAttachReadState::NotReadable);
    }
    let response = response
        .error_for_status()
        .with_context(|| format!("moemail list messages request failed for {remote_email_id}"))?;

    let payload = response
        .json::<MoeMailMessageListPayload>()
        .await
        .context("failed to decode moemail message list response")?;
    Ok(MoeMailAttachReadState::Readable(payload.messages))
}

async fn moemail_get_message(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    remote_email_id: &str,
    message_id: &str,
) -> Result<MoeMailMessageDetail> {
    let response = client
        .get(
            config
                .base_url
                .join(&format!("/api/emails/{remote_email_id}/{message_id}"))
                .context("invalid moemail message detail endpoint")?,
        )
        .header("X-API-Key", config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to load moemail message {message_id}"))?
        .error_for_status()
        .with_context(|| format!("moemail message request failed for {message_id}"))?;

    let payload = response
        .json::<MoeMailMessageDetailPayload>()
        .await
        .context("failed to decode moemail message detail response")?;
    Ok(payload.message)
}

async fn moemail_get_message_for_attach(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    remote_email_id: &str,
    message_id: &str,
) -> Result<MoeMailAttachReadState<MoeMailMessageDetail>> {
    let response = client
        .get(
            config
                .base_url
                .join(&format!("/api/emails/{remote_email_id}/{message_id}"))
                .context("invalid moemail message detail endpoint")?,
        )
        .header("X-API-Key", config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to load moemail message {message_id}"))?;
    if moemail_attach_status_is_not_readable(response.status()) {
        return Ok(MoeMailAttachReadState::NotReadable);
    }
    let response = response
        .error_for_status()
        .with_context(|| format!("moemail message request failed for {message_id}"))?;

    let payload = response
        .json::<MoeMailMessageDetailPayload>()
        .await
        .context("failed to decode moemail message detail response")?;
    Ok(MoeMailAttachReadState::Readable(payload.message))
}

async fn moemail_delete_email(
    client: &Client,
    config: &UpstreamAccountsMoeMailConfig,
    remote_email_id: &str,
) -> Result<()> {
    client
        .delete(
            config
                .base_url
                .join(&format!("/api/emails/{remote_email_id}"))
                .context("invalid moemail delete endpoint")?,
        )
        .header("X-API-Key", config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to delete moemail mailbox {remote_email_id}"))?
        .error_for_status()
        .with_context(|| format!("moemail delete request failed for {remote_email_id}"))?;
    Ok(())
}

async fn refresh_oauth_mailbox_session_status(
    state: &AppState,
    row: &OauthMailboxSessionRow,
) -> Result<OauthMailboxSessionRow> {
    let config = upstream_mailbox_config(&state.config).map_err(|(_, message)| anyhow!(message))?;
    let mut messages =
        moemail_list_messages(&state.http_clients.shared, config, &row.remote_email_id).await?;
    sort_mailbox_messages_desc(&mut messages);

    let unseen_messages = collect_unseen_mailbox_messages(messages, row.last_message_id.as_deref());
    let (fresh_code, fresh_invite) = resolve_mailbox_message_state(
        &state.http_clients.shared,
        config,
        &row.remote_email_id,
        &unseen_messages,
    )
    .await?;
    let latest_code = merge_mailbox_code(fresh_code, parsed_code_from_mailbox_row(row));
    let latest_invite = merge_mailbox_invite(fresh_invite, parsed_invite_from_mailbox_row(row));
    let next_last_message_id =
        next_mailbox_cursor_after_refresh(row.last_message_id.as_deref(), &unseen_messages);

    let now_iso = format_utc_iso(Utc::now());
    sqlx::query(
        r#"
        UPDATE pool_oauth_mailbox_sessions
        SET latest_code_value = ?2,
            latest_code_source = ?3,
            latest_code_updated_at = ?4,
            invite_subject = ?5,
            invite_copy_value = ?6,
            invite_copy_label = ?7,
            invite_updated_at = ?8,
            invited = ?9,
            last_message_id = ?10,
            updated_at = ?11
        WHERE session_id = ?1
        "#,
    )
    .bind(&row.session_id)
    .bind(latest_code.as_ref().map(|value| value.value.clone()))
    .bind(latest_code.as_ref().map(|value| value.source.clone()))
    .bind(latest_code.as_ref().map(|value| value.updated_at.clone()))
    .bind(latest_invite.as_ref().map(|value| value.subject.clone()))
    .bind(latest_invite.as_ref().map(|value| value.copy_value.clone()))
    .bind(latest_invite.as_ref().map(|value| value.copy_label.clone()))
    .bind(latest_invite.as_ref().map(|value| value.updated_at.clone()))
    .bind(if latest_invite.is_some() { 1 } else { 0 })
    .bind(next_last_message_id)
    .bind(&now_iso)
    .execute(&state.pool)
    .await?;

    load_oauth_mailbox_session(&state.pool, &row.session_id)
        .await?
        .ok_or_else(|| anyhow!("mailbox session disappeared after status refresh"))
}

fn normalize_tag_name(value: &str) -> Result<String, (StatusCode, String)> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "tag name is required".to_string()));
    }
    if trimmed.chars().count() > 48 {
        return Err((
            StatusCode::BAD_REQUEST,
            "tag name must be 48 characters or fewer".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

fn normalize_positive_i64(
    value: Option<i64>,
    field_name: &str,
) -> Result<Option<i64>, (StatusCode, String)> {
    match value {
        Some(number) if number <= 0 => Err((
            StatusCode::BAD_REQUEST,
            format!("{field_name} must be a positive integer"),
        )),
        other => Ok(other),
    }
}

fn normalize_bulk_upstream_account_ids(
    account_ids: &[i64],
) -> Result<Vec<i64>, (StatusCode, String)> {
    let mut normalized = account_ids
        .iter()
        .copied()
        .filter(|value| *value > 0)
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    if normalized.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "accountIds must contain at least one positive integer".to_string(),
        ));
    }
    Ok(normalized)
}

fn normalize_upstream_account_list_page(value: Option<usize>) -> usize {
    value.filter(|page| *page > 0).unwrap_or(1)
}

fn normalize_upstream_account_list_page_size(value: Option<usize>) -> usize {
    value
        .filter(|page_size| UPSTREAM_ACCOUNT_LIST_PAGE_SIZE_OPTIONS.contains(page_size))
        .unwrap_or(DEFAULT_UPSTREAM_ACCOUNT_LIST_PAGE_SIZE)
}

#[derive(Debug, Default, Clone, Copy)]
struct LegacyUpstreamAccountStatusFilter {
    work_status: Option<&'static str>,
    enable_status: Option<&'static str>,
    health_status: Option<&'static str>,
    sync_state: Option<&'static str>,
}

fn normalize_upstream_account_work_status_filter(value: Option<&str>) -> Option<&'static str> {
    let normalized = value?.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    match normalized.as_str() {
        UPSTREAM_ACCOUNT_WORK_STATUS_WORKING => Some(UPSTREAM_ACCOUNT_WORK_STATUS_WORKING),
        UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED => Some(UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED),
        UPSTREAM_ACCOUNT_WORK_STATUS_IDLE => Some(UPSTREAM_ACCOUNT_WORK_STATUS_IDLE),
        UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED => {
            Some(UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED)
        }
        UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE => Some(UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE),
        _ => None,
    }
}

fn normalize_upstream_account_enable_status_filter(value: Option<&str>) -> Option<&'static str> {
    let normalized = value?.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    match normalized.as_str() {
        UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED => Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
        UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED => Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED),
        _ => None,
    }
}

fn normalize_upstream_account_health_status_filter(value: Option<&str>) -> Option<&'static str> {
    let normalized = value?.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    match normalized.as_str() {
        UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL => Some(UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL),
        UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH => Some(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH),
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE => {
            Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE)
        }
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED => {
            Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED)
        }
        UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER | UPSTREAM_ACCOUNT_STATUS_ERROR => {
            Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER)
        }
        _ => None,
    }
}

fn collect_normalized_upstream_account_filters(
    values: &[String],
    legacy_value: Option<&'static str>,
    normalize: fn(Option<&str>) -> Option<&'static str>,
) -> Vec<&'static str> {
    let mut normalized = Vec::new();

    for value in values {
        let Some(next_value) = normalize(Some(value.as_str())) else {
            continue;
        };
        if !normalized.contains(&next_value) {
            normalized.push(next_value);
        }
    }

    if normalized.is_empty() {
        if let Some(legacy_value) = legacy_value {
            normalized.push(legacy_value);
        }
    }

    normalized
}

fn normalize_legacy_upstream_account_status_filter(
    value: Option<&str>,
) -> LegacyUpstreamAccountStatusFilter {
    let normalized = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase());
    match normalized.as_deref() {
        Some(UPSTREAM_ACCOUNT_STATUS_ACTIVE) => LegacyUpstreamAccountStatusFilter {
            enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
            health_status: Some(UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL),
            sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
            ..LegacyUpstreamAccountStatusFilter::default()
        },
        Some(UPSTREAM_ACCOUNT_STATUS_SYNCING) => LegacyUpstreamAccountStatusFilter {
            enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
            sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_SYNCING),
            ..LegacyUpstreamAccountStatusFilter::default()
        },
        Some(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH) => LegacyUpstreamAccountStatusFilter {
            enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
            health_status: Some(UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH),
            sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
            ..LegacyUpstreamAccountStatusFilter::default()
        },
        Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE) => {
            LegacyUpstreamAccountStatusFilter {
                enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
                health_status: Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE),
                sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
                ..LegacyUpstreamAccountStatusFilter::default()
            }
        }
        Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED) => {
            LegacyUpstreamAccountStatusFilter {
                enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
                health_status: Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED),
                sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
                ..LegacyUpstreamAccountStatusFilter::default()
            }
        }
        Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER) | Some(UPSTREAM_ACCOUNT_STATUS_ERROR) => {
            LegacyUpstreamAccountStatusFilter {
                enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED),
                health_status: Some(UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER),
                sync_state: Some(UPSTREAM_ACCOUNT_SYNC_STATE_IDLE),
                ..LegacyUpstreamAccountStatusFilter::default()
            }
        }
        Some(UPSTREAM_ACCOUNT_STATUS_DISABLED) => LegacyUpstreamAccountStatusFilter {
            enable_status: Some(UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED),
            ..LegacyUpstreamAccountStatusFilter::default()
        },
        _ => LegacyUpstreamAccountStatusFilter::default(),
    }
}

fn normalize_bulk_upstream_account_action(value: &str) -> Result<String, (StatusCode, String)> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        BULK_UPSTREAM_ACCOUNT_ACTION_ENABLE
        | BULK_UPSTREAM_ACCOUNT_ACTION_DISABLE
        | BULK_UPSTREAM_ACCOUNT_ACTION_DELETE
        | BULK_UPSTREAM_ACCOUNT_ACTION_SET_GROUP
        | BULK_UPSTREAM_ACCOUNT_ACTION_ADD_TAGS
        | BULK_UPSTREAM_ACCOUNT_ACTION_REMOVE_TAGS => Ok(normalized),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "unsupported bulk action".to_string(),
        )),
    }
}

fn normalize_tag_rule(
    guard_enabled: bool,
    lookback_hours: Option<i64>,
    max_conversations: Option<i64>,
    allow_cut_out: bool,
    allow_cut_in: bool,
    priority_tier: Option<&str>,
    fast_mode_rewrite_mode: Option<&str>,
    concurrency_limit: Option<i64>,
) -> Result<TagRoutingRule, (StatusCode, String)> {
    let lookback_hours = normalize_positive_i64(lookback_hours, "lookbackHours")?;
    let max_conversations = normalize_positive_i64(max_conversations, "maxConversations")?;
    let priority_tier = normalize_tag_priority_tier(priority_tier)?;
    let fast_mode_rewrite_mode = normalize_tag_fast_mode_rewrite_mode(fast_mode_rewrite_mode)?;
    let concurrency_limit = normalize_concurrency_limit(concurrency_limit, "concurrencyLimit")?;
    if guard_enabled && (lookback_hours.is_none() || max_conversations.is_none()) {
        return Err((
            StatusCode::BAD_REQUEST,
            "lookbackHours and maxConversations are required when guardEnabled is true".to_string(),
        ));
    }
    Ok(TagRoutingRule {
        guard_enabled,
        lookback_hours: if guard_enabled { lookback_hours } else { None },
        max_conversations: if guard_enabled {
            max_conversations
        } else {
            None
        },
        allow_cut_out,
        allow_cut_in,
        priority_tier,
        fast_mode_rewrite_mode,
        concurrency_limit,
    })
}

fn normalize_tag_priority_tier(
    value: Option<&str>,
) -> Result<TagPriorityTier, (StatusCode, String)> {
    let normalized = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("normal");
    match normalized {
        "fallback" => Ok(TagPriorityTier::Fallback),
        "normal" => Ok(TagPriorityTier::Normal),
        "primary" => Ok(TagPriorityTier::Primary),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "priorityTier must be one of: primary, normal, fallback".to_string(),
        )),
    }
}

fn decode_tag_priority_tier(value: &str) -> TagPriorityTier {
    match value.trim() {
        "fallback" => TagPriorityTier::Fallback,
        "primary" => TagPriorityTier::Primary,
        _ => TagPriorityTier::Normal,
    }
}

fn normalize_tag_fast_mode_rewrite_mode(
    value: Option<&str>,
) -> Result<TagFastModeRewriteMode, (StatusCode, String)> {
    let normalized = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("keep_original");
    match normalized {
        "force_remove" => Ok(TagFastModeRewriteMode::ForceRemove),
        "keep_original" => Ok(TagFastModeRewriteMode::KeepOriginal),
        "fill_missing" => Ok(TagFastModeRewriteMode::FillMissing),
        "force_add" => Ok(TagFastModeRewriteMode::ForceAdd),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "fastModeRewriteMode must be one of: force_remove, keep_original, fill_missing, force_add"
                .to_string(),
        )),
    }
}

fn decode_tag_fast_mode_rewrite_mode(value: &str) -> TagFastModeRewriteMode {
    match value.trim() {
        "force_remove" => TagFastModeRewriteMode::ForceRemove,
        "fill_missing" => TagFastModeRewriteMode::FillMissing,
        "force_add" => TagFastModeRewriteMode::ForceAdd,
        _ => TagFastModeRewriteMode::KeepOriginal,
    }
}

fn normalize_concurrency_limit(
    value: Option<i64>,
    field_name: &str,
) -> Result<i64, (StatusCode, String)> {
    let value = value.unwrap_or(0);
    if !(0..=30).contains(&value) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("{field_name} must be between 0 and 30"),
        ));
    }
    Ok(value)
}

fn parse_tag_ids_json(raw: Option<&str>) -> Vec<i64> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<i64>>(raw)
        .unwrap_or_default()
        .into_iter()
        .filter(|value| *value > 0)
        .collect()
}

fn encode_tag_ids_json(tag_ids: &[i64]) -> Result<String> {
    serde_json::to_string(tag_ids).context("failed to encode tag ids")
}

fn account_tag_summary_from_row(row: &AccountTagRow) -> AccountTagSummary {
    AccountTagSummary {
        id: row.tag_id,
        name: row.name.clone(),
        routing_rule: TagRoutingRule {
            guard_enabled: row.guard_enabled != 0,
            lookback_hours: row.lookback_hours,
            max_conversations: row.max_conversations,
            allow_cut_out: row.allow_cut_out != 0,
            allow_cut_in: row.allow_cut_in != 0,
            priority_tier: decode_tag_priority_tier(&row.priority_tier),
            fast_mode_rewrite_mode: decode_tag_fast_mode_rewrite_mode(&row.fast_mode_rewrite_mode),
            concurrency_limit: row.concurrency_limit,
        },
    }
}

fn tag_summary_from_row(row: &TagListRow) -> TagSummary {
    TagSummary {
        id: row.id,
        name: row.name.clone(),
        routing_rule: TagRoutingRule {
            guard_enabled: row.guard_enabled != 0,
            lookback_hours: row.lookback_hours,
            max_conversations: row.max_conversations,
            allow_cut_out: row.allow_cut_out != 0,
            allow_cut_in: row.allow_cut_in != 0,
            priority_tier: decode_tag_priority_tier(&row.priority_tier),
            fast_mode_rewrite_mode: decode_tag_fast_mode_rewrite_mode(&row.fast_mode_rewrite_mode),
            concurrency_limit: row.concurrency_limit,
        },
        account_count: row.account_count,
        group_count: row.group_count,
        updated_at: row.updated_at.clone(),
    }
}

