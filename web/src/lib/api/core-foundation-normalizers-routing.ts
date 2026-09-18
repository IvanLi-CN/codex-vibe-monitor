import {
  DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS,
  DEFAULT_STICKY_KEY_CONVERSATION_LIMIT,
} from "./core-foundation-base";
import {
  normalizeConversationRequestPoint,
  normalizeFiniteNumber,
  normalizePromptCacheConversationInvocationPreview,
} from "./core-foundation-normalizers-base";
import type {
  PoolRoutingSelectionAudit,
  PoolRoutingSelectionScoreSnapshot,
} from "./core-foundation-record-types";
import type {
  BulkPromptCacheConversationBindingActionResponse,
  BulkPromptCacheConversationBindingItemResponse,
  PromptCacheConversationBindingResponse,
  PromptCacheConversationOperationBindingSnapshot,
  PromptCacheConversationOperationEvent,
  PromptCacheConversationOperationEventListResponse,
  PromptCacheConversationOperationInfoType,
  PromptCacheConversationOperationRoutingContext,
  PromptCacheConversationOperationRoutingScope,
  PromptCacheConversationOperationStickySnapshot,
  PromptCacheConversationOperationStickyTransition,
  PromptCacheConversationRewriteMode,
  PromptCacheConversationStickyRoute,
  StickyKeyConversation,
  StickyKeyConversationImplicitFilterKind,
  StickyKeyConversationInvocationPreview,
  StickyKeyConversationRequestPoint,
  UpstreamStickyConversationsResponse,
} from "./core-foundation-settings-types";
import type {
  CodexImagegenRewriteMode,
  EffectiveRoutingRuleSource,
  EffectiveRoutingTimeoutFieldSources,
  PoolRoutingMaintenanceSettings,
  PoolRoutingSettings,
  PoolRoutingTimeoutSettings,
  RequestCompressionAlgorithm,
  RequestCompressionLevelPreset,
} from "./core-upstream";

export function normalizeStickyKeyConversation(raw: unknown): StickyKeyConversation | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const stickyKey = typeof payload.stickyKey === "string" ? payload.stickyKey.trim() : "";
  if (!stickyKey) return null;
  const requestsRaw = Array.isArray(payload.last24hRequests) ? payload.last24hRequests : [];
  const recentInvocationsRaw = Array.isArray(payload.recentInvocations)
    ? payload.recentInvocations
    : [];
  return {
    stickyKey,
    requestCount: normalizeFiniteNumber(payload.requestCount) ?? 0,
    totalTokens: normalizeFiniteNumber(payload.totalTokens) ?? 0,
    totalCost: normalizeFiniteNumber(payload.totalCost) ?? 0,
    createdAt: typeof payload.createdAt === "string" ? payload.createdAt : "",
    lastActivityAt: typeof payload.lastActivityAt === "string" ? payload.lastActivityAt : "",
    recentInvocations: recentInvocationsRaw
      .map(normalizePromptCacheConversationInvocationPreview)
      .filter((item): item is StickyKeyConversationInvocationPreview => item != null),
    last24hRequests: requestsRaw
      .map(normalizeConversationRequestPoint)
      .filter((item): item is StickyKeyConversationRequestPoint => item != null),
  };
}

export function normalizeUpstreamStickyConversationsResponse(
  raw: unknown,
): UpstreamStickyConversationsResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const conversationsRaw = Array.isArray(payload.conversations) ? payload.conversations : [];
  const implicitFilterPayload =
    payload.implicitFilter && typeof payload.implicitFilter === "object"
      ? (payload.implicitFilter as Record<string, unknown>)
      : null;
  const implicitFilterKindRaw =
    typeof implicitFilterPayload?.kind === "string" ? implicitFilterPayload.kind : null;
  const implicitFilterKind: StickyKeyConversationImplicitFilterKind | null =
    implicitFilterKindRaw === "inactiveOutside24h" || implicitFilterKindRaw === "cappedTo50"
      ? implicitFilterKindRaw
      : null;
  const selectionModeRaw = payload.selectionMode === "activityWindow" ? "activityWindow" : "count";
  return {
    rangeStart: typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    selectionMode: selectionModeRaw,
    selectedLimit:
      selectionModeRaw === "count"
        ? (normalizeFiniteNumber(payload.selectedLimit) ?? DEFAULT_STICKY_KEY_CONVERSATION_LIMIT)
        : (normalizeFiniteNumber(payload.selectedLimit) ?? null),
    selectedActivityHours: normalizeFiniteNumber(payload.selectedActivityHours) ?? null,
    implicitFilter: {
      kind: implicitFilterKind,
      filteredCount: normalizeFiniteNumber(implicitFilterPayload?.filteredCount) ?? 0,
    },
    conversations: conversationsRaw
      .map(normalizeStickyKeyConversation)
      .filter((item): item is StickyKeyConversation => item != null),
  };
}

export function normalizePoolRoutingSettings(raw: unknown): PoolRoutingSettings | null {
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
  const cacheHitProtectionRaw =
    payload.cacheHitProtection && typeof payload.cacheHitProtection === "object"
      ? (payload.cacheHitProtection as Record<string, unknown>)
      : null;
  const normalized: PoolRoutingSettings = {
    writesEnabled: typeof payload.writesEnabled === "boolean" ? payload.writesEnabled : true,
    apiKeyConfigured: payload.apiKeyConfigured,
    maskedApiKey: typeof payload.maskedApiKey === "string" ? payload.maskedApiKey : null,
    maintenance,
    requestCompressionAlgorithm:
      payload.requestCompressionAlgorithm === "follow" ||
      payload.requestCompressionAlgorithm === "identity" ||
      payload.requestCompressionAlgorithm === "gzip" ||
      payload.requestCompressionAlgorithm === "deflate" ||
      payload.requestCompressionAlgorithm === "zstd"
        ? (payload.requestCompressionAlgorithm as RequestCompressionAlgorithm)
        : "identity",
    requestCompressionLevelPreset:
      payload.requestCompressionLevelPreset === "fast" ||
      payload.requestCompressionLevelPreset === "balanced" ||
      payload.requestCompressionLevelPreset === "best"
        ? (payload.requestCompressionLevelPreset as RequestCompressionLevelPreset)
        : "balanced",
    codexImagegenRewriteMode:
      payload.codexImagegenRewriteMode === "fill_missing" ||
      payload.codexImagegenRewriteMode === "force_add" ||
      payload.codexImagegenRewriteMode === "force_remove" ||
      payload.codexImagegenRewriteMode === "keep_original"
        ? (payload.codexImagegenRewriteMode as CodexImagegenRewriteMode)
        : "keep_original",
    timeouts: normalizePoolRoutingTimeoutSettings(payload.timeouts),
    cacheHitProtection: {
      enabled: cacheHitProtectionRaw?.enabled === true,
      lowHitRateThresholdPercent:
        typeof cacheHitProtectionRaw?.lowHitRateThresholdPercent === "number" &&
        Number.isFinite(cacheHitProtectionRaw.lowHitRateThresholdPercent)
          ? Math.min(100, Math.max(1, Math.trunc(cacheHitProtectionRaw.lowHitRateThresholdPercent)))
          : 10,
      overflowMode: cacheHitProtectionRaw?.overflowMode === "reroute" ? "reroute" : "queue",
      minimumInputTokens: normalizeFiniteNumber(cacheHitProtectionRaw?.minimumInputTokens) ?? 3840,
    },
  };
  if (typeof payload.priorityHandoffAdmissionEnabled === "boolean") {
    normalized.priorityHandoffAdmissionEnabled = payload.priorityHandoffAdmissionEnabled;
  }
  if (Array.isArray(payload.availableModels)) {
    normalized.availableModels = payload.availableModels.filter(
      (value): value is string => typeof value === "string",
    );
  }
  if (payload.availableModelsMode === "allowlist" || payload.availableModelsMode === "denylist") {
    normalized.availableModelsMode = payload.availableModelsMode;
  }
  return normalized;
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

export function normalizeRoutingTimeoutFieldSources(
  raw: unknown,
): EffectiveRoutingTimeoutFieldSources {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const normalizeSource = (value: unknown) =>
    typeof value === "string" && value.trim() ? value : "root";
  return {
    responsesFirstByteTimeoutSecs: normalizeSource(payload.responsesFirstByteTimeoutSecs),
    compactFirstByteTimeoutSecs: normalizeSource(payload.compactFirstByteTimeoutSecs),
    imageFirstByteTimeoutSecs: normalizeSource(payload.imageFirstByteTimeoutSecs),
    responsesStreamTimeoutSecs: normalizeSource(payload.responsesStreamTimeoutSecs),
    compactStreamTimeoutSecs: normalizeSource(payload.compactStreamTimeoutSecs),
  };
}

export function normalizePromptCacheRewriteMode(
  value: unknown,
): PromptCacheConversationRewriteMode | null {
  return value === "force_remove" ||
    value === "keep_original" ||
    value === "fill_missing" ||
    value === "force_add"
    ? value
    : null;
}

export function normalizePromptCachePolicySource(value: unknown): EffectiveRoutingRuleSource {
  return typeof value === "string" && value.trim() ? value.trim() : "account";
}

export function normalizePromptCacheStickyRoutes(
  raw: unknown,
): PromptCacheConversationStickyRoute[] {
  if (!Array.isArray(raw)) return [];
  return raw.flatMap((value): PromptCacheConversationStickyRoute[] => {
    const route = (value ?? {}) as Record<string, unknown>;
    const upstreamAccountId = normalizeFiniteNumber(route.upstreamAccountId);
    const createdAt = typeof route.createdAt === "string" ? route.createdAt.trim() : "";
    const updatedAt = typeof route.updatedAt === "string" ? route.updatedAt.trim() : "";
    const lastSeenAt = typeof route.lastSeenAt === "string" ? route.lastSeenAt.trim() : "";
    if (upstreamAccountId == null || !createdAt || !updatedAt || !lastSeenAt) return [];
    return [
      {
        modelKey:
          typeof route.modelKey === "string" && route.modelKey.trim()
            ? route.modelKey.trim()
            : null,
        upstreamAccountId,
        upstreamAccountName:
          typeof route.upstreamAccountName === "string" && route.upstreamAccountName.trim()
            ? route.upstreamAccountName.trim()
            : null,
        createdAt,
        updatedAt,
        lastSeenAt,
      },
    ];
  });
}

export function normalizePromptCacheConversationBindingResponse(
  raw: Record<string, unknown>,
  promptCacheKey: string,
): PromptCacheConversationBindingResponse {
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
      : (forwardProxyKeys[0] ?? null);
  const stickyRoutes = normalizePromptCacheStickyRoutes(raw.stickyRoutes);
  return {
    promptCacheKey: typeof raw.promptCacheKey === "string" ? raw.promptCacheKey : promptCacheKey,
    bindingKind:
      raw.bindingKind === "group" || raw.bindingKind === "upstreamAccount"
        ? raw.bindingKind
        : "none",
    groupName:
      typeof raw.groupName === "string" && raw.groupName.trim() ? raw.groupName.trim() : null,
    upstreamAccountId: normalizeFiniteNumber(raw.upstreamAccountId) ?? null,
    upstreamAccountName:
      typeof raw.upstreamAccountName === "string" && raw.upstreamAccountName.trim()
        ? raw.upstreamAccountName.trim()
        : null,
    hasEncryptedSessionOwner:
      typeof raw.hasEncryptedSessionOwner === "boolean" ? raw.hasEncryptedSessionOwner : false,
    encryptedOwnerAccountId: normalizeFiniteNumber(raw.encryptedOwnerAccountId) ?? null,
    encryptedOwnerAccountName:
      typeof raw.encryptedOwnerAccountName === "string" && raw.encryptedOwnerAccountName.trim()
        ? raw.encryptedOwnerAccountName.trim()
        : null,
    encryptedOwnerGroupName:
      typeof raw.encryptedOwnerGroupName === "string" && raw.encryptedOwnerGroupName.trim()
        ? raw.encryptedOwnerGroupName.trim()
        : null,
    stickyRoutes,
    timeouts: normalizePoolRoutingTimeoutSettings(raw.timeouts),
    timeoutFieldSources: normalizeRoutingTimeoutFieldSources(raw.timeoutFieldSources),
    allowSwitchUpstream:
      typeof raw.allowSwitchUpstream === "boolean" ? raw.allowSwitchUpstream : null,
    fastModeRewriteMode: normalizePromptCacheRewriteMode(raw.fastModeRewriteMode),
    imageToolRewriteMode: normalizePromptCacheRewriteMode(raw.imageToolRewriteMode),
    codexImagegenRewriteMode: normalizePromptCacheRewriteMode(raw.codexImagegenRewriteMode),
    availableModels: Array.isArray(raw.availableModels)
      ? raw.availableModels
          .filter((value): value is string => typeof value === "string")
          .map((value) => value.trim())
          .filter(Boolean)
      : null,
    availableModelsMode:
      raw.availableModelsMode === "allowlist" || raw.availableModelsMode === "denylist"
        ? raw.availableModelsMode
        : Array.isArray(raw.availableModels)
          ? "allowlist"
          : null,
    forwardProxyKey,
    forwardProxyKeys:
      forwardProxyKeys.length > 0 ? forwardProxyKeys : forwardProxyKey ? [forwardProxyKey] : [],
    policyFieldSources: {
      allowSwitchUpstream: normalizePromptCachePolicySource(rawPolicySources.allowSwitchUpstream),
      fastModeRewriteMode: normalizePromptCachePolicySource(rawPolicySources.fastModeRewriteMode),
      imageToolRewriteMode: normalizePromptCachePolicySource(rawPolicySources.imageToolRewriteMode),
      codexImagegenRewriteMode: normalizePromptCachePolicySource(
        rawPolicySources.codexImagegenRewriteMode,
      ),
      availableModels: normalizePromptCachePolicySource(rawPolicySources.availableModels),
      availableModelsMode: normalizePromptCachePolicySource(rawPolicySources.availableModelsMode),
      forwardProxyKey: normalizePromptCachePolicySource(rawPolicySources.forwardProxyKey),
    },
    updatedAt: typeof raw.updatedAt === "string" ? raw.updatedAt : null,
  };
}

export function normalizeBulkPromptCacheConversationBindingItemResponse(
  raw: unknown,
): BulkPromptCacheConversationBindingItemResponse | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const promptCacheKey =
    typeof payload.promptCacheKey === "string" ? payload.promptCacheKey.trim() : "";
  if (!promptCacheKey) return null;
  const ok = payload.ok === true;
  const binding =
    payload.binding && typeof payload.binding === "object"
      ? normalizePromptCacheConversationBindingResponse(
          payload.binding as Record<string, unknown>,
          promptCacheKey,
        )
      : null;
  return {
    promptCacheKey,
    ok,
    error: typeof payload.error === "string" && payload.error.trim() ? payload.error.trim() : null,
    binding,
  };
}

export function normalizeBulkPromptCacheConversationBindingActionResponse(
  raw: unknown,
): BulkPromptCacheConversationBindingActionResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const action =
    payload.action === "bind" ||
    payload.action === "clearAndResetAffinity" ||
    payload.action === "setFastModeRewriteMode"
      ? payload.action
      : null;
  if (action == null) {
    throw new Error("Request failed: invalid bulk prompt cache conversation binding payload");
  }
  const items = Array.isArray(payload.items) ? payload.items : [];
  return {
    action,
    totalRequested: normalizeFiniteNumber(payload.totalRequested) ?? 0,
    totalSucceeded: normalizeFiniteNumber(payload.totalSucceeded) ?? 0,
    totalFailed: normalizeFiniteNumber(payload.totalFailed) ?? 0,
    items: items
      .map(normalizeBulkPromptCacheConversationBindingItemResponse)
      .filter((item): item is BulkPromptCacheConversationBindingItemResponse => item != null),
  };
}

export function normalizePromptCacheConversationOperationBindingSnapshot(
  raw: unknown,
): PromptCacheConversationOperationBindingSnapshot | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const bindingKind =
    payload.bindingKind === "group" || payload.bindingKind === "upstreamAccount"
      ? payload.bindingKind
      : payload.bindingKind === "none"
        ? "none"
        : null;
  if (bindingKind == null) return null;
  return {
    bindingKind,
    groupName:
      typeof payload.groupName === "string" && payload.groupName.trim()
        ? payload.groupName.trim()
        : null,
    upstreamAccountId: normalizeFiniteNumber(payload.upstreamAccountId) ?? null,
    upstreamAccountName:
      typeof payload.upstreamAccountName === "string" && payload.upstreamAccountName.trim()
        ? payload.upstreamAccountName.trim()
        : null,
  };
}

export function normalizePromptCacheConversationOperationStickySnapshot(
  raw: unknown,
): PromptCacheConversationOperationStickySnapshot | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const upstreamAccountId = normalizeFiniteNumber(payload.upstreamAccountId);
  if (upstreamAccountId == null) return null;
  return {
    upstreamAccountId,
    upstreamAccountName:
      typeof payload.upstreamAccountName === "string" && payload.upstreamAccountName.trim()
        ? payload.upstreamAccountName.trim()
        : null,
  };
}

export function normalizePoolRoutingSelectionAudit(raw: unknown): PoolRoutingSelectionAudit | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const selectedAccountId = normalizeFiniteNumber(payload.selectedAccountId);
  const selectedAccountName =
    typeof payload.selectedAccountName === "string" ? payload.selectedAccountName.trim() : "";
  const eligibleCandidateCount = normalizeFiniteNumber(payload.eligibleCandidateCount);
  const winnerReasonCode =
    typeof payload.winnerReasonCode === "string" ? payload.winnerReasonCode.trim() : "";
  if (
    selectedAccountId == null ||
    !selectedAccountName ||
    eligibleCandidateCount == null ||
    !winnerReasonCode
  ) {
    return null;
  }
  const excludedCandidates = Array.isArray(payload.excludedCandidates)
    ? payload.excludedCandidates.flatMap((candidate) => {
        const item = candidate as Record<string, unknown>;
        const accountId = normalizeFiniteNumber(item.accountId);
        const accountName = typeof item.accountName === "string" ? item.accountName.trim() : "";
        const reasonCode = typeof item.reasonCode === "string" ? item.reasonCode.trim() : "";
        return accountId == null || !accountName || !reasonCode
          ? []
          : [{ accountId, accountName, reasonCode }];
      })
    : [];
  const normalizeScore = (value: unknown): PoolRoutingSelectionScoreSnapshot | null => {
    const score = (value ?? {}) as Record<string, unknown>;
    const routeBindingFailurePenalty = normalizeFiniteNumber(score.routeBindingFailurePenalty);
    const modelRoutePenalty = normalizeFiniteNumber(score.modelRoutePenalty);
    const routingPriorityRank = normalizeFiniteNumber(score.routingPriorityRank);
    const effectiveLoad = normalizeFiniteNumber(score.effectiveLoad);
    const eligibility = typeof score.eligibility === "string" ? score.eligibility.trim() : "";
    const modelRoutePenaltyCode =
      typeof score.modelRoutePenaltyCode === "string" ? score.modelRoutePenaltyCode.trim() : "";
    const capacityLane = typeof score.capacityLane === "string" ? score.capacityLane.trim() : "";
    const dispatchState = typeof score.dispatchState === "string" ? score.dispatchState.trim() : "";
    const scarcityScore = typeof score.scarcityScore === "string" ? score.scarcityScore.trim() : "";
    if (
      routeBindingFailurePenalty == null ||
      modelRoutePenalty == null ||
      routingPriorityRank == null ||
      effectiveLoad == null ||
      !eligibility ||
      !modelRoutePenaltyCode ||
      !capacityLane ||
      !dispatchState ||
      !scarcityScore
    ) {
      return null;
    }
    return {
      eligibility,
      routeBindingFailurePenalty,
      modelRoutePenalty,
      modelRoutePenaltyCode,
      routingPriorityRank,
      capacityLane,
      dispatchState,
      secondaryResetProximitySecs: normalizeFiniteNumber(score.secondaryResetProximitySecs),
      primaryResetProximitySecs: normalizeFiniteNumber(score.primaryResetProximitySecs),
      scarcityScore,
      effectiveLoad,
      lastSelectedAt:
        typeof score.lastSelectedAt === "string" && score.lastSelectedAt.trim()
          ? score.lastSelectedAt.trim()
          : null,
    };
  };
  const normalized: PoolRoutingSelectionAudit = {
    selectedAccountId,
    selectedAccountName,
    eligibleCandidateCount,
    winnerReasonCode,
    comparedAccountId: normalizeFiniteNumber(payload.comparedAccountId),
    comparedAccountName:
      typeof payload.comparedAccountName === "string" && payload.comparedAccountName.trim()
        ? payload.comparedAccountName.trim()
        : null,
    selectedScore: normalizeScore(payload.selectedScore),
    comparedScore: normalizeScore(payload.comparedScore),
    excludedCandidates,
  };
  if (typeof payload.handoffAdmission === "object" && payload.handoffAdmission !== null) {
    const admission = payload.handoffAdmission as Record<string, unknown>;
    const decision = typeof admission.decision === "string" ? admission.decision.trim() : "";
    const phase = typeof admission.phase === "string" ? admission.phase.trim() : "";
    const verificationSuccessCount = normalizeFiniteNumber(admission.verificationSuccessCount);
    if (decision && phase && verificationSuccessCount != null) {
      normalized.handoffAdmission = { decision, phase, verificationSuccessCount };
      const generation = normalizeFiniteNumber(admission.generation);
      if (generation != null) normalized.handoffAdmission.generation = generation;
      if (typeof admission.trigger === "string" && admission.trigger.trim()) {
        normalized.handoffAdmission.trigger = admission.trigger.trim();
      }
    }
  }
  return normalized;
}

export function normalizePromptCacheConversationOperationRoutingContext(
  raw: unknown,
): PromptCacheConversationOperationRoutingContext | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const reasonCode = typeof payload.reasonCode === "string" ? payload.reasonCode.trim() : "";
  if (!reasonCode) return null;
  const attemptId = (value: unknown) =>
    typeof value === "string" && value.trim() ? value.trim() : null;
  return {
    reasonCode,
    routingSource: attemptId(payload.routingSource),
    routingSelectionAudit: normalizePoolRoutingSelectionAudit(payload.routingSelectionAudit),
    httpStatus: normalizeFiniteNumber(payload.httpStatus) ?? null,
    triggerAttemptId: attemptId(payload.triggerAttemptId),
    causingAttemptId: attemptId(payload.causingAttemptId),
    causingHttpStatus: normalizeFiniteNumber(payload.causingHttpStatus) ?? null,
  };
}

export function normalizePromptCacheConversationOperationRoutingScope(
  raw: unknown,
): PromptCacheConversationOperationRoutingScope | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const kind = payload.kind === "all" || payload.kind === "model" ? payload.kind : null;
  if (kind == null) return null;
  return {
    kind,
    modelKey:
      typeof payload.modelKey === "string" && payload.modelKey.trim()
        ? payload.modelKey.trim()
        : null,
    requestModel:
      typeof payload.requestModel === "string" && payload.requestModel.trim()
        ? payload.requestModel.trim()
        : null,
  };
}

export function normalizePromptCacheConversationOperationStickyTransition(
  raw: unknown,
): PromptCacheConversationOperationStickyTransition | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    modelKey:
      typeof payload.modelKey === "string" && payload.modelKey.trim()
        ? payload.modelKey.trim()
        : null,
    before: normalizePromptCacheConversationOperationStickySnapshot(payload.before),
    after: normalizePromptCacheConversationOperationStickySnapshot(payload.after),
  };
}

export function normalizePromptCacheConversationOperationEvent(
  raw: unknown,
): PromptCacheConversationOperationEvent | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const id = normalizeFiniteNumber(payload.id);
  const promptCacheKey =
    typeof payload.promptCacheKey === "string" ? payload.promptCacheKey.trim() : "";
  const occurredAt = typeof payload.occurredAt === "string" ? payload.occurredAt : "";
  const action =
    payload.action === "manualBindingUpdated" ||
    payload.action === "bindingCleared" ||
    payload.action === "affinityReset" ||
    payload.action === "stickyTargetChanged" ||
    payload.action === "stickyTargetCleared" ||
    payload.action === "stickyMutationSuppressed" ||
    payload.action === "groupBindingPromoted" ||
    payload.action === "conversationPolicyUpdated"
      ? payload.action
      : null;
  const origin =
    payload.origin === "detailDrawer" ||
    payload.origin === "dashboardBulk" ||
    payload.origin === "systemAuto"
      ? payload.origin
      : null;
  if (id == null || !promptCacheKey || !occurredAt || action == null || origin == null) {
    return null;
  }
  return {
    id,
    promptCacheKey,
    action,
    origin,
    infoTypes: Array.isArray(payload.infoTypes)
      ? payload.infoTypes.filter(
          (value): value is PromptCacheConversationOperationInfoType =>
            value === "routing" || value === "forwardProxy" || value === "requestRewrite",
        )
      : [],
    occurredAt,
    headline: typeof payload.headline === "string" ? payload.headline : action,
    changedFields: Array.isArray(payload.changedFields)
      ? payload.changedFields
          .filter((value): value is string => typeof value === "string")
          .map((value) => value.trim())
          .filter(Boolean)
      : [],
    bindingBefore: normalizePromptCacheConversationOperationBindingSnapshot(payload.bindingBefore),
    bindingAfter: normalizePromptCacheConversationOperationBindingSnapshot(payload.bindingAfter),
    stickyBefore: normalizePromptCacheConversationOperationStickySnapshot(payload.stickyBefore),
    stickyAfter: normalizePromptCacheConversationOperationStickySnapshot(payload.stickyAfter),
    invokeId:
      typeof payload.invokeId === "string" && payload.invokeId.trim() ? payload.invokeId : null,
    routingContext: normalizePromptCacheConversationOperationRoutingContext(payload.routingContext),
    routingScope: normalizePromptCacheConversationOperationRoutingScope(payload.routingScope),
    stickyTransitions: Array.isArray(payload.stickyTransitions)
      ? payload.stickyTransitions
          .map(normalizePromptCacheConversationOperationStickyTransition)
          .filter(
            (transition): transition is PromptCacheConversationOperationStickyTransition =>
              transition != null,
          )
      : [],
  };
}

export function normalizePromptCacheConversationOperationEventListResponse(
  raw: unknown,
): PromptCacheConversationOperationEventListResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const itemsRaw = Array.isArray(payload.items) ? payload.items : [];
  return {
    items: itemsRaw
      .map(normalizePromptCacheConversationOperationEvent)
      .filter((item): item is PromptCacheConversationOperationEvent => item != null),
    total: normalizeFiniteNumber(payload.total) ?? 0,
    page: normalizeFiniteNumber(payload.page) ?? 1,
    pageSize: normalizeFiniteNumber(payload.pageSize) ?? 20,
    routingModelFacets: Array.isArray(payload.routingModelFacets)
      ? payload.routingModelFacets
          .filter((value): value is string => typeof value === "string")
          .map((value) => value.trim())
          .filter(Boolean)
      : [],
  };
}
