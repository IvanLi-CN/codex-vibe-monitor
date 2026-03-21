import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "../i18n";
import type {
  PromptCacheConversation,
  PromptCacheConversationUpstreamAccount,
  PromptCacheConversationsResponse,
} from "../lib/api";
import { ConversationSparkline } from "./KeyedConversationTable";
import {
  FALLBACK_CELL,
  findVisibleConversationChartMax,
} from "./keyedConversationChart";
import { Alert } from "./ui/alert";
import { Spinner } from "./ui/spinner";

interface PromptCacheConversationTableProps {
  stats: PromptCacheConversationsResponse | null;
  isLoading: boolean;
  error?: string | null;
}

const PROMPT_CACHE_NOW_TICK_MS = 30_000;
const PROMPT_CACHE_CHART_MAX_WINDOW_MS = 24 * 3_600_000;

function parseEpoch(raw?: string | null) {
  if (!raw) return null;
  const epoch = Date.parse(raw);
  return Number.isNaN(epoch) ? null : epoch;
}

function formatNumber(value: number, formatter: Intl.NumberFormat) {
  if (!Number.isFinite(value)) return FALLBACK_CELL;
  return formatter.format(value);
}

function formatCurrency(value: number, formatter: Intl.NumberFormat) {
  if (!Number.isFinite(value)) return FALLBACK_CELL;
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
}: {
  upstreamAccounts: PromptCacheConversationUpstreamAccount[];
  labels: {
    requestCountCompact: string;
    totalTokensCompact: string;
  };
  numberFormatter: Intl.NumberFormat;
  currencyFormatter: Intl.NumberFormat;
  fallbackAccountLabel: (id: number) => string;
}) {
  if (upstreamAccounts.length === 0) {
    return <div className="text-[11px] text-base-content/55">{FALLBACK_CELL}</div>;
  }

  return (
    <div className="space-y-1.5">
      {upstreamAccounts.slice(0, 3).map((account, index) => (
        <div
          key={`${account.upstreamAccountId ?? "unknown"}-${account.upstreamAccountName ?? "none"}-${index}`}
          className="grid grid-cols-[7.5rem_minmax(0,1fr)] items-center gap-x-2 text-[11px]"
        >
          <span className="truncate font-medium">
            {resolveUpstreamAccountLabel(account, fallbackAccountLabel)}
          </span>
          <span className="min-w-0 truncate text-base-content/62">
            {formatNumber(account.requestCount, numberFormatter)}
            {" "}
            {labels.requestCountCompact}
            {" · "}
            {labels.totalTokensCompact}
            {" "}
            {formatNumber(account.totalTokens, numberFormatter)}
            {" · "}
            {formatCurrency(account.totalCost, currencyFormatter)}
          </span>
        </div>
      ))}
    </div>
  );
}

export function PromptCacheConversationTable({
  stats,
  isLoading,
  error,
}: PromptCacheConversationTableProps) {
  const { t, locale } = useTranslation();
  const [now, setNow] = useState(() => Date.now());
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";

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
  const chartAriaLabel = t("live.conversations.chartAria", { hours: chartHours });
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
  const fallbackAccountLabel = useMemo(
    () => (id: number) =>
      t("live.conversations.accountLabel.idFallback", {
        id: String(Math.trunc(id)),
      }),
    [t],
  );

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
        <Alert>{t("live.conversations.empty")}</Alert>
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

            return (
              <article
                key={`${conversation.promptCacheKey}-mobile`}
                className="space-y-3 rounded-lg border border-base-300/70 bg-base-100/70 p-3"
              >
                <div className="space-y-1">
                  <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                    {t("live.conversations.table.promptCacheKey")}
                  </div>
                  <div className="break-all font-mono text-xs">
                    {conversation.promptCacheKey}
                  </div>
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
              <th className="w-[19%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                {t("live.conversations.table.promptCacheKey")}
              </th>
              <th className="w-[36%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                {t("live.conversations.table.upstreamAccounts")}
              </th>
              <th className="w-[16%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                {t("live.conversations.table.summary")}
              </th>
              <th className="w-[11%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                {totalLabels.time}
              </th>
              <th className="w-[18%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                {chartColumnLabel}
              </th>
            </tr>
          </thead>
          <tbody className="divide-y divide-base-300/65">
            {stats.conversations.map((conversation) => (
              <tr
                key={conversation.promptCacheKey}
                className="transition-colors hover:bg-primary/6"
              >
                <td className="max-w-0 px-2 py-2 align-top sm:px-3 sm:py-3">
                  <div
                    className="truncate font-mono text-xs"
                    title={conversation.promptCacheKey}
                  >
                    {conversation.promptCacheKey}
                  </div>
                </td>
                <td className="px-2 py-2 align-top sm:px-3 sm:py-3">
                  <UpstreamAccountsBlock
                    upstreamAccounts={conversation.upstreamAccounts}
                    labels={totalLabels}
                    numberFormatter={numberFormatter}
                    currencyFormatter={currencyFormatter}
                    fallbackAccountLabel={fallbackAccountLabel}
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
                    <div className="grid grid-cols-[2.5rem_minmax(0,1fr)] items-center gap-x-2">
                      <span className="text-base-content/60">
                        {totalLabels.createdAtShort}
                      </span>
                      <span className="min-w-0 truncate">
                        {formatDateLabel(conversation.createdAt, dateFormatter)}
                      </span>
                    </div>
                    <div className="grid grid-cols-[2.5rem_minmax(0,1fr)] items-center gap-x-2">
                      <span className="text-base-content/60">
                        {totalLabels.lastActivityAtShort}
                      </span>
                      <span className="min-w-0 truncate">
                        {formatDateLabel(conversation.lastActivityAt, dateFormatter)}
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
            ))}
          </tbody>
        </table>
      </div>
      {footerNote ? (
        <p className="px-1 text-[11px] text-base-content/55">{footerNote}</p>
      ) : null}
    </div>
  );
}
