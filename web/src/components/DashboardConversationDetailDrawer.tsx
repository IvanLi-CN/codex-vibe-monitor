import { useId, useMemo } from "react";
import { useTranslation } from "../i18n";
import type {
  DashboardWorkingConversationInvocationModel,
  DashboardWorkingConversationSelection,
} from "../lib/dashboardWorkingConversations";
import { formatDashboardWorkingConversationSequenceId } from "../lib/dashboardWorkingConversations";
import { cn } from "../lib/utils";
import { AccountDetailDrawerShell } from "./AccountDetailDrawerShell";
import { AppIcon } from "./AppIcon";
import { FALLBACK_CELL } from "./invocation-details-shared";
import {
  PromptCacheConversationInvocationTable,
  usePromptCacheConversationHistory,
} from "./prompt-cache-conversation-history-shared";
import { Badge } from "./ui/badge";

interface DashboardConversationDetailDrawerProps {
  open: boolean;
  selection: DashboardWorkingConversationSelection | null;
  onClose: () => void;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
}

type SnapshotToneMeta = {
  badgeVariant:
    | "default"
    | "secondary"
    | "success"
    | "warning"
    | "error"
    | "info";
  icon:
    | "loading"
    | "timer-refresh-outline"
    | "check-circle-outline"
    | "alert-outline"
    | "alert-circle-outline"
    | "information-outline";
};

const SNAPSHOT_TONE_META: Record<string, SnapshotToneMeta> = {
  running: { badgeVariant: "default", icon: "loading" },
  pending: { badgeVariant: "warning", icon: "timer-refresh-outline" },
  success: { badgeVariant: "success", icon: "check-circle-outline" },
  warning: { badgeVariant: "warning", icon: "alert-outline" },
  error: { badgeVariant: "error", icon: "alert-circle-outline" },
  neutral: { badgeVariant: "secondary", icon: "information-outline" },
};

function formatStatusLabel(status: string) {
  const normalized = status.trim();
  if (!normalized) return null;
  const lower = normalized.toLowerCase();
  if (lower === "completed") return "completed";
  if (lower.startsWith("http_")) {
    const code = lower.slice("http_".length);
    if (/^\d{3}$/.test(code)) return `HTTP ${code}`;
  }
  return normalized;
}

function formatTimestamp(
  epoch: number | null | undefined,
  formatter: Intl.DateTimeFormat,
  fallback?: string | null,
) {
  if (typeof epoch === "number" && Number.isFinite(epoch)) {
    return formatter.format(new Date(epoch));
  }
  const normalized = fallback?.trim();
  return normalized || FALLBACK_CELL;
}

function SnapshotField({
  label,
  value,
  title,
}: {
  label: string;
  value: React.ReactNode;
  title?: string;
}) {
  return (
    <div className="grid min-w-0 grid-cols-[4.5rem_minmax(0,1fr)] items-start gap-2">
      <span className="pt-[1px] text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/55">
        {label}
      </span>
      <div
        className="min-w-0 font-mono text-[11px] leading-[1.45] text-base-content/84"
        title={title}
      >
        {value}
      </div>
    </div>
  );
}

function SummaryMetric({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  return (
    <div className="rounded-2xl border border-base-300/70 bg-base-100/65 px-3 py-2.5">
      <div className="text-[10px] font-semibold uppercase tracking-[0.1em] text-base-content/55">
        {label}
      </div>
      <div className="mt-1 font-mono text-sm font-semibold text-base-content">
        {value}
      </div>
    </div>
  );
}

function InvocationSnapshotCard({
  invocation,
  label,
  locale,
  onOpenUpstreamAccount,
}: {
  invocation: DashboardWorkingConversationInvocationModel | null;
  label: string;
  locale: "zh" | "en";
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
}) {
  const { t } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const dateTimeFormatter = useMemo(
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
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag),
    [localeTag],
  );
  const currencyFormatter = useMemo(
    () =>
      new Intl.NumberFormat(localeTag, {
        style: "currency",
        currency: "USD",
        minimumFractionDigits: 4,
        maximumFractionDigits: 4,
      }),
    [localeTag],
  );

  if (!invocation) {
    return (
      <div className="rounded-[1.35rem] border border-dashed border-base-300/75 bg-base-100/45 px-4 py-4">
        <div className="text-sm font-semibold text-base-content">{label}</div>
        <p className="mt-2 text-sm leading-6 text-base-content/65">
          {t("dashboard.workingConversations.previousPlaceholder")}
        </p>
      </div>
    );
  }

  const toneMeta = SNAPSHOT_TONE_META[invocation.tone] ?? SNAPSHOT_TONE_META.neutral;
  const rawStatusLabel =
    formatStatusLabel(invocation.displayStatus) ?? t("table.status.unknown");
  const lowerStatus = rawStatusLabel.trim().toLowerCase();
  const translatedStatusLabel =
    lowerStatus === "running"
      ? t("table.status.running")
      : lowerStatus === "pending"
        ? t("table.status.pending")
        : lowerStatus === "success" || lowerStatus === "completed"
          ? t("table.status.success")
          : lowerStatus === "failed"
            ? t("table.status.failed")
            : lowerStatus === "interrupted"
              ? t("table.status.interrupted")
              : rawStatusLabel;
  const accountLabel =
    invocation.record.upstreamAccountName?.trim() ||
    (typeof invocation.record.upstreamAccountId === "number"
      ? t("live.conversations.accountLabel.idFallback", {
          id: Math.trunc(invocation.record.upstreamAccountId),
        })
      : t("live.conversations.invocations.identityUnavailable"));
  const accountId =
    typeof invocation.record.upstreamAccountId === "number" &&
    Number.isFinite(invocation.record.upstreamAccountId)
      ? Math.trunc(invocation.record.upstreamAccountId)
      : null;
  const occurredAtLabel = formatTimestamp(
    invocation.occurredAtEpoch,
    dateTimeFormatter,
    invocation.preview.occurredAt,
  );
  const endpointLabel = invocation.record.endpoint?.trim() || FALLBACK_CELL;
  const modelLabel = invocation.record.model?.trim() || FALLBACK_CELL;
  const totalTokensLabel =
    typeof invocation.record.totalTokens === "number" &&
    Number.isFinite(invocation.record.totalTokens)
      ? numberFormatter.format(invocation.record.totalTokens)
      : FALLBACK_CELL;
  const costLabel =
    typeof invocation.record.cost === "number" &&
    Number.isFinite(invocation.record.cost)
      ? currencyFormatter.format(invocation.record.cost)
      : FALLBACK_CELL;

  return (
    <div className="rounded-[1.35rem] border border-base-300/70 bg-base-100/65 px-4 py-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-sm font-semibold text-base-content">{label}</h3>
        <Badge
          variant={toneMeta.badgeVariant}
          className="h-6 gap-1 border-transparent bg-base-100/12 px-2.5 py-0 text-[11px] font-semibold shadow-none"
        >
          <AppIcon
            name={toneMeta.icon}
            className={cn(
              "h-3.5 w-3.5",
              invocation.isInFlight &&
                "motion-safe:animate-spin motion-reduce:animate-none",
            )}
            aria-hidden
          />
          <span>{translatedStatusLabel}</span>
        </Badge>
      </div>

      <div className="mt-3 space-y-2.5">
        <SnapshotField label={t("dashboard.workingConversations.conversationDrawer.occurredAtLabel")} value={occurredAtLabel} />
        <SnapshotField
          label={t("dashboard.workingConversations.conversationDrawer.accountLabel")}
          value={
            accountId != null ? (
              <button
                type="button"
                className="inline-flex max-w-full min-w-0 rounded-[0.6rem] border border-base-300/65 bg-base-100/12 px-2 py-1 text-left transition hover:bg-base-100/18 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                onClick={() => onOpenUpstreamAccount?.(accountId, accountLabel)}
                title={accountLabel}
              >
                <span className="truncate">{accountLabel}</span>
              </button>
            ) : (
              <span className="truncate">{accountLabel}</span>
            )
          }
          title={accountLabel}
        />
        <SnapshotField
          label={t("dashboard.workingConversations.modelLabel")}
          value={modelLabel}
          title={modelLabel}
        />
        <SnapshotField
          label={t("live.conversations.invocations.endpoint")}
          value={endpointLabel}
          title={endpointLabel}
        />
        <SnapshotField
          label={t("dashboard.workingConversations.totalTokensLabel")}
          value={totalTokensLabel}
        />
        <SnapshotField
          label={t("dashboard.workingConversations.totalCostLabel")}
          value={costLabel}
        />
      </div>
    </div>
  );
}

export function DashboardConversationDetailDrawer({
  open,
  selection,
  onClose,
  onOpenUpstreamAccount,
}: DashboardConversationDetailDrawerProps) {
  const { t, locale } = useTranslation();
  const titleId = useId();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag),
    [localeTag],
  );
  const currencyFormatter = useMemo(
    () =>
      new Intl.NumberFormat(localeTag, {
        style: "currency",
        currency: "USD",
        minimumFractionDigits: 4,
        maximumFractionDigits: 4,
      }),
    [localeTag],
  );
  const dateTimeFormatter = useMemo(
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
  const {
    visibleRecords,
    effectiveTotal,
    loadedCount,
    isLoading,
    error,
    hasHydrated,
  } = usePromptCacheConversationHistory({
    open,
    conversationKey: selection?.promptCacheKey ?? null,
  });

  const progressLabel = useMemo(() => {
    if (loadedCount < effectiveTotal) {
      return t(
        "dashboard.workingConversations.conversationDrawer.historyProgress",
        {
          loaded: loadedCount,
          total: effectiveTotal,
        },
      );
    }
    return t(
      "dashboard.workingConversations.conversationDrawer.historyProgressComplete",
      {
        count: effectiveTotal,
      },
    );
  }, [effectiveTotal, loadedCount, t]);

  const displaySequenceId = selection
    ? formatDashboardWorkingConversationSequenceId(
        selection.conversationSequenceId,
      )
    : FALLBACK_CELL;
  const createdAtLabel = formatTimestamp(
    selection?.createdAtEpoch,
    dateTimeFormatter,
  );
  const lastActivityAtLabel = formatTimestamp(
    selection?.lastActivityAtEpoch,
    dateTimeFormatter,
    selection?.currentInvocation.preview.occurredAt,
  );
  const tableIsLoading =
    isLoading ||
    (open &&
      selection?.promptCacheKey != null &&
      !hasHydrated &&
      visibleRecords.length === 0 &&
      error == null);

  return (
    <AccountDetailDrawerShell
      open={open}
      labelledBy={titleId}
      closeLabel={t(
        "dashboard.workingConversations.conversationDrawer.close",
      )}
      onClose={onClose}
      shellClassName="max-w-[72rem]"
      header={
        <div className="space-y-2">
          <p className="text-xs font-semibold uppercase tracking-[0.18em] text-primary/70">
            {t("dashboard.workingConversations.conversationDrawer.subtitle")}
          </p>
          <div className="space-y-1">
            <h2 id={titleId} className="text-xl font-semibold text-base-content">
              {t("dashboard.workingConversations.conversationDrawer.title", {
                sequenceId: displaySequenceId,
              })}
            </h2>
            <p className="text-sm leading-6 text-base-content/70">
              {t("dashboard.workingConversations.conversationDrawer.description")}
            </p>
          </div>
        </div>
      }
    >
      <div
        data-testid="dashboard-conversation-detail-drawer"
        className="space-y-5"
      >
        <section className="space-y-4">
          <div className="grid gap-3 sm:grid-cols-3">
            <SummaryMetric
              label={t("dashboard.workingConversations.requestCountLabel")}
              value={
                selection
                  ? numberFormatter.format(selection.requestCount)
                  : FALLBACK_CELL
              }
            />
            <SummaryMetric
              label={t("dashboard.workingConversations.totalTokensLabel")}
              value={
                selection
                  ? numberFormatter.format(selection.totalTokens)
                  : FALLBACK_CELL
              }
            />
            <SummaryMetric
              label={t("dashboard.workingConversations.totalCostLabel")}
              value={
                selection
                  ? currencyFormatter.format(selection.totalCost)
                  : FALLBACK_CELL
              }
            />
          </div>

          <div className="rounded-[1.6rem] border border-base-300/70 bg-base-100/65 px-4 py-4">
            <div className="grid gap-3 md:grid-cols-3">
              <SnapshotField
                label={t(
                  "dashboard.workingConversations.promptCacheKeyLabel",
                )}
                value={
                  <span className="block break-all">
                    {selection?.promptCacheKey ?? FALLBACK_CELL}
                  </span>
                }
                title={selection?.promptCacheKey ?? undefined}
              />
              <SnapshotField
                label={t(
                  "dashboard.workingConversations.conversationDrawer.createdAtLabel",
                )}
                value={createdAtLabel}
              />
              <SnapshotField
                label={t(
                  "dashboard.workingConversations.conversationDrawer.lastActivityAtLabel",
                )}
                value={lastActivityAtLabel}
              />
            </div>
          </div>

          <div className="grid gap-3 xl:grid-cols-2">
            <InvocationSnapshotCard
              invocation={selection?.currentInvocation ?? null}
              label={t("dashboard.workingConversations.currentInvocation")}
              locale={locale}
              onOpenUpstreamAccount={onOpenUpstreamAccount}
            />
            <InvocationSnapshotCard
              invocation={selection?.previousInvocation ?? null}
              label={t("dashboard.workingConversations.previousInvocation")}
              locale={locale}
              onOpenUpstreamAccount={onOpenUpstreamAccount}
            />
          </div>
        </section>

        <section className="space-y-3">
          <div className="space-y-1">
            <h3 className="text-base font-semibold text-base-content">
              {t("dashboard.workingConversations.conversationDrawer.historyTitle")}
            </h3>
            <div className="flex flex-wrap items-center justify-between gap-2 text-sm">
              <span className="text-base-content/70">{progressLabel}</span>
              {loadedCount > 0 && isLoading ? (
                <span className="text-xs text-base-content/58">
                  {t(
                    "dashboard.workingConversations.conversationDrawer.historyLoadingMore",
                  )}
                </span>
              ) : null}
            </div>
          </div>

          <PromptCacheConversationInvocationTable
            records={visibleRecords}
            isLoading={tableIsLoading}
            error={error}
            emptyLabel={t(
              "dashboard.workingConversations.conversationDrawer.historyEmpty",
            )}
            onOpenUpstreamAccount={onOpenUpstreamAccount}
          />
        </section>
      </div>
    </AccountDetailDrawerShell>
  );
}
