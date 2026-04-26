import { useMemo } from "react";
import type {
  ParallelWorkStatsResponse,
  ParallelWorkWindowResponse,
} from "../lib/api";
import { useTranslation } from "../i18n";
import {
  Bar,
  BarChart,
  CartesianGrid,
  ResponsiveContainer,
  Scatter,
  ScatterChart,
  Tooltip,
  XAxis,
  YAxis,
  ZAxis,
} from "recharts";
import { chartBaseTokens, metricAccent, withOpacity } from "../lib/chartTheme";
import { useTheme } from "../theme";
import { Alert } from "./ui/alert";
import { InfoTooltip } from "./ui/info-tooltip";

interface ParallelWorkStatsSectionProps {
  stats: ParallelWorkStatsResponse | null;
  isLoading: boolean;
  error: string | null;
  defaultWindowKey?: ParallelWorkWindowKey;
  rangeLabel?: string;
  bucketLabel?: string;
}

export type ParallelWorkWindowKey = "minute7d" | "hour30d" | "dayAll";

const PARALLEL_WORK_CHART_HEIGHT = 320;

interface ParallelWorkGanttDatum {
  conversationIndex: number;
  conversationLabel: string;
  conversationKey: string;
  timeEpoch: number;
  bucketEndEpoch: number;
  bucketStart: string;
  bucketEnd: string;
  requestCount: number;
}

interface ParallelWorkTrendDatum {
  timeEpoch: number;
  bucketStart: string;
  bucketEnd: string;
  parallelCount: number;
}

function resolveWindowMeta(key: ParallelWorkWindowKey) {
  switch (key) {
    case "minute7d":
      return {
        titleKey: "stats.parallelWork.windows.minute7d.title",
        descriptionKey: "stats.parallelWork.windows.minute7d.description",
        toggleLabelKey: "stats.parallelWork.windows.minute7d.toggleLabel",
      };
    case "hour30d":
      return {
        titleKey: "stats.parallelWork.windows.hour30d.title",
        descriptionKey: "stats.parallelWork.windows.hour30d.description",
        toggleLabelKey: "stats.parallelWork.windows.hour30d.toggleLabel",
      };
    case "dayAll":
      return {
        titleKey: "stats.parallelWork.windows.dayAll.title",
        descriptionKey: "stats.parallelWork.windows.dayAll.description",
        toggleLabelKey: "stats.parallelWork.windows.dayAll.toggleLabel",
      };
  }
}

function formatParallelWorkAxisBucketLabel(
  raw: string,
  localeTag: string,
  showYear: boolean,
  detailed: boolean,
  timeZone: string,
) {
  const value = new Date(raw);
  if (Number.isNaN(value.getTime())) return raw;
  const formatter = new Intl.DateTimeFormat(localeTag, {
    timeZone,
    year: showYear ? "2-digit" : undefined,
    month: "2-digit",
    day: "2-digit",
    hour: detailed ? "2-digit" : undefined,
    minute: detailed ? "2-digit" : undefined,
    hour12: false,
  });
  return formatter.format(value);
}

function toEpoch(raw: string) {
  const value = new Date(raw).getTime();
  return Number.isFinite(value) ? value : null;
}

function buildParallelWorkGanttData(
  window: ParallelWorkWindowResponse,
): ParallelWorkGanttDatum[] {
  return window.conversations.flatMap((conversation, conversationIndex) =>
    conversation.segments.flatMap((segment) => {
      const timeEpoch = toEpoch(segment.bucketStart);
      const bucketEndEpoch = toEpoch(segment.bucketEnd);
      if (timeEpoch == null || bucketEndEpoch == null) return [];
      return [
        {
          conversationIndex,
          conversationLabel: conversation.label,
          conversationKey: conversation.conversationKey,
          timeEpoch,
          bucketEndEpoch,
          bucketStart: segment.bucketStart,
          bucketEnd: segment.bucketEnd,
          requestCount: segment.requestCount,
        },
      ];
    }),
  );
}

function buildParallelWorkTrendData(
  window: ParallelWorkWindowResponse,
): ParallelWorkTrendDatum[] {
  return window.points.flatMap((point) => {
    const timeEpoch = toEpoch(point.bucketStart);
    if (timeEpoch == null) return [];
    return [
      {
        timeEpoch,
        bucketStart: point.bucketStart,
        bucketEnd: point.bucketEnd,
        parallelCount: point.parallelCount,
      },
    ];
  });
}

function parallelWorkWindowIsLongerThanDay(window: ParallelWorkWindowResponse) {
  const start = toEpoch(window.rangeStart);
  const end = toEpoch(window.rangeEnd);
  return start != null && end != null && end - start > 86_400_000;
}

function buildParallelWorkTimeDomain(window: ParallelWorkWindowResponse) {
  const rangeStart = toEpoch(window.rangeStart);
  const rangeEnd = toEpoch(window.rangeEnd);
  if (rangeStart != null && rangeEnd != null && rangeStart < rangeEnd) {
    return [rangeStart, rangeEnd] as [number, number];
  }
  const epochs = window.conversations.flatMap((conversation) =>
    conversation.segments.flatMap((segment) => {
      const start = toEpoch(segment.bucketStart);
      const end = toEpoch(segment.bucketEnd);
      return [start, end].filter((value): value is number => value != null);
    }),
  );
  if (epochs.length === 0) return [0, 1] as [number, number];
  return [Math.min(...epochs), Math.max(...epochs)] as [number, number];
}

function formatAverageCount(value: number | null, locale: string) {
  if (value == null) return "—";
  const formatter = new Intl.NumberFormat(locale, {
    minimumFractionDigits: Number.isInteger(value) ? 0 : 2,
    maximumFractionDigits: 2,
  });
  return formatter.format(value);
}

function formatWholeCount(value: number | null, locale: string) {
  if (value == null) return "—";
  return new Intl.NumberFormat(locale, { maximumFractionDigits: 0 }).format(
    value,
  );
}

function formatParallelWorkBucketRange(
  startRaw: string,
  endRaw: string,
  bucketSeconds: number,
  localeTag: string,
  timeZone: string,
) {
  const start = new Date(startRaw);
  const end = new Date(endRaw);
  if (Number.isNaN(start.getTime()) || Number.isNaN(end.getTime())) {
    return startRaw + " → " + endRaw;
  }

  const formatter = new Intl.DateTimeFormat(localeTag, {
    timeZone,
    year: bucketSeconds >= 86_400 ? "numeric" : undefined,
    month: "2-digit",
    day: "2-digit",
    hour: bucketSeconds >= 86_400 ? undefined : "2-digit",
    minute: bucketSeconds >= 3_600 ? undefined : "2-digit",
    hour12: false,
  });

  return formatter.format(start) + " → " + formatter.format(end);
}

interface ParallelWorkGanttCellShapeProps {
  cx?: number;
  cy?: number;
  payload?: ParallelWorkGanttDatum;
  xAxis?: {
    scale?: (value: number) => number;
  };
}

interface ParallelWorkTooltipPayloadEntry {
  payload?: ParallelWorkGanttDatum;
}

interface ParallelWorkRechartsTooltipContentProps {
  active?: boolean;
  payload?: readonly ParallelWorkTooltipPayloadEntry[];
  bucketSeconds: number;
  countLabel: string;
  conversationLabel: string;
  localeTag: string;
  numberFormatter: Intl.NumberFormat;
  theme: {
    axisText: string;
    tooltipBg: string;
    tooltipBorder: string;
    accent: string;
  };
  timeZone: string;
}

function ParallelWorkRechartsTooltipContent({
  active,
  payload,
  bucketSeconds,
  countLabel,
  conversationLabel,
  localeTag,
  numberFormatter,
  theme,
  timeZone,
}: ParallelWorkRechartsTooltipContentProps) {
  const datum = payload?.find((entry) => entry.payload)?.payload;
  if (!active || !datum) return null;

  return (
    <div
      className="min-w-[13rem] rounded-lg border px-3 py-2 text-xs shadow-lg"
      style={{
        backgroundColor: theme.tooltipBg,
        borderColor: theme.tooltipBorder,
        color: theme.axisText,
      }}
      data-testid="parallel-work-chart-tooltip"
    >
      <div className="mb-2 text-sm font-semibold">
        {datum.conversationLabel}
      </div>
      <div className="space-y-1.5">
        <div>
          {formatParallelWorkBucketRange(
            datum.bucketStart,
            datum.bucketEnd,
            bucketSeconds,
            localeTag,
            timeZone,
          )}
        </div>
        <div className="flex items-center justify-between gap-4">
          <span className="flex items-center gap-2">
            <span
              className="inline-block h-2.5 w-2.5 rounded-sm"
              style={{ backgroundColor: theme.accent }}
              aria-hidden="true"
            />
            <span>{countLabel}</span>
          </span>
          <span className="font-semibold">
            {numberFormatter.format(datum.requestCount)}
          </span>
        </div>
        <div className="text-base-content/55">{conversationLabel}</div>
      </div>
    </div>
  );
}

interface ParallelWorkTrendTooltipPayloadEntry {
  payload?: ParallelWorkTrendDatum;
}

function ParallelWorkTrendTooltipContent({
  active,
  payload,
  bucketSeconds,
  countLabel,
  localeTag,
  numberFormatter,
  theme,
  timeZone,
}: {
  active?: boolean;
  payload?: readonly ParallelWorkTrendTooltipPayloadEntry[];
  bucketSeconds: number;
  countLabel: string;
  localeTag: string;
  numberFormatter: Intl.NumberFormat;
  theme: {
    axisText: string;
    tooltipBg: string;
    tooltipBorder: string;
    accent: string;
  };
  timeZone: string;
}) {
  const datum = payload?.find((entry) => entry.payload)?.payload;
  if (!active || !datum) return null;
  return (
    <div
      className="min-w-[13rem] rounded-lg border px-3 py-2 text-xs shadow-lg"
      style={{
        backgroundColor: theme.tooltipBg,
        borderColor: theme.tooltipBorder,
        color: theme.axisText,
      }}
      data-testid="parallel-work-chart-tooltip"
    >
      <div className="mb-2 text-sm font-semibold">
        {formatParallelWorkBucketRange(
          datum.bucketStart,
          datum.bucketEnd,
          bucketSeconds,
          localeTag,
          timeZone,
        )}
      </div>
      <div className="flex items-center justify-between gap-4">
        <span className="flex items-center gap-2">
          <span
            className="inline-block h-2.5 w-2.5 rounded-sm"
            style={{ backgroundColor: theme.accent }}
            aria-hidden="true"
          />
          <span>{countLabel}</span>
        </span>
        <span className="font-semibold">
          {numberFormatter.format(datum.parallelCount)}
        </span>
      </div>
    </div>
  );
}

function buildWindowDetailsTooltipContent(
  title: string,
  description: string,
  samples?: string | null,
  fallbackNote?: string | null,
) {
  return [
    title.trim(),
    description.trim(),
    samples?.trim(),
    fallbackNote?.trim(),
  ]
    .filter(Boolean)
    .join(" · ");
}

function ParallelWorkWindowInfoTrigger({
  tooltipContent,
  tooltipLabel,
}: {
  tooltipContent: string;
  tooltipLabel: string;
}) {
  return (
    <div className="flex items-center">
      <InfoTooltip
        content={tooltipContent}
        label={tooltipLabel}
        className="shrink-0 text-base-content/46 transition-colors hover:text-base-content/70"
      />
    </div>
  );
}

function ParallelWorkChart({
  window,
  emptyLabel,
  ariaLabel,
  tooltipCountLabel,
  tooltipConversationLabel,
}: {
  window: ParallelWorkWindowResponse;
  emptyLabel: string;
  ariaLabel: string;
  tooltipCountLabel: string;
  tooltipConversationLabel: string;
}) {
  const { locale } = useTranslation();
  const { themeMode } = useTheme();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const effectiveTimeZone = window.effectiveTimeZone ?? "Asia/Shanghai";
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag),
    [localeTag],
  );
  const chartData = useMemo(() => buildParallelWorkGanttData(window), [window]);
  const trendData = useMemo(() => buildParallelWorkTrendData(window), [window]);
  const useTrendChart = parallelWorkWindowIsLongerThanDay(window);
  const timeDomain = useMemo(
    () => buildParallelWorkTimeDomain(window),
    [window],
  );
  const chartColors = useMemo(() => {
    const base = chartBaseTokens(themeMode);
    const accent = metricAccent("totalCount", themeMode);
    return {
      ...base,
      accent,
      accentFill: withOpacity(accent, 0.82),
    };
  }, [themeMode]);
  const conversationCount = window.conversations.length;
  const chartHeight = Math.max(
    PARALLEL_WORK_CHART_HEIGHT,
    Math.min(520, 220 + conversationCount * 2),
  );
  const maxRequestCount = Math.max(
    1,
    ...chartData.map((datum) => datum.requestCount),
  );
  const formatTimeTick = (value: number | string) =>
    formatParallelWorkAxisBucketLabel(
      new Date(Number(value)).toISOString(),
      localeTag,
      false,
      window.bucketSeconds < 86_400,
      effectiveTimeZone,
    );

  if (
    window.points.length === 0 ||
    (!useTrendChart && chartData.length === 0)
  ) {
    return (
      <div className="flex h-32 items-center justify-center rounded-2xl border border-dashed border-base-300/75 bg-base-200/30 text-sm text-base-content/55">
        {emptyLabel}
      </div>
    );
  }

  return (
    <div
      className="w-full rounded-2xl border border-base-300/75 bg-base-100/75 py-2"
      style={{ height: chartHeight }}
      role="img"
      aria-label={ariaLabel}
      data-chart-kind={
        useTrendChart ? "parallel-work-trend" : "parallel-work-gantt"
      }
      data-chart-library="recharts"
    >
      <ResponsiveContainer
        width="100%"
        height="100%"
        minWidth={0}
        minHeight={240}
        initialDimension={{ width: 960, height: chartHeight }}
      >
        {useTrendChart ? (
          <BarChart
            data={trendData}
            margin={{ top: 16, right: 24, left: 6, bottom: 28 }}
          >
            <CartesianGrid
              stroke={chartColors.gridLine}
              strokeDasharray="4 4"
            />
            <XAxis
              dataKey="timeEpoch"
              type="number"
              domain={timeDomain}
              tickFormatter={formatTimeTick}
              minTickGap={24}
              axisLine={{ stroke: chartColors.gridLine }}
              tickLine={{ stroke: chartColors.gridLine }}
              tick={{ fill: chartColors.axisText, fontSize: 11 }}
            />
            <YAxis
              dataKey="parallelCount"
              type="number"
              allowDecimals={false}
              width={48}
              axisLine={{ stroke: chartColors.gridLine }}
              tickLine={{ stroke: chartColors.gridLine }}
              tick={{ fill: chartColors.axisText, fontSize: 11 }}
            />
            <Tooltip
              cursor={{ fill: withOpacity(chartColors.accent, 0.12) }}
              content={(props) => (
                <ParallelWorkTrendTooltipContent
                  {...props}
                  bucketSeconds={window.bucketSeconds}
                  countLabel={tooltipCountLabel}
                  localeTag={localeTag}
                  numberFormatter={numberFormatter}
                  theme={chartColors}
                  timeZone={effectiveTimeZone}
                />
              )}
            />
            <Bar
              dataKey="parallelCount"
              fill={chartColors.accent}
              fillOpacity={0.82}
              radius={[4, 4, 0, 0]}
              isAnimationActive={trendData.length <= 1_200}
              name={tooltipCountLabel}
            />
          </BarChart>
        ) : (
          <ScatterChart
            data={chartData}
            margin={{ top: 16, right: 24, left: 6, bottom: 36 }}
          >
            <CartesianGrid
              stroke={chartColors.gridLine}
              strokeDasharray="4 4"
            />
            <XAxis
              dataKey="timeEpoch"
              type="number"
              domain={timeDomain}
              tickFormatter={formatTimeTick}
              minTickGap={24}
              axisLine={{ stroke: chartColors.gridLine }}
              tickLine={{ stroke: chartColors.gridLine }}
              tick={{ fill: chartColors.axisText, fontSize: 11 }}
              height={46}
            />
            <YAxis
              dataKey="conversationIndex"
              type="number"
              domain={[-0.5, Math.max(0.5, conversationCount - 0.5)]}
              allowDecimals={false}
              interval={0}
              ticks={window.conversations.map((_, index) => index)}
              tickFormatter={(value) =>
                window.conversations[Number(value)]?.label ?? String(value)
              }
              reversed
              width={48}
              axisLine={{ stroke: chartColors.gridLine }}
              tickLine={{ stroke: chartColors.gridLine }}
              tick={{ fill: chartColors.axisText, fontSize: 11 }}
            />
            <ZAxis
              dataKey="requestCount"
              range={[70, 150]}
              domain={[1, maxRequestCount]}
            />
            <Tooltip
              cursor={{ stroke: chartColors.accent, strokeOpacity: 0.26 }}
              content={(props) => (
                <ParallelWorkRechartsTooltipContent
                  {...props}
                  bucketSeconds={window.bucketSeconds}
                  countLabel={tooltipCountLabel}
                  conversationLabel={tooltipConversationLabel}
                  localeTag={localeTag}
                  numberFormatter={numberFormatter}
                  theme={chartColors}
                  timeZone={effectiveTimeZone}
                />
              )}
            />
            <Scatter
              data={chartData}
              dataKey="conversationIndex"
              fill={chartColors.accentFill}
              shape={(props: ParallelWorkGanttCellShapeProps) => {
                if (props.cx == null || props.cy == null) return <g />;
                const xScale = props.xAxis?.scale;
                const xEnd =
                  typeof xScale === "function" && props.payload != null
                    ? xScale(props.payload.bucketEndEpoch)
                    : props.cx;
                const xLeft = Math.min(props.cx, xEnd);
                const width = Math.max(6, Math.abs(xEnd - props.cx));
                return (
                  <rect
                    x={xLeft}
                    y={props.cy - 5}
                    width={width}
                    height={10}
                    rx={3}
                    fill={chartColors.accent}
                    fillOpacity={0.82}
                  />
                );
              }}
              isAnimationActive={chartData.length <= 1_200}
              name={tooltipCountLabel}
            />
          </ScatterChart>
        )}
      </ResponsiveContainer>
    </div>
  );
}

function ParallelWorkWindowCard({
  windowKey,
  window,
  chartTitle,
}: {
  windowKey: ParallelWorkWindowKey;
  window: ParallelWorkWindowResponse;
  chartTitle: string;
}) {
  const { t, locale } = useTranslation();
  const empty = window.completeBucketCount === 0;
  const effectiveTimeZone = window.effectiveTimeZone ?? "Asia/Shanghai";
  const timeZoneFallbackNote = window.timeZoneFallback
    ? t("stats.parallelWork.timeZoneFallback", {
        timeZone: effectiveTimeZone,
      })
    : null;

  return (
    <article
      className="flex flex-col gap-4 rounded-[1.35rem] border border-base-300/75 bg-base-100/82 p-4 shadow-sm"
      data-testid={"parallel-work-card-" + windowKey}
    >
      <div className="grid grid-cols-3 gap-2.5">
        <div className="rounded-2xl border border-base-300/70 bg-base-200/35 px-3 py-2.5">
          <div className="text-[11px] font-medium uppercase tracking-[0.08em] text-base-content/50">
            {t("stats.parallelWork.metrics.min")}
          </div>
          <div className="mt-1 text-xl font-semibold text-base-content">
            {formatWholeCount(window.minCount, locale)}
          </div>
        </div>
        <div className="rounded-2xl border border-base-300/70 bg-base-200/35 px-3 py-2.5">
          <div className="text-[11px] font-medium uppercase tracking-[0.08em] text-base-content/50">
            {t("stats.parallelWork.metrics.max")}
          </div>
          <div className="mt-1 text-xl font-semibold text-base-content">
            {formatWholeCount(window.maxCount, locale)}
          </div>
        </div>
        <div className="rounded-2xl border border-base-300/70 bg-base-200/35 px-3 py-2.5">
          <div className="text-[11px] font-medium uppercase tracking-[0.08em] text-base-content/50">
            {t("stats.parallelWork.metrics.avg")}
          </div>
          <div className="mt-1 text-xl font-semibold text-primary">
            {formatAverageCount(window.avgCount, locale)}
          </div>
        </div>
      </div>

      <ParallelWorkChart
        window={window}
        emptyLabel={t("stats.parallelWork.empty")}
        ariaLabel={t("stats.parallelWork.chartAria", {
          title: chartTitle,
        })}
        tooltipCountLabel={t("stats.parallelWork.tooltip.requestCount")}
        tooltipConversationLabel={t("stats.parallelWork.tooltip.conversation")}
      />

      {empty ? (
        <div className="space-y-2">
          <p className="rounded-2xl border border-dashed border-base-300/75 bg-base-200/20 px-3 py-2 text-sm text-base-content/58">
            {t("stats.parallelWork.empty")}
          </p>
          {timeZoneFallbackNote ? (
            <p className="text-xs text-base-content/50">
              {timeZoneFallbackNote}
            </p>
          ) : null}
        </div>
      ) : (
        <div className="space-y-1.5 text-xs text-base-content/55">
          <div>
            {t("stats.parallelWork.rangeSummary", {
              start: window.rangeStart,
              end: window.rangeEnd,
            })}
          </div>
          {timeZoneFallbackNote ? (
            <div className="text-base-content/50">{timeZoneFallbackNote}</div>
          ) : null}
        </div>
      )}
    </article>
  );
}

function ParallelWorkLoadingCard({
  windowKey,
}: {
  windowKey: ParallelWorkWindowKey;
}) {
  const { t } = useTranslation();

  return (
    <article
      className="flex min-h-[18rem] flex-col gap-4 rounded-[1.35rem] border border-base-300/75 bg-base-100/82 p-4 shadow-sm"
      data-testid={"parallel-work-card-" + windowKey}
    >
      <div className="grid grid-cols-3 gap-2.5">
        {Array.from({ length: 3 }).map((_, index) => (
          <div
            key={index}
            className="rounded-2xl border border-base-300/70 bg-base-200/35 px-3 py-2.5"
          >
            <div className="h-3 w-10 animate-pulse rounded-full bg-base-300/60" />
            <div className="mt-2 h-7 w-12 animate-pulse rounded-full bg-base-300/60" />
          </div>
        ))}
      </div>
      <div className="flex h-32 items-center justify-center rounded-2xl border border-base-300/75 bg-base-100/75 p-2.5 text-sm text-base-content/55">
        {t("stats.parallelWork.loading")}
      </div>
      <div className="h-4 w-full animate-pulse rounded-full bg-base-300/60" />
    </article>
  );
}

function ParallelWorkErrorCard({
  windowKey,
  error,
}: {
  windowKey: ParallelWorkWindowKey;
  error: string;
}) {
  return (
    <article
      className="flex min-h-[14rem] flex-col gap-4 rounded-[1.35rem] border border-base-300/75 bg-base-100/82 p-4 shadow-sm"
      data-testid={"parallel-work-card-" + windowKey}
    >
      <Alert variant="error">{error}</Alert>
    </article>
  );
}

export function ParallelWorkStatsSection({
  stats,
  isLoading,
  error,
  defaultWindowKey = "minute7d",
  rangeLabel,
  bucketLabel,
}: ParallelWorkStatsSectionProps) {
  const { t } = useTranslation();
  const activeWindowKey = defaultWindowKey;
  const activeWindow = stats?.current ?? stats?.[activeWindowKey] ?? null;
  const activeMeta = resolveWindowMeta(activeWindowKey);
  const activeTitle =
    rangeLabel && bucketLabel
      ? `${rangeLabel} · ${bucketLabel}`
      : t(activeMeta.titleKey);
  const activeDescription =
    rangeLabel && bucketLabel
      ? t("stats.parallelWork.currentDescription")
      : t(activeMeta.descriptionKey);
  const activeSamples =
    activeWindow == null
      ? null
      : t("stats.parallelWork.samples", {
          complete: activeWindow.completeBucketCount,
          active: activeWindow.activeBucketCount,
        });
  const activeTimeZoneFallbackNote =
    activeWindow?.timeZoneFallback && activeWindow.effectiveTimeZone
      ? t("stats.parallelWork.timeZoneFallback", {
          timeZone: activeWindow.effectiveTimeZone,
        })
      : null;
  const activeTooltipContent = buildWindowDetailsTooltipContent(
    activeTitle,
    activeDescription,
    activeSamples,
    activeTimeZoneFallbackNote,
  );

  return (
    <section className="surface-panel" data-testid="parallel-work-section">
      <div className="surface-panel-body gap-4">
        <div className="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-start">
          <div className="section-heading min-w-0">
            <div
              className="flex items-center gap-2"
              data-testid={"parallel-work-heading-" + activeWindowKey}
            >
              <h3 className="section-title">{t("stats.parallelWork.title")}</h3>
              <ParallelWorkWindowInfoTrigger
                tooltipContent={activeTooltipContent}
                tooltipLabel={t("stats.parallelWork.detailsTooltipLabel", {
                  title: activeTitle,
                })}
              />
            </div>
            <p className="section-description">
              {t("stats.parallelWork.description")}
            </p>
          </div>
        </div>
        {error ? (
          <ParallelWorkErrorCard windowKey={activeWindowKey} error={error} />
        ) : isLoading || !activeWindow ? (
          <ParallelWorkLoadingCard windowKey={activeWindowKey} />
        ) : (
          <ParallelWorkWindowCard
            windowKey={activeWindowKey}
            window={activeWindow}
            chartTitle={activeTitle}
          />
        )}
      </div>
    </section>
  );
}
