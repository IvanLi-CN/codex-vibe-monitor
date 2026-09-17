import { useMemo, useState } from "react";
import { Alert } from "../../components/ui/alert";
import { SegmentedControl, SegmentedControlItem } from "../../components/ui/segmented-control";
import type { InvocationHistoryOverviewTopicPayload } from "../../hooks/useConversationDetailTopics";
import { useTranslation } from "../../i18n";
import type { InvocationRecordsQuery } from "../../lib/api";
import { FALLBACK_CELL } from "./keyedConversationChart";
import { ConversationActivityChart } from "./PromptCacheActivityChart";
import {
  buildConversationActivityBuckets,
  type ConversationActivityMetric,
  type ConversationActivityRange,
  formatCompactNumber,
  formatDurationMs,
} from "./PromptCacheActivityData";
import { usePromptCacheActivityData } from "./usePromptCacheActivityData";

const CONVERSATION_ACTIVITY_METRICS: Array<{ key: ConversationActivityMetric; labelKey: string }> =
  [
    { key: "totalCount", labelKey: "metric.totalCount" },
    { key: "totalCost", labelKey: "metric.totalCost" },
    { key: "totalTokens", labelKey: "metric.totalTokens" },
  ];

export function PromptCacheConversationActivityOverview({
  open,
  conversationKey,
  historyQueryForConversationKey,
  realtimePayload,
  isRealtimeLoading,
  allowHttpFallback,
  t,
}: {
  open: boolean;
  conversationKey: string | null;
  historyQueryForConversationKey?: (conversationKey: string) => Partial<InvocationRecordsQuery>;
  realtimePayload: InvocationHistoryOverviewTopicPayload | null;
  isRealtimeLoading: boolean;
  allowHttpFallback: boolean;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  const { locale } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const [activeMetric, setActiveMetric] = useState<ConversationActivityMetric>("totalCount");
  const data = usePromptCacheActivityData({
    open,
    conversationKey,
    historyQueryForConversationKey,
    realtimePayload,
    isRealtimeLoading,
    allowHttpFallback,
  });
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag, { maximumFractionDigits: 2, notation: "compact" }),
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
  const bucketSet = useMemo(
    () =>
      buildConversationActivityBuckets({
        records: data.records,
        range: "history" as ConversationActivityRange,
        metric: activeMetric,
        localeTag,
        rangeStartMs: data.chartRangeStartMs,
        rangeEndMs: data.chartRangeEndMs,
      }),
    [activeMetric, data.chartRangeEndMs, data.chartRangeStartMs, data.records, localeTag],
  );
  const metrics = buildActivityMetrics(
    data.summary,
    numberFormatter,
    fullNumberFormatter,
    currencyFormatter,
    data.isLoading,
    t,
  );

  return (
    <ActivityOverviewView
      activeMetric={activeMetric}
      setActiveMetric={setActiveMetric}
      error={data.error}
      metrics={metrics}
      bucketSet={bucketSet}
      isLoading={data.isLoading}
      numberFormatter={numberFormatter}
      currencyFormatter={currencyFormatter}
      chartIsSampled={data.chartIsSampled}
      loadedCount={data.records.length}
      chartTotal={data.chartTotal}
      fullNumberFormatter={fullNumberFormatter}
      t={t}
    />
  );
}

function ActivityOverviewView({
  activeMetric,
  setActiveMetric,
  error,
  metrics,
  bucketSet,
  isLoading,
  numberFormatter,
  currencyFormatter,
  chartIsSampled,
  loadedCount,
  chartTotal,
  fullNumberFormatter,
  t,
}: {
  activeMetric: ConversationActivityMetric;
  setActiveMetric: (metric: ConversationActivityMetric) => void;
  error: string | null;
  metrics: Array<{ label: string; value: string; toneClass: string }>;
  bucketSet: ReturnType<typeof buildConversationActivityBuckets>;
  isLoading: boolean;
  numberFormatter: Intl.NumberFormat;
  currencyFormatter: Intl.NumberFormat;
  chartIsSampled: boolean;
  loadedCount: number;
  chartTotal: number;
  fullNumberFormatter: Intl.NumberFormat;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  return (
    <section className="space-y-3 rounded-xl border border-base-300/70 bg-base-100/55 p-3">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <h3 className="text-sm font-semibold">{t("live.conversations.activity.title")}</h3>
        <SegmentedControl size="compact" role="tablist" aria-label={t("heatmap.metricsToggleAria")}>
          {CONVERSATION_ACTIVITY_METRICS.map((item) => (
            <SegmentedControlItem
              key={item.key}
              active={activeMetric === item.key}
              role="tab"
              aria-selected={activeMetric === item.key}
              onClick={() => setActiveMetric(item.key)}
            >
              {t(item.labelKey)}
            </SegmentedControlItem>
          ))}
        </SegmentedControl>
      </div>
      {error ? (
        <Alert variant="error">
          <span>{t("records.summary.loadError", { error })}</span>
        </Alert>
      ) : null}
      <ActivityMetricGrid metrics={metrics} />
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
            loaded: formatCompactNumber(loadedCount, fullNumberFormatter),
            total: formatCompactNumber(chartTotal, fullNumberFormatter),
          })}
        </p>
      ) : null}
    </section>
  );
}

function buildActivityMetrics(
  summary: ReturnType<typeof usePromptCacheActivityData>["summary"],
  numberFormatter: Intl.NumberFormat,
  fullNumberFormatter: Intl.NumberFormat,
  currencyFormatter: Intl.NumberFormat,
  isLoading: boolean,
  t: (key: string) => string,
) {
  return [
    [
      t("live.conversations.activity.metricRequests"),
      formatCompactNumber(summary?.totalCount, numberFormatter),
      "text-primary",
    ],
    [
      t("live.conversations.activity.metricSuccess"),
      formatCompactNumber(summary?.successCount, numberFormatter),
      "text-success",
    ],
    [
      t("live.conversations.activity.metricFailures"),
      formatCompactNumber(summary?.failureCount, numberFormatter),
      "text-error",
    ],
    [
      t("live.conversations.activity.metricAborts"),
      formatCompactNumber(summary?.exception.clientAbortCount, numberFormatter),
      "text-warning",
    ],
    [
      t("live.conversations.activity.metricTokens"),
      formatCompactNumber(summary?.token.totalTokens, numberFormatter),
      "text-info",
    ],
    [
      t("live.conversations.activity.metricCost"),
      summary == null ? FALLBACK_CELL : currencyFormatter.format(summary.token.totalCost),
      "text-primary",
    ],
    [
      t("live.conversations.activity.metricAvgDuration"),
      formatDurationMs(summary?.network.avgTotalMs, fullNumberFormatter),
      "text-base-content",
    ],
  ].map(([label, value, toneClass]) => ({
    label,
    value: isLoading && summary == null ? "…" : value,
    toneClass,
  }));
}

function ActivityMetricGrid({
  metrics,
}: {
  metrics: Array<{ label: string; value: string; toneClass: string }>;
}) {
  return (
    <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4 xl:grid-cols-7">
      {metrics.map((metric) => (
        <div
          key={metric.label}
          className="rounded-lg border border-base-300/60 bg-base-200/25 px-3 py-2"
        >
          <div className="text-[11px] font-semibold uppercase tracking-[0.12em] text-base-content/55">
            {metric.label}
          </div>
          <div className={`mt-1 text-lg font-semibold ${metric.toneClass}`}>{metric.value}</div>
        </div>
      ))}
    </div>
  );
}
