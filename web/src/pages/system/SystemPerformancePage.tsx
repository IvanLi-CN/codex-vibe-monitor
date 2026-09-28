import { useEffect, useMemo, useState } from "react";
import {
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
import { Chip } from "../../components/ui/chip";
import { SelectField } from "../../components/ui/select-field";
import { useTranslation } from "../../i18n";
import {
  fetchPerformanceHealth,
  fetchPerformanceMetrics,
  type PerformanceMetricsResponse,
  type PerformanceRange,
  type PerformanceSection,
  type PerformanceSeries,
  type PerformanceTelemetryHealth,
} from "../../lib/api";

const REFRESH_INTERVAL_MS = 60_000;
const MAX_RENDERED_POINTS = 180;
const RANGE_OPTIONS: readonly PerformanceRange[] = ["6h", "24h", "7d", "30d", "13mo"];
const SECTION_OPTIONS: readonly (PerformanceSection | "all")[] = [
  "all",
  "overview",
  "storage",
  "projection",
  "maintenance",
  "process",
  "browser",
];

function valueForPoint(series: PerformanceSeries, point: PerformanceSeries["points"][number]) {
  if (series.kind === "gauge") {
    return point.weightedAverage ?? point.last ?? point.max ?? null;
  }
  if (series.kind === "duration") {
    return point.sampleCount > 0 ? point.sum / point.sampleCount : null;
  }
  return point.sampleCount > 0 ? point.sum : null;
}

function formatMetricValue(value: number | null, unit: PerformanceSeries["unit"], unknown: string) {
  if (value == null || !Number.isFinite(value)) return unknown;
  const rounded = value >= 100 ? Math.round(value) : Math.round(value * 10) / 10;
  if (unit === "bytes") {
    if (rounded >= 1024 * 1024 * 1024) return `${(rounded / (1024 * 1024 * 1024)).toFixed(1)} GiB`;
    if (rounded >= 1024 * 1024) return `${(rounded / (1024 * 1024)).toFixed(1)} MiB`;
    if (rounded >= 1024) return `${(rounded / 1024).toFixed(1)} KiB`;
  }
  if (unit === "percent") return `${rounded}%`;
  return `${rounded.toLocaleString()} ${unit === "milliseconds" ? "ms" : unit}`;
}

function chartRows(series: PerformanceSeries) {
  const points =
    series.points.length <= MAX_RENDERED_POINTS
      ? series.points
      : series.points.filter(
          (_, index) =>
            index === 0 ||
            index === series.points.length - 1 ||
            index % Math.ceil(series.points.length / MAX_RENDERED_POINTS) === 0,
        );
  return points.map((point) => ({
    time: new Date(point.bucketStart * 1000).toLocaleString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    }),
    value: valueForPoint(series, point),
  }));
}

function percentileFromHistogram(series: PerformanceSeries, percentile: number): number | null {
  if (series.kind !== "duration") return null;
  const histogram = series.points.reduce(
    (totals, point) => {
      point.histogram?.forEach((value, index) => {
        totals[index] += value;
      });
      return totals;
    },
    [0, 0, 0, 0, 0, 0, 0, 0],
  );
  const total = histogram.reduce((sum, value) => sum + value, 0);
  if (total === 0) return null;
  const target = Math.max(1, Math.ceil(total * percentile));
  let seen = 0;
  const upperBounds = [1, 5, 10, 25, 50, 100, 250, Number.POSITIVE_INFINITY];
  for (let index = 0; index < histogram.length; index += 1) {
    seen += histogram[index];
    if (seen >= target) return upperBounds[index];
  }
  return null;
}

function healthTone(state: string): "success" | "warning" | "error" | "neutral" {
  if (state === "healthy") return "success";
  if (state === "degraded" || state === "starting") return "warning";
  if (state === "unavailable") return "error";
  return "neutral";
}

export default function SystemPerformancePage() {
  const { t } = useTranslation();
  const [range, setRange] = useState<PerformanceRange>("24h");
  const [section, setSection] = useState<PerformanceSection | "all">("all");
  const [metrics, setMetrics] = useState<PerformanceMetricsResponse | null>(null);
  const [health, setHealth] = useState<PerformanceTelemetryHealth | null>(null);
  const [healthError, setHealthError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    let active = true;
    const load = async () => {
      const [metricsResult, healthResult] = await Promise.allSettled([
        fetchPerformanceMetrics({ range, section: section === "all" ? undefined : section }),
        fetchPerformanceHealth(),
      ]);
      if (!active) return;
      if (metricsResult.status === "fulfilled") {
        setMetrics(metricsResult.value);
        setError(null);
      } else {
        setError(
          metricsResult.reason instanceof Error
            ? metricsResult.reason.message
            : String(metricsResult.reason),
        );
      }
      if (healthResult.status === "fulfilled") {
        setHealth(healthResult.value);
        setHealthError(null);
      } else {
        setHealth(null);
        setHealthError(
          healthResult.reason instanceof Error
            ? healthResult.reason.message
            : String(healthResult.reason),
        );
      }
      setIsLoading(false);
    };
    setIsLoading(true);
    void load();
    const timer = window.setInterval(() => void load(), REFRESH_INTERVAL_MS);
    return () => {
      active = false;
      window.clearInterval(timer);
    };
  }, [range, section]);

  const visibleSeries = useMemo(() => metrics?.series ?? [], [metrics]);
  const groupedSeries = useMemo(() => {
    const groups = new Map<string, PerformanceSeries[]>();
    for (const series of visibleSeries) {
      const group = groups.get(series.section) ?? [];
      group.push(series);
      groups.set(series.section, group);
    }
    return groups;
  }, [visibleSeries]);
  const sampleCount = visibleSeries.reduce(
    (sum, series) =>
      sum + series.points.reduce((pointSum, point) => pointSum + point.sampleCount, 0),
    0,
  );
  const unknownPointCount = visibleSeries.reduce(
    (sum, series) =>
      sum +
      (series.points.length === 0
        ? 1
        : series.points.reduce(
            (pointSum, point) => pointSum + (point.sampleCount === 0 ? 1 : 0),
            0,
          )),
    0,
  );
  const statusText = health ? t("system.performance.healthState", { state: health.state }) : null;

  return (
    <section className="surface-panel overflow-hidden" data-testid="system-performance-page">
      <div className="surface-panel-body gap-6">
        <div className="flex flex-col gap-4 xl:flex-row xl:items-end xl:justify-between">
          <div className="section-heading">
            <h2 className="section-title text-2xl">{t("system.performance.title")}</h2>
            <p className="section-description max-w-3xl">{t("system.performance.description")}</p>
          </div>
          <div className="grid gap-3 sm:grid-cols-[auto_13rem]">
            <fieldset
              className="flex flex-wrap items-center gap-2"
              aria-label={t("system.performance.range")}
            >
              {RANGE_OPTIONS.map((option) => (
                <Button
                  key={option}
                  size="sm"
                  variant={range === option ? "default" : "outline"}
                  onClick={() => setRange(option)}
                >
                  {t(`system.performance.range.${option}`)}
                </Button>
              ))}
            </fieldset>
            <SelectField
              value={section}
              onValueChange={(value) => setSection(value as PerformanceSection | "all")}
              options={SECTION_OPTIONS.map((option) => ({
                value: option,
                label:
                  option === "all"
                    ? t("system.performance.allSections")
                    : t(`system.performance.section.${option}`),
              }))}
              aria-label={t("system.performance.section")}
            />
          </div>
        </div>

        {health ? (
          <div className="flex flex-wrap items-center gap-3 border-y border-base-300/70 py-3 text-sm">
            <Chip tone={healthTone(health.state)}>{statusText}</Chip>
            <span className="text-base-content/65">
              {t("system.performance.health")} · {health.droppedSamples.toLocaleString()}{" "}
              {t("system.performance.samples")}
            </span>
            {health.lastError ? (
              <span className="max-w-full break-words text-warning">
                {t("system.performance.healthError", { error: health.lastError })}
              </span>
            ) : null}
          </div>
        ) : null}

        {healthError ? (
          <Alert variant="error" data-testid="system-performance-health-error">
            {t("system.performance.loadError", { error: healthError })}
          </Alert>
        ) : null}

        {error ? (
          <Alert variant="error">{t("system.performance.loadError", { error })}</Alert>
        ) : null}

        {isLoading && !metrics ? (
          <div
            className="flex min-h-48 items-center justify-center text-sm text-base-content/60"
            role="status"
          >
            {t("system.performance.loading")}
          </div>
        ) : metrics ? (
          <>
            <div className="grid gap-3 sm:grid-cols-4">
              <div className="rounded-lg border border-base-300/70 bg-base-100/70 p-4">
                <div className="text-xs uppercase tracking-[0.16em] text-base-content/55">
                  {t("system.performance.coverage")}
                </div>
                <div className="mt-2 text-2xl font-semibold">
                  {Math.round(metrics.coverage * 100)}%
                </div>
              </div>
              <div className="rounded-lg border border-base-300/70 bg-base-100/70 p-4">
                <div className="text-xs uppercase tracking-[0.16em] text-base-content/55">
                  {t("system.performance.epochs")}
                </div>
                <div className="mt-2 text-2xl font-semibold">
                  {metrics.epochs.length.toLocaleString()}
                </div>
              </div>
              <div className="rounded-lg border border-base-300/70 bg-base-100/70 p-4">
                <div className="text-xs uppercase tracking-[0.16em] text-base-content/55">
                  {t("system.performance.samples")}
                </div>
                <div className="mt-2 text-2xl font-semibold">{sampleCount.toLocaleString()}</div>
              </div>
              <div className="rounded-lg border border-base-300/70 bg-base-100/70 p-4">
                <div className="text-xs uppercase tracking-[0.16em] text-base-content/55">
                  {t("system.performance.metricCount")}
                </div>
                <div className="mt-2 text-2xl font-semibold">
                  {visibleSeries.length.toLocaleString()}
                </div>
              </div>
            </div>

            {unknownPointCount > 0 ? (
              <div className="text-xs text-base-content/60">
                {t("system.performance.unknownPoints", {
                  count: unknownPointCount.toLocaleString(),
                })}
              </div>
            ) : null}

            {health?.state === "degraded" || health?.state === "unavailable" ? (
              <Alert variant="warning">
                {health.state === "unavailable"
                  ? t("system.performance.statusUnavailable")
                  : t("system.performance.degraded")}
              </Alert>
            ) : null}

            {visibleSeries.length === 0 ? (
              <div className="rounded-lg border border-dashed border-base-300 p-8 text-center text-sm text-base-content/60">
                {t("system.performance.noData")}
              </div>
            ) : (
              <div className="space-y-8">
                {[...groupedSeries.entries()].map(([group, seriesList]) => (
                  <section
                    key={group}
                    className="space-y-3"
                    data-testid={`performance-section-${group}`}
                  >
                    <div className="flex items-baseline justify-between gap-3 border-b border-base-300/70 pb-2">
                      <h3 className="text-lg font-semibold">
                        {group === "history"
                          ? t("system.performance.section.history")
                          : t(`system.performance.section.${group}`)}
                      </h3>
                      <span className="text-xs text-base-content/55">
                        {seriesList.length.toLocaleString()} {t("system.performance.metricCount")}
                      </span>
                    </div>
                    <div className="grid gap-4 lg:grid-cols-2">
                      {seriesList.map((series) => {
                        const rows = chartRows(series);
                        const latest = series.points.at(-1);
                        const p95 = percentileFromHistogram(series, 0.95);
                        return (
                          <article
                            key={`${series.metricId}:${series.dimension}`}
                            className="rounded-lg border border-base-300/70 bg-base-100/70 p-4"
                            data-testid="performance-series"
                          >
                            <div className="flex flex-wrap items-start justify-between gap-3">
                              <div>
                                <h3 className="font-semibold">{series.metricId}</h3>
                                <p className="text-xs text-base-content/60">
                                  {series.dimension} ·{" "}
                                  {t(`system.performance.metric.${series.unit}`)}
                                </p>
                              </div>
                              <div className="text-right">
                                <div className="text-lg font-semibold">
                                  {formatMetricValue(
                                    latest ? valueForPoint(series, latest) : null,
                                    series.unit,
                                    t("system.performance.unknown"),
                                  )}
                                </div>
                                <div className="text-xs text-base-content/55">
                                  {t("system.performance.latest")}
                                </div>
                                {p95 != null ? (
                                  <div className="mt-1 text-xs text-base-content/60">
                                    p95{" "}
                                    {formatMetricValue(
                                      p95,
                                      series.unit,
                                      t("system.performance.unknown"),
                                    )}
                                  </div>
                                ) : null}
                              </div>
                            </div>
                            <div className="mt-2 text-xs text-base-content/55">
                              {series.points
                                .reduce((sum, point) => sum + point.sampleCount, 0)
                                .toLocaleString()}{" "}
                              {t("system.performance.samples")}
                            </div>
                            <div className="mt-3 h-44" data-testid="performance-series-chart">
                              {rows.length > 0 ? (
                                <ResponsiveContainer width="100%" height="100%">
                                  <LineChart
                                    data={rows}
                                    margin={{ top: 8, right: 8, left: -18, bottom: 0 }}
                                  >
                                    <CartesianGrid
                                      stroke="currentColor"
                                      opacity={0.12}
                                      strokeDasharray="3 3"
                                    />
                                    <XAxis dataKey="time" hide />
                                    <YAxis
                                      width={42}
                                      tick={{ fontSize: 10 }}
                                      tickLine={false}
                                      axisLine={false}
                                    />
                                    <Tooltip
                                      formatter={(value) =>
                                        formatMetricValue(
                                          value == null ? null : Number(value),
                                          series.unit,
                                          t("system.performance.unknown"),
                                        )
                                      }
                                    />
                                    <Line
                                      type="monotone"
                                      dataKey="value"
                                      stroke="#2563eb"
                                      strokeWidth={2}
                                      dot={false}
                                      isAnimationActive={false}
                                      connectNulls={false}
                                    />
                                  </LineChart>
                                </ResponsiveContainer>
                              ) : (
                                <div className="flex h-full items-center justify-center text-xs text-base-content/55">
                                  {t("system.performance.noData")}
                                </div>
                              )}
                            </div>
                          </article>
                        );
                      })}
                    </div>
                  </section>
                ))}
              </div>
            )}
          </>
        ) : (
          <div className="rounded-lg border border-dashed border-base-300 p-8 text-center text-sm text-base-content/60">
            {t("system.performance.noData")}
          </div>
        )}
      </div>
    </section>
  );
}
