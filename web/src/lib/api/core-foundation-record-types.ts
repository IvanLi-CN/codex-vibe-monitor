export interface ApiInvocation {
  id: number;
  invokeId: string;
  occurredAt: string;
  source?: string;
  proxyDisplayName?: string;
  model?: string;
  requestModel?: string;
  responseModel?: string;
  inputTokens?: number;
  outputTokens?: number;
  cacheInputTokens?: number;
  cacheWriteTokens?: number;
  reasoningTokens?: number;
  reasoningEffort?: string;
  totalTokens?: number;
  cost?: number;
  costInput?: number | null;
  costCacheWrite?: number | null;
  costCacheRead?: number | null;
  costOutput?: number | null;
  costReasoning?: number | null;
  status?: string;
  livePhase?: InvocationLivePhase | null;
  errorMessage?: string;
  downstreamStatusCode?: number | null;
  downstreamErrorMessage?: string;
  failureKind?: string;
  streamTerminalEvent?: string;
  upstreamErrorCode?: string;
  upstreamErrorMessage?: string;
  upstreamRequestId?: string;
  failureClass?: "service_failure" | "client_failure" | "client_abort" | "none";
  isActionable?: boolean;
  endpoint?: string;
  compactionRequestKind?: "compact" | "remote_v2" | null;
  compactionResponseKind?: "compact" | "remote_v2" | null;
  imageIntent?: "yes" | "direct_image" | "no" | "unknown" | null;
  requesterIp?: string;
  promptCacheKey?: string;
  stickyKey?: string | null;
  routeMode?: string;
  upstreamAccountId?: number | null;
  upstreamAccountName?: string;
  upstreamAccountPlanType?: string | null;
  responseContentEncoding?: string;
  requestCompressionAlgorithm?: string;
  poolAttemptCount?: number | null;
  poolDistinctAccountCount?: number | null;
  poolAttemptTerminalReason?: string | null;
  upstreamScope?: string;
  transport?: "websocket" | "http" | string | null;
  requestedServiceTier?: string;
  serviceTier?: string;
  billingServiceTier?: string;
  proxyWeightDelta?: number;
  costEstimated?: number;
  priceVersion?: string;
  costAudit?: InvocationCostAudit | null;
  tTotalMs?: number | null;
  tReqReadMs?: number | null;
  tReqParseMs?: number | null;
  tUpstreamConnectMs?: number | null;
  tUpstreamTtfbMs?: number | null;
  firstTokenMs?: number | null;
  tUpstreamStreamMs?: number | null;
  tRespParseMs?: number | null;
  tPersistMs?: number | null;
  timings?: ApiInvocationTimings;
  rawMetadata?: ApiInvocationRawMetadata;
  proxyTimings?: ApiInvocationTimings;
  proxyRawMetadata?: ApiInvocationRawMetadata;
  detailLevel?: "full" | "structured_only";
  detailPrunedAt?: string | null;
  detailPruneReason?: string | null;
  blockedBinding?: BlockedBindingDiagnostic | null;
  createdAt: string;
}

export interface InvocationCostAuditBreakdown {
  input?: number | null;
  cacheWrite?: number | null;
  cacheRead?: number | null;
  output?: number | null;
  reasoning?: number | null;
  total?: number | null;
}

export interface InvocationCostAudit {
  recorded?: InvocationCostAuditBreakdown | null;
  local?: InvocationCostAuditBreakdown | null;
  mismatch: boolean;
  reason?: string | null;
  absoluteDiffUsd?: number | null;
  recordedPriceVersion?: string | null;
  localPriceVersion?: string | null;
}

export type BlockedBindingConstraintSource =
  | "upstreamAccountBinding"
  | "encryptedSessionOwner"
  | string;

export type BlockedBindingRecoveryAction = "clearAndResetAffinity" | string;

export interface BlockedBindingDiagnostic {
  constraintSource: BlockedBindingConstraintSource;
  upstreamAccountId: number | null;
  upstreamAccountLabel?: string | null;
  promptCacheKey?: string | null;
  recoveryAction?: BlockedBindingRecoveryAction | null;
}

export type InvocationLivePhase = "queued" | "requesting" | "responding";

export interface InvocationPhaseCounts {
  queued: number;
  requesting: number;
  responding: number;
}

export interface ApiInvocationTimings {
  requestReadMs?: number | null;
  requestParseMs?: number | null;
  upstreamConnectMs?: number | null;
  upstreamFirstByteMs?: number | null;
  upstreamStreamMs?: number | null;
  responseParseMs?: number | null;
  persistenceMs?: number | null;
  totalMs?: number | null;
  [stage: string]: number | null | undefined;
}

export interface ApiInvocationRawMetadata {
  request?: Record<string, unknown>;
  response?: Record<string, unknown>;
  [key: string]: unknown;
}

export interface ListResponse {
  snapshotId?: number;
  total?: number;
  page?: number;
  pageSize?: number;
  records: ApiInvocation[];
}

export interface ApiPoolUpstreamRequestAttempt {
  attemptId: string;
  invokeId: string;
  occurredAt: string;
  endpoint: string;
  stickyKey?: string | null;
  routingSource?: string | null;
  routingSelectionAudit?: PoolRoutingSelectionAudit | null;
  upstreamAccountId?: number | null;
  upstreamAccountName?: string | null;
  model?: string | null;
  requestModel?: string | null;
  upstreamRequestModel?: string | null;
  modelMappingPattern?: string | null;
  responseModel?: string | null;
  compactionRequestKind?: ApiInvocation["compactionRequestKind"];
  compactionResponseKind?: ApiInvocation["compactionResponseKind"];
  imageIntent?: ApiInvocation["imageIntent"];
  upstreamRouteKey?: string | null;
  proxyBindingKeySnapshot?: string | null;
  attemptIndex: number;
  distinctAccountIndex: number;
  sameAccountRetryIndex: number;
  requesterIp?: string | null;
  startedAt?: string | null;
  finishedAt?: string | null;
  status: string;
  phase?: string | null;
  httpStatus?: number | null;
  downstreamHttpStatus?: number | null;
  failureKind?: string | null;
  errorMessage?: string | null;
  downstreamErrorMessage?: string | null;
  connectLatencyMs?: number | null;
  firstTokenMs?: number | null;
  firstByteLatencyMs?: number | null;
  streamLatencyMs?: number | null;
  upstreamRequestId?: string | null;
  downstreamRequestContentEncoding?: string | null;
  upstreamRequestCompressionAlgorithm?: string | null;
  upstreamRequestCompressionMode?: string | null;
  logicalBodyBytes?: number | null;
  transmittedBodyBytes?: number | null;
  savedBytes?: number | null;
  ratioPct?: number | null;
  approxUploadBytes?: number | null;
  approxDownloadBytes?: number | null;
  createdAt: string;
  invocationRecord?: ApiInvocation | null;
  workflowEntry?: ApiInvocationWorkflowTimelineEntry | null;
}

export interface PoolRoutingSelectionAuditExcludedCandidate {
  accountId: number;
  accountName: string;
  reasonCode: string;
}

export interface PoolRoutingSelectionScoreSnapshot {
  eligibility: string;
  routeBindingFailurePenalty: number;
  modelRoutePenalty: number;
  modelRoutePenaltyCode: string;
  routingPriorityRank: number;
  capacityLane: string;
  dispatchState: string;
  secondaryResetProximitySecs?: number | null;
  primaryResetProximitySecs?: number | null;
  scarcityScore: string;
  effectiveLoad: number;
  lastSelectedAt?: string | null;
}

export interface PoolRoutingSelectionAudit {
  selectedAccountId: number;
  selectedAccountName: string;
  eligibleCandidateCount: number;
  winnerReasonCode: string;
  comparedAccountId?: number | null;
  comparedAccountName?: string | null;
  selectedScore?: PoolRoutingSelectionScoreSnapshot | null;
  comparedScore?: PoolRoutingSelectionScoreSnapshot | null;
  handoffAdmission?: {
    decision: string;
    phase: string;
    verificationSuccessCount: number;
    generation?: number;
    trigger?: "priorityAttraction" | "modelRouteRecovery" | string;
  } | null;
  excludedCandidates: PoolRoutingSelectionAuditExcludedCandidate[];
}

export interface PoolRoutingNoCandidateAuditCandidate {
  accountId: number;
  accountName: string;
  reasonCode: string;
}

export interface PoolRoutingNoCandidateAudit {
  terminalReasonCode: string;
  candidateCount: number;
  eligibleCandidateCount: number;
  reservationConflictCount: number;
  nextEligibleAt?: string | null;
  excludedReasonCounts: Record<string, number>;
  candidates: PoolRoutingNoCandidateAuditCandidate[];
}

export interface UpstreamAccountAttemptStickyKeyOption {
  value: string;
  latestCreatedAt: string;
}

export interface UpstreamAccountAttemptListResponse {
  items: ApiPoolUpstreamRequestAttempt[];
  stickyKeyOptions: UpstreamAccountAttemptStickyKeyOption[];
  total: number;
  page: number;
  pageSize: number;
}

export type InvocationFocus = "token" | "network" | "exception";
export type InvocationSortBy =
  | "occurredAt"
  | "totalTokens"
  | "cost"
  | "tTotalMs"
  | "tUpstreamTtfbMs"
  | "status";
export type InvocationSortOrder = "asc" | "desc";
export type InvocationRangePreset = "today" | "1d" | "7d" | "30d" | "custom";
export type InvocationModelTarget = "request" | "response";
export type InvocationModelRerouteFilter = "all" | "rerouted" | "notRerouted";
export type InvocationSuggestionField =
  | "model"
  | "requestModel"
  | "responseModel"
  | "endpoint"
  | "failureKind"
  | "stickyKey"
  | "promptCacheKey"
  | "requesterIp"
  | "proxyDisplayName"
  | "upstreamAccount"
  | "serviceTier"
  | "reasoningEffort";

export interface InvocationRecordsQuery {
  page?: number;
  pageSize?: number;
  snapshotId?: number;
  anchorId?: string;
  sortBy?: InvocationSortBy;
  sortOrder?: InvocationSortOrder;
  rangePreset?: InvocationRangePreset;
  from?: string;
  to?: string;
  model?: string;
  models?: string[];
  modelTarget?: InvocationModelTarget;
  modelRerouted?: boolean;
  status?: string;
  endpoint?: string;
  invokeId?: string;
  attemptId?: string;
  // Kept for compatibility with older call sites and stale deep links.
  requestId?: string;
  failureClass?: string;
  failureKind?: string;
  promptCacheKey?: string;
  stickyKey?: string;
  upstreamScope?: string;
  proxyDisplayName?: string;
  transport?: string;
  serviceTier?: string;
  reasoningEffort?: string;
  reasoningEfforts?: string[];
  requesterIp?: string;
  upstreamAccountId?: number;
  keyword?: string;
  minTotalTokens?: number;
  maxTotalTokens?: number;
  minTotalMs?: number;
  maxTotalMs?: number;
  suggestField?: InvocationSuggestionField;
  suggestQuery?: string;
  signal?: AbortSignal;
}

export interface InvocationTokenSummary {
  requestCount: number;
  totalTokens: number;
  avgTokensPerRequest: number;
  cacheWriteTokens: number;
  cacheInputTokens: number;
  outputTokens: number;
  totalCost: number;
  maxTokensPerRequest?: number | null;
}

export interface InvocationNetworkSummary {
  avgTtfbMs?: number | null;
  p95TtfbMs?: number | null;
  avgFirstTokenMs?: number | null;
  p95FirstTokenMs?: number | null;
  avgResponseDurationMs?: number | null;
  p95ResponseDurationMs?: number | null;
  avgTotalMs?: number | null;
  p95TotalMs?: number | null;
  maxTotalMs?: number | null;
}

export interface InvocationExceptionSummary {
  failureCount: number;
  serviceFailureCount: number;
  clientFailureCount: number;
  clientAbortCount: number;
  actionableFailureCount: number;
}

export interface InvocationRecordsResponse extends ListResponse {
  snapshotId: number;
  total: number;
  page: number;
  pageSize: number;
}

export interface InvocationRecordLocationResponse extends InvocationRecordsResponse {
  anchorId: string;
  invokeId?: string;
  // Kept for compatibility with older mocks and stale clients.
  requestId?: string;
  attemptId?: string | null;
  targetIndex: number;
  targetAbsoluteIndex: number;
}

export interface InvocationRecordLocationQuery {
  invokeId?: string;
  // Kept for compatibility with older call sites and stale deep links.
  requestId?: string;
  attemptId?: string;
  upstreamAccountId?: number;
  pageSize?: number;
  signal?: AbortSignal;
}

export interface InvocationRecordsSummaryResponse extends StatsResponse {
  snapshotId: number;
  newRecordsCount: number;
  token: InvocationTokenSummary;
  network: InvocationNetworkSummary;
  exception: InvocationExceptionSummary;
}

export interface InvocationRecordsNewCountResponse {
  snapshotId: number;
  newRecordsCount: number;
}

export interface InvocationSuggestionItem {
  value: string;
  label?: string;
  count: number;
}

export interface InvocationSuggestionBucket {
  items: InvocationSuggestionItem[];
  hasMore: boolean;
}

export interface InvocationSuggestionsResponse {
  model: InvocationSuggestionBucket;
  requestModel: InvocationSuggestionBucket;
  responseModel: InvocationSuggestionBucket;
  endpoint: InvocationSuggestionBucket;
  failureKind: InvocationSuggestionBucket;
  stickyKey: InvocationSuggestionBucket;
  promptCacheKey: InvocationSuggestionBucket;
  requesterIp: InvocationSuggestionBucket;
  proxyDisplayName: InvocationSuggestionBucket;
  upstreamAccount: InvocationSuggestionBucket;
  serviceTier: InvocationSuggestionBucket;
  reasoningEffort: InvocationSuggestionBucket;
}

export interface ApiInvocationAbnormalResponseBodyPreview {
  available: boolean;
  previewText?: string | null;
  hasMore: boolean;
  unavailableReason?: string | null;
}

export interface ApiInvocationRecordDetailResponse {
  id: number;
  abnormalResponseBody?: ApiInvocationAbnormalResponseBodyPreview | null;
}

export interface ApiInvocationResponseBodyResponse {
  available: boolean;
  bodyText?: string | null;
  unavailableReason?: string | null;
  headers?: Record<string, unknown> | null;
  routing?: Record<string, unknown> | null;
  bodySize?: number | null;
  bodyTruncated?: boolean | null;
  bodyTruncatedReason?: string | null;
  detailLevel?: string | null;
  detailPruneReason?: string | null;
  captureSource?: string | null;
}

export interface ApiInvocationRequestBodyResponse extends ApiInvocationResponseBodyResponse {}

export interface ApiInvocationWorkflowResponseBody {
  available: boolean;
  bodyText?: string | null;
  unavailableReason?: string | null;
}

export interface ApiInvocationWorkflowAttempt {
  synthetic: boolean;
  attemptId?: string | null;
  occurredAt: string;
  endpoint: string;
  stickyKey?: string | null;
  routingSource?: string | null;
  routingSelectionAudit?: PoolRoutingSelectionAudit | null;
  upstreamAccountId?: number | null;
  upstreamAccountName?: string | null;
  requestModel?: string | null;
  responseModel?: string | null;
  upstreamRouteKey?: string | null;
  proxyBindingKeySnapshot?: string | null;
  attemptIndex: number;
  distinctAccountIndex: number;
  sameAccountRetryIndex: number;
  requesterIp?: string | null;
  startedAt?: string | null;
  finishedAt?: string | null;
  status: string;
  phase?: string | null;
  httpStatus?: number | null;
  downstreamHttpStatus?: number | null;
  failureKind?: string | null;
  errorMessage?: string | null;
  downstreamErrorMessage?: string | null;
  connectLatencyMs?: number | null;
  firstTokenMs?: number | null;
  firstByteLatencyMs?: number | null;
  streamLatencyMs?: number | null;
  upstreamRequestId?: string | null;
  requestSummary?: Record<string, unknown> | null;
  responseSummary?: Record<string, unknown> | null;
}

export interface ApiInvocationWorkflowTimelineEntry {
  blockId: string;
  kind: string;
  occurredAt?: string | null;
  title: string;
  subtitle?: string | null;
  status?: string | null;
  attempt?: ApiInvocationWorkflowAttempt | null;
  detail?: Record<string, unknown> | null;
  responseBody?: ApiInvocationWorkflowResponseBody | null;
}

export interface ApiInvocationWorkflowHero {
  recordId: number;
  invokeId: string;
  promptCacheKey?: string | null;
  routeMode?: string | null;
  endpoint?: string | null;
  requestModel?: string | null;
  responseModel?: string | null;
  finalStatus?: string | null;
  failureClass?: string | null;
  downstreamStatusCode?: number | null;
  upstreamAccountId?: number | null;
  upstreamAccountName?: string | null;
  totalDurationMs?: number | null;
  timelineAttemptCount: number;
  poolAttemptCount?: number | null;
  totalTokens?: number | null;
  cost?: number | null;
  occurredAt?: string | null;
  poolRoutingNoCandidateAudit?: PoolRoutingNoCandidateAudit | null;
}

export interface ApiInvocationWorkflowDetailResponse {
  hero: ApiInvocationWorkflowHero;
  timeline: ApiInvocationWorkflowTimelineEntry[];
  reconstructed: boolean;
  partial: boolean;
  partialReason?: string | null;
}

export interface StatsResponse {
  totalCount: number;
  successCount: number;
  failureCount: number;
  totalCost: number;
  totalTokens: number;
  usageBreakdown?: UsageBreakdown | null;
  inProgressConversationCount?: number | null;
  inProgressRetryConversationCount?: number | null;
  inProgressAvgWaitMs?: number | null;
  inProgressPhaseCounts?: InvocationPhaseCounts | null;
  nonSuccessCost?: number | null;
  nonSuccessTokens?: number | null;
  maintenance?: StatsMaintenanceResponse;
}

export type LongTermStatsRange = "7d" | "30d" | "180d" | "365d";
export type LongTermStatsDimension = "model" | "upstream";

export interface LongTermMetrics {
  calls: number;
  tokens: number | null;
  tokenSamples: number;
  cost: number | null;
  costSamples: number;
  usageTimeMs: number | null;
  usageTimeSamples: number;
  wallTimeMs: number | null;
  wallTimeSamples: number;
  outputSpeedTokensPerSecond: number | null;
  outputSpeedSamples: number;
  firstByteMs: number | null;
  firstByteSamples: number;
  responseMs: number | null;
  responseSamples: number;
}

export interface LongTermDailyPoint extends LongTermMetrics {
  date: string;
}

export interface LongTermSeriesSummary extends LongTermMetrics {
  seriesKey: string;
  displayName: string;
  reasoningEffort?: string | null;
}

export interface LongTermStatsOverviewResponse {
  status: "preparing" | "ready" | "empty" | "error";
  statisticsStartDate?: string | null;
  processedRows: number;
  totalRows: number;
  timezone: "Asia/Shanghai" | string;
  range: LongTermStatsRange;
  global: LongTermMetrics;
  daily: LongTermDailyPoint[];
  models: LongTermSeriesSummary[];
  upstreams: LongTermSeriesSummary[];
}

export interface LongTermSeries {
  seriesKey: string;
  displayName: string;
  reasoningEffort?: string | null;
  points: LongTermDailyPoint[];
}

export interface LongTermStatsSeriesResponse {
  status: LongTermStatsOverviewResponse["status"];
  statisticsStartDate?: string | null;
  processedRows: number;
  totalRows: number;
  timezone: string;
  range: LongTermStatsRange;
  dimension: LongTermStatsDimension;
  series: LongTermSeries[];
}

export interface UsageCostBreakdown {
  input: number;
  cacheWrite: number;
  cacheRead: number;
  output: number;
  reasoning: number;
  unknown: number;
}

export interface UsageBreakdownModel {
  model: string;
  reasoningEffort?: string | null;
  cacheWriteTokens: number;
  cacheReadTokens: number;
  outputTokens: number;
  costs?: UsageCostBreakdown | null;
}

export interface UsageBreakdown {
  cacheWriteTokens: number;
  cacheReadTokens: number;
  outputTokens: number;
  costs?: UsageCostBreakdown | null;
  models: UsageBreakdownModel[];
}

export interface ModelPerformanceMetrics {
  tokensPerMinute: number;
  streamingResponseRate?: number | null;
  avgResponseMs?: number | null;
  avgFirstResponseByteTotalMs?: number | null;
  avgFirstTokenMs?: number | null;
  wallClockUsageDurationMs?: number | null;
  cumulativeUsageDurationMs?: number | null;
  parallelism?: number | null;
}

export interface ModelPerformanceModel extends ModelPerformanceMetrics {
  model: string;
  reasoningEffort?: string | null;
}

export interface ModelPerformance {
  available: boolean;
  total: ModelPerformanceMetrics;
  models: ModelPerformanceModel[];
}

import type { StatsMaintenanceResponse } from "./core-foundation-dashboard-types";
