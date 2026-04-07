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
const CHART_HEIGHT = 132;
const CHART_PADDING_X = 16;
const CHART_PADDING_Y = 14;

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

function buildSparklineGeometry(points: ParallelWorkWindowResponse["points"]) {
  const baselineY = CHART_HEIGHT - CHART_PADDING_Y;

  if (points.length === 0) {
    return {
      linePath: "",
      areaPath: "",
      baselineY,
      chartPoints: [] as ParallelWorkChartPoint[],
    };
  }

  const usableWidth = CHART_WIDTH - CHART_PADDING_X * 2;
  const usableHeight = CHART_HEIGHT - CHART_PADDING_Y * 2;
  const maxCount = Math.max(...points.map((point) => point.parallelCount), 0);
  const projectedPoints = points.map((point, index) => {
    const x =
      points.length === 1
        ? CHART_WIDTH / 2
        : CHART_PADDING_X + (usableWidth * index) / (points.length - 1);
    const ratio = maxCount <= 0 ? 0 : point.parallelCount / maxCount;
    const y = baselineY - ratio * usableHeight;
    return { ...point, x, y };
  });
  const chartPoints = projectedPoints.map((point, index) => {
    const previousX = projectedPoints[index - 1]?.x ?? CHART_PADDING_X;
    const nextX =
      projectedPoints[index + 1]?.x ?? CHART_WIDTH - CHART_PADDING_X;
    const hitStartX = index === 0 ? CHART_PADDING_X : (previousX + point.x) / 2;
    const hitEndX =
      index === projectedPoints.length - 1
        ? CHART_WIDTH - CHART_PADDING_X
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
) {
  const start = new Date(startRaw);
  const end = new Date(endRaw);
  if (Number.isNaN(start.getTime()) || Number.isNaN(end.getTime())) {
    return startRaw + " → " + endRaw;
  }

  const formatter = new Intl.DateTimeFormat(localeTag, {
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
  return window.points.map<InlineChartTooltipData>((point) => ({
    title: formatParallelWorkBucketRange(
      point.bucketStart,
      point.bucketEnd,
      window.bucketSeconds,
      localeTag,
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
) {
  return [title.trim(), description.trim(), samples?.trim()]
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
    <div className="flex min-h-8 items-center">
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
    <div className="flex w-full justify-end overflow-x-auto no-scrollbar sm:w-auto">
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
  const { linePath, areaPath, baselineY, chartPoints } = useMemo(
    () => buildSparklineGeometry(window.points),
    [window.points],
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
            className="h-32 w-full rounded-2xl border border-base-300/75 bg-base-100/75"
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
            <line
              x1={CHART_PADDING_X}
              y1={baselineY}
              x2={CHART_WIDTH - CHART_PADDING_X}
              y2={baselineY}
              stroke="oklch(var(--color-base-content) / 0.14)"
              strokeWidth="1"
            />
            {activePoint ? (
              <line
                x1={activePoint.x}
                y1={CHART_PADDING_Y}
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
                    y={0}
                    width={point.hitWidth}
                    height={CHART_HEIGHT}
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
          </svg>
        );
      }}
    </InlineChartTooltipSurface>
  );
}

function ParallelWorkWindowCard({
  activeWindowKey,
  onWindowSelect,
  windowKey,
  window,
}: {
  activeWindowKey: ParallelWorkWindowKey;
  onWindowSelect: (windowKey: ParallelWorkWindowKey) => void;
  windowKey: ParallelWorkWindowKey;
  window: ParallelWorkWindowResponse;
}) {
  const { t, locale } = useTranslation();
  const meta = resolveWindowMeta(windowKey);
  const empty = window.completeBucketCount === 0;
  const title = t(meta.titleKey);
  const samples = t("stats.parallelWork.samples", {
    complete: window.completeBucketCount,
    active: window.activeBucketCount,
  });
  const tooltipContent = buildWindowDetailsTooltipContent(
    title,
    t(meta.descriptionKey),
    samples,
  );

  return (
    <article
      className="flex min-h-[22rem] flex-col gap-4 rounded-[1.35rem] border border-base-300/75 bg-base-100/82 p-4 shadow-sm"
      data-testid={"parallel-work-card-" + windowKey}
    >
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="flex min-w-0 flex-1 items-center">
          <ParallelWorkWindowInfoTrigger
            tooltipContent={tooltipContent}
            tooltipLabel={t("stats.parallelWork.detailsTooltipLabel", {
              title,
            })}
          />
        </div>
        <div className="sm:pl-4">
          <ParallelWorkWindowToggle
            activeWindowKey={activeWindowKey}
            onWindowSelect={onWindowSelect}
          />
        </div>
      </div>

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
        <p className="rounded-2xl border border-dashed border-base-300/75 bg-base-200/20 px-3 py-2 text-sm text-base-content/58">
          {t("stats.parallelWork.empty")}
        </p>
      ) : (
        <div className="text-xs text-base-content/55">
          {t("stats.parallelWork.rangeSummary", {
            start: window.rangeStart,
            end: window.rangeEnd,
          })}
        </div>
      )}
    </article>
  );
}

function ParallelWorkLoadingCard({
  activeWindowKey,
  onWindowSelect,
  windowKey,
}: {
  activeWindowKey: ParallelWorkWindowKey;
  onWindowSelect: (windowKey: ParallelWorkWindowKey) => void;
  windowKey: ParallelWorkWindowKey;
}) {
  const { t } = useTranslation();
  const meta = resolveWindowMeta(windowKey);
  const title = t(meta.titleKey);
  const tooltipContent = buildWindowDetailsTooltipContent(
    title,
    t(meta.descriptionKey),
  );

  return (
    <article
      className="flex min-h-[22rem] flex-col gap-4 rounded-[1.35rem] border border-base-300/75 bg-base-100/82 p-4 shadow-sm"
      data-testid={"parallel-work-card-" + windowKey}
    >
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="flex min-w-0 flex-1 items-center">
          <ParallelWorkWindowInfoTrigger
            tooltipContent={tooltipContent}
            tooltipLabel={t("stats.parallelWork.detailsTooltipLabel", {
              title,
            })}
          />
        </div>
        <div className="sm:pl-4">
          <ParallelWorkWindowToggle
            activeWindowKey={activeWindowKey}
            onWindowSelect={onWindowSelect}
          />
        </div>
      </div>
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
  activeWindowKey,
  onWindowSelect,
  error,
}: {
  activeWindowKey: ParallelWorkWindowKey;
  onWindowSelect: (windowKey: ParallelWorkWindowKey) => void;
  error: string;
}) {
  const { t } = useTranslation();
  const meta = resolveWindowMeta(activeWindowKey);
  const title = t(meta.titleKey);
  const tooltipContent = buildWindowDetailsTooltipContent(
    title,
    t(meta.descriptionKey),
  );

  return (
    <article
      className="flex min-h-[14rem] flex-col gap-4 rounded-[1.35rem] border border-base-300/75 bg-base-100/82 p-4 shadow-sm"
      data-testid={"parallel-work-card-" + activeWindowKey}
    >
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="flex min-w-0 flex-1 items-center">
          <ParallelWorkWindowInfoTrigger
            tooltipContent={tooltipContent}
            tooltipLabel={t("stats.parallelWork.detailsTooltipLabel", {
              title,
            })}
          />
        </div>
        <div className="sm:pl-4">
          <ParallelWorkWindowToggle
            activeWindowKey={activeWindowKey}
            onWindowSelect={onWindowSelect}
          />
        </div>
      </div>
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

  return (
    <section className="surface-panel" data-testid="parallel-work-section">
      <div className="surface-panel-body gap-4">
        <div className="section-heading">
          <h3 className="section-title">{t("stats.parallelWork.title")}</h3>
          <p className="section-description">
            {t("stats.parallelWork.description")}
          </p>
        </div>
        {error ? (
          <ParallelWorkErrorCard
            activeWindowKey={activeWindowKey}
            onWindowSelect={setActiveWindowKey}
            error={error}
          />
        ) : isLoading || !activeWindow ? (
          <ParallelWorkLoadingCard
            activeWindowKey={activeWindowKey}
            onWindowSelect={setActiveWindowKey}
            windowKey={activeWindowKey}
          />
        ) : (
          <ParallelWorkWindowCard
            activeWindowKey={activeWindowKey}
            onWindowSelect={setActiveWindowKey}
            windowKey={activeWindowKey}
            window={activeWindow}
          />
        )}
      </div>
    </section>
  );
}
