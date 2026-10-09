import {
  type CSSProperties,
  type JSX,
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
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
const DENSITY_GROUP_MIN_WIDTH = 7;
const COVERAGE_HEARTBEAT_MAX_AGE_MS = 60_000;

type ExecutionBar = {
  segment: TaskTimelineSegment;
  startMs: number;
  endMs: number;
  lane: number;
  active: boolean;
};

type ExecutionLayout = {
  segment: TaskTimelineSegment;
  startMs: number;
  endMs: number;
  lane: number;
  active: boolean;
  current: boolean;
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
  taskKey: string | null;
  lane: number;
  members: ExecutionBar[];
  startMs: number;
  endMs: number;
};

type PositionedDeferral = {
  segment: TaskTimelineSegment;
  startMs: number;
  endMs: number;
  order: number;
};

type DeferralRenderGroup = {
  item: PositionedDeferral;
  members: PositionedDeferral[];
};

type DeferralIndex = {
  sorted: PositionedDeferral[];
  starts: number[];
  prefixMaxEnd: number[];
  overlapCounts: Map<string, number>;
  byId: Map<string, PositionedDeferral>;
  dynamic?: PositionedDeferral[];
  base?: DeferralIndex;
};

type CachedDetail = {
  signature: string;
  title: string;
};

function timestamp(value: string | null | undefined, fallback: number): number {
  const parsed = value ? Date.parse(value) : Number.NaN;
  return Number.isFinite(parsed) ? parsed : fallback;
}

const compactTimeFormatter = new Intl.DateTimeFormat("zh-CN", {
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
  timeZone: "Asia/Shanghai",
});
const exactTimeFormatter = new Intl.DateTimeFormat("zh-CN", {
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
  hour12: false,
  timeZone: "Asia/Shanghai",
});

function compactTime(value: number): string {
  return compactTimeFormatter.format(new Date(value));
}

function exactTime(value: number): string {
  return exactTimeFormatter.format(new Date(value));
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

function buildExecutionDensity(
  bars: ExecutionBar[],
  chartWidth: number,
  windowStart: number,
): { groups: DensityGroup[]; singles: ExecutionBar[] } {
  const groups: DensityGroup[] = [];
  const singles: ExecutionBar[] = [];
  const barsByLane = new Map<number, ExecutionBar[]>();
  const x = (time: number) => ((time - windowStart) / TIMELINE_WINDOW_MS) * chartWidth;
  for (const bar of bars) {
    const laneBars = barsByLane.get(bar.lane) ?? [];
    laneBars.push(bar);
    barsByLane.set(bar.lane, laneBars);
  }
  for (const [lane, laneBars] of barsByLane) {
    laneBars.sort((left, right) => left.startMs - right.startMs || left.endMs - right.endMs);
    let members: ExecutionBar[] = [];
    let groupStartMs = 0;
    let groupEndMs = 0;
    const finishGroup = () => {
      if (members.length === 1) {
        singles.push(members[0]);
      } else if (members.length > 1) {
        const taskKey = members[0].segment.taskKey;
        groups.push({
          key: `${lane}:${members[0].segment.segmentId}`,
          taskKey: members.every((member) => member.segment.taskKey === taskKey) ? taskKey : null,
          lane,
          members,
          startMs: groupStartMs,
          endMs: groupEndMs,
        });
      }
      members = [];
    };

    for (const bar of laneBars) {
      const barX = x(bar.startMs);
      const groupStartX = x(groupStartMs);
      const groupEndX = x(groupEndMs);
      const groupWidth = members.length > 1 ? DENSITY_GROUP_MIN_WIDTH : MIN_BAR_WIDTH;
      const visibleGroupEndX = members.length
        ? Math.max(groupEndX, groupStartX + groupWidth)
        : Number.NEGATIVE_INFINITY;
      if (members.length > 0 && barX > visibleGroupEndX) finishGroup();
      if (members.length === 0) {
        groupStartMs = bar.startMs;
        groupEndMs = bar.endMs;
      } else {
        groupEndMs = Math.max(groupEndMs, bar.endMs);
      }
      members.push(bar);
    }
    finishGroup();
  }
  return { groups, singles };
}

function buildDeferralRenderGroups(items: PositionedDeferral[]): DeferralRenderGroup[] {
  const groups = new Map<string, DeferralRenderGroup>();
  for (const item of items) {
    const key = `${item.startMs}:${item.endMs}`;
    const existing = groups.get(key);
    if (existing) existing.members.push(item);
    else groups.set(key, { item, members: [item] });
  }
  return [...groups.values()].flatMap((group) =>
    group.members.length >= 4 ? [group] : group.members.map((item) => ({ item, members: [item] })),
  );
}

function lowerBound(values: number[], target: number): number {
  let low = 0;
  let high = values.length;
  while (low < high) {
    const middle = low + Math.floor((high - low) / 2);
    if (values[middle] < target) low = middle + 1;
    else high = middle;
  }
  return low;
}

function upperBound(values: number[], target: number): number {
  let low = 0;
  let high = values.length;
  while (low < high) {
    const middle = low + Math.floor((high - low) / 2);
    if (values[middle] <= target) low = middle + 1;
    else high = middle;
  }
  return low;
}

function mergeIntervals(
  intervals: Array<{ startMs: number; endMs: number }>,
): Array<{ startMs: number; endMs: number }> {
  const sorted = intervals
    .filter((interval) => interval.endMs > interval.startMs)
    .sort((left, right) => left.startMs - right.startMs || left.endMs - right.endMs);
  const merged: Array<{ startMs: number; endMs: number }> = [];
  for (const interval of sorted) {
    const previous = merged.at(-1);
    if (!previous || interval.startMs > previous.endMs) {
      merged.push({ ...interval });
    } else {
      previous.endMs = Math.max(previous.endMs, interval.endMs);
    }
  }
  return merged;
}

function mergeSortedIntervals(
  first: Array<{ startMs: number; endMs: number }>,
  second: Array<{ startMs: number; endMs: number }>,
): Array<{ startMs: number; endMs: number }> {
  const merged: Array<{ startMs: number; endMs: number }> = [];
  let firstIndex = 0;
  let secondIndex = 0;
  while (firstIndex < first.length || secondIndex < second.length) {
    const left = first[firstIndex];
    const right = second[secondIndex];
    const next =
      right == null || (left != null && left.startMs <= right.startMs)
        ? (first[firstIndex++] ?? null)
        : (second[secondIndex++] ?? null);
    if (!next || next.endMs <= next.startMs) continue;
    const previous = merged.at(-1);
    if (!previous || next.startMs > previous.endMs) merged.push({ ...next });
    else previous.endMs = Math.max(previous.endMs, next.endMs);
  }
  return merged;
}

function subtractIntervals(
  interval: { startMs: number; endMs: number },
  blockers: Array<{ startMs: number; endMs: number }>,
): Array<{ startMs: number; endMs: number }> {
  const remaining: Array<{ startMs: number; endMs: number }> = [];
  let cursor = interval.startMs;
  for (const blocker of blockers) {
    if (blocker.endMs <= cursor) continue;
    if (blocker.startMs >= interval.endMs) break;
    if (blocker.startMs > cursor) {
      remaining.push({ startMs: cursor, endMs: Math.min(blocker.startMs, interval.endMs) });
    }
    cursor = Math.max(cursor, blocker.endMs);
    if (cursor >= interval.endMs) break;
  }
  if (cursor < interval.endMs) remaining.push({ startMs: cursor, endMs: interval.endMs });
  return remaining;
}

function buildDeferralIndex(items: PositionedDeferral[]): DeferralIndex {
  const sorted = [...items].sort(
    (left, right) => left.startMs - right.startMs || left.order - right.order,
  );
  const starts = sorted.map((item) => item.startMs);
  const endSorted = [...sorted].sort(
    (left, right) => left.endMs - right.endMs || left.order - right.order,
  );
  const ends = endSorted.map((item) => item.endMs);
  const prefixMaxEnd: number[] = [];
  let maxEnd = Number.NEGATIVE_INFINITY;
  for (const item of sorted) {
    maxEnd = Math.max(maxEnd, item.endMs);
    prefixMaxEnd.push(maxEnd);
  }
  const overlapCounts = new Map<string, number>();
  for (const item of sorted) {
    const startsBeforeEnd = lowerBound(starts, item.endMs);
    const endsAtOrBeforeStart = upperBound(ends, item.startMs);
    overlapCounts.set(
      item.segment.segmentId,
      Math.max(0, startsBeforeEnd - endsAtOrBeforeStart - 1),
    );
  }
  return {
    sorted,
    starts,
    prefixMaxEnd,
    overlapCounts,
    byId: new Map(sorted.map((item) => [item.segment.segmentId, item])),
  };
}

function buildLiveDeferralIndex(
  staticIndex: DeferralIndex,
  liveItems: PositionedDeferral[],
): DeferralIndex {
  if (liveItems.length === 0) return staticIndex;
  const overlapCounts = new Map(staticIndex.overlapCounts);
  const byId = new Map(staticIndex.byId);
  const dynamic = [...liveItems].sort(
    (left, right) => left.startMs - right.startMs || left.order - right.order,
  );
  for (const liveItem of dynamic) {
    const staticOverlaps = overlappingDeferralsInIndex(liveItem, staticIndex);
    const dynamicOverlaps = dynamic.filter(
      (other) =>
        other.segment.segmentId !== liveItem.segment.segmentId &&
        other.startMs < liveItem.endMs &&
        other.endMs > liveItem.startMs,
    );
    overlapCounts.set(liveItem.segment.segmentId, staticOverlaps.length + dynamicOverlaps.length);
    for (const staticItem of staticOverlaps) {
      overlapCounts.set(
        staticItem.segment.segmentId,
        (overlapCounts.get(staticItem.segment.segmentId) ?? 0) + 1,
      );
    }
    byId.set(liveItem.segment.segmentId, liveItem);
  }
  return {
    ...staticIndex,
    overlapCounts,
    byId,
    dynamic,
    base: staticIndex,
  };
}

function overlappingDeferralsInIndex(
  item: PositionedDeferral,
  index: DeferralIndex,
): PositionedDeferral[] {
  const firstPossible = upperBound(index.prefixMaxEnd, item.startMs);
  const beforeEnd = lowerBound(index.starts, item.endMs);
  return index.sorted
    .slice(firstPossible, beforeEnd)
    .filter(
      (other) =>
        other.segment.segmentId !== item.segment.segmentId &&
        other.startMs < item.endMs &&
        other.endMs > item.startMs,
    );
}

function overlappingDeferrals(
  item: PositionedDeferral,
  index: DeferralIndex,
): PositionedDeferral[] {
  const base = index.base ?? index;
  const matches = overlappingDeferralsInIndex(item, base);
  if (index.dynamic) {
    matches.push(
      ...index.dynamic.filter(
        (other) =>
          other.segment.segmentId !== item.segment.segmentId &&
          other.startMs < item.endMs &&
          other.endMs > item.startMs,
      ),
    );
  }
  return matches.sort((left, right) => left.order - right.order);
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

function executionSummary(bar: ExecutionBar): string {
  return `${bar.segment.title}；结果：${outcomeLabel(bar.segment.status)}；选择查看详情`;
}

function deferralDetail(item: PositionedDeferral): string {
  const { segment } = item;
  return `${segment.title}；${reasonLabel(segment.reason)}；开始 ${exactTime(item.startMs)}；${segment.finishedAt ? `恢复 ${exactTime(timestamp(segment.finishedAt, item.endMs))}` : "尚未确认恢复"}${segment.retryAt ? `；重试时间 ${exactTime(timestamp(segment.retryAt, item.endMs))}` : "；重试时间未知"}`;
}

function deferralTitle(item: PositionedDeferral, overlaps: PositionedDeferral[]): string {
  return [item, ...overlaps].map(deferralDetail).join("\n");
}

function deferralSummary(
  item: PositionedDeferral,
  overlapCount: number,
  members: PositionedDeferral[] = [],
): string {
  const overlapLabel = overlapCount ? `与 ${overlapCount} 条任务让行重叠` : "无重叠任务让行";
  const reasons = [
    ...new Set([item, ...members].map((member) => reasonLabel(member.segment.reason))),
  ];
  const concurrentLabel =
    members.length > 1 ? `；并发 ${members.length} 条任务让行（${reasons.join("、")}）` : "";
  return `${item.segment.title}；${reasonLabel(item.segment.reason)}${concurrentLabel}；${overlapLabel}；选择查看详情`;
}

type ExecutionLayerProps = {
  singles: ExecutionBar[];
  groups: DensityGroup[];
  chartWidth: number;
  windowStart: number;
  dark: boolean;
  taskByKey: Map<string, ManagedTask>;
  getSummary: (bar: ExecutionBar) => string;
  onSelect: (bars: ExecutionBar[]) => void;
  onActivate: (bar: ExecutionBar, node: SVGGElement) => void;
  onDeactivate: (bar: ExecutionBar, node: SVGGElement) => void;
};

const ExecutionLayer = memo(function ExecutionLayer({
  singles,
  groups,
  chartWidth,
  windowStart,
  dark,
  taskByKey,
  getSummary,
  onSelect,
  onActivate,
  onDeactivate,
}: ExecutionLayerProps): JSX.Element {
  const x = (time: number) => ((time - windowStart) / TIMELINE_WINDOW_MS) * chartWidth;
  return (
    <>
      {singles.map((bar) => {
        const barX = x(bar.startMs);
        const barY = AXIS_HEIGHT + bar.lane * LANE_HEIGHT + 3;
        const width = Math.max(MIN_BAR_WIDTH, x(bar.endMs) - barX);
        const color = managedTaskColor(taskByKey.get(bar.segment.taskKey), dark);
        const appearance = outcomeAppearance(bar.segment.status, dark);
        const summary = getSummary(bar);
        return (
          <g
            key={bar.segment.segmentId}
            aria-label={summary}
            className="cursor-pointer outline-none focus-visible:opacity-75"
            onBlur={(event) => {
              if (
                !(event.relatedTarget instanceof Node) ||
                !event.currentTarget.contains(event.relatedTarget)
              ) {
                onDeactivate(bar, event.currentTarget);
              }
            }}
            onClick={() => onSelect([bar])}
            onFocus={(event) => onActivate(bar, event.currentTarget)}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                onSelect([bar]);
              }
            }}
            onMouseEnter={(event) => onActivate(bar, event.currentTarget)}
            onMouseLeave={(event) => {
              if (document.activeElement !== event.currentTarget) {
                onDeactivate(bar, event.currentTarget);
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
            <title>{summary}</title>
          </g>
        );
      })}
      {groups.map((group) => {
        const barX = x(group.startMs);
        const barY = AXIS_HEIGHT + group.lane * LANE_HEIGHT + 3;
        const width = Math.max(DENSITY_GROUP_MIN_WIDTH, x(group.endMs) - barX);
        const color = group.taskKey
          ? managedTaskColor(taskByKey.get(group.taskKey), dark)
          : dark
            ? "#94a3b8"
            : "#64748b";
        const outcome = aggregateOutcome(group.members);
        const appearance = outcomeAppearance(outcome.status, dark);
        const groupTitle = group.taskKey
          ? (taskByKey.get(group.taskKey)?.title ?? group.taskKey)
          : "多任务";
        return (
          <g
            key={group.key}
            aria-label={`${group.members.length} 次${groupTitle}执行；${outcome.summary}；选择查看详情`}
            className="cursor-pointer outline-none focus-visible:opacity-75"
            onClick={() => onSelect(group.members)}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                onSelect(group.members);
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
    </>
  );
});

type DeferralLayerProps = {
  groups: DeferralRenderGroup[];
  chartWidth: number;
  windowStart: number;
  pressureTop: number;
  overlapCounts: Map<string, number>;
  onSelect: (items: PositionedDeferral[]) => void;
  onActivate: (item: PositionedDeferral, node: SVGGElement, members: PositionedDeferral[]) => void;
  onDeactivate: (
    item: PositionedDeferral,
    node: SVGGElement,
    members: PositionedDeferral[],
  ) => void;
};

const DeferralLayer = memo(function DeferralLayer({
  groups,
  chartWidth,
  windowStart,
  pressureTop,
  overlapCounts,
  onSelect,
  onActivate,
  onDeactivate,
}: DeferralLayerProps): JSX.Element {
  const x = (time: number) => ((time - windowStart) / TIMELINE_WINDOW_MS) * chartWidth;
  return (
    <>
      {groups.map(({ item, members }) => {
        const { segment, startMs, endMs } = item;
        const resourceBusy = segment.reason === "resource_busy";
        const color = resourceBusy ? "#d08700" : "#d63850";
        const bandY = pressureTop + 2;
        const bandX = x(startMs);
        const width = Math.max(MIN_BAR_WIDTH, x(endMs) - bandX);
        const overlapCount = overlapCounts.get(segment.segmentId) ?? 0;
        const title = deferralSummary(item, overlapCount, members);
        return (
          <g
            key={segment.segmentId}
            data-task-deferral-id={segment.segmentId}
            aria-label={title}
            className="cursor-pointer outline-none focus-visible:opacity-75"
            onBlur={(event) => {
              if (
                !(event.relatedTarget instanceof Node) ||
                !event.currentTarget.contains(event.relatedTarget)
              ) {
                onDeactivate(item, event.currentTarget, members);
              }
            }}
            onClick={() => onSelect(members)}
            onFocus={(event) => onActivate(item, event.currentTarget, members)}
            onKeyDown={(event) => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                onSelect(members);
              }
            }}
            onMouseEnter={(event) => onActivate(item, event.currentTarget, members)}
            onMouseLeave={(event) => {
              if (document.activeElement !== event.currentTarget) {
                onDeactivate(item, event.currentTarget, members);
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
              fillOpacity={overlapCount ? 0.55 : 1}
              stroke={color}
              strokeWidth={1}
            />
            <title>{title}</title>
          </g>
        );
      })}
    </>
  );
});

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
  const [selectedExecutionIds, setSelectedExecutionIds] = useState<string[] | null>(null);
  const [selectedDeferralIds, setSelectedDeferralIds] = useState<string[] | null>(null);
  const activeExecutionIdRef = useRef<string | null>(null);
  const activeExecutionNodeRef = useRef<SVGGElement | null>(null);
  const activeDeferralIdRef = useRef<string | null>(null);
  const activeDeferralNodeRef = useRef<SVGGElement | null>(null);
  const deferralIndexRef = useRef<DeferralIndex | null>(null);
  const executionBarByIdRef = useRef(new Map<string, ExecutionBar>());
  const executionTitleTimerRef = useRef<number | null>(null);
  const deferralTitleTimerRef = useRef<number | null>(null);
  const staticTimelineLayerRef = useRef<SVGGElement>(null);
  const executionDetailCacheRef = useRef(new Map<string, CachedDetail>());
  const deferralDetailCacheRef = useRef(new Map<string, CachedDetail>());
  const timelineContainerRef = useRef<HTMLDivElement>(null);
  const [chartWidth, setChartWidth] = useState(DEFAULT_CHART_WIDTH);
  const dark =
    typeof document !== "undefined" &&
    (document.documentElement.getAttribute("data-color-mode") === "dark" ||
      document.documentElement.getAttribute("data-theme") === "vibe-dark");
  const windowStart = nowMs - TIMELINE_WINDOW_MS;
  const modelWindowStartRef = useRef(windowStart);
  const executionSnapshotRef = useRef(executions);
  if (executionSnapshotRef.current !== executions) {
    executionSnapshotRef.current = executions;
    modelWindowStartRef.current = windowStart;
  }
  const modelWindowStart = modelWindowStartRef.current;
  const taskByKey = useMemo(() => new Map(tasks.map((task) => [task.taskKey, task])), [tasks]);

  useEffect(() => {
    const currentIds = new Set(executions.map((segment) => segment.segmentId));
    for (const cache of [executionDetailCacheRef.current, deferralDetailCacheRef.current]) {
      for (const id of cache.keys()) {
        if (!currentIds.has(id)) cache.delete(id);
      }
    }
  }, [executions]);

  useEffect(
    () => () => {
      if (deferralTitleTimerRef.current != null) {
        window.clearTimeout(deferralTitleTimerRef.current);
        deferralTitleTimerRef.current = null;
      }
      if (executionTitleTimerRef.current != null) {
        window.clearTimeout(executionTitleTimerRef.current);
        executionTitleTimerRef.current = null;
      }
    },
    [],
  );

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

  useEffect(() => {
    const layer = staticTimelineLayerRef.current;
    if (!layer) return;
    const offset = ((modelWindowStart - windowStart) / TIMELINE_WINDOW_MS) * chartWidth;
    layer.setAttribute("transform", `translate(${offset} 0)`);
  }, [chartWidth, modelWindowStart, windowStart]);

  const executionLayout = useMemo<ExecutionLayout[]>(() => {
    const byUid = new Map<string, TaskTimelineSegment>();
    for (const segment of executions) {
      if (segment.kind === "execution") byUid.set(segment.segmentId, segment);
    }
    for (const run of activeRuns) {
      const prior = byUid.get(run.executionUid);
      if (prior && prior.status !== "running") continue;
      const observedAt = Math.max(
        timestamp(runtimeObservedAt, timestamp(run.startedAt, 0)),
        timestamp(prior?.lastObservedAt, timestamp(run.startedAt, 0)),
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
      [...byUid.values()].map((segment) => {
        const startMs = timestamp(segment.startedAt, timestamp(runtimeObservedAt, 0));
        const isActive = activeUids.has(segment.segmentId);
        const displaySegment =
          !isActive && segment.status === "running" && !segment.finishedAt
            ? { ...segment, status: "unknown" }
            : segment;
        const endMs = segment.finishedAt
          ? timestamp(segment.finishedAt, startMs)
          : timestamp(segment.lastObservedAt, startMs);
        return {
          segment: displaySegment,
          active: isActive,
          current: currentUids.has(segment.segmentId),
          startMs,
          endMs: isActive ? Number.POSITIVE_INFINITY : Math.max(startMs, endMs),
        };
      }),
    );
    return packed.map(({ segment, active, current, startMs, endMs, lane }) => ({
      segment,
      active,
      current,
      startMs,
      endMs,
      lane,
    }));
  }, [activeRuns, executions, runtimeFresh, runtimeObservedAt]);

  const staticExecutionBars = useMemo(() => {
    const bars: ExecutionBar[] = [];
    const staticWindowStart = modelWindowStart;
    const staticWindowEnd = staticWindowStart + TIMELINE_WINDOW_MS;
    for (const item of executionLayout) {
      if (item.active || item.current) continue;
      const endMs = item.endMs;
      if (endMs < staticWindowStart || item.startMs > staticWindowEnd) continue;
      bars.push({
        segment: item.segment,
        active: false,
        lane: item.lane,
        startMs: Math.max(staticWindowStart, item.startMs),
        endMs: Math.min(staticWindowEnd, Math.max(item.startMs, endMs)),
      });
    }
    return bars;
  }, [executionLayout, modelWindowStart]);

  const dynamicExecutionBars = useMemo(() => {
    const bars: ExecutionBar[] = [];
    for (const item of executionLayout) {
      if (!item.active && !item.current) continue;
      const endMs = item.active ? nowMs : Math.min(nowMs, runtimeBoundaryMs, item.endMs);
      if (endMs < windowStart || item.startMs > nowMs) continue;
      bars.push({
        segment: item.segment,
        active: item.active,
        lane: item.lane,
        startMs: Math.max(windowStart, item.startMs),
        endMs: Math.min(nowMs, Math.max(item.startMs, endMs)),
      });
    }
    return bars;
  }, [executionLayout, nowMs, runtimeBoundaryMs, windowStart]);

  const deferrals = useMemo(
    () => executions.filter((segment) => segment.kind === "deferral"),
    [executions],
  );
  const staticRuntimeBoundary = runtimeFresh ? Number.POSITIVE_INFINITY : runtimeBoundaryMs;
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

  const staticPositionedDeferrals = useMemo<PositionedDeferral[]>(() => {
    const staticWindowStart = modelWindowStart;
    const staticWindowEnd = staticWindowStart + TIMELINE_WINDOW_MS;
    return deferrals
      .map((segment, order) => {
        const startMs = timestamp(segment.startedAt, staticWindowStart);
        const active = segment.status === "waiting";
        const live = active && !segment.finishedAt && runtimeFresh;
        const endMs = segment.finishedAt
          ? timestamp(segment.finishedAt, startMs)
          : live
            ? Number.POSITIVE_INFINITY
            : active
              ? Math.min(staticWindowEnd, staticRuntimeBoundary)
              : timestamp(segment.lastObservedAt, startMs);
        return {
          segment,
          startMs: Math.max(staticWindowStart, startMs),
          endMs: Math.min(staticWindowEnd, endMs),
          order,
          live,
        };
      })
      .filter(
        (item) => !item.live && item.endMs >= staticWindowStart && item.startMs <= staticWindowEnd,
      )
      .map(({ live: _live, ...item }) => item);
  }, [deferrals, modelWindowStart, runtimeFresh, staticRuntimeBoundary]);
  const openDeferrals = useMemo(
    () =>
      deferrals.filter(
        (segment) => segment.status === "waiting" && !segment.finishedAt && runtimeFresh,
      ),
    [deferrals, runtimeFresh],
  );
  const livePositionedDeferrals = useMemo<PositionedDeferral[]>(
    () =>
      openDeferrals
        .map((segment) => {
          const order = deferrals.indexOf(segment);
          const startMs = timestamp(segment.startedAt, nowMs);
          return {
            segment,
            startMs: Math.max(windowStart, startMs),
            endMs: nowMs,
            order,
          };
        })
        .filter((item) => item.endMs >= windowStart && item.startMs <= nowMs)
        .sort((left, right) => left.startMs - right.startMs || left.order - right.order),
    [deferrals, nowMs, openDeferrals, windowStart],
  );
  const staticDeferralIndex = useMemo(
    () => buildDeferralIndex(staticPositionedDeferrals),
    [staticPositionedDeferrals],
  );
  const deferralIndex = useMemo(
    () => buildLiveDeferralIndex(staticDeferralIndex, livePositionedDeferrals),
    [livePositionedDeferrals, staticDeferralIndex],
  );
  deferralIndexRef.current = deferralIndex;
  const staticDeferralRenderGroups = useMemo(
    () => buildDeferralRenderGroups(staticPositionedDeferrals),
    [staticPositionedDeferrals],
  );
  const liveDeferralRenderGroups = useMemo(
    () => buildDeferralRenderGroups(livePositionedDeferrals),
    [livePositionedDeferrals],
  );
  const staticExplicitCoverageIntervals = useMemo(
    () =>
      explicitCoverageGaps
        .map((gap) => ({
          startMs: Math.max(modelWindowStart, timestamp(gap.startedAt, modelWindowStart)),
          endMs: Math.min(
            modelWindowStart + TIMELINE_WINDOW_MS,
            timestamp(gap.finishedAt, timestamp(gap.lastObservedAt, modelWindowStart)),
          ),
        }))
        .filter((interval) => interval.endMs > interval.startMs),
    [explicitCoverageGaps, modelWindowStart],
  );
  const staticUnknownCoverageIntervals = useMemo(
    () =>
      mergeIntervals([
        ...staticPositionedDeferrals.map(({ startMs, endMs }) => ({ startMs, endMs })),
        ...staticExplicitCoverageIntervals,
      ]),
    [staticExplicitCoverageIntervals, staticPositionedDeferrals],
  );
  const unknownCoverageIntervals = useMemo(
    () =>
      mergeSortedIntervals(
        staticUnknownCoverageIntervals,
        livePositionedDeferrals.map(({ startMs, endMs }) => ({ startMs, endMs })),
      ),
    [livePositionedDeferrals, staticUnknownCoverageIntervals],
  );
  const visibleCoverageBands: Band[] = coverageBands.flatMap((band): Band[] => {
    if (band.status === "gap") return [band];
    return subtractIntervals(band, unknownCoverageIntervals).map((interval) => ({
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

  const laneCount = Math.max(1, ...executionLayout.map((item) => item.lane + 1));
  const pressureTop = AXIS_HEIGHT + laneCount * LANE_HEIGHT + 18;
  const chartHeight = pressureTop + 12 + 12;
  const x = (time: number) => ((time - windowStart) / TIMELINE_WINDOW_MS) * chartWidth;
  const timeAxisHours =
    chartWidth < 220 ? [0, 12] : chartWidth < 520 ? [0, 6, 12] : TIME_AXIS_HOURS;
  const staticExecutionDensity = useMemo(
    () => buildExecutionDensity(staticExecutionBars, chartWidth, modelWindowStart),
    [chartWidth, modelWindowStart, staticExecutionBars],
  );
  const dynamicExecutionDensity = useMemo(
    () => buildExecutionDensity(dynamicExecutionBars, chartWidth, windowStart),
    [chartWidth, dynamicExecutionBars, windowStart],
  );
  const cachedExecutionTitle = useCallback((bar: ExecutionBar): string => {
    const signature = `${bar.segment.revision}:${bar.segment.status}:${bar.segment.finishedAt ?? bar.segment.lastObservedAt}:${bar.endMs}`;
    const cached = executionDetailCacheRef.current.get(bar.segment.segmentId);
    if (cached?.signature === signature) return cached.title;
    const title = executionTitle(bar);
    executionDetailCacheRef.current.set(bar.segment.segmentId, { signature, title });
    return title;
  }, []);

  const executionBarById = useMemo(() => {
    const next = new Map<string, ExecutionBar>();
    for (const bar of [...staticExecutionBars, ...dynamicExecutionBars]) {
      next.set(bar.segment.segmentId, bar);
    }
    for (const item of executionLayout) {
      if (next.has(item.segment.segmentId)) continue;
      next.set(item.segment.segmentId, {
        segment: item.segment,
        active: item.active,
        lane: item.lane,
        startMs: item.startMs,
        endMs: item.active
          ? nowMs
          : item.current
            ? Math.min(nowMs, runtimeBoundaryMs, item.endMs)
            : item.endMs,
      });
    }
    return next;
  }, [dynamicExecutionBars, executionLayout, nowMs, runtimeBoundaryMs, staticExecutionBars]);
  executionBarByIdRef.current = executionBarById;

  const updateExecutionNode = useCallback(
    (node: SVGGElement, bar: ExecutionBar, expanded: boolean) => {
      const summary = executionSummary(bar);
      if (executionTitleTimerRef.current != null) {
        window.clearTimeout(executionTitleTimerRef.current);
        executionTitleTimerRef.current = null;
      }
      node.setAttribute("aria-label", expanded ? summary : summary);
      const titleElement = node.querySelector("title");
      if (!titleElement) return;
      titleElement.textContent = summary;
      if (!expanded) return;
      executionTitleTimerRef.current = window.setTimeout(() => {
        const currentBar = executionBarByIdRef.current.get(bar.segment.segmentId) ?? bar;
        if (
          activeExecutionNodeRef.current === node &&
          activeExecutionIdRef.current === bar.segment.segmentId
        ) {
          const detail = cachedExecutionTitle(currentBar);
          node.setAttribute("aria-label", detail);
          titleElement.textContent = detail;
        }
        executionTitleTimerRef.current = null;
      }, 80);
    },
    [cachedExecutionTitle],
  );

  const activateExecution = useCallback(
    (bar: ExecutionBar, node: SVGGElement) => {
      activeExecutionIdRef.current = bar.segment.segmentId;
      activeExecutionNodeRef.current = node;
      updateExecutionNode(node, bar, true);
    },
    [updateExecutionNode],
  );

  const deactivateExecution = useCallback(
    (bar: ExecutionBar, node: SVGGElement) => {
      if (activeExecutionIdRef.current !== bar.segment.segmentId) return;
      activeExecutionIdRef.current = null;
      activeExecutionNodeRef.current = null;
      updateExecutionNode(node, bar, false);
    },
    [updateExecutionNode],
  );

  const cachedDeferralTitle = useCallback(
    (item: PositionedDeferral, overlaps: PositionedDeferral[]): string => {
      const signature = [
        item.segment.revision,
        item.startMs,
        item.endMs,
        ...overlaps.flatMap((overlap) => [overlap.segment.segmentId, overlap.segment.revision]),
      ].join(":");
      const cached = deferralDetailCacheRef.current.get(item.segment.segmentId);
      if (cached?.signature === signature) return cached.title;
      const title = deferralTitle(item, overlaps);
      deferralDetailCacheRef.current.set(item.segment.segmentId, { signature, title });
      return title;
    },
    [],
  );

  const updateDeferralNode = useCallback(
    (
      node: SVGGElement,
      item: PositionedDeferral,
      expanded: boolean,
      members: PositionedDeferral[] = [],
    ) => {
      const index = deferralIndexRef.current;
      if (!index) return;
      const summary = deferralSummary(
        item,
        index.overlapCounts.get(item.segment.segmentId) ?? 0,
        members,
      );
      if (!expanded) node.setAttribute("aria-label", summary);
      if (deferralTitleTimerRef.current != null) {
        window.clearTimeout(deferralTitleTimerRef.current);
        deferralTitleTimerRef.current = null;
      }
      const titleElement = node.querySelector("title");
      if (!titleElement) return;
      if (!expanded) {
        titleElement.textContent = summary;
        return;
      }
      deferralTitleTimerRef.current = window.setTimeout(() => {
        const currentIndex = deferralIndexRef.current;
        const currentItem = currentIndex?.byId.get(item.segment.segmentId) ?? item;
        if (expanded) {
          if (
            activeDeferralNodeRef.current === node &&
            activeDeferralIdRef.current === item.segment.segmentId &&
            currentIndex
          ) {
            const detail = cachedDeferralTitle(
              currentItem,
              overlappingDeferrals(currentItem, currentIndex),
            );
            node.setAttribute("aria-label", detail);
            titleElement.textContent = detail;
          }
        } else if (activeDeferralNodeRef.current !== node) {
          titleElement.textContent = summary;
        }
        deferralTitleTimerRef.current = null;
      }, 80);
    },
    [cachedDeferralTitle],
  );

  const activateDeferral = useCallback(
    (item: PositionedDeferral, node: SVGGElement, members: PositionedDeferral[]) => {
      activeDeferralIdRef.current = item.segment.segmentId;
      activeDeferralNodeRef.current = node;
      updateDeferralNode(node, item, true, members);
    },
    [updateDeferralNode],
  );

  const deactivateDeferral = useCallback(
    (item: PositionedDeferral, node: SVGGElement, members: PositionedDeferral[]) => {
      if (activeDeferralIdRef.current !== item.segment.segmentId) return;
      activeDeferralIdRef.current = null;
      activeDeferralNodeRef.current = null;
      updateDeferralNode(node, item, false, members);
    },
    [updateDeferralNode],
  );

  const selectExecutions = useCallback((bars: ExecutionBar[]) => {
    setSelectedExecutionIds(bars.map((bar) => bar.segment.segmentId));
    setSelectedDeferralIds(null);
  }, []);
  const selectDeferral = useCallback((items: PositionedDeferral[]) => {
    setSelectedDeferralIds(items.map((item) => item.segment.segmentId));
    setSelectedExecutionIds(null);
  }, []);

  const selectedExecutions = useMemo(
    () =>
      selectedExecutionIds
        ? selectedExecutionIds
            .map((id) => executionBarById.get(id))
            .filter((bar): bar is ExecutionBar => bar != null)
        : null,
    [executionBarById, selectedExecutionIds],
  );
  const selectedDeferrals = useMemo(
    () =>
      selectedDeferralIds
        ? selectedDeferralIds
            .map((id) => deferralIndex.byId.get(id)?.segment)
            .filter((segment): segment is TaskTimelineSegment => segment != null)
        : null,
    [deferralIndex, selectedDeferralIds],
  );
  const selectedDeferral = selectedDeferrals?.[0] ?? null;

  useEffect(() => {
    if (selectedExecutionIds && selectedExecutions?.length !== selectedExecutionIds.length) {
      setSelectedExecutionIds(
        selectedExecutions?.length ? selectedExecutions.map((bar) => bar.segment.segmentId) : null,
      );
    }
    if (selectedDeferralIds && selectedDeferrals?.length !== selectedDeferralIds.length) {
      setSelectedDeferralIds(
        selectedDeferrals?.length ? selectedDeferrals.map((segment) => segment.segmentId) : null,
      );
    }
  }, [selectedDeferralIds, selectedDeferrals, selectedExecutionIds, selectedExecutions]);

  useEffect(() => {
    const activeId = activeExecutionIdRef.current;
    const node = activeExecutionNodeRef.current;
    if (!activeId || !node) return;
    const bar = executionBarById.get(activeId);
    if (!bar) {
      activeExecutionIdRef.current = null;
      activeExecutionNodeRef.current = null;
      return;
    }
    updateExecutionNode(node, bar, true);
  }, [executionBarById, updateExecutionNode]);

  useEffect(() => {
    const activeId = activeDeferralIdRef.current;
    const node = activeDeferralNodeRef.current;
    if (!activeId || !node) return;
    const item = deferralIndex.byId.get(activeId);
    if (!item) {
      activeDeferralIdRef.current = null;
      activeDeferralNodeRef.current = null;
      return;
    }
    updateDeferralNode(node, item, true);
  }, [deferralIndex, updateDeferralNode]);

  const selectedDescription = selectedExecutions
    ? `${selectedExecutions.length} 次任务执行`
    : selectedDeferrals
      ? selectedDeferrals.length > 1
        ? `${selectedDeferrals.length} 条任务让行`
        : reasonLabel(selectedDeferrals[0]?.reason)
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
            <g ref={staticTimelineLayerRef}>
              <ExecutionLayer
                singles={staticExecutionDensity.singles}
                groups={staticExecutionDensity.groups}
                chartWidth={chartWidth}
                windowStart={modelWindowStart}
                dark={dark}
                taskByKey={taskByKey}
                getSummary={executionSummary}
                onSelect={selectExecutions}
                onActivate={activateExecution}
                onDeactivate={deactivateExecution}
              />
              <DeferralLayer
                groups={staticDeferralRenderGroups}
                chartWidth={chartWidth}
                windowStart={modelWindowStart}
                pressureTop={pressureTop}
                overlapCounts={staticDeferralIndex.overlapCounts}
                onSelect={selectDeferral}
                onActivate={activateDeferral}
                onDeactivate={deactivateDeferral}
              />
            </g>
            <ExecutionLayer
              singles={dynamicExecutionDensity.singles}
              groups={dynamicExecutionDensity.groups}
              chartWidth={chartWidth}
              windowStart={windowStart}
              dark={dark}
              taskByKey={taskByKey}
              getSummary={executionSummary}
              onSelect={selectExecutions}
              onActivate={activateExecution}
              onDeactivate={deactivateExecution}
            />
            <DeferralLayer
              groups={liveDeferralRenderGroups}
              chartWidth={chartWidth}
              windowStart={windowStart}
              pressureTop={pressureTop}
              overlapCounts={deferralIndex.overlapCounts}
              onSelect={selectDeferral}
              onActivate={activateDeferral}
              onDeactivate={deactivateDeferral}
            />
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
              onClick={() => setSelectedExecutionIds(null)}
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
                  <div className="mt-1 break-all font-mono text-[11px] leading-relaxed text-base-content/55">
                    {bar.segment.managedRunId != null
                      ? `运行 ID #${bar.segment.managedRunId}`
                      : `区间 ID ${bar.segment.segmentId}`}
                  </div>
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
      ) : selectedDeferrals ? (
        <div
          className="flex flex-wrap items-center justify-between gap-2 border-b border-base-300/60 pb-3 text-sm"
          aria-live="polite"
        >
          <div>
            <strong>
              {selectedDeferrals.length > 1
                ? `${selectedDeferrals.length} 条任务让行`
                : selectedDeferral?.title}
            </strong>
            {selectedDeferrals.length === 1 ? (
              <>
                <span className="ml-2 text-base-content/65">
                  {reasonLabel(selectedDeferral?.reason)}
                </span>
                <div className="mt-1 text-xs text-base-content/60">{selectedDeferral?.taskKey}</div>
                <div className="mt-1 text-xs text-base-content/60">
                  开始：{exactTime(timestamp(selectedDeferral?.startedAt, nowMs))} · 恢复：
                  {selectedDeferral?.finishedAt
                    ? exactTime(timestamp(selectedDeferral.finishedAt, nowMs))
                    : "尚未确认"}
                  {" · "}重试时间：
                  {selectedDeferral?.retryAt
                    ? exactTime(timestamp(selectedDeferral.retryAt, nowMs))
                    : "未知"}
                </div>
              </>
            ) : null}
          </div>
          {selectedDeferrals.length === 1 && selectedDeferral ? (
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
          ) : (
            <ul className="text-xs text-base-content/65">
              {selectedDeferrals.map((segment) => (
                <li key={segment.segmentId}>
                  {segment.title} · {segment.segmentId} · {reasonLabel(segment.reason)} ·{" "}
                  {segment.finishedAt ? "已恢复" : "尚未确认恢复"}
                </li>
              ))}
            </ul>
          )}
          <button
            type="button"
            className="text-sm text-primary hover:underline"
            onClick={() => setSelectedDeferralIds(null)}
          >
            收起
          </button>
        </div>
      ) : null}
    </section>
  );
}
