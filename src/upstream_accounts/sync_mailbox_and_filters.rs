use super::*;

#[derive(Debug, Clone)]
pub(crate) struct ParsedMailboxCode {
    pub(crate) value: String,
    pub(crate) source: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ParsedMailboxInvite {
    pub(crate) subject: String,
    pub(crate) copy_value: String,
    pub(crate) copy_label: String,
    pub(crate) updated_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KaisouMailMetaPayload {
    pub(crate) domains: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KaisouMailMailboxPayload {
    pub(crate) id: String,
    pub(crate) address: String,
    pub(crate) expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KaisouMailMailboxListPayload {
    pub(crate) mailboxes: Vec<KaisouMailMailboxSummary>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KaisouMailMailboxSummary {
    pub(crate) id: String,
    pub(crate) address: String,
    pub(crate) expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KaisouMailMessageListPayload {
    messages: Vec<KaisouMailMessageSummary>,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KaisouMailMessageSummary {
    pub(crate) id: String,
    pub(crate) subject: Option<String>,
    pub(crate) received_at: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KaisouMailMessageDetailPayload {
    pub(crate) message: KaisouMailMessageDetail,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct KaisouMailMessageDetail {
    pub(crate) id: String,
    pub(crate) subject: Option<String>,
    pub(crate) content: Option<String>,
    pub(crate) html: Option<String>,
    pub(crate) received_at: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct KaisouMailMessageDetailRaw {
    id: String,
    subject: Option<String>,
    content: Option<String>,
    text: Option<String>,
    preview_text: Option<String>,
    html: Option<String>,
    received_at: Option<String>,
}

impl<'de> Deserialize<'de> for KaisouMailMessageDetail {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = KaisouMailMessageDetailRaw::deserialize(deserializer)?;
        Ok(Self {
            id: raw.id,
            subject: raw.subject,
            content: raw.content.or(raw.text).or(raw.preview_text),
            html: raw.html,
            received_at: raw.received_at,
        })
    }
}

pub(crate) const MAILBOX_CODE_CONTEXT_WINDOW_BYTES: usize = 64;
pub(crate) const OAUTH_BRAND_MARKERS: &[&str] = &["openai", "chatgpt"];
pub(crate) const OAUTH_STRONG_CODE_MARKERS: &[&str] = &[
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
pub(crate) const OAUTH_WEAK_CODE_MARKERS: &[&str] = &[
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
pub(crate) const OAUTH_INVITE_SUBJECT_MARKERS: &[&str] = &[
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
pub(crate) const OAUTH_INVITE_BODY_MARKERS: &[&str] = &[
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
pub(crate) static OAUTH_CODE_CANDIDATE_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?:^|[^0-9])([0-9]{4,8})(?:[^0-9]|$)").expect("valid oauth code candidate regex")
});
pub(crate) static URL_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"https?://[^\s"'<>)]+"#).expect("valid url regex"));
pub(crate) static HTML_TAG_REGEX: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"<[^>]+>").expect("valid html tag regex"));
pub(crate) static BASIC_EMAIL_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^[a-z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?)+$")
        .expect("valid basic email regex")
});

pub(crate) fn oauth_mailbox_status_from_row(row: &OauthMailboxSessionRow) -> OauthMailboxStatus {
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

pub(crate) fn oauth_mailbox_session_supported_response(
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

pub(crate) fn oauth_mailbox_session_unsupported_response(
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

pub(crate) fn normalize_mailbox_address(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

pub(crate) fn normalize_mailbox_domain(value: &str) -> Option<String> {
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

pub(crate) fn kaisoumail_supported_domains(payload: &KaisouMailMetaPayload) -> HashSet<String> {
    payload
        .domains
        .iter()
        .map(String::as_str)
        .filter_map(normalize_mailbox_domain)
        .collect()
}

pub(crate) fn kaisoumail_domain_is_supported(
    email_domain: &str,
    supported_domains: &HashSet<String>,
) -> bool {
    if supported_domains.is_empty() {
        return true;
    }
    let Some(email_domain) = normalize_mailbox_domain(email_domain) else {
        return false;
    };
    supported_domains.iter().any(|supported_domain| {
        email_domain == *supported_domain || email_domain.ends_with(&format!(".{supported_domain}"))
    })
}

pub(crate) fn validate_kaisoumail_mailbox_address_matches_request(
    payload: &KaisouMailMailboxPayload,
    requested_email: &str,
) -> Result<()> {
    let returned_email = normalize_mailbox_address(&payload.address)
        .ok_or_else(|| anyhow!("ensured kaisoumail address must not be blank"))?;
    if returned_email != requested_email {
        bail!(
            "ensured kaisoumail address {} does not match requested {}",
            payload.address,
            requested_email
        );
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RequestedManualMailboxAddress {
    Missing,
    Valid(String),
    Invalid(String),
}

pub(crate) fn requested_manual_mailbox_address(
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

pub(crate) fn mailbox_address_is_valid(value: &str) -> bool {
    BASIC_EMAIL_REGEX.is_match(value.trim())
}

pub(crate) fn upstream_mailbox_config(
    config: &AppConfig,
) -> Result<&UpstreamAccountsKaisouMailConfig, (StatusCode, String)> {
    config.upstream_accounts_kaisoumail.as_ref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!(
                "oauth temp mail requires {} and {}",
                ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_BASE_URL, ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_API_KEY
            ),
        )
    })
}

pub(crate) fn validate_mailbox_binding_fields(
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

pub(crate) fn mailbox_addresses_match(left: Option<&str>, right: Option<&str>) -> bool {
    normalize_mailbox_address(left.unwrap_or_default())
        == normalize_mailbox_address(right.unwrap_or_default())
}

pub(crate) fn expired_mailbox_session_requires_remote_delete(row: &OauthMailboxSessionRow) -> bool {
    row.mailbox_source.as_deref() != Some(OAUTH_MAILBOX_SOURCE_ATTACHED)
}

pub(crate) fn normalize_mailbox_session_expires_at(
    value: Option<&str>,
    fallback: DateTime<Utc>,
) -> String {
    value
        .and_then(|raw| {
            DateTime::parse_from_rfc3339(raw)
                .ok()
                .map(|parsed| format_utc_iso(parsed.with_timezone(&Utc)))
        })
        .unwrap_or_else(|| format_utc_iso(fallback))
}

pub(crate) fn mailbox_expires_at_is_expired(value: Option<&str>, now: DateTime<Utc>) -> bool {
    value
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|expires_at| expires_at.with_timezone(&Utc) <= now)
        .unwrap_or(false)
}

pub(crate) async fn validate_mailbox_binding(
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

pub(crate) fn strip_html_tags(raw: &str) -> String {
    HTML_TAG_REGEX.replace_all(raw, " ").into_owned()
}

pub(crate) fn normalize_mailbox_text(raw: &str) -> String {
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

pub(crate) fn mailbox_text_contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

pub(crate) fn mailbox_text_has_brand(text: &str) -> bool {
    mailbox_text_contains_any(text, OAUTH_BRAND_MARKERS)
}

pub(crate) fn clamp_mailbox_context_start(raw: &str, index: usize) -> usize {
    let mut candidate = index.min(raw.len());
    while candidate > 0 && !raw.is_char_boundary(candidate) {
        candidate -= 1;
    }
    candidate
}

pub(crate) fn clamp_mailbox_context_end(raw: &str, index: usize) -> usize {
    let mut candidate = index.min(raw.len());
    while candidate < raw.len() && !raw.is_char_boundary(candidate) {
        candidate += 1;
    }
    candidate
}

pub(crate) fn mailbox_context_slice(raw: &str, start: usize, end: usize, radius: usize) -> &str {
    let context_start = clamp_mailbox_context_start(raw, start.saturating_sub(radius));
    let context_end = clamp_mailbox_context_end(raw, end.saturating_add(radius));
    &raw[context_start..context_end]
}

pub(crate) fn mailbox_context_before(raw: &str, index: usize, radius: usize) -> &str {
    let context_start = clamp_mailbox_context_start(raw, index.saturating_sub(radius));
    let context_end = clamp_mailbox_context_end(raw, index);
    &raw[context_start..context_end]
}

pub(crate) fn extract_mailbox_code_candidate(
    text: &str,
    message_has_brand: bool,
) -> Option<String> {
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

pub(crate) fn mailbox_url_candidate_urls(url: &str) -> Vec<String> {
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

pub(crate) fn mailbox_url_looks_like_direct_invite(url: &str) -> bool {
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

pub(crate) fn mailbox_url_resolve_invite_target(url: &str) -> Option<String> {
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

pub(crate) fn mailbox_url_has_brand(url: &str) -> bool {
    mailbox_url_candidate_urls(url)
        .into_iter()
        .any(|candidate| {
            let lower = candidate.to_ascii_lowercase();
            lower.contains("openai") || lower.contains("chatgpt")
        })
}

pub(crate) fn parse_mailbox_code(detail: &KaisouMailMessageDetail) -> Option<ParsedMailboxCode> {
    let subject = detail.subject.as_deref().unwrap_or_default();
    let content = detail.content.as_deref().unwrap_or_default();
    let html_text = strip_html_tags(detail.html.as_deref().unwrap_or_default());
    let message_context = normalize_mailbox_text(&format!("{subject}\n{content}\n{html_text}"));
    let message_has_brand = mailbox_text_has_brand(&message_context);

    let subject_text = normalize_mailbox_text(subject);
    let subject_has_brand = mailbox_text_has_brand(&subject_text);
    if subject_has_brand
        && let Some(value) = extract_mailbox_code_candidate(&subject_text, subject_has_brand)
    {
        return Some(ParsedMailboxCode {
            value,
            source: "subject".to_string(),
            updated_at: detail
                .received_at
                .clone()
                .unwrap_or_else(|| format_utc_iso(Utc::now())),
        });
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

pub(crate) fn parse_mailbox_invite(
    detail: &KaisouMailMessageDetail,
) -> Option<ParsedMailboxInvite> {
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

pub(crate) fn parsed_code_from_mailbox_row(
    row: &OauthMailboxSessionRow,
) -> Option<ParsedMailboxCode> {
    Some(ParsedMailboxCode {
        value: row.latest_code_value.clone()?,
        source: row.latest_code_source.clone()?,
        updated_at: row.latest_code_updated_at.clone()?,
    })
}

pub(crate) fn parsed_invite_from_mailbox_row(
    row: &OauthMailboxSessionRow,
) -> Option<ParsedMailboxInvite> {
    Some(ParsedMailboxInvite {
        subject: row.invite_subject.clone()?,
        copy_value: row.invite_copy_value.clone()?,
        copy_label: row.invite_copy_label.clone()?,
        updated_at: row.invite_updated_at.clone()?,
    })
}

pub(crate) fn mailbox_updated_at_is_newer_or_equal(candidate: &str, baseline: &str) -> bool {
    match (parse_rfc3339_utc(candidate), parse_rfc3339_utc(baseline)) {
        (Some(candidate), Some(baseline)) => candidate >= baseline,
        _ => candidate >= baseline,
    }
}

pub(crate) fn merge_mailbox_code(
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

pub(crate) fn merge_mailbox_invite(
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

pub(crate) fn sort_mailbox_messages_desc(messages: &mut [KaisouMailMessageSummary]) {
    messages.sort_by(|left, right| right.received_at.cmp(&left.received_at));
}

pub(crate) fn latest_mailbox_message_id(messages: &[KaisouMailMessageSummary]) -> Option<String> {
    messages.first().map(|message| message.id.clone())
}

pub(crate) fn collect_unseen_mailbox_messages(
    messages: Vec<KaisouMailMessageSummary>,
    last_message_id: Option<&str>,
) -> Vec<KaisouMailMessageSummary> {
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

pub(crate) fn next_mailbox_cursor_after_refresh(
    previous_last_message_id: Option<&str>,
    processed_messages: &[KaisouMailMessageSummary],
) -> Option<String> {
    processed_messages
        .first()
        .map(|message| message.id.clone())
        .or_else(|| previous_last_message_id.map(ToOwned::to_owned))
}

pub(crate) async fn resolve_mailbox_message_state(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
    messages: &[KaisouMailMessageSummary],
) -> Result<(Option<ParsedMailboxCode>, Option<ParsedMailboxInvite>)> {
    let mut latest_code = None;
    let mut latest_invite = None;
    for summary in messages.iter() {
        if latest_code.is_some() && latest_invite.is_some() {
            break;
        }
        let detail = kaisoumail_get_message(client, config, &summary.id).await?;
        if latest_code.is_none() {
            latest_code = parse_mailbox_code(&detail);
        }
        if latest_invite.is_none() {
            latest_invite = parse_mailbox_invite(&detail);
        }
    }

    Ok((latest_code, latest_invite))
}

pub(crate) enum KaisouMailAttachReadState<T> {
    Readable(T),
    NotReadable,
}

pub(crate) fn kaisoumail_attach_status_is_not_readable(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::NOT_FOUND
    )
}

pub(crate) async fn resolve_mailbox_message_state_for_attach(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
    messages: &[KaisouMailMessageSummary],
) -> Result<KaisouMailAttachReadState<(Option<ParsedMailboxCode>, Option<ParsedMailboxInvite>)>> {
    let mut latest_code = None;
    let mut latest_invite = None;
    for summary in messages.iter() {
        if latest_code.is_some() && latest_invite.is_some() {
            break;
        }
        let detail = match kaisoumail_get_message_for_attach(client, config, &summary.id).await? {
            KaisouMailAttachReadState::Readable(detail) => detail,
            KaisouMailAttachReadState::NotReadable => {
                return Ok(KaisouMailAttachReadState::NotReadable);
            }
        };
        if latest_code.is_none() {
            latest_code = parse_mailbox_code(&detail);
        }
        if latest_invite.is_none() {
            latest_invite = parse_mailbox_invite(&detail);
        }
    }

    Ok(KaisouMailAttachReadState::Readable((
        latest_code,
        latest_invite,
    )))
}

pub(crate) async fn kaisoumail_create_mailbox(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
) -> Result<KaisouMailMailboxPayload> {
    let response = client
        .post(
            config
                .base_url
                .join("/api/mailboxes")
                .context("invalid kaisoumail mailbox create endpoint")?,
        )
        .bearer_auth(config.api_key.as_str())
        .json(&json!({
            "expiresInMinutes": DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS / 60,
        }))
        .send()
        .await
        .context("failed to create kaisoumail mailbox")?
        .error_for_status()
        .context("kaisoumail mailbox creation request failed")?;

    response
        .json::<KaisouMailMailboxPayload>()
        .await
        .context("failed to decode kaisoumail create mailbox response")
}

pub(crate) async fn kaisoumail_ensure_mailbox_for_address(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
    email_address: &str,
) -> Result<KaisouMailMailboxPayload> {
    let requested_email = normalize_mailbox_address(email_address)
        .ok_or_else(|| anyhow!("manual kaisoumail address must not be blank"))?;
    let response = client
        .post(
            config
                .base_url
                .join("/api/mailboxes/ensure")
                .context("invalid kaisoumail mailbox ensure endpoint")?,
        )
        .bearer_auth(config.api_key.as_str())
        .json(&json!({
            "address": requested_email,
            "expiresInMinutes": DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS / 60,
        }))
        .send()
        .await
        .context("failed to ensure kaisoumail mailbox")?
        .error_for_status()
        .context("kaisoumail mailbox ensure request failed")?;

    let payload = response
        .json::<KaisouMailMailboxPayload>()
        .await
        .context("failed to decode kaisoumail ensure mailbox response")?;
    validate_kaisoumail_mailbox_address_matches_request(&payload, &requested_email)?;
    Ok(payload)
}

pub(crate) async fn kaisoumail_get_meta(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
) -> Result<KaisouMailMetaPayload> {
    let response = client
        .get(
            config
                .base_url
                .join("/api/meta")
                .context("invalid kaisoumail meta endpoint")?,
        )
        .bearer_auth(config.api_key.as_str())
        .send()
        .await
        .context("failed to load kaisoumail config")?
        .error_for_status()
        .context("kaisoumail config request failed")?;

    response
        .json::<KaisouMailMetaPayload>()
        .await
        .context("failed to decode kaisoumail meta response")
}

pub(crate) async fn kaisoumail_list_mailboxes(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
) -> Result<Vec<KaisouMailMailboxSummary>> {
    let response = client
        .get(
            config
                .base_url
                .join("/api/mailboxes")
                .context("invalid kaisoumail mailbox list endpoint")?,
        )
        .bearer_auth(config.api_key.as_str())
        .send()
        .await
        .context("failed to list kaisoumail mailboxes")?
        .error_for_status()
        .context("kaisoumail mailbox list request failed")?;
    let payload = response
        .json::<KaisouMailMailboxListPayload>()
        .await
        .context("failed to decode kaisoumail mailbox list response")?;
    Ok(payload.mailboxes)
}

pub(crate) async fn kaisoumail_list_messages(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
    mailbox_address: &str,
) -> Result<Vec<KaisouMailMessageSummary>> {
    let mut url = config
        .base_url
        .join("/api/messages")
        .context("invalid kaisoumail message list endpoint")?;
    url.query_pairs_mut()
        .append_pair("mailbox", mailbox_address);
    let response = client
        .get(url)
        .bearer_auth(config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to list kaisoumail messages for {mailbox_address}"))?
        .error_for_status()
        .with_context(|| {
            format!("kaisoumail list messages request failed for {mailbox_address}")
        })?;

    let payload = response
        .json::<KaisouMailMessageListPayload>()
        .await
        .context("failed to decode kaisoumail message list response")?;
    Ok(payload.messages)
}

pub(crate) async fn kaisoumail_list_messages_for_attach(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
    mailbox_address: &str,
) -> Result<KaisouMailAttachReadState<Vec<KaisouMailMessageSummary>>> {
    let mut url = config
        .base_url
        .join("/api/messages")
        .context("invalid kaisoumail message list endpoint")?;
    url.query_pairs_mut()
        .append_pair("mailbox", mailbox_address);
    let response = client
        .get(url)
        .bearer_auth(config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to list kaisoumail messages for {mailbox_address}"))?;
    if kaisoumail_attach_status_is_not_readable(response.status()) {
        return Ok(KaisouMailAttachReadState::NotReadable);
    }
    let response = response.error_for_status().with_context(|| {
        format!("kaisoumail list messages request failed for {mailbox_address}")
    })?;

    let payload = response
        .json::<KaisouMailMessageListPayload>()
        .await
        .context("failed to decode kaisoumail message list response")?;
    Ok(KaisouMailAttachReadState::Readable(payload.messages))
}

pub(crate) async fn kaisoumail_get_message(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
    message_id: &str,
) -> Result<KaisouMailMessageDetail> {
    let response = client
        .get(
            config
                .base_url
                .join(&format!("/api/messages/{message_id}"))
                .context("invalid kaisoumail message detail endpoint")?,
        )
        .bearer_auth(config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to load kaisoumail message {message_id}"))?
        .error_for_status()
        .with_context(|| format!("kaisoumail message request failed for {message_id}"))?;

    let payload = response
        .json::<KaisouMailMessageDetailPayload>()
        .await
        .context("failed to decode kaisoumail message detail response")?;
    Ok(payload.message)
}

pub(crate) async fn kaisoumail_get_message_for_attach(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
    message_id: &str,
) -> Result<KaisouMailAttachReadState<KaisouMailMessageDetail>> {
    let response = client
        .get(
            config
                .base_url
                .join(&format!("/api/messages/{message_id}"))
                .context("invalid kaisoumail message detail endpoint")?,
        )
        .bearer_auth(config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to load kaisoumail message {message_id}"))?;
    if kaisoumail_attach_status_is_not_readable(response.status()) {
        return Ok(KaisouMailAttachReadState::NotReadable);
    }
    let response = response
        .error_for_status()
        .with_context(|| format!("kaisoumail message request failed for {message_id}"))?;

    let payload = response
        .json::<KaisouMailMessageDetailPayload>()
        .await
        .context("failed to decode kaisoumail message detail response")?;
    Ok(KaisouMailAttachReadState::Readable(payload.message))
}

pub(crate) async fn kaisoumail_delete_mailbox(
    client: &Client,
    config: &UpstreamAccountsKaisouMailConfig,
    remote_email_id: &str,
) -> Result<()> {
    client
        .delete(
            config
                .base_url
                .join(&format!("/api/mailboxes/{remote_email_id}"))
                .context("invalid kaisoumail delete endpoint")?,
        )
        .bearer_auth(config.api_key.as_str())
        .send()
        .await
        .with_context(|| format!("failed to delete kaisoumail mailbox {remote_email_id}"))?
        .error_for_status()
        .with_context(|| format!("kaisoumail delete request failed for {remote_email_id}"))?;
    Ok(())
}

pub(crate) async fn refresh_oauth_mailbox_session_status(
    state: &AppState,
    row: &OauthMailboxSessionRow,
) -> Result<OauthMailboxSessionRow> {
    let config = upstream_mailbox_config(&state.config).map_err(|(_, message)| anyhow!(message))?;
    let mut messages =
        kaisoumail_list_messages(&state.http_clients.shared, config, &row.email_address).await?;
    sort_mailbox_messages_desc(&mut messages);

    let unseen_messages = collect_unseen_mailbox_messages(messages, row.last_message_id.as_deref());
    let (fresh_code, fresh_invite) =
        resolve_mailbox_message_state(&state.http_clients.shared, config, &unseen_messages).await?;
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

pub(crate) fn normalize_tag_name(value: &str) -> Result<String, (StatusCode, String)> {
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

pub(crate) fn normalize_bulk_upstream_account_ids(
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

pub(crate) fn normalize_upstream_account_list_page(value: Option<usize>) -> usize {
    value.filter(|page| *page > 0).unwrap_or(1)
}

pub(crate) fn normalize_upstream_account_list_page_size(value: Option<usize>) -> usize {
    value
        .filter(|page_size| UPSTREAM_ACCOUNT_LIST_PAGE_SIZE_OPTIONS.contains(page_size))
        .unwrap_or(DEFAULT_UPSTREAM_ACCOUNT_LIST_PAGE_SIZE)
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct LegacyUpstreamAccountStatusFilter {
    pub(crate) work_status: Option<&'static str>,
    pub(crate) enable_status: Option<&'static str>,
    pub(crate) health_status: Option<&'static str>,
    pub(crate) sync_state: Option<&'static str>,
}

pub(crate) fn normalize_upstream_account_work_status_filter(
    value: Option<&str>,
) -> Option<&'static str> {
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

pub(crate) fn normalize_upstream_account_enable_status_filter(
    value: Option<&str>,
) -> Option<&'static str> {
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

pub(crate) fn normalize_upstream_account_health_status_filter(
    value: Option<&str>,
) -> Option<&'static str> {
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

pub(crate) fn collect_normalized_upstream_account_filters(
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

    if normalized.is_empty()
        && let Some(legacy_value) = legacy_value
    {
        normalized.push(legacy_value);
    }

    normalized
}

pub(crate) fn normalize_legacy_upstream_account_status_filter(
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

pub(crate) fn normalize_bulk_upstream_account_action(
    value: &str,
) -> Result<String, (StatusCode, String)> {
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        BULK_UPSTREAM_ACCOUNT_ACTION_ENABLE
        | BULK_UPSTREAM_ACCOUNT_ACTION_DISABLE
        | BULK_UPSTREAM_ACCOUNT_ACTION_DELETE
        | BULK_UPSTREAM_ACCOUNT_ACTION_SET_GROUP => Ok(normalized),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "unsupported bulk action".to_string(),
        )),
    }
}

pub(crate) fn normalize_tag_rule(
    allow_cut_out: bool,
    allow_cut_in: bool,
    priority_tier: Option<&str>,
    fast_mode_rewrite_mode: Option<&str>,
    concurrency_limit: Option<i64>,
    upstream_429_retry_enabled: Option<bool>,
    upstream_429_max_retries: Option<u8>,
    available_models: Option<Vec<String>>,
) -> Result<TagRoutingRule, (StatusCode, String)> {
    let priority_tier = normalize_tag_priority_tier(priority_tier)?;
    let fast_mode_rewrite_mode = normalize_tag_fast_mode_rewrite_mode(fast_mode_rewrite_mode)?;
    let concurrency_limit = normalize_concurrency_limit(concurrency_limit, "concurrencyLimit")?;
    let upstream_429_retry_enabled = upstream_429_retry_enabled.unwrap_or(false);
    let upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
        upstream_429_retry_enabled,
        upstream_429_max_retries
            .map(normalize_group_upstream_429_max_retries)
            .unwrap_or_default(),
    );
    Ok(TagRoutingRule {
        allow_cut_out,
        allow_cut_in,
        priority_tier,
        fast_mode_rewrite_mode,
        concurrency_limit,
        upstream_429_retry_enabled,
        upstream_429_max_retries,
        available_models: normalize_available_models(available_models, "availableModels")?,
    })
}

pub(crate) fn normalize_group_account_routing_rule(
    allow_cut_out: bool,
    allow_cut_in: bool,
    priority_tier: Option<&str>,
    fast_mode_rewrite_mode: Option<&str>,
    image_tool_rewrite_mode: Option<&str>,
    concurrency_limit: Option<i64>,
    upstream_429_retry_enabled: Option<bool>,
    upstream_429_max_retries: Option<u8>,
    available_models: Option<Vec<String>>,
) -> Result<GroupAccountRoutingRule, (StatusCode, String)> {
    let available_models_defined = available_models.is_some();
    let priority_tier = normalize_tag_priority_tier(priority_tier)?;
    let fast_mode_rewrite_mode = normalize_tag_fast_mode_rewrite_mode(fast_mode_rewrite_mode)?;
    let image_tool_rewrite_mode = normalize_image_tool_rewrite_mode(image_tool_rewrite_mode)?;
    let concurrency_limit = normalize_concurrency_limit(concurrency_limit, "concurrencyLimit")?;
    let upstream_429_retry_enabled = upstream_429_retry_enabled.unwrap_or(false);
    let upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
        upstream_429_retry_enabled,
        upstream_429_max_retries
            .map(normalize_group_upstream_429_max_retries)
            .unwrap_or_default(),
    );
    Ok(GroupAccountRoutingRule {
        allow_cut_out,
        allow_cut_in,
        priority_tier,
        fast_mode_rewrite_mode,
        image_tool_rewrite_mode,
        request_compression_algorithm: None,
        concurrency_limit,
        upstream_429_retry_enabled,
        upstream_429_max_retries,
        available_models: normalize_available_models(available_models, "availableModels")?,
        available_models_defined,
        status_change_reasons: default_status_change_reasons(),
        timeouts: None,
    })
}

pub(crate) fn normalize_available_models(
    value: Option<Vec<String>>,
    field_name: &str,
) -> Result<Vec<String>, (StatusCode, String)> {
    let mut normalized = Vec::new();
    let mut seen = HashSet::new();
    for model in value.unwrap_or_default() {
        let model = model.trim();
        if model.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("{field_name} entries must be non-empty"),
            ));
        }
        if seen.insert(model.to_string()) {
            normalized.push(model.to_string());
        }
    }
    Ok(normalized)
}

pub(crate) fn normalize_tag_priority_tier(
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
        "no_new" => Ok(TagPriorityTier::NoNew),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "priorityTier must be one of: primary, normal, fallback, no_new".to_string(),
        )),
    }
}

pub(crate) fn decode_tag_priority_tier(value: &str) -> TagPriorityTier {
    match value.trim() {
        "no_new" => TagPriorityTier::NoNew,
        "fallback" => TagPriorityTier::Fallback,
        "primary" => TagPriorityTier::Primary,
        _ => TagPriorityTier::Normal,
    }
}

pub(crate) fn normalize_tag_fast_mode_rewrite_mode(
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

pub(crate) fn decode_tag_fast_mode_rewrite_mode(value: &str) -> TagFastModeRewriteMode {
    match value.trim() {
        "force_remove" => TagFastModeRewriteMode::ForceRemove,
        "fill_missing" => TagFastModeRewriteMode::FillMissing,
        "force_add" => TagFastModeRewriteMode::ForceAdd,
        _ => TagFastModeRewriteMode::KeepOriginal,
    }
}

pub(crate) fn normalize_image_tool_rewrite_mode(
    value: Option<&str>,
) -> Result<ImageToolRewriteMode, (StatusCode, String)> {
    let normalized = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("keep_original");
    match normalized {
        "force_remove" => Ok(ImageToolRewriteMode::ForceRemove),
        "keep_original" => Ok(ImageToolRewriteMode::KeepOriginal),
        "fill_missing" => Ok(ImageToolRewriteMode::FillMissing),
        "force_add" => Ok(ImageToolRewriteMode::ForceAdd),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "imageToolRewriteMode must be one of: force_remove, keep_original, fill_missing, force_add".to_string(),
        )),
    }
}

pub(crate) fn decode_image_tool_rewrite_mode(value: &str) -> ImageToolRewriteMode {
    ImageToolRewriteMode::from_str(value)
}

pub(crate) fn normalize_request_compression_algorithm(
    value: Option<&str>,
) -> Result<RequestCompressionAlgorithm, (StatusCode, String)> {
    let normalized = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("identity");
    match normalized {
        "follow" => Ok(RequestCompressionAlgorithm::Follow),
        "identity" => Ok(RequestCompressionAlgorithm::Identity),
        "gzip" => Ok(RequestCompressionAlgorithm::Gzip),
        "deflate" => Ok(RequestCompressionAlgorithm::Deflate),
        "zstd" => Ok(RequestCompressionAlgorithm::Zstd),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "requestCompressionAlgorithm must be one of: follow, identity, gzip, deflate, zstd"
                .to_string(),
        )),
    }
}

pub(crate) fn decode_request_compression_algorithm(value: &str) -> RequestCompressionAlgorithm {
    RequestCompressionAlgorithm::from_str(value)
}

pub(crate) fn normalize_request_compression_level_preset(
    value: Option<&str>,
) -> Result<RequestCompressionLevelPreset, (StatusCode, String)> {
    let normalized = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("balanced");
    match normalized {
        "fast" => Ok(RequestCompressionLevelPreset::Fast),
        "balanced" => Ok(RequestCompressionLevelPreset::Balanced),
        "best" => Ok(RequestCompressionLevelPreset::Best),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "requestCompressionLevelPreset must be one of: fast, balanced, best".to_string(),
        )),
    }
}

pub(crate) fn decode_request_compression_level_preset(
    value: Option<&str>,
) -> RequestCompressionLevelPreset {
    value
        .map(RequestCompressionLevelPreset::from_str)
        .unwrap_or_default()
}

pub(crate) fn decode_capability_support(value: Option<&str>) -> CapabilitySupport {
    value
        .map(CapabilitySupport::from_str)
        .unwrap_or(CapabilitySupport::Unknown)
}

pub(crate) fn decode_capability_override(value: Option<&str>) -> Option<CapabilitySupport> {
    let capability = decode_capability_support(value);
    (!matches!(capability, CapabilitySupport::Unknown)).then_some(capability)
}

pub(crate) fn effective_capability_support(
    observed: CapabilitySupport,
    override_value: Option<CapabilitySupport>,
) -> CapabilitySupport {
    override_value.unwrap_or(observed)
}

pub(crate) fn normalize_concurrency_limit(
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

pub(crate) fn parse_tag_ids_json(raw: Option<&str>) -> Vec<i64> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<i64>>(raw)
        .unwrap_or_default()
        .into_iter()
        .filter(|value| *value > 0)
        .collect()
}

pub(crate) fn encode_tag_ids_json(tag_ids: &[i64]) -> Result<String> {
    serde_json::to_string(tag_ids).context("failed to encode tag ids")
}

pub(crate) fn parse_string_array_json(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<String>>(raw)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| normalize_optional_text(Some(value)))
        .collect()
}

pub(crate) fn encode_string_array_json(values: &[String]) -> Result<String> {
    serde_json::to_string(values).context("failed to encode string array")
}

pub(crate) fn account_tag_summary_from_row(row: &AccountTagRow) -> AccountTagSummary {
    AccountTagSummary {
        id: row.tag_id,
        name: row.name.clone(),
        routing_rule: TagRoutingRule {
            allow_cut_out: row.allow_cut_out != 0,
            allow_cut_in: row.allow_cut_in != 0,
            priority_tier: decode_tag_priority_tier(&row.priority_tier),
            fast_mode_rewrite_mode: decode_tag_fast_mode_rewrite_mode(&row.fast_mode_rewrite_mode),
            concurrency_limit: row.concurrency_limit,
            upstream_429_retry_enabled: decode_group_upstream_429_retry_enabled(
                row.upstream_429_retry_enabled,
            ),
            upstream_429_max_retries: normalize_group_upstream_429_retry_metadata(
                decode_group_upstream_429_retry_enabled(row.upstream_429_retry_enabled),
                decode_group_upstream_429_max_retries(row.upstream_429_max_retries),
            ),
            available_models: parse_string_array_json(row.available_models_json.as_deref()),
        },
        system_key: row.system_key.clone(),
        protected: row.protected != 0,
    }
}

pub(crate) fn tag_summary_from_row(row: &TagListRow) -> TagSummary {
    TagSummary {
        id: row.id,
        name: row.name.clone(),
        routing_rule: TagRoutingRule {
            allow_cut_out: row.allow_cut_out != 0,
            allow_cut_in: row.allow_cut_in != 0,
            priority_tier: decode_tag_priority_tier(&row.priority_tier),
            fast_mode_rewrite_mode: decode_tag_fast_mode_rewrite_mode(&row.fast_mode_rewrite_mode),
            concurrency_limit: row.concurrency_limit,
            upstream_429_retry_enabled: decode_group_upstream_429_retry_enabled(
                row.upstream_429_retry_enabled,
            ),
            upstream_429_max_retries: normalize_group_upstream_429_retry_metadata(
                decode_group_upstream_429_retry_enabled(row.upstream_429_retry_enabled),
                decode_group_upstream_429_max_retries(row.upstream_429_max_retries),
            ),
            available_models: parse_string_array_json(row.available_models_json.as_deref()),
        },
        account_count: row.account_count,
        group_count: row.group_count,
        updated_at: row.updated_at.clone(),
        system_key: row.system_key.clone(),
        protected: row.protected != 0,
    }
}

pub(crate) fn status_change_reasons_from_columns(
    policy_status_change_upstream_http_401: Option<i64>,
    policy_status_change_upstream_http_402: Option<i64>,
    policy_status_change_upstream_http_403: Option<i64>,
    policy_status_change_reauth_required: Option<i64>,
    policy_status_change_upstream_http_429_rate_limit: Option<i64>,
    policy_status_change_upstream_http_429_quota_exhausted: Option<i64>,
    policy_status_change_usage_snapshot_exhausted: Option<i64>,
    policy_status_change_quota_still_exhausted: Option<i64>,
    policy_status_change_transport_failure: Option<i64>,
    policy_status_change_upstream_server_overloaded: Option<i64>,
    policy_status_change_upstream_http_5xx: Option<i64>,
) -> StatusChangeReasonSettings {
    let mut reasons = default_status_change_reasons();
    for (reason_code, value) in [
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_401,
            policy_status_change_upstream_http_401,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402,
            policy_status_change_upstream_http_402,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_403,
            policy_status_change_upstream_http_403,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED,
            policy_status_change_reauth_required,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT,
            policy_status_change_upstream_http_429_rate_limit,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
            policy_status_change_upstream_http_429_quota_exhausted,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED,
            policy_status_change_usage_snapshot_exhausted,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED,
            policy_status_change_quota_still_exhausted,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE,
            policy_status_change_transport_failure,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED,
            policy_status_change_upstream_server_overloaded,
        ),
        (
            UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX,
            policy_status_change_upstream_http_5xx,
        ),
    ] {
        if let Some(value) = value {
            reasons.insert(reason_code.to_string(), value != 0);
        }
    }
    reasons
}

pub(crate) fn group_routing_rule_from_columns(
    legacy_concurrency_limit: i64,
    legacy_upstream_429_retry_enabled: bool,
    legacy_upstream_429_max_retries: u8,
    policy_allow_cut_out: Option<i64>,
    policy_allow_cut_in: Option<i64>,
    policy_priority_tier: Option<&str>,
    policy_fast_mode_rewrite_mode: Option<&str>,
    policy_image_tool_rewrite_mode: Option<&str>,
    policy_request_compression_algorithm: Option<&str>,
    policy_concurrency_limit: Option<i64>,
    policy_upstream_429_retry_enabled: Option<i64>,
    policy_upstream_429_max_retries: Option<i64>,
    policy_available_models_json: Option<&str>,
    policy_status_change_upstream_http_401: Option<i64>,
    policy_status_change_upstream_http_402: Option<i64>,
    policy_status_change_upstream_http_403: Option<i64>,
    policy_status_change_reauth_required: Option<i64>,
    policy_status_change_upstream_http_429_rate_limit: Option<i64>,
    policy_status_change_upstream_http_429_quota_exhausted: Option<i64>,
    policy_status_change_usage_snapshot_exhausted: Option<i64>,
    policy_status_change_quota_still_exhausted: Option<i64>,
    policy_status_change_transport_failure: Option<i64>,
    policy_status_change_upstream_server_overloaded: Option<i64>,
    policy_status_change_upstream_http_5xx: Option<i64>,
    policy_responses_first_byte_timeout_secs: Option<i64>,
    policy_compact_first_byte_timeout_secs: Option<i64>,
    policy_image_first_byte_timeout_secs: Option<i64>,
    policy_responses_stream_timeout_secs: Option<i64>,
    policy_compact_stream_timeout_secs: Option<i64>,
) -> GroupAccountRoutingRule {
    let upstream_429_retry_enabled = policy_upstream_429_retry_enabled
        .map(|value| value != 0)
        .unwrap_or(legacy_upstream_429_retry_enabled);
    GroupAccountRoutingRule {
        allow_cut_out: policy_allow_cut_out.map(|value| value != 0).unwrap_or(true),
        allow_cut_in: policy_allow_cut_in.map(|value| value != 0).unwrap_or(true),
        priority_tier: decode_tag_priority_tier(policy_priority_tier.unwrap_or("normal")),
        fast_mode_rewrite_mode: decode_tag_fast_mode_rewrite_mode(
            policy_fast_mode_rewrite_mode.unwrap_or("keep_original"),
        ),
        image_tool_rewrite_mode: decode_image_tool_rewrite_mode(
            policy_image_tool_rewrite_mode.unwrap_or("keep_original"),
        ),
        request_compression_algorithm: policy_request_compression_algorithm
            .map(decode_request_compression_algorithm),
        concurrency_limit: policy_concurrency_limit.unwrap_or(legacy_concurrency_limit),
        upstream_429_retry_enabled,
        upstream_429_max_retries: normalize_group_upstream_429_retry_metadata(
            upstream_429_retry_enabled,
            policy_upstream_429_max_retries
                .map(decode_group_upstream_429_max_retries)
                .unwrap_or(legacy_upstream_429_max_retries),
        ),
        available_models: parse_string_array_json(policy_available_models_json),
        available_models_defined: policy_available_models_json.is_some(),
        status_change_reasons: status_change_reasons_from_columns(
            policy_status_change_upstream_http_401,
            policy_status_change_upstream_http_402,
            policy_status_change_upstream_http_403,
            policy_status_change_reauth_required,
            policy_status_change_upstream_http_429_rate_limit,
            policy_status_change_upstream_http_429_quota_exhausted,
            policy_status_change_usage_snapshot_exhausted,
            policy_status_change_quota_still_exhausted,
            policy_status_change_transport_failure,
            policy_status_change_upstream_server_overloaded,
            policy_status_change_upstream_http_5xx,
        ),
        timeouts: routing_timeout_settings_from_columns(
            policy_responses_first_byte_timeout_secs,
            policy_compact_first_byte_timeout_secs,
            policy_image_first_byte_timeout_secs,
            policy_responses_stream_timeout_secs,
            policy_compact_stream_timeout_secs,
        ),
    }
}

pub(crate) async fn load_group_routing_rule(
    pool: &Pool<Sqlite>,
    group_name: &str,
) -> Result<GroupAccountRoutingRule> {
    #[derive(Debug, FromRow)]
    struct GroupRoutingRuleRow {
        concurrency_limit: Option<i64>,
        upstream_429_retry_enabled: Option<i64>,
        upstream_429_max_retries: Option<i64>,
        policy_allow_cut_out: Option<i64>,
        policy_allow_cut_in: Option<i64>,
        policy_priority_tier: Option<String>,
        policy_fast_mode_rewrite_mode: Option<String>,
        policy_image_tool_rewrite_mode: Option<String>,
        policy_request_compression_algorithm: Option<String>,
        policy_concurrency_limit: Option<i64>,
        policy_upstream_429_retry_enabled: Option<i64>,
        policy_upstream_429_max_retries: Option<i64>,
        policy_available_models_json: Option<String>,
        policy_status_change_upstream_http_401: Option<i64>,
        policy_status_change_upstream_http_402: Option<i64>,
        policy_status_change_upstream_http_403: Option<i64>,
        policy_status_change_reauth_required: Option<i64>,
        policy_status_change_upstream_http_429_rate_limit: Option<i64>,
        policy_status_change_upstream_http_429_quota_exhausted: Option<i64>,
        policy_status_change_usage_snapshot_exhausted: Option<i64>,
        policy_status_change_quota_still_exhausted: Option<i64>,
        policy_status_change_transport_failure: Option<i64>,
        policy_status_change_upstream_server_overloaded: Option<i64>,
        policy_status_change_upstream_http_5xx: Option<i64>,
        policy_responses_first_byte_timeout_secs: Option<i64>,
        policy_compact_first_byte_timeout_secs: Option<i64>,
        policy_image_first_byte_timeout_secs: Option<i64>,
        policy_responses_stream_timeout_secs: Option<i64>,
        policy_compact_stream_timeout_secs: Option<i64>,
    }
    let row = sqlx::query_as::<_, GroupRoutingRuleRow>(
        r#"
        SELECT
            concurrency_limit,
            upstream_429_retry_enabled,
            upstream_429_max_retries,
            policy_allow_cut_out,
            policy_allow_cut_in,
            policy_priority_tier,
            policy_fast_mode_rewrite_mode,
            policy_image_tool_rewrite_mode,
            policy_request_compression_algorithm,
            policy_concurrency_limit,
            policy_upstream_429_retry_enabled,
            policy_upstream_429_max_retries,
            policy_available_models_json,
            policy_status_change_upstream_http_401,
            policy_status_change_upstream_http_402,
            policy_status_change_upstream_http_403,
            policy_status_change_reauth_required,
            policy_status_change_upstream_http_429_rate_limit,
            policy_status_change_upstream_http_429_quota_exhausted,
            policy_status_change_usage_snapshot_exhausted,
            policy_status_change_quota_still_exhausted,
            policy_status_change_transport_failure,
            policy_status_change_upstream_server_overloaded,
            policy_status_change_upstream_http_5xx,
            policy_responses_first_byte_timeout_secs,
            policy_compact_first_byte_timeout_secs,
            policy_image_first_byte_timeout_secs,
            policy_responses_stream_timeout_secs,
            policy_compact_stream_timeout_secs
        FROM pool_upstream_account_group_notes
        WHERE group_name = ?1
        LIMIT 1
        "#,
    )
    .bind(group_name)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(group_routing_rule_from_columns(
            0, false, 0, None, None, None, None, None, None, None, None, None, None, None, None,
            None, None, None, None, None, None, None, None, None, None, None, None, None, None,
        ));
    };
    let upstream_429_retry_enabled =
        decode_group_upstream_429_retry_enabled(row.upstream_429_retry_enabled.unwrap_or_default());
    let upstream_429_max_retries = normalize_group_upstream_429_retry_metadata(
        upstream_429_retry_enabled,
        decode_group_upstream_429_max_retries(row.upstream_429_max_retries.unwrap_or_default()),
    );
    Ok(group_routing_rule_from_columns(
        row.concurrency_limit.unwrap_or_default(),
        upstream_429_retry_enabled,
        upstream_429_max_retries,
        row.policy_allow_cut_out,
        row.policy_allow_cut_in,
        row.policy_priority_tier.as_deref(),
        row.policy_fast_mode_rewrite_mode.as_deref(),
        row.policy_image_tool_rewrite_mode.as_deref(),
        row.policy_request_compression_algorithm.as_deref(),
        row.policy_concurrency_limit,
        row.policy_upstream_429_retry_enabled,
        row.policy_upstream_429_max_retries,
        row.policy_available_models_json.as_deref(),
        row.policy_status_change_upstream_http_401,
        row.policy_status_change_upstream_http_402,
        row.policy_status_change_upstream_http_403,
        row.policy_status_change_reauth_required,
        row.policy_status_change_upstream_http_429_rate_limit,
        row.policy_status_change_upstream_http_429_quota_exhausted,
        row.policy_status_change_usage_snapshot_exhausted,
        row.policy_status_change_quota_still_exhausted,
        row.policy_status_change_transport_failure,
        row.policy_status_change_upstream_server_overloaded,
        row.policy_status_change_upstream_http_5xx,
        row.policy_responses_first_byte_timeout_secs,
        row.policy_compact_first_byte_timeout_secs,
        row.policy_image_first_byte_timeout_secs,
        row.policy_responses_stream_timeout_secs,
        row.policy_compact_stream_timeout_secs,
    ))
}
