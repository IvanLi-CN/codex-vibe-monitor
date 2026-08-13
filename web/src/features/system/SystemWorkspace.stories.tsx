import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useLayoutEffect, useRef } from "react";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { expect, userEvent, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type {
  ExternalApiKeySummary,
  SettingsPayload,
  SystemStatusResponse,
  SystemTaskRunsResponse,
} from "../../lib/api";
import type { RuntimePressureDashboardHotTopicHealth } from "../../lib/api/core-foundation";
import SystemLayout from "../../pages/system/SystemLayout";
import SystemProxyPage from "../../pages/system/SystemProxyPage";
import SystemSettingsPage from "../../pages/system/SystemSettingsPage";
import SystemStatusPage from "../../pages/system/SystemStatusPage";
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
  rawMetricsHealth: { state: "ready", inventoryCursor: 128_076 },
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
): StorybookRequestHandler {
  return async ({ url, init }) => {
    const method = (init?.method ?? "GET").toUpperCase();
    const jsonResponse = (payload: unknown, status = 200) =>
      new Response(JSON.stringify(payload), {
        status,
        headers: { "Content-Type": "application/json" },
      });

    if (url.pathname === "/api/system/status" && method === "GET") {
      return jsonResponse(clone(statusOverride ?? STORYBOOK_SYSTEM_STATUS));
    }

    if (url.pathname === "/api/system/tasks" && method === "GET") {
      return jsonResponse(clone(filterStorybookSystemTasks(url)));
    }

    if (url.pathname === "/api/settings" && method === "GET") {
      return jsonResponse(clone(STORYBOOK_SETTINGS));
    }

    if (url.pathname === "/api/settings/external-api-keys" && method === "GET") {
      return jsonResponse({ items: clone(STORYBOOK_EXTERNAL_API_KEYS) });
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
        <Route path="settings" element={<SystemSettingsPage />} />
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
    await expect(canvas.getByText("实际磁盘占用总览")).toBeVisible();
    await expect(canvas.getByText("数据库记录概况")).toBeVisible();
    await expect(
      canvas.getByText(
        "当前项目磁盘占用 = raw payload 并集总量 + archive + 数据库 + 其他运行文件。",
      ),
    ).toBeVisible();
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
  (label: string) =>
  async ({ canvasElement }: { canvasElement: HTMLElement }) => {
    const canvas = within(canvasElement);
    await expect(await canvas.findByTestId("system-status-runtime-pressure-health")).toBeVisible();
    await expect(await canvas.findByText(`运行压力：${label}`)).toBeVisible();
    await userEvent.click(await canvas.findByText("运行压力详情"));
    await expect(await canvas.findByText("实时路径数据库读取")).toBeVisible();
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
  play: runtimePressurePlay("已延后"),
};

export const StatusRuntimePressureDegraded: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: {
    systemStatusOverride: runtimePressureStatus("degraded"),
    viewport: { defaultViewport: "desktop1660x900" },
  },
  play: runtimePressurePlay("已降级"),
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
      },
    } satisfies SystemStatusResponse,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.click(await canvas.findByText("运行压力详情"));
    await expect(await canvas.findByText("Typed runtime 事件总线")).toBeVisible();
    await expect(canvas.getAllByText("未知").length).toBeGreaterThanOrEqual(2);
  },
};

export const StatusRuntimePressureDegradedMobile: Story = {
  render: () => renderWorkspace("/system/status"),
  tags: ["test"],
  parameters: {
    systemStatusOverride: runtimePressureStatus("degraded"),
    viewport: { defaultViewport: "mobile393" },
  },
  play: runtimePressurePlay("已降级"),
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
      rawMetricsHealth: { state: "preparing", inventoryCursor: 64_000 },
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
    await expect(canvas.getByText("raw payload 总量")).toBeVisible();
    await expect(canvas.getByText("request 侧 raw payload")).toBeVisible();
    await expect(canvas.getByText("response 侧 raw payload")).toBeVisible();
    await expect(canvas.getByText("并集总量")).toBeVisible();
    await expect(canvas.getAllByText("侧向拆分")).toHaveLength(2);
    await expect(canvas.getByTestId("system-status-request-raw-breakdown")).toHaveTextContent(
      "812",
    );
    await expect(canvas.getByTestId("system-status-response-raw-breakdown")).toHaveTextContent(
      "670",
    );
    await expect(canvas.getByText("64 GB")).toBeVisible();
    await expect(canvas.getByText("5.5 GB")).toBeVisible();
    await userEvent.click(canvas.getByText("投影详情"));
    await expect(canvas.getByText("writer_pressure")).toBeVisible();
    await expect(canvas.getByText("2")).toBeVisible();
  },
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
