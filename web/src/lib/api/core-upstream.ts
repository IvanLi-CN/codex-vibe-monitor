import type {
  ForwardProxyBindingNode,
  StickyKeyConversationSelection,
  UpstreamStickyConversationsResponse,
} from "./core-foundation";
import {
  ensureJsonRequestOk,
  fetchJson,
  normalizeCompactSupportState,
  normalizeFiniteNumber,
  normalizeForwardProxyBindingNode,
  normalizePoolRoutingSettings,
  normalizeStringArray,
  normalizeUpstreamStickyConversationsResponse,
  withBase,
} from "./core-foundation";

const OAUTH_LOGIN_SESSION_BASE_UPDATED_AT_HEADER = "X-Codex-Login-Session-Base-Updated-At";

export interface RateWindowActualUsage {
  requestCount: number;
  totalTokens: number;
  totalCost: number;
  inputTokens: number;
  outputTokens: number;
  cacheInputTokens: number;
}

export interface RateWindowSnapshot {
  usedPercent: number;
  usedText: string;
  limitText: string;
  resetsAt?: string | null;
  windowDurationMins: number;
  actualUsage?: RateWindowActualUsage | null;
}

export interface CreditsSnapshot {
  hasCredits: boolean;
  unlimited: boolean;
  balance?: string | null;
}

export interface LocalLimitSnapshot {
  primaryLimit?: number | null;
  secondaryLimit?: number | null;
  limitUnit: string;
}

export interface CompactSupportState {
  status: "unknown" | "supported" | "unsupported" | string;
  observedAt?: string | null;
  reason?: string | null;
}

export interface UpstreamAccountHistoryPoint {
  capturedAt: string;
  primaryUsedPercent?: number | null;
  secondaryUsedPercent?: number | null;
  creditsBalance?: string | null;
}

export interface UpstreamAccountDuplicateInfo {
  peerAccountIds: number[];
  reasons: Array<"sharedChatgptAccountId" | "sharedChatgptUserId" | string>;
}

export type TagPriorityTier = "primary" | "normal" | "fallback" | "no_new";
export type TagFastModeRewriteMode =
  | "force_remove"
  | "keep_original"
  | "fill_missing"
  | "force_add";
export type ImageToolRewriteMode = "keep_original" | "fill_missing" | "force_add" | "force_remove";
export type ImageToolCapability = "supported" | "unsupported" | "unknown";
export type ImageIntent = "yes" | "direct_image" | "no" | "unknown";

export interface TagRoutingRule {
  allowCutOut: boolean;
  allowCutIn: boolean;
  priorityTier?: TagPriorityTier;
  fastModeRewriteMode?: TagFastModeRewriteMode;
  concurrencyLimit?: number | null;
  upstream429RetryEnabled?: boolean;
  upstream429MaxRetries?: number;
  availableModels?: string[];
  availableModelsDefined?: boolean;
}

export type EffectiveRoutingRuleSource =
  | "root"
  | "group"
  | "tag"
  | "account"
  | "conversation"
  | string;

export const STATUS_CHANGE_REASON_CODES = [
  "upstream_http_401",
  "upstream_http_402",
  "upstream_http_403",
  "reauth_required",
  "upstream_http_429_rate_limit",
  "upstream_http_429_quota_exhausted",
  "usage_snapshot_exhausted",
  "quota_still_exhausted",
  "transport_failure",
  "upstream_server_overloaded",
  "upstream_http_5xx",
] as const;

export type StatusChangeReasonCode = (typeof STATUS_CHANGE_REASON_CODES)[number];

export type StatusChangeReasons = Record<StatusChangeReasonCode, boolean>;

export type StatusChangeReasonFieldSources = Record<
  StatusChangeReasonCode,
  EffectiveRoutingRuleSource
>;

export function buildDefaultStatusChangeReasons(): StatusChangeReasons {
  return Object.fromEntries(
    STATUS_CHANGE_REASON_CODES.map((reason) => [reason, true]),
  ) as StatusChangeReasons;
}

export function buildDefaultStatusChangeReasonFieldSources(
  source: EffectiveRoutingRuleSource = "root",
): StatusChangeReasonFieldSources {
  return Object.fromEntries(
    STATUS_CHANGE_REASON_CODES.map((reason) => [reason, source]),
  ) as StatusChangeReasonFieldSources;
}

export interface PoolRoutingTimeoutSettings {
  responsesFirstByteTimeoutSecs: number;
  compactFirstByteTimeoutSecs: number;
  responsesStreamTimeoutSecs: number;
  compactStreamTimeoutSecs: number;
}

export interface GroupAccountRoutingRule extends TagRoutingRule {
  imageToolRewriteMode?: ImageToolRewriteMode;
  statusChangeReasons?: StatusChangeReasons;
  timeouts?: Partial<PoolRoutingTimeoutSettings>;
}

export interface EffectiveRoutingRuleFieldSources {
  allowCutOut: EffectiveRoutingRuleSource;
  allowCutIn: EffectiveRoutingRuleSource;
  priorityTier: EffectiveRoutingRuleSource;
  fastModeRewriteMode: EffectiveRoutingRuleSource;
  imageToolRewriteMode?: EffectiveRoutingRuleSource;
  concurrencyLimit: EffectiveRoutingRuleSource;
  upstream429Retry: EffectiveRoutingRuleSource;
  availableModels?: EffectiveRoutingRuleSource;
  systemDeniedModels?: EffectiveRoutingRuleSource;
}

export interface EffectiveRoutingTimeoutFieldSources {
  responsesFirstByteTimeoutSecs: EffectiveRoutingRuleSource;
  compactFirstByteTimeoutSecs: EffectiveRoutingRuleSource;
  responsesStreamTimeoutSecs: EffectiveRoutingRuleSource;
  compactStreamTimeoutSecs: EffectiveRoutingRuleSource;
}

export interface EffectiveRoutingRule extends GroupAccountRoutingRule {
  systemDeniedModels?: string[];
  sourceTagIds: number[];
  sourceTagNames: string[];
  fieldSources?: EffectiveRoutingRuleFieldSources;
  statusChangeReasonFieldSources?: StatusChangeReasonFieldSources;
  timeouts?: PoolRoutingTimeoutSettings;
  timeoutFieldSources?: EffectiveRoutingTimeoutFieldSources;
}

export interface AccountTagSummary {
  id: number;
  name: string;
  routingRule: TagRoutingRule;
  systemKey?: string | null;
  protected?: boolean;
}

export interface TagSummary {
  id: number;
  name: string;
  routingRule: TagRoutingRule;
  accountCount: number;
  groupCount: number;
  updatedAt: string;
  systemKey?: string | null;
  protected?: boolean;
}

export type TagDetail = TagSummary;

export interface TagListResponse {
  writesEnabled: boolean;
  items: TagSummary[];
}

export type UpstreamAccountForwardProxyState = "assigned" | "pending" | "unconfigured" | string;

export interface UpstreamAccountSummary {
  id: number;
  kind: "oauth_codex" | "api_key_codex" | string;
  provider: string;
  displayName: string;
  groupName?: string | null;
  isMother: boolean;
  status: "active" | "syncing" | "needs_reauth" | "error" | "disabled" | string;
  workStatus?: "working" | "degraded" | "idle" | "rate_limited" | "unavailable" | string;
  enableStatus?: "enabled" | "disabled" | string;
  healthStatus?:
    | "normal"
    | "needs_reauth"
    | "upstream_unavailable"
    | "upstream_rejected"
    | "error_other"
    | string;
  syncState?: "idle" | "syncing" | string;
  displayStatus?:
    | "active"
    | "syncing"
    | "needs_reauth"
    | "upstream_unavailable"
    | "upstream_rejected"
    | "error_other"
    | "disabled"
    | string;
  enabled: boolean;
  email?: string | null;
  chatgptAccountId?: string | null;
  planType?: string | null;
  maskedApiKey?: string | null;
  hasRefreshToken?: boolean;
  lastSyncedAt?: string | null;
  lastSuccessfulSyncAt?: string | null;
  lastActivityAt?: string | null;
  activeConversationCount?: number;
  lastError?: string | null;
  lastErrorAt?: string | null;
  lastAction?: string | null;
  lastActionSource?: string | null;
  lastActionReasonCode?: string | null;
  lastActionReasonMessage?: string | null;
  routingBlockReasonCode?: string | null;
  routingBlockReasonMessage?: string | null;
  lastActionHttpStatus?: number | null;
  lastActionInvokeId?: string | null;
  lastActionAt?: string | null;
  cooldownUntil?: string | null;
  boundProxyKeys?: string[];
  currentForwardProxyKey?: string | null;
  currentForwardProxyDisplayName?: string | null;
  currentForwardProxyState?: UpstreamAccountForwardProxyState;
  tokenExpiresAt?: string | null;
  primaryWindow?: RateWindowSnapshot | null;
  secondaryWindow?: RateWindowSnapshot | null;
  credits?: CreditsSnapshot | null;
  localLimits?: LocalLimitSnapshot | null;
  compactSupport?: CompactSupportState | null;
  duplicateInfo?: UpstreamAccountDuplicateInfo | null;
  imageToolCapability?: ImageToolCapability;
  tags: AccountTagSummary[];
  effectiveRoutingRule: EffectiveRoutingRule;
}

export interface UpstreamAccountActionEvent {
  id: number;
  occurredAt: string;
  action: string;
  source: string;
  accountDisplayName?: string | null;
  accountGroupName?: string | null;
  forwardProxyKey?: string | null;
  forwardProxyDisplayName?: string | null;
  forwardProxyEgressIp?: string | null;
  result?: string | null;
  resultDescription?: string | null;
  reasonCode?: string | null;
  reasonMessage?: string | null;
  httpStatus?: number | null;
  failureKind?: string | null;
  invokeId?: string | null;
  attemptId?: number | null;
  stickyKey?: string | null;
  createdAt: string;
}

export interface UpstreamAccountDetail extends UpstreamAccountSummary {
  note?: string | null;
  upstreamBaseUrl?: string | null;
  chatgptUserId?: string | null;
  verifiedEmail?: string | null;
  lastRefreshedAt?: string | null;
  history: UpstreamAccountHistoryPoint[];
  recentActions?: UpstreamAccountActionEvent[];
}

export interface UpstreamAccountActionEventListResponse {
  items: UpstreamAccountActionEvent[];
  total: number;
  page: number;
  pageSize: number;
}

export interface UpstreamAccountGroupSummary {
  groupName: string;
  accountCount?: number;
  note?: string | null;
  boundProxyKeys?: string[];
  concurrencyLimit?: number | null;
  nodeShuntEnabled?: boolean;
  singleAccountRotationEnabled?: boolean;
  upstream429RetryEnabled?: boolean;
  upstream429MaxRetries?: number;
  routingRule?: GroupAccountRoutingRule;
  effectiveTimeouts?: PoolRoutingTimeoutSettings;
  timeoutFieldSources?: EffectiveRoutingTimeoutFieldSources;
}

export interface UpdatePoolRoutingSettingsPayload {
  apiKey?: string;
  maintenance?: UpdatePoolRoutingMaintenanceSettingsPayload;
  timeouts?: Partial<PoolRoutingTimeoutSettings>;
}

export interface PoolRoutingSettings {
  writesEnabled: boolean;
  apiKeyConfigured: boolean;
  maskedApiKey?: string | null;
  maintenance?: PoolRoutingMaintenanceSettings;
  timeouts?: PoolRoutingTimeoutSettings;
}

export interface PoolRoutingMaintenanceSettings {
  primarySyncIntervalSecs: number;
  secondarySyncIntervalSecs: number;
  priorityAvailableAccountCap: number;
}

export interface UpdatePoolRoutingMaintenanceSettingsPayload {
  primarySyncIntervalSecs?: number;
  secondarySyncIntervalSecs?: number;
  priorityAvailableAccountCap?: number;
}

export interface UpstreamAccountListResponse {
  writesEnabled: boolean;
  items: UpstreamAccountSummary[];
  groups: UpstreamAccountGroupSummary[];
  forwardProxyNodes?: ForwardProxyBindingNode[];
  hasUngroupedAccounts: boolean;
  total?: number;
  page?: number;
  pageSize?: number;
  metrics?: UpstreamAccountListMetrics;
  routing?: PoolRoutingSettings | null;
}

export interface UpstreamAccountWindowUsageItem {
  accountId: number;
  primaryActualUsage: RateWindowActualUsage | null;
  secondaryActualUsage: RateWindowActualUsage | null;
}

export interface UpstreamAccountWindowUsageResponse {
  items: UpstreamAccountWindowUsageItem[];
}

export interface FetchUpstreamAccountsQuery {
  groupExact?: string[];
  groupSearch?: string;
  groupUngrouped?: boolean;
  status?: string;
  workStatus?: string[];
  enableStatus?: string[];
  healthStatus?: string[];
  page?: number;
  pageSize?: number;
  includeAll?: boolean;
  tagIds?: number[];
}

export interface FetchUpstreamAccountActionEventsQuery {
  account?: string;
  group?: string;
  proxyKey?: string;
  result?: string;
  page?: number;
  pageSize?: number;
}

export interface UpstreamAccountListMetrics {
  total: number;
  oauth: number;
  apiKey: number;
  attention: number;
}

export interface BulkUpstreamAccountActionPayload {
  accountIds: number[];
  action: "enable" | "disable" | "delete" | "set_group" | "add_tags" | "remove_tags" | string;
  groupName?: string | null;
  tagIds?: number[];
}

export interface BulkUpstreamAccountActionResult {
  accountId: number;
  displayName?: string | null;
  status: "succeeded" | "failed" | string;
  detail?: string | null;
}

export interface BulkUpstreamAccountActionResponse {
  action: string;
  requestedCount: number;
  completedCount: number;
  succeededCount: number;
  failedCount: number;
  results: BulkUpstreamAccountActionResult[];
}

export interface BulkUpstreamAccountSyncJobPayload {
  accountIds: number[];
}

export interface BulkUpstreamAccountSyncCounts {
  total: number;
  completed: number;
  succeeded: number;
  failed: number;
  skipped: number;
}

export interface BulkUpstreamAccountSyncRow {
  accountId: number;
  displayName: string;
  status: "pending" | "succeeded" | "failed" | "skipped" | string;
  detail?: string | null;
}

export interface BulkUpstreamAccountSyncSnapshot {
  jobId: string;
  status: "running" | "completed" | "failed" | "cancelled" | string;
  rows: BulkUpstreamAccountSyncRow[];
}

export interface BulkUpstreamAccountSyncJobResponse {
  jobId: string;
  snapshot: BulkUpstreamAccountSyncSnapshot;
  counts: BulkUpstreamAccountSyncCounts;
}

export interface BulkUpstreamAccountSyncSnapshotEventPayload {
  snapshot: BulkUpstreamAccountSyncSnapshot;
  counts: BulkUpstreamAccountSyncCounts;
}

export interface BulkUpstreamAccountSyncRowEventPayload {
  row: BulkUpstreamAccountSyncRow;
  counts: BulkUpstreamAccountSyncCounts;
}

export interface BulkUpstreamAccountSyncFailedEventPayload {
  snapshot: BulkUpstreamAccountSyncSnapshot;
  counts: BulkUpstreamAccountSyncCounts;
  error: string;
}

export interface LoginSessionStatusResponse {
  loginId: string;
  status: "pending" | "completed" | "failed" | "expired" | "needs_identity_confirmation" | string;
  authUrl?: string | null;
  redirectUri?: string | null;
  expiresAt: string;
  updatedAt?: string | null;
  accountId?: number | null;
  email?: string | null;
  error?: string | null;
  syncApplied?: boolean | null;
  identityConfirmation?: OauthIdentityConfirmation | null;
}

export interface OauthIdentityConfirmation {
  current: OauthIdentitySummary;
  incoming: OauthIdentitySummary;
}

export interface OauthIdentitySummary {
  accountId?: number | null;
  displayName?: string | null;
  email?: string | null;
  verifiedEmail?: string | null;
  chatgptAccountId?: string | null;
  chatgptUserId?: string | null;
  planType?: string | null;
}

export type OauthMailboxSession = OauthMailboxSessionSupported | OauthMailboxSessionUnsupported;

export interface OauthMailboxSessionSupported {
  supported: true;
  sessionId: string;
  emailAddress: string;
  expiresAt: string;
  source: "generated" | "attached" | string;
}

export interface OauthMailboxSessionUnsupported {
  supported: false;
  emailAddress: string;
  reason: "invalid_format" | "unsupported_domain" | "not_readable" | string;
}

export interface OauthMailboxCodeSummary {
  value: string;
  source: string;
  updatedAt: string;
}

export interface OauthInviteSummary {
  subject: string;
  copyValue: string;
  copyLabel: string;
  updatedAt: string;
}

export interface OauthMailboxStatus {
  sessionId: string;
  emailAddress: string;
  expiresAt: string;
  latestCode?: OauthMailboxCodeSummary | null;
  invite?: OauthInviteSummary | null;
  invited: boolean;
  error?: string | null;
}

export interface CreateOauthLoginSessionPayload {
  displayName?: string;
  email?: string;
  groupName?: string;
  groupBoundProxyKeys?: string[];
  groupNodeShuntEnabled?: boolean;
  groupSingleAccountRotationEnabled?: boolean;
  note?: string;
  groupNote?: string;
  concurrencyLimit?: number;
  accountId?: number;
  tagIds?: number[];
  isMother?: boolean;
  mailboxSessionId?: string;
  mailboxAddress?: string;
}

export interface UpdateOauthLoginSessionPayload {
  displayName?: string;
  email?: string | null;
  groupName?: string;
  groupBoundProxyKeys?: string[];
  groupNodeShuntEnabled?: boolean;
  groupSingleAccountRotationEnabled?: boolean;
  note?: string;
  groupNote?: string;
  concurrencyLimit?: number;
  tagIds?: number[];
  isMother?: boolean;
  mailboxSessionId?: string;
  mailboxAddress?: string;
}

function withOauthLoginSessionBaseUpdatedAtHeader(
  baseUpdatedAt: string | null | undefined,
  init: RequestInit,
): RequestInit {
  const normalizedBaseUpdatedAt = baseUpdatedAt?.trim();
  if (!normalizedBaseUpdatedAt) {
    return init;
  }
  const headers = new Headers(init.headers);
  if (!headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }
  headers.set(OAUTH_LOGIN_SESSION_BASE_UPDATED_AT_HEADER, normalizedBaseUpdatedAt);
  return {
    ...init,
    headers,
  };
}

export interface CompleteOauthLoginSessionPayload {
  callbackUrl: string;
  mailboxSessionId?: string;
  mailboxAddress?: string;
}

export interface CreateOauthMailboxSessionPayload {
  emailAddress?: string;
}

export interface OauthMailboxStatusRequestPayload {
  sessionIds: string[];
}

export interface CreateApiKeyAccountPayload {
  displayName: string;
  email?: string;
  groupName?: string;
  groupBoundProxyKeys?: string[];
  groupNodeShuntEnabled?: boolean;
  groupSingleAccountRotationEnabled?: boolean;
  note?: string;
  groupNote?: string;
  concurrencyLimit?: number;
  upstreamBaseUrl?: string;
  apiKey: string;
  isMother?: boolean;
  localPrimaryLimit?: number;
  localSecondaryLimit?: number;
  localLimitUnit?: string;
  tagIds?: number[];
}

export interface UpdateUpstreamAccountPayload {
  displayName?: string;
  email?: string | null;
  groupName?: string;
  groupBoundProxyKeys?: string[];
  boundProxyKeys?: string[] | null;
  concurrencyLimit?: number;
  groupNodeShuntEnabled?: boolean;
  groupSingleAccountRotationEnabled?: boolean;
  note?: string;
  groupNote?: string;
  upstreamBaseUrl?: string | null;
  enabled?: boolean;
  isMother?: boolean;
  apiKey?: string;
  localPrimaryLimit?: number | null;
  localSecondaryLimit?: number | null;
  localLimitUnit?: string | null;
  tagIds?: number[];
  routingRule?: UpdateGroupAccountRoutingRulePayload;
}

export interface ImportOauthCredentialFilePayload {
  sourceId: string;
  fileName: string;
  content: string;
}

export interface ValidateImportedOauthAccountsPayload {
  groupName?: string;
  groupBoundProxyKeys?: string[];
  groupNodeShuntEnabled?: boolean;
  groupSingleAccountRotationEnabled?: boolean;
  items: ImportOauthCredentialFilePayload[];
}

export interface ImportedOauthMatchSummary {
  accountId: number;
  displayName: string;
  groupName?: string | null;
  status: string;
}

export interface ImportedOauthValidationRow {
  sourceId: string;
  fileName: string;
  email?: string | null;
  chatgptAccountId?: string | null;
  displayName?: string | null;
  tokenExpiresAt?: string | null;
  matchedAccount?: ImportedOauthMatchSummary | null;
  status: "pending" | "duplicate_in_input" | "ok" | "ok_exhausted" | "invalid" | "error" | string;
  detail?: string | null;
  attempts: number;
}

export interface ImportedOauthValidationResponse {
  inputFiles: number;
  uniqueInInput: number;
  duplicateInInput: number;
  rows: ImportedOauthValidationRow[];
}

export interface ImportedOauthValidationCounts {
  pending: number;
  duplicateInInput: number;
  ok: number;
  okExhausted: number;
  invalid: number;
  error: number;
  checked: number;
}

export interface ImportedOauthValidationJobResponse {
  jobId: string;
  snapshot: ImportedOauthValidationResponse;
}

export interface ImportedOauthValidationSnapshotEventPayload {
  snapshot: ImportedOauthValidationResponse;
  counts: ImportedOauthValidationCounts;
}

export interface ImportedOauthValidationRowEventPayload {
  row: ImportedOauthValidationRow;
  counts: ImportedOauthValidationCounts;
}

export interface ImportedOauthValidationFailedEventPayload {
  snapshot: ImportedOauthValidationResponse;
  counts: ImportedOauthValidationCounts;
  error: string;
}

export interface ImportValidatedOauthAccountsPayload {
  items: ImportOauthCredentialFilePayload[];
  selectedSourceIds: string[];
  validationJobId?: string;
  groupName?: string;
  groupBoundProxyKeys?: string[];
  groupNodeShuntEnabled?: boolean;
  groupSingleAccountRotationEnabled?: boolean;
  groupNote?: string;
  concurrencyLimit?: number;
  tagIds?: number[];
}

export interface ImportedOauthImportResult {
  sourceId: string;
  fileName: string;
  email?: string | null;
  chatgptAccountId?: string | null;
  accountId?: number | null;
  status: "created" | "updated_existing" | "failed" | string;
  detail?: string | null;
  matchedAccount?: ImportedOauthMatchSummary | null;
}

export interface ImportedOauthImportSummary {
  inputFiles: number;
  selectedFiles: number;
  created: number;
  updatedExisting: number;
  failed: number;
}

export interface ImportedOauthImportResponse {
  summary: ImportedOauthImportSummary;
  results: ImportedOauthImportResult[];
}

export interface CreateTagPayload extends TagRoutingRule {
  name: string;
}

export type UpdateTagPayload = Partial<CreateTagPayload>;

export interface FetchTagsQuery {
  search?: string;
  hasAccounts?: boolean;
  allowCutIn?: boolean;
  allowCutOut?: boolean;
}

export interface UpdateUpstreamAccountGroupPayload {
  note?: string;
  boundProxyKeys?: string[];
  concurrencyLimit?: number;
  nodeShuntEnabled?: boolean;
  singleAccountRotationEnabled?: boolean;
  upstream429RetryEnabled?: boolean;
  upstream429MaxRetries?: number;
  routingRule?: UpdateGroupAccountRoutingRulePayload;
}

export type NullableRoutingRuleValue<T> = T | null;

export interface UpdateGroupAccountRoutingRulePayload {
  allowCutOut?: NullableRoutingRuleValue<boolean>;
  allowCutIn?: NullableRoutingRuleValue<boolean>;
  priorityTier?: NullableRoutingRuleValue<TagPriorityTier>;
  fastModeRewriteMode?: NullableRoutingRuleValue<TagFastModeRewriteMode>;
  imageToolRewriteMode?: NullableRoutingRuleValue<ImageToolRewriteMode>;
  concurrencyLimit?: NullableRoutingRuleValue<number>;
  upstream429RetryEnabled?: NullableRoutingRuleValue<boolean>;
  upstream429MaxRetries?: NullableRoutingRuleValue<number>;
  availableModels?: NullableRoutingRuleValue<string[]>;
  statusChangeReasons?: Partial<Record<StatusChangeReasonCode, NullableRoutingRuleValue<boolean>>>;
  timeouts?: {
    responsesFirstByteTimeoutSecs?: NullableRoutingRuleValue<number>;
    compactFirstByteTimeoutSecs?: NullableRoutingRuleValue<number>;
    responsesStreamTimeoutSecs?: NullableRoutingRuleValue<number>;
    compactStreamTimeoutSecs?: NullableRoutingRuleValue<number>;
  };
}

function normalizeRateWindowActualUsage(raw: unknown): RateWindowActualUsage | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const requestCount = normalizeFiniteNumber(payload.requestCount);
  const totalTokens = normalizeFiniteNumber(payload.totalTokens);
  const totalCost = normalizeFiniteNumber(payload.totalCost);
  const inputTokens = normalizeFiniteNumber(payload.inputTokens);
  const outputTokens = normalizeFiniteNumber(payload.outputTokens);
  const cacheInputTokens = normalizeFiniteNumber(payload.cacheInputTokens);
  if (
    requestCount == null ||
    totalTokens == null ||
    totalCost == null ||
    inputTokens == null ||
    outputTokens == null ||
    cacheInputTokens == null
  ) {
    return null;
  }
  return {
    requestCount,
    totalTokens,
    totalCost,
    inputTokens,
    outputTokens,
    cacheInputTokens,
  };
}

function normalizeRateWindowSnapshot(raw: unknown): RateWindowSnapshot | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const usedPercent = normalizeFiniteNumber(payload.usedPercent);
  const usedText = typeof payload.usedText === "string" ? payload.usedText : "";
  const limitText = typeof payload.limitText === "string" ? payload.limitText : "";
  const windowDurationMins = normalizeFiniteNumber(payload.windowDurationMins);
  if (usedPercent == null || !usedText || !limitText || windowDurationMins == null) return null;
  return {
    usedPercent,
    usedText,
    limitText,
    resetsAt: typeof payload.resetsAt === "string" ? payload.resetsAt : null,
    windowDurationMins,
    actualUsage: normalizeRateWindowActualUsage(payload.actualUsage),
  };
}

function normalizeCreditsSnapshot(raw: unknown): CreditsSnapshot | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  if (typeof payload.hasCredits !== "boolean" || typeof payload.unlimited !== "boolean")
    return null;
  return {
    hasCredits: payload.hasCredits,
    unlimited: payload.unlimited,
    balance: typeof payload.balance === "string" ? payload.balance : null,
  };
}

function normalizeLocalLimitSnapshot(raw: unknown): LocalLimitSnapshot | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const limitUnit =
    typeof payload.limitUnit === "string" && payload.limitUnit.trim()
      ? payload.limitUnit
      : "requests";
  return {
    primaryLimit: normalizeFiniteNumber(payload.primaryLimit) ?? null,
    secondaryLimit: normalizeFiniteNumber(payload.secondaryLimit) ?? null,
    limitUnit,
  };
}

function normalizeTagRoutingRule(raw: unknown): TagRoutingRule {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const concurrencyLimit = normalizeFiniteNumber(payload.concurrencyLimit);
  const upstream429MaxRetries = normalizeUpstreamAccountGroupMaxRetries(
    payload.upstream429MaxRetries,
  );
  return {
    allowCutOut: payload.allowCutOut !== false,
    allowCutIn: payload.allowCutIn !== false,
    priorityTier:
      payload.priorityTier === "primary" ||
      payload.priorityTier === "fallback" ||
      payload.priorityTier === "no_new"
        ? payload.priorityTier
        : "normal",
    fastModeRewriteMode:
      payload.fastModeRewriteMode === "force_remove" ||
      payload.fastModeRewriteMode === "fill_missing" ||
      payload.fastModeRewriteMode === "force_add"
        ? payload.fastModeRewriteMode
        : "keep_original",
    concurrencyLimit:
      concurrencyLimit != null && concurrencyLimit >= 0 ? Math.min(concurrencyLimit, 30) : 0,
    upstream429RetryEnabled: payload.upstream429RetryEnabled === true,
    upstream429MaxRetries,
    availableModels: normalizeStringArray(payload.availableModels)
      .map((value) => value.trim())
      .filter((value) => value.length > 0),
    availableModelsDefined:
      payload.availableModelsDefined === true || Array.isArray(payload.availableModels),
  };
}

function normalizeImageToolRewriteMode(raw: unknown): ImageToolRewriteMode {
  return raw === "fill_missing" || raw === "force_add" || raw === "force_remove"
    ? raw
    : "keep_original";
}

function normalizeImageToolCapability(raw: unknown): ImageToolCapability {
  return raw === "supported" || raw === "unsupported" ? raw : "unknown";
}

function normalizePoolRoutingTimeoutSettings(raw: unknown): PoolRoutingTimeoutSettings {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    responsesFirstByteTimeoutSecs:
      normalizeFiniteNumber(payload.responsesFirstByteTimeoutSecs) ?? 120,
    compactFirstByteTimeoutSecs:
      normalizeFiniteNumber(payload.compactFirstByteTimeoutSecs) ??
      normalizeFiniteNumber(payload.compactUpstreamHandshakeTimeoutSecs) ??
      300,
    responsesStreamTimeoutSecs: normalizeFiniteNumber(payload.responsesStreamTimeoutSecs) ?? 300,
    compactStreamTimeoutSecs: normalizeFiniteNumber(payload.compactStreamTimeoutSecs) ?? 300,
  };
}

function normalizeOptionalPoolRoutingTimeoutSettings(
  raw: unknown,
): Partial<PoolRoutingTimeoutSettings> | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const payload = raw as Record<string, unknown>;
  const next: Partial<PoolRoutingTimeoutSettings> = {};
  const responsesFirstByteTimeoutSecs = normalizeFiniteNumber(
    payload.responsesFirstByteTimeoutSecs,
  );
  const compactFirstByteTimeoutSecs = normalizeFiniteNumber(payload.compactFirstByteTimeoutSecs);
  const responsesStreamTimeoutSecs = normalizeFiniteNumber(payload.responsesStreamTimeoutSecs);
  const compactStreamTimeoutSecs = normalizeFiniteNumber(payload.compactStreamTimeoutSecs);
  if (responsesFirstByteTimeoutSecs != null) {
    next.responsesFirstByteTimeoutSecs = responsesFirstByteTimeoutSecs;
  }
  if (compactFirstByteTimeoutSecs != null) {
    next.compactFirstByteTimeoutSecs = compactFirstByteTimeoutSecs;
  }
  if (responsesStreamTimeoutSecs != null) {
    next.responsesStreamTimeoutSecs = responsesStreamTimeoutSecs;
  }
  if (compactStreamTimeoutSecs != null) {
    next.compactStreamTimeoutSecs = compactStreamTimeoutSecs;
  }
  return Object.keys(next).length > 0 ? next : undefined;
}

function normalizeRoutingTimeoutFieldSources(raw: unknown): EffectiveRoutingTimeoutFieldSources {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const normalizeSource = (value: unknown): EffectiveRoutingRuleSource =>
    typeof value === "string" && value.trim() ? value : "root";
  return {
    responsesFirstByteTimeoutSecs: normalizeSource(payload.responsesFirstByteTimeoutSecs),
    compactFirstByteTimeoutSecs: normalizeSource(payload.compactFirstByteTimeoutSecs),
    responsesStreamTimeoutSecs: normalizeSource(payload.responsesStreamTimeoutSecs),
    compactStreamTimeoutSecs: normalizeSource(payload.compactStreamTimeoutSecs),
  };
}

function normalizeStatusChangeReasons(raw: unknown): StatusChangeReasons {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const next = buildDefaultStatusChangeReasons();
  for (const reason of STATUS_CHANGE_REASON_CODES) {
    if (typeof payload[reason] === "boolean") {
      next[reason] = payload[reason] === true;
    }
  }
  return next;
}

function normalizeStatusChangeReasonFieldSources(raw: unknown): StatusChangeReasonFieldSources {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const next = buildDefaultStatusChangeReasonFieldSources();
  for (const reason of STATUS_CHANGE_REASON_CODES) {
    if (typeof payload[reason] === "string" && payload[reason].trim()) {
      next[reason] = payload[reason] as EffectiveRoutingRuleSource;
    }
  }
  return next;
}

function normalizeGroupAccountRoutingRule(raw: unknown): GroupAccountRoutingRule {
  const payload = normalizeTagRoutingRule(raw);
  const rawPayload = (raw ?? {}) as Record<string, unknown>;
  return {
    ...payload,
    imageToolRewriteMode: normalizeImageToolRewriteMode(rawPayload.imageToolRewriteMode),
    statusChangeReasons: normalizeStatusChangeReasons(rawPayload.statusChangeReasons),
    timeouts: normalizeOptionalPoolRoutingTimeoutSettings(rawPayload.timeouts),
  };
}

function normalizeAccountTagSummary(raw: unknown): AccountTagSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const id = normalizeFiniteNumber(payload.id);
  const name = typeof payload.name === "string" ? payload.name : "";
  if (id == null || !name) return null;
  return {
    id,
    name,
    routingRule: normalizeTagRoutingRule(payload.routingRule),
    systemKey: typeof payload.systemKey === "string" ? payload.systemKey : null,
    protected: payload.protected === true,
  };
}

export function normalizeEffectiveRoutingRule(raw: unknown): EffectiveRoutingRule {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const rawSources = (payload.fieldSources ?? {}) as Record<string, unknown>;
  const rawTimeoutFieldSources = (payload.timeoutFieldSources ?? {}) as Record<string, unknown>;
  const normalizeSource = (value: unknown): EffectiveRoutingRuleSource =>
    typeof value === "string" && value.trim() ? value : "root";
  const sourceTagIds = Array.isArray(payload.sourceTagIds)
    ? payload.sourceTagIds
        .map(normalizeFiniteNumber)
        .filter((value): value is number => value != null)
    : [];
  const sourceTagNames = Array.isArray(payload.sourceTagNames)
    ? payload.sourceTagNames.filter((value): value is string => typeof value === "string")
    : [];
  return {
    ...normalizeGroupAccountRoutingRule(payload),
    systemDeniedModels: normalizeStringArray(payload.systemDeniedModels)
      .map((value) => value.trim())
      .filter((value) => value.length > 0),
    sourceTagIds,
    sourceTagNames,
    timeouts: normalizePoolRoutingTimeoutSettings(payload.timeouts),
    fieldSources: {
      allowCutOut: normalizeSource(rawSources.allowCutOut),
      allowCutIn: normalizeSource(rawSources.allowCutIn),
      priorityTier: normalizeSource(rawSources.priorityTier),
      fastModeRewriteMode: normalizeSource(rawSources.fastModeRewriteMode),
      imageToolRewriteMode: normalizeSource(rawSources.imageToolRewriteMode),
      concurrencyLimit: normalizeSource(rawSources.concurrencyLimit),
      upstream429Retry: normalizeSource(rawSources.upstream429Retry),
      availableModels: normalizeSource(rawSources.availableModels),
      systemDeniedModels: normalizeSource(rawSources.systemDeniedModels),
    },
    statusChangeReasonFieldSources: normalizeStatusChangeReasonFieldSources(
      payload.statusChangeReasonFieldSources,
    ),
    timeoutFieldSources: {
      responsesFirstByteTimeoutSecs: normalizeSource(
        rawTimeoutFieldSources.responsesFirstByteTimeoutSecs,
      ),
      compactFirstByteTimeoutSecs: normalizeSource(
        rawTimeoutFieldSources.compactFirstByteTimeoutSecs,
      ),
      responsesStreamTimeoutSecs: normalizeSource(
        rawTimeoutFieldSources.responsesStreamTimeoutSecs,
      ),
      compactStreamTimeoutSecs: normalizeSource(rawTimeoutFieldSources.compactStreamTimeoutSecs),
    },
  };
}

function normalizeTagSummary(raw: unknown): TagSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const id = normalizeFiniteNumber(payload.id);
  const name = typeof payload.name === "string" ? payload.name : "";
  const accountCount = normalizeFiniteNumber(payload.accountCount);
  const groupCount = normalizeFiniteNumber(payload.groupCount);
  const updatedAt = typeof payload.updatedAt === "string" ? payload.updatedAt : "";
  if (id == null || !name || accountCount == null || groupCount == null || !updatedAt) return null;
  return {
    id,
    name,
    routingRule: normalizeTagRoutingRule(payload.routingRule),
    accountCount,
    groupCount,
    updatedAt,
    systemKey: typeof payload.systemKey === "string" ? payload.systemKey : null,
    protected: payload.protected === true,
  };
}

function normalizeUpstreamAccountSummary(raw: unknown): UpstreamAccountSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const id = normalizeFiniteNumber(payload.id);
  const displayName = typeof payload.displayName === "string" ? payload.displayName : "";
  const kind = typeof payload.kind === "string" ? payload.kind : "";
  const provider = typeof payload.provider === "string" ? payload.provider : "";
  const status = typeof payload.status === "string" ? payload.status : "error";
  const displayStatus = typeof payload.displayStatus === "string" ? payload.displayStatus : status;
  const enableStatus =
    typeof payload.enableStatus === "string"
      ? payload.enableStatus
      : payload.enabled === false || displayStatus === "disabled"
        ? "disabled"
        : "enabled";
  const syncState =
    typeof payload.syncState === "string"
      ? payload.syncState
      : status === "syncing" || displayStatus === "syncing"
        ? "syncing"
        : "idle";
  const healthStatus =
    typeof payload.healthStatus === "string"
      ? payload.healthStatus
      : displayStatus === "needs_reauth" ||
          displayStatus === "upstream_unavailable" ||
          displayStatus === "upstream_rejected" ||
          displayStatus === "error_other"
        ? displayStatus
        : status === "needs_reauth"
          ? "needs_reauth"
          : status === "error"
            ? "error_other"
            : "normal";
  const workStatus =
    typeof payload.workStatus === "string"
      ? payload.workStatus
      : enableStatus !== "enabled" || syncState === "syncing"
        ? "idle"
        : healthStatus !== "normal"
          ? "unavailable"
          : "idle";
  if (id == null || !displayName || !kind || !provider) return null;
  return {
    id,
    kind,
    provider,
    displayName,
    groupName: typeof payload.groupName === "string" ? payload.groupName : null,
    isMother: payload.isMother === true,
    status,
    workStatus,
    enableStatus,
    healthStatus,
    syncState,
    displayStatus,
    enabled: payload.enabled !== false,
    email: typeof payload.email === "string" ? payload.email : null,
    chatgptAccountId:
      typeof payload.chatgptAccountId === "string" ? payload.chatgptAccountId : null,
    planType: typeof payload.planType === "string" ? payload.planType : null,
    maskedApiKey: typeof payload.maskedApiKey === "string" ? payload.maskedApiKey : null,
    hasRefreshToken: typeof payload.hasRefreshToken === "boolean" ? payload.hasRefreshToken : true,
    lastSyncedAt: typeof payload.lastSyncedAt === "string" ? payload.lastSyncedAt : null,
    lastSuccessfulSyncAt:
      typeof payload.lastSuccessfulSyncAt === "string" ? payload.lastSuccessfulSyncAt : null,
    lastActivityAt: typeof payload.lastActivityAt === "string" ? payload.lastActivityAt : null,
    activeConversationCount: normalizeFiniteNumber(payload.activeConversationCount) ?? 0,
    lastError: typeof payload.lastError === "string" ? payload.lastError : null,
    lastErrorAt: typeof payload.lastErrorAt === "string" ? payload.lastErrorAt : null,
    lastAction: typeof payload.lastAction === "string" ? payload.lastAction : null,
    lastActionSource:
      typeof payload.lastActionSource === "string" ? payload.lastActionSource : null,
    lastActionReasonCode:
      typeof payload.lastActionReasonCode === "string" ? payload.lastActionReasonCode : null,
    lastActionReasonMessage:
      typeof payload.lastActionReasonMessage === "string" ? payload.lastActionReasonMessage : null,
    routingBlockReasonCode:
      typeof payload.routingBlockReasonCode === "string" ? payload.routingBlockReasonCode : null,
    routingBlockReasonMessage:
      typeof payload.routingBlockReasonMessage === "string"
        ? payload.routingBlockReasonMessage
        : null,
    lastActionHttpStatus: normalizeFiniteNumber(payload.lastActionHttpStatus) ?? null,
    lastActionInvokeId:
      typeof payload.lastActionInvokeId === "string" ? payload.lastActionInvokeId : null,
    lastActionAt: typeof payload.lastActionAt === "string" ? payload.lastActionAt : null,
    cooldownUntil: typeof payload.cooldownUntil === "string" ? payload.cooldownUntil : null,
    boundProxyKeys: normalizeStringArray(payload.boundProxyKeys)
      .map((item) => item.trim())
      .filter((item) => item.length > 0),
    currentForwardProxyKey:
      typeof payload.currentForwardProxyKey === "string" ? payload.currentForwardProxyKey : null,
    currentForwardProxyDisplayName:
      typeof payload.currentForwardProxyDisplayName === "string"
        ? payload.currentForwardProxyDisplayName
        : null,
    currentForwardProxyState:
      typeof payload.currentForwardProxyState === "string"
        ? payload.currentForwardProxyState
        : undefined,
    tokenExpiresAt: typeof payload.tokenExpiresAt === "string" ? payload.tokenExpiresAt : null,
    primaryWindow: normalizeRateWindowSnapshot(payload.primaryWindow),
    secondaryWindow: normalizeRateWindowSnapshot(payload.secondaryWindow),
    credits: normalizeCreditsSnapshot(payload.credits),
    localLimits: normalizeLocalLimitSnapshot(payload.localLimits),
    compactSupport: normalizeCompactSupportState(payload.compactSupport),
    duplicateInfo: normalizeUpstreamAccountDuplicateInfo(payload.duplicateInfo),
    imageToolCapability: normalizeImageToolCapability(payload.imageToolCapability),
    tags: Array.isArray(payload.tags)
      ? payload.tags
          .map(normalizeAccountTagSummary)
          .filter((item): item is AccountTagSummary => item != null)
      : [],
    effectiveRoutingRule: normalizeEffectiveRoutingRule(payload.effectiveRoutingRule),
  };
}

function normalizeUpstreamAccountListMetrics(raw: unknown): UpstreamAccountListMetrics {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    total: normalizeFiniteNumber(payload.total) ?? 0,
    oauth: normalizeFiniteNumber(payload.oauth) ?? 0,
    apiKey: normalizeFiniteNumber(payload.apiKey) ?? 0,
    attention: normalizeFiniteNumber(payload.attention) ?? 0,
  };
}

function normalizeUpstreamAccountDuplicateInfo(raw: unknown): UpstreamAccountDuplicateInfo | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const peerAccountIds = Array.isArray(payload.peerAccountIds)
    ? payload.peerAccountIds
        .map((value) => normalizeFiniteNumber(value))
        .filter((value): value is number => value != null)
    : [];
  const reasons = Array.isArray(payload.reasons)
    ? payload.reasons.filter(
        (value): value is string => typeof value === "string" && value.trim().length > 0,
      )
    : [];
  if (peerAccountIds.length === 0 || reasons.length === 0) return null;
  return {
    peerAccountIds,
    reasons,
  };
}

function normalizeUpstreamAccountHistoryPoint(raw: unknown): UpstreamAccountHistoryPoint | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const capturedAt = typeof payload.capturedAt === "string" ? payload.capturedAt : "";
  if (!capturedAt) return null;
  return {
    capturedAt,
    primaryUsedPercent: normalizeFiniteNumber(payload.primaryUsedPercent) ?? null,
    secondaryUsedPercent: normalizeFiniteNumber(payload.secondaryUsedPercent) ?? null,
    creditsBalance: typeof payload.creditsBalance === "string" ? payload.creditsBalance : null,
  };
}

function normalizeUpstreamAccountActionEvent(raw: unknown): UpstreamAccountActionEvent | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const id = normalizeFiniteNumber(payload.id);
  const occurredAt = typeof payload.occurredAt === "string" ? payload.occurredAt : "";
  const action = typeof payload.action === "string" ? payload.action : "";
  const source = typeof payload.source === "string" ? payload.source : "";
  const createdAt = typeof payload.createdAt === "string" ? payload.createdAt : "";
  if (id == null || !occurredAt || !action || !source || !createdAt) {
    return null;
  }
  return {
    id,
    occurredAt,
    action,
    source,
    accountDisplayName:
      typeof payload.accountDisplayName === "string" ? payload.accountDisplayName : null,
    accountGroupName:
      typeof payload.accountGroupName === "string" ? payload.accountGroupName : null,
    forwardProxyKey: typeof payload.forwardProxyKey === "string" ? payload.forwardProxyKey : null,
    forwardProxyDisplayName:
      typeof payload.forwardProxyDisplayName === "string" ? payload.forwardProxyDisplayName : null,
    forwardProxyEgressIp:
      typeof payload.forwardProxyEgressIp === "string" ? payload.forwardProxyEgressIp : null,
    result: typeof payload.result === "string" ? payload.result : null,
    resultDescription:
      typeof payload.resultDescription === "string" ? payload.resultDescription : null,
    reasonCode: typeof payload.reasonCode === "string" ? payload.reasonCode : null,
    reasonMessage: typeof payload.reasonMessage === "string" ? payload.reasonMessage : null,
    httpStatus: normalizeFiniteNumber(payload.httpStatus) ?? null,
    failureKind: typeof payload.failureKind === "string" ? payload.failureKind : null,
    invokeId: typeof payload.invokeId === "string" ? payload.invokeId : null,
    attemptId: normalizeFiniteNumber(payload.attemptId) ?? null,
    stickyKey: typeof payload.stickyKey === "string" ? payload.stickyKey : null,
    createdAt,
  };
}

function normalizeUpstreamAccountDetail(raw: unknown): UpstreamAccountDetail {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const summary = normalizeUpstreamAccountSummary(payload);
  if (!summary) {
    throw new Error("Request failed: invalid upstream account payload");
  }
  const historyRaw = Array.isArray(payload.history) ? payload.history : [];
  return {
    ...summary,
    note: typeof payload.note === "string" ? payload.note : null,
    verifiedEmail: typeof payload.verifiedEmail === "string" ? payload.verifiedEmail : null,
    upstreamBaseUrl: typeof payload.upstreamBaseUrl === "string" ? payload.upstreamBaseUrl : null,
    chatgptUserId: typeof payload.chatgptUserId === "string" ? payload.chatgptUserId : null,
    lastRefreshedAt: typeof payload.lastRefreshedAt === "string" ? payload.lastRefreshedAt : null,
    history: historyRaw
      .map(normalizeUpstreamAccountHistoryPoint)
      .filter((item): item is UpstreamAccountHistoryPoint => item != null),
    recentActions: Array.isArray(payload.recentActions)
      ? payload.recentActions
          .map(normalizeUpstreamAccountActionEvent)
          .filter((item): item is UpstreamAccountActionEvent => item != null)
      : [],
  };
}

function normalizeUpstreamAccountGroupMaxRetries(raw: unknown): number {
  const value = normalizeFiniteNumber(raw);
  if (value == null) return 0;
  return Math.min(5, Math.max(0, Math.trunc(value)));
}

function normalizeUpstreamAccountGroupSummary(raw: unknown): UpstreamAccountGroupSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const groupName = typeof payload.groupName === "string" ? payload.groupName.trim() : "";
  if (!groupName) return null;
  return {
    groupName,
    accountCount: (() => {
      const value = normalizeFiniteNumber(payload.accountCount);
      return value != null && value >= 0 ? Math.trunc(value) : 0;
    })(),
    note: typeof payload.note === "string" ? payload.note : null,
    boundProxyKeys: normalizeStringArray(payload.boundProxyKeys)
      .map((item) => item.trim())
      .filter((item) => item.length > 0),
    concurrencyLimit: (() => {
      const value = normalizeFiniteNumber(payload.concurrencyLimit);
      return value != null && value >= 0 ? Math.min(value, 30) : 0;
    })(),
    nodeShuntEnabled: payload.nodeShuntEnabled === true,
    singleAccountRotationEnabled: payload.singleAccountRotationEnabled === true,
    upstream429RetryEnabled: payload.upstream429RetryEnabled === true,
    upstream429MaxRetries: normalizeUpstreamAccountGroupMaxRetries(payload.upstream429MaxRetries),
    routingRule: normalizeGroupAccountRoutingRule(payload.routingRule),
    effectiveTimeouts: normalizePoolRoutingTimeoutSettings(payload.effectiveTimeouts),
    timeoutFieldSources: normalizeRoutingTimeoutFieldSources(payload.timeoutFieldSources),
  };
}

function normalizeUpstreamAccountListResponse(raw: unknown): UpstreamAccountListResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const itemsRaw = Array.isArray(payload.items) ? payload.items : [];
  const groupsRaw = Array.isArray(payload.groups) ? payload.groups : [];
  const total = normalizeFiniteNumber(payload.total) ?? 0;
  const page = normalizeFiniteNumber(payload.page) ?? 1;
  const pageSize = normalizeFiniteNumber(payload.pageSize) ?? 20;
  return {
    writesEnabled: payload.writesEnabled !== false,
    items: itemsRaw
      .map(normalizeUpstreamAccountSummary)
      .filter((item): item is UpstreamAccountSummary => item != null),
    groups: groupsRaw
      .map(normalizeUpstreamAccountGroupSummary)
      .filter((item): item is UpstreamAccountGroupSummary => item != null),
    forwardProxyNodes: Array.isArray(payload.forwardProxyNodes)
      ? payload.forwardProxyNodes
          .map(normalizeForwardProxyBindingNode)
          .filter((item): item is ForwardProxyBindingNode => item != null)
      : [],
    hasUngroupedAccounts: payload.hasUngroupedAccounts === true,
    total,
    page,
    pageSize,
    metrics: normalizeUpstreamAccountListMetrics(payload.metrics),
    routing: normalizePoolRoutingSettings(payload.routing),
  };
}

function normalizeUpstreamAccountActionEventListResponse(
  raw: unknown,
): UpstreamAccountActionEventListResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const itemsRaw = Array.isArray(payload.items) ? payload.items : [];
  return {
    items: itemsRaw
      .map(normalizeUpstreamAccountActionEvent)
      .filter((item): item is UpstreamAccountActionEvent => item != null),
    total: normalizeFiniteNumber(payload.total) ?? 0,
    page: normalizeFiniteNumber(payload.page) ?? 1,
    pageSize: normalizeFiniteNumber(payload.pageSize) ?? 20,
  };
}

function normalizeUpstreamAccountWindowUsageResponse(
  raw: unknown,
): UpstreamAccountWindowUsageResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const itemsRaw = Array.isArray(payload.items) ? payload.items : [];
  return {
    items: itemsRaw
      .map((item) => {
        const entry = (item ?? {}) as Record<string, unknown>;
        const accountId = normalizeFiniteNumber(entry.accountId);
        if (accountId == null) return null;
        const normalized: UpstreamAccountWindowUsageItem = {
          accountId,
          primaryActualUsage: normalizeRateWindowActualUsage(entry.primaryActualUsage),
          secondaryActualUsage: normalizeRateWindowActualUsage(entry.secondaryActualUsage),
        };
        return normalized;
      })
      .filter((item): item is UpstreamAccountWindowUsageItem => item != null),
  };
}

function normalizeTagListResponse(raw: unknown): TagListResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const itemsRaw = Array.isArray(payload.items) ? payload.items : [];
  return {
    writesEnabled: payload.writesEnabled !== false,
    items: itemsRaw.map(normalizeTagSummary).filter((item): item is TagSummary => item != null),
  };
}

function normalizeLoginSessionStatusResponse(raw: unknown): LoginSessionStatusResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const loginId = typeof payload.loginId === "string" ? payload.loginId : "";
  const expiresAt = typeof payload.expiresAt === "string" ? payload.expiresAt : "";
  if (!loginId || !expiresAt) {
    throw new Error("Request failed: invalid login session payload");
  }
  const accountId = normalizeFiniteNumber(payload.accountId);
  return {
    loginId,
    status: typeof payload.status === "string" ? payload.status : "failed",
    authUrl: typeof payload.authUrl === "string" ? payload.authUrl : null,
    redirectUri: typeof payload.redirectUri === "string" ? payload.redirectUri : null,
    expiresAt,
    updatedAt: typeof payload.updatedAt === "string" ? payload.updatedAt : null,
    accountId: accountId == null ? null : accountId,
    email: typeof payload.email === "string" ? payload.email : null,
    error: typeof payload.error === "string" ? payload.error : null,
    syncApplied: typeof payload.syncApplied === "boolean" ? payload.syncApplied : null,
    identityConfirmation: normalizeOauthIdentityConfirmation(payload.identityConfirmation),
  };
}

function normalizeOauthIdentityConfirmation(raw: unknown): OauthIdentityConfirmation | null {
  const payload = raw as Record<string, unknown> | null | undefined;
  if (!payload || typeof payload !== "object") return null;
  return {
    current: normalizeOauthIdentitySummary(payload.current),
    incoming: normalizeOauthIdentitySummary(payload.incoming),
  };
}

function normalizeOauthIdentitySummary(raw: unknown): OauthIdentitySummary {
  const payload = raw as Record<string, unknown> | null | undefined;
  if (!payload || typeof payload !== "object") return {};
  const accountId = normalizeFiniteNumber(payload.accountId);
  return {
    accountId: accountId == null ? null : accountId,
    displayName: typeof payload.displayName === "string" ? payload.displayName : null,
    email: typeof payload.email === "string" ? payload.email : null,
    verifiedEmail: typeof payload.verifiedEmail === "string" ? payload.verifiedEmail : null,
    chatgptAccountId:
      typeof payload.chatgptAccountId === "string" ? payload.chatgptAccountId : null,
    chatgptUserId: typeof payload.chatgptUserId === "string" ? payload.chatgptUserId : null,
    planType: typeof payload.planType === "string" ? payload.planType : null,
  };
}

function normalizeImportedOauthMatchSummary(raw: unknown): ImportedOauthMatchSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const accountId = normalizeFiniteNumber(payload.accountId);
  const displayName = typeof payload.displayName === "string" ? payload.displayName : "";
  const status = typeof payload.status === "string" ? payload.status : "";
  if (accountId == null || !displayName || !status) return null;
  return {
    accountId,
    displayName,
    groupName: typeof payload.groupName === "string" ? payload.groupName : null,
    status,
  };
}

function normalizeImportedOauthValidationRow(raw: unknown): ImportedOauthValidationRow | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const sourceId = typeof payload.sourceId === "string" ? payload.sourceId : "";
  const fileName = typeof payload.fileName === "string" ? payload.fileName : "";
  const status = typeof payload.status === "string" ? payload.status : "";
  const attempts = normalizeFiniteNumber(payload.attempts);
  if (!sourceId || !fileName || !status || attempts == null) return null;
  return {
    sourceId,
    fileName,
    email: typeof payload.email === "string" ? payload.email : null,
    chatgptAccountId:
      typeof payload.chatgptAccountId === "string" ? payload.chatgptAccountId : null,
    displayName: typeof payload.displayName === "string" ? payload.displayName : null,
    tokenExpiresAt: typeof payload.tokenExpiresAt === "string" ? payload.tokenExpiresAt : null,
    matchedAccount: normalizeImportedOauthMatchSummary(payload.matchedAccount),
    status,
    detail: typeof payload.detail === "string" ? payload.detail : null,
    attempts,
  };
}

function normalizeImportedOauthValidationResponse(raw: unknown): ImportedOauthValidationResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const inputFiles = normalizeFiniteNumber(payload.inputFiles);
  const uniqueInInput = normalizeFiniteNumber(payload.uniqueInInput);
  const duplicateInInput = normalizeFiniteNumber(payload.duplicateInInput);
  const rowsRaw = Array.isArray(payload.rows) ? payload.rows : [];
  if (inputFiles == null || uniqueInInput == null || duplicateInInput == null) {
    throw new Error("Request failed: invalid imported OAuth validation payload");
  }
  return {
    inputFiles,
    uniqueInInput,
    duplicateInInput,
    rows: rowsRaw
      .map(normalizeImportedOauthValidationRow)
      .filter((item): item is ImportedOauthValidationRow => item != null),
  };
}

function normalizeImportedOauthImportResult(raw: unknown): ImportedOauthImportResult | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const sourceId = typeof payload.sourceId === "string" ? payload.sourceId : "";
  const fileName = typeof payload.fileName === "string" ? payload.fileName : "";
  const status = typeof payload.status === "string" ? payload.status : "";
  if (!sourceId || !fileName || !status) return null;
  return {
    sourceId,
    fileName,
    email: typeof payload.email === "string" ? payload.email : null,
    chatgptAccountId:
      typeof payload.chatgptAccountId === "string" ? payload.chatgptAccountId : null,
    accountId: normalizeFiniteNumber(payload.accountId) ?? null,
    status,
    detail: typeof payload.detail === "string" ? payload.detail : null,
    matchedAccount: normalizeImportedOauthMatchSummary(payload.matchedAccount),
  };
}

function normalizeImportedOauthValidationCounts(raw: unknown): ImportedOauthValidationCounts {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const pending = normalizeFiniteNumber(payload.pending);
  const duplicateInInput = normalizeFiniteNumber(payload.duplicateInInput);
  const ok = normalizeFiniteNumber(payload.ok);
  const okExhausted = normalizeFiniteNumber(payload.okExhausted);
  const invalid = normalizeFiniteNumber(payload.invalid);
  const error = normalizeFiniteNumber(payload.error);
  const checked = normalizeFiniteNumber(payload.checked);
  if (
    pending == null ||
    duplicateInInput == null ||
    ok == null ||
    okExhausted == null ||
    invalid == null ||
    error == null ||
    checked == null
  ) {
    throw new Error("Request failed: invalid imported OAuth validation counts payload");
  }
  return {
    pending,
    duplicateInInput,
    ok,
    okExhausted,
    invalid,
    error,
    checked,
  };
}

function normalizeImportedOauthValidationJobResponse(
  raw: unknown,
): ImportedOauthValidationJobResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const jobId = typeof payload.jobId === "string" ? payload.jobId : "";
  if (!jobId) {
    throw new Error("Request failed: invalid imported OAuth validation job payload");
  }
  return {
    jobId,
    snapshot: normalizeImportedOauthValidationResponse(payload.snapshot),
  };
}

export function normalizeImportedOauthValidationSnapshotEventPayload(
  raw: unknown,
): ImportedOauthValidationSnapshotEventPayload {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    snapshot: normalizeImportedOauthValidationResponse(payload.snapshot),
    counts: normalizeImportedOauthValidationCounts(payload.counts),
  };
}

export function normalizeImportedOauthValidationRowEventPayload(
  raw: unknown,
): ImportedOauthValidationRowEventPayload {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const row = normalizeImportedOauthValidationRow(payload.row);
  if (!row) {
    throw new Error("Request failed: invalid imported OAuth validation row event payload");
  }
  return {
    row,
    counts: normalizeImportedOauthValidationCounts(payload.counts),
  };
}

export function normalizeImportedOauthValidationFailedEventPayload(
  raw: unknown,
): ImportedOauthValidationFailedEventPayload {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const error = typeof payload.error === "string" ? payload.error : "";
  if (!error) {
    throw new Error("Request failed: invalid imported OAuth validation failed event payload");
  }
  return {
    snapshot: normalizeImportedOauthValidationResponse(payload.snapshot),
    counts: normalizeImportedOauthValidationCounts(payload.counts),
    error,
  };
}

function normalizeBulkUpstreamAccountActionResult(
  raw: unknown,
): BulkUpstreamAccountActionResult | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const accountId = normalizeFiniteNumber(payload.accountId);
  const status = typeof payload.status === "string" ? payload.status : "";
  if (accountId == null || !status) return null;
  return {
    accountId,
    displayName: typeof payload.displayName === "string" ? payload.displayName : null,
    status,
    detail: typeof payload.detail === "string" ? payload.detail : null,
  };
}

function normalizeBulkUpstreamAccountSyncCounts(raw: unknown): BulkUpstreamAccountSyncCounts {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    total: normalizeFiniteNumber(payload.total) ?? 0,
    completed: normalizeFiniteNumber(payload.completed) ?? 0,
    succeeded: normalizeFiniteNumber(payload.succeeded) ?? 0,
    failed: normalizeFiniteNumber(payload.failed) ?? 0,
    skipped: normalizeFiniteNumber(payload.skipped) ?? 0,
  };
}

function normalizeBulkUpstreamAccountSyncRow(raw: unknown): BulkUpstreamAccountSyncRow | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const accountId = normalizeFiniteNumber(payload.accountId);
  const displayName = typeof payload.displayName === "string" ? payload.displayName : "";
  const status = typeof payload.status === "string" ? payload.status : "";
  if (accountId == null || !displayName || !status) return null;
  return {
    accountId,
    displayName,
    status,
    detail: typeof payload.detail === "string" ? payload.detail : null,
  };
}

function normalizeBulkUpstreamAccountSyncSnapshot(raw: unknown): BulkUpstreamAccountSyncSnapshot {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const jobId = typeof payload.jobId === "string" ? payload.jobId : "";
  const status = typeof payload.status === "string" ? payload.status : "";
  if (!jobId || !status) {
    throw new Error("Request failed: invalid bulk upstream account sync snapshot payload");
  }
  const rows = Array.isArray(payload.rows) ? payload.rows : [];
  return {
    jobId,
    status,
    rows: rows
      .map(normalizeBulkUpstreamAccountSyncRow)
      .filter((item): item is BulkUpstreamAccountSyncRow => item != null),
  };
}

function normalizeBulkUpstreamAccountActionResponse(
  raw: unknown,
): BulkUpstreamAccountActionResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const action = typeof payload.action === "string" ? payload.action : "";
  if (!action) {
    throw new Error("Request failed: invalid bulk upstream account action payload");
  }
  const results = Array.isArray(payload.results) ? payload.results : [];
  return {
    action,
    requestedCount: normalizeFiniteNumber(payload.requestedCount) ?? 0,
    completedCount: normalizeFiniteNumber(payload.completedCount) ?? 0,
    succeededCount: normalizeFiniteNumber(payload.succeededCount) ?? 0,
    failedCount: normalizeFiniteNumber(payload.failedCount) ?? 0,
    results: results
      .map(normalizeBulkUpstreamAccountActionResult)
      .filter((item): item is BulkUpstreamAccountActionResult => item != null),
  };
}

function normalizeBulkUpstreamAccountSyncJobResponse(
  raw: unknown,
): BulkUpstreamAccountSyncJobResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const jobId = typeof payload.jobId === "string" ? payload.jobId : "";
  if (!jobId) {
    throw new Error("Request failed: invalid bulk upstream account sync job payload");
  }
  return {
    jobId,
    snapshot: normalizeBulkUpstreamAccountSyncSnapshot(payload.snapshot),
    counts: normalizeBulkUpstreamAccountSyncCounts(payload.counts),
  };
}

export function normalizeBulkUpstreamAccountSyncSnapshotEventPayload(
  raw: unknown,
): BulkUpstreamAccountSyncSnapshotEventPayload {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    snapshot: normalizeBulkUpstreamAccountSyncSnapshot(payload.snapshot),
    counts: normalizeBulkUpstreamAccountSyncCounts(payload.counts),
  };
}

export function normalizeBulkUpstreamAccountSyncRowEventPayload(
  raw: unknown,
): BulkUpstreamAccountSyncRowEventPayload {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const row = normalizeBulkUpstreamAccountSyncRow(payload.row);
  if (!row) {
    throw new Error("Request failed: invalid bulk upstream account sync row payload");
  }
  return {
    row,
    counts: normalizeBulkUpstreamAccountSyncCounts(payload.counts),
  };
}

export function normalizeBulkUpstreamAccountSyncFailedEventPayload(
  raw: unknown,
): BulkUpstreamAccountSyncFailedEventPayload {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const error = typeof payload.error === "string" ? payload.error : "";
  if (!error) {
    throw new Error("Request failed: invalid bulk upstream account sync failed payload");
  }
  return {
    snapshot: normalizeBulkUpstreamAccountSyncSnapshot(payload.snapshot),
    counts: normalizeBulkUpstreamAccountSyncCounts(payload.counts),
    error,
  };
}

function normalizeImportedOauthImportResponse(raw: unknown): ImportedOauthImportResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const summaryPayload = (payload.summary ?? {}) as Record<string, unknown>;
  const inputFiles = normalizeFiniteNumber(summaryPayload.inputFiles);
  const selectedFiles = normalizeFiniteNumber(summaryPayload.selectedFiles);
  const created = normalizeFiniteNumber(summaryPayload.created);
  const updatedExisting = normalizeFiniteNumber(summaryPayload.updatedExisting);
  const failed = normalizeFiniteNumber(summaryPayload.failed);
  const resultsRaw = Array.isArray(payload.results) ? payload.results : [];
  if (
    inputFiles == null ||
    selectedFiles == null ||
    created == null ||
    updatedExisting == null ||
    failed == null
  ) {
    throw new Error("Request failed: invalid imported OAuth import payload");
  }
  return {
    summary: {
      inputFiles,
      selectedFiles,
      created,
      updatedExisting,
      failed,
    },
    results: resultsRaw
      .map(normalizeImportedOauthImportResult)
      .filter((item): item is ImportedOauthImportResult => item != null),
  };
}

function normalizeOauthMailboxSession(raw: unknown): OauthMailboxSession {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const supported = payload.supported !== false;
  const sessionId = typeof payload.sessionId === "string" ? payload.sessionId : "";
  const emailAddress = typeof payload.emailAddress === "string" ? payload.emailAddress : "";
  const expiresAt = typeof payload.expiresAt === "string" ? payload.expiresAt : "";
  if (!supported) {
    return {
      supported: false,
      emailAddress,
      reason:
        typeof payload.reason === "string" && payload.reason.trim()
          ? payload.reason
          : "not_readable",
    };
  }
  if (!emailAddress) {
    throw new Error("Request failed: invalid OAuth mailbox session payload");
  }
  if (!sessionId || !expiresAt) {
    throw new Error("Request failed: invalid OAuth mailbox session payload");
  }
  return {
    supported: true,
    sessionId,
    emailAddress,
    expiresAt,
    source:
      typeof payload.source === "string" && payload.source.trim() ? payload.source : "generated",
  };
}

function normalizeOauthMailboxCodeSummary(raw: unknown): OauthMailboxCodeSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const value = typeof payload.value === "string" ? payload.value : "";
  const source = typeof payload.source === "string" ? payload.source : "";
  const updatedAt = typeof payload.updatedAt === "string" ? payload.updatedAt : "";
  if (!value || !source || !updatedAt) return null;
  return { value, source, updatedAt };
}

function normalizeOauthInviteSummary(raw: unknown): OauthInviteSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const subject = typeof payload.subject === "string" ? payload.subject : "";
  const copyValue = typeof payload.copyValue === "string" ? payload.copyValue : "";
  const copyLabel = typeof payload.copyLabel === "string" ? payload.copyLabel : "";
  const updatedAt = typeof payload.updatedAt === "string" ? payload.updatedAt : "";
  if (!subject || !copyValue || !copyLabel || !updatedAt) return null;
  return {
    subject,
    copyValue,
    copyLabel,
    updatedAt,
  };
}

function normalizeOauthMailboxStatus(raw: unknown): OauthMailboxStatus | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const sessionId = typeof payload.sessionId === "string" ? payload.sessionId : "";
  const emailAddress = typeof payload.emailAddress === "string" ? payload.emailAddress : "";
  const expiresAt = typeof payload.expiresAt === "string" ? payload.expiresAt : "";
  if (!sessionId || !emailAddress || !expiresAt) return null;
  return {
    sessionId,
    emailAddress,
    expiresAt,
    latestCode: normalizeOauthMailboxCodeSummary(payload.latestCode),
    invite: normalizeOauthInviteSummary(payload.invite),
    invited: payload.invited === true,
    error: typeof payload.error === "string" && payload.error.trim() ? payload.error : null,
  };
}

export async function fetchUpstreamAccounts(
  query?: FetchUpstreamAccountsQuery,
): Promise<UpstreamAccountListResponse> {
  const search = new URLSearchParams();
  for (const groupExact of query?.groupExact ?? []) {
    if (groupExact) search.append("groupExact", groupExact);
  }
  if (query?.groupSearch) search.set("groupSearch", query.groupSearch);
  if (query?.groupUngrouped != null) search.set("groupUngrouped", String(query.groupUngrouped));
  if (query?.status) search.set("status", query.status);
  for (const workStatus of query?.workStatus ?? []) {
    if (workStatus) search.append("workStatus", workStatus);
  }
  for (const enableStatus of query?.enableStatus ?? []) {
    if (enableStatus) search.append("enableStatus", enableStatus);
  }
  for (const healthStatus of query?.healthStatus ?? []) {
    if (healthStatus) search.append("healthStatus", healthStatus);
  }
  if (query?.page != null) search.set("page", String(query.page));
  if (query?.pageSize != null) search.set("pageSize", String(query.pageSize));
  if (query?.includeAll != null) search.set("includeAll", String(query.includeAll));
  for (const tagId of query?.tagIds ?? []) {
    search.append("tagIds", String(tagId));
  }
  const response = await fetchJson<unknown>(
    search.size
      ? `/api/pool/upstream-accounts?${search.toString()}`
      : "/api/pool/upstream-accounts",
  );
  return normalizeUpstreamAccountListResponse(response);
}

export async function fetchUpstreamAccountActionEvents(
  query?: FetchUpstreamAccountActionEventsQuery,
): Promise<UpstreamAccountActionEventListResponse> {
  const search = new URLSearchParams();
  if (query?.account) search.set("account", query.account);
  if (query?.group) search.set("group", query.group);
  if (query?.proxyKey) search.set("proxyKey", query.proxyKey);
  if (query?.result) search.set("result", query.result);
  if (query?.page != null) search.set("page", String(query.page));
  if (query?.pageSize != null) search.set("pageSize", String(query.pageSize));
  const response = await fetchJson<unknown>(
    search.size
      ? `/api/pool/upstream-account-events?${search.toString()}`
      : "/api/pool/upstream-account-events",
  );
  return normalizeUpstreamAccountActionEventListResponse(response);
}

export async function fetchUpstreamAccountWindowUsage(
  accountIds: number[],
): Promise<UpstreamAccountWindowUsageResponse> {
  const normalizedAccountIds = Array.from(
    new Set(accountIds.filter((accountId) => Number.isFinite(accountId) && accountId > 0)),
  );
  if (normalizedAccountIds.length === 0) {
    return { items: [] };
  }
  const response = await fetchJson<unknown>("/api/pool/upstream-accounts/window-usage", {
    method: "POST",
    body: JSON.stringify({
      accountIds: normalizedAccountIds,
    }),
  });
  return normalizeUpstreamAccountWindowUsageResponse(response);
}

export async function fetchForwardProxyBindingNodes(
  keys?: string[],
  options?: { includeCurrent?: boolean; groupName?: string },
): Promise<ForwardProxyBindingNode[]> {
  const search = new URLSearchParams();
  if (options?.includeCurrent) {
    search.set("includeCurrent", "true");
  }
  const normalizedGroupName = options?.groupName?.trim();
  if (normalizedGroupName) {
    search.set("groupName", normalizedGroupName);
  }
  for (const key of keys ?? []) {
    const normalized = key.trim();
    if (!normalized) continue;
    search.append("key", normalized);
  }
  const response = await fetchJson<unknown>(
    search.size
      ? `/api/pool/forward-proxy-binding-nodes?${search.toString()}`
      : "/api/pool/forward-proxy-binding-nodes",
  );
  const items = Array.isArray(response) ? response : [];
  return items
    .map(normalizeForwardProxyBindingNode)
    .filter((item): item is ForwardProxyBindingNode => item != null);
}

export async function fetchTags(query?: FetchTagsQuery): Promise<TagListResponse> {
  const search = new URLSearchParams();
  if (query?.search) search.set("search", query.search);
  if (query?.hasAccounts != null) search.set("hasAccounts", String(query.hasAccounts));
  if (query?.allowCutIn != null) search.set("allowCutIn", String(query.allowCutIn));
  if (query?.allowCutOut != null) search.set("allowCutOut", String(query.allowCutOut));
  const response = await fetchJson<unknown>(
    search.size ? `/api/pool/tags?${search.toString()}` : "/api/pool/tags",
  );
  return normalizeTagListResponse(response);
}

export async function createTag(payload: CreateTagPayload): Promise<TagDetail> {
  const response = await fetchJson<unknown>("/api/pool/tags", {
    method: "POST",
    body: JSON.stringify(payload),
  });
  const normalized = normalizeTagSummary(response);
  if (!normalized) throw new Error("Request failed: invalid tag payload");
  return normalized;
}

export async function updateTag(tagId: number, payload: UpdateTagPayload): Promise<TagDetail> {
  const response = await fetchJson<unknown>(`/api/pool/tags/${tagId}`, {
    method: "PATCH",
    body: JSON.stringify(payload),
  });
  const normalized = normalizeTagSummary(response);
  if (!normalized) throw new Error("Request failed: invalid tag payload");
  return normalized;
}

export async function deleteTag(tagId: number): Promise<void> {
  await fetchJson(`/api/pool/tags/${tagId}`, { method: "DELETE" });
}

export async function updatePoolRoutingSettings(
  payload: UpdatePoolRoutingSettingsPayload,
): Promise<PoolRoutingSettings> {
  const response = await fetchJson<unknown>("/api/pool/routing-settings", {
    method: "PUT",
    body: JSON.stringify(payload),
  });
  const normalized = normalizePoolRoutingSettings(response);
  if (!normalized) {
    throw new Error("Request failed: invalid pool routing settings payload");
  }
  return normalized;
}

export async function fetchUpstreamStickyConversations(
  accountId: number,
  selection: StickyKeyConversationSelection,
  signal?: AbortSignal,
): Promise<UpstreamStickyConversationsResponse> {
  const search = new URLSearchParams();
  if (selection.mode === "count") {
    search.set("limit", String(selection.limit));
  } else {
    search.set("activityHours", String(selection.activityHours));
  }
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-accounts/${accountId}/sticky-keys?${search.toString()}`,
    {
      signal,
    },
  );
  return normalizeUpstreamStickyConversationsResponse(response);
}

export async function fetchUpstreamAccountDetail(
  accountId: number,
  options: { signal?: AbortSignal; includeRecentActions?: boolean } | AbortSignal = {},
): Promise<UpstreamAccountDetail> {
  type DetailOptions = { signal?: AbortSignal; includeRecentActions?: boolean };
  let detailOptions: DetailOptions;
  if (typeof AbortSignal !== "undefined" && options instanceof AbortSignal) {
    detailOptions = { signal: options };
  } else {
    detailOptions = options as DetailOptions;
  }
  const search = new URLSearchParams();
  if (detailOptions.includeRecentActions) {
    search.set("includeRecentActions", "true");
  }
  const response = await fetchJson<unknown>(
    search.size > 0
      ? `/api/pool/upstream-accounts/${accountId}?${search.toString()}`
      : `/api/pool/upstream-accounts/${accountId}`,
    {
      signal: detailOptions.signal,
    },
  );
  return normalizeUpstreamAccountDetail(response);
}

export async function createOauthLoginSession(
  payload: CreateOauthLoginSessionPayload,
): Promise<LoginSessionStatusResponse> {
  const response = await fetchJson<unknown>("/api/pool/upstream-accounts/oauth/login-sessions", {
    method: "POST",
    body: JSON.stringify(payload),
  });
  return normalizeLoginSessionStatusResponse(response);
}

export async function createOauthMailboxSession(
  payload: CreateOauthMailboxSessionPayload = {},
): Promise<OauthMailboxSession> {
  const response = await fetchJson<unknown>("/api/pool/upstream-accounts/oauth/mailbox-sessions", {
    method: "POST",
    body: JSON.stringify(payload),
  });
  return normalizeOauthMailboxSession(response);
}

export async function fetchOauthMailboxStatuses(
  payload: OauthMailboxStatusRequestPayload,
): Promise<OauthMailboxStatus[]> {
  const response = await fetchJson<unknown>(
    "/api/pool/upstream-accounts/oauth/mailbox-sessions/status",
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
  const items = Array.isArray((response as Record<string, unknown> | null)?.items)
    ? ((response as Record<string, unknown>).items as unknown[])
    : [];
  return items
    .map(normalizeOauthMailboxStatus)
    .filter((item): item is OauthMailboxStatus => item != null);
}

export async function deleteOauthMailboxSession(sessionId: string): Promise<void> {
  await fetchJson(
    `/api/pool/upstream-accounts/oauth/mailbox-sessions/${encodeURIComponent(sessionId)}`,
    {
      method: "DELETE",
    },
  );
}

export async function fetchOauthLoginSession(loginId: string): Promise<LoginSessionStatusResponse> {
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-accounts/oauth/login-sessions/${encodeURIComponent(loginId)}`,
  );
  return normalizeLoginSessionStatusResponse(response);
}

export async function updateOauthLoginSession(
  loginId: string,
  payload: UpdateOauthLoginSessionPayload,
  baseUpdatedAt?: string | null,
): Promise<LoginSessionStatusResponse> {
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-accounts/oauth/login-sessions/${encodeURIComponent(loginId)}`,
    withOauthLoginSessionBaseUpdatedAtHeader(baseUpdatedAt, {
      method: "PATCH",
      body: JSON.stringify(payload),
    }),
  );
  return normalizeLoginSessionStatusResponse(response);
}

export async function updateOauthLoginSessionKeepalive(
  loginId: string,
  payload: UpdateOauthLoginSessionPayload,
  baseUpdatedAt?: string | null,
): Promise<void> {
  const response = await fetch(
    withBase(`/api/pool/upstream-accounts/oauth/login-sessions/${encodeURIComponent(loginId)}`),
    withOauthLoginSessionBaseUpdatedAtHeader(baseUpdatedAt, {
      method: "PATCH",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify(payload),
      keepalive: true,
    }),
  );
  await ensureJsonRequestOk(response);
}

export async function reloginUpstreamAccount(
  accountId: number,
): Promise<LoginSessionStatusResponse> {
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-accounts/${accountId}/oauth/relogin`,
    {
      method: "POST",
    },
  );
  return normalizeLoginSessionStatusResponse(response);
}

export async function completeOauthLoginSession(
  loginId: string,
  payload: CompleteOauthLoginSessionPayload,
): Promise<UpstreamAccountDetail> {
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-accounts/oauth/login-sessions/${encodeURIComponent(loginId)}/complete`,
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
  return normalizeUpstreamAccountDetail(response);
}

export async function confirmOauthIdentityOverwrite(
  loginId: string,
): Promise<UpstreamAccountDetail> {
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-accounts/oauth/login-sessions/${encodeURIComponent(loginId)}/confirm-identity-overwrite`,
    {
      method: "POST",
    },
  );
  return normalizeUpstreamAccountDetail(response);
}

export async function validateImportedOauthAccounts(
  payload: ValidateImportedOauthAccountsPayload,
): Promise<ImportedOauthValidationResponse> {
  const response = await fetchJson<unknown>("/api/pool/upstream-accounts/oauth/imports/validate", {
    method: "POST",
    body: JSON.stringify(payload),
  });
  return normalizeImportedOauthValidationResponse(response);
}

export async function createImportedOauthValidationJob(
  payload: ValidateImportedOauthAccountsPayload,
): Promise<ImportedOauthValidationJobResponse> {
  const response = await fetchJson<unknown>(
    "/api/pool/upstream-accounts/oauth/imports/validation-jobs",
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
  return normalizeImportedOauthValidationJobResponse(response);
}

export async function cancelImportedOauthValidationJob(jobId: string): Promise<void> {
  await fetchJson(
    `/api/pool/upstream-accounts/oauth/imports/validation-jobs/${encodeURIComponent(jobId)}`,
    {
      method: "DELETE",
    },
  );
}

export async function importValidatedOauthAccounts(
  payload: ImportValidatedOauthAccountsPayload,
): Promise<ImportedOauthImportResponse> {
  const response = await fetchJson<unknown>("/api/pool/upstream-accounts/oauth/imports", {
    method: "POST",
    body: JSON.stringify(payload),
  });
  return normalizeImportedOauthImportResponse(response);
}

export async function createApiKeyUpstreamAccount(
  payload: CreateApiKeyAccountPayload,
): Promise<UpstreamAccountDetail> {
  const response = await fetchJson<unknown>("/api/pool/upstream-accounts/api-keys", {
    method: "POST",
    body: JSON.stringify(payload),
  });
  return normalizeUpstreamAccountDetail(response);
}

export async function updateUpstreamAccount(
  accountId: number,
  payload: UpdateUpstreamAccountPayload,
): Promise<UpstreamAccountDetail> {
  const response = await fetchJson<unknown>(`/api/pool/upstream-accounts/${accountId}`, {
    method: "PATCH",
    body: JSON.stringify(payload),
  });
  return normalizeUpstreamAccountDetail(response);
}

export async function updateUpstreamAccountGroup(
  groupName: string,
  payload: UpdateUpstreamAccountGroupPayload,
): Promise<UpstreamAccountGroupSummary> {
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-account-groups/${encodeURIComponent(groupName)}`,
    {
      method: "PUT",
      body: JSON.stringify(payload),
    },
  );
  const normalized = normalizeUpstreamAccountGroupSummary(response);
  if (!normalized) {
    throw new Error("Request failed: invalid upstream account group payload");
  }
  return normalized;
}

export async function deleteUpstreamAccountGroup(groupName: string): Promise<void> {
  await fetchJson(`/api/pool/upstream-account-groups/${encodeURIComponent(groupName)}`, {
    method: "DELETE",
  });
}

export async function bulkUpdateUpstreamAccounts(
  payload: BulkUpstreamAccountActionPayload,
): Promise<BulkUpstreamAccountActionResponse> {
  const response = await fetchJson<unknown>("/api/pool/upstream-accounts", {
    method: "POST",
    body: JSON.stringify(payload),
  });
  return normalizeBulkUpstreamAccountActionResponse(response);
}

export async function deleteUpstreamAccount(accountId: number): Promise<void> {
  await fetchJson(`/api/pool/upstream-accounts/${accountId}`, {
    method: "DELETE",
  });
}

export async function syncUpstreamAccount(accountId: number): Promise<UpstreamAccountDetail> {
  const response = await fetchJson<unknown>(`/api/pool/upstream-accounts/${accountId}/sync`, {
    method: "POST",
  });
  return normalizeUpstreamAccountDetail(response);
}

export async function createBulkUpstreamAccountSyncJob(
  payload: BulkUpstreamAccountSyncJobPayload,
): Promise<BulkUpstreamAccountSyncJobResponse> {
  const response = await fetchJson<unknown>("/api/pool/upstream-accounts/bulk-sync-jobs", {
    method: "POST",
    body: JSON.stringify(payload),
  });
  return normalizeBulkUpstreamAccountSyncJobResponse(response);
}

export async function fetchBulkUpstreamAccountSyncJob(
  jobId: string,
): Promise<BulkUpstreamAccountSyncJobResponse> {
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-accounts/bulk-sync-jobs/${encodeURIComponent(jobId)}`,
  );
  return normalizeBulkUpstreamAccountSyncJobResponse(response);
}

export async function cancelBulkUpstreamAccountSyncJob(jobId: string): Promise<void> {
  await fetchJson(`/api/pool/upstream-accounts/bulk-sync-jobs/${encodeURIComponent(jobId)}`, {
    method: "DELETE",
  });
}

export function createEventSource(path: string) {
  return new EventSource(withBase(path));
}

export function createImportedOauthValidationJobEventSource(jobId: string) {
  return createEventSource(
    `/api/pool/upstream-accounts/oauth/imports/validation-jobs/${encodeURIComponent(jobId)}/events`,
  );
}

export function createBulkUpstreamAccountSyncJobEventSource(jobId: string) {
  return createEventSource(
    `/api/pool/upstream-accounts/bulk-sync-jobs/${encodeURIComponent(jobId)}/events`,
  );
}
