import type { ForwardProxyBindingNode, PoolRoutingSelectionAudit } from "./core-foundation";
import {
  normalizeFiniteNumber,
  normalizeForwardProxyBindingNode,
  normalizePoolRoutingSettings,
  normalizeRoutingStateVersion,
  normalizeStringArray,
} from "./core-foundation";
import type {
  ImportedOauthImportResponse,
  ImportedOauthImportResult,
  ImportedOauthMatchSummary,
  ImportedOauthValidationCounts,
  ImportedOauthValidationFailedEventPayload,
  ImportedOauthValidationJobResponse,
  ImportedOauthValidationResponse,
  ImportedOauthValidationRow,
  ImportedOauthValidationRowEventPayload,
  ImportedOauthValidationSnapshotEventPayload,
} from "./core-upstream-contract-types";
import {
  normalizeGroupAccountRoutingRule,
  normalizePoolRoutingTimeoutSettings,
  normalizeRateWindowActualUsage,
  normalizeRoutingTimeoutFieldSources,
  normalizeTagSummary,
  normalizeUpstreamAccountActionEvent,
  normalizeUpstreamAccountGroupMaxRetries,
  normalizeUpstreamAccountHistoryPoint,
  normalizeUpstreamAccountListMetrics,
  normalizeUpstreamAccountSummary,
} from "./core-upstream-normalizers-base";
import type {
  BulkUpstreamAccountActionResponse,
  BulkUpstreamAccountActionResult,
  BulkUpstreamAccountSyncCounts,
  BulkUpstreamAccountSyncFailedEventPayload,
  BulkUpstreamAccountSyncJobResponse,
  BulkUpstreamAccountSyncRow,
  BulkUpstreamAccountSyncRowEventPayload,
  BulkUpstreamAccountSyncSnapshot,
  BulkUpstreamAccountSyncSnapshotEventPayload,
  LoginSessionStatusResponse,
  ModelMapping,
  ModelRoutingHistoryResponse,
  ModelRoutingLiveResponse,
  ModelRoutingState,
  ModelRoutingTimelineRecord,
  OauthIdentityConfirmation,
  OauthIdentitySummary,
  OauthInviteSummary,
  OauthMailboxCodeSummary,
  OauthMailboxSession,
  OauthMailboxStatus,
  TagListResponse,
  TagSummary,
  UpstreamAccountActionEvent,
  UpstreamAccountActionEventListResponse,
  UpstreamAccountDetail,
  UpstreamAccountGroupSummary,
  UpstreamAccountHistoryPoint,
  UpstreamAccountListResponse,
  UpstreamAccountSummary,
  UpstreamAccountWindowUsageItem,
  UpstreamAccountWindowUsageResponse,
} from "./core-upstream-types";

export function normalizeModelRoutingState(raw: unknown): ModelRoutingState | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const model = typeof payload.model === "string" ? payload.model.trim() : "";
  const state = typeof payload.state === "string" ? payload.state : "available";
  const priority = typeof payload.priority === "string" ? payload.priority : "normal";
  const lastSeenAt = typeof payload.lastSeenAt === "string" ? payload.lastSeenAt : "";
  if (!model || !lastSeenAt) return null;
  return {
    model,
    state,
    priority,
    failureCount: Math.max(0, Math.trunc(normalizeFiniteNumber(payload.failureCount) ?? 0)),
    changedAt: typeof payload.changedAt === "string" ? payload.changedAt : null,
    lastSeenAt,
    lastFailureAt: typeof payload.lastFailureAt === "string" ? payload.lastFailureAt : null,
    lastFailureKind: typeof payload.lastFailureKind === "string" ? payload.lastFailureKind : null,
    lastFailureMessage:
      typeof payload.lastFailureMessage === "string" ? payload.lastFailureMessage : null,
    cooldownUntil: typeof payload.cooldownUntil === "string" ? payload.cooldownUntil : null,
    cacheConcurrencyLimit: normalizeFiniteNumber(payload.cacheConcurrencyLimit),
    cacheRecoveryLimit: normalizeFiniteNumber(payload.cacheRecoveryLimit),
    cacheLowHitStreak: Math.max(
      0,
      Math.trunc(normalizeFiniteNumber(payload.cacheLowHitStreak) ?? 0),
    ),
    cacheCooldownLevel: Math.max(
      0,
      Math.trunc(normalizeFiniteNumber(payload.cacheCooldownLevel) ?? 0),
    ),
    cacheLastHitRatePercent: normalizeFiniteNumber(payload.cacheLastHitRatePercent),
    cacheUsageMissingSince:
      typeof payload.cacheUsageMissingSince === "string" ? payload.cacheUsageMissingSince : null,
    cacheUsageMissingReason:
      typeof payload.cacheUsageMissingReason === "string" ? payload.cacheUsageMissingReason : null,
    probeRequired: payload.probeRequired === true,
  };
}

export function normalizeModelRoutingTimelineRecord(
  raw: unknown,
): ModelRoutingTimelineRecord | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const id = typeof payload.id === "string" ? payload.id.trim() : "";
  const kind = typeof payload.kind === "string" ? payload.kind.trim() : "";
  const occurredAt = typeof payload.occurredAt === "string" ? payload.occurredAt : "";
  const accountId = normalizeFiniteNumber(payload.accountId);
  const model = typeof payload.model === "string" ? payload.model.trim() : "";
  if (!id || !kind || !occurredAt || accountId == null || accountId <= 0 || !model) {
    return null;
  }
  return {
    id,
    kind,
    occurredAt,
    accountId: Math.trunc(accountId),
    accountDisplayName:
      typeof payload.accountDisplayName === "string" && payload.accountDisplayName.trim()
        ? payload.accountDisplayName.trim()
        : undefined,
    model,
    attemptId: typeof payload.attemptId === "string" ? payload.attemptId : null,
    invokeId: typeof payload.invokeId === "string" ? payload.invokeId : null,
    attemptIndex: normalizeFiniteNumber(payload.attemptIndex),
    sameAccountRetryIndex: normalizeFiniteNumber(payload.sameAccountRetryIndex),
    routingSource: typeof payload.routingSource === "string" ? payload.routingSource : null,
    routingSelectionAudit:
      payload.routingSelectionAudit && typeof payload.routingSelectionAudit === "object"
        ? (payload.routingSelectionAudit as PoolRoutingSelectionAudit)
        : null,
    status: typeof payload.status === "string" ? payload.status : null,
    httpStatus: normalizeFiniteNumber(payload.httpStatus),
    failureKind: typeof payload.failureKind === "string" ? payload.failureKind : null,
    totalLatencyMs: normalizeFiniteNumber(payload.totalLatencyMs),
    action: typeof payload.action === "string" ? payload.action : null,
    source: typeof payload.source === "string" ? payload.source : null,
    reasonCode: typeof payload.reasonCode === "string" ? payload.reasonCode : null,
    modelRouteStateBefore:
      typeof payload.modelRouteStateBefore === "string" ? payload.modelRouteStateBefore : null,
    modelRouteStateAfter:
      typeof payload.modelRouteStateAfter === "string" ? payload.modelRouteStateAfter : null,
    modelRoutePriorityBefore:
      typeof payload.modelRoutePriorityBefore === "string"
        ? payload.modelRoutePriorityBefore
        : null,
    modelRoutePriorityAfter:
      typeof payload.modelRoutePriorityAfter === "string" ? payload.modelRoutePriorityAfter : null,
    modelRouteFailureCount: normalizeFiniteNumber(payload.modelRouteFailureCount),
    modelRouteCooldownUntil:
      typeof payload.modelRouteCooldownUntil === "string" ? payload.modelRouteCooldownUntil : null,
  };
}

export function normalizeModelRoutingLiveResponse(raw: unknown): ModelRoutingLiveResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const groupsRaw = Array.isArray(payload.groups) ? payload.groups : [];
  const groups = groupsRaw.flatMap((group) => {
    const item = (group ?? {}) as Record<string, unknown>;
    const model = typeof item.model === "string" ? item.model.trim() : "";
    if (!model) return [];
    const accounts = (Array.isArray(item.accounts) ? item.accounts : []).flatMap((account) => {
      const route = normalizeModelRoutingState(account);
      const accountPayload = (account ?? {}) as Record<string, unknown>;
      const accountId = normalizeFiniteNumber(accountPayload.accountId);
      if (!route || accountId == null || accountId <= 0) return [];
      return [
        {
          ...route,
          accountId: Math.trunc(accountId),
          accountDisplayName:
            typeof accountPayload.accountDisplayName === "string" &&
            accountPayload.accountDisplayName.trim()
              ? accountPayload.accountDisplayName.trim()
              : undefined,
        },
      ];
    });
    return [{ model, accounts }];
  });
  return {
    generatedAt: typeof payload.generatedAt === "string" ? payload.generatedAt : "",
    groups,
    records: (Array.isArray(payload.records) ? payload.records : [])
      .map(normalizeModelRoutingTimelineRecord)
      .filter((item): item is ModelRoutingTimelineRecord => item != null),
  };
}

export function normalizeModelRoutingHistoryResponse(raw: unknown): ModelRoutingHistoryResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    items: (Array.isArray(payload.items) ? payload.items : [])
      .map(normalizeModelRoutingTimelineRecord)
      .filter((item): item is ModelRoutingTimelineRecord => item != null),
    nextCursor:
      typeof payload.nextCursor === "string" && payload.nextCursor.trim()
        ? payload.nextCursor
        : null,
  };
}

export function normalizeUpstreamAccountDetail(raw: unknown): UpstreamAccountDetail {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const summary = normalizeUpstreamAccountSummary(payload);
  if (!summary) {
    throw new Error("Request failed: invalid upstream account payload");
  }
  const historyRaw = Array.isArray(payload.history) ? payload.history : [];
  const catalogPayload = (payload.modelCatalog ?? {}) as Record<string, unknown>;
  const catalogModels = Array.isArray(catalogPayload.models)
    ? catalogPayload.models.filter((value): value is string => typeof value === "string")
    : [];
  const catalogErrorPayload = (catalogPayload.error ?? null) as Record<string, unknown> | null;
  return {
    ...summary,
    routingStateVersion: normalizeRoutingStateVersion(payload.routingStateVersion),
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
    modelRoutingStates: Array.isArray(payload.modelRoutingStates)
      ? payload.modelRoutingStates
          .map(normalizeModelRoutingState)
          .filter((item): item is ModelRoutingState => item != null)
      : [],
    modelMappings: Array.isArray(payload.modelMappings)
      ? payload.modelMappings
          .map(normalizeModelMapping)
          .filter((item): item is ModelMapping => item != null)
      : [],
    modelCatalog: {
      models: catalogModels,
      status:
        typeof catalogPayload.status === "string" && catalogPayload.status.trim()
          ? catalogPayload.status
          : "never",
      lastAttemptedAt:
        typeof catalogPayload.lastAttemptedAt === "string" ? catalogPayload.lastAttemptedAt : null,
      lastSuccessfulAt:
        typeof catalogPayload.lastSuccessfulAt === "string"
          ? catalogPayload.lastSuccessfulAt
          : null,
      error:
        catalogErrorPayload &&
        typeof catalogErrorPayload.code === "string" &&
        typeof catalogErrorPayload.message === "string"
          ? { code: catalogErrorPayload.code, message: catalogErrorPayload.message }
          : null,
      stale: catalogPayload.stale === true,
    },
  };
}

export function normalizeModelMapping(raw: unknown): ModelMapping | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const sourceModel = typeof payload.sourceModel === "string" ? payload.sourceModel.trim() : "";
  const targetModel = typeof payload.targetModel === "string" ? payload.targetModel.trim() : "";
  if (!sourceModel || !targetModel) return null;
  return {
    sourceModel,
    targetModel,
    enabled: payload.enabled !== false,
  };
}

export function normalizeUpstreamAccountGroupSummary(
  raw: unknown,
): UpstreamAccountGroupSummary | null {
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

export function normalizeUpstreamAccountListResponse(raw: unknown): UpstreamAccountListResponse {
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

export function normalizeUpstreamAccountActionEventListResponse(
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

export function normalizeUpstreamAccountWindowUsageResponse(
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

export function normalizeTagListResponse(raw: unknown): TagListResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const itemsRaw = Array.isArray(payload.items) ? payload.items : [];
  return {
    writesEnabled: payload.writesEnabled !== false,
    items: itemsRaw.map(normalizeTagSummary).filter((item): item is TagSummary => item != null),
  };
}

export function normalizeLoginSessionStatusResponse(raw: unknown): LoginSessionStatusResponse {
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

export function normalizeOauthIdentityConfirmation(raw: unknown): OauthIdentityConfirmation | null {
  const payload = raw as Record<string, unknown> | null | undefined;
  if (!payload || typeof payload !== "object") return null;
  return {
    current: normalizeOauthIdentitySummary(payload.current),
    incoming: normalizeOauthIdentitySummary(payload.incoming),
  };
}

export function normalizeOauthIdentitySummary(raw: unknown): OauthIdentitySummary {
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

export function normalizeImportedOauthMatchSummary(raw: unknown): ImportedOauthMatchSummary | null {
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

export function normalizeImportedOauthValidationRow(
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
      typeof payload.chatgptAccountId === "string" ? payload.chatgptAccountId : null,
    chatgptUserId: typeof payload.chatgptUserId === "string" ? payload.chatgptUserId : null,
    displayName: typeof payload.displayName === "string" ? payload.displayName : null,
    tokenExpiresAt: typeof payload.tokenExpiresAt === "string" ? payload.tokenExpiresAt : null,
    matchedAccount: normalizeImportedOauthMatchSummary(payload.matchedAccount),
    status,
    detail: typeof payload.detail === "string" ? payload.detail : null,
    attempts,
  };
}

export function normalizeImportedOauthValidationResponse(
  raw: unknown,
): ImportedOauthValidationResponse {
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

export function normalizeImportedOauthImportResult(raw: unknown): ImportedOauthImportResult | null {
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

export function normalizeImportedOauthValidationCounts(
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

export function normalizeImportedOauthValidationJobResponse(
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

export function normalizeBulkUpstreamAccountActionResult(
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

export function normalizeBulkUpstreamAccountSyncCounts(
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

export function normalizeBulkUpstreamAccountSyncRow(
  raw: unknown,
): BulkUpstreamAccountSyncRow | null {
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

export function normalizeBulkUpstreamAccountSyncSnapshot(
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

export function normalizeBulkUpstreamAccountActionResponse(
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

export function normalizeBulkUpstreamAccountSyncJobResponse(
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

export function normalizeImportedOauthImportResponse(raw: unknown): ImportedOauthImportResponse {
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

export function normalizeOauthMailboxSession(raw: unknown): OauthMailboxSession {
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

export function normalizeOauthMailboxCodeSummary(raw: unknown): OauthMailboxCodeSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const value = typeof payload.value === "string" ? payload.value : "";
  const source = typeof payload.source === "string" ? payload.source : "";
  const updatedAt = typeof payload.updatedAt === "string" ? payload.updatedAt : "";
  if (!value || !source || !updatedAt) return null;
  return { value, source, updatedAt };
}

export function normalizeOauthInviteSummary(raw: unknown): OauthInviteSummary | null {
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

export function normalizeOauthMailboxStatus(raw: unknown): OauthMailboxStatus | null {
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
