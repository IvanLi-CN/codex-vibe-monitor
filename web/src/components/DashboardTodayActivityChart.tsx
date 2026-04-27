import { memo, useEffect, useMemo } from "react";
import {
  Area,
  AreaChart,
  Bar,
  CartesianGrid,
  ComposedChart,
  Legend,
  Line,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { useTranslation } from "../i18n";
import {
  chartBaseTokens,
  chartStatusTokens,
  metricAccent,
  withOpacity,
} from "../lib/chartTheme";
import { formatTokensShort } from "../lib/numberFormatters";
import { useTheme } from "../theme";
import type { MetricKey } from "./Last24hTenMinuteHeatmap";
import { Alert } from "./ui/alert";
import type { TimeseriesResponse } from "../lib/api";
import {
  buildTodayMinuteChartData,
  type DashboardTodayMinuteDatum,
} from "./dashboardTodayActivityChartData";
import {
  recordTodayChartRender,
  useDashboardPerformanceDiagnosticsEnabled,
} from "../lib/dashboardPerformanceDiagnostics";

export interface DashboardTodayActivityChartProps {
  response: TimeseriesResponse | null;
  loading: boolean;
  error?: string | null;
  metric: MetricKey | "trend";
  closedNaturalDay?: boolean;
}

function buildChartRenderSignature({
  response,
  loading,
  error,
  metric,
  closedNaturalDay,
}: DashboardTodayActivityChartProps) {
  if (!response) {
    return JSON.stringify({
      metric,
      closedNaturalDay,
      loading,
      error: error ?? null,
      response: null,
    });
  }

  const aggregate = response.points.reduce(
    (current, point) => ({
      totalCount: current.totalCount + point.totalCount,
      failureCount: current.failureCount + point.failureCount,
      totalTokens: current.totalTokens + point.totalTokens,
      totalCost: current.totalCost + point.totalCost,
    }),
    {
      totalCount: 0,
      failureCount: 0,
      totalTokens: 0,
      totalCost: 0,
    },
  );
  const lastPoint = response.points.at(-1) ?? null;

  return JSON.stringify({
    metric,
    closedNaturalDay,
    loading,
    error: error ?? null,
    rangeStart: response.rangeStart,
    rangeEnd: response.rangeEnd,
    bucketSeconds: response.bucketSeconds,
    snapshotId: response.snapshotId ?? null,
    pointCount: response.points.length,
    aggregate,
    lastPoint,
  });
}

function formatCountValue(
  value: number,
  unitLabel: string,
  formatter: Intl.NumberFormat,
) {
  return `${formatter.format(value)} ${unitLabel}`;
}

function formatDurationMs(value: number, formatter: Intl.NumberFormat) {
  if (value >= 1000) {
    return `${formatter.format(value / 1000)}s`;
  }
  return `${formatter.format(value)}ms`;
}

interface TooltipPayloadEntry {
  payload?: DashboardTodayMinuteDatum;
}

interface ChartTooltipContentProps {
  active?: boolean;
  label?: string | number;
  payload?: TooltipPayloadEntry[];
  theme: {
    tooltipBg: string;
    tooltipBorder: string;
    axisText: string;
    success: string;
    failure: string;
    accent: string;
    tokens: string;
    costRate: string;
    firstResponseByteTotal: string;
  };
  renderValue: (
    point: DashboardTodayMinuteDatum,
  ) => Array<{ label: string; value: string; color: string }>;
}

function ChartTooltipContent({
  active,
  label,
  payload,
  theme,
  renderValue,
}: ChartTooltipContentProps) {
  const point = payload?.find((entry) => entry.payload)?.payload;
  if (!active || !point) return null;

  const rows = renderValue(point);
  if (rows.length === 0) return null;

  return (
    <div
      className="min-w-[180px] rounded-lg border px-3 py-2 shadow-lg"
      style={{
        backgroundColor: theme.tooltipBg,
        borderColor: theme.tooltipBorder,
        color: theme.axisText,
      }}
    >
      <div className="text-sm font-semibold">
        {typeof label === "string" ? label : point.tooltipLabel}
      </div>
      <div className="mt-2 space-y-1 text-xs">
        {rows.map((row) => (
          <div
            key={row.label}
            className="flex items-center justify-between gap-4"
          >
            <div className="flex items-center gap-2">
              <span
                className="inline-block h-2.5 w-2.5 rounded-full"
                style={{ backgroundColor: row.color }}
                aria-hidden="true"
              />
              <span>{row.label}</span>
            </div>
            <span className="font-medium">{row.value}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function DashboardTodayActivityChartImpl({
  response,
  loading,
  error,
  metric,
  closedNaturalDay = false,
}: DashboardTodayActivityChartProps) {
  const diagnosticsEnabled = useDashboardPerformanceDiagnosticsEnabled();
  const renderSignature = useMemo(
    () =>
      diagnosticsEnabled
        ? buildChartRenderSignature({
            response,
            loading,
            error,
            metric,
            closedNaturalDay,
          })
        : null,
    [
      closedNaturalDay,
      diagnosticsEnabled,
      error,
      loading,
      metric,
      response,
    ],
  );

  useEffect(() => {
    if (!diagnosticsEnabled || renderSignature == null) {
      return;
    }
    recordTodayChartRender(renderSignature);
  }, [diagnosticsEnabled, renderSignature]);

  const { t, locale } = useTranslation();
  const { themeMode } = useTheme();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag, { maximumFractionDigits: 2 }),
    [localeTag],
  );
  const currencyFormatter = useMemo(
    () =>
      new Intl.NumberFormat(localeTag, {
        style: "currency",
        currency: "USD",
        maximumFractionDigits: 4,
      }),
    [localeTag],
  );
  const chartColors = useMemo(() => {
    const base = chartBaseTokens(themeMode);
    const status = chartStatusTokens(themeMode);
    const accent = metricAccent(metric === "trend" ? "totalTokens" : metric, themeMode);
    return {
      ...base,
      success: status.success,
      successFill: withOpacity(status.success, 0.24),
      failure: status.failure,
      failureFill: withOpacity(status.failure, 0.24),
      accent,
      accentFill: withOpacity(accent, 0.22),
      tokens: metricAccent("totalTokens", themeMode),
      costRate: metricAccent("totalCost", themeMode),
      firstResponseByteTotal: metricAccent("totalCount", themeMode),
    };
  }, [metric, themeMode]);

  const data = useMemo(
    () => buildTodayMinuteChartData(response, { localeTag, closedNaturalDay }),
    [closedNaturalDay, localeTag, response],
  );

  const countUnit = t("unit.calls");
  const countSeriesNames = useMemo(
    () => ({
      success: t("stats.cards.success"),
      failures: t("stats.cards.failures"),
      inFlight: t("chart.inFlight"),
      total: t("chart.totalCount"),
      firstResponseByteTotal: t("chart.firstResponseByteTotal"),
    }),
    [t],
  );
  const areaSeriesName =
    metric === "totalCost" ? t("chart.totalCost") : t("chart.totalTokens");
  const trendSeriesNames = useMemo(
    () => ({
      tokensPerMinute: t("chart.tokensPerMinute"),
      costRate: t("chart.spendRate"),
    }),
    [t],
  );
  const countAxisBound = useMemo(() => {
    const maxValue = data.reduce(
      (current, item) =>
        Math.max(
          current,
          item.successCount + item.inFlightCount,
          item.failureCount,
        ),
      0,
    );
    return Math.max(1, maxValue);
  }, [data]);

  if (error) {
    return <Alert variant="error">{error}</Alert>;
  }

  if (loading && !response) {
    return (
      <div className="h-80 w-full animate-pulse rounded-xl border border-base-300/70 bg-base-200/55" />
    );
  }

  if (!loading && data.length === 0) {
    return <Alert>{t("chart.noDataRange")}</Alert>;
  }

  const chartData =
    data.length > 0
      ? data
      : buildTodayMinuteChartData(response, { localeTag, closedNaturalDay });
  const animate = chartData.length <= 800;
  const chartMode =
    metric === "totalCount"
      ? "count-bars"
      : metric === "trend"
        ? "trend-lines"
        : "cumulative-area";
  const renderCountTooltip = (point: DashboardTodayMinuteDatum) =>
    point.chartSuccessCount == null || point.chartFailureCountNegative == null
      ? []
      : [
          {
            label: countSeriesNames.success,
            value: formatCountValue(
              point.chartSuccessCount,
              countUnit,
              numberFormatter,
            ),
            color: chartColors.success,
          },
          {
            label: countSeriesNames.failures,
            value: formatCountValue(
              Math.abs(point.chartFailureCountNegative),
              countUnit,
              numberFormatter,
            ),
            color: chartColors.failure,
          },
          ...(point.inFlightCount > 0
            ? [
                {
                  label: countSeriesNames.inFlight,
                  value: formatCountValue(
                    point.inFlightCount,
                    countUnit,
                    numberFormatter,
                  ),
                  color: chartColors.accent,
                },
              ]
            : []),
          {
            label: countSeriesNames.total,
            value: formatCountValue(
              point.totalCount,
              countUnit,
              numberFormatter,
            ),
            color: chartColors.accent,
          },
          ...(point.chartFirstResponseByteTotalAvgMs == null
            ? []
            : [
                {
                  label: countSeriesNames.firstResponseByteTotal,
                  value: formatDurationMs(
                    point.chartFirstResponseByteTotalAvgMs,
                    numberFormatter,
                  ),
                  color: chartColors.firstResponseByteTotal,
                },
              ]),
        ];
  const renderAreaTooltip = (point: DashboardTodayMinuteDatum) => [
    ...(metric === "totalCost"
      ? point.cumulativeCost == null
        ? []
        : [
            {
              label: areaSeriesName,
              value: currencyFormatter.format(point.cumulativeCost),
              color: chartColors.accent,
            },
          ]
      : point.cumulativeTokens == null
        ? []
        : [
            {
              label: areaSeriesName,
              value: formatTokensShort(point.cumulativeTokens, localeTag),
              color: chartColors.accent,
            },
          ]),
  ];
  const renderTrendTooltip = (point: DashboardTodayMinuteDatum) =>
    point.chartTokensPerMinute == null || point.chartCostRate == null
      ? []
      : [
          {
            label: trendSeriesNames.tokensPerMinute,
            value: formatTokensShort(point.chartTokensPerMinute, localeTag),
            color: chartColors.tokens,
          },
          {
            label: trendSeriesNames.costRate,
            value: currencyFormatter.format(point.chartCostRate),
            color: chartColors.costRate,
          },
        ];

  return (
    <section
      className="rounded-xl border border-base-300/75 bg-base-200/40 p-4"
      data-testid="dashboard-today-activity-chart"
      data-chart-mode={chartMode}
      data-chart-metric={metric}
    >
      <div className="h-80 w-full" data-chart-kind="dashboard-today-activity">
        <ResponsiveContainer>
          {metric === "totalCount" ? (
            <ComposedChart
              data={chartData}
              margin={{ top: 12, right: 24, left: 0, bottom: 8 }}
              barGap="-100%"
            >
              <CartesianGrid
                stroke={chartColors.gridLine}
                strokeDasharray="3 3"
              />
              <XAxis
                dataKey="index"
                type="number"
                domain={[0, Math.max(0, chartData.length - 1)]}
                minTickGap={28}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
                tickFormatter={(value: number) => {
                  const item =
                    chartData[
                      Math.max(
                        0,
                        Math.min(chartData.length - 1, Math.round(value)),
                      )
                    ];
                  return item?.label ?? String(value);
                }}
              />
              <YAxis
                yAxisId="count"
                domain={[-countAxisBound, countAxisBound]}
                allowDecimals={false}
                tickFormatter={(value) =>
                  numberFormatter.format(Math.abs(Number(value)))
                }
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
              />
              <YAxis
                yAxisId="latency"
                orientation="right"
                tickFormatter={(value) =>
                  formatDurationMs(Number(value), numberFormatter)
                }
                width={72}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
              />
              <Tooltip
                labelFormatter={(value) => {
                  const item =
                    chartData[
                      Math.max(
                        0,
                        Math.min(
                          chartData.length - 1,
                          Math.round(Number(value)),
                        ),
                      )
                    ];
                  return item?.tooltipLabel ?? String(value);
                }}
                content={(props) => (
                  <ChartTooltipContent
                    active={props.active}
                    label={props.label}
                    payload={
                      props.payload as unknown as
                        | TooltipPayloadEntry[]
                        | undefined
                    }
                    theme={chartColors}
                    renderValue={renderCountTooltip}
                  />
                )}
              />
              <Legend wrapperStyle={{ color: chartColors.axisText }} />
              <ReferenceLine yAxisId="count" y={0} stroke={chartColors.gridLine} />
              <Bar
                yAxisId="count"
                dataKey="chartSuccessCount"
                name={countSeriesNames.success}
                stackId="positive"
                fill={chartColors.success}
                radius={[0, 0, 0, 0]}
                isAnimationActive={animate}
              />
              <Bar
                yAxisId="count"
                dataKey="chartInFlightCount"
                name={countSeriesNames.inFlight}
                stackId="positive"
                fill={chartColors.accent}
                radius={[3, 3, 0, 0]}
                isAnimationActive={animate}
              />
              <Bar
                yAxisId="count"
                dataKey="chartFailureCountNegative"
                name={countSeriesNames.failures}
                fill={chartColors.failure}
                radius={[0, 0, 3, 3]}
                isAnimationActive={animate}
              />
              <Line
                yAxisId="latency"
                type="monotone"
                dataKey="chartFirstResponseByteTotalAvgMs"
                name={countSeriesNames.firstResponseByteTotal}
                stroke={chartColors.firstResponseByteTotal}
                strokeWidth={2}
                dot={false}
                connectNulls={false}
                isAnimationActive={animate}
              />
            </ComposedChart>
          ) : metric === "trend" ? (
            <ComposedChart
              data={chartData}
              margin={{ top: 12, right: 24, left: 0, bottom: 8 }}
            >
              <CartesianGrid
                stroke={chartColors.gridLine}
                strokeDasharray="3 3"
              />
              <XAxis
                dataKey="index"
                type="number"
                domain={[0, Math.max(0, chartData.length - 1)]}
                minTickGap={28}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
                tickFormatter={(value: number) => {
                  const item =
                    chartData[
                      Math.max(
                        0,
                        Math.min(chartData.length - 1, Math.round(value)),
                      )
                    ];
                  return item?.label ?? String(value);
                }}
              />
              <YAxis
                yAxisId="tokens"
                tickFormatter={(value) =>
                  formatTokensShort(Number(value), localeTag)
                }
                width={80}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
              />
              <YAxis
                yAxisId="cost"
                orientation="right"
                tickFormatter={(value) =>
                  currencyFormatter.format(Number(value))
                }
                width={90}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
              />
              <Tooltip
                labelFormatter={(value) => {
                  const item =
                    chartData[
                      Math.max(
                        0,
                        Math.min(
                          chartData.length - 1,
                          Math.round(Number(value)),
                        ),
                      )
                    ];
                  return item?.tooltipLabel ?? String(value);
                }}
                content={(props) => (
                  <ChartTooltipContent
                    active={props.active}
                    label={props.label}
                    payload={
                      props.payload as unknown as
                        | TooltipPayloadEntry[]
                        | undefined
                    }
                    theme={chartColors}
                    renderValue={renderTrendTooltip}
                  />
                )}
              />
              <Legend wrapperStyle={{ color: chartColors.axisText }} />
              <Line
                yAxisId="tokens"
                type="monotone"
                dataKey="chartTokensPerMinute"
                name={trendSeriesNames.tokensPerMinute}
                stroke={chartColors.tokens}
                strokeWidth={2}
                dot={false}
                connectNulls={false}
                isAnimationActive={animate}
              />
              <Line
                yAxisId="cost"
                type="monotone"
                dataKey="chartCostRate"
                name={trendSeriesNames.costRate}
                stroke={chartColors.costRate}
                strokeWidth={2}
                dot={false}
                connectNulls={false}
                isAnimationActive={animate}
              />
            </ComposedChart>
          ) : (
            <AreaChart
              data={chartData}
              margin={{ top: 12, right: 24, left: 0, bottom: 8 }}
            >
              <CartesianGrid
                stroke={chartColors.gridLine}
                strokeDasharray="3 3"
              />
              <XAxis
                dataKey="index"
                type="number"
                domain={[0, Math.max(0, chartData.length - 1)]}
                minTickGap={28}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
                tickFormatter={(value: number) => {
                  const item =
                    chartData[
                      Math.max(
                        0,
                        Math.min(chartData.length - 1, Math.round(value)),
                      )
                    ];
                  return item?.label ?? String(value);
                }}
              />
              <YAxis
                tickFormatter={(value) =>
                  metric === "totalCost"
                    ? currencyFormatter.format(Number(value))
                    : formatTokensShort(Number(value), localeTag)
                }
                width={metric === "totalCost" ? 90 : 80}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
              />
              <Tooltip
                labelFormatter={(value) => {
                  const item =
                    chartData[
                      Math.max(
                        0,
                        Math.min(
                          chartData.length - 1,
                          Math.round(Number(value)),
                        ),
                      )
                    ];
                  return item?.tooltipLabel ?? String(value);
                }}
                content={(props) => (
                  <ChartTooltipContent
                    active={props.active}
                    label={props.label}
                    payload={
                      props.payload as unknown as
                        | TooltipPayloadEntry[]
                        | undefined
                    }
                    theme={chartColors}
                    renderValue={renderAreaTooltip}
                  />
                )}
              />
              <Area
                type="monotone"
                dataKey={
                  metric === "totalCost"
                    ? "chartCumulativeCost"
                    : "chartCumulativeTokens"
                }
                name={areaSeriesName}
                stroke={chartColors.accent}
                fill={chartColors.accentFill}
                fillOpacity={1}
                strokeWidth={2}
                isAnimationActive={animate}
              />
            </AreaChart>
          )}
        </ResponsiveContainer>
      </div>
    </section>
  );
}

export const DashboardTodayActivityChart = memo(DashboardTodayActivityChartImpl);
