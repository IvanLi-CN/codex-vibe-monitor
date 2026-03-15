import { getBrowserTimeZone } from "./timeZone";

const rawBase = import.meta.env.VITE_API_BASE_URL ?? "";
const API_BASE = rawBase.endsWith("/") ? rawBase.slice(0, -1) : rawBase;
const FORWARD_PROXY_VALIDATION_TIMEOUT_MS = 5_000;
const FORWARD_PROXY_SUBSCRIPTION_VALIDATION_TIMEOUT_MS = 60_000;

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
  requesterIp?: string;
  upstreamScope?: InvocationUpstreamScope;
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
}

export interface TimeseriesResponse {
  rangeStart: string;
  rangeEnd: string;
  bucketSeconds: number;
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
  if (query.upstreamScope && query.upstreamScope !== "all")
    search.set("upstreamScope", query.upstreamScope);
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

export async function fetchStats() {
  return fetchJson<StatsResponse>("/api/stats");
}

export interface VersionResponse {
  backend: string;
  frontend: string;
}

export type ProxyFastModeRewriteMode =
  | "disabled"
  | "fill_missing"
  | "force_priority";

const DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES = 3;
const MAX_PROXY_UPSTREAM_429_MAX_RETRIES = 5;

export interface ProxySettings {
  hijackEnabled: boolean;
  mergeUpstreamEnabled: boolean;
  models: string[];
  enabledModels: string[];
  defaultHijackEnabled: boolean;
  fastModeRewriteMode: ProxyFastModeRewriteMode;
  upstream429MaxRetries: number;
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

export interface ForwardProxySettings {
  proxyUrls: string[];
  subscriptionUrls: string[];
  subscriptionUpdateIntervalSecs: number;
  insertDirect: boolean;
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

export interface ConversationRequestPoint {
  occurredAt: string;
  status: string;
  isSuccess: boolean;
  requestTokens: number;
  cumulativeTokens: number;
}

export type PromptCacheConversationRequestPoint = ConversationRequestPoint;

export type StickyKeyConversationRequestPoint = ConversationRequestPoint;

export interface PromptCacheConversation {
  promptCacheKey: string;
  requestCount: number;
  totalTokens: number;
  totalCost: number;
  createdAt: string;
  lastActivityAt: string;
  last24hRequests: PromptCacheConversationRequestPoint[];
}

export interface PromptCacheConversationsResponse {
  rangeStart: string;
  rangeEnd: string;
  conversations: PromptCacheConversation[];
}

export interface StickyKeyConversation {
  stickyKey: string;
  requestCount: number;
  totalTokens: number;
  totalCost: number;
  createdAt: string;
  lastActivityAt: string;
  last24hRequests: StickyKeyConversationRequestPoint[];
}

export interface UpstreamStickyConversationsResponse {
  rangeStart: string;
  rangeEnd: string;
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

function normalizeStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((item): item is string => typeof item === "string");
}

function normalizeFiniteNumber(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value)) return undefined;
  return value;
}

function normalizeProxyFastModeRewriteMode(
  value: unknown,
): ProxyFastModeRewriteMode {
  return value === "fill_missing" || value === "force_priority"
    ? value
    : "disabled";
}

function normalizeProxyUpstream429MaxRetries(value: unknown): number {
  if (typeof value !== "number" || !Number.isFinite(value))
    return DEFAULT_PROXY_UPSTREAM_429_MAX_RETRIES;
  const normalized = Math.trunc(value);
  return Math.min(MAX_PROXY_UPSTREAM_429_MAX_RETRIES, Math.max(0, normalized));
}

function normalizeProxySettings(raw: unknown): ProxySettings {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const models = normalizeStringArray(payload.models);
  const hasEnabledModelsField = Object.prototype.hasOwnProperty.call(
    payload,
    "enabledModels",
  );
  const enabledModelsRaw = normalizeStringArray(payload.enabledModels);
  const allowSet = new Set(models);
  const enabledModels = (
    hasEnabledModelsField ? enabledModelsRaw : models
  ).filter((modelId) => allowSet.has(modelId));

  return {
    hijackEnabled: Boolean(payload.hijackEnabled),
    mergeUpstreamEnabled: Boolean(payload.mergeUpstreamEnabled),
    models,
    enabledModels,
    defaultHijackEnabled: Boolean(payload.defaultHijackEnabled),
    fastModeRewriteMode: normalizeProxyFastModeRewriteMode(
      payload.fastModeRewriteMode ?? payload.fast_mode_rewrite_mode,
    ),
    upstream429MaxRetries: normalizeProxyUpstream429MaxRetries(
      payload.upstream429MaxRetries ?? payload.upstream_429_max_retries,
    ),
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
    insertDirect: payload.insertDirect !== false,
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

function normalizePromptCacheConversationRequestPoint(
  raw: unknown,
): PromptCacheConversationRequestPoint | null {
  return normalizeConversationRequestPoint(raw);
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
  return {
    promptCacheKey,
    requestCount: normalizeFiniteNumber(payload.requestCount) ?? 0,
    totalTokens: normalizeFiniteNumber(payload.totalTokens) ?? 0,
    totalCost: normalizeFiniteNumber(payload.totalCost) ?? 0,
    createdAt: typeof payload.createdAt === "string" ? payload.createdAt : "",
    lastActivityAt:
      typeof payload.lastActivityAt === "string" ? payload.lastActivityAt : "",
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
  return {
    stickyKey,
    requestCount: normalizeFiniteNumber(payload.requestCount) ?? 0,
    totalTokens: normalizeFiniteNumber(payload.totalTokens) ?? 0,
    totalCost: normalizeFiniteNumber(payload.totalCost) ?? 0,
    createdAt: typeof payload.createdAt === "string" ? payload.createdAt : "",
    lastActivityAt:
      typeof payload.lastActivityAt === "string" ? payload.lastActivityAt : "",
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
  return {
    rangeStart:
      typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
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
  return {
    apiKeyConfigured: payload.apiKeyConfigured,
    maskedApiKey:
      typeof payload.maskedApiKey === "string" ? payload.maskedApiKey : null,
  };
}

function normalizePromptCacheConversationsResponse(
  raw: unknown,
): PromptCacheConversationsResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const conversationsRaw = Array.isArray(payload.conversations)
    ? payload.conversations
    : [];
  return {
    rangeStart:
      typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
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
    proxy: normalizeProxySettings(payload.proxy),
    forwardProxy: normalizeForwardProxySettings(payload.forwardProxy),
    pricing: normalizePricingSettings(payload.pricing),
  };
}

export interface RateWindowSnapshot {
  usedPercent: number;
  usedText: string;
  limitText: string;
  resetsAt?: string | null;
  windowDurationMins: number;
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
  guardEnabled: boolean
  lookbackHours?: number | null
  maxConversations?: number | null
  allowCutOut: boolean
  allowCutIn: boolean
}

export interface EffectiveConversationGuard {
  tagId: number
  tagName: string
  lookbackHours: number
  maxConversations: number
}

export interface EffectiveRoutingRule extends TagRoutingRule {
  sourceTagIds: number[]
  sourceTagNames: string[]
  guardRules: EffectiveConversationGuard[]
}

export interface AccountTagSummary {
  id: number
  name: string
  routingRule: TagRoutingRule
}

export interface TagSummary {
  id: number
  name: string
  routingRule: TagRoutingRule
  accountCount: number
  groupCount: number
  updatedAt: string
}

export type TagDetail = TagSummary

export interface TagListResponse {
  writesEnabled: boolean
  items: TagSummary[]
}

export interface UpstreamAccountSummary {
  id: number;
  kind: "oauth_codex" | "api_key_codex" | string;
  provider: string;
  displayName: string;
  groupName?: string | null;
  isMother: boolean;
  status: "active" | "syncing" | "needs_reauth" | "error" | "disabled" | string;
  enabled: boolean;
  email?: string | null;
  chatgptAccountId?: string | null;
  planType?: string | null;
  maskedApiKey?: string | null;
  lastSyncedAt?: string | null;
  lastSuccessfulSyncAt?: string | null;
  lastError?: string | null;
  lastErrorAt?: string | null;
  tokenExpiresAt?: string | null;
  primaryWindow?: RateWindowSnapshot | null;
  secondaryWindow?: RateWindowSnapshot | null;
  credits?: CreditsSnapshot | null;
  localLimits?: LocalLimitSnapshot | null;
  duplicateInfo?: UpstreamAccountDuplicateInfo | null;
  tags: AccountTagSummary[];
  effectiveRoutingRule: EffectiveRoutingRule;
}

export interface UpstreamAccountDetail extends UpstreamAccountSummary {
  note?: string | null;
  upstreamBaseUrl?: string | null;
  chatgptUserId?: string | null;
  lastRefreshedAt?: string | null;
  history: UpstreamAccountHistoryPoint[];
}

export interface UpstreamAccountGroupSummary {
  groupName: string
  note?: string | null
}

export interface PoolRoutingSettings {
  apiKeyConfigured: boolean;
  maskedApiKey?: string | null;
}

export interface UpdatePoolRoutingSettingsPayload {
  apiKey?: string;
}

export interface UpstreamAccountListResponse {
  writesEnabled: boolean;
  items: UpstreamAccountSummary[];
  groups: UpstreamAccountGroupSummary[];
  routing?: PoolRoutingSettings | null;
}

export interface LoginSessionStatusResponse {
  loginId: string;
  status: "pending" | "completed" | "failed" | "expired" | string;
  authUrl?: string | null;
  redirectUri?: string | null;
  expiresAt: string;
  accountId?: number | null;
  error?: string | null;
}

export interface CreateOauthLoginSessionPayload {
  displayName?: string;
  groupName?: string;
  note?: string;
  groupNote?: string;
  accountId?: number;
  tagIds?: number[];
  isMother?: boolean;
}

export interface CompleteOauthLoginSessionPayload {
  callbackUrl: string;
}

export interface CreateApiKeyAccountPayload {
  displayName: string;
  groupName?: string;
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
  note?: string
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
  const payload = (raw ?? {}) as Record<string, unknown>
  return {
    guardEnabled: payload.guardEnabled === true,
    lookbackHours: normalizeFiniteNumber(payload.lookbackHours) ?? null,
    maxConversations: normalizeFiniteNumber(payload.maxConversations) ?? null,
    allowCutOut: payload.allowCutOut !== false,
    allowCutIn: payload.allowCutIn !== false,
  }
}

function normalizeEffectiveConversationGuard(raw: unknown): EffectiveConversationGuard | null {
  const payload = (raw ?? {}) as Record<string, unknown>
  const tagId = normalizeFiniteNumber(payload.tagId)
  const tagName = typeof payload.tagName === 'string' ? payload.tagName : ''
  const lookbackHours = normalizeFiniteNumber(payload.lookbackHours)
  const maxConversations = normalizeFiniteNumber(payload.maxConversations)
  if (tagId == null || !tagName || lookbackHours == null || maxConversations == null) return null
  return { tagId, tagName, lookbackHours, maxConversations }
}

function normalizeAccountTagSummary(raw: unknown): AccountTagSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>
  const id = normalizeFiniteNumber(payload.id)
  const name = typeof payload.name === 'string' ? payload.name : ''
  if (id == null || !name) return null
  return {
    id,
    name,
    routingRule: normalizeTagRoutingRule(payload.routingRule),
  }
}

function normalizeEffectiveRoutingRule(raw: unknown): EffectiveRoutingRule {
  const payload = (raw ?? {}) as Record<string, unknown>
  const sourceTagIds = Array.isArray(payload.sourceTagIds)
    ? payload.sourceTagIds.map(normalizeFiniteNumber).filter((value): value is number => value != null)
    : []
  const sourceTagNames = Array.isArray(payload.sourceTagNames)
    ? payload.sourceTagNames.filter((value): value is string => typeof value === 'string')
    : []
  const guardRules = Array.isArray(payload.guardRules)
    ? payload.guardRules.map(normalizeEffectiveConversationGuard).filter((value): value is EffectiveConversationGuard => value != null)
    : []
  return {
    ...normalizeTagRoutingRule(payload),
    sourceTagIds,
    sourceTagNames,
    guardRules,
  }
}

function normalizeTagSummary(raw: unknown): TagSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>
  const id = normalizeFiniteNumber(payload.id)
  const name = typeof payload.name === 'string' ? payload.name : ''
  const accountCount = normalizeFiniteNumber(payload.accountCount)
  const groupCount = normalizeFiniteNumber(payload.groupCount)
  const updatedAt = typeof payload.updatedAt === 'string' ? payload.updatedAt : ''
  if (id == null || !name || accountCount == null || groupCount == null || !updatedAt) return null
  return {
    id,
    name,
    routingRule: normalizeTagRoutingRule(payload.routingRule),
    accountCount,
    groupCount,
    updatedAt,
  }
}

function normalizeUpstreamAccountSummary(raw: unknown): UpstreamAccountSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>
  const id = normalizeFiniteNumber(payload.id)
  const displayName = typeof payload.displayName === 'string' ? payload.displayName : ''
  const kind = typeof payload.kind === 'string' ? payload.kind : ''
  const provider = typeof payload.provider === 'string' ? payload.provider : ''
  const status = typeof payload.status === 'string' ? payload.status : 'error'
  if (id == null || !displayName || !kind || !provider) return null
  return {
    id,
    kind,
    provider,
    displayName,
    groupName: typeof payload.groupName === "string" ? payload.groupName : null,
    isMother: payload.isMother === true,
    status,
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
    lastError: typeof payload.lastError === "string" ? payload.lastError : null,
    lastErrorAt:
      typeof payload.lastErrorAt === "string" ? payload.lastErrorAt : null,
    tokenExpiresAt:
      typeof payload.tokenExpiresAt === "string"
        ? payload.tokenExpiresAt
        : null,
    primaryWindow: normalizeRateWindowSnapshot(payload.primaryWindow),
    secondaryWindow: normalizeRateWindowSnapshot(payload.secondaryWindow),
    credits: normalizeCreditsSnapshot(payload.credits),
    localLimits: normalizeLocalLimitSnapshot(payload.localLimits),
    duplicateInfo: normalizeUpstreamAccountDuplicateInfo(payload.duplicateInfo),
    tags: Array.isArray(payload.tags)
      ? payload.tags.map(normalizeAccountTagSummary).filter((item): item is AccountTagSummary => item != null)
      : [],
    effectiveRoutingRule: normalizeEffectiveRoutingRule(payload.effectiveRoutingRule),
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
  };
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
  };
}

function normalizeUpstreamAccountListResponse(
  raw: unknown,
): UpstreamAccountListResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const itemsRaw = Array.isArray(payload.items) ? payload.items : [];
  const groupsRaw = Array.isArray(payload.groups) ? payload.groups : [];
  return {
    writesEnabled: payload.writesEnabled !== false,
    items: itemsRaw
      .map(normalizeUpstreamAccountSummary)
      .filter((item): item is UpstreamAccountSummary => item != null),
    groups: groupsRaw
      .map(normalizeUpstreamAccountGroupSummary)
      .filter((item): item is UpstreamAccountGroupSummary => item != null),
    routing: normalizePoolRoutingSettings(payload.routing),
  };
}

function normalizeTagListResponse(raw: unknown): TagListResponse {
  const payload = (raw ?? {}) as Record<string, unknown>
  const itemsRaw = Array.isArray(payload.items) ? payload.items : []
  return {
    writesEnabled: payload.writesEnabled !== false,
    items: itemsRaw.map(normalizeTagSummary).filter((item): item is TagSummary => item != null),
  }
}

function normalizeLoginSessionStatusResponse(raw: unknown): LoginSessionStatusResponse {
  const payload = (raw ?? {}) as Record<string, unknown>
  const loginId = typeof payload.loginId === 'string' ? payload.loginId : ''
  const expiresAt = typeof payload.expiresAt === 'string' ? payload.expiresAt : ''
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
    accountId: accountId == null ? null : accountId,
    error: typeof payload.error === "string" ? payload.error : null,
  };
}

export async function fetchVersion(): Promise<VersionResponse> {
  return fetchJson<VersionResponse>("/api/version");
}

export async function fetchSettings(): Promise<SettingsPayload> {
  const response = await fetchJson<unknown>("/api/settings");
  return normalizeSettingsPayload(response);
}

export async function updateProxySettings(payload: {
  hijackEnabled: boolean;
  mergeUpstreamEnabled: boolean;
  enabledModels: string[];
  fastModeRewriteMode: ProxyFastModeRewriteMode;
  upstream429MaxRetries: number;
}): Promise<ProxySettings> {
  const response = await fetchJson<unknown>("/api/settings/proxy", {
    method: "PUT",
    body: JSON.stringify(payload),
  });
  return normalizeProxySettings(response);
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
  insertDirect: boolean;
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

export async function fetchPromptCacheConversations(limit: number) {
  const search = new URLSearchParams();
  search.set("limit", String(limit));
  const response = await fetchJson<unknown>(
    `/api/stats/prompt-cache-conversations?${search.toString()}`,
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
  return fetchJson<TimeseriesResponse>(
    `/api/stats/timeseries?${search.toString()}`,
    { signal: params?.signal },
  );
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

export async function fetchUpstreamAccounts(): Promise<UpstreamAccountListResponse> {
  const response = await fetchJson<unknown>("/api/pool/upstream-accounts");
  return normalizeUpstreamAccountListResponse(response);
}

export async function fetchTags(query?: FetchTagsQuery): Promise<TagListResponse> {
  const search = new URLSearchParams()
  if (query?.search) search.set('search', query.search)
  if (query?.hasAccounts != null) search.set('hasAccounts', String(query.hasAccounts))
  if (query?.guardEnabled != null) search.set('guardEnabled', String(query.guardEnabled))
  if (query?.allowCutIn != null) search.set('allowCutIn', String(query.allowCutIn))
  if (query?.allowCutOut != null) search.set('allowCutOut', String(query.allowCutOut))
  const response = await fetchJson<unknown>(search.size ? `/api/pool/tags?${search.toString()}` : '/api/pool/tags')
  return normalizeTagListResponse(response)
}

export async function createTag(payload: CreateTagPayload): Promise<TagDetail> {
  const response = await fetchJson<unknown>('/api/pool/tags', {
    method: 'POST',
    body: JSON.stringify(payload),
  })
  const normalized = normalizeTagSummary(response)
  if (!normalized) throw new Error('Request failed: invalid tag payload')
  return normalized
}

export async function updateTag(tagId: number, payload: UpdateTagPayload): Promise<TagDetail> {
  const response = await fetchJson<unknown>(`/api/pool/tags/${tagId}`, {
    method: 'PATCH',
    body: JSON.stringify(payload),
  })
  const normalized = normalizeTagSummary(response)
  if (!normalized) throw new Error('Request failed: invalid tag payload')
  return normalized
}

export async function deleteTag(tagId: number): Promise<void> {
  await fetchJson(`/api/pool/tags/${tagId}`, { method: 'DELETE' })
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
  limit: number,
  signal?: AbortSignal,
): Promise<UpstreamStickyConversationsResponse> {
  const search = new URLSearchParams();
  search.set("limit", String(limit));
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
): Promise<UpstreamAccountDetail> {
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-accounts/${accountId}`,
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

export async function fetchOauthLoginSession(
  loginId: string,
): Promise<LoginSessionStatusResponse> {
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-accounts/oauth/login-sessions/${encodeURIComponent(loginId)}`,
  );
  return normalizeLoginSessionStatusResponse(response);
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
      method: 'PUT',
      body: JSON.stringify(payload),
    },
  )
  const normalized = normalizeUpstreamAccountGroupSummary(response)
  if (!normalized) {
    throw new Error('Request failed: invalid upstream account group payload')
  }
  return normalized
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

export function createEventSource(path: string) {
  return new EventSource(withBase(path));
}
