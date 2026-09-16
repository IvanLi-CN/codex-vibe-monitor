import type { PointerEvent as ReactPointerEvent, RefObject } from "react";
import {
  Bar,
  CartesianGrid,
  ComposedChart,
  Line,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { floatingSurfaceStyle } from "../../components/ui/floating-surface";
import {
  type ConversationActivityBucket,
  type ConversationActivityMetric,
  resolveDocumentFloatingSurfaceTheme,
} from "./PromptCacheActivityData";

export interface ConversationActivityChartColors {
  gridLine: string;
  axisText: string;
  success: string;
  failure: string;
  inFlight: string;
  neutral: string;
  firstByte: string;
}

export interface ConversationActivityLegendLabels {
  success: string;
  failure: string;
  inFlight: string;
  neutral: string;
  duration: string;
}

type TooltipRow = { label: string; value: string; color: string };
type RenderTooltip = (bucket: ConversationActivityBucket) => TooltipRow[];
type ChartInteraction = {
  interactionRef: RefObject<HTMLDivElement | null>;
  dragPreviewLayerRef: RefObject<HTMLDivElement | null>;
  handlePointerDown: (event: ReactPointerEvent<HTMLDivElement>) => void;
  handlePointerMove: (event: ReactPointerEvent<HTMLDivElement>) => void;
  handlePointerEnd: (event: ReactPointerEvent<HTMLDivElement>) => void;
  setInteractionLayerRef: (node: HTMLDivElement | null) => void;
};

export function ConversationActivityChartCanvas({
  buckets,
  visibleBuckets,
  xDomain,
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
  visibleWindow,
  viewportSpan,
  visibleTotalCount,
  isZoomed,
  t,
}: {
  buckets: ConversationActivityBucket[];
  visibleBuckets: ConversationActivityBucket[];
  xDomain: [number, number];
  maxCount: number;
  barSize: number;
  metric: ConversationActivityMetric;
  numberFormatter: Intl.NumberFormat;
  colors: ConversationActivityChartColors;
  legendLabels: ConversationActivityLegendLabels;
  renderTooltip: RenderTooltip;
  interaction: ChartInteraction;
  rangeStartMs: number | null;
  rangeEndMs: number | null;
  visibleWindow: { startIndex: number; endIndex: number };
  viewportSpan: number;
  visibleTotalCount: number;
  isZoomed: boolean;
  t: (key: string) => string;
}) {
  return (
    <div
      className="overscroll-x-contain rounded-xl border border-base-300/75 bg-base-200/40 p-4"
      data-testid="conversation-activity-chart"
      data-chart-kind="conversation-activity"
      data-chart-metric={metric}
      data-visible-start-index={visibleWindow.startIndex}
      data-visible-end-index={visibleWindow.endIndex}
      data-visible-span={viewportSpan}
      data-visible-total-count={visibleTotalCount}
      data-zoomed={isZoomed ? "true" : "false"}
      data-chart-range-start={toIsoString(rangeStartMs)}
      data-chart-range-end={toIsoString(rangeEndMs)}
    >
      <div
        ref={interaction.setInteractionLayerRef}
        className="h-80 w-full cursor-grab touch-pan-y overflow-hidden overscroll-x-contain select-none active:cursor-grabbing"
        role="img"
        aria-label={t("live.conversations.activity.chartAria")}
        data-testid="conversation-activity-chart-interaction-layer"
        onPointerDown={interaction.handlePointerDown}
        onPointerMove={interaction.handlePointerMove}
        onPointerUp={interaction.handlePointerEnd}
        onPointerCancel={interaction.handlePointerEnd}
        onLostPointerCapture={interaction.handlePointerEnd}
      >
        <div
          ref={interaction.dragPreviewLayerRef}
          data-testid="conversation-activity-chart-drag-layer"
          className="h-full w-full will-change-transform"
        >
          <ConversationActivityRecharts
            buckets={buckets}
            visibleBuckets={visibleBuckets}
            xDomain={xDomain}
            maxCount={maxCount}
            barSize={barSize}
            numberFormatter={numberFormatter}
            colors={colors}
            legendLabels={legendLabels}
            renderTooltip={renderTooltip}
          />
        </div>
      </div>
      <ConversationActivityChartLegend colors={colors} labels={legendLabels} />
    </div>
  );
}

function toIsoString(value: number | null) {
  return typeof value === "number" && Number.isFinite(value)
    ? new Date(value).toISOString()
    : undefined;
}

function ConversationActivityRecharts({
  buckets,
  visibleBuckets,
  xDomain,
  maxCount,
  barSize,
  numberFormatter,
  colors,
  legendLabels,
  renderTooltip,
}: {
  buckets: ConversationActivityBucket[];
  visibleBuckets: ConversationActivityBucket[];
  xDomain: [number, number];
  maxCount: number;
  barSize: number;
  numberFormatter: Intl.NumberFormat;
  colors: ConversationActivityChartColors;
  legendLabels: ConversationActivityLegendLabels;
  renderTooltip: RenderTooltip;
}) {
  return (
    <ResponsiveContainer width="100%" height={320}>
      <ComposedChart
        data={visibleBuckets}
        margin={{ top: 12, right: 24, left: 0, bottom: 8 }}
        barGap="-100%"
        stackOffset="sign"
      >
        <ConversationActivityAxes
          buckets={buckets}
          xDomain={xDomain}
          maxCount={maxCount}
          numberFormatter={numberFormatter}
          colors={colors}
        />
        <Tooltip
          content={(props) => (
            <ConversationActivityTooltipContent
              active={props.active}
              label={props.label}
              payload={
                props.payload as unknown as
                  | Array<{ payload?: ConversationActivityBucket }>
                  | undefined
              }
              renderValue={renderTooltip}
            />
          )}
        />
        <ReferenceLine yAxisId="count" y={0} stroke={colors.gridLine} />
        <ConversationActivityBars barSize={barSize} colors={colors} labels={legendLabels} />
        <Line
          yAxisId="latency"
          type="monotone"
          dataKey="avgTotalMs"
          name={legendLabels.duration}
          stroke={colors.firstByte}
          strokeOpacity={0.72}
          strokeWidth={1.25}
          dot={{ r: 1.25, strokeWidth: 0, fill: colors.firstByte, fillOpacity: 0.72 }}
          connectNulls={false}
          isAnimationActive={false}
        />
      </ComposedChart>
    </ResponsiveContainer>
  );
}

function ConversationActivityAxes({
  buckets,
  xDomain,
  maxCount,
  numberFormatter,
  colors,
}: {
  buckets: ConversationActivityBucket[];
  xDomain: [number, number];
  maxCount: number;
  numberFormatter: Intl.NumberFormat;
  colors: ConversationActivityChartColors;
}) {
  const resolveBucket = (value: number) =>
    buckets[Math.max(0, Math.min(buckets.length - 1, Math.round(value)))];
  return (
    <>
      <CartesianGrid stroke={colors.gridLine} strokeDasharray="3 3" />
      <XAxis
        dataKey="index"
        type="number"
        domain={xDomain}
        minTickGap={28}
        axisLine={{ stroke: colors.gridLine }}
        tickLine={{ stroke: colors.gridLine }}
        tick={{ fill: colors.axisText, fontSize: 12 }}
        tickFormatter={(value: number) => resolveBucket(value)?.label ?? String(value)}
      />
      <YAxis
        yAxisId="count"
        domain={[-maxCount, maxCount]}
        allowDecimals={false}
        tickFormatter={(value) => numberFormatter.format(Math.abs(Number(value)))}
        axisLine={{ stroke: colors.gridLine }}
        tickLine={{ stroke: colors.gridLine }}
        tick={{ fill: colors.axisText, fontSize: 12 }}
      />
      <YAxis
        yAxisId="latency"
        orientation="right"
        tickFormatter={(value) => `${numberFormatter.format(Number(value))}ms`}
        width={72}
        axisLine={{ stroke: colors.gridLine }}
        tickLine={{ stroke: colors.gridLine }}
        tick={{ fill: colors.axisText, fontSize: 12 }}
      />
    </>
  );
}

function ConversationActivityBars({
  barSize,
  colors,
  labels,
}: {
  barSize: number;
  colors: ConversationActivityChartColors;
  labels: ConversationActivityLegendLabels;
}) {
  return (
    <>
      <Bar
        yAxisId="count"
        dataKey="failureNegative"
        name={labels.failure}
        stackId="positive"
        fill={colors.failure}
        barSize={barSize}
        radius={[0, 0, 3, 3]}
        shape={(props) => renderFailureBar({ ...props, fill: colors.failure })}
        isAnimationActive={false}
      />
      <Bar
        yAxisId="count"
        dataKey="success"
        name={labels.success}
        stackId="positive"
        fill={colors.success}
        barSize={barSize}
        isAnimationActive={false}
      />
      <Bar
        yAxisId="count"
        dataKey="inFlight"
        name={labels.inFlight}
        stackId="positive"
        fill={colors.inFlight}
        barSize={barSize}
        isAnimationActive={false}
      />
      <Bar
        yAxisId="count"
        dataKey="neutral"
        name={labels.neutral}
        stackId="positive"
        fill={colors.neutral}
        barSize={barSize}
        radius={[3, 3, 0, 0]}
        isAnimationActive={false}
      />
    </>
  );
}

function ConversationActivityChartLegend({
  colors,
  labels,
}: {
  colors: ConversationActivityChartColors;
  labels: ConversationActivityLegendLabels;
}) {
  const items = [
    [labels.success, colors.success],
    [labels.failure, colors.failure],
    [labels.inFlight, colors.inFlight],
    [labels.neutral, colors.neutral],
  ];
  return (
    <div className="flex flex-wrap items-center justify-center gap-x-4 gap-y-1 text-xs text-base-content/70">
      {items.map(([label, color]) => (
        <span key={label} className="inline-flex items-center gap-1.5">
          <span className="h-2.5 w-2.5 rounded-sm" style={{ backgroundColor: color }} />
          {label}
        </span>
      ))}
      <span className="inline-flex items-center gap-1.5">
        <span className="h-px w-5 bg-base-content/70" />
        {labels.duration}
      </span>
    </div>
  );
}

function renderFailureBar({
  x,
  y,
  width,
  height,
  fill,
}: {
  x?: number | string;
  y?: number | string;
  width?: number | string;
  height?: number | string;
  fill?: string;
}) {
  const values = [x, y, width, height].map(Number);
  if (values.some((value) => !Number.isFinite(value)) || Number(width) <= 0 || Number(height) === 0)
    return null;
  const [numericX, numericY, numericWidth, numericHeight] = values;
  const left = Math.min(numericX, numericX + numericWidth);
  const right = Math.max(numericX, numericX + numericWidth);
  const top = Math.min(numericY, numericY + numericHeight);
  const bottom = Math.max(numericY, numericY + numericHeight);
  const radius = Math.min(3, (right - left) / 2, (bottom - top) / 2);
  return (
    <path
      data-conversation-failure-bar-shape="negative"
      d={`M${left},${top} H${right} V${bottom - radius} Q${right},${bottom} ${right - radius},${bottom} H${left + radius} Q${left},${bottom} ${left},${bottom - radius} Z`}
      fill={fill}
      stroke="none"
    />
  );
}

function ConversationActivityTooltipContent({
  active,
  label,
  payload,
  renderValue,
}: {
  active?: boolean;
  label?: string | number;
  payload?: Array<{ payload?: ConversationActivityBucket }>;
  renderValue: RenderTooltip;
}) {
  const bucket = payload?.find((entry) => entry.payload)?.payload;
  if (!active || !bucket) return null;
  const rows = renderValue(bucket);
  const theme = resolveDocumentFloatingSurfaceTheme();
  return (
    <div
      role="tooltip"
      data-theme={theme}
      data-inline-chart-tooltip="true"
      className="min-w-[11rem] max-w-[14rem] rounded-xl border px-3 py-2 text-[11px] leading-tight text-base-content"
      style={floatingSurfaceStyle("neutral", theme)}
    >
      <div className="text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/60">
        {typeof label === "string" ? label : bucket.tooltipLabel}
      </div>
      <div className="mt-2 space-y-1.5">
        {rows.map((row) => (
          <div key={row.label} className="flex items-start gap-2">
            <span
              className="mt-[5px] h-1.5 w-1.5 shrink-0 rounded-full"
              style={{ backgroundColor: row.color }}
              aria-hidden="true"
            />
            <div className="min-w-0 flex-1">
              <div className="text-base-content/62">{row.label}</div>
              <div className="mt-0.5 font-mono text-[12px] font-semibold tracking-tight text-base-content">
                {row.value}
              </div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
