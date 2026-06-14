import {
  Fragment,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
} from "react";
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
import { useTranslation } from "../i18n";
import type {
  ApiInvocation,
  InvocationRecordsQuery,
  InvocationRecordsSummaryResponse,
  PromptCacheConversation,
  PromptCacheConversationBindingResponse,
  PromptCacheConversationBindingKind,
  PromptCacheConversationUpstreamAccount,
  PromptCacheConversationsResponse,
  UpstreamAccountSummary,
} from "../lib/api";
import {
  fetchInvocationRecords,
  fetchInvocationRecordsSummary,
  fetchPromptCacheConversationBinding,
  fetchUpstreamAccounts,
  updatePromptCacheConversationBinding,
} from "../lib/api";
import {
  chartBaseTokens,
  chartStatusTokens,
  metricAccent,
} from "../lib/chartTheme";
import { resolvePromptCacheInvocationOutcome } from "../lib/conversationRequestPoint";
import { mergeInvocationRecordCollections } from "../lib/invocationLiveMerge";
import { invocationStableKey } from "../lib/invocation";
import { buildInvocationFromPromptCachePreview } from "../lib/promptCacheLive";
import { subscribeToSse, subscribeToSseOpen } from "../lib/sse";
import type { ThemeMode } from "../theme";
import { AccountDetailDrawerShell } from "./AccountDetailDrawerShell";
import { AppIcon } from "./AppIcon";
import { InvocationTable } from "./InvocationTable";
import { ConversationSparkline } from "./KeyedConversationTable";
import { Button } from "./ui/button";
import {
  FALLBACK_CELL,
  findVisibleConversationChartMax,
} from "./keyedConversationChart";
import { Alert } from "./ui/alert";
import { floatingSurfaceStyle, type FloatingSurfaceTheme } from "./ui/floating-surface";
import { SegmentedControl, SegmentedControlItem } from "./ui/segmented-control";
import { SelectField } from "./ui/select-field";
import { Spinner } from "./ui/spinner";

interface PromptCacheConversationTableProps {
  stats: PromptCacheConversationsResponse | null;
  isLoading: boolean;
  error?: string | null;
  expandedPromptCacheKeys?: string[];
  onToggleExpandedPromptCacheKey?: (promptCacheKey: string) => void;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  keyColumnLabel?: string;
  emptyLabel?: string;
  historyQueryForConversationKey?: (
    conversationKey: string,
  ) => Partial<InvocationRecordsQuery>;
  historyRecordMatchesConversationKey?: (
    record: ApiInvocation,
    conversationKey: string,
  ) => boolean;
}

type ConversationHistoryQueryBuilder = NonNullable<
  PromptCacheConversationTableProps["historyQueryForConversationKey"]
>;
type ConversationHistoryRecordMatcher = NonNullable<
  PromptCacheConversationTableProps["historyRecordMatchesConversationKey"]
>;

const PROMPT_CACHE_NOW_TICK_MS = 30_000;
const PROMPT_CACHE_CHART_MAX_WINDOW_MS = 24 * 3_600_000;
const PROMPT_CACHE_HISTORY_PAGE_SIZE = 50;
const PROMPT_CACHE_ACTIVITY_PAGE_SIZE = 200;
const PROMPT_CACHE_ACTIVITY_MAX_CHART_RECORDS = 1_000;
const PROMPT_CACHE_HISTORY_RESYNC_THROTTLE_MS = 1_000;
const PROMPT_CACHE_ACTIVITY_RESYNC_THROTTLE_MS = 1_000;
const CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS = 30;
const CONVERSATION_ACTIVITY_WHEEL_THRESHOLD = 2;
const CONVERSATION_ACTIVITY_WHEEL_ZOOM_INTENSITY = 0.0018;
const CONVERSATION_ACTIVITY_WHEEL_PAN_INTENSITY = 0.012;
const CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_THRESHOLD_PX = 8;
const CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_RATIO = 1.45;
const CONVERSATION_ACTIVITY_POINTER_FREE_DIAGONAL_RATIO = 0.72;

type ConversationActivityRange = "today" | "yesterday" | "1d" | "7d" | "history";
type ConversationActivityMetric = "totalCount" | "totalCost" | "totalTokens";
type ConversationActivityDragAxis = "pending" | "horizontal" | "vertical" | "free";
type ConversationBindingDraftKind = PromptCacheConversationBindingKind;

const CONVERSATION_ACTIVITY_METRICS: Array<{
  key: ConversationActivityMetric;
  labelKey: string;
}> = [
  { key: "totalCount", labelKey: "metric.totalCount" },
  { key: "totalCost", labelKey: "metric.totalCost" },
  { key: "totalTokens", labelKey: "metric.totalTokens" },
];

function parseEpoch(raw?: string | null) {
  if (!raw) return null;
  const epoch = Date.parse(raw);
  return Number.isNaN(epoch) ? null : epoch;
}

function formatNumber(
  value: number | null | undefined,
  formatter: Intl.NumberFormat,
) {
  if (typeof value !== "number" || !Number.isFinite(value))
    return FALLBACK_CELL;
  return formatter.format(value);
}

function formatCurrency(
  value: number | null | undefined,
  formatter: Intl.NumberFormat,
) {
  if (typeof value !== "number" || !Number.isFinite(value))
    return FALLBACK_CELL;
  return formatter.format(value);
}

function formatDateLabel(raw: string, formatter: Intl.DateTimeFormat) {
  const value = new Date(raw);
  if (Number.isNaN(value.getTime())) return raw || FALLBACK_CELL;
  return formatter.format(value);
}

function conversationBindingAccountLabel(account: UpstreamAccountSummary) {
  const identity = account.email?.trim() || account.displayName.trim();
  const group = account.groupName?.trim();
  return group ? `${identity} · ${group}` : identity;
}

function accountCanBePromptCacheBindingTarget(account: UpstreamAccountSummary) {
  if (
    account.provider !== "codex" ||
    !account.enabled ||
    account.status !== "active"
  ) {
    return false;
  }
  if (account.kind === "api_key_codex") {
    return Boolean(account.maskedApiKey?.trim());
  }
  if (account.kind === "oauth_codex") {
    return account.hasRefreshToken !== false;
  }
  return true;
}

function currentBindingLabel(
  binding: PromptCacheConversationBindingResponse | null,
  t: (key: string, values?: Record<string, string | number>) => string,
) {
  if (!binding || binding.bindingKind === "none") {
    return t("live.conversations.drawer.binding.currentNone");
  }
  if (binding.bindingKind === "group" && binding.groupName) {
    return t("live.conversations.drawer.binding.currentGroup", {
      group: binding.groupName,
    });
  }
  if (
    binding.bindingKind === "upstreamAccount" &&
    binding.upstreamAccountId != null
  ) {
    return t("live.conversations.drawer.binding.currentAccount", {
      account: binding.upstreamAccountName || `#${binding.upstreamAccountId}`,
    });
  }
  return t("live.conversations.drawer.binding.currentNone");
}

function encryptedOwnerLabel(
  binding: PromptCacheConversationBindingResponse | null,
) {
  if (!binding?.hasEncryptedSessionOwner) return null;
  const accountLabel =
    binding.encryptedOwnerAccountName?.trim() ||
    (binding.encryptedOwnerAccountId != null
      ? `#${binding.encryptedOwnerAccountId}`
      : null);
  if (!accountLabel) return null;
  const groupLabel = binding.encryptedOwnerGroupName?.trim();
  return groupLabel ? `${accountLabel} · ${groupLabel}` : accountLabel;
}

function nextBindingWouldOverrideEncryptedOwner(
  binding: PromptCacheConversationBindingResponse | null,
  nextBindingKind: ConversationBindingDraftKind,
  nextBindingGroupName: string,
  nextBindingAccountId: string,
  bindingAccounts: UpstreamAccountSummary[],
) {
  if (!binding?.hasEncryptedSessionOwner) return false;
  if (nextBindingKind === "none") return true;
  if (nextBindingKind === "upstreamAccount") {
    const nextId = Number(nextBindingAccountId);
    return (
      Number.isFinite(nextId) && nextId !== binding.encryptedOwnerAccountId
    );
  }
  if (nextBindingKind === "group") {
    const targetGroup = nextBindingGroupName.trim();
    const ownerGroup = binding.encryptedOwnerGroupName?.trim() ?? "";
    if (targetGroup && ownerGroup && targetGroup === ownerGroup) return false;
    if (
      binding.encryptedOwnerAccountId != null &&
      bindingAccounts.some(
        (account) =>
          account.id === binding.encryptedOwnerAccountId &&
          (account.groupName?.trim() ?? "") === targetGroup,
      )
    ) {
      return false;
    }
    return true;
  }
  return false;
}

function resolveUpstreamAccountLabel(
  account: PromptCacheConversationUpstreamAccount,
  fallbackAccountLabel: (id: number) => string,
) {
  const trimmedName = account.upstreamAccountName?.trim();
  if (trimmedName) return trimmedName;
  if (
    typeof account.upstreamAccountId === "number" &&
    Number.isFinite(account.upstreamAccountId)
  ) {
    return fallbackAccountLabel(Math.trunc(account.upstreamAccountId));
  }
  return FALLBACK_CELL;
}

function canOpenPromptCacheUpstreamAccount(
  account: PromptCacheConversationUpstreamAccount,
) {
  return (
    typeof account.upstreamAccountId === "number" &&
    Number.isFinite(account.upstreamAccountId)
  );
}

function SummaryBlock({
  conversation,
  labels,
  numberFormatter,
  currencyFormatter,
}: {
  conversation: PromptCacheConversation;
  labels: {
    requestCount: string;
    totalTokens: string;
    totalCost: string;
  };
  numberFormatter: Intl.NumberFormat;
  currencyFormatter: Intl.NumberFormat;
}) {
  const items = [
    {
      label: labels.requestCount,
      value: formatNumber(conversation.requestCount, numberFormatter),
    },
    {
      label: labels.totalTokens,
      value: formatNumber(conversation.totalTokens, numberFormatter),
    },
    {
      label: labels.totalCost,
      value: formatCurrency(conversation.totalCost, currencyFormatter),
    },
  ];

  return (
    <div className="space-y-1.5">
      {items.map((item) => (
        <div
          key={item.label}
          className="flex items-center justify-between gap-3 text-[11px]"
        >
          <span className="text-base-content/60">{item.label}</span>
          <span className="text-right font-medium">{item.value}</span>
        </div>
      ))}
    </div>
  );
}

function UpstreamAccountsBlock({
  upstreamAccounts,
  labels,
  numberFormatter,
  currencyFormatter,
  fallbackAccountLabel,
  onOpenAccountDetail,
}: {
  upstreamAccounts: PromptCacheConversationUpstreamAccount[];
  labels: {
    requestCountCompact: string;
    totalTokensCompact: string;
  };
  numberFormatter: Intl.NumberFormat;
  currencyFormatter: Intl.NumberFormat;
  fallbackAccountLabel: (id: number) => string;
  onOpenAccountDetail?: (
    account: PromptCacheConversationUpstreamAccount,
  ) => void;
}) {
  if (upstreamAccounts.length === 0) {
    return (
      <div className="text-[11px] text-base-content/55">{FALLBACK_CELL}</div>
    );
  }

  return (
    <div className="space-y-1.5">
      {upstreamAccounts.slice(0, 3).map((account, index) => {
        const accountLabel = resolveUpstreamAccountLabel(
          account,
          fallbackAccountLabel,
        );
        const clickable = canOpenPromptCacheUpstreamAccount(account);

        return (
          <div
            key={`${account.upstreamAccountId ?? "unknown"}-${account.upstreamAccountName ?? "none"}-${index}`}
            className="grid grid-cols-[7.5rem_minmax(0,1fr)] items-center gap-x-2 text-[11px]"
          >
            {clickable ? (
              <button
                type="button"
                className="truncate text-left font-medium transition hover:text-primary hover:underline focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                onClick={() => onOpenAccountDetail?.(account)}
                title={accountLabel}
              >
                {accountLabel}
              </button>
            ) : (
              <span className="truncate font-medium">{accountLabel}</span>
            )}
            <span className="min-w-0 truncate text-base-content/62">
              {formatNumber(account.requestCount, numberFormatter)}{" "}
              {labels.requestCountCompact}
              {" · "}
              {labels.totalTokensCompact}{" "}
              {formatNumber(account.totalTokens, numberFormatter)}
              {" · "}
              {formatCurrency(account.totalCost, currencyFormatter)}
            </span>
          </div>
        );
      })}
    </div>
  );
}

function PromptCacheConversationInvocationTable({
  records,
  isLoading,
  error,
  emptyLabel,
  onOpenUpstreamAccount,
  scrollElement,
}: {
  records: ApiInvocation[];
  isLoading: boolean;
  error?: string | null;
  emptyLabel: string;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  scrollElement?: HTMLElement | null;
}) {
  const hasLoadedRecords = records.length > 0;

  if (hasLoadedRecords) {
    return (
      <div className="space-y-3">
        {error ? (
          <Alert variant="error">
            <span>{error}</span>
          </Alert>
        ) : null}
        <InvocationTable
          records={records}
          isLoading={false}
          error={null}
          emptyLabel={emptyLabel}
          onOpenUpstreamAccount={onOpenUpstreamAccount}
          scrollElement={scrollElement}
        />
      </div>
    );
  }

  return (
    <InvocationTable
      records={records}
      isLoading={isLoading}
      error={error}
      emptyLabel={emptyLabel}
      onOpenUpstreamAccount={onOpenUpstreamAccount}
      scrollElement={scrollElement}
    />
  );
}

function startOfLocalDay(value: Date) {
  const next = new Date(value);
  next.setHours(0, 0, 0, 0);
  return next;
}

function resolveConversationActivityRange(range: ConversationActivityRange) {
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

function buildConversationActivityQuery(
  conversationKey: string,
  range: ConversationActivityRange,
  historyQueryForConversationKey?: ConversationHistoryQueryBuilder,
): Partial<InvocationRecordsQuery> {
  const base = historyQueryForConversationKey?.(conversationKey) ?? {
    promptCacheKey: conversationKey,
  };
  const { page, pageSize, snapshotId, sortBy, sortOrder, signal, ...filters } =
    base;
  void page;
  void pageSize;
  void snapshotId;
  void sortBy;
  void sortOrder;
  void signal;
  return {
    ...filters,
    ...resolveConversationActivityRange(range),
  };
}

function formatCompactNumber(value: number | null | undefined, formatter: Intl.NumberFormat) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return formatter.format(value);
}

function formatDurationMs(value: number | null | undefined, formatter: Intl.NumberFormat) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  const seconds = value / 1000;
  const maximumFractionDigits = Math.abs(seconds) >= 10 ? 1 : 2;
  return `${formatter.format(Number(seconds.toFixed(maximumFractionDigits)))} s`;
}

function getConversationActivityValue(
  record: ApiInvocation,
  metric: ConversationActivityMetric,
) {
  if (metric === "totalCost") return record.cost ?? 0;
  if (metric === "totalTokens") return record.totalTokens ?? 0;
  return 1;
}

interface ConversationActivityBucket {
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

interface ConversationActivityBucketSet {
  buckets: ConversationActivityBucket[];
  rangeStartMs: number;
  rangeEndMs: number;
}

function resolveDocumentThemeMode(): ThemeMode {
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

function resolveDocumentFloatingSurfaceTheme(): FloatingSurfaceTheme {
  return resolveDocumentThemeMode() === "dark" ? "vibe-dark" : "vibe-light";
}

interface ConversationActivityViewport {
  startIndex: number;
  endIndex: number;
}

function clampConversationActivityValue(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function normalizeConversationActivityViewport(
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

function shiftConversationActivityViewport(
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

function isSameConversationActivityViewport(
  left: ConversationActivityViewport,
  right: ConversationActivityViewport,
) {
  return left.startIndex === right.startIndex && left.endIndex === right.endIndex;
}

function zoomConversationActivityViewport(
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

function buildConversationActivityBuckets({
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
  const buckets: ConversationActivityBucket[] = Array.from(
    { length: bucketCount },
    (_, index) => {
      const bucketStart = startMs + index * bucketMs;
      return {
        index,
        label: labelFormatter.format(new Date(bucketStart)),
        tooltipLabel: labelFormatter.format(new Date(bucketStart)),
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
    },
  );

  for (const record of records) {
    const occurredAt = Date.parse(record.occurredAt);
    if (!Number.isFinite(occurredAt) || occurredAt < startMs || occurredAt > endMs) {
      continue;
    }
    const index = Math.min(
      buckets.length - 1,
      Math.max(0, Math.floor((occurredAt - startMs) / bucketMs)),
    );
    const bucket = buckets[index];
    if (!bucket) continue;
    const outcome = resolvePromptCacheInvocationOutcome(record);
    const metricValue = getConversationActivityValue(record, metric);
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

  for (const bucket of buckets) {
    bucket.failureNegative = bucket.failure > 0 ? -bucket.failure : 0;
    bucket.avgTotalMs =
      bucket.totalMsSamples > 0 ? bucket.totalMs / bucket.totalMsSamples : null;
  }

  return { buckets, rangeStartMs: startMs, rangeEndMs: endMs };
}

interface ConversationActivityTooltipPayloadEntry {
  payload?: ConversationActivityBucket;
}

interface ConversationActivityBarShapeProps {
  x?: number | string;
  y?: number | string;
  width?: number | string;
  height?: number | string;
  fill?: string;
}

function renderAlignedFailureBarShape({
  x,
  y,
  width,
  height,
  fill,
}: ConversationActivityBarShapeProps) {
  const numericX = Number(x);
  const numericY = Number(y);
  const numericWidth = Number(width);
  const numericHeight = Number(height);
  if (
    !Number.isFinite(numericX) ||
    !Number.isFinite(numericY) ||
    !Number.isFinite(numericWidth) ||
    !Number.isFinite(numericHeight) ||
    numericWidth <= 0 ||
    numericHeight === 0
  ) {
    return null;
  }
  const left = Math.min(numericX, numericX + numericWidth);
  const right = Math.max(numericX, numericX + numericWidth);
  const top = Math.min(numericY, numericY + numericHeight);
  const bottom = Math.max(numericY, numericY + numericHeight);
  const normalizedWidth = right - left;
  const normalizedHeight = bottom - top;
  const radius = Math.min(3, normalizedWidth / 2, normalizedHeight / 2);

  return (
    <path
      data-conversation-failure-bar-shape="negative"
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

function ConversationActivityTooltipContent({
  active,
  label,
  payload,
  renderValue,
}: {
  active?: boolean;
  label?: string | number;
  payload?: ConversationActivityTooltipPayloadEntry[];
  renderValue: (
    bucket: ConversationActivityBucket,
  ) => Array<{ label: string; value: string; color: string }>;
}) {
  const bucket = payload?.find((entry) => entry.payload)?.payload;
  if (!active || !bucket) return null;

  const rows = renderValue(bucket);
  if (rows.length === 0) return null;
  const surfaceTheme = resolveDocumentFloatingSurfaceTheme();

  return (
    <div
      role="tooltip"
      data-theme={surfaceTheme}
      data-inline-chart-tooltip="true"
      className="min-w-[11rem] max-w-[14rem] rounded-xl border px-3 py-2 text-[11px] leading-tight text-base-content"
      style={{
        ...floatingSurfaceStyle("neutral", surfaceTheme),
      }}
    >
      <div className="text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/60">
        {typeof label === "string" ? label : bucket.tooltipLabel}
      </div>
      <div className="mt-2 space-y-1.5">
        {rows.map((row) => (
          <div
            key={row.label}
            className="flex items-start gap-2"
          >
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

function ConversationActivityChart({
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
  const [viewport, setViewport] = useState<ConversationActivityViewport>({
    startIndex: 0,
    endIndex: Math.max(0, buckets.length - 1),
  });
  const viewportRef = useRef<ConversationActivityViewport>(viewport);
  const viewportIdentity = `${buckets.length}:${buckets[0]?.tooltipLabel ?? "empty"}:${buckets.at(-1)?.tooltipLabel ?? "empty"}`;
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
    axis: ConversationActivityDragAxis;
    viewport: ConversationActivityViewport;
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
        return normalizeConversationActivityViewport(
          { startIndex: 0, endIndex: Math.max(0, buckets.length - 1) },
          buckets.length,
        );
      }
      return normalizeConversationActivityViewport(current, buckets.length);
    });
  }, [buckets.length, viewportIdentity]);

  const visibleWindow = normalizeConversationActivityViewport(
    viewport,
    buckets.length,
  );
  const visibleBuckets = buckets.slice(
    visibleWindow.startIndex,
    visibleWindow.endIndex + 1,
  );
  const visibleTotalCount = visibleBuckets.reduce(
    (sum, bucket) => sum + bucket.totalCount,
    0,
  );
  const viewportSpan = visibleWindow.endIndex - visibleWindow.startIndex + 1;
  const isZoomed = buckets.length > 0 && viewportSpan < buckets.length;
  const xDomain: [number, number] = [visibleWindow.startIndex, visibleWindow.endIndex];
  const barSize = useMemo(() => {
    if (buckets.length <= 0) return 1;
    const zoomFactor = buckets.length / Math.max(1, viewportSpan);
    const minimumReadableBarSize = buckets.length <= 60 ? 5 : 1;
    return clampConversationActivityValue(
      Math.round(zoomFactor * 0.75),
      minimumReadableBarSize,
      10,
    );
  }, [buckets.length, viewportSpan]);

  const getAnchorRatio = useCallback((clientX: number) => {
    const rect = interactionRef.current?.getBoundingClientRect();
    if (!rect || rect.width <= 0) return 0.5;
    return clampConversationActivityValue((clientX - rect.left) / rect.width, 0, 1);
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
            Math.max(
              1,
              Math.round(
                CONVERSATION_ACTIVITY_WHEEL_PAN_INTENSITY *
                  CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS,
              ),
            );
        setViewport((current) => {
          const normalized = normalizeConversationActivityViewport(
            current,
            buckets.length,
          );
          const next = shiftConversationActivityViewport(
            normalized,
            buckets.length,
            roundedDelta,
          );
          return isSameConversationActivityViewport(normalized, next)
            ? current
            : next;
        });
      });
    },
    [buckets.length],
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
          const normalized = normalizeConversationActivityViewport(
            current,
            buckets.length,
          );
          const next = zoomConversationActivityViewport(
            normalized,
            buckets.length,
            pendingDelta * CONVERSATION_ACTIVITY_WHEEL_ZOOM_INTENSITY,
            pendingAnchorRatio,
          );
          return isSameConversationActivityViewport(normalized, next)
            ? current
            : next;
        });
      });
    },
    [buckets.length],
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
      if (buckets.length <= CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS) return;

      const horizontalIntent =
        Math.abs(event.deltaX) >= CONVERSATION_ACTIVITY_WHEEL_THRESHOLD &&
        Math.abs(event.deltaX) >= Math.abs(event.deltaY) &&
        !event.ctrlKey;
      const hasZoomIntent =
        event.ctrlKey ||
        event.metaKey ||
        event.altKey;
      if (!horizontalIntent && !hasZoomIntent) return;

      event.preventDefault();
      if (horizontalIntent) {
        const normalized = normalizeConversationActivityViewport(
          viewportRef.current,
          buckets.length,
        );
        const width = interactionRef.current?.getBoundingClientRect().width ?? 1;
        const span = normalized.endIndex - normalized.startIndex + 1;
        scheduleWheelPan((event.deltaX / Math.max(1, width)) * span);
        return;
      }

      scheduleWheelZoom(event.deltaY, getAnchorRatio(event.clientX));
    },
    [buckets.length, getAnchorRatio, scheduleWheelPan, scheduleWheelZoom],
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
      if (
        event.button !== 0 ||
        buckets.length <= CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS
      ) {
        return;
      }
      dragPreviewOffsetRef.current = 0;
      if (dragPreviewLayerRef.current) {
        dragPreviewLayerRef.current.style.transform = "";
      }
      const normalized = normalizeConversationActivityViewport(
        viewport,
        buckets.length,
      );
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
    [buckets.length, viewport],
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

        if (distance < CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_THRESHOLD_PX) return;

        if (deltaX >= deltaY * CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_RATIO) {
          drag.axis = "horizontal";
        } else if (deltaY >= deltaX * CONVERSATION_ACTIVITY_POINTER_AXIS_LOCK_RATIO) {
          drag.axis = "vertical";
          dragRef.current = null;
          if (event.currentTarget.hasPointerCapture(event.pointerId)) {
            event.currentTarget.releasePointerCapture(event.pointerId);
          }
          return;
        } else if (
          Math.min(deltaX, deltaY) >=
          Math.max(deltaX, deltaY) *
            CONVERSATION_ACTIVITY_POINTER_FREE_DIAGONAL_RATIO
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
          ((drag.startClientX - drag.currentClientX) / Math.max(1, width)) *
            span,
        );
        setViewport((current) => {
          const next = shiftConversationActivityViewport(
            drag.viewport,
            buckets.length,
            deltaIndexes,
          );
          return isSameConversationActivityViewport(
            normalizeConversationActivityViewport(current, buckets.length),
            next,
          )
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
    [buckets.length],
  );

  useEffect(
    () => () => {
      if (dragPreviewFrameRef.current != null) {
        window.cancelAnimationFrame(dragPreviewFrameRef.current);
      }
    },
    [],
  );
  const chartColors = useMemo(() => {
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
  }, [themeMode]);
  const maxCount = Math.max(
    1,
    ...visibleBuckets.map((bucket) =>
      Math.max(bucket.success + bucket.inFlight + bucket.neutral, bucket.failure),
    ),
  );
  const formatMetricValue = (value: number) => {
    if (metric === "totalCost") return currencyFormatter.format(value);
    return numberFormatter.format(value);
  };
  const countUnit = t("unit.calls");
  const legendLabels = {
    success: t("live.conversations.activity.legendSuccess"),
    failure: t("live.conversations.activity.legendFailure"),
    inFlight: t("live.conversations.activity.legendInFlight"),
    neutral: t("live.conversations.activity.legendNeutral"),
    duration: t("table.details.firstResponseByteTotal"),
  };
  const renderTooltip = (bucket: ConversationActivityBucket) => [
    {
      label: legendLabels.success,
      value: `${formatMetricValue(bucket.success)} ${metric === "totalCount" ? countUnit : ""}`.trim(),
      color: chartColors.success,
    },
    {
      label: legendLabels.failure,
      value: `${formatMetricValue(bucket.failure)} ${metric === "totalCount" ? countUnit : ""}`.trim(),
      color: chartColors.failure,
    },
    {
      label: legendLabels.inFlight,
      value: `${formatMetricValue(bucket.inFlight)} ${metric === "totalCount" ? countUnit : ""}`.trim(),
      color: chartColors.inFlight,
    },
    {
      label: legendLabels.neutral,
      value: `${formatMetricValue(bucket.neutral)} ${metric === "totalCount" ? countUnit : ""}`.trim(),
      color: chartColors.neutral,
    },
    {
      label: legendLabels.duration,
      value:
        bucket.avgTotalMs == null
          ? "-"
          : `${numberFormatter.format(bucket.avgTotalMs)} ms`,
      color: chartColors.firstByte,
    },
  ];

  if (loading && buckets.length === 0) {
    return (
      <div className="flex h-80 items-center justify-center gap-2 rounded-xl border border-base-300/75 bg-base-200/40 text-sm text-base-content/60">
        <Spinner size="sm" aria-label={t("chart.loadingDetailed")} />
        <span>{t("chart.loadingDetailed")}</span>
      </div>
    );
  }

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
      data-chart-range-start={
        typeof rangeStartMs === "number" && Number.isFinite(rangeStartMs)
          ? new Date(rangeStartMs).toISOString()
          : undefined
      }
      data-chart-range-end={
        typeof rangeEndMs === "number" && Number.isFinite(rangeEndMs)
          ? new Date(rangeEndMs).toISOString()
          : undefined
      }
    >
      <div
        ref={setInteractionLayerRef}
        className="h-80 w-full cursor-grab touch-pan-y overflow-hidden overscroll-x-contain select-none active:cursor-grabbing"
        role="img"
        aria-label={t("live.conversations.activity.chartAria")}
        data-testid="conversation-activity-chart-interaction-layer"
        data-chart-kind="conversation-activity"
        data-min-visible-buckets={CONVERSATION_ACTIVITY_MIN_VISIBLE_BUCKETS}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerEnd}
        onPointerCancel={handlePointerEnd}
        onLostPointerCapture={handlePointerEnd}
      >
        <div
          ref={dragPreviewLayerRef}
          data-testid="conversation-activity-chart-drag-layer"
          className="h-full w-full will-change-transform"
          style={{ transform: undefined }}
        >
        <ResponsiveContainer width="100%" height={320}>
          <ComposedChart
            data={visibleBuckets}
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
                const bucket =
                  buckets[
                    Math.max(
                      0,
                      Math.min(buckets.length - 1, Math.round(value)),
                    )
                  ];
                return bucket?.label ?? String(value);
              }}
            />
            <YAxis
              yAxisId="count"
              domain={[-maxCount, maxCount]}
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
                const bucket =
                  buckets[
                    Math.max(
                      0,
                      Math.min(
                        buckets.length - 1,
                        Math.round(Number(value)),
                      ),
                    )
                  ];
                return bucket?.tooltipLabel ?? String(value);
              }}
              content={(props) => (
                <ConversationActivityTooltipContent
                  active={props.active}
                  label={props.label}
                  payload={
                    props.payload as unknown as
                      | ConversationActivityTooltipPayloadEntry[]
                      | undefined
                  }
                  renderValue={renderTooltip}
                />
              )}
            />
            <ReferenceLine yAxisId="count" y={0} stroke={chartColors.gridLine} />
            <Bar
              yAxisId="count"
              dataKey="failureNegative"
              name={legendLabels.failure}
              stackId="positive"
              fill={chartColors.failure}
              barSize={barSize}
              radius={[0, 0, 3, 3]}
              shape={(props: ConversationActivityBarShapeProps) =>
                renderAlignedFailureBarShape({
                  ...props,
                  fill: chartColors.failure,
                })
              }
              isAnimationActive={false}
            />
            <Bar
              yAxisId="count"
              dataKey="success"
              name={legendLabels.success}
              stackId="positive"
              fill={chartColors.success}
              barSize={barSize}
              radius={[0, 0, 0, 0]}
              isAnimationActive={false}
            />
            <Bar
              yAxisId="count"
              dataKey="inFlight"
              name={legendLabels.inFlight}
              stackId="positive"
              fill={chartColors.inFlight}
              barSize={barSize}
              radius={[0, 0, 0, 0]}
              isAnimationActive={false}
            />
            <Bar
              yAxisId="count"
              dataKey="neutral"
              name={legendLabels.neutral}
              stackId="positive"
              fill={chartColors.neutral}
              barSize={barSize}
              radius={[3, 3, 0, 0]}
              isAnimationActive={false}
            />
            <Line
              yAxisId="latency"
              type="monotone"
              dataKey="avgTotalMs"
              name={legendLabels.duration}
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
              isAnimationActive={false}
            />
          </ComposedChart>
        </ResponsiveContainer>
        </div>
        <div className="flex flex-wrap items-center justify-center gap-x-4 gap-y-1 text-xs text-base-content/70">
          <span className="inline-flex items-center gap-1.5">
            <span className="h-2.5 w-2.5 rounded-sm bg-success" />
            {legendLabels.success}
          </span>
          <span className="inline-flex items-center gap-1.5">
            <span className="h-2.5 w-2.5 rounded-sm bg-error" />
            {legendLabels.failure}
          </span>
          <span className="inline-flex items-center gap-1.5">
            <span
              className="h-2.5 w-2.5 rounded-sm"
              style={{ backgroundColor: chartColors.inFlight }}
            />
            {legendLabels.inFlight}
          </span>
          <span className="inline-flex items-center gap-1.5">
            <span
              className="h-2.5 w-2.5 rounded-sm"
              style={{ backgroundColor: chartColors.neutral }}
            />
            {legendLabels.neutral}
          </span>
          <span className="inline-flex items-center gap-1.5">
            <span className="h-px w-5 bg-base-content/70" />
            {legendLabels.duration}
          </span>
        </div>
      </div>
    </div>
  );
}

function PromptCacheConversationActivityOverview({
  open,
  conversationKey,
  disableLiveUpdates,
  historyQueryForConversationKey,
  historyRecordMatchesConversationKey,
  t,
}: {
  open: boolean;
  conversationKey: string | null;
  disableLiveUpdates: boolean;
  historyQueryForConversationKey?: ConversationHistoryQueryBuilder;
  historyRecordMatchesConversationKey?: ConversationHistoryRecordMatcher;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  const { locale } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const activeRange: ConversationActivityRange = "history";
  const [activeMetric, setActiveMetric] =
    useState<ConversationActivityMetric>("totalCount");
  const [summary, setSummary] =
    useState<InvocationRecordsSummaryResponse | null>(null);
  const [records, setRecords] = useState<ApiInvocation[]>([]);
  const [chartRangeStartMs, setChartRangeStartMs] = useState<number | null>(
    null,
  );
  const [chartRangeEndMs, setChartRangeEndMs] = useState<number | null>(null);
  const [chartTotal, setChartTotal] = useState(0);
  const [chartIsSampled, setChartIsSampled] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestSeqRef = useRef(0);
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastRefreshAtRef = useRef(0);
  const activeLoadControllerRef = useRef<AbortController | null>(null);
  const isLoadingRef = useRef(false);

  const numberFormatter = useMemo(
    () =>
      new Intl.NumberFormat(localeTag, {
        maximumFractionDigits: 2,
        notation: "compact",
      }),
    [localeTag],
  );
  const fullNumberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag, { maximumFractionDigits: 2 }),
    [localeTag],
  );
  const currencyFormatter = useMemo(
    () =>
      new Intl.NumberFormat(localeTag, {
        style: "currency",
        currency: "USD",
        minimumFractionDigits: 2,
        maximumFractionDigits: 4,
      }),
    [localeTag],
  );

  const load = useCallback(
    async ({ silent = false }: { silent?: boolean } = {}) => {
      if (!open || !conversationKey) return;
      const requestSeq = requestSeqRef.current + 1;
      requestSeqRef.current = requestSeq;
      activeLoadControllerRef.current?.abort();
      const controller = new AbortController();
      activeLoadControllerRef.current = controller;
      const filters = buildConversationActivityQuery(
        conversationKey,
        activeRange,
        historyQueryForConversationKey,
      );
      const shouldManageLoading = !silent || isLoadingRef.current;
      if (shouldManageLoading) {
        isLoadingRef.current = true;
        setIsLoading(true);
      }
      try {
        const summaryResponse = await fetchInvocationRecordsSummary({
          ...filters,
          signal: controller.signal,
        });
        if (requestSeq !== requestSeqRef.current) return;
        setSummary(summaryResponse);

        let page = 1;
        let snapshotId: number | undefined;
        let loaded: ApiInvocation[] = [];
        let totalRecords = 0;
        while (true) {
          const response = await fetchInvocationRecords({
            ...filters,
            page,
            pageSize: PROMPT_CACHE_ACTIVITY_PAGE_SIZE,
            sortBy: "occurredAt",
            sortOrder: "desc",
            ...(snapshotId != null ? { snapshotId } : {}),
            signal: controller.signal,
          });
          if (requestSeq !== requestSeqRef.current) return;
          snapshotId = response.snapshotId;
          totalRecords = response.total;
          loaded = [...loaded, ...response.records].slice(
            0,
            PROMPT_CACHE_ACTIVITY_MAX_CHART_RECORDS,
          );
          if (
            loaded.length >= response.total ||
            loaded.length >= PROMPT_CACHE_ACTIVITY_MAX_CHART_RECORDS ||
            response.records.length === 0
          ) {
            break;
          }
          page += 1;
        }
        if (requestSeq !== requestSeqRef.current) return;
        let startBoundaryMs = Number.POSITIVE_INFINITY;
        let endBoundaryMs = Number.NEGATIVE_INFINITY;
        for (const record of loaded) {
          const occurredAt = Date.parse(record.occurredAt);
          if (!Number.isFinite(occurredAt)) continue;
          startBoundaryMs = Math.min(startBoundaryMs, occurredAt);
          endBoundaryMs = Math.max(endBoundaryMs, occurredAt);
        }
        if (totalRecords > loaded.length && snapshotId != null) {
          const oldestPage = await fetchInvocationRecords({
            ...filters,
            page: Math.max(
              1,
              Math.ceil(totalRecords / PROMPT_CACHE_ACTIVITY_PAGE_SIZE),
            ),
            pageSize: PROMPT_CACHE_ACTIVITY_PAGE_SIZE,
            sortBy: "occurredAt",
            sortOrder: "desc",
            snapshotId,
            signal: controller.signal,
          });
          if (requestSeq !== requestSeqRef.current) return;
          for (const record of oldestPage.records) {
            const occurredAt = Date.parse(record.occurredAt);
            if (!Number.isFinite(occurredAt)) continue;
            startBoundaryMs = Math.min(startBoundaryMs, occurredAt);
            endBoundaryMs = Math.max(endBoundaryMs, occurredAt);
          }
        }
        setRecords(loaded);
        setChartRangeStartMs(
          Number.isFinite(startBoundaryMs) ? startBoundaryMs : null,
        );
        setChartRangeEndMs(Number.isFinite(endBoundaryMs) ? endBoundaryMs : null);
        setChartTotal(totalRecords);
        setChartIsSampled(loaded.length < totalRecords);
        setError(null);
      } catch (err) {
        if (requestSeq !== requestSeqRef.current) return;
        if (
          (err instanceof DOMException && err.name === "AbortError") ||
          (err instanceof Error && err.name === "AbortError")
        ) {
          return;
        }
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (requestSeq === requestSeqRef.current && shouldManageLoading) {
          isLoadingRef.current = false;
          setIsLoading(false);
        }
        if (activeLoadControllerRef.current === controller) {
          activeLoadControllerRef.current = null;
        }
      }
    },
    [conversationKey, historyQueryForConversationKey, open],
  );

  useEffect(() => {
    requestSeqRef.current += 1;
    activeLoadControllerRef.current?.abort();
    activeLoadControllerRef.current = null;
    if (refreshTimerRef.current) {
      clearTimeout(refreshTimerRef.current);
      refreshTimerRef.current = null;
    }
    if (!open || !conversationKey) {
      setSummary(null);
      setRecords([]);
      setChartRangeStartMs(null);
      setChartRangeEndMs(null);
      setChartTotal(0);
      setChartIsSampled(false);
      isLoadingRef.current = false;
      setIsLoading(false);
      setError(null);
      return;
    }
    setSummary(null);
    setRecords([]);
    setChartRangeStartMs(null);
    setChartRangeEndMs(null);
    setChartTotal(0);
    setChartIsSampled(false);
    isLoadingRef.current = false;
    setError(null);
    void load();
  }, [conversationKey, load, open]);

  const triggerRefresh = useCallback(() => {
    const now = Date.now();
    const delay = Math.max(
      0,
      PROMPT_CACHE_ACTIVITY_RESYNC_THROTTLE_MS -
        (now - lastRefreshAtRef.current),
    );
    const run = () => {
      refreshTimerRef.current = null;
      lastRefreshAtRef.current = Date.now();
      void load({ silent: true });
    };
    if (delay === 0) {
      if (refreshTimerRef.current) clearTimeout(refreshTimerRef.current);
      run();
      return;
    }
    if (refreshTimerRef.current) return;
    refreshTimerRef.current = setTimeout(run, delay);
  }, [load]);

  useEffect(() => {
    if (disableLiveUpdates) return;
    if (!open || !conversationKey) return;
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type !== "records") return;
      const matching = payload.records.some(
        (record) =>
          historyRecordMatchesConversationKey?.(record, conversationKey) ??
          record.promptCacheKey?.trim() === conversationKey,
      );
      if (!matching) return;
      triggerRefresh();
    });
    return unsubscribe;
  }, [
    conversationKey,
    disableLiveUpdates,
    historyRecordMatchesConversationKey,
    open,
    triggerRefresh,
  ]);

  useEffect(
    () => () => {
      requestSeqRef.current += 1;
      activeLoadControllerRef.current?.abort();
      activeLoadControllerRef.current = null;
      if (refreshTimerRef.current) clearTimeout(refreshTimerRef.current);
    },
    [],
  );

  const bucketSet = useMemo(
    () =>
      buildConversationActivityBuckets({
        records,
        range: activeRange,
        metric: activeMetric,
        localeTag,
        rangeStartMs: chartRangeStartMs,
        rangeEndMs: chartRangeEndMs,
      }),
    [activeMetric, chartRangeEndMs, chartRangeStartMs, localeTag, records],
  );

  const metrics = [
    {
      label: t("live.conversations.activity.metricRequests"),
      value: formatCompactNumber(summary?.totalCount, numberFormatter),
      toneClass: "text-primary",
    },
    {
      label: t("live.conversations.activity.metricSuccess"),
      value: formatCompactNumber(summary?.successCount, numberFormatter),
      toneClass: "text-success",
    },
    {
      label: t("live.conversations.activity.metricFailures"),
      value: formatCompactNumber(summary?.failureCount, numberFormatter),
      toneClass: "text-error",
    },
    {
      label: t("live.conversations.activity.metricAborts"),
      value: formatCompactNumber(summary?.exception.clientAbortCount, numberFormatter),
      toneClass: "text-warning",
    },
    {
      label: t("live.conversations.activity.metricTokens"),
      value: formatCompactNumber(summary?.token.totalTokens, numberFormatter),
      toneClass: "text-info",
    },
    {
      label: t("live.conversations.activity.metricCost"),
      value:
        summary == null
          ? FALLBACK_CELL
          : currencyFormatter.format(summary.token.totalCost),
      toneClass: "text-primary",
    },
    {
      label: t("live.conversations.activity.metricAvgDuration"),
      value: formatDurationMs(summary?.network.avgTotalMs, fullNumberFormatter),
      toneClass: "text-base-content",
    },
  ];

  return (
    <section className="space-y-3 rounded-xl border border-base-300/70 bg-base-100/55 p-3">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <h3 className="text-sm font-semibold">
          {t("live.conversations.activity.title")}
        </h3>
        <SegmentedControl
          size="compact"
          role="tablist"
          aria-label={t("heatmap.metricsToggleAria")}
        >
          {CONVERSATION_ACTIVITY_METRICS.map((metric) => (
            <SegmentedControlItem
              key={metric.key}
              active={activeMetric === metric.key}
              role="tab"
              aria-selected={activeMetric === metric.key}
              onClick={() => setActiveMetric(metric.key)}
            >
              {t(metric.labelKey)}
            </SegmentedControlItem>
          ))}
        </SegmentedControl>
      </div>
      {error ? (
        <Alert variant="error">
          <span>{t("records.summary.loadError", { error })}</span>
        </Alert>
      ) : null}
      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4 xl:grid-cols-7">
        {metrics.map((metric) => (
          <div
            key={metric.label}
            className="rounded-lg border border-base-300/60 bg-base-200/25 px-3 py-2"
          >
            <div className="text-[11px] font-semibold uppercase tracking-[0.12em] text-base-content/55">
              {metric.label}
            </div>
            <div className={`mt-1 text-lg font-semibold ${metric.toneClass}`}>
              {isLoading && summary == null ? "…" : metric.value}
            </div>
          </div>
        ))}
      </div>
      <ConversationActivityChart
        buckets={bucketSet.buckets}
        rangeStartMs={bucketSet.rangeStartMs}
        rangeEndMs={bucketSet.rangeEndMs}
        metric={activeMetric}
        loading={isLoading}
        numberFormatter={numberFormatter}
        currencyFormatter={currencyFormatter}
        t={t}
      />
      {chartIsSampled ? (
        <p className="text-xs text-base-content/60">
          {t("live.conversations.activity.sampledChart", {
            loaded: formatCompactNumber(records.length, fullNumberFormatter),
            total: formatCompactNumber(chartTotal, fullNumberFormatter),
          })}
        </p>
      ) : null}
    </section>
  );
}

export function PromptCacheConversationHistoryDrawer({
  open,
  conversationKey,
  conversationLabel,
  disableLiveUpdates = false,
  onClose,
  t,
  onOpenUpstreamAccount,
  historyQueryForConversationKey,
  historyRecordMatchesConversationKey,
}: {
  open: boolean;
  conversationKey: string | null;
  conversationLabel?: string | null;
  disableLiveUpdates?: boolean;
  onClose: () => void;
  t: (key: string, values?: Record<string, string | number>) => string;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  historyQueryForConversationKey?: ConversationHistoryQueryBuilder;
  historyRecordMatchesConversationKey?: ConversationHistoryRecordMatcher;
}) {
  const titleId = useId();
  const requestSeqRef = useRef(0);
  const hasHydratedRef = useRef(false);
  const inFlightRef = useRef(false);
  const pendingLoadRef = useRef<{ silent?: boolean; append?: boolean } | null>(null);
  const pendingOpenResyncRef = useRef(false);
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastRefreshAtRef = useRef(0);
  const activeLoadControllerRef = useRef<AbortController | null>(null);
  const historySnapshotIdRef = useRef<number | undefined>(undefined);
  const historyNextPageRef = useRef(1);
  const historyHasMoreRef = useRef(false);
  const recordsRef = useRef<ApiInvocation[]>([]);
  const [drawerBodyElement, setDrawerBodyElement] = useState<HTMLDivElement | null>(null);
  const [records, setRecords] = useState<ApiInvocation[]>([]);
  const [liveRecords, setLiveRecords] = useState<ApiInvocation[]>([]);
  const [total, setTotal] = useState(0);
  const [isLoading, setIsLoading] = useState(false);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [binding, setBinding] =
    useState<PromptCacheConversationBindingResponse | null>(null);
  const [bindingKind, setBindingKind] =
    useState<ConversationBindingDraftKind>("none");
  const [bindingGroupName, setBindingGroupName] = useState("");
  const [bindingAccountId, setBindingAccountId] = useState("");
  const [bindingAccounts, setBindingAccounts] = useState<
    UpstreamAccountSummary[]
  >([]);
  const [bindingGroups, setBindingGroups] = useState<string[]>([]);
  const [bindingLoading, setBindingLoading] = useState(false);
  const [bindingSaving, setBindingSaving] = useState(false);
  const [bindingError, setBindingError] = useState<string | null>(null);

  const clearPendingRefreshTimer = useCallback(() => {
    if (!refreshTimerRef.current) return;
    clearTimeout(refreshTimerRef.current);
    refreshTimerRef.current = null;
  }, []);

  useEffect(() => {
    recordsRef.current = records;
  }, [records]);

  const runLoad = useCallback(
    async ({
      silent = false,
      append = false,
    }: { silent?: boolean; append?: boolean } = {}) => {
      if (!open || !conversationKey) return;
      if (append && !historyHasMoreRef.current) return;

      inFlightRef.current = true;
      const requestSeq = requestSeqRef.current + 1;
      requestSeqRef.current = requestSeq;
      activeLoadControllerRef.current?.abort();
      const controller = new AbortController();
      activeLoadControllerRef.current = controller;
      const shouldShowLoading = !append && !(silent && hasHydratedRef.current);
      if (shouldShowLoading) setIsLoading(true);
      if (append) setIsLoadingMore(true);
      try {
        const historyFilters = historyQueryForConversationKey?.(
          conversationKey,
        ) ?? {
          promptCacheKey: conversationKey,
        };
        const page = append ? historyNextPageRef.current : 1;
        const response = await fetchInvocationRecords({
          ...historyFilters,
          page,
          pageSize: PROMPT_CACHE_HISTORY_PAGE_SIZE,
          sortBy: "occurredAt",
          sortOrder: "desc",
          ...(append && historySnapshotIdRef.current != null
            ? { snapshotId: historySnapshotIdRef.current }
            : {}),
          signal: controller.signal,
        });
        if (requestSeq !== requestSeqRef.current) return;

        const previousSnapshotId = historySnapshotIdRef.current;
        const previousNextPage = historyNextPageRef.current;
        const snapshotChanged =
          silent &&
          hasHydratedRef.current &&
          previousSnapshotId != null &&
          response.snapshotId !== previousSnapshotId;
        historySnapshotIdRef.current = response.snapshotId;
        const loaded =
          snapshotChanged
            ? mergeInvocationRecordCollections(
                response.records,
                recordsRef.current,
              ).slice(
                0,
                recordsRef.current.length + PROMPT_CACHE_HISTORY_PAGE_SIZE,
              )
            : append
            ? mergeInvocationRecordCollections(recordsRef.current, response.records)
            : silent && hasHydratedRef.current
              ? mergeInvocationRecordCollections(
                  response.records,
                  recordsRef.current,
                ).slice(
                  0,
                  Math.max(recordsRef.current.length, response.records.length),
                )
              : response.records;
        recordsRef.current = loaded;
        historyNextPageRef.current =
          snapshotChanged
            ? 2
            : append || !silent || !hasHydratedRef.current
            ? page + 1
            : Math.max(
                previousNextPage,
                Math.floor(loaded.length / PROMPT_CACHE_HISTORY_PAGE_SIZE) + 1,
              );
        historyHasMoreRef.current =
          loaded.length < response.total &&
          (append ? response.records.length > 0 : loaded.length > 0);
        setRecords(loaded);
        setTotal(response.total);

        if (requestSeq !== requestSeqRef.current) return;
        hasHydratedRef.current = true;
        const loadedStableKeys = new Set(loaded.map(invocationStableKey));
        setLiveRecords((current) =>
          current.filter(
            (record) => !loadedStableKeys.has(invocationStableKey(record)),
          ),
        );
        setError(null);
        if (pendingOpenResyncRef.current) {
          pendingOpenResyncRef.current = false;
          const pendingSilent = pendingLoadRef.current?.silent ?? true;
          pendingLoadRef.current = { silent: pendingSilent };
        }
      } catch (err) {
        if (requestSeq !== requestSeqRef.current) return;
        if (
          (err instanceof DOMException && err.name === "AbortError") ||
          (err instanceof Error && err.name === "AbortError")
        ) {
          return;
        }
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (requestSeq === requestSeqRef.current && shouldShowLoading) {
          setIsLoading(false);
        }
        if (requestSeq === requestSeqRef.current && append) {
          setIsLoadingMore(false);
        }
        if (requestSeq === requestSeqRef.current) {
          inFlightRef.current = false;
        }
        const pendingLoad = pendingLoadRef.current;
        if (requestSeq === requestSeqRef.current && pendingLoad) {
          pendingLoadRef.current = null;
          void runLoad(pendingLoad);
        }
      }
    },
    [conversationKey, historyQueryForConversationKey, open],
  );

  const load = useCallback(
    async (options: { silent?: boolean; append?: boolean } = {}) => {
      const silent = options.silent ?? false;
      const append = options.append ?? false;
      if (inFlightRef.current) {
        const pendingSilent = pendingLoadRef.current?.silent ?? true;
        pendingLoadRef.current = {
          silent: pendingSilent && silent,
          append: pendingLoadRef.current?.append || append,
        };
        return;
      }
      await runLoad({ silent, append });
    },
    [runLoad],
  );

  const triggerSseRefresh = useCallback(() => {
    const now = Date.now();
    const delay = Math.max(
      0,
      PROMPT_CACHE_HISTORY_RESYNC_THROTTLE_MS -
        (now - lastRefreshAtRef.current),
    );
    const run = () => {
      refreshTimerRef.current = null;
      lastRefreshAtRef.current = Date.now();
      void load({ silent: true });
    };
    if (delay === 0) {
      clearPendingRefreshTimer();
      run();
      return;
    }
    if (refreshTimerRef.current) return;
    refreshTimerRef.current = setTimeout(run, delay);
  }, [clearPendingRefreshTimer, load]);

  const triggerOpenResync = useCallback(
    (force = false) => {
      if (!hasHydratedRef.current) {
        pendingOpenResyncRef.current = true;
        return;
      }
      const now = Date.now();
      if (
        !force &&
        now - lastRefreshAtRef.current < PROMPT_CACHE_HISTORY_RESYNC_THROTTLE_MS
      ) {
        return;
      }
      lastRefreshAtRef.current = now;
      void load({ silent: true });
    },
    [load],
  );

  useEffect(() => {
    requestSeqRef.current += 1;
    hasHydratedRef.current = false;
    inFlightRef.current = false;
    pendingLoadRef.current = null;
    pendingOpenResyncRef.current = false;
    lastRefreshAtRef.current = 0;
    activeLoadControllerRef.current?.abort();
    activeLoadControllerRef.current = null;
    historySnapshotIdRef.current = undefined;
    historyNextPageRef.current = 1;
    historyHasMoreRef.current = false;
    recordsRef.current = [];
    clearPendingRefreshTimer();

    if (!open || !conversationKey) {
      setRecords([]);
      setLiveRecords([]);
      setTotal(0);
      setIsLoading(false);
      setIsLoadingMore(false);
      setError(null);
      return;
    }

    setRecords([]);
    setLiveRecords([]);
    setTotal(0);
    setIsLoading(false);
    setIsLoadingMore(false);
    setError(null);
    void load();
  }, [clearPendingRefreshTimer, conversationKey, load, open]);

  useEffect(() => {
    if (!open || !conversationKey) {
      setBinding(null);
      setBindingKind("none");
      setBindingGroupName("");
      setBindingAccountId("");
      setBindingAccounts([]);
      setBindingGroups([]);
      setBindingLoading(false);
      setBindingSaving(false);
      setBindingError(null);
      return;
    }

    const controller = new AbortController();
    setBindingLoading(true);
    setBindingError(null);
    void Promise.all([
      fetchPromptCacheConversationBinding(conversationKey, controller.signal),
      fetchUpstreamAccounts({ includeAll: true, pageSize: 500 }),
    ])
      .then(([nextBinding, accountList]) => {
        if (controller.signal.aborted) return;
        const accounts = accountList.items.filter(
          accountCanBePromptCacheBindingTarget,
        );
        const groups = Array.from(
          new Set(
            accounts
              .map((account) => account.groupName ?? "")
              .map((groupName) => groupName.trim())
              .filter((groupName) => groupName.length > 0),
          ),
        ).sort((left, right) => left.localeCompare(right));
        setBinding(nextBinding);
        setBindingKind(nextBinding.bindingKind);
        setBindingGroupName(nextBinding.groupName ?? groups[0] ?? "");
        setBindingAccountId(
          nextBinding.upstreamAccountId != null
            ? String(nextBinding.upstreamAccountId)
            : accounts[0]
              ? String(accounts[0].id)
              : "",
        );
        setBindingAccounts(accounts);
        setBindingGroups(groups);
      })
      .catch((err) => {
        if (controller.signal.aborted) return;
        setBindingError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!controller.signal.aborted) setBindingLoading(false);
      });

    return () => controller.abort();
  }, [conversationKey, open]);

  useEffect(() => {
    if (disableLiveUpdates) return;
    if (!open || !conversationKey) return;
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type !== "records") return;
      const matching = payload.records.filter(
        (record) =>
          historyRecordMatchesConversationKey?.(record, conversationKey) ??
          record.promptCacheKey?.trim() === conversationKey,
      );
      if (matching.length === 0) return;
      setLiveRecords((current) =>
        mergeInvocationRecordCollections(matching, current).slice(
          0,
          PROMPT_CACHE_HISTORY_PAGE_SIZE,
        ),
      );
      triggerSseRefresh();
    });
    return unsubscribe;
  }, [
    conversationKey,
    disableLiveUpdates,
    historyRecordMatchesConversationKey,
    open,
    triggerSseRefresh,
  ]);

  useEffect(() => {
    if (disableLiveUpdates) return;
    if (!open) return;
    const unsubscribe = subscribeToSseOpen(() => {
      triggerOpenResync(true);
    });
    return unsubscribe;
  }, [disableLiveUpdates, open, triggerOpenResync]);

  useEffect(
    () => () => {
      activeLoadControllerRef.current?.abort();
      clearPendingRefreshTimer();
      pendingLoadRef.current = null;
      pendingOpenResyncRef.current = false;
    },
    [clearPendingRefreshTimer],
  );

  useEffect(() => {
    if (!open || !drawerBodyElement) return;
    const maybeLoadMore = () => {
      if (
        isLoading ||
        isLoadingMore ||
        inFlightRef.current ||
        !historyHasMoreRef.current
      ) {
        return;
      }
      const remaining =
        drawerBodyElement.scrollHeight -
        drawerBodyElement.scrollTop -
        drawerBodyElement.clientHeight;
      if (remaining <= 420) {
        void load({ append: true, silent: true });
      }
    };
    drawerBodyElement.addEventListener("scroll", maybeLoadMore, {
      passive: true,
    });
    return () => {
      drawerBodyElement.removeEventListener("scroll", maybeLoadMore);
    };
  }, [drawerBodyElement, isLoading, isLoadingMore, load, open, records.length]);

  const visibleRecords = useMemo(
    () => mergeInvocationRecordCollections(liveRecords, records),
    [liveRecords, records],
  );
  const displayTitle = conversationLabel?.trim() || conversationKey || FALLBACK_CELL;
  const shouldShowConversationKey =
    Boolean(conversationLabel?.trim()) &&
    Boolean(conversationKey?.trim()) &&
    conversationLabel?.trim() !== conversationKey?.trim();
  const effectiveTotal = useMemo(() => {
    const loadedStableKeys = new Set(records.map(invocationStableKey));
    const optimisticCount = liveRecords.reduce(
      (count, record) =>
        count + (loadedStableKeys.has(invocationStableKey(record)) ? 0 : 1),
      0,
    );
    return total + optimisticCount;
  }, [liveRecords, records, total]);
  const loadedCount = visibleRecords.length;
  const bindingSubmitDisabled =
    !conversationKey ||
    !binding ||
    bindingLoading ||
    bindingSaving ||
    (bindingKind === "group" && !bindingGroupName) ||
    (bindingKind === "upstreamAccount" && !bindingAccountId);
  const bindingStatusLabel = currentBindingLabel(binding, t);
  const encryptedOwnerStatusLabel = encryptedOwnerLabel(binding);
  const bindingKindOptions = [
    {
      value: "none",
      label: t("live.conversations.drawer.binding.kindNone"),
    },
    {
      value: "group",
      label: t("live.conversations.drawer.binding.kindGroup"),
      disabled: bindingGroups.length === 0,
    },
    {
      value: "upstreamAccount",
      label: t("live.conversations.drawer.binding.kindAccount"),
      disabled: bindingAccounts.length === 0,
    },
  ];
  const saveBinding = useCallback(async () => {
    if (!conversationKey || bindingSubmitDisabled) return;
    if (
      nextBindingWouldOverrideEncryptedOwner(
        binding,
        bindingKind,
        bindingGroupName,
        bindingAccountId,
        bindingAccounts,
      )
    ) {
      const ownerLabel = encryptedOwnerLabel(binding) ?? "unknown owner";
      const confirmed = window.confirm(
        `This conversation already has encrypted session owner ${ownerLabel}. Changing the binding may make future requests fail with invalid_encrypted_content. Continue?`,
      );
      if (!confirmed) return;
    }
    setBindingSaving(true);
    setBindingError(null);
    try {
      const nextBinding = await updatePromptCacheConversationBinding(
        conversationKey,
        bindingKind === "group"
          ? { bindingKind: "group", groupName: bindingGroupName }
          : bindingKind === "upstreamAccount"
            ? {
                bindingKind: "upstreamAccount",
                upstreamAccountId: Number(bindingAccountId),
              }
            : { bindingKind: "none" },
      );
      setBinding(nextBinding);
      setBindingKind(nextBinding.bindingKind);
      setBindingGroupName(nextBinding.groupName ?? bindingGroups[0] ?? "");
      setBindingAccountId(
        nextBinding.upstreamAccountId != null
          ? String(nextBinding.upstreamAccountId)
          : bindingAccounts[0]
            ? String(bindingAccounts[0].id)
            : "",
      );
    } catch (err) {
      setBindingError(err instanceof Error ? err.message : String(err));
    } finally {
      setBindingSaving(false);
    }
  }, [
    binding,
    bindingAccountId,
    bindingAccounts,
    bindingGroupName,
    bindingGroups,
    bindingKind,
    bindingSubmitDisabled,
    conversationKey,
  ]);

  return (
    <AccountDetailDrawerShell
      open={open}
      labelledBy={titleId}
      closeLabel={t("live.conversations.drawer.close")}
      onClose={onClose}
      onBodyElementChange={setDrawerBodyElement}
      shellClassName="max-w-[78rem]"
      header={
        <div className="grid gap-3 xl:grid-cols-[minmax(0,1fr)_minmax(25rem,30rem)]">
          <div className="space-y-3">
            <div className="section-heading">
              <p className="text-xs font-semibold uppercase tracking-[0.2em] text-primary/75">
                {t("live.conversations.drawer.eyebrow")}
              </p>
              <h2 id={titleId} className="section-title break-all">
                {displayTitle}
              </h2>
              {shouldShowConversationKey ? (
                <p className="break-all font-mono text-xs text-base-content/62">
                  {conversationKey}
                </p>
              ) : null}
              <p className="section-description">
                {t("live.conversations.drawer.description")}
              </p>
            </div>
            <div className="text-sm text-base-content/70">
              {effectiveTotal > 0 && loadedCount >= effectiveTotal
                ? t("live.conversations.drawer.progressComplete", {
                    count: effectiveTotal,
                  })
                : t("live.conversations.drawer.progress", {
                    loaded: loadedCount,
                    total: effectiveTotal,
                  })}
            </div>
          </div>
          <div className="rounded border border-base-content/10 bg-base-200/50 p-3 text-sm">
            <div className="flex items-start justify-between gap-3">
              <div>
                <p className="font-semibold text-base-content">
                  {t("live.conversations.drawer.binding.title")}
                </p>
              </div>
              {bindingLoading ? (
                <Spinner
                  size="sm"
                  aria-label={t("live.conversations.drawer.binding.loading")}
                />
              ) : null}
            </div>
            <p className="mt-2 text-xs text-base-content/70">
              {bindingStatusLabel}
            </p>
            {encryptedOwnerStatusLabel ? (
              <p className="mt-1 text-xs text-warning">
                {t("live.conversations.drawer.binding.encryptedOwner", {
                  owner: encryptedOwnerStatusLabel,
                })}
              </p>
            ) : null}
            {binding?.hasEncryptedSessionOwner && binding.bindingKind === "none" ? (
              <p className="mt-1 text-xs text-base-content/60">
                {t("live.conversations.drawer.binding.encryptedOwnerHint")}
              </p>
            ) : null}
            <div className="mt-3 grid gap-2 sm:grid-cols-[8.5rem_minmax(0,1fr)_auto]">
              <SelectField
                value={bindingKind}
                disabled={bindingLoading || bindingSaving}
                aria-label={t("live.conversations.drawer.binding.kind")}
                size="sm"
                options={bindingKindOptions}
                onValueChange={(value) =>
                  setBindingKind(value as ConversationBindingDraftKind)
                }
              />
              {bindingKind === "group" ? (
                <SelectField
                  value={bindingGroupName}
                  disabled={bindingLoading || bindingSaving}
                  aria-label={t("live.conversations.drawer.binding.group")}
                  size="sm"
                  options={bindingGroups.map((groupName) => ({
                    value: groupName,
                    label: groupName,
                  }))}
                  onValueChange={setBindingGroupName}
                />
              ) : bindingKind === "upstreamAccount" ? (
                <SelectField
                  value={bindingAccountId}
                  disabled={bindingLoading || bindingSaving}
                  aria-label={t("live.conversations.drawer.binding.account")}
                  size="sm"
                  options={bindingAccounts.map((account) => ({
                    value: String(account.id),
                    label: conversationBindingAccountLabel(account),
                  }))}
                  onValueChange={setBindingAccountId}
                />
              ) : (
                <div className="hidden sm:block" aria-hidden="true" />
              )}
              <Button
                type="button"
                size="sm"
                disabled={bindingSubmitDisabled}
                onClick={() => void saveBinding()}
              >
                {bindingSaving
                  ? t("live.conversations.drawer.binding.saving")
                  : t("live.conversations.drawer.binding.save")}
              </Button>
            </div>
            {bindingError ? (
              <p className="mt-2 text-xs text-error">{bindingError}</p>
            ) : null}
          </div>
        </div>
      }
    >
      <div className="space-y-3">
        <PromptCacheConversationActivityOverview
          open={open}
          conversationKey={conversationKey}
          disableLiveUpdates={disableLiveUpdates}
          historyQueryForConversationKey={historyQueryForConversationKey}
          historyRecordMatchesConversationKey={
            historyRecordMatchesConversationKey
          }
          t={t}
        />
        <PromptCacheConversationInvocationTable
          records={visibleRecords}
          isLoading={isLoading}
          error={error}
          emptyLabel={t("live.conversations.drawer.empty")}
          onOpenUpstreamAccount={onOpenUpstreamAccount}
          scrollElement={drawerBodyElement}
        />
        {isLoadingMore ? (
          <div className="flex items-center justify-center gap-2 py-2 text-sm text-base-content/60">
            <Spinner size="sm" aria-label={t("chart.loadingDetailed")} />
            <span>{t("live.conversations.drawer.loadingMore")}</span>
          </div>
        ) : null}
      </div>
    </AccountDetailDrawerShell>
  );
}

export function PromptCacheConversationTable({
  stats,
  isLoading,
  error,
  expandedPromptCacheKeys,
  onToggleExpandedPromptCacheKey,
  onOpenUpstreamAccount,
  keyColumnLabel,
  emptyLabel,
  historyQueryForConversationKey,
  historyRecordMatchesConversationKey,
}: PromptCacheConversationTableProps) {
  const { t, locale } = useTranslation();
  const [now, setNow] = useState(() => Date.now());
  const [historyDrawerPromptCacheKey, setHistoryDrawerPromptCacheKey] =
    useState<string | null>(null);
  const [internalExpandedPromptCacheKeys, setInternalExpandedPromptCacheKeys] =
    useState<string[]>([]);
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const isExpansionControlled = expandedPromptCacheKeys != null;

  useEffect(() => {
    const timer = setInterval(() => {
      setNow(Date.now());
    }, PROMPT_CACHE_NOW_TICK_MS);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    if (!stats) return;
    setNow(Date.now());
  }, [stats]);

  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag),
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
  const dateFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(localeTag, {
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
        hour12: false,
      }),
    [localeTag],
  );

  const chartRangeOverride = useMemo(() => {
    if (!stats || stats.conversations.length === 0) return null;
    const earliestCreatedAt = stats.conversations.reduce<number | null>(
      (earliest, conversation) => {
        const createdAt = parseEpoch(conversation.createdAt);
        if (createdAt == null) return earliest;
        return earliest == null ? createdAt : Math.min(earliest, createdAt);
      },
      null,
    );
    if (earliestCreatedAt == null) return null;
    const chartRangeStart = Math.max(
      earliestCreatedAt,
      now - PROMPT_CACHE_CHART_MAX_WINDOW_MS,
    );
    return {
      rangeStart: new Date(chartRangeStart).toISOString(),
      rangeEnd: new Date(now).toISOString(),
    };
  }, [now, stats]);

  const chartHours = useMemo(() => {
    const rangeStartEpoch = parseEpoch(
      chartRangeOverride?.rangeStart ?? stats?.rangeStart ?? "",
    );
    const rangeEndEpoch = parseEpoch(
      chartRangeOverride?.rangeEnd ?? stats?.rangeEnd ?? "",
    );
    if (
      rangeStartEpoch == null ||
      rangeEndEpoch == null ||
      rangeEndEpoch <= rangeStartEpoch
    ) {
      return 24;
    }
    return Math.max(
      1,
      Math.ceil((rangeEndEpoch - rangeStartEpoch) / 3_600_000),
    );
  }, [
    chartRangeOverride?.rangeEnd,
    chartRangeOverride?.rangeStart,
    stats?.rangeEnd,
    stats?.rangeStart,
  ]);

  const footerNote = useMemo(() => {
    if (
      !stats ||
      stats.implicitFilter.filteredCount <= 0 ||
      stats.implicitFilter.kind == null
    ) {
      return null;
    }
    if (stats.implicitFilter.kind === "inactiveOutside24h") {
      if (
        stats.selectionMode === "activityWindow" &&
        stats.selectedActivityHours != null
      ) {
        return t(
          "live.conversations.implicitFilter.inactiveOutsideActivityWindow",
          {
            count: stats.implicitFilter.filteredCount,
            hours: stats.selectedActivityHours,
          },
        );
      }
      return t("live.conversations.implicitFilter.inactiveOutside24h", {
        count: stats.implicitFilter.filteredCount,
      });
    }
    return t("live.conversations.implicitFilter.cappedTo50", {
      count: stats.implicitFilter.filteredCount,
    });
  }, [stats, t]);

  const tooltipLabels = useMemo(
    () => ({
      status: t("live.conversations.chart.tooltip.status"),
      requestTokens: t("live.conversations.chart.tooltip.requestTokens"),
      cumulativeTokens: t("live.conversations.chart.tooltip.cumulativeTokens"),
    }),
    [t],
  );
  const chartInteractionHint = t("live.chart.tooltip.instructions");
  const resolvedKeyColumnLabel =
    keyColumnLabel ?? t("live.conversations.table.promptCacheKey");
  const resolvedEmptyLabel = emptyLabel ?? t("live.conversations.empty");
  const chartAriaLabel = t("live.conversations.chartAria", {
    hours: chartHours,
  });
  const chartColumnLabel = t("live.conversations.table.chartWindow", {
    hours: chartHours,
  });
  const rangeStart = chartRangeOverride?.rangeStart ?? stats?.rangeStart ?? "";
  const rangeEnd = chartRangeOverride?.rangeEnd ?? stats?.rangeEnd ?? "";
  const conversationChartMax = useMemo(
    () =>
      findVisibleConversationChartMax(
        stats?.conversations ?? [],
        rangeStart,
        rangeEnd,
      ),
    [rangeEnd, rangeStart, stats?.conversations],
  );
  const totalLabels = useMemo(
    () => ({
      requestCount: t("live.conversations.table.requestCount"),
      totalTokens: t("live.conversations.table.totalTokens"),
      totalCost: t("live.conversations.table.totalCost"),
      requestCountCompact: t("live.conversations.table.requestCountCompact"),
      totalTokensCompact: t("live.conversations.table.totalTokensCompact"),
      time: t("live.conversations.table.time"),
      createdAtShort: t("live.conversations.table.createdAtShort"),
      lastActivityAtShort: t("live.conversations.table.lastActivityAtShort"),
    }),
    [t],
  );
  const previewLabels = useMemo(
    () => ({
      empty: t("live.conversations.preview.empty"),
      expandAction: t("live.conversations.actions.expandPreview"),
      collapseAction: t("live.conversations.actions.collapsePreview"),
      historyAction: t("live.conversations.actions.openHistory"),
    }),
    [t],
  );
  const fallbackAccountLabel = useMemo(
    () => (id: number) =>
      t("live.conversations.accountLabel.idFallback", {
        id: String(Math.trunc(id)),
      }),
    [t],
  );
  const effectiveExpandedPromptCacheKeys = isExpansionControlled
    ? expandedPromptCacheKeys
    : internalExpandedPromptCacheKeys;
  const expandedPromptCacheKeySet = useMemo(
    () => new Set(effectiveExpandedPromptCacheKeys),
    [effectiveExpandedPromptCacheKeys],
  );

  useEffect(() => {
    if (isExpansionControlled || !stats) return;
    const visiblePromptCacheKeys = new Set(
      stats.conversations.map((conversation) => conversation.promptCacheKey),
    );
    setInternalExpandedPromptCacheKeys((current) =>
      current.filter((promptCacheKey) =>
        visiblePromptCacheKeys.has(promptCacheKey),
      ),
    );
  }, [isExpansionControlled, stats]);

  const openAccountDrawer = (
    account: PromptCacheConversationUpstreamAccount,
  ) => {
    if (!canOpenPromptCacheUpstreamAccount(account)) return;
    setHistoryDrawerPromptCacheKey(null);
    onOpenUpstreamAccount?.(
      Math.trunc(Number(account.upstreamAccountId)),
      resolveUpstreamAccountLabel(account, fallbackAccountLabel),
    );
  };
  const openHistoryDrawer = (promptCacheKey: string) => {
    setHistoryDrawerPromptCacheKey(promptCacheKey);
  };
  const closeHistoryDrawer = () => {
    setHistoryDrawerPromptCacheKey(null);
  };
  const togglePromptCachePreview = (promptCacheKey: string) => {
    if (!isExpansionControlled) {
      setInternalExpandedPromptCacheKeys((current) =>
        current.includes(promptCacheKey)
          ? current.filter((value) => value !== promptCacheKey)
          : [...current, promptCacheKey],
      );
    }
    onToggleExpandedPromptCacheKey?.(promptCacheKey);
  };

  if (error) {
    return (
      <Alert variant="error">
        <span>{error}</span>
      </Alert>
    );
  }

  if (isLoading) {
    return (
      <div className="flex justify-center py-8">
        <Spinner size="lg" aria-label={t("chart.loadingDetailed")} />
      </div>
    );
  }

  if (!stats || stats.conversations.length === 0) {
    return (
      <div className="space-y-2">
        <Alert>{resolvedEmptyLabel}</Alert>
        {footerNote ? (
          <p className="px-1 text-[11px] text-base-content/55">{footerNote}</p>
        ) : null}
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <div className="overflow-hidden rounded-xl border border-base-300/75 bg-base-100/55">
        <div className="space-y-3 p-3 sm:hidden">
          {stats.conversations.map((conversation) => {
            const createdAtLabel = formatDateLabel(
              conversation.createdAt,
              dateFormatter,
            );
            const lastActivityLabel = formatDateLabel(
              conversation.lastActivityAt,
              dateFormatter,
            );
            const isExpanded = expandedPromptCacheKeySet.has(
              conversation.promptCacheKey,
            );

            return (
              <article
                key={`${conversation.promptCacheKey}-mobile`}
                className="space-y-3 rounded-lg border border-base-300/70 bg-base-100/70 p-3"
              >
                <div className="space-y-2">
                  <div className="space-y-2">
                    <div className="min-w-0 space-y-1">
                      <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                        {resolvedKeyColumnLabel}
                      </div>
                      <div className="break-all font-mono text-xs">
                        {conversation.promptCacheKey}
                      </div>
                    </div>
                    <div className="flex items-center gap-1">
                      <button
                        type="button"
                        className="inline-flex h-8 w-8 items-center justify-center rounded-full border border-base-300/70 bg-base-100/80 text-base-content/72 transition hover:border-primary/40 hover:text-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                        aria-label={
                          isExpanded
                            ? previewLabels.collapseAction
                            : previewLabels.expandAction
                        }
                        aria-expanded={isExpanded}
                        onClick={() =>
                          togglePromptCachePreview(conversation.promptCacheKey)
                        }
                      >
                        <AppIcon
                          name={isExpanded ? "chevron-up" : "chevron-down"}
                          className="h-4 w-4"
                          aria-hidden
                        />
                      </button>
                      <button
                        type="button"
                        className="inline-flex h-8 w-8 items-center justify-center rounded-full border border-base-300/70 bg-base-100/80 text-base-content/72 transition hover:border-primary/40 hover:text-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                        aria-label={previewLabels.historyAction}
                        onClick={() =>
                          openHistoryDrawer(conversation.promptCacheKey)
                        }
                      >
                        <AppIcon
                          name="account-details-outline"
                          className="h-4 w-4"
                          aria-hidden
                        />
                      </button>
                    </div>
                  </div>
                  {isExpanded ? (
                    <div className="rounded-lg border border-base-300/70 bg-base-200/30 p-3">
                      <PromptCacheConversationInvocationTable
                        records={conversation.recentInvocations.map(
                          buildInvocationFromPromptCachePreview,
                        )}
                        isLoading={false}
                        emptyLabel={previewLabels.empty}
                        onOpenUpstreamAccount={onOpenUpstreamAccount}
                      />
                    </div>
                  ) : null}
                </div>

                <div className="space-y-1">
                  <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                    {t("live.conversations.table.upstreamAccounts")}
                  </div>
                  <UpstreamAccountsBlock
                    upstreamAccounts={conversation.upstreamAccounts}
                    labels={totalLabels}
                    numberFormatter={numberFormatter}
                    currencyFormatter={currencyFormatter}
                    fallbackAccountLabel={fallbackAccountLabel}
                    onOpenAccountDetail={openAccountDrawer}
                  />
                </div>

                <div className="space-y-1">
                  <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                    {t("live.conversations.table.summary")}
                  </div>
                  <SummaryBlock
                    conversation={conversation}
                    labels={totalLabels}
                    numberFormatter={numberFormatter}
                    currencyFormatter={currencyFormatter}
                  />
                </div>

                <div className="space-y-1">
                  <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                    {totalLabels.time}
                  </div>
                  <dl className="space-y-1 text-xs">
                    <div className="flex items-center justify-between gap-3">
                      <dt className="text-base-content/60">
                        {totalLabels.createdAtShort}
                      </dt>
                      <dd className="text-right">{createdAtLabel}</dd>
                    </div>
                    <div className="flex items-center justify-between gap-3">
                      <dt className="text-base-content/60">
                        {totalLabels.lastActivityAtShort}
                      </dt>
                      <dd className="text-right">{lastActivityLabel}</dd>
                    </div>
                  </dl>
                </div>

                <div className="space-y-1">
                  <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                    {chartColumnLabel}
                  </div>
                  <ConversationSparkline
                    conversation={conversation}
                    rangeStart={rangeStart}
                    rangeEnd={rangeEnd}
                    maxCumulativeTokens={conversationChartMax}
                    localeTag={localeTag}
                    tooltipLabels={tooltipLabels}
                    interactionHint={chartInteractionHint}
                    ariaLabel={`${conversation.promptCacheKey} ${chartAriaLabel}`}
                    conversationKey={conversation.promptCacheKey}
                  />
                </div>
              </article>
            );
          })}
        </div>

        <table className="hidden w-full table-fixed text-xs sm:table">
          <thead className="bg-base-200/70 uppercase tracking-[0.08em] text-base-content/65">
            <tr>
              <th className="w-[18%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                {resolvedKeyColumnLabel}
              </th>
              <th className="w-[34%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                {t("live.conversations.table.upstreamAccounts")}
              </th>
              <th className="w-[15%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                {t("live.conversations.table.summary")}
              </th>
              <th className="w-[15%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                {totalLabels.time}
              </th>
              <th className="w-[18%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                {chartColumnLabel}
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-base-300/65">
            {stats.conversations.map((conversation) => {
              const isExpanded = expandedPromptCacheKeySet.has(
                conversation.promptCacheKey,
              );

              return (
                <Fragment key={conversation.promptCacheKey}>
                  <tr className="transition-colors hover:bg-primary/6">
                    <td className="max-w-0 px-2 py-2 align-top sm:px-3 sm:py-3">
                      <div className="space-y-2">
                        <div
                          className="truncate font-mono text-xs"
                          title={conversation.promptCacheKey}
                        >
                          {conversation.promptCacheKey}
                        </div>
                        <div className="flex items-center gap-1">
                          <button
                            type="button"
                            className="inline-flex h-8 w-8 items-center justify-center rounded-full border border-base-300/70 bg-base-100/80 text-base-content/72 transition hover:border-primary/40 hover:text-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                            aria-label={
                              isExpanded
                                ? previewLabels.collapseAction
                                : previewLabels.expandAction
                            }
                            aria-expanded={isExpanded}
                            onClick={() =>
                              togglePromptCachePreview(
                                conversation.promptCacheKey,
                              )
                            }
                          >
                            <AppIcon
                              name={isExpanded ? "chevron-up" : "chevron-down"}
                              className="h-4 w-4"
                              aria-hidden
                            />
                          </button>
                          <button
                            type="button"
                            className="inline-flex h-8 w-8 items-center justify-center rounded-full border border-base-300/70 bg-base-100/80 text-base-content/72 transition hover:border-primary/40 hover:text-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                            aria-label={previewLabels.historyAction}
                            onClick={() =>
                              openHistoryDrawer(conversation.promptCacheKey)
                            }
                          >
                            <AppIcon
                              name="account-details-outline"
                              className="h-4 w-4"
                              aria-hidden
                            />
                          </button>
                        </div>
                      </div>
                    </td>
                    <td className="px-2 py-2 align-top sm:px-3 sm:py-3">
                      <UpstreamAccountsBlock
                        upstreamAccounts={conversation.upstreamAccounts}
                        labels={totalLabels}
                        numberFormatter={numberFormatter}
                        currencyFormatter={currencyFormatter}
                        fallbackAccountLabel={fallbackAccountLabel}
                        onOpenAccountDetail={openAccountDrawer}
                      />
                    </td>
                    <td className="px-2 py-2 align-top sm:px-3 sm:py-3">
                      <SummaryBlock
                        conversation={conversation}
                        labels={totalLabels}
                        numberFormatter={numberFormatter}
                        currencyFormatter={currencyFormatter}
                      />
                    </td>
                    <td className="px-2 py-2 align-top sm:px-3 sm:py-3">
                      <div className="space-y-1.5 text-[11px]">
                        <div className="grid grid-cols-[2rem_minmax(0,1fr)] items-center gap-x-2">
                          <span className="text-base-content/60">
                            {totalLabels.createdAtShort}
                          </span>
                          <span className="whitespace-nowrap font-medium tabular-nums">
                            {formatDateLabel(
                              conversation.createdAt,
                              dateFormatter,
                            )}
                          </span>
                        </div>
                        <div className="grid grid-cols-[2rem_minmax(0,1fr)] items-center gap-x-2">
                          <span className="text-base-content/60">
                            {totalLabels.lastActivityAtShort}
                          </span>
                          <span className="whitespace-nowrap font-medium tabular-nums">
                            {formatDateLabel(
                              conversation.lastActivityAt,
                              dateFormatter,
                            )}
                          </span>
                        </div>
                      </div>
                    </td>
                    <td className="px-2 py-2 align-top sm:px-3 sm:py-3">
                      <ConversationSparkline
                        conversation={conversation}
                        rangeStart={rangeStart}
                        rangeEnd={rangeEnd}
                        maxCumulativeTokens={conversationChartMax}
                        localeTag={localeTag}
                        tooltipLabels={tooltipLabels}
                        interactionHint={chartInteractionHint}
                        ariaLabel={`${conversation.promptCacheKey} ${chartAriaLabel}`}
                        conversationKey={conversation.promptCacheKey}
                      />
                    </td>
                  </tr>
                  {isExpanded ? (
                    <tr className="bg-base-200/20">
                      <td colSpan={5} className="px-3 pb-4 pt-0">
                        <div className="border-t border-base-300/60 pt-3">
                          <PromptCacheConversationInvocationTable
                            records={conversation.recentInvocations.map(
                              buildInvocationFromPromptCachePreview,
                            )}
                            isLoading={false}
                            emptyLabel={previewLabels.empty}
                            onOpenUpstreamAccount={onOpenUpstreamAccount}
                          />
                        </div>
                      </td>
                    </tr>
                  ) : null}
                </Fragment>
              );
            })}
          </tbody>
        </table>
      </div>
      {footerNote ? (
        <p className="px-1 text-[11px] text-base-content/55">{footerNote}</p>
      ) : null}
      <PromptCacheConversationHistoryDrawer
        open={historyDrawerPromptCacheKey != null}
        conversationKey={historyDrawerPromptCacheKey}
        onClose={closeHistoryDrawer}
        t={t}
        onOpenUpstreamAccount={onOpenUpstreamAccount}
        historyQueryForConversationKey={historyQueryForConversationKey}
        historyRecordMatchesConversationKey={
          historyRecordMatchesConversationKey
        }
      />
    </div>
  );
}
