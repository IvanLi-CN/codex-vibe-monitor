import {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type ReactElement,
} from "react";
import {
  Area,
  AreaChart,
  Bar,
  CartesianGrid,
  ComposedChart,
  Legend,
  Line,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { useTranslation } from "../i18n";
import {
  chartBaseTokens,
  chartStatusTokens,
  metricAccent,
  withOpacity,
} from "../lib/chartTheme";
import { formatTokensShort } from "../lib/numberFormatters";
import { useTheme } from "../theme";
import type { MetricKey } from "./Last24hTenMinuteHeatmap";
import { Alert } from "./ui/alert";
import type { TimeseriesResponse } from "../lib/api";
import {
  buildTodayMinuteChartData,
  type DashboardTodayMinuteDatum,
} from "./dashboardTodayActivityChartData";
import {
  recordTodayChartRender,
  useDashboardPerformanceDiagnosticsEnabled,
} from "../lib/dashboardPerformanceDiagnostics";

export interface DashboardTodayActivityChartProps {
  response: TimeseriesResponse | null;
  loading: boolean;
  error?: string | null;
  metric: MetricKey | "trend";
  closedNaturalDay?: boolean;
}

function buildChartRenderSignature({
  response,
  loading,
  error,
  metric,
  closedNaturalDay,
}: DashboardTodayActivityChartProps) {
  if (!response) {
    return JSON.stringify({
      metric,
      closedNaturalDay,
      loading,
      error: error ?? null,
      response: null,
    });
  }

  const aggregate = response.points.reduce(
    (current, point) => ({
      totalCount: current.totalCount + point.totalCount,
      failureCount: current.failureCount + point.failureCount,
      totalTokens: current.totalTokens + point.totalTokens,
      totalCost: current.totalCost + point.totalCost,
    }),
    {
      totalCount: 0,
      failureCount: 0,
      totalTokens: 0,
      totalCost: 0,
    },
  );
  const lastPoint = response.points.at(-1) ?? null;

  return JSON.stringify({
    metric,
    closedNaturalDay,
    loading,
    error: error ?? null,
    rangeStart: response.rangeStart,
    rangeEnd: response.rangeEnd,
    bucketSeconds: response.bucketSeconds,
    snapshotId: response.snapshotId ?? null,
    pointCount: response.points.length,
    aggregate,
    lastPoint,
  });
}

function formatCountValue(
  value: number,
  unitLabel: string,
  formatter: Intl.NumberFormat,
) {
  return `${formatter.format(value)} ${unitLabel}`;
}

const MIN_VISIBLE_MINUTES = 30;
const HORIZONTAL_WHEEL_THRESHOLD = 2;
const WHEEL_ZOOM_INTENSITY = 0.0018;
const WHEEL_PAN_INTENSITY = 0.012;
const POINTER_AXIS_LOCK_THRESHOLD_PX = 8;
const POINTER_AXIS_LOCK_RATIO = 1.45;
const POINTER_FREE_DIAGONAL_RATIO = 0.72;

interface ChartViewport {
  startIndex: number;
  endIndex: number;
}

type PointerDragAxis = "pending" | "horizontal" | "vertical" | "free";

function clampValue(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function normalizeViewport(
  viewport: ChartViewport,
  pointCount: number,
): ChartViewport {
  if (pointCount <= 0) {
    return { startIndex: 0, endIndex: 0 };
  }

  const maxIndex = pointCount - 1;
  const minSpan = Math.min(MIN_VISIBLE_MINUTES, pointCount);
  const currentSpan = Math.max(
    minSpan,
    Math.min(pointCount, viewport.endIndex - viewport.startIndex + 1),
  );
  const startIndex = clampValue(
    Math.round(viewport.startIndex),
    0,
    Math.max(0, pointCount - currentSpan),
  );

  return {
    startIndex,
    endIndex: Math.min(maxIndex, startIndex + currentSpan - 1),
  };
}

function shiftViewport(
  viewport: ChartViewport,
  pointCount: number,
  deltaIndexes: number,
): ChartViewport {
  const span = viewport.endIndex - viewport.startIndex + 1;
  return normalizeViewport(
    {
      startIndex: viewport.startIndex + deltaIndexes,
      endIndex: viewport.startIndex + deltaIndexes + span - 1,
    },
    pointCount,
  );
}

function isSameViewport(left: ChartViewport, right: ChartViewport) {
  return (
    left.startIndex === right.startIndex &&
    left.endIndex === right.endIndex
  );
}

function zoomViewport(
  viewport: ChartViewport,
  pointCount: number,
  zoomDelta: number,
  anchorRatio: number,
): ChartViewport {
  if (pointCount <= 0) return viewport;

  const currentSpan = viewport.endIndex - viewport.startIndex + 1;
  const nextSpan = clampValue(
    Math.round(currentSpan * Math.exp(zoomDelta)),
    Math.min(MIN_VISIBLE_MINUTES, pointCount),
    pointCount,
  );
  const safeAnchorRatio = clampValue(anchorRatio, 0, 1);
  const anchorIndex = viewport.startIndex + (currentSpan - 1) * safeAnchorRatio;
  const nextStart = Math.round(anchorIndex - (nextSpan - 1) * safeAnchorRatio);

  return normalizeViewport(
    {
      startIndex: nextStart,
      endIndex: nextStart + nextSpan - 1,
    },
    pointCount,
  );
}

interface TooltipPayloadEntry {
  payload?: DashboardTodayMinuteDatum;
}

interface FailureBarShapeProps {
  fill?: string;
  x?: number | string;
  y?: number | string;
  width?: number | string;
  height?: number | string;
}

function NegativeFailureBarShape({
  fill = "currentColor",
  x,
  y,
  width,
  height,
}: FailureBarShapeProps): ReactElement | null {
  const rectX = Number(x);
  const rectY = Number(y);
  const rectWidth = Number(width);
  const rectHeight = Number(height);

  if (
    !Number.isFinite(rectX) ||
    !Number.isFinite(rectY) ||
    !Number.isFinite(rectWidth) ||
    !Number.isFinite(rectHeight) ||
    rectWidth === 0 ||
    rectHeight === 0
  ) {
    return null;
  }

  const left = Math.min(rectX, rectX + rectWidth);
  const right = Math.max(rectX, rectX + rectWidth);
  const top = Math.min(rectY, rectY + rectHeight);
  const bottom = Math.max(rectY, rectY + rectHeight);
  const normalizedWidth = right - left;
  const normalizedHeight = bottom - top;
  const radius = Math.min(3, normalizedWidth / 2, normalizedHeight / 2);

  return (
    <path
      data-dashboard-failure-bar-shape="negative"
      d={[
        `M${left},${top}`,
        `H${right}`,
        `V${bottom - radius}`,
        `Q${right},${bottom} ${right - radius},${bottom}`,
        `H${left + radius}`,
        `Q${left},${bottom} ${left},${bottom - radius}`,
        "Z",
      ].join(" ")}
      fill={fill}
      stroke="none"
    />
  );
}

interface ChartTooltipContentProps {
  active?: boolean;
  label?: string | number;
  payload?: TooltipPayloadEntry[];
  theme: {
    tooltipBg: string;
    tooltipBorder: string;
    axisText: string;
    success: string;
    failure: string;
    accent: string;
    spend: string;
    firstByte: string;
  };
  renderValue: (
    point: DashboardTodayMinuteDatum,
  ) => Array<{ label: string; value: string; color: string }>;
}

function ChartTooltipContent({
  active,
  label,
  payload,
  theme,
  renderValue,
}: ChartTooltipContentProps) {
  const point = payload?.find((entry) => entry.payload)?.payload;
  if (!active || !point) return null;

  const rows = renderValue(point);
  if (rows.length === 0) return null;

  return (
    <div
      className="min-w-[180px] rounded-lg border px-3 py-2 shadow-lg"
      style={{
        backgroundColor: theme.tooltipBg,
        borderColor: theme.tooltipBorder,
        color: theme.axisText,
      }}
    >
      <div className="text-sm font-semibold">
        {typeof label === "string" ? label : point.tooltipLabel}
      </div>
      <div className="mt-2 space-y-1 text-xs">
        {rows.map((row) => (
          <div
            key={row.label}
            className="flex items-center justify-between gap-4"
          >
            <div className="flex items-center gap-2">
              <span
                className="inline-block h-2.5 w-2.5 rounded-full"
                style={{ backgroundColor: row.color }}
                aria-hidden="true"
              />
              <span>{row.label}</span>
            </div>
            <span className="font-medium">{row.value}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function DashboardTodayActivityChartImpl({
  response,
  loading,
  error,
  metric,
  closedNaturalDay = false,
}: DashboardTodayActivityChartProps) {
  const diagnosticsEnabled = useDashboardPerformanceDiagnosticsEnabled();
  const renderSignature = useMemo(
    () =>
      diagnosticsEnabled
        ? buildChartRenderSignature({
            response,
            loading,
            error,
            metric,
            closedNaturalDay,
          })
        : null,
    [
      closedNaturalDay,
      diagnosticsEnabled,
      error,
      loading,
      metric,
      response,
    ],
  );

  useEffect(() => {
    if (!diagnosticsEnabled || renderSignature == null) {
      return;
    }
    recordTodayChartRender(renderSignature);
  }, [diagnosticsEnabled, renderSignature]);

  const { t, locale } = useTranslation();
  const { themeMode } = useTheme();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag, { maximumFractionDigits: 2 }),
    [localeTag],
  );
  const currencyFormatter = useMemo(
    () =>
      new Intl.NumberFormat(localeTag, {
        style: "currency",
        currency: "USD",
        maximumFractionDigits: 4,
      }),
    [localeTag],
  );
  const chartColors = useMemo(() => {
    const base = chartBaseTokens(themeMode);
    const status = chartStatusTokens(themeMode);
    const accent = metric === "trend"
      ? metricAccent("totalTokens", themeMode)
      : metricAccent(metric, themeMode);
    const spend = metricAccent("totalCost", themeMode);
    return {
      ...base,
      success: status.success,
      successFill: withOpacity(status.success, 0.24),
      failure: status.failure,
      failureFill: withOpacity(status.failure, 0.24),
      accent,
      accentFill: withOpacity(accent, 0.22),
      spend,
      spendFill: withOpacity(spend, 0.18),
      firstByte: themeMode === "dark" ? "#cbd5e1" : "#475569",
    };
  }, [metric, themeMode]);

  const data = useMemo(
    () => buildTodayMinuteChartData(response, { localeTag, closedNaturalDay }),
    [closedNaturalDay, localeTag, response],
  );
  const [viewport, setViewport] = useState<ChartViewport>({
    startIndex: 0,
    endIndex: Math.max(0, data.length - 1),
  });
  const viewportRef = useRef<ChartViewport>(viewport);
  const viewportIdentity = `${closedNaturalDay ? "closed" : "live"}:${response?.rangeStart ?? "empty"}:${response?.bucketSeconds ?? "none"}`;
  const viewportIdentityRef = useRef(viewportIdentity);
  const interactionRef = useRef<HTMLDivElement | null>(null);
  const wheelListenerElementRef = useRef<HTMLDivElement | null>(null);
  const dragPreviewLayerRef = useRef<HTMLDivElement | null>(null);
  const dragRef = useRef<{
    pointerId: number;
    startClientX: number;
    startClientY: number;
    currentClientX: number;
    currentClientY: number;
    axis: PointerDragAxis;
    viewport: ChartViewport;
  } | null>(null);
  const dragPreviewOffsetRef = useRef(0);
  const dragPreviewFrameRef = useRef<number | null>(null);
  const wheelPanDeltaRef = useRef(0);
  const wheelPanFrameRef = useRef<number | null>(null);
  const wheelZoomDeltaRef = useRef(0);
  const wheelZoomAnchorRatioRef = useRef(0.5);
  const wheelZoomFrameRef = useRef<number | null>(null);

  useEffect(() => {
    viewportRef.current = viewport;
  }, [viewport]);

  useEffect(() => {
    setViewport((current) => {
      if (viewportIdentityRef.current !== viewportIdentity) {
        viewportIdentityRef.current = viewportIdentity;
        return normalizeViewport(
          { startIndex: 0, endIndex: Math.max(0, data.length - 1) },
          data.length,
        );
      }
      return normalizeViewport(current, data.length);
    });
  }, [data.length, viewportIdentity]);

  const countUnit = t("unit.calls");
  const countSeriesNames = useMemo(
    () => ({
      success: t("stats.cards.success"),
      failures: t("stats.cards.failures"),
      inFlight: t("chart.inFlight"),
      total: t("chart.totalCount"),
      firstByteTotal: t("chart.firstResponseByteTotal"),
    }),
    [t],
  );
  const areaSeriesName =
    metric === "totalCost" ? t("chart.totalCost") : t("chart.totalTokens");
  const trendSeriesNames = useMemo(
    () => ({
      tokensPerMinute: t("chart.tokensPerMinute"),
      spendRate: t("chart.spendRate"),
    }),
    [t],
  );
  const chartData =
    data.length > 0
      ? data
      : buildTodayMinuteChartData(response, { localeTag, closedNaturalDay });
  const visibleWindow = normalizeViewport(viewport, chartData.length);
  const visibleChartData = chartData.slice(
    visibleWindow.startIndex,
    visibleWindow.endIndex + 1,
  );
  const tenMinuteTrendData = chartData.filter(
    (point) =>
      point.chartTokensPerMinute != null || point.chartSpendRate != null,
  );
  const visibleTenMinuteTrendData = tenMinuteTrendData.filter(
    (point) =>
      point.index >= visibleWindow.startIndex &&
      point.index <= visibleWindow.endIndex,
  );
  const viewportSpan =
    visibleWindow.endIndex - visibleWindow.startIndex + 1;
  const isZoomed = chartData.length > 0 && viewportSpan < chartData.length;
  const xDomain: [number, number] = [
    visibleWindow.startIndex,
    visibleWindow.endIndex,
  ];
  const countBarSize = useMemo(() => {
    if (chartData.length <= 0) return 1;
    const zoomFactor = chartData.length / Math.max(1, viewportSpan);
    return clampValue(Math.round(zoomFactor * 0.75), 1, 10);
  }, [chartData.length, viewportSpan]);

  const countAxisBound = useMemo(() => {
    const maxValue = chartData.reduce(
      (current, item) =>
        Math.max(
          current,
          item.successCount + item.inFlightCount,
          item.failureCount,
        ),
      0,
    );
    return Math.max(1, maxValue);
  }, [chartData]);

  const getAnchorRatio = useCallback((clientX: number) => {
    const rect = interactionRef.current?.getBoundingClientRect();
    if (!rect || rect.width <= 0) return 0.5;
    return clampValue((clientX - rect.left) / rect.width, 0, 1);
  }, []);

  const scheduleWheelPan = useCallback(
    (deltaIndexes: number) => {
      wheelPanDeltaRef.current += deltaIndexes;
      if (wheelPanFrameRef.current != null) return;

      wheelPanFrameRef.current = window.requestAnimationFrame(() => {
        wheelPanFrameRef.current = null;
        const pendingDelta = wheelPanDeltaRef.current;
        wheelPanDeltaRef.current = 0;
        if (pendingDelta === 0) return;

        const roundedDelta =
          Math.round(pendingDelta) ||
          Math.sign(pendingDelta) *
            Math.max(1, Math.round(WHEEL_PAN_INTENSITY * MIN_VISIBLE_MINUTES));
        setViewport((current) => {
          const normalized = normalizeViewport(current, chartData.length);
          const next = shiftViewport(
            normalized,
            chartData.length,
            roundedDelta,
          );
          return isSameViewport(normalized, next) ? current : next;
        });
      });
    },
    [chartData.length],
  );

  const scheduleWheelZoom = useCallback(
    (deltaY: number, anchorRatio: number) => {
      wheelZoomDeltaRef.current += deltaY;
      wheelZoomAnchorRatioRef.current = anchorRatio;
      if (wheelZoomFrameRef.current != null) return;

      wheelZoomFrameRef.current = window.requestAnimationFrame(() => {
        wheelZoomFrameRef.current = null;
        const pendingDelta = wheelZoomDeltaRef.current;
        const pendingAnchorRatio = wheelZoomAnchorRatioRef.current;
        wheelZoomDeltaRef.current = 0;
        if (pendingDelta === 0) return;

        setViewport((current) => {
          const normalized = normalizeViewport(current, chartData.length);
          const next = zoomViewport(
            normalized,
            chartData.length,
            pendingDelta * WHEEL_ZOOM_INTENSITY,
            pendingAnchorRatio,
          );
          return isSameViewport(normalized, next) ? current : next;
        });
      });
    },
    [chartData.length],
  );

  useEffect(
    () => () => {
      if (wheelPanFrameRef.current != null) {
        window.cancelAnimationFrame(wheelPanFrameRef.current);
      }
      if (wheelZoomFrameRef.current != null) {
        window.cancelAnimationFrame(wheelZoomFrameRef.current);
      }
    },
    [],
  );

  const handleWheel = useCallback(
    (event: WheelEvent) => {
      if (chartData.length <= MIN_VISIBLE_MINUTES) return;

      const horizontalIntent =
        Math.abs(event.deltaX) >= HORIZONTAL_WHEEL_THRESHOLD &&
        Math.abs(event.deltaX) >= Math.abs(event.deltaY) &&
        !event.ctrlKey;
      const hasZoomIntent =
        event.ctrlKey ||
        event.metaKey ||
        event.altKey ||
        Math.abs(event.deltaY) >= HORIZONTAL_WHEEL_THRESHOLD;
      if (!horizontalIntent && !hasZoomIntent) return;

      event.preventDefault();
      if (horizontalIntent) {
        const normalized = normalizeViewport(viewportRef.current, chartData.length);
        const width = interactionRef.current?.getBoundingClientRect().width ?? 1;
        const span = normalized.endIndex - normalized.startIndex + 1;
        scheduleWheelPan((event.deltaX / Math.max(1, width)) * span);
        return;
      }

      scheduleWheelZoom(event.deltaY, getAnchorRatio(event.clientX));
    },
    [chartData.length, getAnchorRatio, scheduleWheelPan, scheduleWheelZoom],
  );

  const setInteractionLayerRef = useCallback(
    (node: HTMLDivElement | null) => {
      if (wheelListenerElementRef.current) {
        wheelListenerElementRef.current.removeEventListener("wheel", handleWheel);
        wheelListenerElementRef.current = null;
      }

      interactionRef.current = node;
      if (!node) return;

      node.addEventListener("wheel", handleWheel, { passive: false });
      wheelListenerElementRef.current = node;
    },
    [handleWheel],
  );

  const handlePointerDown = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      if (event.button !== 0 || chartData.length <= MIN_VISIBLE_MINUTES) return;
      dragPreviewOffsetRef.current = 0;
      if (dragPreviewLayerRef.current) {
        dragPreviewLayerRef.current.style.transform = "";
      }
      const normalized = normalizeViewport(viewport, chartData.length);
      dragRef.current = {
        pointerId: event.pointerId,
        startClientX: event.clientX,
        startClientY: event.clientY,
        currentClientX: event.clientX,
        currentClientY: event.clientY,
        axis: "pending",
        viewport: normalized,
      };
      event.currentTarget.setPointerCapture(event.pointerId);
    },
    [chartData.length, viewport],
  );

  const scheduleDragPreview = useCallback(() => {
    if (dragPreviewFrameRef.current != null) return;

    dragPreviewFrameRef.current = window.requestAnimationFrame(() => {
      dragPreviewFrameRef.current = null;
      const drag = dragRef.current;
      if (!drag) return;

      const previewOffsetPx = drag.currentClientX - drag.startClientX;
      if (previewOffsetPx === dragPreviewOffsetRef.current) return;
      dragPreviewOffsetRef.current = previewOffsetPx;
      if (dragPreviewLayerRef.current) {
        dragPreviewLayerRef.current.style.transform =
          previewOffsetPx === 0
            ? ""
            : `translate3d(${previewOffsetPx}px, 0, 0)`;
      }
    });
  }, []);

  const handlePointerMove = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      drag.currentClientX = event.clientX;
      drag.currentClientY = event.clientY;

      if (drag.axis === "pending") {
        const deltaX = Math.abs(drag.currentClientX - drag.startClientX);
        const deltaY = Math.abs(drag.currentClientY - drag.startClientY);
        const distance = Math.hypot(deltaX, deltaY);

        if (distance < POINTER_AXIS_LOCK_THRESHOLD_PX) return;

        if (deltaX >= deltaY * POINTER_AXIS_LOCK_RATIO) {
          drag.axis = "horizontal";
        } else if (deltaY >= deltaX * POINTER_AXIS_LOCK_RATIO) {
          drag.axis = "vertical";
          dragRef.current = null;
          if (event.currentTarget.hasPointerCapture(event.pointerId)) {
            event.currentTarget.releasePointerCapture(event.pointerId);
          }
          return;
        } else if (
          Math.min(deltaX, deltaY) >=
          Math.max(deltaX, deltaY) * POINTER_FREE_DIAGONAL_RATIO
        ) {
          drag.axis = "free";
        } else {
          return;
        }
      }

      if (drag.axis === "vertical") return;
      scheduleDragPreview();
    },
    [scheduleDragPreview],
  );

  const handlePointerEnd = useCallback(
    (event: ReactPointerEvent<HTMLDivElement>) => {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;

      if (drag.axis === "horizontal" || drag.axis === "free") {
        const width = interactionRef.current?.getBoundingClientRect().width ?? 1;
        const span = drag.viewport.endIndex - drag.viewport.startIndex + 1;
        const deltaIndexes = Math.round(
          ((drag.startClientX - drag.currentClientX) / Math.max(1, width)) * span,
        );
        setViewport((current) => {
          const next = shiftViewport(drag.viewport, chartData.length, deltaIndexes);
          return isSameViewport(normalizeViewport(current, chartData.length), next)
            ? current
            : next;
        });
      }

      dragRef.current = null;
      dragPreviewOffsetRef.current = 0;
      if (dragPreviewLayerRef.current) {
        dragPreviewLayerRef.current.style.transform = "";
      }
      if (event.currentTarget.hasPointerCapture(event.pointerId)) {
        event.currentTarget.releasePointerCapture(event.pointerId);
      }
    },
    [chartData.length],
  );

  useEffect(
    () => () => {
      if (dragPreviewFrameRef.current != null) {
        window.cancelAnimationFrame(dragPreviewFrameRef.current);
      }
    },
    [],
  );

  if (error) {
    return <Alert variant="error">{error}</Alert>;
  }

  if (loading && !response) {
    return (
      <div className="h-80 w-full animate-pulse rounded-xl border border-base-300/70 bg-base-200/55" />
    );
  }

  if (!loading && data.length === 0) {
    return <Alert>{t("chart.noDataRange")}</Alert>;
  }

  const animate = false;
  const chartMode =
    metric === "totalCount"
      ? "count-bars"
      : metric === "trend"
        ? "trend-area"
        : "cumulative-area";
  const renderCountTooltip = (point: DashboardTodayMinuteDatum) =>
    point.chartSuccessCount == null || point.chartFailureCountNegative == null
      ? []
      : [
          {
            label: countSeriesNames.success,
            value: formatCountValue(
              point.chartSuccessCount,
              countUnit,
              numberFormatter,
            ),
            color: chartColors.success,
          },
          {
            label: countSeriesNames.failures,
            value: formatCountValue(
              Math.abs(point.chartFailureCountNegative),
              countUnit,
              numberFormatter,
            ),
            color: chartColors.failure,
          },
          {
            label: countSeriesNames.inFlight,
            value: formatCountValue(
              point.inFlightCount,
              countUnit,
              numberFormatter,
            ),
            color: chartColors.accent,
          },
          {
            label: countSeriesNames.firstByteTotal,
            value:
              point.chartFirstResponseByteTotalAvgMs == null
                ? "-"
                : `${numberFormatter.format(point.chartFirstResponseByteTotalAvgMs)} ms`,
            color: chartColors.firstByte,
          },
        ];
  const renderAreaTooltip = (point: DashboardTodayMinuteDatum) => [
    ...(metric === "totalCost"
      ? point.cumulativeCost == null
        ? []
        : [
            {
              label: areaSeriesName,
              value: currencyFormatter.format(point.cumulativeCost),
              color: chartColors.accent,
            },
          ]
      : point.cumulativeTokens == null
        ? []
        : [
            {
              label: areaSeriesName,
              value: formatTokensShort(point.cumulativeTokens, localeTag),
              color: chartColors.accent,
            },
          ]),
  ];
  const renderTrendTooltip = (point: DashboardTodayMinuteDatum) => [
    ...(point.chartTokensPerMinute == null
      ? []
      : [
          {
            label: trendSeriesNames.tokensPerMinute,
            value: formatTokensShort(point.chartTokensPerMinute, localeTag),
            color: chartColors.accent,
          },
        ]),
    ...(point.chartSpendRate == null
      ? []
      : [
          {
            label: trendSeriesNames.spendRate,
            value: currencyFormatter.format(point.chartSpendRate),
            color: chartColors.spend,
          },
        ]),
  ];

  return (
    <section
      className="overscroll-x-contain rounded-xl border border-base-300/75 bg-base-200/40 p-4"
      data-testid="dashboard-today-activity-chart"
      data-chart-mode={chartMode}
      data-chart-metric={metric}
      data-visible-start-index={visibleWindow.startIndex}
      data-visible-end-index={visibleWindow.endIndex}
      data-visible-span={viewportSpan}
      data-zoomed={isZoomed ? "true" : "false"}
    >
      <div
        ref={setInteractionLayerRef}
        className="h-80 w-full cursor-grab touch-pan-y overflow-hidden overscroll-x-contain select-none active:cursor-grabbing"
        data-testid="dashboard-today-activity-chart-interaction-layer"
        data-chart-kind="dashboard-today-activity"
        data-min-visible-minutes={MIN_VISIBLE_MINUTES}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerEnd}
        onPointerCancel={handlePointerEnd}
        onLostPointerCapture={handlePointerEnd}
      >
        <div
          ref={dragPreviewLayerRef}
          data-testid="dashboard-today-activity-chart-drag-layer"
          className="h-full w-full will-change-transform"
          style={{ transform: undefined }}
        >
          <ResponsiveContainer>
          {metric === "totalCount" ? (
            <ComposedChart
              data={visibleChartData}
              margin={{ top: 12, right: 24, left: 0, bottom: 8 }}
              barGap="-100%"
              stackOffset="sign"
            >
              <CartesianGrid
                stroke={chartColors.gridLine}
                strokeDasharray="3 3"
              />
              <XAxis
                dataKey="index"
                type="number"
                domain={xDomain}
                minTickGap={28}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
                tickFormatter={(value: number) => {
                  const item =
                    chartData[
                      Math.max(
                        0,
                        Math.min(chartData.length - 1, Math.round(value)),
                      )
                    ];
                  return item?.label ?? String(value);
                }}
              />
              <YAxis
                yAxisId="count"
                domain={[-countAxisBound, countAxisBound]}
                allowDecimals={false}
                tickFormatter={(value) =>
                  numberFormatter.format(Math.abs(Number(value)))
                }
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
              />
              <YAxis
                yAxisId="latency"
                orientation="right"
                tickFormatter={(value) => `${numberFormatter.format(Number(value))}ms`}
                width={72}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
              />
              <Tooltip
                labelFormatter={(value) => {
                  const item =
                    chartData[
                      Math.max(
                        0,
                        Math.min(
                          chartData.length - 1,
                          Math.round(Number(value)),
                        ),
                      )
                    ];
                  return item?.tooltipLabel ?? String(value);
                }}
                content={(props) => (
                  <ChartTooltipContent
                    active={props.active}
                    label={props.label}
                    payload={
                      props.payload as unknown as
                        | TooltipPayloadEntry[]
                        | undefined
                    }
                    theme={chartColors}
                    renderValue={renderCountTooltip}
                  />
                )}
              />
              <Legend wrapperStyle={{ color: chartColors.axisText }} />
              <ReferenceLine yAxisId="count" y={0} stroke={chartColors.gridLine} />
              <Bar
                yAxisId="count"
                dataKey="chartSuccessCount"
                name={countSeriesNames.success}
                stackId="positive"
                fill={chartColors.success}
                barSize={countBarSize}
                radius={[0, 0, 0, 0]}
                isAnimationActive={animate}
              />
              <Bar
                yAxisId="count"
                dataKey="chartInFlightCount"
                name={countSeriesNames.inFlight}
                stackId="positive"
                fill={chartColors.accent}
                barSize={countBarSize}
                radius={[3, 3, 0, 0]}
                isAnimationActive={animate}
              />
              <Bar
                yAxisId="count"
                dataKey="chartFailureCountNegative"
                name={countSeriesNames.failures}
                stackId="positive"
                fill={chartColors.failure}
                barSize={countBarSize}
                shape={<NegativeFailureBarShape />}
                isAnimationActive={animate}
              />
              <Line
                yAxisId="latency"
                type="monotone"
                dataKey="chartFirstResponseByteTotalAvgMs"
                name={countSeriesNames.firstByteTotal}
                stroke={chartColors.firstByte}
                strokeOpacity={0.72}
                strokeWidth={1.25}
                dot={{
                  r: 1.25,
                  strokeWidth: 0,
                  fill: chartColors.firstByte,
                  fillOpacity: 0.72,
                }}
                connectNulls={false}
                isAnimationActive={animate}
              />
            </ComposedChart>
          ) : metric === "trend" ? (
            <ComposedChart
              data={visibleTenMinuteTrendData}
              margin={{ top: 12, right: 24, left: 0, bottom: 8 }}
            >
              <CartesianGrid
                stroke={chartColors.gridLine}
                strokeDasharray="3 3"
              />
              <XAxis
                dataKey="index"
                type="number"
                domain={xDomain}
                minTickGap={28}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
                tickFormatter={(value: number) => {
                  const item =
                    chartData[
                      Math.max(
                        0,
                        Math.min(chartData.length - 1, Math.round(value)),
                      )
                    ];
                  return item?.label ?? String(value);
                }}
              />
              <YAxis
                yAxisId="tokens"
                tickFormatter={(value) => formatTokensShort(Number(value), localeTag)}
                width={80}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
              />
              <YAxis
                yAxisId="spend"
                orientation="right"
                tickFormatter={(value) => currencyFormatter.format(Number(value))}
                width={90}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
              />
              <Tooltip
                labelFormatter={(value) => {
                  const item =
                    chartData[
                      Math.max(
                        0,
                        Math.min(
                          chartData.length - 1,
                          Math.round(Number(value)),
                        ),
                      )
                    ];
                  return item?.tooltipLabel ?? String(value);
                }}
                content={(props) => (
                  <ChartTooltipContent
                    active={props.active}
                    label={props.label}
                    payload={
                      props.payload as unknown as
                        | TooltipPayloadEntry[]
                        | undefined
                    }
                    theme={chartColors}
                    renderValue={renderTrendTooltip}
                  />
                )}
              />
              <Legend wrapperStyle={{ color: chartColors.axisText }} />
              <Area
                yAxisId="tokens"
                type="monotone"
                dataKey="chartTokensPerMinute"
                name={trendSeriesNames.tokensPerMinute}
                stroke={chartColors.accent}
                fill={chartColors.accentFill}
                fillOpacity={1}
                strokeWidth={2}
                dot={false}
                connectNulls={false}
                isAnimationActive={animate}
              />
              <Area
                yAxisId="spend"
                type="monotone"
                dataKey="chartSpendRate"
                name={trendSeriesNames.spendRate}
                stroke={chartColors.spend}
                fill={chartColors.spendFill}
                fillOpacity={1}
                strokeWidth={2}
                dot={false}
                connectNulls={false}
                isAnimationActive={animate}
              />
            </ComposedChart>
          ) : (
            <AreaChart
              data={visibleChartData}
              margin={{ top: 12, right: 24, left: 0, bottom: 8 }}
            >
              <CartesianGrid
                stroke={chartColors.gridLine}
                strokeDasharray="3 3"
              />
              <XAxis
                dataKey="index"
                type="number"
                domain={xDomain}
                minTickGap={28}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
                tickFormatter={(value: number) => {
                  const item =
                    chartData[
                      Math.max(
                        0,
                        Math.min(chartData.length - 1, Math.round(value)),
                      )
                    ];
                  return item?.label ?? String(value);
                }}
              />
              <YAxis
                tickFormatter={(value) =>
                  metric === "totalCost"
                    ? currencyFormatter.format(Number(value))
                    : formatTokensShort(Number(value), localeTag)
                }
                width={metric === "totalCost" ? 90 : 80}
                axisLine={{ stroke: chartColors.gridLine }}
                tickLine={{ stroke: chartColors.gridLine }}
                tick={{ fill: chartColors.axisText, fontSize: 12 }}
              />
              <Tooltip
                labelFormatter={(value) => {
                  const item =
                    chartData[
                      Math.max(
                        0,
                        Math.min(
                          chartData.length - 1,
                          Math.round(Number(value)),
                        ),
                      )
                    ];
                  return item?.tooltipLabel ?? String(value);
                }}
                content={(props) => (
                  <ChartTooltipContent
                    active={props.active}
                    label={props.label}
                    payload={
                      props.payload as unknown as
                        | TooltipPayloadEntry[]
                        | undefined
                    }
                    theme={chartColors}
                    renderValue={renderAreaTooltip}
                  />
                )}
              />
              <Area
                type="monotone"
                dataKey={
                  metric === "totalCost"
                    ? "chartCumulativeCost"
                    : "chartCumulativeTokens"
                }
                name={areaSeriesName}
                stroke={chartColors.accent}
                fill={chartColors.accentFill}
                fillOpacity={1}
                strokeWidth={2}
                isAnimationActive={animate}
              />
            </AreaChart>
          )}
          </ResponsiveContainer>
        </div>
      </div>
    </section>
  );
}

export const DashboardTodayActivityChart = memo(DashboardTodayActivityChartImpl);
