import { useMemo } from "react";
import {
  Area,
  AreaChart,
  Bar,
  CartesianGrid,
  ComposedChart,
  Legend,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { Alert } from "../../components/ui/alert";
import { Spinner } from "../../components/ui/spinner";
import { useTranslation } from "../../i18n";
import type { TimeseriesPoint } from "../../lib/api";
import { chartBaseTokens, metricAccent, withOpacity } from "../../lib/chartTheme";
import { useTheme } from "../../theme";
import { buildTimeseriesChartData, resolveTimeseriesChartMode } from "./timeseriesChartModel";

interface TimeseriesChartProps {
  points: TimeseriesPoint[];
  isLoading: boolean;
  bucketSeconds?: number;
  showDate?: boolean;
}

type TimeseriesChartData = ReturnType<typeof buildTimeseriesChartData>;
type TimeseriesChartColors = ReturnType<typeof chartBaseTokens> & {
  tokenColor: string;
  tokenFill: string;
  countColor: string;
  countFill: string;
  costColor: string;
  costFill: string;
};

interface TimeseriesChartViewProps {
  chartMode: ReturnType<typeof resolveTimeseriesChartMode>;
  chartData: TimeseriesChartData;
  chartColors: TimeseriesChartColors;
  numberFormatter: Intl.NumberFormat;
  currencyFormatter: Intl.NumberFormat;
  seriesNames: { totalTokens: string; totalCost: string; totalCount: string };
  animate: boolean;
}

function TimeseriesAxes({
  chartColors,
  numberFormatter,
  currencyFormatter,
}: Pick<TimeseriesChartViewProps, "chartColors" | "numberFormatter" | "currencyFormatter">) {
  return (
    <>
      <CartesianGrid stroke={chartColors.gridLine} strokeDasharray="3 3" />
      <XAxis
        dataKey="label"
        minTickGap={32}
        angle={-15}
        dy={8}
        height={60}
        interval="preserveStartEnd"
        axisLine={{ stroke: chartColors.gridLine }}
        tickLine={{ stroke: chartColors.gridLine }}
        tick={{ fill: chartColors.axisText, fontSize: 12 }}
      />
      <YAxis
        yAxisId="tokens"
        orientation="left"
        tickFormatter={(value) => numberFormatter.format(value as number)}
        axisLine={{ stroke: chartColors.gridLine }}
        tickLine={{ stroke: chartColors.gridLine }}
        tick={{ fill: chartColors.axisText, fontSize: 12 }}
      />
      <YAxis yAxisId="count" hide />
      <YAxis
        yAxisId="cost"
        orientation="right"
        tickFormatter={(value) => currencyFormatter.format(value as number)}
        width={90}
        axisLine={{ stroke: chartColors.gridLine }}
        tickLine={{ stroke: chartColors.gridLine }}
        tick={{ fill: chartColors.axisText, fontSize: 12 }}
      />
    </>
  );
}

function TimeseriesTooltip({
  chartColors,
  currencyFormatter,
  numberFormatter,
  seriesNames,
}: Pick<
  TimeseriesChartViewProps,
  "chartColors" | "currencyFormatter" | "numberFormatter" | "seriesNames"
>) {
  const formatValue = (value: number, key: keyof typeof seriesNames) =>
    key === "totalCost" ? currencyFormatter.format(value) : numberFormatter.format(value);

  return (
    <Tooltip
      formatter={(value, key) => [
        formatValue(value as number, key as keyof typeof seriesNames),
        seriesNames[key as keyof typeof seriesNames],
      ]}
      contentStyle={{
        backgroundColor: chartColors.tooltipBg,
        borderColor: chartColors.tooltipBorder,
        borderRadius: 10,
      }}
      labelStyle={{ color: chartColors.axisText, fontWeight: 600 }}
      itemStyle={{ color: chartColors.axisText }}
    />
  );
}

function TimeseriesAreaChart({
  chartData,
  chartColors,
  numberFormatter,
  currencyFormatter,
  seriesNames,
  animate,
}: Omit<TimeseriesChartViewProps, "chartMode">) {
  return (
    <AreaChart data={chartData} margin={{ top: 16, right: 32, left: 0, bottom: 8 }}>
      <TimeseriesAxes
        chartColors={chartColors}
        numberFormatter={numberFormatter}
        currencyFormatter={currencyFormatter}
      />
      <TimeseriesTooltip
        chartColors={chartColors}
        currencyFormatter={currencyFormatter}
        numberFormatter={numberFormatter}
        seriesNames={seriesNames}
      />
      <Legend wrapperStyle={{ color: chartColors.axisText }} />
      <Area
        type="monotone"
        dataKey="totalTokens"
        name={seriesNames.totalTokens}
        yAxisId="tokens"
        stroke={chartColors.tokenColor}
        fill={chartColors.tokenFill}
        fillOpacity={1}
        strokeWidth={2}
        isAnimationActive={animate}
      />
      <Area
        type="monotone"
        dataKey="totalCount"
        name={seriesNames.totalCount}
        yAxisId="count"
        stroke={chartColors.countColor}
        fill={chartColors.countFill}
        fillOpacity={1}
        strokeWidth={2}
        isAnimationActive={animate}
      />
      <Area
        type="monotone"
        dataKey="totalCost"
        name={seriesNames.totalCost}
        yAxisId="cost"
        stroke={chartColors.costColor}
        fill={chartColors.costFill}
        fillOpacity={1}
        strokeWidth={2}
        isAnimationActive={animate}
      />
    </AreaChart>
  );
}

function TimeseriesBarChart({
  chartData,
  chartColors,
  numberFormatter,
  currencyFormatter,
  seriesNames,
  animate,
}: Omit<TimeseriesChartViewProps, "chartMode">) {
  return (
    <ComposedChart data={chartData} margin={{ top: 16, right: 32, left: 0, bottom: 8 }}>
      <TimeseriesAxes
        chartColors={chartColors}
        numberFormatter={numberFormatter}
        currencyFormatter={currencyFormatter}
      />
      <TimeseriesTooltip
        chartColors={chartColors}
        currencyFormatter={currencyFormatter}
        numberFormatter={numberFormatter}
        seriesNames={seriesNames}
      />
      <Legend wrapperStyle={{ color: chartColors.axisText }} />
      <Bar
        yAxisId="tokens"
        dataKey="totalTokens"
        name={seriesNames.totalTokens}
        fill={chartColors.tokenColor}
        radius={[4, 4, 0, 0]}
        isAnimationActive={animate}
      />
      <Bar
        yAxisId="count"
        dataKey="totalCount"
        name={seriesNames.totalCount}
        fill={chartColors.countColor}
        radius={[4, 4, 0, 0]}
        isAnimationActive={animate}
      />
      <Bar
        yAxisId="cost"
        dataKey="totalCost"
        name={seriesNames.totalCost}
        fill={chartColors.costColor}
        radius={[4, 4, 0, 0]}
        isAnimationActive={animate}
      />
    </ComposedChart>
  );
}

function TimeseriesChartView({
  chartMode,
  chartData,
  chartColors,
  numberFormatter,
  currencyFormatter,
  seriesNames,
  animate,
}: TimeseriesChartViewProps) {
  return (
    <div
      className="h-96 w-full"
      data-chart-kind="stats-timeseries-trend"
      data-chart-mode={chartMode}
    >
      <ResponsiveContainer>
        {chartMode === "cumulative-area" ? (
          <TimeseriesAreaChart
            chartData={chartData}
            chartColors={chartColors}
            numberFormatter={numberFormatter}
            currencyFormatter={currencyFormatter}
            seriesNames={seriesNames}
            animate={animate}
          />
        ) : (
          <TimeseriesBarChart
            chartData={chartData}
            chartColors={chartColors}
            numberFormatter={numberFormatter}
            currencyFormatter={currencyFormatter}
            seriesNames={seriesNames}
            animate={animate}
          />
        )}
      </ResponsiveContainer>
    </div>
  );
}

export function TimeseriesChart({
  points,
  isLoading,
  bucketSeconds,
  showDate = true,
}: TimeseriesChartProps) {
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
    const tokenColor = metricAccent("totalTokens", themeMode);
    const countColor = metricAccent("totalCount", themeMode);
    const costColor = metricAccent("totalCost", themeMode);
    return {
      ...base,
      tokenColor,
      tokenFill: withOpacity(tokenColor, 0.22),
      countColor,
      countFill: withOpacity(countColor, 0.22),
      costColor,
      costFill: withOpacity(costColor, 0.22),
    };
  }, [themeMode]);

  if (isLoading) {
    return (
      <div className="flex justify-center py-10">
        <Spinner size="lg" aria-label={t("chart.loadingDetailed")} />
      </div>
    );
  }

  if (points.length === 0) {
    return <Alert>{t("chart.noDataRange")}</Alert>;
  }

  const chartMode = resolveTimeseriesChartMode(points.length);
  const chartData = buildTimeseriesChartData(points, bucketSeconds, showDate);

  // Keep animations for normal point counts; auto-disable only for extreme cases to avoid UI lockups
  const animate = chartData.length <= 800;

  const seriesNames = {
    totalTokens: t("chart.totalTokens"),
    totalCost: t("chart.totalCost"),
    totalCount: t("chart.totalCount"),
  };

  return (
    <TimeseriesChartView
      chartMode={chartMode}
      chartData={chartData}
      chartColors={chartColors}
      numberFormatter={numberFormatter}
      currencyFormatter={currencyFormatter}
      seriesNames={seriesNames}
      animate={animate}
    />
  );
}
