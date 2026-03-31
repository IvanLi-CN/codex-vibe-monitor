import { getBrowserTimeZone } from "./timeZone";
import { normalizeForwardProxyProtocolLabel } from "./forwardProxyDisplay";

const rawBase = import.meta.env.VITE_API_BASE_URL ?? "";
const API_BASE = rawBase.endsWith("/") ? rawBase.slice(0, -1) : rawBase;
const OAUTH_LOGIN_SESSION_BASE_UPDATED_AT_HEADER =
  "X-Codex-Login-Session-Base-Updated-At";
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

const withBase = (path: string) => `${API_BASE}${path}`;

async function fetchJson<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(withBase(path), {
    headers: {
      "Content-Type": "application/json",
    },
    ...init,
  });

  if (!response.ok) {
    const rawText = await response.text();
    const compactText = rawText.replace(/\s+/g, " ").trim();
    const detail = (compactText || response.statusText || "").slice(0, 220);
    throw new Error(
      detail
        ? `Request failed: ${response.status} ${detail}`
        : `Request failed: ${response.status}`,
    );
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

async function ensureJsonRequestOk(response: Response): Promise<void> {
  if (response.ok) {
    return;
  }

  const rawText = await response.text();
  const compactText = rawText.replace(/\s+/g, " ").trim();
  const detail = (compactText || response.statusText || "").slice(0, 220);
  throw new Error(
    detail
      ? `Request failed: ${response.status} ${detail}`
      : `Request failed: ${response.status}`,
  );
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
      .filter((part) =>
        ["weekday", "year", "month", "day"].includes(part.type),
      )
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
  const shifted = new Date(Date.UTC(parts.year, parts.month - 1, parts.day + days));
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
  const localMidnightUtc = Date.UTC(parts.year, parts.month - 1, parts.day, 0, 0, 0);
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
  failureKind?: string | null;
  errorMessage?: string | null;
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
  | "proxy"
  | "endpoint"
  | "failureKind"
  | "promptCacheKey"
  | "requesterIp";
export type InvocationUpstreamScope = "all" | "external" | "internal";

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
  proxy?: string;
  endpoint?: string;
  failureClass?: string;
  failureKind?: string;
  promptCacheKey?: string;
  stickyKey?: string;
  requesterIp?: string;
  upstreamScope?: InvocationUpstreamScope;
  upstreamAccountId?: number;
  keyword?: string;
  minTotalTokens?: number;
  maxTotalTokens?: number;
  minTotalMs?: number;
  maxTotalMs?: number;
  suggestField?: InvocationSuggestionField;
  suggestQuery?: string;
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
  proxy: InvocationSuggestionBucket;
  endpoint: InvocationSuggestionBucket;
  failureKind: InvocationSuggestionBucket;
  promptCacheKey: InvocationSuggestionBucket;
  requesterIp: InvocationSuggestionBucket;
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
  if (query.proxy) search.set("proxy", query.proxy);
  if (query.endpoint) search.set("endpoint", query.endpoint);
  if (query.failureClass) search.set("failureClass", query.failureClass);
  if (query.failureKind) search.set("failureKind", query.failureKind);
  if (query.promptCacheKey) search.set("promptCacheKey", query.promptCacheKey);
  if (query.stickyKey) search.set("stickyKey", query.stickyKey);
  if (query.upstreamScope && query.upstreamScope !== "all")
    search.set("upstreamScope", query.upstreamScope);
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
  failureKind?: ApiInvocation["failureKind"];
  isActionable?: ApiInvocation["isActionable"];
  responseContentEncoding?: ApiInvocation["responseContentEncoding"];
  requestedServiceTier?: ApiInvocation["requestedServiceTier"];
  serviceTier?: ApiInvocation["serviceTier"];
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
  upstreamAccounts: PromptCacheConversationUpstreamAccount[];
  recentInvocations: PromptCacheConversationInvocationPreview[];
  last24hRequests: PromptCacheConversationRequestPoint[];
}

export type PromptCacheConversationSelectionMode = "count" | "activityWindow";

export type PromptCacheConversationImplicitFilterKind =
  | "inactiveOutside24h"
  | "cappedTo50";

export interface PromptCacheConversationImplicitFilter {
  kind: PromptCacheConversationImplicitFilterKind | null;
  filteredCount: number;
}

export type PromptCacheConversationSelection =
  | { mode: "count"; limit: number }
  | { mode: "activityWindow"; activityHours: number };

export interface PromptCacheConversationsResponse {
  rangeStart: string;
  rangeEnd: string;
  selectionMode: PromptCacheConversationSelectionMode;
  selectedLimit: number | null;
  selectedActivityHours: number | null;
  implicitFilter: PromptCacheConversationImplicitFilter;
  conversations: PromptCacheConversation[];
}

export type StickyKeyConversationSelectionMode =
  PromptCacheConversationSelectionMode;

export type StickyKeyConversationImplicitFilterKind =
  PromptCacheConversationImplicitFilterKind;

export interface StickyKeyConversationImplicitFilter {
  kind: StickyKeyConversationImplicitFilterKind | null;
  filteredCount: number;
}

export type StickyKeyConversationSelection = PromptCacheConversationSelection;

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

function normalizeStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((item): item is string => typeof item === "string");
}

function normalizeFiniteNumber(value: unknown): number | undefined {
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
    rangeStart: typeof payload.rangeStart === "string" ? payload.rangeStart : "",
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

function normalizeForwardProxyBindingNode(
  raw: unknown,
): ForwardProxyBindingNode | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const key = typeof payload.key === "string" ? payload.key.trim() : "";
  if (!key) return null;
  const bucketsRaw = Array.isArray(payload.last24h) ? payload.last24h : [];
  return {
    key,
    source: typeof payload.source === "string" ? payload.source : "manual",
    displayName:
      typeof payload.displayName === "string" && payload.displayName.trim()
        ? payload.displayName.trim()
        : key,
    protocolLabel: normalizeForwardProxyProtocolLabel(
      typeof payload.protocolLabel === "string" ? payload.protocolLabel : undefined,
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
      typeof payload.reasoningEffort === "string" && payload.reasoningEffort.trim()
        ? payload.reasoningEffort.trim()
        : undefined,
    errorMessage:
      typeof payload.errorMessage === "string" && payload.errorMessage.trim()
        ? payload.errorMessage
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
    upstreamAccounts: upstreamAccountsRaw
      .map(normalizePromptCacheConversationUpstreamAccount)
      .filter(
        (item): item is PromptCacheConversationUpstreamAccount => item != null,
      ),
    recentInvocations: recentInvocationsRaw
      .map(normalizePromptCacheConversationInvocationPreview)
      .filter(
        (item): item is PromptCacheConversationInvocationPreview => item != null,
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

function normalizeUpstreamStickyConversationsResponse(
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

function normalizePoolRoutingSettings(
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

function normalizeCompactSupportState(raw: unknown): CompactSupportState {
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
    implicitFilter: {
      kind: implicitFilterKind,
      filteredCount:
        normalizeFiniteNumber(implicitFilterPayload?.filteredCount) ?? 0,
    },
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

export interface TagRoutingRule {
  guardEnabled: boolean;
  lookbackHours?: number | null;
  maxConversations?: number | null;
  allowCutOut: boolean;
  allowCutIn: boolean;
}

export interface EffectiveConversationGuard {
  tagId: number;
  tagName: string;
  lookbackHours: number;
  maxConversations: number;
}

export interface EffectiveRoutingRule extends TagRoutingRule {
  sourceTagIds: number[];
  sourceTagNames: string[];
  guardRules: EffectiveConversationGuard[];
}

export interface AccountTagSummary {
  id: number;
  name: string;
  routingRule: TagRoutingRule;
}

export interface TagSummary {
  id: number;
  name: string;
  routingRule: TagRoutingRule;
  accountCount: number;
  groupCount: number;
  updatedAt: string;
}

export type TagDetail = TagSummary;

export interface TagListResponse {
  writesEnabled: boolean;
  items: TagSummary[];
}

export interface UpstreamAccountSummary {
  id: number;
  kind: "oauth_codex" | "api_key_codex" | string;
  provider: string;
  displayName: string;
  groupName?: string | null;
  isMother: boolean;
  status: "active" | "syncing" | "needs_reauth" | "error" | "disabled" | string;
  workStatus?:
    | "working"
    | "degraded"
    | "idle"
    | "rate_limited"
    | "unavailable"
    | string;
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
  lastActionHttpStatus?: number | null;
  lastActionInvokeId?: string | null;
  lastActionAt?: string | null;
  tokenExpiresAt?: string | null;
  primaryWindow?: RateWindowSnapshot | null;
  secondaryWindow?: RateWindowSnapshot | null;
  credits?: CreditsSnapshot | null;
  localLimits?: LocalLimitSnapshot | null;
  compactSupport?: CompactSupportState | null;
  duplicateInfo?: UpstreamAccountDuplicateInfo | null;
  tags: AccountTagSummary[];
  effectiveRoutingRule: EffectiveRoutingRule;
}

export interface UpstreamAccountActionEvent {
  id: number;
  occurredAt: string;
  action: string;
  source: string;
  reasonCode?: string | null;
  reasonMessage?: string | null;
  httpStatus?: number | null;
  failureKind?: string | null;
  invokeId?: string | null;
  stickyKey?: string | null;
  createdAt: string;
}

export interface UpstreamAccountDetail extends UpstreamAccountSummary {
  note?: string | null;
  upstreamBaseUrl?: string | null;
  chatgptUserId?: string | null;
  lastRefreshedAt?: string | null;
  history: UpstreamAccountHistoryPoint[];
  recentActions?: UpstreamAccountActionEvent[];
}

export interface UpstreamAccountGroupSummary {
  groupName: string;
  note?: string | null;
  boundProxyKeys?: string[];
  upstream429RetryEnabled?: boolean;
  upstream429MaxRetries?: number;
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

export interface PoolRoutingTimeoutSettings {
  responsesFirstByteTimeoutSecs: number;
  compactFirstByteTimeoutSecs: number;
  responsesStreamTimeoutSecs: number;
  compactStreamTimeoutSecs: number;
}

export interface UpdatePoolRoutingSettingsPayload {
  apiKey?: string;
  maintenance?: UpdatePoolRoutingMaintenanceSettingsPayload;
  timeouts?: Partial<PoolRoutingTimeoutSettings>;
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

export interface FetchUpstreamAccountsQuery {
  groupSearch?: string;
  groupUngrouped?: boolean;
  status?: string;
  workStatus?: string[];
  enableStatus?: string[];
  healthStatus?: string[];
  page?: number;
  pageSize?: number;
  tagIds?: number[];
}

export interface UpstreamAccountListMetrics {
  total: number;
  oauth: number;
  apiKey: number;
  attention: number;
}

export interface BulkUpstreamAccountActionPayload {
  accountIds: number[];
  action:
    | "enable"
    | "disable"
    | "delete"
    | "set_group"
    | "add_tags"
    | "remove_tags"
    | string;
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
  status: "pending" | "completed" | "failed" | "expired" | string;
  authUrl?: string | null;
  redirectUri?: string | null;
  expiresAt: string;
  updatedAt?: string | null;
  accountId?: number | null;
  error?: string | null;
  syncApplied?: boolean | null;
}

export type OauthMailboxSession =
  | OauthMailboxSessionSupported
  | OauthMailboxSessionUnsupported;

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
  groupName?: string;
  groupBoundProxyKeys?: string[];
  note?: string;
  groupNote?: string;
  accountId?: number;
  tagIds?: number[];
  isMother?: boolean;
  mailboxSessionId?: string;
  mailboxAddress?: string;
}

export interface UpdateOauthLoginSessionPayload {
  displayName?: string;
  groupName?: string;
  groupBoundProxyKeys?: string[];
  note?: string;
  groupNote?: string;
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
  headers.set(
    OAUTH_LOGIN_SESSION_BASE_UPDATED_AT_HEADER,
    normalizedBaseUpdatedAt,
  );
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
  groupName?: string;
  groupBoundProxyKeys?: string[];
  note?: string;
  groupNote?: string;
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
  groupName?: string;
  groupBoundProxyKeys?: string[];
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
}

export interface ImportOauthCredentialFilePayload {
  sourceId: string;
  fileName: string;
  content: string;
}

export interface ValidateImportedOauthAccountsPayload {
  groupName?: string;
  groupBoundProxyKeys?: string[];
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
  status:
    | "pending"
    | "duplicate_in_input"
    | "ok"
    | "ok_exhausted"
    | "invalid"
    | "error"
    | string;
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
  groupNote?: string;
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
  guardEnabled?: boolean;
  allowCutIn?: boolean;
  allowCutOut?: boolean;
}

export interface UpdateUpstreamAccountGroupPayload {
  note?: string;
  boundProxyKeys?: string[];
  upstream429RetryEnabled?: boolean;
  upstream429MaxRetries?: number;
}

function normalizeRateWindowActualUsage(
  raw: unknown,
): RateWindowActualUsage | null {
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
  const limitText =
    typeof payload.limitText === "string" ? payload.limitText : "";
  const windowDurationMins = normalizeFiniteNumber(payload.windowDurationMins);
  if (
    usedPercent == null ||
    !usedText ||
    !limitText ||
    windowDurationMins == null
  )
    return null;
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
  if (
    typeof payload.hasCredits !== "boolean" ||
    typeof payload.unlimited !== "boolean"
  )
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
  return {
    guardEnabled: payload.guardEnabled === true,
    lookbackHours: normalizeFiniteNumber(payload.lookbackHours) ?? null,
    maxConversations: normalizeFiniteNumber(payload.maxConversations) ?? null,
    allowCutOut: payload.allowCutOut !== false,
    allowCutIn: payload.allowCutIn !== false,
  };
}

function normalizeEffectiveConversationGuard(
  raw: unknown,
): EffectiveConversationGuard | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const tagId = normalizeFiniteNumber(payload.tagId);
  const tagName = typeof payload.tagName === "string" ? payload.tagName : "";
  const lookbackHours = normalizeFiniteNumber(payload.lookbackHours);
  const maxConversations = normalizeFiniteNumber(payload.maxConversations);
  if (
    tagId == null ||
    !tagName ||
    lookbackHours == null ||
    maxConversations == null
  )
    return null;
  return { tagId, tagName, lookbackHours, maxConversations };
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
  };
}

function normalizeEffectiveRoutingRule(raw: unknown): EffectiveRoutingRule {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const sourceTagIds = Array.isArray(payload.sourceTagIds)
    ? payload.sourceTagIds
        .map(normalizeFiniteNumber)
        .filter((value): value is number => value != null)
    : [];
  const sourceTagNames = Array.isArray(payload.sourceTagNames)
    ? payload.sourceTagNames.filter(
        (value): value is string => typeof value === "string",
      )
    : [];
  const guardRules = Array.isArray(payload.guardRules)
    ? payload.guardRules
        .map(normalizeEffectiveConversationGuard)
        .filter((value): value is EffectiveConversationGuard => value != null)
    : [];
  return {
    ...normalizeTagRoutingRule(payload),
    sourceTagIds,
    sourceTagNames,
    guardRules,
  };
}

function normalizeTagSummary(raw: unknown): TagSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const id = normalizeFiniteNumber(payload.id);
  const name = typeof payload.name === "string" ? payload.name : "";
  const accountCount = normalizeFiniteNumber(payload.accountCount);
  const groupCount = normalizeFiniteNumber(payload.groupCount);
  const updatedAt =
    typeof payload.updatedAt === "string" ? payload.updatedAt : "";
  if (
    id == null ||
    !name ||
    accountCount == null ||
    groupCount == null ||
    !updatedAt
  )
    return null;
  return {
    id,
    name,
    routingRule: normalizeTagRoutingRule(payload.routingRule),
    accountCount,
    groupCount,
    updatedAt,
  };
}

function normalizeUpstreamAccountSummary(
  raw: unknown,
): UpstreamAccountSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const id = normalizeFiniteNumber(payload.id);
  const displayName =
    typeof payload.displayName === "string" ? payload.displayName : "";
  const kind = typeof payload.kind === "string" ? payload.kind : "";
  const provider = typeof payload.provider === "string" ? payload.provider : "";
  const status = typeof payload.status === "string" ? payload.status : "error";
  const displayStatus =
    typeof payload.displayStatus === "string" ? payload.displayStatus : status;
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
      typeof payload.chatgptAccountId === "string"
        ? payload.chatgptAccountId
        : null,
    planType: typeof payload.planType === "string" ? payload.planType : null,
    maskedApiKey:
      typeof payload.maskedApiKey === "string" ? payload.maskedApiKey : null,
    lastSyncedAt:
      typeof payload.lastSyncedAt === "string" ? payload.lastSyncedAt : null,
    lastSuccessfulSyncAt:
      typeof payload.lastSuccessfulSyncAt === "string"
        ? payload.lastSuccessfulSyncAt
        : null,
    lastActivityAt:
      typeof payload.lastActivityAt === "string"
        ? payload.lastActivityAt
        : null,
    activeConversationCount:
      normalizeFiniteNumber(payload.activeConversationCount) ?? 0,
    lastError: typeof payload.lastError === "string" ? payload.lastError : null,
    lastErrorAt:
      typeof payload.lastErrorAt === "string" ? payload.lastErrorAt : null,
    lastAction:
      typeof payload.lastAction === "string" ? payload.lastAction : null,
    lastActionSource:
      typeof payload.lastActionSource === "string"
        ? payload.lastActionSource
        : null,
    lastActionReasonCode:
      typeof payload.lastActionReasonCode === "string"
        ? payload.lastActionReasonCode
        : null,
    lastActionReasonMessage:
      typeof payload.lastActionReasonMessage === "string"
        ? payload.lastActionReasonMessage
        : null,
    lastActionHttpStatus:
      normalizeFiniteNumber(payload.lastActionHttpStatus) ?? null,
    lastActionInvokeId:
      typeof payload.lastActionInvokeId === "string"
        ? payload.lastActionInvokeId
        : null,
    lastActionAt:
      typeof payload.lastActionAt === "string" ? payload.lastActionAt : null,
    tokenExpiresAt:
      typeof payload.tokenExpiresAt === "string"
        ? payload.tokenExpiresAt
        : null,
    primaryWindow: normalizeRateWindowSnapshot(payload.primaryWindow),
    secondaryWindow: normalizeRateWindowSnapshot(payload.secondaryWindow),
    credits: normalizeCreditsSnapshot(payload.credits),
    localLimits: normalizeLocalLimitSnapshot(payload.localLimits),
    compactSupport: normalizeCompactSupportState(payload.compactSupport),
    duplicateInfo: normalizeUpstreamAccountDuplicateInfo(payload.duplicateInfo),
    tags: Array.isArray(payload.tags)
      ? payload.tags
          .map(normalizeAccountTagSummary)
          .filter((item): item is AccountTagSummary => item != null)
      : [],
    effectiveRoutingRule: normalizeEffectiveRoutingRule(
      payload.effectiveRoutingRule,
    ),
  };
}

function normalizeUpstreamAccountListMetrics(
  raw: unknown,
): UpstreamAccountListMetrics {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    total: normalizeFiniteNumber(payload.total) ?? 0,
    oauth: normalizeFiniteNumber(payload.oauth) ?? 0,
    apiKey: normalizeFiniteNumber(payload.apiKey) ?? 0,
    attention: normalizeFiniteNumber(payload.attention) ?? 0,
  };
}

function normalizeUpstreamAccountDuplicateInfo(
  raw: unknown,
): UpstreamAccountDuplicateInfo | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const peerAccountIds = Array.isArray(payload.peerAccountIds)
    ? payload.peerAccountIds
        .map((value) => normalizeFiniteNumber(value))
        .filter((value): value is number => value != null)
    : [];
  const reasons = Array.isArray(payload.reasons)
    ? payload.reasons.filter(
        (value): value is string =>
          typeof value === "string" && value.trim().length > 0,
      )
    : [];
  if (peerAccountIds.length === 0 || reasons.length === 0) return null;
  return {
    peerAccountIds,
    reasons,
  };
}

function normalizeUpstreamAccountHistoryPoint(
  raw: unknown,
): UpstreamAccountHistoryPoint | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const capturedAt =
    typeof payload.capturedAt === "string" ? payload.capturedAt : "";
  if (!capturedAt) return null;
  return {
    capturedAt,
    primaryUsedPercent:
      normalizeFiniteNumber(payload.primaryUsedPercent) ?? null,
    secondaryUsedPercent:
      normalizeFiniteNumber(payload.secondaryUsedPercent) ?? null,
    creditsBalance:
      typeof payload.creditsBalance === "string"
        ? payload.creditsBalance
        : null,
  };
}

function normalizeUpstreamAccountActionEvent(
  raw: unknown,
): UpstreamAccountActionEvent | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const id = normalizeFiniteNumber(payload.id);
  const occurredAt =
    typeof payload.occurredAt === "string" ? payload.occurredAt : "";
  const action = typeof payload.action === "string" ? payload.action : "";
  const source = typeof payload.source === "string" ? payload.source : "";
  const createdAt =
    typeof payload.createdAt === "string" ? payload.createdAt : "";
  if (id == null || !occurredAt || !action || !source || !createdAt) {
    return null;
  }
  return {
    id,
    occurredAt,
    action,
    source,
    reasonCode:
      typeof payload.reasonCode === "string" ? payload.reasonCode : null,
    reasonMessage:
      typeof payload.reasonMessage === "string" ? payload.reasonMessage : null,
    httpStatus: normalizeFiniteNumber(payload.httpStatus) ?? null,
    failureKind:
      typeof payload.failureKind === "string" ? payload.failureKind : null,
    invokeId: typeof payload.invokeId === "string" ? payload.invokeId : null,
    stickyKey:
      typeof payload.stickyKey === "string" ? payload.stickyKey : null,
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
    upstreamBaseUrl:
      typeof payload.upstreamBaseUrl === "string"
        ? payload.upstreamBaseUrl
        : null,
    chatgptUserId:
      typeof payload.chatgptUserId === "string" ? payload.chatgptUserId : null,
    lastRefreshedAt:
      typeof payload.lastRefreshedAt === "string"
        ? payload.lastRefreshedAt
        : null,
    history: historyRaw
      .map(normalizeUpstreamAccountHistoryPoint)
      .filter((item): item is UpstreamAccountHistoryPoint => item != null),
    recentActions: Array.isArray(payload.recentActions)
      ? payload.recentActions
          .map(normalizeUpstreamAccountActionEvent)
          .filter(
            (item): item is UpstreamAccountActionEvent => item != null,
          )
      : [],
  };
}

function normalizeUpstreamAccountGroupMaxRetries(raw: unknown): number {
  const value = normalizeFiniteNumber(raw);
  if (value == null) return 0;
  return Math.min(5, Math.max(0, Math.trunc(value)));
}

function normalizeUpstreamAccountGroupSummary(
  raw: unknown,
): UpstreamAccountGroupSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const groupName =
    typeof payload.groupName === "string" ? payload.groupName.trim() : "";
  if (!groupName) return null;
  return {
    groupName,
    note: typeof payload.note === "string" ? payload.note : null,
    boundProxyKeys: normalizeStringArray(payload.boundProxyKeys).map((item) =>
      item.trim(),
    ).filter((item) => item.length > 0),
    upstream429RetryEnabled: payload.upstream429RetryEnabled === true,
    upstream429MaxRetries: normalizeUpstreamAccountGroupMaxRetries(
      payload.upstream429MaxRetries,
    ),
  };
}

function normalizeUpstreamAccountListResponse(
  raw: unknown,
): UpstreamAccountListResponse {
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

function normalizeTagListResponse(raw: unknown): TagListResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const itemsRaw = Array.isArray(payload.items) ? payload.items : [];
  return {
    writesEnabled: payload.writesEnabled !== false,
    items: itemsRaw
      .map(normalizeTagSummary)
      .filter((item): item is TagSummary => item != null),
  };
}

function normalizeLoginSessionStatusResponse(
  raw: unknown,
): LoginSessionStatusResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const loginId = typeof payload.loginId === "string" ? payload.loginId : "";
  const expiresAt =
    typeof payload.expiresAt === "string" ? payload.expiresAt : "";
  if (!loginId || !expiresAt) {
    throw new Error("Request failed: invalid login session payload");
  }
  const accountId = normalizeFiniteNumber(payload.accountId);
  return {
    loginId,
    status: typeof payload.status === "string" ? payload.status : "failed",
    authUrl: typeof payload.authUrl === "string" ? payload.authUrl : null,
    redirectUri:
      typeof payload.redirectUri === "string" ? payload.redirectUri : null,
    expiresAt,
    updatedAt: typeof payload.updatedAt === "string" ? payload.updatedAt : null,
    accountId: accountId == null ? null : accountId,
    error: typeof payload.error === "string" ? payload.error : null,
    syncApplied:
      typeof payload.syncApplied === "boolean" ? payload.syncApplied : null,
  };
}

function normalizeImportedOauthMatchSummary(
  raw: unknown,
): ImportedOauthMatchSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const accountId = normalizeFiniteNumber(payload.accountId);
  const displayName =
    typeof payload.displayName === "string" ? payload.displayName : "";
  const status = typeof payload.status === "string" ? payload.status : "";
  if (accountId == null || !displayName || !status) return null;
  return {
    accountId,
    displayName,
    groupName: typeof payload.groupName === "string" ? payload.groupName : null,
    status,
  };
}

function normalizeImportedOauthValidationRow(
  raw: unknown,
): ImportedOauthValidationRow | null {
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
      typeof payload.chatgptAccountId === "string"
        ? payload.chatgptAccountId
        : null,
    displayName:
      typeof payload.displayName === "string" ? payload.displayName : null,
    tokenExpiresAt:
      typeof payload.tokenExpiresAt === "string"
        ? payload.tokenExpiresAt
        : null,
    matchedAccount: normalizeImportedOauthMatchSummary(payload.matchedAccount),
    status,
    detail: typeof payload.detail === "string" ? payload.detail : null,
    attempts,
  };
}

function normalizeImportedOauthValidationResponse(
  raw: unknown,
): ImportedOauthValidationResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const inputFiles = normalizeFiniteNumber(payload.inputFiles);
  const uniqueInInput = normalizeFiniteNumber(payload.uniqueInInput);
  const duplicateInInput = normalizeFiniteNumber(payload.duplicateInInput);
  const rowsRaw = Array.isArray(payload.rows) ? payload.rows : [];
  if (inputFiles == null || uniqueInInput == null || duplicateInInput == null) {
    throw new Error(
      "Request failed: invalid imported OAuth validation payload",
    );
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

function normalizeImportedOauthImportResult(
  raw: unknown,
): ImportedOauthImportResult | null {
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
      typeof payload.chatgptAccountId === "string"
        ? payload.chatgptAccountId
        : null,
    accountId: normalizeFiniteNumber(payload.accountId) ?? null,
    status,
    detail: typeof payload.detail === "string" ? payload.detail : null,
    matchedAccount: normalizeImportedOauthMatchSummary(payload.matchedAccount),
  };
}

function normalizeImportedOauthValidationCounts(
  raw: unknown,
): ImportedOauthValidationCounts {
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
    throw new Error(
      "Request failed: invalid imported OAuth validation counts payload",
    );
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
    throw new Error(
      "Request failed: invalid imported OAuth validation job payload",
    );
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
    throw new Error(
      "Request failed: invalid imported OAuth validation row event payload",
    );
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
    throw new Error(
      "Request failed: invalid imported OAuth validation failed event payload",
    );
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
    displayName:
      typeof payload.displayName === "string" ? payload.displayName : null,
    status,
    detail: typeof payload.detail === "string" ? payload.detail : null,
  };
}

function normalizeBulkUpstreamAccountSyncCounts(
  raw: unknown,
): BulkUpstreamAccountSyncCounts {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    total: normalizeFiniteNumber(payload.total) ?? 0,
    completed: normalizeFiniteNumber(payload.completed) ?? 0,
    succeeded: normalizeFiniteNumber(payload.succeeded) ?? 0,
    failed: normalizeFiniteNumber(payload.failed) ?? 0,
    skipped: normalizeFiniteNumber(payload.skipped) ?? 0,
  };
}

function normalizeBulkUpstreamAccountSyncRow(
  raw: unknown,
): BulkUpstreamAccountSyncRow | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const accountId = normalizeFiniteNumber(payload.accountId);
  const displayName =
    typeof payload.displayName === "string" ? payload.displayName : "";
  const status = typeof payload.status === "string" ? payload.status : "";
  if (accountId == null || !displayName || !status) return null;
  return {
    accountId,
    displayName,
    status,
    detail: typeof payload.detail === "string" ? payload.detail : null,
  };
}

function normalizeBulkUpstreamAccountSyncSnapshot(
  raw: unknown,
): BulkUpstreamAccountSyncSnapshot {
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

function normalizeImportedOauthImportResponse(
  raw: unknown,
): ImportedOauthImportResponse {
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
  const sessionId =
    typeof payload.sessionId === "string" ? payload.sessionId : "";
  const emailAddress =
    typeof payload.emailAddress === "string" ? payload.emailAddress : "";
  const expiresAt =
    typeof payload.expiresAt === "string" ? payload.expiresAt : "";
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
      typeof payload.source === "string" && payload.source.trim()
        ? payload.source
        : "generated",
  };
}

function normalizeOauthMailboxCodeSummary(
  raw: unknown,
): OauthMailboxCodeSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const value = typeof payload.value === "string" ? payload.value : "";
  const source = typeof payload.source === "string" ? payload.source : "";
  const updatedAt =
    typeof payload.updatedAt === "string" ? payload.updatedAt : "";
  if (!value || !source || !updatedAt) return null;
  return { value, source, updatedAt };
}

function normalizeOauthInviteSummary(raw: unknown): OauthInviteSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const subject = typeof payload.subject === "string" ? payload.subject : "";
  const copyValue =
    typeof payload.copyValue === "string" ? payload.copyValue : "";
  const copyLabel =
    typeof payload.copyLabel === "string" ? payload.copyLabel : "";
  const updatedAt =
    typeof payload.updatedAt === "string" ? payload.updatedAt : "";
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
  const sessionId =
    typeof payload.sessionId === "string" ? payload.sessionId : "";
  const emailAddress =
    typeof payload.emailAddress === "string" ? payload.emailAddress : "";
  const expiresAt =
    typeof payload.expiresAt === "string" ? payload.expiresAt : "";
  if (!sessionId || !emailAddress || !expiresAt) return null;
  return {
    sessionId,
    emailAddress,
    expiresAt,
    latestCode: normalizeOauthMailboxCodeSummary(payload.latestCode),
    invite: normalizeOauthInviteSummary(payload.invite),
    invited: payload.invited === true,
    error:
      typeof payload.error === "string" && payload.error.trim()
        ? payload.error
        : null,
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
  search.set("timeZone", resolveForwardProxyHistoryTimeZone(range, params?.timeZone));
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
  const search = new URLSearchParams();
  if (selection.mode === "count") {
    search.set("limit", String(selection.limit));
  } else {
    search.set("activityHours", String(selection.activityHours));
  }
  const response = await fetchJson<unknown>(
    `/api/stats/prompt-cache-conversations?${search.toString()}`,
    {
      signal,
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

export async function fetchUpstreamAccounts(
  query?: FetchUpstreamAccountsQuery,
): Promise<UpstreamAccountListResponse> {
  const search = new URLSearchParams();
  if (query?.groupSearch) search.set("groupSearch", query.groupSearch);
  if (query?.groupUngrouped != null)
    search.set("groupUngrouped", String(query.groupUngrouped));
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

export async function fetchTags(
  query?: FetchTagsQuery,
): Promise<TagListResponse> {
  const search = new URLSearchParams();
  if (query?.search) search.set("search", query.search);
  if (query?.hasAccounts != null)
    search.set("hasAccounts", String(query.hasAccounts));
  if (query?.guardEnabled != null)
    search.set("guardEnabled", String(query.guardEnabled));
  if (query?.allowCutIn != null)
    search.set("allowCutIn", String(query.allowCutIn));
  if (query?.allowCutOut != null)
    search.set("allowCutOut", String(query.allowCutOut));
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

export async function updateTag(
  tagId: number,
  payload: UpdateTagPayload,
): Promise<TagDetail> {
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
  signal?: AbortSignal,
): Promise<UpstreamAccountDetail> {
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-accounts/${accountId}`,
    {
      signal,
    },
  );
  return normalizeUpstreamAccountDetail(response);
}

export async function createOauthLoginSession(
  payload: CreateOauthLoginSessionPayload,
): Promise<LoginSessionStatusResponse> {
  const response = await fetchJson<unknown>(
    "/api/pool/upstream-accounts/oauth/login-sessions",
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
  return normalizeLoginSessionStatusResponse(response);
}

export async function createOauthMailboxSession(
  payload: CreateOauthMailboxSessionPayload = {},
): Promise<OauthMailboxSession> {
  const response = await fetchJson<unknown>(
    "/api/pool/upstream-accounts/oauth/mailbox-sessions",
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
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
  const items = Array.isArray(
    (response as Record<string, unknown> | null)?.items,
  )
    ? ((response as Record<string, unknown>).items as unknown[])
    : [];
  return items
    .map(normalizeOauthMailboxStatus)
    .filter((item): item is OauthMailboxStatus => item != null);
}

export async function deleteOauthMailboxSession(
  sessionId: string,
): Promise<void> {
  await fetchJson(
    `/api/pool/upstream-accounts/oauth/mailbox-sessions/${encodeURIComponent(sessionId)}`,
    {
      method: "DELETE",
    },
  );
}

export async function fetchOauthLoginSession(
  loginId: string,
): Promise<LoginSessionStatusResponse> {
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
    withBase(
      `/api/pool/upstream-accounts/oauth/login-sessions/${encodeURIComponent(loginId)}`,
    ),
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

export async function validateImportedOauthAccounts(
  payload: ValidateImportedOauthAccountsPayload,
): Promise<ImportedOauthValidationResponse> {
  const response = await fetchJson<unknown>(
    "/api/pool/upstream-accounts/oauth/imports/validate",
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
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
  )
  return normalizeImportedOauthValidationJobResponse(response)
}

export async function cancelImportedOauthValidationJob(
  jobId: string,
): Promise<void> {
  await fetchJson(
    `/api/pool/upstream-accounts/oauth/imports/validation-jobs/${encodeURIComponent(jobId)}`,
    {
      method: "DELETE",
    },
  )
}

export async function importValidatedOauthAccounts(
  payload: ImportValidatedOauthAccountsPayload,
): Promise<ImportedOauthImportResponse> {
  const response = await fetchJson<unknown>(
    "/api/pool/upstream-accounts/oauth/imports",
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
  return normalizeImportedOauthImportResponse(response);
}

export async function createApiKeyUpstreamAccount(
  payload: CreateApiKeyAccountPayload,
): Promise<UpstreamAccountDetail> {
  const response = await fetchJson<unknown>(
    "/api/pool/upstream-accounts/api-keys",
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
  return normalizeUpstreamAccountDetail(response);
}

export async function updateUpstreamAccount(
  accountId: number,
  payload: UpdateUpstreamAccountPayload,
): Promise<UpstreamAccountDetail> {
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-accounts/${accountId}`,
    {
      method: "PATCH",
      body: JSON.stringify(payload),
    },
  );
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

export async function syncUpstreamAccount(
  accountId: number,
): Promise<UpstreamAccountDetail> {
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-accounts/${accountId}/sync`,
    {
      method: "POST",
    },
  );
  return normalizeUpstreamAccountDetail(response);
}

export async function createBulkUpstreamAccountSyncJob(
  payload: BulkUpstreamAccountSyncJobPayload,
): Promise<BulkUpstreamAccountSyncJobResponse> {
  const response = await fetchJson<unknown>(
    "/api/pool/upstream-accounts/bulk-sync-jobs",
    {
      method: "POST",
      body: JSON.stringify(payload),
    },
  );
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

export async function cancelBulkUpstreamAccountSyncJob(
  jobId: string,
): Promise<void> {
  await fetchJson(
    `/api/pool/upstream-accounts/bulk-sync-jobs/${encodeURIComponent(jobId)}`,
    {
      method: "DELETE",
    },
  );
}

export function createEventSource(path: string) {
  return new EventSource(withBase(path));
}

export function createImportedOauthValidationJobEventSource(jobId: string) {
  return createEventSource(
    `/api/pool/upstream-accounts/oauth/imports/validation-jobs/${encodeURIComponent(jobId)}/events`,
  )
}

export function createBulkUpstreamAccountSyncJobEventSource(jobId: string) {
  return createEventSource(
    `/api/pool/upstream-accounts/bulk-sync-jobs/${encodeURIComponent(jobId)}/events`,
  );
}
