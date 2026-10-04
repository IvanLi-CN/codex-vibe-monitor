import { type KeyboardEvent, useMemo, useRef, useState } from "react";
import {
  Area,
  CartesianGrid,
  ComposedChart,
  Line,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { useCompactViewport } from "../../hooks/useCompactViewport";
import type {
  RetentionBacklogTrendPoint,
  TaskMeasurementCapabilities,
  TaskWorkloadMetric,
  TaskWorkloadTrend as TaskWorkloadTrendData,
} from "../../lib/api";
import { chartBaseTokens, taskWorkloadTokens, withOpacity } from "../../lib/chartTheme";
import { useTheme } from "../../theme";
import {
  buildWorkloadChartModels,
  selectWorkloadRunWindow,
  WORKLOAD_SERIES,
  type WorkloadChartModel,
  type WorkloadPlotDatum,
  type WorkloadSeriesKey,
  workloadSeriesLabel,
  workloadSubsetPartition,
} from "./taskWorkloadTrendModel";

const COMPACT_FRAME_HEIGHT = 380;
const WIDE_FRAME_HEIGHT = 300;
const RUN_WINDOWS = [20, 50, 100] as const;

type View = "runs" | "retention";

interface TaskWorkloadTrendProps {
  taskKey: string;
  capabilities?: TaskMeasurementCapabilities;
  trend?: TaskWorkloadTrendData | null;
  retentionTrend?: RetentionBacklogTrendPoint[] | null;
  state: "loading" | "ready" | "error";
  error?: string | null;
}

function formatTime(value?: string | null): string {
  if (!value) return "未知";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "未知";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
    timeZone: "Asia/Shanghai",
  }).format(date);
}

function formatBucket(value: string): string {
  const timestamp = Date.parse(value);
  if (Number.isNaN(timestamp)) return "未知";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    hour12: false,
    timeZone: "Asia/Shanghai",
  }).format(new Date(timestamp));
}

function formatCompactTime(value: string): string {
  const timestamp = Date.parse(value);
  if (Number.isNaN(timestamp)) return "";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    timeZone: "Asia/Shanghai",
  }).format(new Date(timestamp));
}

function formatAxisValue(value: number, compact: boolean): string {
  return compact
    ? new Intl.NumberFormat("zh-CN", {
        notation: "compact",
        maximumFractionDigits: 1,
      }).format(value)
    : value.toLocaleString();
}

function metricValue(metric?: TaskWorkloadMetric | null): string {
  if (metric?.value == null) return "未知";
  return `${metric.value.toLocaleString()} ${metric.unit}`;
}

function coverageLabel(coverage?: string): string {
  switch (coverage) {
    case "exact":
      return "完整快照";
    case "window":
      return "本次运行窗口";
    case "lower_bound":
      return "下界";
    case "unknown":
      return "未知";
    default:
      return coverage ?? "未知";
  }
}

function statusLabel(status: string): string {
  switch (status) {
    case "success":
      return "成功";
    case "failed":
      return "失败";
    case "running":
      return "运行中";
    case "skipped":
      return "确认跳过";
    case "interrupted":
      return "中断";
    case "unknown":
      return "未知";
    default:
      return status;
  }
}

function createEmptyPlotData(): WorkloadPlotDatum[] {
  return Array.from({ length: 5 }, (_, index) => ({
    index,
    label: "",
    sample: null,
  }));
}

function createEmptyBacklogData(): RetentionBacklogTrendPoint[] {
  return Array.from({ length: 7 }, (_, index) => ({
    bucketStart: `empty-${index}`,
    state: "missing",
    invocationCount: null,
    maxOverdueSeconds: null,
  }));
}

function workloadHint(
  state: TaskWorkloadTrendProps["state"],
  error: string | null | undefined,
  trend: TaskWorkloadTrendData | null | undefined,
): string | null {
  if (state === "loading") return "加载中";
  if (state === "error") return `读取失败：${error ?? "未知错误"}`;
  if (!trend) return "暂无运行计量";
  if (trend.samples.length === 0) return "暂无运行样本";
  if (
    !trend.samples.some((sample) => WORKLOAD_SERIES.some((series) => sample[series]?.value != null))
  ) {
    return "暂无计数";
  }
  return null;
}

function sampleDetail(sample: NonNullable<WorkloadPlotDatum["sample"]>) {
  const partition = workloadSubsetPartition(sample);
  return (
    <div className="min-w-60 space-y-1.5 text-xs">
      <div className="font-semibold">
        {formatTime(sample.attemptedAt)} · {statusLabel(sample.status)}
      </div>
      {sample.actualStartedAt ? <div>实际开始：{formatTime(sample.actualStartedAt)}</div> : null}
      <div>触发：{sample.triggerKind}</div>
      {WORKLOAD_SERIES.map((series) => {
        const metric = sample[series];
        return (
          <div key={series} className="border-t border-base-300/60 pt-1">
            <span className="font-medium">{workloadSeriesLabel(series)}：</span>
            <span>{metricValue(metric)}</span>
            <span className="ml-1 text-base-content/65">{coverageLabel(metric?.coverage)}</span>
            {metric ? (
              <div className="break-all text-base-content/60">
                范围：{metric.scope} · {metric.range}
              </div>
            ) : null}
          </div>
        );
      })}
      {partition ? (
        <div className="border-t border-base-300/60 pt-1 text-base-content/75">
          未处理候选：{partition.discoveredRemainder.toLocaleString()} · 仍待发现：
          {partition.pendingRemainder.toLocaleString()}
        </div>
      ) : null}
      {sample.reason ? (
        <div className="max-w-72 break-words text-error">{sample.reason}</div>
      ) : null}
    </div>
  );
}

function RunMetricChart({
  model,
  colors,
  hidden,
  axis,
  hint,
  height,
  compact,
}: {
  model: WorkloadChartModel;
  colors: ReturnType<typeof taskWorkloadTokens>;
  hidden: Set<WorkloadSeriesKey>;
  axis: ReturnType<typeof chartBaseTokens>;
  hint: string | null;
  height: number;
  compact: boolean;
}) {
  const chartData = model.data.length > 0 ? model.data : createEmptyPlotData();
  return (
    <figure className="relative w-full" style={{ height }} aria-label={`工作量：${model.unit}`}>
      <ResponsiveContainer width="100%" height="100%">
        <ComposedChart
          data={chartData}
          accessibilityLayer
          margin={{ top: 8, right: compact ? 4 : 12, left: 0, bottom: compact ? 10 : 4 }}
        >
          <CartesianGrid stroke={axis.gridLine} strokeDasharray="3 3" />
          <XAxis
            dataKey="label"
            type="category"
            minTickGap={28}
            tick={{ fill: axis.axisText, fontSize: 10 }}
            tickFormatter={(value) =>
              value ? (compact ? formatCompactTime(String(value)) : formatTime(String(value))) : ""
            }
            tickLine={{ stroke: axis.gridLine }}
            axisLine={{ stroke: axis.gridLine }}
          />
          <YAxis
            width={compact ? 46 : 62}
            tick={{ fill: axis.axisText, fontSize: compact ? 9 : 10 }}
            tickFormatter={(value) => formatAxisValue(Number(value), compact)}
            tickLine={{ stroke: axis.gridLine }}
            axisLine={{ stroke: axis.gridLine }}
            domain={[
              (minimum: number) => Math.min(0, minimum),
              (maximum: number) => Math.max(0, maximum),
            ]}
            allowDataOverflow={false}
          />
          <ReferenceLine y={0} stroke={axis.axisText} strokeOpacity={0.55} />
          <Tooltip
            content={({ active, payload }) => {
              const sample = payload?.[0]?.payload?.sample as
                | WorkloadPlotDatum["sample"]
                | undefined;
              if (!active || !sample) return null;
              return (
                <div className="max-w-sm rounded-md border border-base-300 bg-base-100 p-3 text-base-content shadow-lg">
                  {sampleDetail(sample)}
                </div>
              );
            }}
          />
          {model.segments
            .filter((segment) => !hidden.has(segment.series))
            .sort(
              (left, right) =>
                WORKLOAD_SERIES.indexOf(left.series) - WORKLOAD_SERIES.indexOf(right.series),
            )
            .map((segment) => (
              <Area
                key={segment.key}
                dataKey={segment.key}
                name={workloadSeriesLabel(segment.series)}
                type="linear"
                stroke={colors[segment.series]}
                strokeWidth={1.8}
                fill={withOpacity(colors[segment.series], 0.18)}
                connectNulls={false}
                isAnimationActive={false}
                dot={{ r: 2.5, fill: colors[segment.series], strokeWidth: 0 }}
                activeDot={{ r: 4, strokeWidth: 1 }}
              />
            ))}
          {WORKLOAD_SERIES.filter((series) => !hidden.has(series)).flatMap((series) =>
            Array.from(new Set(chartData.flatMap((datum) => Object.keys(datum))))
              .filter((key) => key.startsWith(`bridge_${series}_`))
              .map((key) => (
                <Line
                  key={key}
                  dataKey={key}
                  type="linear"
                  stroke={colors[series]}
                  strokeWidth={2}
                  strokeDasharray="5 4"
                  dot={false}
                  connectNulls
                  isAnimationActive={false}
                />
              )),
          )}
        </ComposedChart>
      </ResponsiveContainer>
      {hint ? (
        <div className="pointer-events-none absolute inset-x-3 top-1/2 -translate-y-1/2 text-center text-sm text-base-content/65">
          <span className="bg-base-100/85 px-2 py-1">{hint}</span>
        </div>
      ) : null}
      <div className="absolute right-3 top-1 text-[10px] text-base-content/60">{model.unit}</div>
    </figure>
  );
}

function BacklogChart({
  points,
  metric,
  color,
  axis,
  height,
  compact,
}: {
  points: RetentionBacklogTrendPoint[];
  metric: "invocationCount" | "maxOverdueHours";
  color: string;
  axis: ReturnType<typeof chartBaseTokens>;
  height: number;
  compact: boolean;
}) {
  const actualPoints = points.map((point) => ({
    ...point,
    label: point.state === "observed" ? formatBucket(point.bucketStart) : "",
    value:
      point.state !== "observed"
        ? null
        : metric === "invocationCount"
          ? (point.invocationCount ?? null)
          : point.maxOverdueSeconds == null
            ? null
            : point.maxOverdueSeconds / 3600,
  }));
  const chartData =
    actualPoints.length > 0
      ? actualPoints
      : createEmptyBacklogData().map((point) => ({ ...point, label: "", value: null }));
  const hasValue = actualPoints.some((point) => point.value != null);
  return (
    <div className="relative w-full" style={{ height }}>
      <div className="absolute right-3 top-1 z-10 text-[10px] text-base-content/60">
        {metric === "invocationCount" ? "invocation 行" : "小时"}
      </div>
      <ResponsiveContainer width="100%" height="100%">
        <ComposedChart
          data={chartData}
          accessibilityLayer
          margin={{ top: 8, right: compact ? 4 : 12, left: 0, bottom: compact ? 10 : 4 }}
        >
          <CartesianGrid stroke={axis.gridLine} strokeDasharray="3 3" />
          <XAxis
            dataKey="label"
            minTickGap={28}
            tick={{ fill: axis.axisText, fontSize: 10 }}
            tickLine={{ stroke: axis.gridLine }}
            axisLine={{ stroke: axis.gridLine }}
          />
          <YAxis
            width={compact ? 46 : 62}
            tick={{ fill: axis.axisText, fontSize: compact ? 9 : 10 }}
            tickFormatter={(value) =>
              metric === "invocationCount"
                ? formatAxisValue(Number(value), compact)
                : `${Number(value).toFixed(1)}h`
            }
            tickLine={{ stroke: axis.gridLine }}
            axisLine={{ stroke: axis.gridLine }}
            domain={[0, "auto"]}
          />
          <Tooltip
            content={({ active, payload }) => {
              const point = payload?.[0]?.payload as (typeof chartData)[number] | undefined;
              if (!active || !point) return null;
              return (
                <div className="rounded-md border border-base-300 bg-base-100 p-3 text-xs text-base-content shadow-lg">
                  <div className="font-semibold">
                    {point.label || formatBucket(point.bucketStart)}
                  </div>
                  {point.state !== "observed" ? <div>缺测</div> : null}
                  {metric === "invocationCount" ? (
                    <div>
                      待归档：{point.invocationCount?.toLocaleString() ?? "未知"} invocation 行
                    </div>
                  ) : (
                    <div>
                      最长逾期：
                      {point.maxOverdueSeconds == null
                        ? "未知"
                        : `${(point.maxOverdueSeconds / 3600).toFixed(1)} 小时`}
                    </div>
                  )}
                  {point.observedAt ? <div>观测：{formatTime(point.observedAt)}</div> : null}
                  {point.retentionDays != null ? (
                    <div>保留策略：{point.retentionDays} 天</div>
                  ) : null}
                  {point.cutoff ? <div className="break-all">截止：{point.cutoff}</div> : null}
                </div>
              );
            }}
          />
          <Line
            dataKey="value"
            type="linear"
            stroke={color}
            strokeWidth={2}
            dot={{ r: 1.5 }}
            activeDot={{ r: 4 }}
            connectNulls={false}
            isAnimationActive={false}
          />
        </ComposedChart>
      </ResponsiveContainer>
      {!hasValue ? (
        <div className="pointer-events-none absolute inset-x-3 top-1/2 -translate-y-1/2 text-center text-sm text-base-content/65">
          <span className="bg-base-100/85 px-2 py-1">暂无快照</span>
        </div>
      ) : null}
    </div>
  );
}

export function TaskWorkloadTrend({
  taskKey,
  trend,
  capabilities,
  retentionTrend,
  state,
  error,
}: TaskWorkloadTrendProps) {
  const { themeMode } = useTheme();
  const isCompactViewport = useCompactViewport();
  const chartHeight = isCompactViewport ? COMPACT_FRAME_HEIGHT : WIDE_FRAME_HEIGHT;
  const [view, setView] = useState<View>("runs");
  const [runWindow, setRunWindow] = useState<(typeof RUN_WINDOWS)[number]>(
    isCompactViewport ? 20 : 100,
  );
  const [hiddenSeries, setHiddenSeries] = useState<Set<WorkloadSeriesKey>>(() => new Set());
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const visibleSamples = useMemo(
    () =>
      selectWorkloadRunWindow(trend?.samples ?? [], trend?.coverageGaps ?? [], runWindow, taskKey),
    [runWindow, taskKey, trend?.coverageGaps, trend?.samples],
  );
  const models = useMemo(
    () => buildWorkloadChartModels(visibleSamples, capabilities),
    [capabilities, visibleSamples],
  );
  const hasVisibleCoverageGap = visibleSamples.some((sample) =>
    sample.sampleId.startsWith("coverage-gap:"),
  );
  const colors = taskWorkloadTokens(themeMode);
  const axis = chartBaseTokens(themeMode);
  const isRetention = taskKey === "retention_archive";
  const backlogPoints = retentionTrend ?? [];
  const runHint = workloadHint(state, error, trend);
  const runPanelHeight =
    (isCompactViewport ? 72 : 32) +
    models.length * (chartHeight + 12) +
    (hasVisibleCoverageGap ? 40 : 0);
  const retentionRows = isCompactViewport ? 2 : 1;
  const retentionPanelHeight = retentionRows * (chartHeight + 20) + (retentionRows - 1) * 12;
  const panelMinHeight = isRetention
    ? Math.max(runPanelHeight, retentionPanelHeight)
    : runPanelHeight;

  const toggleSeries = (series: WorkloadSeriesKey) => {
    setHiddenSeries((current) => {
      const next = new Set(current);
      if (next.has(series)) next.delete(series);
      else next.add(series);
      return next;
    });
  };

  const handleTabKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const activeIndex = view === "runs" ? 0 : 1;
    const lastIndex = isRetention ? 1 : 0;
    let nextIndex: number | null = null;
    if (event.key === "ArrowRight") nextIndex = (activeIndex + 1) % (lastIndex + 1);
    if (event.key === "ArrowLeft") nextIndex = (activeIndex + lastIndex) % (lastIndex + 1);
    if (event.key === "Home") nextIndex = 0;
    if (event.key === "End") nextIndex = lastIndex;
    if (nextIndex == null) return;
    event.preventDefault();
    const nextView = nextIndex === 0 ? "runs" : "retention";
    setView(nextView);
    tabRefs.current[nextIndex]?.focus();
  };

  const legend = (
    <fieldset className="flex flex-wrap gap-x-4 gap-y-2">
      <legend className="sr-only">运行计量图例</legend>
      {WORKLOAD_SERIES.map((series) => {
        const hasCapability = capabilities?.[series]?.supported;
        const hasSamples = trend?.samples.some((sample) => sample[series] != null) ?? false;
        const hasObservation = trend?.samples.some((sample) => sample[series]?.value != null);
        const availability =
          hasCapability === false ? "不适用" : !hasSamples || !hasObservation ? "暂无观测" : null;
        return (
          <button
            key={series}
            type="button"
            aria-pressed={!hiddenSeries.has(series)}
            aria-label={`${hiddenSeries.has(series) ? "显示" : "隐藏"}${workloadSeriesLabel(series)}`}
            className={`inline-flex min-h-8 items-center gap-2 text-sm ${hiddenSeries.has(series) ? "text-base-content/45" : "text-base-content"}`}
            onClick={() => toggleSeries(series)}
          >
            <span
              className="h-3 w-3 shrink-0 border"
              style={{
                backgroundColor: hasSamples ? colors[series] : "transparent",
                borderColor: colors[series],
              }}
            />
            <span>
              {workloadSeriesLabel(series)}
              {availability ? ` · ${availability}` : ""}
            </span>
          </button>
        );
      })}
    </fieldset>
  );

  const runWindowControl = (
    <fieldset className="flex shrink-0 items-center gap-1">
      <legend className="sr-only">运行样本范围</legend>
      {RUN_WINDOWS.map((count) => (
        <button
          key={count}
          type="button"
          aria-label={`最近 ${count} 次`}
          aria-pressed={runWindow === count}
          className={`min-h-8 px-2 text-xs ${runWindow === count ? "bg-primary/15 font-medium text-primary" : "text-base-content/65"}`}
          onClick={() => setRunWindow(count)}
        >
          {count}次
        </button>
      ))}
    </fieldset>
  );

  return (
    <section className="border-t border-base-300/70 pt-4" aria-labelledby="task-workload-heading">
      <div className="mb-3 flex flex-wrap items-baseline justify-between gap-2">
        <h3 id="task-workload-heading" className="text-base font-semibold">
          运行趋势
        </h3>
      </div>
      <div
        role="tablist"
        aria-label="运行趋势视图"
        className="mb-3 flex gap-1 border-b border-base-300/70"
        onKeyDown={handleTabKeyDown}
      >
        <button
          ref={(element) => {
            tabRefs.current[0] = element;
          }}
          type="button"
          role="tab"
          id="task-workload-runs-tab"
          aria-selected={view === "runs"}
          aria-controls="task-workload-runs-panel"
          tabIndex={view === "runs" ? 0 : -1}
          className={`border-b-2 px-3 py-2 text-sm ${view === "runs" ? "border-primary font-medium text-primary" : "border-transparent text-base-content/65"}`}
          onClick={() => setView("runs")}
        >
          最近 100 次运行
        </button>
        {isRetention ? (
          <button
            ref={(element) => {
              tabRefs.current[1] = element;
            }}
            type="button"
            role="tab"
            id="task-workload-retention-tab"
            aria-selected={view === "retention"}
            aria-controls="task-workload-retention-panel"
            tabIndex={view === "retention" ? 0 : -1}
            className={`border-b-2 px-3 py-2 text-sm ${view === "retention" ? "border-primary font-medium text-primary" : "border-transparent text-base-content/65"}`}
            onClick={() => setView("retention")}
          >
            最近 7 天归档积压
          </button>
        ) : null}
      </div>
      <div data-testid="task-workload-panels" style={{ minHeight: panelMinHeight }}>
        <div
          id="task-workload-runs-panel"
          role="tabpanel"
          aria-labelledby="task-workload-runs-tab"
          hidden={view !== "runs"}
          className="space-y-3"
        >
          <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1">
            {legend}
            {runWindowControl}
          </div>
          {hasVisibleCoverageGap ? (
            <p role="note" className="text-xs text-warning">
              观测缺口
            </p>
          ) : null}
          {view === "runs"
            ? models.map((model) => (
                <RunMetricChart
                  key={model.unit}
                  model={model}
                  colors={colors}
                  hidden={hiddenSeries}
                  axis={axis}
                  hint={runHint}
                  height={chartHeight}
                  compact={isCompactViewport}
                />
              ))
            : null}
        </div>
        {isRetention ? (
          <div
            id="task-workload-retention-panel"
            role="tabpanel"
            aria-labelledby="task-workload-retention-tab"
            hidden={view !== "retention"}
            className="space-y-3"
          >
            {view === "retention" ? (
              <div className="grid min-w-0 gap-3 md:grid-cols-2">
                <div>
                  <div className="mb-1 text-sm font-medium">待归档数量</div>
                  <BacklogChart
                    points={backlogPoints}
                    metric="invocationCount"
                    color={colors.pending}
                    axis={axis}
                    height={chartHeight}
                    compact={isCompactViewport}
                  />
                </div>
                <div>
                  <div className="mb-1 text-sm font-medium">最长逾期</div>
                  <BacklogChart
                    points={backlogPoints}
                    metric="maxOverdueHours"
                    color={colors.discovered}
                    axis={axis}
                    height={chartHeight}
                    compact={isCompactViewport}
                  />
                </div>
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
    </section>
  );
}

export function TaskWorkloadSummary({ trend }: { trend?: TaskWorkloadTrendData | null }) {
  if (!trend) return null;
  const items: Array<{ label: string; value: string; detail?: string }> = [];
  if (trend.latestPending?.value != null && trend.latestPending.coverage === "exact") {
    items.push({
      label: "最近准确待处理量",
      value: metricValue(trend.latestPending),
      detail: `观测于 ${formatTime(trend.latestPending.observedAt)}`,
    });
  }
  if (trend.latestProcessed?.value != null && trend.latestProcessed.coverage === "window") {
    items.push({
      label: "最近一次处理",
      value: metricValue(trend.latestProcessed),
      detail: coverageLabel(trend.latestProcessed.coverage),
    });
  }
  if (trend.processingRatePerSecond != null && trend.processingRatePerSecond > 0) {
    items.push({
      label: "近期处理速率",
      value: `${trend.processingRatePerSecond.toLocaleString("zh-CN", { maximumFractionDigits: 2 })} ${trend.latestProcessed?.unit ?? "单位"}/秒`,
    });
  }
  if (trend.clearanceEstimateReason === "cleared" && trend.latestPending?.value === 0) {
    items.push({
      label: "积压状态",
      value: "已清空",
      detail: `准确观测于 ${formatTime(trend.latestObservedAt)}`,
    });
  } else if (trend.clearanceEta) {
    const etaTime = Date.parse(trend.clearanceEta);
    items.push({
      label: "清零预估",
      value:
        Number.isFinite(etaTime) && etaTime < Date.now()
          ? "等待新观测"
          : formatTime(trend.clearanceEta),
      detail: [trend.clearanceEstimateWindow, trend.clearanceEstimateCoverage]
        .filter(Boolean)
        .join(" · "),
    });
  } else if (trend.latestPending?.value != null) {
    const reason = {
      capture_gap: "观测存在缺口，清零预估暂不可用",
      no_net_backlog_decline: "积压未下降，暂无法估算",
      task_disabled: "任务已停用，暂停清零预估",
      stale_observation: "观测已过期，等待新数据",
      insufficient_samples: "积压快照不足，暂无法估算",
      insufficient_span: "观测跨度不足，暂无法估算",
      estimate_out_of_range: "趋势超出可估算范围",
    }[trend.clearanceEstimateReason];
    if (reason) {
      items.push({ label: "清零预估", value: reason });
    }
  }
  if (items.length === 0) return null;
  return (
    <dl className="grid grid-cols-2 gap-3 xl:grid-cols-4">
      {items.map((item) => (
        <div key={item.label} className="min-w-0 border-l-2 border-primary/55 pl-3">
          <dt className="text-xs text-base-content/60">{item.label}</dt>
          <dd className="mt-1 break-words text-lg font-semibold">{item.value}</dd>
          {item.detail ? (
            <dd className="mt-1 text-xs text-base-content/55">{item.detail}</dd>
          ) : null}
        </div>
      ))}
    </dl>
  );
}
