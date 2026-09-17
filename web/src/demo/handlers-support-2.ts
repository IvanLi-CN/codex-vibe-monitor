import type { ApiInvocationWorkflowDetailResponse, LongTermMetrics } from "../lib/api";
import {
  DEMO_INVOCATION_REQUEST_BODY_SIZE,
  DEMO_INVOCATION_REQUEST_BODY_TRANSMITTED_BYTES,
  DEMO_INVOCATION_RESPONSE_BODY_SIZE,
  type DemoAccount,
  demoAccounts,
  demoAttemptPhase,
  type demoDashboardActivityAccounts,
  demoForwardProxyNodes,
  demoModelRouteTimestamp,
  demoModelRoutingLiveTimeline,
  demoSummary,
  formatDemoAttemptId,
  invocations,
  simulatedAccountDisplayName,
} from "./handlers-support-1";
import { demoModel, demoNow } from "./model";

function demoDashboardActivitySummary(accounts: ReturnType<typeof demoDashboardActivityAccounts>) {
  const base = demoSummary();
  const sum = (read: (account: (typeof accounts)[number]) => number) =>
    accounts.reduce((total, account) => total + read(account), 0);
  const totalCount = sum((account) => account.requestCount);
  const successCount = sum((account) => account.successCount);
  const failureCount = sum((account) => account.failureCount);
  const totalTokens = sum((account) => account.totalTokens);
  const totalCost = Number(sum((account) => account.totalCost).toFixed(2));
  const cacheInputTokens = Math.round(sum((account) => account.totalTokens * account.cacheHitRate));
  return {
    ...base,
    totalCount,
    successCount,
    failureCount,
    totalTokens,
    totalCost,
    inProgressConversationCount: sum((account) => account.inProgressInvocationCount),
    token: {
      ...base.token,
      requestCount: totalCount,
      totalTokens,
      avgTokensPerRequest: totalCount === 0 ? 0 : Math.round(totalTokens / totalCount),
      cacheInputTokens,
      totalCost,
    },
    exception: {
      ...base.exception,
      failureCount,
      serviceFailureCount: failureCount,
      clientFailureCount: 0,
      clientAbortCount: 0,
      actionableFailureCount: failureCount,
    },
  };
}

function timeseries() {
  const empty = demoModel.snapshot.scene === "empty";
  const start = Date.parse(demoNow()) - 24 * 3_600_000;
  return {
    rangeStart: new Date(start).toISOString(),
    rangeEnd: demoNow(),
    bucketSeconds: 3600,
    effectiveBucket: "1h",
    availableBuckets: ["1m", "15m", "1h", "1d"],
    points: empty
      ? []
      : Array.from({ length: 24 }, (_, index) => ({
          bucketStart: new Date(start + index * 3_600_000).toISOString(),
          bucketEnd: new Date(start + (index + 1) * 3_600_000).toISOString(),
          totalCount: 920 + index * 61,
          successCount: 886 + index * 57,
          failureCount: 34 + (index % 3),
          totalTokens: 104_000_000 + index * 4_200_000,
          totalCost: 42.1 + index * 1.2,
          avgLatencyMs: 210 + index * 4,
        })),
  };
}

function parallelWork() {
  const start = Date.parse(demoNow()) - 24 * 3_600_000;
  const points =
    demoModel.snapshot.scene === "empty"
      ? []
      : Array.from({ length: 24 }, (_, index) => ({
          bucketStart: new Date(start + index * 3_600_000).toISOString(),
          bucketEnd: new Date(start + (index + 1) * 3_600_000).toISOString(),
          parallelCount: 2 + (index % 5),
        }));
  const current = {
    rangeStart: new Date(start).toISOString(),
    rangeEnd: demoNow(),
    bucketSeconds: 3600,
    completeBucketCount: points.length,
    activeBucketCount: points.length,
    activeMinuteCount: points.length,
    minCount: points.length ? 2 : null,
    maxCount: points.length ? 6 : null,
    avgCount: points.length ? 4 : null,
    effectiveTimeZone: "Asia/Shanghai",
    timeZoneFallback: false,
    points,
    conversations: [],
  };
  return { current, minute7d: current, hour30d: current, dayAll: current };
}

function promptCacheConversations() {
  const nowMs = Date.parse(demoNow());
  if (demoModel.snapshot.scene === "empty") {
    return {
      rangeStart: new Date(nowMs - 24 * 3_600_000).toISOString(),
      rangeEnd: demoNow(),
      selectionMode: "count",
      selectedLimit: 50,
      selectedActivityHours: null,
      selectedActivityMinutes: null,
      implicitFilter: { kind: null, filteredCount: 0 },
      totalMatched: 0,
      hasMore: false,
      nextCursor: null,
      conversations: [],
    };
  }
  const records = invocations();
  const accounts = new Map(demoAccounts().map((account) => [account.id, account]));
  const conversation = (
    promptCacheKey: string,
    encryptedOwnerAccountId: number | null,
    requestCount: number,
  ) => {
    const recent = records.filter((record) => record.promptCacheKey === promptCacheKey).slice(0, 4);
    const owner = encryptedOwnerAccountId == null ? null : accounts.get(encryptedOwnerAccountId);
    const upstreamAccounts = Array.from(
      new Set(
        recent.map((record) => record.upstreamAccountId).filter((id) => typeof id === "number"),
      ),
    ).map((id) => {
      const account = accounts.get(id);
      const accountRecords = recent.filter((record) => record.upstreamAccountId === id);
      return {
        upstreamAccountId: id,
        upstreamAccountName: account?.displayName ?? null,
        requestCount: accountRecords.length,
        totalTokens: accountRecords.reduce((total, record) => total + (record.totalTokens ?? 0), 0),
        totalCost: Number(
          accountRecords.reduce((total, record) => total + (record.cost ?? 0), 0).toFixed(4),
        ),
        lastActivityAt: accountRecords[0]?.occurredAt ?? demoNow(),
      };
    });
    return {
      promptCacheKey,
      hasEncryptedSessionOwner: owner != null,
      encryptedOwnerAccountId,
      encryptedOwnerAccountName: owner?.displayName ?? null,
      encryptedOwnerGroupName: owner?.groupName ?? null,
      requestCount,
      totalTokens: recent.reduce((total, record) => total + (record.totalTokens ?? 0), 0),
      totalCost: Number(recent.reduce((total, record) => total + (record.cost ?? 0), 0).toFixed(4)),
      createdAt: new Date(nowMs - requestCount * 90_000).toISOString(),
      lastActivityAt: recent[0]?.occurredAt ?? demoNow(),
      lastTerminalAt: recent.find((record) => record.status !== "running")?.occurredAt ?? null,
      lastInFlightAt: recent.find((record) => record.status === "running")?.occurredAt ?? null,
      upstreamAccounts,
      recentInvocations: recent,
      last24hRequests: Array.from({ length: 12 }, (_, index) => ({
        occurredAt: new Date(nowMs - (12 - index) * 90 * 60_000).toISOString(),
        status: index === 6 && promptCacheKey === "demo-research-batch" ? "http_429" : "success",
        isSuccess: !(index === 6 && promptCacheKey === "demo-research-batch"),
        outcome: index === 6 && promptCacheKey === "demo-research-batch" ? "failure" : "success",
      })),
    };
  };
  return {
    rangeStart: new Date(nowMs - 24 * 3_600_000).toISOString(),
    rangeEnd: demoNow(),
    selectionMode: "count",
    selectedLimit: 50,
    selectedActivityHours: null,
    selectedActivityMinutes: null,
    implicitFilter: { kind: null, filteredCount: 0 },
    totalMatched: 11,
    hasMore: false,
    nextCursor: null,
    conversations: [
      conversation("demo-conversation-a", 101, 38),
      conversation("demo-research-batch", 105, 27),
      conversation("demo-image-workflow", 104, 19),
      conversation("demo-indexing", null, 14),
      conversation("demo-conversation-b", null, 11),
      conversation("demo-conversation-c", 107, 9),
      conversation("demo-conversation-d", 114, 12),
      conversation("demo-edge-monitor", 111, 17),
      conversation("demo-batch-west", 112, 14),
      conversation("demo-mobile-e2e", 114, 8),
      conversation("demo-recovery", 115, 10),
    ],
  };
}

function forwardProxyLive() {
  if (demoModel.snapshot.scene === "empty") {
    return {
      rangeStart: "2026-07-10T00:00:00Z",
      rangeEnd: demoNow(),
      bucketSeconds: 3600,
      nodes: [],
    };
  }
  const nodes = demoForwardProxyNodes();
  return {
    rangeStart: "2026-07-10T00:00:00Z",
    rangeEnd: demoNow(),
    bucketSeconds: 3600,
    nodes: nodes.map((node, nodeIndex) => ({
      ...node,
      last24h: Array.from({ length: 8 }, (_, index) => ({
        bucketStart: `2026-07-10T${String(index + 1).padStart(2, "0")}:00:00Z`,
        bucketEnd: `2026-07-10T${String(index + 2).padStart(2, "0")}:00:00Z`,
        successCount: 11 + nodeIndex * 3 + index,
        failureCount:
          demoModel.snapshot.scene === "attention" && nodeIndex === 1 && index >= 6
            ? 3
            : index % 4 === 0
              ? 1
              : 0,
      })),
      weight24h: Array.from({ length: 8 }, (_, index) => ({
        bucketStart: `2026-07-10T${String(index + 1).padStart(2, "0")}:00:00Z`,
        bucketEnd: `2026-07-10T${String(index + 2).padStart(2, "0")}:00:00Z`,
        sampleCount: 11 + nodeIndex * 3 + index,
        minWeight: Number(((node.weight as number) - 0.12).toFixed(2)),
        maxWeight: Number(((node.weight as number) + 0.04).toFixed(2)),
        avgWeight: Number(((node.weight as number) - 0.02).toFixed(2)),
        lastWeight: node.weight,
      })),
    })),
  };
}

function demoAccountGroups(oauthItems: DemoAccount[]) {
  return [
    {
      groupName: "production",
      note: "Primary workload with priority capacity.",
      accountCount: oauthItems.filter((item) => item.groupName === "production").length,
      boundProxyKeys: ["demo-tokyo", "demo-singapore"],
      concurrencyLimit: 12,
      nodeShuntEnabled: true,
      singleAccountRotationEnabled: false,
      upstream429RetryEnabled: true,
      upstream429MaxRetries: 2,
      routingRule: {
        allowCutIn: true,
        allowCutOut: true,
        priorityTier: "primary",
        fastModeRewriteMode: "keep_original",
        concurrencyLimit: 12,
        upstream429RetryEnabled: true,
        upstream429MaxRetries: 2,
      },
    },
    {
      groupName: "research",
      note: "Long-running research and batch jobs.",
      accountCount: oauthItems.filter((item) => item.groupName === "research").length,
      boundProxyKeys: ["demo-tokyo", "demo-frankfurt"],
      concurrencyLimit: 8,
      nodeShuntEnabled: true,
      singleAccountRotationEnabled: true,
      upstream429RetryEnabled: true,
      upstream429MaxRetries: 3,
      routingRule: {
        allowCutIn: true,
        allowCutOut: true,
        priorityTier: "normal",
        fastModeRewriteMode: "keep_original",
        concurrencyLimit: 8,
        upstream429RetryEnabled: true,
        upstream429MaxRetries: 3,
      },
    },
    {
      groupName: "standby",
      note: "Fallback capacity retained for recovery routing.",
      accountCount: oauthItems.filter((item) => item.groupName === "standby").length,
      boundProxyKeys: ["demo-frankfurt", "demo-singapore"],
      concurrencyLimit: 4,
      nodeShuntEnabled: false,
      singleAccountRotationEnabled: false,
      upstream429RetryEnabled: true,
      upstream429MaxRetries: 1,
      routingRule: {
        allowCutIn: false,
        allowCutOut: true,
        priorityTier: "fallback",
        fastModeRewriteMode: "keep_original",
        concurrencyLimit: 4,
        upstream429RetryEnabled: true,
        upstream429MaxRetries: 1,
      },
    },
    {
      groupName: "edge",
      note: "Regional monitoring and mobile smoke checks.",
      accountCount: oauthItems.filter((item) => item.groupName === "edge").length,
      boundProxyKeys: ["demo-sydney", "demo-virginia"],
      concurrencyLimit: 6,
      nodeShuntEnabled: true,
      singleAccountRotationEnabled: false,
      upstream429RetryEnabled: true,
      upstream429MaxRetries: 2,
      routingRule: {
        allowCutIn: true,
        allowCutOut: true,
        priorityTier: "normal",
        fastModeRewriteMode: "fill_missing",
        concurrencyLimit: 6,
        upstream429RetryEnabled: true,
        upstream429MaxRetries: 2,
      },
    },
  ];
}

function accountList(kind?: string | null) {
  const allItems = demoModel.snapshot.scene === "empty" ? [] : demoAccounts();
  const items = kind ? allItems.filter((item) => item.kind === kind) : allItems;
  const oauthItems = items.filter((item) => item.kind === "oauth_codex");
  return {
    items,
    total: items.length,
    page: 1,
    pageSize: 50,
    groups: kind === "api_key_codex" ? [] : demoAccountGroups(oauthItems),
    forwardProxyNodes: demoForwardProxyNodes(),
    writesEnabled: true,
    availableModels: ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.4-mini"],
    hasUngroupedAccounts: oauthItems.some((item) => item.groupName == null),
    metrics: {
      total: items.length,
      oauth: items.filter((item) => item.kind === "oauth_codex").length,
      apiKey: items.filter((item) => item.kind === "api_key_codex").length,
      attention: items.filter((item) => item.healthStatus !== "normal").length,
    },
    routing: {
      writesEnabled: true,
      apiKeyConfigured: true,
      maskedApiKey: "cvm_pool••••••",
      maintenance: {
        primarySyncIntervalSecs: 300,
        secondarySyncIntervalSecs: 1800,
        priorityAvailableAccountCap: 100,
      },
      timeouts: {
        responsesFirstByteTimeoutSecs: 30,
        compactFirstByteTimeoutSecs: 45,
        imageFirstByteTimeoutSecs: 300,
        responsesStreamTimeoutSecs: 300,
        compactStreamTimeoutSecs: 420,
      },
    },
  };
}

function demoProjectionHealth() {
  return {
    terminal: {
      state: "healthy",
      cursorLag: 0,
      dirtyBucketCount: 0,
      pendingEventCount: 0,
    },
    longTerm: {
      state: "healthy",
      cursorLag: 0,
      dirtyBucketCount: 0,
      pendingEventCount: 0,
      lastFlushElapsedMs: 72,
      lastFlushAgeMs: 1_200,
    },
  };
}

function demoRuntimeDeliveryHealth(runtimeState: string) {
  return {
    activity: {
      materializationCount: 418,
      serializationCount: 418,
      payloadCloneCount: 0,
      frameBytesCount: 1_048_576,
      laggedCount: runtimeState === "degraded" ? 1 : 0,
      skippedCount: 0,
      businessPayloadCount: 418,
      jsonOverlayCount: 0,
    },
    summary: {
      materializationCount: 29,
      serializationCount: 29,
      payloadCloneCount: 0,
      frameBytesCount: 65_536,
      laggedCount: 0,
      skippedCount: 0,
      businessPayloadCount: 29,
      jsonOverlayCount: 0,
    },
    networkTimeseries: {
      materializationCount: 104,
      serializationCount: 104,
      payloadCloneCount: 0,
      frameBytesCount: 131_072,
      laggedCount: 0,
      skippedCount: 0,
      businessPayloadCount: 104,
      jsonOverlayCount: 0,
    },
    networkRecent: {
      materializationCount: 104,
      serializationCount: 104,
      payloadCloneCount: 0,
      frameBytesCount: 65_536,
      laggedCount: 0,
      skippedCount: 0,
      businessPayloadCount: 104,
      jsonOverlayCount: 0,
    },
  };
}

function demoRuntimePressureHealth(runtimeState: string, accountingError: boolean) {
  return {
    state: accountingError ? "accounting_error" : runtimeState,
    process: {
      rssBytes: 1_073_741_824,
      rssAnonBytes: 805_306_368,
      swapBytes: runtimeState === "degraded" ? 268_435_456 : 0,
      peakRssBytes: 1_342_177_280,
      threads: 18,
      managedBytes: 536_870_912,
      unattributedAnonBytes: 268_435_456,
      pressureLevel: runtimeState === "degraded" ? "elevated" : "normal",
    },
    allocator: { mallocArenaMax: "8" },
    writerAccounting: {
      state: accountingError ? "degraded" : "healthy",
      pendingDepth: accountingError ? 24 : 3,
      pendingBytes: accountingError ? 12_582_912 : 524_288,
      transferBytes: 67_108_864,
      retryCount: accountingError ? 4 : 0,
      invariantViolationCount: accountingError ? 1 : 0,
      degradedReason: accountingError ? "pending_bytes_underflow" : undefined,
    },
    retentionWriteHealth: {
      state:
        runtimeState === "deferred"
          ? "deferred"
          : runtimeState === "degraded"
            ? "degraded"
            : "healthy",
      operation: "invocation_detail_prune",
      admissionMode: runtimeState === "degraded" ? "fairness" : "normal",
      batchRows: 4,
      estimatedBytes: 16_384,
      prepareElapsedMs: 36,
      lockWaitMs: runtimeState === "degraded" ? 15_004 : 2,
      executeMs: runtimeState === "degraded" ? 251 : 47,
      commitMs: runtimeState === "degraded" ? 36 : 18,
      budgetBreachCount: runtimeState === "degraded" ? 1 : 0,
      deferReason: runtimeState === "deferred" ? "pressure_cooldown:30000ms" : undefined,
      starvationAgeMs: runtimeState === "degraded" ? 15_004 : undefined,
      p1WaiterCount: runtimeState === "degraded" ? 1 : 0,
      candidateRemainingHint: 1,
    },
    dashboardProjection: {
      mode: "auto",
      state: runtimeState === "degraded" ? "degraded" : "healthy",
      producerState: runtimeState === "deferred" ? "idle" : "running",
      activeSubscriberCount: 2,
      livePathDbReadCount: 0,
      buildCount: 418,
      revision: 771,
      snapshotOrigin: "runtime_projection",
      lastGoodAgeMs: 320,
      degradedReason: runtimeState === "degraded" ? "projection_stale" : undefined,
      lastDeferReason: runtimeState === "deferred" ? "writer_pressure" : undefined,
      sliceCounters: {
        current: {
          buildCount: 418,
          revisionCount: 771,
          cadenceMissCount: runtimeState === "degraded" ? 2 : 0,
        },
        network: { buildCount: 42, revisionCount: 104, cadenceMissCount: 0 },
        terminal: { buildCount: 8, revisionCount: 29, cadenceMissCount: 0 },
      },
    },
    delivery: demoRuntimeDeliveryHealth(runtimeState),
    eventBus: {
      state: runtimeState === "degraded" ? "degraded" : "healthy",
      publishedCount: 912,
      processedEventCount: 856,
      coalescedEventCount: 56,
      businessPayloadCloneCount: 0,
      topicWorkCount: 856,
      routerLaggedCount: runtimeState === "degraded" ? 2 : 0,
      routerGapCount: runtimeState === "degraded" ? 1 : 0,
      cursorRecoveryCount: runtimeState === "degraded" ? 1 : 0,
    },
    backfill: {
      state: runtimeState === "deferred" ? "deferred" : "healthy",
      wakeGeneration: 14,
      wakeCount: 14,
      dueDispatchCount: 28,
      noopSuppressedCount: 42,
      pressureDeferCount: runtimeState === "deferred" ? 3 : 0,
      failureCount: 0,
      wokenTaskCount: 0,
      scheduledTaskCount: 5,
      deferredTaskCount: runtimeState === "deferred" ? 1 : 0,
      failedTaskCount: 0,
    },
  };
}

function systemStatus() {
  const pressureState = demoModel.snapshot.scene.replace("runtime-pressure-", "");
  const runtimeState = pressureState === demoModel.snapshot.scene ? "healthy" : pressureState;
  const accountingError = runtimeState === "accounting-error";
  return {
    liveInvocationsCount: 128_076,
    successCount: 124_882,
    nonSuccessCount: 3_194,
    completedArchiveBatchesCount: 384,
    archivedBodies: { count: 118_420, bytes: 8_441_053_184 },
    rawBodies: { count: 1_482, bytes: 84_221_184 },
    requestRawBodies: { count: 812, bytes: 76_221_184 },
    responseRawBodies: { count: 670, bytes: 8_000_000 },
    databaseBytes: 618_659_840,
    otherFilesBytes: 142_344_192,
    rawMetricsHealth: { state: "ready", inventoryCursor: 128_076 },
    projectionHealth: demoProjectionHealth(),
    runtimePressureHealth: demoRuntimePressureHealth(runtimeState, accountingError),
    refreshedAt: demoNow(),
  };
}

function forwardProxyBindingNodes() {
  const nodes = demoForwardProxyNodes();
  return nodes.map((node, index) => ({
    key: node.key,
    aliasKeys: [],
    source: node.source,
    displayName: node.displayName,
    protocolLabel: node.endpointUrl?.toString().startsWith("http:") ? "HTTP" : "SOCKS5",
    egressIp: `198.51.100.${31 + index}`,
    egressIpCheckedAt: `2026-07-10T09:${String(12 + index).padStart(2, "0")}:00Z`,
    egressIpProvider: "demo resolver",
    egressIpError: null,
    egressIpErrorAt: null,
    penalized: node.penalized,
    selectable: true,
    last24h: Array.from({ length: 6 }, (_, bucketIndex) => ({
      bucketStart: `2026-07-10T${String(bucketIndex + 3).padStart(2, "0")}:00:00Z`,
      bucketEnd: `2026-07-10T${String(bucketIndex + 4).padStart(2, "0")}:00:00Z`,
      successCount: 14 + index * 4 + bucketIndex,
      failureCount:
        demoModel.snapshot.scene === "attention" && index === 1 && bucketIndex === 5 ? 3 : 0,
    })),
  }));
}

function accountRoutingEvents(accounts: DemoAccount[]) {
  return demoModelRoutingLiveTimeline()
    .filter((event) => event.kind === "event")
    .flatMap((event, index) => {
      const account = accounts.find((candidate) => candidate.id === event.accountId);
      if (!account) return [];
      const proxyKey = account.boundProxyKeys?.[0] ?? null;
      const stateAfter = event.modelRouteStateAfter ?? null;
      return [
        {
          id: 7100 + index,
          action: event.action ?? "model_route_state_changed",
          source: event.source ?? "call",
          result: stateAfter === "available" ? "success" : "failed",
          upstreamAccountId: account.id,
          accountDisplayName: account.displayName,
          accountGroupName: account.groupName,
          forwardProxyKey: proxyKey,
          forwardProxyDisplayName: account.currentForwardProxyDisplayName ?? null,
          forwardProxyEgressIp:
            proxyKey === "demo-tokyo"
              ? "198.51.100.31"
              : proxyKey === "demo-frankfurt"
                ? "198.51.100.32"
                : "198.51.100.33",
          reasonCode: event.reasonCode ?? null,
          reasonMessage: event.reasonCode
            ? "Route state changed after an upstream HTTP 502."
            : null,
          httpStatus: event.reasonCode === "upstream_http_5xx" ? 502 : null,
          model: event.model,
          modelRouteStateBefore: event.modelRouteStateBefore ?? null,
          modelRouteStateAfter: stateAfter,
          modelRoutePriorityBefore:
            event.modelRouteStateBefore === "available" ? "normal" : "demoted",
          modelRoutePriorityAfter:
            stateAfter === "cooling_down"
              ? "excluded"
              : stateAfter === "degraded"
                ? "demoted"
                : "normal",
          modelRouteFailureCount: event.modelRouteFailureCount ?? null,
          modelRouteCooldownUntil: event.modelRouteCooldownUntil ?? null,
          failureKind: stateAfter === "available" ? null : "model",
          invokeId: event.invokeId ?? null,
          stickyKey: null,
          occurredAt: event.occurredAt,
          createdAt: event.occurredAt,
        },
      ];
    });
}

function accountMaintenanceEvents(accounts: DemoAccount[]) {
  const maintenanceActions: Array<readonly [string, string, number]> = [
    ["sync_succeeded", "success", 30],
    ["usage_snapshot_updated", "success", 60],
    ["forward_proxy_assigned", "success", 120],
    ["routing_rule_updated", "success", 180],
    ["forward_proxy_health_checked", "success", 240],
    ["quota_window_reset_observed", "success", 300],
    ["sync_succeeded", "success", 360],
    ["usage_snapshot_updated", "success", 420],
    ["sync_succeeded", "success", 480],
    ["routing_rule_updated", "success", 540],
    ["forward_proxy_health_checked", "success", 600],
    ["usage_snapshot_updated", "success", 660],
  ];
  return maintenanceActions.map(([action, result, minutesAgo], index) => {
    const account = accounts[(index + 1) % accounts.length] ?? accounts[0];
    const proxyKey = account.boundProxyKeys?.[0] ?? null;
    const occurredAt = demoModelRouteTimestamp(minutesAgo);
    return {
      id: 7200 + index,
      action,
      source: "sync_maintenance",
      result,
      upstreamAccountId: account.id,
      accountDisplayName: account.displayName,
      accountGroupName: account.groupName,
      forwardProxyKey: proxyKey,
      forwardProxyDisplayName: account.currentForwardProxyDisplayName ?? null,
      forwardProxyEgressIp:
        proxyKey === "demo-tokyo"
          ? "198.51.100.31"
          : proxyKey === "demo-frankfurt"
            ? "198.51.100.32"
            : "198.51.100.33",
      reasonCode: null,
      reasonMessage: null,
      httpStatus: null,
      model: null,
      modelRouteStateBefore: null,
      modelRouteStateAfter: null,
      modelRoutePriorityBefore: null,
      modelRoutePriorityAfter: null,
      modelRouteFailureCount: null,
      modelRouteCooldownUntil: null,
      failureKind: null,
      invokeId: null,
      stickyKey: null,
      occurredAt,
      createdAt: occurredAt,
    };
  });
}

function accountEvents() {
  if (demoModel.snapshot.scene === "empty") return [];
  const accounts = demoAccounts();
  const routingEvents = accountRoutingEvents(accounts);
  const maintenanceEvents = accountMaintenanceEvents(accounts);
  return [...routingEvents, ...maintenanceEvents].sort(
    (left, right) => Date.parse(right.occurredAt) - Date.parse(left.occurredAt),
  );
}

const DEMO_SYSTEM_TASKS = [
  [
    1,
    "archive_rollup",
    "scheduler",
    "success",
    "Hourly invocation archive rollup completed.",
    "Rolled up 12 completed archive batches and compacted aggregate counters.",
    3,
    2,
    14_203,
  ],
  [
    2,
    "upstream_account_sync",
    "scheduler",
    "success",
    "Production pool quota snapshot completed.",
    "Synchronized 6 production accounts through the assigned relay nodes.",
    9,
    8,
    22_118,
  ],
  [
    3,
    "forward_proxy_subscription_refresh",
    "manual",
    "success",
    "Relay subscription refreshed.",
    "Five demo relay nodes were retained and their health probes completed.",
    18,
    18,
    8_447,
  ],
  [
    4,
    "raw_body_compression",
    "scheduler",
    "running",
    "Compressing retained invocation response bodies.",
    "The demo task is intentionally in progress to populate active task status.",
    32,
    null,
    1_920_000,
  ],
  [
    5,
    "pricing_catalog_refresh",
    "scheduler",
    "success",
    "Pricing catalog is current.",
    "Validated three configured models against the demo pricing catalog.",
    47,
    47,
    3_126,
  ],
  [
    6,
    "upstream_account_sync",
    "scheduler",
    "failed",
    "Standby account health check timed out.",
    "The recovery relay exceeded the simulated upstream timeout threshold; retry is queued.",
    66,
    65,
    31_022,
  ],
  [
    7,
    "historical_backfill",
    "manual",
    "skipped",
    "No historical gaps require backfill.",
    "The demo datastore already contains all required hourly buckets.",
    91,
    91,
    862,
  ],
  [
    8,
    "forward_proxy_latency_probe",
    "scheduler",
    "success",
    "All relay latency probes completed.",
    "Measured egress, OAuth upstream, and responses latency for five relay nodes.",
    113,
    112,
    42_907,
  ],
  [
    9,
    "prompt_cache_cleanup",
    "scheduler",
    "success",
    "Prompt cache retention sweep completed.",
    "Retained active conversations and removed no demo records.",
    146,
    145,
    9_441,
  ],
  [
    10,
    "usage_snapshot_reconciliation",
    "manual",
    "success",
    "Usage window reconciliation completed.",
    "Compared current primary and secondary windows across all demo accounts.",
    188,
    187,
    27_630,
  ],
] as const;

function systemTasks() {
  if (demoModel.snapshot.scene === "empty") return [];
  const at = (minutesAgo: number) =>
    new Date(Date.parse(demoNow()) - minutesAgo * 60_000).toISOString();
  return DEMO_SYSTEM_TASKS.map(
    ([
      id,
      taskKind,
      triggerKind,
      status,
      summary,
      detail,
      startedMinutesAgo,
      finishedMinutesAgo,
      durationMs,
    ]) => ({
      id,
      taskKind,
      triggerKind,
      status,
      summary,
      detail,
      startedAt: at(startedMinutesAgo),
      ...(finishedMinutesAgo == null ? {} : { finishedAt: at(finishedMinutesAgo) }),
      durationMs,
    }),
  );
}

function demoPoolAttemptBase(record: ReturnType<typeof invocations>[number], invokeId: string) {
  return {
    invokeId,
    occurredAt: record.occurredAt,
    endpoint: record.endpoint ?? "/v1/responses",
    model: record.model ?? null,
    requestModel: record.requestModel ?? record.model ?? null,
    responseModel: record.responseModel ?? null,
    stickyKey: record.stickyKey ?? null,
    requesterIp: record.requesterIp ?? null,
    createdAt: record.occurredAt,
  };
}

function poolAttempts(invokeId: string) {
  const record = invocations().find((item) => item.invokeId === invokeId);
  if (!record) return [];
  if (record.poolAttemptCount === 0) return [];
  const accountId = record.upstreamAccountId ?? 101;
  const fallback = accountId === 105 ? 106 : 102;
  const needsRetry = (record.poolAttemptCount ?? 1) > 1;
  const startedAt = record.occurredAt;
  const base = demoPoolAttemptBase(record, invokeId);
  const first = {
    ...base,
    id: record.id * 10 + 1,
    attemptId: record.id === 9002 ? "qPvNNAK8" : formatDemoAttemptId(record.id * 100 + 1),
    upstreamAccountId: accountId,
    upstreamAccountName: record.upstreamAccountName ?? null,
    upstreamRouteKey: "pool",
    proxyBindingKeySnapshot:
      record.proxyDisplayName === "Tokyo demo relay"
        ? "demo-tokyo"
        : record.proxyDisplayName === "Frankfurt recovery relay"
          ? "demo-frankfurt"
          : "demo-singapore",
    attemptIndex: 1,
    distinctAccountIndex: 1,
    sameAccountRetryIndex: 0,
    startedAt,
    finishedAt: needsRetry
      ? `2026-07-10T09:24:00Z`
      : record.status === "running"
        ? null
        : `2026-07-10T09:25:00Z`,
    status: needsRetry ? "failed" : (record.status ?? "success"),
    phase: demoAttemptPhase(record.status, record.firstTokenMs),
    httpStatus: needsRetry ? 429 : (record.downstreamStatusCode ?? 200),
    downstreamHttpStatus: needsRetry ? 429 : (record.downstreamStatusCode ?? 200),
    failureKind: needsRetry ? "rate_limited" : (record.failureKind ?? null),
    errorMessage: needsRetry ? "Simulated retry after rate limit." : (record.errorMessage ?? null),
    connectLatencyMs: record.tUpstreamConnectMs ?? 42,
    firstTokenMs: record.firstTokenMs ?? null,
    firstByteLatencyMs: record.tUpstreamTtfbMs ?? null,
    streamLatencyMs: record.tUpstreamStreamMs ?? null,
    upstreamRequestId: `up_demo_${record.id}_1`,
  };
  if (!needsRetry) return [first];
  return [
    {
      ...first,
      firstTokenMs: null,
      streamLatencyMs: null,
    },
    {
      ...first,
      id: record.id * 10 + 2,
      attemptId: record.id === 9002 ? "DEMO-SUCCESS-1" : formatDemoAttemptId(record.id * 100 + 2),
      upstreamAccountId: record.id === 9002 ? 102 : fallback,
      upstreamAccountName:
        record.id === 9002
          ? simulatedAccountDisplayName(102)
          : (demoAccounts().find((account) => account.id === fallback)?.displayName ?? null),
      attemptIndex: 2,
      distinctAccountIndex: 2,
      sameAccountRetryIndex: 0,
      routingSource: "freshAssignment",
      routingSelectionAudit:
        record.id === 9002
          ? {
              selectedAccountId: 102,
              selectedAccountName: simulatedAccountDisplayName(102),
              eligibleCandidateCount: 1,
              winnerReasonCode: "onlyEligibleCandidate",
              comparedAccountId: null,
              comparedAccountName: null,
              excludedCandidates: [
                {
                  accountId: 115,
                  accountName: simulatedAccountDisplayName(115),
                  reasonCode: "modelNotAllowed",
                },
              ],
            }
          : null,
      proxyBindingKeySnapshot: "demo-frankfurt",
      status: record.status === "http_502" ? "failed" : "success",
      httpStatus: record.status === "http_502" ? 502 : 200,
      downstreamHttpStatus: record.status === "http_502" ? 502 : 200,
      failureKind: record.status === "http_502" ? "upstream_timeout" : null,
      errorMessage: record.status === "http_502" ? "Simulated recovery relay timeout." : null,
      startedAt: "2026-07-10T09:24:02Z",
      finishedAt: "2026-07-10T09:24:05Z",
      firstTokenMs: record.firstTokenMs ?? null,
      streamLatencyMs: record.tUpstreamStreamMs ?? null,
      upstreamRequestId: `up_demo_${record.id}_2`,
    },
  ];
}

function upstreamAccountAttempts(accountId: number, search: URLSearchParams) {
  const type = search.get("type")?.trim() ?? "";
  const model = search.get("model")?.trim().toLowerCase() ?? "";
  const stickyKey = search.get("stickyKey")?.trim() ?? "";
  const page = Math.max(1, Number(search.get("page") ?? 1) || 1);
  const pageSize = Math.max(1, Number(search.get("pageSize") ?? 50) || 50);
  const endpointMatchesType = (endpoint: string) => {
    if (!type) return true;
    if (type === "image") return endpoint.startsWith("/v1/images/");
    if (type === "compact") return endpoint.includes("/compact");
    if (type === "remote_v2") return endpoint.includes("/remote");
    return !endpoint.startsWith("/v1/images/") && !endpoint.includes("/compact");
  };
  const items = invocations()
    .flatMap((record) => poolAttempts(record.invokeId))
    .filter((attempt) => attempt.upstreamAccountId === accountId)
    .filter((attempt) => endpointMatchesType(attempt.endpoint))
    .filter((attempt) => !model || attempt.requestModel?.toLowerCase().includes(model))
    .filter((attempt) => !stickyKey || attempt.stickyKey === stickyKey)
    .sort((left, right) => right.createdAt.localeCompare(left.createdAt));
  const start = (page - 1) * pageSize;
  const stickyKeyOptions = Array.from(
    new Set(items.map((attempt) => attempt.stickyKey).filter(Boolean)),
  ).map((value) => ({ value }));

  return {
    items: items.slice(start, start + pageSize),
    total: items.length,
    page,
    pageSize,
    stickyKeyOptions,
  };
}

function buildDemoInvocationContext(
  record: ReturnType<typeof invocations>[number],
  requestModel: string,
  responseModel: string,
) {
  const requestHeaders = {
    userAgent: "monitor-ui/1.0",
    xForwardedFor: record.requesterIp ?? "203.0.113.24",
    forwarded: `for=${record.requesterIp ?? "203.0.113.24"};proto=https`,
  };
  const requestCompression = {
    algorithm: "zstd",
    mode: "recompressed",
    logicalBodyBytes: DEMO_INVOCATION_REQUEST_BODY_SIZE,
    transmittedBodyBytes: DEMO_INVOCATION_REQUEST_BODY_TRANSMITTED_BYTES,
    savedBytes: DEMO_INVOCATION_REQUEST_BODY_SIZE - DEMO_INVOCATION_REQUEST_BODY_TRANSMITTED_BYTES,
    ratioPct: -63,
    approxUploadBytes: DEMO_INVOCATION_REQUEST_BODY_TRANSMITTED_BYTES,
    approxDownloadBytes: 135_800,
  };
  const routing = {
    routeMode: record.routeMode ?? "pool",
    proxyDisplayName: record.proxyDisplayName ?? "Tokyo demo relay",
    upstreamRouteKey: `route-${record.routeMode ?? "pool"}-primary`,
    proxyBindingKey: "fpb_demo_tokyo_primary",
    promptCacheKey: record.promptCacheKey ?? null,
    stickyKey: record.stickyKey ?? null,
  };
  return {
    requestHeaders,
    requestCompression,
    routing,
    routeRequest: {
      endpoint: record.endpoint ?? "/v1/responses",
      routeMode: record.routeMode ?? "pool",
      transport: record.transport ?? "http",
      requestModel,
      responseModel,
      requestedServiceTier: record.requestedServiceTier ?? "priority",
      reasoningEffort: record.reasoningEffort ?? "high",
      compactionRequestKind: "remote_v2",
      promptCacheKey: record.promptCacheKey ?? null,
      stickyKey: record.stickyKey ?? null,
      requesterIp: record.requesterIp ?? null,
      routing,
      headers: requestHeaders,
      bodyCapture: {
        availableAtInvocationLevel: false,
        size: DEMO_INVOCATION_REQUEST_BODY_SIZE,
        truncated: false,
        detailLevel: "full",
      },
    },
  };
}

function buildDemoResponseSummary(
  record: ReturnType<typeof invocations>[number],
  finalStatus: string | null | undefined,
) {
  return {
    status: finalStatus,
    phase: record.status === "running" ? "responding" : "completed",
    serviceTier: "default",
    billingServiceTier: record.billingServiceTier ?? "standard",
    responseContentEncoding: "identity",
    compactionResponseKind: "remote_v2",
    outputItems: 1,
    headers: {
      contentEncoding: "identity",
      upstreamRequestId: `req_demo_${record.id}`,
      cvmInvokeId: record.invokeId,
    },
    delivery: {
      forwardedChunkCount: 12,
      forwardedBytes: 138_649,
      usageObserved: true,
      downstreamClosePhase: null,
    },
    responseBodyCapture: {
      availableAtInvocationLevel: true,
      availableAtAttemptLevel: true,
      size: DEMO_INVOCATION_RESPONSE_BODY_SIZE,
      truncated: false,
      detailLevel: "full",
    },
    usage: { totalTokens: record.totalTokens ?? null },
  };
}

function buildDemoAttemptBlock(
  record: ReturnType<typeof invocations>[number],
  requestModel: string,
  responseModel: string,
  finalStatus: string | null | undefined,
  context: ReturnType<typeof buildDemoInvocationContext>,
  responseSummary: ReturnType<typeof buildDemoResponseSummary>,
) {
  const status = finalStatus ?? "failed";
  return {
    blockId: `attempt-${record.id}-1`,
    kind: "attempt" as const,
    occurredAt: record.occurredAt,
    title: "Attempt 1",
    subtitle: record.upstreamAccountName ?? record.proxyDisplayName ?? "Demo account",
    status,
    attempt: {
      synthetic: false,
      attemptId: "qPvNNAK8",
      occurredAt: record.occurredAt,
      endpoint: record.endpoint ?? "/v1/responses",
      stickyKey: record.stickyKey ?? null,
      upstreamAccountId: record.upstreamAccountId ?? null,
      upstreamAccountName: record.upstreamAccountName ?? null,
      requestModel,
      responseModel,
      upstreamRouteKey: context.routing.upstreamRouteKey,
      proxyBindingKeySnapshot: context.routing.proxyBindingKey,
      attemptIndex: 1,
      distinctAccountIndex: 1,
      sameAccountRetryIndex: 0,
      requesterIp: record.requesterIp ?? null,
      startedAt: record.occurredAt,
      finishedAt: record.tTotalMs == null ? null : record.occurredAt,
      status,
      phase: record.status === "running" ? "responding" : "completed",
      httpStatus: 200,
      downstreamHttpStatus: record.downstreamStatusCode ?? 200,
      failureKind: record.failureKind ?? null,
      errorMessage: record.errorMessage ?? null,
      connectLatencyMs: record.tUpstreamConnectMs ?? 184,
      firstTokenMs: record.firstTokenMs ?? null,
      firstByteLatencyMs: record.tUpstreamTtfbMs ?? 0,
      streamLatencyMs: record.tUpstreamStreamMs ?? null,
      upstreamRequestId: `req_demo_${record.id}`,
      requestSummary: {
        endpoint: record.endpoint ?? "/v1/responses",
        routeMode: record.routeMode ?? "pool",
        transport: record.transport ?? "http",
        requestModel,
        responseModel,
        stickyKey: record.stickyKey ?? null,
        requestedServiceTier: record.requestedServiceTier ?? "priority",
        reasoningEffort: record.reasoningEffort ?? "high",
        compactionRequestKind: "remote_v2",
        headers: context.requestHeaders,
        routing: context.routing,
        compression: context.requestCompression,
        bodyCapture: {
          availableAtInvocationLevel: false,
          size: DEMO_INVOCATION_REQUEST_BODY_SIZE,
          truncated: false,
          detailLevel: "full",
        },
      },
      responseSummary,
    },
  };
}

function buildDemoFailureBlock(record: ReturnType<typeof invocations>[number]) {
  return {
    blockId: `final-${record.id}`,
    kind: "systemFinalFailure" as const,
    occurredAt: record.occurredAt,
    title: "Final downstream response",
    subtitle: record.failureKind ?? "service_failure",
    status: record.status ?? "failed",
    detail: {
      downstreamStatusCode: record.downstreamStatusCode ?? 502,
      failureClass: record.failureClass,
      failureKind: record.failureKind ?? null,
      errorMessage: record.errorMessage ?? null,
    },
    responseBody: {
      available: true,
      bodyText: JSON.stringify(
        {
          error: record.errorMessage ?? "demo invocation failed",
          invoke_id: record.invokeId,
          code: record.failureKind ?? "service_failure",
          status: record.downstreamStatusCode ?? 502,
        },
        null,
        2,
      ),
    },
  };
}

function buildDemoInvocationHero(
  record: ReturnType<typeof invocations>[number],
  isNoCandidate: boolean,
  requestModel: string,
  responseModel: string,
  finalStatus: string | null | undefined,
) {
  return {
    recordId: record.id,
    invokeId: record.invokeId,
    promptCacheKey: record.promptCacheKey ?? null,
    routeMode: record.routeMode ?? null,
    endpoint: record.endpoint ?? null,
    requestModel,
    responseModel,
    finalStatus,
    failureClass: record.failureClass ?? null,
    downstreamStatusCode: record.downstreamStatusCode ?? null,
    upstreamAccountId: record.upstreamAccountId ?? null,
    upstreamAccountName: record.upstreamAccountName ?? null,
    totalDurationMs: record.tTotalMs ?? null,
    timelineAttemptCount: isNoCandidate ? 0 : 1,
    poolAttemptCount: record.poolAttemptCount ?? 1,
    poolRoutingNoCandidateAudit: isNoCandidate
      ? {
          terminalReasonCode: "modelConcurrencyLimit",
          candidateCount: 3,
          eligibleCandidateCount: 2,
          reservationConflictCount: 2,
          nextEligibleAt: null,
          excludedReasonCounts: { modelConcurrencyLimit: 2 },
          candidates: [
            {
              accountId: 106,
              accountName: simulatedAccountDisplayName(106),
              reasonCode: "modelConcurrencyLimit",
            },
            {
              accountId: 102,
              accountName: simulatedAccountDisplayName(102),
              reasonCode: "modelConcurrencyLimit",
            },
          ],
        }
      : null,
    totalTokens: record.totalTokens ?? null,
    cost: record.cost ?? null,
    occurredAt: record.occurredAt,
  };
}

function buildDemoInvocationWorkflowDetail(
  record: ReturnType<typeof invocations>[number],
): ApiInvocationWorkflowDetailResponse {
  const isNoCandidate =
    record.poolAttemptCount === 0 && record.failureKind === "pool_no_available_account";
  const finalStatus =
    record.status === "success"
      ? "completed"
      : record.status === "running"
        ? "running"
        : record.status;
  const requestModel = record.requestModel ?? record.model ?? "gpt-5.6-sol";
  const responseModel = record.responseModel ?? record.model ?? requestModel;
  const context = buildDemoInvocationContext(record, requestModel, responseModel);
  const attemptResponseSummary = buildDemoResponseSummary(record, finalStatus);

  const timeline: ApiInvocationWorkflowDetailResponse["timeline"] = [
    {
      blockId: `route-${record.id}`,
      kind: "routingDecision",
      occurredAt: record.occurredAt,
      title: "Route resolution",
      subtitle: `${requestModel} · ${record.endpoint ?? "/v1/responses"}`,
      status: record.routeMode ?? "pool",
      detail: {
        request: context.routeRequest,
        requestHeaders: context.requestHeaders,
        requestBody: {
          availableAtInvocationLevel: false,
          size: DEMO_INVOCATION_REQUEST_BODY_SIZE,
          truncated: false,
          detailLevel: "full",
        },
        routeMode: record.routeMode ?? "pool",
        poolAttemptCount: record.poolAttemptCount ?? 1,
      },
    },
    buildDemoAttemptBlock(
      record,
      requestModel,
      responseModel,
      finalStatus,
      context,
      attemptResponseSummary,
    ),
  ];

  if (isNoCandidate) {
    timeline.splice(1, 1);
  }

  if (record.failureClass && record.failureClass !== "none") {
    timeline.push(buildDemoFailureBlock(record));
  }

  return {
    hero: buildDemoInvocationHero(record, isNoCandidate, requestModel, responseModel, finalStatus),
    timeline,
    reconstructed: false,
    partial: false,
    partialReason: null,
  };
}

function recordsToSuggestionCounts<T>(
  records: T[],
  selector: (record: T) => string | null | undefined,
) {
  const counts = new Map<string, number>();
  for (const record of records) {
    const value = selector(record);
    if (!value) continue;
    counts.set(value, (counts.get(value) ?? 0) + 1);
  }
  return Array.from(counts.entries()).sort(
    ([, leftCount], [, rightCount]) => rightCount - leftCount,
  );
}

function demoLongTermMetrics(tokens: number, calls: number, cost: number): LongTermMetrics {
  return {
    calls,
    tokens,
    tokenSamples: calls,
    cost,
    costSamples: calls,
    usageTimeMs: calls * 820,
    usageTimeSamples: calls,
    wallTimeMs: calls * 460,
    wallTimeSamples: calls,
    outputSpeedTokensPerSecond: 42.5,
    outputSpeedSamples: calls,
    firstByteMs: 312,
    firstByteSamples: calls,
    responseMs: 1_420,
    responseSamples: calls,
  };
}

function demoLongTermOverview(range: string) {
  const empty = demoModel.snapshot.scene === "empty";
  const length = range === "7d" ? 7 : range === "30d" ? 30 : range === "180d" ? 180 : 365;
  const endDate = new Date(demoNow());
  const endDateUtc = new Date(
    Date.UTC(endDate.getUTCFullYear(), endDate.getUTCMonth(), endDate.getUTCDate()),
  );
  const startDateUtc = new Date(endDateUtc);
  startDateUtc.setUTCDate(startDateUtc.getUTCDate() - length + 1);
  const days = Array.from({ length }, (_, index) => {
    const date = new Date(startDateUtc);
    date.setUTCDate(startDateUtc.getUTCDate() + index);
    return date.toISOString().slice(0, 10);
  });
  const models = empty
    ? []
    : [
        ["model:gpt-5.6-sol|reasoning:high", "gpt-5.6-sol", 128_000, 211, 9.8],
        ["model:gpt-5.6-sol|reasoning:medium", "gpt-5.6-sol", 86_000, 142, 6.4],
        ["model:gpt-5.6-sol|reasoning:low", "gpt-5.6-sol", 72_000, 117, 5.3],
        ["model:gpt-5.6-terra|reasoning:high", "gpt-5.6-terra", 61_000, 98, 4.7],
        ["model:gpt-5.6-luna|reasoning:medium", "gpt-5.6-luna", 48_000, 76, 3.8],
        ["model:o3|reasoning:high", "o3", 39_000, 63, 2.9],
        ["model:claude-sonnet-4|reasoning:medium", "claude-sonnet-4", 31_000, 51, 2.4],
        [
          "model:very-long-model-name-for-legend-wrapping|reasoning:minimal",
          "very-long-model-name-for-legend-wrapping",
          22_000,
          38,
          1.7,
        ],
      ].map(([seriesKey, displayName, tokens, calls, cost]) => ({
        seriesKey,
        displayName,
        reasoningEffort: String(seriesKey).split("|reasoning:")[1] ?? null,
        ...demoLongTermMetrics(Number(tokens), Number(calls), Number(cost)),
      }));
  const upstreams = empty
    ? []
    : [
        ["account:1", "Primary API key", 154_000, 260, 11.2],
        ["account:2", "Research API key", 61_000, 108, 5.1],
        ["account:3", "Staging API key", 52_000, 81, 4.1],
        ["account:4", "Batch workloads", 43_000, 70, 3.5],
        ["account:5", "Partner integration", 35_000, 61, 2.8],
        ["account:6", "Archive importer", 28_000, 49, 2.2],
        ["account:7", "Automation service", 25_000, 43, 2],
        ["other", "Other upstream account with a deliberately long display name", 22_000, 44, 1.8],
      ].map(([seriesKey, displayName, tokens, calls, cost]) => ({
        seriesKey,
        displayName,
        ...demoLongTermMetrics(Number(tokens), Number(calls), Number(cost)),
      }));
  const totals = models.reduce(
    (total, item) => ({
      tokens: total.tokens + (item.tokens ?? 0),
      calls: total.calls + item.calls,
      cost: total.cost + (item.cost ?? 0),
    }),
    { tokens: 0, calls: 0, cost: 0 },
  );
  return {
    status: empty ? "empty" : "ready",
    statisticsStartDate: "2026-01-01",
    processedRows: empty ? 0 : totals.calls,
    totalRows: empty ? 0 : totals.calls,
    timezone: "Asia/Shanghai",
    range,
    global: empty
      ? demoLongTermMetrics(0, 0, 0)
      : demoLongTermMetrics(totals.tokens, totals.calls, totals.cost),
    daily: empty
      ? []
      : days.map((date, index) => ({
          date,
          ...demoLongTermMetrics(
            Math.round(totals.tokens / length) + index * 400,
            Math.round(totals.calls / length) + (index % 7),
            Number((totals.cost / length + index / 100).toFixed(2)),
          ),
        })),
    models,
    upstreams,
  };
}

function demoLongTermSeries(url: URL) {
  const overview = demoLongTermOverview(url.searchParams.get("range") ?? "7d") as ReturnType<
    typeof demoLongTermOverview
  >;
  const dimension = url.searchParams.get("dimension") ?? "model";
  const keys = url.searchParams.getAll("key").filter(Boolean);
  const source = dimension === "upstream" ? overview.upstreams : overview.models;
  const sparseDates = new Set(
    overview.daily
      .filter(
        (_, index) =>
          index === 0 ||
          index === Math.floor(overview.daily.length * 0.2) ||
          index >= Math.floor(overview.daily.length * 0.72),
      )
      .map((point) => point.date),
  );
  const pointCount = Math.max(1, sparseDates.size);
  return {
    status: overview.status,
    statisticsStartDate: overview.statisticsStartDate,
    processedRows: overview.processedRows,
    totalRows: overview.totalRows,
    timezone: overview.timezone,
    range: overview.range,
    dimension,
    series: source
      .filter((item) => keys.includes(String(item.seriesKey)))
      .map((item) => ({
        seriesKey: String(item.seriesKey),
        displayName: item.displayName,
        reasoningEffort: "reasoningEffort" in item ? item.reasoningEffort : null,
        points: overview.daily
          .filter((point) => sparseDates.has(point.date))
          .map((point) => ({
            ...point,
            tokens: Math.round((item.tokens ?? 0) / pointCount),
            cost: Number(((item.cost ?? 0) / pointCount).toFixed(2)),
            calls: Math.round(item.calls / pointCount),
          })),
      })),
  };
}

function filterDemoInvocations(url: URL) {
  let records = invocations();
  const model = url.searchParams.get("model");
  const status = url.searchParams.get("status");
  const endpoint = url.searchParams.get("endpoint");
  const invokeId = url.searchParams.get("invokeId") ?? url.searchParams.get("requestId");
  const attemptId = url.searchParams.get("attemptId");
  const upstreamAccountId = Number(url.searchParams.get("upstreamAccountId"));
  const promptCacheKey = url.searchParams.get("promptCacheKey");
  const stickyKey = url.searchParams.get("stickyKey");
  const keyword = url.searchParams.get("keyword")?.toLowerCase();
  if (model) records = records.filter((record) => record.model === model);
  if (status) records = records.filter((record) => record.status === status);
  if (endpoint) records = records.filter((record) => record.endpoint === endpoint);
  if (invokeId) records = records.filter((record) => record.invokeId === invokeId);
  if (attemptId) {
    records = records.filter((record) =>
      poolAttempts(record.invokeId).some((attempt) => attempt.attemptId === attemptId),
    );
  }
  if (Number.isFinite(upstreamAccountId) && upstreamAccountId > 0)
    records = records.filter((record) => record.upstreamAccountId === upstreamAccountId);
  if (promptCacheKey)
    records = records.filter((record) => record.promptCacheKey === promptCacheKey);
  if (stickyKey) records = records.filter((record) => record.stickyKey === stickyKey);
  if (keyword)
    records = records.filter((record) => JSON.stringify(record).toLowerCase().includes(keyword));
  return records;
}

export {
  accountEvents,
  accountList,
  buildDemoInvocationWorkflowDetail,
  demoDashboardActivitySummary,
  demoLongTermMetrics,
  demoLongTermOverview,
  demoLongTermSeries,
  filterDemoInvocations,
  forwardProxyBindingNodes,
  forwardProxyLive,
  parallelWork,
  poolAttempts,
  promptCacheConversations,
  recordsToSuggestionCounts,
  systemStatus,
  systemTasks,
  timeseries,
  upstreamAccountAttempts,
};
