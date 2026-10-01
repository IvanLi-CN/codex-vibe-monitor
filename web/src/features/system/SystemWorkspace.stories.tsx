import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useLayoutEffect, useRef } from "react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { expect, userEvent, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type {
  ExternalApiKeySummary,
  ManagedTask,
  ManagedTaskDetail,
  ModelsDevSyncPreview,
  PricingEntry,
  SettingsPayload,
  SystemStatusResponse,
  SystemTaskRunsResponse,
} from "../../lib/api";
import type { RuntimePressureDashboardHotTopicHealth } from "../../lib/api/core-foundation";
import SystemLayout from "../../pages/system/SystemLayout";
import SystemModelsPage from "../../pages/system/SystemModelsPage";
import SystemProxyPage from "../../pages/system/SystemProxyPage";
import SystemSettingsPage from "../../pages/system/SystemSettingsPage";
import SystemStatusPage from "../../pages/system/SystemStatusPage";
import SystemTaskDetailPage from "../../pages/system/SystemTaskDetailPage";
import SystemTasksPage from "../../pages/system/SystemTasksPage";
import {
  FullPageStorySurface,
  StorybookPageEnvironment,
  type StorybookRequestHandler,
} from "../../storybook/storybookPageHelpers";

function hotTopic(
  state = "healthy",
  overrides: Partial<RuntimePressureDashboardHotTopicHealth> = {},
): RuntimePressureDashboardHotTopicHealth {
  return {
    topicClass: "hot_projection",
    state,
    activeSubscriberCount: 2,
    builderCount: 418,
    genericFallbackBuildCount: 0,
    livePathDbReadCount: 0,
    materializationCount: 418,
    serializationCount: 418,
    payloadCloneCount: 0,
    frameReused: 352,
    cadenceMissCount: 0,
    reconnectChurnCount: 0,
    ...overrides,
  };
}

const STORYBOOK_SYSTEM_STATUS: SystemStatusResponse = {
  liveInvocationsCount: 128_076,
  successCount: 124_882,
  nonSuccessCount: 3_194,
  completedArchiveBatchesCount: 384,
  archivedBodies: { count: 118_420, bytes: 8_441_053_184 },
  rawBodies: { count: 1_482, bytes: 84_221_184_000 },
  requestRawBodies: { count: 812, bytes: 76_221_184_000 },
  responseRawBodies: { count: 670, bytes: 8_000_000_000 },
  databaseBytes: 618_659_840,
  otherFilesBytes: 142_344_192,
  rawMetricsHealth: {
    state: "ready",
    inventoryCursor: 128_076,
    physicalCoverage: "partial",
  },
  projectionHealth: {
    terminal: {
      state: "healthy",
      cursorLag: 0,
      dirtyBucketCount: 0,
      pendingEventCount: 3,
      lastFlushAgeMs: 320,
    },
    longTerm: {
      state: "healthy",
      cursorLag: 0,
      dirtyBucketCount: 0,
      pendingEventCount: 0,
      lastFlushElapsedMs: 84,
      lastFlushAgeMs: 1_812,
    },
  },
  runtimePressureHealth: {
    state: "healthy",
    process: {
      rssBytes: 1_073_741_824,
      rssAnonBytes: 805_306_368,
      swapBytes: 0,
      peakRssBytes: 1_342_177_280,
      threads: 18,
      managedBytes: 536_870_912,
      unattributedAnonBytes: 268_435_456,
      pressureLevel: "normal",
    },
    allocator: { mallocArenaMax: "8" },
    writerAccounting: {
      state: "healthy",
      pendingDepth: 3,
      pendingBytes: 524_288,
      transferBytes: 67_108_864,
      retryCount: 0,
      invariantViolationCount: 0,
    },
    retentionWriteHealth: {
      state: "healthy",
      operation: "invocation_detail_prune",
      admissionMode: "normal",
      batchRows: 4,
      estimatedBytes: 16_384,
      prepareElapsedMs: 36,
      lockWaitMs: 2,
      executeMs: 47,
      commitMs: 18,
      rawReferenceCheckMs: 1,
      budgetBreachCount: 0,
      p1WaiterCount: 0,
      candidateRemainingHint: 1,
    },
    retentionRecovery: {
      state: "healthy",
      stage: "orphan_sweep",
      preparedCount: 0,
      quarantinedCount: 0,
      expiredBacklogCount: 0,
      lastProgressAt: "2026-06-22T08:00:00Z",
    },
    rawOrphanSweep: {
      state: "idle",
      inspectedEntries: 128,
      referencedSkipped: 19,
      quarantined: 4,
      removed: 2,
      removedBytes: 131_072,
      lastProgressAt: "2026-06-22T08:01:00Z",
      nextRetryAt: "2026-06-22T08:01:01Z",
      lastSettledPass: {
        settledAt: "2026-06-22T08:01:00Z",
        complete: true,
        inspectedEntries: 128,
        referencedSkipped: 19,
        quarantined: 4,
        removed: 2,
        removedBytes: 131_072,
      },
      lastNonzeroRemoval: {
        removedAt: "2026-06-22T08:00:58Z",
        removed: 2,
        removedBytes: 131_072,
      },
    },
    rawCapture: {
      state: "capturing",
      inventoryState: "ready",
      rawBytes: 8_589_934_592,
      availableBytes: 64_424_509_184,
      reservedBytes: 0,
      rawCloseBytes: 17_179_869_184,
      rawResumeBytes: 12_884_901_888,
      availableCloseBytes: 21_474_836_480,
      availableResumeBytes: 32_212_254_720,
      expiredBacklogCount: 0,
      backlogNonGrowing: true,
    },
    dashboardProjection: {
      mode: "auto",
      state: "healthy",
      producerState: "running",
      activeSubscriberCount: 2,
      livePathDbReadCount: 0,
      buildCount: 418,
      revision: 771,
      snapshotOrigin: "runtime_projection",
      lastGoodAgeMs: 320,
      sliceCounters: {
        current: { buildCount: 418, revisionCount: 771, cadenceMissCount: 0 },
        network: { buildCount: 42, revisionCount: 104, cadenceMissCount: 0 },
        terminal: { buildCount: 8, revisionCount: 29, cadenceMissCount: 0 },
      },
    },
    delivery: {
      activity: {
        materializationCount: 418,
        serializationCount: 418,
        payloadCloneCount: 0,
        frameBytesCount: 1_048_576,
        laggedCount: 0,
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
    },
    dashboardHotTopics: {
      state: "healthy",
      activity: hotTopic(),
      summary: hotTopic(),
      networkTimeseries: hotTopic(),
      networkRecent: hotTopic(),
      workingConversations: hotTopic(),
      parallelWork: hotTopic(),
      timeseries: hotTopic(),
    },
    eventBus: {
      state: "healthy",
      publishedCount: 912,
      processedEventCount: 856,
      coalescedEventCount: 56,
      businessPayloadCloneCount: 0,
      topicWorkCount: 856,
      routerLaggedCount: 0,
      routerGapCount: 0,
      cursorRecoveryCount: 0,
    },
    backfill: {
      state: "healthy",
      wakeGeneration: 14,
      wakeCount: 14,
      dueDispatchCount: 28,
      noopSuppressedCount: 42,
      pressureDeferCount: 0,
      failureCount: 0,
      wokenTaskCount: 0,
      scheduledTaskCount: 5,
      deferredTaskCount: 0,
      failedTaskCount: 0,
    },
  },
  refreshedAt: "2026-06-22T09:28:00Z",
};

const STORYBOOK_SYSTEM_TASK_ITEMS: SystemTaskRunsResponse["items"] = [
  {
    id: 41,
    taskKind: "forward_proxy_subscription_refresh",
    triggerKind: "interval",
    status: "success",
    summary: "refreshed 3 subscriptions and added 18 nodes",
    detail: "Completed in background maintenance loop without manual intervention.",
    startedAt: "2026-06-22T09:20:00Z",
    finishedAt: "2026-06-22T09:20:02Z",
    durationMs: 2014,
  },
  {
    id: 40,
    taskKind: "retention_archive",
    triggerKind: "interval",
    status: "success",
    summary: "compressed=27 archived_invocations=860 pruned_details=860 orphan_raw_removed=4",
    detail: "Archive maintenance rotated raw payloads and trimmed invocation details.",
    startedAt: "2026-06-22T09:00:00Z",
    finishedAt: "2026-06-22T09:00:11Z",
    durationMs: 11182,
  },
  {
    id: 39,
    taskKind: "startup_backfill",
    triggerKind: "startup",
    status: "success",
    summary: "replayed retained raw captures into usage rollups",
    detail: "Startup backfill completed before the main scheduler resumed normal polling.",
    startedAt: "2026-06-22T08:58:00Z",
    finishedAt: "2026-06-22T08:58:12Z",
    durationMs: 12103,
  },
  {
    id: 38,
    taskKind: "scheduler_poll",
    triggerKind: "interval",
    status: "failed",
    summary: "pool poll timed out while upstream was degraded",
    detail: "The scheduler retried after a handshake timeout and recovered on the next interval.",
    startedAt: "2026-06-22T08:40:00Z",
    finishedAt: "2026-06-22T08:40:10Z",
    durationMs: 10000,
  },
];

for (let index = 0; index < 21; index += 1) {
  const id = 37 - index;
  STORYBOOK_SYSTEM_TASK_ITEMS.push({
    id,
    taskKind:
      index % 4 === 0
        ? "scheduler_poll"
        : index % 4 === 1
          ? "retention_archive"
          : index % 4 === 2
            ? "startup_backfill"
            : "forward_proxy_subscription_refresh",
    triggerKind: index % 3 === 0 ? "interval" : "startup",
    status: index % 5 === 0 ? "failed" : "success",
    summary: `storybook task run ${id} summary`,
    detail: `Synthetic task run ${id} keeps pagination states visible in the system workspace story.`,
    startedAt: `2026-06-21T${String(23 - (index % 10)).padStart(2, "0")}:00:00Z`,
    finishedAt: `2026-06-21T${String(23 - (index % 10)).padStart(2, "0")}:00:05Z`,
    durationMs: 5000 + index * 73,
  });
}

function filterStorybookSystemTasks(url: URL): SystemTaskRunsResponse {
  const taskKind = url.searchParams.get("taskKind")?.trim();
  const status = url.searchParams.get("status")?.trim();
  const startedAtFrom = url.searchParams.get("startedAtFrom")?.trim();
  const startedAtTo = url.searchParams.get("startedAtTo")?.trim();
  const startedAtFromMs = startedAtFrom ? Date.parse(startedAtFrom) : Number.NaN;
  const startedAtToMs = startedAtTo ? Date.parse(startedAtTo) : Number.NaN;
  const page = Number(url.searchParams.get("page") ?? "1");
  const pageSize = Number(
    url.searchParams.get("pageSize") ?? url.searchParams.get("limit") ?? "20",
  );
  const filtered = STORYBOOK_SYSTEM_TASK_ITEMS.filter((item) => {
    const startedAtMs = Date.parse(item.startedAt);
    if (taskKind && item.taskKind !== taskKind) return false;
    if (status && item.status !== status) return false;
    if (startedAtFrom && Number.isFinite(startedAtFromMs) && startedAtMs < startedAtFromMs)
      return false;
    if (startedAtTo && Number.isFinite(startedAtToMs) && startedAtMs > startedAtToMs) return false;
    return true;
  });
  const safePage = Math.max(1, page);
  const safePageSize = Math.min(100, Math.max(1, pageSize));
  const start = (safePage - 1) * safePageSize;
  return {
    total: filtered.length,
    page: safePage,
    pageSize: safePageSize,
    items: filtered.slice(start, start + safePageSize),
  };
}

const STORYBOOK_MANAGED_TASKS: ManagedTask[] = [
  {
    taskKey: "retention_archive",
    title: "数据保留与归档",
    description: "按保留策略归档并清理历史数据",
    triggerMode: "interval",
    enabled: true,
    intervalSecs: 3600,
    cronExpr: null,
    nextTriggerAt: "2026-10-01T01:00:00Z",
    isManual: false,
    effectiveSchedule: {
      source: "default",
      intervalSecs: 3600,
      cronExpr: null,
      nextTriggerAt: "2026-10-01T01:00:00Z",
    },
  },
  {
    taskKey: "forward_proxy_subscription_refresh",
    title: "正向代理订阅刷新",
    description: "刷新代理订阅并更新代理节点状态",
    triggerMode: "event",
    enabled: true,
    intervalSecs: null,
    cronExpr: null,
    nextTriggerAt: null,
    isManual: false,
  },
];

const STORYBOOK_RETENTION_TASK_DETAIL: ManagedTaskDetail = {
  task: STORYBOOK_MANAGED_TASKS[0],
  progress: {
    total: 128_000,
    completed: 96_000,
    phase: "archive",
    checkpoint: "invocation_id=983040",
    etaSeconds: null,
    updatedAt: "2026-10-01T00:12:30Z",
    freshness: "fresh",
    unit: "invocations",
    sourceScope: "id <= 1000000",
    lastProgressAt: "2026-10-01T00:12:30Z",
    waitReason: "prompt_cache_materialization_pending",
    nextRetryAt: "2026-10-01T00:13:00Z",
    stages: [
      { name: "archive", status: "running", completed: 96_000, total: 128_000 },
      {
        name: "prompt_cache",
        status: "pending",
        completed: 0,
        total: 3,
        waitReason: "prompt_cache_materialization_pending",
      },
      { name: "orphan_cleanup", status: "queued", completed: 0, total: 12 },
    ],
  },
  recentRuns: [
    {
      id: 104,
      triggerKind: "manual",
      startedAt: "2026-10-01T00:10:00Z",
      finishedAt: "2026-10-01T00:11:04Z",
      durationMs: 64_000,
      status: "success",
      summary: "归档阶段完成，统计刷新仍在后台继续。",
      processedCount: 96_000,
      updatedCount: 96_000,
      completion: "partial",
      coreCompletion: "completed",
      details: {
        completion: "partial",
        coreCompletion: "completed",
        budgetMs: 60_000,
        elapsedMs: 64_000,
        settlementMs: 4_000,
        budgetExhausted: true,
        waitReason: "prompt_cache_materialization_pending",
        promptCacheStats: {
          state: "unavailable",
          pending: 3,
          reason: "prompt_cache_materialization_pending",
        },
      },
    },
  ],
  performance: {
    runCount: 24,
    successCount: 22,
    failureCount: 1,
    averageDurationMs: 48_500,
    latestDurationMs: 64_000,
    observedAt: "2026-10-01T00:12:30Z",
    coverage: 0.92,
  },
};

const STORYBOOK_SETTINGS: SettingsPayload = {
  proxy: {
    hijackEnabled: true,
    mergeUpstreamEnabled: true,
    fastModeRewriteMode: "disabled",
    upstream429MaxRetries: 3,
    websocketEnabled: true,
    upstreamWebsocketDefaultEnabled: true,
    requestBodyLoggingEnabled: true,
    responseBodyLoggingEnabled: true,
    encryptedSessionOwnerRoutingEnabled: false,
    defaultHijackEnabled: false,
    models: ["gpt-5.5", "gpt-5.5-pro", "gpt-5.4"],
    enabledModels: ["gpt-5.5", "gpt-5.5-pro"],
  },
  forwardProxy: {
    proxyUrls: ["http://tokyo-edge.internal:8080", "socks5://singapore-edge.internal:1080"],
    subscriptionUrls: ["https://example.com/subscription.base64"],
    subscriptionUpdateIntervalSecs: 3600,
    nodes: [
      {
        key: "tokyo-edge",
        source: "manual",
        displayName: "tokyo-edge.internal:8080",
        endpointUrl: "http://tokyo-edge.internal:8080",
        weight: 0.92,
        penalized: false,
        stats: {
          oneMinute: { attempts: 14, successRate: 0.93, avgLatencyMs: 182 },
          fifteenMinutes: { attempts: 168, successRate: 0.94, avgLatencyMs: 190 },
          oneHour: { attempts: 672, successRate: 0.94, avgLatencyMs: 204 },
          oneDay: { attempts: 1612, successRate: 0.95, avgLatencyMs: 216 },
          sevenDays: { attempts: 9120, successRate: 0.95, avgLatencyMs: 228 },
        },
      },
      {
        key: "singapore-edge",
        source: "manual",
        displayName: "singapore-edge.internal:1080",
        endpointUrl: "socks5://singapore-edge.internal:1080",
        weight: 0.71,
        penalized: false,
        stats: {
          oneMinute: { attempts: 10, successRate: 0.88, avgLatencyMs: 236 },
          fifteenMinutes: { attempts: 134, successRate: 0.9, avgLatencyMs: 242 },
          oneHour: { attempts: 588, successRate: 0.91, avgLatencyMs: 255 },
          oneDay: { attempts: 1450, successRate: 0.91, avgLatencyMs: 269 },
          sevenDays: { attempts: 8220, successRate: 0.92, avgLatencyMs: 278 },
        },
      },
    ],
  },
  pricing: {
    catalogVersion: "storybook-system-2026-06",
    entries: [
      {
        model: "gpt-5.6-sol",
        inputPer1m: 5,
        outputPer1m: 30,
        cacheInputPer1m: 0.5,
        cacheReadPer1m: 0.5,
        cacheWritePer1m: 6.25,
        reasoningPer1m: null,
        source: "official",
      },
      {
        model: "gpt-5.6-terra",
        inputPer1m: 2.5,
        outputPer1m: 15,
        cacheInputPer1m: null,
        cacheReadPer1m: 0.25,
        cacheWritePer1m: 3.125,
        reasoningPer1m: null,
        source: "official",
      },
    ],
  },
};

const STORYBOOK_MODELS_SETTINGS: SettingsPayload = {
  ...STORYBOOK_SETTINGS,
  proxy: {
    ...STORYBOOK_SETTINGS.proxy,
    models: ["gpt-6-sol", "claude-sonnet-4", "gemini-2.5-pro", "local-unpriced"],
    enabledModels: ["gpt-6-sol", "gemini-2.5-pro"],
  },
  pricing: {
    catalogVersion: "storybook-models-2026-09",
    entries: [
      {
        model: "gpt-6-sol",
        inputPer1m: 2,
        outputPer1m: 10,
        cacheInputPer1m: 0.2,
        cacheReadPer1m: 0.2,
        cacheWritePer1m: 2.5,
        reasoningPer1m: null,
        source: "official",
      },
      {
        model: "claude-sonnet-4",
        inputPer1m: 3,
        outputPer1m: 15,
        cacheInputPer1m: null,
        cacheReadPer1m: 0.3,
        cacheWritePer1m: null,
        reasoningPer1m: null,
        source: "custom",
      },
      {
        model: "gemini-2.5-pro",
        inputPer1m: 1.25,
        outputPer1m: 10,
        cacheInputPer1m: null,
        cacheReadPer1m: null,
        cacheWritePer1m: null,
        reasoningPer1m: null,
        source: "official",
      },
    ],
  },
};

const STORYBOOK_MODELS_DEV_PREVIEW: ModelsDevSyncPreview = {
  fetchedAt: "2026-09-30T00:00:00Z",
  providerCount: 3,
  candidateCount: 4,
  providers: [
    { id: "openai", name: "OpenAI", docUrl: "https://platform.openai.com/docs" },
    { id: "openrouter", name: "OpenRouter", docUrl: "https://openrouter.ai/docs" },
    { id: "deepseek", name: "DeepSeek", docUrl: "https://api-docs.deepseek.com/" },
  ],
  candidates: [
    {
      model: "gpt-6-sol",
      name: "GPT-6 Sol",
      providerId: "openai",
      providerName: "OpenAI",
      docUrl: "https://platform.openai.com/docs",
      inputPer1m: 2.25,
      outputPer1m: 11,
      cacheReadPer1m: 0.22,
      cacheWritePer1m: 2.8,
      reasoningPer1m: null,
      unsupportedDimensions: [],
      importable: true,
    },
    {
      model: "gpt-6-sol",
      name: "GPT-6 Sol",
      providerId: "openrouter",
      providerName: "OpenRouter",
      docUrl: "https://openrouter.ai/docs",
      inputPer1m: 2.5,
      outputPer1m: 12,
      cacheReadPer1m: null,
      cacheWritePer1m: null,
      reasoningPer1m: null,
      unsupportedDimensions: ["image"],
      importable: true,
    },
    {
      model: "claude-sonnet-4",
      name: "Claude Sonnet 4",
      providerId: "openrouter",
      providerName: "OpenRouter",
      docUrl: "https://openrouter.ai/docs",
      inputPer1m: 3.5,
      outputPer1m: 17,
      cacheReadPer1m: 0.35,
      cacheWritePer1m: null,
      reasoningPer1m: null,
      unsupportedDimensions: [],
      importable: true,
    },
    {
      model: "deepseek-v3.2",
      name: "DeepSeek V3.2",
      providerId: "deepseek",
      providerName: "DeepSeek",
      docUrl: "https://api-docs.deepseek.com/",
      inputPer1m: 0.28,
      outputPer1m: 0.42,
      cacheReadPer1m: 0.028,
      cacheWritePer1m: null,
      reasoningPer1m: 0.42,
      unsupportedDimensions: ["batch", "image"],
      importable: true,
    },
  ],
};

const STORYBOOK_EXTERNAL_API_KEYS: ExternalApiKeySummary[] = [
  {
    id: 11,
    name: "Partner sync",
    status: "active",
    prefix: "cvm_ext_sys",
    lastUsedAt: "2026-06-22T08:22:00Z",
    createdAt: "2026-06-21T10:00:00Z",
    updatedAt: "2026-06-22T08:22:00Z",
  },
];

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function buildSystemWorkspaceRequestHandler(
  statusOverride?: SystemStatusResponse,
  settingsOverride?: SettingsPayload,
  failFirstModelsPreview = false,
): StorybookRequestHandler {
  const settings = clone(settingsOverride ?? STORYBOOK_SETTINGS);
  const retentionTaskDetail = clone(STORYBOOK_RETENTION_TASK_DETAIL);
  let previewFailuresRemaining = failFirstModelsPreview ? 1 : 0;
  return async ({ url, init }) => {
    const method = (init?.method ?? "GET").toUpperCase();
    const jsonResponse = (payload: unknown, status = 200) =>
      new Response(JSON.stringify(payload), {
        status,
        headers: { "Content-Type": "application/json" },
      });
    const parseBody = <T,>(fallback: T): T => {
      if (typeof init?.body !== "string" || !init.body) return fallback;
      try {
        return JSON.parse(init.body) as T;
      } catch {
        return fallback;
      }
    };

    if (url.pathname === "/api/system/status" && method === "GET") {
      return jsonResponse(clone(statusOverride ?? STORYBOOK_SYSTEM_STATUS));
    }

    if (url.pathname === "/api/system/managed-tasks" && method === "GET") {
      return jsonResponse(clone(STORYBOOK_MANAGED_TASKS));
    }

    if (url.pathname === "/api/system/managed-tasks/retention_archive") {
      if (method === "PATCH") {
        const body = parseBody<{
          enabled?: boolean;
          intervalSecs?: number | null;
          cronExpr?: string | null;
        }>({});
        retentionTaskDetail.task = {
          ...retentionTaskDetail.task,
          enabled: body.enabled ?? retentionTaskDetail.task.enabled,
          intervalSecs:
            body.intervalSecs === undefined
              ? retentionTaskDetail.task.intervalSecs
              : body.intervalSecs,
          cronExpr: body.cronExpr === undefined ? retentionTaskDetail.task.cronExpr : body.cronExpr,
          effectiveSchedule: {
            source: "override",
            intervalSecs: body.intervalSecs ?? retentionTaskDetail.task.intervalSecs ?? null,
            cronExpr: body.cronExpr ?? retentionTaskDetail.task.cronExpr ?? null,
            nextTriggerAt: retentionTaskDetail.task.nextTriggerAt ?? null,
          },
        };
      }
      return jsonResponse(clone(retentionTaskDetail));
    }

    if (url.pathname === "/api/system/managed-tasks/retention_archive/run" && method === "POST") {
      return jsonResponse(clone(retentionTaskDetail));
    }

    if (url.pathname === "/api/system/tasks" && method === "GET") {
      return jsonResponse(clone(filterStorybookSystemTasks(url)));
    }

    if (url.pathname === "/api/stats/invocation-timeline" && method === "GET") {
      return jsonResponse({
        rangeStart: "2026-01-01T00:00:00.000Z",
        rangeEnd: "2026-01-02T00:00:00.000Z",
        asOf: "2026-01-02T00:00:00.000Z",
        total: 0,
        hasMore: false,
        nextCursor: null,
        records: [],
      });
    }

    if (url.pathname === "/api/settings" && method === "GET") {
      return jsonResponse(clone(settings));
    }

    if (url.pathname === "/api/settings/external-api-keys" && method === "GET") {
      return jsonResponse({ items: clone(STORYBOOK_EXTERNAL_API_KEYS) });
    }

    if (url.pathname === "/api/settings/models/sync/preview" && method === "POST") {
      if (previewFailuresRemaining > 0) {
        previewFailuresRemaining -= 1;
        return jsonResponse({ message: "models.dev is temporarily unavailable" }, 502);
      }
      return jsonResponse(clone(STORYBOOK_MODELS_DEV_PREVIEW));
    }

    if (url.pathname === "/api/settings/models/sync/apply" && method === "POST") {
      const body = parseBody<{ entries?: PricingEntry[] }>({});
      const selectedEntries = (body.entries ?? []).map((entry) => ({
        ...entry,
        source: "models.dev",
      }));
      const pricesByModel = new Map(settings.pricing.entries.map((entry) => [entry.model, entry]));
      selectedEntries.forEach((entry) => pricesByModel.set(entry.model, entry));
      settings.pricing.entries = Array.from(pricesByModel.values()).sort((a, b) =>
        a.model.localeCompare(b.model),
      );
      settings.proxy.models = Array.from(
        new Set([...settings.proxy.models, ...selectedEntries.map((entry) => entry.model)]),
      ).sort((a, b) => a.localeCompare(b));
      return jsonResponse(clone(settings.pricing));
    }

    if (url.pathname === "/api/settings/models/preset" && method === "PUT") {
      const body = parseBody<{ model?: string; enabled?: boolean }>({});
      const model = String(body.model ?? "");
      const enabled = new Set(settings.proxy.enabledModels);
      if (body.enabled) enabled.add(model);
      else enabled.delete(model);
      settings.proxy.enabledModels = settings.proxy.models.filter((candidate) =>
        enabled.has(candidate),
      );
      return jsonResponse(clone(settings.proxy));
    }

    if (url.pathname === "/api/settings/models" && method === "DELETE") {
      const body = parseBody<{ model?: string }>({});
      const model = String(body.model ?? "");
      settings.proxy.models = settings.proxy.models.filter((candidate) => candidate !== model);
      settings.proxy.enabledModels = settings.proxy.enabledModels.filter(
        (candidate) => candidate !== model,
      );
      settings.pricing.entries = settings.pricing.entries.filter((entry) => entry.model !== model);
      return jsonResponse({ deletedModel: model });
    }

    if (url.pathname === "/api/settings/pricing" && method === "PUT") {
      const body = parseBody<{ catalogVersion?: string; entries?: PricingEntry[] }>({});
      settings.pricing = {
        catalogVersion: body.catalogVersion ?? settings.pricing.catalogVersion,
        entries: body.entries ?? settings.pricing.entries,
      };
      settings.proxy.models = Array.from(
        new Set([
          ...settings.proxy.models,
          ...settings.pricing.entries.map((entry) => entry.model),
        ]),
      ).sort((a, b) => a.localeCompare(b));
      return jsonResponse(clone(settings.pricing));
    }

    return undefined;
  };
}

function StorybookSystemWorkspaceRoutes() {
  return (
    <Routes>
      <Route path="/system" element={<SystemLayout />}>
        <Route path="status" element={<SystemStatusPage />} />
        <Route path="tasks" element={<SystemTasksPage />} />
        <Route path="tasks/:taskKey" element={<SystemTaskDetailPage />} />
        <Route path="settings" element={<SystemSettingsPage />} />
        <Route path="models" element={<SystemModelsPage />} />
        <Route path="proxy" element={<SystemProxyPage />} />
      </Route>
    </Routes>
  );
}

function StorybookSystemWorkspaceMock({ children }: { children: ReactNode }) {
  const originalFetchRef = useRef<typeof window.fetch | null>(null);

  useLayoutEffect(() => {
    originalFetchRef.current = window.fetch.bind(window);
    return () => {
      if (originalFetchRef.current) {
        window.fetch = originalFetchRef.current;
      }
    };
  }, []);

  return <>{children}</>;
}

const meta = {
  title: "System/SystemWorkspace",
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    viewport: { defaultViewport: "desktop1660" },
  },
  decorators: [
    (Story, context) => (
      <I18nProvider>
        <StorybookSystemWorkspaceMock>
          <StorybookPageEnvironment
            onRequest={buildSystemWorkspaceRequestHandler(
              context.parameters.systemStatusOverride as SystemStatusResponse | undefined,
              context.parameters.settingsOverride as SettingsPayload | undefined,
              context.parameters.failFirstModelsPreview === true,
            )}
          >
            <FullPageStorySurface>
              <Story />
            </FullPageStorySurface>
          </StorybookPageEnvironment>
        </StorybookSystemWorkspaceMock>
      </I18nProvider>
    ),
  ],
} satisfies Meta;

export default meta;

type Story = StoryObj<typeof meta>;

function renderWorkspace(initialEntry: string) {
  return (
    <MemoryRouter initialEntries={[initialEntry]}>
      <StorybookSystemWorkspaceRoutes />
    </MemoryRouter>
  );
}

export const Status: Story = {
  render: () => renderWorkspace("/system/status"),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("heading", { name: "系统状态" })).toBeVisible();
    await expect(canvas.getByTestId("system-status-overview")).toBeVisible();
    await expect(canvas.getByTestId("system-status-projection-health")).toBeVisible();
    await expect(canvas.getByRole("link", { name: "状态" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    await expect(canvas.getByText("已追踪项目存储总览")).toBeVisible();
    await expect(canvas.getByText("数据库记录概况")).toBeVisible();
    await expect(
      canvas.getByText(
        "已追踪项目存储 = 已追踪 raw 盘点 + archive + 数据库 + 其他运行文件；raw 盘点或 archive 体积不可用时保持未知，也不代表完整物理文件系统占用。",
      ),
    ).toBeVisible();
    await expect(canvas.getByTestId("system-status-overview")).toHaveTextContent("受限");
    await expect(canvas.getByTestId("system-status-request-raw-breakdown")).toBeVisible();
    await expect(canvas.getByTestId("system-status-response-raw-breakdown")).toBeVisible();
    await expect(canvas.getAllByText("侧向拆分")).toHaveLength(2);
    await expect(canvas.getByTestId("system-status-request-raw-breakdown")).toHaveTextContent(
      "数量",
    );
    await expect(canvas.getByTestId("system-status-response-raw-breakdown")).toHaveTextContent(
      "数量",
    );
  },
};

function runtimePressureStatus(
  state: "healthy" | "deferred" | "degraded" | "accounting_error",
): SystemStatusResponse {
  const base = STORYBOOK_SYSTEM_STATUS.runtimePressureHealth!;
  return {
    ...STORYBOOK_SYSTEM_STATUS,
    runtimePressureHealth: {
      ...base,
      state,
      process: {
        ...base.process,
        swapBytes: state === "degraded" ? 268_435_456 : 0,
        pressureLevel: state === "degraded" ? "elevated" : "normal",
      },
      writerAccounting: {
        ...base.writerAccounting,
        state: state === "accounting_error" ? "degraded" : "healthy",
        invariantViolationCount: state === "accounting_error" ? 1 : 0,
        degradedReason: state === "accounting_error" ? "pending_bytes_underflow" : undefined,
      },
      retentionWriteHealth: {
        ...base.retentionWriteHealth!,
        state: state === "deferred" ? "deferred" : state === "degraded" ? "degraded" : "healthy",
        admissionMode:
          state === "deferred" ? undefined : state === "degraded" ? "fairness" : "normal",
        deferReason: state === "deferred" ? "pressure_cooldown:30000ms" : undefined,
        budgetBreachCount: state === "degraded" ? 1 : 0,
        lockWaitMs: state === "degraded" ? 15_004 : 2,
        executeMs: state === "degraded" ? 251 : 47,
        commitMs: state === "degraded" ? 36 : 18,
        rawReferenceCheckMs:
          state === "healthy" || state === "accounting_error"
            ? base.retentionWriteHealth?.rawReferenceCheckMs
            : undefined,
        starvationAgeMs: state === "degraded" ? 15_004 : undefined,
      },
      dashboardProjection: {
        ...base.dashboardProjection,
        state: state === "degraded" ? "degraded" : "healthy",
        producerState: state === "deferred" ? "idle" : "running",
        degradedReason: state === "degraded" ? "projection_stale" : undefined,
        lastDeferReason: state === "deferred" ? "writer_pressure" : undefined,
      },
      eventBus: {
        ...base.eventBus!,
        state: state === "degraded" ? "degraded" : "healthy",
        routerLaggedCount: state === "degraded" ? 2 : 0,
        routerGapCount: state === "degraded" ? 1 : 0,
        cursorRecoveryCount: state === "degraded" ? 1 : 0,
      },
      backfill: {
        ...base.backfill!,
        state: state === "deferred" ? "deferred" : "healthy",
        deferredTaskCount: state === "deferred" ? 1 : 0,
        pressureDeferCount: state === "deferred" ? 3 : 0,
      },
    },
  };
}

function retentionRecoveryStatus(
  state: "healthy" | "recovering" | "deferred" | "degraded",
): SystemStatusResponse {
  const status = runtimePressureStatus(
    state === "healthy" ? "healthy" : state === "deferred" ? "deferred" : "degraded",
  );
  const base = status.runtimePressureHealth!;
  return {
    ...status,
    runtimePressureHealth: {
      ...base,
      state: state === "degraded" ? "degraded" : state === "deferred" ? "deferred" : "healthy",
      retentionRecovery: {
        ...base.retentionRecovery!,
        state,
        stage:
          state === "healthy"
            ? "orphan_sweep"
            : state === "deferred"
              ? "prepared_reconcile"
              : state === "recovering"
                ? "prepared_reconcile"
                : "finalizing",
        preparedCount: state === "healthy" ? 0 : 18,
        quarantinedCount: state === "degraded" ? 3 : 1,
        expiredBacklogCount: state === "healthy" ? 0 : 42,
        oldestBacklogAgeSecs: state === "healthy" ? undefined : 86_400,
        nextRetryAt: state === "healthy" ? undefined : "2026-06-22T08:05:00Z",
        failureStage: state === "degraded" ? "status_refresh" : undefined,
        failureFingerprint: state === "degraded" ? "7d38a1c0b4c8e2f1" : undefined,
        deferReason: state === "recovering" || state === "deferred" ? "sqlite_pressure" : undefined,
        consecutiveFailureCount: state === "degraded" ? 4 : 0,
      },
    },
  };
}

function rawOrphanSweepStatus(
  state: "idle" | "scanning" | "deferred" | "degraded" | "unknown",
): SystemStatusResponse {
  const status = runtimePressureStatus(state === "degraded" ? "degraded" : "healthy");
  const base = status.runtimePressureHealth!;
  return {
    ...status,
    runtimePressureHealth: {
      ...base,
      rawOrphanSweep:
        state === "unknown"
          ? undefined
          : {
              ...base.rawOrphanSweep!,
              state,
              inspectedEntries: state === "scanning" ? 0 : 96,
              referencedSkipped: state === "scanning" ? 0 : 12,
              quarantined: state === "scanning" ? 0 : state === "degraded" ? 3 : 2,
              removed: state === "scanning" || state === "degraded" ? 0 : 1,
              removedBytes: state === "scanning" || state === "degraded" ? 0 : 65_536,
              lastSettledPass: {
                ...base.rawOrphanSweep!.lastSettledPass!,
                complete: state !== "degraded",
                inspectedEntries: state === "degraded" ? 96 : 128,
                quarantined: state === "degraded" ? 3 : 4,
                removed: state === "degraded" ? 0 : 2,
                removedBytes: state === "degraded" ? 0 : 131_072,
              },
              nextRetryAt: state === "deferred" ? "2026-06-22T08:05:00Z" : undefined,
              deferReason:
                state === "deferred"
                  ? "sqlite_pressure"
                  : state === "degraded"
                    ? "retry_backoff"
                    : undefined,
              admissionStage: state === "deferred" ? "background_slot" : undefined,
              admissionCause: state === "deferred" ? "background_busy" : undefined,
              failureFingerprint: state === "degraded" ? "7d38a1c0b4c8e2f1" : undefined,
            },
    },
  };
}

function hotTopicStatus(
  scenario: "healthy" | "deferred" | "hot-db-read" | "cadence-miss",
): SystemStatusResponse {
  const status = runtimePressureStatus(
    scenario === "healthy" ? "healthy" : scenario === "deferred" ? "deferred" : "degraded",
  );
  const base = status.runtimePressureHealth!;
  const topics = base.dashboardHotTopics!;
  return {
    ...status,
    runtimePressureHealth: {
      ...base,
      process: { ...base.process, pressureLevel: "normal", swapBytes: 0 },
      dashboardProjection: {
        ...base.dashboardProjection,
        state: "healthy",
        degradedReason: undefined,
        lastDeferReason: undefined,
      },
      eventBus: {
        ...base.eventBus!,
        state: "healthy",
        routerLaggedCount: 0,
        routerGapCount: 0,
        cursorRecoveryCount: 0,
      },
      dashboardHotTopics: {
        ...topics,
        state:
          scenario === "healthy" ? "healthy" : scenario === "deferred" ? "deferred" : "degraded",
        workingConversations:
          scenario === "deferred" ? hotTopic("deferred") : topics.workingConversations,
        parallelWork:
          scenario === "hot-db-read"
            ? hotTopic("degraded", { livePathDbReadCount: 3 })
            : topics.parallelWork,
        activity:
          scenario === "cadence-miss"
            ? hotTopic("degraded", { cadenceMissCount: 4 })
            : topics.activity,
      },
    },
  };
}

const hotTopicPlay =
  (testId: string, metric: string) =>
  async ({ canvasElement }: { canvasElement: HTMLElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByTestId("system-status-dashboard-hot-topics")).toBeVisible();
    await expect(await canvas.findByTestId(testId)).toHaveTextContent(metric);
  };

export const StatusHotTopicsHealthy: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: hotTopicStatus("healthy") },
  play: hotTopicPlay("system-status-hot-topic-activity", "DB 0"),
};

export const StatusHotTopicsDeferred: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: hotTopicStatus("deferred") },
  play: hotTopicPlay("system-status-hot-topic-workingConversations", "已延后"),
};

export const StatusHotTopicsHotDbRead: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: {
    systemStatusOverride: hotTopicStatus("hot-db-read"),
    viewport: { defaultViewport: "desktop1660x900" },
  },
  play: hotTopicPlay("system-status-hot-topic-parallelWork", "DB 3"),
};

export const StatusHotTopicsCadenceMiss: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: {
    systemStatusOverride: hotTopicStatus("cadence-miss"),
    viewport: { defaultViewport: "mobile393" },
  },
  play: hotTopicPlay("system-status-hot-topic-activity", "cadence 4"),
};

const runtimePressurePlay =
  (label: string, expectedReference = "1ms") =>
  async ({ canvasElement }: { canvasElement: HTMLElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByTestId("system-status-runtime-pressure-health")).toBeVisible();
    await expect(await canvas.findByText(`运行压力：${label}`)).toBeVisible();
    await userEvent.click(await canvas.findByText("运行压力详情"));
    await expect(await canvas.findByText("实时路径数据库读取")).toBeVisible();
    await expect(await canvas.findByText("保留写入健康状态")).toBeVisible();
    await expect(
      await canvas.findByText(new RegExp(`raw 引用确认 ${expectedReference}`)),
    ).toBeVisible();
  };

export const StatusRuntimePressureHealthy: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: runtimePressureStatus("healthy") },
  play: runtimePressurePlay("健康"),
};

export const StatusRuntimePressureDeferred: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: runtimePressureStatus("deferred") },
  play: runtimePressurePlay("已延后", "-"),
};

export const StatusRuntimePressureDegraded: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: {
    systemStatusOverride: runtimePressureStatus("degraded"),
    viewport: { defaultViewport: "desktop1660x900" },
  },
  play: runtimePressurePlay("已降级", "-"),
};

export const StatusRuntimePressureAccountingError: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: runtimePressureStatus("accounting_error") },
  play: runtimePressurePlay("核算异常"),
};

export const StatusRuntimePressureUnknown: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: {
    systemStatusOverride: {
      ...STORYBOOK_SYSTEM_STATUS,
      runtimePressureHealth: {
        ...STORYBOOK_SYSTEM_STATUS.runtimePressureHealth!,
        eventBus: undefined,
        backfill: undefined,
        retentionRecovery: undefined,
      },
    } satisfies SystemStatusResponse,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByText("运行压力详情"));
    await expect(await canvas.findByText("Typed runtime 事件总线")).toBeVisible();
    await expect(canvas.getAllByText("未知").length).toBeGreaterThanOrEqual(2);
    const recovery = canvas.getByTestId("system-status-retention-recovery");
    await expect(recovery).toHaveTextContent("未知");
    await expect(recovery).toHaveTextContent("未知 / 未知");
    await expect(recovery).not.toHaveTextContent("0 / 0");
  },
};

export const StatusRetentionRecoveryHealthy: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: retentionRecoveryStatus("healthy") },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByText("运行压力详情"));
    await expect(canvas.getByTestId("system-status-retention-recovery")).toHaveTextContent("健康");
    await expect(canvas.getByTestId("system-status-retention-recovery")).toHaveTextContent(
      "孤儿清扫",
    );
    await expect(canvas.getByTestId("system-status-runtime-pressure-health")).toHaveTextContent(
      "1ms",
    );
  },
};

export const StatusRetentionRecoveryRecovering: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: retentionRecoveryStatus("recovering") },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByText("运行压力详情"));
    await expect(canvas.getByTestId("system-status-retention-recovery")).toHaveTextContent(
      "恢复中",
    );
    await expect(canvas.getByTestId("system-status-retention-recovery")).toHaveTextContent(
      "准备对账",
    );
    await expect(canvas.getByTestId("system-status-retention-recovery")).toHaveTextContent(
      "SQLite 压力",
    );
    await expect(canvas.getByTestId("system-status-retention-recovery")).toHaveTextContent("42");
    await expect(canvas.getByTestId("system-status-retention-recovery-live")).toHaveTextContent(
      "SQLite 压力",
    );
  },
};

export const StatusRetentionRecoveryDeferred: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: retentionRecoveryStatus("deferred") },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByText("运行压力详情"));
    const recovery = canvas.getByTestId("system-status-retention-recovery");
    await expect(recovery).toHaveTextContent("已延后");
    await expect(recovery).toHaveTextContent("准备对账");
    await expect(recovery).toHaveTextContent("SQLite 压力");
    await expect(canvas.getByTestId("system-status-retention-recovery-live")).toHaveTextContent(
      "SQLite 压力",
    );
  },
};

export const StatusRetentionRecoveryDegraded: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: retentionRecoveryStatus("degraded") },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByText("运行压力详情"));
    const recovery = canvas.getByTestId("system-status-retention-recovery");
    await expect(recovery).toHaveTextContent("最终化");
    await expect(recovery).toHaveTextContent("失败阶段 状态刷新 · 7d38a1c0b4c8e2f1");
    await expect(recovery).toHaveTextContent("连续失败：4");
  },
};

export const StatusRawOrphanSweepUnknown: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: rawOrphanSweepStatus("unknown") },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByText("运行压力详情"));
    const sweep = canvas.getByTestId("system-status-raw-orphan-sweep");
    await expect(sweep).toHaveTextContent("未知");
  },
};

export const StatusRawOrphanSweepScanning: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: rawOrphanSweepStatus("scanning") },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByText("运行压力详情"));
    const sweep = canvas.getByTestId("system-status-raw-orphan-sweep");
    await expect(sweep).toHaveTextContent("扫描中");
    await expect(within(sweep).getAllByText("0", { exact: true })).toHaveLength(4);
    await expect(sweep).toHaveTextContent("0 B");
  },
};

export const StatusRawOrphanSweepDeferred: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: rawOrphanSweepStatus("deferred") },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByText("运行压力详情"));
    const sweep = canvas.getByTestId("system-status-raw-orphan-sweep");
    await expect(sweep).toHaveTextContent("已延后");
    await expect(sweep).toHaveTextContent("SQLite 压力");
    await expect(sweep).toHaveTextContent("下次重试");
    await expect(sweep).toHaveTextContent("后台任务繁忙");
    await expect(sweep).toHaveTextContent("最近一次已结算轮次");
    await expect(sweep).toHaveTextContent("完整");
  },
};

export const StatusRawOrphanSweepDegraded: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: rawOrphanSweepStatus("degraded") },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByText("运行压力详情"));
    const sweep = canvas.getByTestId("system-status-raw-orphan-sweep");
    await expect(sweep).toHaveTextContent("异常");
    await expect(sweep).toHaveTextContent("重试退避");
    await expect(sweep).toHaveTextContent("7d38a1c0b4c8e2f1");
    await expect(sweep).toHaveTextContent("最近一次已结算轮次");
    await expect(sweep).toHaveTextContent("部分完成");
  },
};

export const StatusRawCaptureSuppressed: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: {
    systemStatusOverride: {
      ...STORYBOOK_SYSTEM_STATUS,
      runtimePressureHealth: {
        ...STORYBOOK_SYSTEM_STATUS.runtimePressureHealth!,
        state: "degraded",
        rawCapture: {
          ...STORYBOOK_SYSTEM_STATUS.runtimePressureHealth!.rawCapture!,
          state: "storage_suppressed",
          reason: "filesystem_low",
          availableBytes: 18_000_000_000,
          reservedBytes: 4_194_304,
          backlogNonGrowing: false,
        },
      },
    } satisfies SystemStatusResponse,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByText("运行压力详情"));
    const panel = canvas.getByTestId("system-status-raw-capture");
    await expect(panel).toHaveTextContent("已抑制落盘");
    await expect(panel).toHaveTextContent("文件系统可用空间不足");
    await expect(panel).toHaveTextContent("已就绪");
    await expect(panel).toHaveTextContent("原始占用水位");
    await expect(panel).toHaveTextContent("文件系统水位");
    await expect(panel).toHaveTextContent("增长中");
    await expect(panel).toHaveTextContent("16 GiB");
    await expect(panel).toHaveTextContent("20 GiB");
    await expect(panel.querySelectorAll("[aria-live]")).toHaveLength(0);
    await expect(canvasElement.querySelector("span.sr-only[aria-live=polite]")).toHaveTextContent(
      "原始载荷熔断：已抑制落盘；文件系统可用空间不足；库存已就绪",
    );
  },
};

export const StatusRawCaptureCapturing: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByText("运行压力详情"));
    const panel = canvas.getByTestId("system-status-raw-capture");
    await expect(panel).toHaveTextContent("正常采集");
    await expect(panel).toHaveTextContent("当前未触发抑制");
    await expect(panel).toHaveTextContent("关闭阈值");
    await expect(panel).toHaveTextContent("恢复阈值");
    await expect(panel).toHaveTextContent("16 GiB");
    await expect(canvasElement).toHaveTextContent("原始载荷熔断：正常采集");
  },
};

export const StatusRawCaptureUnknown: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: {
    systemStatusOverride: {
      ...STORYBOOK_SYSTEM_STATUS,
      runtimePressureHealth: {
        ...STORYBOOK_SYSTEM_STATUS.runtimePressureHealth!,
        rawCapture: undefined,
      },
    } satisfies SystemStatusResponse,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByText("运行压力详情"));
    const panel = canvas.getByTestId("system-status-raw-capture");
    await expect(panel).toHaveTextContent("未知");
    await expect(panel).toHaveTextContent("未知");
    await expect(panel).toHaveTextContent("过期积压趋势");
  },
};

export const StatusRuntimePressureDegradedMobile: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: {
    systemStatusOverride: runtimePressureStatus("degraded"),
    viewport: { defaultViewport: "mobile393" },
  },
  play: runtimePressurePlay("已降级", "-"),
};

export const StatusRequestHeavy: Story = {
  render: () => renderWorkspace("/system/status"),
  parameters: {
    systemStatusOverride: {
      ...STORYBOOK_SYSTEM_STATUS,
      rawBodies: { count: 1_482, bytes: 69_000_000_000 },
      requestRawBodies: { count: 812, bytes: 68_719_476_736 },
      responseRawBodies: { count: 670, bytes: 5_905_580_032 },
      archivedBodies: { count: 118_420, bytes: 649_117_696 },
      databaseBytes: 5_261_484_032,
      otherFilesBytes: 8_806,
      rawMetricsHealth: {
        state: "preparing",
        inventoryCursor: 64_000,
        physicalCoverage: "unknown",
      },
      projectionHealth: {
        terminal: {
          state: "dirty_last_good",
          cursorLag: 16,
          dirtyBucketCount: 0,
          pendingEventCount: 16,
          lastDeferReason: "pending_event_count",
        },
        longTerm: {
          state: "deferred",
          cursorLag: 16,
          dirtyBucketCount: 2,
          pendingEventCount: 16,
          lastDeferReason: "writer_pressure",
        },
      },
    } satisfies SystemStatusResponse,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("system-status-overview")).toBeVisible();
    await expect(canvas.getByText("已追踪 raw payload 总量")).toBeVisible();
    await expect(canvas.getByText("已追踪 request 侧 raw payload")).toBeVisible();
    await expect(canvas.getByText("已追踪 response 侧 raw payload")).toBeVisible();
    await expect(canvas.getByText("并集总量")).toBeVisible();
    await expect(canvas.getAllByText("侧向拆分")).toHaveLength(2);
    await expect(canvas.getByTestId("system-status-request-raw-breakdown")).toHaveTextContent(
      "812",
    );
    await expect(canvas.getByTestId("system-status-response-raw-breakdown")).toHaveTextContent(
      "670",
    );
    const overview = canvas.getByTestId("system-status-overview");
    await expect(overview).toHaveTextContent("未知");
    await expect(overview).not.toHaveTextContent("0 B");
    await expect(overview).toHaveTextContent(
      "Raw payload 盘点仍在后台建立；在覆盖可用前，raw 字节数和项目总量保持未知。",
    );
    await userEvent.click(canvas.getByText("投影详情"));
    await expect(canvas.getByText("writer_pressure")).toBeVisible();
    await expect(canvas.getByText("2")).toBeVisible();
  },
};

type RawInventoryUnavailableState = "preparing" | "deferred" | "error" | "unknown";

function rawInventoryUnavailableStatus(state: RawInventoryUnavailableState): SystemStatusResponse {
  return {
    ...STORYBOOK_SYSTEM_STATUS,
    rawBodies: { ...STORYBOOK_SYSTEM_STATUS.rawBodies, bytes: 0 },
    requestRawBodies: { ...STORYBOOK_SYSTEM_STATUS.requestRawBodies, bytes: 0 },
    responseRawBodies: { ...STORYBOOK_SYSTEM_STATUS.responseRawBodies, bytes: 0 },
    rawMetricsHealth: {
      state,
      inventoryCursor: 64_000,
      physicalCoverage: "unknown",
    },
  };
}

function rawInventoryUnavailablePlay(message: string) {
  return async ({ canvasElement }: { canvasElement: HTMLElement }) => {
    const canvas = within(canvasElement);
    const overview = await canvas.findByTestId("system-status-overview");
    await expect(overview).toBeVisible();
    await expect(overview).toHaveTextContent("已追踪项目存储总览");
    await expect(overview).toHaveTextContent("未知");
    await expect(overview).not.toHaveTextContent("0 B");
    await expect(overview).toHaveTextContent(message);
  };
}

export const StatusRawInventoryPreparing: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: rawInventoryUnavailableStatus("preparing") },
  play: rawInventoryUnavailablePlay(
    "Raw payload 盘点仍在后台建立；在覆盖可用前，raw 字节数和项目总量保持未知。",
  ),
};

export const StatusRawInventoryDeferred: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: rawInventoryUnavailableStatus("deferred") },
  play: rawInventoryUnavailablePlay(
    "数据库压力较高，Raw payload 盘点已延后；raw 字节数和项目总量保持未知。",
  ),
};

export const StatusRawInventoryError: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: rawInventoryUnavailableStatus("error") },
  play: rawInventoryUnavailablePlay(
    "Raw payload 盘点需要恢复；恢复覆盖前，raw 字节数和项目总量保持未知。",
  ),
};

export const StatusRawInventoryUnknown: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: { systemStatusOverride: rawInventoryUnavailableStatus("unknown") },
  play: rawInventoryUnavailablePlay("Raw payload 盘点覆盖范围未知；raw 字节数和项目总量保持未知。"),
};

export const Tasks: Story = {
  render: () => renderWorkspace("/system/tasks"),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("heading", { name: "后台任务" })).toBeVisible();
    await expect(canvas.getByTestId("system-tasks-list")).toBeVisible();
    await expect(canvas.getByText(/forward_proxy_subscription_refresh/)).toBeVisible();
  },
};

export const TaskDetail: Story = {
  render: () => renderWorkspace("/system/tasks/retention_archive"),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("heading", { name: "数据保留与归档" })).toBeVisible();
    await expect(canvas.getByText("默认计划 · 3600s")).toBeVisible();
    await expect(canvas.getByText("invocations")).toBeVisible();
    await expect(canvas.getByText("partial")).toBeVisible();
    await expect(canvas.getByText("暂不可用（积压 3）")).toBeVisible();
    await expect(canvas.getByText("prompt_cache")).toBeVisible();
    await expect(canvas.getByText("92.0%")).toBeVisible();
  },
};

export const Settings: Story = {
  render: () => renderWorkspace("/system/settings"),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("heading", { name: "系统设置" })).toBeVisible();
    await expect(canvas.getByText("价格配置")).toBeVisible();
    await expect(canvas.getByText("External API Keys")).toBeVisible();
  },
};

export const ProxyPage: Story = {
  render: () => renderWorkspace("/system/proxy"),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("heading", { name: "代理" })).toBeVisible();
    await expect(canvas.getByText("正向代理路由")).toBeVisible();
    await expect(canvas.getByTestId("settings-forward-proxy-desktop-table")).toBeVisible();
  },
};

export const Models: Story = {
  render: () => renderWorkspace("/system/models"),
  tags: ["test"],
  parameters: {
    settingsOverride: STORYBOOK_MODELS_SETTINGS,
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: { story: "Merged model directory with local prices and preset switches." },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.findByRole("heading", { name: "模型" })).resolves.toBeVisible();
    await expect(canvas.findAllByText("gpt-6-sol")).resolves.toHaveLength(2);
    await expect(canvas.findAllByText("local-unpriced")).resolves.toHaveLength(2);
    await expect(canvas.getByRole("button", { name: "全部同步" })).toBeVisible();
    await expect(canvas.getByRole("link", { name: "模型" })).toHaveAttribute(
      "aria-current",
      "page",
    );
  },
};

export const ModelsDark: Story = {
  ...Models,
  globals: { themeMode: "dark" },
};

export const ModelsMobile: Story = {
  ...Models,
  parameters: {
    ...Models.parameters,
    viewport: { defaultViewport: "mobile393" },
  },
};

export const ModelsMobileDark: Story = {
  ...ModelsMobile,
  globals: { themeMode: "dark" },
};

export const ModelsSyncReview: Story = {
  ...Models,
  parameters: {
    ...Models.parameters,
    docs: { description: { story: "Preview with a resolved cross-provider price conflict." } },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole("button", { name: "全部同步" }));
    const page = within(canvasElement.ownerDocument.body);
    await expect(page.findByRole("dialog")).resolves.toBeVisible();
    const providerChoice = await page.findByRole("combobox", {
      name: "为 gpt-6-sol 选择一个供应商报价",
    });
    await userEvent.click(providerChoice);
    await userEvent.click(await page.findByRole("option", { name: "OpenRouter (openrouter)" }));
    await expect(page.getByRole("checkbox", { name: "同步 gpt-6-sol 的价格" })).toBeChecked();
    await expect(page.getByText("新模型")).toBeVisible();
    await expect(page.getByText(/不导入：/)).toBeVisible();
  },
};

export const ModelsSyncReviewMobile: Story = {
  ...ModelsSyncReview,
  parameters: {
    ...ModelsSyncReview.parameters,
    viewport: { defaultViewport: "mobile393" },
  },
};

export const ModelsSyncRetry: Story = {
  ...Models,
  parameters: {
    ...Models.parameters,
    failFirstModelsPreview: true,
    docs: { description: { story: "Retrieval failure and the successful retry path." } },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByRole("button", { name: "全部同步" }));
    const page = within(canvasElement.ownerDocument.body);
    await expect(page.findByRole("alert")).resolves.toHaveTextContent(
      "models.dev is temporarily unavailable",
    );
    await userEvent.click(await page.findByRole("button", { name: "重试" }));
    await expect(page.findByText("deepseek-v3.2")).resolves.toBeVisible();
  },
};
