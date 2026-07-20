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
import { floatingSurfaceStyle } from "../../components/ui/floating-surface";
import { Spinner } from "../../components/ui/spinner";
import { useTranslation } from "../../i18n";
import type { DashboardNetworkTimeseriesResponse } from "../../lib/api";
import { chartBaseTokens, withOpacity } from "../../lib/chartTheme";
import { cn } from "../../lib/utils";
import { useTheme } from "../../theme";
import { AppIcon, type AppIconName } from "../shared/AppIcon";
import {
  formatDashboardNetworkBytes,
  formatDashboardNetworkSpeed,
} from "./dashboardNetworkFormatting";

type ChartDatum = DashboardNetworkTimeseriesResponse["points"][number] & {
  chartLabel: string;
  chartTooltipLabel: string;
};

type DashboardNetworkTooltipRow = {
  seriesKey: "uploadBytesPerSecond" | "downloadBytesPerSecond";
  iconName: AppIconName;
  label: string;
  value: string;
};

type DashboardNetworkTooltipPayloadEntry = {
  dataKey?: string | number;
  value?: number | string;
  payload?: ChartDatum;
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

export function buildDashboardNetworkTooltipRows(
  payload: readonly DashboardNetworkTooltipPayloadEntry[] | undefined,
  localeTag: string,
  t: (key: string) => string,
): DashboardNetworkTooltipRow[] {
  const rows: DashboardNetworkTooltipRow[] = [];
  for (const seriesKey of ["uploadBytesPerSecond", "downloadBytesPerSecond"] as const) {
    const entry = payload?.find((candidate) => candidate.dataKey === seriesKey);
    const point = entry?.payload;
    if (!point) continue;
    const isUploadSeries = seriesKey === "uploadBytesPerSecond";
    const rawValue = entry?.value;
    const numericValue =
      typeof rawValue === "number"
        ? rawValue
        : typeof rawValue === "string"
          ? Number(rawValue)
          : isUploadSeries
            ? point.uploadBytesPerSecond
            : point.downloadBytesPerSecond;
    rows.push({
      seriesKey,
      iconName: isUploadSeries ? "arrow-up-bold" : "arrow-down-bold",
      label: isUploadSeries
        ? t("dashboard.activityOverview.networkUpload")
        : t("dashboard.activityOverview.networkDownload"),
      value: `${formatDashboardNetworkSpeed(numericValue, localeTag)} · ${formatDashboardNetworkBytes(
        isUploadSeries ? point.uploadBytes : point.downloadBytes,
        localeTag,
      )}`,
    });
  }
  return rows;
}

export function DashboardNetworkActivityChart({
  response,
  loading,
  error,
}: DashboardNetworkActivityChartProps) {
  const { t, locale } = useTranslation();
  const { themeMode } = useTheme();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const surfaceTheme = themeMode === "dark" ? "vibe-dark" : "vibe-light";
  const chartColors = useMemo(() => {
    const base = chartBaseTokens(themeMode);
    return {
      ...base,
      upload: themeMode === "dark" ? "#38bdf8" : "#0284c7",
      download: themeMode === "dark" ? "#34d399" : "#059669",
    };
  }, [themeMode]);
  const panelStyle = useMemo(
    () => ({
      background:
        "linear-gradient(180deg, oklch(var(--color-base-100) / 0.96) 0%, oklch(var(--color-base-100) / 0.9) 100%)",
      boxShadow:
        "inset 0 1px 0 color-mix(in oklab, oklch(var(--color-base-content)) 8%, transparent), 0 18px 36px color-mix(in oklab, oklch(var(--color-base-content)) 14%, transparent)",
    }),
    [],
  );

  if (loading && response == null) {
    return (
      <div
        data-theme={surfaceTheme}
        style={panelStyle}
        className="flex min-h-[23rem] items-center justify-center rounded-2xl border border-base-300/60"
      >
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
      data-theme={surfaceTheme}
      style={panelStyle}
      className="overflow-hidden rounded-2xl border border-base-300/60 px-2 py-3 sm:px-4"
    >
      {loading ? (
        <div className="mb-3 flex items-center justify-end px-1 text-xs text-base-content/55">
          <div className="flex items-center gap-2">
            <Spinner size="sm" aria-label={t("chart.loadingDetailed")} />
            <span>{t("dashboard.activityOverview.networkRefreshing")}</span>
          </div>
        </div>
      ) : null}
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
              content={({ active, payload }) => {
                const rows = buildDashboardNetworkTooltipRows(
                  payload as readonly DashboardNetworkTooltipPayloadEntry[] | undefined,
                  localeTag,
                  t,
                );
                const point = payload?.[0]?.payload as ChartDatum | undefined;
                if (!active || !point || rows.length === 0) {
                  return null;
                }
                return (
                  <div
                    data-testid="dashboard-network-activity-tooltip"
                    data-theme={surfaceTheme}
                    style={floatingSurfaceStyle("neutral", surfaceTheme)}
                    className="min-w-[13rem] rounded-2xl border px-3.5 py-3 text-base-content"
                  >
                    <div className="text-sm font-semibold tracking-tight text-base-content/80">
                      {point.chartTooltipLabel}
                    </div>
                    <div className="mt-2.5 space-y-2">
                      {rows.map((row) => (
                        <div key={row.seriesKey} className="flex items-center gap-2.5">
                          <span
                            className={cn(
                              "inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-full border",
                              row.seriesKey === "uploadBytesPerSecond"
                                ? themeMode === "dark"
                                  ? "border-sky-300/25 bg-sky-400/12 text-sky-200"
                                  : "border-sky-500/20 bg-sky-500/10 text-sky-700"
                                : themeMode === "dark"
                                  ? "border-emerald-300/25 bg-emerald-400/12 text-emerald-200"
                                  : "border-emerald-500/20 bg-emerald-500/10 text-emerald-700",
                            )}
                            aria-hidden="true"
                          >
                            <AppIcon name={row.iconName} className="h-4 w-4" />
                          </span>
                          <div className="min-w-0">
                            <div className="text-[11px] font-medium tracking-[0.08em] text-base-content/58">
                              {row.label}
                            </div>
                            <div
                              className={cn(
                                "mt-0.5 font-mono text-[13px] font-semibold tracking-tight",
                                row.seriesKey === "uploadBytesPerSecond"
                                  ? themeMode === "dark"
                                    ? "text-sky-200"
                                    : "text-sky-700"
                                  : themeMode === "dark"
                                    ? "text-emerald-200"
                                    : "text-emerald-700",
                              )}
                            >
                              {row.value}
                            </div>
                          </div>
                        </div>
                      ))}
                    </div>
                  </div>
                );
              }}
              wrapperStyle={{ outline: "none" }}
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
