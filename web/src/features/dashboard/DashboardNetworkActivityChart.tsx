import { useMemo } from "react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  Legend,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { Alert } from "../../components/ui/alert";
import { Spinner } from "../../components/ui/spinner";
import { useTranslation } from "../../i18n";
import type { DashboardNetworkTimeseriesResponse } from "../../lib/api";
import { chartBaseTokens, withOpacity } from "../../lib/chartTheme";
import { useTheme } from "../../theme";
import {
  formatDashboardNetworkBytes,
  formatDashboardNetworkSpeed,
} from "./dashboardNetworkFormatting";

type ChartDatum = DashboardNetworkTimeseriesResponse["points"][number] & {
  chartLabel: string;
  chartTooltipLabel: string;
};

interface DashboardNetworkActivityChartProps {
  response?: DashboardNetworkTimeseriesResponse | null;
  loading: boolean;
  error?: string | null;
}

function formatBucketLabel(date: Date, showDate: boolean, localeTag: string) {
  return new Intl.DateTimeFormat(localeTag, {
    month: showDate ? "numeric" : undefined,
    day: showDate ? "numeric" : undefined,
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

export function DashboardNetworkActivityChart({
  response,
  loading,
  error,
}: DashboardNetworkActivityChartProps) {
  const { t, locale } = useTranslation();
  const { themeMode } = useTheme();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const chartColors = useMemo(() => {
    const base = chartBaseTokens(themeMode);
    return {
      ...base,
      upload: themeMode === "dark" ? "#38bdf8" : "#0284c7",
      download: themeMode === "dark" ? "#34d399" : "#059669",
    };
  }, [themeMode]);

  if (loading && response == null) {
    return (
      <div className="flex min-h-[23rem] items-center justify-center rounded-2xl border border-base-300/60 bg-base-100/55">
        <Spinner size="lg" aria-label={t("chart.loadingDetailed")} />
      </div>
    );
  }

  if (error) {
    return <Alert>{error}</Alert>;
  }

  if (!response || response.points.length === 0) {
    return <Alert>{t("chart.noDataRange")}</Alert>;
  }

  const rangeStart = new Date(response.rangeStart);
  const rangeEnd = new Date(response.rangeEnd);
  const showDate =
    response.range === "1d" ||
    rangeStart.toLocaleDateString(localeTag) !== rangeEnd.toLocaleDateString(localeTag);
  const chartData: ChartDatum[] = response.points.map((point) => {
    const bucketStart = new Date(point.bucketStart);
    const bucketEnd = new Date(point.bucketEnd);
    return {
      ...point,
      chartLabel: formatBucketLabel(bucketStart, showDate, localeTag),
      chartTooltipLabel: `${formatBucketLabel(bucketStart, showDate, localeTag)} - ${formatBucketLabel(
        bucketEnd,
        showDate,
        localeTag,
      )}`,
    };
  });

  return (
    <div
      data-testid="dashboard-network-activity-chart"
      className="overflow-hidden rounded-2xl border border-base-300/60 bg-base-100/55 px-2 py-3 sm:px-4"
    >
      <div className="mb-3 flex items-center justify-between gap-3 px-1">
        <div className="text-sm font-medium text-base-content/74">
          {t("dashboard.activityOverview.networkLiveNote")}
        </div>
        {loading ? (
          <div className="flex items-center gap-2 text-xs text-base-content/55">
            <Spinner size="sm" aria-label={t("chart.loadingDetailed")} />
            <span>{t("dashboard.activityOverview.networkRefreshing")}</span>
          </div>
        ) : null}
      </div>
      <div className="h-[22rem] w-full">
        <ResponsiveContainer width="100%" height="100%">
          <AreaChart data={chartData} margin={{ top: 12, right: 20, left: 0, bottom: 4 }}>
            <defs>
              <linearGradient id="dashboard-network-upload-fill" x1="0" x2="0" y1="0" y2="1">
                <stop offset="0%" stopColor={withOpacity(chartColors.upload, 0.38)} />
                <stop offset="100%" stopColor={withOpacity(chartColors.upload, 0.04)} />
              </linearGradient>
              <linearGradient id="dashboard-network-download-fill" x1="0" x2="0" y1="0" y2="1">
                <stop offset="0%" stopColor={withOpacity(chartColors.download, 0.34)} />
                <stop offset="100%" stopColor={withOpacity(chartColors.download, 0.04)} />
              </linearGradient>
            </defs>
            <CartesianGrid stroke={chartColors.gridLine} strokeDasharray="3 3" />
            <XAxis
              dataKey="chartLabel"
              axisLine={{ stroke: chartColors.gridLine }}
              tickLine={{ stroke: chartColors.gridLine }}
              tick={{ fill: chartColors.axisText, fontSize: 12 }}
              minTickGap={24}
            />
            <YAxis
              axisLine={{ stroke: chartColors.gridLine }}
              tickLine={{ stroke: chartColors.gridLine }}
              tick={{ fill: chartColors.axisText, fontSize: 12 }}
              tickFormatter={(value) => formatDashboardNetworkSpeed(Number(value), localeTag)}
              width={72}
            />
            <Tooltip
              formatter={(value, key, item) => {
                const label =
                  key === "uploadBytesPerSecond"
                    ? t("dashboard.activityOverview.networkUpload")
                    : t("dashboard.activityOverview.networkDownload");
                const payload = item.payload as ChartDatum;
                const numericValue =
                  typeof value === "number" ? value : typeof value === "string" ? Number(value) : 0;
                return [
                  `${formatDashboardNetworkSpeed(numericValue, localeTag)} · ${formatDashboardNetworkBytes(
                    key === "uploadBytesPerSecond" ? payload.uploadBytes : payload.downloadBytes,
                    localeTag,
                  )}`,
                  label,
                ];
              }}
              labelFormatter={(_, payload) => {
                const point = payload?.[0]?.payload as ChartDatum | undefined;
                return point?.chartTooltipLabel ?? "";
              }}
              contentStyle={{
                backgroundColor: chartColors.tooltipBg,
                borderColor: chartColors.tooltipBorder,
                borderRadius: 16,
              }}
              labelStyle={{ color: chartColors.axisText, fontWeight: 600 }}
              itemStyle={{ color: chartColors.axisText }}
            />
            <Legend wrapperStyle={{ color: chartColors.axisText }} />
            <Area
              type="monotone"
              dataKey="uploadBytesPerSecond"
              name={t("dashboard.activityOverview.networkUpload")}
              stroke={chartColors.upload}
              fill="url(#dashboard-network-upload-fill)"
              strokeWidth={2.25}
              dot={false}
              activeDot={{ r: 3 }}
            />
            <Area
              type="monotone"
              dataKey="downloadBytesPerSecond"
              name={t("dashboard.activityOverview.networkDownload")}
              stroke={chartColors.download}
              fill="url(#dashboard-network-download-fill)"
              strokeWidth={2.25}
              dot={false}
              activeDot={{ r: 3 }}
            />
          </AreaChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}

export default DashboardNetworkActivityChart;
