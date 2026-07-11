import { getBrowserTimeZone } from "../timeZone";
import { normalizeForwardProxyProtocolLabel } from "../forwardProxyDisplay";
import type {
  CompactSupportState,
  EffectiveRoutingRule,
  EffectiveRoutingRuleSource,
  EffectiveRoutingTimeoutFieldSources,
  PoolRoutingMaintenanceSettings,
  PoolRoutingSettings,
  PoolRoutingTimeoutSettings,
} from "./core-upstream";
import { normalizeEffectiveRoutingRule } from "./core-upstream";

const rawBase =
  import.meta.env.VITE_APP_RUNTIME === "demo"
    ? import.meta.env.BASE_URL
    : import.meta.env.VITE_API_BASE_URL ?? "";
const API_BASE = rawBase.endsWith("/") ? rawBase.slice(0, -1) : rawBase;
const FORWARD_PROXY_VALIDATION_TIMEOUT_MS = 5_000;
const FORWARD_PROXY_SUBSCRIPTION_VALIDATION_TIMEOUT_MS = 60_000;
const FORWARD_PROXY_HISTORY_DAY_MS = 86_400_000;
export const DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS = {
  primarySyncIntervalSecs: 300,
  secondarySyncIntervalSecs: 1_800,
  priorityAvailableAccountCap: 100,
} as const;

type ZonedDateParts = {
  year: number;
  month: number;
  day: number;
  weekday: number;
};

export const withBase = (path: string) => `${API_BASE}${path}`;

export class ApiRequestError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "ApiRequestError";
    this.status = status;
    Object.setPrototypeOf(this, new.target.prototype);
  }
}

function buildRequestError(
  response: Response,
  rawText: string,
): ApiRequestError {
  const compactText = rawText.replace(/\s+/g, " ").trim();
  const detail = (compactText || response.statusText || "").slice(0, 220);
  return new ApiRequestError(
    response.status,
    detail
      ? `Request failed: ${response.status} ${detail}`
      : `Request failed: ${response.status}`,
  );
}

export async function fetchJson<T>(
  path: string,
  init?: RequestInit,
): Promise<T> {
  const { data } = await fetchJsonResponse<T>(path, init);
  return data;
}

async function fetchJsonResponse<T>(
  path: string,
  init?: RequestInit,
): Promise<{ data: T; response: Response }> {
  const response = await fetch(withBase(path), {
    headers: {
      "Content-Type": "application/json",
    },
    ...init,
  });

  if (!response.ok) {
    const rawText = await response.text();
    throw buildRequestError(response, rawText);
  }

  if (response.status === 204) {
    return { data: undefined as T, response };
  }

  const rawText = await response.text();
  if (!rawText.trim()) {
    return { data: undefined as T, response };
  }

  return {
    data: JSON.parse(rawText) as T,
    response,
  };
}

export async function ensureJsonRequestOk(response: Response): Promise<void> {
  if (response.ok) {
    return;
  }

  const rawText = await response.text();
  throw buildRequestError(response, rawText);
}

function parseForwardProxyHistoryRangeSeconds(range: string): number | null {
  if (range.endsWith("mo")) {
    const value = Number(range.slice(0, -2));
    return Number.isFinite(value) ? value * 30 * 86_400 : null;
  }
  const unit = range.slice(-1);
  const value = Number(range.slice(0, -1));
  if (!Number.isFinite(value)) return null;
  switch (unit) {
    case "d":
      return value * 86_400;
    case "h":
      return value * 3_600;
    case "m":
      return value * 60;
    default:
      return null;
  }
}

function getForwardProxyHistoryOffsetMinutes(
  date: Date,
  timeZone: string,
): number | null {
  const timeZoneName = new Intl.DateTimeFormat("en-US", {
    timeZone,
    timeZoneName: "shortOffset",
    hour: "2-digit",
  })
    .formatToParts(date)
    .find((part) => part.type === "timeZoneName")?.value;
  const normalized = (timeZoneName ?? "").replace(/^UTC/, "GMT");
  if (!normalized || normalized === "GMT") {
    return 0;
  }
  const match = normalized.match(/^GMT([+-])(\d{1,2})(?::(\d{2}))?$/i);
  if (!match) {
    return null;
  }
  const sign = match[1] === "-" ? -1 : 1;
  const hours = Number(match[2] ?? "0");
  const minutes = Number(match[3] ?? "0");
  return sign * (hours * 60 + minutes);
}

function getForwardProxyHistoryDateParts(
  date: Date,
  timeZone: string,
): ZonedDateParts | null {
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone,
    weekday: "short",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).formatToParts(date);
  const partMap = Object.fromEntries(
    parts
      .filter((part) => ["weekday", "year", "month", "day"].includes(part.type))
      .map((part) => [part.type, part.value]),
  );
  const weekdayMap: Record<string, number> = {
    Mon: 0,
    Tue: 1,
    Wed: 2,
    Thu: 3,
    Fri: 4,
    Sat: 5,
    Sun: 6,
  };
  const year = Number(partMap.year);
  const month = Number(partMap.month);
  const day = Number(partMap.day);
  const weekday = weekdayMap[partMap.weekday ?? ""];
  if (
    !Number.isFinite(year) ||
    !Number.isFinite(month) ||
    !Number.isFinite(day) ||
    weekday === undefined
  ) {
    return null;
  }
  return { year, month, day, weekday };
}

function addUtcDays(parts: ZonedDateParts, days: number): ZonedDateParts {
  const shifted = new Date(
    Date.UTC(parts.year, parts.month - 1, parts.day + days),
  );
  return {
    year: shifted.getUTCFullYear(),
    month: shifted.getUTCMonth() + 1,
    day: shifted.getUTCDate(),
    weekday: shifted.getUTCDay(),
  };
}

function forwardProxyHistoryLocalMidnightUtcMillis(
  timeZone: string,
  parts: Pick<ZonedDateParts, "year" | "month" | "day">,
): number {
  const localMidnightUtc = Date.UTC(
    parts.year,
    parts.month - 1,
    parts.day,
    0,
    0,
    0,
  );
  let candidate = localMidnightUtc;
  for (let index = 0; index < 4; index += 1) {
    const offsetMinutes = getForwardProxyHistoryOffsetMinutes(
      new Date(candidate),
      timeZone,
    );
    if (offsetMinutes === null) {
      break;
    }
    const adjusted = localMidnightUtc - offsetMinutes * 60_000;
    if (adjusted === candidate) {
      return candidate;
    }
    candidate = adjusted;
  }
  return candidate;
}

function resolveForwardProxyHistoryRangeMillis(
  range: string,
  timeZone: string,
  now: Date,
): { startMs: number; endMs: number } | null {
  const localNow = getForwardProxyHistoryDateParts(now, timeZone);
  if (!localNow) {
    return null;
  }

  if (range === "today") {
    return {
      startMs: forwardProxyHistoryLocalMidnightUtcMillis(timeZone, localNow),
      endMs: now.getTime(),
    };
  }
  if (range === "thisWeek") {
    const weekStart = addUtcDays(localNow, -localNow.weekday);
    return {
      startMs: forwardProxyHistoryLocalMidnightUtcMillis(timeZone, weekStart),
      endMs: now.getTime(),
    };
  }
  if (range === "thisMonth") {
    return {
      startMs: forwardProxyHistoryLocalMidnightUtcMillis(timeZone, {
        year: localNow.year,
        month: localNow.month,
        day: 1,
      }),
      endMs: now.getTime(),
    };
  }

  const durationSeconds = parseForwardProxyHistoryRangeSeconds(range);
  if (durationSeconds === null) {
    return null;
  }
  return {
    startMs: now.getTime() - durationSeconds * 1_000,
    endMs: now.getTime(),
  };
}

function resolveForwardProxyHistoryTimeZone(
  range: string,
  timeZone?: string,
): string {
  const candidate = timeZone ?? getBrowserTimeZone();
  try {
    const rangeWindow = resolveForwardProxyHistoryRangeMillis(
      range,
      candidate,
      new Date(),
    );
    if (!rangeWindow) {
      return candidate;
    }
    for (
      let currentMs = rangeWindow.startMs;
      currentMs < rangeWindow.endMs;
      currentMs += FORWARD_PROXY_HISTORY_DAY_MS
    ) {
      const offsetMinutes = getForwardProxyHistoryOffsetMinutes(
        new Date(currentMs),
        candidate,
      );
      if (offsetMinutes !== null && offsetMinutes % 60 !== 0) {
        throw new Error(
          `unsupported timeZone for forward proxy hourly timeseries: ${candidate}; hourly buckets require whole-hour UTC offsets`,
        );
      }
    }
    const lastSampleMs = Math.max(rangeWindow.startMs, rangeWindow.endMs - 1);
    const lastOffsetMinutes = getForwardProxyHistoryOffsetMinutes(
      new Date(lastSampleMs),
      candidate,
    );
    if (lastOffsetMinutes !== null && lastOffsetMinutes % 60 !== 0) {
      throw new Error(
        `unsupported timeZone for forward proxy hourly timeseries: ${candidate}; hourly buckets require whole-hour UTC offsets`,
      );
    }
    return candidate;
  } catch (error) {
    if (
      error instanceof Error &&
      error.message.startsWith(
        "unsupported timeZone for forward proxy hourly timeseries:",
      )
    ) {
      throw error;
    }
    return candidate;
  }
}

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
  tTotalMs?: number | null;
  tReqReadMs?: number | null;
  tReqParseMs?: number | null;
  tUpstreamConnectMs?: number | null;
  tUpstreamTtfbMs?: number | null;
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
  createdAt: string;
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
  id: number;
  invokeId: string;
  occurredAt: string;
  endpoint: string;
  stickyKey?: string | null;
  upstreamAccountId?: number | null;
  upstreamAccountName?: string | null;
  model?: string | null;
  totalTokens?: number | null;
  cost?: number | null;
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
  firstByteLatencyMs?: number | null;
  streamLatencyMs?: number | null;
  upstreamRequestId?: string | null;
  createdAt: string;
}

export interface UpstreamAccountAttemptListResponse {
  items: ApiPoolUpstreamRequestAttempt[];
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
export type InvocationSuggestionField =
  | "model"
  | "endpoint"
  | "failureKind"
  | "promptCacheKey"
  | "requesterIp";

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
  status?: string;
  endpoint?: string;
  requestId?: string;
  failureClass?: string;
  failureKind?: string;
  promptCacheKey?: string;
  stickyKey?: string;
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
  cacheInputTokens: number;
  totalCost: number;
}

export interface InvocationNetworkSummary {
  avgTtfbMs?: number | null;
  p95TtfbMs?: number | null;
  avgTotalMs?: number | null;
  p95TotalMs?: number | null;
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

export interface InvocationRecordLocationResponse
  extends InvocationRecordsResponse {
  anchorId: string;
  targetIndex: number;
  targetAbsoluteIndex: number;
}

export interface InvocationRecordLocationQuery {
  requestId: string;
  upstreamAccountId: number;
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
  count: number;
}

export interface InvocationSuggestionBucket {
  items: InvocationSuggestionItem[];
  hasMore: boolean;
}

export interface InvocationSuggestionsResponse {
  model: InvocationSuggestionBucket;
  endpoint: InvocationSuggestionBucket;
  failureKind: InvocationSuggestionBucket;
  promptCacheKey: InvocationSuggestionBucket;
  requesterIp: InvocationSuggestionBucket;
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

export interface UpstreamAccountActivityAccount {
  accountKey?: string;
  upstreamAccountId: number | null;
  displayName: string;
  isUnassigned?: boolean;
  groupName?: string | null;
  planType?: string | null;
  enabled?: boolean | null;
  displayStatus?: string | null;
  enableStatus?: string | null;
  workStatus?: string | null;
  healthStatus?: string | null;
  syncState?: string | null;
  lastError?: string | null;
  lastActionReasonMessage?: string | null;
  requestCount: number;
  successCount: number;
  failureCount: number;
  nonSuccessCount: number;
  totalTokens: number;
  successTokens: number;
  nonSuccessTokens: number;
  failureTokens: number;
  failureCost: number;
  totalCost: number;
  usageBreakdown: UsageBreakdown;
  cacheHitRate?: number | null;
  tokensPerMinute?: number | null;
  spendRate?: number | null;
  firstByteAvgMs?: number | null;
  firstResponseByteTotalAvgMs?: number | null;
  avgTotalMs?: number | null;
  inProgressInvocationCount?: number | null;
  inProgressPhaseCounts?: InvocationPhaseCounts | null;
  retryInvocationCount?: number | null;
  effectiveRoutingRule: EffectiveRoutingRule;
  recentInvocations: PromptCacheConversationInvocationPreview[];
}

export interface UpstreamAccountActivityResponse {
  range: string;
  rangeStart: string;
  rangeEnd: string;
  accounts: UpstreamAccountActivityAccount[];
}

export interface DashboardActivityRateWindow {
  start: string;
  end: string;
  windowMinutes: number;
  mode: string;
}

export interface DashboardActivitySummary {
  stats: StatsResponse;
  tokensPerMinute?: number | null;
  spendRate?: number | null;
}

export interface DashboardActivityResponse {
  range: string;
  rangeStart: string;
  rangeEnd: string;
  snapshotId: number;
  rateWindow: DashboardActivityRateWindow;
  summary: DashboardActivitySummary;
  accounts?: UpstreamAccountActivityAccount[];
}

export interface StatsMaintenanceResponse {
  rawCompressionBacklog?: RawCompressionBacklogResponse;
  startupBackfill?: StartupBackfillResponse;
  historicalRollupBackfill?: HistoricalRollupBackfillResponse;
}

export interface RawCompressionBacklogResponse {
  oldestUncompressedAgeSecs: number;
  uncompressedCount: number;
  uncompressedBytes: number;
  alertLevel: "ok" | "warn" | "critical";
}

export interface StartupBackfillResponse {
  upstreamActivityArchivePendingAccounts: number;
  zeroUpdateStreak: number;
  nextRunAfter?: string | null;
}

export interface HistoricalRollupBackfillResponse {
  pendingBuckets: number;
  legacyArchivePending: number;
  lastMaterializedHour?: string | null;
  alertLevel: "none" | "warn" | "critical";
}

export interface TimeseriesPoint {
  bucketStart: string;
  bucketEnd: string;
  totalCount: number;
  successCount: number;
  failureCount: number;
  inFlightCount?: number;
  inFlightPhaseCounts?: InvocationPhaseCounts | null;
  totalTokens: number;
  cacheInputTokens?: number;
  totalCost: number;
  nonSuccessCost?: number;
  avgTotalMs?: number | null;
  totalLatencySampleCount?: number | null;
  firstByteSampleCount?: number;
  firstByteAvgMs?: number | null;
  firstByteP95Ms?: number | null;
  firstResponseByteTotalSampleCount?: number;
  firstResponseByteTotalAvgMs?: number | null;
  firstResponseByteTotalP95Ms?: number | null;
}

export interface TimeseriesResponse {
  rangeStart: string;
  rangeEnd: string;
  bucketSeconds: number;
  snapshotId?: number;
  effectiveBucket?: string;
  availableBuckets?: string[];
  bucketLimitedToDaily?: boolean;
  points: TimeseriesPoint[];
}

export interface ParallelWorkPoint {
  bucketStart: string;
  bucketEnd: string;
  parallelCount: number;
}

export interface ParallelWorkConversation {
  conversationId: string;
  start: string;
  end: string;
  requestCount: number;
}

export interface ParallelWorkWindowResponse {
  rangeStart: string;
  rangeEnd: string;
  bucketSeconds: number;
  completeBucketCount: number;
  activeBucketCount: number;
  minCount: number | null;
  maxCount: number | null;
  avgCount: number | null;
  effectiveTimeZone?: string;
  timeZoneFallback?: boolean;
  points: ParallelWorkPoint[];
  conversations?: ParallelWorkConversation[];
}

export interface ParallelWorkStatsResponse {
  current: ParallelWorkWindowResponse;
  minute7d: ParallelWorkWindowResponse;
  hour30d: ParallelWorkWindowResponse;
  dayAll: ParallelWorkWindowResponse;
}

export interface ErrorDistributionItem {
  reason: string;
  count: number;
}

export interface ErrorDistributionResponse {
  rangeStart: string;
  rangeEnd: string;
  items: ErrorDistributionItem[];
}

export type FailureScope = "all" | "service" | "client" | "abort";

export interface FailureSummaryResponse {
  rangeStart: string;
  rangeEnd: string;
  totalFailures: number;
  serviceFailureCount: number;
  clientFailureCount: number;
  clientAbortCount: number;
  actionableFailureCount: number;
  actionableFailureRate: number;
}

export interface PerfStageStats {
  stage: string;
  count: number;
  avgMs: number;
  p50Ms: number;
  p90Ms: number;
  p99Ms: number;
  maxMs: number;
}

export interface PerfStatsResponse {
  rangeStart: string;
  rangeEnd: string;
  items?: PerfStageStats[];
  stages?: PerfStageStats[];
}

export interface PerfStatsQuery {
  range?: string;
  bucket?: string;
  settlementHour?: number;
  timeZone?: string;
  source?: string;
  model?: string;
  endpoint?: string;
}

export interface QuotaSnapshot {
  capturedAt: string;
  amountLimit?: number;
  usedAmount?: number;
  remainingAmount?: number;
  period?: string;
  periodResetTime?: string;
  expireTime?: string;
  isActive: boolean;
  totalCost: number;
  totalRequests: number;
  totalTokens: number;
  lastRequestTime?: string;
  billingType?: string;
  remainingCount?: number;
  usedCount?: number;
  subTypeName?: string;
}

export type BroadcastPayload =
  | {
      type: "records";
      records: ApiInvocation[];
    }
  | {
      type: "pool_attempts";
      invokeId: string;
      attempts: ApiPoolUpstreamRequestAttempt[];
    }
  | {
      type: "summary";
      window: string;
      summary: StatsResponse;
    }
  | {
      type: "quota";
      snapshot: QuotaSnapshot;
    }
  | {
      type: "version";
      version: string;
    };

function appendInvocationRecordsQuery(
  search: URLSearchParams,
  query: InvocationRecordsQuery,
) {
  if (query.page != null) search.set("page", String(query.page));
  if (query.pageSize != null) search.set("pageSize", String(query.pageSize));
  if (query.snapshotId != null)
    search.set("snapshotId", String(query.snapshotId));
  if (query.anchorId) search.set("anchorId", query.anchorId);
  if (query.sortBy) search.set("sortBy", query.sortBy);
  if (query.sortOrder) search.set("sortOrder", query.sortOrder);
  if (query.rangePreset) search.set("rangePreset", query.rangePreset);
  if (query.from) search.set("from", query.from);
  if (query.to) search.set("to", query.to);
  if (query.model) search.set("model", query.model);
  if (query.status) search.set("status", query.status);
  if (query.endpoint) search.set("endpoint", query.endpoint);
  if (query.requestId) search.set("requestId", query.requestId);
  if (query.failureClass) search.set("failureClass", query.failureClass);
  if (query.failureKind) search.set("failureKind", query.failureKind);
  if (query.promptCacheKey) search.set("promptCacheKey", query.promptCacheKey);
  if (query.stickyKey) search.set("stickyKey", query.stickyKey);
  if (query.upstreamAccountId != null)
    search.set("upstreamAccountId", String(query.upstreamAccountId));
  if (query.requesterIp) search.set("requesterIp", query.requesterIp);
  if (query.keyword) search.set("keyword", query.keyword);
  if (query.minTotalTokens != null)
    search.set("minTotalTokens", String(query.minTotalTokens));
  if (query.maxTotalTokens != null)
    search.set("maxTotalTokens", String(query.maxTotalTokens));
  if (query.minTotalMs != null)
    search.set("minTotalMs", String(query.minTotalMs));
  if (query.maxTotalMs != null)
    search.set("maxTotalMs", String(query.maxTotalMs));
  if (query.suggestField) search.set("suggestField", query.suggestField);
  if (query.suggestQuery) search.set("suggestQuery", query.suggestQuery);
}

export async function fetchInvocations(
  limit: number,
  params?: { model?: string; status?: string },
) {
  const search = new URLSearchParams();
  search.set("limit", String(limit));
  if (params?.model) search.set("model", params.model);
  if (params?.status) search.set("status", params.status);

  return fetchJson<ListResponse>(`/api/invocations?${search.toString()}`);
}

export async function fetchInvocationRecords(query: InvocationRecordsQuery) {
  const search = new URLSearchParams();
  appendInvocationRecordsQuery(search, query);
  return fetchJson<InvocationRecordsResponse>(
    `/api/invocations?${search.toString()}`,
    { signal: query.signal },
  );
}

export async function fetchInvocationRecordLocation(
  query: InvocationRecordLocationQuery,
) {
  const search = new URLSearchParams({
    requestId: query.requestId,
    upstreamAccountId: String(query.upstreamAccountId),
    pageSize: String(query.pageSize ?? 50),
  });
  return fetchJson<InvocationRecordLocationResponse>(
    `/api/invocations/locate?${search.toString()}`,
    { signal: query.signal },
  );
}

export async function fetchInvocationRecordsSummary(
  query: InvocationRecordsQuery,
) {
  const search = new URLSearchParams();
  appendInvocationRecordsQuery(search, query);
  return fetchJson<InvocationRecordsSummaryResponse>(
    `/api/invocations/summary?${search.toString()}`,
    { signal: query.signal },
  );
}

export async function fetchInvocationRecordsNewCount(
  query: InvocationRecordsQuery,
) {
  const search = new URLSearchParams();
  appendInvocationRecordsQuery(search, query);
  return fetchJson<InvocationRecordsNewCountResponse>(
    `/api/invocations/new-count?${search.toString()}`,
  );
}

export async function fetchInvocationSuggestions(
  query: InvocationRecordsQuery,
) {
  const search = new URLSearchParams();
  appendInvocationRecordsQuery(search, query);
  return fetchJson<InvocationSuggestionsResponse>(
    `/api/invocations/suggestions?${search.toString()}`,
  );
}

export async function fetchInvocationPoolAttempts(invokeId: string) {
  return fetchJson<ApiPoolUpstreamRequestAttempt[]>(
    `/api/invocations/${encodeURIComponent(invokeId)}/pool-attempts`,
  );
}

export async function fetchUpstreamAccountAttempts(
  accountId: number,
  options?: { page?: number; pageSize?: number; signal?: AbortSignal },
) {
  const search = new URLSearchParams({
    page: String(options?.page ?? 1),
    pageSize: String(options?.pageSize ?? 50),
  });
  return fetchJson<UpstreamAccountAttemptListResponse>(
    `/api/pool/upstream-accounts/${encodeURIComponent(String(accountId))}/call-attempts?${search.toString()}`,
    { signal: options?.signal },
  );
}

export async function locateUpstreamAccountAttempt(
  accountId: number,
  attemptId: number,
  options?: { pageSize?: number; signal?: AbortSignal },
) {
  const search = new URLSearchParams({
    attemptId: String(attemptId),
    pageSize: String(options?.pageSize ?? 50),
  });
  return fetchJson<UpstreamAccountAttemptListResponse>(
    `/api/pool/upstream-accounts/${encodeURIComponent(String(accountId))}/call-attempts/locate?${search.toString()}`,
    { signal: options?.signal },
  );
}

export async function fetchInvocationRecordDetail(id: number) {
  return fetchJson<ApiInvocationRecordDetailResponse>(
    `/api/invocations/${encodeURIComponent(String(id))}/detail`,
  );
}

export async function fetchInvocationResponseBody(id: number) {
  return fetchJson<ApiInvocationResponseBodyResponse>(
    `/api/invocations/${encodeURIComponent(String(id))}/response-body`,
  );
}

export async function fetchStats() {
  return fetchJson<StatsResponse>("/api/stats");
}

export interface VersionResponse {
  backend: string;
  frontend: string;
}

export interface PricingEntry {
  model: string;
  inputPer1m: number;
  outputPer1m: number;
  cacheInputPer1m?: number | null;
  cacheReadPer1m?: number | null;
  cacheWritePer1m?: number | null;
  reasoningPer1m?: number | null;
  source: string;
}

export interface PricingSettings {
  catalogVersion: string;
  entries: PricingEntry[];
}

export type ProxyFastModeRewriteMode =
  | "disabled"
  | "fill_missing"
  | "force_priority";

export interface ProxySettings {
  hijackEnabled: boolean;
  mergeUpstreamEnabled: boolean;
  fastModeRewriteMode: ProxyFastModeRewriteMode;
  upstream429MaxRetries: number;
  websocketEnabled: boolean;
  upstreamWebsocketDefaultEnabled: boolean;
  requestBodyLoggingEnabled: boolean;
  responseBodyLoggingEnabled: boolean;
  encryptedSessionOwnerRoutingEnabled: boolean;
  defaultHijackEnabled: boolean;
  models: string[];
  enabledModels: string[];
}

export interface ForwardProxyWindowStats {
  attempts: number;
  successRate?: number;
  avgLatencyMs?: number;
}

export interface ForwardProxyNodeStats {
  oneMinute: ForwardProxyWindowStats;
  fifteenMinutes: ForwardProxyWindowStats;
  oneHour: ForwardProxyWindowStats;
  oneDay: ForwardProxyWindowStats;
  sevenDays: ForwardProxyWindowStats;
}

export interface ForwardProxyNode {
  key: string;
  source: string;
  displayName: string;
  endpointUrl?: string;
  weight: number;
  penalized: boolean;
  stats: ForwardProxyNodeStats;
}

export interface ForwardProxyBindingNode {
  key: string;
  aliasKeys?: string[];
  source: string;
  displayName: string;
  protocolLabel: string;
  egressIp?: string | null;
  egressIpCheckedAt?: string | null;
  egressIpProvider?: string | null;
  egressIpError?: string | null;
  egressIpErrorAt?: string | null;
  penalized: boolean;
  selectable: boolean;
  last24h: ForwardProxyHourlyBucket[];
}

export interface ForwardProxySettings {
  proxyUrls: string[];
  subscriptionUrls: string[];
  subscriptionUpdateIntervalSecs: number;
  nodes: ForwardProxyNode[];
}

export interface ForwardProxyRefreshSubscriptionsResult {
  forwardProxy: ForwardProxySettings;
  subscriptionCount: number;
  addedNodeCount: number;
  refreshedAt: string;
}

export interface ForwardProxyLatencyTargetResult {
  ok: boolean;
  latencyMs?: number;
  ip?: string;
  httpStatus?: number;
  error?: string;
}

export interface ForwardProxyLatencyTestNodeProgress {
  key: string;
  displayName: string;
  round: number;
  totalRounds: number;
  completedRounds: number;
  successCount: number;
  attemptCount: number;
  averageLatencyMs?: number;
  egressIp: ForwardProxyLatencyTargetResult;
  oauthUpstream: ForwardProxyLatencyTargetResult;
  codexResponses: ForwardProxyLatencyTargetResult;
  allTargetsOk: boolean;
  failedTargets: string[];
  done: boolean;
  timedOut: boolean;
  message: string;
}

export interface ForwardProxyLatencyTestStreamEvent {
  kind: "progress" | "completed";
  node: ForwardProxyLatencyTestNodeProgress;
}

export interface ForwardProxyHourlyBucket {
  bucketStart: string;
  bucketEnd: string;
  successCount: number;
  failureCount: number;
}

export interface ForwardProxyWeightBucket {
  bucketStart: string;
  bucketEnd: string;
  sampleCount: number;
  minWeight: number;
  maxWeight: number;
  avgWeight: number;
  lastWeight: number;
}

export interface ForwardProxyLiveNode {
  key: string;
  source: string;
  displayName: string;
  endpointUrl?: string;
  weight: number;
  penalized: boolean;
  stats: ForwardProxyNodeStats;
  last24h: ForwardProxyHourlyBucket[];
  weight24h: ForwardProxyWeightBucket[];
}

export interface ForwardProxyLiveStatsResponse {
  rangeStart: string;
  rangeEnd: string;
  bucketSeconds: number;
  nodes: ForwardProxyLiveNode[];
}

export interface ForwardProxyTimeseriesNode {
  key: string;
  source: string;
  displayName: string;
  endpointUrl?: string;
  weight: number;
  penalized: boolean;
  buckets: ForwardProxyHourlyBucket[];
  weightBuckets: ForwardProxyWeightBucket[];
}

export interface ForwardProxyTimeseriesResponse {
  rangeStart: string;
  rangeEnd: string;
  bucketSeconds: number;
  effectiveBucket: string;
  availableBuckets: string[];
  nodes: ForwardProxyTimeseriesNode[];
}

export interface ConversationRequestPoint {
  occurredAt: string;
  status: string;
  isSuccess: boolean;
  outcome?: ConversationRequestOutcome | null;
  requestTokens: number;
  cumulativeTokens: number;
}

export type ConversationRequestOutcome =
  | "success"
  | "failure"
  | "neutral"
  | "in_flight";

export type PromptCacheConversationRequestPoint = ConversationRequestPoint;

export type StickyKeyConversationRequestPoint = ConversationRequestPoint;

export interface PromptCacheConversationUpstreamAccount {
  upstreamAccountId: number | null;
  upstreamAccountName: string | null;
  requestCount: number;
  totalTokens: number;
  totalCost: number;
  lastActivityAt: string;
}

export interface PromptCacheConversationInvocationPreview {
  id: number;
  invokeId: string;
  promptCacheKey?: string | null;
  occurredAt: string;
  status: string;
  livePhase?: InvocationLivePhase | null;
  failureClass: Exclude<ApiInvocation["failureClass"], undefined> | null;
  routeMode: string | null;
  model: string | null;
  requestModel?: string | null;
  responseModel?: string | null;
  totalTokens: number;
  cost: number | null;
  proxyDisplayName: string | null;
  upstreamAccountId: number | null;
  upstreamAccountName: string | null;
  upstreamAccountPlanType?: string | null;
  endpoint: string | null;
  compactionRequestKind?: ApiInvocation["compactionRequestKind"];
  compactionResponseKind?: ApiInvocation["compactionResponseKind"];
  imageIntent?: ApiInvocation["imageIntent"];
  source?: ApiInvocation["source"];
  inputTokens?: ApiInvocation["inputTokens"];
  outputTokens?: ApiInvocation["outputTokens"];
  cacheInputTokens?: ApiInvocation["cacheInputTokens"];
  cacheWriteTokens?: ApiInvocation["cacheWriteTokens"];
  costInput?: ApiInvocation["costInput"];
  costCacheWrite?: ApiInvocation["costCacheWrite"];
  costCacheRead?: ApiInvocation["costCacheRead"];
  costOutput?: ApiInvocation["costOutput"];
  costReasoning?: ApiInvocation["costReasoning"];
  reasoningTokens?: ApiInvocation["reasoningTokens"];
  reasoningEffort?: ApiInvocation["reasoningEffort"];
  errorMessage?: ApiInvocation["errorMessage"];
  downstreamStatusCode?: ApiInvocation["downstreamStatusCode"];
  downstreamErrorMessage?: ApiInvocation["downstreamErrorMessage"];
  failureKind?: ApiInvocation["failureKind"];
  isActionable?: ApiInvocation["isActionable"];
  responseContentEncoding?: ApiInvocation["responseContentEncoding"];
  transport?: ApiInvocation["transport"];
  requestedServiceTier?: ApiInvocation["requestedServiceTier"];
  serviceTier?: ApiInvocation["serviceTier"];
  billingServiceTier?: ApiInvocation["billingServiceTier"];
  tReqReadMs?: ApiInvocation["tReqReadMs"];
  tReqParseMs?: ApiInvocation["tReqParseMs"];
  tUpstreamConnectMs?: ApiInvocation["tUpstreamConnectMs"];
  tUpstreamTtfbMs?: ApiInvocation["tUpstreamTtfbMs"];
  tUpstreamStreamMs?: ApiInvocation["tUpstreamStreamMs"];
  tRespParseMs?: ApiInvocation["tRespParseMs"];
  tPersistMs?: ApiInvocation["tPersistMs"];
  tTotalMs?: ApiInvocation["tTotalMs"];
}

export interface PromptCacheConversation {
  promptCacheKey: string;
  requestCount: number;
  totalTokens: number;
  totalCost: number;
  createdAt: string;
  lastActivityAt: string;
  lastTerminalAt?: string | null;
  lastInFlightAt?: string | null;
  cursor?: string | null;
  hasEncryptedSessionOwner: boolean;
  encryptedOwnerAccountId?: number | null;
  encryptedOwnerAccountName?: string | null;
  encryptedOwnerGroupName?: string | null;
  upstreamAccounts: PromptCacheConversationUpstreamAccount[];
  recentInvocations: PromptCacheConversationInvocationPreview[];
  last24hRequests: PromptCacheConversationRequestPoint[];
}

export type PromptCacheConversationBindingKind =
  | "none"
  | "group"
  | "upstreamAccount";
export type PromptCacheConversationRewriteMode =
  | "force_remove"
  | "keep_original"
  | "fill_missing"
  | "force_add";

export interface PromptCacheConversationBindingResponse {
  promptCacheKey: string;
  bindingKind: PromptCacheConversationBindingKind;
  groupName: string | null;
  upstreamAccountId: number | null;
  upstreamAccountName: string | null;
  hasEncryptedSessionOwner: boolean;
  encryptedOwnerAccountId: number | null;
  encryptedOwnerAccountName: string | null;
  encryptedOwnerGroupName: string | null;
  timeouts: PoolRoutingTimeoutSettings;
  timeoutFieldSources: EffectiveRoutingTimeoutFieldSources;
  allowSwitchUpstream?: boolean | null;
  fastModeRewriteMode?: PromptCacheConversationRewriteMode | null;
  imageToolRewriteMode?: PromptCacheConversationRewriteMode | null;
  availableModels?: string[] | null;
  forwardProxyKey?: string | null;
  forwardProxyKeys?: string[];
  policyFieldSources?: {
    allowSwitchUpstream: EffectiveRoutingRuleSource;
    fastModeRewriteMode: EffectiveRoutingRuleSource;
    imageToolRewriteMode: EffectiveRoutingRuleSource;
    availableModels: EffectiveRoutingRuleSource;
    forwardProxyKey: EffectiveRoutingRuleSource;
  };
  updatedAt: string | null;
}

export type PromptCacheConversationBindingTimeoutPatch = {
  responsesFirstByteTimeoutSecs?: number | null;
  compactFirstByteTimeoutSecs?: number | null;
  responsesStreamTimeoutSecs?: number | null;
  compactStreamTimeoutSecs?: number | null;
};

export type UpdatePromptCacheConversationBindingPayload =
  | {
      bindingKind: "none";
      timeouts?: PromptCacheConversationBindingTimeoutPatch;
      allowSwitchUpstream?: boolean | null;
      fastModeRewriteMode?: PromptCacheConversationRewriteMode | null;
      imageToolRewriteMode?: PromptCacheConversationRewriteMode | null;
      availableModels?: string[] | null;
      forwardProxyKey?: string | null;
      forwardProxyKeys?: string[] | null;
    }
  | {
      bindingKind: "group";
      groupName: string;
      timeouts?: PromptCacheConversationBindingTimeoutPatch;
      allowSwitchUpstream?: boolean | null;
      fastModeRewriteMode?: PromptCacheConversationRewriteMode | null;
      imageToolRewriteMode?: PromptCacheConversationRewriteMode | null;
      availableModels?: string[] | null;
      forwardProxyKey?: string | null;
      forwardProxyKeys?: string[] | null;
    }
  | {
      bindingKind: "upstreamAccount";
      upstreamAccountId: number;
      timeouts?: PromptCacheConversationBindingTimeoutPatch;
      allowSwitchUpstream?: boolean | null;
      fastModeRewriteMode?: PromptCacheConversationRewriteMode | null;
      imageToolRewriteMode?: PromptCacheConversationRewriteMode | null;
      availableModels?: string[] | null;
      forwardProxyKey?: string | null;
      forwardProxyKeys?: string[] | null;
    };

export type PromptCacheConversationSelectionMode = "count" | "activityWindow";
export type PromptCacheConversationDetailLevel = "full" | "compact";

export type PromptCacheConversationImplicitFilterKind =
  | "inactiveOutside24h"
  | "cappedTo50";

export interface PromptCacheConversationImplicitFilter {
  kind: PromptCacheConversationImplicitFilterKind | null;
  filteredCount: number;
}

export type PromptCacheConversationSelection =
  | { mode: "count"; limit: number }
  | { mode: "activityWindow"; activityHours: number }
  | { mode: "activityWindow"; activityMinutes: number };

export interface PromptCacheConversationsResponse {
  rangeStart: string;
  rangeEnd: string;
  snapshotAt?: string | null;
  selectionMode: PromptCacheConversationSelectionMode;
  selectedLimit: number | null;
  selectedActivityHours: number | null;
  selectedActivityMinutes?: number | null;
  implicitFilter: PromptCacheConversationImplicitFilter;
  totalMatched?: number | null;
  hasMore?: boolean;
  nextCursor?: string | null;
  conversations: PromptCacheConversation[];
}

export interface PromptCacheConversationPageQuery {
  pageSize?: number;
  cursor?: string | null;
  snapshotAt?: string | null;
  detail?: PromptCacheConversationDetailLevel;
  recentInvocationLimit?: number;
  signal?: AbortSignal;
}

export type StickyKeyConversationSelectionMode =
  PromptCacheConversationSelectionMode;

export type StickyKeyConversationImplicitFilterKind =
  PromptCacheConversationImplicitFilterKind;

export interface StickyKeyConversationImplicitFilter {
  kind: StickyKeyConversationImplicitFilterKind | null;
  filteredCount: number;
}

export type StickyKeyConversationSelection =
  | { mode: "count"; limit: number }
  | { mode: "activityWindow"; activityHours: number };

export type StickyKeyConversationInvocationPreview =
  PromptCacheConversationInvocationPreview;

export interface StickyKeyConversation {
  stickyKey: string;
  requestCount: number;
  totalTokens: number;
  totalCost: number;
  createdAt: string;
  lastActivityAt: string;
  recentInvocations: StickyKeyConversationInvocationPreview[];
  last24hRequests: StickyKeyConversationRequestPoint[];
}

export interface UpstreamStickyConversationsResponse {
  rangeStart: string;
  rangeEnd: string;
  selectionMode: StickyKeyConversationSelectionMode;
  selectedLimit: number | null;
  selectedActivityHours: number | null;
  implicitFilter: StickyKeyConversationImplicitFilter;
  conversations: StickyKeyConversation[];
}

export type ForwardProxyValidationKind = "proxyUrl" | "subscriptionUrl";

export interface ForwardProxyValidationResult {
  ok: boolean;
  message: string;
  normalizedValue?: string;
  discoveredNodes?: number;
  latencyMs?: number;
}

function forwardProxyValidationTimeoutMs(
  kind: ForwardProxyValidationKind,
): number {
  return kind === "subscriptionUrl"
    ? FORWARD_PROXY_SUBSCRIPTION_VALIDATION_TIMEOUT_MS
    : FORWARD_PROXY_VALIDATION_TIMEOUT_MS;
}

export interface SettingsPayload {
  proxy: ProxySettings;
  forwardProxy: ForwardProxySettings;
  pricing: PricingSettings;
}

export interface SystemStatusMetric {
  count: number;
  bytes: number;
}

export interface SystemStatusResponse {
  liveInvocationsCount: number;
  successCount: number;
  nonSuccessCount: number;
  completedArchiveBatchesCount: number;
  archivedBodies: SystemStatusMetric;
  rawBodies: SystemStatusMetric;
  requestRawBodies: SystemStatusMetric;
  responseRawBodies: SystemStatusMetric;
  databaseBytes: number;
  otherFilesBytes: number;
  refreshedAt: string;
}

export interface SystemTaskRun {
  id: number;
  taskKind: string;
  triggerKind: string;
  status: string;
  summary?: string;
  detail?: string;
  startedAt: string;
  finishedAt?: string;
  durationMs?: number;
}

export interface SystemTaskRunsResponse {
  items: SystemTaskRun[];
  total: number;
  page: number;
  pageSize: number;
}

export interface ExternalApiKeySummary {
  id: number;
  name: string;
  status: string;
  prefix: string;
  lastUsedAt?: string;
  createdAt: string;
  updatedAt: string;
}

export interface ExternalApiKeyListResponse {
  items: ExternalApiKeySummary[];
}

export interface ExternalApiKeyMutationResponse {
  key: ExternalApiKeySummary;
}

export interface ExternalApiKeySecretResponse {
  key: ExternalApiKeySummary;
  secret: string;
}

export function normalizeStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((item): item is string => typeof item === "string");
}

export function normalizeFiniteNumber(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value)) return undefined;
  return value;
}

function normalizeInvocationLivePhase(value: unknown): InvocationLivePhase | null {
  if (typeof value !== "string") return null;
  const phase = value.trim().toLowerCase();
  if (phase === "queued" || phase === "requesting" || phase === "responding") {
    return phase;
  }
  return null;
}

function normalizeInvocationPhaseCounts(value: unknown): InvocationPhaseCounts | null {
  if (!value || typeof value !== "object") return null;
  const payload = value as Record<string, unknown>;
  return {
    queued: Math.max(0, normalizeFiniteNumber(payload.queued) ?? 0),
    requesting: Math.max(0, normalizeFiniteNumber(payload.requesting) ?? 0),
    responding: Math.max(0, normalizeFiniteNumber(payload.responding) ?? 0),
  };
}

function normalizeUsageCostBreakdown(raw: unknown): UsageCostBreakdown | null {
  if (!raw || typeof raw !== 'object') return null
  const payload = raw as Record<string, unknown>
  const input = normalizeFiniteNumber(payload.input)
  const cacheWrite = normalizeFiniteNumber(payload.cacheWrite)
  const cacheRead = normalizeFiniteNumber(payload.cacheRead)
  const output = normalizeFiniteNumber(payload.output)
  const reasoning = normalizeFiniteNumber(payload.reasoning)
  if ([input, cacheWrite, cacheRead, output, reasoning].some((value) => value == null)) return null
  return {
    input: input ?? 0,
    cacheWrite: cacheWrite ?? 0,
    cacheRead: cacheRead ?? 0,
    output: output ?? 0,
    reasoning: reasoning ?? 0,
    unknown: normalizeFiniteNumber(payload.unknown) ?? 0,
  }
}

function normalizeUsageBreakdown(raw: unknown): UsageBreakdown | null {
  if (!raw || typeof raw !== 'object') return null
  const payload = raw as Record<string, unknown>
  const models = Array.isArray(payload.models)
    ? payload.models.flatMap((rawModel) => {
        const model = (rawModel ?? {}) as Record<string, unknown>
        const name = typeof model.model === 'string' ? model.model.trim() : ''
        if (!name) return []
        return [{
          model: name,
          reasoningEffort: typeof model.reasoningEffort === 'string' && model.reasoningEffort.trim()
            ? model.reasoningEffort.trim()
            : null,
          cacheWriteTokens: normalizeFiniteNumber(model.cacheWriteTokens) ?? 0,
          cacheReadTokens: normalizeFiniteNumber(model.cacheReadTokens) ?? 0,
          outputTokens: normalizeFiniteNumber(model.outputTokens) ?? 0,
          costs: normalizeUsageCostBreakdown(model.costs),
        }]
      })
    : []
  return {
    cacheWriteTokens: normalizeFiniteNumber(payload.cacheWriteTokens) ?? 0,
    cacheReadTokens: normalizeFiniteNumber(payload.cacheReadTokens) ?? 0,
    outputTokens: normalizeFiniteNumber(payload.outputTokens) ?? 0,
    costs: normalizeUsageCostBreakdown(payload.costs),
    models,
  }
}

function normalizeTimeseriesPoint(raw: unknown): TimeseriesPoint | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const bucketStart =
    typeof payload.bucketStart === "string" ? payload.bucketStart : "";
  const bucketEnd =
    typeof payload.bucketEnd === "string" ? payload.bucketEnd : "";
  if (!bucketStart || !bucketEnd) return null;
  const totalCount = normalizeFiniteNumber(payload.totalCount) ?? 0;
  const successCount = normalizeFiniteNumber(payload.successCount) ?? 0;
  const failureCount = normalizeFiniteNumber(payload.failureCount) ?? 0;
  const inFlightCount = normalizeFiniteNumber(payload.inFlightCount) ?? 0;
  const inFlightPhaseCounts = normalizeInvocationPhaseCounts(
    payload.inFlightPhaseCounts,
  );
  const hasCalls =
    Math.max(totalCount, successCount + failureCount + Math.max(inFlightCount, 0)) >
    0;
  return {
    bucketStart,
    bucketEnd,
    totalCount,
    successCount,
    failureCount,
    inFlightCount,
    inFlightPhaseCounts,
    totalTokens: normalizeFiniteNumber(payload.totalTokens) ?? 0,
    cacheInputTokens: normalizeFiniteNumber(payload.cacheInputTokens) ?? 0,
    totalCost: normalizeFiniteNumber(payload.totalCost) ?? 0,
    nonSuccessCost: normalizeFiniteNumber(payload.nonSuccessCost) ?? 0,
    avgTotalMs: hasCalls
      ? (normalizeFiniteNumber(payload.avgTotalMs) ?? null)
      : null,
    totalLatencySampleCount:
      hasCalls
        ? (normalizeFiniteNumber(payload.totalLatencySampleCount) ?? null)
        : null,
    firstByteSampleCount: hasCalls
      ? (normalizeFiniteNumber(payload.firstByteSampleCount) ?? 0)
      : 0,
    firstByteAvgMs: hasCalls
      ? (normalizeFiniteNumber(payload.firstByteAvgMs) ?? null)
      : null,
    firstByteP95Ms: hasCalls
      ? (normalizeFiniteNumber(payload.firstByteP95Ms) ?? null)
      : null,
    firstResponseByteTotalSampleCount: hasCalls
      ? (normalizeFiniteNumber(payload.firstResponseByteTotalSampleCount) ?? 0)
      : 0,
    firstResponseByteTotalAvgMs: hasCalls
      ? (normalizeFiniteNumber(payload.firstResponseByteTotalAvgMs) ?? null)
      : null,
    firstResponseByteTotalP95Ms: hasCalls
      ? (normalizeFiniteNumber(payload.firstResponseByteTotalP95Ms) ?? null)
      : null,
  };
}

function normalizeTimeseriesResponse(raw: unknown): TimeseriesResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const pointsRaw = Array.isArray(payload.points) ? payload.points : [];
  return {
    rangeStart:
      typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    bucketSeconds: normalizeFiniteNumber(payload.bucketSeconds) ?? 3600,
    snapshotId: normalizeFiniteNumber(payload.snapshotId) ?? undefined,
    effectiveBucket:
      typeof payload.effectiveBucket === "string"
        ? payload.effectiveBucket
        : undefined,
    availableBuckets: normalizeStringArray(payload.availableBuckets),
    bucketLimitedToDaily: payload.bucketLimitedToDaily === true,
    points: pointsRaw
      .map(normalizeTimeseriesPoint)
      .filter((point): point is TimeseriesPoint => point != null),
  };
}

function normalizeParallelWorkPoint(raw: unknown): ParallelWorkPoint | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const bucketStart =
    typeof payload.bucketStart === "string" ? payload.bucketStart : "";
  const bucketEnd =
    typeof payload.bucketEnd === "string" ? payload.bucketEnd : "";
  if (!bucketStart || !bucketEnd) return null;
  return {
    bucketStart,
    bucketEnd,
    parallelCount: normalizeFiniteNumber(payload.parallelCount) ?? 0,
  };
}

function normalizeParallelWorkConversation(
  raw: unknown,
): ParallelWorkConversation | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const conversationId =
    typeof payload.conversationId === "string" ? payload.conversationId : "";
  const start = typeof payload.start === "string" ? payload.start : "";
  const end = typeof payload.end === "string" ? payload.end : "";
  if (!conversationId || !start || !end) return null;
  return {
    conversationId,
    start,
    end,
    requestCount: normalizeFiniteNumber(payload.requestCount) ?? 0,
  };
}

function normalizeParallelWorkWindowResponse(
  raw: unknown,
): ParallelWorkWindowResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const pointsRaw = Array.isArray(payload.points) ? payload.points : [];
  const conversationsRaw = Array.isArray(payload.conversations)
    ? payload.conversations
    : [];
  const effectiveTimeZone =
    typeof payload.effectiveTimeZone === "string" &&
    payload.effectiveTimeZone.trim()
      ? payload.effectiveTimeZone.trim()
      : "Asia/Shanghai";
  return {
    rangeStart:
      typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    bucketSeconds: normalizeFiniteNumber(payload.bucketSeconds) ?? 0,
    completeBucketCount:
      normalizeFiniteNumber(payload.completeBucketCount) ?? 0,
    activeBucketCount: normalizeFiniteNumber(payload.activeBucketCount) ?? 0,
    minCount:
      payload.minCount == null
        ? null
        : (normalizeFiniteNumber(payload.minCount) ?? null),
    maxCount:
      payload.maxCount == null
        ? null
        : (normalizeFiniteNumber(payload.maxCount) ?? null),
    avgCount:
      payload.avgCount == null
        ? null
        : (normalizeFiniteNumber(payload.avgCount) ?? null),
    effectiveTimeZone,
    timeZoneFallback: payload.timeZoneFallback === true,
    points: pointsRaw
      .map(normalizeParallelWorkPoint)
      .filter((point): point is ParallelWorkPoint => point != null),
    conversations: conversationsRaw
      .map(normalizeParallelWorkConversation)
      .filter(
        (conversation): conversation is ParallelWorkConversation =>
          conversation != null,
      ),
  };
}

function normalizeParallelWorkStatsResponse(
  raw: unknown,
): ParallelWorkStatsResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const current = normalizeParallelWorkWindowResponse(
    payload.current ?? payload.minute7d,
  );
  return {
    current,
    minute7d: normalizeParallelWorkWindowResponse(payload.minute7d ?? current),
    hour30d: normalizeParallelWorkWindowResponse(payload.hour30d ?? current),
    dayAll: normalizeParallelWorkWindowResponse(payload.dayAll ?? current),
  };
}

function normalizePricingEntry(raw: unknown): PricingEntry | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const model = typeof payload.model === "string" ? payload.model.trim() : "";
  const inputPer1m = normalizeFiniteNumber(payload.inputPer1m);
  const outputPer1m = normalizeFiniteNumber(payload.outputPer1m);
  if (!model || inputPer1m === undefined || outputPer1m === undefined)
    return null;
  const legacyCacheInputPer1m = normalizeFiniteNumber(payload.cacheInputPer1m);
  const cacheReadPer1m =
    normalizeFiniteNumber(payload.cacheReadPer1m) ?? legacyCacheInputPer1m;
  const cacheWritePer1m = normalizeFiniteNumber(payload.cacheWritePer1m);
  const reasoningPer1m = normalizeFiniteNumber(payload.reasoningPer1m);
  return {
    model,
    inputPer1m,
    outputPer1m,
    cacheInputPer1m: cacheReadPer1m ?? null,
    cacheReadPer1m: cacheReadPer1m ?? null,
    cacheWritePer1m: cacheWritePer1m ?? null,
    reasoningPer1m: reasoningPer1m ?? null,
    source:
      typeof payload.source === "string" && payload.source.trim()
        ? payload.source.trim()
        : "custom",
  };
}

function normalizePricingSettings(raw: unknown): PricingSettings {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const entriesRaw = Array.isArray(payload.entries) ? payload.entries : [];
  const entries = entriesRaw
    .map(normalizePricingEntry)
    .filter((entry): entry is PricingEntry => entry != null)
    .sort((a, b) => a.model.localeCompare(b.model));
  return {
    catalogVersion:
      typeof payload.catalogVersion === "string" &&
      payload.catalogVersion.trim()
        ? payload.catalogVersion.trim()
        : "custom",
    entries,
  };
}

function normalizeProxyFastModeRewriteMode(
  raw: unknown,
): ProxyFastModeRewriteMode {
  return raw === "fill_missing" || raw === "force_priority"
    ? raw
    : "disabled";
}

function normalizeProxySettings(raw: unknown): ProxySettings {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const models = normalizeStringArray(payload.models);
  const enabledModelSet = new Set(normalizeStringArray(payload.enabledModels));
  return {
    hijackEnabled: payload.hijackEnabled === true,
    mergeUpstreamEnabled: payload.mergeUpstreamEnabled === true,
    fastModeRewriteMode: normalizeProxyFastModeRewriteMode(
      payload.fastModeRewriteMode,
    ),
    upstream429MaxRetries: Math.max(
      0,
      Math.min(5, Math.trunc(normalizeFiniteNumber(payload.upstream429MaxRetries) ?? 3)),
    ),
    websocketEnabled: payload.websocketEnabled === true,
    upstreamWebsocketDefaultEnabled: payload.upstreamWebsocketDefaultEnabled === true,
    requestBodyLoggingEnabled: payload.requestBodyLoggingEnabled !== false,
    responseBodyLoggingEnabled: payload.responseBodyLoggingEnabled !== false,
    encryptedSessionOwnerRoutingEnabled:
      payload.encryptedSessionOwnerRoutingEnabled === true,
    defaultHijackEnabled: payload.defaultHijackEnabled === true,
    models,
    enabledModels: models.filter((model) => enabledModelSet.has(model)),
  };
}

function normalizeForwardProxyWindowStats(
  raw: unknown,
): ForwardProxyWindowStats {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const attempts = normalizeFiniteNumber(payload.attempts) ?? 0;
  const successRate = normalizeFiniteNumber(payload.successRate);
  const avgLatencyMs = normalizeFiniteNumber(payload.avgLatencyMs);
  return {
    attempts,
    successRate,
    avgLatencyMs,
  };
}

function emptyForwardProxyNodeStats(): ForwardProxyNodeStats {
  return {
    oneMinute: { attempts: 0 },
    fifteenMinutes: { attempts: 0 },
    oneHour: { attempts: 0 },
    oneDay: { attempts: 0 },
    sevenDays: { attempts: 0 },
  };
}

function normalizeForwardProxyNode(raw: unknown): ForwardProxyNode | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const key = typeof payload.key === "string" ? payload.key : "";
  if (!key) return null;
  const statsPayload = (payload.stats ?? {}) as Record<string, unknown>;
  return {
    key,
    source: typeof payload.source === "string" ? payload.source : "manual",
    displayName:
      typeof payload.displayName === "string" ? payload.displayName : key,
    endpointUrl:
      typeof payload.endpointUrl === "string" ? payload.endpointUrl : undefined,
    weight: normalizeFiniteNumber(payload.weight) ?? 0,
    penalized: Boolean(payload.penalized),
    stats: {
      oneMinute: normalizeForwardProxyWindowStats(statsPayload.oneMinute),
      fifteenMinutes: normalizeForwardProxyWindowStats(
        statsPayload.fifteenMinutes,
      ),
      oneHour: normalizeForwardProxyWindowStats(statsPayload.oneHour),
      oneDay: normalizeForwardProxyWindowStats(statsPayload.oneDay),
      sevenDays: normalizeForwardProxyWindowStats(statsPayload.sevenDays),
    },
  };
}

export function normalizeForwardProxyBindingNode(
  raw: unknown,
): ForwardProxyBindingNode | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const key = typeof payload.key === "string" ? payload.key.trim() : "";
  if (!key) return null;
  const bucketsRaw = Array.isArray(payload.last24h) ? payload.last24h : [];
  return {
    key,
    aliasKeys: normalizeStringArray(payload.aliasKeys),
    source: typeof payload.source === "string" ? payload.source : "manual",
    displayName:
      typeof payload.displayName === "string" && payload.displayName.trim()
        ? payload.displayName.trim()
        : key,
    protocolLabel: normalizeForwardProxyProtocolLabel(
      typeof payload.protocolLabel === "string"
        ? payload.protocolLabel
        : undefined,
    ),
    egressIp: typeof payload.egressIp === "string" ? payload.egressIp : null,
    egressIpCheckedAt:
      typeof payload.egressIpCheckedAt === "string"
        ? payload.egressIpCheckedAt
        : null,
    egressIpProvider:
      typeof payload.egressIpProvider === "string"
        ? payload.egressIpProvider
        : null,
    egressIpError:
      typeof payload.egressIpError === "string" ? payload.egressIpError : null,
    egressIpErrorAt:
      typeof payload.egressIpErrorAt === "string"
        ? payload.egressIpErrorAt
        : null,
    penalized: Boolean(payload.penalized),
    selectable: payload.selectable === true,
    last24h: bucketsRaw
      .map(normalizeForwardProxyHourlyBucket)
      .filter((item): item is ForwardProxyHourlyBucket => item != null),
  };
}

function normalizeForwardProxySettings(raw: unknown): ForwardProxySettings {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const nodesRaw = Array.isArray(payload.nodes) ? payload.nodes : [];
  const nodes = nodesRaw
    .map(normalizeForwardProxyNode)
    .filter((node): node is ForwardProxyNode => node != null)
    .sort((a, b) => a.displayName.localeCompare(b.displayName));
  return {
    proxyUrls: normalizeStringArray(payload.proxyUrls),
    subscriptionUrls: normalizeStringArray(payload.subscriptionUrls),
    subscriptionUpdateIntervalSecs:
      normalizeFiniteNumber(payload.subscriptionUpdateIntervalSecs) ?? 3600,
    nodes: nodes.map((node) => ({
      ...node,
      stats: node.stats ?? emptyForwardProxyNodeStats(),
    })),
  };
}

function normalizeForwardProxyLatencyTargetResult(
  raw: unknown,
): ForwardProxyLatencyTargetResult {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const latencyMs = normalizeFiniteNumber(payload.latencyMs);
  const httpStatus = normalizeFiniteNumber(payload.httpStatus);
  return {
    ok: payload.ok === true,
    latencyMs,
    ip: typeof payload.ip === "string" ? payload.ip : undefined,
    httpStatus:
      httpStatus == null ? undefined : Math.max(0, Math.trunc(httpStatus)),
    error: typeof payload.error === "string" ? payload.error : undefined,
  };
}

export function normalizeForwardProxyLatencyTestStreamEvent(
  raw: unknown,
): ForwardProxyLatencyTestStreamEvent | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const nodeRaw = (payload.node ?? {}) as Record<string, unknown>;
  const key = typeof nodeRaw.key === "string" ? nodeRaw.key : "";
  if (!key) return null;
  const kind = payload.kind === "completed" ? "completed" : "progress";
  const egressIp = normalizeForwardProxyLatencyTargetResult(nodeRaw.egressIp);
  const oauthUpstream = normalizeForwardProxyLatencyTargetResult(
    nodeRaw.oauthUpstream,
  );
  const codexResponses = normalizeForwardProxyLatencyTargetResult(
    nodeRaw.codexResponses,
  );
  const derivedFailedTargets = [
    egressIp.ok ? null : "egressIp",
    oauthUpstream.ok ? null : "oauthUpstream",
    codexResponses.ok ? null : "codexResponses",
  ].filter((target): target is string => target != null);
  const failedTargets =
    Array.isArray(nodeRaw.failedTargets) && nodeRaw.failedTargets.length > 0
      ? normalizeStringArray(nodeRaw.failedTargets)
      : derivedFailedTargets;
  return {
    kind,
    node: {
      key,
      displayName:
        typeof nodeRaw.displayName === "string" ? nodeRaw.displayName : key,
      round: normalizeFiniteNumber(nodeRaw.round) ?? 0,
      totalRounds: normalizeFiniteNumber(nodeRaw.totalRounds) ?? 5,
      completedRounds: normalizeFiniteNumber(nodeRaw.completedRounds) ?? 0,
      successCount: normalizeFiniteNumber(nodeRaw.successCount) ?? 0,
      attemptCount: normalizeFiniteNumber(nodeRaw.attemptCount) ?? 0,
      averageLatencyMs:
        normalizeFiniteNumber(nodeRaw.averageLatencyMs) ?? undefined,
      egressIp,
      oauthUpstream,
      codexResponses,
      allTargetsOk:
        typeof nodeRaw.allTargetsOk === "boolean"
          ? nodeRaw.allTargetsOk
          : failedTargets.length === 0,
      failedTargets,
      done: nodeRaw.done === true || kind === "completed",
      timedOut: nodeRaw.timedOut === true,
      message: typeof nodeRaw.message === "string" ? nodeRaw.message : "",
    },
  };
}

function normalizeForwardProxyRefreshSubscriptionsResult(
  raw: unknown,
): ForwardProxyRefreshSubscriptionsResult {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    forwardProxy: normalizeForwardProxySettings(payload.forwardProxy),
    subscriptionCount: normalizeFiniteNumber(payload.subscriptionCount) ?? 0,
    addedNodeCount: normalizeFiniteNumber(payload.addedNodeCount) ?? 0,
    refreshedAt:
      typeof payload.refreshedAt === "string" ? payload.refreshedAt : "",
  };
}

function normalizeForwardProxyHourlyBucket(
  raw: unknown,
): ForwardProxyHourlyBucket | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const bucketStart =
    typeof payload.bucketStart === "string" ? payload.bucketStart : "";
  const bucketEnd =
    typeof payload.bucketEnd === "string" ? payload.bucketEnd : "";
  if (!bucketStart || !bucketEnd) return null;
  return {
    bucketStart,
    bucketEnd,
    successCount: normalizeFiniteNumber(payload.successCount) ?? 0,
    failureCount: normalizeFiniteNumber(payload.failureCount) ?? 0,
  };
}

function normalizeForwardProxyWeightBucket(
  raw: unknown,
): ForwardProxyWeightBucket | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const bucketStart =
    typeof payload.bucketStart === "string" ? payload.bucketStart : "";
  const bucketEnd =
    typeof payload.bucketEnd === "string" ? payload.bucketEnd : "";
  if (!bucketStart || !bucketEnd) return null;
  const sampleCount = normalizeFiniteNumber(payload.sampleCount) ?? 0;
  const minWeight = normalizeFiniteNumber(payload.minWeight);
  const maxWeight = normalizeFiniteNumber(payload.maxWeight);
  const avgWeight = normalizeFiniteNumber(payload.avgWeight);
  const lastWeight = normalizeFiniteNumber(payload.lastWeight);
  if (
    minWeight === undefined ||
    maxWeight === undefined ||
    avgWeight === undefined ||
    lastWeight === undefined
  ) {
    return null;
  }
  return {
    bucketStart,
    bucketEnd,
    sampleCount,
    minWeight,
    maxWeight,
    avgWeight,
    lastWeight,
  };
}

function normalizeForwardProxyLiveNode(
  raw: unknown,
): ForwardProxyLiveNode | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const base = normalizeForwardProxyNode(raw);
  if (!base) return null;
  const bucketsRaw = Array.isArray(payload.last24h) ? payload.last24h : [];
  const last24h = bucketsRaw
    .map(normalizeForwardProxyHourlyBucket)
    .filter((item): item is ForwardProxyHourlyBucket => item != null);
  const weightBucketsRaw = Array.isArray(payload.weight24h)
    ? payload.weight24h
    : [];
  const weight24h = weightBucketsRaw
    .map(normalizeForwardProxyWeightBucket)
    .filter((item): item is ForwardProxyWeightBucket => item != null);
  return {
    key: base.key,
    source: base.source,
    displayName: base.displayName,
    endpointUrl: base.endpointUrl,
    weight: base.weight,
    penalized: base.penalized,
    stats: base.stats,
    last24h,
    weight24h,
  };
}

function normalizeForwardProxyLiveStatsResponse(
  raw: unknown,
): ForwardProxyLiveStatsResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const nodesRaw = Array.isArray(payload.nodes) ? payload.nodes : [];
  const nodes = nodesRaw
    .map(normalizeForwardProxyLiveNode)
    .filter((node): node is ForwardProxyLiveNode => node != null)
    .sort((a, b) => a.displayName.localeCompare(b.displayName));
  return {
    rangeStart:
      typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    bucketSeconds: normalizeFiniteNumber(payload.bucketSeconds) ?? 3600,
    nodes,
  };
}

function normalizeForwardProxyTimeseriesNode(
  raw: unknown,
): ForwardProxyTimeseriesNode | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const base = normalizeForwardProxyNode(raw);
  if (!base) return null;
  const bucketsRaw = Array.isArray(payload.buckets) ? payload.buckets : [];
  const weightBucketsRaw = Array.isArray(payload.weightBuckets)
    ? payload.weightBuckets
    : [];
  return {
    key: base.key,
    source: base.source,
    displayName: base.displayName,
    endpointUrl: base.endpointUrl,
    weight: base.weight,
    penalized: base.penalized,
    buckets: bucketsRaw
      .map(normalizeForwardProxyHourlyBucket)
      .filter((item): item is ForwardProxyHourlyBucket => item != null),
    weightBuckets: weightBucketsRaw
      .map(normalizeForwardProxyWeightBucket)
      .filter((item): item is ForwardProxyWeightBucket => item != null),
  };
}

function normalizeForwardProxyTimeseriesResponse(
  raw: unknown,
): ForwardProxyTimeseriesResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const nodesRaw = Array.isArray(payload.nodes) ? payload.nodes : [];
  const nodes = nodesRaw
    .map(normalizeForwardProxyTimeseriesNode)
    .filter((node): node is ForwardProxyTimeseriesNode => node != null)
    .sort((a, b) => a.displayName.localeCompare(b.displayName));
  const availableBucketsRaw = Array.isArray(payload.availableBuckets)
    ? payload.availableBuckets
    : [];
  return {
    rangeStart:
      typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    bucketSeconds: normalizeFiniteNumber(payload.bucketSeconds) ?? 3600,
    effectiveBucket:
      typeof payload.effectiveBucket === "string"
        ? payload.effectiveBucket
        : "1h",
    availableBuckets: availableBucketsRaw.filter(
      (item): item is string => typeof item === "string" && item.length > 0,
    ),
    nodes,
  };
}

function normalizePromptCacheConversationRequestPoint(
  raw: unknown,
): PromptCacheConversationRequestPoint | null {
  return normalizeConversationRequestPoint(raw);
}

function normalizePromptCacheConversationUpstreamAccount(
  raw: unknown,
): PromptCacheConversationUpstreamAccount | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    upstreamAccountId: normalizeFiniteNumber(payload.upstreamAccountId) ?? null,
    upstreamAccountName:
      typeof payload.upstreamAccountName === "string"
        ? payload.upstreamAccountName.trim() || null
        : null,
    requestCount: normalizeFiniteNumber(payload.requestCount) ?? 0,
    totalTokens: normalizeFiniteNumber(payload.totalTokens) ?? 0,
    totalCost: normalizeFiniteNumber(payload.totalCost) ?? 0,
    lastActivityAt:
      typeof payload.lastActivityAt === "string" ? payload.lastActivityAt : "",
  };
}

function normalizePromptCacheConversationInvocationPreview(
  raw: unknown,
): PromptCacheConversationInvocationPreview | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const invokeId =
    typeof payload.invokeId === "string" ? payload.invokeId.trim() : "";
  const occurredAt =
    typeof payload.occurredAt === "string" ? payload.occurredAt : "";
  if (!invokeId || !occurredAt) return null;
  const failureClass =
    typeof payload.failureClass === "string"
      ? payload.failureClass.trim().toLowerCase()
      : "";
  return {
    id: normalizeFiniteNumber(payload.id) ?? 0,
    invokeId,
    promptCacheKey:
      typeof payload.promptCacheKey === "string" && payload.promptCacheKey.trim()
        ? payload.promptCacheKey.trim()
        : null,
    occurredAt,
    status:
      typeof payload.status === "string" && payload.status.trim()
        ? payload.status.trim()
        : "unknown",
    livePhase: normalizeInvocationLivePhase(payload.livePhase),
    failureClass:
      failureClass === "none" ||
      failureClass === "service_failure" ||
      failureClass === "client_failure" ||
      failureClass === "client_abort"
        ? failureClass
        : null,
    routeMode:
      typeof payload.routeMode === "string" && payload.routeMode.trim()
        ? payload.routeMode.trim()
        : null,
    model:
      typeof payload.model === "string" && payload.model.trim()
        ? payload.model.trim()
        : null,
    requestModel:
      typeof payload.requestModel === "string" && payload.requestModel.trim()
        ? payload.requestModel.trim()
        : null,
    responseModel:
      typeof payload.responseModel === "string" && payload.responseModel.trim()
        ? payload.responseModel.trim()
        : null,
    totalTokens: normalizeFiniteNumber(payload.totalTokens) ?? 0,
    cost: normalizeFiniteNumber(payload.cost) ?? null,
    proxyDisplayName:
      typeof payload.proxyDisplayName === "string" &&
      payload.proxyDisplayName.trim()
        ? payload.proxyDisplayName.trim()
        : null,
    upstreamAccountId: normalizeFiniteNumber(payload.upstreamAccountId) ?? null,
    upstreamAccountName:
      typeof payload.upstreamAccountName === "string" &&
      payload.upstreamAccountName.trim()
        ? payload.upstreamAccountName.trim()
        : null,
    upstreamAccountPlanType:
      typeof payload.upstreamAccountPlanType === "string" &&
      payload.upstreamAccountPlanType.trim()
        ? payload.upstreamAccountPlanType.trim()
        : null,
    endpoint:
      typeof payload.endpoint === "string" && payload.endpoint.trim()
        ? payload.endpoint.trim()
        : null,
    compactionRequestKind:
      payload.compactionRequestKind === "compact" ||
      payload.compactionRequestKind === "remote_v2"
        ? payload.compactionRequestKind
        : null,
    compactionResponseKind:
      payload.compactionResponseKind === "compact" ||
      payload.compactionResponseKind === "remote_v2"
        ? payload.compactionResponseKind
        : null,
    imageIntent:
      payload.imageIntent === "yes" ||
      payload.imageIntent === "direct_image" ||
      payload.imageIntent === "no" ||
      payload.imageIntent === "unknown"
        ? payload.imageIntent
        : null,
    source:
      typeof payload.source === "string" && payload.source.trim()
        ? payload.source.trim()
        : undefined,
    inputTokens: normalizeFiniteNumber(payload.inputTokens),
    outputTokens: normalizeFiniteNumber(payload.outputTokens),
    cacheInputTokens: normalizeFiniteNumber(payload.cacheInputTokens),
    cacheWriteTokens: normalizeFiniteNumber(payload.cacheWriteTokens),
    reasoningTokens: normalizeFiniteNumber(payload.reasoningTokens),
    costInput: normalizeFiniteNumber(payload.costInput),
    costCacheWrite: normalizeFiniteNumber(payload.costCacheWrite),
    costCacheRead: normalizeFiniteNumber(payload.costCacheRead),
    costOutput: normalizeFiniteNumber(payload.costOutput),
    costReasoning: normalizeFiniteNumber(payload.costReasoning),
    reasoningEffort:
      typeof payload.reasoningEffort === "string" &&
      payload.reasoningEffort.trim()
        ? payload.reasoningEffort.trim()
        : undefined,
    errorMessage:
      typeof payload.errorMessage === "string" && payload.errorMessage.trim()
        ? payload.errorMessage
        : undefined,
    downstreamStatusCode: normalizeFiniteNumber(payload.downstreamStatusCode),
    downstreamErrorMessage:
      typeof payload.downstreamErrorMessage === "string" &&
      payload.downstreamErrorMessage.trim()
        ? payload.downstreamErrorMessage
        : undefined,
    failureKind:
      typeof payload.failureKind === "string" && payload.failureKind.trim()
        ? payload.failureKind.trim()
        : undefined,
    isActionable:
      typeof payload.isActionable === "boolean"
        ? payload.isActionable
        : undefined,
    responseContentEncoding:
      typeof payload.responseContentEncoding === "string" &&
      payload.responseContentEncoding.trim()
        ? payload.responseContentEncoding.trim()
        : undefined,
    requestedServiceTier:
      typeof payload.requestedServiceTier === "string" &&
      payload.requestedServiceTier.trim()
        ? payload.requestedServiceTier.trim()
        : undefined,
    serviceTier:
      typeof payload.serviceTier === "string" && payload.serviceTier.trim()
        ? payload.serviceTier.trim()
        : undefined,
    billingServiceTier:
      typeof payload.billingServiceTier === "string" &&
      payload.billingServiceTier.trim()
        ? payload.billingServiceTier.trim()
        : undefined,
    tReqReadMs: normalizeFiniteNumber(payload.tReqReadMs),
    tReqParseMs: normalizeFiniteNumber(payload.tReqParseMs),
    tUpstreamConnectMs: normalizeFiniteNumber(payload.tUpstreamConnectMs),
    tUpstreamTtfbMs: normalizeFiniteNumber(payload.tUpstreamTtfbMs),
    tUpstreamStreamMs: normalizeFiniteNumber(payload.tUpstreamStreamMs),
    tRespParseMs: normalizeFiniteNumber(payload.tRespParseMs),
    tPersistMs: normalizeFiniteNumber(payload.tPersistMs),
    tTotalMs: normalizeFiniteNumber(payload.tTotalMs),
  };
}

function normalizePromptCacheConversation(
  raw: unknown,
): PromptCacheConversation | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const promptCacheKey =
    typeof payload.promptCacheKey === "string"
      ? payload.promptCacheKey.trim()
      : "";
  if (!promptCacheKey) return null;
  const requestsRaw = Array.isArray(payload.last24hRequests)
    ? payload.last24hRequests
    : [];
  const recentInvocationsRaw = Array.isArray(payload.recentInvocations)
    ? payload.recentInvocations
    : [];
  const upstreamAccountsRaw = Array.isArray(payload.upstreamAccounts)
    ? payload.upstreamAccounts
    : [];
  return {
    promptCacheKey,
    requestCount: normalizeFiniteNumber(payload.requestCount) ?? 0,
    totalTokens: normalizeFiniteNumber(payload.totalTokens) ?? 0,
    totalCost: normalizeFiniteNumber(payload.totalCost) ?? 0,
    createdAt: typeof payload.createdAt === "string" ? payload.createdAt : "",
    lastActivityAt:
      typeof payload.lastActivityAt === "string" ? payload.lastActivityAt : "",
    lastTerminalAt:
      typeof payload.lastTerminalAt === "string"
        ? payload.lastTerminalAt
        : null,
    lastInFlightAt:
      typeof payload.lastInFlightAt === "string"
        ? payload.lastInFlightAt
        : null,
    cursor: typeof payload.cursor === "string" ? payload.cursor : null,
    hasEncryptedSessionOwner:
      typeof payload.hasEncryptedSessionOwner === "boolean"
        ? payload.hasEncryptedSessionOwner
        : false,
    encryptedOwnerAccountId:
      normalizeFiniteNumber(payload.encryptedOwnerAccountId) ?? null,
    encryptedOwnerAccountName:
      typeof payload.encryptedOwnerAccountName === "string" &&
      payload.encryptedOwnerAccountName.trim()
        ? payload.encryptedOwnerAccountName.trim()
        : null,
    encryptedOwnerGroupName:
      typeof payload.encryptedOwnerGroupName === "string" &&
      payload.encryptedOwnerGroupName.trim()
        ? payload.encryptedOwnerGroupName.trim()
        : null,
    upstreamAccounts: upstreamAccountsRaw
      .map(normalizePromptCacheConversationUpstreamAccount)
      .filter(
        (item): item is PromptCacheConversationUpstreamAccount => item != null,
      ),
    recentInvocations: recentInvocationsRaw
      .map(normalizePromptCacheConversationInvocationPreview)
      .filter(
        (item): item is PromptCacheConversationInvocationPreview =>
          item != null,
      ),
    last24hRequests: requestsRaw
      .map(normalizePromptCacheConversationRequestPoint)
      .filter(
        (item): item is PromptCacheConversationRequestPoint => item != null,
      ),
  };
}

function normalizeConversationRequestPoint(
  raw: unknown,
): ConversationRequestPoint | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const occurredAt =
    typeof payload.occurredAt === "string" ? payload.occurredAt : "";
  if (!occurredAt) return null;
  return {
    occurredAt,
    status: typeof payload.status === "string" ? payload.status : "unknown",
    isSuccess: payload.isSuccess === true,
    outcome:
      payload.outcome === "success" ||
      payload.outcome === "failure" ||
      payload.outcome === "neutral" ||
      payload.outcome === "in_flight"
        ? payload.outcome
        : null,
    requestTokens: normalizeFiniteNumber(payload.requestTokens) ?? 0,
    cumulativeTokens: normalizeFiniteNumber(payload.cumulativeTokens) ?? 0,
  };
}

function normalizeStickyKeyConversation(
  raw: unknown,
): StickyKeyConversation | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const stickyKey =
    typeof payload.stickyKey === "string" ? payload.stickyKey.trim() : "";
  if (!stickyKey) return null;
  const requestsRaw = Array.isArray(payload.last24hRequests)
    ? payload.last24hRequests
    : [];
  const recentInvocationsRaw = Array.isArray(payload.recentInvocations)
    ? payload.recentInvocations
    : [];
  return {
    stickyKey,
    requestCount: normalizeFiniteNumber(payload.requestCount) ?? 0,
    totalTokens: normalizeFiniteNumber(payload.totalTokens) ?? 0,
    totalCost: normalizeFiniteNumber(payload.totalCost) ?? 0,
    createdAt: typeof payload.createdAt === "string" ? payload.createdAt : "",
    lastActivityAt:
      typeof payload.lastActivityAt === "string" ? payload.lastActivityAt : "",
    recentInvocations: recentInvocationsRaw
      .map(normalizePromptCacheConversationInvocationPreview)
      .filter(
        (item): item is StickyKeyConversationInvocationPreview => item != null,
      ),
    last24hRequests: requestsRaw
      .map(normalizeConversationRequestPoint)
      .filter(
        (item): item is StickyKeyConversationRequestPoint => item != null,
      ),
  };
}

export function normalizeUpstreamStickyConversationsResponse(
  raw: unknown,
): UpstreamStickyConversationsResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const conversationsRaw = Array.isArray(payload.conversations)
    ? payload.conversations
    : [];
  const implicitFilterPayload =
    payload.implicitFilter && typeof payload.implicitFilter === "object"
      ? (payload.implicitFilter as Record<string, unknown>)
      : null;
  const implicitFilterKindRaw =
    typeof implicitFilterPayload?.kind === "string"
      ? implicitFilterPayload.kind
      : null;
  const implicitFilterKind: StickyKeyConversationImplicitFilterKind | null =
    implicitFilterKindRaw === "inactiveOutside24h" ||
    implicitFilterKindRaw === "cappedTo50"
      ? implicitFilterKindRaw
      : null;
  const selectionModeRaw =
    payload.selectionMode === "activityWindow" ? "activityWindow" : "count";
  return {
    rangeStart:
      typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    selectionMode: selectionModeRaw,
    selectedLimit:
      selectionModeRaw === "count"
        ? (normalizeFiniteNumber(payload.selectedLimit) ??
          DEFAULT_STICKY_KEY_CONVERSATION_LIMIT)
        : (normalizeFiniteNumber(payload.selectedLimit) ?? null),
    selectedActivityHours:
      normalizeFiniteNumber(payload.selectedActivityHours) ?? null,
    implicitFilter: {
      kind: implicitFilterKind,
      filteredCount:
        normalizeFiniteNumber(implicitFilterPayload?.filteredCount) ?? 0,
    },
    conversations: conversationsRaw
      .map(normalizeStickyKeyConversation)
      .filter((item): item is StickyKeyConversation => item != null),
  };
}

export function normalizePoolRoutingSettings(
  raw: unknown,
): PoolRoutingSettings | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  if (typeof payload.apiKeyConfigured !== "boolean") return null;
  const maintenanceRaw =
    payload.maintenance && typeof payload.maintenance === "object"
      ? (payload.maintenance as Record<string, unknown>)
      : null;
  const maintenance: PoolRoutingMaintenanceSettings = {
    primarySyncIntervalSecs:
      typeof maintenanceRaw?.primarySyncIntervalSecs === "number" &&
      Number.isFinite(maintenanceRaw.primarySyncIntervalSecs)
        ? Math.trunc(maintenanceRaw.primarySyncIntervalSecs)
        : DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.primarySyncIntervalSecs,
    secondarySyncIntervalSecs:
      typeof maintenanceRaw?.secondarySyncIntervalSecs === "number" &&
      Number.isFinite(maintenanceRaw.secondarySyncIntervalSecs)
        ? Math.trunc(maintenanceRaw.secondarySyncIntervalSecs)
        : DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.secondarySyncIntervalSecs,
    priorityAvailableAccountCap:
      typeof maintenanceRaw?.priorityAvailableAccountCap === "number" &&
      Number.isFinite(maintenanceRaw.priorityAvailableAccountCap)
        ? Math.trunc(maintenanceRaw.priorityAvailableAccountCap)
        : DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.priorityAvailableAccountCap,
  };
  return {
    writesEnabled:
      typeof payload.writesEnabled === "boolean" ? payload.writesEnabled : true,
    apiKeyConfigured: payload.apiKeyConfigured,
    maskedApiKey:
      typeof payload.maskedApiKey === "string" ? payload.maskedApiKey : null,
    maintenance,
    timeouts: normalizePoolRoutingTimeoutSettings(payload.timeouts),
  };
}

function normalizePoolRoutingTimeoutSettings(
  raw: unknown,
): PoolRoutingTimeoutSettings {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    responsesFirstByteTimeoutSecs:
      normalizeFiniteNumber(payload.responsesFirstByteTimeoutSecs) ?? 120,
    compactFirstByteTimeoutSecs:
      normalizeFiniteNumber(payload.compactFirstByteTimeoutSecs) ??
      normalizeFiniteNumber(payload.compactUpstreamHandshakeTimeoutSecs) ??
      300,
    responsesStreamTimeoutSecs:
      normalizeFiniteNumber(payload.responsesStreamTimeoutSecs) ?? 300,
    compactStreamTimeoutSecs:
      normalizeFiniteNumber(payload.compactStreamTimeoutSecs) ?? 300,
  };
}

function normalizeRoutingTimeoutFieldSources(
  raw: unknown,
): EffectiveRoutingTimeoutFieldSources {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const normalizeSource = (value: unknown) =>
    typeof value === "string" && value.trim() ? value : "root";
  return {
    responsesFirstByteTimeoutSecs: normalizeSource(
      payload.responsesFirstByteTimeoutSecs,
    ),
    compactFirstByteTimeoutSecs: normalizeSource(
      payload.compactFirstByteTimeoutSecs,
    ),
    responsesStreamTimeoutSecs: normalizeSource(
      payload.responsesStreamTimeoutSecs,
    ),
    compactStreamTimeoutSecs: normalizeSource(
      payload.compactStreamTimeoutSecs,
    ),
  };
}

function normalizePromptCacheConversationBindingResponse(
  raw: Record<string, unknown>,
  promptCacheKey: string,
): PromptCacheConversationBindingResponse {
  const normalizeRewriteMode = (
    value: unknown,
  ): PromptCacheConversationRewriteMode | null =>
    value === "force_remove" ||
    value === "keep_original" ||
    value === "fill_missing" ||
    value === "force_add"
      ? value
      : null;
  const normalizePolicySource = (value: unknown): EffectiveRoutingRuleSource =>
    typeof value === "string" && value.trim() ? value.trim() : "account";
  const rawPolicySources =
    raw.policyFieldSources && typeof raw.policyFieldSources === "object"
      ? (raw.policyFieldSources as Record<string, unknown>)
      : {};
  const forwardProxyKeys = Array.isArray(raw.forwardProxyKeys)
    ? raw.forwardProxyKeys
        .filter((value): value is string => typeof value === "string")
        .map((value) => value.trim())
        .filter(Boolean)
    : [];
  const forwardProxyKey =
    typeof raw.forwardProxyKey === "string" && raw.forwardProxyKey.trim()
      ? raw.forwardProxyKey.trim()
      : forwardProxyKeys[0] ?? null;
  return {
    promptCacheKey:
      typeof raw.promptCacheKey === "string" ? raw.promptCacheKey : promptCacheKey,
    bindingKind:
      raw.bindingKind === "group" || raw.bindingKind === "upstreamAccount"
        ? raw.bindingKind
        : "none",
    groupName:
      typeof raw.groupName === "string" && raw.groupName.trim()
        ? raw.groupName.trim()
        : null,
    upstreamAccountId: normalizeFiniteNumber(raw.upstreamAccountId) ?? null,
    upstreamAccountName:
      typeof raw.upstreamAccountName === "string" && raw.upstreamAccountName.trim()
        ? raw.upstreamAccountName.trim()
        : null,
    hasEncryptedSessionOwner:
      typeof raw.hasEncryptedSessionOwner === "boolean"
        ? raw.hasEncryptedSessionOwner
        : false,
    encryptedOwnerAccountId:
      normalizeFiniteNumber(raw.encryptedOwnerAccountId) ?? null,
    encryptedOwnerAccountName:
      typeof raw.encryptedOwnerAccountName === "string" &&
      raw.encryptedOwnerAccountName.trim()
        ? raw.encryptedOwnerAccountName.trim()
        : null,
    encryptedOwnerGroupName:
      typeof raw.encryptedOwnerGroupName === "string" &&
      raw.encryptedOwnerGroupName.trim()
        ? raw.encryptedOwnerGroupName.trim()
        : null,
    timeouts: normalizePoolRoutingTimeoutSettings(raw.timeouts),
    timeoutFieldSources: normalizeRoutingTimeoutFieldSources(
      raw.timeoutFieldSources,
    ),
    allowSwitchUpstream:
      typeof raw.allowSwitchUpstream === "boolean"
        ? raw.allowSwitchUpstream
        : null,
    fastModeRewriteMode: normalizeRewriteMode(raw.fastModeRewriteMode),
    imageToolRewriteMode: normalizeRewriteMode(raw.imageToolRewriteMode),
    availableModels: Array.isArray(raw.availableModels)
      ? raw.availableModels
          .filter((value): value is string => typeof value === "string")
          .map((value) => value.trim())
          .filter(Boolean)
      : null,
    forwardProxyKey,
    forwardProxyKeys:
      forwardProxyKeys.length > 0
        ? forwardProxyKeys
        : forwardProxyKey
          ? [forwardProxyKey]
          : [],
    policyFieldSources: {
      allowSwitchUpstream: normalizePolicySource(
        rawPolicySources.allowSwitchUpstream,
      ),
      fastModeRewriteMode: normalizePolicySource(
        rawPolicySources.fastModeRewriteMode,
      ),
      imageToolRewriteMode: normalizePolicySource(
        rawPolicySources.imageToolRewriteMode,
      ),
      availableModels: normalizePolicySource(rawPolicySources.availableModels),
      forwardProxyKey: normalizePolicySource(rawPolicySources.forwardProxyKey),
    },
    updatedAt: typeof raw.updatedAt === "string" ? raw.updatedAt : null,
  };
}

export function normalizeCompactSupportState(
  raw: unknown,
): CompactSupportState {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const status =
    payload.status === "supported" || payload.status === "unsupported"
      ? payload.status
      : "unknown";
  return {
    status,
    observedAt:
      typeof payload.observedAt === "string" ? payload.observedAt : null,
    reason: typeof payload.reason === "string" ? payload.reason : null,
  };
}

function normalizePromptCacheConversationsResponse(
  raw: unknown,
): PromptCacheConversationsResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const conversationsRaw = Array.isArray(payload.conversations)
    ? payload.conversations
    : [];
  const implicitFilterPayload =
    payload.implicitFilter && typeof payload.implicitFilter === "object"
      ? (payload.implicitFilter as Record<string, unknown>)
      : null;
  const implicitFilterKindRaw =
    typeof implicitFilterPayload?.kind === "string"
      ? implicitFilterPayload.kind
      : null;
  const implicitFilterKind: PromptCacheConversationImplicitFilterKind | null =
    implicitFilterKindRaw === "inactiveOutside24h" ||
    implicitFilterKindRaw === "cappedTo50"
      ? implicitFilterKindRaw
      : null;
  const selectionModeRaw =
    payload.selectionMode === "activityWindow" ? "activityWindow" : "count";
  return {
    rangeStart:
      typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    snapshotAt:
      typeof payload.snapshotAt === "string" ? payload.snapshotAt : null,
    selectionMode: selectionModeRaw,
    selectedLimit:
      selectionModeRaw === "count"
        ? (normalizeFiniteNumber(payload.selectedLimit) ??
          DEFAULT_PROMPT_CACHE_CONVERSATION_LIMIT)
        : (normalizeFiniteNumber(payload.selectedLimit) ?? null),
    selectedActivityHours:
      selectionModeRaw === "activityWindow"
        ? (normalizeFiniteNumber(payload.selectedActivityHours) ?? null)
        : (normalizeFiniteNumber(payload.selectedActivityHours) ?? null),
    selectedActivityMinutes:
      selectionModeRaw === "activityWindow"
        ? (normalizeFiniteNumber(payload.selectedActivityMinutes) ?? null)
        : (normalizeFiniteNumber(payload.selectedActivityMinutes) ?? null),
    implicitFilter: {
      kind: implicitFilterKind,
      filteredCount:
        normalizeFiniteNumber(implicitFilterPayload?.filteredCount) ?? 0,
    },
    totalMatched: normalizeFiniteNumber(payload.totalMatched) ?? null,
    hasMore: payload.hasMore === true,
    nextCursor:
      typeof payload.nextCursor === "string" ? payload.nextCursor : null,
    conversations: conversationsRaw
      .map(normalizePromptCacheConversation)
      .filter((item): item is PromptCacheConversation => item != null),
  };
}

function normalizeForwardProxyValidationResult(
  raw: unknown,
): ForwardProxyValidationResult {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    ok: payload.ok === true,
    message:
      typeof payload.message === "string" && payload.message.trim()
        ? payload.message
        : "validation failed",
    normalizedValue:
      typeof payload.normalizedValue === "string"
        ? payload.normalizedValue
        : undefined,
    discoveredNodes: normalizeFiniteNumber(payload.discoveredNodes),
    latencyMs: normalizeFiniteNumber(payload.latencyMs),
  };
}

function normalizeUpstreamAccountActivityAccount(
  raw: unknown,
): UpstreamAccountActivityAccount | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const upstreamAccountId = normalizeFiniteNumber(payload.upstreamAccountId);
  const isUnassigned = payload.isUnassigned === true;
  const displayName =
    typeof payload.displayName === "string" ? payload.displayName.trim() : "";
  if ((upstreamAccountId == null && !isUnassigned) || !displayName) {
    return null;
  }
  const recentInvocations = Array.isArray(payload.recentInvocations)
    ? payload.recentInvocations
        .map(normalizePromptCacheConversationInvocationPreview)
        .filter((item): item is PromptCacheConversationInvocationPreview => item != null)
    : [];
  return {
    accountKey:
      typeof payload.accountKey === "string" && payload.accountKey.trim()
        ? payload.accountKey.trim()
        : upstreamAccountId == null
          ? "unassigned"
          : `upstream:${upstreamAccountId}`,
    upstreamAccountId: upstreamAccountId ?? null,
    displayName,
    isUnassigned,
    groupName:
      typeof payload.groupName === "string" ? payload.groupName.trim() : null,
    planType:
      typeof payload.planType === "string" ? payload.planType.trim() : null,
    enabled: typeof payload.enabled === "boolean" ? payload.enabled : null,
    displayStatus:
      typeof payload.displayStatus === "string"
        ? payload.displayStatus.trim()
        : null,
    enableStatus:
      typeof payload.enableStatus === "string"
        ? payload.enableStatus.trim()
        : null,
    workStatus:
      typeof payload.workStatus === "string" ? payload.workStatus.trim() : null,
    healthStatus:
      typeof payload.healthStatus === "string"
        ? payload.healthStatus.trim()
        : null,
    syncState:
      typeof payload.syncState === "string" ? payload.syncState.trim() : null,
    lastError:
      typeof payload.lastError === "string" ? payload.lastError.trim() : null,
    lastActionReasonMessage:
      typeof payload.lastActionReasonMessage === "string"
        ? payload.lastActionReasonMessage.trim()
        : null,
    requestCount: normalizeFiniteNumber(payload.requestCount) ?? 0,
    successCount: normalizeFiniteNumber(payload.successCount) ?? 0,
    failureCount: normalizeFiniteNumber(payload.failureCount) ?? 0,
    nonSuccessCount: normalizeFiniteNumber(payload.nonSuccessCount) ?? 0,
    totalTokens: normalizeFiniteNumber(payload.totalTokens) ?? 0,
    successTokens: normalizeFiniteNumber(payload.successTokens) ?? 0,
    nonSuccessTokens: normalizeFiniteNumber(payload.nonSuccessTokens) ?? 0,
    failureTokens: normalizeFiniteNumber(payload.failureTokens) ?? 0,
    failureCost: normalizeFiniteNumber(payload.failureCost) ?? 0,
    totalCost: normalizeFiniteNumber(payload.totalCost) ?? 0,
    usageBreakdown: normalizeUsageBreakdown(payload.usageBreakdown) ?? {
      cacheWriteTokens: 0,
      cacheReadTokens: 0,
      outputTokens: 0,
      costs: null,
      models: [],
    },
    cacheHitRate: normalizeFiniteNumber(payload.cacheHitRate),
    tokensPerMinute: normalizeFiniteNumber(payload.tokensPerMinute),
    spendRate: normalizeFiniteNumber(payload.spendRate),
    firstByteAvgMs: normalizeFiniteNumber(payload.firstByteAvgMs),
    firstResponseByteTotalAvgMs: normalizeFiniteNumber(
      payload.firstResponseByteTotalAvgMs,
    ),
    avgTotalMs: normalizeFiniteNumber(payload.avgTotalMs),
    inProgressInvocationCount: normalizeFiniteNumber(
      payload.inProgressInvocationCount,
    ),
    inProgressPhaseCounts: normalizeInvocationPhaseCounts(
      payload.inProgressPhaseCounts,
    ),
    retryInvocationCount: normalizeFiniteNumber(payload.retryInvocationCount),
    effectiveRoutingRule: normalizeEffectiveRoutingRule(
      payload.effectiveRoutingRule,
    ),
    recentInvocations,
  };
}

function normalizeStatsResponse(raw: unknown): StatsResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    totalCount: normalizeFiniteNumber(payload.totalCount) ?? 0,
    successCount: normalizeFiniteNumber(payload.successCount) ?? 0,
    failureCount: normalizeFiniteNumber(payload.failureCount) ?? 0,
    totalCost: normalizeFiniteNumber(payload.totalCost) ?? 0,
    totalTokens: normalizeFiniteNumber(payload.totalTokens) ?? 0,
    usageBreakdown: normalizeUsageBreakdown(payload.usageBreakdown),
    inProgressConversationCount: normalizeFiniteNumber(
      payload.inProgressConversationCount,
    ),
    inProgressRetryConversationCount: normalizeFiniteNumber(
      payload.inProgressRetryConversationCount,
    ),
    inProgressAvgWaitMs: normalizeFiniteNumber(payload.inProgressAvgWaitMs),
    inProgressPhaseCounts: normalizeInvocationPhaseCounts(
      payload.inProgressPhaseCounts,
    ),
    nonSuccessCost: normalizeFiniteNumber(payload.nonSuccessCost),
    nonSuccessTokens: normalizeFiniteNumber(payload.nonSuccessTokens),
    maintenance: payload.maintenance as StatsMaintenanceResponse | undefined,
  };
}

function normalizeUpstreamAccountActivityResponse(
  raw: unknown,
): UpstreamAccountActivityResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    range: typeof payload.range === "string" ? payload.range : "",
    rangeStart: typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    accounts: Array.isArray(payload.accounts)
      ? payload.accounts
          .map(normalizeUpstreamAccountActivityAccount)
          .filter((item): item is UpstreamAccountActivityAccount => item != null)
      : [],
  };
}

function normalizeDashboardActivityResponse(raw: unknown): DashboardActivityResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const summaryPayload = (payload.summary ?? {}) as Record<string, unknown>;
  const rateWindowPayload = (payload.rateWindow ?? {}) as Record<string, unknown>;
  return {
    range: typeof payload.range === "string" ? payload.range : "",
    rangeStart: typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    snapshotId: normalizeFiniteNumber(payload.snapshotId) ?? 0,
    rateWindow: {
      start:
        typeof rateWindowPayload.start === "string"
          ? rateWindowPayload.start
          : "",
      end:
        typeof rateWindowPayload.end === "string"
          ? rateWindowPayload.end
          : "",
      windowMinutes:
        normalizeFiniteNumber(rateWindowPayload.windowMinutes) ?? 0,
      mode:
        typeof rateWindowPayload.mode === "string"
          ? rateWindowPayload.mode
          : "",
    },
    summary: {
      stats: normalizeStatsResponse(summaryPayload.stats),
      tokensPerMinute: normalizeFiniteNumber(summaryPayload.tokensPerMinute),
      spendRate: normalizeFiniteNumber(summaryPayload.spendRate),
    },
    accounts: Array.isArray(payload.accounts)
      ? payload.accounts
          .map(normalizeUpstreamAccountActivityAccount)
          .filter((item): item is UpstreamAccountActivityAccount => item != null)
      : undefined,
  };
}

function normalizeSettingsPayload(raw: unknown): SettingsPayload {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    proxy: normalizeProxySettings(payload.proxy),
    forwardProxy: normalizeForwardProxySettings(payload.forwardProxy),
    pricing: normalizePricingSettings(payload.pricing),
  };
}

function normalizeSystemStatusMetric(raw: unknown): SystemStatusMetric {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    count: normalizeFiniteNumber(payload.count) ?? 0,
    bytes: normalizeFiniteNumber(payload.bytes) ?? 0,
  };
}

function normalizeSystemStatusResponse(raw: unknown): SystemStatusResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    liveInvocationsCount: normalizeFiniteNumber(payload.liveInvocationsCount) ?? 0,
    successCount: normalizeFiniteNumber(payload.successCount) ?? 0,
    nonSuccessCount: normalizeFiniteNumber(payload.nonSuccessCount) ?? 0,
    completedArchiveBatchesCount: normalizeFiniteNumber(payload.completedArchiveBatchesCount) ?? 0,
    archivedBodies: normalizeSystemStatusMetric(payload.archivedBodies),
    rawBodies: normalizeSystemStatusMetric(payload.rawBodies),
    requestRawBodies: normalizeSystemStatusMetric(payload.requestRawBodies),
    responseRawBodies: normalizeSystemStatusMetric(payload.responseRawBodies),
    databaseBytes: normalizeFiniteNumber(payload.databaseBytes) ?? 0,
    otherFilesBytes: normalizeFiniteNumber(payload.otherFilesBytes) ?? 0,
    refreshedAt: typeof payload.refreshedAt === "string" ? payload.refreshedAt : "",
  };
}

function normalizeSystemTaskRun(raw: unknown): SystemTaskRun | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const id = normalizeFiniteNumber(payload.id);
  const taskKind = typeof payload.taskKind === "string" ? payload.taskKind : "";
  const triggerKind = typeof payload.triggerKind === "string" ? payload.triggerKind : "";
  const status = typeof payload.status === "string" ? payload.status : "";
  const startedAt = typeof payload.startedAt === "string" ? payload.startedAt : "";
  if (id == null || !taskKind || !triggerKind || !status || !startedAt) {
    return null;
  }
  return {
    id,
    taskKind,
    triggerKind,
    status,
    summary: typeof payload.summary === "string" ? payload.summary : undefined,
    detail: typeof payload.detail === "string" ? payload.detail : undefined,
    startedAt,
    finishedAt: typeof payload.finishedAt === "string" ? payload.finishedAt : undefined,
    durationMs: normalizeFiniteNumber(payload.durationMs),
  };
}

function normalizeSystemTaskRunsResponse(raw: unknown): SystemTaskRunsResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const itemsRaw = Array.isArray(payload.items) ? payload.items : [];
  return {
    items: itemsRaw
      .map(normalizeSystemTaskRun)
      .filter((item): item is SystemTaskRun => item != null),
    total: normalizeFiniteNumber(payload.total) ?? 0,
    page: normalizeFiniteNumber(payload.page) ?? 1,
    pageSize: normalizeFiniteNumber(payload.pageSize) ?? 20,
  };
}

function normalizeExternalApiKeySummary(
  raw: unknown,
): ExternalApiKeySummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const id = normalizeFiniteNumber(payload.id);
  const name = typeof payload.name === "string" ? payload.name : "";
  const status = typeof payload.status === "string" ? payload.status : "";
  const prefix = typeof payload.prefix === "string" ? payload.prefix : "";
  const createdAt =
    typeof payload.createdAt === "string" ? payload.createdAt : "";
  const updatedAt =
    typeof payload.updatedAt === "string" ? payload.updatedAt : "";
  if (
    id == null ||
    !name ||
    !status ||
    !prefix ||
    !createdAt ||
    !updatedAt
  ) {
    return null;
  }
  return {
    id,
    name,
    status,
    prefix,
    lastUsedAt:
      typeof payload.lastUsedAt === "string" ? payload.lastUsedAt : undefined,
    createdAt,
    updatedAt,
  };
}

function normalizeExternalApiKeyListResponse(
  raw: unknown,
): ExternalApiKeyListResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const items = Array.isArray(payload.items)
    ? payload.items
        .map((item) => normalizeExternalApiKeySummary(item))
        .filter((item): item is ExternalApiKeySummary => item != null)
    : [];
  return { items };
}

function normalizeExternalApiKeyMutationResponse(
  raw: unknown,
): ExternalApiKeyMutationResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const key = normalizeExternalApiKeySummary(payload.key);
  if (!key) {
    throw new Error("invalid external API key response");
  }
  return { key };
}

function normalizeExternalApiKeySecretResponse(
  raw: unknown,
): ExternalApiKeySecretResponse {
  const payload = normalizeExternalApiKeyMutationResponse(raw);
  const response = (raw ?? {}) as Record<string, unknown>;
  const secret = typeof response.secret === "string" ? response.secret : "";
  if (!secret) {
    throw new Error("invalid external API key secret response");
  }
  return {
    key: payload.key,
    secret,
  };
}

export async function fetchVersion(): Promise<VersionResponse> {
  return fetchJson<VersionResponse>("/api/version");
}

export async function fetchSettings(): Promise<SettingsPayload> {
  const response = await fetchJson<unknown>("/api/settings");
  return normalizeSettingsPayload(response);
}

export async function fetchSystemStatus(): Promise<SystemStatusResponse> {
  const response = await fetchJson<unknown>("/api/system/status");
  return normalizeSystemStatusResponse(response);
}

export async function fetchSystemTaskRuns(params?: {
  taskKind?: string;
  status?: string;
  startedAtFrom?: string;
  startedAtTo?: string;
  limit?: number;
  page?: number;
  pageSize?: number;
}): Promise<SystemTaskRunsResponse> {
  const query = new URLSearchParams();
  if (params?.taskKind) query.set("taskKind", params.taskKind);
  if (params?.status) query.set("status", params.status);
  if (params?.startedAtFrom) query.set("startedAtFrom", params.startedAtFrom);
  if (params?.startedAtTo) query.set("startedAtTo", params.startedAtTo);
  if (params?.limit != null) query.set("limit", String(params.limit));
  if (params?.page != null) query.set("page", String(params.page));
  if (params?.pageSize != null) query.set("pageSize", String(params.pageSize));
  const suffix = query.toString() ? `?${query.toString()}` : "";
  const response = await fetchJson<unknown>(`/api/system/tasks${suffix}`);
  return normalizeSystemTaskRunsResponse(response);
}

export async function fetchExternalApiKeys(): Promise<ExternalApiKeyListResponse> {
  const response = await fetchJson<unknown>("/api/settings/external-api-keys");
  return normalizeExternalApiKeyListResponse(response);
}

export async function createExternalApiKey(payload: {
  name: string;
}): Promise<ExternalApiKeySecretResponse> {
  const response = await fetchJson<unknown>("/api/settings/external-api-keys", {
    method: "POST",
    body: JSON.stringify(payload),
  });
  return normalizeExternalApiKeySecretResponse(response);
}

export async function rotateExternalApiKey(
  id: number,
): Promise<ExternalApiKeySecretResponse> {
  const response = await fetchJson<unknown>(
    `/api/settings/external-api-keys/${id}/rotate`,
    {
      method: "POST",
    },
  );
  return normalizeExternalApiKeySecretResponse(response);
}

export async function disableExternalApiKey(
  id: number,
): Promise<ExternalApiKeyMutationResponse> {
  const response = await fetchJson<unknown>(
    `/api/settings/external-api-keys/${id}/disable`,
    {
      method: "POST",
    },
  );
  return normalizeExternalApiKeyMutationResponse(response);
}

export async function updatePricingSettings(
  payload: PricingSettings,
): Promise<PricingSettings> {
  const response = await fetchJson<unknown>("/api/settings/pricing", {
    method: "PUT",
    body: JSON.stringify(payload),
  });
  return normalizePricingSettings(response);
}

export async function updateProxySettings(payload: {
  hijackEnabled: boolean;
  mergeUpstreamEnabled: boolean;
  fastModeRewriteMode?: ProxyFastModeRewriteMode;
  upstream429MaxRetries: number;
  websocketEnabled: boolean;
  upstreamWebsocketDefaultEnabled: boolean;
  requestBodyLoggingEnabled: boolean;
  responseBodyLoggingEnabled: boolean;
  encryptedSessionOwnerRoutingEnabled: boolean;
  enabledModels: string[];
}): Promise<ProxySettings> {
  const response = await fetchJson<unknown>("/api/settings/proxy", {
    method: "PUT",
    body: JSON.stringify(payload),
  });
  return normalizeProxySettings(response);
}

export async function updateForwardProxySettings(payload: {
  proxyUrls: string[];
  subscriptionUrls: string[];
  subscriptionUpdateIntervalSecs: number;
}): Promise<ForwardProxySettings> {
  const response = await fetchJson<unknown>("/api/settings/forward-proxy", {
    method: "PUT",
    body: JSON.stringify(payload),
  });
  return normalizeForwardProxySettings(response);
}

export async function refreshForwardProxySubscriptions(): Promise<ForwardProxyRefreshSubscriptionsResult> {
  const response = await fetchJson<unknown>(
    "/api/settings/forward-proxy/refresh-subscriptions",
    { method: "POST", body: JSON.stringify({}) },
  );
  return normalizeForwardProxyRefreshSubscriptionsResult(response);
}

export function createForwardProxyNodeLatencyTestEventSource(
  proxyKey: string,
): EventSource {
  return new EventSource(
    withBase(
      `/api/settings/forward-proxy/nodes/${encodeURIComponent(proxyKey)}/test-stream`,
    ),
  );
}

export function createForwardProxyNodesLatencyTestEventSource(
  proxyKeys: string[],
): EventSource {
  const query = new URLSearchParams();
  for (const proxyKey of proxyKeys) {
    query.append("key", proxyKey);
  }
  return new EventSource(
    withBase(
      `/api/settings/forward-proxy/nodes/test-stream?${query.toString()}`,
    ),
  );
}

export async function validateForwardProxyCandidate(payload: {
  kind: ForwardProxyValidationKind;
  value: string;
}): Promise<ForwardProxyValidationResult> {
  const controller = new AbortController();
  const timeoutMs = forwardProxyValidationTimeoutMs(payload.kind);
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetchJson<unknown>(
      "/api/settings/forward-proxy/validate",
      {
        method: "POST",
        body: JSON.stringify(payload),
        signal: controller.signal,
      },
    );
    return normalizeForwardProxyValidationResult(response);
  } catch (err) {
    if (err instanceof Error && err.name === "AbortError") {
      throw new Error(
        `validation request timed out after ${Math.floor(timeoutMs / 1000)}s`,
      );
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }
}

export async function fetchSummary(
  window: string,
  options?: { limit?: number; timeZone?: string; upstreamAccountId?: number; signal?: AbortSignal },
) {
  const search = new URLSearchParams();
  search.set("window", window);
  search.set("timeZone", options?.timeZone ?? getBrowserTimeZone());
  if (options?.limit !== undefined) {
    search.set("limit", String(options.limit));
  }
  if (options?.upstreamAccountId !== undefined) {
    search.set("upstreamAccountId", String(options.upstreamAccountId));
  }
  return fetchJson<StatsResponse>(`/api/stats/summary?${search.toString()}`, {
    signal: options?.signal,
  });
}

export async function fetchUpstreamAccountActivity(
  range: string,
  options?: { recentLimit?: number; timeZone?: string; signal?: AbortSignal },
) {
  const search = new URLSearchParams();
  search.set("range", range);
  search.set("timeZone", options?.timeZone ?? getBrowserTimeZone());
  if (options?.recentLimit !== undefined) {
    search.set("recentLimit", String(options.recentLimit));
  }
  const response = await fetchJson<unknown>(
    `/api/stats/upstream-account-activity?${search.toString()}`,
    { signal: options?.signal },
  );
  return normalizeUpstreamAccountActivityResponse(response);
}

export async function fetchDashboardActivity(
  range: string,
  options?: {
    recentLimit?: number;
    timeZone?: string;
    includeAccounts?: boolean;
    signal?: AbortSignal;
  },
) {
  const search = new URLSearchParams();
  search.set("range", range);
  search.set("timeZone", options?.timeZone ?? getBrowserTimeZone());
  if (options?.recentLimit !== undefined) {
    search.set("recentLimit", String(options.recentLimit));
  }
  if (options?.includeAccounts !== undefined) {
    search.set("includeAccounts", options.includeAccounts ? "true" : "false");
  }
  const response = await fetchJson<unknown>(
    `/api/stats/dashboard-activity?${search.toString()}`,
    { signal: options?.signal },
  );
  return normalizeDashboardActivityResponse(response);
}

export async function fetchForwardProxyLiveStats() {
  const response = await fetchJson<unknown>("/api/stats/forward-proxy");
  return normalizeForwardProxyLiveStatsResponse(response);
}

export async function fetchForwardProxyTimeseries(
  range: string,
  params?: { bucket?: string; timeZone?: string; signal?: AbortSignal },
) {
  const search = new URLSearchParams();
  search.set("range", range);
  search.set(
    "timeZone",
    resolveForwardProxyHistoryTimeZone(range, params?.timeZone),
  );
  if (params?.bucket) {
    search.set("bucket", params.bucket);
  }
  const response = await fetchJson<unknown>(
    `/api/stats/forward-proxy/timeseries?${search.toString()}`,
    { signal: params?.signal },
  );
  return normalizeForwardProxyTimeseriesResponse(response);
}

const DEFAULT_PROMPT_CACHE_CONVERSATION_LIMIT = 50;
const DEFAULT_STICKY_KEY_CONVERSATION_LIMIT = 50;

export async function fetchPromptCacheConversations(
  selection: PromptCacheConversationSelection,
  signal?: AbortSignal,
) {
  return fetchPromptCacheConversationsPage(selection, { signal });
}

export async function fetchPromptCacheConversationBinding(
  promptCacheKey: string,
  signal?: AbortSignal,
): Promise<PromptCacheConversationBindingResponse> {
  const raw = await fetchJson<Record<string, unknown>>(
    `/api/stats/prompt-cache-conversation-bindings/${encodeURIComponent(
      promptCacheKey,
    )}`,
    { signal },
  );
  return normalizePromptCacheConversationBindingResponse(raw, promptCacheKey);
}

export async function updatePromptCacheConversationBinding(
  promptCacheKey: string,
  payload: UpdatePromptCacheConversationBindingPayload,
  signal?: AbortSignal,
): Promise<PromptCacheConversationBindingResponse> {
  const raw = await fetchJson<Record<string, unknown>>(
    `/api/stats/prompt-cache-conversation-bindings/${encodeURIComponent(
      promptCacheKey,
    )}`,
    {
      method: "PATCH",
      body: JSON.stringify(payload),
      signal,
    },
  );
  return normalizePromptCacheConversationBindingResponse(raw, promptCacheKey);
}

export async function fetchPromptCacheConversationsPage(
  selection: PromptCacheConversationSelection,
  options: PromptCacheConversationPageQuery = {},
) {
  const search = new URLSearchParams();
  if (selection.mode === "count") {
    search.set("limit", String(selection.limit));
  } else if ("activityMinutes" in selection) {
    search.set("activityMinutes", String(selection.activityMinutes));
  } else {
    search.set("activityHours", String(selection.activityHours));
  }
  if (options.pageSize != null) {
    search.set("pageSize", String(options.pageSize));
  }
  if (options.cursor) {
    search.set("cursor", options.cursor);
  }
  if (options.snapshotAt) {
    search.set("snapshotAt", options.snapshotAt);
  }
  if (options.detail) {
    search.set("detail", options.detail);
  }
  if (options.recentInvocationLimit != null) {
    search.set("recentInvocationLimit", String(options.recentInvocationLimit));
  }
  const response = await fetchJson<unknown>(
    `/api/stats/prompt-cache-conversations?${search.toString()}`,
    {
      signal: options.signal,
    },
  );
  return normalizePromptCacheConversationsResponse(response);
}

export async function fetchTimeseries(
  range: string,
  params?: {
    bucket?: string;
    settlementHour?: number;
    timeZone?: string;
    upstreamAccountId?: number;
    signal?: AbortSignal;
  },
) {
  const search = new URLSearchParams();
  search.set("range", range);
  search.set("timeZone", params?.timeZone ?? getBrowserTimeZone());
  if (params?.bucket) search.set("bucket", params.bucket);
  if (params?.settlementHour !== undefined)
    search.set("settlementHour", String(params.settlementHour));
  if (params?.upstreamAccountId !== undefined)
    search.set("upstreamAccountId", String(params.upstreamAccountId));
  const response = await fetchJson<unknown>(
    `/api/stats/timeseries?${search.toString()}`,
    { signal: params?.signal },
  );
  return normalizeTimeseriesResponse(response);
}

export async function fetchParallelWorkStats(params?: {
  range?: string;
  bucket?: string;
  timeZone?: string;
  upstreamAccountId?: number;
  signal?: AbortSignal;
}) {
  const response = await fetchParallelWorkStatsConditional(params);
  if (!response.data) {
    throw new ApiRequestError(304, "Request failed: 304 parallel-work payload not modified");
  }
  return response.data;
}

export async function fetchParallelWorkStatsConditional(params?: {
  range?: string;
  bucket?: string;
  timeZone?: string;
  upstreamAccountId?: number;
  signal?: AbortSignal;
  etag?: string | null;
}): Promise<{
  data: ParallelWorkStatsResponse | null;
  etag: string | null;
  notModified: boolean;
}> {
  const search = new URLSearchParams();
  if (params?.range) search.set("range", params.range);
  if (params?.bucket) search.set("bucket", params.bucket);
  if (params?.upstreamAccountId !== undefined) {
    search.set("upstreamAccountId", String(params.upstreamAccountId));
  }
  search.set("timeZone", params?.timeZone ?? getBrowserTimeZone());
  const headers: HeadersInit = {
    "Content-Type": "application/json",
  };
  if (params?.etag) {
    headers["If-None-Match"] = params.etag;
  }
  const response = await fetch(
    withBase(`/api/stats/parallel-work?${search.toString()}`),
    { headers, signal: params?.signal },
  );
  const etag = response.headers.get("ETag");

  if (response.status === 304) {
    return {
      data: null,
      etag,
      notModified: true,
    };
  }

  if (!response.ok) {
    const rawText = await response.text();
    throw buildRequestError(response, rawText);
  }

  const rawText = await response.text();
  const payload = rawText.trim() ? JSON.parse(rawText) : undefined;
  return {
    data: normalizeParallelWorkStatsResponse(payload),
    etag,
    notModified: false,
  };
}

export async function fetchErrorDistribution(
  range: string,
  params?: { top?: number; scope?: FailureScope; timeZone?: string },
) {
  const search = new URLSearchParams();
  search.set("range", range);
  search.set("timeZone", params?.timeZone ?? getBrowserTimeZone());
  if (params?.top != null) search.set("top", String(params.top));
  if (params?.scope) search.set("scope", params.scope);
  return fetchJson<ErrorDistributionResponse>(
    `/api/stats/errors?${search.toString()}`,
  );
}

export async function fetchFailureSummary(
  range: string,
  params?: { timeZone?: string },
) {
  const search = new URLSearchParams();
  search.set("range", range);
  search.set("timeZone", params?.timeZone ?? getBrowserTimeZone());
  return fetchJson<FailureSummaryResponse>(
    `/api/stats/failures/summary?${search.toString()}`,
  );
}

export async function fetchPerfStats(params?: PerfStatsQuery) {
  const search = new URLSearchParams();
  if (params?.range) search.set("range", params.range);
  if (params?.bucket) search.set("bucket", params.bucket);
  if (params?.settlementHour !== undefined)
    search.set("settlementHour", String(params.settlementHour));
  search.set("timeZone", params?.timeZone ?? getBrowserTimeZone());
  if (params?.source) search.set("source", params.source);
  if (params?.model) search.set("model", params.model);
  if (params?.endpoint) search.set("endpoint", params.endpoint);

  const query = search.toString();
  return fetchJson<PerfStatsResponse>(
    query ? `/api/stats/perf?${query}` : "/api/stats/perf",
  );
}

export async function fetchQuotaSnapshot() {
  return fetchJson<QuotaSnapshot>("/api/quota/latest");
}
