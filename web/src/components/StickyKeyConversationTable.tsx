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
  UpstreamStickyConversationsResponse,
} from "../lib/api";
import { fetchInvocationRecords } from "../lib/api";
import { buildInvocationFromPromptCachePreview } from "../lib/promptCacheLive";
import { AccountDetailDrawerShell } from "./AccountDetailDrawerShell";
import { AppIcon } from "./AppIcon";
import { InvocationTable } from "./InvocationTable";
import { ConversationSparkline } from "./KeyedConversationTable";
import {
  FALLBACK_CELL,
  findVisibleConversationChartMax,
} from "./keyedConversationChart";
import { Alert } from "./ui/alert";
import { Button } from "./ui/button";
import { Spinner } from "./ui/spinner";

interface StickyKeyConversationTableProps {
  accountId: number | null;
  stats: UpstreamStickyConversationsResponse | null;
  isLoading: boolean;
  error?: string | null;
  expandedStickyKeys?: string[];
  onToggleExpandedStickyKey?: (stickyKey: string) => void;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
}

const STICKY_HISTORY_PAGE_SIZE = 200;

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

function StickyConversationInvocationTable({
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

function StickyConversationHistoryDrawer({
  open,
  accountId,
  stickyKey,
  onClose,
  onOpenUpstreamAccount,
}: {
  open: boolean;
  accountId: number | null;
  stickyKey: string | null;
  onClose: () => void;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
}) {
  const { t } = useTranslation();
  const titleId = useId();
  const requestSeqRef = useRef(0);
  const snapshotIdRef = useRef<number | null>(null);
  const [records, setRecords] = useState<ApiInvocation[]>([]);
  const [total, setTotal] = useState(0);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [page, setPage] = useState(0);

  const runLoad = useCallback(
    async (nextPage: number, mode: "replace" | "append") => {
      if (!open || !stickyKey || accountId == null) return;

      const requestSeq = requestSeqRef.current + 1;
      requestSeqRef.current = requestSeq;
      setIsLoading(true);
      if (mode === "replace") {
        setError(null);
      }

      try {
        const response = await fetchInvocationRecords({
          stickyKey,
          upstreamAccountId: accountId,
          page: nextPage,
          pageSize: STICKY_HISTORY_PAGE_SIZE,
          sortBy: "occurredAt",
          sortOrder: "desc",
          ...(mode === "append" && snapshotIdRef.current != null
            ? { snapshotId: snapshotIdRef.current }
            : {}),
        });
        if (requestSeq !== requestSeqRef.current) return;

        snapshotIdRef.current = response.snapshotId;
        setTotal(response.total);
        setPage(nextPage);
        setRecords((current) =>
          mode === "append" ? [...current, ...response.records] : response.records,
        );
        setError(null);
      } catch (err) {
        if (requestSeq !== requestSeqRef.current) return;
        setError(err instanceof Error ? err.message : String(err));
      } finally {
        if (requestSeq === requestSeqRef.current) {
          setIsLoading(false);
        }
      }
    },
    [accountId, open, stickyKey],
  );

  useEffect(() => {
    requestSeqRef.current += 1;
    setRecords([]);
    setTotal(0);
    setPage(0);
    snapshotIdRef.current = null;
    setError(null);
    setIsLoading(false);

    if (!open || !stickyKey || accountId == null) {
      return;
    }

    void runLoad(1, "replace");
  }, [accountId, open, runLoad, stickyKey]);

  const hasMore = total > records.length;
  const handleLoadMore = useCallback(() => {
    if (isLoading || !hasMore) return;
    void runLoad(page + 1, "append");
  }, [hasMore, isLoading, page, runLoad]);

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
              {stickyKey || FALLBACK_CELL}
            </h2>
            <p className="section-description">
              {t("live.conversations.drawer.description")}
            </p>
          </div>
          <div className="text-sm text-base-content/70">
            {total > 0 && records.length >= total
              ? t("live.conversations.drawer.progressComplete", {
                  count: total,
                })
              : t("live.conversations.drawer.progress", {
                  loaded: records.length,
                  total,
                })}
          </div>
        </div>
      }
    >
      <div className="space-y-3">
        <StickyConversationInvocationTable
          records={records}
          isLoading={isLoading && records.length === 0}
          error={error}
          emptyLabel={t("live.conversations.drawer.empty")}
          onOpenUpstreamAccount={onOpenUpstreamAccount}
        />
        {hasMore && !isLoading ? (
          <div className="flex justify-center">
            <Button type="button" variant="outline" size="sm" onClick={handleLoadMore}>
              {t("live.conversations.drawer.loadMore")}
            </Button>
          </div>
        ) : null}
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

export function StickyKeyConversationTable({
  accountId,
  stats,
  isLoading,
  error,
  expandedStickyKeys,
  onToggleExpandedStickyKey,
  onOpenUpstreamAccount,
}: StickyKeyConversationTableProps) {
  const { t, locale } = useTranslation();
  const [historyDrawerStickyKey, setHistoryDrawerStickyKey] = useState<string | null>(null);
  const [internalExpandedStickyKeys, setInternalExpandedStickyKeys] = useState<string[]>([]);
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const isExpansionControlled = expandedStickyKeys != null;

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
        stats.selectionMode === "activityWindow"
        && stats.selectedActivityHours != null
      ) {
        return t(
          "live.conversations.implicitFilter.inactiveOutsideActivityWindow",
          {
            count: stats.implicitFilter.filteredCount,
            hours: stats.selectedActivityHours,
          },
        );
      }
      return t(
        "live.conversations.implicitFilter.inactiveOutside24h",
        { count: stats.implicitFilter.filteredCount },
      );
    }
    return t(
      "live.conversations.implicitFilter.cappedTo50",
      { count: stats.implicitFilter.filteredCount },
    );
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
  const rangeStart = stats?.rangeStart ?? "";
  const rangeEnd = stats?.rangeEnd ?? "";
  const chartHours = useMemo(() => {
    const startEpoch = Date.parse(rangeStart);
    const endEpoch = Date.parse(rangeEnd);
    if (!Number.isFinite(startEpoch) || !Number.isFinite(endEpoch) || endEpoch <= startEpoch) {
      return 24;
    }
    return Math.max(1, Math.ceil((endEpoch - startEpoch) / 3_600_000));
  }, [rangeEnd, rangeStart]);
  const chartAriaLabel = t("live.conversations.chartAria", { hours: chartHours });
  const chartColumnLabel = t("live.conversations.table.chartWindow", {
    hours: chartHours,
  });
  const conversationChartMax = useMemo(
    () =>
      findVisibleConversationChartMax(
        stats?.conversations ?? [],
        rangeStart,
        rangeEnd,
      ),
    [rangeEnd, rangeStart, stats?.conversations],
  );

  const effectiveExpandedStickyKeys = isExpansionControlled
    ? expandedStickyKeys
    : internalExpandedStickyKeys;
  const expandedStickyKeySet = useMemo(
    () => new Set(effectiveExpandedStickyKeys),
    [effectiveExpandedStickyKeys],
  );

  useEffect(() => {
    if (isExpansionControlled || !stats) return;
    const visibleStickyKeys = new Set(
      stats.conversations.map((conversation) => conversation.stickyKey),
    );
    setInternalExpandedStickyKeys((current) =>
      current.filter((stickyKey) => visibleStickyKeys.has(stickyKey)),
    );
  }, [isExpansionControlled, stats]);

  const openHistoryDrawer = (stickyKey: string) => {
    setHistoryDrawerStickyKey(stickyKey);
  };

  const closeHistoryDrawer = () => {
    setHistoryDrawerStickyKey(null);
  };

  const toggleStickyPreview = (stickyKey: string) => {
    if (!isExpansionControlled) {
      setInternalExpandedStickyKeys((current) =>
        current.includes(stickyKey)
          ? current.filter((value) => value !== stickyKey)
          : [...current, stickyKey],
      );
    }
    onToggleExpandedStickyKey?.(stickyKey);
  };

  const openAccountDrawer = useCallback(
    (nextAccountId: number, accountLabel: string) => {
      setHistoryDrawerStickyKey(null);
      onOpenUpstreamAccount?.(nextAccountId, accountLabel);
    },
    [onOpenUpstreamAccount],
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
        <Alert>{t("accountPool.upstreamAccounts.stickyConversations.empty")}</Alert>
        {footerNote ? (
          <p className="px-1 text-[11px] text-base-content/55">{footerNote}</p>
        ) : null}
      </div>
    );
  }

  return (
    <>
      <div className="space-y-2">
        <div className="overflow-hidden rounded-xl border border-base-300/75 bg-base-100/55">
          <div className="space-y-3 p-3 sm:hidden">
            {stats.conversations.map((conversation) => {
              const stickyKey = conversation.stickyKey;
              const isExpanded = expandedStickyKeySet.has(stickyKey);
              const previewRecords = conversation.recentInvocations.map(
                buildInvocationFromPromptCachePreview,
              );

              return (
                <article
                  key={`${stickyKey}-mobile`}
                  className="space-y-3 rounded-lg border border-base-300/70 bg-base-100/70 p-3"
                >
                  <div className="space-y-1">
                    <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                      {t("accountPool.upstreamAccounts.stickyConversations.table.stickyKey")}
                    </div>
                    <div className="break-all font-mono text-xs">{stickyKey}</div>
                  </div>

                  <dl className="grid grid-cols-2 gap-x-3 gap-y-2 text-xs">
                    <div>
                      <dt className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                        {t("live.conversations.table.requestCount")}
                      </dt>
                      <dd>{formatNumber(conversation.requestCount, numberFormatter)}</dd>
                    </div>
                    <div>
                      <dt className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                        {t("live.conversations.table.totalTokens")}
                      </dt>
                      <dd>{formatNumber(conversation.totalTokens, numberFormatter)}</dd>
                    </div>
                    <div>
                      <dt className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                        {t("live.conversations.table.totalCost")}
                      </dt>
                      <dd>{formatCurrency(conversation.totalCost, currencyFormatter)}</dd>
                    </div>
                    <div>
                      <dt className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                        {t("live.conversations.table.createdAt")}
                      </dt>
                      <dd>{formatDateLabel(conversation.createdAt, dateFormatter)}</dd>
                    </div>
                    <div className="col-span-2">
                      <dt className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                        {t("live.conversations.table.lastActivityAt")}
                      </dt>
                      <dd>{formatDateLabel(conversation.lastActivityAt, dateFormatter)}</dd>
                    </div>
                  </dl>

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
                      ariaLabel={`${stickyKey} ${chartAriaLabel}`}
                      conversationKey={stickyKey}
                    />
                  </div>

                  <div className="flex flex-wrap gap-2">
                    <Button
                      type="button"
                      size="sm"
                      variant="ghost"
                      className="gap-2"
                      onClick={() => toggleStickyPreview(stickyKey)}
                    >
                      <AppIcon
                        name={isExpanded ? "chevron-up" : "chevron-down"}
                        className="h-4 w-4"
                        aria-hidden
                      />
                      <span>
                        {isExpanded
                          ? t("live.conversations.actions.collapsePreview")
                          : t("live.conversations.actions.expandPreview")}
                      </span>
                    </Button>
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      className="gap-2"
                      onClick={() => openHistoryDrawer(stickyKey)}
                    >
                      <AppIcon name="database-outline" className="h-4 w-4" aria-hidden />
                      <span>{t("live.conversations.actions.openHistory")}</span>
                    </Button>
                  </div>

                  {isExpanded ? (
                    <div className="rounded-lg border border-base-300/65 bg-base-200/48 p-3">
                      <StickyConversationInvocationTable
                        records={previewRecords}
                        isLoading={false}
                        error={null}
                        emptyLabel={t("live.conversations.preview.empty")}
                        onOpenUpstreamAccount={openAccountDrawer}
                      />
                    </div>
                  ) : null}
                </article>
              );
            })}
          </div>

          <table className="hidden w-full table-fixed text-xs sm:table">
            <thead className="bg-base-200/70 uppercase tracking-[0.08em] text-base-content/65">
              <tr>
                <th className="w-[18%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                  {t("accountPool.upstreamAccounts.stickyConversations.table.stickyKey")}
                </th>
                <th className="w-[8%] px-2 py-2 text-right font-semibold sm:px-3 sm:py-3">
                  {t("live.conversations.table.requestCount")}
                </th>
                <th className="w-[12%] px-2 py-2 text-right font-semibold sm:px-3 sm:py-3">
                  {t("live.conversations.table.totalTokens")}
                </th>
                <th className="w-[12%] px-2 py-2 text-right font-semibold sm:px-3 sm:py-3">
                  {t("live.conversations.table.totalCost")}
                </th>
                <th className="w-[12%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                  {t("live.conversations.table.createdAt")}
                </th>
                <th className="w-[12%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                  {t("live.conversations.table.lastActivityAt")}
                </th>
                <th className="w-[12%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                  {chartColumnLabel}
                </th>
                <th className="w-[14%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
                  {t("accountPool.upstreamAccounts.stickyConversations.table.actions")}
                </th>
              </tr>
            </thead>
            <tbody className="divide-y divide-base-300/65">
              {stats.conversations.map((conversation) => {
                const stickyKey = conversation.stickyKey;
                const isExpanded = expandedStickyKeySet.has(stickyKey);
                const previewRecords = conversation.recentInvocations.map(
                  buildInvocationFromPromptCachePreview,
                );

                return (
                  <Fragment key={stickyKey}>
                    <tr className="transition-colors hover:bg-primary/6">
                      <td className="max-w-0 px-2 py-2 align-middle sm:px-3 sm:py-3">
                        <div className="truncate font-mono text-xs" title={stickyKey}>
                          {stickyKey}
                        </div>
                      </td>
                      <td className="px-2 py-2 text-right align-middle sm:px-3 sm:py-3">
                        {formatNumber(conversation.requestCount, numberFormatter)}
                      </td>
                      <td className="px-2 py-2 text-right align-middle sm:px-3 sm:py-3">
                        {formatNumber(conversation.totalTokens, numberFormatter)}
                      </td>
                      <td className="px-2 py-2 text-right align-middle sm:px-3 sm:py-3">
                        {formatCurrency(conversation.totalCost, currencyFormatter)}
                      </td>
                      <td className="px-2 py-2 align-middle sm:px-3 sm:py-3">
                        {formatDateLabel(conversation.createdAt, dateFormatter)}
                      </td>
                      <td className="px-2 py-2 align-middle sm:px-3 sm:py-3">
                        {formatDateLabel(conversation.lastActivityAt, dateFormatter)}
                      </td>
                      <td className="px-2 py-2 align-middle sm:px-3 sm:py-3">
                        <ConversationSparkline
                          conversation={conversation}
                          rangeStart={rangeStart}
                          rangeEnd={rangeEnd}
                          maxCumulativeTokens={conversationChartMax}
                          localeTag={localeTag}
                          tooltipLabels={tooltipLabels}
                          interactionHint={chartInteractionHint}
                          ariaLabel={`${stickyKey} ${chartAriaLabel}`}
                          conversationKey={stickyKey}
                        />
                      </td>
                      <td className="px-2 py-2 align-middle sm:px-3 sm:py-3">
                        <div className="flex flex-wrap items-center gap-2">
                          <Button
                            type="button"
                            size="sm"
                            variant="ghost"
                            className="h-8 gap-2 px-2"
                            onClick={() => toggleStickyPreview(stickyKey)}
                            aria-expanded={isExpanded}
                          >
                            <AppIcon
                              name={isExpanded ? "chevron-up" : "chevron-down"}
                              className="h-4 w-4"
                              aria-hidden
                            />
                            <span className="sr-only">
                              {isExpanded
                                ? t("live.conversations.actions.collapsePreview")
                                : t("live.conversations.actions.expandPreview")}
                            </span>
                          </Button>
                          <Button
                            type="button"
                            size="sm"
                            variant="outline"
                            className="h-8 gap-2 px-2"
                            onClick={() => openHistoryDrawer(stickyKey)}
                          >
                            <AppIcon name="database-outline" className="h-4 w-4" aria-hidden />
                            <span>{t("live.conversations.actions.openHistory")}</span>
                          </Button>
                        </div>
                      </td>
                    </tr>
                    {isExpanded ? (
                      <tr className="bg-base-200/25">
                        <td colSpan={8} className="px-3 py-3">
                          <StickyConversationInvocationTable
                            records={previewRecords}
                            isLoading={false}
                            error={null}
                            emptyLabel={t("live.conversations.preview.empty")}
                            onOpenUpstreamAccount={openAccountDrawer}
                          />
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
      </div>

      <StickyConversationHistoryDrawer
        open={historyDrawerStickyKey != null}
        accountId={accountId}
        stickyKey={historyDrawerStickyKey}
        onClose={closeHistoryDrawer}
        onOpenUpstreamAccount={openAccountDrawer}
      />
    </>
  );
}
