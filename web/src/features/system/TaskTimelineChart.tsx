import { type CSSProperties, type JSX, useEffect, useMemo, useRef, useState } from "react";
import { Link } from "react-router-dom";
import type {
  CurrentTaskExecution,
  ManagedTask,
  TaskTimelineCoverage,
  TaskTimelineSegment,
} from "../../lib/api";
import { managedTaskColor } from "./managedTaskColor";

const DAY_MS = 24 * 60 * 60 * 1000;
const CHART_WIDTH = 1200;
const AXIS_HEIGHT = 34;
const LANE_HEIGHT = 17;
const MIN_BAR_WIDTH = 3;
const ACTIVE_MAX_AGE_MS = 6_000;
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
        status: item.droppedEvents > 0 ? ("gap" as const) : ("normal" as const),
      };
    })
    .filter((item) => item.endMs > item.startMs)
    .sort((left, right) => left.startMs - right.startMs);
  const merged: Array<{ startMs: number; endMs: number; status: "normal" | "gap" }> = [];
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
      ? `${Math.round(segment.durationMs / 1000)} 秒`
      : bar.active
        ? `${Math.round((bar.endMs - bar.startMs) / 1000)} 秒（进行中）`
        : "未知";
  return [
    segment.title,
    `触发：${segment.triggerKind ?? "未知"}`,
    `开始：${compactTime(bar.startMs)}`,
    `结束：${segment.finishedAt ? compactTime(end) : bar.active ? "进行中" : `最后确认于 ${compactTime(end)}`}`,
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
  runtimeObservedAt,
  runtimeReceivedAt,
}: {
  tasks: ManagedTask[];
  executions: TaskTimelineSegment[];
  activeRuns: CurrentTaskExecution[];
  coverage: TaskTimelineCoverage[];
  nowMs: number;
  runtimeObservedAt: string | null;
  runtimeReceivedAt: number | null;
}): JSX.Element {
  const [selectedExecutions, setSelectedExecutions] = useState<ExecutionBar[] | null>(null);
  const [selectedDeferral, setSelectedDeferral] = useState<TaskTimelineSegment | null>(null);
  const timelineScrollerRef = useRef<HTMLDivElement>(null);
  const dark =
    typeof document !== "undefined" &&
    (document.documentElement.getAttribute("data-color-mode") === "dark" ||
      document.documentElement.getAttribute("data-theme") === "vibe-dark");
  const windowStart = nowMs - DAY_MS;
  const runtimeAgeMs =
    runtimeReceivedAt == null
      ? Number.POSITIVE_INFINITY
      : Math.max(0, performance.now() - runtimeReceivedAt);
  const runtimeFresh = runtimeAgeMs <= ACTIVE_MAX_AGE_MS;
  const taskByKey = useMemo(() => new Map(tasks.map((task) => [task.taskKey, task])), [tasks]);

  useEffect(() => {
    const scroller = timelineScrollerRef.current;
    if (scroller) scroller.scrollLeft = Math.max(0, scroller.scrollWidth - scroller.clientWidth);
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
    return packed.map((item) => ({ ...item, startMs: Math.max(windowStart, item.startMs) }));
  }, [activeRuns, executions, nowMs, runtimeFresh, runtimeObservedAt, windowStart]);

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
      coverageBands.push({ startMs: cursor, endMs: startMs, status: "gap", lane: 0 });
    if (endMs > startMs) coverageBands.push({ startMs, endMs, status: interval.status, lane: 0 });
    cursor = Math.max(cursor, endMs);
  }
  if (cursor < nowMs) coverageBands.push({ startMs: cursor, endMs: nowMs, status: "gap", lane: 0 });

  const packedDeferrals = packLanes(
    deferrals
      .map((segment) => {
        const startMs = timestamp(segment.startedAt, nowMs);
        const active = segment.status === "waiting";
        const endMs = segment.finishedAt
          ? timestamp(segment.finishedAt, nowMs)
          : active && runtimeFresh
            ? nowMs
            : timestamp(segment.lastObservedAt, startMs);
        return { segment, startMs: Math.max(windowStart, startMs), endMs: Math.min(nowMs, endMs) };
      })
      .filter((item) => item.endMs >= windowStart && item.startMs <= nowMs),
  );
  const visibleCoverageBands: Band[] = coverageBands.flatMap((band): Band[] => {
    if (band.status === "gap") return [band];
    let clearIntervals = [{ startMs: band.startMs, endMs: band.endMs }];
    const unknownIntervals = [
      ...packedDeferrals.map(({ startMs, endMs }) => ({ startMs, endMs })),
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
    return clearIntervals.map((interval) => ({ ...interval, status: "normal", lane: 0 }));
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
  const pressureLaneCount = Math.max(1, ...packedDeferrals.map((item) => item.lane + 1));
  const pressureTop = AXIS_HEIGHT + laneCount * LANE_HEIGHT + 18;
  const chartHeight = pressureTop + pressureLaneCount * 12 + 12;
  const x = (time: number) => ((time - windowStart) / DAY_MS) * CHART_WIDTH;
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
            最近 24 小时
          </h3>
          <p className="text-sm text-base-content/60">执行结果与任务让行记录</p>
        </div>
        <ul
          className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-base-content/65"
          aria-label="时间图图例"
        >
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
      <div className="grid grid-cols-[6.5rem_minmax(0,1fr)] items-start border-y border-base-300/70">
        <div className="pt-9 text-xs text-base-content/65">
          <div className="flex h-[var(--task-chart-execution-height)] items-start pt-1">
            任务执行
          </div>
          <div className="mt-[18px] pr-1 leading-tight">准入 / 压力</div>
        </div>
        <div
          ref={timelineScrollerRef}
          className="overflow-x-auto overscroll-x-contain"
          data-testid="task-timeline-scroll-container"
        >
          <svg
            aria-label="任务执行和任务让行的最近 24 小时时间图"
            className="block w-full min-w-[760px] text-base-content/55"
            height={chartHeight}
            role="group"
            viewBox={`0 0 ${CHART_WIDTH} ${chartHeight}`}
            width={CHART_WIDTH}
            style={
              { "--task-chart-execution-height": `${laneCount * LANE_HEIGHT}px` } as CSSProperties
            }
          >
            <title>最近 24 小时任务执行时间图</title>
            {[0, 6, 12, 18, 24].map((hour) => {
              const lineX = (hour / 24) * CHART_WIDTH;
              const labelTime = windowStart + hour * 60 * 60 * 1000;
              return (
                <g key={hour}>
                  <line
                    x1={lineX}
                    x2={lineX}
                    y1={AXIS_HEIGHT}
                    y2={pressureTop + pressureLaneCount * 12}
                    stroke="currentColor"
                    strokeOpacity="0.14"
                  />
                  <text
                    x={Math.min(lineX + 4, CHART_WIDTH - 70)}
                    y={20}
                    fill="currentColor"
                    fontSize="11"
                  >
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
                x2={CHART_WIDTH}
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
                height={pressureLaneCount * 12}
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
                    stroke={bar.active ? color : "none"}
                    strokeWidth={bar.active ? 1 : 0}
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
              return (
                <g
                  key={group.key}
                  aria-label={`${group.members.length} 次${taskByKey.get(group.taskKey)?.title ?? group.taskKey}执行，选择查看详情`}
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
                  <rect x={barX} y={barY} width={width} height={11} rx={2} fill={color} />
                  <text x={barX + width + 2} y={barY + 9} fill="currentColor" fontSize="9">
                    ×{group.members.length}
                  </text>
                  <title>{`${group.members.length} 次密集执行；选择查看每次执行`}</title>
                </g>
              );
            })}
            {packedDeferrals.map(({ segment, startMs, endMs, lane }) => {
              const resourceBusy = segment.reason === "resource_busy";
              const color = resourceBusy ? "#d08700" : "#d63850";
              const bandY = pressureTop + lane * 12 + 2;
              const bandX = x(startMs);
              const width = Math.max(MIN_BAR_WIDTH, x(endMs) - bandX);
              const title = `${segment.title}；${reasonLabel(segment.reason)}；开始 ${compactTime(startMs)}；${segment.finishedAt ? `恢复 ${compactTime(timestamp(segment.finishedAt, endMs))}` : "尚未确认恢复"}${segment.retryAt ? `；重试时间 ${compactTime(timestamp(segment.retryAt, endMs))}` : "；重试时间未知"}`;
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
                  <rect x={bandX} y={bandY} width={width} height={8} rx={2} fill={color} />
                  <title>{title}</title>
                </g>
              );
            })}
            <line
              x1={CHART_WIDTH}
              x2={CHART_WIDTH}
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
                className="grid gap-1 py-2 text-sm sm:grid-cols-[minmax(0,1fr)_auto_auto] sm:items-center"
              >
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
                <span className="text-xs text-base-content/65">
                  {compactTime(timestamp(bar.segment.startedAt, nowMs))}
                </span>
                <span className="text-xs text-base-content/65">
                  {outcomeLabel(bar.segment.status)}
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
