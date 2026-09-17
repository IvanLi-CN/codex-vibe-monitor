import { useCallback, useEffect, useMemo, useRef } from "react";
import { Spinner } from "../../components/ui/spinner";
import { chartBaseTokens, chartStatusTokens, metricAccent } from "../../lib/chartTheme";
import {
  ConversationActivityChartCanvas,
  type ConversationActivityChartColors,
  type ConversationActivityLegendLabels,
} from "./ConversationActivityChartCanvas";
import {
  useConversationActivityDrag,
  useConversationActivityViewport,
  useConversationActivityWheel,
} from "./ConversationActivityChartInteraction";
import {
  type ConversationActivityBucket,
  type ConversationActivityMetric,
  clampConversationActivityValue,
  normalizeConversationActivityViewport,
  resolveDocumentThemeMode,
} from "./PromptCacheActivityData";

export function ConversationActivityChart({
  buckets,
  rangeStartMs,
  rangeEndMs,
  metric,
  loading,
  numberFormatter,
  currencyFormatter,
  t,
}: {
  buckets: ConversationActivityBucket[];
  rangeStartMs: number | null;
  rangeEndMs: number | null;
  metric: ConversationActivityMetric;
  loading: boolean;
  numberFormatter: Intl.NumberFormat;
  currencyFormatter: Intl.NumberFormat;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  const themeMode = resolveDocumentThemeMode();
  const { viewport, viewportRef, setViewport } = useConversationActivityViewport(buckets);
  const drag = useConversationActivityDrag(buckets, viewport, setViewport);
  const handleWheel = useConversationActivityWheel(
    buckets,
    viewportRef,
    drag.interactionRef,
    setViewport,
  );
  const wheelElementRef = useRef<HTMLDivElement | null>(null);
  const setInteractionLayerRef = useCallback(
    (node: HTMLDivElement | null) => {
      wheelElementRef.current?.removeEventListener("wheel", handleWheel);
      drag.interactionRef.current = node;
      wheelElementRef.current = node;
      node?.addEventListener("wheel", handleWheel, { passive: false });
    },
    [drag.interactionRef, handleWheel],
  );
  useEffect(
    () => () => wheelElementRef.current?.removeEventListener("wheel", handleWheel),
    [handleWheel],
  );

  const visibleWindow = normalizeConversationActivityViewport(viewport, buckets.length);
  const visibleBuckets = buckets.slice(visibleWindow.startIndex, visibleWindow.endIndex + 1);
  const viewportSpan = visibleWindow.endIndex - visibleWindow.startIndex + 1;
  const isZoomed = buckets.length > 0 && viewportSpan < buckets.length;
  const colors = useMemo(() => chartColors(themeMode), [themeMode]);
  const maxCount = Math.max(
    1,
    ...visibleBuckets.map((bucket) =>
      Math.max(bucket.success + bucket.inFlight + bucket.neutral, bucket.failure),
    ),
  );
  const barSize = resolveBarSize(buckets.length, viewportSpan);
  const legendLabels = useMemo<ConversationActivityLegendLabels>(
    () => ({
      success: t("live.conversations.activity.legendSuccess"),
      failure: t("live.conversations.activity.legendFailure"),
      inFlight: t("live.conversations.activity.legendInFlight"),
      neutral: t("live.conversations.activity.legendNeutral"),
      duration: t("table.details.totalLatency"),
    }),
    [t],
  );
  const renderTooltip = useCallback(
    (bucket: ConversationActivityBucket) =>
      buildTooltipRows(
        bucket,
        metric,
        colors,
        numberFormatter,
        currencyFormatter,
        t("unit.calls"),
        legendLabels,
      ),
    [colors, currencyFormatter, legendLabels, metric, numberFormatter, t],
  );
  return (
    <ConversationActivityChartView
      loading={loading}
      buckets={buckets}
      visibleBuckets={visibleBuckets}
      visibleWindow={visibleWindow}
      viewportSpan={viewportSpan}
      maxCount={maxCount}
      barSize={barSize}
      metric={metric}
      numberFormatter={numberFormatter}
      colors={colors}
      legendLabels={legendLabels}
      renderTooltip={renderTooltip}
      interaction={{ ...drag, setInteractionLayerRef }}
      rangeStartMs={rangeStartMs}
      rangeEndMs={rangeEndMs}
      isZoomed={isZoomed}
      t={t}
    />
  );
}

function ConversationActivityChartView({
  loading,
  buckets,
  visibleBuckets,
  visibleWindow,
  viewportSpan,
  maxCount,
  barSize,
  metric,
  numberFormatter,
  colors,
  legendLabels,
  renderTooltip,
  interaction,
  rangeStartMs,
  rangeEndMs,
  isZoomed,
  t,
}: {
  loading: boolean;
  buckets: ConversationActivityBucket[];
  visibleBuckets: ConversationActivityBucket[];
  visibleWindow: { startIndex: number; endIndex: number };
  viewportSpan: number;
  maxCount: number;
  barSize: number;
  metric: ConversationActivityMetric;
  numberFormatter: Intl.NumberFormat;
  colors: ConversationActivityChartColors;
  legendLabels: ConversationActivityLegendLabels;
  renderTooltip: (
    bucket: ConversationActivityBucket,
  ) => Array<{ label: string; value: string; color: string }>;
  interaction: Parameters<typeof ConversationActivityChartCanvas>[0]["interaction"];
  rangeStartMs: number | null;
  rangeEndMs: number | null;
  isZoomed: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  if (loading && buckets.length === 0) {
    return (
      <div className="flex h-80 items-center justify-center gap-2 rounded-xl border border-base-300/75 bg-base-200/40 text-sm text-base-content/60">
        <Spinner size="sm" aria-label={t("chart.loadingDetailed")} />
        <span>{t("chart.loadingDetailed")}</span>
      </div>
    );
  }
  return (
    <ConversationActivityChartCanvas
      buckets={buckets}
      visibleBuckets={visibleBuckets}
      xDomain={[visibleWindow.startIndex, visibleWindow.endIndex]}
      maxCount={maxCount}
      barSize={barSize}
      metric={metric}
      numberFormatter={numberFormatter}
      colors={colors}
      legendLabels={legendLabels}
      renderTooltip={renderTooltip}
      interaction={interaction}
      rangeStartMs={rangeStartMs}
      rangeEndMs={rangeEndMs}
      visibleWindow={visibleWindow}
      viewportSpan={viewportSpan}
      visibleTotalCount={visibleBuckets.reduce((sum, bucket) => sum + bucket.totalCount, 0)}
      isZoomed={isZoomed}
      t={t}
    />
  );
}

function resolveBarSize(bucketCount: number, viewportSpan: number) {
  if (bucketCount <= 0) return 1;
  return clampConversationActivityValue(
    Math.round((bucketCount / Math.max(1, viewportSpan)) * 0.75),
    bucketCount <= 60 ? 5 : 1,
    10,
  );
}

function chartColors(themeMode: "light" | "dark"): ConversationActivityChartColors {
  const base = chartBaseTokens(themeMode);
  const status = chartStatusTokens(themeMode);
  return {
    ...base,
    success: status.success,
    failure: status.failure,
    inFlight: metricAccent("totalCount", themeMode),
    neutral: themeMode === "dark" ? "#94a3b8" : "#64748b",
    firstByte: themeMode === "dark" ? "#cbd5e1" : "#475569",
  };
}

function buildTooltipRows(
  bucket: ConversationActivityBucket,
  metric: ConversationActivityMetric,
  colors: ConversationActivityChartColors,
  numberFormatter: Intl.NumberFormat,
  currencyFormatter: Intl.NumberFormat,
  countUnit: string,
  labels: ConversationActivityLegendLabels,
) {
  const formatValue = (value: number) =>
    `${metric === "totalCost" ? currencyFormatter.format(value) : numberFormatter.format(value)} ${metric === "totalCount" ? countUnit : ""}`.trim();
  return [
    { label: labels.success, value: formatValue(bucket.success), color: colors.success },
    { label: labels.failure, value: formatValue(bucket.failure), color: colors.failure },
    { label: labels.inFlight, value: formatValue(bucket.inFlight), color: colors.inFlight },
    { label: labels.neutral, value: formatValue(bucket.neutral), color: colors.neutral },
    {
      label: labels.duration,
      value: bucket.avgTotalMs == null ? "-" : `${numberFormatter.format(bucket.avgTotalMs)} ms`,
      color: colors.firstByte,
    },
  ];
}
