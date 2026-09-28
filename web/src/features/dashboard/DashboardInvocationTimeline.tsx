// biome-ignore-all lint/a11y/noNoninteractiveTabindex: the scroll viewport must be focusable
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { floatingSurfaceStyle } from "../../components/ui/floating-surface";
import { usePortaledTheme } from "../../components/ui/use-portaled-theme";
import { useCompactViewport } from "../../hooks/useCompactViewport";
import { useInvocationTimeline } from "../../hooks/useInvocationTimeline";
import useSseStatus from "../../hooks/useSseStatus";
import { useTranslation } from "../../i18n";
import type {
  InvocationTimelineRecord,
  InvocationTimelineResponse,
  TimeseriesResponse,
} from "../../lib/api";
import {
  recordTodayChartDataCommit,
  recordTodayChartRender,
} from "../../lib/dashboardPerformanceDiagnostics";
import { AppIcon } from "../shared/AppIcon";

// The dense timeline viewport is a keyboard-scrollable region for screen-reader users.

interface DashboardInvocationTimelineProps {
  response: TimeseriesResponse | null;
  loading: boolean;
  error?: string | null;
  closedNaturalDay?: boolean;
  upstreamAccountId?: number;
  liveRevision?: number;
  timelineData?: InvocationTimelineResponse | null;
  timelineStatusOverride?: "refreshing" | "stale";
}

export interface LaneRecord {
  record: InvocationTimelineRecord;
  startMs: number;
  endMs: number;
  lane: number;
}

interface LaneEnd {
  endMs: number;
  lane: number;
}

function pushMinHeap<T>(heap: T[], value: T, compare: (left: T, right: T) => number) {
  let index = heap.length;
  heap.push(value);
  while (index > 0) {
    const parentIndex = Math.floor((index - 1) / 2);
    const parent = heap[parentIndex];
    if (parent === undefined || compare(parent, value) <= 0) break;
    heap[index] = parent;
    index = parentIndex;
  }
  heap[index] = value;
}

function popMinHeap<T>(heap: T[], compare: (left: T, right: T) => number): T | undefined {
  const first = heap[0];
  if (first === undefined) return undefined;
  const last = heap.pop();
  if (last === undefined) return first;
  if (heap.length === 0) return first;

  let index = 0;
  while (true) {
    const leftIndex = index * 2 + 1;
    const rightIndex = leftIndex + 1;
    if (leftIndex >= heap.length) break;
    const left = heap[leftIndex];
    if (left === undefined) break;
    const right = heap[rightIndex];
    const childIndex = right !== undefined && compare(right, left) < 0 ? rightIndex : leftIndex;
    const child = heap[childIndex];
    if (child === undefined || compare(last, child) <= 0) break;
    heap[index] = child;
    index = childIndex;
  }
  heap[index] = last;
  return first;
}

export function getInvocationTimelineLaneCount(lanes: LaneRecord[]) {
  return Math.max(1, ...lanes.map((item) => item.lane + 1));
}

export function shouldShowTimelineUnavailable(
  response: TimeseriesResponse | null,
  bounds: { startMs: number; endMs: number } | null,
  timelineDataOverride: InvocationTimelineResponse | null | undefined,
) {
  return response != null && bounds == null && timelineDataOverride == null;
}

export function shouldAdvanceInvocationTimelineBars(
  closedNaturalDay: boolean,
  liveConnected: boolean,
  hasTimelineDataOverride: boolean,
  isStale = false,
) {
  return !closedNaturalDay && liveConnected && !hasTimelineDataOverride && !isStale;
}

export function hasInvocationTimelineRefreshError(
  error: string | null | undefined,
  timelineError: string | null | undefined,
  hasTimelineDataOverride: boolean,
) {
  return !hasTimelineDataOverride && Boolean(error || timelineError);
}

function TimelineSurfaceState({
  message,
  loading = false,
  compact = false,
}: {
  message: string;
  loading?: boolean;
  compact?: boolean;
}) {
  return (
    <div
      data-testid="dashboard-invocation-timeline-state"
      className="flex min-h-64 items-center justify-center rounded-lg border border-base-content/10 bg-base-300/20 p-6"
      role={loading ? "status" : "alert"}
      style={{
        minHeight: `${compact ? INVOCATION_CHART_HEIGHT_COMPACT_PX : INVOCATION_CHART_HEIGHT_DESKTOP_PX}px`,
      }}
    >
      <Alert variant={loading ? "info" : "warning"}>{message}</Alert>
    </div>
  );
}

function parseEpoch(value: string | null | undefined) {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function parseDurationMs(value: number | null | undefined) {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : null;
}

function resolveTerminalEndMs(record: InvocationTimelineRecord, startMs: number) {
  const durationMs = parseDurationMs(record.tTotalMs);
  return durationMs != null ? startMs + durationMs : startMs;
}

function formatDuration(
  record: InvocationTimelineRecord,
  translate: (key: string, values?: Record<string, string | number>) => string,
) {
  if (record.isInFlight) return translate("dashboard.activityOverview.timelineDurationRunning");
  if (record.tTotalMs == null) {
    return translate("dashboard.activityOverview.timelineDurationUnknown");
  }
  if (record.tTotalMs < 1_000) return `${Math.round(record.tTotalMs)} ms`;
  return `${(record.tTotalMs / 1_000).toFixed(record.tTotalMs >= 10_000 ? 0 : 1)} s`;
}

function formatInvocationTime(startMs: number, locale: string) {
  return new Intl.DateTimeFormat(locale === "zh" ? "zh-CN" : "en-US", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(new Date(startMs));
}

function resolveStatus(record: InvocationTimelineRecord) {
  if (record.isInFlight) {
    if (record.livePhase === "responding") return "responding";
    if (record.livePhase === "requesting") return "requesting";
    return "queued";
  }
  if (record.status === "failed" || record.failureClass === "service_failure") return "failed";
  if (record.status === "interrupted" || record.failureClass === "client_abort") {
    return "interrupted";
  }
  if (record.status === "unknown" || record.tTotalMs == null) return "unknown";
  return "success";
}

function statusClass(status: string) {
  switch (status) {
    case "failed":
      return "bg-error/80 border-error text-error-content";
    case "interrupted":
      return "bg-warning/80 border-warning text-warning-content";
    case "requesting":
      return "bg-info/80 border-info text-info-content";
    case "responding":
      return "bg-secondary/80 border-secondary text-secondary-content";
    case "queued":
      return "bg-base-content/35 border-base-content/55 text-base-content";
    case "unknown":
      return "bg-base-content/25 border-base-content/45 text-base-content/75";
    default:
      return "bg-success/80 border-success text-success-content";
  }
}

const INVOCATION_MIN_VISIBLE_LANES = 4;
const INVOCATION_CHART_HEIGHT_COMPACT_PX = 336;
const INVOCATION_CHART_HEIGHT_DESKTOP_PX = 320;
const INVOCATION_X_AXIS_HEIGHT_PX = 28;
const INVOCATION_LANE_MIN_HEIGHT_PX = 1;
const INVOCATION_LANE_MAX_HEIGHT_PX = 16;
const INVOCATION_LANE_GAP_PX = 1;
const INVOCATION_CALLS_AXIS_LABEL_OFFSET_PX = 16;
const INVOCATION_TTFT_AXIS_TOP_PX = 20;

export interface InvocationTimelineLayout {
  chartHeightPx: number;
  laneAreaHeightPx: number;
  laneHeight: number;
  laneGap: number;
  laneStep: number;
  lanePlotHeight: number;
  laneContentHeight: number;
  visibleLaneCount: number;
}

export interface InvocationTimelineVisibleLaneRange {
  firstLane: number;
  lastLane: number;
}

export function resolveInvocationTimelineLayout(
  actualLaneCount: number,
  isCompactViewport: boolean,
): InvocationTimelineLayout {
  const visibleLaneCount = Math.max(INVOCATION_MIN_VISIBLE_LANES, Math.max(1, actualLaneCount));
  const chartHeightPx = isCompactViewport
    ? INVOCATION_CHART_HEIGHT_COMPACT_PX
    : INVOCATION_CHART_HEIGHT_DESKTOP_PX;
  const laneAreaHeightPx = chartHeightPx - INVOCATION_X_AXIS_HEIGHT_PX;
  const heightWithGap = Math.floor(
    (laneAreaHeightPx - (visibleLaneCount - 1) * INVOCATION_LANE_GAP_PX) / visibleLaneCount,
  );
  const laneHeight =
    heightWithGap > 2
      ? Math.min(INVOCATION_LANE_MAX_HEIGHT_PX, heightWithGap)
      : Math.max(
          INVOCATION_LANE_MIN_HEIGHT_PX,
          Math.min(2, Math.floor(laneAreaHeightPx / visibleLaneCount)),
        );
  const laneGap = laneHeight > 2 ? INVOCATION_LANE_GAP_PX : 0;
  const laneStep = laneHeight + laneGap;
  const laneContentHeight =
    visibleLaneCount * laneHeight + Math.max(0, visibleLaneCount - 1) * laneGap;
  const lanePlotHeight = Math.max(laneAreaHeightPx, laneContentHeight + 24);
  return {
    chartHeightPx,
    laneAreaHeightPx,
    laneHeight,
    laneGap,
    laneStep,
    lanePlotHeight,
    laneContentHeight,
    visibleLaneCount,
  };
}

export interface InvocationTimelineTooltipPosition {
  x: number;
  y: number;
}

export function resolveInvocationTimelineTooltipPosition(
  anchor: { x: number; y: number },
  bounds: { width: number; height: number },
  tooltip: { width: number; height: number },
): InvocationTimelineTooltipPosition {
  const offset = 12;
  const padding = 8;
  let x = anchor.x + offset;
  let y = anchor.y + offset;
  if (x + tooltip.width > bounds.width - padding) {
    x = anchor.x - tooltip.width - offset;
  }
  if (y + tooltip.height > bounds.height - padding) {
    y = anchor.y - tooltip.height - offset;
  }
  return {
    x: Math.min(Math.max(x, padding), Math.max(padding, bounds.width - tooltip.width - padding)),
    y: Math.min(Math.max(y, padding), Math.max(padding, bounds.height - tooltip.height - padding)),
  };
}

export function resolveInvocationTimelineScrollTop(
  sameView: boolean,
  rememberedScrollTop: number,
  maxScrollTop: number,
) {
  const clampScrollTop = (value: number) => Math.min(maxScrollTop, Math.max(0, value));
  if (sameView) return clampScrollTop(rememberedScrollTop);
  return maxScrollTop;
}

export function resolveVisibleInvocationLaneRange(
  laneCount: number,
  laneStep: number,
  lanePlotHeight: number,
  laneAreaHeight: number,
  scrollTop: number,
  overscan = 2,
): InvocationTimelineVisibleLaneRange {
  if (laneCount <= 0 || laneStep <= 0) return { firstLane: 0, lastLane: -1 };
  const firstVisibleLane = Math.floor((lanePlotHeight - (scrollTop + laneAreaHeight)) / laneStep);
  const lastVisibleLane = Math.ceil((lanePlotHeight - scrollTop) / laneStep) - 1;
  return {
    firstLane: Math.max(0, firstVisibleLane - overscan),
    lastLane: Math.min(laneCount - 1, lastVisibleLane + overscan),
  };
}
export function assignInvocationTimelineLanes(
  records: InvocationTimelineRecord[],
  asOf: string,
  nowMs = Date.now(),
  advanceInFlight = true,
  snapshotAtMs?: number,
): LaneRecord[] {
  const referenceNowMs = snapshotAtMs ?? parseEpoch(asOf) ?? nowMs;
  const activeLanes: LaneEnd[] = [];
  const availableLanes: number[] = [];
  let nextLane = 0;
  return records
    .map((record) => ({ record, startMs: parseEpoch(record.occurredAt) }))
    .filter(
      (item): item is { record: InvocationTimelineRecord; startMs: number } => item.startMs != null,
    )
    .sort((left, right) => left.startMs - right.startMs || left.record.id - right.record.id)
    .map(({ record, startMs }) => {
      const endMs = record.isInFlight
        ? Math.max(referenceNowMs, advanceInFlight ? nowMs : referenceNowMs, startMs + 1)
        : Math.max(resolveTerminalEndMs(record, startMs), startMs + 1);
      while (activeLanes[0]?.endMs !== undefined && activeLanes[0].endMs <= startMs) {
        const availableLane = popMinHeap(
          activeLanes,
          (left, right) => left.endMs - right.endMs || left.lane - right.lane,
        );
        if (availableLane) {
          pushMinHeap(availableLanes, availableLane.lane, (left, right) => left - right);
        }
      }
      const lane = popMinHeap(availableLanes, (left, right) => left - right) ?? nextLane++;
      pushMinHeap(
        activeLanes,
        { lane, endMs },
        (left, right) => left.endMs - right.endMs || left.lane - right.lane,
      );
      return { record, startMs, endMs, lane };
    });
}

function buildTtftPath(
  points: TimeseriesResponse["points"],
  windowStartMs: number,
  windowEndMs: number,
  plotHeightPx: number,
  plotTopPx: number,
) {
  const values = points
    .map((point) => {
      const xMs = parseEpoch(point.bucketStart);
      const value = point.firstTokenAvgMs;
      return xMs != null &&
        value != null &&
        value >= 0 &&
        xMs >= windowStartMs &&
        xMs <= windowEndMs
        ? { xMs, value }
        : null;
    })
    .filter((point): point is { xMs: number; value: number } => point != null);
  if (values.length === 0) return { path: "", maxValue: 1 };
  const maxValue = Math.max(1, ...values.map((point) => point.value));
  const path = values
    .map((point, index) => {
      const x = ((point.xMs - windowStartMs) / Math.max(1, windowEndMs - windowStartMs)) * 100;
      const yPx = plotHeightPx - (point.value / maxValue) * Math.max(0, plotHeightPx - plotTopPx);
      const y = (yPx / Math.max(1, plotHeightPx)) * 100;
      return `${index === 0 ? "M" : "L"} ${x.toFixed(2)} ${y.toFixed(2)}`;
    })
    .join(" ");
  return { path, maxValue };
}

export function DashboardInvocationTimeline({
  response,
  loading,
  error,
  closedNaturalDay = false,
  upstreamAccountId,
  liveRevision,
  timelineData: timelineDataOverride,
  timelineStatusOverride,
}: DashboardInvocationTimelineProps) {
  const { locale, t } = useTranslation();
  const isCompactViewport = useCompactViewport();
  const sseStatus = useSseStatus();
  const liveRefreshAllowed =
    closedNaturalDay || !["reconnecting", "disabled"].includes(sseStatus.phase);
  const liveConnected = closedNaturalDay || sseStatus.phase === "connected";
  const [hoverMs, setHoverMs] = useState<number | null>(null);
  const [hoverAnchor, setHoverAnchor] = useState<{ x: number; y: number } | null>(null);
  const [tooltipPosition, setTooltipPosition] = useState<{ x: number; y: number } | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [laneScrollTop, setLaneScrollTop] = useState(0);
  const lastHoverUpdateMs = useRef(0);
  const laneScrollRef = useRef<HTMLDivElement | null>(null);
  const callsAxisScrollRef = useRef<HTMLDivElement | null>(null);
  const plotRef = useRef<HTMLDivElement | null>(null);
  const tooltipRef = useRef<HTMLDivElement | null>(null);
  const lastLaneScrollTopRef = useRef(0);
  const autoScrollViewKeyRef = useRef<string | null>(null);
  const timeline = useInvocationTimeline({
    response,
    closedNaturalDay,
    upstreamAccountId,
    liveRevision,
    liveRefreshAllowed,
    enabled: !timelineDataOverride,
  });

  const renderedData = timelineDataOverride ?? timeline.data;
  const renderedError = timelineDataOverride ? null : timeline.error;
  const timelineIsRefreshing = timeline.isRefreshing || timelineStatusOverride === "refreshing";
  const hasTimelineRefreshError = hasInvocationTimelineRefreshError(
    error,
    renderedError,
    timelineDataOverride != null,
  );
  const timelineIsStale =
    timeline.isStale || timelineStatusOverride === "stale" || hasTimelineRefreshError;
  const timelineIsFrozen = timeline.isFrozen || hasTimelineRefreshError;
  useEffect(() => {
    if (closedNaturalDay || !renderedData || !response) return;
    recordTodayChartDataCommit("today");
    const lastPoint = response.points.at(-1);
    recordTodayChartRender(
      `${renderedData.rangeStart}:${renderedData.rangeEnd}:${renderedData.asOf}:${renderedData.records.length}:${response.rangeStart}:${response.rangeEnd}:${lastPoint?.totalCount ?? ""}`,
    );
  }, [closedNaturalDay, renderedData, response]);
  const advanceLiveBars = shouldAdvanceInvocationTimelineBars(
    closedNaturalDay,
    liveConnected,
    timelineDataOverride != null,
    timelineIsStale,
  );
  const lanes = useMemo(
    () =>
      renderedData
        ? assignInvocationTimelineLanes(
            renderedData.records,
            renderedData.asOf,
            nowMs,
            advanceLiveBars,
            renderedData.snapshotAtMs,
          )
        : [],
    [advanceLiveBars, nowMs, renderedData],
  );
  const laneCount = getInvocationTimelineLaneCount(lanes);
  const laneLayout = resolveInvocationTimelineLayout(laneCount, isCompactViewport);
  const tooltipTheme = usePortaledTheme(plotRef.current);
  const visibleLaneRange = resolveVisibleInvocationLaneRange(
    laneLayout.visibleLaneCount,
    laneLayout.laneStep,
    laneLayout.lanePlotHeight,
    laneLayout.laneAreaHeightPx,
    laneScrollTop,
  );
  const visibleLanes = lanes.filter(
    (item) => item.lane >= visibleLaneRange.firstLane && item.lane <= visibleLaneRange.lastLane,
  );
  const hasInFlightLanes = lanes.some((item) => item.record.isInFlight);
  const plotWindow = timeline.window;
  const autoScrollViewKey = `${timeline.bounds?.startMs ?? "empty"}:${closedNaturalDay}:${upstreamAccountId ?? "all"}:${timelineDataOverride ? "override" : "remote"}`;
  useEffect(() => {
    if (closedNaturalDay || !liveConnected || !hasInFlightLanes) return;
    const timer = globalThis.setInterval(() => setNowMs(Date.now()), 1_000);
    return () => globalThis.clearInterval(timer);
  }, [closedNaturalDay, hasInFlightLanes, liveConnected]);

  useEffect(() => {
    const scrollElement = laneScrollRef.current;
    const callsAxisScrollElement = callsAxisScrollRef.current;
    if (!scrollElement || !callsAxisScrollElement || !renderedData) return;
    const visibleHeightPx = Math.min(
      scrollElement.clientHeight,
      laneLayout.chartHeightPx - INVOCATION_X_AXIS_HEIGHT_PX,
    );
    const maxScrollTop = Math.max(0, scrollElement.scrollHeight - visibleHeightPx);
    const sameView = autoScrollViewKeyRef.current === autoScrollViewKey;
    const nextScrollTop = resolveInvocationTimelineScrollTop(
      sameView,
      lastLaneScrollTopRef.current,
      maxScrollTop,
    );
    if (!sameView) autoScrollViewKeyRef.current = autoScrollViewKey;
    if (scrollElement.scrollTop !== nextScrollTop) scrollElement.scrollTop = nextScrollTop;
    if (callsAxisScrollElement.scrollTop !== nextScrollTop) {
      callsAxisScrollElement.scrollTop = nextScrollTop;
    }
    lastLaneScrollTopRef.current = nextScrollTop;
    setLaneScrollTop((current) => (current === nextScrollTop ? current : nextScrollTop));
  }, [autoScrollViewKey, laneLayout.chartHeightPx, renderedData]);

  useLayoutEffect(() => {
    const plot = plotRef.current;
    const tooltip = tooltipRef.current;
    if (hoverMs == null || !hoverAnchor || !plot || !tooltip) {
      setTooltipPosition(null);
      return undefined;
    }

    const updatePosition = () => {
      const tooltipRect = tooltip.getBoundingClientRect();
      const next = resolveInvocationTimelineTooltipPosition(
        hoverAnchor,
        { width: plot.clientWidth, height: laneLayout.laneAreaHeightPx },
        { width: tooltipRect.width, height: tooltipRect.height },
      );
      setTooltipPosition((current) =>
        current?.x === next.x && current.y === next.y ? current : next,
      );
    };

    updatePosition();
    window.addEventListener("resize", updatePosition);
    const observer =
      typeof ResizeObserver === "undefined" ? null : new ResizeObserver(updatePosition);
    observer?.observe(plot);
    observer?.observe(tooltip);
    return () => {
      window.removeEventListener("resize", updatePosition);
      observer?.disconnect();
    };
  }, [hoverAnchor, hoverMs, laneLayout.laneAreaHeightPx]);

  const ttft = useMemo(
    () =>
      plotWindow && response
        ? buildTtftPath(
            response.points,
            plotWindow.startMs,
            plotWindow.endMs,
            laneLayout.laneAreaHeightPx,
            INVOCATION_TTFT_AXIS_TOP_PX,
          )
        : { path: "", maxValue: 1 },
    [laneLayout.laneAreaHeightPx, plotWindow, response],
  );

  const stateMessage = timelineDataOverride
    ? null
    : !response && !loading
      ? t("dashboard.activityOverview.timelineUnavailable")
      : hasTimelineRefreshError
        ? t("dashboard.activityOverview.timelineUnavailable")
        : !closedNaturalDay && !liveRefreshAllowed
          ? t("dashboard.activityOverview.timelineOffline")
          : shouldShowTimelineUnavailable(response, timeline.bounds, timelineDataOverride)
            ? t("dashboard.activityOverview.timelineUnavailable")
            : null;
  if (stateMessage && !renderedData) {
    return (
      <div data-testid="dashboard-today-activity-chart">
        <TimelineSurfaceState message={stateMessage} compact={isCompactViewport} />
      </div>
    );
  }
  if (!plotWindow || (timeline.isLoading && !renderedData && !timelineDataOverride)) {
    return (
      <div data-testid="dashboard-today-activity-chart">
        <TimelineSurfaceState message={t("chart.loading")} loading compact={isCompactViewport} />
      </div>
    );
  }

  const windowSpan = Math.max(1, plotWindow.endMs - plotWindow.startMs);
  const xFor = (value: number) => ((value - plotWindow.startMs) / windowSpan) * 100;
  const hoverStats =
    hoverMs == null
      ? null
      : lanes.reduce(
          (stats, item) => {
            if (item.startMs > hoverMs || item.endMs < hoverMs) return stats;
            stats.total += 1;
            if (resolveStatus(item.record) === "queued") stats.queued += 1;
            else stats.running += 1;
            return stats;
          },
          { total: 0, running: 0, queued: 0 },
        );
  const {
    chartHeightPx,
    laneAreaHeightPx,
    laneHeight,
    laneGap,
    laneStep,
    lanePlotHeight,
    visibleLaneCount,
  } = laneLayout;
  const plotOriginTopPx = laneAreaHeightPx;
  const callAxisCapacity = Math.max(
    1,
    Math.floor((laneAreaHeightPx - INVOCATION_CALLS_AXIS_LABEL_OFFSET_PX) / laneStep),
  );
  const callAxisMaxValue = Math.max(visibleLaneCount, callAxisCapacity, 1);
  const callAxisTickCount = callAxisMaxValue <= 20 ? callAxisMaxValue + 1 : 5;
  const visibleAxisTopValue = Math.min(
    callAxisMaxValue,
    Math.ceil((lanePlotHeight - laneScrollTop) / laneStep),
  );
  const visibleAxisBottomValue = Math.max(
    0,
    Math.floor((lanePlotHeight - (laneScrollTop + laneAreaHeightPx)) / laneStep),
  );
  const visibleAxisValueSpan = Math.max(1, visibleAxisTopValue - visibleAxisBottomValue);
  const callAxisTopForValue = (value: number) => lanePlotHeight - value * laneStep;
  const callAxisTicks = Array.from({ length: callAxisTickCount }, (_, index) => {
    const fraction = index / Math.max(1, callAxisTickCount - 1);
    const value =
      index === callAxisTickCount - 1
        ? visibleAxisBottomValue
        : Math.round(visibleAxisTopValue - visibleAxisValueSpan * fraction);
    return {
      value,
      top: callAxisTopForValue(value),
    };
  });
  const ttftTicks = Array.from({ length: 5 }, (_, index) => {
    const fraction = index / 4;
    return {
      value: Math.round(ttft.maxValue * (1 - fraction)),
      top:
        INVOCATION_TTFT_AXIS_TOP_PX + fraction * (laneAreaHeightPx - INVOCATION_TTFT_AXIS_TOP_PX),
    };
  });
  const zeroIsVisible =
    callAxisTopForValue(0) >= laneScrollTop &&
    callAxisTopForValue(0) <= laneScrollTop + laneAreaHeightPx;
  const laneTopFor = (lane: number) => callAxisTopForValue(lane + 1) + laneGap;
  const firstCallAxisGridValue = Math.max(1, visibleLaneRange.firstLane);
  const lastCallAxisGridValue = Math.min(callAxisMaxValue, visibleLaneRange.lastLane + 1);
  const callAxisGridValues =
    callAxisMaxValue <= 20
      ? Array.from({ length: callAxisMaxValue }, (_, index) => index + 1)
      : Array.from(
          { length: Math.max(0, lastCallAxisGridValue - firstCallAxisGridValue + 1) },
          (_, index) => firstCallAxisGridValue + index,
        );

  return (
    <div data-testid="dashboard-today-activity-chart">
      <div className="flex flex-col gap-3" data-testid="dashboard-invocation-timeline">
        <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-base-content/65">
          <div className="flex items-center gap-3">
            <span className="font-semibold text-base-content">
              {t("dashboard.activityOverview.timelineTitle")}
            </span>
            {timelineIsRefreshing ? (
              <span className="text-info">{t("dashboard.activityOverview.timelineLive")}</span>
            ) : null}
            {timelineIsStale ? (
              <span className="text-warning">{t("dashboard.activityOverview.timelineStale")}</span>
            ) : null}
            {timelineIsFrozen ? (
              <span className="text-warning">
                {t("dashboard.activityOverview.timelineOffline")}
              </span>
            ) : null}
          </div>
          <div className="flex items-center gap-1">
            <button
              type="button"
              className="icon-button inline-flex h-8 w-8 items-center justify-center"
              aria-label={t("dashboard.activityOverview.timelineShortenWindow")}
              title={t("dashboard.activityOverview.timelineShortenWindow")}
              onClick={() => {
                const center = (plotWindow.startMs + plotWindow.endMs) / 2;
                const span = Math.max(5 * 60_000, windowSpan / 2);
                timeline.setWindow({
                  startMs: center - span / 2,
                  endMs: center + span / 2,
                });
              }}
            >
              <AppIcon name="minus" className="h-4 w-4" aria-hidden />
            </button>
            <button
              type="button"
              className="icon-button inline-flex h-8 w-8 items-center justify-center"
              aria-label={t("dashboard.activityOverview.timelineLengthenWindow")}
              title={t("dashboard.activityOverview.timelineLengthenWindow")}
              onClick={() => {
                const center = (plotWindow.startMs + plotWindow.endMs) / 2;
                const span = windowSpan * 2;
                timeline.setWindow({
                  startMs: center - span / 2,
                  endMs: center + span / 2,
                });
              }}
            >
              <AppIcon name="plus" className="h-4 w-4" aria-hidden />
            </button>
            <button
              type="button"
              className="icon-button inline-flex h-8 w-8 items-center justify-center"
              aria-label={t("dashboard.activityOverview.timelinePanEarlier")}
              title={t("dashboard.activityOverview.timelinePanEarlier")}
              onClick={() =>
                timeline.setWindow({
                  startMs: plotWindow.startMs - windowSpan / 2,
                  endMs: plotWindow.endMs - windowSpan / 2,
                })
              }
            >
              <AppIcon name="arrow-left" className="h-4 w-4" aria-hidden />
            </button>
            <button
              type="button"
              className="icon-button inline-flex h-8 w-8 items-center justify-center"
              aria-label={t("dashboard.activityOverview.timelinePanLater")}
              title={t("dashboard.activityOverview.timelinePanLater")}
              onClick={() =>
                timeline.setWindow({
                  startMs: plotWindow.startMs + windowSpan / 2,
                  endMs: plotWindow.endMs + windowSpan / 2,
                })
              }
            >
              <AppIcon name="arrow-right-bold" className="h-4 w-4" aria-hidden />
            </button>
          </div>
        </div>

        <div className="min-w-0 rounded-lg border border-base-content/10 bg-base-300/20">
          <div className="min-w-0 p-3">
            <div
              data-testid="dashboard-invocation-timeline-lanes"
              data-total-calls={renderedData?.total ?? 0}
              data-total-lanes={laneCount}
              className="relative h-[21rem] overflow-hidden overscroll-contain desktop:h-80"
              style={{ height: `${chartHeightPx}px` }}
            >
              <div
                className="relative flex"
                data-testid="dashboard-invocation-timeline-interaction-area"
                style={{ height: `${chartHeightPx}px` }}
                onPointerMove={(event) => {
                  const plot = plotRef.current;
                  if (!plot) return;
                  const rect = plot.getBoundingClientRect();
                  const plotWidth = Math.max(1, rect.width);
                  const ratio = Math.min(1, Math.max(0, (event.clientX - rect.left) / plotWidth));
                  const hoverStepMs = Math.max(1_000, Math.min(15_000, windowSpan / 240));
                  const nextHoverMs =
                    plotWindow.startMs +
                    Math.round((ratio * windowSpan) / hoverStepMs) * hoverStepMs;
                  setHoverAnchor({
                    x: Math.min(rect.width, Math.max(0, event.clientX - rect.left)),
                    y: Math.min(laneAreaHeightPx, Math.max(0, event.clientY - rect.top)),
                  });
                  const now = performance.now();
                  if (hoverMs != null && now - lastHoverUpdateMs.current < 50) return;
                  lastHoverUpdateMs.current = now;
                  setHoverMs((current) => (current === nextHoverMs ? current : nextHoverMs));
                }}
                onPointerLeave={() => {
                  setHoverMs(null);
                  setHoverAnchor(null);
                }}
              >
                <div
                  data-testid="dashboard-invocation-timeline-calls-axis"
                  className="relative w-16 shrink-0 text-[10px] text-base-content/50"
                  style={{ height: `${laneAreaHeightPx}px` }}
                >
                  <span className="pointer-events-none absolute left-0 top-0 z-20 leading-4">
                    {t("dashboard.activityOverview.timelineCallsAxis")}
                  </span>
                  <div
                    ref={callsAxisScrollRef}
                    className="absolute inset-x-0 top-0 overflow-y-auto overscroll-contain"
                    aria-hidden="true"
                    style={{
                      height: `${laneAreaHeightPx}px`,
                      scrollbarWidth: "none",
                    }}
                    onScroll={(event) => {
                      const scrollTop = event.currentTarget.scrollTop;
                      lastLaneScrollTopRef.current = scrollTop;
                      setLaneScrollTop(scrollTop);
                      const laneScroll = laneScrollRef.current;
                      if (laneScroll && laneScroll.scrollTop !== scrollTop) {
                        laneScroll.scrollTop = scrollTop;
                      }
                    }}
                  >
                    <div className="relative" style={{ height: `${lanePlotHeight}px` }}>
                      {callAxisTicks
                        .filter((tick) => tick.value > 0)
                        .map((tick, index) => (
                          <span
                            data-call-axis-tick
                            key={`${tick.value}-${tick.top}`}
                            className={`absolute left-0 ${index === 0 ? "" : "-translate-y-1/2"}`}
                            style={{ top: `${tick.top}px` }}
                          >
                            {tick.value}
                          </span>
                        ))}
                      {zeroIsVisible ? (
                        <>
                          <span
                            data-testid="dashboard-invocation-timeline-lane-zero"
                            className="absolute left-0 -translate-y-full"
                            style={{ top: `${lanePlotHeight}px` }}
                          >
                            0
                          </span>
                          <span
                            aria-hidden="true"
                            className="absolute right-0 h-px w-2 bg-base-content/25"
                            style={{ top: `${lanePlotHeight}px` }}
                          />
                        </>
                      ) : null}
                    </div>
                  </div>
                </div>
                <div
                  ref={plotRef}
                  data-testid="dashboard-invocation-timeline-plot"
                  className="relative min-w-0 flex-1"
                  style={{ height: `${chartHeightPx}px` }}
                >
                  <svg
                    data-testid="dashboard-invocation-timeline-ttft-overlay"
                    className="pointer-events-none absolute inset-x-0 top-0 z-0 w-full overflow-visible"
                    style={{ height: `${laneAreaHeightPx}px` }}
                    viewBox="0 0 100 100"
                    preserveAspectRatio="none"
                    role="img"
                    aria-label={t("dashboard.activityOverview.timelineTtftAxis")}
                  >
                    <path
                      d={ttft.path}
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="2"
                      vectorEffect="non-scaling-stroke"
                      className="text-info"
                    />
                  </svg>
                  <section
                    data-testid="dashboard-invocation-timeline-lane-scroll"
                    ref={laneScrollRef}
                    className="absolute inset-x-0 top-0 overflow-y-auto overscroll-contain"
                    tabIndex={0}
                    aria-label={`${t("dashboard.activityOverview.timelineTitle")}, ${t("dashboard.activityOverview.timelineCalls", { count: renderedData?.total ?? 0 })}`}
                    style={{ height: `${laneAreaHeightPx}px` }}
                    onScroll={(event) => {
                      const scrollTop = event.currentTarget.scrollTop;
                      lastLaneScrollTopRef.current = scrollTop;
                      setLaneScrollTop(scrollTop);
                      const callsAxisScroll = callsAxisScrollRef.current;
                      if (callsAxisScroll && callsAxisScroll.scrollTop !== scrollTop) {
                        callsAxisScroll.scrollTop = scrollTop;
                      }
                    }}
                  >
                    <div className="relative" style={{ height: `${lanePlotHeight}px` }}>
                      <div
                        className="absolute inset-x-0 top-0 z-10"
                        style={{ height: `${lanePlotHeight}px` }}
                      >
                        {callAxisGridValues.map((value) => (
                          <div
                            data-call-axis-grid
                            data-call-axis-value={value}
                            key={`call-axis-${value}`}
                            className="absolute inset-x-0 border-t border-dashed border-base-content/10"
                            style={{
                              top: `${callAxisTopForValue(value)}px`,
                            }}
                          />
                        ))}
                        {visibleLanes.map((item) => {
                          const status = resolveStatus(item.record);
                          const left = Math.max(0, Math.min(100, xFor(item.startMs)));
                          const right = Math.max(left, Math.min(100, xFor(item.endMs)));
                          const width = Math.max(0.25, right - left);
                          const statusLabel =
                            {
                              success: t("dashboard.activityOverview.timelineStatusSuccess"),
                              requesting: t("dashboard.activityOverview.timelineStatusRequesting"),
                              responding: t("dashboard.activityOverview.timelineStatusResponding"),
                              queued: t("dashboard.activityOverview.timelineStatusQueued"),
                              failed: t("dashboard.activityOverview.timelineStatusFailed"),
                              unknown: t("dashboard.activityOverview.timelineStatusUnknown"),
                              interrupted: t(
                                "dashboard.activityOverview.timelineStatusInterrupted",
                              ),
                            }[status] ?? t("dashboard.activityOverview.timelineStatusUnknown");
                          const occurredAtLabel = t(
                            "dashboard.activityOverview.timelineOccurredAt",
                            { time: formatInvocationTime(item.startMs, locale) },
                          );
                          const durationLabel = formatDuration(item.record, t);
                          const ttftLabel =
                            item.record.firstTokenMs != null
                              ? t("dashboard.activityOverview.timelineTtftValue", {
                                  value: Math.round(item.record.firstTokenMs),
                                })
                              : "";
                          const accessibleLabel = t(
                            "dashboard.activityOverview.timelineInvocationAria",
                            {
                              id: item.record.invokeId,
                              occurredAt: occurredAtLabel,
                              status: statusLabel,
                              duration: durationLabel,
                              ttft: ttftLabel,
                            },
                          );
                          return (
                            <div
                              role="img"
                              key={`${item.record.invokeId}:${item.record.occurredAt}`}
                              data-call-value={item.lane + 1}
                              className={`absolute flex appearance-none items-center overflow-visible ${laneHeight > 2 ? "rounded border shadow-sm" : "rounded-sm"} ${statusClass(status)}`}
                              aria-label={accessibleLabel}
                              style={{
                                left: `${left}%`,
                                top: `${laneTopFor(item.lane)}px`,
                                width: `${width}%`,
                                minWidth: "8px",
                                height: `${laneHeight}px`,
                                borderWidth: laneHeight > 2 ? undefined : 0,
                              }}
                              title={accessibleLabel}
                            />
                          );
                        })}
                      </div>
                    </div>
                  </section>
                  {hoverMs != null ? (
                    <div
                      className="pointer-events-none absolute inset-x-0 top-0 z-20 w-px bg-info/80"
                      style={{
                        height: `${laneAreaHeightPx}px`,
                        left: `calc(${xFor(hoverMs)}% - 0.5px)`,
                      }}
                    />
                  ) : null}
                  {hoverMs != null && hoverStats != null && hoverAnchor ? (
                    <div
                      ref={tooltipRef}
                      role="tooltip"
                      data-testid="dashboard-invocation-timeline-hover-tooltip"
                      data-hover-time-ms={hoverMs}
                      aria-hidden={tooltipPosition == null}
                      className="pointer-events-none absolute z-30 w-max min-w-0 max-w-[14rem] rounded-xl border px-3 py-2 text-[11px] leading-tight text-base-content transition-[opacity,transform] duration-150 ease-out motion-reduce:transition-none sm:min-w-[11rem]"
                      style={{
                        ...floatingSurfaceStyle("neutral", tooltipTheme),
                        left: tooltipPosition?.x ?? 8,
                        top: tooltipPosition?.y ?? 8,
                        maxWidth: "calc(100% - 16px)",
                        visibility: tooltipPosition ? "visible" : "hidden",
                      }}
                    >
                      <div className="font-semibold text-base-content">
                        {formatInvocationTime(hoverMs, locale)}
                      </div>
                      <div className="mt-1.5 space-y-1 text-base-content/75">
                        <div>
                          {t("dashboard.activityOverview.timelineParallel", {
                            count: hoverStats.total,
                          })}
                        </div>
                        <div>
                          {t("dashboard.activityOverview.timelineRunning", {
                            count: hoverStats.running,
                          })}
                        </div>
                        <div>
                          {t("dashboard.activityOverview.timelineQueued", {
                            count: hoverStats.queued,
                          })}
                        </div>
                      </div>
                    </div>
                  ) : null}
                  <div
                    data-testid="dashboard-invocation-timeline-x-axis"
                    className="pointer-events-none absolute inset-x-0 bottom-0 z-20 flex h-7 items-end border-t border-base-content/10 text-[11px] text-base-content/55"
                  >
                    {[0, 25, 50, 75, 100].map((tick) => (
                      <span
                        key={tick}
                        className={`absolute bottom-1 whitespace-nowrap ${tick === 25 || tick === 75 ? "hidden sm:inline" : ""} ${tick === 0 ? "" : tick === 100 ? "-translate-x-full" : "-translate-x-1/2"}`}
                        style={{ left: `${tick}%` }}
                      >
                        {new Date(
                          plotWindow.startMs + (windowSpan * tick) / 100,
                        ).toLocaleTimeString([], {
                          hour: "2-digit",
                          minute: "2-digit",
                        })}
                      </span>
                    ))}
                  </div>
                </div>
                <div
                  className="relative w-16 shrink-0 text-right text-[10px] text-base-content/50"
                  style={{ height: `${chartHeightPx}px` }}
                >
                  <span className="absolute right-0 top-0 leading-4">
                    {t("dashboard.activityOverview.timelineTtftAxis")}
                  </span>
                  {ttftTicks.map((tick, index) => (
                    <span
                      data-ttft-axis-tick
                      data-testid={
                        index === ttftTicks.length - 1
                          ? "dashboard-invocation-timeline-ttft-zero"
                          : undefined
                      }
                      key={`${tick.value}-${tick.top}`}
                      className={`absolute right-0 ${index === 0 ? "" : index === ttftTicks.length - 1 ? "-translate-y-full" : "-translate-y-1/2"}`}
                      style={{ top: `${tick.top}px` }}
                    >
                      {tick.value} ms
                    </span>
                  ))}
                  <span
                    aria-hidden="true"
                    className="absolute left-0 h-px w-2 bg-base-content/25"
                    style={{
                      top: `${ttftTicks.at(-1)?.top ?? plotOriginTopPx}px`,
                    }}
                  />
                </div>
              </div>
            </div>
          </div>
        </div>

        <div
          data-testid="dashboard-invocation-timeline-legend"
          className="flex flex-wrap items-center justify-center gap-3 text-[11px] text-base-content/65"
        >
          {[
            ["success", t("dashboard.activityOverview.timelineStatusSuccess")],
            ["requesting", t("dashboard.activityOverview.timelineStatusRequesting")],
            ["responding", t("dashboard.activityOverview.timelineStatusResponding")],
            ["queued", t("dashboard.activityOverview.timelineStatusQueued")],
            ["failed", t("dashboard.activityOverview.timelineStatusFailed")],
            ["interrupted", t("dashboard.activityOverview.timelineStatusInterrupted")],
            ["unknown", t("dashboard.activityOverview.timelineStatusUnknown")],
          ].map(([status, label]) => (
            <span key={status} className="inline-flex items-center gap-1">
              <span className={`h-2 w-2 rounded-sm ${statusClass(status).split(" ")[0]}`} />
              {label}
            </span>
          ))}
        </div>
        {renderedData?.total === 0 && !loading ? (
          <Alert variant="info">{t("dashboard.activityOverview.timelineEmpty")}</Alert>
        ) : null}
      </div>
    </div>
  );
}
