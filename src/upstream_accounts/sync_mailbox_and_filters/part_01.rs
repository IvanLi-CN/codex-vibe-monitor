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
