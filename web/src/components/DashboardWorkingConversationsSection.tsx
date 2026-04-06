import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { useTranslation } from "../i18n";
import type { TranslationKey } from "../i18n";
import type {
  DashboardWorkingConversationCardModel,
  DashboardWorkingConversationInvocationModel,
  DashboardWorkingConversationTone,
} from "../lib/dashboardWorkingConversations";
import { cn } from "../lib/utils";
import { AppIcon } from "./AppIcon";
import { Alert } from "./ui/alert";
import { Badge } from "./ui/badge";
import { Spinner } from "./ui/spinner";
import {
  FALLBACK_CELL,
  buildInvocationDetailViewModel,
  renderEndpointSummary,
  renderFastIndicator,
} from "./invocation-details-shared";

interface DashboardWorkingConversationsSectionProps {
  cards: DashboardWorkingConversationCardModel[];
  isLoading: boolean;
  error?: string | null;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
}

type StatusMeta = {
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
  labelKey?: TranslationKey;
  label?: string;
  cardToneClassName: string;
  slotSurfaceClassName: string;
  beaconClassName: string;
};

const CARD_CLASS_NAME =
  "relative overflow-hidden rounded-[1.1rem] p-2.5 sm:p-3 shadow-[inset_0_1px_0_rgba(255,255,255,0.04),0_16px_28px_rgba(2,6,23,0.18)] transition-shadow duration-200 hover:shadow-[inset_0_1px_0_rgba(255,255,255,0.05),0_20px_34px_rgba(2,6,23,0.22)] focus-within:shadow-[inset_0_1px_0_rgba(255,255,255,0.06),0_0_0_1px_rgba(56,189,248,0.2),0_20px_34px_rgba(2,6,23,0.22)]";

const SLOT_CLASS_NAME =
  "flex flex-col overflow-hidden rounded-[0.95rem] px-2.5 py-2 shadow-[inset_0_1px_0_rgba(255,255,255,0.04)]";

const CARD_SURFACE_CLASS_NAME = "working-conversation-card-surface";

const INVOCATION_SURFACE_CLASS_NAME = "working-conversation-slot-surface";

const STATUS_META: Record<DashboardWorkingConversationTone, StatusMeta> = {
  running: {
    badgeVariant: "default",
    icon: "loading",
    labelKey: "table.status.running",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
    beaconClassName: "bg-primary/85",
  },
  pending: {
    badgeVariant: "warning",
    icon: "timer-refresh-outline",
    labelKey: "table.status.pending",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
    beaconClassName: "bg-warning/85",
  },
  success: {
    badgeVariant: "success",
    icon: "check-circle-outline",
    labelKey: "table.status.success",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
    beaconClassName: "bg-success/85",
  },
  warning: {
    badgeVariant: "warning",
    icon: "alert-outline",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
    beaconClassName: "bg-warning/85",
  },
  error: {
    badgeVariant: "error",
    icon: "alert-circle-outline",
    labelKey: "table.status.failed",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
    beaconClassName: "bg-error/90",
  },
  neutral: {
    badgeVariant: "secondary",
    icon: "information-outline",
    labelKey: "table.status.unknown",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
    beaconClassName: "bg-base-content/55",
  },
};

function formatStatusLabel(status: string) {
  const normalized = status.trim();
  if (!normalized) return null;
  const lower = normalized.toLowerCase();
  if (lower.startsWith("http_")) {
    const code = lower.slice("http_".length);
    if (/^\d{3}$/.test(code)) return `HTTP ${code}`;
    return normalized.toUpperCase().replace("_", " ");
  }
  return normalized;
}

function formatCompactAccountLabel(accountLabel: string) {
  const normalized = accountLabel.trim();
  if (!normalized) return FALLBACK_CELL;

  const atIndex = normalized.indexOf("@");
  const base = atIndex > 0 ? normalized.slice(0, atIndex) : normalized;
  return base.length > 20 ? `${base.slice(0, 20)}…` : base;
}

function formatCompactMilliseconds(value: number | null | undefined) {
  if (typeof value !== "number" || !Number.isFinite(value))
    return FALLBACK_CELL;
  if (Math.abs(value) >= 100) return `${Math.round(value)}`;
  return `${Number(value.toFixed(1))}`;
}

function resolveStatusMeta(
  tone: DashboardWorkingConversationTone,
  status: string,
): StatusMeta {
  const base = STATUS_META[tone];
  const normalized = status.trim().toLowerCase();
  if (normalized.startsWith("http_4")) {
    return {
      ...base,
      badgeVariant: "warning",
      icon: "alert-outline",
      label: formatStatusLabel(status) ?? status,
    };
  }
  if (normalized.startsWith("http_5")) {
    return {
      ...base,
      badgeVariant: "error",
      icon: "alert-circle-outline",
      label: formatStatusLabel(status) ?? status,
    };
  }
  return base;
}

function SummaryMetric({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="flex min-w-0 items-baseline gap-1.5 rounded-[0.65rem] bg-base-100/4 px-2 py-1">
      <span className="truncate text-[8px] uppercase tracking-[0.14em] text-base-content/42">
        {label}
      </span>
      <span className="truncate font-mono text-[10px] font-semibold text-base-content">
        {value}
      </span>
    </div>
  );
}

function InvocationMetaLine({
  label,
  value,
  title,
  toneClassName,
}: {
  label: string;
  value: ReactNode;
  title?: string;
  toneClassName?: string;
}) {
  return (
    <div className="grid min-w-0 grid-cols-[2.2rem_minmax(0,1fr)] items-start gap-1.5">
      <span className="pt-[1px] text-[8px] font-semibold uppercase tracking-[0.12em] text-base-content/42">
        {label}
      </span>
      <div
        className={cn(
          "min-w-0 font-mono text-[8.5px] font-semibold leading-[1.35] text-base-content/84",
          toneClassName,
        )}
        title={title}
      >
        {value}
      </div>
    </div>
  );
}

function resolveInvocationLineLabels(locale: "zh" | "en") {
  return locale === "zh"
    ? {
        account: "账号",
        usage: "用量",
        timing: "耗时",
        error: "错误",
      }
    : {
        account: "Account",
        usage: "Usage",
        timing: "Timing",
        error: "Error",
      };
}

function PlaceholderSlot() {
  const { t } = useTranslation();

  return (
    <div
      data-testid="dashboard-working-conversation-placeholder"
      className={cn(SLOT_CLASS_NAME, INVOCATION_SURFACE_CLASS_NAME)}
    >
      <div className="flex items-center justify-between gap-2">
        <div className="shrink-0 text-[9px] font-semibold uppercase tracking-[0.14em] text-base-content/55">
          {t("dashboard.workingConversations.previousInvocation")}
        </div>
        <div className="font-mono text-[9px] text-base-content/62">
          {t("dashboard.workingConversations.previousPlaceholder")}
        </div>
        <div className="flex-1" />
        <Badge
          variant="secondary"
          className="h-5 border-transparent bg-base-100/10 px-2 py-0 text-[9px] text-base-content/58 shadow-none"
        >
          {t("dashboard.workingConversations.placeholderBadge")}
        </Badge>
      </div>
      <p
        className="mt-1.5 text-[8.5px] leading-[1.35] text-base-content/56"
        title={t("dashboard.workingConversations.previousPlaceholderHint")}
      >
        {t("dashboard.workingConversations.previousPlaceholderHint")}
      </p>
      <div className="mt-2 space-y-1" aria-hidden>
        {Array.from({ length: 3 }, (_, index) => (
          <div
            key={index}
            className="working-conversation-placeholder-line h-3 rounded-[0.5rem]"
          />
        ))}
      </div>
    </div>
  );
}

function InvocationSlot({
  invocation,
  label,
  nowMs,
  locale,
  onOpenUpstreamAccount,
}: {
  invocation: DashboardWorkingConversationInvocationModel;
  label: string;
  nowMs: number;
  locale: "zh" | "en";
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
}) {
  const { t } = useTranslation();
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
  const timestampFormatter = useMemo(
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
  const timeOnlyFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(localeTag, {
        hour: "2-digit",
        minute: "2-digit",
        second: "2-digit",
        hour12: false,
      }),
    [localeTag],
  );

  const renderAccountValue = useCallback(
    (
      accountLabel: string,
      accountId: number | null,
      accountClickable: boolean,
      className?: string,
    ) => {
      if (!accountClickable || accountId == null) {
        return (
          <span className={cn("truncate", className)} title={accountLabel}>
            {accountLabel}
          </span>
        );
      }

      return (
        <button
          type="button"
          className={cn(
            "inline-flex min-w-0 cursor-pointer appearance-none items-center truncate border-0 bg-transparent p-0 text-left font-inherit text-current no-underline transition-opacity duration-200 hover:opacity-80 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary",
            className,
          )}
          onClick={() => onOpenUpstreamAccount?.(accountId, accountLabel)}
          title={accountLabel}
        >
          {accountLabel}
        </button>
      );
    },
    [onOpenUpstreamAccount],
  );

  const viewModel = useMemo(
    () =>
      buildInvocationDetailViewModel({
        record: invocation.record,
        normalizedStatus: invocation.displayStatus.trim().toLowerCase(),
        t,
        locale,
        localeTag,
        nowMs,
        numberFormatter,
        currencyFormatter,
        renderAccountValue,
      }),
    [
      currencyFormatter,
      invocation.displayStatus,
      invocation.record,
      locale,
      localeTag,
      nowMs,
      numberFormatter,
      renderAccountValue,
      t,
    ],
  );

  const statusMeta = resolveStatusMeta(
    invocation.tone,
    invocation.displayStatus,
  );
  const statusLabel = statusMeta.labelKey
    ? t(statusMeta.labelKey)
    : (statusMeta.label ?? t("table.status.unknown"));
  const occurredAtLabel =
    invocation.occurredAtEpoch != null
      ? timestampFormatter.format(new Date(invocation.occurredAtEpoch))
      : invocation.preview.occurredAt || FALLBACK_CELL;
  const occurredAtShortLabel =
    invocation.occurredAtEpoch != null
      ? timeOnlyFormatter.format(new Date(invocation.occurredAtEpoch))
      : occurredAtLabel;

  const compactAccountLabel = formatCompactAccountLabel(viewModel.accountLabel);
  const lineLabels = resolveInvocationLineLabels(locale);
  const requestReadValue = viewModel.timingPairs[0]?.value ?? FALLBACK_CELL;
  const requestParseValue = viewModel.timingPairs[1]?.value ?? FALLBACK_CELL;
  const upstreamConnectValue = viewModel.timingPairs[2]?.value ?? FALLBACK_CELL;
  const upstreamTtfbValue = viewModel.timingPairs[3]?.value ?? FALLBACK_CELL;
  const upstreamStreamValue = viewModel.timingPairs[4]?.value ?? FALLBACK_CELL;
  const responseParseValue = viewModel.timingPairs[5]?.value ?? FALLBACK_CELL;
  const persistValue = viewModel.timingPairs[6]?.value ?? FALLBACK_CELL;
  const compactCostValue = viewModel.costValue.startsWith("US$")
    ? `$${viewModel.costValue.slice(3)}`
    : viewModel.costValue;
  const compactTimingSummary = `RQ ${formatCompactMilliseconds(invocation.record.tReqReadMs)}/${formatCompactMilliseconds(invocation.record.tReqParseMs)} · UP ${formatCompactMilliseconds(invocation.record.tUpstreamConnectMs)}/${formatCompactMilliseconds(invocation.record.tUpstreamTtfbMs)}/${formatCompactMilliseconds(invocation.record.tUpstreamStreamMs)} · ED ${formatCompactMilliseconds(invocation.record.tRespParseMs)}/${formatCompactMilliseconds(invocation.record.tPersistMs)} · TT ${typeof invocation.record.tTotalMs === "number" && Number.isFinite(invocation.record.tTotalMs) ? `${formatCompactMilliseconds(invocation.record.tTotalMs)}ms` : viewModel.totalLatencyValue}`;

  return (
    <div className={cn(SLOT_CLASS_NAME, statusMeta.slotSurfaceClassName)}>
      <div className="flex min-h-5 items-start justify-between gap-3">
        <div className="flex min-w-0 items-center gap-1.5">
          <div className="shrink-0 text-[9px] font-semibold uppercase tracking-[0.12em] text-base-content/55">
            {label}
          </div>
          <div className="shrink-0 font-mono text-[9px] text-base-content/68">
            {occurredAtShortLabel}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1.5 self-start">
          <Badge
            variant={statusMeta.badgeVariant}
            className="h-4.5 gap-1 border-transparent bg-base-100/12 px-1.5 py-0 text-[8.5px] font-semibold leading-none shadow-none"
          >
            <AppIcon
              name={statusMeta.icon}
              className={cn(
                "h-2.5 w-2.5 shrink-0",
                invocation.isInFlight &&
                  "motion-safe:animate-spin motion-reduce:animate-none",
              )}
              aria-hidden
            />
            <span>{statusLabel}</span>
          </Badge>
          <div className="flex h-5 shrink-0 items-center">
            {renderEndpointSummary(
              viewModel.endpointDisplay,
              t,
              "h-4.5 rounded-full border-transparent bg-base-100/10 px-1.5 py-0 text-[8.5px] font-semibold leading-none text-base-content/72 shadow-none",
            )}
          </div>
          {viewModel.errorMessage ? (
            <span
              className="inline-flex h-4.5 w-4.5 items-center justify-center rounded-full bg-base-100/12 text-error/90"
              title={viewModel.errorMessage}
              aria-label={viewModel.errorMessage}
            >
              <AppIcon
                name="alert-circle-outline"
                className="h-2.25 w-2.25"
                aria-hidden
              />
            </span>
          ) : null}
        </div>
      </div>

      <div className="mt-1.5 space-y-1">
        <InvocationMetaLine
          label={lineLabels.account}
          value={
            <div className="flex min-w-0 items-center gap-1 text-[8.5px] leading-[1.3] text-base-content">
              <div className="min-w-0 truncate font-mono font-semibold">
                <div className="min-w-0 truncate">
                  {viewModel.accountClickable && viewModel.accountId != null ? (
                    <button
                      type="button"
                      className="inline-flex min-w-0 cursor-pointer appearance-none items-center truncate border-0 bg-transparent p-0 text-left font-mono text-[8.5px] font-semibold text-base-content no-underline transition-opacity duration-200 hover:opacity-80 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                      onClick={() =>
                        onOpenUpstreamAccount?.(
                          viewModel.accountId ?? 0,
                          viewModel.accountLabel,
                        )
                      }
                      title={viewModel.accountLabel}
                      aria-label={viewModel.accountLabel}
                    >
                      {compactAccountLabel}
                    </button>
                  ) : (
                    <span className="truncate" title={viewModel.accountLabel}>
                      {compactAccountLabel}
                    </span>
                  )}
                </div>
              </div>
              <span className="shrink-0 text-base-content/28">·</span>
              <div
                className="flex min-w-0 items-center gap-1 text-base-content/70"
                title={`${viewModel.modelValue} · ${viewModel.proxyDisplayName}`}
              >
                <span className="min-w-0 truncate font-mono">
                  {viewModel.modelValue}
                </span>
                {renderFastIndicator(viewModel.fastIndicatorState, t)}
              </div>
            </div>
          }
        />

        <InvocationMetaLine
          label={lineLabels.usage}
          title={`${t("table.column.inputTokens")}: ${viewModel.inputTokensValue} · ${t("table.column.cacheInputTokens")}: ${viewModel.cacheInputTokensValue} · ${t("table.column.outputTokens")}: ${viewModel.outputTokensValue} · ${t("table.column.totalTokens")}: ${viewModel.totalTokensValue} · ${t("table.column.costUsd")}: ${viewModel.costValue} · ${t("table.column.reasoningEffort")}: ${viewModel.reasoningEffortValue} · ${t("table.details.reasoningTokens")}: ${viewModel.reasoningTokensValue}`}
          value={
            <div className="flex min-w-0 flex-wrap items-center gap-x-1 gap-y-0.5">
              <span>IN {viewModel.inputTokensValue}</span>
              <span className="text-base-content/28">·</span>
              <span>C {viewModel.cacheInputTokensValue}</span>
              <span className="text-base-content/28">·</span>
              <span>O {viewModel.outputTokensValue}</span>
              <span className="text-base-content/28">·</span>
              <span>T {viewModel.totalTokensValue}</span>
              <span className="text-base-content/28">·</span>
              <span>{compactCostValue}</span>
              <span className="text-base-content/28">·</span>
              <span>{viewModel.reasoningEffortValue}</span>
            </div>
          }
        />

        <InvocationMetaLine
          label={lineLabels.timing}
          title={`${t("table.details.timingsTitle")}: REQ ${requestReadValue}/${requestParseValue} · UP ${upstreamConnectValue}/${upstreamTtfbValue}/${upstreamStreamValue} · END ${responseParseValue}/${persistValue} · TOT ${viewModel.totalLatencyValue}`}
          value={
            <div className="min-w-0 text-base-content/70">
              {compactTimingSummary}
            </div>
          }
        />

        {viewModel.errorMessage ? (
          <InvocationMetaLine
            label={lineLabels.error}
            value={viewModel.errorMessage}
            title={viewModel.errorMessage}
            toneClassName="text-error"
          />
        ) : null}
      </div>
    </div>
  );
}

export function DashboardWorkingConversationsSection({
  cards,
  isLoading,
  error,
  onOpenUpstreamAccount,
}: DashboardWorkingConversationsSectionProps) {
  const { t, locale } = useTranslation();
  const [nowMs, setNowMs] = useState(() => Date.now());
  const hasInFlightCards = cards.some(
    (card) =>
      card.currentInvocation.isInFlight ||
      card.previousInvocation?.isInFlight === true,
  );
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
  const timestampFormatter = useMemo(
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

  useEffect(() => {
    if (!hasInFlightCards) return;
    setNowMs(Date.now());
    const timer = window.setInterval(() => {
      setNowMs(Date.now());
    }, 1000);
    return () => window.clearInterval(timer);
  }, [hasInFlightCards]);

  useEffect(() => {
    setNowMs(Date.now());
  }, [cards]);

  if (error) {
    return (
      <section
        className="surface-panel"
        data-testid="dashboard-working-conversations"
      >
        <div className="surface-panel-body gap-4 !p-3 sm:!p-5">
          <div className="section-heading">
            <h2 className="section-title">
              {t("dashboard.section.workingConversationsTitle")}
            </h2>
          </div>
          <Alert variant="error">
            <span>{error}</span>
          </Alert>
        </div>
      </section>
    );
  }

  return (
    <section
      className="surface-panel overflow-hidden"
      data-testid="dashboard-working-conversations"
    >
      <div className="surface-panel-body gap-5 !p-3 sm:!p-5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="section-heading">
            <h2 className="section-title">
              {t("dashboard.section.workingConversationsTitle")}
            </h2>
            <p className="section-description">
              {t("dashboard.section.workingConversationsSubtitle")}
            </p>
          </div>
          <Badge
            variant="default"
            className="rounded-full px-3 py-1 font-mono text-xs font-semibold"
          >
            {t("dashboard.workingConversations.countBadge", {
              count: cards.length,
            })}
          </Badge>
        </div>

        {isLoading && cards.length === 0 ? (
          <div className="flex min-h-44 items-center justify-center gap-3 rounded-2xl border border-dashed border-base-300/75 bg-base-100/45">
            <Spinner size="sm" aria-label={t("chart.loadingDetailed")} />
            <span className="text-sm text-base-content/70">
              {t("chart.loadingDetailed")}
            </span>
          </div>
        ) : null}

        {!isLoading && cards.length === 0 ? (
          <div className="rounded-2xl border border-dashed border-base-300/75 bg-base-100/45 px-5 py-8 text-sm text-base-content/65">
            {t("dashboard.workingConversations.empty")}
          </div>
        ) : null}

        {cards.length > 0 ? (
          <div className="grid grid-cols-1 gap-4 xl:grid-cols-2 2xl:grid-cols-3">
            {cards.map((card) => {
              const currentStatusMeta = resolveStatusMeta(
                card.currentInvocation.tone,
                card.currentInvocation.displayStatus,
              );
              const currentStatusLabel = currentStatusMeta.labelKey
                ? t(currentStatusMeta.labelKey)
                : (currentStatusMeta.label ?? t("table.status.unknown"));
              const sortAnchorLabel =
                card.sortAnchorEpoch != null
                  ? timestampFormatter.format(new Date(card.sortAnchorEpoch))
                  : FALLBACK_CELL;

              return (
                <article
                  key={card.promptCacheKey}
                  data-testid="dashboard-working-conversation-card"
                  className={cn(
                    CARD_CLASS_NAME,
                    currentStatusMeta.cardToneClassName,
                  )}
                >
                  <div className="relative">
                    <div className="flex items-start justify-between gap-2">
                      <div className="min-w-0 flex-1">
                        <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[10px] text-base-content/58">
                          <span
                            className={cn(
                              "inline-flex h-2 w-2 rounded-full",
                              currentStatusMeta.beaconClassName,
                              card.currentInvocation.isInFlight &&
                                "motion-safe:animate-pulse motion-reduce:animate-none",
                            )}
                            aria-hidden
                          />
                          <span className="font-semibold uppercase tracking-[0.14em] text-base-content/74">
                            {currentStatusLabel}
                          </span>
                          <span className="font-mono">{sortAnchorLabel}</span>
                        </div>
                        <div className="mt-1.5 flex min-w-0 items-center gap-2">
                          <div className="shrink-0 font-mono text-[0.95rem] font-semibold tracking-[0.08em] text-base-content">
                            {card.conversationSequenceId}
                          </div>
                          <div
                            className="min-w-0 truncate font-mono text-[9px] text-base-content/48"
                            title={card.promptCacheKey}
                          >
                            {card.promptCacheKey}
                          </div>
                        </div>
                      </div>
                    </div>

                    <div className="mt-2">
                      <div className="grid grid-cols-3 gap-1.5">
                        <SummaryMetric
                          label={t(
                            "dashboard.workingConversations.requestCountLabel",
                          )}
                          value={numberFormatter.format(card.requestCount)}
                        />
                        <SummaryMetric
                          label={t(
                            "dashboard.workingConversations.totalTokensLabel",
                          )}
                          value={numberFormatter.format(card.totalTokens)}
                        />
                        <SummaryMetric
                          label={t(
                            "dashboard.workingConversations.totalCostLabel",
                          )}
                          value={currencyFormatter.format(card.totalCost)}
                        />
                      </div>
                    </div>

                    <div className="mt-2.5 space-y-1.5 sm:mt-3 sm:space-y-2">
                      <InvocationSlot
                        invocation={card.currentInvocation}
                        label={t(
                          "dashboard.workingConversations.currentInvocation",
                        )}
                        nowMs={nowMs}
                        locale={locale}
                        onOpenUpstreamAccount={onOpenUpstreamAccount}
                      />
                      {card.previousInvocation ? (
                        <InvocationSlot
                          invocation={card.previousInvocation}
                          label={t(
                            "dashboard.workingConversations.previousInvocation",
                          )}
                          nowMs={nowMs}
                          locale={locale}
                          onOpenUpstreamAccount={onOpenUpstreamAccount}
                        />
                      ) : (
                        <PlaceholderSlot />
                      )}
                    </div>
                  </div>
                </article>
              );
            })}
          </div>
        ) : null}
      </div>
    </section>
  );
}
