/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it } from "vitest";
import type {
  CurrentTaskExecution,
  ManagedTask,
  TaskTimelineCoverage,
  TaskTimelineSegment,
} from "../../lib/api";
import { TaskTimelineChart } from "./TaskTimelineChart";

const task: ManagedTask = {
  taskKey: "retention_archive",
  title: "数据保留与归档",
  description: "归档任务",
  triggerMode: "interval",
  enabled: true,
  isManual: false,
  displayColorLight: "#c2410c",
  displayColorDark: "#f59e0b",
};
const secondTask: ManagedTask = {
  ...task,
  taskKey: "long_term_projection",
  title: "长期统计投影",
  displayColorLight: "#15803d",
  displayColorDark: "#4ade80",
};

const segment = (id: string, startMs: number, endMs: number): TaskTimelineSegment => ({
  segmentId: id,
  kind: "execution",
  taskKey: task.taskKey,
  title: task.title,
  startedAt: new Date(startMs).toISOString(),
  lastObservedAt: new Date(endMs).toISOString(),
  finishedAt: new Date(endMs).toISOString(),
  durationMs: endMs - startMs,
  status: "success",
  triggerKind: "interval",
  executionClass: "maintenance_retention",
  sessionId: "test-session",
  revision: 1,
});

const deferral = (
  id: string,
  startMs: number,
  endMs: number,
  reason: string,
): TaskTimelineSegment => ({
  ...segment(id, startMs, endMs),
  kind: "deferral",
  status: "released",
  reason,
});

let host: HTMLDivElement | null = null;
let root: Root | null = null;
let resizeObserverDescriptor: PropertyDescriptor | undefined;
let observedChartWidth = 1_200;

class ChartResizeObserverMock {
  static readonly instances = new Set<ChartResizeObserverMock>();

  private readonly callback: ResizeObserverCallback;
  private readonly targets = new Set<Element>();

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback;
    ChartResizeObserverMock.instances.add(this);
  }

  observe(target: Element) {
    this.targets.add(target);
    this.notify(target, observedChartWidth);
  }

  disconnect() {
    this.targets.clear();
    ChartResizeObserverMock.instances.delete(this);
  }

  private notify(target: Element, width: number) {
    this.callback(
      [
        {
          target,
          contentRect: { width } as DOMRectReadOnly,
        } as ResizeObserverEntry,
      ],
      this as unknown as ResizeObserver,
    );
  }

  static resize(width: number) {
    observedChartWidth = width;
    for (const observer of ChartResizeObserverMock.instances) {
      for (const target of observer.targets) observer.notify(target, width);
    }
  }
}

function renderChart(props: {
  nowMs: number;
  tasks?: ManagedTask[];
  executions?: TaskTimelineSegment[];
  activeRuns?: CurrentTaskExecution[];
  runtimeFresh?: boolean;
  runtimeBoundaryMs?: number;
  chartWidth?: number;
  coverage?: TaskTimelineCoverage[];
}) {
  observedChartWidth = props.chartWidth ?? 1_200;
  resizeObserverDescriptor ??= Object.getOwnPropertyDescriptor(globalThis, "ResizeObserver");
  Object.defineProperty(globalThis, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: ChartResizeObserverMock,
  });
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  const render = (nowMs: number, overrides: Partial<typeof props> = {}) => {
    const next = { ...props, ...overrides };
    return act(() =>
      root?.render(
        <MemoryRouter>
          <TaskTimelineChart
            tasks={next.tasks ?? [task]}
            executions={next.executions ?? []}
            activeRuns={next.activeRuns ?? []}
            coverage={next.coverage ?? []}
            nowMs={nowMs}
            runtimeFresh={next.runtimeFresh ?? true}
            runtimeBoundaryMs={next.runtimeBoundaryMs ?? nowMs}
            runtimeObservedAt={new Date(nowMs).toISOString()}
          />
        </MemoryRouter>,
      ),
    );
  };
  render(props.nowMs);
  return { render };
}

function settleDeferralDetail() {
  return new Promise<void>((resolve) => window.setTimeout(resolve, 100));
}

afterEach(() => {
  act(() => root?.unmount());
  host?.remove();
  if (resizeObserverDescriptor) {
    Object.defineProperty(globalThis, "ResizeObserver", resizeObserverDescriptor);
  } else {
    Reflect.deleteProperty(globalThis, "ResizeObserver");
  }
  resizeObserverDescriptor = undefined;
  observedChartWidth = 1_200;
  host = null;
  root = null;
});

describe("TaskTimelineChart", () => {
  it("fits the timeline into a narrow container without a horizontal scroller", () => {
    renderChart({
      nowMs: Date.parse("2026-10-02T00:00:00.000Z"),
      chartWidth: 249,
    });
    const container = host?.querySelector<HTMLDivElement>("[data-testid='task-timeline-viewport']");
    const grid = container?.parentElement;
    const rowLabels = host?.querySelector<HTMLDivElement>(
      "[data-testid='task-timeline-row-labels']",
    );
    const svg = container?.querySelector("svg");
    const ticks = Array.from(svg?.querySelectorAll(":scope > g > text") ?? []);

    expect(container?.className).not.toContain("overflow-x-auto");
    expect(grid?.className).toContain("grid-cols-1");
    expect(rowLabels?.className).toContain("sr-only");
    expect(svg?.getAttribute("width")).toBe("249");
    expect(svg?.getAttribute("viewBox")?.startsWith("0 0 249 ")).toBe(true);
    expect(ticks).toHaveLength(3);
    const labelStarts = ticks.map((tick) => Number(tick.getAttribute("x")));
    expect(labelStarts[0]).toBeGreaterThanOrEqual(0);
    expect(labelStarts[1] - labelStarts[0]).toBeGreaterThanOrEqual(70);
    expect(labelStarts[2] - labelStarts[1]).toBeGreaterThanOrEqual(70);
    expect(labelStarts[2] + 70).toBeLessThanOrEqual(249);

    act(() => ChartResizeObserverMock.resize(190));
    expect(svg?.getAttribute("width")).toBe("190");
    expect(svg?.querySelectorAll(":scope > g > text")).toHaveLength(2);
    expect(
      Array.from(svg?.querySelectorAll(":scope > g > text") ?? []).map((tick) =>
        Number(tick.getAttribute("x")),
      ),
    ).toEqual([4, 120]);
  });

  it("shows a rolling 12-hour window with three-hour axis ticks", () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    renderChart({
      nowMs,
      executions: [
        segment("outside-window", nowMs - 13 * 3_600_000, nowMs - 12.5 * 3_600_000),
        segment("inside-window", nowMs - 11 * 3_600_000, nowMs - 10 * 3_600_000),
      ],
    });

    const svg = host?.querySelector("svg");
    const ticks = Array.from(svg?.querySelectorAll(":scope > g > text") ?? []);
    const tickPositions = ticks.map((tick) =>
      Number(tick.parentElement?.querySelector("line")?.getAttribute("x1")),
    );
    const bars = Array.from(host?.querySelectorAll<SVGGElement>('g[role="button"]') ?? []);

    expect(tickPositions).toEqual([0, 300, 600, 900, 1200]);
    expect(bars).toHaveLength(1);
    expect(bars[0].getAttribute("aria-label")).toContain("数据保留与归档");
    expect(Number(bars[0].querySelector("rect")?.getAttribute("x"))).toBeCloseTo(100);
  });

  it("renders the advancing local clock as a visible chart value", () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    const { render } = renderChart({ nowMs });
    expect(host?.querySelector('[data-testid="task-timeline-now"]')?.textContent).toContain(
      "10/02 08:00:00",
    );
    render(nowMs + 1_000);
    expect(host?.querySelector('[data-testid="task-timeline-now"]')?.textContent).toContain(
      "10/02 08:00:01",
    );
  });

  it("allocates separate lanes to overlapping executions and uses the saved task color", () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    renderChart({
      nowMs,
      executions: [
        segment("first", nowMs - 20_000, nowMs - 5_000),
        segment("second", nowMs - 10_000, nowMs - 2_000),
      ],
    });
    const bars = Array.from(host?.querySelectorAll<SVGGElement>('g[role="button"]') ?? []);
    expect(bars).toHaveLength(2);
    expect(bars[0].querySelector("rect")?.getAttribute("y")).not.toBe(
      bars[1].querySelector("rect")?.getAttribute("y"),
    );
    expect(bars[0].querySelector("rect")?.getAttribute("fill")).toBe("#c2410c");
  });

  it("keeps overlapping pressure causes in one shared chart row", async () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    const one = deferral("resource-wait", nowMs - 60_000, nowMs - 20_000, "resource_busy");
    renderChart({
      nowMs,
      executions: [
        one,
        deferral("pressure-wait", nowMs - 50_000, nowMs - 10_000, "pressure_cooldown"),
      ],
    });

    const svg = host?.querySelector("svg");
    const bars = Array.from(host?.querySelectorAll<SVGGElement>('g[role="button"]') ?? []);
    const heights = Number(svg?.getAttribute("height"));

    expect(bars).toHaveLength(2);
    expect(bars[0].querySelector("rect")?.getAttribute("y")).toBe(
      bars[1].querySelector("rect")?.getAttribute("y"),
    );
    expect(bars[0].getAttribute("aria-label")).toContain("资源占用等待");
    expect(bars[0].getAttribute("aria-label")).toContain("与 1 条任务让行重叠");
    expect(bars[0].getAttribute("aria-label")).not.toContain("开始 ");
    act(() => bars[0].focus());
    await settleDeferralDetail();
    expect(bars[0].getAttribute("aria-label")).toContain("压力冷却让行");
    expect(bars[0].getAttribute("aria-label")).toContain("开始 ");
    expect(bars[0].querySelector("title")?.textContent).toContain("开始 ");
    act(() => bars[0].blur());
    expect(bars[0].getAttribute("aria-label")).not.toContain("开始 ");
    expect(bars[0].querySelector("title")?.textContent).not.toContain("开始 ");
    expect(heights).toBeGreaterThan(0);
    expect(host?.querySelector('[data-testid="task-timeline-row-labels"]')?.textContent).toContain(
      "准入 / 压力",
    );
  });

  it("does not treat endpoint-only deferrals as overlapping", () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    renderChart({
      nowMs,
      executions: [
        deferral("endpoint-a", nowMs - 30_000, nowMs - 10_000, "resource_busy"),
        deferral("endpoint-b", nowMs - 10_000, nowMs - 1_000, "pressure_cooldown"),
      ],
    });
    const bars = Array.from(host?.querySelectorAll<SVGGElement>('g[role="button"]') ?? []);
    expect(bars).toHaveLength(2);
    expect(bars[0].getAttribute("aria-label")).toContain("无重叠任务让行");
    expect(bars[1].getAttribute("aria-label")).toContain("无重叠任务让行");
    expect(bars[0].querySelector("rect")?.getAttribute("fill-opacity")).toBe("1");
  });

  it("includes open deferrals when calculating static overlap opacity", () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    const open = {
      ...deferral("open-wait", nowMs - 60_000, nowMs - 1_000, "pressure_cooldown"),
      status: "waiting",
      finishedAt: null,
      lastObservedAt: new Date(nowMs - 1_000).toISOString(),
    } satisfies TaskTimelineSegment;
    renderChart({
      nowMs,
      executions: [deferral("closed-wait", nowMs - 90_000, nowMs - 30_000, "resource_busy"), open],
    });

    const closedBar = host?.querySelector<SVGGElement>('[data-task-deferral-id="closed-wait"]');
    expect(closedBar?.getAttribute("aria-label")).toContain("与 1 条任务让行重叠");
    expect(closedBar?.querySelector("rect")?.getAttribute("fill-opacity")).toBe("0.55");
  });

  it("counts dense open deferrals without pairwise render work", () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    const openDeferrals = Array.from({ length: 600 }, (_, index) => ({
      ...deferral(
        `open-dense-${index}`,
        nowMs - 60_000 + index * 10,
        nowMs - 1_000,
        "resource_busy",
      ),
      status: "waiting" as const,
      finishedAt: null,
      lastObservedAt: new Date(nowMs - 1_000).toISOString(),
    }));
    renderChart({ nowMs, executions: openDeferrals });

    const firstBar = host?.querySelector<SVGGElement>('[data-task-deferral-id="open-dense-0"]');
    expect(firstBar?.getAttribute("aria-label")).toContain("与 599 条任务让行重叠");
  });

  it("counts mixed dense static and live deferrals without pairwise render work", () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    const closedDeferrals = Array.from({ length: 300 }, (_, index) =>
      deferral(
        `closed-dense-${index}`,
        nowMs - 60_000 + index * 10,
        nowMs - 1_000,
        "resource_busy",
      ),
    );
    const openDeferrals = Array.from({ length: 300 }, (_, index) => ({
      ...deferral(
        `live-dense-${index}`,
        nowMs - 50_000 + index * 10,
        nowMs - 1_000,
        "pressure_cooldown",
      ),
      status: "waiting" as const,
      finishedAt: null,
      lastObservedAt: new Date(nowMs - 1_000).toISOString(),
    }));
    renderChart({ nowMs, executions: [...closedDeferrals, ...openDeferrals] });

    const staticBar = host?.querySelector<SVGGElement>('[data-task-deferral-id="closed-dense-0"]');
    const liveBar = host?.querySelector<SVGGElement>('[data-task-deferral-id="live-dense-0"]');
    expect(staticBar?.getAttribute("aria-label")).toContain("与 599 条任务让行重叠");
    expect(liveBar?.getAttribute("aria-label")).toContain("与 599 条任务让行重叠");
  });

  it("keeps high-density deferral details lazy and refreshes a focused interval", async () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    const dense = Array.from({ length: 1_000 }, (_, index) => {
      const group = Math.floor(index / 4);
      const start = nowMs - 11 * 60 * 60_000 + group * 30_000;
      return deferral(
        `dense-deferral-${index}`,
        start,
        start + 120_000,
        index % 2 ? "pressure_cooldown" : "resource_busy",
      );
    });
    const { render } = renderChart({ nowMs, executions: dense });
    const bars = Array.from(host?.querySelectorAll<SVGGElement>('g[role="button"]') ?? []);
    expect(bars).toHaveLength(250);
    expect(bars[0].getAttribute("aria-label")).not.toContain("开始 ");
    act(() => bars[0].focus());
    await settleDeferralDetail();
    expect(bars[0].getAttribute("aria-label")).toContain("开始 ");
    expect(bars[0].getAttribute("aria-label")).toContain("压力冷却让行");

    const revised = dense.map((item, index) =>
      index === 0
        ? {
            ...item,
            reason: "pressure_cooldown",
            finishedAt: new Date(nowMs - 1_000).toISOString(),
            lastObservedAt: new Date(nowMs - 1_000).toISOString(),
            revision: 2,
          }
        : item,
    );
    render(nowMs, { executions: revised });
    await settleDeferralDetail();
    expect(bars[0].getAttribute("aria-label")).toContain("恢复");
    expect(bars[0].getAttribute("aria-label")).toContain("压力冷却让行");
  });

  it("marks failed executions separately and lazily exposes complete timing details", async () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    const failed = {
      ...segment("failed-run", nowMs - 120_000, nowMs - 60_000),
      triggerKind: "manual",
      status: "failed",
    } satisfies TaskTimelineSegment;
    renderChart({ nowMs, executions: [failed] });

    const bar = host?.querySelector<SVGGElement>('g[role="button"]');
    const rect = bar?.querySelector("rect");
    expect(rect?.getAttribute("stroke")).toBe("#be123c");
    expect(rect?.getAttribute("stroke-dasharray")).toBe("2 1");
    expect(bar?.getAttribute("aria-label")).toContain("结果：失败");
    expect(bar?.getAttribute("aria-label")).not.toContain("开始：");
    expect(bar?.querySelector("title")?.textContent).not.toContain("开始：");

    act(() => bar?.focus());
    await settleDeferralDetail();
    expect(bar?.getAttribute("aria-label")).toContain("开始：");
    expect(bar?.querySelector("title")?.textContent).toContain("开始：");

    act(() => bar?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(host?.textContent).toContain("触发：manual");
    expect(host?.textContent).toContain("实际开始：");
    expect(host?.textContent).toContain("实际结束：");
    expect(host?.textContent).toContain("实际用时：1 分 00 秒");
    expect(host?.textContent).toContain("结果：失败");
  });

  it("refreshes the static window when a new historical snapshot arrives", () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    const { render } = renderChart({ nowMs });
    expect(host?.querySelectorAll('g[role="button"]')).toHaveLength(0);

    render(nowMs + 3_000, {
      executions: [segment("arrived-after-mount", nowMs + 1_000, nowMs + 2_000)],
    });
    expect(host?.querySelectorAll('g[role="button"]')).toHaveLength(1);
  });

  it("refreshes selected deferral details from the latest revision", () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    const waiting = {
      ...deferral("selected-wait", nowMs - 60_000, nowMs - 10_000, "resource_busy"),
      status: "waiting",
      finishedAt: null,
      lastObservedAt: new Date(nowMs - 10_000).toISOString(),
    } satisfies TaskTimelineSegment;
    const { render } = renderChart({ nowMs, executions: [waiting] });
    const bar = host?.querySelector<SVGGElement>('g[role="button"]');
    act(() => bar?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(host?.textContent).toContain("尚未确认");

    render(nowMs, {
      executions: [
        {
          ...waiting,
          status: "released",
          finishedAt: new Date(nowMs - 2_000).toISOString(),
          lastObservedAt: new Date(nowMs - 2_000).toISOString(),
          revision: 2,
        },
      ],
    });
    expect(host?.textContent).toContain("恢复");
    expect(host?.textContent).not.toContain("尚未确认");
  });

  it("opens every member when a dense deferral group is selected", () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    renderChart({
      nowMs,
      executions: Array.from({ length: 4 }, (_, index) =>
        deferral(`dense-wait-${index}`, nowMs - 60_000, nowMs - 10_000, "resource_busy"),
      ),
    });
    const group = host?.querySelector<SVGGElement>('g[role="button"]');
    act(() => group?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(host?.textContent).toContain("4 条任务让行");
    expect(host?.textContent).toContain("dense-wait-3");
  });

  it("advances an open execution bar when the shared clock advances", () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    const activeRun: CurrentTaskExecution = {
      executionId: 2,
      executionUid: "active-run",
      taskKey: task.taskKey,
      title: task.title,
      triggerKind: "interval",
      phase: "processing",
      executionClass: "maintenance_retention",
      startedAt: new Date(nowMs - 3_600_000).toISOString(),
      elapsedMs: 3_600_000,
    };
    const { render } = renderChart({ nowMs, activeRuns: [activeRun] });
    const activeBar = host?.querySelector<SVGRectElement>('g[role="button"] rect');
    expect(activeBar?.getAttribute("stroke")).toBe("#0e7490");
    const before = Number(activeBar?.getAttribute("width"));
    render(nowMs + 1_000);
    const after = Number(
      host?.querySelector<SVGRectElement>('g[role="button"] rect')?.getAttribute("width"),
    );
    expect(after).toBeGreaterThan(before);
  });

  it("stops extrapolating a runtime run after its observation expires", async () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    const activeRun: CurrentTaskExecution = {
      executionId: 3,
      executionUid: "stale-run",
      taskKey: task.taskKey,
      title: task.title,
      triggerKind: "interval",
      phase: "processing",
      executionClass: "maintenance_retention",
      startedAt: new Date(nowMs - 3_600_000).toISOString(),
      elapsedMs: 3_600_000,
    };
    renderChart({
      nowMs,
      activeRuns: [activeRun],
      runtimeFresh: false,
      runtimeBoundaryMs: nowMs - 1_000,
    });
    const bar = host?.querySelector<SVGGElement>('g[role="button"]');
    expect(bar?.getAttribute("aria-label")).toContain("结果：未知");
    act(() => bar?.focus());
    await settleDeferralDetail();
    expect(bar?.getAttribute("aria-label")).toContain("最后观测于");
  });

  it("shows persistence outages as unknown coverage", () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    const gap: TaskTimelineSegment = {
      segmentId: "coverage-gap",
      kind: "coverage_gap",
      taskKey: "__timeline__",
      title: "观测缺口",
      startedAt: new Date(nowMs - 600_000).toISOString(),
      lastObservedAt: new Date(nowMs - 300_000).toISOString(),
      finishedAt: new Date(nowMs - 300_000).toISOString(),
      status: "unknown",
      reason: "maintenance_store_write_unavailable",
      sessionId: "test-session",
      revision: 2,
    };
    renderChart({ nowMs, executions: [gap] });
    expect(host?.querySelector("svg title")?.textContent).toContain("最近 12 小时");
    expect(
      Array.from(host?.querySelectorAll("svg rect title") ?? []).map((item) => item.textContent),
    ).toContain("maintenance_store_write_unavailable");
  });

  it("limits dropped-event coverage gaps to their recorded interval", () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    const gap: TaskTimelineSegment = {
      ...segment("overflow-gap", nowMs - 5 * 60_000, nowMs - 3 * 60_000),
      kind: "coverage_gap",
      taskKey: "__timeline__",
      title: "观测缺口",
      status: "unknown",
      reason: "event_channel_overflow",
    };
    const coverage: TaskTimelineCoverage[] = [
      {
        sessionId: "test-session",
        startedAt: new Date(nowMs - 12 * 60 * 60_000).toISOString(),
        lastSeenAt: new Date(nowMs - 10_000).toISOString(),
        endedAt: null,
        droppedEvents: 4,
      },
    ];
    renderChart({ nowMs, executions: [gap], coverage });

    const normalBands = Array.from(host?.querySelectorAll<SVGRectElement>("svg rect") ?? []).filter(
      (rect) => rect.querySelector("title")?.textContent === "有记录器覆盖且未观测到任务让行",
    );
    const gapTitle = Array.from(host?.querySelectorAll("svg rect title") ?? []).find((title) =>
      title.textContent?.includes("event_channel_overflow"),
    );
    const chartWidth = Number(host?.querySelector("svg")?.getAttribute("width"));
    const coveredWidth = normalBands.reduce(
      (total, rect) => total + Number(rect.getAttribute("width")),
      0,
    );

    expect(gapTitle).toBeTruthy();
    expect(coveredWidth).toBeGreaterThan(chartWidth * 0.95);
  });

  it("groups dense short executions and opens every member for inspection", () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    renderChart({
      nowMs,
      executions: Array.from({ length: 8 }, (_, index) => {
        const start = nowMs - 15_000 + index * 100;
        const run = segment(`dense-${index}`, start, start + 30);
        return index === 4 ? { ...run, status: "failed" } : run;
      }),
    });
    const aggregate = host?.querySelector<SVGGElement>('g[role="button"]');
    expect(aggregate?.getAttribute("aria-label")).toContain("8 次");
    expect(aggregate?.getAttribute("aria-label")).toContain("失败 1");
    act(() => aggregate?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(host?.textContent).toContain("8 次任务执行");
    expect(host?.textContent).toContain("数据保留与归档");
    expect(host?.textContent).toContain("区间 ID dense-7");
    expect(host?.querySelectorAll("ul.divide-y li")).toHaveLength(8);
  });

  it("keeps interleaved tasks in one inspectable density group", () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    renderChart({
      nowMs,
      tasks: [task, secondTask],
      executions: Array.from({ length: 10 }, (_, index) => {
        const start = nowMs - 15_000 + index * 100;
        const run = segment(`mixed-dense-${index}`, start, start + 30);
        return index % 2 === 0
          ? { ...run, managedRunId: index + 100 }
          : { ...run, taskKey: secondTask.taskKey, title: secondTask.title };
      }),
    });

    const groups = Array.from(host?.querySelectorAll<SVGGElement>('g[role="button"]') ?? []);
    expect(groups).toHaveLength(1);
    expect(groups[0].getAttribute("aria-label")).toContain("10 次多任务执行");
    expect(groups[0].querySelector("rect")?.getAttribute("fill")).toBe("#64748b");

    act(() => groups[0].dispatchEvent(new MouseEvent("click", { bubbles: true })));

    expect(host?.querySelectorAll("ul.divide-y li")).toHaveLength(10);
    expect(host?.textContent).toContain(task.title);
    expect(host?.textContent).toContain(secondTask.title);
    expect(host?.textContent).toContain("运行 ID #100");
    expect(host?.textContent).toContain("区间 ID mixed-dense-9");
  });

  it("merges adjacent density buckets when their hit areas overlap", () => {
    const nowMs = Date.parse("2026-10-02T00:00:00.000Z");
    const chartWidth = 354;
    const windowStart = nowMs - 12 * 60 * 60 * 1000;
    const startAtPixel = (pixel: number) =>
      windowStart + (pixel / chartWidth) * 12 * 60 * 60 * 1000;

    renderChart({
      nowMs,
      chartWidth,
      executions: [100.2, 100.8, 101.2, 101.8].map((pixel, index) => {
        const start = startAtPixel(pixel);
        return segment(`adjacent-dense-${index}`, start, start + 30);
      }),
    });

    const groups = Array.from(host?.querySelectorAll<SVGGElement>('g[role="button"]') ?? []);
    expect(groups).toHaveLength(1);
    expect(groups[0].getAttribute("aria-label")).toContain("4 次");

    act(() => groups[0].dispatchEvent(new MouseEvent("click", { bubbles: true })));

    const details = Array.from(host?.querySelectorAll("ul.divide-y li") ?? []);
    const identities = details.map((item) => item.textContent?.match(/区间 ID ([^\s]+)/)?.[1]);
    expect(details).toHaveLength(4);
    expect(new Set(identities).size).toBe(4);
    expect(host?.textContent).toContain("区间 ID adjacent-dense-3");
  });
});
