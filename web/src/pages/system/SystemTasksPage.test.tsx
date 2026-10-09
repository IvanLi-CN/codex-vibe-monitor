/** @vitest-environment jsdom */

import { waitFor, within } from "@testing-library/dom";
import userEvent from "@testing-library/user-event";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import SystemTasksPage from "./SystemTasksPage";

const apiMocks = vi.hoisted(() => ({
  fetchManagedTasks: vi.fn(),
  fetchManagedTaskRuntime: vi.fn(),
  fetchManagedTaskTimeline: vi.fn(),
  fetchManagedTaskWorkload: vi.fn(),
}));
const streamMocks = vi.hoisted(() => ({
  useSubscriptionTopic: vi.fn(),
  useSseStatus: vi.fn(),
  requestImmediateReconnect: vi.fn(),
  runtimeLastReceivedAt: 0,
  timelineLastReceivedAt: 0 as number | null,
  runtimeData: null as unknown,
  timelineData: null as unknown,
  runtimeRefresh: vi.fn(),
  timelineRefresh: vi.fn(),
  catalogRefresh: vi.fn(),
}));
vi.mock("../../lib/api", async () => ({
  ...(await vi.importActual<typeof import("../../lib/api")>("../../lib/api")),
  fetchManagedTasks: apiMocks.fetchManagedTasks,
  fetchManagedTaskRuntime: apiMocks.fetchManagedTaskRuntime,
  fetchManagedTaskTimeline: apiMocks.fetchManagedTaskTimeline,
  fetchManagedTaskWorkload: apiMocks.fetchManagedTaskWorkload,
}));
vi.mock("../../hooks/useSubscriptionTopic", () => ({
  useSubscriptionTopic: streamMocks.useSubscriptionTopic,
}));
vi.mock("../../hooks/useSseStatus", () => ({
  default: streamMocks.useSseStatus,
}));
vi.mock("../../lib/sse", async () => ({
  ...(await vi.importActual<typeof import("../../lib/sse")>("../../lib/sse")),
  requestImmediateReconnect: streamMocks.requestImmediateReconnect,
}));

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function pageElement() {
  return (
    <MemoryRouter>
      <I18nProvider>
        <SystemTasksPage />
      </I18nProvider>
    </MemoryRouter>
  );
}

function renderPage() {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => root?.render(pageElement()));
}

function updatePage() {
  act(() => root?.render(pageElement()));
}

async function flushEffects() {
  await act(async () => {
    await Promise.resolve();
  });
}

describe("SystemTasksPage", () => {
  beforeEach(() => {
    HTMLElement.prototype.scrollIntoView = () => undefined;
    HTMLElement.prototype.hasPointerCapture = () => false;
    HTMLElement.prototype.setPointerCapture = () => undefined;
    HTMLElement.prototype.releasePointerCapture = () => undefined;
    const runtimeFixture = {
      observedAt: "2026-10-01T00:00:00.000Z",
      activeRuns: [
        {
          executionId: 42,
          executionUid: "run-42",
          taskKey: "retention_archive",
          title: "数据保留与归档",
          activeChildTaskKey: null,
          activeChildTitle: null,
          triggerKind: "interval",
          phase: "processing",
          executionClass: "maintenance_retention",
          startedAt: "2026-10-01T00:00:00.000Z",
          elapsedMs: 12_000,
        },
      ],
      queuedRuns: [
        {
          runId: 43,
          taskKey: "retention_archive",
          title: "数据保留与归档",
          triggerKind: "manual",
          requestedAt: "2026-10-01T00:00:01.000Z",
          waitingMs: 5_000,
          position: 1,
        },
      ],
      queuedRunsAvailable: true,
      admissionWaits: [
        {
          id: "wait-44",
          taskKey: "retention_archive",
          title: "数据保留与归档",
          reason: "pressure_cooldown",
          startedAt: "2026-10-01T00:00:00.000Z",
          waitingMs: 8_000,
          retryAt: null,
        },
      ],
      admissionWaitsAvailable: true,
    };
    apiMocks.fetchManagedTaskRuntime.mockResolvedValue(runtimeFixture);
    apiMocks.fetchManagedTaskWorkload.mockResolvedValue({
      revision: 0,
      coverage: "no recorded attempts",
      samples: [],
      clearanceEstimateReason: "insufficient_samples",
    });
    streamMocks.runtimeData = runtimeFixture;
    const timelineFixture = {
      observedAt: "2026-10-01T00:00:02.000Z",
      windowStart: "2026-09-30T00:00:02.000Z",
      windowEnd: "2026-10-01T00:00:02.000Z",
      watermark: 1,
      segments: [],
      coverage: [],
      nextCursor: null,
      resetRequired: false,
    };
    apiMocks.fetchManagedTaskTimeline.mockResolvedValue(timelineFixture);
    streamMocks.timelineData = {
      watermark: timelineFixture.watermark,
      observedAt: timelineFixture.observedAt,
    };
    streamMocks.useSseStatus.mockReturnValue({
      phase: "connected",
      downtimeMs: 0,
      nextRetryAt: null,
      autoReconnect: true,
    });
    streamMocks.runtimeLastReceivedAt = Date.now();
    streamMocks.timelineLastReceivedAt = Date.now();
    streamMocks.useSubscriptionTopic.mockImplementation((descriptor: { topic: string } | null) => {
      if (descriptor?.topic === "system.managed-tasks.runtime") {
        return {
          data: streamMocks.runtimeData,
          lastReceivedAt: streamMocks.runtimeLastReceivedAt,
          error: null,
          isLoading: false,
          refresh: streamMocks.runtimeRefresh,
        };
      }
      if (descriptor?.topic === "system.managed-tasks.timeline") {
        return {
          data: streamMocks.timelineData,
          lastReceivedAt: streamMocks.timelineLastReceivedAt,
          error: null,
          isLoading: false,
          refresh: streamMocks.timelineRefresh,
        };
      }
      if (descriptor?.topic === "system.managed-tasks.catalog") {
        return {
          data: null,
          lastReceivedAt: null,
          error: null,
          isLoading: true,
          refresh: streamMocks.catalogRefresh,
        };
      }
      return {
        data: null,
        lastReceivedAt: null,
        error: null,
        isLoading: true,
        refresh: vi.fn(),
      };
    });
    apiMocks.fetchManagedTasks.mockResolvedValue([
      {
        taskKey: "retention_archive",
        title: "数据保留与归档",
        description: "按保留策略归档并清理历史数据",
        triggerMode: "interval",
        enabled: true,
        intervalSecs: 300,
        cronExpr: null,
        isManual: false,
        displayColorLight: "#c2410c",
        displayColorDark: "#f59e0b",
      },
      {
        taskKey: "raw_compression",
        title: "原始载荷压缩",
        description: "压缩冷数据原始载荷",
        triggerMode: "manual",
        enabled: false,
        intervalSecs: null,
        cronExpr: null,
        isManual: true,
        displayColorLight: "#0f766e",
        displayColorDark: "#2dd4bf",
      },
    ]);
  });
  afterEach(() => {
    act(() => root?.unmount());
    host?.remove();
    host = null;
    root = null;
    vi.useRealTimers();
    apiMocks.fetchManagedTasks.mockReset();
    apiMocks.fetchManagedTaskRuntime.mockReset();
    apiMocks.fetchManagedTaskTimeline.mockReset();
    apiMocks.fetchManagedTaskWorkload.mockReset();
    streamMocks.useSubscriptionTopic.mockReset();
    streamMocks.useSseStatus.mockReset();
    streamMocks.requestImmediateReconnect.mockReset();
    streamMocks.catalogRefresh.mockReset();
  });

  it("loads the managed task directory", async () => {
    renderPage();
    await flushEffects();
    expect(apiMocks.fetchManagedTasks).toHaveBeenCalledTimes(1);
    expect(streamMocks.useSubscriptionTopic).toHaveBeenCalledWith({
      topic: "system.managed-tasks.runtime",
    });
    expect(streamMocks.useSubscriptionTopic).toHaveBeenCalledWith({
      topic: "system.managed-tasks.timeline",
      params: { schemaVersion: "2" },
    });
    expect(streamMocks.useSubscriptionTopic).toHaveBeenCalledWith({
      topic: "system.managed-tasks.catalog",
    });
    expect(apiMocks.fetchManagedTaskRuntime).not.toHaveBeenCalled();
    expect(apiMocks.fetchManagedTaskTimeline).toHaveBeenCalledTimes(1);
    expect(apiMocks.fetchManagedTaskTimeline).toHaveBeenCalledWith(
      expect.objectContaining({ limit: 500, windowHours: 12 }),
    );
    expect(host?.textContent).toContain("数据保留与归档");
    expect(host?.textContent).toContain("正在执行");
    expect(host?.textContent).toContain("已入队");
    expect(host?.textContent).toContain("等待准入 / 压力延后");
    expect(host?.textContent).toContain("压力冷却让行");
    expect(host?.textContent).toContain("请求时间");
    expect(host?.textContent).toContain("等待开始");
    expect(host?.textContent).toContain("08:00:01");
    expect(within(host as HTMLElement).getAllByText("第 1 位 · 手动")).toHaveLength(1);
    expect(host?.textContent).toContain("raw_compression");
    expect(host?.textContent).not.toContain("查看运行计量");
  });

  it("shows manual and scheduled task modes", async () => {
    renderPage();
    await flushEffects();
    expect(host?.textContent).toContain("固定间隔");
    expect(host?.textContent).toContain("手动");
    expect(host?.textContent).toContain("已停用");
  });

  it("does not report an empty timeline after an HTTP baseline arrives before SSE", async () => {
    streamMocks.timelineData = null;
    streamMocks.timelineLastReceivedAt = null;
    renderPage();
    await waitFor(() => expect(apiMocks.fetchManagedTaskTimeline).toHaveBeenCalledTimes(1));
    await flushEffects();

    expect(host?.textContent).not.toContain("尚无可用的时间线记录");
  });

  it("combines enabled and trigger filters without hiding the running area", async () => {
    renderPage();
    await flushEffects();
    const user = userEvent.setup();
    const page = within(host as HTMLElement);
    await user.click(page.getByRole("combobox", { name: "启用状态" }));
    await user.click(within(document.body).getByRole("option", { name: "已停用" }));
    expect(host?.textContent).toContain("显示 1 / 2");
    expect(page.getByRole("link", { name: /原始载荷压缩/ })).toBeTruthy();
    expect(page.queryByRole("link", { name: /数据保留与归档/ })).toBeNull();
    expect(host?.textContent).toContain("正在执行");
    expect(host?.textContent).toContain("第 1 位");

    const checkboxes = Array.from(
      host?.querySelectorAll<HTMLInputElement>('input[type="checkbox"]') ?? [],
    );
    for (const checkbox of checkboxes) {
      if (checkbox.checked) await user.click(checkbox);
    }
    const manual = checkboxes.find((checkbox) =>
      checkbox.parentElement?.textContent?.includes("手动"),
    );
    expect(manual).toBeDefined();
    await user.click(manual as HTMLInputElement);
    expect(host?.textContent).toContain("显示 1 / 2");
    expect(host?.textContent).toContain("原始载荷压缩");
  });

  it("shows an explicit SSE disconnect state and marks runtime observations unknown", async () => {
    streamMocks.useSseStatus.mockReturnValue({
      phase: "reconnecting",
      downtimeMs: 10_000,
      nextRetryAt: Date.now() + 2_000,
      autoReconnect: true,
    });
    streamMocks.runtimeLastReceivedAt = Date.now();
    streamMocks.timelineLastReceivedAt = Date.now();
    streamMocks.useSubscriptionTopic.mockImplementation((descriptor: { topic: string } | null) => ({
      data: null,
      lastReceivedAt: null,
      error: descriptor?.topic === "system.managed-tasks.runtime" ? "unavailable" : null,
      isLoading: false,
      refresh: vi.fn(),
    }));
    renderPage();
    await waitFor(() => expect(host?.textContent).toContain("实时数据断开，正在重连"));
    expect(host?.textContent).toContain("当前是否有任务正在工作未知");
    expect(host?.textContent).toContain("观测未知");
  });

  it("shows the connecting state while waiting for the first SSE snapshots", async () => {
    streamMocks.useSseStatus.mockReturnValue({
      phase: "connecting",
      downtimeMs: 0,
      nextRetryAt: null,
      autoReconnect: true,
    });
    renderPage();
    await flushEffects();
    expect(within(host as HTMLElement).getByRole("status").textContent).toContain(
      "实时数据连接中，正在等待服务端快照",
    );
  });

  it("hydrates timeline pages and restarts the baseline when a cursor expires", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-10-01T00:00:10.000Z"));
    const originalSegment = {
      segmentId: "timeline-run-1",
      kind: "execution",
      taskKey: "retention_archive",
      title: "数据保留与归档",
      startedAt: "2026-10-01T00:00:00.000Z",
      lastObservedAt: "2026-10-01T00:00:04.000Z",
      finishedAt: "2026-10-01T00:00:04.000Z",
      durationMs: 4_000,
      status: "failed",
      triggerKind: "manual",
      executionClass: null,
      reason: null,
      retryAt: null,
      activeChildTaskKey: null,
      activeChildTitle: null,
      managedRunId: 1,
      sessionId: "session-one",
      revision: 1,
    };
    apiMocks.fetchManagedTaskTimeline.mockResolvedValueOnce({
      observedAt: "2026-10-01T00:00:05.000Z",
      windowStart: "2026-09-30T12:00:05.000Z",
      windowEnd: "2026-10-01T00:00:05.000Z",
      watermark: 1,
      segments: [originalSegment],
      coverage: [],
      nextCursor: null,
      resetRequired: false,
    });
    streamMocks.timelineData = { watermark: 1, observedAt: "2026-10-01T00:00:05.000Z" };
    renderPage();
    await flushEffects();
    const initialBars = host?.querySelectorAll('[data-testid="task-timeline"] g[role="button"]');
    expect(
      Array.from(initialBars ?? []).some((bar) =>
        bar.getAttribute("aria-label")?.includes("结果：失败"),
      ),
    ).toBe(true);

    apiMocks.fetchManagedTaskTimeline.mockResolvedValueOnce({
      observedAt: "2026-10-01T00:00:08.000Z",
      windowStart: "2026-09-30T12:00:08.000Z",
      windowEnd: "2026-10-01T00:00:08.000Z",
      watermark: 2,
      segments: [
        { ...originalSegment, status: "success", revision: 2 },
        {
          ...originalSegment,
          segmentId: "timeline-run-2",
          startedAt: "2026-09-30T22:00:00.000Z",
          lastObservedAt: "2026-09-30T22:00:05.000Z",
          finishedAt: "2026-09-30T22:00:05.000Z",
          status: "success",
          revision: 1,
        },
      ],
      coverage: [],
      nextCursor: null,
      resetRequired: false,
    });
    streamMocks.timelineData = { watermark: 2, observedAt: "2026-10-01T00:00:08.000Z" };
    updatePage();
    await flushEffects();

    const bars = Array.from(
      host?.querySelectorAll<SVGGElement>('[data-testid="task-timeline"] g[role="button"]') ?? [],
    );
    expect(bars).toHaveLength(3);
    expect(
      bars.filter((bar) => bar.getAttribute("aria-label")?.includes("结果：成功")),
    ).toHaveLength(2);
    expect(bars.some((bar) => bar.getAttribute("aria-label")?.includes("结果：失败"))).toBe(false);
    expect(apiMocks.fetchManagedTaskTimeline).toHaveBeenLastCalledWith(
      expect.objectContaining({ afterRevision: 1, limit: 500 }),
    );

    apiMocks.fetchManagedTaskTimeline
      .mockResolvedValueOnce({
        observedAt: "2026-10-01T00:00:10.000Z",
        windowStart: "2026-09-30T12:00:10.000Z",
        windowEnd: "2026-10-01T00:00:10.000Z",
        watermark: 3,
        segments: [],
        coverage: [],
        nextCursor: null,
        resetRequired: true,
      })
      .mockResolvedValueOnce({
        observedAt: "2026-10-01T00:00:10.000Z",
        windowStart: "2026-09-30T12:00:10.000Z",
        windowEnd: "2026-10-01T00:00:10.000Z",
        watermark: 3,
        segments: [
          {
            ...originalSegment,
            segmentId: "resnapshot-run",
            title: "重新同步后的执行",
            startedAt: "2026-10-01T00:00:08.000Z",
            lastObservedAt: "2026-10-01T00:00:09.000Z",
            finishedAt: "2026-10-01T00:00:09.000Z",
            status: "success",
            revision: 1,
          },
        ],
        coverage: [],
        nextCursor: null,
        resetRequired: false,
      });
    streamMocks.timelineData = { watermark: 3, observedAt: "2026-10-01T00:00:10.000Z" };
    updatePage();
    await flushEffects();
    await waitFor(() => expect(apiMocks.fetchManagedTaskTimeline).toHaveBeenCalledTimes(4));
    const resnapshotBars = Array.from(
      host?.querySelectorAll<SVGGElement>('[data-testid="task-timeline"] g[role="button"]') ?? [],
    );
    expect(
      resnapshotBars.some((bar) => bar.getAttribute("aria-label")?.includes("重新同步后的执行")),
    ).toBe(true);
    expect(
      resnapshotBars.some((bar) => bar.getAttribute("aria-label")?.includes("结果：失败")),
    ).toBe(false);
  });

  it("offers manual reconnection when SSE updates are disabled", async () => {
    streamMocks.useSseStatus.mockReturnValue({
      phase: "disabled",
      downtimeMs: 0,
      nextRetryAt: null,
      autoReconnect: false,
    });
    renderPage();
    await flushEffects();
    await userEvent.click(within(host as HTMLElement).getByRole("button", { name: "重新连接" }));
    expect(streamMocks.requestImmediateReconnect).toHaveBeenCalledTimes(1);
    expect(host?.textContent).toContain("实时数据连接已停止，当前状态未知");
  });

  it("advances visible time and execution duration between SSE events", async () => {
    vi.useFakeTimers();
    const start = new Date("2026-10-01T00:00:00.000Z");
    vi.setSystemTime(start);
    streamMocks.runtimeLastReceivedAt = Date.now();
    streamMocks.timelineLastReceivedAt = Date.now();
    renderPage();
    await flushEffects();
    const before = host?.querySelector('[data-testid="task-timeline-now"]')?.textContent;
    expect(host?.textContent).toContain("0 分 12 秒");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1_000);
    });

    const after = host?.querySelector('[data-testid="task-timeline-now"]')?.textContent;
    expect(after).not.toBe(before);
    expect(host?.textContent).toContain("0 分 13 秒");
    expect(apiMocks.fetchManagedTaskRuntime).not.toHaveBeenCalled();
    expect(apiMocks.fetchManagedTaskTimeline).toHaveBeenCalledTimes(1);
    vi.useRealTimers();
  });
});
