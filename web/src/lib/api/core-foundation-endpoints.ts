import { getBrowserTimeZone } from "../timeZone";
import {
  ApiRequestError,
  buildRequestError,
  fetchJson,
  forwardProxyValidationTimeoutMs,
  resolveForwardProxyHistoryTimeZone,
  withBase,
} from "./core-foundation-base";
import type {
  ErrorDistributionResponse,
  FailureScope,
  FailureSummaryResponse,
  ParallelWorkStatsResponse,
  PerfStatsQuery,
  PerfStatsResponse,
  QuotaSnapshot,
} from "./core-foundation-dashboard-types";
import {
  normalizeForwardProxyLiveStatsResponse,
  normalizeForwardProxyRefreshSubscriptionsResult,
  normalizeForwardProxySettings,
  normalizeForwardProxyTimeseriesResponse,
  normalizeParallelWorkStatsResponse,
  normalizePricingSettings,
  normalizeProxySettings,
  normalizeTimeseriesResponse,
} from "./core-foundation-normalizers-base";
import {
  normalizeDashboardActivityRecentResponse,
  normalizeDashboardActivityResponse,
  normalizeDashboardNetworkTimeseriesResponse,
  normalizeDashboardRecentNetworkWindowResponse,
  normalizeExternalApiKeyListResponse,
  normalizeExternalApiKeyMutationResponse,
  normalizeExternalApiKeySecretResponse,
  normalizeForwardProxyValidationResult,
  normalizePromptCacheConversationsResponse,
  normalizeSettingsPayload,
  normalizeSystemStatusResponse,
  normalizeSystemTaskRunsResponse,
  normalizeUpstreamAccountActivityResponse,
} from "./core-foundation-normalizers-dashboard";
import {
  normalizeBulkPromptCacheConversationBindingActionResponse,
  normalizePromptCacheConversationBindingResponse,
  normalizePromptCacheConversationOperationEventListResponse,
} from "./core-foundation-normalizers-routing";
import type {
  ApiInvocationRecordDetailResponse,
  ApiInvocationRequestBodyResponse,
  ApiInvocationResponseBodyResponse,
  ApiInvocationWorkflowDetailResponse,
  ApiPoolUpstreamRequestAttempt,
  InvocationRecordLocationQuery,
  InvocationRecordLocationResponse,
  InvocationRecordsNewCountResponse,
  InvocationRecordsQuery,
  InvocationRecordsResponse,
  InvocationRecordsSummaryResponse,
  InvocationSuggestionsResponse,
  ListResponse,
  LongTermStatsDimension,
  LongTermStatsOverviewResponse,
  LongTermStatsRange,
  LongTermStatsSeriesResponse,
  StatsResponse,
  UpstreamAccountAttemptListResponse,
} from "./core-foundation-record-types";
import type {
  BulkPromptCacheConversationBindingActionPayload,
  BulkPromptCacheConversationBindingActionResponse,
  FetchPromptCacheConversationOperationEventsQuery,
  ForwardProxyRefreshSubscriptionsResult,
  ForwardProxySettings,
  ForwardProxyValidationKind,
  ForwardProxyValidationResult,
  PricingSettings,
  PromptCacheConversationBindingResponse,
  PromptCacheConversationOperationEventListResponse,
  PromptCacheConversationPageQuery,
  PromptCacheConversationSelection,
  ProxyFastModeRewriteMode,
  ProxySettings,
  UpdatePromptCacheConversationBindingPayload,
  VersionResponse,
} from "./core-foundation-settings-types";
import type {
  ExternalApiKeyListResponse,
  ExternalApiKeyMutationResponse,
  ExternalApiKeySecretResponse,
  SettingsPayload,
  SystemStatusResponse,
  SystemTaskRunsResponse,
} from "./core-foundation-system-types";

function appendInvocationRecordsQuery(search: URLSearchParams, query: InvocationRecordsQuery) {
  if (query.page != null) search.set("page", String(query.page));
  if (query.pageSize != null) search.set("pageSize", String(query.pageSize));
  if (query.snapshotId != null) search.set("snapshotId", String(query.snapshotId));
  if (query.anchorId) search.set("anchorId", query.anchorId);
  if (query.sortBy) search.set("sortBy", query.sortBy);
  if (query.sortOrder) search.set("sortOrder", query.sortOrder);
  if (query.rangePreset) search.set("rangePreset", query.rangePreset);
  if (query.from) search.set("from", query.from);
  if (query.to) search.set("to", query.to);
  if (query.model) search.set("model", query.model);
  if (query.models && query.models.length > 0) search.set("models", query.models.join(","));
  if (query.modelTarget) search.set("modelTarget", query.modelTarget);
  if (query.modelRerouted != null) {
    search.set("modelRerouted", query.modelRerouted ? "true" : "false");
  }
  if (query.status) search.set("status", query.status);
  if (query.endpoint) search.set("endpoint", query.endpoint);
  const invokeId = query.invokeId ?? query.requestId;
  if (invokeId) search.set("invokeId", invokeId);
  if (query.attemptId) search.set("attemptId", query.attemptId);
  if (query.failureClass) search.set("failureClass", query.failureClass);
  if (query.failureKind) search.set("failureKind", query.failureKind);
  if (query.promptCacheKey) search.set("promptCacheKey", query.promptCacheKey);
  if (query.stickyKey) search.set("stickyKey", query.stickyKey);
  if (query.upstreamScope) search.set("upstreamScope", query.upstreamScope);
  if (query.upstreamAccountId != null)
    search.set("upstreamAccountId", String(query.upstreamAccountId));
  if (query.proxyDisplayName) search.set("proxyDisplayName", query.proxyDisplayName);
  if (query.transport) search.set("transport", query.transport);
  if (query.serviceTier) search.set("serviceTier", query.serviceTier);
  if (query.reasoningEffort) search.set("reasoningEffort", query.reasoningEffort);
  if (query.reasoningEfforts && query.reasoningEfforts.length > 0) {
    search.set("reasoningEfforts", query.reasoningEfforts.join(","));
  }
  if (query.requesterIp) search.set("requesterIp", query.requesterIp);
  if (query.keyword) search.set("keyword", query.keyword);
  if (query.minTotalTokens != null) search.set("minTotalTokens", String(query.minTotalTokens));
  if (query.maxTotalTokens != null) search.set("maxTotalTokens", String(query.maxTotalTokens));
  if (query.minTotalMs != null) search.set("minTotalMs", String(query.minTotalMs));
  if (query.maxTotalMs != null) search.set("maxTotalMs", String(query.maxTotalMs));
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
  return fetchJson<InvocationRecordsResponse>(`/api/invocations?${search.toString()}`, {
    signal: query.signal,
  });
}

export async function fetchInvocationRecordLocation(query: InvocationRecordLocationQuery) {
  const search = new URLSearchParams({
    pageSize: String(query.pageSize ?? 50),
  });
  const invokeId = query.invokeId ?? query.requestId;
  if (invokeId) search.set("invokeId", invokeId);
  if (query.attemptId) search.set("attemptId", query.attemptId);
  if (query.upstreamAccountId != null) {
    search.set("upstreamAccountId", String(query.upstreamAccountId));
  }
  return fetchJson<InvocationRecordLocationResponse>(
    `/api/invocations/locate?${search.toString()}`,
    { signal: query.signal },
  );
}

export async function fetchInvocationRecordsSummary(query: InvocationRecordsQuery) {
  const search = new URLSearchParams();
  appendInvocationRecordsQuery(search, query);
  return fetchJson<InvocationRecordsSummaryResponse>(
    `/api/invocations/summary?${search.toString()}`,
    { signal: query.signal },
  );
}

export async function fetchInvocationRecordsNewCount(query: InvocationRecordsQuery) {
  const search = new URLSearchParams();
  appendInvocationRecordsQuery(search, query);
  return fetchJson<InvocationRecordsNewCountResponse>(
    `/api/invocations/new-count?${search.toString()}`,
  );
}

export async function fetchInvocationSuggestions(query: InvocationRecordsQuery) {
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
  options?: {
    type?: "normal" | "remote_v2" | "compact" | "image";
    model?: string;
    stickyKey?: string;
    page?: number;
    pageSize?: number;
    signal?: AbortSignal;
  },
) {
  const search = new URLSearchParams({
    page: String(options?.page ?? 1),
    pageSize: String(options?.pageSize ?? 50),
  });
  if (options?.type) search.set("type", options.type);
  if (options?.model?.trim()) search.set("model", options.model.trim());
  if (options?.stickyKey?.trim()) search.set("stickyKey", options.stickyKey.trim());
  return fetchJson<UpstreamAccountAttemptListResponse>(
    `/api/pool/upstream-accounts/${encodeURIComponent(String(accountId))}/call-attempts?${search.toString()}`,
    { signal: options?.signal },
  );
}

export async function locateUpstreamAccountAttempt(
  accountId: number,
  attemptId: string,
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

export async function fetchInvocationAttemptResponseBody(id: number, attemptPublicId: string) {
  return fetchJson<ApiInvocationResponseBodyResponse>(
    `/api/invocations/${encodeURIComponent(String(id))}/attempts/${encodeURIComponent(attemptPublicId)}/response-body`,
  );
}

export async function fetchInvocationRequestBody(id: number) {
  return fetchJson<ApiInvocationRequestBodyResponse>(
    `/api/invocations/${encodeURIComponent(String(id))}/request-body`,
  );
}

export async function fetchInvocationWorkflowDetail(id: number) {
  return fetchJson<ApiInvocationWorkflowDetailResponse>(
    `/api/invocations/${encodeURIComponent(String(id))}/workflow-detail`,
  );
}

export async function fetchStats() {
  return fetchJson<StatsResponse>("/api/stats");
}

export async function fetchLongTermStatsOverview(range: LongTermStatsRange) {
  return fetchJson<LongTermStatsOverviewResponse>(
    `/api/stats/long-term/overview?range=${encodeURIComponent(range)}`,
  );
}

export async function fetchLongTermStatsSeries(
  range: LongTermStatsRange,
  dimension: LongTermStatsDimension,
  keys: string[],
) {
  const search = new URLSearchParams({ range, dimension });
  for (const key of keys) search.append("key", key);
  return fetchJson<LongTermStatsSeriesResponse>(`/api/stats/long-term/series?${search.toString()}`);
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
  cursor?: string;
}): Promise<SystemTaskRunsResponse> {
  const query = new URLSearchParams();
  if (params?.taskKind) query.set("taskKind", params.taskKind);
  if (params?.status) query.set("status", params.status);
  if (params?.startedAtFrom) query.set("startedAtFrom", params.startedAtFrom);
  if (params?.startedAtTo) query.set("startedAtTo", params.startedAtTo);
  if (params?.limit != null) query.set("limit", String(params.limit));
  if (params?.page != null) query.set("page", String(params.page));
  if (params?.pageSize != null) query.set("pageSize", String(params.pageSize));
  if (params?.cursor) query.set("cursor", params.cursor);
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

export async function rotateExternalApiKey(id: number): Promise<ExternalApiKeySecretResponse> {
  const response = await fetchJson<unknown>(`/api/settings/external-api-keys/${id}/rotate`, {
    method: "POST",
  });
  return normalizeExternalApiKeySecretResponse(response);
}

export async function disableExternalApiKey(id: number): Promise<ExternalApiKeyMutationResponse> {
  const response = await fetchJson<unknown>(`/api/settings/external-api-keys/${id}/disable`, {
    method: "POST",
  });
  return normalizeExternalApiKeyMutationResponse(response);
}

export async function updatePricingSettings(payload: PricingSettings): Promise<PricingSettings> {
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
  const response = await fetchJson<unknown>("/api/settings/forward-proxy/refresh-subscriptions", {
    method: "POST",
    body: JSON.stringify({}),
  });
  return normalizeForwardProxyRefreshSubscriptionsResult(response);
}

export function createForwardProxyNodeLatencyTestEventSource(proxyKey: string): EventSource {
  return new EventSource(
    withBase(`/api/settings/forward-proxy/nodes/${encodeURIComponent(proxyKey)}/test-stream`),
  );
}

export function createForwardProxyNodesLatencyTestEventSource(proxyKeys: string[]): EventSource {
  const query = new URLSearchParams();
  for (const proxyKey of proxyKeys) {
    query.append("key", proxyKey);
  }
  return new EventSource(
    withBase(`/api/settings/forward-proxy/nodes/test-stream?${query.toString()}`),
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
    const response = await fetchJson<unknown>("/api/settings/forward-proxy/validate", {
      method: "POST",
      body: JSON.stringify(payload),
      signal: controller.signal,
    });
    return normalizeForwardProxyValidationResult(response);
  } catch (err) {
    if (err instanceof Error && err.name === "AbortError") {
      throw new Error(`validation request timed out after ${Math.floor(timeoutMs / 1000)}s`);
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }
}

export async function fetchSummary(
  window: string,
  options?: {
    limit?: number;
    timeZone?: string;
    upstreamAccountId?: number;
    signal?: AbortSignal;
  },
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
    includeRecent?: boolean;
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
  if (options?.includeRecent !== undefined) {
    search.set("includeRecent", options.includeRecent ? "true" : "false");
  }
  const response = await fetchJson<unknown>(`/api/stats/dashboard-activity?${search.toString()}`, {
    signal: options?.signal,
  });
  return normalizeDashboardActivityResponse(response);
}

export async function fetchDashboardActivityRecent(options: {
  rangeStart: string;
  rangeEnd: string;
  snapshotId: number;
  recentLimit?: number;
  signal?: AbortSignal;
}) {
  const search = new URLSearchParams({
    rangeStart: options.rangeStart,
    rangeEnd: options.rangeEnd,
    snapshotId: String(options.snapshotId),
  });
  if (options.recentLimit !== undefined) {
    search.set("recentLimit", String(options.recentLimit));
  }
  const response = await fetchJson<unknown>(
    `/api/stats/dashboard-activity/recent?${search.toString()}`,
    { signal: options.signal },
  );
  return normalizeDashboardActivityRecentResponse(response);
}

export async function fetchDashboardNetworkTimeseries(
  range: string,
  options?: {
    timeZone?: string;
    upstreamAccountId?: number;
    signal?: AbortSignal;
  },
) {
  const search = new URLSearchParams();
  search.set("range", range);
  search.set("timeZone", options?.timeZone ?? getBrowserTimeZone());
  if (options?.upstreamAccountId !== undefined) {
    search.set("upstreamAccountId", String(options.upstreamAccountId));
  }
  const response = await fetchJson<unknown>(
    `/api/stats/dashboard-network-timeseries?${search.toString()}`,
    { signal: options?.signal },
  );
  return normalizeDashboardNetworkTimeseriesResponse(response);
}

export async function fetchDashboardRecentNetworkWindow(options?: { signal?: AbortSignal }) {
  const response = await fetchJson<unknown>("/api/stats/dashboard-network-recent", {
    signal: options?.signal,
  });
  return normalizeDashboardRecentNetworkWindowResponse(response);
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
    `/api/stats/prompt-cache-conversation-bindings/${encodeURIComponent(promptCacheKey)}`,
    { signal },
  );
  return normalizePromptCacheConversationBindingResponse(raw, promptCacheKey);
}

export async function fetchPromptCacheConversationOperationEvents(
  promptCacheKey: string,
  query?: FetchPromptCacheConversationOperationEventsQuery & {
    signal?: AbortSignal;
  },
): Promise<PromptCacheConversationOperationEventListResponse> {
  const search = new URLSearchParams();
  if (query?.page != null) {
    search.set("page", String(query.page));
  }
  if (query?.pageSize != null) {
    search.set("pageSize", String(query.pageSize));
  }
  if (query?.infoType) {
    search.set("infoType", query.infoType);
  }
  if (query?.routingScope) {
    search.set("routingScope", query.routingScope);
  }
  if (query?.routingModel?.trim()) {
    search.set("routingModel", query.routingModel.trim());
  }
  const suffix = search.toString() ? `?${search.toString()}` : "";
  const raw = await fetchJson<unknown>(
    `/api/stats/prompt-cache-conversation-binding-events/${encodeURIComponent(promptCacheKey)}${suffix}`,
    { signal: query?.signal },
  );
  return normalizePromptCacheConversationOperationEventListResponse(raw);
}

export async function updatePromptCacheConversationBinding(
  promptCacheKey: string,
  payload: UpdatePromptCacheConversationBindingPayload,
  signal?: AbortSignal,
): Promise<PromptCacheConversationBindingResponse> {
  const raw = await fetchJson<Record<string, unknown>>(
    `/api/stats/prompt-cache-conversation-bindings/${encodeURIComponent(promptCacheKey)}`,
    {
      method: "PATCH",
      body: JSON.stringify(payload),
      signal,
    },
  );
  return normalizePromptCacheConversationBindingResponse(raw, promptCacheKey);
}

export async function resetPromptCacheConversationAffinity(
  promptCacheKey: string,
  signal?: AbortSignal,
): Promise<PromptCacheConversationBindingResponse> {
  const raw = await fetchJson<Record<string, unknown>>(
    `/api/stats/prompt-cache-conversation-bindings/reset-affinity/${encodeURIComponent(promptCacheKey)}`,
    { method: "POST", signal },
  );
  return normalizePromptCacheConversationBindingResponse(raw, promptCacheKey);
}

export async function bulkUpdatePromptCacheConversationBindings(
  payload: BulkPromptCacheConversationBindingActionPayload,
  signal?: AbortSignal,
): Promise<BulkPromptCacheConversationBindingActionResponse> {
  const raw = await fetchJson<unknown>(
    "/api/stats/prompt-cache-conversation-bindings/bulk-actions",
    {
      method: "POST",
      body: JSON.stringify(payload),
      signal,
    },
  );
  return normalizeBulkPromptCacheConversationBindingActionResponse(raw);
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
  if (options.blockedBindingUpstreamAccountId != null) {
    search.set("blockedBindingUpstreamAccountId", String(options.blockedBindingUpstreamAccountId));
  }
  if (options.blockedBindingConstraintSource) {
    search.set("blockedBindingConstraintSource", options.blockedBindingConstraintSource);
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
  const response = await fetchJson<unknown>(`/api/stats/timeseries?${search.toString()}`, {
    signal: params?.signal,
  });
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
  const response = await fetch(withBase(`/api/stats/parallel-work?${search.toString()}`), {
    headers,
    signal: params?.signal,
  });
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
  return fetchJson<ErrorDistributionResponse>(`/api/stats/errors?${search.toString()}`);
}

export async function fetchFailureSummary(range: string, params?: { timeZone?: string }) {
  const search = new URLSearchParams();
  search.set("range", range);
  search.set("timeZone", params?.timeZone ?? getBrowserTimeZone());
  return fetchJson<FailureSummaryResponse>(`/api/stats/failures/summary?${search.toString()}`);
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
  return fetchJson<PerfStatsResponse>(query ? `/api/stats/perf?${query}` : "/api/stats/perf");
}

export async function fetchQuotaSnapshot() {
  return fetchJson<QuotaSnapshot>("/api/quota/latest");
}
