import {
  normalizeBlockedBindingDiagnostic,
  normalizeCompactSupportState,
  normalizeFiniteNumber,
  normalizeStringArray,
} from "./core-foundation";
import type {
  AccountTagSummary,
  CapabilityOverride,
  CapabilitySupport,
  CodexImagegenRewriteMode,
  CreditsSnapshot,
  EffectiveRoutingRule,
  EffectiveRoutingRuleSource,
  EffectiveRoutingTimeoutFieldSources,
  GroupAccountRoutingRule,
  ImageToolRewriteMode,
  LocalLimitSnapshot,
  PoolRoutingTimeoutSettings,
  RateWindowActualUsage,
  RateWindowSnapshot,
  RequestCompressionAlgorithm,
  StatusChangeReasonFieldSources,
  StatusChangeReasons,
  TagRoutingRule,
  TagSummary,
  UpstreamAccountActionEvent,
  UpstreamAccountDuplicateInfo,
  UpstreamAccountHistoryPoint,
  UpstreamAccountListMetrics,
  UpstreamAccountSummary,
  UpstreamCapabilityState,
} from "./core-upstream-types";
import {
  buildDefaultStatusChangeReasonFieldSources,
  buildDefaultStatusChangeReasons,
  STATUS_CHANGE_REASON_CODES,
} from "./core-upstream-types";

export function normalizeRateWindowActualUsage(raw: unknown): RateWindowActualUsage | null {
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

export function normalizeRateWindowSnapshot(raw: unknown): RateWindowSnapshot | null {
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

export function normalizeCreditsSnapshot(raw: unknown): CreditsSnapshot | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  if (typeof payload.hasCredits !== "boolean" || typeof payload.unlimited !== "boolean")
    return null;
  return {
    hasCredits: payload.hasCredits,
    unlimited: payload.unlimited,
    balance: typeof payload.balance === "string" ? payload.balance : null,
  };
}

export function normalizeLocalLimitSnapshot(raw: unknown): LocalLimitSnapshot | null {
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

export function normalizeTagRoutingRule(raw: unknown): TagRoutingRule {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const availableModelsDefined =
    payload.availableModelsDefined === true ||
    (payload.availableModelsDefined == null && Array.isArray(payload.availableModels));
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
    availableModelsDefined,
    availableModelsMode:
      payload.availableModelsMode === "allowlist" || payload.availableModelsMode === "denylist"
        ? payload.availableModelsMode
        : availableModelsDefined
          ? "allowlist"
          : undefined,
  };
}

export function normalizeImageToolRewriteMode(raw: unknown): ImageToolRewriteMode {
  return raw === "fill_missing" || raw === "force_add" || raw === "force_remove"
    ? raw
    : "keep_original";
}

export function normalizeCodexImagegenRewriteMode(raw: unknown): CodexImagegenRewriteMode | null {
  if (raw === null) return null;
  return normalizeImageToolRewriteMode(raw);
}

export function normalizeCapabilitySupport(raw: unknown): CapabilitySupport {
  return raw === "supported" || raw === "unsupported" ? raw : "unknown";
}

export function normalizeCapabilityOverride(raw: unknown): CapabilityOverride | null {
  return raw === "supported" || raw === "unsupported" ? raw : null;
}

export function normalizeUpstreamCapabilityState(raw: unknown): UpstreamCapabilityState {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    observed: normalizeCapabilitySupport(payload.observed),
    override: normalizeCapabilityOverride(payload.override),
    effective: normalizeCapabilitySupport(payload.effective),
    observedAt: typeof payload.observedAt === "string" ? payload.observedAt : null,
    reason: typeof payload.reason === "string" ? payload.reason : null,
  };
}

export function normalizePoolRoutingTimeoutSettings(raw: unknown): PoolRoutingTimeoutSettings {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    responsesFirstByteTimeoutSecs:
      normalizeFiniteNumber(payload.responsesFirstByteTimeoutSecs) ?? 120,
    compactFirstByteTimeoutSecs:
      normalizeFiniteNumber(payload.compactFirstByteTimeoutSecs) ??
      normalizeFiniteNumber(payload.compactUpstreamHandshakeTimeoutSecs) ??
      300,
    imageFirstByteTimeoutSecs: normalizeFiniteNumber(payload.imageFirstByteTimeoutSecs) ?? 300,
    responsesStreamTimeoutSecs: normalizeFiniteNumber(payload.responsesStreamTimeoutSecs) ?? 300,
    compactStreamTimeoutSecs: normalizeFiniteNumber(payload.compactStreamTimeoutSecs) ?? 300,
  };
}

export function normalizeOptionalPoolRoutingTimeoutSettings(
  raw: unknown,
): Partial<PoolRoutingTimeoutSettings> | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const payload = raw as Record<string, unknown>;
  const next: Partial<PoolRoutingTimeoutSettings> = {};
  const responsesFirstByteTimeoutSecs = normalizeFiniteNumber(
    payload.responsesFirstByteTimeoutSecs,
  );
  const compactFirstByteTimeoutSecs = normalizeFiniteNumber(payload.compactFirstByteTimeoutSecs);
  const imageFirstByteTimeoutSecs = normalizeFiniteNumber(payload.imageFirstByteTimeoutSecs);
  const responsesStreamTimeoutSecs = normalizeFiniteNumber(payload.responsesStreamTimeoutSecs);
  const compactStreamTimeoutSecs = normalizeFiniteNumber(payload.compactStreamTimeoutSecs);
  if (responsesFirstByteTimeoutSecs != null) {
    next.responsesFirstByteTimeoutSecs = responsesFirstByteTimeoutSecs;
  }
  if (compactFirstByteTimeoutSecs != null) {
    next.compactFirstByteTimeoutSecs = compactFirstByteTimeoutSecs;
  }
  if (imageFirstByteTimeoutSecs != null) {
    next.imageFirstByteTimeoutSecs = imageFirstByteTimeoutSecs;
  }
  if (responsesStreamTimeoutSecs != null) {
    next.responsesStreamTimeoutSecs = responsesStreamTimeoutSecs;
  }
  if (compactStreamTimeoutSecs != null) {
    next.compactStreamTimeoutSecs = compactStreamTimeoutSecs;
  }
  return Object.keys(next).length > 0 ? next : undefined;
}

export function normalizeRoutingTimeoutFieldSources(
  raw: unknown,
): EffectiveRoutingTimeoutFieldSources {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const normalizeSource = (value: unknown): EffectiveRoutingRuleSource =>
    typeof value === "string" && value.trim() ? value : "root";
  return {
    responsesFirstByteTimeoutSecs: normalizeSource(payload.responsesFirstByteTimeoutSecs),
    compactFirstByteTimeoutSecs: normalizeSource(payload.compactFirstByteTimeoutSecs),
    imageFirstByteTimeoutSecs: normalizeSource(payload.imageFirstByteTimeoutSecs),
    responsesStreamTimeoutSecs: normalizeSource(payload.responsesStreamTimeoutSecs),
    compactStreamTimeoutSecs: normalizeSource(payload.compactStreamTimeoutSecs),
  };
}

export function normalizeStatusChangeReasons(raw: unknown): StatusChangeReasons {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const next = buildDefaultStatusChangeReasons();
  for (const reason of STATUS_CHANGE_REASON_CODES) {
    if (typeof payload[reason] === "boolean") {
      next[reason] = payload[reason] === true;
    }
  }
  return next;
}

export function normalizeStatusChangeReasonFieldSources(
  raw: unknown,
): StatusChangeReasonFieldSources {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const next = buildDefaultStatusChangeReasonFieldSources();
  for (const reason of STATUS_CHANGE_REASON_CODES) {
    if (typeof payload[reason] === "string" && payload[reason].trim()) {
      next[reason] = payload[reason] as EffectiveRoutingRuleSource;
    }
  }
  return next;
}

export function normalizeRequestCompressionAlgorithm(value: unknown): RequestCompressionAlgorithm {
  if (
    value === "follow" ||
    value === "identity" ||
    value === "gzip" ||
    value === "deflate" ||
    value === "zstd"
  ) {
    return value;
  }
  return "identity";
}

export function normalizeOptionalRequestCompressionAlgorithm(
  value: unknown,
): RequestCompressionAlgorithm | undefined {
  return typeof value === "string" && value.trim().length > 0
    ? normalizeRequestCompressionAlgorithm(value)
    : undefined;
}

export function normalizeGroupAccountRoutingRule(raw: unknown): GroupAccountRoutingRule {
  const payload = normalizeTagRoutingRule(raw);
  const rawPayload = (raw ?? {}) as Record<string, unknown>;
  return {
    ...payload,
    imageToolRewriteMode: normalizeImageToolRewriteMode(rawPayload.imageToolRewriteMode),
    codexImagegenRewriteMode: normalizeCodexImagegenRewriteMode(
      rawPayload.codexImagegenRewriteMode,
    ),
    requestCompressionAlgorithm: normalizeOptionalRequestCompressionAlgorithm(
      rawPayload.requestCompressionAlgorithm,
    ),
    statusChangeReasons: normalizeStatusChangeReasons(rawPayload.statusChangeReasons),
    timeouts: normalizeOptionalPoolRoutingTimeoutSettings(rawPayload.timeouts),
  };
}

export function normalizeAccountTagSummary(raw: unknown): AccountTagSummary | null {
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
      codexImagegenRewriteMode: normalizeSource(rawSources.codexImagegenRewriteMode),
      requestCompressionAlgorithm: normalizeSource(rawSources.requestCompressionAlgorithm),
      concurrencyLimit: normalizeSource(rawSources.concurrencyLimit),
      upstream429Retry: normalizeSource(rawSources.upstream429Retry),
      availableModels: normalizeSource(rawSources.availableModels),
      availableModelsMode: normalizeSource(rawSources.availableModelsMode),
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
      imageFirstByteTimeoutSecs: normalizeSource(rawTimeoutFieldSources.imageFirstByteTimeoutSecs),
      responsesStreamTimeoutSecs: normalizeSource(
        rawTimeoutFieldSources.responsesStreamTimeoutSecs,
      ),
      compactStreamTimeoutSecs: normalizeSource(rawTimeoutFieldSources.compactStreamTimeoutSecs),
    },
  };
}

export function normalizeTagSummary(raw: unknown): TagSummary | null {
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

function resolveUpstreamAccountSummaryStatus(payload: Record<string, unknown>) {
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
  return { status, displayStatus, enableStatus, syncState, healthStatus, workStatus };
}

export function normalizeUpstreamAccountSummary(raw: unknown): UpstreamAccountSummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const id = normalizeFiniteNumber(payload.id);
  const displayName = typeof payload.displayName === "string" ? payload.displayName : "";
  const kind = typeof payload.kind === "string" ? payload.kind : "";
  const provider = typeof payload.provider === "string" ? payload.provider : "";
  const { status, displayStatus, enableStatus, syncState, healthStatus, workStatus } =
    resolveUpstreamAccountSummaryStatus(payload);
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
    routingBlockUntil:
      typeof payload.routingBlockUntil === "string" ? payload.routingBlockUntil : null,
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
    responseEndpointCapability: normalizeUpstreamCapabilityState(
      payload.responseEndpointCapability,
    ),
    chatCompletionsCapability: normalizeUpstreamCapabilityState(payload.chatCompletionsCapability),
    imageEndpointCapability: normalizeUpstreamCapabilityState(payload.imageEndpointCapability),
    responseImageToolCapability: normalizeUpstreamCapabilityState(
      payload.responseImageToolCapability,
    ),
    codexImagegenCapability: normalizeUpstreamCapabilityState(payload.codexImagegenCapability),
    standaloneSearchCapability: normalizeUpstreamCapabilityState(
      payload.standaloneSearchCapability,
    ),
    tags: Array.isArray(payload.tags)
      ? payload.tags
          .map(normalizeAccountTagSummary)
          .filter((item): item is AccountTagSummary => item != null)
      : [],
    effectiveRoutingRule: normalizeEffectiveRoutingRule(payload.effectiveRoutingRule),
  };
}

export function normalizeUpstreamAccountListMetrics(raw: unknown): UpstreamAccountListMetrics {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    total: normalizeFiniteNumber(payload.total) ?? 0,
    oauth: normalizeFiniteNumber(payload.oauth) ?? 0,
    apiKey: normalizeFiniteNumber(payload.apiKey) ?? 0,
    attention: normalizeFiniteNumber(payload.attention) ?? 0,
  };
}

export function normalizeUpstreamAccountDuplicateInfo(
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
        (value): value is string => typeof value === "string" && value.trim().length > 0,
      )
    : [];
  if (peerAccountIds.length === 0 || reasons.length === 0) return null;
  return {
    peerAccountIds,
    reasons,
  };
}

export function normalizeUpstreamAccountHistoryPoint(
  raw: unknown,
): UpstreamAccountHistoryPoint | null {
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

export function normalizeUpstreamAccountActionEvent(
  raw: unknown,
): UpstreamAccountActionEvent | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const id = normalizeFiniteNumber(payload.id);
  const occurredAt = typeof payload.occurredAt === "string" ? payload.occurredAt : "";
  const action = typeof payload.action === "string" ? payload.action : "";
  const source = typeof payload.source === "string" ? payload.source : "";
  const createdAt = typeof payload.createdAt === "string" ? payload.createdAt : "";
  const attemptId =
    typeof payload.attemptId === "string" && payload.attemptId.trim()
      ? payload.attemptId.trim()
      : null;
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
    attemptId,
    stickyKey: typeof payload.stickyKey === "string" ? payload.stickyKey : null,
    model: typeof payload.model === "string" ? payload.model : null,
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
    modelRouteFailureCount: normalizeFiniteNumber(payload.modelRouteFailureCount) ?? null,
    modelRouteCooldownUntil:
      typeof payload.modelRouteCooldownUntil === "string" ? payload.modelRouteCooldownUntil : null,
    blockedBinding: normalizeBlockedBindingDiagnostic(payload.blockedBinding),
    createdAt,
  };
}
export function normalizeUpstreamAccountGroupMaxRetries(raw: unknown): number {
  const value = normalizeFiniteNumber(raw);
  if (value == null) return 0;
  return Math.min(5, Math.max(0, Math.trunc(value)));
}
