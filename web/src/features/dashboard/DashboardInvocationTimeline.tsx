import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { useCompactViewport } from "../../hooks/useCompactViewport";
import { useInvocationTimeline } from "../../hooks/useInvocationTimeline";
import useSseStatus from "../../hooks/useSseStatus";
import { useTranslation } from "../../i18n";
import type {
  InvocationTimelineRecord,
  InvocationTimelineResponse,
  TimeseriesResponse,
} from "../../lib/api";
import { recordTodayChartRender } from "../../lib/dashboardPerformanceDiagnostics";
import { AppIcon } from "../shared/AppIcon";

interface DashboardInvocationTimelineProps {
  response: TimeseriesResponse | null;
  loading: boolean;
  error?: string | null;
  closedNaturalDay?: boolean;
  upstreamAccountId?: number;
  liveRevision?: number;
  timelineData?: InvocationTimelineResponse | null;
  fallback: ReactNode;
}

export interface LaneRecord {
  record: InvocationTimelineRecord;
  startMs: number;
  endMs: number;
  lane: number;
}

export function getInvocationTimelineLaneCount(lanes: LaneRecord[]) {
  return Math.max(1, ...lanes.map((item) => item.lane + 1));
}

export function shouldFallbackForInvalidTimelineBounds(
  response: TimeseriesResponse | null,
  bounds: { startMs: number; endMs: number } | null,
  timelineDataOverride: InvocationTimelineResponse | null | undefined,
) {
  return response != null && bounds == null && timelineDataOverride == null;
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

function formatDuration(record: InvocationTimelineRecord) {
  if (record.isInFlight) return "进行中";
  if (record.tTotalMs == null) return "时长未知";
  if (record.tTotalMs < 1_000) return `${Math.round(record.tTotalMs)} ms`;
  return `${(record.tTotalMs / 1_000).toFixed(record.tTotalMs >= 10_000 ? 0 : 1)} s`;
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
const INVOCATION_LANE_MIN_HEIGHT_PX = 8;
const INVOCATION_LANE_MAX_HEIGHT_PX = 16;
const INVOCATION_LANE_GAP_PX = 1;

export interface InvocationTimelineLayout {
  chartHeightPx: number;
  laneAreaHeightPx: number;
  laneHeight: number;
  laneStep: number;
  lanePlotHeight: number;
  laneContentHeight: number;
  laneOffset: number;
  visibleLaneCount: number;
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
  const laneHeight = Math.max(
    INVOCATION_LANE_MIN_HEIGHT_PX,
    Math.min(
      INVOCATION_LANE_MAX_HEIGHT_PX,
      Math.floor(
        (laneAreaHeightPx - (visibleLaneCount - 1) * INVOCATION_LANE_GAP_PX) / visibleLaneCount,
      ),
    ),
  );
  const laneStep = laneHeight + INVOCATION_LANE_GAP_PX;
  const laneContentHeight =
    visibleLaneCount * laneHeight + Math.max(0, visibleLaneCount - 1) * INVOCATION_LANE_GAP_PX;
  const lanePlotHeight = Math.max(laneAreaHeightPx, laneContentHeight + 24);
  const laneOffset = Math.max(12, Math.floor((lanePlotHeight - laneContentHeight) / 2));
  return {
    chartHeightPx,
    laneAreaHeightPx,
    laneHeight,
    laneStep,
    lanePlotHeight,
    laneContentHeight,
    laneOffset,
    visibleLaneCount,
  };
}

export function assignInvocationTimelineLanes(
  records: InvocationTimelineRecord[],
  asOf: string,
  nowMs = Date.now(),
  advanceInFlight = true,
): LaneRecord[] {
  const referenceNowMs = parseEpoch(asOf) ?? nowMs;
  const laneEnds: number[] = [];
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
      const laneEndMs = endMs;
      let lane = laneEnds.findIndex((laneEnd) => laneEnd <= startMs);
      if (lane < 0) lane = laneEnds.length;
      laneEnds[lane] = laneEndMs;
      return { record, startMs, endMs, lane };
    });
}

function buildTtftPath(
  points: TimeseriesResponse["points"],
  windowStartMs: number,
  windowEndMs: number,
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
      const y = 100 - (point.value / maxValue) * 88 - 6;
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
  fallback,
}: DashboardInvocationTimelineProps) {
  const { t } = useTranslation();
  const isCompactViewport = useCompactViewport();
  const sseStatus = useSseStatus();
  const liveRefreshAllowed =
    closedNaturalDay || !["reconnecting", "disabled"].includes(sseStatus.phase);
  const liveConnected = closedNaturalDay || sseStatus.phase === "connected";
  const [hoverMs, setHoverMs] = useState<number | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [laneScrollTop, setLaneScrollTop] = useState(0);
  const lastHoverUpdateMs = useRef(0);
  const laneScrollRef = useRef<HTMLDivElement | null>(null);
  const callsAxisScrollRef = useRef<HTMLDivElement | null>(null);
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
  useEffect(() => {
    if (closedNaturalDay || !renderedData || !response) return;
    const lastPoint = response.points.at(-1);
    recordTodayChartRender(
      `${renderedData.rangeStart}:${renderedData.rangeEnd}:${renderedData.asOf}:${renderedData.records.length}:${response.rangeStart}:${response.rangeEnd}:${lastPoint?.totalCount ?? ""}`,
    );
  }, [closedNaturalDay, renderedData, response]);
  const lanes = useMemo(
    () =>
      renderedData
        ? assignInvocationTimelineLanes(
            renderedData.records,
            renderedData.asOf,
            nowMs,
            liveConnected,
          )
        : [],
    [liveConnected, nowMs, renderedData],
  );
  const laneCount = getInvocationTimelineLaneCount(lanes);
  const laneLayout = resolveInvocationTimelineLayout(laneCount, isCompactViewport);
  const hasInFlightLanes = lanes.some((item) => item.record.isInFlight);

  useEffect(() => {
    if (closedNaturalDay || !liveConnected || !hasInFlightLanes) return;
    const timer = globalThis.setInterval(() => setNowMs(Date.now()), 1_000);
    return () => globalThis.clearInterval(timer);
  }, [closedNaturalDay, hasInFlightLanes, liveConnected]);

  useEffect(() => {
    const scrollElement = laneScrollRef.current;
    const callsAxisScrollElement = callsAxisScrollRef.current;
    if (!scrollElement || !callsAxisScrollElement) return;
    const maxScrollTop = Math.max(0, scrollElement.scrollHeight - laneLayout.laneAreaHeightPx);
    if (maxScrollTop > 0 && scrollElement.scrollTop === 0) {
      scrollElement.scrollTop = maxScrollTop;
      callsAxisScrollElement.scrollTop = maxScrollTop;
    }
  }, [laneLayout.laneAreaHeightPx]);

  const plotWindow = timeline.window;
  const ttft = useMemo(
    () =>
      plotWindow && response
        ? buildTtftPath(response.points, plotWindow.startMs, plotWindow.endMs)
        : { path: "", maxValue: 1 },
    [plotWindow, response],
  );

  if (error || (!response && !loading && !timelineDataOverride)) return <>{fallback}</>;
  if (!closedNaturalDay && !liveRefreshAllowed && !timelineDataOverride) return <>{fallback}</>;
  if (shouldFallbackForInvalidTimelineBounds(response, timeline.bounds, timelineDataOverride)) {
    return <>{fallback}</>;
  }
  if (!plotWindow || (timeline.isLoading && !renderedData && !timelineDataOverride)) {
    return (
      <div
        className="min-h-64 animate-pulse rounded-lg bg-base-200/45"
        role="status"
        aria-label={t("chart.loading")}
      />
    );
  }
  if (renderedError || error || renderedData?.overLimit) return <>{fallback}</>;

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
    laneStep,
    lanePlotHeight,
    visibleLaneCount,
  } = laneLayout;
  const plotOriginTopPx = laneAreaHeightPx;
  const callAxisMaxValue = Math.max(visibleLaneCount, 1);
  const useLinearLanePositions = callAxisMaxValue <= 10;
  const linearLaneStep = laneStep;
  const callAxisTickCount = useLinearLanePositions ? callAxisMaxValue + 1 : 5;
  const visibleAxisTopValue = Math.min(
    callAxisMaxValue,
    Math.ceil((lanePlotHeight - laneScrollTop) / linearLaneStep),
  );
  const visibleAxisBottomValue = Math.max(
    0,
    Math.floor((lanePlotHeight - (laneScrollTop + laneAreaHeightPx)) / linearLaneStep),
  );
  const visibleAxisValueSpan = Math.max(1, visibleAxisTopValue - visibleAxisBottomValue);
  const callAxisTicks = Array.from({ length: callAxisTickCount }, (_, index) => {
    const fraction = index / Math.max(1, callAxisTickCount - 1);
    const value =
      index === callAxisTickCount - 1
        ? visibleAxisBottomValue
        : Math.round(visibleAxisTopValue - visibleAxisValueSpan * fraction);
    return {
      value,
      top: lanePlotHeight - value * linearLaneStep,
    };
  });
  const zeroIsVisible =
    lanePlotHeight >= laneScrollTop && lanePlotHeight <= laneScrollTop + laneAreaHeightPx;
  const laneTopFor = (lane: number) =>
    lanePlotHeight - (lane + 1) * linearLaneStep - laneHeight / 2;
  const laneCenterFor = (lane: number) => lanePlotHeight - (lane + 1) * linearLaneStep;

  return (
    <div data-testid="dashboard-today-activity-chart">
      <div className="flex flex-col gap-3" data-testid="dashboard-invocation-timeline">
        <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-base-content/65">
          <div className="flex items-center gap-3">
            <span className="font-semibold text-base-content">
              {t("dashboard.activityOverview.timelineTitle")}
            </span>
            <span>
              {t("dashboard.activityOverview.timelineCalls", {
                count: renderedData?.total ?? 0,
              })}
            </span>
            {timeline.isRefreshing ? (
              <span className="text-info">{t("dashboard.activityOverview.timelineLive")}</span>
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

        <div className="overflow-x-auto rounded-lg border border-base-content/10 bg-base-300/20">
          <div className="min-w-0 p-3 sm:min-w-[680px]">
            <div
              data-testid="dashboard-invocation-timeline-lanes"
              className="relative h-[21rem] overflow-hidden overscroll-contain desktop:h-80"
              style={{ height: `${chartHeightPx}px` }}
            >
              <div
                className="relative flex"
                style={{ height: `${chartHeightPx}px` }}
                onPointerMove={(event) => {
                  const rect = event.currentTarget.getBoundingClientRect();
                  const ratio = Math.min(
                    1,
                    Math.max(0, (event.clientX - rect.left - 48) / Math.max(1, rect.width - 112)),
                  );
                  const hoverStepMs = Math.max(1_000, Math.min(15_000, windowSpan / 240));
                  const nextHoverMs =
                    plotWindow.startMs +
                    Math.round((ratio * windowSpan) / hoverStepMs) * hoverStepMs;
                  const now = performance.now();
                  if (now - lastHoverUpdateMs.current < 50) return;
                  lastHoverUpdateMs.current = now;
                  setHoverMs((current) => (current === nextHoverMs ? current : nextHoverMs));
                }}
                onPointerLeave={() => setHoverMs(null)}
              >
                <div
                  data-testid="dashboard-invocation-timeline-calls-axis"
                  className="relative w-12 shrink-0 text-[10px] text-base-content/50"
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
                <div className="relative min-w-0 flex-1" style={{ height: `${chartHeightPx}px` }}>
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
                  <div
                    data-testid="dashboard-invocation-timeline-lane-scroll"
                    ref={laneScrollRef}
                    className="absolute inset-x-0 top-0 overflow-y-auto overscroll-contain"
                    style={{ height: `${laneAreaHeightPx}px` }}
                    onScroll={(event) => {
                      const scrollTop = event.currentTarget.scrollTop;
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
                        {Array.from({ length: visibleLaneCount }, (_, lane) => (
                          <div
                            key={`lane-${lane}`}
                            className="absolute inset-x-0 border-t border-dashed border-base-content/10"
                            style={{
                              top: `${laneCenterFor(lane)}px`,
                            }}
                          />
                        ))}
                        {lanes.map((item) => {
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
                          const ttftLabel =
                            item.record.firstTokenMs != null
                              ? ` · TTFT ${Math.round(item.record.firstTokenMs)} ms`
                              : "";
                          return (
                            <button
                              type="button"
                              key={`${item.record.invokeId}:${item.record.occurredAt}`}
                              className={`absolute flex appearance-none items-center overflow-visible rounded border shadow-sm ${statusClass(status)}`}
                              aria-label={`${item.record.invokeId} · ${statusLabel} · ${formatDuration(item.record)}${ttftLabel}`}
                              style={{
                                left: `${left}%`,
                                top: `${laneTopFor(item.lane)}px`,
                                width: `${width}%`,
                                minWidth: "8px",
                                height: `${laneHeight}px`,
                              }}
                              title={`${item.record.invokeId} · ${formatDuration(item.record)}${ttftLabel}`}
                              onFocus={() => setHoverMs((item.startMs + item.endMs) / 2)}
                              onBlur={() => setHoverMs(null)}
                              onKeyDown={(event) => {
                                if (event.key === "Enter" || event.key === " ") {
                                  event.preventDefault();
                                  setHoverMs((item.startMs + item.endMs) / 2);
                                }
                              }}
                            />
                          );
                        })}
                      </div>
                    </div>
                  </div>
                  {hoverMs != null ? (
                    <div
                      className="pointer-events-none absolute inset-x-0 top-0 z-20 w-px bg-info/80"
                      style={{
                        height: `${laneAreaHeightPx}px`,
                        left: `calc(${xFor(hoverMs)}% - 0.5px)`,
                      }}
                    />
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
                  <span className="absolute right-0 top-5">{Math.round(ttft.maxValue)} ms</span>
                  <span
                    data-testid="dashboard-invocation-timeline-ttft-zero"
                    className="absolute right-0 -translate-y-full"
                    style={{ top: `${plotOriginTopPx}px` }}
                  >
                    0 ms
                  </span>
                  <span
                    aria-hidden="true"
                    className="absolute left-0 h-px w-2 bg-base-content/25"
                    style={{ top: `${plotOriginTopPx}px` }}
                  />
                </div>
              </div>
            </div>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-3 text-[11px] text-base-content/65">
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
        {hoverStats ? (
          <div className="text-xs text-base-content/70">
            {new Date(hoverMs ?? 0).toLocaleTimeString()} ·{" "}
            {t("dashboard.activityOverview.timelineParallel", {
              count: hoverStats.total,
            })}{" "}
            ·{" "}
            {t("dashboard.activityOverview.timelineRunning", {
              count: hoverStats.running,
            })}{" "}
            ·{" "}
            {t("dashboard.activityOverview.timelineQueued", {
              count: hoverStats.queued,
            })}
          </div>
        ) : null}
        {renderedData?.total === 0 && !loading ? (
          <Alert variant="info">{t("dashboard.activityOverview.timelineEmpty")}</Alert>
        ) : null}
      </div>
    </div>
  );
}
