import { Fragment, useEffect, useId, useMemo, useRef, useState } from "react";
import { useTranslation } from "../i18n";
import type {
  ApiInvocation,
  PromptCacheConversation,
  PromptCacheConversationInvocationPreview,
  PromptCacheConversationUpstreamAccount,
  PromptCacheConversationsResponse,
} from "../lib/api";
import { fetchInvocationRecords } from "../lib/api";
import { InvocationAccountDetailDrawer } from "./InvocationAccountDetailDrawer";
import { AccountDetailDrawerShell } from "./AccountDetailDrawerShell";
import { AppIcon } from "./AppIcon";
import { InvocationTable } from "./InvocationTable";
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
  expandedPromptCacheKeys?: string[];
  onToggleExpandedPromptCacheKey?: (promptCacheKey: string) => void;
}

const PROMPT_CACHE_NOW_TICK_MS = 30_000;
const PROMPT_CACHE_CHART_MAX_WINDOW_MS = 24 * 3_600_000;
const PROMPT_CACHE_HISTORY_PAGE_SIZE = 200;

type PromptCachePreviewRecordExtras = Partial<
  Pick<
    ApiInvocation,
    | "source"
    | "inputTokens"
    | "outputTokens"
    | "cacheInputTokens"
    | "reasoningTokens"
    | "reasoningEffort"
    | "errorMessage"
    | "failureKind"
    | "isActionable"
    | "responseContentEncoding"
    | "requestedServiceTier"
    | "serviceTier"
    | "tReqReadMs"
    | "tReqParseMs"
    | "tUpstreamConnectMs"
    | "tUpstreamTtfbMs"
    | "tUpstreamStreamMs"
    | "tRespParseMs"
    | "tPersistMs"
    | "tTotalMs"
  >
>;

function parseEpoch(raw?: string | null) {
  if (!raw) return null;
  const epoch = Date.parse(raw);
  return Number.isNaN(epoch) ? null : epoch;
}

function formatNumber(value: number | null | undefined, formatter: Intl.NumberFormat) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
  return formatter.format(value);
}

function formatCurrency(value: number | null | undefined, formatter: Intl.NumberFormat) {
  if (typeof value !== "number" || !Number.isFinite(value)) return FALLBACK_CELL;
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

function normalizePromptCacheInvocationPreview(
  preview: PromptCacheConversationInvocationPreview,
): PromptCacheConversationInvocationPreview {
  return {
    id: preview.id,
    invokeId: preview.invokeId,
    occurredAt: preview.occurredAt,
    status: preview.status?.trim() || "unknown",
    failureClass: preview.failureClass ?? null,
    routeMode: preview.routeMode?.trim() || null,
    model: preview.model?.trim() || null,
    totalTokens: preview.totalTokens ?? 0,
    cost: preview.cost ?? null,
    proxyDisplayName: preview.proxyDisplayName?.trim() || null,
    upstreamAccountId: preview.upstreamAccountId ?? null,
    upstreamAccountName: preview.upstreamAccountName?.trim() || null,
    endpoint: preview.endpoint?.trim() || null,
  };
}

function buildInvocationTableRecordFromPreview(
  preview: PromptCacheConversationInvocationPreview,
): ApiInvocation {
  const normalizedPreview = normalizePromptCacheInvocationPreview(preview);
  const previewExtras = preview as PromptCacheConversationInvocationPreview &
    PromptCachePreviewRecordExtras;

  return {
    id: normalizedPreview.id,
    invokeId: normalizedPreview.invokeId,
    occurredAt: normalizedPreview.occurredAt,
    source: previewExtras.source ?? undefined,
    status: normalizedPreview.status,
    failureClass: normalizedPreview.failureClass ?? undefined,
    failureKind: previewExtras.failureKind ?? undefined,
    isActionable: previewExtras.isActionable,
    model: normalizedPreview.model ?? undefined,
    inputTokens: previewExtras.inputTokens,
    outputTokens: previewExtras.outputTokens,
    cacheInputTokens: previewExtras.cacheInputTokens,
    reasoningTokens: previewExtras.reasoningTokens,
    reasoningEffort: previewExtras.reasoningEffort,
    totalTokens: normalizedPreview.totalTokens,
    cost: normalizedPreview.cost ?? undefined,
    errorMessage: previewExtras.errorMessage ?? undefined,
    endpoint: normalizedPreview.endpoint ?? undefined,
    routeMode: normalizedPreview.routeMode ?? undefined,
    upstreamAccountId: normalizedPreview.upstreamAccountId,
    upstreamAccountName: normalizedPreview.upstreamAccountName ?? undefined,
    proxyDisplayName: normalizedPreview.proxyDisplayName ?? undefined,
    responseContentEncoding: previewExtras.responseContentEncoding ?? undefined,
    requestedServiceTier: previewExtras.requestedServiceTier ?? undefined,
    serviceTier: previewExtras.serviceTier ?? undefined,
    tReqReadMs: previewExtras.tReqReadMs,
    tReqParseMs: previewExtras.tReqParseMs,
    tUpstreamConnectMs: previewExtras.tUpstreamConnectMs,
    tUpstreamTtfbMs: previewExtras.tUpstreamTtfbMs,
    tUpstreamStreamMs: previewExtras.tUpstreamStreamMs,
    tRespParseMs: previewExtras.tRespParseMs,
    tPersistMs: previewExtras.tPersistMs,
    tTotalMs: previewExtras.tTotalMs,
    createdAt: normalizedPreview.occurredAt,
  };
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
  onOpenAccountDetail?: (account: PromptCacheConversationUpstreamAccount) => void;
}) {
  if (upstreamAccounts.length === 0) {
    return <div className="text-[11px] text-base-content/55">{FALLBACK_CELL}</div>;
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
}: {
  records: ApiInvocation[];
  isLoading: boolean;
  error?: string | null;
  emptyLabel: string;
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
    />
  );
}

function PromptCacheConversationHistoryDrawer({
  open,
  promptCacheKey,
  onClose,
  t,
}: {
  open: boolean;
  promptCacheKey: string | null;
  onClose: () => void;
  t: (key: string, values?: Record<string, string | number>) => string;
}) {
  const titleId = useId();
  const requestSeqRef = useRef(0);
  const [records, setRecords] = useState<ApiInvocation[]>([]);
  const [total, setTotal] = useState(0);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open || !promptCacheKey) {
      requestSeqRef.current += 1;
      setRecords([]);
      setTotal(0);
      setIsLoading(false);
      setError(null);
      return;
    }

    const requestSeq = requestSeqRef.current + 1;
    requestSeqRef.current = requestSeq;
    setRecords([]);
    setTotal(0);
    setIsLoading(true);
    setError(null);

    void (async () => {
      let page = 1;
      let snapshotId: number | undefined;
      let loaded: ApiInvocation[] = [];
      let totalRecords = 0;

      while (true) {
        const response = await fetchInvocationRecords({
          promptCacheKey,
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

      if (requestSeq === requestSeqRef.current) {
        setIsLoading(false);
      }
    })().catch((err) => {
      if (requestSeq !== requestSeqRef.current) return;
      setError(err instanceof Error ? err.message : String(err));
      setIsLoading(false);
    });
  }, [open, promptCacheKey]);

  const loadedCount = records.length;

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
              {promptCacheKey || FALLBACK_CELL}
            </h2>
            <p className="section-description">
              {t("live.conversations.drawer.description")}
            </p>
          </div>
          <div className="text-sm text-base-content/70">
            {total > 0 && loadedCount >= total
              ? t("live.conversations.drawer.progressComplete", {
                  count: total,
                })
              : t("live.conversations.drawer.progress", {
                  loaded: loadedCount,
                  total,
                })}
          </div>
        </div>
      }
    >
      <div className="space-y-3">
        <PromptCacheConversationInvocationTable
          records={records}
          isLoading={isLoading}
          error={error}
          emptyLabel={t("live.conversations.drawer.empty")}
        />
        {isLoading && records.length > 0 ? (
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
}: PromptCacheConversationTableProps) {
  const { t, locale } = useTranslation();
  const [now, setNow] = useState(() => Date.now());
  const [drawerAccountId, setDrawerAccountId] = useState<number | null>(null);
  const [drawerAccountLabel, setDrawerAccountLabel] = useState<string | null>(null);
  const [historyDrawerPromptCacheKey, setHistoryDrawerPromptCacheKey] = useState<string | null>(null);
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

  const openAccountDrawer = (account: PromptCacheConversationUpstreamAccount) => {
    if (!canOpenPromptCacheUpstreamAccount(account)) return;
    setHistoryDrawerPromptCacheKey(null);
    setDrawerAccountId(Math.trunc(Number(account.upstreamAccountId)));
    setDrawerAccountLabel(resolveUpstreamAccountLabel(account, fallbackAccountLabel));
  };
  const closeAccountDrawer = () => {
    setDrawerAccountId(null);
    setDrawerAccountLabel(null);
  };
  const openHistoryDrawer = (promptCacheKey: string) => {
    closeAccountDrawer();
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
                        {t("live.conversations.table.promptCacheKey")}
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
                        onClick={() => openHistoryDrawer(conversation.promptCacheKey)}
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
                          buildInvocationTableRecordFromPreview,
                        )}
                        isLoading={false}
                        emptyLabel={previewLabels.empty}
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
                {t("live.conversations.table.promptCacheKey")}
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
                            {formatDateLabel(conversation.createdAt, dateFormatter)}
                          </span>
                        </div>
                        <div className="grid grid-cols-[2rem_minmax(0,1fr)] items-center gap-x-2">
                          <span className="text-base-content/60">
                            {totalLabels.lastActivityAtShort}
                          </span>
                          <span className="whitespace-nowrap font-medium tabular-nums">
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
                  {isExpanded ? (
                    <tr className="bg-base-200/20">
                      <td colSpan={5} className="px-3 pb-4 pt-0">
                        <div className="border-t border-base-300/60 pt-3">
                          <PromptCacheConversationInvocationTable
                            records={conversation.recentInvocations.map(
                              buildInvocationTableRecordFromPreview,
                            )}
                            isLoading={false}
                            emptyLabel={previewLabels.empty}
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
      <InvocationAccountDetailDrawer
        open={drawerAccountId != null}
        accountId={drawerAccountId}
        accountLabel={drawerAccountLabel}
        onClose={closeAccountDrawer}
      />
      <PromptCacheConversationHistoryDrawer
        open={historyDrawerPromptCacheKey != null}
        promptCacheKey={historyDrawerPromptCacheKey}
        onClose={closeHistoryDrawer}
        t={t}
      />
    </div>
  );
}
