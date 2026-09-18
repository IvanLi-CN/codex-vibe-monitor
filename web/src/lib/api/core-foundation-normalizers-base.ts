import { normalizeForwardProxyProtocolLabel } from "../forwardProxyDisplay";
import type {
  ParallelWorkConversation,
  ParallelWorkPoint,
  ParallelWorkStatsResponse,
  ParallelWorkWindowResponse,
  RoutingStateVersion,
  TimeseriesPoint,
  TimeseriesResponse,
} from "./core-foundation-dashboard-types";
import type {
  BlockedBindingDiagnostic,
  InvocationLivePhase,
  InvocationPhaseCounts,
  UsageBreakdown,
  UsageCostBreakdown,
} from "./core-foundation-record-types";
import type {
  ConversationRequestPoint,
  ForwardProxyBindingNode,
  ForwardProxyHourlyBucket,
  ForwardProxyLatencyTargetResult,
  ForwardProxyLatencyTestStreamEvent,
  ForwardProxyLiveNode,
  ForwardProxyLiveStatsResponse,
  ForwardProxyNode,
  ForwardProxyNodeStats,
  ForwardProxyRefreshSubscriptionsResult,
  ForwardProxySettings,
  ForwardProxyTimeseriesNode,
  ForwardProxyTimeseriesResponse,
  ForwardProxyWeightBucket,
  ForwardProxyWindowStats,
  PricingEntry,
  PricingSettings,
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationManualBinding,
  PromptCacheConversationRequestPoint,
  PromptCacheConversationUpstreamAccount,
  ProxyFastModeRewriteMode,
  ProxySettings,
} from "./core-foundation-settings-types";
export function normalizeStringArray(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.filter((item): item is string => typeof item === "string");
}

export function normalizeFiniteNumber(value: unknown): number | undefined {
  if (typeof value !== "number" || !Number.isFinite(value)) return undefined;
  return value;
}

export function normalizeRoutingStateVersion(value: unknown): RoutingStateVersion | null {
  const payload = (value ?? {}) as Record<string, unknown>;
  return typeof payload.epoch === "string" &&
    payload.epoch.trim() &&
    typeof payload.generation === "string" &&
    /^\d+$/.test(payload.generation)
    ? { epoch: payload.epoch, generation: payload.generation }
    : null;
}

export function normalizePositiveFiniteNumber(value: unknown): number | undefined {
  const normalized = normalizeFiniteNumber(value);
  return normalized != null && normalized > 0 ? normalized : undefined;
}

export function normalizeInvocationLivePhase(value: unknown): InvocationLivePhase | null {
  if (typeof value !== "string") return null;
  const phase = value.trim().toLowerCase();
  if (phase === "queued" || phase === "requesting" || phase === "responding") {
    return phase;
  }
  return null;
}

export function normalizeInvocationPhaseCounts(value: unknown): InvocationPhaseCounts | null {
  if (!value || typeof value !== "object") return null;
  const payload = value as Record<string, unknown>;
  return {
    queued: Math.max(0, normalizeFiniteNumber(payload.queued) ?? 0),
    requesting: Math.max(0, normalizeFiniteNumber(payload.requesting) ?? 0),
    responding: Math.max(0, normalizeFiniteNumber(payload.responding) ?? 0),
  };
}

export function normalizeUsageCostBreakdown(raw: unknown): UsageCostBreakdown | null {
  if (!raw || typeof raw !== "object") return null;
  const payload = raw as Record<string, unknown>;
  const input = normalizeFiniteNumber(payload.input);
  const cacheWrite = normalizeFiniteNumber(payload.cacheWrite);
  const cacheRead = normalizeFiniteNumber(payload.cacheRead);
  const output = normalizeFiniteNumber(payload.output);
  const reasoning = normalizeFiniteNumber(payload.reasoning);
  if ([input, cacheWrite, cacheRead, output, reasoning].some((value) => value == null)) return null;
  return {
    input: input ?? 0,
    cacheWrite: cacheWrite ?? 0,
    cacheRead: cacheRead ?? 0,
    output: output ?? 0,
    reasoning: reasoning ?? 0,
    unknown: normalizeFiniteNumber(payload.unknown) ?? 0,
  };
}

export function normalizeUsageBreakdown(raw: unknown): UsageBreakdown | null {
  if (!raw || typeof raw !== "object") return null;
  const payload = raw as Record<string, unknown>;
  const models = Array.isArray(payload.models)
    ? payload.models.flatMap((rawModel) => {
        const model = (rawModel ?? {}) as Record<string, unknown>;
        const name = typeof model.model === "string" ? model.model.trim() : "";
        if (!name) return [];
        return [
          {
            model: name,
            reasoningEffort:
              typeof model.reasoningEffort === "string" && model.reasoningEffort.trim()
                ? model.reasoningEffort.trim()
                : null,
            cacheWriteTokens: normalizeFiniteNumber(model.cacheWriteTokens) ?? 0,
            cacheReadTokens: normalizeFiniteNumber(model.cacheReadTokens) ?? 0,
            outputTokens: normalizeFiniteNumber(model.outputTokens) ?? 0,
            costs: normalizeUsageCostBreakdown(model.costs),
          },
        ];
      })
    : [];
  return {
    cacheWriteTokens: normalizeFiniteNumber(payload.cacheWriteTokens) ?? 0,
    cacheReadTokens: normalizeFiniteNumber(payload.cacheReadTokens) ?? 0,
    outputTokens: normalizeFiniteNumber(payload.outputTokens) ?? 0,
    costs: normalizeUsageCostBreakdown(payload.costs),
    models,
  };
}

export function normalizeTimeseriesPoint(raw: unknown): TimeseriesPoint | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const bucketStart = typeof payload.bucketStart === "string" ? payload.bucketStart : "";
  const bucketEnd = typeof payload.bucketEnd === "string" ? payload.bucketEnd : "";
  if (!bucketStart || !bucketEnd) return null;
  const totalCount = normalizeFiniteNumber(payload.totalCount) ?? 0;
  const successCount = normalizeFiniteNumber(payload.successCount) ?? 0;
  const failureCount = normalizeFiniteNumber(payload.failureCount) ?? 0;
  const inFlightCount = normalizeFiniteNumber(payload.inFlightCount) ?? 0;
  const inFlightPhaseCounts = normalizeInvocationPhaseCounts(payload.inFlightPhaseCounts);
  const hasCalls =
    Math.max(totalCount, successCount + failureCount + Math.max(inFlightCount, 0)) > 0;
  return {
    bucketStart,
    bucketEnd,
    totalCount,
    successCount,
    failureCount,
    inFlightCount,
    inFlightPhaseCounts,
    totalTokens: normalizeFiniteNumber(payload.totalTokens) ?? 0,
    inputTokens: normalizeFiniteNumber(payload.inputTokens) ?? undefined,
    outputTokens: normalizeFiniteNumber(payload.outputTokens) ?? undefined,
    cacheInputTokens: normalizeFiniteNumber(payload.cacheInputTokens) ?? 0,
    reasoningTokens: normalizeFiniteNumber(payload.reasoningTokens) ?? undefined,
    totalCost: normalizeFiniteNumber(payload.totalCost) ?? 0,
    nonSuccessCost: normalizeFiniteNumber(payload.nonSuccessCost) ?? 0,
    avgTotalMs: hasCalls ? (normalizeFiniteNumber(payload.avgTotalMs) ?? null) : null,
    totalLatencySampleCount: hasCalls
      ? (normalizeFiniteNumber(payload.totalLatencySampleCount) ?? null)
      : null,
    firstByteSampleCount: hasCalls ? (normalizeFiniteNumber(payload.firstByteSampleCount) ?? 0) : 0,
    firstByteAvgMs: hasCalls ? (normalizeFiniteNumber(payload.firstByteAvgMs) ?? null) : null,
    firstByteP95Ms: hasCalls ? (normalizeFiniteNumber(payload.firstByteP95Ms) ?? null) : null,
    firstResponseByteTotalSampleCount: hasCalls
      ? (normalizeFiniteNumber(payload.firstResponseByteTotalSampleCount) ?? 0)
      : 0,
    firstResponseByteTotalAvgMs: hasCalls
      ? (normalizeFiniteNumber(payload.firstResponseByteTotalAvgMs) ?? null)
      : null,
    firstResponseByteTotalP95Ms: hasCalls
      ? (normalizeFiniteNumber(payload.firstResponseByteTotalP95Ms) ?? null)
      : null,
    firstTokenSampleCount: hasCalls
      ? (normalizeFiniteNumber(payload.firstTokenSampleCount) ?? 0)
      : 0,
    firstTokenAvgMs: hasCalls ? (normalizeFiniteNumber(payload.firstTokenAvgMs) ?? null) : null,
    firstTokenP95Ms: hasCalls ? (normalizeFiniteNumber(payload.firstTokenP95Ms) ?? null) : null,
  };
}

export function normalizeTimeseriesResponse(raw: unknown): TimeseriesResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const pointsRaw = Array.isArray(payload.points) ? payload.points : [];
  return {
    rangeStart: typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    bucketSeconds: normalizeFiniteNumber(payload.bucketSeconds) ?? 3600,
    snapshotId: normalizeFiniteNumber(payload.snapshotId) ?? undefined,
    effectiveBucket:
      typeof payload.effectiveBucket === "string" ? payload.effectiveBucket : undefined,
    availableBuckets: normalizeStringArray(payload.availableBuckets),
    bucketLimitedToDaily: payload.bucketLimitedToDaily === true,
    points: pointsRaw
      .map(normalizeTimeseriesPoint)
      .filter((point): point is TimeseriesPoint => point != null),
  };
}

export function normalizeParallelWorkPoint(raw: unknown): ParallelWorkPoint | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const bucketStart = typeof payload.bucketStart === "string" ? payload.bucketStart : "";
  const bucketEnd = typeof payload.bucketEnd === "string" ? payload.bucketEnd : "";
  if (!bucketStart || !bucketEnd) return null;
  return {
    bucketStart,
    bucketEnd,
    parallelCount: normalizeFiniteNumber(payload.parallelCount) ?? 0,
  };
}

export function normalizeParallelWorkConversation(raw: unknown): ParallelWorkConversation | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const conversationId = typeof payload.conversationId === "string" ? payload.conversationId : "";
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

export function normalizeParallelWorkWindowResponse(raw: unknown): ParallelWorkWindowResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const pointsRaw = Array.isArray(payload.points) ? payload.points : [];
  const conversationsRaw = Array.isArray(payload.conversations) ? payload.conversations : [];
  const effectiveTimeZone =
    typeof payload.effectiveTimeZone === "string" && payload.effectiveTimeZone.trim()
      ? payload.effectiveTimeZone.trim()
      : "Asia/Shanghai";
  return {
    rangeStart: typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    bucketSeconds: normalizeFiniteNumber(payload.bucketSeconds) ?? 0,
    completeBucketCount: normalizeFiniteNumber(payload.completeBucketCount) ?? 0,
    activeBucketCount: normalizeFiniteNumber(payload.activeBucketCount) ?? 0,
    activeMinuteCount:
      payload.activeMinuteCount == null
        ? null
        : (normalizeFiniteNumber(payload.activeMinuteCount) ?? null),
    minCount: payload.minCount == null ? null : (normalizeFiniteNumber(payload.minCount) ?? null),
    maxCount: payload.maxCount == null ? null : (normalizeFiniteNumber(payload.maxCount) ?? null),
    avgCount: payload.avgCount == null ? null : (normalizeFiniteNumber(payload.avgCount) ?? null),
    effectiveTimeZone,
    timeZoneFallback: payload.timeZoneFallback === true,
    points: pointsRaw
      .map(normalizeParallelWorkPoint)
      .filter((point): point is ParallelWorkPoint => point != null),
    conversations: conversationsRaw
      .map(normalizeParallelWorkConversation)
      .filter((conversation): conversation is ParallelWorkConversation => conversation != null),
  };
}

export function normalizeParallelWorkStatsResponse(raw: unknown): ParallelWorkStatsResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const current = normalizeParallelWorkWindowResponse(payload.current ?? payload.minute7d);
  return {
    current,
    minute7d: normalizeParallelWorkWindowResponse(payload.minute7d ?? current),
    hour30d: normalizeParallelWorkWindowResponse(payload.hour30d ?? current),
    dayAll: normalizeParallelWorkWindowResponse(payload.dayAll ?? current),
  };
}

export function normalizePricingEntry(raw: unknown): PricingEntry | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const model = typeof payload.model === "string" ? payload.model.trim() : "";
  const inputPer1m = normalizeFiniteNumber(payload.inputPer1m);
  const outputPer1m = normalizeFiniteNumber(payload.outputPer1m);
  if (!model || inputPer1m === undefined || outputPer1m === undefined) return null;
  const legacyCacheInputPer1m = normalizeFiniteNumber(payload.cacheInputPer1m);
  const cacheReadPer1m = normalizeFiniteNumber(payload.cacheReadPer1m) ?? legacyCacheInputPer1m;
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

export function normalizePricingSettings(raw: unknown): PricingSettings {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const entriesRaw = Array.isArray(payload.entries) ? payload.entries : [];
  const entries = entriesRaw
    .map(normalizePricingEntry)
    .filter((entry): entry is PricingEntry => entry != null)
    .sort((a, b) => a.model.localeCompare(b.model));
  return {
    catalogVersion:
      typeof payload.catalogVersion === "string" && payload.catalogVersion.trim()
        ? payload.catalogVersion.trim()
        : "custom",
    entries,
  };
}

export function normalizeProxyFastModeRewriteMode(raw: unknown): ProxyFastModeRewriteMode {
  return raw === "fill_missing" || raw === "force_priority" ? raw : "disabled";
}

export function normalizeProxySettings(raw: unknown): ProxySettings {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const models = normalizeStringArray(payload.models);
  const enabledModelSet = new Set(normalizeStringArray(payload.enabledModels));
  return {
    hijackEnabled: payload.hijackEnabled === true,
    mergeUpstreamEnabled: payload.mergeUpstreamEnabled === true,
    fastModeRewriteMode: normalizeProxyFastModeRewriteMode(payload.fastModeRewriteMode),
    upstream429MaxRetries: Math.max(
      0,
      Math.min(5, Math.trunc(normalizeFiniteNumber(payload.upstream429MaxRetries) ?? 3)),
    ),
    websocketEnabled: payload.websocketEnabled === true,
    upstreamWebsocketDefaultEnabled: payload.upstreamWebsocketDefaultEnabled === true,
    requestBodyLoggingEnabled: payload.requestBodyLoggingEnabled !== false,
    responseBodyLoggingEnabled: payload.responseBodyLoggingEnabled !== false,
    encryptedSessionOwnerRoutingEnabled: payload.encryptedSessionOwnerRoutingEnabled === true,
    defaultHijackEnabled: payload.defaultHijackEnabled === true,
    models,
    imageModels: normalizeStringArray(payload.imageModels),
    enabledModels: models.filter((model) => enabledModelSet.has(model)),
  };
}

export function normalizeForwardProxyWindowStats(raw: unknown): ForwardProxyWindowStats {
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

export function emptyForwardProxyNodeStats(): ForwardProxyNodeStats {
  return {
    oneMinute: { attempts: 0 },
    fifteenMinutes: { attempts: 0 },
    oneHour: { attempts: 0 },
    oneDay: { attempts: 0 },
    sevenDays: { attempts: 0 },
  };
}

export function normalizeForwardProxyNode(raw: unknown): ForwardProxyNode | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const key = typeof payload.key === "string" ? payload.key : "";
  if (!key) return null;
  const statsPayload = (payload.stats ?? {}) as Record<string, unknown>;
  return {
    key,
    source: typeof payload.source === "string" ? payload.source : "manual",
    displayName: typeof payload.displayName === "string" ? payload.displayName : key,
    endpointUrl: typeof payload.endpointUrl === "string" ? payload.endpointUrl : undefined,
    weight: normalizeFiniteNumber(payload.weight) ?? 0,
    penalized: Boolean(payload.penalized),
    stats: {
      oneMinute: normalizeForwardProxyWindowStats(statsPayload.oneMinute),
      fifteenMinutes: normalizeForwardProxyWindowStats(statsPayload.fifteenMinutes),
      oneHour: normalizeForwardProxyWindowStats(statsPayload.oneHour),
      oneDay: normalizeForwardProxyWindowStats(statsPayload.oneDay),
      sevenDays: normalizeForwardProxyWindowStats(statsPayload.sevenDays),
    },
  };
}

export function normalizeForwardProxyBindingNode(raw: unknown): ForwardProxyBindingNode | null {
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
      typeof payload.protocolLabel === "string" ? payload.protocolLabel : undefined,
    ),
    egressIp: typeof payload.egressIp === "string" ? payload.egressIp : null,
    egressIpCheckedAt:
      typeof payload.egressIpCheckedAt === "string" ? payload.egressIpCheckedAt : null,
    egressIpProvider:
      typeof payload.egressIpProvider === "string" ? payload.egressIpProvider : null,
    egressIpError: typeof payload.egressIpError === "string" ? payload.egressIpError : null,
    egressIpErrorAt: typeof payload.egressIpErrorAt === "string" ? payload.egressIpErrorAt : null,
    penalized: Boolean(payload.penalized),
    selectable: payload.selectable === true,
    last24h: bucketsRaw
      .map(normalizeForwardProxyHourlyBucket)
      .filter((item): item is ForwardProxyHourlyBucket => item != null),
  };
}

export function normalizeForwardProxySettings(raw: unknown): ForwardProxySettings {
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

export function normalizeForwardProxyLatencyTargetResult(
  raw: unknown,
): ForwardProxyLatencyTargetResult {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const latencyMs = normalizeFiniteNumber(payload.latencyMs);
  const httpStatus = normalizeFiniteNumber(payload.httpStatus);
  return {
    ok: payload.ok === true,
    latencyMs,
    ip: typeof payload.ip === "string" ? payload.ip : undefined,
    httpStatus: httpStatus == null ? undefined : Math.max(0, Math.trunc(httpStatus)),
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
  const oauthUpstream = normalizeForwardProxyLatencyTargetResult(nodeRaw.oauthUpstream);
  const codexResponses = normalizeForwardProxyLatencyTargetResult(nodeRaw.codexResponses);
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
      displayName: typeof nodeRaw.displayName === "string" ? nodeRaw.displayName : key,
      round: normalizeFiniteNumber(nodeRaw.round) ?? 0,
      totalRounds: normalizeFiniteNumber(nodeRaw.totalRounds) ?? 5,
      completedRounds: normalizeFiniteNumber(nodeRaw.completedRounds) ?? 0,
      successCount: normalizeFiniteNumber(nodeRaw.successCount) ?? 0,
      attemptCount: normalizeFiniteNumber(nodeRaw.attemptCount) ?? 0,
      averageLatencyMs: normalizeFiniteNumber(nodeRaw.averageLatencyMs) ?? undefined,
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

export function normalizeForwardProxyRefreshSubscriptionsResult(
  raw: unknown,
): ForwardProxyRefreshSubscriptionsResult {
  const payload = (raw ?? {}) as Record<string, unknown>;
  return {
    forwardProxy: normalizeForwardProxySettings(payload.forwardProxy),
    subscriptionCount: normalizeFiniteNumber(payload.subscriptionCount) ?? 0,
    addedNodeCount: normalizeFiniteNumber(payload.addedNodeCount) ?? 0,
    refreshedAt: typeof payload.refreshedAt === "string" ? payload.refreshedAt : "",
  };
}

export function normalizeForwardProxyHourlyBucket(raw: unknown): ForwardProxyHourlyBucket | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const bucketStart = typeof payload.bucketStart === "string" ? payload.bucketStart : "";
  const bucketEnd = typeof payload.bucketEnd === "string" ? payload.bucketEnd : "";
  if (!bucketStart || !bucketEnd) return null;
  return {
    bucketStart,
    bucketEnd,
    successCount: normalizeFiniteNumber(payload.successCount) ?? 0,
    failureCount: normalizeFiniteNumber(payload.failureCount) ?? 0,
  };
}

export function normalizeForwardProxyWeightBucket(raw: unknown): ForwardProxyWeightBucket | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const bucketStart = typeof payload.bucketStart === "string" ? payload.bucketStart : "";
  const bucketEnd = typeof payload.bucketEnd === "string" ? payload.bucketEnd : "";
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

export function normalizeForwardProxyLiveNode(raw: unknown): ForwardProxyLiveNode | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const base = normalizeForwardProxyNode(raw);
  if (!base) return null;
  const bucketsRaw = Array.isArray(payload.last24h) ? payload.last24h : [];
  const last24h = bucketsRaw
    .map(normalizeForwardProxyHourlyBucket)
    .filter((item): item is ForwardProxyHourlyBucket => item != null);
  const weightBucketsRaw = Array.isArray(payload.weight24h) ? payload.weight24h : [];
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

export function normalizeForwardProxyLiveStatsResponse(
  raw: unknown,
): ForwardProxyLiveStatsResponse {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const nodesRaw = Array.isArray(payload.nodes) ? payload.nodes : [];
  const nodes = nodesRaw
    .map(normalizeForwardProxyLiveNode)
    .filter((node): node is ForwardProxyLiveNode => node != null)
    .sort((a, b) => a.displayName.localeCompare(b.displayName));
  return {
    rangeStart: typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    bucketSeconds: normalizeFiniteNumber(payload.bucketSeconds) ?? 3600,
    nodes,
  };
}

export function normalizeForwardProxyTimeseriesNode(
  raw: unknown,
): ForwardProxyTimeseriesNode | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const base = normalizeForwardProxyNode(raw);
  if (!base) return null;
  const bucketsRaw = Array.isArray(payload.buckets) ? payload.buckets : [];
  const weightBucketsRaw = Array.isArray(payload.weightBuckets) ? payload.weightBuckets : [];
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

export function normalizeForwardProxyTimeseriesResponse(
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
    rangeStart: typeof payload.rangeStart === "string" ? payload.rangeStart : "",
    rangeEnd: typeof payload.rangeEnd === "string" ? payload.rangeEnd : "",
    bucketSeconds: normalizeFiniteNumber(payload.bucketSeconds) ?? 3600,
    effectiveBucket: typeof payload.effectiveBucket === "string" ? payload.effectiveBucket : "1h",
    availableBuckets: availableBucketsRaw.filter(
      (item): item is string => typeof item === "string" && item.length > 0,
    ),
    nodes,
  };
}

export function normalizePromptCacheConversationRequestPoint(
  raw: unknown,
): PromptCacheConversationRequestPoint | null {
  return normalizeConversationRequestPoint(raw);
}

export function normalizeBlockedBindingDiagnostic(raw: unknown): BlockedBindingDiagnostic | null {
  if (!raw || typeof raw !== "object") return null;
  const payload = raw as Record<string, unknown>;
  const constraintSource =
    typeof payload.constraintSource === "string" && payload.constraintSource.trim()
      ? payload.constraintSource.trim()
      : null;
  if (!constraintSource) return null;
  const upstreamAccountId = normalizeFiniteNumber(payload.upstreamAccountId) ?? null;
  const upstreamAccountLabel =
    typeof payload.upstreamAccountLabel === "string" && payload.upstreamAccountLabel.trim()
      ? payload.upstreamAccountLabel.trim()
      : null;
  const promptCacheKey =
    typeof payload.promptCacheKey === "string" && payload.promptCacheKey.trim()
      ? payload.promptCacheKey.trim()
      : null;
  const recoveryAction =
    typeof payload.recoveryAction === "string" && payload.recoveryAction.trim()
      ? payload.recoveryAction.trim()
      : null;
  return {
    constraintSource,
    upstreamAccountId,
    upstreamAccountLabel,
    promptCacheKey,
    recoveryAction,
  };
}

export function normalizePromptCacheConversationUpstreamAccount(
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
    lastActivityAt: typeof payload.lastActivityAt === "string" ? payload.lastActivityAt : "",
  };
}

export function normalizeInvocationPreviewIdentity(
  payload: Record<string, unknown>,
  invokeId: string,
  occurredAt: string,
  failureClass: string,
) {
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
    failureClass: normalizeInvocationPreviewFailureClass(failureClass),
    routeMode:
      typeof payload.routeMode === "string" && payload.routeMode.trim()
        ? payload.routeMode.trim()
        : null,
    model: typeof payload.model === "string" && payload.model.trim() ? payload.model.trim() : null,
    requestModel:
      typeof payload.requestModel === "string" && payload.requestModel.trim()
        ? payload.requestModel.trim()
        : null,
    responseModel:
      typeof payload.responseModel === "string" && payload.responseModel.trim()
        ? payload.responseModel.trim()
        : null,
  };
}

export function normalizeInvocationPreviewFailureClass(
  value: string,
): PromptCacheConversationInvocationPreview["failureClass"] {
  return value === "none" ||
    value === "service_failure" ||
    value === "client_failure" ||
    value === "client_abort"
    ? value
    : null;
}

export function normalizeInvocationPreviewAccounting(payload: Record<string, unknown>) {
  return {
    totalTokens: normalizeFiniteNumber(payload.totalTokens) ?? 0,
    cost: normalizeFiniteNumber(payload.cost) ?? null,
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
  };
}

export function normalizeInvocationPreviewTransport(payload: Record<string, unknown>) {
  const text = (key: string) =>
    typeof payload[key] === "string" && payload[key].trim() ? String(payload[key]).trim() : null;
  const optionalText = (key: string) => (text(key) ? (text(key) ?? undefined) : undefined);
  const compactionKind = (
    value: unknown,
  ): PromptCacheConversationInvocationPreview["compactionRequestKind"] =>
    value === "compact" || value === "remote_v2" ? value : null;
  return {
    proxyDisplayName: text("proxyDisplayName"),
    upstreamAccountId: normalizeFiniteNumber(payload.upstreamAccountId) ?? null,
    upstreamAccountName: text("upstreamAccountName"),
    upstreamAccountPlanType: text("upstreamAccountPlanType"),
    endpoint: text("endpoint"),
    compactionRequestKind: compactionKind(payload.compactionRequestKind),
    compactionResponseKind: compactionKind(payload.compactionResponseKind),
    imageIntent: normalizeInvocationPreviewImageIntent(payload.imageIntent),
    source: optionalText("source"),
    reasoningEffort: optionalText("reasoningEffort"),
    errorMessage:
      typeof payload.errorMessage === "string" && payload.errorMessage.trim()
        ? payload.errorMessage
        : undefined,
    downstreamStatusCode: normalizeFiniteNumber(payload.downstreamStatusCode),
    downstreamErrorMessage:
      typeof payload.downstreamErrorMessage === "string" && payload.downstreamErrorMessage.trim()
        ? payload.downstreamErrorMessage
        : undefined,
    failureKind: optionalText("failureKind"),
    isActionable: typeof payload.isActionable === "boolean" ? payload.isActionable : undefined,
    responseContentEncoding: optionalText("responseContentEncoding"),
    requestCompressionAlgorithm: optionalText("requestCompressionAlgorithm"),
    requestedServiceTier: optionalText("requestedServiceTier"),
    serviceTier: optionalText("serviceTier"),
    billingServiceTier: optionalText("billingServiceTier"),
    tReqReadMs: normalizeFiniteNumber(payload.tReqReadMs),
    tReqParseMs: normalizeFiniteNumber(payload.tReqParseMs),
    tUpstreamConnectMs: normalizeFiniteNumber(payload.tUpstreamConnectMs),
    tUpstreamTtfbMs: normalizeFiniteNumber(payload.tUpstreamTtfbMs),
    firstTokenMs: normalizeFiniteNumber(payload.firstTokenMs),
    tUpstreamStreamMs: normalizePositiveFiniteNumber(payload.tUpstreamStreamMs),
    tRespParseMs: normalizeFiniteNumber(payload.tRespParseMs),
    tPersistMs: normalizeFiniteNumber(payload.tPersistMs),
    tTotalMs: normalizeFiniteNumber(payload.tTotalMs),
    blockedBinding: normalizeBlockedBindingDiagnostic(payload.blockedBinding),
  };
}

export function normalizeInvocationPreviewImageIntent(
  value: unknown,
): PromptCacheConversationInvocationPreview["imageIntent"] {
  return value === "yes" || value === "direct_image" || value === "no" || value === "unknown"
    ? value
    : null;
}

export function normalizePromptCacheConversationInvocationPreview(
  raw: unknown,
): PromptCacheConversationInvocationPreview | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const invokeId = typeof payload.invokeId === "string" ? payload.invokeId.trim() : "";
  const occurredAt = typeof payload.occurredAt === "string" ? payload.occurredAt : "";
  if (!invokeId || !occurredAt) return null;
  const failureClass =
    typeof payload.failureClass === "string" ? payload.failureClass.trim().toLowerCase() : "";
  return {
    ...normalizeInvocationPreviewIdentity(payload, invokeId, occurredAt, failureClass),
    ...normalizeInvocationPreviewAccounting(payload),
    ...normalizeInvocationPreviewTransport(payload),
  };
}

export function normalizePromptCacheConversation(raw: unknown): PromptCacheConversation | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const promptCacheKey =
    typeof payload.promptCacheKey === "string" ? payload.promptCacheKey.trim() : "";
  if (!promptCacheKey) return null;
  const requestsRaw = Array.isArray(payload.last24hRequests) ? payload.last24hRequests : [];
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
    lastActivityAt: typeof payload.lastActivityAt === "string" ? payload.lastActivityAt : "",
    lastTerminalAt: typeof payload.lastTerminalAt === "string" ? payload.lastTerminalAt : null,
    lastInFlightAt: typeof payload.lastInFlightAt === "string" ? payload.lastInFlightAt : null,
    inFlightPhaseCounts: normalizeInvocationPhaseCounts(payload.inFlightPhaseCounts) ?? {
      queued: 0,
      requesting: 0,
      responding: 0,
    },
    cursor: typeof payload.cursor === "string" ? payload.cursor : null,
    hasEncryptedSessionOwner:
      typeof payload.hasEncryptedSessionOwner === "boolean"
        ? payload.hasEncryptedSessionOwner
        : false,
    encryptedOwnerAccountId: normalizeFiniteNumber(payload.encryptedOwnerAccountId) ?? null,
    encryptedOwnerAccountName:
      typeof payload.encryptedOwnerAccountName === "string" &&
      payload.encryptedOwnerAccountName.trim()
        ? payload.encryptedOwnerAccountName.trim()
        : null,
    encryptedOwnerGroupName:
      typeof payload.encryptedOwnerGroupName === "string" && payload.encryptedOwnerGroupName.trim()
        ? payload.encryptedOwnerGroupName.trim()
        : null,
    manualBinding: normalizePromptCacheConversationManualBinding(payload.manualBinding),
    upstreamAccounts: upstreamAccountsRaw
      .map(normalizePromptCacheConversationUpstreamAccount)
      .filter((item): item is PromptCacheConversationUpstreamAccount => item != null),
    recentInvocations: recentInvocationsRaw
      .map(normalizePromptCacheConversationInvocationPreview)
      .filter((item): item is PromptCacheConversationInvocationPreview => item != null),
    last24hRequests: requestsRaw
      .map(normalizePromptCacheConversationRequestPoint)
      .filter((item): item is PromptCacheConversationRequestPoint => item != null),
    blockedBinding: normalizeBlockedBindingDiagnostic(payload.blockedBinding),
  };
}

export function normalizePromptCacheConversationManualBinding(
  raw: unknown,
): PromptCacheConversationManualBinding | null {
  if (!raw || typeof raw !== "object") return null;
  const payload = raw as Record<string, unknown>;
  const bindingKind =
    payload.bindingKind === "group" || payload.bindingKind === "upstreamAccount"
      ? payload.bindingKind
      : null;
  if (!bindingKind) return null;

  const groupName =
    typeof payload.groupName === "string" && payload.groupName.trim()
      ? payload.groupName.trim()
      : null;
  const upstreamAccountId = normalizeFiniteNumber(payload.upstreamAccountId) ?? null;
  const upstreamAccountName =
    typeof payload.upstreamAccountName === "string" && payload.upstreamAccountName.trim()
      ? payload.upstreamAccountName.trim()
      : null;

  if (bindingKind === "group" && !groupName) {
    return null;
  }
  if (
    bindingKind === "upstreamAccount" &&
    upstreamAccountId == null &&
    upstreamAccountName == null
  ) {
    return null;
  }

  return {
    bindingKind,
    groupName,
    upstreamAccountId,
    upstreamAccountName,
  };
}

export function normalizeConversationRequestPoint(raw: unknown): ConversationRequestPoint | null {
  const payload = (raw ?? {}) as Record<string, unknown>;
  const occurredAt = typeof payload.occurredAt === "string" ? payload.occurredAt : "";
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
