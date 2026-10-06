import type { JSX } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useSubscriptionTopic } from "../../hooks/useSubscriptionTopic";
import {
  fetchManagedTaskWorkload,
  type ManagedTask,
  type TaskWorkloadMetric,
  type TaskWorkloadSample,
  type TaskWorkloadTrend,
} from "../../lib/api";

const WINDOW_MS = 24 * 60 * 60 * 1_000;
const MAX_CACHE_ENTRIES = 64;
const MAX_CONCURRENT_LOADS = 4;
const workloadCache = new Map<string, TaskWorkloadTrend>();
const workloadLoads = new Map<string, Promise<TaskWorkloadTrend>>();
const workloadQueue: Array<{
  key: string;
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

function pumpLoads(): void {
  while (activeLoads < MAX_CONCURRENT_LOADS && workloadQueue.length > 0) {
    const next = workloadQueue.shift();
    if (!next) return;
    activeLoads += 1;
    void fetchManagedTaskWorkload(next.key, { windowHours: 24, limit: 200 })
      .then((trend) => {
        workloadCache.set(next.key, trend);
        trimCache();
        next.resolve(trend);
      })
      .catch(next.reject)
      .finally(() => {
        activeLoads -= 1;
        workloadLoads.delete(next.key);
        pumpLoads();
      });
  }
}

function loadWorkload(taskKey: string, refresh = false): Promise<TaskWorkloadTrend> {
  const cached = refresh ? undefined : workloadCache.get(taskKey);
  if (cached) {
    workloadCache.delete(taskKey);
    workloadCache.set(taskKey, cached);
    return Promise.resolve(cached);
  }
  const existing = workloadLoads.get(taskKey);
  if (existing) return existing;
  const promise = new Promise<TaskWorkloadTrend>((resolve, reject) => {
    workloadQueue.push({ key: taskKey, resolve, reject });
    pumpLoads();
  });
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
}

function metricUnit(sample: TaskWorkloadSample, metric: SparklineMetric): string | null {
  const unit = sample[metric]?.unit?.trim();
  return unit ? unit : null;
}

function chartUnit(samples: TaskWorkloadSample[], trend: TaskWorkloadTrend | null): string | null {
  const observedUnits = new Set(
    samples.flatMap((sample) =>
      (["pending", "discovered", "processed"] as const).flatMap((metric) => {
        const unit = metricUnit(sample, metric);
        return unit ? [unit] : [];
      }),
    ),
  );
  if (observedUnits.size > 0) return observedUnits.values().next().value ?? null;
  const capabilityUnits = new Set(
    (["pending", "discovered", "processed"] as const).flatMap((metric) => {
      const unit = trend?.capabilities?.[metric]?.unit?.trim();
      return unit ? [unit] : [];
    }),
  );
  return capabilityUnits.values().next().value ?? null;
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

function chartPath(
  samples: TaskWorkloadSample[],
  metric: SparklineMetric,
  maxValue: number,
  now: number,
  unit: string | null,
): ChartPath | null {
  const points = samples.flatMap((sample) => {
    const time = sampleTime(sample);
    const value = metricValue(sample[metric]);
    if (
      time == null ||
      value == null ||
      (unit != null && metricUnit(sample, metric) !== unit) ||
      time < now - WINDOW_MS ||
      time > now
    )
      return [];
    const x = ((time - (now - WINDOW_MS)) / WINDOW_MS) * 320;
    const y = 48 - (Math.max(0, value) / maxValue) * 40;
    return [{ x, y }];
  });
  if (points.length < 2) return null;
  return {
    d: `M ${points.map(({ x, y }) => `${x.toFixed(2)},${y.toFixed(2)}`).join(" L ")}`,
    firstX: points[0].x,
    lastX: points.at(-1)?.x ?? points[0].x,
  };
}

function areaPath(path: ChartPath, baselineY: number): string {
  return `${path.d} L ${path.lastX.toFixed(2)},${baselineY} L ${path.firstX.toFixed(2)},${baselineY} Z`;
}

function formatMetric(value: number | null | undefined): string {
  return value == null ? "未知" : value.toLocaleString("zh-CN");
}

function formatWorkloadMetric(metric: TaskWorkloadMetric | null | undefined): string {
  if (!metric) return "未知";
  return `${formatMetric(metric.value)} · ${metric.unit || "单位未知"} · ${metric.range || "范围未知"}`;
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
  const [visibilityGeneration, setVisibilityGeneration] = useState(0);
  const [now, setNow] = useState(() => Date.now());
  const [trend, setTrend] = useState<TaskWorkloadTrend | null>(
    () => workloadCache.get(task.taskKey) ?? null,
  );
  const [loadError, setLoadError] = useState<string | null>(null);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [topicSlotAcquired, setTopicSlotAcquired] = useState(false);
  const activeVisible = visible && pageVisible;
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
      if (nextVisible) setVisibilityGeneration((generation) => generation + 1);
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
    if (!activeVisible) return;
    let active = true;
    const cached = workloadCache.get(task.taskKey);
    if (cached) setTrend(cached);
    void loadWorkload(task.taskKey, visibilityGeneration > 0)
      .then((next) => {
        if (active) {
          setTrend(next);
          setLoadError(null);
        }
      })
      .catch((reason: unknown) => {
        if (active) setLoadError(reason instanceof Error ? reason.message : String(reason));
      });
    return () => {
      active = false;
    };
  }, [activeVisible, task.taskKey, visibilityGeneration]);

  useEffect(() => {
    if (topic.data) {
      setTrend(topic.data);
      workloadCache.set(task.taskKey, topic.data);
      trimCache();
    }
  }, [task.taskKey, topic.data]);

  useEffect(() => {
    if (!activeVisible) return;
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, [activeVisible]);

  const samples = trend?.samples ?? [];
  const unit = chartUnit(samples, trend);
  const mixedUnits = hasMixedUnits(samples);
  const values = samples.flatMap((sample) =>
    (["pending", "discovered", "processed"] as const).flatMap((key) => {
      const metric = sample[key];
      const value = metricValue(metric);
      return value == null || (unit != null && metricUnit(sample, key) !== unit) ? [] : [value];
    }),
  );
  const maxValue = Math.max(1, ...values);
  const paths = useMemo(
    () => ({
      pending: chartPath(samples, "pending", maxValue, now, unit),
      discovered: chartPath(samples, "discovered", maxValue, now, unit),
      processed: chartPath(samples, "processed", maxValue, now, unit),
    }),
    [maxValue, now, samples, unit],
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
      ? "单位不一致"
      : coverageLabel(trend?.coverage ?? "loading");
  const hasSeries = Object.values(paths).some(Boolean);
  const textColor = dark ? "#d8e4f0" : "#344454";
  const chart =
    activeVisible && trend && hasSeries ? (
      <svg
        role="img"
        className="absolute inset-0 h-full w-full"
        viewBox="0 0 320 56"
        preserveAspectRatio="none"
      >
        <title>{`${task.title}最近 24 小时 P/D/C 工作量趋势（单位：${unit ?? "未知"}）`}</title>
        <path
          d="M 0,48 L 320,48 L 320,56 L 0,56 Z"
          fill={dark ? "#132a3b" : "#e8f1f7"}
          opacity={backgroundMode ? "0.28" : "0.45"}
        />
        {paths.pending ? (
          <path d={areaPath(paths.pending, 48)} fill="#42a5f5" opacity="0.08" />
        ) : null}
        {paths.discovered ? (
          <path d={areaPath(paths.discovered, 48)} fill="#a66cff" opacity="0.08" />
        ) : null}
        {paths.processed ? (
          <path d={areaPath(paths.processed, 48)} fill="#28c98b" opacity="0.1" />
        ) : null}
        {paths.pending ? (
          <path
            d={paths.pending.d}
            fill="none"
            stroke="#42a5f5"
            strokeWidth={backgroundMode ? 0.9 : 1.5}
            vectorEffect="non-scaling-stroke"
          />
        ) : null}
        {paths.discovered ? (
          <path
            d={paths.discovered.d}
            fill="none"
            stroke="#a66cff"
            strokeWidth={backgroundMode ? 0.9 : 1.5}
            vectorEffect="non-scaling-stroke"
          />
        ) : null}
        {paths.processed ? (
          <path
            d={paths.processed.d}
            fill="none"
            stroke="#28c98b"
            strokeWidth={backgroundMode ? 1 : 1.7}
            vectorEffect="non-scaling-stroke"
          />
        ) : null}
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
  const statusLabel = activeVisible ? (trend?.truncated ? "最近 200 次" : status) : "进入视口加载";
  const controls = (
    <>
      <div
        className={
          backgroundMode
            ? "pointer-events-none absolute inset-x-3 top-2 z-20 flex justify-end"
            : "relative z-10 flex min-h-12 items-start justify-between gap-2 text-[11px]"
        }
      >
        <div
          className={
            backgroundMode
              ? "pointer-events-auto flex max-w-full flex-wrap items-center justify-end gap-x-2 gap-y-0.5 rounded-sm bg-base-100/45 px-1.5 py-0.5 text-[11px]"
              : "flex min-w-0 flex-wrap gap-x-2 gap-y-0.5"
          }
          style={{ color: textColor }}
        >
          <span>
            <i className="mr-1 inline-block size-1.5 rounded-full bg-sky-400" />P{" "}
            {formatMetric(latestValues.pending)}
          </span>
          <span>
            <i className="mr-1 inline-block size-1.5 rounded-full bg-violet-400" />D{" "}
            {formatMetric(latestValues.discovered)}
          </span>
          <span>
            <i className="mr-1 inline-block size-1.5 rounded-full bg-emerald-400" />C{" "}
            {formatMetric(latestValues.processed)}
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
      {detailsOpen ? (
        <div
          role="dialog"
          aria-label={`${task.title}运行计量详情`}
          className={
            backgroundMode
              ? "pointer-events-auto absolute right-3 top-9 z-30 grid w-[min(28rem,calc(100%-1.5rem))] gap-x-3 gap-y-1 rounded-sm bg-base-100/95 p-2 text-[11px] text-base-content/75 shadow-lg sm:grid-cols-2"
              : "relative z-20 mt-1 grid gap-x-3 gap-y-1 border-t border-base-300/50 pt-2 text-[11px] text-base-content/75 sm:grid-cols-2"
          }
        >
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
        </div>
      ) : null}
    </>
  );

  if (backgroundMode) {
    return (
      <>
        <div
          ref={containerRef}
          className="pointer-events-none absolute inset-0 z-0 min-w-0 overflow-hidden"
          aria-busy={activeVisible && trend == null}
          data-testid={`task-workload-sparkline-${task.taskKey}`}
        >
          {chart}
        </div>
        {controls}
      </>
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
    </div>
  );
}
