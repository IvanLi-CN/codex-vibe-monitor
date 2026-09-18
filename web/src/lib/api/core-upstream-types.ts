import type {
  BlockedBindingDiagnostic,
  ForwardProxyBindingNode,
  PoolRoutingSelectionAudit,
  RoutingStateVersion,
} from "./core-foundation";

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
export type CodexImagegenRewriteMode = ImageToolRewriteMode;
export type CapabilitySupport = "supported" | "unsupported" | "unknown";
export type CapabilityOverride = Exclude<CapabilitySupport, "unknown">;
export type ImageIntent = "yes" | "direct_image" | "no" | "unknown";
export type AvailableModelsMode = "allowlist" | "denylist";
export type RequestCompressionAlgorithm = "follow" | "identity" | "gzip" | "deflate" | "zstd";
export type RequestCompressionLevelPreset = "fast" | "balanced" | "best";

export interface UpstreamCapabilityState {
  observed: CapabilitySupport;
  override?: CapabilityOverride | null;
  effective: CapabilitySupport;
  observedAt?: string | null;
  reason?: string | null;
}

export interface TagRoutingRule {
  allowCutOut: boolean;
  allowCutIn: boolean;
  priorityTier?: TagPriorityTier;
  fastModeRewriteMode?: TagFastModeRewriteMode;
  concurrencyLimit?: number | null;
  upstream429RetryEnabled?: boolean;
  upstream429MaxRetries?: number;
  availableModels?: string[];
  availableModelsMode?: AvailableModelsMode;
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
  imageFirstByteTimeoutSecs?: number;
  responsesStreamTimeoutSecs: number;
  compactStreamTimeoutSecs: number;
}

export interface GroupAccountRoutingRule extends TagRoutingRule {
  imageToolRewriteMode?: ImageToolRewriteMode;
  codexImagegenRewriteMode?: CodexImagegenRewriteMode | null;
  requestCompressionAlgorithm?: RequestCompressionAlgorithm;
  statusChangeReasons?: StatusChangeReasons;
  timeouts?: Partial<PoolRoutingTimeoutSettings>;
}

export interface EffectiveRoutingRuleFieldSources {
  allowCutOut: EffectiveRoutingRuleSource;
  allowCutIn: EffectiveRoutingRuleSource;
  priorityTier: EffectiveRoutingRuleSource;
  fastModeRewriteMode: EffectiveRoutingRuleSource;
  imageToolRewriteMode?: EffectiveRoutingRuleSource;
  codexImagegenRewriteMode?: EffectiveRoutingRuleSource;
  requestCompressionAlgorithm?: EffectiveRoutingRuleSource;
  concurrencyLimit: EffectiveRoutingRuleSource;
  upstream429Retry: EffectiveRoutingRuleSource;
  availableModels?: EffectiveRoutingRuleSource;
  availableModelsMode?: EffectiveRoutingRuleSource;
  systemDeniedModels?: EffectiveRoutingRuleSource;
}

export interface EffectiveRoutingTimeoutFieldSources {
  responsesFirstByteTimeoutSecs: EffectiveRoutingRuleSource;
  compactFirstByteTimeoutSecs: EffectiveRoutingRuleSource;
  imageFirstByteTimeoutSecs?: EffectiveRoutingRuleSource;
  responsesStreamTimeoutSecs: EffectiveRoutingRuleSource;
  compactStreamTimeoutSecs: EffectiveRoutingRuleSource;
}

export interface EffectiveRoutingRule extends GroupAccountRoutingRule {
  requestCompressionAlgorithm?: RequestCompressionAlgorithm;
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
  routingBlockUntil?: string | null;
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
  responseEndpointCapability?: UpstreamCapabilityState | null;
  chatCompletionsCapability?: UpstreamCapabilityState | null;
  imageEndpointCapability?: UpstreamCapabilityState | null;
  responseImageToolCapability?: UpstreamCapabilityState | null;
  codexImagegenCapability?: UpstreamCapabilityState | null;
  standaloneSearchCapability?: UpstreamCapabilityState | null;
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
  attemptId?: string | null;
  stickyKey?: string | null;
  model?: string | null;
  modelRouteStateBefore?: string | null;
  modelRouteStateAfter?: string | null;
  modelRoutePriorityBefore?: string | null;
  modelRoutePriorityAfter?: string | null;
  modelRouteFailureCount?: number | null;
  modelRouteCooldownUntil?: string | null;
  blockedBinding?: BlockedBindingDiagnostic | null;
  createdAt: string;
}

export interface ModelRoutingState {
  model: string;
  state: string;
  priority: string;
  failureCount: number;
  changedAt?: string | null;
  lastSeenAt: string;
  lastFailureAt?: string | null;
  lastFailureKind?: string | null;
  lastFailureMessage?: string | null;
  cooldownUntil?: string | null;
  cacheConcurrencyLimit?: number | null;
  cacheRecoveryLimit?: number | null;
  cacheLowHitStreak?: number;
  cacheCooldownLevel?: number;
  cacheLastHitRatePercent?: number | null;
  cacheUsageMissingSince?: string | null;
  cacheUsageMissingReason?: string | null;
  probeRequired?: boolean;
}

export type ModelRoutingLiveWindow = "15m" | "1h" | "6h" | "24h";

export interface ModelRoutingLiveAccount extends ModelRoutingState {
  accountId: number;
  accountDisplayName?: string;
}

export interface ModelRoutingLiveModelGroup {
  model: string;
  accounts: ModelRoutingLiveAccount[];
}

export interface ModelRoutingTimelineRecord {
  id: string;
  kind: "attempt" | "event" | string;
  occurredAt: string;
  accountId: number;
  accountDisplayName?: string;
  model: string;
  attemptId?: string | null;
  invokeId?: string | null;
  attemptIndex?: number | null;
  sameAccountRetryIndex?: number | null;
  routingSource?: string | null;
  routingSelectionAudit?: PoolRoutingSelectionAudit | null;
  status?: string | null;
  httpStatus?: number | null;
  failureKind?: string | null;
  totalLatencyMs?: number | null;
  action?: string | null;
  source?: string | null;
  reasonCode?: string | null;
  modelRouteStateBefore?: string | null;
  modelRouteStateAfter?: string | null;
  modelRoutePriorityBefore?: string | null;
  modelRoutePriorityAfter?: string | null;
  modelRouteFailureCount?: number | null;
  modelRouteCooldownUntil?: string | null;
}

export interface ModelRoutingLiveResponse {
  generatedAt: string;
  groups: ModelRoutingLiveModelGroup[];
  records: ModelRoutingTimelineRecord[];
}

export interface ModelRoutingHistoryResponse {
  items: ModelRoutingTimelineRecord[];
  nextCursor?: string | null;
}

export interface FetchModelRoutingLiveQuery {
  window?: ModelRoutingLiveWindow;
  model?: string;
  state?: "available" | "degraded" | "cooling_down" | string;
  limit?: number;
  signal?: AbortSignal;
}

export interface FetchModelRoutingHistoryQuery {
  model: string;
  cursor?: string;
  pageSize?: number;
  signal?: AbortSignal;
}

export interface UpstreamAccountDetail extends UpstreamAccountSummary {
  routingStateVersion?: RoutingStateVersion | null;
  note?: string | null;
  upstreamBaseUrl?: string | null;
  chatgptUserId?: string | null;
  verifiedEmail?: string | null;
  lastRefreshedAt?: string | null;
  history: UpstreamAccountHistoryPoint[];
  recentActions?: UpstreamAccountActionEvent[];
  modelRoutingStates?: ModelRoutingState[];
  modelMappings?: ModelMapping[];
  modelCatalog?: UpstreamAccountModelCatalog;
}

export type UpstreamAccountModelCatalogStatus =
  | "never"
  | "refreshing"
  | "ready"
  | "stale"
  | "failed"
  | string;

export interface UpstreamAccountModelCatalogError {
  code: string;
  message: string;
}

export interface UpstreamAccountModelCatalog {
  models: string[];
  status: UpstreamAccountModelCatalogStatus;
  lastAttemptedAt?: string | null;
  lastSuccessfulAt?: string | null;
  error?: UpstreamAccountModelCatalogError | null;
  stale: boolean;
}

export interface ModelMapping {
  sourceModel: string;
  targetModel: string;
  enabled: boolean;
}

export interface UpdateUpstreamAccountModelMappingsPayload {
  modelMappings: ModelMapping[];
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
  requestCompressionAlgorithm?: RequestCompressionAlgorithm;
  requestCompressionLevelPreset?: RequestCompressionLevelPreset;
  codexImagegenRewriteMode?: CodexImagegenRewriteMode;
  availableModels?: string[];
  availableModelsMode?: AvailableModelsMode;
  timeouts?: Partial<PoolRoutingTimeoutSettings>;
  cacheHitProtection?: Partial<CacheHitProtectionSettings>;
  priorityHandoffAdmissionEnabled?: boolean;
}

export interface PoolRoutingSettings {
  writesEnabled: boolean;
  apiKeyConfigured: boolean;
  maskedApiKey?: string | null;
  maintenance?: PoolRoutingMaintenanceSettings;
  requestCompressionAlgorithm?: RequestCompressionAlgorithm;
  requestCompressionLevelPreset?: RequestCompressionLevelPreset;
  codexImagegenRewriteMode?: CodexImagegenRewriteMode;
  availableModels?: string[];
  availableModelsMode?: AvailableModelsMode;
  timeouts?: PoolRoutingTimeoutSettings;
  cacheHitProtection?: CacheHitProtectionSettings;
  priorityHandoffAdmissionEnabled?: boolean;
}

export type CacheHitOverflowMode = "queue" | "reroute";

export interface CacheHitProtectionSettings {
  enabled: boolean;
  lowHitRateThresholdPercent: number;
  overflowMode: CacheHitOverflowMode;
  minimumInputTokens: number;
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
  kind?: "oauth_codex" | "api_key_codex" | string;
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
  kind?: "oauth_codex" | "api_key_codex" | string;
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
