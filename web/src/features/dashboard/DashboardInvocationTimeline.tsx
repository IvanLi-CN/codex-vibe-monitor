import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { useInvocationTimeline } from "../../hooks/useInvocationTimeline";
import useSseStatus from "../../hooks/useSseStatus";
import { useTranslation } from "../../i18n";
import type {
  InvocationTimelineRecord,
  InvocationTimelineResponse,
  TimeseriesResponse,
} from "../../lib/api";
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

function parseEpoch(value: string | null | undefined) {
  if (!value) return null;
  const parsed = Date.parse(value);
  return Number.isFinite(parsed) ? parsed : null;
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
      const terminalEnd = parseEpoch(record.endAt);
      const endMs = record.isInFlight
        ? Math.max(referenceNowMs, advanceInFlight ? nowMs : referenceNowMs, startMs + 1)
        : Math.max(terminalEnd ?? startMs + 1, startMs + 1);
      const laneEndMs = record.isInFlight
        ? endMs
        : Math.max(terminalEnd ?? referenceNowMs, startMs + 1);
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
  const sseStatus = useSseStatus();
  const liveRefreshAllowed =
    closedNaturalDay || !["reconnecting", "disabled"].includes(sseStatus.phase);
  const liveConnected = closedNaturalDay || sseStatus.phase === "connected";
  const [hoverMs, setHoverMs] = useState<number | null>(null);
  const [nowMs, setNowMs] = useState(() => Date.now());
  const lastHoverUpdateMs = useRef(0);
  const timeline = useInvocationTimeline({
    response,
    closedNaturalDay,
    upstreamAccountId,
    liveRevision,
    liveRefreshAllowed,
    enabled: !timelineDataOverride,
  });

  useEffect(() => {
    if (closedNaturalDay || !liveConnected) return;
    const timer = globalThis.setInterval(() => setNowMs(Date.now()), 1_000);
    return () => globalThis.clearInterval(timer);
  }, [closedNaturalDay, liveConnected]);

  const renderedData = timelineDataOverride ?? timeline.data;
  const renderedError = timelineDataOverride ? null : timeline.error;
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
  const laneCount = Math.max(1, (lanes.at(-1)?.lane ?? 0) + 1);
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
  const overviewPoints = response?.points ?? [];
  const overviewMax = Math.max(1, ...overviewPoints.map((point) => point.totalCount));

  return (
    <div className="flex flex-col gap-3" data-testid="dashboard-invocation-timeline">
      <div className="flex flex-wrap items-center justify-between gap-2 text-xs text-base-content/65">
        <div className="flex items-center gap-3">
          <span className="font-semibold text-base-content">
            {t("dashboard.activityOverview.timelineTitle")}
          </span>
          <span>
            {t("dashboard.activityOverview.timelineCalls", { count: renderedData?.total ?? 0 })}
          </span>
          {timeline.isRefreshing ? (
            <span className="text-info">{t("dashboard.activityOverview.timelineLive")}</span>
          ) : null}
        </div>
        <div className="flex items-center gap-1">
          <button
            type="button"
            className="icon-button h-8 w-8"
            aria-label={t("dashboard.activityOverview.timelineShortenWindow")}
            title={t("dashboard.activityOverview.timelineShortenWindow")}
            onClick={() => {
              const center = (plotWindow.startMs + plotWindow.endMs) / 2;
              const span = Math.max(5 * 60_000, windowSpan / 2);
              timeline.setWindow({ startMs: center - span / 2, endMs: center + span / 2 });
            }}
          >
            <AppIcon name="minus" className="h-4 w-4" aria-hidden />
          </button>
          <button
            type="button"
            className="icon-button h-8 w-8"
            aria-label={t("dashboard.activityOverview.timelineLengthenWindow")}
            title={t("dashboard.activityOverview.timelineLengthenWindow")}
            onClick={() => {
              const center = (plotWindow.startMs + plotWindow.endMs) / 2;
              const span = windowSpan * 2;
              timeline.setWindow({ startMs: center - span / 2, endMs: center + span / 2 });
            }}
          >
            <AppIcon name="plus" className="h-4 w-4" aria-hidden />
          </button>
          <button
            type="button"
            className="icon-button h-8 w-8"
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
            className="icon-button h-8 w-8"
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
        <div className="min-w-[680px] p-3">
          <div className="mb-2 flex items-center gap-2 text-[11px] text-base-content/55">
            <span className="w-12 shrink-0">
              {t("dashboard.activityOverview.timelineLaneAxis")}
            </span>
            <div className="relative h-5 flex-1">
              {[0, 25, 50, 75, 100].map((tick) => (
                <span key={tick} className="absolute -translate-x-1/2" style={{ left: `${tick}%` }}>
                  {new Date(plotWindow.startMs + (windowSpan * tick) / 100).toLocaleTimeString([], {
                    hour: "2-digit",
                    minute: "2-digit",
                  })}
                </span>
              ))}
            </div>
            <span className="w-16 text-right">
              {t("dashboard.activityOverview.timelineTtftAxis")}
            </span>
          </div>

          <div
            className="relative flex"
            onPointerMove={(event) => {
              const rect = event.currentTarget.getBoundingClientRect();
              const ratio = Math.min(
                1,
                Math.max(0, (event.clientX - rect.left - 48) / Math.max(1, rect.width - 112)),
              );
              const hoverStepMs = Math.max(1_000, Math.min(15_000, windowSpan / 240));
              const nextHoverMs =
                plotWindow.startMs + Math.round((ratio * windowSpan) / hoverStepMs) * hoverStepMs;
              const now = performance.now();
              if (now - lastHoverUpdateMs.current < 50) return;
              lastHoverUpdateMs.current = now;
              setHoverMs((current) => (current === nextHoverMs ? current : nextHoverMs));
            }}
            onPointerLeave={() => setHoverMs(null)}
          >
            <div className="relative w-12 shrink-0" style={{ height: `${laneCount * 30 + 12}px` }}>
              <span className="absolute left-0 top-1 text-[10px] text-base-content/50">
                {laneCount}
              </span>
              <span className="absolute bottom-1 left-0 text-[10px] text-base-content/50">1</span>
            </div>
            <div className="relative flex-1" style={{ height: `${laneCount * 30 + 12}px` }}>
              {Array.from({ length: laneCount }, (_, lane) => (
                <div
                  key={`lane-${lane}`}
                  className="absolute inset-x-0 border-t border-dashed border-base-content/10"
                  style={{ top: `${lane * 30 + 14}px` }}
                />
              ))}
              {lanes.map((item) => {
                const status = resolveStatus(item.record);
                const left = Math.max(0, Math.min(100, xFor(item.startMs)));
                const right = Math.max(left, Math.min(100, xFor(item.endMs)));
                const width = Math.max(0.25, right - left);
                const marker =
                  item.record.firstTokenMs != null
                    ? Math.max(left, Math.min(100, xFor(item.startMs + item.record.firstTokenMs)))
                    : null;
                const statusLabel =
                  {
                    success: t("dashboard.activityOverview.timelineStatusSuccess"),
                    requesting: t("dashboard.activityOverview.timelineStatusRequesting"),
                    responding: t("dashboard.activityOverview.timelineStatusResponding"),
                    queued: t("dashboard.activityOverview.timelineStatusQueued"),
                    failed: t("dashboard.activityOverview.timelineStatusFailed"),
                    unknown: t("dashboard.activityOverview.timelineStatusUnknown"),
                    interrupted: t("dashboard.activityOverview.timelineStatusUnknown"),
                  }[status] ?? t("dashboard.activityOverview.timelineStatusUnknown");
                return (
                  <button
                    type="button"
                    key={`${item.record.invokeId}:${item.record.occurredAt}`}
                    className={`absolute flex appearance-none items-center overflow-visible rounded border px-1 text-left text-[10px] font-medium shadow-sm ${statusClass(status)}`}
                    aria-label={`${item.record.invokeId} · ${statusLabel} · ${formatDuration(item.record)}`}
                    style={{
                      left: `${left}%`,
                      top: `${item.lane * 30 + 5}px`,
                      width: `${width}%`,
                      minWidth: "3px",
                      height: "18px",
                    }}
                    title={`${item.record.invokeId} · ${formatDuration(item.record)}`}
                    onFocus={() => setHoverMs((item.startMs + item.endMs) / 2)}
                    onBlur={() => setHoverMs(null)}
                    onKeyDown={(event) => {
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        setHoverMs((item.startMs + item.endMs) / 2);
                      }
                    }}
                  >
                    <span className="truncate">{item.record.invokeId.slice(0, 8)}</span>
                    {marker != null ? (
                      <span
                        className="absolute -top-1 h-5 w-px bg-base-content"
                        style={{ left: `${((marker - left) / Math.max(width, 0.25)) * 100}%` }}
                      />
                    ) : null}
                  </button>
                );
              })}
              {hoverMs != null ? (
                <div
                  className="pointer-events-none absolute inset-y-0 w-px bg-info/80"
                  style={{ left: `${xFor(hoverMs)}%` }}
                />
              ) : null}
            </div>
            <div className="w-16 shrink-0 text-right text-[10px] text-base-content/50">
              <span>{Math.round(ttft.maxValue)} ms</span>
              <span className="absolute bottom-0 right-0">0 ms</span>
            </div>
          </div>

          <div className="mt-2 flex items-center gap-2">
            <span className="w-12 shrink-0 text-[10px] text-base-content/50">
              {t("dashboard.activityOverview.timelineTtftAxis")}
            </span>
            <svg
              className="h-16 flex-1 overflow-visible"
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
            <span className="w-16" />
          </div>
        </div>
      </div>

      <div className="flex items-center gap-2 text-[11px] text-base-content/60">
        <span className="w-12 shrink-0">{t("dashboard.activityOverview.timelineAllDay")}</span>
        <div className="flex h-8 min-w-0 flex-1 items-end gap-px">
          {overviewPoints.map((point) => {
            const height = Math.max(2, (point.totalCount / overviewMax) * 100);
            const pointStart = parseEpoch(point.bucketStart) ?? plotWindow.startMs;
            const active = pointStart >= plotWindow.startMs && pointStart < plotWindow.endMs;
            return (
              <button
                key={point.bucketStart}
                type="button"
                className={`min-w-0 flex-1 rounded-t-sm ${active ? "bg-info" : "bg-base-content/25 hover:bg-base-content/45"}`}
                style={{ height: `${height}%` }}
                aria-label={`${t("dashboard.activityOverview.timelineCalls", { count: point.totalCount })} · ${new Date(point.bucketStart).toLocaleTimeString()}`}
                onClick={() => {
                  const start = parseEpoch(point.bucketStart);
                  const end = parseEpoch(point.bucketEnd);
                  if (start != null && end != null)
                    timeline.setWindow({ startMs: start, endMs: end });
                }}
              />
            );
          })}
        </div>
      </div>

      <div className="flex flex-wrap items-center gap-3 text-[11px] text-base-content/65">
        {[
          ["success", t("dashboard.activityOverview.timelineStatusSuccess")],
          ["requesting", t("dashboard.activityOverview.timelineStatusRequesting")],
          ["responding", t("dashboard.activityOverview.timelineStatusResponding")],
          ["queued", t("dashboard.activityOverview.timelineStatusQueued")],
          ["failed", t("dashboard.activityOverview.timelineStatusFailed")],
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
          {t("dashboard.activityOverview.timelineParallel", { count: hoverStats.total })} ·{" "}
          {t("dashboard.activityOverview.timelineRunning", { count: hoverStats.running })} ·{" "}
          {t("dashboard.activityOverview.timelineQueued", { count: hoverStats.queued })}
        </div>
      ) : null}
      {renderedData?.total === 0 && !loading ? (
        <Alert variant="info">{t("dashboard.activityOverview.timelineEmpty")}</Alert>
      ) : null}
    </div>
  );
}
