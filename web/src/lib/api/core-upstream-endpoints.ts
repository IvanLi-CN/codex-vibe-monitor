import type {
  ForwardProxyBindingNode,
  StickyKeyConversationSelection,
  UpstreamStickyConversationsResponse,
} from "./core-foundation";
import {
  ensureJsonRequestOk,
  fetchJson,
  normalizeForwardProxyBindingNode,
  normalizePoolRoutingSettings,
  normalizeUpstreamStickyConversationsResponse,
  withBase,
} from "./core-foundation";
import type {
  ApiKeyGroupMigrationPreflight,
  ApiKeyGroupMigrationResult,
  CompleteOauthLoginSessionPayload,
  ConfirmApiKeyGroupMigrationPayload,
  CreateApiKeyAccountPayload,
  CreateOauthMailboxSessionPayload,
  CreateTagPayload,
  FetchTagsQuery,
  ImportedOauthImportResponse,
  ImportedOauthValidationJobResponse,
  ImportedOauthValidationResponse,
  ImportValidatedOauthAccountsPayload,
  OauthMailboxStatusRequestPayload,
  UpdateTagPayload,
  UpdateUpstreamAccountGroupPayload,
  UpdateUpstreamAccountPayload,
  ValidateImportedOauthAccountsPayload,
} from "./core-upstream-contract-types";
import { normalizeTagSummary } from "./core-upstream-normalizers-base";
import {
  normalizeBulkUpstreamAccountActionResponse,
  normalizeBulkUpstreamAccountSyncJobResponse,
  normalizeImportedOauthImportResponse,
  normalizeImportedOauthValidationJobResponse,
  normalizeImportedOauthValidationResponse,
  normalizeLoginSessionStatusResponse,
  normalizeModelRoutingHistoryResponse,
  normalizeModelRoutingLiveResponse,
  normalizeModelRoutingState,
  normalizeOauthMailboxSession,
  normalizeOauthMailboxStatus,
  normalizeTagListResponse,
  normalizeUpstreamAccountActionEventListResponse,
  normalizeUpstreamAccountDetail,
  normalizeUpstreamAccountGroupSummary,
  normalizeUpstreamAccountListResponse,
  normalizeUpstreamAccountWindowUsageResponse,
} from "./core-upstream-normalizers-response";
import type {
  BulkUpstreamAccountActionPayload,
  BulkUpstreamAccountActionResponse,
  BulkUpstreamAccountSyncJobPayload,
  BulkUpstreamAccountSyncJobResponse,
  CreateOauthLoginSessionPayload,
  FetchModelRoutingHistoryQuery,
  FetchModelRoutingLiveQuery,
  FetchUpstreamAccountActionEventsQuery,
  FetchUpstreamAccountsQuery,
  LoginSessionStatusResponse,
  ModelRoutingHistoryResponse,
  ModelRoutingLiveResponse,
  ModelRoutingState,
  OauthMailboxSession,
  OauthMailboxStatus,
  PoolRoutingSettings,
  TagDetail,
  TagListResponse,
  UpdateOauthLoginSessionPayload,
  UpdatePoolRoutingSettingsPayload,
  UpdateUpstreamAccountModelMappingsPayload,
  UpstreamAccountActionEventListResponse,
  UpstreamAccountDetail,
  UpstreamAccountGroupSummary,
  UpstreamAccountListResponse,
  UpstreamAccountWindowUsageResponse,
} from "./core-upstream-types";

const OAUTH_LOGIN_SESSION_BASE_UPDATED_AT_HEADER = "X-Codex-Login-Session-Base-Updated-At";

function withOauthLoginSessionBaseUpdatedAtHeader(
  baseUpdatedAt: string | null | undefined,
  init: RequestInit,
): RequestInit {
  const normalizedBaseUpdatedAt = baseUpdatedAt?.trim();
  if (!normalizedBaseUpdatedAt) return init;
  const headers = new Headers(init.headers);
  if (!headers.has("Content-Type")) headers.set("Content-Type", "application/json");
  headers.set(OAUTH_LOGIN_SESSION_BASE_UPDATED_AT_HEADER, normalizedBaseUpdatedAt);
  return { ...init, headers };
}

export async function fetchUpstreamAccounts(
  query?: FetchUpstreamAccountsQuery,
): Promise<UpstreamAccountListResponse> {
  const search = new URLSearchParams();
  if (query?.kind) search.set("kind", query.kind);
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
  if (query?.kind) search.set("kind", query.kind);
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

export async function fetchModelRoutingLive(
  query?: FetchModelRoutingLiveQuery,
): Promise<ModelRoutingLiveResponse> {
  const search = new URLSearchParams();
  if (query?.window) search.set("window", query.window);
  if (query?.model?.trim()) search.set("model", query.model.trim());
  if (query?.state?.trim()) search.set("state", query.state.trim());
  if (query?.limit != null) search.set("limit", String(query.limit));
  const response = await fetchJson<unknown>(
    search.size
      ? `/api/pool/model-routing-live?${search.toString()}`
      : "/api/pool/model-routing-live",
    { signal: query?.signal },
  );
  return normalizeModelRoutingLiveResponse(response);
}

export async function fetchUpstreamAccountModelRoutingEvents(
  accountId: number,
  query: FetchModelRoutingHistoryQuery,
): Promise<ModelRoutingHistoryResponse> {
  const search = new URLSearchParams({ model: query.model.trim() });
  if (query.cursor?.trim()) search.set("cursor", query.cursor.trim());
  if (query.pageSize != null) search.set("pageSize", String(query.pageSize));
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-accounts/${accountId}/model-routing-events?${search.toString()}`,
    { signal: query.signal },
  );
  return normalizeModelRoutingHistoryResponse(response);
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

export async function fetchPoolRoutingSettings(): Promise<PoolRoutingSettings> {
  const response = await fetchJson<unknown>("/api/pool/routing-settings");
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

export async function resetUpstreamAccountModelRouting(
  accountId: number,
  model: string,
): Promise<ModelRoutingState> {
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-accounts/${accountId}/model-routing/reset`,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ model }),
    },
  );
  const normalized = normalizeModelRoutingState(response);
  if (!normalized) throw new Error("Request failed: invalid model routing response");
  return normalized;
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

export async function preflightApiKeyGroupMigration(): Promise<ApiKeyGroupMigrationPreflight> {
  return fetchJson<ApiKeyGroupMigrationPreflight>(
    "/api/pool/upstream-accounts/api-keys/migration/preflight",
    { method: "POST" },
  );
}

export async function confirmApiKeyGroupMigration(
  payload: ConfirmApiKeyGroupMigrationPayload,
): Promise<ApiKeyGroupMigrationResult> {
  return fetchJson<ApiKeyGroupMigrationResult>(
    "/api/pool/upstream-accounts/api-keys/migration/confirm",
    { method: "POST", body: JSON.stringify(payload) },
  );
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

export async function updateUpstreamAccountModelMappings(
  accountId: number,
  payload: UpdateUpstreamAccountModelMappingsPayload,
): Promise<UpstreamAccountDetail> {
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-accounts/${accountId}/model-mappings`,
    {
      method: "PUT",
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

export async function refreshUpstreamAccountModels(
  accountId: number,
): Promise<UpstreamAccountDetail> {
  const response = await fetchJson<unknown>(
    `/api/pool/upstream-accounts/${accountId}/models/refresh`,
    { method: "POST" },
  );
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
  const resolvedPath = withBase(path);
  if (typeof window !== "undefined" && window.__CVM_DEMO_CREATE_EVENT_SOURCE__) {
    return window.__CVM_DEMO_CREATE_EVENT_SOURCE__(resolvedPath);
  }
  return new EventSource(resolvedPath);
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
