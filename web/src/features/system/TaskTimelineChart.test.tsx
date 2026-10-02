/** @vitest-environment jsdom */

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { afterEach, describe, expect, it } from "vitest";
import type { CurrentTaskExecution, ManagedTask, TaskTimelineSegment } from "../../lib/api";
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

let host: HTMLDivElement | null = null;
let root: Root | null = null;
let scrollWidthDescriptor: PropertyDescriptor | undefined;
let clientWidthDescriptor: PropertyDescriptor | undefined;

function renderChart(props: {
  nowMs: number;
  executions?: TaskTimelineSegment[];
  activeRuns?: CurrentTaskExecution[];
  runtimeReceivedAt?: number;
}) {
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
            coverage={[]}
            nowMs={nowMs}
            runtimeObservedAt={new Date(nowMs).toISOString()}
            runtimeReceivedAt={props.runtimeReceivedAt ?? performance.now()}
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
  if (scrollWidthDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "scrollWidth", scrollWidthDescriptor);
  } else {
    Reflect.deleteProperty(HTMLElement.prototype, "scrollWidth");
  }
  if (clientWidthDescriptor) {
    Object.defineProperty(HTMLElement.prototype, "clientWidth", clientWidthDescriptor);
  } else {
    Reflect.deleteProperty(HTMLElement.prototype, "clientWidth");
  }
  scrollWidthDescriptor = undefined;
  clientWidthDescriptor = undefined;
  host = null;
  root = null;
});

describe("TaskTimelineChart", () => {
  it("opens the timeline at the newest end on narrow screens", () => {
    scrollWidthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "scrollWidth");
    clientWidthDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "clientWidth");
    Object.defineProperty(HTMLElement.prototype, "scrollWidth", {
      configurable: true,
      get: () => 1_200,
    });
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get: () => 393,
    });

    renderChart({ nowMs: Date.parse("2026-10-02T00:00:00.000Z") });
    const scroller = host?.querySelector<HTMLDivElement>(
      "[data-testid='task-timeline-scroll-container']",
    );
    expect(scroller?.scrollLeft).toBe(807);
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
      runtimeReceivedAt: performance.now() - 7_000,
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
    expect(host?.querySelector("svg title")?.textContent).toContain("最近 24 小时");
    expect(
      Array.from(host?.querySelectorAll("svg rect title") ?? []).map((item) => item.textContent),
    ).toContain("maintenance_store_write_unavailable");
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
