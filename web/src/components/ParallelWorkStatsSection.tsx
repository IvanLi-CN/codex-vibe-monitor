import {
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  useId,
  useMemo,
} from "react";
import {
  Area,
  AreaChart,
  CartesianGrid,
  Line,
  ResponsiveContainer,
  XAxis,
  YAxis,
} from "recharts";
import type {
  ParallelWorkConversation,
  ParallelWorkStatsResponse,
  ParallelWorkWindowResponse,
} from "../lib/api";
import { useTranslation } from "../i18n";
import { chartBaseTokens, metricAccent, withOpacity } from "../lib/chartTheme";
import { useTheme } from "../theme";
import { Alert } from "./ui/alert";
import {
  InlineChartTooltipSurface,
  type InlineChartTooltipData,
} from "./ui/inline-chart-tooltip";
import { InfoTooltip } from "./ui/info-tooltip";

interface ParallelWorkStatsSectionProps {
  stats: ParallelWorkStatsResponse | null;
  isLoading: boolean;
  error: string | null;
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
  localeTag: string,
): Array<{ index: number; label: string }> {
  if (window.points.length === 0) return [];
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
    index,
    label: formatParallelWorkAxisBucketLabel(
      window.points[index]?.bucketStart ?? "",
      localeTag,
      showYear,
      useDetailedLabels,
      effectiveTimeZone,
    ),
  }));
}

function buildParallelWorkChartData(
  window: ParallelWorkWindowResponse,
  localeTag: string,
) {
  const xAxisTicks = buildParallelWorkXAxisTicks(window, localeTag);
  const labelsByIndex = new Map(
    xAxisTicks.map((tick) => [tick.index, tick.label]),
  );

  return window.points.map((point, index) => ({
    ...point,
    index,
    axisLabel: labelsByIndex.get(index) ?? "",
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

function formatParallelWorkTimeLabel(raw: string, localeTag: string, timeZone: string) {
  const value = new Date(raw);
  if (Number.isNaN(value.getTime())) return raw;
  return new Intl.DateTimeFormat(localeTag, {
    timeZone,
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  }).format(value);
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

function buildParallelWorkConversationTooltipData(
  conversations: ParallelWorkConversation[],
  localeTag: string,
  timeZone: string,
  requestCountLabel: string,
  numberFormatter: Intl.NumberFormat,
) {
  return conversations.map<InlineChartTooltipData>((conversation, index) => ({
    title:
      "Conversation " +
      (index + 1) +
      " · " +
      formatParallelWorkTimeLabel(conversation.start, localeTag, timeZone) +
      " → " +
      formatParallelWorkTimeLabel(conversation.end, localeTag, timeZone),
    rows: [
      {
        label: requestCountLabel,
        value: numberFormatter.format(conversation.requestCount),
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

interface ParallelWorkChartDotProps {
  cx?: number;
  cy?: number;
  index?: number;
  highlightedIndex: number | null;
  strokeColor: string;
  fillColor: string;
  surfaceColor: string;
}

interface ParallelWorkXAxisTickProps {
  x?: number;
  y?: number;
  payload?: {
    value?: number | string;
  };
  labelsByIndex: Map<number, string>;
  maxIndex: number;
  fill: string;
}

function ParallelWorkXAxisTick({
  x,
  y,
  payload,
  labelsByIndex,
  maxIndex,
  fill,
}: ParallelWorkXAxisTickProps) {
  if (typeof x !== "number" || typeof y !== "number") return null;
  const index = Number(payload?.value ?? 0);
  const label = labelsByIndex.get(index) ?? "";
  const textAnchor =
    index <= 0 ? "start" : index >= maxIndex ? "end" : "middle";

  return (
    <text
      x={x}
      y={y}
      dy={13}
      fill={fill}
      fontSize={11}
      textAnchor={textAnchor}
    >
      {label}
    </text>
  );
}

function ParallelWorkChartDot({
  cx,
  cy,
  index,
  highlightedIndex,
  strokeColor,
  fillColor,
  surfaceColor,
}: ParallelWorkChartDotProps) {
  if (typeof cx !== "number" || typeof cy !== "number") return null;
  const active = index === highlightedIndex;

  return (
    <circle
      cx={cx}
      cy={cy}
      r={active ? 4.5 : 3}
      fill={active ? strokeColor : fillColor}
      stroke={surfaceColor}
      strokeWidth={active ? 1.8 : 1.25}
    />
  );
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
  const { themeMode } = useTheme();
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
  const chartData = useMemo(
    () => buildParallelWorkChartData(window, localeTag),
    [localeTag, window],
  );
  const xAxisTicks = useMemo(
    () => buildParallelWorkXAxisTicks(window, localeTag),
    [localeTag, window],
  );
  const chartColors = useMemo(() => {
    const base = chartBaseTokens(themeMode);
    const accent = metricAccent("totalCount", themeMode);
    return {
      ...base,
      accent,
      accentFill: withOpacity(accent, 0.2),
      accentDot: withOpacity(accent, 0.82),
      surface: themeMode === "dark" ? "#111827" : "#ffffff",
    };
  }, [themeMode]);
  const yAxisDomainMax = Math.max(1, scaleMaxCount);
  const xAxisDomainMax = Math.max(0, chartData.length - 1);
  const animate = chartData.length <= 800;
  const overlayLabelByIndex = useMemo(
    () => new Map(xAxisTicks.map((tick) => [tick.index, tick.label])),
    [xAxisTicks],
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
        const resolveOverlayIndex = (
          clientX: number,
          currentTarget: HTMLElement,
        ) => {
          if (chartData.length <= 1) return 0;
          const rect = currentTarget.getBoundingClientRect();
          if (rect.width <= 0) return defaultIndex;
          const ratio = Math.max(
            0,
            Math.min(1, (clientX - rect.left) / rect.width),
          );
          return Math.max(
            0,
            Math.min(chartData.length - 1, Math.round(ratio * xAxisDomainMax)),
          );
        };
        const resolveMarkerLeft = (index: number) =>
          chartData.length <= 1 ? 50 : (index / xAxisDomainMax) * 100;
        const handleOverlayPointer = (
          event: ReactPointerEvent<HTMLButtonElement>,
          handler:
            | "onPointerEnter"
            | "onPointerMove"
            | "onPointerDown",
        ) => {
          const index = resolveOverlayIndex(event.clientX, event.currentTarget);
          getItemProps(index)[handler](event as never);
        };
        const handleOverlayMouse = (
          event: ReactMouseEvent<HTMLButtonElement>,
          handler: "onMouseEnter" | "onMouseMove" | "onClick",
        ) => {
          const index = resolveOverlayIndex(event.clientX, event.currentTarget);
          const itemProps = getItemProps(index);
          if (handler === "onClick") {
            itemProps.onClick();
            itemProps.onMouseEnter(event as never);
            return;
          }
          itemProps[handler](event as never);
        };

        return (
          <div
            className="relative h-44 w-full rounded-2xl border border-base-300/75 bg-base-100/75"
            data-chart-kind="parallel-work-sparkline"
            data-chart-mode="recharts-area"
          >
            <ResponsiveContainer>
              <AreaChart
                data={chartData}
                margin={{ top: 14, right: 16, left: -8, bottom: 8 }}
              >
                <defs>
                  <linearGradient id={gradientId} x1="0" x2="0" y1="0" y2="1">
                    <stop offset="0%" stopColor={chartColors.accentFill} />
                    <stop
                      offset="100%"
                      stopColor={withOpacity(chartColors.accent, 0.03)}
                    />
                  </linearGradient>
                </defs>
                <CartesianGrid
                  stroke={chartColors.gridLine}
                  strokeDasharray="4 4"
                  vertical={false}
                />
                <XAxis
                  dataKey="index"
                  type="number"
                  domain={[0, xAxisDomainMax]}
                  ticks={xAxisTicks.map((tick) => tick.index)}
                  interval={0}
                  axisLine={{ stroke: chartColors.gridLine }}
                  tickLine={{ stroke: chartColors.gridLine }}
                  tick={
                    <ParallelWorkXAxisTick
                      labelsByIndex={overlayLabelByIndex}
                      maxIndex={xAxisDomainMax}
                      fill={chartColors.axisText}
                    />
                  }
                />
                <YAxis
                  domain={[0, yAxisDomainMax]}
                  allowDecimals={false}
                  width={46}
                  tickCount={3}
                  tickFormatter={(value) =>
                    numberFormatter.format(Number(value))
                  }
                  axisLine={{ stroke: chartColors.gridLine }}
                  tickLine={{ stroke: chartColors.gridLine }}
                  tick={{ fill: chartColors.axisText, fontSize: 11 }}
                />
                <Area
                  type="monotone"
                  dataKey="parallelCount"
                  stroke="none"
                  fill={"url(#" + gradientId + ")"}
                  fillOpacity={1}
                  dot={false}
                  activeDot={false}
                  isAnimationActive={animate}
                />
                <Line
                  type="monotone"
                  dataKey="parallelCount"
                  stroke={chartColors.accent}
                  strokeWidth={3}
                  dot={(props) => (
                    <ParallelWorkChartDot
                      {...(props as unknown as ParallelWorkChartDotProps)}
                      highlightedIndex={highlightedIndex}
                      strokeColor={chartColors.accent}
                      fillColor={chartColors.accentDot}
                      surfaceColor={chartColors.surface}
                    />
                  )}
                  activeDot={false}
                  isAnimationActive={animate}
                />
              </AreaChart>
            </ResponsiveContainer>
            <div className="absolute bottom-8 left-[38px] right-4 top-3">
              {chartData.map((point, index) => {
                const { ref } = getItemProps(index);
                return (
                  <span
                    key={point.bucketStart + "-" + point.bucketEnd}
                    ref={ref}
                    aria-hidden="true"
                    className="pointer-events-none absolute top-1/2 h-px w-px -translate-x-1/2 -translate-y-1/2"
                    style={{
                      left: `${resolveMarkerLeft(index)}%`,
                    }}
                  />
                );
              })}
              <button
                type="button"
                tabIndex={-1}
                data-testid="parallel-work-interaction-overlay"
                className="absolute inset-0 cursor-pointer rounded-sm bg-transparent p-0 text-transparent outline-none focus-visible:ring-2 focus-visible:ring-primary/70"
                aria-label={ariaLabel}
                onPointerEnter={(event) =>
                  handleOverlayPointer(event, "onPointerEnter")
                }
                onPointerMove={(event) =>
                  handleOverlayPointer(event, "onPointerMove")
                }
                onPointerDown={(event) =>
                  handleOverlayPointer(event, "onPointerDown")
                }
                onMouseEnter={(event) =>
                  handleOverlayMouse(event, "onMouseEnter")
                }
                onMouseMove={(event) => handleOverlayMouse(event, "onMouseMove")}
                onMouseDown={() => getItemProps(defaultIndex).onMouseDown()}
                onTouchStart={(event) => {
                  const firstTouch = event.touches[0];
                  const index = firstTouch
                    ? resolveOverlayIndex(
                        firstTouch.clientX,
                        event.currentTarget,
                      )
                    : defaultIndex;
                  getItemProps(index).onTouchStart();
                }}
                onClick={(event) => handleOverlayMouse(event, "onClick")}
              />
            </div>
          </div>
        );
      }}
    </InlineChartTooltipSurface>
  );
}

function ParallelWorkConversationGantt({
  window,
  emptyLabel,
  ariaLabel,
  interactionHint,
  requestCountLabel,
}: {
  window: ParallelWorkWindowResponse;
  emptyLabel: string;
  ariaLabel: string;
  interactionHint: string;
  requestCountLabel: string;
}) {
  const { locale } = useTranslation();
  const { themeMode } = useTheme();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag),
    [localeTag],
  );
  const conversations = useMemo(
    () =>
      [...(window.conversations ?? [])].sort(
        (a, b) => Date.parse(a.start) - Date.parse(b.start),
      ),
    [window.conversations],
  );
  const effectiveTimeZone = window.effectiveTimeZone ?? "Asia/Shanghai";
  const tooltipData = useMemo(
    () =>
      buildParallelWorkConversationTooltipData(
        conversations,
        localeTag,
        effectiveTimeZone,
        requestCountLabel,
        numberFormatter,
      ),
    [conversations, effectiveTimeZone, localeTag, numberFormatter, requestCountLabel],
  );
  const chartColors = useMemo(() => {
    const base = chartBaseTokens(themeMode);
    const accent = metricAccent("totalCount", themeMode);
    return {
      ...base,
      accent,
      accentFill: withOpacity(accent, 0.2),
      axis: themeMode === "dark" ? "rgba(229, 231, 235, 0.7)" : "rgba(55, 65, 81, 0.68)",
    };
  }, [themeMode]);
  const rangeStartMs = Date.parse(window.rangeStart);
  const rangeEndMs = Date.parse(window.rangeEnd);
  const rangeMs = Math.max(1, rangeEndMs - rangeStartMs);
  const axisLabels = [
    window.rangeStart,
    new Date(rangeStartMs + rangeMs / 2).toISOString(),
    window.rangeEnd,
  ];

  if (conversations.length === 0 || Number.isNaN(rangeStartMs) || Number.isNaN(rangeEndMs)) {
    return (
      <div className="flex h-32 items-center justify-center rounded-2xl border border-dashed border-base-300/75 bg-base-200/30 text-sm text-base-content/55">
        {emptyLabel}
      </div>
    );
  }

  return (
    <InlineChartTooltipSurface
      items={tooltipData}
      defaultIndex={Math.max(0, conversations.length - 1)}
      ariaLabel={ariaLabel}
      interactionHint={interactionHint}
      className="w-full py-0.5"
      chartClassName="w-full"
    >
      {({ highlightedIndex, getItemProps }) => (
        <div
          className="relative max-h-[34rem] w-full overflow-auto rounded-2xl border border-base-300/75 bg-base-100/75"
          data-chart-kind="parallel-work-sparkline"
          data-chart-mode="conversation-gantt"
          data-testid="parallel-work-conversation-gantt"
        >
          <div className="min-w-[720px] px-4 pb-3 pt-3">
            <div className="grid grid-cols-[4.25rem_minmax(0,1fr)] gap-x-3 gap-y-0.5">
              <div className="sticky top-0 z-10 bg-base-100/95 pb-1 text-[10px] font-medium uppercase tracking-[0.08em] text-base-content/45">
                {locale === "zh" ? "对话" : "Conversation"}
              </div>
              <div className="sticky top-0 z-10 h-5 border-b border-base-300/70 bg-base-100/95">
                {axisLabels.map((label, index) => (
                  <span
                    key={label + index}
                    className="absolute top-0 -translate-x-1/2 text-[11px] text-base-content/50 first:translate-x-0 last:-translate-x-full"
                    style={{ left: `${index * 50}%` }}
                  >
                    {formatParallelWorkTimeLabel(label, localeTag, effectiveTimeZone)}
                  </span>
                ))}
              </div>
              {conversations.map((conversation, index) => {
                const startMs = Date.parse(conversation.start);
                const endMs = Date.parse(conversation.end);
                const left = Math.max(0, Math.min(100, ((startMs - rangeStartMs) / rangeMs) * 100));
                const right = Math.max(0, Math.min(100, ((rangeEndMs - endMs) / rangeMs) * 100));
                const width = Math.max(0.8, 100 - left - right);
                const active = highlightedIndex === index;
                const itemProps = getItemProps(index);
                return (
                  <div className="contents" key={conversation.conversationId}>
                    <div className="flex h-4 items-center truncate pr-2 text-[11px] font-medium text-base-content/62">
                      {"#" + (index + 1)}
                    </div>
                    <div className="relative h-4 border-l border-base-300/55">
                      <div
                        aria-hidden="true"
                        className="absolute inset-y-0 border-r border-dashed border-base-300/55"
                        style={{ left: "50%" }}
                      />
                      <button
                        {...itemProps}
                        type="button"
                        data-testid="parallel-work-conversation-bar"
                        className="absolute top-1/2 h-2.5 -translate-y-1/2 rounded-full outline-none transition-all focus-visible:ring-2 focus-visible:ring-primary/70"
                        style={{
                          left: `${left}%`,
                          width: `${width}%`,
                          minWidth: 8,
                          backgroundColor: active ? chartColors.accent : chartColors.accentFill,
                          boxShadow: active ? `0 0 0 2px ${withOpacity(chartColors.accent, 0.22)}` : "none",
                        }}
                        aria-label={
                          "Conversation " +
                          (index + 1) +
                          " " +
                          formatParallelWorkTimeLabel(conversation.start, localeTag, effectiveTimeZone) +
                          " to " +
                          formatParallelWorkTimeLabel(conversation.end, localeTag, effectiveTimeZone)
                        }
                      />
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        </div>
      )}
    </InlineChartTooltipSurface>
  );
}

function ParallelWorkWindowCard({ window }: { window: ParallelWorkWindowResponse }) {
  const { t, locale } = useTranslation();
  const empty = window.completeBucketCount === 0;
  const rangeDurationMs = Date.parse(window.rangeEnd) - Date.parse(window.rangeStart);
  const useConversationTimeline =
    Number.isFinite(rangeDurationMs) &&
    rangeDurationMs > 0 &&
    rangeDurationMs <= 24 * 60 * 60 * 1000 &&
    (window.conversations?.length ?? 0) > 0;
  const effectiveTimeZone = window.effectiveTimeZone ?? "Asia/Shanghai";
  const timeZoneFallbackNote = window.timeZoneFallback
    ? t("stats.parallelWork.timeZoneFallback", {
        timeZone: effectiveTimeZone,
      })
    : null;

  return (
    <article
      className="flex flex-col gap-4 rounded-[1.35rem] border border-base-300/75 bg-base-100/82 p-4 shadow-sm"
      data-testid="parallel-work-card-current"
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

      {useConversationTimeline ? (
        <ParallelWorkConversationGantt
          window={window}
          emptyLabel={t("stats.parallelWork.empty")}
          ariaLabel={t("stats.parallelWork.chartAria", {
            title: t("stats.parallelWork.title"),
          })}
          interactionHint={t("live.chart.tooltip.instructions")}
          requestCountLabel={t("stats.parallelWork.tooltip.requestCount")}
        />
      ) : (
        <ParallelWorkSparkline
          window={window}
          emptyLabel={t("stats.parallelWork.empty")}
          ariaLabel={t("stats.parallelWork.chartAria", {
            title: t("stats.parallelWork.title"),
          })}
          interactionHint={t("live.chart.tooltip.instructions")}
          tooltipCountLabel={t("stats.parallelWork.tooltip.parallelCount")}
        />
      )}

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

function ParallelWorkLoadingCard() {
  const { t } = useTranslation();

  return (
    <article
      className="flex min-h-[18rem] flex-col gap-4 rounded-[1.35rem] border border-base-300/75 bg-base-100/82 p-4 shadow-sm"
      data-testid="parallel-work-card-current"
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

function ParallelWorkErrorCard({ error }: { error: string }) {
  return (
    <article
      className="flex min-h-[14rem] flex-col gap-4 rounded-[1.35rem] border border-base-300/75 bg-base-100/82 p-4 shadow-sm"
      data-testid="parallel-work-card-current"
    >
      <Alert variant="error">{error}</Alert>
    </article>
  );
}

export function ParallelWorkStatsSection({
  stats,
  isLoading,
  error,
}: ParallelWorkStatsSectionProps) {
  const { t } = useTranslation();
  const activeWindow = stats?.current ?? null;
  const activeTitle = t("stats.parallelWork.title");
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
    t("stats.parallelWork.description"),
    activeSamples,
    activeTimeZoneFallbackNote,
  );

  return (
    <section className="surface-panel" data-testid="parallel-work-section">
      <div className="surface-panel-body gap-4">
        <div className="section-heading min-w-0">
          <div
            className="flex items-center gap-2"
            data-testid="parallel-work-heading-current"
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
        {error ? (
          <ParallelWorkErrorCard error={error} />
        ) : isLoading || !activeWindow ? (
          <ParallelWorkLoadingCard />
        ) : (
          <ParallelWorkWindowCard window={activeWindow} />
        )}
      </div>
    </section>
  );
}
