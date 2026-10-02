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
  const render = (nowMs: number) =>
    act(() =>
      root?.render(
        <MemoryRouter>
          <TaskTimelineChart
            tasks={[task]}
            executions={props.executions ?? []}
            activeRuns={props.activeRuns ?? []}
            coverage={props.coverage ?? []}
            nowMs={nowMs}
            runtimeFresh={props.runtimeFresh ?? true}
            runtimeBoundaryMs={props.runtimeBoundaryMs ?? nowMs}
            runtimeObservedAt={new Date(nowMs).toISOString()}
          />
        </MemoryRouter>,
      ),
    );
  render(props.nowMs);
  return { render };
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

  it("keeps overlapping pressure causes in one shared chart row", () => {
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
    expect(bars[0].getAttribute("aria-label")).toContain("压力冷却让行");
    expect(heights).toBeGreaterThan(0);
    expect(host?.querySelector('[data-testid="task-timeline-row-labels"]')?.textContent).toContain(
      "准入 / 压力",
    );
  });

  it("marks failed executions separately and exposes complete timing details", () => {
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

    act(() => bar?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    expect(host?.textContent).toContain("触发：manual");
    expect(host?.textContent).toContain("实际开始：");
    expect(host?.textContent).toContain("实际结束：");
    expect(host?.textContent).toContain("实际用时：1 分 00 秒");
    expect(host?.textContent).toContain("结果：失败");
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

  it("stops extrapolating a runtime run after its observation expires", () => {
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
    expect(host?.querySelectorAll("ul.divide-y li")).toHaveLength(8);
  });
});
