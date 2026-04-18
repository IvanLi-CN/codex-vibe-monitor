import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
} from "react";
import { useWindowVirtualizer } from "@tanstack/react-virtual";
import { useTranslation } from "../i18n";
import type { TranslationKey } from "../i18n";
import type {
  DashboardWorkingConversationCardModel,
  DashboardWorkingConversationInvocationModel,
  DashboardWorkingConversationInvocationSelection,
  DashboardWorkingConversationTone,
} from "../lib/dashboardWorkingConversations";
import {
  DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE,
  formatDashboardWorkingConversationSequenceId,
} from "../lib/dashboardWorkingConversations";
import { cn } from "../lib/utils";
import { AppIcon } from "./AppIcon";
import {
  getReasoningEffortTone,
  REASONING_EFFORT_TONE_CLASSNAMES,
} from "./invocation-table-reasoning";
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
  totalMatched?: number;
  hasMore?: boolean;
  isLoading: boolean;
  isLoadingMore?: boolean;
  error?: string | null;
  onLoadMore?: () => void;
  setRefreshTargetCount?: (count: number) => void;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  onOpenInvocation?: (
    selection: DashboardWorkingConversationInvocationSelection,
  ) => void;
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
const DASHBOARD_WORKING_CONVERSATION_ROW_GAP_PX = 16;

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

function CompactReasoningEffortBadge({ value }: { value: string }) {
  if (value === FALLBACK_CELL) {
    return (
      <span
        data-testid="dashboard-working-conversation-reasoning-effort"
        className="inline-flex shrink-0 items-center font-mono text-[7.5px] font-semibold text-base-content/48"
        title={value}
      >
        {value}
      </span>
    );
  }

  const tone = getReasoningEffortTone(value);

  return (
    <span
      data-testid="dashboard-working-conversation-reasoning-effort"
      data-reasoning-effort-tone={tone}
      className={cn(
        "inline-flex max-w-[5rem] shrink-0 items-center rounded-full border px-1.5 py-0.5 text-[7px] font-semibold leading-none tracking-[0.01em]",
        REASONING_EFFORT_TONE_CLASSNAMES[tone],
      )}
      title={value}
    >
      <span className="truncate whitespace-nowrap">{value}</span>
    </span>
  );
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
  if (normalized === "interrupted") {
    return {
      ...base,
      badgeVariant: "error",
      icon: "alert-circle-outline",
      labelKey: "table.status.interrupted",
    };
  }
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
    <div className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] items-baseline gap-1 rounded-[0.65rem] bg-base-100/4 px-1.5 py-1 sm:px-2">
      <span className="truncate text-[7px] font-semibold text-base-content/48 sm:text-[7.5px]">
        {label}
      </span>
      <span className="min-w-0 truncate text-right font-mono text-[9.5px] font-semibold text-base-content sm:text-[10px]">
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
  slotKind,
  conversationSequenceId,
  promptCacheKey,
  nowMs,
  locale,
  onOpenUpstreamAccount,
  onOpenInvocation,
}: {
  invocation: DashboardWorkingConversationInvocationModel;
  label: string;
  slotKind: "current" | "previous";
  conversationSequenceId: string;
  promptCacheKey: string;
  nowMs: number;
  locale: "zh" | "en";
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  onOpenInvocation?: (
    selection: DashboardWorkingConversationInvocationSelection,
  ) => void;
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
          onClick={(event) => {
            event.stopPropagation();
            onOpenUpstreamAccount?.(accountId, accountLabel);
          }}
          onKeyDown={(event) => {
            event.stopPropagation();
          }}
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

  const lineLabels = resolveInvocationLineLabels(locale);
  const fastIndicator = renderFastIndicator(viewModel.fastIndicatorState, t);
  const displayConversationSequenceId =
    formatDashboardWorkingConversationSequenceId(conversationSequenceId);
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
  const invocationActionLabel = `${t("dashboard.workingConversations.openInvocation")} · ${label} · ${displayConversationSequenceId} · ${invocation.record.invokeId}`;

  const handleOpenInvocation = useCallback(() => {
    onOpenInvocation?.({
      slotKind,
      conversationSequenceId,
      promptCacheKey,
      invocation,
    });
  }, [
    conversationSequenceId,
    invocation,
    onOpenInvocation,
    promptCacheKey,
    slotKind,
  ]);

  const handleSlotKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLDivElement>) => {
      if (event.target !== event.currentTarget) return;
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      handleOpenInvocation();
    },
    [handleOpenInvocation],
  );

  return (
    <div
      role="button"
      tabIndex={0}
      aria-label={invocationActionLabel}
      data-testid="dashboard-working-conversation-slot"
      data-slot-kind={slotKind}
      className={cn(
        SLOT_CLASS_NAME,
        statusMeta.slotSurfaceClassName,
        "cursor-pointer transition-colors duration-200 hover:bg-base-100/10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary",
      )}
      onClick={handleOpenInvocation}
      onKeyDown={handleSlotKeyDown}
    >
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
          {viewModel.collapsedErrorSummary ? (
            <span
              className="inline-flex h-4.5 w-4.5 items-center justify-center rounded-full bg-base-100/12 text-error/90"
              title={viewModel.collapsedErrorSummary}
              aria-label={viewModel.collapsedErrorSummary}
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
            <div className="flex min-w-0 flex-col gap-1 text-[8.5px] leading-[1.3] text-base-content">
              <div className="min-w-0 font-mono font-semibold">
                <div className="min-w-0">
                  {viewModel.accountClickable && viewModel.accountId != null ? (
                    <button
                      type="button"
                      className="inline-flex min-w-0 max-w-full cursor-pointer appearance-none items-start rounded-[0.45rem] border border-base-300/55 bg-base-100/12 px-1.5 py-0.75 text-left font-mono text-[8.5px] font-semibold text-base-content no-underline transition-colors duration-200 hover:bg-base-100/18 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                      onClick={(event) => {
                        event.stopPropagation();
                        onOpenUpstreamAccount?.(
                          viewModel.accountId ?? 0,
                          viewModel.accountLabel,
                        );
                      }}
                      onKeyDown={(event) => {
                        event.stopPropagation();
                      }}
                      title={viewModel.accountLabel}
                      aria-label={viewModel.accountLabel}
                    >
                      <span className="line-clamp-2 break-all text-left">
                        {viewModel.accountLabel}
                      </span>
                    </button>
                  ) : (
                    <span
                      className="line-clamp-2 break-all rounded-[0.45rem] border border-base-300/45 bg-base-100/8 px-1.5 py-0.75 text-left"
                      title={viewModel.accountLabel}
                    >
                      {viewModel.accountLabel}
                    </span>
                  )}
                </div>
              </div>
              <div
                className="flex min-w-0 flex-wrap items-center gap-x-1 gap-y-0.5 text-base-content/70"
                title={`${viewModel.modelValue} · ${viewModel.reasoningEffortValue} · ${viewModel.serviceTierValue} · ${viewModel.proxyDisplayName}`}
              >
                <span
                  data-testid="dashboard-working-conversation-model-name"
                  className="min-w-0 truncate font-mono"
                >
                  {viewModel.modelValue}
                </span>
                <span className="shrink-0 text-base-content/28">·</span>
                <CompactReasoningEffortBadge
                  value={viewModel.reasoningEffortValue}
                />
                {fastIndicator ? (
                  <>
                    <span className="shrink-0 text-base-content/28">·</span>
                    {fastIndicator}
                  </>
                ) : null}
              </div>
            </div>
          }
        />

        <InvocationMetaLine
          label={lineLabels.usage}
          title={`${t("table.column.inputTokens")}: ${viewModel.inputTokensValue} · ${t("table.column.cacheInputTokens")}: ${viewModel.cacheInputTokensValue} · ${t("table.column.outputTokens")}: ${viewModel.outputTokensValue} · ${t("table.column.totalTokens")}: ${viewModel.totalTokensValue} · ${t("table.column.costUsd")}: ${viewModel.costValue} · ${t("table.details.reasoningTokens")}: ${viewModel.reasoningTokensValue}`}
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

        {viewModel.collapsedErrorSummary ? (
          <InvocationMetaLine
            label={lineLabels.error}
            value={viewModel.collapsedErrorSummary}
            title={viewModel.collapsedErrorSummary}
            toneClassName="text-error"
          />
        ) : null}
      </div>
    </div>
  );
}

function resolveDashboardWorkingConversationColumnCount(width: number) {
  if (width >= 1660) return 4;
  if (width >= 1536) return 3;
  if (width >= 1280) return 2;
  return 1;
}

function splitDashboardWorkingConversationGridTracks(template: string) {
  const tracks: string[] = [];
  let currentTrack = "";
  let depth = 0;

  for (const character of template.trim()) {
    if (character === "(") {
      depth += 1;
      currentTrack += character;
      continue;
    }
    if (character === ")") {
      currentTrack += character;
      depth = Math.max(0, depth - 1);
      continue;
    }
    if (/\s/.test(character) && depth === 0) {
      if (currentTrack.trim().length > 0) {
        tracks.push(currentTrack.trim());
        currentTrack = "";
      }
      continue;
    }
    currentTrack += character;
  }

  if (currentTrack.trim().length > 0) {
    tracks.push(currentTrack.trim());
  }

  return tracks;
}

function resolveDashboardWorkingConversationCssColumnCount(
  container: HTMLDivElement | null,
) {
  if (!container || typeof window === "undefined") return null;
  const template = window
    .getComputedStyle(container)
    .gridTemplateColumns.trim();
  if (!template || template === "none" || template === "subgrid") {
    return null;
  }

  const tracks = splitDashboardWorkingConversationGridTracks(template);
  if (tracks.length === 0) return null;

  const count = tracks.reduce((total, track) => {
    const repeatMatch = track.match(/^repeat\(\s*(\d+)\s*,[\s\S]*\)$/i);
    if (repeatMatch) {
      return total + Number.parseInt(repeatMatch[1] ?? "0", 10);
    }
    return total + 1;
  }, 0);

  return count > 0 ? count : null;
}

function chunkDashboardWorkingConversationRows(
  cards: DashboardWorkingConversationCardModel[],
  columnCount: number,
) {
  if (columnCount <= 1) {
    return cards.map((card) => [card]);
  }
  const rows: DashboardWorkingConversationCardModel[][] = [];
  for (let index = 0; index < cards.length; index += columnCount) {
    rows.push(cards.slice(index, index + columnCount));
  }
  return rows;
}

interface DashboardWorkingConversationAnchorCardElement extends HTMLElement {
  __dashboardWorkingConversationAnchorKey?: string;
}

function readDashboardWorkingConversationAnchorKey(card: HTMLElement) {
  return (
    (card as DashboardWorkingConversationAnchorCardElement)
      .__dashboardWorkingConversationAnchorKey ?? ""
  )
    .trim();
}

function captureVisibleCardAnchor(
  container: HTMLDivElement,
  hasVirtualizedRowsAbove = false,
) {
  const containerRect = container.getBoundingClientRect();
  const topBoundary = Math.max(0, containerRect.top);
  const viewportBottom =
    typeof window === "undefined" ? Number.POSITIVE_INFINITY : window.innerHeight;
  const cards = Array.from(
    container.querySelectorAll<HTMLElement>(
      '[data-testid="dashboard-working-conversation-card"]',
    ),
  );
  let hasHiddenContentAbove = hasVirtualizedRowsAbove;
  for (const card of cards) {
    const rect = card.getBoundingClientRect();
    if (rect.top < topBoundary) {
      hasHiddenContentAbove = true;
    }
    if (rect.bottom <= topBoundary) {
      continue;
    }
    if (rect.top >= viewportBottom) continue;
    const anchorKey = readDashboardWorkingConversationAnchorKey(card);
    if (!anchorKey) continue;
    return {
      anchorKey,
      top: rect.top - topBoundary,
      hasHiddenContentAbove,
    };
  }
  return null;
}

export function DashboardWorkingConversationsSection({
  cards,
  totalMatched,
  hasMore = false,
  isLoading,
  isLoadingMore = false,
  error,
  onLoadMore,
  setRefreshTargetCount,
  onOpenUpstreamAccount,
  onOpenInvocation,
}: DashboardWorkingConversationsSectionProps) {
  const { t, locale } = useTranslation();
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [containerWidth, setContainerWidth] = useState(0);
  const [viewportWidth, setViewportWidth] = useState(() =>
    typeof window === "undefined" ? 0 : window.innerWidth,
  );
  const [gridElement, setGridElement] = useState<HTMLDivElement | null>(null);
  const [scrollMargin, setScrollMargin] = useState(0);
  const visibleAnchorRef = useRef<{
    anchorKey: string;
    top: number;
  } | null>(null);
  const loadMoreRequestPendingRef = useRef(false);
  const previousLoadingMoreRef = useRef(isLoadingMore);
  const previousRowsLengthRef = useRef(cards.length);
  const setGridContainerRef = useCallback((node: HTMLDivElement | null) => {
    setGridElement(node);
  }, []);
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
  const countBadgeValue = totalMatched ?? cards.length;
  const cssColumnCount =
    resolveDashboardWorkingConversationCssColumnCount(gridElement);
  const columnCount =
    cssColumnCount ??
    resolveDashboardWorkingConversationColumnCount(
      Math.max(containerWidth, viewportWidth),
    );
  const rows = useMemo(
    () => chunkDashboardWorkingConversationRows(cards, columnCount),
    [cards, columnCount],
  );
  const rowVirtualizer = useWindowVirtualizer({
    count: rows.length,
    estimateSize: () => 360,
    overscan: 3,
    scrollMargin,
  });
  const virtualRows = rowVirtualizer.getVirtualItems();
  const fallbackRowCount = Math.min(
    rows.length,
    Math.max(
      1,
      Math.ceil(
        DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE / Math.max(columnCount, 1),
      ),
    ),
  );
  const renderedRows =
    virtualRows.length > 0
      ? virtualRows
      : rows.slice(0, fallbackRowCount).map((_, index) => ({
          key: index,
          index,
          start: scrollMargin + index * 360,
        }));
  const hasVirtualizedRowsAbove =
    renderedRows.length > 0 ? renderedRows[0]!.index > 0 : false;
  const totalSize =
    virtualRows.length > 0 ? rowVirtualizer.getTotalSize() : rows.length * 360;
  const refreshTargetCount = useMemo(() => {
    if (cards.length === 0) {
      return DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE;
    }
    const deepestVisibleRowIndex = virtualRows.reduce(
      (maxIndex, row) => Math.max(maxIndex, row.index),
      0,
    );
    return Math.min(
      cards.length,
      Math.max(
        DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE,
        (deepestVisibleRowIndex + 1) * columnCount,
      ),
    );
  }, [cards.length, columnCount, virtualRows]);

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

  useEffect(() => {
    const updateLayoutMetrics = () => {
      setViewportWidth(typeof window === "undefined" ? 0 : window.innerWidth);
      setContainerWidth(gridElement?.clientWidth ?? 0);
      if (!gridElement || typeof window === "undefined") {
        setScrollMargin(0);
        return;
      }
      const nextScrollMargin =
        gridElement.getBoundingClientRect().top + window.scrollY;
      setScrollMargin((current) =>
        Math.abs(current - nextScrollMargin) > 0.5
          ? nextScrollMargin
          : current,
      );
    };

    updateLayoutMetrics();
    if (!gridElement) {
      return;
    }

    window.addEventListener("resize", updateLayoutMetrics);
    window.addEventListener("scroll", updateLayoutMetrics, { passive: true });
    if (typeof ResizeObserver === "undefined") {
      return () => {
        window.removeEventListener("resize", updateLayoutMetrics);
        window.removeEventListener("scroll", updateLayoutMetrics);
      };
    }

    const observer = new ResizeObserver(() => {
      updateLayoutMetrics();
    });
    observer.observe(gridElement);
    if (document.body) {
      observer.observe(document.body);
    }
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", updateLayoutMetrics);
      window.removeEventListener("scroll", updateLayoutMetrics);
    };
  }, [gridElement]);

  useEffect(() => {
    if (
      !hasMore ||
      previousRowsLengthRef.current !== rows.length ||
      (previousLoadingMoreRef.current && !isLoadingMore)
    ) {
      loadMoreRequestPendingRef.current = false;
    }
    previousRowsLengthRef.current = rows.length;
    previousLoadingMoreRef.current = isLoadingMore;
  }, [hasMore, isLoadingMore, rows.length]);

  useEffect(() => {
    const container = gridElement;
    if (!container || !hasMore || !onLoadMore) return;
    const maybeLoadMore = (trigger: "mount" | "scroll") => {
      if (isLoadingMore || loadMoreRequestPendingRef.current) return;
      if (typeof window === "undefined") return;
      const containerRect = container.getBoundingClientRect();
      if (containerRect.bottom <= 0) {
        return;
      }
      const sectionStartsBelowFold = containerRect.top >= window.innerHeight - 1;
      if (trigger === "mount" && sectionStartsBelowFold) {
        return;
      }
      const remaining = containerRect.bottom - window.innerHeight;
      if (remaining <= 320) {
        loadMoreRequestPendingRef.current = true;
        onLoadMore();
      }
    };
    const mountTimer = window.setTimeout(() => {
      const nextAnchor = captureVisibleCardAnchor(
        container,
        hasVirtualizedRowsAbove,
      );
      visibleAnchorRef.current = nextAnchor?.hasHiddenContentAbove
        ? nextAnchor
        : null;
      maybeLoadMore("mount");
    }, 0);
    const handleScroll = () => {
      const nextAnchor = captureVisibleCardAnchor(
        container,
        hasVirtualizedRowsAbove,
      );
      visibleAnchorRef.current = nextAnchor?.hasHiddenContentAbove
        ? nextAnchor
        : null;
      maybeLoadMore("scroll");
    };
    window.addEventListener("scroll", handleScroll, { passive: true });
    return () => {
      window.clearTimeout(mountTimer);
      window.removeEventListener("scroll", handleScroll);
    };
  }, [
    gridElement,
    hasMore,
    hasVirtualizedRowsAbove,
    isLoadingMore,
    onLoadMore,
    rows.length,
  ]);

  useEffect(() => {
    setRefreshTargetCount?.(refreshTargetCount);
  }, [refreshTargetCount, setRefreshTargetCount]);

  useLayoutEffect(() => {
    const container = gridElement;
    const pendingAnchor = visibleAnchorRef.current;
    if (container && pendingAnchor?.anchorKey) {
      const anchoredCard = Array.from(
        container.querySelectorAll<HTMLElement>(
          '[data-testid="dashboard-working-conversation-card"]',
        ),
      ).find((card) => readDashboardWorkingConversationAnchorKey(card) === pendingAnchor.anchorKey);
      if (anchoredCard) {
        const containerTopBoundary = Math.max(
          0,
          container.getBoundingClientRect().top,
        );
        const nextTop =
          anchoredCard.getBoundingClientRect().top - containerTopBoundary;
        const delta = nextTop - pendingAnchor.top;
        if (Math.abs(delta) > 0.5 && typeof window !== "undefined") {
          window.scrollBy(0, delta);
        }
      }
    }
    const nextAnchor = container
      ? captureVisibleCardAnchor(container, hasVirtualizedRowsAbove)
      : null;
    visibleAnchorRef.current = nextAnchor?.hasHiddenContentAbove
      ? nextAnchor
      : null;
  }, [cards, columnCount, gridElement, hasVirtualizedRowsAbove]);

  if (error && cards.length === 0) {
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
              count: countBadgeValue,
            })}
          </Badge>
        </div>

        {error && cards.length > 0 ? (
          <Alert variant="error">
            <span>{error}</span>
          </Alert>
        ) : null}

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
          <div
            data-testid="dashboard-working-conversations-grid"
            ref={setGridContainerRef}
            className="grid grid-cols-1 xl:grid-cols-2 2xl:grid-cols-3 desktop1660:grid-cols-4"
          >
            <div
              className="col-span-full"
              style={{ height: `${totalSize}px`, position: "relative" }}
            >
              {renderedRows.map((virtualRow) => {
                const rowCards = rows[virtualRow.index] ?? [];
                return (
                  <div
                    key={virtualRow.key}
                    ref={rowVirtualizer.measureElement}
                    data-testid="dashboard-working-conversations-row"
                    data-row-index={virtualRow.index}
                    data-index={virtualRow.index}
                    style={{
                      position: "absolute",
                      top: 0,
                      left: 0,
                      width: "100%",
                      transform: `translateY(${virtualRow.start - scrollMargin}px)`,
                      paddingBottom:
                        virtualRow.index === rows.length - 1
                          ? 0
                          : `${DASHBOARD_WORKING_CONVERSATION_ROW_GAP_PX}px`,
                    }}
                  >
                    <div
                      className="grid grid-cols-1 gap-4 xl:grid-cols-2 2xl:grid-cols-3 desktop1660:grid-cols-4"
                      style={{
                        gridTemplateColumns: `repeat(${columnCount}, minmax(0, 1fr))`,
                      }}
                    >
                      {rowCards.map((card) => {
                        const currentStatusMeta = resolveStatusMeta(
                          card.currentInvocation.tone,
                          card.currentInvocation.displayStatus,
                        );
                        const displaySequenceId =
                          formatDashboardWorkingConversationSequenceId(
                            card.conversationSequenceId,
                          );
                        const currentStatusLabel = currentStatusMeta.labelKey
                          ? t(currentStatusMeta.labelKey)
                          : (currentStatusMeta.label ??
                            t("table.status.unknown"));
                        const sortAnchorLabel =
                          card.sortAnchorEpoch != null
                            ? timestampFormatter.format(
                                new Date(card.sortAnchorEpoch),
                              )
                            : FALLBACK_CELL;

                        return (
                          <article
                            key={card.promptCacheKey}
                            ref={(node) => {
                              if (!node) return;
                              (
                                node as DashboardWorkingConversationAnchorCardElement
                              ).__dashboardWorkingConversationAnchorKey =
                                card.promptCacheKey;
                            }}
                            data-testid="dashboard-working-conversation-card"
                            data-conversation-sequence-id={displaySequenceId}
                            className={cn(
                              CARD_CLASS_NAME,
                              currentStatusMeta.cardToneClassName,
                            )}
                          >
                            <div className="relative">
                              <div className="flex min-w-0 items-center justify-between gap-3">
                                <div className="min-w-0 shrink truncate font-mono text-[0.95rem] font-semibold tracking-[0.08em] text-base-content">
                                  {displaySequenceId}
                                </div>
                                <div className="flex shrink-0 items-center justify-end gap-2 whitespace-nowrap text-[10px] text-base-content/62">
                                  <span className="font-mono">
                                    {sortAnchorLabel}
                                  </span>
                                  <span className="font-semibold uppercase tracking-[0.14em] text-base-content/76">
                                    {currentStatusLabel}
                                  </span>
                                  <span
                                    className={cn(
                                      "inline-flex h-2 w-2 rounded-full",
                                      currentStatusMeta.beaconClassName,
                                      card.currentInvocation.isInFlight &&
                                        "motion-safe:animate-pulse motion-reduce:animate-none",
                                    )}
                                    aria-hidden
                                  />
                                </div>
                              </div>

                              <div className="mt-2">
                                <div className="grid grid-cols-3 gap-1.5">
                                  <SummaryMetric
                                    label={t(
                                      "dashboard.workingConversations.requestCountLabel",
                                    )}
                                    value={numberFormatter.format(
                                      card.requestCount,
                                    )}
                                  />
                                  <SummaryMetric
                                    label={t(
                                      "dashboard.workingConversations.totalTokensLabel",
                                    )}
                                    value={numberFormatter.format(
                                      card.totalTokens,
                                    )}
                                  />
                                  <SummaryMetric
                                    label={t(
                                      "dashboard.workingConversations.totalCostLabel",
                                    )}
                                    value={currencyFormatter.format(
                                      card.totalCost,
                                    )}
                                  />
                                </div>
                              </div>

                              <div className="mt-2.5 space-y-1.5 sm:mt-3 sm:space-y-2">
                                <InvocationSlot
                                  invocation={card.currentInvocation}
                                  label={t(
                                    "dashboard.workingConversations.currentInvocation",
                                  )}
                                  slotKind="current"
                                  conversationSequenceId={
                                    card.conversationSequenceId
                                  }
                                  promptCacheKey={card.promptCacheKey}
                                  nowMs={nowMs}
                                  locale={locale}
                                  onOpenUpstreamAccount={onOpenUpstreamAccount}
                                  onOpenInvocation={onOpenInvocation}
                                />
                                {card.previousInvocation ? (
                                  <InvocationSlot
                                    invocation={card.previousInvocation}
                                    label={t(
                                      "dashboard.workingConversations.previousInvocation",
                                    )}
                                    slotKind="previous"
                                    conversationSequenceId={
                                      card.conversationSequenceId
                                    }
                                    promptCacheKey={card.promptCacheKey}
                                    nowMs={nowMs}
                                    locale={locale}
                                    onOpenUpstreamAccount={
                                      onOpenUpstreamAccount
                                    }
                                    onOpenInvocation={onOpenInvocation}
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
                  </div>
                );
              })}
            </div>
            {isLoadingMore ? (
              <div className="col-span-full flex items-center justify-center gap-2 py-4 text-sm text-base-content/65">
                <Spinner size="sm" aria-label={t("chart.loadingDetailed")} />
                <span>{t("chart.loadingDetailed")}</span>
              </div>
            ) : null}
          </div>
        ) : null}
      </div>
    </section>
  );
}
