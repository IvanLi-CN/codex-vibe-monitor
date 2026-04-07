import { useId, useMemo, useState } from "react";
import type {
  ParallelWorkStatsResponse,
  ParallelWorkWindowResponse,
} from "../lib/api";
import { useTranslation } from "../i18n";
import { Alert } from "./ui/alert";
import {
  InlineChartTooltipSurface,
  type InlineChartTooltipData,
} from "./ui/inline-chart-tooltip";
import { InfoTooltip } from "./ui/info-tooltip";
import { SegmentedControl, SegmentedControlItem } from "./ui/segmented-control";

interface ParallelWorkStatsSectionProps {
  stats: ParallelWorkStatsResponse | null;
  isLoading: boolean;
  error: string | null;
  defaultWindowKey?: ParallelWorkWindowKey;
}

export type ParallelWorkWindowKey = "minute7d" | "hour30d" | "dayAll";

const WINDOW_KEYS: ParallelWorkWindowKey[] = ["minute7d", "hour30d", "dayAll"];

const CHART_WIDTH = 640;
const CHART_HEIGHT = 176;
const CHART_MARGIN_LEFT = 42;
const CHART_MARGIN_RIGHT = 16;
const CHART_MARGIN_TOP = 14;
const CHART_MARGIN_BOTTOM = 28;
const CHART_PLOT_WIDTH = CHART_WIDTH - CHART_MARGIN_LEFT - CHART_MARGIN_RIGHT;
const CHART_PLOT_HEIGHT = CHART_HEIGHT - CHART_MARGIN_TOP - CHART_MARGIN_BOTTOM;

type ParallelWorkChartPoint = ParallelWorkWindowResponse["points"][number] & {
  x: number;
  y: number;
  hitStartX: number;
  hitWidth: number;
};

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

function buildSparklineGeometry(
  points: ParallelWorkWindowResponse["points"],
  scaleMaxCount: number,
) {
  const baselineY = CHART_HEIGHT - CHART_MARGIN_BOTTOM;

  if (points.length === 0) {
    return {
      linePath: "",
      areaPath: "",
      baselineY,
      chartPoints: [] as ParallelWorkChartPoint[],
    };
  }

  const projectedPoints = points.map((point, index) => {
    const x =
      points.length === 1
        ? CHART_MARGIN_LEFT + CHART_PLOT_WIDTH / 2
        : CHART_MARGIN_LEFT + (CHART_PLOT_WIDTH * index) / (points.length - 1);
    const ratio = scaleMaxCount <= 0 ? 0 : point.parallelCount / scaleMaxCount;
    const y = baselineY - ratio * CHART_PLOT_HEIGHT;
    return { ...point, x, y };
  });
  const chartPoints = projectedPoints.map((point, index) => {
    const previousX = projectedPoints[index - 1]?.x ?? CHART_MARGIN_LEFT;
    const nextX =
      projectedPoints[index + 1]?.x ?? CHART_WIDTH - CHART_MARGIN_RIGHT;
    const hitStartX =
      index === 0 ? CHART_MARGIN_LEFT : (previousX + point.x) / 2;
    const hitEndX =
      index === projectedPoints.length - 1
        ? CHART_WIDTH - CHART_MARGIN_RIGHT
        : (point.x + nextX) / 2;

    return {
      ...point,
      hitStartX,
      hitWidth: Math.max(hitEndX - hitStartX, 12),
    };
  });

  const linePath = chartPoints
    .map(
      (coord, index) =>
        (index === 0 ? "M " : "L ") +
        coord.x.toFixed(2) +
        " " +
        coord.y.toFixed(2),
    )
    .join(" ");
  const areaPath =
    chartPoints.length === 1
      ? linePath +
        " L " +
        chartPoints[0].x.toFixed(2) +
        " " +
        baselineY.toFixed(2) +
        " Z"
      : linePath +
        " L " +
        chartPoints[chartPoints.length - 1].x.toFixed(2) +
        " " +
        baselineY.toFixed(2) +
        " L " +
        chartPoints[0].x.toFixed(2) +
        " " +
        baselineY.toFixed(2) +
        " Z";

  return {
    linePath,
    areaPath,
    baselineY,
    chartPoints,
  };
}

function buildParallelWorkYAxisTicks(
  scaleMaxCount: number,
  localeTag: string,
): Array<{ value: number; y: number; label: string }> {
  const formatter = new Intl.NumberFormat(localeTag, {
    maximumFractionDigits: 0,
  });
  const values = Array.from(
    new Set(
      [0, Math.ceil(scaleMaxCount / 2), scaleMaxCount].filter(
        (value) => value >= 0,
      ),
    ),
  ).sort((left, right) => left - right);

  return values.reverse().map((value) => ({
    value,
    y:
      scaleMaxCount <= 0
        ? CHART_HEIGHT - CHART_MARGIN_BOTTOM
        : CHART_HEIGHT -
          CHART_MARGIN_BOTTOM -
          (value / scaleMaxCount) * CHART_PLOT_HEIGHT,
    label: formatter.format(value),
  }));
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

function buildParallelWorkXAxisTicks(
  window: ParallelWorkWindowResponse,
  chartPoints: ParallelWorkChartPoint[],
  localeTag: string,
) {
  if (window.points.length === 0 || chartPoints.length === 0) return [];
  const effectiveTimeZone = window.effectiveTimeZone ?? "Asia/Shanghai";
  const candidateIndexes = Array.from(
    new Set([0, Math.floor((window.points.length - 1) / 2), window.points.length - 1]),
  );
  const years = new Set(
    window.points.map((point) => new Date(point.bucketStart).getFullYear()),
  );
  const showYear = years.size > 1;
  const baseLabels = candidateIndexes.map((index) =>
    formatParallelWorkAxisBucketLabel(
      window.points[index]?.bucketStart ?? "",
      localeTag,
      showYear,
      false,
      effectiveTimeZone,
    ),
  );
  const useDetailedLabels =
    new Set(baseLabels).size !== baseLabels.length && window.bucketSeconds < 86_400;

  return candidateIndexes.map((index) => ({
    anchor:
      index === 0
        ? ("start" as const)
        : index === window.points.length - 1
          ? ("end" as const)
          : ("middle" as const),
    x: chartPoints[index]?.x ?? CHART_MARGIN_LEFT,
    label: formatParallelWorkAxisBucketLabel(
      window.points[index]?.bucketStart ?? "",
      localeTag,
      showYear,
      useDetailedLabels,
      effectiveTimeZone,
    ),
  }));
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

function buildParallelWorkTooltipData(
  window: ParallelWorkWindowResponse,
  localeTag: string,
  countLabel: string,
  numberFormatter: Intl.NumberFormat,
) {
  const effectiveTimeZone = window.effectiveTimeZone ?? "Asia/Shanghai";
  return window.points.map<InlineChartTooltipData>((point) => ({
    title: formatParallelWorkBucketRange(
      point.bucketStart,
      point.bucketEnd,
      window.bucketSeconds,
      localeTag,
      effectiveTimeZone,
    ),
    rows: [
      {
        label: countLabel,
        value: numberFormatter.format(point.parallelCount),
        tone: "accent",
      },
    ],
  }));
}

function resolveParallelWorkDefaultIndex(
  points: ParallelWorkWindowResponse["points"],
) {
  for (let index = points.length - 1; index >= 0; index -= 1) {
    if ((points[index]?.parallelCount ?? 0) > 0) return index;
  }
  return Math.max(0, points.length - 1);
}

function buildWindowDetailsTooltipContent(
  title: string,
  description: string,
  samples?: string | null,
  fallbackNote?: string | null,
) {
  return [title.trim(), description.trim(), samples?.trim(), fallbackNote?.trim()]
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

function ParallelWorkWindowToggle({
  activeWindowKey,
  onWindowSelect,
}: {
  activeWindowKey: ParallelWorkWindowKey;
  onWindowSelect: (windowKey: ParallelWorkWindowKey) => void;
}) {
  const { t } = useTranslation();

  return (
    <div className="flex justify-end">
      <SegmentedControl
        size="compact"
        className="min-w-max"
        role="tablist"
        aria-label={t("stats.parallelWork.windowToggleAria")}
        data-testid="parallel-work-window-toggle"
      >
        {WINDOW_KEYS.map((windowKey) => {
          const meta = resolveWindowMeta(windowKey);
          const active = windowKey === activeWindowKey;
          return (
            <SegmentedControlItem
              key={windowKey}
              active={active}
              role="tab"
              aria-selected={active}
              onClick={() => onWindowSelect(windowKey)}
              data-testid={"parallel-work-window-trigger-" + windowKey}
            >
              {t(meta.toggleLabelKey)}
            </SegmentedControlItem>
          );
        })}
      </SegmentedControl>
    </div>
  );
}

function ParallelWorkSparkline({
  window,
  emptyLabel,
  ariaLabel,
  interactionHint,
  tooltipCountLabel,
}: {
  window: ParallelWorkWindowResponse;
  emptyLabel: string;
  ariaLabel: string;
  interactionHint: string;
  tooltipCountLabel: string;
}) {
  const { locale } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag),
    [localeTag],
  );
  const tooltipData = useMemo(
    () =>
      buildParallelWorkTooltipData(
        window,
        localeTag,
        tooltipCountLabel,
        numberFormatter,
      ),
    [localeTag, numberFormatter, tooltipCountLabel, window],
  );
  const defaultIndex = useMemo(
    () => resolveParallelWorkDefaultIndex(window.points),
    [window.points],
  );
  const scaleMaxCount = useMemo(
    () =>
      Math.max(
        window.maxCount ?? 0,
        ...window.points.map((point) => point.parallelCount),
        0,
      ),
    [window.maxCount, window.points],
  );
  const { linePath, areaPath, baselineY, chartPoints } = useMemo(
    () => buildSparklineGeometry(window.points, scaleMaxCount),
    [scaleMaxCount, window.points],
  );
  const yAxisTicks = useMemo(
    () => buildParallelWorkYAxisTicks(scaleMaxCount, localeTag),
    [localeTag, scaleMaxCount],
  );
  const xAxisTicks = useMemo(
    () => buildParallelWorkXAxisTicks(window, chartPoints, localeTag),
    [chartPoints, localeTag, window],
  );
  const gradientId = useId().replace(/:/g, "");

  if (window.points.length === 0) {
    return (
      <div className="flex h-32 items-center justify-center rounded-2xl border border-dashed border-base-300/75 bg-base-200/30 text-sm text-base-content/55">
        {emptyLabel}
      </div>
    );
  }

  return (
    <InlineChartTooltipSurface
      items={tooltipData}
      defaultIndex={defaultIndex}
      ariaLabel={ariaLabel}
      interactionHint={interactionHint}
      className="w-full py-0.5"
      chartClassName="w-full"
    >
      {({ highlightedIndex, getItemProps }) => {
        const activePoint =
          highlightedIndex != null
            ? (chartPoints[highlightedIndex] ?? null)
            : null;

        return (
          <svg
            viewBox={"0 0 " + CHART_WIDTH + " " + CHART_HEIGHT}
            className="h-44 w-full rounded-2xl border border-base-300/75 bg-base-100/75"
            preserveAspectRatio="none"
            aria-hidden="true"
            data-chart-kind="parallel-work-sparkline"
          >
            <defs>
              <linearGradient id={gradientId} x1="0" x2="0" y1="0" y2="1">
                <stop
                  offset="0%"
                  stopColor="oklch(var(--color-primary) / 0.22)"
                />
                <stop
                  offset="100%"
                  stopColor="oklch(var(--color-primary) / 0.03)"
                />
              </linearGradient>
            </defs>
            {yAxisTicks.map((tick) => (
              <g key={"y-" + tick.value}>
                <line
                  x1={CHART_MARGIN_LEFT}
                  y1={tick.y}
                  x2={CHART_WIDTH - CHART_MARGIN_RIGHT}
                  y2={tick.y}
                  stroke="oklch(var(--color-base-content) / 0.12)"
                  strokeWidth="1"
                  strokeDasharray={tick.value === 0 ? undefined : "4 4"}
                />
                <text
                  x={CHART_MARGIN_LEFT - 8}
                  y={tick.y}
                  dy="0.32em"
                  textAnchor="end"
                  fontSize="11"
                  fill="oklch(var(--color-base-content) / 0.5)"
                  data-axis="y-tick"
                >
                  {tick.label}
                </text>
              </g>
            ))}
            {activePoint ? (
              <line
                x1={activePoint.x}
                y1={CHART_MARGIN_TOP}
                x2={activePoint.x}
                y2={baselineY}
                stroke="oklch(var(--color-primary) / 0.4)"
                strokeWidth="1.4"
                strokeDasharray="5 4"
              />
            ) : null}
            <path
              d={areaPath}
              fill={"url(#" + gradientId + ")"}
              stroke="none"
            />
            <path
              d={linePath}
              fill="none"
              stroke="oklch(var(--color-primary))"
              strokeWidth="3"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
            {chartPoints.map((point, index) => {
              const isActive = highlightedIndex === index;
              const itemProps = getItemProps(index);
              const { ref, onClick, onMouseEnter, ...restItemProps } =
                itemProps;
              return (
                <g key={point.bucketStart + "-" + point.bucketEnd}>
                  <circle
                    cx={point.x}
                    cy={point.y}
                    r={isActive ? "4.5" : "3"}
                    fill={
                      isActive
                        ? "oklch(var(--color-primary))"
                        : "oklch(var(--color-primary) / 0.82)"
                    }
                    stroke="oklch(var(--color-base-100) / 0.96)"
                    strokeWidth={isActive ? "1.8" : "1.25"}
                  />
                  <rect
                    ref={ref}
                    x={point.hitStartX}
                    y={CHART_MARGIN_TOP}
                    width={point.hitWidth}
                    height={CHART_PLOT_HEIGHT}
                    fill="transparent"
                    className="cursor-pointer"
                    {...restItemProps}
                    onClick={(event) => {
                      onClick();
                      onMouseEnter(event as never);
                    }}
                  />
                </g>
              );
            })}
            {xAxisTicks.map((tick) => (
              <g key={"x-" + tick.x + "-" + tick.label}>
                <line
                  x1={tick.x}
                  y1={baselineY}
                  x2={tick.x}
                  y2={baselineY + 5}
                  stroke="oklch(var(--color-base-content) / 0.18)"
                  strokeWidth="1"
                />
                <text
                  x={tick.x}
                  y={CHART_HEIGHT - 8}
                  textAnchor={tick.anchor}
                  fontSize="11"
                  fill="oklch(var(--color-base-content) / 0.5)"
                  data-axis="x-tick"
                >
                  {tick.label}
                </text>
              </g>
            ))}
          </svg>
        );
      }}
    </InlineChartTooltipSurface>
  );
}

function ParallelWorkWindowCard({
  windowKey,
  window,
}: {
  windowKey: ParallelWorkWindowKey;
  window: ParallelWorkWindowResponse;
}) {
  const { t, locale } = useTranslation();
  const meta = resolveWindowMeta(windowKey);
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

      <ParallelWorkSparkline
        window={window}
        emptyLabel={t("stats.parallelWork.empty")}
        ariaLabel={t("stats.parallelWork.chartAria", {
          title: t(meta.titleKey),
        })}
        interactionHint={t("live.chart.tooltip.instructions")}
        tooltipCountLabel={t("stats.parallelWork.tooltip.parallelCount")}
      />

      {empty ? (
        <div className="space-y-2">
          <p className="rounded-2xl border border-dashed border-base-300/75 bg-base-200/20 px-3 py-2 text-sm text-base-content/58">
            {t("stats.parallelWork.empty")}
          </p>
          {timeZoneFallbackNote ? (
            <p className="text-xs text-base-content/50">{timeZoneFallbackNote}</p>
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
}: ParallelWorkStatsSectionProps) {
  const { t } = useTranslation();
  const [activeWindowKey, setActiveWindowKey] =
    useState<ParallelWorkWindowKey>(defaultWindowKey);
  const activeWindow = stats?.[activeWindowKey] ?? null;
  const activeMeta = resolveWindowMeta(activeWindowKey);
  const activeTitle = t(activeMeta.titleKey);
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
    t(activeMeta.descriptionKey),
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
          <div
            className="flex justify-start sm:justify-end"
            data-testid={"parallel-work-controls-" + activeWindowKey}
          >
            <ParallelWorkWindowToggle
              activeWindowKey={activeWindowKey}
              onWindowSelect={setActiveWindowKey}
            />
          </div>
        </div>
        {error ? (
          <ParallelWorkErrorCard windowKey={activeWindowKey} error={error} />
        ) : isLoading || !activeWindow ? (
          <ParallelWorkLoadingCard windowKey={activeWindowKey} />
        ) : (
          <ParallelWorkWindowCard windowKey={activeWindowKey} window={activeWindow} />
        )}
      </div>
    </section>
  );
}
