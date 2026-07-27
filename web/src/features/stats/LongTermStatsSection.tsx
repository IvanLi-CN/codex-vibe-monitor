import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useMemo, useRef, useState } from "react";
import {
  CartesianGrid,
  type DefaultLegendContentProps,
  Legend,
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
import { chartBaseTokens, piePalette } from "../../lib/chartTheme";
import { useTheme } from "../../theme";
import { ModelIdentity } from "../shared/ModelIdentity";

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

function mergeSeriesPoints(series: LongTermSeries[], metric: MetricKey) {
  const byDate = new Map<string, Record<string, string | number | null>>();
  for (const item of series) {
    for (const point of item.points) {
      const existing = byDate.get(point.date) ?? { date: point.date };
      existing[item.seriesKey] = metricValue(point, metric);
      byDate.set(point.date, existing);
    }
  }
  return [...byDate.values()].sort((left, right) =>
    String(left.date).localeCompare(String(right.date)),
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
}: {
  series: LongTermSeries[];
  metric: MetricKey;
  emptyLabel: string;
  modelSeries?: boolean;
}) {
  const { themeMode } = useTheme();
  const colors = chartBaseTokens(themeMode);
  const seriesColors = piePalette(themeMode);
  const chartData = mergeSeriesPoints(series, metric);
  if (series.length === 0 || chartData.length === 0) {
    return (
      <div className="flex h-64 items-center justify-center rounded-lg border border-dashed border-base-300 text-sm opacity-70">
        {emptyLabel}
      </div>
    );
  }
  return (
    <div className="h-64 w-full min-w-0" data-chart-kind="long-term-series">
      <ResponsiveContainer>
        <LineChart data={chartData} margin={{ top: 8, right: 12, left: 4, bottom: 4 }}>
          <CartesianGrid stroke={colors.gridLine} strokeDasharray="3 3" />
          <XAxis dataKey="date" tick={{ fill: colors.axisText, fontSize: 11 }} minTickGap={24} />
          <YAxis
            tick={{ fill: colors.axisText, fontSize: 11 }}
            tickFormatter={(value) => formatMetric(Number(value), metric)}
            width={76}
          />
          <Tooltip
            contentStyle={{
              backgroundColor: colors.tooltipBg,
              borderColor: colors.tooltipBorder,
              borderRadius: 8,
            }}
            formatter={(value, key) => [
              formatMetric(value == null ? null : Number(value), metric),
              String(key),
            ]}
          />
          <Legend content={<LongTermChartLegend modelSeries={modelSeries} />} />
          {series.map((item, index) => (
            <Line
              key={item.seriesKey}
              type="monotone"
              dataKey={item.seriesKey}
              name={item.displayName}
              stroke={seriesColors[index % seriesColors.length] ?? metricColors[metric]}
              strokeWidth={2}
              dot={false}
              connectNulls
            />
          ))}
        </LineChart>
      </ResponsiveContainer>
    </div>
  );
}

function LongTermChartLegend({
  payload,
  modelSeries,
}: DefaultLegendContentProps & { modelSeries: boolean }) {
  if (!payload || payload.length === 0) return null;

  return (
    <div className="flex flex-wrap items-center justify-center gap-x-4 gap-y-1 px-2 pt-1 text-xs text-base-content/75">
      {payload.map((entry) => {
        const label = entry.value ?? "";
        return (
          <span
            key={String(entry.dataKey ?? label)}
            className="inline-flex min-w-0 items-center gap-1.5"
          >
            <span
              className="h-2 w-2 flex-none rounded-full"
              style={{ backgroundColor: entry.color ?? "currentColor" }}
              aria-hidden
            />
            {modelSeries ? (
              <ModelIdentity model={label} className="max-w-[12rem] justify-start" />
            ) : (
              <span className="max-w-[12rem] truncate" title={label}>
                {label}
              </span>
            )}
          </span>
        );
      })}
    </div>
  );
}

function SeriesTable({
  title,
  entries,
  selectedKeys,
  onToggle,
  sortMetric,
  onSort,
  search,
  onSearch,
  modelEntries = false,
}: {
  title: string;
  entries: LongTermSeriesSummary[];
  selectedKeys: string[];
  onToggle: (key: string) => void;
  sortMetric: MetricKey;
  onSort: (metric: MetricKey) => void;
  search: string;
  onSearch: (value: string) => void;
  modelEntries?: boolean;
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
  const rowVirtualizer = useVirtualizer({
    count: filtered.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 48,
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
            className="sticky top-0 z-10 h-10 border-b border-base-300 bg-base-200/95 text-xs font-semibold backdrop-blur"
            style={{ width: gridWidth }}
          >
            <span className="sticky left-0 z-20 inline-flex h-full w-12 items-center bg-base-200/95 px-3" />
            <span className="sticky left-12 z-20 inline-flex h-full w-60 items-center bg-base-200/95 px-3">
              {t("stats.longTerm.name")}
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
          <div
            className="relative"
            style={{ height: rowVirtualizer.getTotalSize(), width: gridWidth }}
          >
            {rowVirtualizer.getVirtualItems().map((virtualRow) => {
              const entry = filtered[virtualRow.index];
              const selected = selectedKeys.includes(entry.seriesKey);
              return (
                <div
                  key={entry.seriesKey}
                  className="absolute left-0 border-b border-base-300/50 text-sm"
                  style={{
                    transform: `translateY(${virtualRow.start}px)`,
                    height: virtualRow.size,
                    width: gridWidth,
                  }}
                >
                  <span className="sticky left-0 z-10 inline-flex h-full w-12 items-center bg-base-100 px-3">
                    <input
                      type="checkbox"
                      checked={selected}
                      onChange={() => onToggle(entry.seriesKey)}
                      aria-label={`${t("stats.longTerm.select")} ${entry.displayName}`}
                      disabled={!selected && selectedKeys.length >= 8}
                    />
                  </span>
                  <span className="sticky left-12 z-10 inline-flex h-full w-60 min-w-0 flex-col justify-center bg-base-100 px-3 pr-3">
                    {modelEntries ? (
                      <ModelIdentity
                        model={entry.displayName}
                        className="max-w-full justify-start"
                        textClassName="block truncate font-medium"
                      />
                    ) : (
                      <span className="block truncate font-medium" title={entry.displayName}>
                        {entry.displayName}
                      </span>
                    )}
                    {entry.reasoningEffort ? (
                      <span className="block truncate text-xs opacity-60">
                        {entry.reasoningEffort}
                      </span>
                    ) : null}
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
                          className="absolute top-0 h-full truncate px-1 text-left tabular-nums hover:text-primary"
                          style={{ left: virtualColumn.start, width: virtualColumn.size }}
                          onClick={() => onSort(metric)}
                        >
                          {formatMetric(metricValue(entry, metric), metric)}
                        </button>
                      );
                    })}
                  </div>
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
}

export function LongTermStatsSection({
  initialRange = "7d",
  overviewOverride,
  seriesOverride,
}: LongTermStatsSectionProps) {
  const { t } = useTranslation();
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
  const modelSeries = seriesOverride ?? fetchedSeries?.series ?? [];
  const upstreamKeys = useMemo(() => upstreamSelection.slice(0, 8), [upstreamSelection]);
  const {
    series: fetchedUpstreamSeries,
    isSeriesLoading: isUpstreamSeriesLoading,
    seriesError: upstreamSeriesError,
  } = useLongTermStats(range, "upstream", upstreamKeys, !overviewOverride, overview);
  const upstreamSeries = seriesOverride ? [] : (fetchedUpstreamSeries?.series ?? []);

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
        displayName: t("stats.longTerm.global"),
        points: overview?.daily ?? [],
      },
    ],
    [overview?.daily, t],
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
            <div className="space-y-3">
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
              <div className="space-y-3">
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
                />
              </div>
              <div className="space-y-3">
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
                />
              </div>
            </div>
            <div className="space-y-3">
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
              />
            </div>
            <SeriesTable
              title={t("stats.longTerm.models")}
              entries={modelTable}
              modelEntries
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
            <div className="space-y-3">
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
              />
            </div>
            {seriesError || upstreamSeriesError ? (
              <Alert variant="error">{seriesError ?? upstreamSeriesError}</Alert>
            ) : null}
            <SeriesTable
              title={t("stats.longTerm.upstreams")}
              entries={upstreamTable}
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
