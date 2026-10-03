/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import SystemStatusPage from "./SystemStatusPage";

const apiMocks = vi.hoisted(() => ({
  fetchSystemStatus: vi.fn(),
  fetchSystemStorage: vi.fn(),
}));

vi.mock("../../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../../lib/api")>("../../lib/api");
  return {
    ...actual,
    fetchSystemStatus: apiMocks.fetchSystemStatus,
    fetchSystemStorage: apiMocks.fetchSystemStorage,
  };
});

let host: HTMLDivElement | null = null;
let root: Root | null = null;
let originalLocalStorageDescriptor: PropertyDescriptor | undefined;

function createMemoryStorage(): Storage {
  const store = new Map<string, string>();

  return {
    get length() {
      return store.size;
    },
    clear() {
      store.clear();
    },
    getItem(key: string) {
      return store.get(key) ?? null;
    },
    key(index: number) {
      return Array.from(store.keys())[index] ?? null;
    },
    removeItem(key: string) {
      store.delete(key);
    },
    setItem(key: string, value: string) {
      store.set(key, value);
    },
  };
}

function renderPage() {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);

  act(() => {
    root?.render(
      <I18nProvider>
        <SystemStatusPage />
      </I18nProvider>,
    );
  });
}

function pendingUntilAbort(signal: AbortSignal) {
  return new Promise<never>((_resolve, reject) => {
    signal.addEventListener("abort", () => reject(new Error("aborted")), { once: true });
  });
}

function hotTopic(state = "healthy", overrides: Partial<Record<string, string | number>> = {}) {
  return {
    topicClass: "hot_projection",
    state,
    activeSubscriberCount: 2,
    builderCount: 12,
    genericFallbackBuildCount: 0,
    livePathDbReadCount: 0,
    materializationCount: 12,
    serializationCount: 12,
    payloadCloneCount: 0,
    frameReused: 8,
    cadenceMissCount: 0,
    reconnectChurnCount: 0,
    ...overrides,
  };
}

describe("SystemStatusPage", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    originalLocalStorageDescriptor ??= Object.getOwnPropertyDescriptor(window, "localStorage");
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      value: createMemoryStorage(),
    });
    window.localStorage.setItem("codex-vibe-monitor.locale", "zh");
    apiMocks.fetchSystemStatus.mockResolvedValue({
      liveInvocationsCount: 6,
      successCount: 3,
      nonSuccessCount: 3,
      completedArchiveBatchesCount: 2,
      archivedBodies: { count: 9, bytes: 4_096 },
      rawBodies: { count: 5, bytes: 6_144 },
      requestRawBodies: { count: 2, bytes: 4_096 },
      responseRawBodies: { count: 3, bytes: 2_048 },
      databaseBytes: 2_048,
      otherFilesBytes: 8_192,
      rawMetricsHealth: { state: "ready", inventoryCursor: 6, physicalCoverage: "partial" },
      projectionHealth: {
        terminal: {
          state: "healthy",
          cursorLag: 0,
          dirtyBucketCount: 0,
          pendingEventCount: 0,
        },
        longTerm: {
          state: "repairing",
          cursorLag: 0,
          dirtyBucketCount: 0,
          pendingEventCount: 0,
        },
      },
      refreshedAt: "2026-06-22T08:00:00Z",
    });
    apiMocks.fetchSystemStorage.mockResolvedValue({
      totalBytes: 16 * 1024 ** 3,
      sampledAt: "2026-06-22T08:00:00Z",
      state: "ready",
      scanInProgress: false,
      stale: false,
      reason: null,
    });
  });

  afterEach(() => {
    act(() => {
      root?.unmount();
    });
    host?.remove();
    host = null;
    root = null;
    apiMocks.fetchSystemStatus.mockReset();
    apiMocks.fetchSystemStorage.mockReset();
    window.localStorage.removeItem("codex-vibe-monitor.locale");
    if (originalLocalStorageDescriptor) {
      Object.defineProperty(window, "localStorage", originalLocalStorageDescriptor);
    }
    vi.useRealTimers();
  });

  it("loads system status immediately and refreshes every minute", async () => {
    renderPage();

    await act(async () => {
      await Promise.resolve();
    });

    expect(apiMocks.fetchSystemStatus).toHaveBeenCalledTimes(1);
    expect(apiMocks.fetchSystemStorage).toHaveBeenCalledTimes(1);
    expect(host?.querySelector('[data-testid="system-status-layout"]')).not.toBeNull();
    expect(host?.querySelector('[data-testid="system-status-overview"]')).not.toBeNull();
    expect(host?.querySelector('[data-testid="system-status-projection-health"]')).not.toBeNull();
    expect(
      host?.querySelector('[data-testid="system-status-runtime-pressure-health"]'),
    ).not.toBeNull();
    expect(host?.textContent ?? "").toContain("运行压力：未知");
    expect(host?.textContent ?? "").toContain("这些字段不会为状态刷新新增 SQLite 查询");
    expect(host?.querySelector('[data-testid="system-status-raw-metrics-health"]')).not.toBeNull();
    expect(host?.textContent ?? "").toContain(
      "Raw payload 已完成关联文件盘点；未关联的物理 raw 残留仍不在此总量内。",
    );
    expect(host?.textContent ?? "").toContain("修复中");
    expect(host?.querySelector('[data-testid="system-status-records-section"]')).not.toBeNull();
    expect(host?.querySelector('[data-testid="system-status-archive-section"]')).not.toBeNull();
    expect(host?.textContent ?? "").toContain("项目存储总体积");
    expect(host?.textContent ?? "").toContain("16 GiB");
    expect(host?.textContent ?? "").toContain("采样时间：");
    expect(host?.textContent ?? "").toContain("业务存储指标");
    expect(host?.textContent ?? "").toContain("数据库记录概况");
    expect(host?.textContent ?? "").toContain("归档与逻辑体量");
    expect(host?.textContent ?? "").toContain(
      "数据目录及单独配置的外置 raw、archive、数据库和 Xray runtime 路径",
    );
    expect(host?.textContent ?? "").not.toContain(
      "先展示服务能够验证的项目存储，再标明 raw 盘点是否覆盖物理存储。",
    );
    expect(
      host?.querySelectorAll('[data-testid="system-status-project-disk-formula"]'),
    ).toHaveLength(0);
    expect(host?.textContent ?? "").toContain("并集总量");
    expect(host?.textContent ?? "").toContain("侧向拆分");
    expect(host?.textContent ?? "").toContain("live invocations");
    expect(host?.textContent ?? "").toContain("已完成归档批次数");
    expect(
      host?.querySelector('[data-testid="system-status-request-raw-breakdown"]')?.textContent ?? "",
    ).toContain("request 侧 raw payload");
    expect(
      host?.querySelector('[data-testid="system-status-request-raw-breakdown"]')?.textContent ?? "",
    ).toContain("体积");
    expect(
      host?.querySelector('[data-testid="system-status-request-raw-breakdown"]')?.textContent ?? "",
    ).toContain("数量");
    expect(
      host?.querySelector('[data-testid="system-status-request-raw-breakdown"]')?.textContent ?? "",
    ).toContain("侧向拆分");
    expect(
      host?.querySelector('[data-testid="system-status-request-raw-breakdown"]')?.textContent ?? "",
    ).toContain("4.0 KB");
    expect(
      host?.querySelector('[data-testid="system-status-request-raw-breakdown"]')?.textContent ?? "",
    ).toContain("2");
    expect(
      host?.querySelector('[data-testid="system-status-response-raw-breakdown"]')?.textContent ??
        "",
    ).toContain("response 侧 raw payload");
    expect(
      host?.querySelector('[data-testid="system-status-response-raw-breakdown"]')?.textContent ??
        "",
    ).toContain("2.0 KB");
    expect(
      host?.querySelector('[data-testid="system-status-response-raw-breakdown"]')?.textContent ??
        "",
    ).toContain("3");
    expect(host?.textContent ?? "").toContain(
      "raw payload 只按已持久化盘点的 request + response 关联文件统计；request / response 只解释侧向分布，不能直接相加回并集，未关联的物理 raw 残留不在此视图内。",
    );

    await act(async () => {
      vi.advanceTimersByTime(60_000);
      await Promise.resolve();
    });

    expect(apiMocks.fetchSystemStatus).toHaveBeenCalledTimes(2);
    expect(apiMocks.fetchSystemStorage).toHaveBeenCalledTimes(2);
  });

  it("keeps the measured project total visible while raw inventory is preparing", async () => {
    apiMocks.fetchSystemStatus.mockResolvedValueOnce({
      liveInvocationsCount: 6,
      successCount: 3,
      nonSuccessCount: 3,
      completedArchiveBatchesCount: 2,
      archivedBodies: { count: 9, bytes: 4_096 },
      rawBodies: { count: 5, bytes: 0 },
      requestRawBodies: { count: 2, bytes: 0 },
      responseRawBodies: { count: 3, bytes: 0 },
      databaseBytes: 2_048,
      otherFilesBytes: 8_192,
      rawMetricsHealth: {
        state: "preparing",
        inventoryCursor: 0,
        physicalCoverage: "unknown",
      },
      projectionHealth: {
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
        },
      },
      refreshedAt: "2026-06-22T08:00:00Z",
    });

    renderPage();

    await act(async () => {
      await Promise.resolve();
    });

    const overviewText =
      host?.querySelector('[data-testid="system-status-overview"]')?.textContent ?? "";
    expect(overviewText).toContain("未知");
    expect(overviewText).not.toContain("0 B");
    expect(overviewText).not.toContain("实际磁盘");
    expect(overviewText).toContain(
      "Raw payload 盘点仍在后台建立；在覆盖可用前，raw 字节数保持未知。",
    );
    const storageSummary = host?.querySelector('[data-testid="system-storage-summary"]');
    expect(storageSummary?.textContent).toContain("16 GiB");
    expect(storageSummary?.textContent).not.toContain("未知");
  });

  it("renders the storage snapshot when the business status request returns 503", async () => {
    apiMocks.fetchSystemStatus.mockRejectedValueOnce(new Error("503 Service Unavailable"));

    renderPage();

    await act(async () => {
      await Promise.resolve();
    });

    expect(host?.querySelector('[data-testid="system-storage-summary"]')?.textContent).toContain(
      "16 GiB",
    );
    expect(host?.textContent).toContain("503 Service Unavailable");
    expect(host?.querySelector('[data-testid="system-status-layout"]')).toBeNull();
  });

  it("retains the previous storage value and sample time after a refresh failure", async () => {
    apiMocks.fetchSystemStorage.mockResolvedValueOnce({
      totalBytes: 8 * 1024 ** 3,
      sampledAt: "2026-06-22T08:00:00Z",
      state: "ready",
      scanInProgress: false,
      stale: false,
      reason: null,
    });
    apiMocks.fetchSystemStorage.mockRejectedValueOnce(new Error("network unavailable"));
    renderPage();

    await act(async () => {
      await Promise.resolve();
    });
    await act(async () => {
      vi.advanceTimersByTime(60_000);
      await Promise.resolve();
      await Promise.resolve();
    });

    const summary = host?.querySelector('[data-testid="system-storage-summary"]');
    expect(summary?.textContent).toContain("8 GiB");
    expect(summary?.textContent).toContain("采样时间：");
    expect(summary?.textContent).toContain("network unavailable");
  });

  it.each([
    ["status", "fetchSystemStatus", "fetchSystemStorage"],
    ["storage", "fetchSystemStorage", "fetchSystemStatus"],
  ] as const)("times out the %s request independently", async (_name, pendingKey, resolvedKey) => {
    const pendingRequest = apiMocks[pendingKey];
    const resolvedRequest = apiMocks[resolvedKey];
    pendingRequest.mockImplementation(pendingUntilAbort);

    renderPage();

    const pendingSignal = pendingRequest.mock.calls[0]?.[0] as AbortSignal;
    const resolvedSignal = resolvedRequest.mock.calls[0]?.[0] as AbortSignal;
    expect(pendingSignal.aborted).toBe(false);
    expect(resolvedSignal.aborted).toBe(false);

    await act(async () => {
      await Promise.resolve();
    });
    await act(async () => {
      vi.advanceTimersByTime(10_000);
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(pendingSignal.aborted).toBe(true);
    expect(resolvedSignal.aborted).toBe(false);
    expect(host?.textContent).toContain("Request timed out after 10 seconds");
  });

  it("aborts both requests when the page unmounts", async () => {
    apiMocks.fetchSystemStatus.mockImplementation(pendingUntilAbort);
    apiMocks.fetchSystemStorage.mockImplementation(pendingUntilAbort);

    renderPage();

    const statusSignal = apiMocks.fetchSystemStatus.mock.calls[0]?.[0] as AbortSignal;
    const storageSignal = apiMocks.fetchSystemStorage.mock.calls[0]?.[0] as AbortSignal;
    await act(async () => {
      root?.unmount();
      await Promise.resolve();
    });

    expect(statusSignal.aborted).toBe(true);
    expect(storageSignal.aborted).toBe(true);
  });

  it.each([
    "HTTP 404",
    "Invalid system storage response",
  ])("keeps the last measured total after a %s storage response", async (message) => {
    apiMocks.fetchSystemStorage.mockResolvedValueOnce({
      totalBytes: 8 * 1024 ** 3,
      sampledAt: "2026-06-22T08:00:00Z",
      state: "ready",
      scanInProgress: false,
      stale: false,
      reason: null,
    });
    apiMocks.fetchSystemStorage.mockRejectedValueOnce(new Error(message));
    renderPage();

    await act(async () => {
      await Promise.resolve();
    });
    await act(async () => {
      vi.advanceTimersByTime(60_000);
      await Promise.resolve();
      await Promise.resolve();
    });

    const summary = host?.querySelector('[data-testid="system-storage-summary"]');
    expect(summary?.textContent).toContain("8 GiB");
    expect(summary?.textContent).toContain(message);
    expect(summary?.textContent).not.toContain("20 KiB");
  });

  it("shows zero and an explicit unknown state without inventing a zero value", async () => {
    apiMocks.fetchSystemStorage.mockResolvedValueOnce({
      totalBytes: 0,
      sampledAt: "2026-06-22T08:00:00Z",
      state: "ready",
      scanInProgress: false,
      stale: false,
      reason: null,
    });
    renderPage();
    await act(async () => {
      await Promise.resolve();
    });
    expect(host?.querySelector('[data-testid="system-storage-summary"]')?.textContent).toContain(
      "0 B",
    );

    apiMocks.fetchSystemStorage.mockResolvedValueOnce({
      totalBytes: null,
      sampledAt: null,
      state: "unknown",
      scanInProgress: false,
      stale: false,
      reason: null,
    });
    act(() => root?.unmount());
    host?.remove();
    host = null;
    root = null;
    renderPage();
    await act(async () => {
      await Promise.resolve();
    });
    const summary = host?.querySelector('[data-testid="system-storage-summary"]')?.textContent;
    expect(summary).toContain("未知");
    expect(summary).not.toContain("0 B");
  });

  it("keeps missing event bus and backfill diagnostics visibly unknown", async () => {
    apiMocks.fetchSystemStatus.mockResolvedValueOnce({
      liveInvocationsCount: 6,
      successCount: 3,
      nonSuccessCount: 3,
      completedArchiveBatchesCount: 2,
      archivedBodies: { count: 9, bytes: 4_096 },
      rawBodies: { count: 5, bytes: 6_144 },
      requestRawBodies: { count: 2, bytes: 4_096 },
      responseRawBodies: { count: 3, bytes: 2_048 },
      databaseBytes: 2_048,
      otherFilesBytes: 8_192,
      rawMetricsHealth: { state: "ready", inventoryCursor: 6, physicalCoverage: "partial" },
      projectionHealth: {
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
        },
      },
      runtimePressureHealth: {
        state: "healthy",
        process: {
          rssBytes: 0,
          rssAnonBytes: 0,
          swapBytes: 0,
          peakRssBytes: 0,
          threads: 0,
          managedBytes: 0,
          unattributedAnonBytes: 0,
          pressureLevel: "normal",
        },
        allocator: { mallocArenaMax: "8" },
        writerAccounting: {
          state: "healthy",
          pendingDepth: 0,
          pendingBytes: 0,
          transferBytes: 0,
          retryCount: 0,
          invariantViolationCount: 0,
        },
        retentionWriteHealth: {
          state: "deferred",
          operation: "invocation_detail_prune",
          batchRows: 4,
          estimatedBytes: 16_384,
          prepareElapsedMs: 36,
          lockWaitMs: 15_000,
          executeMs: 0,
          commitMs: 0,
          budgetBreachCount: 0,
          deferReason: "pressure_cooldown:30000ms",
          p1WaiterCount: 2,
          candidateRemainingHint: 1,
        },
        dashboardProjection: {
          mode: "auto",
          state: "healthy",
          producerState: "idle",
          activeSubscriberCount: 0,
          livePathDbReadCount: 0,
          buildCount: 0,
          revision: 0,
          snapshotOrigin: "none",
          sliceCounters: {
            current: { buildCount: 0, revisionCount: 0, cadenceMissCount: 0 },
            network: { buildCount: 0, revisionCount: 0, cadenceMissCount: 0 },
            terminal: { buildCount: 0, revisionCount: 0, cadenceMissCount: 0 },
          },
        },
        delivery: {
          activity: {
            materializationCount: 0,
            serializationCount: 0,
            payloadCloneCount: 0,
            frameBytesCount: 0,
            laggedCount: 0,
            skippedCount: 0,
            businessPayloadCount: 0,
            jsonOverlayCount: 0,
          },
          summary: {
            materializationCount: 0,
            serializationCount: 0,
            payloadCloneCount: 0,
            frameBytesCount: 0,
            laggedCount: 0,
            skippedCount: 0,
            businessPayloadCount: 0,
            jsonOverlayCount: 0,
          },
          networkTimeseries: {
            materializationCount: 0,
            serializationCount: 0,
            payloadCloneCount: 0,
            frameBytesCount: 0,
            laggedCount: 0,
            skippedCount: 0,
            businessPayloadCount: 0,
            jsonOverlayCount: 0,
          },
          networkRecent: {
            materializationCount: 0,
            serializationCount: 0,
            payloadCloneCount: 0,
            frameBytesCount: 0,
            laggedCount: 0,
            skippedCount: 0,
            businessPayloadCount: 0,
            jsonOverlayCount: 0,
          },
        },
        dashboardHotTopics: {
          state: "degraded",
          activity: hotTopic("degraded", { cadenceMissCount: 3 }),
          summary: hotTopic(),
          networkTimeseries: hotTopic(),
          networkRecent: hotTopic(),
          workingConversations: hotTopic(),
          parallelWork: hotTopic("degraded", { livePathDbReadCount: 2 }),
          timeseries: hotTopic(),
        },
      },
      refreshedAt: "2026-06-22T08:00:00Z",
    });

    renderPage();

    await act(async () => {
      await Promise.resolve();
    });

    const pageText = host?.textContent ?? "";
    expect(pageText).toContain("Typed runtime 事件总线");
    expect(pageText).toContain("启动回填");
    expect(pageText).toContain("当前后端尚未发布这一 additive 诊断字段。");
    expect(
      host?.querySelector('[data-testid="system-status-dashboard-hot-topics"]'),
    ).not.toBeNull();
    expect(
      host?.querySelector('[data-testid="system-status-hot-topic-activity"]')?.textContent,
    ).toContain("cadence 3");
    expect(
      host?.querySelector('[data-testid="system-status-hot-topic-parallelWork"]')?.textContent,
    ).toContain("DB 2");
    expect(host?.querySelectorAll('[data-testid^="system-status-hot-topic-"]')).toHaveLength(7);
    expect(pageText).toContain("保留写入健康状态");
    expect(pageText).toContain("已延后");
    expect(pageText).toContain("4 / 16 KB");
    expect(pageText).toContain("raw 引用确认 -");
    expect(pageText).toContain("pressure_cooldown:30000ms");
  });
});
