import type { JSX } from "react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { useSubscriptionTopic } from "../../hooks/useSubscriptionTopic";
import {
  fetchManagedTaskWorkload,
  type ManagedTask,
  type TaskWorkloadMetric,
  type TaskWorkloadSample,
  type TaskWorkloadTrend,
} from "../../lib/api";
import { AppIcon } from "../shared/AppIcon";

const WINDOW_MS = 24 * 60 * 60 * 1_000;
const CACHE_FRESHNESS_MS = 30_000;
const MAX_CACHE_ENTRIES = 64;
const MAX_CONCURRENT_LOADS = 4;
const workloadCache = new Map<string, TaskWorkloadTrend>();
const workloadLiveRevisions = new Map<string, number>();
const workloadLoads = new Map<string, Promise<TaskWorkloadTrend>>();
const workloadQueue: Array<{
  key: string;
  cancelled: boolean;
  promise: Promise<TaskWorkloadTrend>;
  resolve: (value: TaskWorkloadTrend) => void;
  reject: (reason: unknown) => void;
}> = [];
let activeLoads = 0;
const workloadTopicQueue: Array<{
  key: string;
  cancelled: boolean;
  grant: () => void;
}> = [];
const activeWorkloadTopics = new Set<string>();

function pumpWorkloadTopics(): void {
  while (activeWorkloadTopics.size < MAX_CONCURRENT_LOADS && workloadTopicQueue.length > 0) {
    const next = workloadTopicQueue.shift();
    if (!next || next.cancelled) continue;
    activeWorkloadTopics.add(next.key);
    next.grant();
  }
}

function acquireWorkloadTopic(key: string, grant: () => void): () => void {
  if (activeWorkloadTopics.has(key)) {
    grant();
    return () => undefined;
  }
  const request = { key, cancelled: false, grant };
  workloadTopicQueue.push(request);
  pumpWorkloadTopics();
  return () => {
    request.cancelled = true;
    if (activeWorkloadTopics.delete(key)) pumpWorkloadTopics();
  };
}

function trimCache(): void {
  while (workloadCache.size > MAX_CACHE_ENTRIES) {
    const oldest = workloadCache.keys().next().value;
    if (oldest == null) return;
    workloadCache.delete(oldest);
  }
}

function cacheTrend(
  taskKey: string,
  trend: TaskWorkloadTrend,
  source: "http" | "topic" = "http",
): void {
  const liveRevision = workloadLiveRevisions.get(taskKey);
  if (source === "http" && liveRevision != null && trend.revision <= liveRevision) return;
  if (source === "topic") {
    workloadLiveRevisions.set(taskKey, Math.max(liveRevision ?? -1, trend.revision));
  }
  const cached = workloadCache.get(taskKey);
  if (cached && trend.revision < cached.revision) return;
  workloadCache.set(taskKey, trend);
  trimCache();
}

function isFreshTrend(trend: TaskWorkloadTrend): boolean {
  const observedAt = Date.parse(trend.observedAt ?? trend.windowEnd ?? "");
  return Number.isFinite(observedAt) && Date.now() - observedAt <= CACHE_FRESHNESS_MS;
}

function cancelQueuedWorkload(taskKey: string): void {
  for (const request of workloadQueue) {
    if (request.key === taskKey && !request.cancelled) {
      request.cancelled = true;
      if (workloadLoads.get(taskKey) === request.promise) workloadLoads.delete(taskKey);
      request.reject(new DOMException("workload load cancelled", "AbortError"));
    }
  }
}

function pumpLoads(): void {
  while (activeLoads < MAX_CONCURRENT_LOADS && workloadQueue.length > 0) {
    const next = workloadQueue.shift();
    if (!next) return;
    if (next.cancelled) {
      if (workloadLoads.get(next.key) === next.promise) workloadLoads.delete(next.key);
      next.reject(new DOMException("workload load cancelled", "AbortError"));
      continue;
    }
    activeLoads += 1;
    void fetchManagedTaskWorkload(next.key, { windowHours: 24, limit: 200 })
      .then((trend) => {
        cacheTrend(next.key, trend, "http");
        next.resolve(trend);
      })
      .catch(next.reject)
      .finally(() => {
        activeLoads -= 1;
        if (workloadLoads.get(next.key) === next.promise) workloadLoads.delete(next.key);
        pumpLoads();
      });
  }
}

function loadWorkload(taskKey: string, refresh = false): Promise<TaskWorkloadTrend> {
  const cached = refresh ? undefined : workloadCache.get(taskKey);
  if (cached && isFreshTrend(cached)) {
    workloadCache.delete(taskKey);
    workloadCache.set(taskKey, cached);
    return Promise.resolve(cached);
  }
  const existing = workloadLoads.get(taskKey);
  if (existing) return existing;
  let resolveLoad!: (value: TaskWorkloadTrend) => void;
  let rejectLoad!: (reason: unknown) => void;
  const promise = new Promise<TaskWorkloadTrend>((resolve, reject) => {
    resolveLoad = resolve;
    rejectLoad = reject;
  });
  workloadQueue.push({
    key: taskKey,
    cancelled: false,
    promise,
    resolve: resolveLoad,
    reject: rejectLoad,
  });
  pumpLoads();
  workloadLoads.set(taskKey, promise);
  return promise;
}

function metricValue(metric: TaskWorkloadMetric | null | undefined): number | null {
  return typeof metric?.value === "number" && Number.isFinite(metric.value) ? metric.value : null;
}

function sampleTime(sample: TaskWorkloadSample): number | null {
  const value = Date.parse(sample.attemptedAt);
  return Number.isFinite(value) ? value : null;
}

type SparklineMetric = "pending" | "discovered" | "processed";

interface ChartPath {
  d: string;
  firstX: number;
  lastX: number;
  points: Array<{ x: number; y: number }>;
}

function metricIdentity(sample: TaskWorkloadSample, metric: SparklineMetric): string {
  const value = sample[metric];
  return `${value?.unit ?? ""}\u0000${value?.scope ?? ""}\u0000${value?.range ?? ""}`;
}

function metricUnit(sample: TaskWorkloadSample, metric: SparklineMetric): string | null {
  const unit = sample[metric]?.unit?.trim();
  return unit ? unit : null;
}

function chartUnits(
  samples: TaskWorkloadSample[],
  trend: TaskWorkloadTrend | null,
): Record<SparklineMetric, string | null> {
  return (["pending", "discovered", "processed"] as const).reduce(
    (result, metric) => {
      const observedUnit = samples
        .map((sample) => metricUnit(sample, metric))
        .find((unit): unit is string => unit != null);
      result[metric] = observedUnit ?? trend?.capabilities?.[metric]?.unit?.trim() ?? null;
      return result;
    },
    {} as Record<SparklineMetric, string | null>,
  );
}

function hasMixedUnits(samples: TaskWorkloadSample[]): boolean {
  const units = new Set(
    samples.flatMap((sample) =>
      (["pending", "discovered", "processed"] as const).flatMap((metric) => {
        const unit = metricUnit(sample, metric);
        return unit ? [unit] : [];
      }),
    ),
  );
  return units.size > 1;
}

function chartPaths(
  samples: TaskWorkloadSample[],
  metric: SparklineMetric,
  maxValue: number,
  now: number,
  unit: string | null,
  coverageGaps: TaskWorkloadTrend["coverageGaps"],
): ChartPath[] {
  const ordered = [...samples].sort((left, right) => {
    const timeOrder = (sampleTime(left) ?? 0) - (sampleTime(right) ?? 0);
    return timeOrder || left.sampleId.localeCompare(right.sampleId);
  });
  const paths: ChartPath[] = [];
  let points: Array<{ x: number; y: number }> = [];
  let previousTime: number | null = null;
  let previousIdentity: string | null = null;
  const flush = () => {
    if (points.length > 0) {
      paths.push({
        d: `M ${points.map(({ x, y }) => `${x.toFixed(2)},${y.toFixed(2)}`).join(" L ")}`,
        firstX: points[0].x,
        lastX: points.at(-1)?.x ?? points[0].x,
        points,
      });
    }
    points = [];
  };
  for (const sample of ordered) {
    const time = sampleTime(sample);
    const value = metricValue(sample[metric]);
    if (
      time == null ||
      value == null ||
      sample[metric]?.coverage === "unknown" ||
      (unit == null ? metricUnit(sample, metric) != null : metricUnit(sample, metric) !== unit) ||
      time < now - WINDOW_MS ||
      time > now
    ) {
      flush();
      previousTime = null;
      previousIdentity = null;
      continue;
    }
    const previous = previousTime;
    const gapBetweenSamples =
      previous != null &&
      (coverageGaps ?? []).some((gap) => {
        const start = Date.parse(gap.startedAt);
        const end = gap.finishedAt == null ? now : Date.parse(gap.finishedAt);
        return Number.isFinite(start) && Number.isFinite(end) && start <= time && end >= previous;
      });
    const identity = metricIdentity(sample, metric);
    if (gapBetweenSamples || (previousIdentity != null && previousIdentity !== identity)) flush();
    const x = ((time - (now - WINDOW_MS)) / WINDOW_MS) * 320;
    const y = 48 - (Math.max(0, value) / maxValue) * 40;
    points.push({ x, y });
    previousTime = time;
    previousIdentity = identity;
  }
  flush();
  return paths;
}

function areaPath(path: ChartPath, baselineY: number): string {
  return `${path.d} L ${path.lastX.toFixed(2)},${baselineY} L ${path.firstX.toFixed(2)},${baselineY} Z`;
}

function formatMetric(value: number | null | undefined): string {
  return value == null ? "未知" : value.toLocaleString("zh-CN");
}

function formatWorkloadMetric(metric: TaskWorkloadMetric | null | undefined): string {
  if (!metric) return "未知";
  return `${formatMetric(metric.value)} · ${metric.unit || "单位未知"} · ${metric.scope || "范围未知"} · ${metric.range || "范围未知"} · ${coverageLabel(metric.coverage)}`;
}

function formatDuration(durationMs: number | null | undefined): string {
  if (durationMs == null || !Number.isFinite(durationMs)) return "未知";
  if (durationMs < 1_000) return `${Math.max(0, Math.round(durationMs))} 毫秒`;
  const seconds = Math.floor(durationMs / 1_000);
  const minutes = Math.floor(seconds / 60);
  const remainder = seconds % 60;
  return minutes > 0 ? `${minutes} 分 ${String(remainder).padStart(2, "0")} 秒` : `${remainder} 秒`;
}

function formatTime(value: string | null | undefined): string {
  if (!value) return "未知";
  const timestamp = Date.parse(value);
  if (!Number.isFinite(timestamp)) return "未知";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
    timeZone: "Asia/Shanghai",
  }).format(new Date(timestamp));
}

function resultLabel(status: string | undefined): string {
  switch (status) {
    case "success":
      return "成功";
    case "failed":
      return "失败";
    case "partial":
      return "部分完成";
    case "skipped":
      return "确认跳过";
    case "running":
      return "运行中";
    case "unknown":
      return "未知";
    default:
      return status || "未知";
  }
}

function coverageLabel(coverage: string | undefined): string {
  switch (coverage) {
    case "observed":
    case "recorded":
      return "已记录";
    case "no recorded attempts":
      return "暂无运行记录";
    case "not applicable":
      return "不适用";
    case "some metrics unknown":
      return "部分未知";
    case "coverage gap":
      return "观测有缺口";
    default:
      return coverage || "未知";
  }
}

export interface TaskWorkloadSparklineProps {
  task: ManagedTask;
  dark: boolean;
  mode?: "standalone" | "background";
}

export function TaskWorkloadSparkline({
  task,
  dark,
  mode = "standalone",
}: TaskWorkloadSparklineProps): JSX.Element {
  const backgroundMode = mode === "background";
  const containerRef = useRef<HTMLDivElement | null>(null);
  const [visible, setVisible] = useState(false);
  const [pageVisible, setPageVisible] = useState(
    () => typeof document === "undefined" || document.visibilityState !== "hidden",
  );
  const [now, setNow] = useState(() => Date.now());
  const [trend, setTrend] = useState<TaskWorkloadTrend | null>(
    () => workloadCache.get(task.taskKey) ?? null,
  );
  const trendRevisionRef = useRef(trend?.revision ?? -1);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const detailsButtonRef = useRef<HTMLButtonElement | null>(null);
  const [detailsPosition, setDetailsPosition] = useState<{
    top: number;
    left: number;
    width: number;
  } | null>(null);
  const [topicSlotAcquired, setTopicSlotAcquired] = useState(false);
  const activeVisible = visible && pageVisible;
  const applyTrend = useCallback(
    (next: TaskWorkloadTrend, source: "http" | "topic"): boolean => {
      const liveRevision = workloadLiveRevisions.get(task.taskKey);
      // SSE is the live source. A same-revision HTTP response may have been
      // assembled before the event was flushed, so it must not roll the row back.
      if (source === "http" && liveRevision != null && next.revision <= liveRevision) {
        return false;
      }
      if (next.revision < trendRevisionRef.current) return false;
      if (source === "topic" && next.revision === trendRevisionRef.current) return false;
      trendRevisionRef.current = next.revision;
      setTrend(next);
      cacheTrend(task.taskKey, next, source);
      return true;
    },
    [task.taskKey],
  );
  const topic = useSubscriptionTopic<TaskWorkloadTrend>(
    topicSlotAcquired
      ? {
          topic: "system.managed-tasks.workload",
          params: { taskKey: task.taskKey, windowHours: "24", limit: "200" },
        }
      : null,
    topicSlotAcquired,
  );

  useEffect(() => {
    const node = containerRef.current;
    if (!node) return;
    if (typeof IntersectionObserver === "undefined") {
      setVisible(true);
      return;
    }
    const observer = new IntersectionObserver(
      ([entry]) => setVisible(entry?.isIntersecting === true),
      { root: null, rootMargin: "0px", threshold: 0 },
    );
    observer.observe(node);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const onVisibilityChange = () => {
      const nextVisible = document.visibilityState !== "hidden";
      setPageVisible(nextVisible);
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => document.removeEventListener("visibilitychange", onVisibilityChange);
  }, []);

  useEffect(() => {
    if (!activeVisible) return;
    const release = acquireWorkloadTopic(task.taskKey, () => setTopicSlotAcquired(true));
    return () => {
      release();
      setTopicSlotAcquired(false);
    };
  }, [activeVisible, task.taskKey]);

  useEffect(() => {
    const cached = workloadCache.get(task.taskKey) ?? null;
    trendRevisionRef.current = cached?.revision ?? -1;
    setTrend(cached);
  }, [task.taskKey]);

  useEffect(() => {
    if (!activeVisible) return;
    let active = true;
    void loadWorkload(task.taskKey)
      .then((next) => {
        if (active && applyTrend(next, "http")) {
          setLoadError(null);
        }
      })
      .catch((reason: unknown) => {
        if (active) setLoadError(reason instanceof Error ? reason.message : String(reason));
      });
    return () => {
      active = false;
      cancelQueuedWorkload(task.taskKey);
    };
  }, [activeVisible, applyTrend, task.taskKey]);

  useEffect(() => {
    if (topic.data) {
      applyTrend(topic.data, "topic");
      setLoadError(null);
    }
  }, [applyTrend, topic.data]);

  useEffect(() => {
    if (!activeVisible) return;
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, [activeVisible]);

  const samples = trend?.samples ?? [];
  const units = chartUnits(samples, trend);
  const mixedUnits = hasMixedUnits(samples);
  const maxValues = (["pending", "discovered", "processed"] as const).reduce(
    (result, metric) => {
      const values = samples.flatMap((sample) => {
        const value = metricValue(sample[metric]);
        const sampleUnit = metricUnit(sample, metric);
        return value == null ||
          sample[metric]?.coverage === "unknown" ||
          (units[metric] == null ? sampleUnit != null : sampleUnit !== units[metric])
          ? []
          : [value];
      });
      result[metric] = Math.max(1, ...values);
      return result;
    },
    {} as Record<SparklineMetric, number>,
  );
  const paths = useMemo(
    () => ({
      pending: chartPaths(
        samples,
        "pending",
        maxValues.pending,
        now,
        units.pending,
        trend?.coverageGaps,
      ),
      discovered: chartPaths(
        samples,
        "discovered",
        maxValues.discovered,
        now,
        units.discovered,
        trend?.coverageGaps,
      ),
      processed: chartPaths(
        samples,
        "processed",
        maxValues.processed,
        now,
        units.processed,
        trend?.coverageGaps,
      ),
    }),
    [maxValues, now, samples, trend?.coverageGaps, units],
  );
  const latest = samples.at(-1);
  const latestValues = {
    pending: metricValue(latest?.pending),
    discovered: metricValue(latest?.discovered),
    processed: metricValue(latest?.processed),
  };
  const latestSample = samples.at(-1) ?? null;
  const loadFailure = topic.error ?? loadError;
  const status = loadFailure
    ? "加载失败"
    : mixedUnits
      ? "单位分别显示"
      : coverageLabel(trend?.coverage ?? "loading");
  const hasSeries = Object.values(paths).some((series) => series.length > 0);
  const unitLabel = (["pending", "discovered", "processed"] as const)
    .map((metric) => `${metric[0].toUpperCase()}:${units[metric] ?? "未知"}`)
    .join("，");
  const textColor = dark ? "#d8e4f0" : "#344454";
  const chart =
    activeVisible && trend && hasSeries ? (
      <svg
        role="img"
        className="absolute inset-0 h-full w-full"
        viewBox="0 0 320 56"
        preserveAspectRatio="none"
      >
        <title>{`${task.title}最近 24 小时 P/D/C 工作量趋势（${unitLabel}）`}</title>
        <desc>
          {mixedUnits
            ? "P、D、C 使用独立纵向尺度，共用最近 24 小时横轴；原始单位保留在图例和详情中。"
            : "P、D、C 使用同一纵向尺度，共用最近 24 小时横轴。"}
        </desc>
        <path
          d="M 0,48 L 320,48 L 320,56 L 0,56 Z"
          fill={dark ? "#132a3b" : "#e8f1f7"}
          opacity={backgroundMode ? "0.28" : "0.45"}
        />
        {(
          [
            ["pending", "#42a5f5", "0.08", backgroundMode ? 0.9 : 1.5],
            ["discovered", "#a66cff", "0.08", backgroundMode ? 0.9 : 1.5],
            ["processed", "#28c98b", "0.1", backgroundMode ? 1 : 1.7],
          ] as const
        ).flatMap(([metric, color, fillOpacity, strokeWidth]) =>
          paths[metric].flatMap((path) => {
            const pathKey = `${metric}-${path.firstX}-${path.lastX}-${path.points.length}`;
            const point = path.points[0];
            const markerDelta = point.x >= 319.99 ? -0.01 : 0.01;
            return [
              path.points.length > 1 ? (
                <path
                  key={`${pathKey}-area`}
                  d={areaPath(path, 48)}
                  fill={color}
                  opacity={fillOpacity}
                />
              ) : null,
              path.points.length > 1 ? (
                <path
                  key={`${pathKey}-line`}
                  d={path.d}
                  fill="none"
                  stroke={color}
                  strokeWidth={strokeWidth}
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  vectorEffect="non-scaling-stroke"
                />
              ) : (
                <path
                  key={`${pathKey}-point`}
                  data-chart-marker="point"
                  d={`M ${point.x.toFixed(2)},${point.y.toFixed(2)} h ${markerDelta.toFixed(2)}`}
                  fill="none"
                  stroke={color}
                  strokeWidth={backgroundMode ? 3.4 : 4}
                  strokeLinecap="round"
                  vectorEffect="non-scaling-stroke"
                />
              ),
            ];
          }),
        )}
        <line
          x1="0"
          x2="320"
          y1="48"
          y2="48"
          stroke={textColor}
          strokeOpacity="0.28"
          strokeWidth={backgroundMode ? 0.8 : 1}
          vectorEffect="non-scaling-stroke"
        />
      </svg>
    ) : null;
  const statusLabel = activeVisible
    ? loadFailure
      ? status
      : trend?.truncated
        ? "最近 200 次"
        : status
    : "进入视口加载";
  const detailsFields = (
    <>
      <div>触发时间：{formatTime(latestSample?.attemptedAt)}</div>
      <div>实际用时：{formatDuration(latestSample?.durationMs)}</div>
      <div>结果：{resultLabel(latestSample?.status)}</div>
      <div>来源：{latestSample?.triggerKind ?? "未知"}</div>
      <div>待处理量：{formatWorkloadMetric(latestSample?.pending)}</div>
      <div>本次发现：{formatWorkloadMetric(latestSample?.discovered)}</div>
      <div>本次处理：{formatWorkloadMetric(latestSample?.processed)}</div>
      <div className="sm:col-span-2">
        缺失原因：
        {latestSample?.reason ?? (latestSample ? trend?.coverage : (loadFailure ?? status))}
      </div>
    </>
  );
  const detailsPanel =
    detailsOpen && !backgroundMode ? (
      <div
        role="dialog"
        aria-label={`${task.title}运行计量详情`}
        className="relative z-20 mt-1 grid gap-x-3 gap-y-1 border-t border-base-300/50 pt-2 text-[11px] text-base-content/75 sm:grid-cols-2"
      >
        {detailsFields}
      </div>
    ) : null;
  useLayoutEffect(() => {
    if (!backgroundMode || !detailsOpen || typeof window === "undefined") {
      setDetailsPosition(null);
      return;
    }
    const updatePosition = () => {
      const button = detailsButtonRef.current;
      if (!button) return;
      const rect = button.getBoundingClientRect();
      const margin = 12;
      const width = Math.min(448, Math.max(0, window.innerWidth - margin * 2));
      const availableHeight = Math.max(0, window.innerHeight - margin * 2);
      const panelHeightHint = Math.min(240, availableHeight);
      const top = Math.max(
        margin,
        Math.min(rect.bottom + 8, window.innerHeight - margin - panelHeightHint),
      );
      const maxLeft = Math.max(margin, window.innerWidth - margin - width);
      const left = Math.min(Math.max(margin, rect.right - width), maxLeft);
      setDetailsPosition({ top, left, width });
    };
    updatePosition();
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    return () => {
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
    };
  }, [backgroundMode, detailsOpen]);
  useEffect(() => {
    if (!backgroundMode || !detailsOpen || typeof MutationObserver === "undefined") return;
    const button = detailsButtonRef.current;
    const row = button?.closest("[data-task-catalog-row]");
    if (!button || !row) return;
    const closeIfHidden = () => {
      const rowHidden =
        row.hasAttribute("hidden") ||
        row.getAttribute("aria-hidden") === "true" ||
        getComputedStyle(row).display === "none";
      if (rowHidden || button.getClientRects().length === 0) {
        setDetailsOpen(false);
      }
    };
    closeIfHidden();
    const observer = new MutationObserver(closeIfHidden);
    observer.observe(row, { attributes: true, attributeFilter: ["hidden", "class", "style"] });
    return () => observer.disconnect();
  }, [backgroundMode, detailsOpen]);
  const backgroundDetailsPanel =
    backgroundMode && detailsOpen && detailsPosition && typeof document !== "undefined"
      ? createPortal(
          <div
            role="dialog"
            aria-label={`${task.title}运行计量详情`}
            className="pointer-events-auto fixed z-[70] grid max-h-[calc(100vh-1.5rem)] max-w-[calc(100vw-1.5rem)] gap-x-3 gap-y-1 overflow-auto rounded-sm bg-base-100/95 p-2 text-[11px] text-base-content/75 shadow-lg sm:grid-cols-2"
            style={detailsPosition}
          >
            {detailsFields}
          </div>,
          document.body,
        )
      : null;
  const controls = (
    <div className="relative z-10 flex min-h-12 items-start justify-between gap-2 text-[11px]">
      <div className="flex min-w-0 flex-wrap gap-x-2 gap-y-0.5" style={{ color: textColor }}>
        <span>
          <i className="mr-1 inline-block size-1.5 rounded-full bg-sky-400" />P{" "}
          {formatMetric(latestValues.pending)} {units.pending ?? "单位未知"}
        </span>
        <span>
          <i className="mr-1 inline-block size-1.5 rounded-full bg-violet-400" />D{" "}
          {formatMetric(latestValues.discovered)} {units.discovered ?? "单位未知"}
        </span>
        <span>
          <i className="mr-1 inline-block size-1.5 rounded-full bg-emerald-400" />C{" "}
          {formatMetric(latestValues.processed)} {units.processed ?? "单位未知"}
        </span>
        <span className="text-base-content/55">{statusLabel}</span>
        <button
          type="button"
          className="link link-primary whitespace-nowrap text-[11px]"
          aria-expanded={detailsOpen}
          onClick={(event) => {
            event.preventDefault();
            event.stopPropagation();
            setDetailsOpen((open) => !open);
          }}
        >
          查看运行计量
        </button>
      </div>
    </div>
  );

  if (backgroundMode) {
    return (
      <div
        ref={containerRef}
        className="pointer-events-none absolute inset-0 z-20 min-w-0 overflow-visible"
        aria-busy={activeVisible && trend == null}
        data-testid={`task-workload-sparkline-${task.taskKey}`}
      >
        {chart}
        <button
          ref={detailsButtonRef}
          type="button"
          className="pointer-events-auto absolute right-2 top-2 z-20 inline-flex size-6 items-center justify-center rounded-sm bg-base-100/70 text-base-content/65 shadow-sm transition-colors hover:bg-base-100 hover:text-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-primary"
          aria-expanded={detailsOpen}
          aria-label={`${task.title}运行计量详情`}
          title={`${task.title}运行计量详情`}
          onClick={() => setDetailsOpen((open) => !open)}
        >
          <AppIcon name="information-outline" className="size-4" aria-hidden />
        </button>
        {backgroundDetailsPanel}
      </div>
    );
  }

  return (
    <div
      ref={containerRef}
      className="relative min-h-14 min-w-0 overflow-hidden rounded-sm border border-base-300/40 bg-base-200/20 px-2 py-1"
      aria-busy={activeVisible && trend == null}
      data-testid={`task-workload-sparkline-${task.taskKey}`}
    >
      {chart}
      {controls}
      {detailsPanel}
    </div>
  );
}
