import { type CSSProperties, type JSX, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import type {
  CurrentTaskExecution,
  ManagedTask,
  TaskTimelineCoverage,
  TaskTimelineSegment,
} from "../../lib/api";
import { managedTaskColor } from "./managedTaskColor";

const HOUR_MS = 60 * 60 * 1000;
const TIMELINE_WINDOW_MS = 12 * HOUR_MS;
const TIME_AXIS_HOURS = [0, 3, 6, 9, 12];
const DEFAULT_CHART_WIDTH = 1200;
const AXIS_LABEL_WIDTH = 70;
const AXIS_HEIGHT = 34;
const LANE_HEIGHT = 17;
const MIN_BAR_WIDTH = 3;
const COVERAGE_HEARTBEAT_MAX_AGE_MS = 60_000;

type ExecutionBar = {
  segment: TaskTimelineSegment;
  startMs: number;
  endMs: number;
  lane: number;
  active: boolean;
};

type Band = {
  startMs: number;
  endMs: number;
  status: string;
  lane: number;
  segment?: TaskTimelineSegment;
};

type DensityGroup = {
  key: string;
  taskKey: string;
  lane: number;
  members: ExecutionBar[];
  startMs: number;
  endMs: number;
};

function timestamp(value: string | null | undefined, fallback: number): number {
  const parsed = value ? Date.parse(value) : Number.NaN;
  return Number.isFinite(parsed) ? parsed : fallback;
}

function compactTime(value: number): string {
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    timeZone: "Asia/Shanghai",
  }).format(new Date(value));
}

function exactTime(value: number): string {
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
    timeZone: "Asia/Shanghai",
  }).format(new Date(value));
}

function durationLabel(durationMs: number): string {
  const seconds = Math.max(0, Math.round(durationMs / 1000));
  if (seconds < 60) return `${seconds} 秒`;
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  if (minutes < 60) return `${minutes} 分 ${String(remainingSeconds).padStart(2, "0")} 秒`;
  return `${Math.floor(minutes / 60)} 小时 ${String(minutes % 60).padStart(2, "0")} 分`;
}

function outcomeLabel(status: string): string {
  switch (status) {
    case "success":
      return "成功";
    case "failed":
      return "失败";
    case "skipped":
      return "已跳过";
    case "interrupted":
      return "中断，结束时间未知";
    case "cancelled":
      return "已取消";
    case "running":
      return "进行中";
    case "released":
      return "已恢复";
    case "waiting":
      return "等待中";
    default:
      return "未知";
  }
}

function outcomeAppearance(
  status: string,
  dark: boolean,
): { stroke: string; dashArray?: string } | null {
  switch (status) {
    case "success":
      return null;
    case "running":
      return { stroke: dark ? "#22d3ee" : "#0e7490" };
    case "failed":
      return { stroke: dark ? "#fb7185" : "#be123c", dashArray: "2 1" };
    case "interrupted":
      return { stroke: dark ? "#cbd5e1" : "#475569", dashArray: "3 2" };
    case "skipped":
    case "cancelled":
      return { stroke: dark ? "#60a5fa" : "#2563eb", dashArray: "1 2" };
    case "unknown":
      return { stroke: dark ? "#cbd5e1" : "#64748b", dashArray: "4 2" };
    default:
      return { stroke: dark ? "#cbd5e1" : "#64748b", dashArray: "4 2" };
  }
}

function aggregateOutcome(members: ExecutionBar[]): {
  status: string;
  summary: string;
} {
  const counts = new Map<string, number>();
  for (const member of members) {
    const status = member.segment.status;
    counts.set(status, (counts.get(status) ?? 0) + 1);
  }
  const summary = [...counts]
    .map(([status, count]) => `${outcomeLabel(status)} ${count}`)
    .join("，");
  const priority = ["failed", "interrupted", "unknown", "running", "cancelled", "skipped"];
  return {
    status: priority.find((status) => counts.has(status)) ?? "success",
    summary,
  };
}

function reasonLabel(reason: string | null | undefined): string {
  switch (reason) {
    case "pressure_cooldown":
      return "压力冷却让行";
    case "resource_busy":
      return "资源占用等待";
    default:
      return reason || "原因未知";
  }
}

function packLanes<T extends { startMs: number; endMs: number }>(
  items: T[],
): Array<T & { lane: number }> {
  const ends: number[] = [];
  return [...items]
    .sort((left, right) => left.startMs - right.startMs || left.endMs - right.endMs)
    .map((item) => {
      let lane = ends.findIndex((end) => end <= item.startMs);
      if (lane < 0) lane = ends.length;
      ends[lane] = item.endMs;
      return { ...item, lane };
    });
}

function mergeCoverage(
  coverage: TaskTimelineCoverage[],
  nowMs: number,
): Array<{ startMs: number; endMs: number; status: "normal" | "gap" }> {
  const intervals = coverage
    .map((item) => {
      const startMs = timestamp(item.startedAt, nowMs);
      const seenMs = timestamp(item.lastSeenAt, startMs);
      const activeCoverage = !item.endedAt && nowMs - seenMs <= COVERAGE_HEARTBEAT_MAX_AGE_MS;
      const endMs = item.endedAt
        ? timestamp(item.endedAt, seenMs)
        : activeCoverage
          ? nowMs
          : seenMs;
      return {
        startMs,
        endMs,
        status: "normal" as const,
      };
    })
    .filter((item) => item.endMs > item.startMs)
    .sort((left, right) => left.startMs - right.startMs);
  const merged: Array<{
    startMs: number;
    endMs: number;
    status: "normal" | "gap";
  }> = [];
  for (const interval of intervals) {
    const last = merged.at(-1);
    if (!last || interval.startMs > last.endMs || interval.status !== last.status) {
      merged.push(interval);
    } else {
      last.endMs = Math.max(last.endMs, interval.endMs);
    }
  }
  return merged;
}

function executionTitle(bar: ExecutionBar): string {
  const segment = bar.segment;
  const end = segment.finishedAt ? timestamp(segment.finishedAt, bar.endMs) : bar.endMs;
  const duration =
    segment.durationMs != null
      ? durationLabel(segment.durationMs)
      : bar.active
        ? `${durationLabel(bar.endMs - bar.startMs)}（进行中）`
        : "未知";
  return [
    segment.title,
    `触发：${segment.triggerKind ?? "未知"}`,
    `开始：${exactTime(bar.startMs)}`,
    `结束：${segment.finishedAt ? exactTime(end) : bar.active ? "进行中" : `未确认，最后观测于 ${exactTime(end)}`}`,
    `用时：${duration}`,
    `结果：${outcomeLabel(segment.status)}`,
    segment.activeChildTitle ? `当前子阶段：${segment.activeChildTitle}` : null,
  ]
    .filter(Boolean)
    .join("；");
}

export function TaskTimelineChart({
  tasks,
  executions,
  activeRuns,
  coverage,
  nowMs,
  runtimeFresh,
  runtimeBoundaryMs,
  runtimeObservedAt,
}: {
  tasks: ManagedTask[];
  executions: TaskTimelineSegment[];
  activeRuns: CurrentTaskExecution[];
  coverage: TaskTimelineCoverage[];
  nowMs: number;
  runtimeFresh: boolean;
  runtimeBoundaryMs: number;
  runtimeObservedAt: string | null;
}): JSX.Element {
  const [selectedExecutions, setSelectedExecutions] = useState<ExecutionBar[] | null>(null);
  const [selectedDeferral, setSelectedDeferral] = useState<TaskTimelineSegment | null>(null);
  const timelineContainerRef = useRef<HTMLDivElement>(null);
  const [chartWidth, setChartWidth] = useState(DEFAULT_CHART_WIDTH);
  const dark =
    typeof document !== "undefined" &&
    (document.documentElement.getAttribute("data-color-mode") === "dark" ||
      document.documentElement.getAttribute("data-theme") === "vibe-dark");
  const windowStart = nowMs - TIMELINE_WINDOW_MS;
  const taskByKey = useMemo(() => new Map(tasks.map((task) => [task.taskKey, task])), [tasks]);

  useEffect(() => {
    const container = timelineContainerRef.current;
    if (!container) return;

    const updateWidth = (width: number) => {
      if (width > 0) setChartWidth(Math.max(1, Math.floor(width)));
    };
    updateWidth(container.getBoundingClientRect().width);

    if (typeof ResizeObserver === "undefined") {
      const handleResize = () => updateWidth(container.getBoundingClientRect().width);
      window.addEventListener("resize", handleResize);
      return () => window.removeEventListener("resize", handleResize);
    }

    const observer = new ResizeObserver((entries) => {
      const entry = entries.at(-1);
      if (entry) updateWidth(entry.contentRect.width);
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  const executionBars = useMemo(() => {
    const byUid = new Map<string, TaskTimelineSegment>();
    for (const segment of executions) {
      if (segment.kind === "execution") byUid.set(segment.segmentId, segment);
    }
    for (const run of activeRuns) {
      const prior = byUid.get(run.executionUid);
      if (prior && prior.status !== "running") continue;
      const observedAt = Math.max(
        timestamp(runtimeObservedAt, timestamp(run.startedAt, nowMs)),
        timestamp(prior?.lastObservedAt, timestamp(run.startedAt, nowMs)),
      );
      byUid.set(run.executionUid, {
        segmentId: run.executionUid,
        kind: "execution",
        taskKey: run.taskKey,
        title: run.title,
        startedAt: run.startedAt,
        lastObservedAt: new Date(observedAt).toISOString(),
        finishedAt: null,
        durationMs: null,
        status: "running",
        triggerKind: run.triggerKind,
        executionClass: run.executionClass ?? null,
        reason: null,
        retryAt: null,
        activeChildTaskKey: run.activeChildTaskKey ?? prior?.activeChildTaskKey ?? null,
        activeChildTitle: run.activeChildTitle ?? prior?.activeChildTitle ?? null,
        managedRunId: prior?.managedRunId ?? null,
        sessionId: prior?.sessionId ?? "runtime",
        revision: prior?.revision ?? 0,
      });
    }

    const currentUids = new Set(activeRuns.map((run) => run.executionUid));
    const activeUids = new Set(runtimeFresh ? activeRuns.map((run) => run.executionUid) : []);
    const packed = packLanes(
      [...byUid.values()]
        .map((segment) => {
          const startMs = timestamp(segment.startedAt, nowMs);
          const isActive = activeUids.has(segment.segmentId);
          const displaySegment =
            !isActive && segment.status === "running" && !segment.finishedAt
              ? { ...segment, status: "unknown" }
              : segment;
          const endedMs = segment.finishedAt
            ? timestamp(segment.finishedAt, nowMs)
            : isActive
              ? nowMs
              : currentUids.has(segment.segmentId)
                ? Math.min(nowMs, runtimeBoundaryMs)
                : timestamp(segment.lastObservedAt, startMs);
          return {
            segment: displaySegment,
            active: isActive,
            startMs: Math.max(windowStart, startMs),
            endMs: Math.min(nowMs, Math.max(startMs, endedMs)),
            realStartMs: startMs,
          };
        })
        .filter((item) => item.endMs >= windowStart && item.realStartMs <= nowMs)
        .map(({ realStartMs: _realStartMs, ...item }) => item),
    );
    return packed.map((item) => ({
      ...item,
      startMs: Math.max(windowStart, item.startMs),
    }));
  }, [
    activeRuns,
    executions,
    nowMs,
    runtimeBoundaryMs,
    runtimeFresh,
    runtimeObservedAt,
    windowStart,
  ]);

  const deferrals = useMemo(
    () => executions.filter((segment) => segment.kind === "deferral"),
    [executions],
  );
  const explicitCoverageGaps = useMemo(
    () => executions.filter((segment) => segment.kind === "coverage_gap"),
    [executions],
  );
  const knownCoverage = useMemo(() => mergeCoverage(coverage, nowMs), [coverage, nowMs]);
  const coverageBands: Band[] = [];
  let cursor = windowStart;
  for (const interval of knownCoverage) {
    const startMs = Math.max(windowStart, interval.startMs);
    const endMs = Math.min(nowMs, interval.endMs);
    if (startMs > cursor)
      coverageBands.push({
        startMs: cursor,
        endMs: startMs,
        status: "gap",
        lane: 0,
      });
    if (endMs > startMs) coverageBands.push({ startMs, endMs, status: interval.status, lane: 0 });
    cursor = Math.max(cursor, endMs);
  }
  if (cursor < nowMs)
    coverageBands.push({
      startMs: cursor,
      endMs: nowMs,
      status: "gap",
      lane: 0,
    });

  const positionedDeferrals = deferrals
    .map((segment) => {
      const startMs = timestamp(segment.startedAt, nowMs);
      const active = segment.status === "waiting";
      const endMs = segment.finishedAt
        ? timestamp(segment.finishedAt, nowMs)
        : active && runtimeFresh
          ? nowMs
          : active
            ? Math.min(nowMs, runtimeBoundaryMs)
            : timestamp(segment.lastObservedAt, startMs);
      return {
        segment,
        startMs: Math.max(windowStart, startMs),
        endMs: Math.min(nowMs, endMs),
      };
    })
    .filter((item) => item.endMs >= windowStart && item.startMs <= nowMs);
  const visibleCoverageBands: Band[] = coverageBands.flatMap((band): Band[] => {
    if (band.status === "gap") return [band];
    let clearIntervals = [{ startMs: band.startMs, endMs: band.endMs }];
    const unknownIntervals = [
      ...positionedDeferrals.map(({ startMs, endMs }) => ({ startMs, endMs })),
      ...explicitCoverageGaps.map((gap) => ({
        startMs: Math.max(windowStart, timestamp(gap.startedAt, nowMs)),
        endMs: Math.min(nowMs, timestamp(gap.finishedAt, timestamp(gap.lastObservedAt, nowMs))),
      })),
    ];
    for (const deferral of unknownIntervals) {
      const next: Array<{ startMs: number; endMs: number }> = [];
      for (const interval of clearIntervals) {
        if (deferral.endMs <= interval.startMs || deferral.startMs >= interval.endMs) {
          next.push(interval);
          continue;
        }
        if (deferral.startMs > interval.startMs) {
          next.push({ startMs: interval.startMs, endMs: deferral.startMs });
        }
        if (deferral.endMs < interval.endMs) {
          next.push({ startMs: deferral.endMs, endMs: interval.endMs });
        }
      }
      clearIntervals = next;
    }
    return clearIntervals.map((interval) => ({
      ...interval,
      status: "normal",
      lane: 0,
    }));
  });
  visibleCoverageBands.push(
    ...explicitCoverageGaps.map((gap) => ({
      startMs: Math.max(windowStart, timestamp(gap.startedAt, nowMs)),
      endMs: Math.min(nowMs, timestamp(gap.finishedAt, timestamp(gap.lastObservedAt, nowMs))),
      status: "gap",
      lane: 0,
      segment: gap,
    })),
  );

  const laneCount = Math.max(1, ...executionBars.map((item) => item.lane + 1));
  const pressureTop = AXIS_HEIGHT + laneCount * LANE_HEIGHT + 18;
  const chartHeight = pressureTop + 12 + 12;
  const x = (time: number) => ((time - windowStart) / TIMELINE_WINDOW_MS) * chartWidth;
  const timeAxisHours =
    chartWidth < 220 ? [0, 12] : chartWidth < 520 ? [0, 6, 12] : TIME_AXIS_HOURS;
  const executionDensityGroups = new Map<string, DensityGroup>();
  const singleExecutionBars: ExecutionBar[] = [];
  for (const bar of executionBars) {
    const width = Math.max(MIN_BAR_WIDTH, x(bar.endMs) - x(bar.startMs));
    if (width > 4) {
      singleExecutionBars.push(bar);
      continue;
    }
    const densityKey = `${bar.lane}:${bar.segment.taskKey}:${Math.floor(x(bar.startMs))}`;
    const group = executionDensityGroups.get(densityKey);
    if (group) {
      group.members.push(bar);
      group.startMs = Math.min(group.startMs, bar.startMs);
      group.endMs = Math.max(group.endMs, bar.endMs);
    } else {
      executionDensityGroups.set(densityKey, {
        key: densityKey,
        taskKey: bar.segment.taskKey,
        lane: bar.lane,
        members: [bar],
        startMs: bar.startMs,
        endMs: bar.endMs,
      });
    }
  }
  const densityGroups = [...executionDensityGroups.values()].filter(
    (group) => group.members.length > 1,
  );
  for (const group of executionDensityGroups.values()) {
    if (group.members.length === 1) singleExecutionBars.push(group.members[0]);
  }

  const selectedDescription = selectedExecutions
    ? `${selectedExecutions.length} 次任务执行`
    : selectedDeferral
      ? reasonLabel(selectedDeferral.reason)
      : null;

  return (
    <section
      aria-labelledby="task-timeline-heading"
      className="space-y-3"
      data-testid="task-timeline"
    >
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div>
          <h3 id="task-timeline-heading" className="text-lg font-semibold">
            最近 12 小时
          </h3>
          <p className="text-sm text-base-content/60">执行结果与任务让行记录</p>
        </div>
        <time
          className="font-mono text-xs tabular-nums text-base-content/60"
          data-testid="task-timeline-now"
          dateTime={new Date(nowMs).toISOString()}
        >
          当前时间 {exactTime(nowMs)}
        </time>
        <ul
          className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-base-content/65"
          aria-label="时间图图例"
        >
          <li className="font-medium text-base-content/75">执行结果</li>
          <li className="inline-flex items-center gap-1.5">
            <i className="size-2.5 rounded-sm bg-base-content/55" />
            成功
          </li>
          <li className="inline-flex items-center gap-1.5">
            <i
              className="size-2.5 rounded-sm border"
              style={{ borderColor: dark ? "#22d3ee" : "#0e7490" }}
            />
            进行中
          </li>
          <li className="inline-flex items-center gap-1.5">
            <i
              className="size-2.5 rounded-sm border border-dashed"
              style={{ borderColor: dark ? "#fb7185" : "#be123c" }}
            />
            失败
          </li>
          <li className="inline-flex items-center gap-1.5">
            <i
              className="size-2.5 rounded-sm border border-dashed"
              style={{ borderColor: dark ? "#cbd5e1" : "#475569" }}
            />
            中断 / 未知
          </li>
          <li className="inline-flex items-center gap-1.5">
            <i
              className="size-2.5 rounded-sm border border-dotted"
              style={{ borderColor: dark ? "#60a5fa" : "#2563eb" }}
            />
            取消 / 跳过
          </li>
          <li className="font-medium text-base-content/75">准入 / 压力</li>
          <li className="inline-flex items-center gap-1.5">
            <i className="size-2 rounded-full bg-emerald-600" />
            正常
          </li>
          <li className="inline-flex items-center gap-1.5">
            <i className="size-2 rounded-full bg-amber-500" />
            资源占用
          </li>
          <li className="inline-flex items-center gap-1.5">
            <i className="size-2 rounded-full bg-rose-600" />
            压力让行
          </li>
          <li className="inline-flex items-center gap-1.5">
            <i className="size-2 rounded-full bg-slate-400" />
            观测缺口
          </li>
        </ul>
      </div>
      <div className="grid grid-cols-1 items-start border-y border-base-300/70 sm:grid-cols-[6.5rem_minmax(0,1fr)]">
        <div
          className="sr-only pt-9 text-xs text-base-content/65 sm:not-sr-only"
          data-testid="task-timeline-row-labels"
        >
          <div className="flex h-[var(--task-chart-execution-height)] items-start pt-1">
            任务执行
          </div>
          <div className="mt-[18px] pr-1 leading-tight">准入 / 压力</div>
        </div>
        <div ref={timelineContainerRef} className="min-w-0" data-testid="task-timeline-viewport">
          <svg
            aria-label="任务执行和任务让行的最近 12 小时时间图"
            className="block w-full text-base-content/55"
            height={chartHeight}
            role="group"
            viewBox={`0 0 ${chartWidth} ${chartHeight}`}
            width={chartWidth}
            style={
              {
                "--task-chart-execution-height": `${laneCount * LANE_HEIGHT}px`,
              } as CSSProperties
            }
          >
            <title>最近 12 小时任务执行时间图</title>
            {timeAxisHours.map((hour) => {
              const lineX = (hour / 12) * chartWidth;
              const labelX =
                chartWidth < 520
                  ? hour === 0
                    ? 4
                    : hour === 12
                      ? chartWidth - AXIS_LABEL_WIDTH
                      : lineX - AXIS_LABEL_WIDTH / 2
                  : Math.max(4, Math.min(lineX + 4, chartWidth - AXIS_LABEL_WIDTH));
              const labelTime = windowStart + hour * HOUR_MS;
              return (
                <g key={hour}>
                  <line
                    x1={lineX}
                    x2={lineX}
                    y1={AXIS_HEIGHT}
                    y2={pressureTop + 12}
                    stroke="currentColor"
                    strokeOpacity="0.14"
                  />
                  <text x={labelX} y={20} fill="currentColor" fontSize="11">
                    {compactTime(labelTime)}
                  </text>
                </g>
              );
            })}
            {Array.from(
              { length: laneCount },
              (_, lane) => AXIS_HEIGHT + (lane + 1) * LANE_HEIGHT - 1,
            ).map((lineY) => (
              <line
                key={`lane-${lineY}`}
                x1={0}
                x2={chartWidth}
                y1={lineY}
                y2={lineY}
                stroke="currentColor"
                strokeOpacity="0.08"
              />
            ))}
            {visibleCoverageBands.map((band) => (
              <rect
                key={`coverage-${band.startMs}-${band.endMs}-${band.status}-${band.segment?.segmentId ?? ""}`}
                x={x(band.startMs)}
                y={pressureTop}
                width={Math.max(1, x(band.endMs) - x(band.startMs))}
                height={12}
                fill={band.status === "normal" ? "#16a34a" : "#64748b"}
                fillOpacity={band.status === "normal" ? 0.2 : 0.28}
              >
                <title>
                  {band.status === "normal"
                    ? "有记录器覆盖且未观测到任务让行"
                    : (band.segment?.reason ?? "观测缺口")}
                </title>
              </rect>
            ))}
            {singleExecutionBars.map((bar) => {
              const barX = x(bar.startMs);
              const barY = AXIS_HEIGHT + bar.lane * LANE_HEIGHT + 3;
              const width = Math.max(MIN_BAR_WIDTH, x(bar.endMs) - barX);
              const color = managedTaskColor(taskByKey.get(bar.segment.taskKey), dark);
              const appearance = outcomeAppearance(bar.segment.status, dark);
              return (
                <g
                  key={bar.segment.segmentId}
                  aria-label={executionTitle(bar)}
                  className="cursor-pointer outline-none focus-visible:opacity-75"
                  onClick={() => {
                    setSelectedExecutions([bar]);
                    setSelectedDeferral(null);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" || event.key === " ") {
                      event.preventDefault();
                      setSelectedExecutions([bar]);
                      setSelectedDeferral(null);
                    }
                  }}
                  role="button"
                  tabIndex={0}
                >
                  <rect
                    x={barX}
                    y={barY}
                    width={width}
                    height={11}
                    rx={2}
                    fill={color}
                    fillOpacity={bar.active ? 1 : 0.82}
                    stroke={appearance?.stroke ?? (bar.active ? color : "none")}
                    strokeDasharray={appearance?.dashArray}
                    strokeWidth={appearance ? 1.5 : bar.active ? 1 : 0}
                  />
                  <title>{executionTitle(bar)}</title>
                </g>
              );
            })}
            {densityGroups.map((group) => {
              const barX = x(group.startMs);
              const barY = AXIS_HEIGHT + group.lane * LANE_HEIGHT + 3;
              const width = Math.max(7, x(group.endMs) - barX);
              const color = managedTaskColor(taskByKey.get(group.taskKey), dark);
              const outcome = aggregateOutcome(group.members);
              const appearance = outcomeAppearance(outcome.status, dark);
              return (
                <g
                  key={group.key}
                  aria-label={`${group.members.length} 次${taskByKey.get(group.taskKey)?.title ?? group.taskKey}执行；${outcome.summary}；选择查看详情`}
                  className="cursor-pointer outline-none focus-visible:opacity-75"
                  onClick={() => {
                    setSelectedExecutions(group.members);
                    setSelectedDeferral(null);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" || event.key === " ") {
                      event.preventDefault();
                      setSelectedExecutions(group.members);
                      setSelectedDeferral(null);
                    }
                  }}
                  role="button"
                  tabIndex={0}
                >
                  <rect
                    x={barX}
                    y={barY}
                    width={width}
                    height={11}
                    rx={2}
                    fill={color}
                    stroke={appearance?.stroke ?? "none"}
                    strokeDasharray={appearance?.dashArray}
                    strokeWidth={appearance ? 1.5 : 0}
                  />
                  <text x={barX + width + 2} y={barY + 9} fill="currentColor" fontSize="9">
                    ×{group.members.length}
                  </text>
                  <title>{`${group.members.length} 次密集执行；选择查看每次执行`}</title>
                </g>
              );
            })}
            {positionedDeferrals.map(({ segment, startMs, endMs }) => {
              const resourceBusy = segment.reason === "resource_busy";
              const color = resourceBusy ? "#d08700" : "#d63850";
              const bandY = pressureTop + 2;
              const bandX = x(startMs);
              const width = Math.max(MIN_BAR_WIDTH, x(endMs) - bandX);
              const overlapping = positionedDeferrals.filter(
                (other) =>
                  other.segment.segmentId !== segment.segmentId &&
                  other.startMs < endMs &&
                  other.endMs > startMs,
              );
              const detail = (item: (typeof positionedDeferrals)[number]) =>
                `${item.segment.title}；${reasonLabel(item.segment.reason)}；开始 ${exactTime(item.startMs)}；${item.segment.finishedAt ? `恢复 ${exactTime(timestamp(item.segment.finishedAt, item.endMs))}` : "尚未确认恢复"}${item.segment.retryAt ? `；重试时间 ${exactTime(timestamp(item.segment.retryAt, item.endMs))}` : "；重试时间未知"}`;
              const title = [
                positionedDeferrals.find((item) => item.segment.segmentId === segment.segmentId),
                ...overlapping,
              ]
                .filter((item): item is (typeof positionedDeferrals)[number] => item != null)
                .map(detail)
                .join("\n");
              return (
                <g
                  key={segment.segmentId}
                  aria-label={title}
                  className="cursor-pointer outline-none focus-visible:opacity-75"
                  onClick={() => {
                    setSelectedDeferral(segment);
                    setSelectedExecutions(null);
                  }}
                  onKeyDown={(event) => {
                    if (event.key === "Enter" || event.key === " ") {
                      event.preventDefault();
                      setSelectedDeferral(segment);
                      setSelectedExecutions(null);
                    }
                  }}
                  role="button"
                  tabIndex={0}
                >
                  <rect
                    x={bandX}
                    y={bandY}
                    width={width}
                    height={8}
                    rx={2}
                    fill={color}
                    fillOpacity={overlapping.length ? 0.55 : 1}
                    stroke={color}
                    strokeWidth={1}
                  />
                  <title>{title}</title>
                </g>
              );
            })}
            <line
              x1={chartWidth}
              x2={chartWidth}
              y1={AXIS_HEIGHT}
              y2={chartHeight - 4}
              stroke="#dc3545"
              strokeDasharray="3 3"
            />
          </svg>
        </div>
      </div>
      {selectedExecutions ? (
        <div className="space-y-2 border-b border-base-300/60 pb-3" aria-live="polite">
          <div className="flex items-center justify-between gap-2">
            <h4 className="font-medium">{selectedDescription}</h4>
            <button
              type="button"
              className="text-sm text-primary hover:underline"
              onClick={() => setSelectedExecutions(null)}
            >
              收起
            </button>
          </div>
          <ul className="divide-y divide-base-300/50" aria-label="执行记录详情">
            {selectedExecutions.map((bar) => (
              <li
                key={bar.segment.segmentId}
                className="grid gap-1 py-2 text-sm sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center"
              >
                <div className="min-w-0">
                  <Link
                    className="flex min-w-0 items-center gap-2 font-medium hover:underline"
                    to={`/system/tasks/${encodeURIComponent(bar.segment.taskKey)}`}
                  >
                    <span
                      className="size-2 shrink-0 rounded-full"
                      style={{
                        backgroundColor: managedTaskColor(taskByKey.get(bar.segment.taskKey), dark),
                      }}
                    />
                    <span className="truncate">{bar.segment.title}</span>
                  </Link>
                  <div className="mt-1 text-xs leading-relaxed text-base-content/65">
                    触发：{bar.segment.triggerKind ?? "未知"} · 实际开始：
                    {exactTime(timestamp(bar.segment.startedAt, nowMs))} · 实际结束：
                    {bar.segment.finishedAt
                      ? exactTime(timestamp(bar.segment.finishedAt, nowMs))
                      : bar.active
                        ? "进行中"
                        : `未确认（最后观测于 ${exactTime(timestamp(bar.segment.lastObservedAt, nowMs))}）`}
                    {" · "}实际用时：
                    {bar.segment.durationMs != null
                      ? durationLabel(bar.segment.durationMs)
                      : bar.active
                        ? `${durationLabel(bar.endMs - bar.startMs)}（进行中）`
                        : "未知"}
                  </div>
                </div>
                <span className="text-xs text-base-content/65">
                  结果：{outcomeLabel(bar.segment.status)}
                </span>
              </li>
            ))}
          </ul>
        </div>
      ) : selectedDeferral ? (
        <div
          className="flex flex-wrap items-center justify-between gap-2 border-b border-base-300/60 pb-3 text-sm"
          aria-live="polite"
        >
          <div>
            <strong>{selectedDeferral.title}</strong>
            <span className="ml-2 text-base-content/65">
              {reasonLabel(selectedDeferral.reason)}
            </span>
            <div className="mt-1 text-xs text-base-content/60">{selectedDeferral.taskKey}</div>
            <div className="mt-1 text-xs text-base-content/60">
              开始：{exactTime(timestamp(selectedDeferral.startedAt, nowMs))} · 恢复：
              {selectedDeferral.finishedAt
                ? exactTime(timestamp(selectedDeferral.finishedAt, nowMs))
                : "尚未确认"}
              {" · "}重试时间：
              {selectedDeferral.retryAt
                ? exactTime(timestamp(selectedDeferral.retryAt, nowMs))
                : "未知"}
            </div>
          </div>
          <span className="text-xs text-base-content/65">
            受影响任务：{selectedDeferral.title}；恢复条件：
            {selectedDeferral.finishedAt
              ? `已于 ${compactTime(timestamp(selectedDeferral.finishedAt, nowMs))} 恢复`
              : selectedDeferral.retryAt
                ? `预计 ${compactTime(timestamp(selectedDeferral.retryAt, nowMs))} 后重新准入`
                : selectedDeferral.status === "waiting"
                  ? "等待后端确认资源释放或压力恢复"
                  : "恢复时间未知"}
          </span>
          <button
            type="button"
            className="text-sm text-primary hover:underline"
            onClick={() => setSelectedDeferral(null)}
          >
            收起
          </button>
        </div>
      ) : null}
    </section>
  );
}
