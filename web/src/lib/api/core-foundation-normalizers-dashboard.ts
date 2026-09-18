import { DEFAULT_PROMPT_CACHE_CONVERSATION_LIMIT } from "./core-foundation-base";
import type {
  DashboardActivityRecentResponse,
  DashboardActivityResponse,
  DashboardNetworkRealtimeRate,
  DashboardNetworkTimeseriesPoint,
  DashboardNetworkTimeseriesResponse,
  DashboardRecentNetworkWindowPoint,
  DashboardRecentNetworkWindowResponse,
  StatsMaintenanceResponse,
  UpstreamAccountActivityAccount,
  UpstreamAccountActivityResponse,
} from "./core-foundation-dashboard-types";
import {
  normalizeFiniteNumber,
  normalizeForwardProxySettings,
  normalizeInvocationPhaseCounts,
  normalizePricingSettings,
  normalizePromptCacheConversation,
  normalizePromptCacheConversationInvocationPreview,
  normalizeProxySettings,
  normalizeRoutingStateVersion,
  normalizeUsageBreakdown,
} from "./core-foundation-normalizers-base";
import type {
  ModelPerformance,
  ModelPerformanceMetrics,
  StatsResponse,
} from "./core-foundation-record-types";
import type {
  ForwardProxyValidationResult,
  PromptCacheConversation,
  PromptCacheConversationImplicitFilterKind,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationsResponse,
} from "./core-foundation-settings-types";
import type {
  ExternalApiKeyListResponse,
  ExternalApiKeyMutationResponse,
  ExternalApiKeySecretResponse,
  ExternalApiKeySummary,
  RuntimePressureHealth,
  SettingsPayload,
  SystemProjectionConsumerHealth,
  SystemProjectionHealth,
  SystemRawMetricsHealth,
  SystemStatusMetric,
  SystemStatusResponse,
  SystemTaskRun,
  SystemTaskRunsResponse,
} from "./core-foundation-system-types";
import type { CompactSupportState } from "./core-upstream";
import { normalizeEffectiveRoutingRule } from "./core-upstream";
export function normalizeCompactSupportState(raw: unknown): CompactSupportState {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const status =
    payload.status === "supported" || payload.status === "unsupported" ? payload.status : "unknown";
  return {
    status,
    observedAt: typeof payload.observedAt === "string" ? payload.observedAt : null,
    reason: typeof payload.reason === "string" ? payload.reason : null,
  };
}

export function normalizePromptCacheConversationsResponse(
  raw: unknown,
): PromptCacheConversationsResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const conversationsRaw = Array.isArray(payload.conversations) ? payload.conversations : [];
  const implicitFilterPayload =
    payload.implicitFilter && typeof payload.implicitFilter === "object"
      ? (payload.implicitFilter as Record<string, unknown>)
      : null;
  const implicitFilterKindRaw =
    typeof implicitFilterPayload?.kind === "string" ? implicitFilterPayload.kind : null;
  const implicitFilterKind: PromptCacheConversationImplicitFilterKind | null =
    implicitFilterKindRaw === "inactiveOutside24h" || implicitFilterKindRaw === "cappedTo50"
      ? implicitFilterKindRaw
      : null;
  const selectionModeRaw = payload.selectionMode === "activityWindow" ? "activityWindow" : "count";
  return {
    rangeStart: typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    snapshotAt: typeof payload.snapshotAt === "string" ? payload.snapshotAt : null,
    selectionMode: selectionModeRaw,
    selectedLimit:
      selectionModeRaw === "count"
        ? (normalizeFiniteNumber(payload.selectedLimit) ?? DEFAULT_PROMPT_CACHE_CONVERSATION_LIMIT)
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
      filteredCount: normalizeFiniteNumber(implicitFilterPayload?.filteredCount) ?? 0,
    },
    totalMatched: normalizeFiniteNumber(payload.totalMatched) ?? null,
    hasMore: payload.hasMore === true,
    nextCursor: typeof payload.nextCursor === "string" ? payload.nextCursor : null,
    conversations: conversationsRaw
      .map(normalizePromptCacheConversation)
      .filter((item): item is PromptCacheConversation => item != null),
  };
}

export function normalizeForwardProxyValidationResult(raw: unknown): ForwardProxyValidationResult {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    ok: payload.ok === true,
    message:
      typeof payload.message === "string" && payload.message.trim()
        ? payload.message
        : "validation failed",
    normalizedValue:
      typeof payload.normalizedValue === "string" ? payload.normalizedValue : undefined,
    discoveredNodes: normalizeFiniteNumber(payload.discoveredNodes),
    latencyMs: normalizeFiniteNumber(payload.latencyMs),
  };
}

export function normalizeUpstreamAccountActivityAccount(
  raw: unknown,
): UpstreamAccountActivityAccount | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const upstreamAccountId = normalizeFiniteNumber(payload.upstreamAccountId);
  const isUnassigned = payload.isUnassigned === true;
  const displayName = typeof payload.displayName === "string" ? payload.displayName.trim() : "";
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
    groupName: typeof payload.groupName === "string" ? payload.groupName.trim() : null,
    planType: typeof payload.planType === "string" ? payload.planType.trim() : null,
    enabled: typeof payload.enabled === "boolean" ? payload.enabled : null,
    displayStatus: typeof payload.displayStatus === "string" ? payload.displayStatus.trim() : null,
    enableStatus: typeof payload.enableStatus === "string" ? payload.enableStatus.trim() : null,
    workStatus: typeof payload.workStatus === "string" ? payload.workStatus.trim() : null,
    healthStatus: typeof payload.healthStatus === "string" ? payload.healthStatus.trim() : null,
    syncState: typeof payload.syncState === "string" ? payload.syncState.trim() : null,
    lastError: typeof payload.lastError === "string" ? payload.lastError.trim() : null,
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
    modelPerformance: normalizeModelPerformance(payload.modelPerformance),
    cacheHitRate: normalizeFiniteNumber(payload.cacheHitRate),
    tokensPerMinute: normalizeFiniteNumber(payload.tokensPerMinute),
    spendRate: normalizeFiniteNumber(payload.spendRate),
    firstByteAvgMs: normalizeFiniteNumber(payload.firstByteAvgMs),
    firstResponseByteTotalAvgMs: normalizeFiniteNumber(payload.firstResponseByteTotalAvgMs),
    firstTokenAvgMs: normalizeFiniteNumber(payload.firstTokenAvgMs),
    avgTotalMs: normalizeFiniteNumber(payload.avgTotalMs),
    currentFirstTokenAvgMs: normalizeFiniteNumber(payload.currentFirstTokenAvgMs),
    currentFirstResponseByteTotalAvgMs: normalizeFiniteNumber(
      payload.currentFirstResponseByteTotalAvgMs,
    ),
    currentAvgTotalMs: normalizeFiniteNumber(payload.currentAvgTotalMs),
    currentAvgResponseMs: normalizeFiniteNumber(payload.currentAvgResponseMs),
    inProgressInvocationCount: normalizeFiniteNumber(payload.inProgressInvocationCount),
    inProgressPhaseCounts: normalizeInvocationPhaseCounts(payload.inProgressPhaseCounts),
    retryInvocationCount: normalizeFiniteNumber(payload.retryInvocationCount),
    uploadBytesPerSecond: normalizeFiniteNumber(payload.uploadBytesPerSecond) ?? 0,
    downloadBytesPerSecond: normalizeFiniteNumber(payload.downloadBytesPerSecond) ?? 0,
    effectiveRoutingRule: normalizeEffectiveRoutingRule(payload.effectiveRoutingRule),
    recentInvocations,
  };
}

export function normalizeModelPerformanceMetrics(raw: unknown): ModelPerformanceMetrics {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    tokensPerMinute: normalizeFiniteNumber(payload.tokensPerMinute) ?? 0,
    streamingResponseRate: normalizeFiniteNumber(payload.streamingResponseRate),
    avgResponseMs: normalizeFiniteNumber(payload.avgResponseMs),
    avgFirstResponseByteTotalMs: normalizeFiniteNumber(payload.avgFirstResponseByteTotalMs),
    avgFirstTokenMs: normalizeFiniteNumber(payload.avgFirstTokenMs),
    wallClockUsageDurationMs: normalizeFiniteNumber(payload.wallClockUsageDurationMs),
    cumulativeUsageDurationMs: normalizeFiniteNumber(payload.cumulativeUsageDurationMs),
    parallelism: normalizeFiniteNumber(payload.parallelism),
  };
}

export function normalizeModelPerformance(raw: unknown): ModelPerformance {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const models = Array.isArray(payload.models)
    ? payload.models.flatMap((item) => {
        const modelPayload = item as Record<string, unknown>;
        const model = typeof modelPayload.model === "string" ? modelPayload.model.trim() : "";
        if (!model) return [];
        return [
          {
            model,
            reasoningEffort:
              typeof modelPayload.reasoningEffort === "string"
                ? modelPayload.reasoningEffort.trim() || null
                : null,
            ...normalizeModelPerformanceMetrics(modelPayload),
          },
        ];
      })
    : [];
  return {
    available: payload.available === true,
    total: normalizeModelPerformanceMetrics(payload.total),
    models,
  };
}

export function normalizeStatsResponse(raw: unknown): StatsResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    totalCount: normalizeFiniteNumber(payload.totalCount) ?? 0,
    successCount: normalizeFiniteNumber(payload.successCount) ?? 0,
    failureCount: normalizeFiniteNumber(payload.failureCount) ?? 0,
    totalCost: normalizeFiniteNumber(payload.totalCost) ?? 0,
    totalTokens: normalizeFiniteNumber(payload.totalTokens) ?? 0,
    usageBreakdown: normalizeUsageBreakdown(payload.usageBreakdown),
    inProgressConversationCount: normalizeFiniteNumber(payload.inProgressConversationCount),
    inProgressRetryConversationCount: normalizeFiniteNumber(
      payload.inProgressRetryConversationCount,
    ),
    inProgressAvgWaitMs: normalizeFiniteNumber(payload.inProgressAvgWaitMs),
    inProgressPhaseCounts: normalizeInvocationPhaseCounts(payload.inProgressPhaseCounts),
    nonSuccessCost: normalizeFiniteNumber(payload.nonSuccessCost),
    nonSuccessTokens: normalizeFiniteNumber(payload.nonSuccessTokens),
    maintenance: payload.maintenance as StatsMaintenanceResponse | undefined,
  };
}

export function normalizeUpstreamAccountActivityResponse(
  raw: unknown,
): UpstreamAccountActivityResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    range: typeof payload.range === "string" ? payload.range : "",
    rangeStart: typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    routingStateVersion: normalizeRoutingStateVersion(payload.routingStateVersion),
    networkLiveBucket: normalizeDashboardNetworkTimeseriesPoint(payload.networkLiveBucket),
    networkRealtimeRate: normalizeDashboardNetworkRealtimeRate(payload.networkRealtimeRate),
    accounts: Array.isArray(payload.accounts)
      ? payload.accounts
          .map(normalizeUpstreamAccountActivityAccount)
          .filter((item): item is UpstreamAccountActivityAccount => item != null)
      : [],
  };
}

export function normalizeDashboardActivityResponse(raw: unknown): DashboardActivityResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const summaryPayload = (payload.summary ?? {}) as Record<string, unknown>;
  const rateWindowPayload = (payload.rateWindow ?? {}) as Record<string, unknown>;
  return {
    range: typeof payload.range === "string" ? payload.range : "",
    rangeStart: typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    snapshotId: normalizeFiniteNumber(payload.snapshotId) ?? 0,
    routingStateVersion: normalizeRoutingStateVersion(payload.routingStateVersion),
    liveRevision: normalizeFiniteNumber(payload.liveRevision) ?? 0,
    rateWindow: {
      start: typeof rateWindowPayload.start === "string" ? rateWindowPayload.start : "",
      end: typeof rateWindowPayload.end === "string" ? rateWindowPayload.end : "",
      windowMinutes: normalizeFiniteNumber(rateWindowPayload.windowMinutes) ?? 0,
      mode: typeof rateWindowPayload.mode === "string" ? rateWindowPayload.mode : "",
    },
    summary: {
      stats: normalizeStatsResponse(summaryPayload.stats),
      tokensPerMinute: normalizeFiniteNumber(summaryPayload.tokensPerMinute),
      spendRate: normalizeFiniteNumber(summaryPayload.spendRate),
      currentFirstResponseByteTotalAvgMs: normalizeFiniteNumber(
        summaryPayload.currentFirstResponseByteTotalAvgMs,
      ),
      currentFirstTokenAvgMs: normalizeFiniteNumber(summaryPayload.currentFirstTokenAvgMs),
      currentAvgTotalMs: normalizeFiniteNumber(summaryPayload.currentAvgTotalMs),
      currentAvgResponseMs: normalizeFiniteNumber(summaryPayload.currentAvgResponseMs),
      modelPerformance: normalizeModelPerformance(summaryPayload.modelPerformance),
    },
    networkLiveBucket: normalizeDashboardNetworkTimeseriesPoint(payload.networkLiveBucket),
    networkRealtimeRate: normalizeDashboardNetworkRealtimeRate(payload.networkRealtimeRate),
    accounts: Array.isArray(payload.accounts)
      ? payload.accounts
          .map(normalizeUpstreamAccountActivityAccount)
          .filter((item): item is UpstreamAccountActivityAccount => item != null)
      : undefined,
  };
}

export function normalizeDashboardActivityRecentResponse(
  raw: unknown,
): DashboardActivityRecentResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    rangeStart: typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    snapshotId: normalizeFiniteNumber(payload.snapshotId) ?? 0,
    accounts: Array.isArray(payload.accounts)
      ? payload.accounts.map((entry) => {
          const account = (entry ?? {}) as Record<string, unknown>;
          return {
            accountKey: typeof account.accountKey === "string" ? account.accountKey : "",
            recentInvocations: Array.isArray(account.recentInvocations)
              ? account.recentInvocations
                  .map(normalizePromptCacheConversationInvocationPreview)
                  .filter((item): item is PromptCacheConversationInvocationPreview => item != null)
              : [],
          };
        })
      : [],
  };
}

export function normalizeDashboardNetworkTimeseriesPoint(
  raw: unknown,
): DashboardNetworkTimeseriesPoint | null {
  if (!raw || typeof raw !== "object") {
    return null;
  }
  const point = raw as Record<string, unknown>;
  return {
    bucketStart: typeof point.bucketStart === "string" ? point.bucketStart : "",
    bucketEnd: typeof point.bucketEnd === "string" ? point.bucketEnd : "",
    uploadBytesPerSecond: normalizeFiniteNumber(point.uploadBytesPerSecond) ?? 0,
    downloadBytesPerSecond: normalizeFiniteNumber(point.downloadBytesPerSecond) ?? 0,
    uploadBytes: normalizeFiniteNumber(point.uploadBytes) ?? 0,
    downloadBytes: normalizeFiniteNumber(point.downloadBytes) ?? 0,
    isLiveBucket: point.isLiveBucket === true,
  };
}

export function normalizeDashboardNetworkRealtimeRate(
  raw: unknown,
): DashboardNetworkRealtimeRate | null {
  if (!raw || typeof raw !== "object") {
    return null;
  }
  const point = raw as Record<string, unknown>;
  const sampleStart = typeof point.sampleStart === "string" ? point.sampleStart : null;
  const sampleEnd = typeof point.sampleEnd === "string" ? point.sampleEnd : null;
  if (sampleStart == null || sampleEnd == null) {
    return null;
  }
  return {
    sampleStart,
    sampleEnd,
    sampleSeconds: normalizeFiniteNumber(point.sampleSeconds) ?? 1,
    uploadBytesPerSecond: normalizeFiniteNumber(point.uploadBytesPerSecond) ?? 0,
    downloadBytesPerSecond: normalizeFiniteNumber(point.downloadBytesPerSecond) ?? 0,
    uploadBytes: normalizeFiniteNumber(point.uploadBytes) ?? 0,
    downloadBytes: normalizeFiniteNumber(point.downloadBytes) ?? 0,
  };
}

export function normalizeDashboardNetworkTimeseriesResponse(
  raw: unknown,
): DashboardNetworkTimeseriesResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    range: typeof payload.range === "string" ? payload.range : "",
    rangeStart: typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    snapshotId: normalizeFiniteNumber(payload.snapshotId) ?? 0,
    bucketSeconds: normalizeFiniteNumber(payload.bucketSeconds) ?? 300,
    points: Array.isArray(payload.points)
      ? payload.points
          .map(normalizeDashboardNetworkTimeseriesPoint)
          .filter((item): item is DashboardNetworkTimeseriesPoint => item != null)
      : [],
  };
}

export function normalizeDashboardRecentNetworkWindowPoint(
  raw: unknown,
): DashboardRecentNetworkWindowPoint | null {
  if (!raw || typeof raw !== "object") {
    return null;
  }
  const point = raw as Record<string, unknown>;
  return {
    sampleStart: typeof point.sampleStart === "string" ? point.sampleStart : "",
    sampleEnd: typeof point.sampleEnd === "string" ? point.sampleEnd : "",
    uploadBytesPerSecond: normalizeFiniteNumber(point.uploadBytesPerSecond) ?? 0,
    downloadBytesPerSecond: normalizeFiniteNumber(point.downloadBytesPerSecond) ?? 0,
    uploadBytes: normalizeFiniteNumber(point.uploadBytes) ?? 0,
    downloadBytes: normalizeFiniteNumber(point.downloadBytes) ?? 0,
    isAvailable: point.isAvailable === true,
  };
}

export function normalizeDashboardRecentNetworkWindowResponse(
  raw: unknown,
): DashboardRecentNetworkWindowResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    rangeStart: typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    windowSeconds: normalizeFiniteNumber(payload.windowSeconds) ?? 300,
    sampleSeconds: normalizeFiniteNumber(payload.sampleSeconds) ?? 1,
    isWarmingUp: payload.isWarmingUp === true,
    points: Array.isArray(payload.points)
      ? payload.points
          .map(normalizeDashboardRecentNetworkWindowPoint)
          .filter((item): item is DashboardRecentNetworkWindowPoint => item != null)
      : [],
  };
}

export function normalizeSettingsPayload(raw: unknown): SettingsPayload {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    proxy: normalizeProxySettings(payload.proxy),
    forwardProxy: normalizeForwardProxySettings(payload.forwardProxy),
    pricing: normalizePricingSettings(payload.pricing),
  };
}

export function normalizeSystemStatusMetric(raw: unknown): SystemStatusMetric {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    count: normalizeFiniteNumber(payload.count) ?? 0,
    bytes: normalizeFiniteNumber(payload.bytes) ?? 0,
  };
}

export function normalizeSystemProjectionConsumerHealth(
  raw: unknown,
): SystemProjectionConsumerHealth {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    state: typeof payload.state === "string" ? payload.state : "preparing",
    cursorLag: normalizeFiniteNumber(payload.cursorLag) ?? 0,
    dirtyBucketCount: normalizeFiniteNumber(payload.dirtyBucketCount) ?? 0,
    pendingEventCount: normalizeFiniteNumber(payload.pendingEventCount) ?? 0,
    lastFlushElapsedMs: normalizeFiniteNumber(payload.lastFlushElapsedMs) ?? undefined,
    lastFlushAgeMs: normalizeFiniteNumber(payload.lastFlushAgeMs) ?? undefined,
    lastRepairScope:
      typeof payload.lastRepairScope === "string" ? payload.lastRepairScope : undefined,
    lastDeferReason:
      typeof payload.lastDeferReason === "string" ? payload.lastDeferReason : undefined,
    lastErrorKind: typeof payload.lastErrorKind === "string" ? payload.lastErrorKind : undefined,
  };
}

export function normalizeSystemProjectionHealth(raw: unknown): SystemProjectionHealth {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    terminal: normalizeSystemProjectionConsumerHealth(payload.terminal),
    longTerm: normalizeSystemProjectionConsumerHealth(payload.longTerm),
  };
}

export function normalizeSystemRawMetricsHealth(raw: unknown): SystemRawMetricsHealth {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    state: typeof payload.state === "string" ? payload.state : "preparing",
    inventoryCursor: normalizeFiniteNumber(payload.inventoryCursor) ?? 0,
    updatedAgeMs: normalizeFiniteNumber(payload.updatedAgeMs) ?? undefined,
  };
}

export function runtimePressureNumber(value: unknown): number {
  return normalizeFiniteNumber(value) ?? 0;
}

export function runtimePressureString(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}

export function normalizeRuntimePressureProcess(raw: unknown) {
  const process = (raw ?? {}) as Record<string, unknown>;
  return {
    rssBytes: runtimePressureNumber(process.rssBytes),
    rssAnonBytes: runtimePressureNumber(process.rssAnonBytes),
    swapBytes: runtimePressureNumber(process.swapBytes),
    peakRssBytes: runtimePressureNumber(process.peakRssBytes),
    threads: runtimePressureNumber(process.threads),
    managedBytes: runtimePressureNumber(process.managedBytes),
    unattributedAnonBytes: runtimePressureNumber(process.unattributedAnonBytes),
    pressureLevel: runtimePressureString(process.pressureLevel) ?? "unknown",
  };
}

export function normalizeRuntimePressureWriter(raw: unknown) {
  const writer = (raw ?? {}) as Record<string, unknown>;
  return {
    state: runtimePressureString(writer.state) ?? "unknown",
    pendingDepth: runtimePressureNumber(writer.pendingDepth),
    pendingBytes: runtimePressureNumber(writer.pendingBytes),
    transferBytes: runtimePressureNumber(writer.transferBytes),
    retryCount: runtimePressureNumber(writer.retryCount),
    p2FlushAttemptCount: runtimePressureNumber(writer.p2FlushAttemptCount),
    p2PressureDeferCount: runtimePressureNumber(writer.p2PressureDeferCount),
    p2LockRetryCount: runtimePressureNumber(writer.p2LockRetryCount),
    p2NextAttemptInMs: runtimePressureNumber(writer.p2NextAttemptInMs),
    p2DeferredAgeMs: runtimePressureNumber(writer.p2DeferredAgeMs),
    p2WakeReason: runtimePressureString(writer.p2WakeReason),
    invariantViolationCount: runtimePressureNumber(writer.invariantViolationCount),
    degradedReason: runtimePressureString(writer.degradedReason),
  };
}

export function normalizeRuntimePressureProjection(raw: unknown) {
  const projection = (raw ?? {}) as Record<string, unknown>;
  return {
    mode: runtimePressureString(projection.mode) ?? "unknown",
    activeTopicCount: runtimePressureNumber(projection.activeTopicCount),
    dirtyKeyCount: runtimePressureNumber(projection.dirtyKeyCount),
    coalescedEventCount: runtimePressureNumber(projection.coalescedEventCount),
    fullHydrationCount: runtimePressureNumber(projection.fullHydrationCount),
    boundedKeyHydrationCount: runtimePressureNumber(projection.boundedKeyHydrationCount),
    livePathDbReadCount: runtimePressureNumber(projection.livePathDbReadCount),
    baselineAgeMs: runtimePressureNumber(projection.baselineAgeMs),
    responseSource: runtimePressureString(projection.responseSource) ?? "unknown",
  };
}

export function normalizeRuntimePressureWriteCoordinator(raw: unknown) {
  if (!raw || typeof raw !== "object") return undefined;
  const coordinator = raw as Record<string, unknown>;
  return {
    mode: runtimePressureString(coordinator.mode) ?? "unknown",
    activeWriteClass: runtimePressureString(coordinator.activeWriteClass),
    p1WaiterCount: runtimePressureNumber(coordinator.p1WaiterCount),
    interactiveWaiterCount: runtimePressureNumber(coordinator.interactiveWaiterCount),
    p2WaiterCount: runtimePressureNumber(coordinator.p2WaiterCount),
    maintenanceWaiterCount: runtimePressureNumber(coordinator.maintenanceWaiterCount),
    maintenanceFairnessAdmissionCount: runtimePressureNumber(
      coordinator.maintenanceFairnessAdmissionCount,
    ),
    directWriteBypassCount: runtimePressureNumber(coordinator.directWriteBypassCount),
  };
}

export function normalizeRuntimePressureRetention(raw: unknown) {
  if (!raw || typeof raw !== "object") return undefined;
  const retention = raw as Record<string, unknown>;
  return {
    state: runtimePressureString(retention.state) ?? "unknown",
    operation: runtimePressureString(retention.operation),
    admissionMode: runtimePressureString(retention.admissionMode),
    batchRows: runtimePressureNumber(retention.batchRows),
    estimatedBytes: runtimePressureNumber(retention.estimatedBytes),
    prepareElapsedMs: runtimePressureNumber(retention.prepareElapsedMs),
    lockWaitMs: runtimePressureNumber(retention.lockWaitMs),
    executeMs: runtimePressureNumber(retention.executeMs),
    commitMs: runtimePressureNumber(retention.commitMs),
    budgetBreachCount: runtimePressureNumber(retention.budgetBreachCount),
    deferReason: runtimePressureString(retention.deferReason),
    starvationAgeMs: normalizeFiniteNumber(retention.starvationAgeMs) ?? undefined,
    p1WaiterCount: runtimePressureNumber(retention.p1WaiterCount),
    candidateRemainingHint: runtimePressureNumber(retention.candidateRemainingHint),
    lastError: runtimePressureString(retention.lastError),
  };
}

export function normalizeRuntimePressureSlice(raw: unknown) {
  const slice = (raw ?? {}) as Record<string, unknown>;
  return {
    buildCount: runtimePressureNumber(slice.buildCount),
    revisionCount: runtimePressureNumber(slice.revisionCount),
    cadenceMissCount: runtimePressureNumber(slice.cadenceMissCount),
  };
}

export function normalizeRuntimePressureDashboard(raw: unknown) {
  const projection = (raw ?? {}) as Record<string, unknown>;
  const slices = (projection.sliceCounters ?? {}) as Record<string, unknown>;
  return {
    mode: runtimePressureString(projection.mode) ?? "unknown",
    state: runtimePressureString(projection.state) ?? "unknown",
    producerState: runtimePressureString(projection.producerState) ?? "unknown",
    activeSubscriberCount: runtimePressureNumber(projection.activeSubscriberCount),
    livePathDbReadCount: runtimePressureNumber(projection.livePathDbReadCount),
    buildCount: runtimePressureNumber(projection.buildCount),
    revision: runtimePressureNumber(projection.revision),
    snapshotOrigin: runtimePressureString(projection.snapshotOrigin) ?? "unknown",
    lastGoodAgeMs: normalizeFiniteNumber(projection.lastGoodAgeMs) ?? undefined,
    degradedReason: runtimePressureString(projection.degradedReason),
    lastDeferReason: runtimePressureString(projection.lastDeferReason),
    sliceCounters: {
      current: normalizeRuntimePressureSlice(slices.current),
      network: normalizeRuntimePressureSlice(slices.network),
      terminal: normalizeRuntimePressureSlice(slices.terminal),
    },
  };
}

export function normalizeRuntimePressureDeliveryTopic(raw: unknown) {
  const topic = (raw ?? {}) as Record<string, unknown>;
  return {
    materializationCount: runtimePressureNumber(topic.materializationCount),
    serializationCount: runtimePressureNumber(topic.serializationCount),
    payloadCloneCount: runtimePressureNumber(topic.payloadCloneCount),
    frameBytesCount: runtimePressureNumber(topic.frameBytesCount),
    laggedCount: runtimePressureNumber(topic.laggedCount),
    skippedCount: runtimePressureNumber(topic.skippedCount),
    businessPayloadCount: runtimePressureNumber(topic.businessPayloadCount),
    jsonOverlayCount: runtimePressureNumber(topic.jsonOverlayCount),
  };
}

export function normalizeRuntimePressureHotTopic(raw: unknown) {
  const topic = (raw ?? {}) as Record<string, unknown>;
  return {
    topicClass: runtimePressureString(topic.topicClass) ?? "unknown",
    state: runtimePressureString(topic.state) ?? "unknown",
    activeSubscriberCount: runtimePressureNumber(topic.activeSubscriberCount),
    builderCount: runtimePressureNumber(topic.builderCount),
    genericFallbackBuildCount: runtimePressureNumber(topic.genericFallbackBuildCount),
    livePathDbReadCount: runtimePressureNumber(topic.livePathDbReadCount),
    materializationCount: runtimePressureNumber(topic.materializationCount),
    serializationCount: runtimePressureNumber(topic.serializationCount),
    payloadCloneCount: runtimePressureNumber(topic.payloadCloneCount),
    frameReused: runtimePressureNumber(topic.frameReused),
    cadenceMissCount: runtimePressureNumber(topic.cadenceMissCount),
    reconnectChurnCount: runtimePressureNumber(topic.reconnectChurnCount),
  };
}

export function normalizeRuntimePressureHealth(raw: unknown): RuntimePressureHealth | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const payload = raw as Record<string, unknown>;
  const requestPipeline = payload.requestPipeline as Record<string, unknown> | undefined;
  const eventBus = payload.eventBus as Record<string, unknown> | undefined;
  const backfill = payload.backfill as Record<string, unknown> | undefined;
  const hotTopics = payload.dashboardHotTopics as Record<string, unknown> | undefined;
  return {
    state: runtimePressureString(payload.state) ?? "unknown",
    process: normalizeRuntimePressureProcess(payload.process),
    allocator: {
      mallocArenaMax:
        runtimePressureString(
          (payload.allocator as Record<string, unknown> | undefined)?.mallocArenaMax,
        ) ?? "unknown",
    },
    writerAccounting: normalizeRuntimePressureWriter(payload.writerAccounting),
    promptCacheProjection: normalizeRuntimePressureProjection(payload.promptCacheProjection),
    proxySqliteWriteCoordinator: normalizeRuntimePressureWriteCoordinator(
      payload.proxySqliteWriteCoordinator,
    ),
    retentionWriteHealth: normalizeRuntimePressureRetention(payload.retentionWriteHealth),
    dashboardProjection: normalizeRuntimePressureDashboard(payload.dashboardProjection),
    delivery: {
      activity: normalizeRuntimePressureDeliveryTopic(
        (payload.delivery as Record<string, unknown> | undefined)?.activity,
      ),
      summary: normalizeRuntimePressureDeliveryTopic(
        (payload.delivery as Record<string, unknown> | undefined)?.summary,
      ),
      networkTimeseries: normalizeRuntimePressureDeliveryTopic(
        (payload.delivery as Record<string, unknown> | undefined)?.networkTimeseries,
      ),
      networkRecent: normalizeRuntimePressureDeliveryTopic(
        (payload.delivery as Record<string, unknown> | undefined)?.networkRecent,
      ),
    },
    dashboardHotTopics: hotTopics
      ? {
          state: runtimePressureString(hotTopics.state) ?? "unknown",
          activity: normalizeRuntimePressureHotTopic(hotTopics.activity),
          summary: normalizeRuntimePressureHotTopic(hotTopics.summary),
          networkTimeseries: normalizeRuntimePressureHotTopic(hotTopics.networkTimeseries),
          networkRecent: normalizeRuntimePressureHotTopic(hotTopics.networkRecent),
          workingConversations: normalizeRuntimePressureHotTopic(hotTopics.workingConversations),
          parallelWork: normalizeRuntimePressureHotTopic(hotTopics.parallelWork),
          timeseries: normalizeRuntimePressureHotTopic(hotTopics.timeseries),
        }
      : undefined,
    requestPipeline: requestPipeline
      ? {
          mode: runtimePressureString(requestPipeline.mode) ?? "unknown",
          lastSnapshotKind: runtimePressureString(requestPipeline.lastSnapshotKind) ?? "none",
          semanticParseCount: runtimePressureNumber(requestPipeline.semanticParseCount),
          wholeBodyMaterializationCount: runtimePressureNumber(
            requestPipeline.wholeBodyMaterializationCount,
          ),
          rewriteBufferPeakBytes: runtimePressureNumber(requestPipeline.rewriteBufferPeakBytes),
          lastFallbackReason: runtimePressureString(requestPipeline.lastFallbackReason),
          parseWindowCount: runtimePressureNumber(requestPipeline.parseWindowCount),
          parseWindowCpuMs: runtimePressureNumber(requestPipeline.parseWindowCpuMs),
          parseWindowWallMs: runtimePressureNumber(requestPipeline.parseWindowWallMs),
          parseWindowBytes: runtimePressureNumber(requestPipeline.parseWindowBytes),
        }
      : undefined,
    eventBus: eventBus
      ? {
          state: runtimePressureString(eventBus.state) ?? "unknown",
          publishedCount: runtimePressureNumber(eventBus.publishedCount),
          processedEventCount: runtimePressureNumber(eventBus.processedEventCount),
          coalescedEventCount: runtimePressureNumber(eventBus.coalescedEventCount),
          businessPayloadCloneCount: runtimePressureNumber(eventBus.businessPayloadCloneCount),
          topicWorkCount: runtimePressureNumber(eventBus.topicWorkCount),
          routerLaggedCount: runtimePressureNumber(eventBus.routerLaggedCount),
          routerGapCount: runtimePressureNumber(eventBus.routerGapCount),
          cursorRecoveryCount: runtimePressureNumber(eventBus.cursorRecoveryCount),
        }
      : undefined,
    backfill: backfill
      ? {
          state: runtimePressureString(backfill.state) ?? "unknown",
          wakeGeneration: runtimePressureNumber(backfill.wakeGeneration),
          wakeCount: runtimePressureNumber(backfill.wakeCount),
          dueDispatchCount: runtimePressureNumber(backfill.dueDispatchCount),
          noopSuppressedCount: runtimePressureNumber(backfill.noopSuppressedCount),
          pressureDeferCount: runtimePressureNumber(backfill.pressureDeferCount),
          failureCount: runtimePressureNumber(backfill.failureCount),
          wokenTaskCount: runtimePressureNumber(backfill.wokenTaskCount),
          scheduledTaskCount: runtimePressureNumber(backfill.scheduledTaskCount),
          deferredTaskCount: runtimePressureNumber(backfill.deferredTaskCount),
          failedTaskCount: runtimePressureNumber(backfill.failedTaskCount),
        }
      : undefined,
  };
}

export function normalizeSystemStatusResponse(raw: unknown): SystemStatusResponse {
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
    projectionHealth: normalizeSystemProjectionHealth(payload.projectionHealth),
    rawMetricsHealth: normalizeSystemRawMetricsHealth(payload.rawMetricsHealth),
    runtimePressureHealth: normalizeRuntimePressureHealth(payload.runtimePressureHealth),
    refreshedAt: typeof payload.refreshedAt === "string" ? payload.refreshedAt : "",
  };
}

export function normalizeSystemTaskRun(raw: unknown): SystemTaskRun | null {
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

export function normalizeSystemTaskRunsResponse(raw: unknown): SystemTaskRunsResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const itemsRaw = Array.isArray(payload.items) ? payload.items : [];
  return {
    items: itemsRaw
      .map(normalizeSystemTaskRun)
      .filter((item): item is SystemTaskRun => item != null),
    total: normalizeFiniteNumber(payload.total) ?? 0,
    page: normalizeFiniteNumber(payload.page) ?? 1,
    pageSize: normalizeFiniteNumber(payload.pageSize) ?? 20,
    nextCursor: typeof payload.nextCursor === "string" ? payload.nextCursor : undefined,
  };
}

export function normalizeExternalApiKeySummary(raw: unknown): ExternalApiKeySummary | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const id = normalizeFiniteNumber(payload.id);
  const name = typeof payload.name === "string" ? payload.name : "";
  const status = typeof payload.status === "string" ? payload.status : "";
  const prefix = typeof payload.prefix === "string" ? payload.prefix : "";
  const createdAt = typeof payload.createdAt === "string" ? payload.createdAt : "";
  const updatedAt = typeof payload.updatedAt === "string" ? payload.updatedAt : "";
  if (id == null || !name || !status || !prefix || !createdAt || !updatedAt) {
    return null;
  }
  return {
    id,
    name,
    status,
    prefix,
    lastUsedAt: typeof payload.lastUsedAt === "string" ? payload.lastUsedAt : undefined,
    createdAt,
    updatedAt,
  };
}

export function normalizeExternalApiKeyListResponse(raw: unknown): ExternalApiKeyListResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const items = Array.isArray(payload.items)
    ? payload.items
        .map((item) => normalizeExternalApiKeySummary(item))
        .filter((item): item is ExternalApiKeySummary => item != null)
    : [];
  return { items };
}

export function normalizeExternalApiKeyMutationResponse(
  raw: unknown,
): ExternalApiKeyMutationResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const key = normalizeExternalApiKeySummary(payload.key);
  if (!key) {
    throw new Error("invalid external API key response");
  }
  return { key };
}

export function normalizeExternalApiKeySecretResponse(raw: unknown): ExternalApiKeySecretResponse {
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
