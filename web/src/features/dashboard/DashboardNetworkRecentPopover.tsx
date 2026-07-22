import {
  type KeyboardEvent,
  type MouseEvent,
  type PointerEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { Alert } from "../../components/ui/alert";
import { BubblePopoverContent } from "../../components/ui/bubble-popover";
import {
  Dialog,
  DialogCloseIcon,
  DialogContent,
  DialogDescription,
  DialogTitle,
} from "../../components/ui/dialog";
import { floatingSurfaceStyle } from "../../components/ui/floating-surface";
import { Popover, PopoverTrigger } from "../../components/ui/popover";
import { Spinner } from "../../components/ui/spinner";
import { useCompactViewport } from "../../hooks/useCompactViewport";
import { useDashboardRecentNetworkWindow } from "../../hooks/useDashboardRecentNetworkWindow";
import { useTranslation } from "../../i18n";
import type { DashboardRecentNetworkWindowResponse } from "../../lib/api";
import { chartBaseTokens, withOpacity } from "../../lib/chartTheme";
import { cn } from "../../lib/utils";
import { useTheme } from "../../theme";
import { AppIcon } from "../shared/AppIcon";
import {
  formatDashboardNetworkBytes,
  formatDashboardNetworkSpeed,
} from "./dashboardNetworkFormatting";

type RecentChartDatum = DashboardRecentNetworkWindowResponse["points"][number] & {
  chartTimestamp: number;
  chartLabel: string;
  chartTooltipLabel: string;
  uploadChartValue: number | null;
  downloadChartValue: number | null;
};

type DashboardRecentTooltipPayloadEntry = {
  dataKey?: string | number;
  payload?: RecentChartDatum;
  value?: number | string | null;
};

type DashboardRecentTooltipRow = {
  seriesKey: "uploadChartValue" | "downloadChartValue";
  label: string;
  value: string;
  iconName: "arrow-up-bold" | "arrow-down-bold";
};

const HOVER_CLOSE_DELAY_MS = 120;

function formatRecentBucketLabel(date: Date, localeTag: string) {
  return new Intl.DateTimeFormat(localeTag, {
    hour: "2-digit",
    minute: "2-digit",
  }).format(date);
}

function formatRecentElapsedTick(timestamp: number, rangeStartTimestamp: number) {
  const elapsedSeconds = Math.max(0, Math.round((timestamp - rangeStartTimestamp) / 1000));
  const minutes = Math.floor(elapsedSeconds / 60);
  const seconds = elapsedSeconds % 60;
  return `${minutes.toString().padStart(2, "0")}:${seconds.toString().padStart(2, "0")}`;
}

function formatRecentTooltipLabel(start: Date, end: Date, localeTag: string) {
  const formatter = new Intl.DateTimeFormat(localeTag, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
  return `${formatter.format(start)} - ${formatter.format(end)}`;
}

function buildRecentTooltipRows(
  payload: readonly DashboardRecentTooltipPayloadEntry[] | undefined,
  localeTag: string,
  t: (key: string) => string,
): DashboardRecentTooltipRow[] {
  const rows: DashboardRecentTooltipRow[] = [];
  for (const seriesKey of ["uploadChartValue", "downloadChartValue"] as const) {
    const entry = payload?.find((candidate) => candidate.dataKey === seriesKey);
    const point = entry?.payload;
    if (!point?.isAvailable) {
      continue;
    }
    const isUploadSeries = seriesKey === "uploadChartValue";
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

export function DashboardNetworkRecentPanel({
  response,
  loading,
  stale = false,
  error,
  className,
}: {
  response?: DashboardRecentNetworkWindowResponse | null;
  loading: boolean;
  stale?: boolean;
  error?: string | null;
  className?: string;
}) {
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
        "linear-gradient(180deg, oklch(var(--color-base-100) / 0.97) 0%, oklch(var(--color-base-100) / 0.91) 100%)",
      boxShadow:
        "inset 0 1px 0 color-mix(in oklab, oklch(var(--color-base-content)) 8%, transparent), 0 18px 40px color-mix(in oklab, oklch(var(--color-base-content)) 16%, transparent)",
    }),
    [],
  );

  if (loading && response == null) {
    return (
      <div
        data-testid="dashboard-network-recent-panel"
        data-theme={surfaceTheme}
        style={panelStyle}
        className={cn(
          "flex min-h-[24rem] items-center justify-center rounded-[1.4rem] border border-base-300/60",
          className,
        )}
      >
        <Spinner size="lg" aria-label={t("dashboard.networkRecent.loading")} />
      </div>
    );
  }

  if (error) {
    return (
      <div
        data-testid="dashboard-network-recent-panel"
        data-theme={surfaceTheme}
        style={panelStyle}
        className={cn("rounded-[1.4rem] border border-base-300/60 p-4", className)}
      >
        <Alert variant="error">{error}</Alert>
      </div>
    );
  }

  if (!response || response.points.length === 0) {
    return (
      <div
        data-testid="dashboard-network-recent-panel"
        data-theme={surfaceTheme}
        style={panelStyle}
        className={cn("rounded-[1.4rem] border border-base-300/60 p-4", className)}
      >
        <Alert>{t("dashboard.networkRecent.empty")}</Alert>
      </div>
    );
  }

  let latestAvailablePoint: DashboardRecentNetworkWindowResponse["points"][number] | null = null;
  for (let index = response.points.length - 1; index >= 0; index -= 1) {
    const point = response.points[index];
    if (point?.isAvailable) {
      latestAvailablePoint = point;
      break;
    }
  }
  const currentUploadSpeed = formatDashboardNetworkSpeed(
    latestAvailablePoint?.uploadBytesPerSecond ?? 0,
    localeTag,
  );
  const currentDownloadSpeed = formatDashboardNetworkSpeed(
    latestAvailablePoint?.downloadBytesPerSecond ?? 0,
    localeTag,
  );
  const chartData: RecentChartDatum[] = response.points.map((point) => {
    const sampleStart = new Date(point.sampleStart);
    const sampleEnd = new Date(point.sampleEnd);
    return {
      ...point,
      chartTimestamp: sampleStart.getTime(),
      chartLabel: formatRecentBucketLabel(sampleStart, localeTag),
      chartTooltipLabel: formatRecentTooltipLabel(sampleStart, sampleEnd, localeTag),
      uploadChartValue: point.isAvailable ? point.uploadBytesPerSecond : null,
      downloadChartValue: point.isAvailable ? point.downloadBytesPerSecond : null,
    };
  });
  const rangeStartTimestamp = new Date(response.rangeStart).getTime();
  const lastPointTimestamp = chartData.at(-1)?.chartTimestamp ?? rangeStartTimestamp;
  const chartTickValues: number[] = [];
  for (let offsetSeconds = 0; offsetSeconds < response.windowSeconds; offsetSeconds += 60) {
    chartTickValues.push(rangeStartTimestamp + offsetSeconds * 1000);
  }
  if (chartTickValues.at(-1) !== lastPointTimestamp) {
    chartTickValues.push(lastPointTimestamp);
  }
  const uniqueChartTickValues = chartTickValues.filter(
    (tick, index, all) => index === 0 || tick > (all[index - 1] ?? Number.NEGATIVE_INFINITY),
  );

  return (
    <div
      data-testid="dashboard-network-recent-panel"
      data-theme={surfaceTheme}
      style={panelStyle}
      className={cn("overflow-hidden rounded-[1.4rem] border border-base-300/60", className)}
    >
      <div className="flex flex-col gap-4 px-4 pb-4 pt-4 sm:px-5">
        <div className="flex flex-col gap-3 border-b border-base-300/55 pb-3 sm:flex-row sm:items-start sm:justify-between">
          <div className="min-w-0">
            <div className="flex items-center gap-2">
              <span className="inline-flex h-8 w-8 items-center justify-center rounded-full border border-info/25 bg-info/10 text-info">
                <AppIcon name="speedometer" className="h-4.5 w-4.5" aria-hidden />
              </span>
              <div className="min-w-0">
                <div className="text-base font-semibold tracking-tight text-base-content">
                  {t("dashboard.networkRecent.title")}
                </div>
                <div className="text-xs leading-5 text-base-content/64">
                  {t("dashboard.networkRecent.subtitle")}
                </div>
              </div>
            </div>
          </div>
          <div
            data-testid="dashboard-network-recent-current-speed"
            className="grid shrink-0 grid-cols-[auto_auto] gap-x-2 gap-y-1 pt-0.5 text-[0.74rem] leading-5 sm:justify-end"
            aria-live="polite"
          >
            <span
              className={cn("inline-flex items-center gap-1.5 font-medium text-base-content/68")}
            >
              <AppIcon
                name="arrow-up-bold"
                className={cn(
                  "h-3.5 w-3.5",
                  themeMode === "dark" ? "text-sky-300" : "text-sky-700",
                )}
                aria-hidden
              />
              {t("dashboard.activityOverview.networkUpload")}：
            </span>
            <span
              className={cn(
                "font-mono text-[0.82rem] font-bold tracking-tight tabular-nums",
                themeMode === "dark" ? "text-sky-300" : "text-sky-700",
              )}
            >
              {currentUploadSpeed}
            </span>
            <span
              className={cn("inline-flex items-center gap-1.5 font-medium text-base-content/68")}
            >
              <AppIcon
                name="arrow-down-bold"
                className={cn(
                  "h-3.5 w-3.5",
                  themeMode === "dark" ? "text-emerald-300" : "text-emerald-700",
                )}
                aria-hidden
              />
              {t("dashboard.activityOverview.networkDownload")}：
            </span>
            <span
              className={cn(
                "font-mono text-[0.82rem] font-bold tracking-tight tabular-nums",
                themeMode === "dark" ? "text-emerald-300" : "text-emerald-700",
              )}
            >
              {currentDownloadSpeed}
            </span>
          </div>
        </div>

        <div
          className="relative h-[22rem] w-full overflow-hidden rounded-2xl"
          data-testid="dashboard-network-recent-chart"
        >
          <ResponsiveContainer width="100%" height="100%">
            <AreaChart data={chartData} margin={{ top: 8, right: 18, left: 0, bottom: 4 }}>
              <defs>
                <linearGradient
                  id="dashboard-network-recent-upload-fill"
                  x1="0"
                  x2="0"
                  y1="0"
                  y2="1"
                >
                  <stop offset="0%" stopColor={withOpacity(chartColors.upload, 0.34)} />
                  <stop offset="100%" stopColor={withOpacity(chartColors.upload, 0.02)} />
                </linearGradient>
                <linearGradient
                  id="dashboard-network-recent-download-fill"
                  x1="0"
                  x2="0"
                  y1="0"
                  y2="1"
                >
                  <stop offset="0%" stopColor={withOpacity(chartColors.download, 0.3)} />
                  <stop offset="100%" stopColor={withOpacity(chartColors.download, 0.02)} />
                </linearGradient>
              </defs>
              <CartesianGrid stroke={chartColors.gridLine} strokeDasharray="3 3" />
              <XAxis
                type="number"
                dataKey="chartTimestamp"
                scale="time"
                domain={["dataMin", "dataMax"]}
                ticks={uniqueChartTickValues}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
                tickFormatter={(value) =>
                  formatRecentElapsedTick(Number(value), rangeStartTimestamp)
                }
                minTickGap={44}
              />
              <YAxis
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
                tickFormatter={(value) => formatDashboardNetworkSpeed(Number(value), localeTag)}
                width={74}
              />
              <Tooltip
                content={({ active, payload }) => {
                  const point = payload?.[0]?.payload as RecentChartDatum | undefined;
                  if (!active || !point) {
                    return null;
                  }
                  if (!point.isAvailable) {
                    return null;
                  }
                  const rows = buildRecentTooltipRows(
                    payload as readonly DashboardRecentTooltipPayloadEntry[] | undefined,
                    localeTag,
                    t,
                  );
                  return (
                    <div
                      data-testid="dashboard-network-recent-tooltip"
                      data-theme={surfaceTheme}
                      style={floatingSurfaceStyle("neutral", surfaceTheme)}
                      className="min-w-[13rem] rounded-2xl border px-3.5 py-3 text-base-content"
                    >
                      <div className="text-sm font-semibold tracking-tight text-base-content/82">
                        {point.chartTooltipLabel}
                      </div>
                      <div className="mt-2.5 space-y-2">
                        {rows.map((row) => (
                          <div key={row.seriesKey} className="flex items-center gap-2.5">
                            <span
                              className={cn(
                                "inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-full border",
                                row.seriesKey === "uploadChartValue"
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
                                  row.seriesKey === "uploadChartValue"
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
              />
              <Area
                type="monotone"
                dataKey="uploadChartValue"
                stroke={chartColors.upload}
                fill="url(#dashboard-network-recent-upload-fill)"
                strokeWidth={2}
                dot={false}
                activeDot={{ r: 4 }}
                isAnimationActive={false}
                connectNulls={false}
              />
              <Area
                type="monotone"
                dataKey="downloadChartValue"
                stroke={chartColors.download}
                fill="url(#dashboard-network-recent-download-fill)"
                strokeWidth={2}
                dot={false}
                activeDot={{ r: 4 }}
                isAnimationActive={false}
                connectNulls={false}
              />
            </AreaChart>
          </ResponsiveContainer>
          {stale ? (
            <div
              data-testid="dashboard-network-recent-stale-overlay"
              className="absolute inset-0 z-10 flex flex-col items-center justify-center gap-3 bg-base-100/62 text-base-content shadow-[inset_0_0_0_1px_color-mix(in_oklab,oklch(var(--color-base-content))_8%,transparent)] backdrop-blur-[2px]"
            >
              <Spinner size="lg" aria-label={t("dashboard.networkRecent.staleLoading")} />
              <div className="rounded-full border border-base-300/70 bg-base-100/82 px-3 py-1.5 text-xs font-semibold tracking-wide text-base-content/78">
                {t("dashboard.networkRecent.staleLoading")}
              </div>
            </div>
          ) : null}
        </div>
      </div>
    </div>
  );
}

export function DashboardNetworkRecentPopover({
  trigger,
  triggerAriaLabel,
  triggerClassName,
}: {
  trigger: ReactNode;
  triggerAriaLabel: string;
  triggerClassName?: string;
}) {
  const { t } = useTranslation();
  const isCompactViewport = useCompactViewport();
  const closeTimerRef = useRef<number | null>(null);
  const [hoverOpen, setHoverOpen] = useState(false);
  const [lockedOpen, setLockedOpen] = useState(false);
  const [compactOpen, setCompactOpen] = useState(false);
  const panelOpen = isCompactViewport ? compactOpen : hoverOpen || lockedOpen;
  const { data, isLoading, isStale, error } = useDashboardRecentNetworkWindow(panelOpen);

  const clearCloseTimer = useCallback(() => {
    if (closeTimerRef.current != null) {
      window.clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
  }, []);

  const closeDesktop = useCallback(() => {
    clearCloseTimer();
    setHoverOpen(false);
    setLockedOpen(false);
  }, [clearCloseTimer]);

  const scheduleDesktopClose = useCallback(() => {
    clearCloseTimer();
    closeTimerRef.current = window.setTimeout(() => {
      setHoverOpen(false);
      if (!lockedOpen) {
        setLockedOpen(false);
      }
      closeTimerRef.current = null;
    }, HOVER_CLOSE_DELAY_MS);
  }, [clearCloseTimer, lockedOpen]);

  useEffect(() => () => clearCloseTimer(), [clearCloseTimer]);

  useEffect(() => {
    if (isCompactViewport) {
      closeDesktop();
      return;
    }
    setCompactOpen(false);
  }, [closeDesktop, isCompactViewport]);

  const handleDesktopTriggerMouseEnter = useCallback(() => {
    clearCloseTimer();
    setHoverOpen(true);
  }, [clearCloseTimer]);

  const handleDesktopTriggerMouseLeave = useCallback(() => {
    if (lockedOpen) {
      return;
    }
    scheduleDesktopClose();
  }, [lockedOpen, scheduleDesktopClose]);

  const handleDesktopTriggerClick = useCallback(
    (event: MouseEvent<HTMLButtonElement>) => {
      event.preventDefault();
      clearCloseTimer();
      setHoverOpen(true);
      setLockedOpen((current) => !current);
    },
    [clearCloseTimer],
  );

  const handleDesktopTriggerKeyDown = useCallback(
    (event: KeyboardEvent<HTMLButtonElement>) => {
      if (event.key !== "Enter" && event.key !== " ") {
        return;
      }
      event.preventDefault();
      clearCloseTimer();
      setHoverOpen(true);
      setLockedOpen((current) => !current);
    },
    [clearCloseTimer],
  );

  const handleCompactTriggerClick = useCallback(() => {
    setCompactOpen(true);
  }, []);

  const handleContentPointerEnter = useCallback(
    (_event: PointerEvent<HTMLElement>) => {
      clearCloseTimer();
      setHoverOpen(true);
    },
    [clearCloseTimer],
  );

  const handleContentPointerLeave = useCallback(() => {
    if (lockedOpen) {
      return;
    }
    scheduleDesktopClose();
  }, [lockedOpen, scheduleDesktopClose]);

  const triggerButton = (
    <button
      type="button"
      className={cn(
        "inline-flex min-w-0 rounded-full bg-transparent text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100",
        triggerClassName,
      )}
      aria-label={triggerAriaLabel}
      aria-haspopup="dialog"
      aria-expanded={panelOpen}
      data-testid="dashboard-network-recent-trigger"
      onMouseEnter={!isCompactViewport ? handleDesktopTriggerMouseEnter : undefined}
      onMouseLeave={!isCompactViewport ? handleDesktopTriggerMouseLeave : undefined}
      onClick={isCompactViewport ? handleCompactTriggerClick : handleDesktopTriggerClick}
      onKeyDown={!isCompactViewport ? handleDesktopTriggerKeyDown : undefined}
    >
      {trigger}
    </button>
  );

  if (isCompactViewport) {
    return (
      <>
        {triggerButton}
        <Dialog open={compactOpen} onOpenChange={setCompactOpen}>
          <DialogContent
            className="max-h-[min(100dvh-0.5rem,100dvh)] overflow-hidden"
            data-testid="dashboard-network-recent-dialog"
          >
            <div className="flex items-start gap-3 border-b border-base-300/70 px-4 py-4 sm:px-5">
              <div className="min-w-0 flex-1">
                <DialogTitle className="min-w-0 text-lg">
                  {t("dashboard.networkRecent.title")}
                </DialogTitle>
                <DialogDescription className="mt-1 text-sm leading-6 text-base-content/68">
                  {t("dashboard.networkRecent.subtitle")}
                </DialogDescription>
              </div>
              <DialogCloseIcon aria-label={t("dashboard.networkRecent.close")} />
            </div>
            <div className="max-h-[calc(min(100dvh-0.5rem,100dvh)-5.5rem)] overflow-y-auto px-4 py-4 sm:px-5">
              <DashboardNetworkRecentPanel
                response={data}
                loading={isLoading}
                stale={isStale}
                error={error}
              />
            </div>
          </DialogContent>
        </Dialog>
      </>
    );
  }

  return (
    <Popover
      open={panelOpen}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) {
          closeDesktop();
        }
      }}
    >
      <PopoverTrigger asChild>{triggerButton}</PopoverTrigger>
      <BubblePopoverContent
        align="end"
        side="bottom"
        sideOffset={10}
        className="w-[min(52rem,calc(100vw-1rem))] max-w-[min(52rem,calc(100vw-1rem))] border-none bg-transparent p-0 shadow-none"
        onPointerEnter={handleContentPointerEnter}
        onPointerLeave={handleContentPointerLeave}
        data-testid="dashboard-network-recent-popover"
      >
        <DashboardNetworkRecentPanel
          response={data}
          loading={isLoading}
          stale={isStale}
          error={error}
        />
      </BubblePopoverContent>
    </Popover>
  );
}
