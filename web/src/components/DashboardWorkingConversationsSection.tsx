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
import type { UpstreamAccountActivityAccount } from "../lib/api";
import {
  DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE,
  buildDashboardWorkingConversationInvocationModel,
  formatDashboardWorkingConversationSequenceId,
} from "../lib/dashboardWorkingConversations";
import { cn } from "../lib/utils";
import { AppIcon } from "./AppIcon";
import {
  getReasoningEffortTone,
  REASONING_EFFORT_TONE_CLASSNAMES,
} from "./invocation-table-reasoning";
import {
  compactUpstreamPlanLabel,
  shouldShowUpstreamPlanBadge,
  upstreamPlanBadgeRecipe,
} from "../lib/upstreamAccountBadges";
import { Alert } from "./ui/alert";
import { Badge } from "./ui/badge";
import { SegmentedControl, SegmentedControlItem } from "./ui/segmented-control";
import { Spinner } from "./ui/spinner";
import { Tooltip } from "./ui/tooltip";
import {
  FALLBACK_CELL,
  buildInvocationDetailViewModel,
  renderEndpointSummary,
  renderFastIndicator,
  renderImageIntentBadge,
  renderInvocationModelBadge,
} from "./invocation-details-shared";
import { renderInvocationTransportBadge } from "./invocation-transport-badge";
import { useDashboardUpstreamAccountActivity } from "../hooks/useDashboardUpstreamAccountActivity";
import type { DashboardActivityRangeKey } from "./dashboardActivityRange";

interface DashboardWorkingConversationsSectionProps {
  activeRange: DashboardActivityRangeKey;
  cards: DashboardWorkingConversationCardModel[];
  totalMatched?: number;
  hasMore?: boolean;
  isLoading: boolean;
  isLoadingMore?: boolean;
  error?: string | null;
  onLoadMore?: () => void;
  setRefreshTargetCount?: (count: number) => void;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  onOpenConversation?: (selection: DashboardWorkingConversationSelection) => void;
  onOpenInvocation?: (
    selection: DashboardWorkingConversationInvocationSelection,
  ) => void;
}

type DashboardWorkspaceView = "conversations" | "upstreamAccounts";

export interface DashboardWorkingConversationSelection {
  conversationSequenceId: string;
  promptCacheKey: string;
}

const ACCOUNT_CARD_CLASS_NAME =
  "flex h-full w-full max-w-full flex-col rounded-[1rem] border border-[rgba(148,163,184,0.32)] bg-base-100/72 p-4 shadow-[0_6px_12px_rgba(15,23,42,0.07)] desktop1660:min-h-[34.5rem]";

const ACCOUNT_CARD_INNER_BORDER_CLASS_NAME = "border-[rgba(148,163,184,0.22)]";
const ACCOUNT_CARD_INNER_RING_CLASS_NAME = "ring-[rgba(148,163,184,0.22)]";

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

function CompactAccountPlanBadge({ planType }: { planType: string | null }) {
  if (!shouldShowUpstreamPlanBadge(planType)) return null;
  const label = compactUpstreamPlanLabel(planType);
  if (!label) return null;
  const recipe = upstreamPlanBadgeRecipe(planType);

  return (
    <Badge
      variant={recipe?.variant ?? "secondary"}
      data-testid="dashboard-working-conversation-account-plan"
      data-plan={recipe?.dataPlan}
      className={cn(
        "h-4 shrink-0 px-1.5 py-0 text-[7.5px] font-semibold leading-none",
        recipe?.className,
      )}
      title={planType ?? undefined}
    >
      {label}
    </Badge>
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

function formatAccountPercentValue(
  value: number | null | undefined,
  localeTag: string,
) {
  if (value == null || !Number.isFinite(value)) return FALLBACK_CELL;
  return new Intl.NumberFormat(localeTag, {
    style: "percent",
    maximumFractionDigits: 1,
  }).format(value);
}

function formatAccountNumberValue(
  value: number | null | undefined,
  localeTag: string,
  maximumFractionDigits = 0,
) {
  if (value == null || !Number.isFinite(value)) return FALLBACK_CELL;
  return new Intl.NumberFormat(localeTag, {
    maximumFractionDigits,
  }).format(value);
}

function formatAccountCurrencyValue(
  value: number | null | undefined,
  localeTag: string,
  maximumFractionDigits = 2,
) {
  if (value == null || !Number.isFinite(value)) return FALLBACK_CELL;
  return new Intl.NumberFormat(localeTag, {
    style: "currency",
    currency: "USD",
    maximumFractionDigits,
  }).format(value);
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

type AccountMetricTone =
  | "neutral"
  | "primary"
  | "secondary"
  | "success"
  | "warning"
  | "error"
  | "info";

type AccountActivityStatus = {
  kind: "busy" | "attention" | "steady";
  badgeVariant: "info" | "warning" | "success";
  label: string;
  summary: string;
};

const ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES: Record<AccountMetricTone, string> = {
  neutral: "text-base-content",
  primary: "text-primary",
  secondary: "text-secondary",
  success: "text-success",
  warning: "text-warning-content",
  error: "text-error",
  info: "text-info",
};

const ACCOUNT_METRIC_DOT_TONE_CLASSNAMES: Record<AccountMetricTone, string> = {
  neutral: "bg-base-content/38",
  primary: "bg-primary/90",
  secondary: "bg-secondary/90",
  success: "bg-success/90",
  warning: "bg-warning/90",
  error: "bg-error/90",
  info: "bg-info/90",
};

function AccountStatusBadge({
  label,
  variant,
  description,
}: {
  label: string;
  variant: AccountActivityStatus["badgeVariant"];
  description?: string;
}) {
  const surfaceClassName = {
    info: "border-[rgba(148,163,184,0.26)] bg-base-100/86",
    warning: "border-[rgba(148,163,184,0.26)] bg-base-100/86",
    success: "border-[rgba(148,163,184,0.26)] bg-base-100/86",
  }[variant];
  const dotClassName = {
    info: "bg-info/90",
    warning: "bg-warning/90",
    success: "bg-success/90",
  }[variant];

  return (
    <Badge
      variant="secondary"
      data-testid="dashboard-upstream-account-status"
      title={description}
      aria-label={description ? `${label} · ${description}` : label}
      data-motion-surface
      className={cn(
        "min-h-6 gap-2 border px-2.5 py-0.5 text-[11px] font-semibold text-base-content",
        surfaceClassName,
      )}
    >
      <span className={cn("h-1.5 w-1.5 rounded-full", dotClassName)} aria-hidden="true" />
      {label}
    </Badge>
  );
}

function AccountHeroMetric({
  label,
  value,
  tone,
  hint,
  children,
}: {
  label: string;
  value: string;
  tone: "neutral" | "primary" | "secondary" | "success" | "warning" | "info";
  hint?: string;
  children?: ReactNode;
}) {
  const toneSurfaceClassName = {
    neutral: cn("bg-base-100/72 ring-1 ring-inset", ACCOUNT_CARD_INNER_RING_CLASS_NAME),
    primary: cn("bg-base-100/72 ring-1 ring-inset", ACCOUNT_CARD_INNER_RING_CLASS_NAME),
    secondary: cn("bg-base-100/72 ring-1 ring-inset", ACCOUNT_CARD_INNER_RING_CLASS_NAME),
    success: cn("bg-base-100/72 ring-1 ring-inset", ACCOUNT_CARD_INNER_RING_CLASS_NAME),
    warning: cn("bg-base-100/72 ring-1 ring-inset", ACCOUNT_CARD_INNER_RING_CLASS_NAME),
    info: cn("bg-base-100/72 ring-1 ring-inset", ACCOUNT_CARD_INNER_RING_CLASS_NAME),
  }[tone];
  const valueClassName =
    value === FALLBACK_CELL
      ? "text-base-content/55"
      : ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[
          tone === "neutral" ? "neutral" : tone
        ];

  return (
    <div
      data-motion-surface
      className={cn("rounded-[0.85rem] px-3 py-2.5", toneSurfaceClassName)}
    >
      <div className="text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/54">
        {label}
      </div>
      <div
        className={cn(
          "mt-1 font-mono text-[1.08rem] font-semibold leading-none",
          valueClassName,
        )}
      >
        {value}
      </div>
      {hint ? (
        <div className="mt-1 text-[11px] leading-4 text-base-content/58">{hint}</div>
      ) : null}
      {children ? <div className="mt-1.5">{children}</div> : null}
    </div>
  );
}

function AccountSegmentList({
  segments,
  className,
  testId,
  showLabel = false,
}: {
  segments: Array<{
    label: string;
    value: ReactNode;
    tone: AccountMetricTone;
  }>;
  className?: string;
  testId?: string;
  showLabel?: boolean;
}) {
  return (
    <div
      data-testid={testId}
      aria-label={segments
        .map((segment) => `${segment.label} ${String(segment.value)}`)
        .join(" · ")}
      className={cn(
        "flex flex-wrap items-center gap-x-3 gap-y-1.5",
        showLabel && "gap-x-4",
        className,
      )}
    >
      {segments.map((segment) => (
        <Tooltip
          key={`${segment.label}-${String(segment.value)}`}
          content={
            <span className="font-medium">
              {segment.label}
              <span className="font-mono font-semibold"> {String(segment.value)}</span>
            </span>
          }
          clickToOpen
          className="rounded-md"
          triggerProps={{
            tabIndex: 0,
            "aria-label": `${segment.label} ${String(segment.value)}`,
          }}
        >
          <span
            data-testid="dashboard-upstream-account-segment"
            data-motion-surface
            className={cn(
              "inline-flex items-center whitespace-nowrap rounded-md px-1 py-0.5 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary",
              showLabel ? "gap-1.5" : "gap-1.5",
            )}
          >
            <span
              className={cn(
                "h-1.5 w-1.5 rounded-full",
                ACCOUNT_METRIC_DOT_TONE_CLASSNAMES[segment.tone],
              )}
              aria-hidden="true"
            />
            {showLabel ? (
              <span className="text-[11px] font-semibold leading-none text-base-content/72">
                {segment.label}
              </span>
            ) : null}
            <span
              className={cn(
                "font-mono text-[12px] font-semibold leading-none",
                ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[segment.tone],
              )}
            >
              {segment.value}
            </span>
          </span>
        </Tooltip>
      ))}
    </div>
  );
}

function AccountRecentInvocationRow({
  invocation,
  locale,
  nowMs,
  onOpenUpstreamAccount,
  onOpenInvocation,
}: {
  invocation: DashboardWorkingConversationInvocationModel;
  locale: "zh" | "en";
  nowMs: number;
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
          data-motion-surface
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
      ? timestampFormatter.format(new Date(invocation.occurredAtEpoch))
      : occurredAtLabel;
  const compactCostValue = viewModel.costValue.startsWith("US$")
    ? `$${viewModel.costValue.slice(3)}`
    : viewModel.costValue;
  const compactTimingSummary = `RQ ${formatCompactMilliseconds(invocation.record.tReqReadMs)}/${formatCompactMilliseconds(invocation.record.tReqParseMs)} · UP ${formatCompactMilliseconds(invocation.record.tUpstreamConnectMs)}/${formatCompactMilliseconds(invocation.record.tUpstreamTtfbMs)}/${formatCompactMilliseconds(invocation.record.tUpstreamStreamMs)} · ED ${formatCompactMilliseconds(invocation.record.tRespParseMs)}/${formatCompactMilliseconds(invocation.record.tPersistMs)} · TT ${typeof invocation.record.tTotalMs === "number" && Number.isFinite(invocation.record.tTotalMs) ? `${formatCompactMilliseconds(invocation.record.tTotalMs)}ms` : viewModel.totalLatencyValue}`;
  const invocationActionLabel = `${t("dashboard.workingConversations.openInvocation")} · ${invocation.record.invokeId}`;
  const fastIndicator = renderFastIndicator(viewModel.fastIndicatorState, t);

  const handleOpenInvocation = useCallback(() => {
    onOpenInvocation?.({
      slotKind: "current",
      conversationSequenceId: invocation.record.invokeId,
      promptCacheKey: invocation.preview.invokeId,
      invocation,
    });
  }, [invocation, onOpenInvocation]);

  const handleRowKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLButtonElement>) => {
      if (event.target !== event.currentTarget) return;
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      handleOpenInvocation();
    },
    [handleOpenInvocation],
  );

  return (
    <button
      type="button"
      aria-label={invocationActionLabel}
      data-testid="dashboard-upstream-account-recent-row"
      data-motion-surface
      className={cn(
        "w-full rounded-[0.85rem] border bg-base-100/58 px-3.5 py-2.5 text-left transition-colors duration-200 hover:bg-base-100/72 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary",
        ACCOUNT_CARD_INNER_BORDER_CLASS_NAME,
      )}
      onClick={handleOpenInvocation}
      onKeyDown={handleRowKeyDown}
    >
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="font-mono text-[12px] font-semibold text-base-content/88">
              {invocation.record.invokeId}
            </span>
            <Badge
              variant={statusMeta.badgeVariant}
              className="min-h-5 gap-1 border-transparent bg-base-200/82 px-2 py-0.5 text-[9px] font-semibold leading-none shadow-none"
            >
              <AppIcon
                name={statusMeta.icon}
                className={cn(
                  "h-2.25 w-2.25 shrink-0",
                  invocation.isInFlight &&
                    "motion-safe:animate-spin motion-reduce:animate-none",
                )}
                aria-hidden
              />
              <span>{statusLabel}</span>
            </Badge>
            {renderInvocationTransportBadge(
              invocation.record,
              "min-h-5 border-[rgba(148,163,184,0.24)] bg-primary/8 px-2 py-0.5 text-[9px]",
            )}
            {fastIndicator}
          </div>
          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-[10px] leading-[1.45] text-base-content/70">
            <span>{occurredAtShortLabel}</span>
            <span className="text-base-content/28">·</span>
            <span className="truncate">{viewModel.accountLabel}</span>
            <span className="text-base-content/28">·</span>
            <span className="min-w-0">
              {renderInvocationModelBadge(viewModel.modelValue, {
                t,
                hasMismatch: viewModel.modelHasMismatch,
                className: "max-w-full",
                textClassName: "font-mono",
                iconClassName: "h-3 w-3",
                testId: "dashboard-upstream-account-recent-model",
              })}
            </span>
            {viewModel.reasoningEffortValue !== FALLBACK_CELL ? (
              <>
                <span className="text-base-content/28">·</span>
                <CompactReasoningEffortBadge value={viewModel.reasoningEffortValue} />
              </>
            ) : null}
            {renderEndpointSummary(
              viewModel.endpointDisplay,
              t,
              "min-h-5 border-transparent bg-base-200/82 px-2 py-0.5 text-[9px] font-semibold leading-none text-base-content/76 shadow-none",
            ) ? (
              <>
                <span className="text-base-content/28">·</span>
                {renderEndpointSummary(
                  viewModel.endpointDisplay,
                  t,
                  "min-h-5 border-transparent bg-base-200/82 px-2 py-0.5 text-[9px] font-semibold leading-none text-base-content/76 shadow-none",
                )}
              </>
            ) : null}
          </div>
        </div>
        <div className="text-right">
          <div className="font-mono text-[11px] font-semibold text-base-content/88">
            {viewModel.totalTokensValue}
          </div>
          <div className="text-[10px] text-base-content/62">{compactCostValue}</div>
        </div>
      </div>
      <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 font-mono text-[10px] leading-[1.45] text-base-content/72">
        <span>IN {viewModel.inputTokensValue}</span>
        <span className="text-base-content/28">·</span>
        <span>C {viewModel.cacheInputTokensValue}</span>
        <span className="text-base-content/28">·</span>
        <span>O {viewModel.outputTokensValue}</span>
        <span className="text-base-content/28">·</span>
        <span>T {viewModel.totalTokensValue}</span>
        <span className="text-base-content/28">·</span>
        <span>{compactTimingSummary}</span>
      </div>
      {viewModel.collapsedErrorSummary ? (
        <div
          className="mt-1 truncate text-[10px] text-error"
          title={viewModel.collapsedErrorSummary}
        >
          {viewModel.collapsedErrorSummary}
        </div>
      ) : null}
    </button>
  );
}

function summarizeRecentInvocations(
  invocations: DashboardWorkingConversationInvocationModel[],
) {
  return invocations.reduce(
    (summary, invocation) => {
      if (invocation.isInFlight) {
        summary.inFlightCount += 1;
        return summary;
      }
      if (invocation.tone === "warning" || invocation.tone === "error") {
        summary.nonSuccessCount += 1;
        if (invocation.tone === "error") {
          summary.failureCount += 1;
        }
        return summary;
      }
      if (invocation.tone === "success") {
        summary.successCount += 1;
      }
      return summary;
    },
    {
      totalCount: invocations.length,
      inFlightCount: 0,
      nonSuccessCount: 0,
      failureCount: 0,
      successCount: 0,
    },
  );
}

function resolveAccountActivityStatus({
  account,
  locale,
  localeTag,
}: {
  account: UpstreamAccountActivityAccount;
  locale: "zh" | "en";
  localeTag: string;
}): AccountActivityStatus {
  const inProgress = account.inProgressInvocationCount ?? 0;
  const retry = account.retryInvocationCount ?? 0;
  const failure = account.failureCount ?? 0;
  const nonSuccess = account.nonSuccessCount ?? 0;

  if (inProgress > 0) {
    return {
      kind: "busy",
      badgeVariant: "info",
      label: locale === "zh" ? "繁忙" : "Busy",
      summary:
        retry > 0
          ? locale === "zh"
            ? `当前有 ${formatAccountNumberValue(inProgress, localeTag, 0)} 个进行中调用，其中 ${formatAccountNumberValue(retry, localeTag, 0)} 个处于重试。`
            : `${formatAccountNumberValue(inProgress, localeTag, 0)} in-flight invocations, including ${formatAccountNumberValue(retry, localeTag, 0)} retrying.`
          : locale === "zh"
            ? `当前有 ${formatAccountNumberValue(inProgress, localeTag, 0)} 个进行中调用，账号仍在承压。`
            : `${formatAccountNumberValue(inProgress, localeTag, 0)} in-flight invocations are still loading this account.`,
    };
  }

  if (retry > 0 || failure > 0 || nonSuccess > 0) {
    return {
      kind: "attention",
      badgeVariant: "warning",
      label: locale === "zh" ? "关注" : "Attention",
      summary:
        locale === "zh"
          ? `当前无进行中调用，但范围内有 ${formatAccountNumberValue(failure, localeTag, 0)} 次失败、${formatAccountNumberValue(retry, localeTag, 0)} 次重试。`
          : `No live invocations right now, but this range still contains ${formatAccountNumberValue(failure, localeTag, 0)} failures and ${formatAccountNumberValue(retry, localeTag, 0)} retries.`,
    };
  }

  return {
    kind: "steady",
    badgeVariant: "success",
    label: locale === "zh" ? "稳定" : "Steady",
    summary:
      locale === "zh"
        ? "当前无进行中或重试调用，范围内活动已回到稳定状态。"
        : "No in-flight or retrying invocations are active in this range.",
  };
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
          {renderInvocationTransportBadge(
            invocation.record,
            "h-4.5 border-primary/45 bg-primary/10 px-1.5 text-[8.5px]",
          )}
          <div className="flex h-5 shrink-0 items-center">
            <div className="flex items-center gap-1">
              {renderEndpointSummary(
                viewModel.endpointDisplay,
                t,
                "h-4.5 rounded-full border-transparent bg-base-100/10 px-1.5 py-0 text-[8.5px] font-semibold leading-none text-base-content/72 shadow-none",
              )}
              {renderImageIntentBadge(
                viewModel.imageIntentDisplay,
                t,
                "h-4.5 border-transparent bg-base-100/10 px-1.5 py-0 text-[8.5px] font-semibold leading-none text-base-content/72 shadow-none",
              )}
            </div>
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
            <div
              data-testid="dashboard-working-conversation-account-line"
              className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5 text-[8.5px] leading-[1.3] text-base-content sm:flex-nowrap"
            >
              <div className="flex min-w-[7rem] max-w-full flex-1 items-baseline gap-1.5 font-mono font-semibold">
                {viewModel.accountClickable && viewModel.accountId != null ? (
                  <button
                    type="button"
                    data-testid="dashboard-working-conversation-account-chip"
                    className="inline-flex min-w-0 max-w-full cursor-pointer appearance-none items-baseline border-0 bg-transparent p-0 text-left font-mono text-[8.5px] font-semibold text-base-content no-underline transition-colors duration-200 hover:text-primary focus-visible:rounded-[0.2rem] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
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
                    <span
                      data-testid="dashboard-working-conversation-account-name"
                      className="block min-w-0 truncate whitespace-nowrap text-left"
                    >
                      {viewModel.accountLabel}
                    </span>
                  </button>
                ) : (
                  <span
                    data-testid="dashboard-working-conversation-account-chip"
                    className="inline-flex min-w-0 max-w-full items-baseline"
                    title={viewModel.accountLabel}
                  >
                    <span
                      data-testid="dashboard-working-conversation-account-name"
                      className="block min-w-0 truncate whitespace-nowrap text-left"
                    >
                      {viewModel.accountLabel}
                    </span>
                  </span>
                )}
                <CompactAccountPlanBadge planType={viewModel.accountPlanType} />
              </div>
              <div
                data-testid="dashboard-working-conversation-account-meta"
                className="flex min-w-0 shrink-0 flex-wrap items-center gap-x-1 gap-y-0.5 text-base-content/70 sm:flex-nowrap"
                title={`${viewModel.modelValue} · ${viewModel.reasoningEffortValue} · ${viewModel.serviceTierValue} · ${viewModel.proxyDisplayName}`}
              >
                <span
                  data-testid="dashboard-working-conversation-model-name"
                  className="min-w-0"
                >
                  {renderInvocationModelBadge(viewModel.modelValue, {
                    t,
                    hasMismatch: viewModel.modelHasMismatch,
                    className: "max-w-full",
                    textClassName: "font-mono",
                    iconClassName: "h-3 w-3",
                    testId: "dashboard-working-conversation-model",
                  })}
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

function resolveDashboardUpstreamAccountColumnCount(width: number) {
  return width >= 1660 ? 2 : 1;
}

function chunkDashboardUpstreamAccountRows(
  accounts: UpstreamAccountActivityAccount[],
  columnCount: number,
) {
  if (columnCount <= 1) {
    return accounts.map((account) => [account]);
  }
  const rows: UpstreamAccountActivityAccount[][] = [];
  for (let index = 0; index < accounts.length; index += columnCount) {
    rows.push(accounts.slice(index, index + columnCount));
  }
  return rows;
}

function DashboardUpstreamAccountActivityCard({
  account,
  locale,
  localeTag,
  nowMs,
  onOpenUpstreamAccount,
  onOpenInvocation,
}: {
  account: UpstreamAccountActivityAccount;
  locale: "zh" | "en";
  localeTag: string;
  nowMs: number;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  onOpenInvocation?: (
    selection: DashboardWorkingConversationInvocationSelection,
  ) => void;
}) {
  const { t } = useTranslation();
  const recentInvocations = useMemo(
    () =>
      account.recentInvocations.map((preview: DashboardWorkingConversationInvocationModel["preview"]) =>
        buildDashboardWorkingConversationInvocationModel(preview),
      ),
    [account.recentInvocations],
  );
  const accountStatus = useMemo(
    () => resolveAccountActivityStatus({ account, locale, localeTag }),
    [account, locale, localeTag],
  );
  const recentSummary = useMemo(
    () => summarizeRecentInvocations(recentInvocations),
    [recentInvocations],
  );
  const requestSummarySegments = useMemo(
    () => [
      {
        label: locale === "zh" ? "成功" : "Success",
        value: formatAccountNumberValue(account.successCount, localeTag, 0),
        tone: "success" as const,
      },
      {
        label: locale === "zh" ? "失败" : "Failure",
        value: formatAccountNumberValue(account.failureCount, localeTag, 0),
        tone: "error" as const,
      },
      {
        label: locale === "zh" ? "非成功" : "Non-success",
        value: formatAccountNumberValue(account.nonSuccessCount, localeTag, 0),
        tone: "warning" as const,
      },
    ],
    [account.failureCount, account.nonSuccessCount, account.successCount, locale, localeTag],
  );
  const tokenSummarySegments = useMemo(
    () => [
      {
        label: locale === "zh" ? "成功" : "Success",
        value: formatAccountNumberValue(account.successTokens, localeTag, 0),
        tone: "primary" as const,
      },
      {
        label: locale === "zh" ? "非成功" : "Non-success",
        value: formatAccountNumberValue(account.nonSuccessTokens, localeTag, 0),
        tone: "warning" as const,
      },
    ],
    [account.nonSuccessTokens, account.successTokens, locale, localeTag],
  );
  const recentBridgeSegments = useMemo(() => {
    const segments = [];
    if (recentSummary.inFlightCount > 0) {
      segments.push({
        label: locale === "zh" ? "进行中" : "In flight",
        value: formatAccountNumberValue(recentSummary.inFlightCount, localeTag, 0),
        tone: "info" as const,
      });
    }
    if (recentSummary.failureCount > 0) {
      segments.push({
        label: locale === "zh" ? "失败" : "Failure",
        value: formatAccountNumberValue(recentSummary.failureCount, localeTag, 0),
        tone: "error" as const,
      });
    }
    if (recentSummary.nonSuccessCount > recentSummary.failureCount) {
      segments.push({
        label: locale === "zh" ? "非成功" : "Non-success",
        value: formatAccountNumberValue(
          recentSummary.nonSuccessCount - recentSummary.failureCount,
          localeTag,
          0,
        ),
        tone: "warning" as const,
      });
    }
    if (recentSummary.successCount > 0) {
      segments.push({
        label: locale === "zh" ? "成功" : "Success",
        value: formatAccountNumberValue(recentSummary.successCount, localeTag, 0),
        tone: "success" as const,
      });
    }
    return segments;
  }, [locale, localeTag, recentSummary.failureCount, recentSummary.inFlightCount, recentSummary.nonSuccessCount, recentSummary.successCount]);
  const firstByteValue =
    account.firstByteAvgMs == null
      ? FALLBACK_CELL
      : `${formatAccountNumberValue(account.firstByteAvgMs, localeTag, 1)} ms`;
  const totalRequestValue = formatAccountNumberValue(account.requestCount, localeTag, 0);

  return (
    <article
      data-testid="dashboard-upstream-account-card"
      data-account-status={accountStatus.kind}
      className={ACCOUNT_CARD_CLASS_NAME}
    >
      <div
        data-testid="dashboard-upstream-account-header-row"
        className="flex flex-wrap items-start justify-between gap-3"
      >
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <button
              type="button"
              data-motion-surface
              className="inline-flex min-h-11 min-w-0 max-w-full cursor-pointer appearance-none items-center border-0 bg-transparent py-1 text-left text-[1rem] font-semibold text-base-content transition-opacity duration-200 hover:opacity-80 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
              onClick={() =>
                onOpenUpstreamAccount?.(account.upstreamAccountId, account.displayName)
              }
            >
              <span className="truncate">{account.displayName}</span>
            </button>
            <AccountStatusBadge
              label={accountStatus.label}
              variant={accountStatus.badgeVariant}
              description={accountStatus.summary}
            />
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-2 text-[13px] leading-[1.4] text-base-content/68">
            <span>{t("dashboard.upstreamAccounts.channelName", { name: account.displayName })}</span>
            {account.groupName ? <span>{account.groupName}</span> : null}
            {shouldShowUpstreamPlanBadge(account.planType) ? (
              <Badge
                variant={upstreamPlanBadgeRecipe(account.planType)?.variant ?? "secondary"}
                className={cn(
                  "h-5 px-2 py-0 text-[10px] font-semibold",
                  upstreamPlanBadgeRecipe(account.planType)?.className,
                )}
              >
                {compactUpstreamPlanLabel(account.planType)}
              </Badge>
            ) : null}
          </div>
        </div>
        <div className="shrink-0 rounded-full bg-base-200/78 px-3 py-1 font-mono text-xs font-semibold text-base-content/72">
          #{account.upstreamAccountId}
        </div>
      </div>

      <div className="mt-4 flex flex-col gap-2.5">
        <div className="grid gap-2.5 sm:grid-cols-2 xl:grid-cols-4">
          <AccountHeroMetric
            label="TPM"
            value={formatAccountNumberValue(account.tokensPerMinute, localeTag, 0)}
            tone="primary"
          />
          <AccountHeroMetric
            label={t("dashboard.today.spendRate")}
            value={formatAccountCurrencyValue(account.spendRate, localeTag, 2)}
            tone="warning"
          />
          <AccountHeroMetric
            label={locale === "zh" ? "进行中调用" : "In-flight"}
            value={formatAccountNumberValue(
              account.inProgressInvocationCount ?? null,
              localeTag,
              0,
            )}
            tone="info"
          />
          <AccountHeroMetric
            label={locale === "zh" ? "重试调用" : "Retrying"}
            value={formatAccountNumberValue(
              account.retryInvocationCount ?? null,
              localeTag,
              0,
            )}
            tone="warning"
          />
          <AccountHeroMetric
            label={locale === "zh" ? "请求数" : "Requests"}
            value={totalRequestValue}
            tone="neutral"
          >
            <AccountSegmentList
              segments={requestSummarySegments}
              testId="dashboard-upstream-account-request-breakdown"
            />
          </AccountHeroMetric>
          <AccountHeroMetric
            label={t("dashboard.today.firstResponseTime")}
            value={firstByteValue}
            tone={firstByteValue === FALLBACK_CELL ? "neutral" : "secondary"}
          />
          <AccountHeroMetric
            label="Token"
            value={formatAccountNumberValue(account.totalTokens, localeTag, 0)}
            tone="success"
          >
            <AccountSegmentList
              segments={tokenSummarySegments}
              testId="dashboard-upstream-account-token-breakdown"
            />
          </AccountHeroMetric>
          <AccountHeroMetric
            label={locale === "zh" ? "缓存命中率" : "Cache hit"}
            value={formatAccountPercentValue(account.cacheHitRate, localeTag)}
            tone="secondary"
          />
        </div>
      </div>

      <div
        className={cn(
          "mt-3.5 flex flex-1 flex-col border-t pt-2.5",
          ACCOUNT_CARD_INNER_BORDER_CLASS_NAME,
        )}
      >
        <div className="mb-2 flex flex-wrap items-center justify-between gap-x-3 gap-y-1.5">
          <div className="text-xs font-semibold leading-5 tracking-[0.06em] text-base-content/62">
            {t("dashboard.upstreamAccounts.recentInvocations")}
          </div>
          {recentBridgeSegments.length > 0 ? (
            <AccountSegmentList
              segments={recentBridgeSegments}
              testId="dashboard-upstream-account-recent-breakdown"
              showLabel
              className="justify-end"
            />
          ) : null}
        </div>
        <div className="grid flex-1 auto-rows-fr gap-1.5">
          {recentInvocations.map((invocation: DashboardWorkingConversationInvocationModel) => (
            <AccountRecentInvocationRow
              key={invocation.record.id}
              invocation={invocation}
              locale={locale}
              nowMs={nowMs}
              onOpenUpstreamAccount={onOpenUpstreamAccount}
              onOpenInvocation={onOpenInvocation}
            />
          ))}
        </div>
      </div>
    </article>
  );
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
  activeRange,
  cards,
  totalMatched,
  hasMore = false,
  isLoading,
  isLoadingMore = false,
  error,
  onLoadMore,
  setRefreshTargetCount,
  onOpenUpstreamAccount,
  onOpenConversation,
  onOpenInvocation,
}: DashboardWorkingConversationsSectionProps) {
  const { t, locale } = useTranslation();
  const [activeView, setActiveView] =
    useState<DashboardWorkspaceView>("conversations");
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
  const upstreamAccountsDisabled = activeRange === "usage";
  const upstreamAccountActivityEnabled =
    !upstreamAccountsDisabled && activeView === "upstreamAccounts";
  const {
    data: upstreamAccountActivity,
    isLoading: upstreamAccountActivityLoading,
    error: upstreamAccountActivityError,
  } = useDashboardUpstreamAccountActivity(
    activeRange,
    upstreamAccountActivityEnabled,
  );
  const upstreamAccounts = useMemo(
    () => upstreamAccountActivity?.accounts ?? [],
    [upstreamAccountActivity],
  );
  useEffect(() => {
    if (upstreamAccountsDisabled && activeView === "upstreamAccounts") {
      setActiveView("conversations");
    }
  }, [activeView, upstreamAccountsDisabled]);
  const countBadgeValue = totalMatched ?? cards.length;
  const accountCountBadgeValue = upstreamAccounts.length;
  const countBadgeLabel =
    activeView === "conversations"
      ? t("dashboard.workingConversations.countBadge", {
          count: countBadgeValue,
        })
      : t("dashboard.upstreamAccounts.countBadge", {
          count: accountCountBadgeValue,
        });
  const upstreamAccountRows = useMemo(
    () =>
      chunkDashboardUpstreamAccountRows(
        upstreamAccounts,
        resolveDashboardUpstreamAccountColumnCount(
          Math.max(containerWidth, viewportWidth),
        ),
      ),
    [containerWidth, upstreamAccounts, viewportWidth],
  );
  const sectionSubtitle =
    activeView === "upstreamAccounts"
      ? t("dashboard.upstreamAccounts.subtitle")
      : t("dashboard.section.workingConversationsSubtitle");
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
              {sectionSubtitle}
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <SegmentedControl
              size="compact"
              role="tablist"
              aria-label="Dashboard workspace view"
            >
              <SegmentedControlItem
                active={activeView === "conversations"}
                role="tab"
                aria-selected={activeView === "conversations"}
                className="h-11 px-3.5 text-[0.95rem]"
                onClick={() => setActiveView("conversations")}
              >
                对话
              </SegmentedControlItem>
              <SegmentedControlItem
                active={activeView === "upstreamAccounts"}
                role="tab"
                aria-selected={activeView === "upstreamAccounts"}
                disabled={upstreamAccountsDisabled}
                className="h-11 px-3.5 text-[0.95rem]"
                onClick={() => setActiveView("upstreamAccounts")}
              >
                上游账号
              </SegmentedControlItem>
            </SegmentedControl>
            <Badge
              variant="default"
              className="rounded-full px-3 py-1 font-mono text-xs font-semibold"
            >
              {countBadgeLabel}
            </Badge>
          </div>
        </div>

        {error && cards.length > 0 ? (
          <Alert variant="error">
            <span>{error}</span>
          </Alert>
        ) : null}

        {activeView === "upstreamAccounts" ? (
          <>
            {upstreamAccountActivityError ? (
              <Alert variant="error">
                <span>{upstreamAccountActivityError}</span>
              </Alert>
            ) : null}
            {upstreamAccountActivityLoading && upstreamAccounts.length === 0 ? (
              <div className="flex min-h-44 items-center justify-center gap-3 rounded-2xl border border-dashed border-base-300/75 bg-base-100/45">
                <Spinner size="sm" aria-label={t("chart.loadingDetailed")} />
                <span className="text-sm text-base-content/70">
                  {t("chart.loadingDetailed")}
                </span>
              </div>
            ) : null}
            {!upstreamAccountActivityLoading && upstreamAccounts.length === 0 ? (
              <div className="rounded-2xl border border-dashed border-base-300/75 bg-base-100/45 px-5 py-8 text-sm text-base-content/65">
                {t("dashboard.upstreamAccounts.empty")}
              </div>
            ) : null}
            {upstreamAccounts.length > 0 ? (
              <div
                data-testid="dashboard-upstream-account-grid"
                className="grid grid-cols-1 gap-4 desktop1660:grid-cols-2"
              >
                {upstreamAccountRows.flat().map((account) => (
                  <DashboardUpstreamAccountActivityCard
                    key={account.upstreamAccountId}
                    account={account}
                    locale={locale}
                    localeTag={localeTag}
                    nowMs={nowMs}
                    onOpenUpstreamAccount={onOpenUpstreamAccount}
                    onOpenInvocation={onOpenInvocation}
                  />
                ))}
              </div>
            ) : null}
          </>
        ) : null}

        {activeView === "conversations" && isLoading && cards.length === 0 ? (
          <div className="flex min-h-44 items-center justify-center gap-3 rounded-2xl border border-dashed border-base-300/75 bg-base-100/45">
            <Spinner size="sm" aria-label={t("chart.loadingDetailed")} />
            <span className="text-sm text-base-content/70">
              {t("chart.loadingDetailed")}
            </span>
          </div>
        ) : null}

        {activeView === "conversations" && !isLoading && cards.length === 0 ? (
          <div className="rounded-2xl border border-dashed border-base-300/75 bg-base-100/45 px-5 py-8 text-sm text-base-content/65">
            {t("dashboard.workingConversations.empty")}
          </div>
        ) : null}

        {activeView === "conversations" && cards.length > 0 ? (
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
                        const sequenceConversationActionLabel = `${t("dashboard.workingConversations.openConversation")} · ${displaySequenceId} · ${card.promptCacheKey}`;

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
                                {onOpenConversation ? (
                                  <button
                                    type="button"
                                    data-testid="dashboard-working-conversation-sequence-button"
                                    className="inline-flex min-w-0 shrink cursor-pointer appearance-none items-center border-0 bg-transparent p-0 text-left font-mono text-[0.95rem] font-semibold tracking-[0.08em] text-base-content transition-opacity duration-200 hover:opacity-80 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                                    aria-label={sequenceConversationActionLabel}
                                    title={sequenceConversationActionLabel}
                                    onClick={() => {
                                      onOpenConversation({
                                        conversationSequenceId:
                                          card.conversationSequenceId,
                                        promptCacheKey: card.promptCacheKey,
                                      });
                                    }}
                                  >
                                    <span className="min-w-0 truncate">
                                      {displaySequenceId}
                                    </span>
                                  </button>
                                ) : (
                                  <div className="min-w-0 shrink truncate font-mono text-[0.95rem] font-semibold tracking-[0.08em] text-base-content">
                                    {displaySequenceId}
                                  </div>
                                )}
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
