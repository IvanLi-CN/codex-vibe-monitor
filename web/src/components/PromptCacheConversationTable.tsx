import {
  Fragment,
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "../i18n";
import type {
  ApiInvocation,
  InvocationRecordsQuery,
  InvocationRecordsSummaryResponse,
  PromptCacheConversation,
  PromptCacheConversationUpstreamAccount,
  PromptCacheConversationsResponse,
} from "../lib/api";
import { fetchInvocationRecords, fetchInvocationRecordsSummary } from "../lib/api";
import { resolvePromptCacheInvocationOutcome } from "../lib/conversationRequestPoint";
import { mergeInvocationRecordCollections } from "../lib/invocationLiveMerge";
import { invocationStableKey } from "../lib/invocation";
import { buildInvocationFromPromptCachePreview } from "../lib/promptCacheLive";
import { subscribeToSse, subscribeToSseOpen } from "../lib/sse";
import { AccountDetailDrawerShell } from "./AccountDetailDrawerShell";
import { AppIcon } from "./AppIcon";
import { InvocationTable } from "./InvocationTable";
import { ConversationSparkline } from "./KeyedConversationTable";
import {
  FALLBACK_CELL,
  findVisibleConversationChartMax,
} from "./keyedConversationChart";
import { Alert } from "./ui/alert";
import { SegmentedControl, SegmentedControlItem } from "./ui/segmented-control";
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
const PROMPT_CACHE_HISTORY_PAGE_SIZE = 200;
const PROMPT_CACHE_ACTIVITY_PAGE_SIZE = 200;
const PROMPT_CACHE_ACTIVITY_MAX_CHART_RECORDS = 1_000;
const PROMPT_CACHE_HISTORY_RESYNC_THROTTLE_MS = 1_000;
const PROMPT_CACHE_ACTIVITY_RESYNC_THROTTLE_MS = 1_000;

type ConversationActivityRange = "today" | "yesterday" | "1d" | "7d" | "history";
type ConversationActivityMetric = "totalCount" | "totalCost" | "totalTokens";

const CONVERSATION_ACTIVITY_RANGES: Array<{
  key: ConversationActivityRange;
  labelKey: string;
}> = [
  { key: "today", labelKey: "dashboard.activityOverview.rangeToday" },
  { key: "yesterday", labelKey: "dashboard.activityOverview.rangeYesterday" },
  { key: "1d", labelKey: "dashboard.activityOverview.range24h" },
  { key: "7d", labelKey: "dashboard.activityOverview.range7d" },
  { key: "history", labelKey: "dashboard.activityOverview.rangeUsage" },
];

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
}: {
  records: ApiInvocation[];
  isLoading: boolean;
  error?: string | null;
  emptyLabel: string;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
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
  label: string;
  tooltipLabel: string;
  success: number;
  failure: number;
  inFlight: number;
  neutral: number;
  totalCount: number;
  totalCost: number;
  totalTokens: number;
  totalMs: number;
  totalMsSamples: number;
}

function buildConversationActivityBuckets({
  records,
  range,
  metric,
  localeTag,
}: {
  records: ApiInvocation[];
  range: ConversationActivityRange;
  metric: ConversationActivityMetric;
  localeTag: string;
}) {
  const now = new Date();
  const rangeBounds = resolveConversationActivityRange(range);
  let startMs = rangeBounds.from ? Date.parse(rangeBounds.from) : Number.POSITIVE_INFINITY;
  let endMs = rangeBounds.to ? Date.parse(rangeBounds.to) : Number.NEGATIVE_INFINITY;

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
    endMs = now.getTime();
    startMs = endMs - 86_400_000;
  }

  const targetBuckets =
    range === "today" || range === "yesterday" ? 24 : range === "1d" ? 24 : 28;
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
        label: labelFormatter.format(new Date(bucketStart)),
        tooltipLabel: labelFormatter.format(new Date(bucketStart)),
        success: 0,
        failure: 0,
        inFlight: 0,
        neutral: 0,
        totalCount: 0,
        totalCost: 0,
        totalTokens: 0,
        totalMs: 0,
        totalMsSamples: 0,
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

  return buckets;
}

function ConversationActivityChart({
  buckets,
  metric,
  loading,
  numberFormatter,
  currencyFormatter,
  t,
}: {
  buckets: ConversationActivityBucket[];
  metric: ConversationActivityMetric;
  loading: boolean;
  numberFormatter: Intl.NumberFormat;
  currencyFormatter: Intl.NumberFormat;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  const width = 640;
  const height = 180;
  const padding = { top: 14, right: 18, bottom: 30, left: 28 };
  const innerWidth = width - padding.left - padding.right;
  const innerHeight = height - padding.top - padding.bottom;
  const maxMetric = Math.max(
    1,
    ...buckets.map((bucket) =>
      bucket.success + bucket.failure + bucket.inFlight + bucket.neutral,
    ),
  );
  const maxDuration = Math.max(
    1,
    ...buckets.map((bucket) =>
      bucket.totalMsSamples > 0 ? bucket.totalMs / bucket.totalMsSamples : 0,
    ),
  );
  const barGap = 2;
  const barWidth = Math.max(2, innerWidth / Math.max(buckets.length, 1) - barGap);
  const durationPoints = buckets
    .map((bucket, index) => {
      if (bucket.totalMsSamples <= 0) return null;
      const x =
        padding.left +
        (index / Math.max(buckets.length - 1, 1)) * innerWidth;
      const avg = bucket.totalMs / bucket.totalMsSamples;
      const y = padding.top + innerHeight - (avg / maxDuration) * innerHeight;
      return `${x.toFixed(2)},${y.toFixed(2)}`;
    })
    .filter((point): point is string => point != null)
    .join(" ");

  const formatMetricValue = (value: number) => {
    if (metric === "totalCost") return currencyFormatter.format(value);
    return numberFormatter.format(value);
  };

  if (loading && buckets.length === 0) {
    return (
      <div className="flex h-44 items-center justify-center gap-2 rounded-lg border border-base-300/70 bg-base-200/20 text-sm text-base-content/60">
        <Spinner size="sm" aria-label={t("chart.loadingDetailed")} />
        <span>{t("chart.loadingDetailed")}</span>
      </div>
    );
  }

  return (
    <div className="rounded-lg border border-base-300/70 bg-base-200/20 p-3">
      <svg
        viewBox={`0 0 ${width} ${height}`}
        role="img"
        aria-label={t("live.conversations.activity.chartAria")}
        className="h-44 w-full overflow-visible"
      >
        <line
          x1={padding.left}
          x2={width - padding.right}
          y1={padding.top + innerHeight}
          y2={padding.top + innerHeight}
          className="stroke-base-content/20"
        />
        {buckets.map((bucket, index) => {
          const total =
            bucket.success + bucket.failure + bucket.inFlight + bucket.neutral;
          const x = padding.left + index * (barWidth + barGap);
          let y = padding.top + innerHeight;
          const segments = [
            { key: "success", value: bucket.success, className: "fill-success" },
            { key: "failure", value: bucket.failure, className: "fill-error" },
            { key: "inFlight", value: bucket.inFlight, className: "fill-info" },
            { key: "neutral", value: bucket.neutral, className: "fill-base-content/35" },
          ];
          return (
            <g key={`${bucket.label}-${index}`}>
              <title>
                {`${bucket.tooltipLabel} · ${formatMetricValue(total)}`}
              </title>
              {segments.map((segment) => {
                if (segment.value <= 0) return null;
                const segmentHeight = Math.max(
                  1,
                  (segment.value / maxMetric) * innerHeight,
                );
                y -= segmentHeight;
                return (
                  <rect
                    key={segment.key}
                    x={x}
                    y={y}
                    width={barWidth}
                    height={segmentHeight}
                    className={segment.className}
                    opacity={0.88}
                  />
                );
              })}
            </g>
          );
        })}
        {durationPoints ? (
          <polyline
            points={durationPoints}
            fill="none"
            className="stroke-base-content/70"
            strokeWidth={2}
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        ) : null}
        {buckets[0] ? (
          <text
            x={padding.left}
            y={height - 8}
            className="fill-base-content/55 text-[10px]"
          >
            {buckets[0].label}
          </text>
        ) : null}
        {buckets.at(-1) ? (
          <text
            x={width - padding.right}
            y={height - 8}
            textAnchor="end"
            className="fill-base-content/55 text-[10px]"
          >
            {buckets.at(-1)?.label}
          </text>
        ) : null}
      </svg>
      <div className="mt-2 flex flex-wrap items-center justify-center gap-x-4 gap-y-1 text-xs text-base-content/70">
        <span className="inline-flex items-center gap-1.5">
          <span className="h-2.5 w-2.5 rounded-sm bg-success" />
          {t("live.conversations.activity.legendSuccess")}
        </span>
        <span className="inline-flex items-center gap-1.5">
          <span className="h-2.5 w-2.5 rounded-sm bg-error" />
          {t("live.conversations.activity.legendFailure")}
        </span>
        <span className="inline-flex items-center gap-1.5">
          <span className="h-2.5 w-2.5 rounded-sm bg-info" />
          {t("live.conversations.activity.legendInFlight")}
        </span>
        <span className="inline-flex items-center gap-1.5">
          <span className="h-px w-5 bg-base-content/70" />
          {t("live.conversations.activity.legendDuration")}
        </span>
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
  const [activeRange, setActiveRange] =
    useState<ConversationActivityRange>("today");
  const [activeMetric, setActiveMetric] =
    useState<ConversationActivityMetric>("totalCount");
  const [summary, setSummary] =
    useState<InvocationRecordsSummaryResponse | null>(null);
  const [records, setRecords] = useState<ApiInvocation[]>([]);
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
        setRecords(loaded);
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
    [activeRange, conversationKey, historyQueryForConversationKey, open],
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
      setChartTotal(0);
      setChartIsSampled(false);
      isLoadingRef.current = false;
      setIsLoading(false);
      setError(null);
      return;
    }
    setSummary(null);
    setRecords([]);
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

  const buckets = useMemo(
    () =>
      buildConversationActivityBuckets({
        records,
        range: activeRange,
        metric: activeMetric,
        localeTag,
      }),
    [activeMetric, activeRange, localeTag, records],
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
        <div className="flex flex-wrap items-center gap-3">
          <h3 className="text-sm font-semibold">
            {t("live.conversations.activity.title")}
          </h3>
          <SegmentedControl
            size="compact"
            role="tablist"
            aria-label={t("dashboard.activityOverview.rangeToggleAria")}
          >
            {CONVERSATION_ACTIVITY_RANGES.map((range) => (
              <SegmentedControlItem
                key={range.key}
                active={activeRange === range.key}
                role="tab"
                aria-selected={activeRange === range.key}
                onClick={() => setActiveRange(range.key)}
              >
                {t(range.labelKey)}
              </SegmentedControlItem>
            ))}
          </SegmentedControl>
        </div>
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
        buckets={buckets}
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
  const pendingLoadRef = useRef<{ silent?: boolean } | null>(null);
  const pendingOpenResyncRef = useRef(false);
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastRefreshAtRef = useRef(0);
  const [records, setRecords] = useState<ApiInvocation[]>([]);
  const [liveRecords, setLiveRecords] = useState<ApiInvocation[]>([]);
  const [total, setTotal] = useState(0);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const clearPendingRefreshTimer = useCallback(() => {
    if (!refreshTimerRef.current) return;
    clearTimeout(refreshTimerRef.current);
    refreshTimerRef.current = null;
  }, []);

  const runLoad = useCallback(
    async ({ silent = false }: { silent?: boolean } = {}) => {
      if (!open || !conversationKey) return;

      inFlightRef.current = true;
      const requestSeq = requestSeqRef.current + 1;
      requestSeqRef.current = requestSeq;
      const shouldShowLoading = !(silent && hasHydratedRef.current);
      if (shouldShowLoading) setIsLoading(true);
      try {
        let page = 1;
        let snapshotId: number | undefined;
        let loaded: ApiInvocation[] = [];
        let totalRecords = 0;

        while (true) {
          const historyFilters = historyQueryForConversationKey?.(
            conversationKey,
          ) ?? {
            promptCacheKey: conversationKey,
          };
          const response = await fetchInvocationRecords({
            ...historyFilters,
            page,
            pageSize: PROMPT_CACHE_HISTORY_PAGE_SIZE,
            sortBy: "occurredAt",
            sortOrder: "desc",
            ...(snapshotId != null ? { snapshotId } : {}),
          });
          if (requestSeq !== requestSeqRef.current) return;

          snapshotId = response.snapshotId;
          totalRecords = response.total;
          loaded = [...loaded, ...response.records];
          setRecords(loaded);
          setTotal(totalRecords);

          if (loaded.length >= totalRecords || response.records.length === 0) {
            break;
          }
          page += 1;
        }

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
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (requestSeq === requestSeqRef.current && shouldShowLoading) {
          setIsLoading(false);
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
    async (options: { silent?: boolean } = {}) => {
      const silent = options.silent ?? false;
      if (inFlightRef.current) {
        const pendingSilent = pendingLoadRef.current?.silent ?? true;
        pendingLoadRef.current = { silent: pendingSilent && silent };
        return;
      }
      await runLoad({ silent });
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
    clearPendingRefreshTimer();

    if (!open || !conversationKey) {
      setRecords([]);
      setLiveRecords([]);
      setTotal(0);
      setIsLoading(false);
      setError(null);
      return;
    }

    setRecords([]);
    setLiveRecords([]);
    setTotal(0);
    setIsLoading(false);
    setError(null);
    void load();
  }, [clearPendingRefreshTimer, conversationKey, load, open]);

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
      clearPendingRefreshTimer();
      pendingLoadRef.current = null;
      pendingOpenResyncRef.current = false;
    },
    [clearPendingRefreshTimer],
  );

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

  return (
    <AccountDetailDrawerShell
      open={open}
      labelledBy={titleId}
      closeLabel={t("live.conversations.drawer.close")}
      onClose={onClose}
      shellClassName="max-w-[78rem]"
      header={
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
        />
        {isLoading && visibleRecords.length > 0 ? (
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
