pub(crate) const ENV_UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET: &str =
    "UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID: &str = "UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_OAUTH_ISSUER: &str = "UPSTREAM_ACCOUNTS_OAUTH_ISSUER";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_USAGE_BASE_URL: &str = "UPSTREAM_ACCOUNTS_USAGE_BASE_URL";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS: &str =
    "UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS: &str =
    "UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS: &str =
    "UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS: &str =
    "UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_BASE_URL: &str =
    "UPSTREAM_ACCOUNTS_KAISOUMAIL_BASE_URL";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_API_KEY: &str =
    "UPSTREAM_ACCOUNTS_KAISOUMAIL_API_KEY";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_MAIL_DOMAIN: &str =
    "UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_MAIL_DOMAIN";
pub(crate) const ENV_UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_SUBDOMAIN: &str =
    "UPSTREAM_ACCOUNTS_KAISOUMAIL_DEFAULT_SUBDOMAIN";

pub(crate) const LEGACY_ENV_UPSTREAM_ACCOUNTS_MOEMAIL_BASE_URL: &str =
    "UPSTREAM_ACCOUNTS_MOEMAIL_BASE_URL";
pub(crate) const LEGACY_ENV_UPSTREAM_ACCOUNTS_MOEMAIL_API_KEY: &str =
    "UPSTREAM_ACCOUNTS_MOEMAIL_API_KEY";
pub(crate) const LEGACY_ENV_UPSTREAM_ACCOUNTS_MOEMAIL_DEFAULT_DOMAIN: &str =
    "UPSTREAM_ACCOUNTS_MOEMAIL_DEFAULT_DOMAIN";

pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_OAUTH_ISSUER: &str = "https://auth.openai.com";
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_USAGE_BASE_URL: &str = "https://chatgpt.com/backend-api";
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_LOGIN_SESSION_TTL_SECS: u64 = 10 * 60;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS: u64 = 5 * 60;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_REFRESH_LEAD_TIME_SECS: u64 = 5 * 60;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_HISTORY_RETENTION_DAYS: u64 = 30;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_SECONDARY_SYNC_INTERVAL_SECS: u64 = 30 * 60;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_PRIORITY_AVAILABLE_ACCOUNT_CAP: usize = 100;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_MAINTENANCE_PARALLELISM: usize = 4;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNTS_MAILBOX_SESSION_TTL_SECS: u64 = 60 * 60;
pub(crate) const DEFAULT_MANUAL_OAUTH_CALLBACK_PORT: u16 = 1455;
pub(crate) const MIN_UPSTREAM_ACCOUNTS_SYNC_INTERVAL_SECS: u64 = 60;
pub(crate) const UPSTREAM_ACCOUNT_MAINTENANCE_TICK_SECS: u64 = 60;
pub(crate) const UPSTREAM_ACCOUNT_UPSTREAM_REJECTED_MAINTENANCE_COOLDOWN_SECS: i64 = 6 * 60 * 60;
pub(crate) const OAUTH_MAILBOX_SOURCE_GENERATED: &str = "generated";
pub(crate) const OAUTH_MAILBOX_SOURCE_ATTACHED: &str = "attached";

pub(crate) const UPSTREAM_ACCOUNT_KIND_OAUTH_CODEX: &str = "oauth_codex";
pub(crate) const UPSTREAM_ACCOUNT_KIND_API_KEY_CODEX: &str = "api_key_codex";
pub(crate) const UPSTREAM_ACCOUNT_PROVIDER_CODEX: &str = "codex";
pub(crate) const DEFAULT_UPSTREAM_ACCOUNT_GROUP_NAME: &str = "未分组";
pub(crate) const UPSTREAM_ACCOUNT_STATUS_ACTIVE: &str = "active";
pub(crate) const UPSTREAM_ACCOUNT_STATUS_SYNCING: &str = "syncing";
pub(crate) const UPSTREAM_ACCOUNT_STATUS_NEEDS_REAUTH: &str = "needs_reauth";
pub(crate) const UPSTREAM_ACCOUNT_STATUS_ERROR: &str = "error";
pub(crate) const UPSTREAM_ACCOUNT_STATUS_DISABLED: &str = "disabled";
pub(crate) const UPSTREAM_ACCOUNT_ENABLE_STATUS_ENABLED: &str = "enabled";
pub(crate) const UPSTREAM_ACCOUNT_ENABLE_STATUS_DISABLED: &str = "disabled";
pub(crate) const UPSTREAM_ACCOUNT_WORK_STATUS_WORKING: &str = "working";
pub(crate) const UPSTREAM_ACCOUNT_WORK_STATUS_DEGRADED: &str = "degraded";
pub(crate) const UPSTREAM_ACCOUNT_WORK_STATUS_IDLE: &str = "idle";
pub(crate) const UPSTREAM_ACCOUNT_WORK_STATUS_RATE_LIMITED: &str = "rate_limited";
pub(crate) const UPSTREAM_ACCOUNT_WORK_STATUS_UNAVAILABLE: &str = "unavailable";
pub(crate) const UPSTREAM_ACCOUNT_HEALTH_STATUS_NORMAL: &str = "normal";
pub(crate) const UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_UNAVAILABLE: &str =
    "upstream_unavailable";
pub(crate) const UPSTREAM_ACCOUNT_DISPLAY_STATUS_UPSTREAM_REJECTED: &str = "upstream_rejected";
pub(crate) const UPSTREAM_ACCOUNT_DISPLAY_STATUS_ERROR_OTHER: &str = "error_other";
pub(crate) const UPSTREAM_ACCOUNT_SYNC_STATE_IDLE: &str = "idle";
pub(crate) const UPSTREAM_ACCOUNT_SYNC_STATE_SYNCING: &str = "syncing";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_ROUTE_RECOVERED: &str = "route_recovered";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_ROUTE_COOLDOWN_STARTED: &str = "route_cooldown_started";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_ROUTE_RETRYABLE_FAILURE: &str = "route_retryable_failure";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_ROUTE_HARD_UNAVAILABLE: &str = "route_hard_unavailable";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_DEGRADED: &str = "model_route_degraded";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_COOLDOWN: &str = "model_route_cooldown";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RECOVERED: &str = "model_route_recovered";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_RESET: &str = "model_route_reset";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_MODEL_ROUTE_CACHE_OBSERVATION_MISSING: &str =
    "model_route_cache_observation_missing";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_STATUS_CHANGE_SUPPRESSED: &str =
    "status_change_suppressed";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SYNC_SUCCEEDED: &str = "sync_succeeded";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SYNC_DEFERRED: &str = "sync_deferred";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SYNC_HARD_UNAVAILABLE: &str = "sync_hard_unavailable";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SYNC_RECOVERY_BLOCKED: &str = "sync_recovery_blocked";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SYNC_FAILED: &str = "sync_failed";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_ACCOUNT_UPDATED: &str = "account_updated";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_CREDENTIAL_REFRESH_SUCCEEDED: &str =
    "credential_refresh_succeeded";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_CREDENTIAL_REFRESH_FAILED: &str =
    "credential_refresh_failed";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_CREDENTIAL_REAUTH_REQUIRED: &str =
    "credential_reauth_required";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_USAGE_WINDOWS_REFRESH_SUCCEEDED: &str =
    "usage_windows_refresh_succeeded";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_USAGE_WINDOWS_REFRESH_PARTIAL: &str =
    "usage_windows_refresh_partial";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_USAGE_WINDOWS_REFRESH_FAILED: &str =
    "usage_windows_refresh_failed";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SOURCE_CALL: &str = "call";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MANUAL: &str = "sync_manual";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_MAINTENANCE: &str = "sync_maintenance";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SOURCE_SYNC_POST_CREATE: &str = "sync_post_create";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SOURCE_OAUTH_IMPORT: &str = "oauth_import";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SOURCE_OAUTH_CALLBACK: &str = "oauth_callback";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SOURCE_ACCOUNT_UPDATE: &str = "account_update";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SOURCE_CREDENTIAL_REFRESH: &str = "credential_refresh";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_SOURCE_USAGE_WINDOWS_REFRESH: &str =
    "usage_windows_refresh";
pub(crate) const UPSTREAM_ACCOUNT_REFRESH_OPERATION_CREDENTIALS: &str = "credentials";
pub(crate) const UPSTREAM_ACCOUNT_REFRESH_OPERATION_USAGE_WINDOWS: &str = "usage_windows";
pub(crate) const UPSTREAM_ACCOUNT_REFRESH_STATE_IDLE: &str = "idle";
pub(crate) const UPSTREAM_ACCOUNT_REFRESH_STATE_REFRESHING: &str = "refreshing";
pub(crate) const UPSTREAM_ACCOUNT_REFRESH_STATE_SUCCEEDED: &str = "succeeded";
pub(crate) const UPSTREAM_ACCOUNT_REFRESH_STATE_RETRYING: &str = "retrying";
pub(crate) const UPSTREAM_ACCOUNT_REFRESH_STATE_PARTIAL: &str = "partial";
pub(crate) const UPSTREAM_ACCOUNT_REFRESH_STATE_NEEDS_REAUTH: &str = "needs_reauth";
pub(crate) const UPSTREAM_ACCOUNT_REFRESH_STATE_ERROR: &str = "error";
pub(crate) const UPSTREAM_ACCOUNT_REFRESH_RETRY_INTERVAL_SECS: i64 = 60;
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_OK: &str = "sync_ok";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_ACCOUNT_UPDATED: &str = "account_updated";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_SYNC_ERROR: &str = "sync_error";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_EGRESS_THROTTLED: &str = "egress_throttled";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED: &str =
    "usage_snapshot_exhausted";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED: &str =
    "quota_still_exhausted";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_RECOVERY_UNCONFIRMED_MANUAL_REQUIRED: &str =
    "recovery_unconfirmed_manual_required";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_401: &str = "upstream_http_401";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402: &str = "upstream_http_402";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_403: &str = "upstream_http_403";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT: &str =
    "upstream_http_429_rate_limit";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED: &str =
    "upstream_http_429_quota_exhausted";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE: &str = "transport_failure";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED: &str =
    "upstream_server_overloaded";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED: &str = "reauth_required";
pub(crate) const UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX: &str = "upstream_http_5xx";
pub(crate) const LEGACY_UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_REJECTED: &str =
    "upstream_rejected";
pub(crate) const UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED: &str =
    "group_node_shunt_unassigned";
pub(crate) const UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_GROUP_NODE_SHUNT_UNASSIGNED_MESSAGE: &str =
    "分组节点分流策略控制，未排节点";
pub(crate) const UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_RECENT_UPSTREAM_STREAM_ERRORS: &str =
    "recent_upstream_stream_errors";
pub(crate) const UPSTREAM_ACCOUNT_ROUTING_BLOCK_REASON_RECENT_UPSTREAM_STREAM_ERRORS_MESSAGE: &str =
    "近期连续上游流错误，自动路由将在五分钟后恢复";
pub(crate) const UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_ASSIGNED: &str = "assigned";
pub(crate) const UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_PENDING: &str = "pending";
pub(crate) const UPSTREAM_ACCOUNT_FORWARD_PROXY_STATE_UNCONFIGURED: &str = "unconfigured";
pub(crate) const BULK_UPSTREAM_ACCOUNT_ACTION_ENABLE: &str = "enable";
pub(crate) const BULK_UPSTREAM_ACCOUNT_ACTION_DISABLE: &str = "disable";
pub(crate) const BULK_UPSTREAM_ACCOUNT_ACTION_DELETE: &str = "delete";
pub(crate) const BULK_UPSTREAM_ACCOUNT_ACTION_SET_GROUP: &str = "set_group";
pub(crate) const BULK_UPSTREAM_ACCOUNT_ACTION_ADD_TAGS: &str = "add_tags";
pub(crate) const BULK_UPSTREAM_ACCOUNT_ACTION_REMOVE_TAGS: &str = "remove_tags";
pub(crate) const BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_PENDING: &str = "pending";
pub(crate) const BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SUCCEEDED: &str = "succeeded";
pub(crate) const BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_FAILED: &str = "failed";
pub(crate) const BULK_UPSTREAM_ACCOUNT_SYNC_STATUS_SKIPPED: &str = "skipped";
pub(crate) const BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_RUNNING: &str = "running";
pub(crate) const BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_COMPLETED: &str = "completed";
pub(crate) const BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_FAILED: &str = "failed";
pub(crate) const BULK_UPSTREAM_ACCOUNT_SYNC_JOB_STATUS_CANCELLED: &str = "cancelled";
pub(crate) const LOGIN_SESSION_STATUS_PENDING: &str = "pending";
pub(crate) const LOGIN_SESSION_STATUS_COMPLETED: &str = "completed";
pub(crate) const LOGIN_SESSION_STATUS_FAILED: &str = "failed";
pub(crate) const LOGIN_SESSION_STATUS_EXPIRED: &str = "expired";
pub(crate) const LOGIN_SESSION_STATUS_NEEDS_IDENTITY_CONFIRMATION: &str =
    "needs_identity_confirmation";
pub(crate) const LOGIN_SESSION_BASE_UPDATED_AT_HEADER: &str =
    "x-codex-login-session-base-updated-at";
pub(crate) const IMPORT_VALIDATION_STATUS_OK: &str = "ok";
pub(crate) const IMPORT_VALIDATION_STATUS_OK_EXHAUSTED: &str = "ok_exhausted";
pub(crate) const IMPORT_VALIDATION_STATUS_INVALID: &str = "invalid";
pub(crate) const IMPORT_VALIDATION_STATUS_ERROR: &str = "error";
pub(crate) const IMPORT_VALIDATION_STATUS_DUPLICATE_IN_INPUT: &str = "duplicate_in_input";
pub(crate) const IMPORT_RESULT_STATUS_CREATED: &str = "created";
pub(crate) const IMPORT_RESULT_STATUS_UPDATED_EXISTING: &str = "updated_existing";
pub(crate) const IMPORT_RESULT_STATUS_FAILED: &str = "failed";
pub(crate) const DEFAULT_OAUTH_SCOPE: &str = "openid profile email offline_access";
pub(crate) const DEFAULT_OAUTH_AUDIENCE: &str = "https://api.openai.com/v1";
pub(crate) const DEFAULT_OAUTH_PROMPT: &str = "login";
pub(crate) const OAUTH_ORIGINATOR: &str = "Codex Desktop";
pub(crate) const DEFAULT_USAGE_LIMIT_ID: &str = "codex";
pub(crate) const DEFAULT_API_KEY_LIMIT_UNIT: &str = "requests";
pub(crate) const POOL_SETTINGS_SINGLETON_ID: i64 = 1;
pub(crate) const DEFAULT_STICKY_KEY_LIMIT: i64 = 50;
pub(crate) const STICKY_KEY_ACTIVITY_MODE_LIMIT: i64 = 50;
pub(crate) const DEFAULT_UPSTREAM_ACCOUNT_LIST_PAGE_SIZE: usize = 20;
pub(crate) const UPSTREAM_ACCOUNT_LIST_PAGE_SIZE_OPTIONS: [usize; 3] = [20, 50, 100];
pub(crate) const POOL_ROUTE_ACTIVE_STICKY_WINDOW_MINUTES: i64 = 5;
pub(crate) const POOL_ROUTE_TEMPORARY_FAILURE_STREAK_THRESHOLD: i64 = 5;
pub(crate) const POOL_ROUTE_TEMPORARY_FAILURE_DEGRADED_WINDOW_SECS: i64 = 30;
pub(crate) const POOL_ROUTE_TEMPORARY_FAILURE_COOLDOWN_MAX_SECS: i64 = 60;
pub(crate) const STATUS_CHANGE_REASON_CODES: [&str; 11] = [
    UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_401,
    UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_402,
    UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_403,
    UPSTREAM_ACCOUNT_ACTION_REASON_REAUTH_REQUIRED,
    UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_RATE_LIMIT,
    UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_429_QUOTA_EXHAUSTED,
    UPSTREAM_ACCOUNT_ACTION_REASON_USAGE_SNAPSHOT_EXHAUSTED,
    UPSTREAM_ACCOUNT_ACTION_REASON_QUOTA_STILL_EXHAUSTED,
    UPSTREAM_ACCOUNT_ACTION_REASON_TRANSPORT_FAILURE,
    UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_SERVER_OVERLOADED,
    UPSTREAM_ACCOUNT_ACTION_REASON_UPSTREAM_HTTP_5XX,
];
pub(crate) const COMPACT_SUPPORT_STATUS_UNKNOWN: &str = "unknown";
pub(crate) const COMPACT_SUPPORT_STATUS_SUPPORTED: &str = "supported";
pub(crate) const COMPACT_SUPPORT_STATUS_UNSUPPORTED: &str = "unsupported";
pub(crate) const USAGE_PATH_STYLE_CHATGPT: &str = "/wham/usage";
pub(crate) const USAGE_PATH_STYLE_CODEX_API: &str = "/api/codex/usage";
pub(crate) const UPSTREAM_USAGE_BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/134.0.0.0 Safari/537.36";
