import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { Alert } from "../../components/ui/alert";
import { Button } from "../../components/ui/button";
import { SelectField } from "../../components/ui/select-field";
import { useLongTermStats } from "../../hooks/useLongTermStats";
import { useTranslation } from "../../i18n";
import type {
  LongTermMetrics,
  LongTermSeries,
  LongTermSeriesSummary,
  LongTermStatsDimension,
  LongTermStatsOverviewResponse,
  LongTermStatsRange,
} from "../../lib/api";
import { chartBaseTokens } from "../../lib/chartTheme";
import { useTheme } from "../../theme";
import { ModelPerformanceModelIdentity } from "../dashboard/ModelPerformanceModelIdentity";
import { ModelIdentity, resolveModelIdentityIcon } from "../shared/ModelIdentity";
import { type LongTermSeriesVisual, resolveLongTermSeriesVisuals } from "./longTermSeriesVisuals";

type MetricKey =
  | "tokens"
  | "cost"
  | "calls"
  | "usageTimeMs"
  | "wallTimeMs"
  | "outputSpeedTokensPerSecond"
  | "firstByteMs"
  | "responseMs";

const RANGE_OPTIONS: Array<{ value: LongTermStatsRange; label: string }> = [
  { value: "7d", label: "7d" },
  { value: "30d", label: "30d" },
  { value: "180d", label: "180d" },
  { value: "365d", label: "365d" },
];

const MODEL_TIME_METRICS: MetricKey[] = ["usageTimeMs", "wallTimeMs"];
const MODEL_PERFORMANCE_METRICS: MetricKey[] = [
  "outputSpeedTokensPerSecond",
  "firstByteMs",
  "responseMs",
];
const USAGE_METRICS: MetricKey[] = ["tokens", "cost", "calls"];

export const globalTrendMetricLabelKeys = {
  tokens: "stats.longTerm.global.tokens",
  cost: "stats.longTerm.global.cost",
  calls: "stats.longTerm.global.calls",
} as const;

export function globalTrendMetricLabelKey(metric: MetricKey): string {
  return (
    globalTrendMetricLabelKeys[metric as keyof typeof globalTrendMetricLabelKeys] ??
    globalTrendMetricLabelKeys.tokens
  );
}

const metricLabelKeys: Record<MetricKey, string> = {
  tokens: "stats.longTerm.metric.tokens",
  cost: "stats.longTerm.metric.cost",
  calls: "stats.longTerm.metric.calls",
  usageTimeMs: "stats.longTerm.metric.usageTime",
  wallTimeMs: "stats.longTerm.metric.wallTime",
  outputSpeedTokensPerSecond: "stats.longTerm.metric.outputSpeed",
  firstByteMs: "stats.longTerm.metric.firstByte",
  responseMs: "stats.longTerm.metric.response",
};

const metricColors: Record<MetricKey, string> = {
  tokens: "#0f766e",
  cost: "#c2410c",
  calls: "#1d4ed8",
  usageTimeMs: "#7c3aed",
  wallTimeMs: "#0891b2",
  outputSpeedTokensPerSecond: "#2563eb",
  firstByteMs: "#d97706",
  responseMs: "#db2777",
};

function metricValue(metrics: LongTermMetrics, metric: MetricKey): number | null {
  return metrics[metric];
}

function formatMetric(value: number | null, metric: MetricKey): string {
  if (value == null || !Number.isFinite(value)) return "—";
  if (metric === "cost") return `$${value.toFixed(2)}`;
  if (metric === "calls") return new Intl.NumberFormat().format(value);
  if (metric === "tokens") return new Intl.NumberFormat().format(value);
  if (metric === "outputSpeedTokensPerSecond") return `${value.toFixed(1)} tok/s`;
  return `${Math.round(value)} ms`;
}

function metricSortValue(item: LongTermSeriesSummary, metric: MetricKey): number {
  return metricValue(item, metric) ?? Number.NEGATIVE_INFINITY;
}

type ChartDatum = Record<string, string | number | null> & { date: string };

function completeDailyDateRange(dates: readonly string[]): string[] {
  const bounds = [...new Set(dates)].sort((left, right) => left.localeCompare(right));
  if (bounds.length < 2) return bounds;

  const start = new Date(`${bounds[0]}T00:00:00.000Z`);
  const end = new Date(`${bounds.at(-1)}T00:00:00.000Z`);
  const result: string[] = [];
  for (const current = start; current <= end; current.setUTCDate(current.getUTCDate() + 1)) {
    result.push(current.toISOString().slice(0, 10));
  }
  return result;
}

export function mergeSeriesPoints(
  series: LongTermSeries[],
  metric: MetricKey,
  stackedArea = false,
  canonicalDates: readonly string[] = [],
): ChartDatum[] {
  const pointsBySeries = new Map(
    series.map((item) => [
      item.seriesKey,
      new Map(item.points.map((point) => [point.date, point])),
    ]),
  );
  const pointDates = series.flatMap((item) => item.points.map((point) => point.date));
  const dates = stackedArea
    ? completeDailyDateRange(canonicalDates.length > 0 ? canonicalDates : pointDates)
    : [...new Set(pointDates)].sort((left, right) => left.localeCompare(right));

  return dates.map((date) => {
    const datum: ChartDatum = { date };
    for (const item of series) {
      const point = pointsBySeries.get(item.seriesKey)?.get(date);
      const value = point ? metricValue(point, metric) : null;
      datum[item.seriesKey] = stackedArea ? (value ?? 0) : value;
    }
    return datum;
  });
}

function orderSeriesBySummary(
  series: LongTermSeries[],
  entries: LongTermSeriesSummary[],
): LongTermSeries[] {
  const order = new Map(entries.map((entry, index) => [entry.seriesKey, index]));
  return [...series].sort(
    (left, right) =>
      (order.get(left.seriesKey) ?? Number.MAX_SAFE_INTEGER) -
      (order.get(right.seriesKey) ?? Number.MAX_SAFE_INTEGER),
  );
}

function StatusMessage({
  overview,
  isLoading,
  error,
}: {
  overview: LongTermStatsOverviewResponse | null;
  isLoading: boolean;
  error: string | null;
}) {
  const { t } = useTranslation();
  if (error) return <Alert variant="error">{error}</Alert>;
  if (isLoading)
    return (
      <div className="rounded-lg border border-base-300/70 p-6 text-sm opacity-70">
        {t("stats.longTerm.loading")}
      </div>
    );
  if (overview?.status === "preparing")
    return (
      <Alert variant="info">
        <div>{t("stats.longTerm.preparing")}</div>
        {overview.totalRows > 0 ? (
          <div className="mt-1 text-xs opacity-75">
            {t("stats.longTerm.preparingProgress", {
              processed: overview.processedRows,
              total: overview.totalRows,
            })}
          </div>
        ) : null}
      </Alert>
    );
  if (overview?.status === "error")
    return <Alert variant="error">{t("stats.longTerm.error")}</Alert>;
  if (overview?.status === "empty")
    return (
      <div className="rounded-lg border border-dashed border-base-300 p-6 text-sm opacity-70">
        {t("stats.longTerm.empty")}
      </div>
    );
  return null;
}

function MetricToggle({
  value,
  options,
  onChange,
}: {
  value: MetricKey;
  options: MetricKey[];
  onChange: (value: MetricKey) => void;
}) {
  const { t } = useTranslation();
  return (
    <fieldset className="flex flex-wrap gap-1 rounded-lg border border-base-300/70 bg-base-200/40 p-1">
      {options.map((metric) => (
        <Button
          key={metric}
          size="sm"
          variant={metric === value ? "default" : "ghost"}
          onClick={() => onChange(metric)}
          aria-pressed={metric === value}
        >
          {t(metricLabelKeys[metric])}
        </Button>
      ))}
    </fieldset>
  );
}

function LongTermChart({
  series,
  metric,
  emptyLabel,
  modelSeries = false,
  stackedArea = false,
  canonicalDates,
  visuals,
}: {
  series: LongTermSeries[];
  metric: MetricKey;
  emptyLabel: string;
  modelSeries?: boolean;
  stackedArea?: boolean;
  canonicalDates?: readonly string[];
  visuals?: ReadonlyMap<string, LongTermSeriesVisual>;
}) {
  const { t } = useTranslation();
  const { themeMode } = useTheme();
  const colors = chartBaseTokens(themeMode);
  const chartData = mergeSeriesPoints(series, metric, stackedArea, canonicalDates);
  const fallbackVisuals = useMemo(
    () =>
      new Map(
        series.map((item) => [
          item.seriesKey,
          {
            color: metricColors[metric],
            fill: metricColors[metric],
            fillOpacity: 0.42,
            label: item.displayName,
            stroke: metricColors[metric],
            strokeDasharray: undefined,
          },
        ]),
      ),
    [metric, series],
  );
  const resolvedVisuals = visuals ?? fallbackVisuals;
  if (series.length === 0 || chartData.length === 0) {
    return (
      <div className="flex h-64 items-center justify-center rounded-lg border border-dashed border-base-300 text-sm opacity-70">
        {emptyLabel}
      </div>
    );
  }
  return (
    <div
      className="w-full min-w-0"
      data-chart-kind={stackedArea ? "long-term-stacked-area" : "long-term-series"}
      data-chart-mode={stackedArea ? "stacked-area" : "line"}
    >
      <div className="h-64">
        <ResponsiveContainer>
          {stackedArea ? (
            <AreaChart data={chartData} margin={{ top: 8, right: 12, left: 4, bottom: 4 }}>
              <CartesianGrid stroke={colors.gridLine} strokeDasharray="3 3" />
              <XAxis
                dataKey="date"
                tick={{ fill: colors.axisText, fontSize: 11 }}
                minTickGap={24}
              />
              <YAxis
                tick={{ fill: colors.axisText, fontSize: 11 }}
                tickFormatter={(value) => formatMetric(Number(value), metric)}
                width={76}
              />
              <Tooltip
                content={
                  <LongTermSeriesTooltip
                    metric={metric}
                    series={series}
                    colors={colors}
                    visuals={resolvedVisuals}
                    totalLabel={t("stats.longTerm.total")}
                  />
                }
              />
              {series.map((item) => {
                const visual = resolvedVisuals.get(item.seriesKey);
                if (!visual) return null;
                return (
                  <Area
                    key={item.seriesKey}
                    type="monotone"
                    dataKey={item.seriesKey}
                    name={visual.label}
                    stackId="long-term-usage"
                    stroke={visual.stroke}
                    strokeDasharray={visual.strokeDasharray}
                    fill={visual.fill}
                    fillOpacity={visual.fillOpacity}
                    strokeWidth={2}
                    connectNulls={false}
                  />
                );
              })}
            </AreaChart>
          ) : (
            <LineChart data={chartData} margin={{ top: 8, right: 12, left: 4, bottom: 4 }}>
              <CartesianGrid stroke={colors.gridLine} strokeDasharray="3 3" />
              <XAxis
                dataKey="date"
                tick={{ fill: colors.axisText, fontSize: 11 }}
                minTickGap={24}
              />
              <YAxis
                tick={{ fill: colors.axisText, fontSize: 11 }}
                tickFormatter={(value) => formatMetric(Number(value), metric)}
                width={76}
              />
              <Tooltip
                content={
                  <LongTermSeriesTooltip
                    metric={metric}
                    series={series}
                    colors={colors}
                    visuals={resolvedVisuals}
                  />
                }
              />
              {series.map((item) => {
                const visual = resolvedVisuals.get(item.seriesKey);
                if (!visual) return null;
                return (
                  <Line
                    key={item.seriesKey}
                    type="monotone"
                    dataKey={item.seriesKey}
                    name={visual.label}
                    stroke={visual.stroke}
                    strokeDasharray={visual.strokeDasharray}
                    strokeWidth={2}
                    dot={false}
                    connectNulls
                  />
                );
              })}
            </LineChart>
          )}
        </ResponsiveContainer>
      </div>
      <LongTermChartLegend series={series} visuals={resolvedVisuals} modelSeries={modelSeries} />
    </div>
  );
}

function LongTermSeriesTooltip({
  active,
  payload,
  label,
  metric,
  series,
  colors,
  visuals,
  totalLabel,
}: {
  active?: boolean;
  label?: string | number;
  payload?: Array<{ dataKey?: string | number; color?: string; payload?: ChartDatum }>;
  metric: MetricKey;
  series: LongTermSeries[];
  colors: ReturnType<typeof chartBaseTokens>;
  visuals: ReadonlyMap<string, LongTermSeriesVisual>;
  totalLabel?: string;
}) {
  if (!active || !payload?.length) return null;
  const datum = payload.find((entry) => entry.payload)?.payload;
  if (!datum) return null;
  const total = series.reduce((sum, item) => {
    const value = datum[item.seriesKey];
    return typeof value === "number" && Number.isFinite(value) ? sum + value : sum;
  }, 0);
  return (
    <div
      className="rounded-lg border px-3 py-2 text-xs shadow-lg"
      style={{
        backgroundColor: colors.tooltipBg,
        borderColor: colors.tooltipBorder,
        color: colors.axisText,
      }}
    >
      <div className="mb-1 font-semibold">{String(label ?? datum.date)}</div>
      <div className="space-y-0.5">
        {series.map((item) => {
          const value = datum[item.seriesKey];
          const visual = visuals.get(item.seriesKey);
          if (!visual) return null;
          return (
            <div key={item.seriesKey} className="flex items-center justify-between gap-4">
              <span className="inline-flex min-w-0 items-center gap-1.5">
                <SeriesSwatch visual={visual} />
                <span className="max-w-[14rem] break-words" title={visual.label}>
                  {visual.label}
                </span>
              </span>
              <span className="tabular-nums">
                {formatMetric(typeof value === "number" ? value : null, metric)}
              </span>
            </div>
          );
        })}
        {totalLabel ? (
          <div className="mt-1 flex items-center justify-between gap-4 border-t border-current/15 pt-1 font-semibold">
            <span>{totalLabel}</span>
            <span className="tabular-nums">{formatMetric(total, metric)}</span>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function SeriesSwatch({ visual, seriesKey }: { visual: LongTermSeriesVisual; seriesKey?: string }) {
  return (
    <svg className="h-3 w-7 flex-none" viewBox="0 0 28 12" aria-hidden data-series-key={seriesKey}>
      <title>{visual.label}</title>
      <line
        x1="1"
        x2="27"
        y1="6"
        y2="6"
        stroke={visual.stroke}
        strokeWidth="3"
        strokeDasharray={visual.strokeDasharray}
        strokeLinecap="round"
      />
    </svg>
  );
}

function LongTermChartLegend({
  series,
  visuals,
  modelSeries,
}: {
  series: LongTermSeries[];
  visuals: ReadonlyMap<string, LongTermSeriesVisual>;
  modelSeries: boolean;
}) {
  const { t } = useTranslation();
  if (series.length === 0) return null;

  return (
    <div
      className="grid grid-cols-1 gap-x-4 gap-y-1.5 px-1 pt-3 text-xs text-base-content/80 sm:grid-cols-2 xl:grid-cols-4"
      data-testid="long-term-series-legend"
    >
      {series.map((item) => {
        const visual = visuals.get(item.seriesKey);
        if (!visual) return null;
        const showIcon = modelSeries && resolveModelIdentityIcon(item.displayName) !== null;
        const visibleLabel = showIcon
          ? item.reasoningEffort?.trim() || t("stats.longTerm.unspecified")
          : visual.label;
        return (
          <span
            key={item.seriesKey}
            className="inline-flex min-w-0 items-center gap-1.5 leading-4"
            data-series-key={item.seriesKey}
            data-long-term-legend-display={showIcon ? "icon-and-effort" : "full-label"}
            title={visual.label}
          >
            <span className="sr-only">{visual.label}</span>
            <span className="contents" aria-hidden="true">
              <SeriesSwatch visual={visual} seriesKey={item.seriesKey} />
              {showIcon ? <ModelIdentity model={item.displayName} className="h-4 w-4" /> : null}
              <span
                className="min-w-0 break-words"
                data-long-term-legend-label={showIcon ? "effort" : "full"}
              >
                {visibleLabel}
              </span>
            </span>
          </span>
        );
      })}
    </div>
  );
}

function SeriesTable({
  title,
  entries,
  totalMetrics,
  selectedKeys,
  onToggle,
  sortMetric,
  onSort,
  search,
  onSearch,
  modelEntries = false,
  visuals,
}: {
  title: string;
  entries: LongTermSeriesSummary[];
  totalMetrics?: LongTermMetrics;
  selectedKeys: string[];
  onToggle: (key: string) => void;
  sortMetric: MetricKey;
  onSort: (metric: MetricKey) => void;
  search: string;
  onSearch: (value: string) => void;
  modelEntries?: boolean;
  visuals: ReadonlyMap<string, LongTermSeriesVisual>;
}) {
  const { t } = useTranslation();
  const parentRef = useRef<HTMLDivElement>(null);
  const filtered = useMemo(() => {
    const needle = search.trim().toLowerCase();
    return entries
      .filter(
        (entry) =>
          !needle ||
          `${entry.displayName} ${entry.reasoningEffort ?? ""}`.toLowerCase().includes(needle),
      )
      .sort(
        (left, right) => metricSortValue(right, sortMetric) - metricSortValue(left, sortMetric),
      );
  }, [entries, search, sortMetric]);
  const columns: MetricKey[] = [
    "tokens",
    "cost",
    "calls",
    "usageTimeMs",
    "wallTimeMs",
    "outputSpeedTokensPerSecond",
    "firstByteMs",
    "responseMs",
  ];
  const rowHeight = modelEntries ? 40 : 48;
  const rowVirtualizer = useVirtualizer({
    count: filtered.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => rowHeight,
    overscan: 8,
  });
  const columnVirtualizer = useVirtualizer({
    count: columns.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 120,
    horizontal: true,
    overscan: 2,
  });
  const stickyColumnsWidth = 18 * 16;
  const gridWidth = stickyColumnsWidth + columnVirtualizer.getTotalSize();
  const virtualColumns = columnVirtualizer.getVirtualItems();
  const renderMetricCells = (metrics: LongTermMetrics, button = true) => (
    <div
      className="absolute left-[18rem] top-0 h-full"
      style={{ width: columnVirtualizer.getTotalSize() }}
    >
      {virtualColumns.map((virtualColumn) => {
        const metric = columns[virtualColumn.index];
        const content = formatMetric(metricValue(metrics, metric), metric);
        return button ? (
          <button
            type="button"
            key={metric}
            className="absolute top-0 h-full truncate px-1 text-left tabular-nums hover:text-primary"
            style={{ left: virtualColumn.start, width: virtualColumn.size }}
            onClick={() => onSort(metric)}
          >
            {content}
          </button>
        ) : (
          <span
            key={metric}
            className="absolute top-0 inline-flex h-full items-center truncate px-1 text-left tabular-nums font-semibold"
            style={{ left: virtualColumn.start, width: virtualColumn.size }}
          >
            {content}
          </span>
        );
      })}
    </div>
  );
  return (
    <section className="space-y-3" data-testid={`long-term-table-${title}`}>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <h4 className="text-sm font-semibold">{title}</h4>
        <input
          className="h-9 min-w-[12rem] rounded-md border border-base-300 bg-base-100 px-3 text-sm"
          value={search}
          onChange={(event) => onSearch(event.target.value)}
          placeholder={t("stats.longTerm.search")}
          aria-label={t("stats.longTerm.search")}
        />
      </div>
      <div
        ref={parentRef}
        className="max-h-[22rem] overflow-auto rounded-lg border border-base-300/70"
      >
        <div style={{ width: gridWidth }}>
          <div
            className="sticky top-0 z-10 flex h-10 border-b border-base-300 bg-base-200/95 text-xs font-semibold backdrop-blur"
            style={{ width: gridWidth }}
          >
            <span className="sticky left-0 z-20 flex h-full w-12 shrink-0 items-center bg-base-200/95 px-3" />
            <span
              className="sticky left-12 z-20 flex h-full w-60 shrink-0 items-center bg-base-200/95 px-3"
              data-testid={`long-term-table-${title}-identity-header`}
            >
              {modelEntries ? t("stats.longTerm.modelAndReasoning") : t("stats.longTerm.name")}
            </span>
            <div
              className="absolute left-[18rem] top-0 h-full"
              style={{ width: columnVirtualizer.getTotalSize() }}
            >
              {virtualColumns.map((virtualColumn) => {
                const metric = columns[virtualColumn.index];
                return (
                  <button
                    type="button"
                    key={metric}
                    className="absolute top-0 h-full px-1 text-left hover:text-primary"
                    style={{ left: virtualColumn.start, width: virtualColumn.size }}
                    onClick={() => onSort(metric)}
                  >
                    {t(metricLabelKeys[metric])}
                    {sortMetric === metric ? " ↓" : ""}
                  </button>
                );
              })}
            </div>
          </div>
          {modelEntries && totalMetrics ? (
            <div
              className="sticky top-10 z-[9] flex h-10 border-b border-base-300 bg-base-100 text-sm font-semibold"
              style={{ width: gridWidth }}
              data-testid="long-term-model-total-row"
            >
              <span className="sticky left-0 z-10 flex h-full w-12 shrink-0 items-center bg-base-100 px-3" />
              <span className="sticky left-12 z-10 flex h-full w-60 shrink-0 items-center bg-base-100 px-3">
                {t("stats.longTerm.total")}
              </span>
              {renderMetricCells(totalMetrics, false)}
            </div>
          ) : null}
          <div
            className="relative"
            style={{ height: rowVirtualizer.getTotalSize(), width: gridWidth }}
          >
            {rowVirtualizer.getVirtualItems().map((virtualRow) => {
              const entry = filtered[virtualRow.index];
              const selected = selectedKeys.includes(entry.seriesKey);
              const visual = visuals.get(entry.seriesKey);
              return (
                <div
                  key={entry.seriesKey}
                  className="absolute left-0 flex border-b border-base-300/50 text-sm"
                  style={{
                    transform: `translateY(${virtualRow.start}px)`,
                    height: virtualRow.size,
                    width: gridWidth,
                  }}
                >
                  <span className="sticky left-0 z-10 flex h-full w-12 shrink-0 items-center bg-base-100 px-3">
                    <input
                      type="checkbox"
                      checked={selected}
                      onChange={() => onToggle(entry.seriesKey)}
                      aria-label={`${t("stats.longTerm.select")} ${entry.displayName}`}
                      disabled={!selected && selectedKeys.length >= 8}
                    />
                  </span>
                  <span className="sticky left-12 z-10 flex h-full w-60 min-w-0 shrink-0 items-center bg-base-100 px-3 pr-3">
                    {selected && visual ? (
                      <SeriesSwatch visual={visual} seriesKey={entry.seriesKey} />
                    ) : null}
                    {modelEntries ? (
                      <ModelPerformanceModelIdentity
                        model={entry.displayName}
                        effort={entry.reasoningEffort ?? t("stats.longTerm.unspecified")}
                        effortValue={entry.reasoningEffort}
                        className="max-w-full"
                        testId={`long-term-model-identity-${entry.seriesKey}`}
                      />
                    ) : (
                      <span className="block truncate font-medium" title={entry.displayName}>
                        {entry.displayName}
                      </span>
                    )}
                  </span>
                  {renderMetricCells(entry)}
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </section>
  );
}

export interface LongTermStatsSectionProps {
  initialRange?: LongTermStatsRange;
  overviewOverride?: LongTermStatsOverviewResponse;
  seriesOverride?: LongTermSeries[];
  upstreamSeriesOverride?: LongTermSeries[];
}

export function LongTermStatsSection({
  initialRange = "7d",
  overviewOverride,
  seriesOverride,
  upstreamSeriesOverride,
}: LongTermStatsSectionProps) {
  const { t } = useTranslation();
  const { themeMode } = useTheme();
  const [range, setRange] = useState<LongTermStatsRange>(initialRange);
  const [modelMetric, setModelMetric] = useState<MetricKey>("usageTimeMs");
  const [performanceMetric, setPerformanceMetric] = useState<MetricKey>(
    "outputSpeedTokensPerSecond",
  );
  const [globalMetric, setGlobalMetric] = useState<MetricKey>("tokens");
  const [usageMetric, setUsageMetric] = useState<MetricKey>("tokens");
  const [upstreamMetric, setUpstreamMetric] = useState<MetricKey>("tokens");
  const [modelSelection, setModelSelection] = useState<string[]>([]);
  const [upstreamSelection, setUpstreamSelection] = useState<string[]>([]);
  const selectionRangeRef = useRef<LongTermStatsRange | null>(null);
  const [modelSearch, setModelSearch] = useState("");
  const [upstreamSearch, setUpstreamSearch] = useState("");
  const [modelSort, setModelSort] = useState<MetricKey>("tokens");
  const [upstreamSort, setUpstreamSort] = useState<MetricKey>("tokens");
  const dimension: LongTermStatsDimension = "model";
  const {
    overview: fetchedOverview,
    series: fetchedSeries,
    isLoading,
    error,
    seriesError,
  } = useLongTermStats(range, dimension, modelSelection, !overviewOverride);
  const overview = overviewOverride ?? fetchedOverview;
  const modelSeries = useMemo(
    () =>
      orderSeriesBySummary(seriesOverride ?? fetchedSeries?.series ?? [], overview?.models ?? []),
    [fetchedSeries?.series, overview?.models, seriesOverride],
  );
  const upstreamKeys = useMemo(() => upstreamSelection.slice(0, 8), [upstreamSelection]);
  const {
    series: fetchedUpstreamSeries,
    isSeriesLoading: isUpstreamSeriesLoading,
    seriesError: upstreamSeriesError,
  } = useLongTermStats(range, "upstream", upstreamKeys, !overviewOverride, overview);
  const upstreamSeries = useMemo(
    () =>
      orderSeriesBySummary(
        upstreamSeriesOverride ?? (overviewOverride ? [] : (fetchedUpstreamSeries?.series ?? [])),
        overview?.upstreams ?? [],
      ),
    [fetchedUpstreamSeries?.series, overview?.upstreams, overviewOverride, upstreamSeriesOverride],
  );
  const unspecifiedLabel = t("stats.longTerm.unspecified");
  const modelVisuals = useMemo(
    () => resolveLongTermSeriesVisuals(modelSeries, "model", themeMode, unspecifiedLabel),
    [modelSeries, themeMode, unspecifiedLabel],
  );
  const upstreamVisuals = useMemo(
    () => resolveLongTermSeriesVisuals(upstreamSeries, "upstream", themeMode, unspecifiedLabel),
    [themeMode, unspecifiedLabel, upstreamSeries],
  );

  useEffect(() => {
    if (!overview || overview.range !== range || overview.status !== "ready") return;
    const isInitial = selectionRangeRef.current === null;
    const rangeChanged = selectionRangeRef.current !== range;
    selectionRangeRef.current = range;
    const reconcile = (current: string[], entries: LongTermSeriesSummary[]) => {
      const available = new Set(entries.map((entry) => entry.seriesKey));
      const kept = current.filter((key) => available.has(key));
      const next = [
        ...kept,
        ...entries.map((entry) => entry.seriesKey).filter((key) => !kept.includes(key)),
      ];
      if (!isInitial && !rangeChanged) {
        return kept.slice(0, 8);
      }
      return next.slice(0, Math.min(8, Math.max(3, kept.length)));
    };
    setModelSelection((current) => reconcile(current, overview.models));
    setUpstreamSelection((current) => reconcile(current, overview.upstreams));
  }, [overview, range]);

  const globalChartSeries = useMemo(
    () => [
      {
        seriesKey: "global",
        displayName: t(globalTrendMetricLabelKey(globalMetric)),
        points: overview?.daily ?? [],
      },
    ],
    [globalMetric, overview?.daily, t],
  );
  const stackedAreaDates = useMemo(
    () => overview?.daily.map((point) => point.date) ?? [],
    [overview?.daily],
  );
  const status = (
    <StatusMessage overview={overview} isLoading={isLoading && !overviewOverride} error={error} />
  );
  const modelTable = overview?.models ?? [];
  const upstreamTable = overview?.upstreams ?? [];
  return (
    <section className="surface-panel" data-testid="long-term-stats-section">
      <div className="surface-panel-body gap-6">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="section-heading">
            <h2 className="section-title">{t("stats.longTerm.title")}</h2>
            <p className="section-description">{t("stats.longTerm.subtitle")}</p>
          </div>
          <SelectField
            options={RANGE_OPTIONS.map((option) => ({
              ...option,
              label: t(`stats.longTerm.range.${option.value}`),
            }))}
            value={range}
            onValueChange={(value) => setRange(value as LongTermStatsRange)}
            triggerClassName="min-w-[7rem]"
            data-testid="long-term-range-select"
            aria-label={t("stats.longTerm.rangeLabel")}
          />
        </div>
        {status}
        {overview && overview.status === "ready" ? (
          <>
            <div className="metric-grid grid-cols-1 sm:grid-cols-3">
              <div className="metric-cell">
                <div className="metric-label">{t("stats.longTerm.tokens")}</div>
                <div className="metric-value">{formatMetric(overview.global.tokens, "tokens")}</div>
              </div>
              <div className="metric-cell">
                <div className="metric-label">{t("stats.longTerm.cost")}</div>
                <div className="metric-value">{formatMetric(overview.global.cost, "cost")}</div>
              </div>
              <div className="metric-cell">
                <div className="metric-label">{t("stats.longTerm.calls")}</div>
                <div className="metric-value">{formatMetric(overview.global.calls, "calls")}</div>
              </div>
            </div>
            <div className="space-y-3" data-testid="long-term-chart-global-trend">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <h3 className="section-title text-base">{t("stats.longTerm.globalTrend")}</h3>
                <MetricToggle
                  value={globalMetric}
                  options={USAGE_METRICS}
                  onChange={setGlobalMetric}
                />
              </div>
              <LongTermChart
                series={globalChartSeries}
                metric={globalMetric}
                emptyLabel={t("stats.longTerm.emptyChart")}
              />
            </div>
            <div className="grid gap-6 xl:grid-cols-2">
              <div className="space-y-3" data-testid="long-term-chart-model-time">
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <h3 className="section-title text-base">{t("stats.longTerm.modelTime")}</h3>
                  <MetricToggle
                    value={modelMetric}
                    options={MODEL_TIME_METRICS}
                    onChange={setModelMetric}
                  />
                </div>
                <LongTermChart
                  series={modelSeries}
                  metric={modelMetric}
                  emptyLabel={t("stats.longTerm.emptyChart")}
                  modelSeries
                  visuals={modelVisuals}
                />
              </div>
              <div className="space-y-3" data-testid="long-term-chart-model-performance">
                <div className="flex flex-wrap items-center justify-between gap-3">
                  <h3 className="section-title text-base">
                    {t("stats.longTerm.modelPerformance")}
                  </h3>
                  <MetricToggle
                    value={performanceMetric}
                    options={MODEL_PERFORMANCE_METRICS}
                    onChange={setPerformanceMetric}
                  />
                </div>
                <LongTermChart
                  series={modelSeries}
                  metric={performanceMetric}
                  emptyLabel={t("stats.longTerm.emptyChart")}
                  modelSeries
                  visuals={modelVisuals}
                />
              </div>
            </div>
            <div className="space-y-3" data-testid="long-term-chart-model-usage">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <h3 className="section-title text-base">{t("stats.longTerm.modelUsage")}</h3>
                <MetricToggle
                  value={usageMetric}
                  options={USAGE_METRICS}
                  onChange={setUsageMetric}
                />
              </div>
              <LongTermChart
                series={modelSeries}
                metric={usageMetric}
                emptyLabel={t("stats.longTerm.emptyChart")}
                modelSeries
                stackedArea
                canonicalDates={stackedAreaDates}
                visuals={modelVisuals}
              />
            </div>
            <SeriesTable
              title={t("stats.longTerm.models")}
              entries={modelTable}
              totalMetrics={overview.global}
              modelEntries
              visuals={modelVisuals}
              selectedKeys={modelSelection}
              onToggle={(key) =>
                setModelSelection((current) =>
                  current.includes(key)
                    ? current.filter((item) => item !== key)
                    : current.length >= 8
                      ? current
                      : [...current, key],
                )
              }
              sortMetric={modelSort}
              onSort={setModelSort}
              search={modelSearch}
              onSearch={setModelSearch}
            />
            <div className="space-y-3" data-testid="long-term-chart-upstream-usage">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <h3 className="section-title text-base">{t("stats.longTerm.upstreamUsage")}</h3>
                <MetricToggle
                  value={upstreamMetric}
                  options={USAGE_METRICS}
                  onChange={setUpstreamMetric}
                />
              </div>
              <LongTermChart
                series={upstreamSeries}
                metric={upstreamMetric}
                emptyLabel={
                  isUpstreamSeriesLoading
                    ? t("stats.longTerm.loading")
                    : t("stats.longTerm.emptyChart")
                }
                stackedArea
                canonicalDates={stackedAreaDates}
                visuals={upstreamVisuals}
              />
            </div>
            {seriesError || upstreamSeriesError ? (
              <Alert variant="error">{seriesError ?? upstreamSeriesError}</Alert>
            ) : null}
            <SeriesTable
              title={t("stats.longTerm.upstreams")}
              entries={upstreamTable}
              visuals={upstreamVisuals}
              selectedKeys={upstreamSelection}
              onToggle={(key) =>
                setUpstreamSelection((current) =>
                  current.includes(key)
                    ? current.filter((item) => item !== key)
                    : current.length >= 8
                      ? current
                      : [...current, key],
                )
              }
              sortMetric={upstreamSort}
              onSort={setUpstreamSort}
              search={upstreamSearch}
              onSearch={setUpstreamSearch}
            />
          </>
        ) : null}
      </div>
    </section>
  );
}
