import { getBrowserTimeZone } from "../timeZone";
import { normalizeForwardProxyProtocolLabel } from "../forwardProxyDisplay";
import type {
  CompactSupportState,
  PoolRoutingMaintenanceSettings,
  PoolRoutingSettings,
  PoolRoutingTimeoutSettings,
} from "./core-upstream";

const rawBase = import.meta.env.VITE_API_BASE_URL ?? "";
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
    return undefined as T;
  }

  const rawText = await response.text();
  if (!rawText.trim()) {
    return undefined as T;
  }

  return JSON.parse(rawText) as T;
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
  inputTokens?: number;
  outputTokens?: number;
  cacheInputTokens?: number;
  reasoningTokens?: number;
  reasoningEffort?: string;
  totalTokens?: number;
  cost?: number;
  status?: string;
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
  requesterIp?: string;
  promptCacheKey?: string;
  stickyKey?: string | null;
  routeMode?: string;
  upstreamAccountId?: number | null;
  upstreamAccountName?: string;
  responseContentEncoding?: string;
  poolAttemptCount?: number | null;
  poolDistinctAccountCount?: number | null;
  poolAttemptTerminalReason?: string | null;
  upstreamScope?: string;
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
  upstreamRouteKey?: string | null;
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
  maintenance?: StatsMaintenanceResponse;
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
  totalTokens: number;
  totalCost: number;
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
}

export interface ParallelWorkStatsResponse {
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

export async function fetchInvocationRecordsSummary(
  query: InvocationRecordsQuery,
) {
  const search = new URLSearchParams();
  appendInvocationRecordsQuery(search, query);
  return fetchJson<InvocationRecordsSummaryResponse>(
    `/api/invocations/summary?${search.toString()}`,
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
  reasoningPer1m?: number | null;
  source: string;
}

export interface PricingSettings {
  catalogVersion: string;
  entries: PricingEntry[];
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
  requestTokens: number;
  cumulativeTokens: number;
}

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
  occurredAt: string;
  status: string;
  failureClass: Exclude<ApiInvocation["failureClass"], undefined> | null;
  routeMode: string | null;
  model: string | null;
  totalTokens: number;
  cost: number | null;
  proxyDisplayName: string | null;
  upstreamAccountId: number | null;
  upstreamAccountName: string | null;
  endpoint: string | null;
  source?: ApiInvocation["source"];
  inputTokens?: ApiInvocation["inputTokens"];
  outputTokens?: ApiInvocation["outputTokens"];
  cacheInputTokens?: ApiInvocation["cacheInputTokens"];
  reasoningTokens?: ApiInvocation["reasoningTokens"];
  reasoningEffort?: ApiInvocation["reasoningEffort"];
  errorMessage?: ApiInvocation["errorMessage"];
  downstreamStatusCode?: ApiInvocation["downstreamStatusCode"];
  downstreamErrorMessage?: ApiInvocation["downstreamErrorMessage"];
  failureKind?: ApiInvocation["failureKind"];
  isActionable?: ApiInvocation["isActionable"];
  responseContentEncoding?: ApiInvocation["responseContentEncoding"];
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
  upstreamAccounts: PromptCacheConversationUpstreamAccount[];
  recentInvocations: PromptCacheConversationInvocationPreview[];
  last24hRequests: PromptCacheConversationRequestPoint[];
}

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
  forwardProxy: ForwardProxySettings;
  pricing: PricingSettings;
}

export function normalizeStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((item): item is string => typeof item === "string");
}

export function normalizeFiniteNumber(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value)) return undefined;
  return value;
}

function normalizeTimeseriesPoint(raw: unknown): TimeseriesPoint | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const bucketStart =
    typeof payload.bucketStart === "string" ? payload.bucketStart : "";
  const bucketEnd =
    typeof payload.bucketEnd === "string" ? payload.bucketEnd : "";
  if (!bucketStart || !bucketEnd) return null;
  return {
    bucketStart,
    bucketEnd,
    totalCount: normalizeFiniteNumber(payload.totalCount) ?? 0,
    successCount: normalizeFiniteNumber(payload.successCount) ?? 0,
    failureCount: normalizeFiniteNumber(payload.failureCount) ?? 0,
    totalTokens: normalizeFiniteNumber(payload.totalTokens) ?? 0,
    totalCost: normalizeFiniteNumber(payload.totalCost) ?? 0,
    firstByteSampleCount:
      normalizeFiniteNumber(payload.firstByteSampleCount) ?? 0,
    firstByteAvgMs: normalizeFiniteNumber(payload.firstByteAvgMs) ?? null,
    firstByteP95Ms: normalizeFiniteNumber(payload.firstByteP95Ms) ?? null,
    firstResponseByteTotalSampleCount:
      normalizeFiniteNumber(payload.firstResponseByteTotalSampleCount) ?? 0,
    firstResponseByteTotalAvgMs:
      normalizeFiniteNumber(payload.firstResponseByteTotalAvgMs) ?? null,
    firstResponseByteTotalP95Ms:
      normalizeFiniteNumber(payload.firstResponseByteTotalP95Ms) ?? null,
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

function normalizeParallelWorkWindowResponse(
  raw: unknown,
): ParallelWorkWindowResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const pointsRaw = Array.isArray(payload.points) ? payload.points : [];
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
  };
}

function normalizeParallelWorkStatsResponse(
  raw: unknown,
): ParallelWorkStatsResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    minute7d: normalizeParallelWorkWindowResponse(payload.minute7d),
    hour30d: normalizeParallelWorkWindowResponse(payload.hour30d),
    dayAll: normalizeParallelWorkWindowResponse(payload.dayAll),
  };
}

function normalizePricingEntry(raw: unknown): PricingEntry | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const model = typeof payload.model === "string" ? payload.model.trim() : "";
  const inputPer1m = normalizeFiniteNumber(payload.inputPer1m);
  const outputPer1m = normalizeFiniteNumber(payload.outputPer1m);
  if (!model || inputPer1m === undefined || outputPer1m === undefined)
    return null;
  const cacheInputPer1m = normalizeFiniteNumber(payload.cacheInputPer1m);
  const reasoningPer1m = normalizeFiniteNumber(payload.reasoningPer1m);
  return {
    model,
    inputPer1m,
    outputPer1m,
    cacheInputPer1m: cacheInputPer1m ?? null,
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
    occurredAt,
    status:
      typeof payload.status === "string" && payload.status.trim()
        ? payload.status.trim()
        : "unknown",
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
    endpoint:
      typeof payload.endpoint === "string" && payload.endpoint.trim()
        ? payload.endpoint.trim()
        : null,
    source:
      typeof payload.source === "string" && payload.source.trim()
        ? payload.source.trim()
        : undefined,
    inputTokens: normalizeFiniteNumber(payload.inputTokens),
    outputTokens: normalizeFiniteNumber(payload.outputTokens),
    cacheInputTokens: normalizeFiniteNumber(payload.cacheInputTokens),
    reasoningTokens: normalizeFiniteNumber(payload.reasoningTokens),
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
      typeof payload.lastTerminalAt === "string" ? payload.lastTerminalAt : null,
    lastInFlightAt:
      typeof payload.lastInFlightAt === "string" ? payload.lastInFlightAt : null,
    cursor: typeof payload.cursor === "string" ? payload.cursor : null,
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
    nextCursor: typeof payload.nextCursor === "string" ? payload.nextCursor : null,
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

function normalizeSettingsPayload(raw: unknown): SettingsPayload {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    forwardProxy: normalizeForwardProxySettings(payload.forwardProxy),
    pricing: normalizePricingSettings(payload.pricing),
  };
}

export async function fetchVersion(): Promise<VersionResponse> {
  return fetchJson<VersionResponse>("/api/version");
}

export async function fetchSettings(): Promise<SettingsPayload> {
  const response = await fetchJson<unknown>("/api/settings");
  return normalizeSettingsPayload(response);
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
  options?: { limit?: number; timeZone?: string; signal?: AbortSignal },
) {
  const search = new URLSearchParams();
  search.set("window", window);
  search.set("timeZone", options?.timeZone ?? getBrowserTimeZone());
  if (options?.limit !== undefined) {
    search.set("limit", String(options.limit));
  }
  return fetchJson<StatsResponse>(`/api/stats/summary?${search.toString()}`, {
    signal: options?.signal,
  });
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
    signal?: AbortSignal;
  },
) {
  const search = new URLSearchParams();
  search.set("range", range);
  search.set("timeZone", params?.timeZone ?? getBrowserTimeZone());
  if (params?.bucket) search.set("bucket", params.bucket);
  if (params?.settlementHour !== undefined)
    search.set("settlementHour", String(params.settlementHour));
  const response = await fetchJson<unknown>(
    `/api/stats/timeseries?${search.toString()}`,
    { signal: params?.signal },
  );
  return normalizeTimeseriesResponse(response);
}

export async function fetchParallelWorkStats(params?: {
  timeZone?: string;
  signal?: AbortSignal;
}) {
  const search = new URLSearchParams();
  search.set("timeZone", params?.timeZone ?? getBrowserTimeZone());
  const response = await fetchJson<unknown>(
    `/api/stats/parallel-work?${search.toString()}`,
    { signal: params?.signal },
  );
  return normalizeParallelWorkStatsResponse(response);
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
