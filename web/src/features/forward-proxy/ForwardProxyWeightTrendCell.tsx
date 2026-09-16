import { useCallback, useMemo, useState } from "react";
import {
  type InlineChartTooltipData,
  InlineChartTooltipSurface,
} from "../../components/ui/inline-chart-tooltip";
import type {
  ForwardProxyHourlyBucket,
  ForwardProxyLiveNode,
  ForwardProxyWeightBucket,
  ForwardProxyWindowStats,
} from "../../lib/api";
import {
  type ForwardProxyRequestTooltipLabels,
  ForwardProxyRequestTrendChart,
} from "./ForwardProxyRequestTrendChart";

function formatSuccessRate(value?: number) {
  if (value == null || Number.isNaN(value)) return "—";
  return `${(value * 100).toFixed(1)}%`;
}

function formatLatency(value?: number) {
  if (value == null || Number.isNaN(value)) return "—";
  return `${value.toFixed(0)} ms`;
}

function formatBucketRangeLabel(startRaw: string, endRaw: string, localeTag: string) {
  const start = new Date(startRaw);
  const end = new Date(endRaw);
  const formatter = new Intl.DateTimeFormat(localeTag, {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
  const startLabel = Number.isNaN(start.getTime()) ? startRaw : formatter.format(start);
  const endLabel = Number.isNaN(end.getTime()) ? endRaw : formatter.format(end);
  return `${startLabel} - ${endLabel}`;
}

export interface WeightTooltipLabels {
  samples: string;
  min: string;
  max: string;
  avg: string;
  last: string;
}

export function formatWeight(value: number) {
  if (!Number.isFinite(value)) return "—";
  return value.toFixed(2);
}

function buildWeightTooltipData(
  bucket: ForwardProxyWeightBucket,
  localeTag: string,
  labels: WeightTooltipLabels,
): InlineChartTooltipData {
  return {
    title: formatBucketRangeLabel(bucket.bucketStart, bucket.bucketEnd, localeTag),
    rows: [
      { label: labels.samples, value: String(bucket.sampleCount), tone: "accent" },
      { label: labels.min, value: formatWeight(bucket.minWeight), tone: "error" },
      { label: labels.max, value: formatWeight(bucket.maxWeight), tone: "success" },
      { label: labels.avg, value: formatWeight(bucket.avgWeight), tone: "accent" },
      {
        label: labels.last,
        value: formatWeight(bucket.lastWeight),
        tone: bucket.lastWeight >= 0 ? "success" : "error",
      },
    ],
  };
}

export interface WeightTrendGeometry {
  chartWidth: number;
  chartHeight: number;
  bucketWidth: number;
  zeroY: number;
  linePath: string;
  areaPath: string;
  points: Array<{ x: number; y: number }>;
}

export interface WeightTrendScale {
  minValue: number;
  maxValue: number;
}

function buildWeightTrendGeometry(
  buckets: ForwardProxyWeightBucket[],
  scale: WeightTrendScale,
): WeightTrendGeometry | null {
  if (buckets.length === 0) return null;
  const chartWidth = 216;
  const chartHeight = 40;
  const values = buckets.map((bucket) => bucket.lastWeight);
  const minValue = scale.minValue;
  const maxValue = scale.maxValue;
  const span = Math.max(maxValue - minValue, Number.EPSILON);
  const bucketWidth = chartWidth / buckets.length;
  const points = values.map((value, index) => {
    const ratio = Math.max(0, Math.min(1, (value - minValue) / span));
    const x = bucketWidth * index + bucketWidth / 2;
    const y = chartHeight - ratio * chartHeight;
    return { x, y };
  });
  const firstPoint = points[0];
  const lastPoint = points[points.length - 1];
  if (!firstPoint || !lastPoint) return null;

  const zeroRatio = (0 - minValue) / span;
  const zeroY = chartHeight - Math.max(0, Math.min(1, zeroRatio)) * chartHeight;

  const linePath = points
    .map((point, index) => `${index === 0 ? "M" : "L"} ${point.x.toFixed(2)} ${point.y.toFixed(2)}`)
    .join(" ");
  const areaPath = `${linePath} L ${lastPoint.x.toFixed(2)} ${zeroY.toFixed(2)} L ${firstPoint.x.toFixed(2)} ${zeroY.toFixed(2)} Z`;

  return {
    chartWidth,
    chartHeight,
    bucketWidth,
    zeroY,
    linePath,
    areaPath,
    points,
  };
}

export function WindowCell({ value }: { value: ForwardProxyWindowStats }) {
  return (
    <div className="space-y-0.5 text-[11px] leading-tight">
      <div>{formatSuccessRate(value.successRate)}</div>
      <div className="text-base-content/65">{formatLatency(value.avgLatencyMs)}</div>
    </div>
  );
}

export const forwardProxyWindowColumns = [
  {
    labelKey: "live.proxy.table.sevenDays",
    selectStats: (stats: ForwardProxyLiveNode["stats"]) => stats.sevenDays,
  },
  {
    labelKey: "live.proxy.table.oneDay",
    selectStats: (stats: ForwardProxyLiveNode["stats"]) => stats.oneDay,
  },
  {
    labelKey: "live.proxy.table.oneHour",
    selectStats: (stats: ForwardProxyLiveNode["stats"]) => stats.oneHour,
  },
  {
    labelKey: "live.proxy.table.fifteenMinutes",
    selectStats: (stats: ForwardProxyLiveNode["stats"]) => stats.fifteenMinutes,
  },
  {
    labelKey: "live.proxy.table.oneMinute",
    selectStats: (stats: ForwardProxyLiveNode["stats"]) => stats.oneMinute,
  },
] as const;

export const forwardProxyWindowHeaderClassNames = [
  "w-[18%] px-1 py-2 text-center font-semibold sm:w-[13%] sm:px-2 sm:py-3 md:w-[8%] lg:w-[8%]",
  "hidden px-2 py-3 text-center font-semibold md:table-cell md:w-[8%] lg:w-[8%]",
  "hidden px-2 py-3 text-center font-semibold md:table-cell md:w-[8%] lg:w-[8%]",
  "hidden px-2 py-3 text-center font-semibold lg:table-cell lg:w-[8%]",
  "hidden px-2 py-3 text-center font-semibold lg:table-cell lg:w-[8%]",
] as const;

function resolveLinkedActiveIndex<T extends { bucketStart: string }>(
  buckets: T[],
  activeBucketStart: string | null,
) {
  if (!activeBucketStart) return null;
  const index = buckets.findIndex((bucket) => bucket.bucketStart === activeBucketStart);
  return index >= 0 ? index : null;
}

export function ProxyTrendCells({
  node,
  weightBuckets,
  requestBucketScaleMax,
  weightTrendScale,
  localeTag,
  requestTooltipLabels,
  weightTooltipLabels,
  requestTrendAriaLabel,
  weightTrendAriaLabel,
  chartInteractionHint,
}: {
  node: ForwardProxyLiveNode;
  weightBuckets: ForwardProxyWeightBucket[];
  requestBucketScaleMax: number;
  weightTrendScale: WeightTrendScale;
  localeTag: string;
  requestTooltipLabels: ForwardProxyRequestTooltipLabels;
  weightTooltipLabels: WeightTooltipLabels;
  requestTrendAriaLabel: string;
  weightTrendAriaLabel: string;
  chartInteractionHint: string;
}) {
  const [activeBucketStart, setActiveBucketStart] = useState<string | null>(null);
  const linkedRequestIndex = useMemo(
    () => resolveLinkedActiveIndex(node.last24h, activeBucketStart),
    [activeBucketStart, node.last24h],
  );
  const linkedWeightIndex = useMemo(
    () => resolveLinkedActiveIndex(weightBuckets, activeBucketStart),
    [activeBucketStart, weightBuckets],
  );

  const handleRequestActiveIndexChange = useCallback(
    (index: number | null) => {
      setActiveBucketStart(index == null ? null : (node.last24h[index]?.bucketStart ?? null));
    },
    [node.last24h],
  );

  const handleWeightActiveIndexChange = useCallback(
    (index: number | null) => {
      setActiveBucketStart(index == null ? null : (weightBuckets[index]?.bucketStart ?? null));
    },
    [weightBuckets],
  );

  return (
    <>
      <td className="px-2 py-2 align-middle sm:px-3 sm:py-3">
        <RequestTrendCell
          buckets={node.last24h}
          scaleMax={requestBucketScaleMax}
          localeTag={localeTag}
          tooltipLabels={requestTooltipLabels}
          ariaLabel={`${node.displayName} ${requestTrendAriaLabel}`}
          interactionHint={chartInteractionHint}
          linkedActiveIndex={linkedRequestIndex}
          onActiveIndexChange={handleRequestActiveIndexChange}
        />
      </td>
      <td className="px-2 py-2 align-middle sm:px-3 sm:py-3">
        <WeightTrendCell
          buckets={weightBuckets}
          scale={weightTrendScale}
          localeTag={localeTag}
          tooltipLabels={weightTooltipLabels}
          ariaLabel={`${node.displayName} ${weightTrendAriaLabel}`}
          interactionHint={chartInteractionHint}
          clipId={`weight-trend-${node.key.replace(/[^a-zA-Z0-9_-]/g, "-")}`}
          linkedActiveIndex={linkedWeightIndex}
          onActiveIndexChange={handleWeightActiveIndexChange}
        />
      </td>
    </>
  );
}

function RequestTrendCell({
  buckets,
  scaleMax,
  localeTag,
  tooltipLabels,
  ariaLabel,
  interactionHint,
  linkedActiveIndex,
  onActiveIndexChange,
}: {
  buckets: ForwardProxyHourlyBucket[];
  scaleMax: number;
  localeTag: string;
  tooltipLabels: ForwardProxyRequestTooltipLabels;
  ariaLabel: string;
  interactionHint: string;
  linkedActiveIndex?: number | null;
  onActiveIndexChange?: (index: number | null) => void;
}) {
  return (
    <ForwardProxyRequestTrendChart
      buckets={buckets}
      scaleMax={scaleMax}
      localeTag={localeTag}
      tooltipLabels={tooltipLabels}
      ariaLabel={ariaLabel}
      interactionHint={interactionHint}
      linkedActiveIndex={linkedActiveIndex}
      onActiveIndexChange={onActiveIndexChange}
      variant="table"
      dataChartKind="proxy-request-trend"
      emptyState={<div className="text-[11px] text-base-content/55">—</div>}
    />
  );
}

export function WeightTrendCell({
  buckets,
  scale,
  localeTag,
  tooltipLabels,
  ariaLabel,
  interactionHint,
  clipId,
  linkedActiveIndex,
  onActiveIndexChange,
}: {
  buckets: ForwardProxyWeightBucket[];
  scale: WeightTrendScale;
  localeTag: string;
  tooltipLabels: WeightTooltipLabels;
  ariaLabel: string;
  interactionHint: string;
  clipId: string;
  linkedActiveIndex?: number | null;
  onActiveIndexChange?: (index: number | null) => void;
}) {
  const geometry = buildWeightTrendGeometry(buckets, scale);
  const tooltipData = useMemo(
    () => buckets.map((bucket) => buildWeightTooltipData(bucket, localeTag, tooltipLabels)),
    [buckets, localeTag, tooltipLabels],
  );
  if (!geometry) {
    return <div className="text-[11px] text-base-content/55">—</div>;
  }

  const positiveClipId = `${clipId}-positive`;
  const negativeClipId = `${clipId}-negative`;
  const positiveHeight = Math.max(geometry.zeroY, 0);
  const negativeHeight = Math.max(geometry.chartHeight - geometry.zeroY, 0);
  const defaultIndex = Math.max(0, buckets.length - 1);

  return (
    <InlineChartTooltipSurface
      items={tooltipData}
      defaultIndex={defaultIndex}
      ariaLabel={ariaLabel}
      interactionHint={interactionHint}
      linkedActiveIndex={linkedActiveIndex}
      onActiveIndexChange={onActiveIndexChange}
      className="py-0.5"
      chartClassName="flex h-11 items-end"
    >
      {({ highlightedIndex, getItemProps }) => (
        <WeightTrendSvg
          ariaLabel={ariaLabel}
          buckets={buckets}
          geometry={geometry}
          positiveClipId={positiveClipId}
          negativeClipId={negativeClipId}
          positiveHeight={positiveHeight}
          negativeHeight={negativeHeight}
          highlightedIndex={highlightedIndex}
          getItemProps={getItemProps}
        />
      )}
    </InlineChartTooltipSurface>
  );
}

function WeightTrendSvg({
  ariaLabel,
  buckets,
  geometry,
  positiveClipId,
  negativeClipId,
  positiveHeight,
  negativeHeight,
  highlightedIndex,
  getItemProps,
}: {
  ariaLabel: string;
  buckets: ForwardProxyWeightBucket[];
  geometry: WeightTrendGeometry;
  positiveClipId: string;
  negativeClipId: string;
  positiveHeight: number;
  negativeHeight: number;
  highlightedIndex: number | null;
  getItemProps: (index: number) => React.SVGProps<SVGRectElement>;
}) {
  const activePoint = highlightedIndex != null ? geometry.points[highlightedIndex] : null;
  const activeBucket = highlightedIndex != null ? buckets[highlightedIndex] : null;
  return (
    <svg
      viewBox={`0 0 ${geometry.chartWidth} ${geometry.chartHeight}`}
      className="block h-10 w-full rounded-md border border-base-300/55 bg-base-100/40"
      data-chart-kind="proxy-weight-trend"
    >
      <title>{ariaLabel}</title>
      <WeightTrendDefs
        geometry={geometry}
        positiveClipId={positiveClipId}
        negativeClipId={negativeClipId}
        positiveHeight={positiveHeight}
        negativeHeight={negativeHeight}
      />
      <WeightTrendPaths
        geometry={geometry}
        positiveClipId={positiveClipId}
        negativeClipId={negativeClipId}
        activePoint={activePoint}
      />
      <WeightTrendPoints
        buckets={buckets}
        points={geometry.points}
        highlightedIndex={highlightedIndex}
        activePoint={activePoint}
        activeBucket={activeBucket}
      />
      <WeightTrendHitAreas buckets={buckets} geometry={geometry} getItemProps={getItemProps} />
    </svg>
  );
}

function WeightTrendDefs({
  geometry,
  positiveClipId,
  negativeClipId,
  positiveHeight,
  negativeHeight,
}: {
  geometry: WeightTrendGeometry;
  positiveClipId: string;
  negativeClipId: string;
  positiveHeight: number;
  negativeHeight: number;
}) {
  return (
    <defs>
      <clipPath id={positiveClipId}>
        <rect x={0} y={0} width={geometry.chartWidth} height={positiveHeight} />
      </clipPath>
      <clipPath id={negativeClipId}>
        <rect x={0} y={geometry.zeroY} width={geometry.chartWidth} height={negativeHeight} />
      </clipPath>
    </defs>
  );
}

function WeightTrendPaths({
  geometry,
  positiveClipId,
  negativeClipId,
  activePoint,
}: {
  geometry: WeightTrendGeometry;
  positiveClipId: string;
  negativeClipId: string;
  activePoint: { x: number; y: number } | null;
}) {
  const strokeWidth = activePoint ? "1.9" : "1.6";
  return (
    <>
      <line
        x1={0}
        y1={geometry.zeroY}
        x2={geometry.chartWidth}
        y2={geometry.zeroY}
        stroke="oklch(var(--color-base-content) / 0.15)"
        strokeWidth="1"
      />
      {activePoint ? (
        <line
          x1={activePoint.x}
          y1={0}
          x2={activePoint.x}
          y2={geometry.chartHeight}
          stroke="oklch(var(--color-primary) / 0.45)"
          strokeWidth="1"
          strokeDasharray="3 2"
        />
      ) : null}
      <path
        d={geometry.areaPath}
        fill="oklch(var(--color-success) / 0.18)"
        clipPath={`url(#${positiveClipId})`}
      />
      <path
        d={geometry.areaPath}
        fill="oklch(var(--color-error) / 0.16)"
        clipPath={`url(#${negativeClipId})`}
      />
      <path
        d={geometry.linePath}
        fill="none"
        stroke="oklch(var(--color-success) / 0.95)"
        clipPath={`url(#${positiveClipId})`}
        strokeWidth={strokeWidth}
        strokeLinejoin="round"
        strokeLinecap="round"
      />
      <path
        d={geometry.linePath}
        fill="none"
        stroke="oklch(var(--color-error) / 0.92)"
        clipPath={`url(#${negativeClipId})`}
        strokeWidth={strokeWidth}
        strokeLinejoin="round"
        strokeLinecap="round"
      />
    </>
  );
}

function WeightTrendPoints({
  buckets,
  points,
  highlightedIndex,
  activePoint,
  activeBucket,
}: {
  buckets: ForwardProxyWeightBucket[];
  points: Array<{ x: number; y: number }>;
  highlightedIndex: number | null;
  activePoint: { x: number; y: number } | null;
  activeBucket: ForwardProxyWeightBucket | null;
}) {
  return (
    <>
      {points.map((point, index) => {
        const isActive = highlightedIndex === index;
        const isPositive = (buckets[index]?.lastWeight ?? 0) >= 0;
        return (
          <circle
            key={`${buckets[index]?.bucketStart ?? `${point.x}-${point.y}`}-dot`}
            cx={point.x}
            cy={point.y}
            r={isActive ? 2.6 : 1.5}
            fill={
              isPositive ? "oklch(var(--color-success) / 0.95)" : "oklch(var(--color-error) / 0.9)"
            }
            stroke={isActive ? "oklch(var(--color-base-100) / 0.95)" : "none"}
            strokeWidth={isActive ? "1.2" : "0"}
          />
        );
      })}
      {activePoint && activeBucket ? (
        <circle
          cx={activePoint.x}
          cy={activePoint.y}
          r="4"
          fill="none"
          stroke={
            activeBucket.lastWeight >= 0
              ? "oklch(var(--color-success) / 0.45)"
              : "oklch(var(--color-error) / 0.45)"
          }
          strokeWidth="1"
        />
      ) : null}
    </>
  );
}

function WeightTrendHitAreas({
  buckets,
  geometry,
  getItemProps,
}: {
  buckets: ForwardProxyWeightBucket[];
  geometry: WeightTrendGeometry;
  getItemProps: (index: number) => React.SVGProps<SVGRectElement>;
}) {
  return (
    <>
      {buckets.map((bucket, index) => (
        <rect
          key={`${bucket.bucketStart}-hit`}
          x={geometry.bucketWidth * index}
          y={0}
          width={geometry.bucketWidth}
          height={geometry.chartHeight}
          fill="transparent"
          className="cursor-pointer"
          {...getItemProps(index)}
        />
      ))}
    </>
  );
}
