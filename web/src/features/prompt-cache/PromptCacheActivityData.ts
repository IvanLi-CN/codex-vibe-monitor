import type { FloatingSurfaceTheme } from "../../components/ui/floating-surface";
import type { ApiInvocation } from "../../lib/api";
import { resolvePromptCacheInvocationOutcome } from "../../lib/conversationRequestPoint";
import type { ThemeMode } from "../../theme";
import { FALLBACK_CELL } from "./keyedConversationChart";

export type ConversationActivityRange = "today" | "yesterday" | "1d" | "7d" | "history";
export type ConversationActivityMetric = "totalCount" | "totalCost" | "totalTokens";
export type ConversationActivityDragAxis = "pending" | "horizontal" | "vertical" | "free";
export const CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS = 30;
export const CONVERSATION_ACTIVITY_WHEEL_THRESHOLD = 2;
export const CONVERSATION_ACTIVITY_WHEEL_ZOOM_INTENSITY = 0.0018;
export const CONVERSATION_ACTIVITY_WHEEL_PAN_INTENSITY = 0.012;
export const CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_THRESHOLD_PX = 8;
export const CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_RATIO = 1.45;
export const CONVERSATION_ACTIVITY_POINTER_FREE_DIAGONAL_RATIO = 0.72;

export function resolveConversationActivityRange(range: ConversationActivityRange) {
  if (range === "history") return {};

  const now = new Date();
  if (range === "today") {
    return {
      from: startOfLocalDay(now).toISOString(),
      to: now.toISOString(),
    };
  }
  if (range === "yesterday") {
    const end = startOfLocalDay(now);
    const start = new Date(end);
    start.setDate(start.getDate() - 1);
    return {
      from: start.toISOString(),
      to: end.toISOString(),
    };
  }
  const durationMs = range === "7d" ? 7 * 86_400_000 : 86_400_000;
  return {
    from: new Date(now.getTime() - durationMs).toISOString(),
    to: now.toISOString(),
  };
}

function startOfLocalDay(value: Date) {
  const next = new Date(value);
  next.setHours(0, 0, 0, 0);
  return next;
}

export function formatCompactNumber(
  value: number | null | undefined,
  formatter: Intl.NumberFormat,
) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return formatter.format(value);
}

export function formatDurationMs(value: number | null | undefined, formatter: Intl.NumberFormat) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  const seconds = value / 1000;
  const maximumFractionDigits = Math.abs(seconds) >= 10 ? 1 : 2;
  return `${formatter.format(Number(seconds.toFixed(maximumFractionDigits)))} s`;
}

function getConversationActivityValue(record: ApiInvocation, metric: ConversationActivityMetric) {
  if (metric === "totalCost") return record.cost ?? 0;
  if (metric === "totalTokens") return record.totalTokens ?? 0;
  return 1;
}

export interface ConversationActivityBucket {
  index: number;
  label: string;
  tooltipLabel: string;
  success: number;
  failure: number;
  failureNegative: number;
  inFlight: number;
  neutral: number;
  totalCount: number;
  totalCost: number;
  totalTokens: number;
  totalMs: number;
  totalMsSamples: number;
  avgTotalMs: number | null;
}

export interface ConversationActivityBucketSet {
  buckets: ConversationActivityBucket[];
  rangeStartMs: number;
  rangeEndMs: number;
}

export function resolveDocumentThemeMode(): ThemeMode {
  if (typeof document === "undefined") return "light";
  const theme =
    document.body.getAttribute("data-theme") ??
    document.documentElement.getAttribute("data-theme") ??
    "";
  const normalizedTheme = theme.toLowerCase();
  if (normalizedTheme.includes("dark")) return "dark";
  if (normalizedTheme.includes("light")) return "light";
  const colorMode =
    document.body.getAttribute("data-color-mode") ??
    document.documentElement.getAttribute("data-color-mode");
  if (colorMode === "dark" || colorMode === "light") return colorMode;
  return "light";
}

export function resolveDocumentFloatingSurfaceTheme(): FloatingSurfaceTheme {
  return resolveDocumentThemeMode() === "dark" ? "vibe-dark" : "vibe-light";
}

export interface ConversationActivityViewport {
  startIndex: number;
  endIndex: number;
}

export function clampConversationActivityValue(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

export function normalizeConversationActivityViewport(
  viewport: ConversationActivityViewport,
  pointCount: number,
): ConversationActivityViewport {
  if (pointCount <= 0) {
    return { startIndex: 0, endIndex: 0 };
  }

  const maxIndex = pointCount - 1;
  const minSpan = Math.min(CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS, pointCount);
  const currentSpan = Math.max(
    minSpan,
    Math.min(pointCount, viewport.endIndex - viewport.startIndex + 1),
  );
  const startIndex = clampConversationActivityValue(
    Math.round(viewport.startIndex),
    0,
    Math.max(0, pointCount - currentSpan),
  );

  return {
    startIndex,
    endIndex: Math.min(maxIndex, startIndex + currentSpan - 1),
  };
}

export function shiftConversationActivityViewport(
  viewport: ConversationActivityViewport,
  pointCount: number,
  deltaIndexes: number,
): ConversationActivityViewport {
  const span = viewport.endIndex - viewport.startIndex + 1;
  return normalizeConversationActivityViewport(
    {
      startIndex: viewport.startIndex + deltaIndexes,
      endIndex: viewport.startIndex + deltaIndexes + span - 1,
    },
    pointCount,
  );
}

export function isSameConversationActivityViewport(
  left: ConversationActivityViewport,
  right: ConversationActivityViewport,
) {
  return left.startIndex === right.startIndex && left.endIndex === right.endIndex;
}

export function zoomConversationActivityViewport(
  viewport: ConversationActivityViewport,
  pointCount: number,
  zoomDelta: number,
  anchorRatio: number,
): ConversationActivityViewport {
  if (pointCount <= 0) return viewport;

  const currentSpan = viewport.endIndex - viewport.startIndex + 1;
  const nextSpan = clampConversationActivityValue(
    Math.round(currentSpan * Math.exp(zoomDelta)),
    Math.min(CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS, pointCount),
    pointCount,
  );
  const safeAnchorRatio = clampConversationActivityValue(anchorRatio, 0, 1);
  const anchorIndex = viewport.startIndex + (currentSpan - 1) * safeAnchorRatio;
  const nextStart = Math.round(anchorIndex - (nextSpan - 1) * safeAnchorRatio);

  return normalizeConversationActivityViewport(
    {
      startIndex: nextStart,
      endIndex: nextStart + nextSpan - 1,
    },
    pointCount,
  );
}

function resolveConversationActivityBounds(
  records: ApiInvocation[],
  range: ConversationActivityRange,
  rangeStartMs?: number | null,
  rangeEndMs?: number | null,
) {
  const now = new Date();
  const rangeBounds = resolveConversationActivityRange(range);
  let startMs =
    typeof rangeStartMs === "number" && Number.isFinite(rangeStartMs)
      ? rangeStartMs
      : rangeBounds.from
        ? Date.parse(rangeBounds.from)
        : Number.POSITIVE_INFINITY;
  let endMs =
    typeof rangeEndMs === "number" && Number.isFinite(rangeEndMs)
      ? rangeEndMs
      : rangeBounds.to
        ? Date.parse(rangeBounds.to)
        : Number.NEGATIVE_INFINITY;

  if (range === "history") {
    for (const record of records) {
      const occurredAt = Date.parse(record.occurredAt);
      if (!Number.isFinite(occurredAt)) continue;
      startMs = Math.min(startMs, occurredAt);
      endMs = Math.max(endMs, occurredAt);
    }
    if (!Number.isFinite(startMs) || !Number.isFinite(endMs)) {
      endMs = now.getTime();
      startMs = endMs - 86_400_000;
    }
  }

  if (!Number.isFinite(startMs) || !Number.isFinite(endMs) || endMs <= startMs) {
    if (
      range === "history" &&
      Number.isFinite(startMs) &&
      Number.isFinite(endMs) &&
      startMs === endMs
    ) {
      endMs = startMs + 60_000;
    } else {
      endMs = now.getTime();
      startMs = endMs - 86_400_000;
    }
  }
  return { startMs, endMs };
}

function createConversationActivityBuckets(
  startMs: number,
  endMs: number,
  range: ConversationActivityRange,
  localeTag: string,
) {
  const targetBuckets =
    endMs - startMs <= 86_400_000
      ? Math.ceil((endMs - startMs) / 60_000) + 1
      : range === "today" || range === "yesterday"
        ? 24
        : range === "1d"
          ? 24
          : 720;
  const bucketMs = Math.max(60_000, Math.ceil((endMs - startMs) / targetBuckets));
  const bucketCount = Math.max(1, Math.ceil((endMs - startMs) / bucketMs));
  const labelFormatter = new Intl.DateTimeFormat(localeTag, {
    month: range === "history" || range === "7d" ? "2-digit" : undefined,
    day: range === "history" || range === "7d" ? "2-digit" : undefined,
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    hourCycle: "h23",
  });
  const buckets: ConversationActivityBucket[] = Array.from({ length: bucketCount }, (_, index) => {
    const bucketStart = startMs + index * bucketMs;
    const label = labelFormatter.format(new Date(bucketStart));
    return {
      index,
      label,
      tooltipLabel: label,
      success: 0,
      failure: 0,
      failureNegative: 0,
      inFlight: 0,
      neutral: 0,
      totalCount: 0,
      totalCost: 0,
      totalTokens: 0,
      totalMs: 0,
      totalMsSamples: 0,
      avgTotalMs: null,
    };
  });
  return { buckets, bucketMs };
}

function populateConversationActivityBuckets(
  buckets: ConversationActivityBucket[],
  records: ApiInvocation[],
  startMs: number,
  endMs: number,
  bucketMs: number,
  metric: ConversationActivityMetric,
) {
  for (const record of records) {
    const occurredAt = Date.parse(record.occurredAt);
    if (!Number.isFinite(occurredAt) || occurredAt < startMs || occurredAt > endMs) continue;
    const index = Math.min(
      buckets.length - 1,
      Math.max(0, Math.floor((occurredAt - startMs) / bucketMs)),
    );
    const bucket = buckets[index];
    if (!bucket) continue;
    const metricValue = getConversationActivityValue(record, metric);
    const outcome = resolvePromptCacheInvocationOutcome(record);
    if (outcome === "success") bucket.success += metricValue;
    else if (outcome === "failure") bucket.failure += metricValue;
    else if (outcome === "in_flight") bucket.inFlight += metricValue;
    else bucket.neutral += metricValue;
    bucket.totalCount += 1;
    bucket.totalCost += record.cost ?? 0;
    bucket.totalTokens += record.totalTokens ?? 0;
    if (typeof record.tTotalMs === "number" && Number.isFinite(record.tTotalMs)) {
      bucket.totalMs += record.tTotalMs;
      bucket.totalMsSamples += 1;
    }
  }
}

function finalizeConversationActivityBuckets(buckets: ConversationActivityBucket[]) {
  for (const bucket of buckets) {
    bucket.failureNegative = bucket.failure > 0 ? -bucket.failure : 0;
    bucket.avgTotalMs = bucket.totalMsSamples > 0 ? bucket.totalMs / bucket.totalMsSamples : null;
  }
}

export function buildConversationActivityBuckets({
  records,
  range,
  metric,
  localeTag,
  rangeStartMs,
  rangeEndMs,
}: {
  records: ApiInvocation[];
  range: ConversationActivityRange;
  metric: ConversationActivityMetric;
  localeTag: string;
  rangeStartMs?: number | null;
  rangeEndMs?: number | null;
}): ConversationActivityBucketSet {
  const { startMs, endMs } = resolveConversationActivityBounds(
    records,
    range,
    rangeStartMs,
    rangeEndMs,
  );
  const { buckets, bucketMs } = createConversationActivityBuckets(startMs, endMs, range, localeTag);
  populateConversationActivityBuckets(buckets, records, startMs, endMs, bucketMs, metric);
  finalizeConversationActivityBuckets(buckets);

  return { buckets, rangeStartMs: startMs, rangeEndMs: endMs };
}
