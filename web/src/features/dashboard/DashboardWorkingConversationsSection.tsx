import { useWindowVirtualizer } from "@tanstack/react-virtual";
import {
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { InvocationErrorSummary } from "../../components/InvocationErrorSummary";
import { Alert } from "../../components/ui/alert";
import { Badge } from "../../components/ui/badge";
import { BubblePopoverContent } from "../../components/ui/bubble-popover";
import { Button } from "../../components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../../components/ui/dialog";
import { Popover, PopoverTrigger } from "../../components/ui/popover";
import { SegmentedControl, SegmentedControlItem } from "../../components/ui/segmented-control";
import { SelectField } from "../../components/ui/select-field";
import { Spinner } from "../../components/ui/spinner";
import { Tooltip } from "../../components/ui/tooltip";
import { useDashboardUpstreamAccountActivity } from "../../hooks/useDashboardUpstreamAccountActivity";
import { DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MAX } from "../../hooks/useDashboardWorkingConversations";
import type { TranslationKey } from "../../i18n";
import { useTranslation } from "../../i18n";
import type {
  ModelPerformance,
  PromptCacheConversationRewriteMode,
  TagFastModeRewriteMode,
  TagPriorityTier,
  UpdateGroupAccountRoutingRulePayload,
  UpstreamAccountActivityAccount,
  UpstreamAccountActivityResponse,
  UpstreamAccountGroupSummary,
  UpstreamAccountSummary,
} from "../../lib/api";
import {
  bulkUpdatePromptCacheConversationBindings,
  fetchUpstreamAccounts,
  updateUpstreamAccount,
} from "../../lib/api";
import type {
  DashboardWorkingConversationCardModel,
  DashboardWorkingConversationInvocationModel,
  DashboardWorkingConversationInvocationSelection,
  DashboardWorkingConversationTone,
} from "../../lib/dashboardWorkingConversations";
import {
  buildDashboardWorkingConversationInvocationModel,
  DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE,
  formatDashboardWorkingConversationSequenceId,
  hashDashboardWorkingConversationKey,
} from "../../lib/dashboardWorkingConversations";
import {
  type InvocationEndpointDisplay,
  type InvocationImageIntentDisplay,
  isImageInvocationEndpointKind,
  resolveFirstResponseByteTotalMs,
} from "../../lib/invocation";
import {
  compactUpstreamPlanLabel,
  shouldShowUpstreamPlanBadge,
  upstreamPlanBadgeRecipe,
} from "../../lib/upstreamAccountBadges";
import { emitUpstreamAccountsChanged } from "../../lib/upstreamAccountsEvents";
import { cn } from "../../lib/utils";
import { InvocationPhaseBadge, InvocationPhaseSegments } from "../invocations/InvocationPhaseBadge";
import {
  buildInvocationDetailViewModel,
  FALLBACK_CELL,
  INVOCATION_ACCOUNT_ROUTING_IN_PROGRESS_CLASS_NAME,
  renderEndpointSummary,
  renderFastIndicator,
  renderInvocationModelBadge,
} from "../invocations/invocation-details-shared";
import {
  getReasoningEffortTone,
  REASONING_EFFORT_TONE_CLASSNAMES,
} from "../invocations/invocation-table-reasoning";
import { renderInvocationTransportBadge } from "../invocations/invocation-transport-badge";
import { AdaptiveDisplayValue } from "../shared/AdaptiveMetricValue";
import { AnimatedDigits } from "../shared/AnimatedDigits";
import { AppIcon, type AppIconName } from "../shared/AppIcon";
import {
  type AdaptiveDisplayValueSpec,
  buildAdaptiveCurrencyAmountTextSpec,
  buildAdaptiveCurrencyTextSpec,
  buildAdaptiveDurationTextSpec,
  buildAdaptiveNumberTextSpec,
  buildAdaptivePercentTextSpec,
  buildAdaptiveTextSpec,
} from "../shared/adaptiveMetricValueSpec";
import {
  DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY,
  type DashboardActivityRangeKey,
  type DashboardWorkspaceView,
  persistDashboardWorkspaceView,
  readPersistedDashboardWorkspaceView,
} from "./dashboardActivityRange";
import { formatDashboardNetworkSpeed } from "./dashboardNetworkFormatting";
import {
  compareDashboardConversationCards,
  compareDashboardUpstreamAccounts,
  DASHBOARD_CONVERSATION_SORT_STORAGE_KEY,
  DASHBOARD_UPSTREAM_ACCOUNT_SORT_STORAGE_KEY,
  type DashboardWorkspaceSort,
  nextDashboardWorkspaceSort,
  persistDashboardWorkspaceSort,
  readDashboardWorkspaceSort,
} from "./dashboardWorkspaceSort";
import { ModelPerformanceTrigger } from "./ModelPerformanceTrigger";
import { UsageBreakdownTooltip } from "./UsageBreakdownTooltip";

export interface DashboardOpenUpstreamAccountOptions {
  tab?: "overview" | "routing" | "healthEvents";
}

interface DashboardWorkingConversationsSectionProps {
  activeRange: DashboardActivityRangeKey;
  cards: DashboardWorkingConversationCardModel[];
  totalMatched?: number;
  hasMore?: boolean;
  recentPreviewLimit?: number;
  isLoading: boolean;
  isLoadingMore?: boolean;
  error?: string | null;
  onLoadMore?: () => void;
  setRefreshTargetCount?: (count: number) => void;
  onOpenUpstreamAccount?: (
    accountId: number,
    accountLabel: string,
    options?: DashboardOpenUpstreamAccountOptions,
  ) => void;
  onOpenConversation?: (selection: DashboardWorkingConversationSelection) => void;
  onOpenInvocation?: (selection: DashboardWorkingConversationInvocationSelection) => void;
  upstreamAccountActivity?: UpstreamAccountActivityResponse | null;
  upstreamAccountActivityLoading?: boolean;
  upstreamAccountActivityRefreshing?: boolean;
  upstreamAccountActivityError?: string | null;
  upstreamAccountRecentLoading?: boolean;
  upstreamAccountRecentError?: string | null;
  onRetryUpstreamAccountRecent?: () => void;
  upstreamAccountRecentPreviewLimit?: number;
  onUpstreamAccountActivityEnabledChange?: (enabled: boolean) => void;
  onUpstreamAccountPolicyChanged?: () => void;
  onConversationsChanged?: () => void;
}

function readBrowserOfflineState() {
  return typeof navigator === "undefined" ? false : !navigator.onLine;
}

export interface DashboardWorkingConversationSelection {
  conversationSequenceId: string;
  promptCacheKey: string;
  tab?: "overview" | "calls" | "settings";
}

const ACCOUNT_CARD_CLASS_NAME =
  "flex h-full min-w-0 w-full max-w-full flex-col overflow-hidden rounded-[1rem] border border-[rgba(148,163,184,0.32)] bg-base-100/72 p-4 shadow-[0_6px_12px_rgba(15,23,42,0.07)] desktop1660:min-h-[31.5rem]";

const ACCOUNT_CARD_INNER_BORDER_CLASS_NAME = "border-[rgba(148,163,184,0.22)]";
const ACCOUNT_CARD_INNER_RING_CLASS_NAME = "ring-[rgba(148,163,184,0.22)]";
const DASHBOARD_RECENT_SKELETON_IDS = Array.from(
  { length: DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MAX },
  (_, index) => `recent-skeleton-${index + 1}`,
);
const DASHBOARD_ACCOUNT_SKELETON_IDS = ["account-skeleton-primary", "account-skeleton-secondary"];
const DASHBOARD_ACCOUNT_METRIC_SKELETON_IDS = [
  "metric-requests",
  "metric-success",
  "metric-tokens",
  "metric-cost",
];
const DASHBOARD_ACCOUNT_RECENT_SKELETON_IDS = [
  "recent-row-1",
  "recent-row-2",
  "recent-row-3",
  "recent-row-4",
];
const UPSTREAM_ACCOUNT_REFRESH_CHIP_SHOW_DELAY_MS = 300;
const UPSTREAM_ACCOUNT_REFRESH_CHIP_MIN_VISIBLE_MS = 600;
const MANUAL_BINDING_BADGE_CLASS_NAME =
  "inline-flex min-w-0 max-w-full items-center rounded-full border px-2 py-0.5 text-[10.5px] font-medium leading-4";
const MANUAL_BINDING_BADGE_BUTTON_CLASS_NAME =
  "inline-flex min-w-0 max-w-[20rem] shrink appearance-none rounded-full border-0 bg-transparent p-0 text-left transition-opacity duration-200 hover:opacity-80 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary";
const MANUAL_BINDING_BADGE_TEXT_CLASS_NAME =
  "block min-w-0 max-w-[20rem] truncate whitespace-nowrap";

type DashboardManualBindingBadgeMeta = {
  displayValue: string;
  accessibleLabel: string;
  toneClassName: string;
};

type DashboardConversationBulkBindTargetKind = "group" | "upstreamAccount";

type DashboardConversationBulkFeedback = {
  variant: "success" | "warning" | "error";
  message: string;
};

type DashboardConversationBulkBindingTargetsState = {
  accounts: UpstreamAccountSummary[];
  groups: string[];
  loading: boolean;
  loaded: boolean;
  error: string | null;
};

function hasMultiSelectModifier(
  event: Pick<ReactMouseEvent<HTMLElement>, "metaKey" | "ctrlKey" | "button">,
) {
  return event.button === 0 && (event.metaKey || event.ctrlKey);
}

function resolveDashboardManualBindingBadgeMeta(
  binding: DashboardWorkingConversationCardModel["manualBinding"],
  t: (key: TranslationKey, params?: Record<string, string | number>) => string,
): DashboardManualBindingBadgeMeta | null {
  if (!binding) return null;
  if (binding.bindingKind === "group") {
    const groupName = binding.groupName?.trim();
    if (!groupName) return null;
    return {
      displayValue: groupName,
      accessibleLabel: t("live.conversations.drawer.binding.currentGroup", {
        group: groupName,
      }),
      toneClassName: "border-info/35 bg-info/15 text-info",
    };
  }

  const upstreamAccountLabel =
    binding.upstreamAccountName?.trim() ||
    (binding.upstreamAccountId != null ? `#${binding.upstreamAccountId}` : "");
  if (!upstreamAccountLabel) return null;
  return {
    displayValue: upstreamAccountLabel,
    accessibleLabel: t("live.conversations.drawer.binding.currentAccount", {
      account: upstreamAccountLabel,
    }),
    toneClassName: "border-secondary/45 bg-secondary/14 text-secondary",
  };
}

function conversationBindingAccountLabel(account: UpstreamAccountSummary) {
  const identity = account.email?.trim() || account.displayName.trim();
  const group = account.groupName?.trim();
  return group ? `${identity} · ${group}` : identity;
}

function accountCanBePromptCacheBindingTarget(account: UpstreamAccountSummary) {
  if (account.provider !== "codex" || !account.enabled || account.status !== "active") {
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

function normalizeConversationBindingGroups(
  groups: UpstreamAccountGroupSummary[],
  accounts: UpstreamAccountSummary[],
  localeTag: string,
) {
  return Array.from(
    new Set(
      [
        ...groups.map((group) => group.groupName?.trim() ?? ""),
        ...accounts.map((account) => account.groupName?.trim() ?? ""),
      ].filter((groupName) => groupName.length > 0),
    ),
  ).sort((left, right) => left.localeCompare(right, localeTag));
}

function formatDashboardConversationBulkFailureMessage(
  failedItems: Array<{ promptCacheKey: string; error: string | null }>,
  locale: "zh" | "en",
) {
  if (failedItems.length === 0) return null;
  const sample = failedItems
    .slice(0, 3)
    .map((item) =>
      item.error?.trim() ? `${item.promptCacheKey}: ${item.error.trim()}` : item.promptCacheKey,
    )
    .join(" · ");
  if (locale === "zh") {
    return `有 ${failedItems.length} 个对话批量操作失败：${sample}`;
  }
  return `${failedItems.length} conversations failed to update: ${sample}`;
}

type StatusMeta = {
  badgeVariant: "default" | "secondary" | "success" | "warning" | "error" | "info";
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
};

const CARD_CLASS_NAME =
  "relative min-w-0 overflow-hidden rounded-[1.1rem] p-2.5 sm:p-3 shadow-[inset_0_1px_0_rgba(255,255,255,0.04),0_16px_28px_rgba(2,6,23,0.18)] transition-shadow duration-200 hover:shadow-[inset_0_1px_0_rgba(255,255,255,0.05),0_20px_34px_rgba(2,6,23,0.22)] focus-within:shadow-[inset_0_1px_0_rgba(255,255,255,0.06),0_0_0_1px_rgba(56,189,248,0.2),0_20px_34px_rgba(2,6,23,0.22)]";

const SLOT_CLASS_NAME =
  "flex min-w-0 flex-col overflow-hidden rounded-[0.95rem] px-2.5 py-2 shadow-[inset_0_1px_0_rgba(255,255,255,0.04)]";

const CARD_SURFACE_CLASS_NAME = "working-conversation-card-surface";

const INVOCATION_SURFACE_CLASS_NAME = "working-conversation-slot-surface";
const DASHBOARD_WORKING_CONVERSATION_ROW_GAP_PX = 16;
const UPSTREAM_ACCOUNT_RECENT_COMPACT_BADGE_CLASS_NAME =
  "min-h-5 border-transparent bg-base-200/82 px-2 py-0.5 text-[9px] font-semibold leading-none text-base-content/76 shadow-none";

const UPSTREAM_ACCOUNT_RECENT_IDENTITY_CHIP_CLASS_NAME =
  "inline-flex h-[1.2rem] max-w-[4.8rem] shrink-0 items-center rounded-full border px-1.5 font-mono text-[10px] font-semibold leading-none tracking-[0.04em]";

const ACCOUNT_HEADER_BADGE_CLASS_NAME =
  "inline-flex h-6 shrink-0 items-center rounded-full border px-2.5 text-[11px] font-semibold leading-none";
const ACCOUNT_CARD_STACKED_HEADER_BREAKPOINT_PX = 620;
const ACCOUNT_CARD_HERO_SINGLE_COLUMN_BREAKPOINT_PX = 300;
const ACCOUNT_CARD_HERO_TWO_COLUMN_BREAKPOINT_PX = 760;
const ACCOUNT_CARD_RECENT_STACK_BREAKPOINT_PX = 520;

const UPSTREAM_ACCOUNT_RECENT_IDENTITY_TONE_CLASSNAMES = [
  "dashboard-upstream-account-identity-chip--tone-sky",
  "dashboard-upstream-account-identity-chip--tone-cyan",
  "dashboard-upstream-account-identity-chip--tone-blue",
  "dashboard-upstream-account-identity-chip--tone-violet",
  "dashboard-upstream-account-identity-chip--tone-indigo",
  "dashboard-upstream-account-identity-chip--tone-fuchsia",
  "dashboard-upstream-account-identity-chip--tone-teal",
  "dashboard-upstream-account-identity-chip--tone-emerald",
] as const;

function resolveConversationIdentityToneClassName(seed: string) {
  const hash = hashDashboardWorkingConversationKey(seed);
  const hashValue = Number.parseInt(hash, 16) >>> 0;
  const mixedHash = (hashValue ^ (hashValue >>> 7) ^ (hashValue >>> 13) ^ (hashValue >>> 21)) >>> 0;
  const toneIndex = mixedHash % UPSTREAM_ACCOUNT_RECENT_IDENTITY_TONE_CLASSNAMES.length;
  return UPSTREAM_ACCOUNT_RECENT_IDENTITY_TONE_CLASSNAMES[toneIndex];
}

const STATUS_META: Record<DashboardWorkingConversationTone, StatusMeta> = {
  running: {
    badgeVariant: "default",
    icon: "loading",
    labelKey: "table.status.running",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
  },
  pending: {
    badgeVariant: "warning",
    icon: "timer-refresh-outline",
    labelKey: "table.status.pending",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
  },
  success: {
    badgeVariant: "success",
    icon: "check-circle-outline",
    labelKey: "table.status.success",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
  },
  warning: {
    badgeVariant: "warning",
    icon: "alert-outline",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
  },
  error: {
    badgeVariant: "error",
    icon: "alert-circle-outline",
    labelKey: "table.status.failed",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
  },
  neutral: {
    badgeVariant: "secondary",
    icon: "information-outline",
    labelKey: "table.status.unknown",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
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
        "inline-flex min-h-5 max-w-[5rem] shrink-0 items-center rounded-full border px-2 py-0.5 text-[9px] font-semibold leading-none tracking-[0.01em]",
        REASONING_EFFORT_TONE_CLASSNAMES[tone],
      )}
      title={value}
    >
      <span className="truncate whitespace-nowrap">{value}</span>
    </span>
  );
}

function CompactLatencyPills({
  firstResponseByteTotalValue,
  responseTimeValue,
  t,
  className,
}: {
  firstResponseByteTotalValue: string;
  responseTimeValue: string;
  t: ReturnType<typeof useTranslation>["t"];
  className?: string;
}) {
  const firstResponseTimeLabel = t("dashboard.today.firstResponseTime");
  const responseTimeLabel = t("dashboard.today.responseTime");

  return (
    <div
      data-testid="dashboard-compact-latency-pills"
      className={cn(
        "inline-flex min-w-0 shrink-0 flex-wrap items-center justify-end gap-2 font-mono text-[11px] font-semibold leading-none text-base-content/86",
        className,
      )}
      aria-label={`${firstResponseTimeLabel} ${firstResponseByteTotalValue}; ${responseTimeLabel} ${responseTimeValue}`}
      title={`${firstResponseTimeLabel}: ${firstResponseByteTotalValue} · ${responseTimeLabel}: ${responseTimeValue}`}
    >
      <span
        data-testid="dashboard-compact-latency-first-byte"
        className="inline-flex min-w-0 items-center gap-1 text-secondary"
      >
        <AppIcon name="timer-outline" className="h-3.5 w-3.5 shrink-0" aria-hidden />
        <span className="truncate whitespace-nowrap">{firstResponseByteTotalValue}</span>
      </span>
      <span
        data-testid="dashboard-compact-latency-response-time"
        className="inline-flex min-w-0 items-center gap-1 text-primary"
      >
        <AppIcon name="speedometer" className="h-3.5 w-3.5 shrink-0" aria-hidden />
        <span className="truncate whitespace-nowrap">{responseTimeValue}</span>
      </span>
    </div>
  );
}

function DashboardImageToolIconBadge({
  endpointDisplay,
  imageIntentDisplay,
  t,
  className,
}: {
  endpointDisplay: Pick<InvocationEndpointDisplay, "kind">;
  imageIntentDisplay: InvocationImageIntentDisplay;
  t: ReturnType<typeof useTranslation>["t"];
  className?: string;
}) {
  if (isImageInvocationEndpointKind(endpointDisplay.kind)) {
    return null;
  }

  if (
    !imageIntentDisplay.showsBadge ||
    imageIntentDisplay.badgeVariant == null ||
    imageIntentDisplay.badgeLabelKey == null
  ) {
    return null;
  }

  const label = t(imageIntentDisplay.badgeLabelKey);

  return (
    <Badge
      variant={imageIntentDisplay.badgeVariant}
      className={cn(
        "h-5 w-5 justify-center overflow-hidden px-0 py-0 text-[11px] leading-none shadow-none",
        className,
      )}
      data-testid="dashboard-image-tool-icon-badge"
      data-image-intent-kind={imageIntentDisplay.kind}
      aria-label={label}
      title={label}
      role="img"
    >
      <AppIcon name="image-outline" className="h-3.5 w-3.5" aria-hidden />
    </Badge>
  );
}

function renderUpstreamAccountRecentModelDisplay(
  hasMismatch: boolean,
  modelValue: string,
  requestModelValue: string,
  responseModelValue: string,
  t: ReturnType<typeof useTranslation>["t"],
) {
  const shouldRenderMismatch =
    hasMismatch && requestModelValue !== FALLBACK_CELL && responseModelValue !== FALLBACK_CELL;

  if (!shouldRenderMismatch) {
    return renderInvocationModelBadge(modelValue, {
      t,
      hasMismatch: false,
      className: "max-w-full",
      textClassName: "font-mono",
      iconClassName: "h-3 w-3",
      testId: "dashboard-upstream-account-recent-model",
    });
  }

  return (
    <div
      data-testid="dashboard-upstream-account-recent-model"
      className="flex min-w-0 items-center gap-1"
      title={`${requestModelValue} -> ${responseModelValue}`}
    >
      <span className="truncate font-mono leading-none text-base-content/84">
        {requestModelValue}
      </span>
      <span
        className="inline-flex h-4 w-4 flex-none items-center justify-center text-base-content/55"
        aria-label={t("table.model.routingMismatchAria")}
        data-testid="dashboard-upstream-account-recent-model-routing-indicator"
        role="img"
      >
        <AppIcon name="compare-horizontal" className="h-3 w-3" aria-hidden />
      </span>
      <span className="truncate font-mono leading-none text-base-content/88">
        {responseModelValue}
      </span>
    </div>
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

function resolveStatusMeta(tone: DashboardWorkingConversationTone, status: string): StatusMeta {
  const base = STATUS_META[tone];
  const normalized = status.trim().toLowerCase();
  if (normalized === "warning_success") {
    return {
      ...base,
      badgeVariant: "warning",
      icon: "alert-outline",
      labelKey: "table.status.warningSuccess",
    };
  }
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

function statusInlineToneClassName(variant: StatusMeta["badgeVariant"]) {
  if (variant === "success") return "text-success";
  if (variant === "warning") return "text-warning";
  if (variant === "error") return "text-error";
  if (variant === "info") return "text-info";
  if (variant === "default") return "text-primary";
  return "text-base-content/62";
}

function buildStatusAssistiveLabel(label: string, detail?: string | null) {
  const resolvedDetail = detail?.trim();
  if (!resolvedDetail) return label;
  return `${label} · ${resolvedDetail}`;
}

function InlineInvocationStatus({
  meta,
  label,
  className,
  showLabel = true,
  detail,
}: {
  meta: StatusMeta;
  label: string;
  className?: string;
  showLabel?: boolean;
  detail?: string | null;
}) {
  const toneClassName = statusInlineToneClassName(meta.badgeVariant);
  const assistiveLabel = buildStatusAssistiveLabel(label, detail);
  if (!showLabel) {
    return (
      <Tooltip
        side="bottom"
        sideOffset={8}
        className={cn(
          "h-5 w-5 items-center justify-center rounded-full bg-base-100/12",
          toneClassName,
          className,
        )}
        content={assistiveLabel}
        contentClassName="max-w-[min(32rem,calc(100vw-1rem))] whitespace-pre-wrap break-words"
        triggerProps={{
          "data-testid": "dashboard-inline-invocation-status",
          "aria-label": assistiveLabel,
          role: "img",
        }}
      >
        <AppIcon
          name={meta.icon}
          className={cn("h-3.5 w-3.5 shrink-0", meta.icon === "loading" && "animate-spin")}
          aria-hidden
        />
      </Tooltip>
    );
  }

  return (
    <span
      data-testid="dashboard-inline-invocation-status"
      className={cn(
        "inline-flex items-center gap-1 whitespace-nowrap text-[11px] font-semibold leading-none",
        toneClassName,
        className,
      )}
    >
      <AppIcon
        name={meta.icon}
        className={cn("h-3.5 w-3.5 shrink-0", meta.icon === "loading" && "animate-spin")}
        aria-hidden
      />
      <span>{label}</span>
    </span>
  );
}

function formatAccountPercentValue(value: number | null | undefined, localeTag: string) {
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

function formatAccountDurationValue(value: number | null | undefined, localeTag: string) {
  if (value == null || !Number.isFinite(value)) return FALLBACK_CELL;
  const abs = Math.abs(value);
  if (abs >= 1000) {
    const seconds = value / 1000;
    const maximumFractionDigits = abs >= 100_000 ? 1 : 2;
    return `${formatAccountNumberValue(seconds, localeTag, maximumFractionDigits)} s`;
  }
  return `${formatAccountNumberValue(value, localeTag, abs >= 100 ? 0 : 1)} ms`;
}

function countCompactDisplayDigits(value: number) {
  const absoluteValue = Math.abs(value);
  if (absoluteValue < 1) return 1;
  return Math.trunc(absoluteValue).toString().length;
}

function resolveCompactSecondsFractionDigits(seconds: number) {
  return Math.max(0, Math.min(2, 4 - countCompactDisplayDigits(seconds)));
}

function formatCompactLatencySecondsValue(value: number | null | undefined, localeTag: string) {
  if (value == null || !Number.isFinite(value)) return FALLBACK_CELL;

  const seconds = value / 1000;
  const firstPassFractionDigits = resolveCompactSecondsFractionDigits(seconds);
  const firstPassRounded = Number(seconds.toFixed(firstPassFractionDigits));
  const fractionDigits = resolveCompactSecondsFractionDigits(firstPassRounded);
  const rounded = Number(seconds.toFixed(fractionDigits));

  return `${rounded.toLocaleString(localeTag, {
    useGrouping: false,
    minimumFractionDigits: 0,
    maximumFractionDigits: fractionDigits,
  })} s`;
}

function formatCompactElapsedSecondsFromTimestamp(
  occurredAt: string | null | undefined,
  localeTag: string,
  nowMs: number,
) {
  const occurredMs = occurredAt ? Date.parse(occurredAt) : Number.NaN;
  if (!Number.isFinite(occurredMs)) return FALLBACK_CELL;
  return formatCompactLatencySecondsValue(Math.max(0, nowMs - occurredMs), localeTag);
}

function finiteNumber(value: number | null | undefined) {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function accountCostShare(numerator: number | null | undefined, total: number | null | undefined) {
  const resolvedNumerator = finiteNumber(numerator);
  const resolvedTotal = finiteNumber(total);
  if (resolvedNumerator == null || resolvedTotal == null) return null;
  if (resolvedTotal <= 0) return resolvedNumerator <= 0 ? 0 : null;
  return Math.max(0, resolvedNumerator) / resolvedTotal;
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

type AccountMetricDetailRow = {
  label: string;
  value: string;
  tone?: AccountMetricTone;
};

type AccountMetricDetailSection = {
  title: string;
  rows: AccountMetricDetailRow[];
};

type AccountDisplayValue = {
  spec: AdaptiveDisplayValueSpec;
  fullText: string;
  ariaText: string;
};

function toAccountDisplayValue(spec: AdaptiveDisplayValueSpec): AccountDisplayValue {
  return {
    spec,
    fullText: spec.fullValue,
    ariaText: spec.fullValue,
  };
}

function buildAccountPercentDisplayValue(value: number | null | undefined, localeTag: string) {
  return toAccountDisplayValue(
    buildAdaptivePercentTextSpec(value ?? null, localeTag, {
      maximumFractionDigits: 1,
    }),
  );
}

function buildAccountNumberDisplayValue(
  value: number | null | undefined,
  localeTag: string,
  maximumFractionDigits = 0,
) {
  return toAccountDisplayValue(
    buildAdaptiveNumberTextSpec(value ?? null, localeTag, maximumFractionDigits),
  );
}

function buildAccountCurrencyDisplayValue(
  value: number | null | undefined,
  localeTag: string,
  maximumFractionDigits = 2,
) {
  if (value == null || !Number.isFinite(value)) {
    return toAccountDisplayValue(
      buildAdaptiveTextSpec(FALLBACK_CELL, [
        { key: "placeholder", value: FALLBACK_CELL, priority: 0 },
      ]),
    );
  }

  const precisionCandidates = Array.from(
    { length: maximumFractionDigits + 1 },
    (_, index) => maximumFractionDigits - index,
  );
  const fullValue = new Intl.NumberFormat(localeTag, {
    style: "currency",
    currency: "USD",
    minimumFractionDigits: maximumFractionDigits,
    maximumFractionDigits,
  }).format(value);

  return toAccountDisplayValue(
    buildAdaptiveTextSpec(fullValue, [
      {
        key: "full",
        value: fullValue,
        priority: 0,
      },
      ...precisionCandidates
        .filter((precision) => precision !== maximumFractionDigits)
        .map((precision, index) => ({
          key: `standard-${precision}`,
          value: new Intl.NumberFormat(localeTag, {
            style: "currency",
            currency: "USD",
            minimumFractionDigits: precision,
            maximumFractionDigits: precision,
          }).format(value),
          priority: index + 1,
        })),
      ...buildAdaptiveCurrencyTextSpec(value, localeTag).candidates.map((candidate, index) => ({
        key: candidate.key,
        value: candidate.value,
        priority: 20 + index,
      })),
    ]),
  );
}

function buildAccountCurrencyAmountDisplayValue(
  value: number | null | undefined,
  localeTag: string,
  maximumFractionDigits = 2,
) {
  return toAccountDisplayValue(
    buildAdaptiveCurrencyAmountTextSpec(value ?? null, localeTag, {
      maximumFractionDigits,
      minimumFractionDigits: maximumFractionDigits,
    }),
  );
}

function buildAccountDurationDisplayValue(value: number | null | undefined, localeTag: string) {
  return toAccountDisplayValue(buildAdaptiveDurationTextSpec(value ?? null, localeTag));
}

const ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES: Record<AccountMetricTone, string> = {
  neutral: "text-base-content",
  primary: "text-primary",
  secondary: "text-secondary",
  success: "text-success",
  warning: "text-accent",
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

const ACCOUNT_INLINE_METRIC_ICON_AND_GAP_PX = 26;
const ACCOUNT_INLINE_TPM_SPLIT_VALUE_WIDTH_BUDGET_CH = 6;

type AccountQuickPolicyDraft = {
  priorityTier: TagPriorityTier;
  allowCutOut: boolean;
  allowCutIn: boolean;
  fastModeRewriteMode: TagFastModeRewriteMode;
};

type AccountQuickPolicyTone = "neutral" | "success" | "warning" | "primary";

type AccountAttentionBadge = {
  key: string;
  label: string;
  tone: "warning" | "error" | "info";
  title?: string;
};

function accountPolicyDraftFromRule(
  account: UpstreamAccountActivityAccount,
): AccountQuickPolicyDraft {
  const rule = account.effectiveRoutingRule ?? {
    allowCutOut: true,
    allowCutIn: true,
    priorityTier: "normal" as TagPriorityTier,
    fastModeRewriteMode: "keep_original" as TagFastModeRewriteMode,
  };
  return {
    priorityTier: rule.priorityTier ?? "normal",
    allowCutOut: rule.allowCutOut !== false,
    allowCutIn: rule.allowCutIn !== false,
    fastModeRewriteMode: rule.fastModeRewriteMode ?? "keep_original",
  };
}

function cycleAccountPriorityPolicy(draft: AccountQuickPolicyDraft): AccountQuickPolicyDraft {
  if (draft.priorityTier === "normal") {
    return { ...draft, priorityTier: "fallback" };
  }
  if (draft.priorityTier === "fallback") {
    return { ...draft, priorityTier: "primary" };
  }
  if (draft.priorityTier === "primary") {
    return { ...draft, priorityTier: "no_new" };
  }
  return { ...draft, priorityTier: "normal" };
}

function cycleAccountFastModePolicy(draft: AccountQuickPolicyDraft): AccountQuickPolicyDraft {
  if (draft.fastModeRewriteMode === "keep_original") {
    return { ...draft, fastModeRewriteMode: "fill_missing" };
  }
  if (draft.fastModeRewriteMode === "fill_missing") {
    return { ...draft, fastModeRewriteMode: "force_add" };
  }
  if (draft.fastModeRewriteMode === "force_add") {
    return { ...draft, fastModeRewriteMode: "force_remove" };
  }
  return { ...draft, fastModeRewriteMode: "keep_original" };
}

function priorityPolicyLabel(draft: AccountQuickPolicyDraft, locale: "zh" | "en") {
  if (draft.priorityTier === "no_new") return locale === "zh" ? "禁新" : "No new";
  if (draft.priorityTier === "primary") return locale === "zh" ? "主力" : "Primary";
  if (draft.priorityTier === "fallback") return locale === "zh" ? "兜底" : "Fallback";
  return locale === "zh" ? "普通" : "Normal";
}

function fastModePolicyLabel(mode: TagFastModeRewriteMode, locale: "zh" | "en") {
  if (mode === "fill_missing") return locale === "zh" ? "补Fast" : "+Fast";
  if (mode === "force_add") return locale === "zh" ? "强制Fast" : "Force Fast";
  if (mode === "force_remove") return locale === "zh" ? "禁Fast" : "No Fast";
  return locale === "zh" ? "不改Fast" : "Leave Fast";
}

function fastModePolicyCycleTitle(currentLabel: string, locale: "zh" | "en") {
  if (locale === "zh") {
    return `Fast 改写策略：${currentLabel}。点击循环 不改Fast / 补Fast / 强制Fast / 禁Fast`;
  }
  return `Fast rewrite policy: ${currentLabel}. Cycle Leave Fast / +Fast / Force Fast / No Fast`;
}

function fastModePolicyAriaLabel(currentLabel: string, locale: "zh" | "en") {
  if (locale === "zh") {
    return `Fast 改写策略：${currentLabel}，点击切换`;
  }
  return `Fast rewrite policy: ${currentLabel}, click to cycle`;
}

function priorityPolicyTone(draft: AccountQuickPolicyDraft): AccountQuickPolicyTone {
  if (draft.priorityTier === "no_new") return "warning";
  if (draft.priorityTier === "primary") return "primary";
  if (draft.priorityTier === "fallback") return "success";
  return "neutral";
}

function fastModePolicyTone(mode: TagFastModeRewriteMode): AccountQuickPolicyTone {
  if (mode === "force_remove") return "warning";
  if (mode === "force_add") return "primary";
  if (mode === "fill_missing") return "success";
  return "neutral";
}

function booleanBlockPolicyTone(isBlocked: boolean): AccountQuickPolicyTone {
  return isBlocked ? "warning" : "neutral";
}

const ACCOUNT_QUICK_POLICY_TONE_CLASSNAMES: Record<AccountQuickPolicyTone, string> = {
  neutral: "border-base-300/80 bg-base-100/75 text-base-content/60",
  success:
    "border-success/40 bg-success/15 text-success shadow-[inset_0_1px_0_rgba(255,255,255,0.18)]",
  warning:
    "border-warning/50 bg-warning/15 text-base-content shadow-[inset_0_1px_0_rgba(255,255,255,0.18)]",
  primary:
    "border-primary/50 bg-primary/15 text-primary shadow-[inset_0_1px_0_rgba(255,255,255,0.18)]",
};

function normalizeStatusToken(value: string | null | undefined) {
  return value?.trim().toLowerCase().replace(/-/g, "_") ?? "";
}

function resolveAccountAttentionBadges(
  account: UpstreamAccountActivityAccount,
  locale: "zh" | "en",
): AccountAttentionBadge[] {
  const labels = {
    disabled: locale === "zh" ? "禁用" : "Disabled",
    syncing: locale === "zh" ? "同步中" : "Syncing",
    upstreamRejected: locale === "zh" ? "上游拒绝" : "Rejected",
    upstreamUnavailable: locale === "zh" ? "上游不可达" : "Unavailable",
    needsReauth: locale === "zh" ? "需重登" : "Reauth",
    rateLimited: locale === "zh" ? "限流" : "Limited",
    degraded: locale === "zh" ? "降级" : "Degraded",
    otherError: locale === "zh" ? "其它异常" : "Other error",
    unavailable: locale === "zh" ? "不可用" : "Unavailable",
  };
  const detail = account.lastActionReasonMessage || account.lastError || undefined;
  const badges: AccountAttentionBadge[] = [];
  const seen = new Set<string>();
  const add = (badge: AccountAttentionBadge) => {
    if (seen.has(badge.key)) return;
    seen.add(badge.key);
    badges.push(badge);
  };
  const enableStatus = normalizeStatusToken(account.enableStatus);
  const displayStatus = normalizeStatusToken(account.displayStatus);
  const healthStatus = normalizeStatusToken(account.healthStatus);
  const syncState = normalizeStatusToken(account.syncState);
  const workStatus = normalizeStatusToken(account.workStatus);

  if (account.enabled === false || enableStatus === "disabled" || displayStatus === "disabled") {
    add({
      key: "disabled",
      label: labels.disabled,
      tone: "warning",
      title: detail,
    });
  }
  if (syncState === "syncing" || displayStatus === "syncing") {
    add({ key: "syncing", label: labels.syncing, tone: "info", title: detail });
  }

  const healthSource = healthStatus || displayStatus;
  if (healthSource === "upstream_rejected") {
    add({
      key: "upstream_rejected",
      label: labels.upstreamRejected,
      tone: "error",
      title: detail,
    });
  } else if (healthSource === "upstream_unavailable") {
    add({
      key: "upstream_unavailable",
      label: labels.upstreamUnavailable,
      tone: "error",
      title: detail,
    });
  } else if (healthSource === "needs_reauth") {
    add({
      key: "needs_reauth",
      label: labels.needsReauth,
      tone: "error",
      title: detail,
    });
  } else if (healthSource === "error_other" || healthSource === "error") {
    add({
      key: "error_other",
      label: labels.otherError,
      tone: "error",
      title: detail,
    });
  }

  if (workStatus === "rate_limited" || displayStatus === "rate_limited") {
    add({
      key: "rate_limited",
      label: labels.rateLimited,
      tone: "warning",
      title: detail,
    });
  }
  if (workStatus === "degraded" || displayStatus === "degraded") {
    add({
      key: "degraded",
      label: labels.degraded,
      tone: "warning",
      title: detail,
    });
  }
  if (workStatus === "unavailable" && badges.length === 0) {
    add({
      key: "unavailable",
      label: labels.unavailable,
      tone: "error",
      title: detail,
    });
  }
  return badges;
}

function AccountAttentionBadges({
  account,
  locale,
  clickable,
  onClick,
}: {
  account: UpstreamAccountActivityAccount;
  locale: "zh" | "en";
  clickable: boolean;
  onClick?: () => void;
}) {
  const badges = resolveAccountAttentionBadges(account, locale);
  if (badges.length === 0) return null;
  const title = badges.map((badge) => badge.label).join(" · ");
  return (
    <button
      type="button"
      data-testid="dashboard-upstream-account-attention-badges"
      disabled={!clickable}
      className={cn(
        "inline-flex min-h-6 max-w-full flex-wrap items-center gap-1.5 rounded-full border border-base-300/70 bg-base-100/86 px-1.5 py-0.5 text-[11px] font-semibold transition-opacity duration-200 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary",
        clickable ? "cursor-pointer hover:opacity-80" : "cursor-default",
      )}
      title={title}
      aria-label={`${title} · ${locale === "zh" ? "打开账号健康事件" : "Open health events"}`}
      onClick={(event) => {
        event.stopPropagation();
        onClick?.();
      }}
      onKeyDown={(event) => {
        event.stopPropagation();
      }}
    >
      {badges.map((badge) => (
        <span
          key={badge.key}
          data-testid="dashboard-upstream-account-attention-badge"
          title={badge.title ?? badge.label}
          className={cn(
            ACCOUNT_HEADER_BADGE_CLASS_NAME,
            badge.tone === "error"
              ? "border-error/38 bg-error/10 text-error"
              : badge.tone === "warning"
                ? "border-warning/45 bg-warning/12 text-base-content"
                : "border-info/35 bg-info/12 text-info",
          )}
        >
          {badge.label}
        </span>
      ))}
    </button>
  );
}

function AccountQuickPolicyChips({
  draft,
  locale,
  disabled,
  isSaving,
  onCyclePriority,
  onCycleFastMode,
  onToggleCutOut,
  onToggleCutIn,
}: {
  draft: AccountQuickPolicyDraft;
  locale: "zh" | "en";
  disabled: boolean;
  isSaving: boolean;
  onCyclePriority: () => void;
  onCycleFastMode: () => void;
  onToggleCutOut: () => void;
  onToggleCutIn: () => void;
}) {
  const fastModeLabel = fastModePolicyLabel(draft.fastModeRewriteMode, locale);
  const cutOutActive = !draft.allowCutOut;
  const cutInActive = !draft.allowCutIn;
  const priorityTone = priorityPolicyTone(draft);
  const fastModeTone = fastModePolicyTone(draft.fastModeRewriteMode);
  const cutOutTone = booleanBlockPolicyTone(cutOutActive);
  const cutInTone = booleanBlockPolicyTone(cutInActive);
  const chipBase =
    "inline-flex h-6 shrink-0 items-center justify-center rounded-full border px-2.5 text-[11px] font-semibold leading-none transition-colors duration-150 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary disabled:cursor-not-allowed disabled:opacity-55";
  return (
    <div
      data-testid="dashboard-upstream-account-policy-badges"
      data-saving={isSaving ? "true" : "false"}
      className="flex min-w-0 flex-wrap items-center gap-1.5 overflow-hidden"
    >
      <button
        type="button"
        data-testid="dashboard-upstream-account-policy-badge"
        data-policy-key="priority-new-conversations"
        data-policy-tone={priorityTone}
        disabled={disabled}
        className={cn(chipBase, ACCOUNT_QUICK_POLICY_TONE_CLASSNAMES[priorityTone])}
        title={
          locale === "zh"
            ? "点击切换 普通 / 兜底 / 主力 / 禁新"
            : "Cycle normal / fallback / primary / no new"
        }
        aria-label={locale === "zh" ? "切换账号优先级" : "Cycle account priority"}
        onClick={(event) => {
          event.stopPropagation();
          onCyclePriority();
        }}
        onKeyDown={(event) => {
          event.stopPropagation();
        }}
      >
        {priorityPolicyLabel(draft, locale)}
      </button>
      <button
        type="button"
        data-testid="dashboard-upstream-account-policy-badge"
        data-policy-key="fast-mode-rewrite"
        data-policy-tone={fastModeTone}
        disabled={disabled}
        className={cn(chipBase, ACCOUNT_QUICK_POLICY_TONE_CLASSNAMES[fastModeTone])}
        title={fastModePolicyCycleTitle(fastModeLabel, locale)}
        aria-label={fastModePolicyAriaLabel(fastModeLabel, locale)}
        onClick={(event) => {
          event.stopPropagation();
          onCycleFastMode();
        }}
        onKeyDown={(event) => {
          event.stopPropagation();
        }}
      >
        {fastModeLabel}
      </button>
      <button
        type="button"
        data-testid="dashboard-upstream-account-policy-badge"
        data-policy-key="allow-cut-out"
        data-policy-tone={cutOutTone}
        disabled={disabled}
        className={cn(chipBase, ACCOUNT_QUICK_POLICY_TONE_CLASSNAMES[cutOutTone])}
        title={locale === "zh" ? "点击切换账号级禁出" : "Toggle account-level cut out"}
        aria-label={locale === "zh" ? "切换禁出" : "Toggle cut out"}
        onClick={(event) => {
          event.stopPropagation();
          onToggleCutOut();
        }}
        onKeyDown={(event) => {
          event.stopPropagation();
        }}
      >
        {locale === "zh" ? "禁出" : "No out"}
      </button>
      <button
        type="button"
        data-testid="dashboard-upstream-account-policy-badge"
        data-policy-key="allow-cut-in"
        data-policy-tone={cutInTone}
        disabled={disabled}
        className={cn(chipBase, ACCOUNT_QUICK_POLICY_TONE_CLASSNAMES[cutInTone])}
        title={locale === "zh" ? "点击切换账号级禁入" : "Toggle account-level cut in"}
        aria-label={locale === "zh" ? "切换禁入" : "Toggle cut in"}
        onClick={(event) => {
          event.stopPropagation();
          onToggleCutIn();
        }}
        onKeyDown={(event) => {
          event.stopPropagation();
        }}
      >
        {locale === "zh" ? "禁入" : "No in"}
      </button>
    </div>
  );
}

function AccountHeroMetric({
  label,
  value,
  tone,
  iconName,
  hint,
  detailSections,
  tooltipContent,
  metricKey,
  children,
}: {
  label: string;
  value: AccountDisplayValue;
  tone: "neutral" | "primary" | "secondary" | "success" | "warning" | "info";
  iconName: AppIconName;
  hint?: string;
  detailSections?: AccountMetricDetailSection[];
  tooltipContent?: ReactNode;
  metricKey?: string;
  children?: ReactNode;
}) {
  const valueTestId = metricKey ? `dashboard-upstream-account-${metricKey}-value` : undefined;
  const toneSurfaceClassName = {
    neutral: cn("bg-base-100/72 ring-1 ring-inset", ACCOUNT_CARD_INNER_RING_CLASS_NAME),
    primary: cn("bg-base-100/72 ring-1 ring-inset", ACCOUNT_CARD_INNER_RING_CLASS_NAME),
    secondary: cn("bg-base-100/72 ring-1 ring-inset", ACCOUNT_CARD_INNER_RING_CLASS_NAME),
    success: cn("bg-base-100/72 ring-1 ring-inset", ACCOUNT_CARD_INNER_RING_CLASS_NAME),
    warning: cn("bg-base-100/72 ring-1 ring-inset", ACCOUNT_CARD_INNER_RING_CLASS_NAME),
    info: cn("bg-base-100/72 ring-1 ring-inset", ACCOUNT_CARD_INNER_RING_CLASS_NAME),
  }[tone];
  const valueClassName =
    value.fullText === FALLBACK_CELL
      ? "text-base-content/55"
      : ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[tone === "neutral" ? "neutral" : tone];
  const iconClassName =
    value.fullText === FALLBACK_CELL
      ? "text-base-content/45"
      : ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[tone === "neutral" ? "neutral" : tone];

  const card = (
    <div
      data-testid="dashboard-upstream-account-metric-card"
      data-metric={metricKey}
      data-motion-surface
      className={cn(
        "h-full w-full rounded-[0.85rem] px-3 py-2.5 transition-colors duration-200",
        detailSections?.length || tooltipContent ? "cursor-help focus-within:outline-none" : null,
        toneSurfaceClassName,
      )}
    >
      <div className="text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/54">
        {label}
      </div>
      <div className="mt-1 flex min-w-0 items-center gap-1.5">
        <span
          aria-hidden
          data-testid={metricKey ? `dashboard-upstream-account-${metricKey}-icon` : undefined}
          className={cn(
            "flex h-[1.35rem] w-[1.35rem] shrink-0 items-center justify-center text-[1.22rem] leading-none",
            iconClassName,
          )}
        >
          <AppIcon name={iconName} className={cn(iconName === "send" && "-rotate-45")} />
        </span>
        <div
          className={cn(
            "min-w-0 flex-1 overflow-hidden text-ellipsis font-mono text-[1.08rem] font-semibold leading-none",
            valueClassName,
          )}
        >
          <AdaptiveDisplayValue
            spec={value.spec}
            className="block min-w-0 max-w-full"
            data-testid={valueTestId}
            animateDigits
          />
        </div>
      </div>
      {hint ? <div className="mt-1 text-[11px] leading-4 text-base-content/58">{hint}</div> : null}
      {children ? <div className="mt-1.5">{children}</div> : null}
    </div>
  );

  if (!detailSections?.length && !tooltipContent) return card;

  return (
    <Tooltip
      clickToOpen
      side="top"
      sideOffset={12}
      triggerElement="div"
      className="h-full w-full rounded-[0.85rem] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
      contentClassName={
        tooltipContent
          ? "max-w-[min(42rem,calc(100vw-1rem))] w-[min(42rem,calc(100vw-1rem))] px-3.5 py-3"
          : "w-[min(21rem,calc(100vw-1rem))] px-3.5 py-3"
      }
      content={
        tooltipContent ?? (
          <AccountMetricDetailTooltip
            label={label}
            value={value.fullText}
            valueClassName={valueClassName}
            sections={detailSections ?? []}
          />
        )
      }
      triggerProps={{
        tabIndex: 0,
        "aria-label": `${label} ${value.ariaText}`,
      }}
    >
      {card}
    </Tooltip>
  );
}

function AccountMetricDetailTooltip({
  label,
  value,
  valueClassName,
  sections,
}: {
  label: string;
  value: string;
  valueClassName: string;
  sections: AccountMetricDetailSection[];
}) {
  return (
    <div data-testid="dashboard-upstream-account-metric-tooltip" className="space-y-3">
      <div className="flex min-w-0 items-baseline justify-between gap-4 border-b border-base-300/45 pb-2">
        <div className="min-w-0 text-[11px] font-semibold leading-4 text-base-content/62">
          {label}
        </div>
        <div
          className={cn(
            "min-w-0 truncate text-right font-mono text-[1rem] font-semibold leading-none",
            valueClassName,
          )}
        >
          {value}
        </div>
      </div>
      {sections.map((section) => (
        <div key={section.title} className="space-y-1.5">
          <div className="text-[10px] font-semibold leading-4 text-base-content/52">
            {section.title}
          </div>
          <div className="space-y-1">
            {section.rows.map((row) => (
              <div
                key={`${section.title}:${row.label}`}
                className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-baseline gap-3"
              >
                <span className="min-w-0 truncate text-[11px] leading-4 text-base-content/68">
                  {row.label}
                </span>
                <span
                  className={cn(
                    "min-w-0 max-w-[12rem] truncate text-right font-mono text-[11px] font-semibold leading-4 text-base-content",
                    row.tone ? ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[row.tone] : null,
                  )}
                >
                  {row.value}
                </span>
              </div>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

function AccountInlineMetric({
  label,
  value,
  tone,
  iconName,
  metricKey,
  className,
  alignment = "start",
  fillAvailableWidth = false,
  valueWidthBudgetCh,
  modelPerformance,
  modelPerformanceTitle,
}: {
  label: string;
  value: AccountDisplayValue;
  tone: AccountMetricTone;
  iconName: AppIconName;
  metricKey?: string;
  className?: string;
  alignment?: "start" | "center" | "end";
  fillAvailableWidth?: boolean;
  valueWidthBudgetCh?: number;
  modelPerformance?: ModelPerformance | null;
  modelPerformanceTitle?: string;
}) {
  const valueTestId = metricKey
    ? `dashboard-upstream-account-inline-${metricKey}-value`
    : undefined;
  const slotTestId = metricKey ? `dashboard-upstream-account-inline-${metricKey}-slot` : undefined;
  const valueClassName =
    value.fullText === FALLBACK_CELL
      ? "text-base-content/55"
      : ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[tone];
  const iconClassName =
    value.fullText === FALLBACK_CELL
      ? "text-base-content/45"
      : ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[tone];
  const iconAdjustmentClassName =
    iconName === "send"
      ? "-rotate-45 -translate-y-[0.5px]"
      : iconName === "speedometer"
        ? "-translate-y-px"
        : iconName === "cash-clock"
          ? "translate-y-[0.5px]"
          : null;
  const widthMeasureRef = useRef<HTMLSpanElement | null>(null);
  const [availableWidthPx, setAvailableWidthPx] = useState<number | undefined>(undefined);

  useLayoutEffect(() => {
    if (!fillAvailableWidth) {
      setAvailableWidthPx(undefined);
      return undefined;
    }

    const element = widthMeasureRef.current;
    if (!element) return undefined;

    const syncWidth = () => {
      const nextWidth = Math.max(0, element.clientWidth - ACCOUNT_INLINE_METRIC_ICON_AND_GAP_PX);
      setAvailableWidthPx((current) => (current === nextWidth ? current : nextWidth));
    };

    syncWidth();
    window.addEventListener("resize", syncWidth);

    if (typeof ResizeObserver === "undefined") {
      return () => {
        window.removeEventListener("resize", syncWidth);
      };
    }

    const observer = new ResizeObserver(() => {
      syncWidth();
    });
    observer.observe(element);

    return () => {
      observer.disconnect();
      window.removeEventListener("resize", syncWidth);
    };
  }, [fillAvailableWidth]);

  const triggerAlignmentClassName =
    alignment === "center"
      ? "justify-center"
      : alignment === "end"
        ? "justify-end"
        : "justify-start";
  const triggerAriaLabel = `${label} ${value.ariaText}`;
  const modelPerformanceAriaLabel = `${triggerAriaLabel} ${modelPerformanceTitle}`;

  const metric = (
    <span
      className={cn(
        "inline-flex h-[1.35rem] min-w-0 max-w-full shrink-0 items-center gap-1.5 whitespace-nowrap",
        className,
      )}
    >
      <span
        aria-hidden
        className={cn(
          "flex h-[1.2rem] w-[1.2rem] shrink-0 items-center justify-center text-[1.05rem] leading-none",
          iconClassName,
        )}
      >
        <AppIcon name={iconName} className={iconAdjustmentClassName ?? undefined} />
      </span>
      <span
        className={cn(
          "inline-flex h-[1.2rem] min-w-0 items-center overflow-hidden font-mono text-[1.02rem] font-semibold leading-none",
          valueClassName,
        )}
      >
        <AdaptiveDisplayValue
          spec={value.spec}
          className="block min-w-0 max-w-full"
          availableWidthPx={availableWidthPx}
          maxWidthCh={fillAvailableWidth ? undefined : valueWidthBudgetCh}
          data-testid={valueTestId}
          animateDigits
        />
      </span>
    </span>
  );

  const wrappedMetric = (
    <span
      ref={fillAvailableWidth ? widthMeasureRef : undefined}
      data-testid={slotTestId}
      className={cn("min-w-0", fillAvailableWidth ? "block w-full" : "inline-block max-w-full")}
    >
      {modelPerformance && modelPerformanceTitle ? (
        <ModelPerformanceTrigger
          title={modelPerformanceTitle}
          ariaLabel={modelPerformanceAriaLabel}
          performance={modelPerformance}
          className={cn(
            "rounded-md",
            fillAvailableWidth ? `w-full ${triggerAlignmentClassName}` : null,
          )}
        >
          {metric}
        </ModelPerformanceTrigger>
      ) : (
        <Tooltip
          content={
            <span className="font-medium">
              {label}
              <span className="font-mono font-semibold"> {value.fullText}</span>
            </span>
          }
          clickToOpen
          className={cn(
            "rounded-md",
            fillAvailableWidth ? `w-full ${triggerAlignmentClassName}` : null,
          )}
          triggerProps={{
            tabIndex: 0,
            "aria-label": triggerAriaLabel,
          }}
        >
          {metric}
        </Tooltip>
      )}
    </span>
  );

  return wrappedMetric;
}

function NetworkSpeedInline({
  uploadBytesPerSecond,
  downloadBytesPerSecond,
  localeTag,
  uploadLabel,
  downloadLabel,
  testId,
  className,
}: {
  uploadBytesPerSecond: number;
  downloadBytesPerSecond: number;
  localeTag: string;
  uploadLabel: string;
  downloadLabel: string;
  testId?: string;
  className?: string;
}) {
  const uploadValue = formatDashboardNetworkSpeed(uploadBytesPerSecond, localeTag);
  const downloadValue = formatDashboardNetworkSpeed(downloadBytesPerSecond, localeTag);

  return (
    <div
      data-testid={testId}
      className={cn(
        "inline-flex min-w-0 max-w-full flex-wrap items-center gap-x-3 gap-y-1 rounded-full border border-base-300/65 bg-base-100/78 px-2.5 py-1",
        className,
      )}
      aria-label={`${uploadLabel} ${uploadValue}; ${downloadLabel} ${downloadValue}`}
      title={`${uploadLabel}: ${uploadValue} · ${downloadLabel}: ${downloadValue}`}
    >
      <span className="inline-flex min-w-0 items-center gap-1 whitespace-nowrap text-sky-500 dark:text-sky-300">
        <AppIcon name="arrow-up-bold" className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        <span className="font-mono text-[0.82rem] font-semibold leading-none">
          <AnimatedDigits value={uploadValue} />
        </span>
      </span>
      <span className="inline-flex min-w-0 items-center gap-1 whitespace-nowrap text-emerald-500 dark:text-emerald-300">
        <AppIcon name="arrow-down-bold" className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        <span className="font-mono text-[0.82rem] font-semibold leading-none">
          <AnimatedDigits value={downloadValue} />
        </span>
      </span>
    </div>
  );
}

function AccountSegmentList({
  segments,
  className,
  testId,
  showLabel = false,
  showIconWhenLabelHidden = false,
  enableTooltips = true,
}: {
  segments: Array<{
    label: string;
    value: AccountDisplayValue;
    tone: AccountMetricTone;
    iconName?: AppIconName;
  }>;
  className?: string;
  testId?: string;
  showLabel?: boolean;
  showIconWhenLabelHidden?: boolean;
  enableTooltips?: boolean;
}) {
  const renderedSegments = segments.map((segment) => (
    <span
      key={`${segment.label}-${segment.value.fullText}`}
      data-testid="dashboard-upstream-account-segment"
      data-motion-surface
      className={cn(
        "inline-flex items-center whitespace-nowrap rounded-md px-1 py-0.5 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary",
        showLabel ? "gap-1.5" : "gap-1.5",
      )}
    >
      {showLabel ? (
        <>
          {segment.iconName ? (
            <AppIcon
              name={segment.iconName}
              className={cn(
                "h-3.5 w-3.5 shrink-0",
                ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[segment.tone],
              )}
              aria-hidden
            />
          ) : (
            <span
              className={cn(
                "h-1.5 w-1.5 rounded-full",
                ACCOUNT_METRIC_DOT_TONE_CLASSNAMES[segment.tone],
              )}
              aria-hidden="true"
            />
          )}
          <span className="text-[11px] font-semibold leading-none text-base-content/72">
            {segment.label}
          </span>
        </>
      ) : null}
      {!showLabel ? (
        showIconWhenLabelHidden && segment.iconName ? (
          <AppIcon
            name={segment.iconName}
            className={cn(
              "h-3.5 w-3.5 shrink-0",
              ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[segment.tone],
            )}
            aria-hidden
          />
        ) : (
          <span
            className={cn(
              "h-1.5 w-1.5 rounded-full",
              ACCOUNT_METRIC_DOT_TONE_CLASSNAMES[segment.tone],
            )}
            aria-hidden="true"
          />
        )
      ) : null}
      <span
        className={cn(
          "min-w-0 font-mono text-[12px] font-semibold leading-none",
          ACCOUNT_METRIC_VALUE_TONE_CLASSNAMES[segment.tone],
        )}
      >
        <AdaptiveDisplayValue spec={segment.value.spec} className="block min-w-0 max-w-full" />
      </span>
    </span>
  ));

  return (
    <div
      data-testid={testId}
      aria-label={segments
        .map((segment) => `${segment.label} ${segment.value.ariaText}`)
        .join(" · ")}
      className={cn(
        "flex flex-wrap items-center gap-x-3 gap-y-1.5",
        showLabel && "gap-x-4",
        className,
      )}
    >
      {enableTooltips
        ? segments.map((segment, index) => (
            <Tooltip
              key={`${segment.label}-${segment.value.fullText}`}
              content={
                <span className="font-medium">
                  {segment.label}
                  <span className="font-mono font-semibold"> {segment.value.fullText}</span>
                </span>
              }
              clickToOpen
              className="rounded-md"
              triggerProps={{
                tabIndex: 0,
                "aria-label": `${segment.label} ${segment.value.ariaText}`,
              }}
            >
              {renderedSegments[index]}
            </Tooltip>
          ))
        : renderedSegments}
    </div>
  );
}

function AccountRecentInvocationRow({
  invocation,
  locale,
  nowMs,
  onOpenUpstreamAccount,
  onOpenConversation,
  onOpenInvocation,
}: {
  invocation: DashboardWorkingConversationInvocationModel;
  locale: "zh" | "en";
  nowMs: number;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  onOpenConversation?: (selection: DashboardWorkingConversationSelection) => void;
  onOpenInvocation?: (selection: DashboardWorkingConversationInvocationSelection) => void;
}) {
  const { t } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag), [localeTag]);
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
  const statusMeta = resolveStatusMeta(invocation.tone, invocation.displayStatus);
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
  const displayPromptCacheKey = invocation.preview.promptCacheKey?.trim() ?? "";
  const displayConversationSequenceId = displayPromptCacheKey
    ? formatDashboardWorkingConversationSequenceId(
        `WC-${hashDashboardWorkingConversationKey(displayPromptCacheKey).slice(0, 6)}`,
      )
    : "";
  const conversationIdentityToneClassName = displayPromptCacheKey
    ? resolveConversationIdentityToneClassName(displayPromptCacheKey)
    : null;
  const requestModelValue = viewModel.requestModelValue;
  const responseModelValue = viewModel.responseModelValue;
  const compactLatencyValues = useMemo(() => {
    const normalizedStatus = invocation.displayStatus.trim().toLowerCase();
    return {
      firstResponseByteTotalValue: formatCompactLatencySecondsValue(
        resolveFirstResponseByteTotalMs(invocation.record),
        localeTag,
      ),
      responseTimeValue:
        normalizedStatus === "running" || normalizedStatus === "pending"
          ? formatCompactElapsedSecondsFromTimestamp(invocation.record.occurredAt, localeTag, nowMs)
          : formatCompactLatencySecondsValue(invocation.record.tTotalMs, localeTag),
    };
  }, [invocation.displayStatus, invocation.record, localeTag, nowMs]);
  const invocationActionLabel = `${t("dashboard.workingConversations.openInvocation")} · ${invocation.record.invokeId}`;
  const conversationActionLabel = displayPromptCacheKey
    ? `${t("dashboard.workingConversations.openConversation")} · ${displayConversationSequenceId} · ${displayPromptCacheKey}`
    : null;
  const fastIndicator = renderFastIndicator(viewModel.fastIndicatorState, t);

  const handleOpenInvocation = useCallback(() => {
    onOpenInvocation?.({
      slotKind: "current",
      conversationSequenceId: invocation.record.invokeId,
      promptCacheKey:
        invocation.preview.promptCacheKey?.trim() || invocation.record.promptCacheKey?.trim() || "",
      invocation,
    });
  }, [invocation, onOpenInvocation]);

  const handleOpenConversation = useCallback(() => {
    if (!displayPromptCacheKey) return;
    onOpenConversation?.({
      conversationSequenceId: `WC-${hashDashboardWorkingConversationKey(displayPromptCacheKey).slice(0, 6)}`,
      promptCacheKey: displayPromptCacheKey,
    });
  }, [displayPromptCacheKey, onOpenConversation]);

  const handleRowKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLButtonElement>) => {
      if (event.target !== event.currentTarget) return;
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      handleOpenInvocation();
    },
    [handleOpenInvocation],
  );

  const handleIdentityChipClick = useCallback(
    (event: React.MouseEvent<HTMLButtonElement>) => {
      event.stopPropagation();
      handleOpenConversation();
    },
    [handleOpenConversation],
  );

  const handleIdentityChipKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLButtonElement>) => {
      event.stopPropagation();
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      handleOpenConversation();
    },
    [handleOpenConversation],
  );

  return (
    <button
      type="button"
      aria-label={invocationActionLabel}
      data-testid="dashboard-upstream-account-recent-row"
      data-motion-surface
      className={cn(
        "min-w-0 w-full max-w-full rounded-[0.85rem] border bg-base-100/58 px-3.5 py-2.5 text-left transition-colors duration-200 hover:bg-base-100/72 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary",
        ACCOUNT_CARD_INNER_BORDER_CLASS_NAME,
      )}
      onClick={handleOpenInvocation}
      onKeyDown={handleRowKeyDown}
    >
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-1.5">
            <div
              className="flex min-w-0 items-center gap-1.5"
              data-testid="dashboard-upstream-account-recent-identity"
            >
              {displayConversationSequenceId ? (
                <>
                  <button
                    type="button"
                    data-testid="dashboard-upstream-account-recent-identity-chip"
                    className={cn(
                      UPSTREAM_ACCOUNT_RECENT_IDENTITY_CHIP_CLASS_NAME,
                      "cursor-pointer transition-opacity duration-200 hover:opacity-80 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary",
                      conversationIdentityToneClassName,
                    )}
                    aria-label={conversationActionLabel ?? undefined}
                    title={conversationActionLabel ?? displayConversationSequenceId}
                    onClick={handleIdentityChipClick}
                    onKeyDown={handleIdentityChipKeyDown}
                  >
                    <span className="truncate whitespace-nowrap">
                      {displayConversationSequenceId}
                    </span>
                  </button>
                  <AppIcon
                    name="chevron-right"
                    className="h-3 w-3 shrink-0 text-base-content/45"
                    aria-hidden
                  />
                </>
              ) : null}
              <span
                className="truncate font-mono text-[12px] font-semibold text-base-content/88"
                title={invocation.record.invokeId}
              >
                {invocation.record.invokeId}
              </span>
            </div>
            {invocation.livePhase ? (
              <InvocationPhaseBadge
                phase={invocation.livePhase}
                appearance="inline"
                motion="dynamic"
                showLabel={false}
              />
            ) : (
              <InlineInvocationStatus
                meta={statusMeta}
                label={statusLabel}
                showLabel={false}
                detail={viewModel.collapsedErrorSummary}
              />
            )}
            {renderInvocationTransportBadge(
              invocation.record,
              "min-h-5 border-[rgba(148,163,184,0.24)] bg-primary/8 px-2 py-0.5 text-[9.5px]",
            )}
            {renderEndpointSummary(
              viewModel.endpointDisplay,
              t,
              UPSTREAM_ACCOUNT_RECENT_COMPACT_BADGE_CLASS_NAME,
            )}
            <DashboardImageToolIconBadge
              endpointDisplay={viewModel.endpointDisplay}
              imageIntentDisplay={viewModel.imageIntentDisplay}
              t={t}
            />
            {fastIndicator}
            <CompactLatencyPills
              firstResponseByteTotalValue={compactLatencyValues.firstResponseByteTotalValue}
              responseTimeValue={compactLatencyValues.responseTimeValue}
              t={t}
            />
          </div>
          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-[11px] leading-[1.45] text-base-content/72">
            <span>{occurredAtShortLabel}</span>
            <span className="text-base-content/28">·</span>
            <span className="min-w-0">
              {renderUpstreamAccountRecentModelDisplay(
                viewModel.modelHasMismatch,
                viewModel.modelValue,
                requestModelValue,
                responseModelValue,
                t,
              )}
            </span>
            {viewModel.reasoningEffortValue !== FALLBACK_CELL ? (
              <>
                <span className="text-base-content/28">·</span>
                <CompactReasoningEffortBadge value={viewModel.reasoningEffortValue} />
              </>
            ) : null}
          </div>
        </div>
        <div className="text-right">
          <div className="font-mono text-[12px] font-semibold text-base-content/88">
            {viewModel.totalTokensValue}
          </div>
          <div className="text-[10.5px] text-base-content/62">{compactCostValue}</div>
        </div>
      </div>
      <div className="mt-1.5 flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 font-mono text-[10.5px] leading-[1.45] text-base-content/74">
        <span>IN {viewModel.inputTokensValue}</span>
        <span className="text-base-content/28">·</span>
        <span
          title="Cache write tokens"
          aria-label={`Cache write tokens: ${viewModel.cacheWriteTokensValue}`}
        >
          CW {viewModel.cacheWriteTokensValue}
        </span>
        <span className="text-base-content/28">·</span>
        <span
          title="Cache read tokens"
          aria-label={`Cache read tokens: ${viewModel.cacheInputTokensValue}`}
        >
          C {viewModel.cacheInputTokensValue}
        </span>
        <span className="text-base-content/28">·</span>
        <span>O {viewModel.outputTokensValue}</span>
        <span className="text-base-content/28">·</span>
        <span>T {viewModel.totalTokensValue}</span>
      </div>
      {viewModel.collapsedErrorSummary ? (
        <InvocationErrorSummary
          className="mt-1 max-w-full"
          textClassName="text-[10px] text-error"
          message={viewModel.collapsedErrorSummary}
        />
      ) : null}
    </button>
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
      <span className="pt-[1px] text-[8.5px] font-semibold uppercase tracking-[0.1em] text-base-content/48">
        {label}
      </span>
      <div
        className={cn(
          "min-w-0 font-mono text-[9.5px] font-semibold leading-[1.35] text-base-content/86",
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
          <div key={index} className="working-conversation-placeholder-line h-3 rounded-[0.5rem]" />
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
  interactionsDisabled = false,
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
  interactionsDisabled?: boolean;
  onOpenUpstreamAccount?: (accountId: number, accountLabel: string) => void;
  onOpenInvocation?: (selection: DashboardWorkingConversationInvocationSelection) => void;
}) {
  const { t } = useTranslation();
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag), [localeTag]);
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
      if (interactionsDisabled || !accountClickable || accountId == null) {
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

  const statusMeta = resolveStatusMeta(invocation.tone, invocation.displayStatus);
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
  const compactCostValue = viewModel.costValue.startsWith("US$")
    ? `$${viewModel.costValue.slice(3)}`
    : viewModel.costValue;
  const compactLatencyValues = useMemo(() => {
    const normalizedStatus = invocation.displayStatus.trim().toLowerCase();
    return {
      firstResponseByteTotalValue: formatCompactLatencySecondsValue(
        resolveFirstResponseByteTotalMs(invocation.record),
        localeTag,
      ),
      responseTimeValue:
        normalizedStatus === "running" || normalizedStatus === "pending"
          ? formatCompactElapsedSecondsFromTimestamp(invocation.record.occurredAt, localeTag, nowMs)
          : formatCompactLatencySecondsValue(invocation.record.tTotalMs, localeTag),
    };
  }, [invocation.displayStatus, invocation.record, localeTag, nowMs]);
  const invocationActionLabel = `${t("dashboard.workingConversations.openInvocation")} · ${label} · ${displayConversationSequenceId} · ${invocation.record.invokeId}`;

  const handleOpenInvocation = useCallback(() => {
    if (interactionsDisabled) return;
    onOpenInvocation?.({
      slotKind,
      conversationSequenceId,
      promptCacheKey,
      invocation,
    });
  }, [
    conversationSequenceId,
    interactionsDisabled,
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
      role={interactionsDisabled ? undefined : "button"}
      tabIndex={interactionsDisabled ? undefined : 0}
      aria-label={interactionsDisabled ? undefined : invocationActionLabel}
      data-testid="dashboard-working-conversation-slot"
      data-slot-kind={slotKind}
      className={cn(
        SLOT_CLASS_NAME,
        statusMeta.slotSurfaceClassName,
        interactionsDisabled
          ? "transition-colors duration-200"
          : "cursor-pointer transition-colors duration-200 hover:bg-base-100/10 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary",
      )}
      onClick={interactionsDisabled ? undefined : handleOpenInvocation}
      onKeyDown={interactionsDisabled ? undefined : handleSlotKeyDown}
    >
      <div
        data-testid="dashboard-working-conversation-slot-header"
        className="grid min-h-5 min-w-0 grid-cols-[auto_minmax(0,1fr)] items-center gap-x-2 gap-y-1"
      >
        <div className="flex min-w-0 shrink-0 items-center gap-1.5">
          <div
            data-testid="dashboard-working-conversation-slot-label"
            className="shrink-0 text-[10px] font-semibold uppercase tracking-[0.08em] text-base-content/62"
          >
            {label}
          </div>
          <div
            data-testid="dashboard-working-conversation-slot-time"
            className="shrink-0 font-mono text-[10px] text-base-content/72"
          >
            {occurredAtShortLabel}
          </div>
        </div>
        <div
          data-testid="dashboard-working-conversation-slot-readings"
          className="flex min-w-0 flex-nowrap items-center justify-end gap-1"
        >
          <div className="flex min-w-0 shrink items-center justify-end gap-1">
            {invocation.livePhase ? (
              <InvocationPhaseBadge
                phase={invocation.livePhase}
                appearance="inline"
                motion="dynamic"
                showLabel={false}
              />
            ) : (
              <InlineInvocationStatus
                meta={statusMeta}
                label={statusLabel}
                showLabel={false}
                detail={viewModel.collapsedErrorSummary}
              />
            )}
            {renderInvocationTransportBadge(
              invocation.record,
              "h-5 border-primary/45 bg-primary/10 px-1.5 text-[9.5px]",
            )}
            <div className="flex h-5 shrink items-center">
              <div className="flex items-center gap-1">
                {renderEndpointSummary(
                  viewModel.endpointDisplay,
                  t,
                  "h-5 rounded-full border-transparent bg-base-100/10 px-1 py-0 text-[9px] font-semibold leading-none text-base-content/76 shadow-none",
                )}
                <DashboardImageToolIconBadge
                  endpointDisplay={viewModel.endpointDisplay}
                  imageIntentDisplay={viewModel.imageIntentDisplay}
                  t={t}
                />
              </div>
            </div>
          </div>
          <CompactLatencyPills
            firstResponseByteTotalValue={compactLatencyValues.firstResponseByteTotalValue}
            responseTimeValue={compactLatencyValues.responseTimeValue}
            t={t}
            className="shrink-0 flex-nowrap gap-1 text-[11px]"
          />
        </div>
      </div>

      <div className="mt-1.5 space-y-1">
        <InvocationMetaLine
          label={lineLabels.account}
          value={
            <div
              data-testid="dashboard-working-conversation-account-line"
              className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0.5 text-[9.5px] leading-[1.3] text-base-content sm:flex-nowrap"
            >
              <div className="flex min-w-[7rem] max-w-full flex-1 items-baseline gap-1.5 font-mono font-semibold">
                {viewModel.accountClickable && viewModel.accountId != null ? (
                  interactionsDisabled ? (
                    <span
                      data-testid="dashboard-working-conversation-account-chip"
                      className={cn(
                        "inline-flex min-w-0 max-w-full items-baseline font-mono text-[9.5px] font-semibold text-base-content",
                        viewModel.accountRoutingInProgress &&
                          INVOCATION_ACCOUNT_ROUTING_IN_PROGRESS_CLASS_NAME,
                      )}
                      title={viewModel.accountLabel}
                    >
                      <span
                        data-testid="dashboard-working-conversation-account-name"
                        className="block min-w-0 truncate whitespace-nowrap text-left"
                      >
                        {viewModel.accountLabel}
                      </span>
                    </span>
                  ) : (
                    <button
                      type="button"
                      data-testid="dashboard-working-conversation-account-chip"
                      className={cn(
                        "inline-flex min-w-0 max-w-full cursor-pointer appearance-none items-baseline border-0 bg-transparent p-0 text-left font-mono text-[9.5px] font-semibold text-base-content no-underline transition-colors duration-200 hover:text-primary focus-visible:rounded-[0.2rem] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary",
                        viewModel.accountRoutingInProgress &&
                          INVOCATION_ACCOUNT_ROUTING_IN_PROGRESS_CLASS_NAME,
                      )}
                      onClick={(event) => {
                        event.stopPropagation();
                        onOpenUpstreamAccount?.(viewModel.accountId ?? 0, viewModel.accountLabel);
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
                  )
                ) : (
                  <span
                    data-testid="dashboard-working-conversation-account-chip"
                    className={cn(
                      "inline-flex min-w-0 max-w-full items-baseline",
                      viewModel.accountRoutingInProgress &&
                        INVOCATION_ACCOUNT_ROUTING_IN_PROGRESS_CLASS_NAME,
                    )}
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
                <span data-testid="dashboard-working-conversation-model-name" className="min-w-0">
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
                <CompactReasoningEffortBadge value={viewModel.reasoningEffortValue} />
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
          title={`${t("table.column.inputTokens")}: ${viewModel.inputTokensValue} · Cache write: ${viewModel.cacheWriteTokensValue} · ${t("table.column.cacheInputTokens")}: ${viewModel.cacheInputTokensValue} · ${t("table.column.outputTokens")}: ${viewModel.outputTokensValue} · ${t("table.column.totalTokens")}: ${viewModel.totalTokensValue} · ${t("table.column.costUsd")}: ${viewModel.costValue} · ${t("table.details.reasoningTokens")}: ${viewModel.reasoningTokensValue}`}
          value={
            <div className="flex min-w-0 flex-wrap items-center gap-x-1 gap-y-0.5">
              <span>IN {viewModel.inputTokensValue}</span>
              <span className="text-base-content/28">·</span>
              <span
                title="Cache write tokens"
                aria-label={`Cache write tokens: ${viewModel.cacheWriteTokensValue}`}
              >
                CW {viewModel.cacheWriteTokensValue}
              </span>
              <span className="text-base-content/28">·</span>
              <span
                title="Cache read tokens"
                aria-label={`Cache read tokens: ${viewModel.cacheInputTokensValue}`}
              >
                C {viewModel.cacheInputTokensValue}
              </span>
              <span className="text-base-content/28">·</span>
              <span>O {viewModel.outputTokensValue}</span>
              <span className="text-base-content/28">·</span>
              <span>T {viewModel.totalTokensValue}</span>
              <span className="text-base-content/28">·</span>
              <span>{compactCostValue}</span>
            </div>
          }
        />

        {viewModel.collapsedErrorSummary ? (
          <InvocationMetaLine
            label={lineLabels.error}
            value={
              <InvocationErrorSummary
                className="max-w-full"
                textClassName="text-[9.5px] text-error"
                message={viewModel.collapsedErrorSummary}
              />
            }
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

function resolveDashboardWorkingConversationCssColumnCount(container: HTMLDivElement | null) {
  if (!container || typeof window === "undefined") return null;
  const template = window.getComputedStyle(container).gridTemplateColumns.trim();
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
  recentPreviewLimit,
  onOpenUpstreamAccount,
  onOpenConversation,
  onOpenInvocation,
  onPolicyChanged,
  recentLoading = false,
  recentError,
  onRetryRecent,
}: {
  account: UpstreamAccountActivityAccount;
  locale: "zh" | "en";
  localeTag: string;
  nowMs: number;
  recentPreviewLimit: number;
  onOpenUpstreamAccount?: (
    accountId: number,
    accountLabel: string,
    options?: DashboardOpenUpstreamAccountOptions,
  ) => void;
  onOpenConversation?: (selection: DashboardWorkingConversationSelection) => void;
  onOpenInvocation?: (selection: DashboardWorkingConversationInvocationSelection) => void;
  onPolicyChanged?: () => void;
  recentLoading?: boolean;
  recentError?: string | null;
  onRetryRecent?: () => void;
}) {
  const { t } = useTranslation();
  const cardRef = useRef<HTMLElement | null>(null);
  const [cardWidth, setCardWidth] = useState(0);
  const serverPolicyDraft = useMemo(() => accountPolicyDraftFromRule(account), [account]);
  const [policyDraft, setPolicyDraft] = useState<AccountQuickPolicyDraft>(serverPolicyDraft);
  const [policySaveError, setPolicySaveError] = useState<string | null>(null);
  const [isSavingPolicy, setIsSavingPolicy] = useState(false);
  const debounceTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingPatchRef = useRef<UpdateGroupAccountRoutingRulePayload | null>(null);
  const pendingDraftRef = useRef<AccountQuickPolicyDraft | null>(null);
  const lastCommittedPolicyRef = useRef<AccountQuickPolicyDraft>(serverPolicyDraft);
  const mountedRef = useRef(true);
  const flushPolicySaveRef = useRef<((updateUi?: boolean) => Promise<void>) | null>(null);
  const saveSeqRef = useRef(0);
  const recentInvocations = useMemo(
    () =>
      account.recentInvocations.map(
        (preview: DashboardWorkingConversationInvocationModel["preview"]) =>
          buildDashboardWorkingConversationInvocationModel(preview),
      ),
    [account.recentInvocations],
  );
  const handleOpenHealthEventsTab = useCallback(() => {
    if (account.upstreamAccountId == null) return;
    onOpenUpstreamAccount?.(account.upstreamAccountId, account.displayName, {
      tab: "healthEvents",
    });
  }, [account.displayName, account.upstreamAccountId, onOpenUpstreamAccount]);
  useLayoutEffect(() => {
    const card = cardRef.current;
    if (!card) return undefined;

    const updateCardWidth = () => {
      const nextWidth = card.clientWidth;
      setCardWidth((current) => (Math.abs(current - nextWidth) > 0.5 ? nextWidth : current));
    };

    updateCardWidth();
    const frame = window.requestAnimationFrame(updateCardWidth);
    window.addEventListener("resize", updateCardWidth);

    if (typeof ResizeObserver === "undefined") {
      return () => {
        window.cancelAnimationFrame(frame);
        window.removeEventListener("resize", updateCardWidth);
      };
    }

    const observer = new ResizeObserver(updateCardWidth);
    observer.observe(card);

    return () => {
      window.cancelAnimationFrame(frame);
      window.removeEventListener("resize", updateCardWidth);
      observer.disconnect();
    };
  }, []);
  useEffect(() => {
    lastCommittedPolicyRef.current = serverPolicyDraft;
    if (!debounceTimerRef.current && !isSavingPolicy) {
      setPolicyDraft(serverPolicyDraft);
    }
  }, [isSavingPolicy, serverPolicyDraft]);
  const flushPolicySave = useCallback(
    async (updateUi = true) => {
      const accountId = account.upstreamAccountId;
      const patch = pendingPatchRef.current;
      const nextDraft = pendingDraftRef.current;
      pendingPatchRef.current = null;
      pendingDraftRef.current = null;
      debounceTimerRef.current = null;
      if (accountId == null || !patch || !nextDraft) return;
      const seq = saveSeqRef.current + 1;
      saveSeqRef.current = seq;
      const rollbackDraft = lastCommittedPolicyRef.current;
      if (updateUi && mountedRef.current) {
        setIsSavingPolicy(true);
      }
      try {
        await updateUpstreamAccount(accountId, { routingRule: patch });
        if (saveSeqRef.current !== seq) return;
        lastCommittedPolicyRef.current = nextDraft;
        if (updateUi && mountedRef.current) {
          setPolicySaveError(null);
        }
        emitUpstreamAccountsChanged();
        if (mountedRef.current) {
          onPolicyChanged?.();
        }
      } catch (err) {
        if (saveSeqRef.current !== seq) return;
        if (updateUi && mountedRef.current && !pendingPatchRef.current) {
          setPolicyDraft(rollbackDraft);
        }
        if (updateUi && mountedRef.current) {
          setPolicySaveError(err instanceof Error ? err.message : String(err));
        }
      } finally {
        if (updateUi && mountedRef.current && saveSeqRef.current === seq) {
          setIsSavingPolicy(false);
        }
      }
    },
    [account.upstreamAccountId, onPolicyChanged],
  );
  flushPolicySaveRef.current = flushPolicySave;
  useEffect(
    () => () => {
      mountedRef.current = false;
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current);
        void flushPolicySaveRef.current?.(false);
      }
    },
    [],
  );
  const schedulePolicySave = useCallback(
    (nextDraft: AccountQuickPolicyDraft, patch: UpdateGroupAccountRoutingRulePayload) => {
      if (account.upstreamAccountId == null) return;
      setPolicyDraft(nextDraft);
      setPolicySaveError(null);
      pendingPatchRef.current = {
        ...(pendingPatchRef.current ?? {}),
        ...patch,
      };
      pendingDraftRef.current = nextDraft;
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current);
      }
      debounceTimerRef.current = setTimeout(() => {
        void flushPolicySave();
      }, 1000);
    },
    [account.upstreamAccountId, flushPolicySave],
  );
  const handleCyclePriorityPolicy = useCallback(() => {
    const nextDraft = cycleAccountPriorityPolicy(policyDraft);
    schedulePolicySave(nextDraft, {
      priorityTier: nextDraft.priorityTier,
    });
  }, [policyDraft, schedulePolicySave]);
  const handleToggleCutOut = useCallback(() => {
    const nextDraft = {
      ...policyDraft,
      allowCutOut: !policyDraft.allowCutOut,
    };
    schedulePolicySave(nextDraft, { allowCutOut: nextDraft.allowCutOut });
  }, [policyDraft, schedulePolicySave]);
  const handleToggleCutIn = useCallback(() => {
    const nextDraft = {
      ...policyDraft,
      allowCutIn: !policyDraft.allowCutIn,
    };
    schedulePolicySave(nextDraft, { allowCutIn: nextDraft.allowCutIn });
  }, [policyDraft, schedulePolicySave]);
  const handleCycleFastModePolicy = useCallback(() => {
    const nextDraft = cycleAccountFastModePolicy(policyDraft);
    schedulePolicySave(nextDraft, {
      fastModeRewriteMode: nextDraft.fastModeRewriteMode,
    });
  }, [policyDraft, schedulePolicySave]);
  const requestSummarySegments = useMemo(
    () => [
      {
        label: locale === "zh" ? "成功" : "Success",
        value: buildAccountNumberDisplayValue(account.successCount, localeTag, 0),
        tone: "success" as const,
      },
      {
        label: locale === "zh" ? "失败" : "Failure",
        value: buildAccountNumberDisplayValue(account.failureCount, localeTag, 0),
        tone: "error" as const,
      },
      {
        label: locale === "zh" ? "其他" : "Other",
        value: buildAccountNumberDisplayValue(
          Math.max(0, account.nonSuccessCount - account.failureCount),
          localeTag,
          0,
        ),
        tone: "warning" as const,
      },
    ],
    [account.failureCount, account.nonSuccessCount, account.successCount, locale, localeTag],
  );
  const costSummarySegments = useMemo(() => {
    const failureCostShare = accountCostShare(account.failureCost, account.totalCost);
    return [
      {
        label: locale === "zh" ? "失败" : "Failure",
        value: buildAccountCurrencyDisplayValue(account.failureCost, localeTag, 2),
        tone: "error" as const,
      },
      {
        label: locale === "zh" ? "失败成本比率" : "Failure cost ratio",
        value: buildAccountPercentDisplayValue(failureCostShare, localeTag),
        tone: "error" as const,
      },
    ];
  }, [account.failureCost, account.totalCost, locale, localeTag]);
  const tokenSummarySegments = useMemo(
    () => [
      {
        label: locale === "zh" ? "缓存命中率" : "Cache hit",
        value: buildAccountPercentDisplayValue(account.cacheHitRate, localeTag),
        tone: "secondary" as const,
      },
      {
        label: locale === "zh" ? "失败" : "Failure",
        value: buildAccountNumberDisplayValue(account.failureTokens, localeTag, 0),
        tone: "error" as const,
      },
    ],
    [account.cacheHitRate, account.failureTokens, locale, localeTag],
  );
  const recentBridgeSegments = useMemo(() => {
    const segments = [];
    if (account.failureCount > 0) {
      segments.push({
        label: locale === "zh" ? "失败" : "Failure",
        value: buildAccountNumberDisplayValue(account.failureCount, localeTag, 0),
        tone: "error" as const,
        iconName: "alert-circle-outline" as const,
      });
    }
    if (account.nonSuccessCount > account.failureCount) {
      segments.push({
        label: locale === "zh" ? "非成功" : "Non-success",
        value: buildAccountNumberDisplayValue(
          account.nonSuccessCount - account.failureCount,
          localeTag,
          0,
        ),
        tone: "warning" as const,
        iconName: "alert-outline" as const,
      });
    }
    if (account.successCount > 0) {
      segments.push({
        label: locale === "zh" ? "成功" : "Success",
        value: buildAccountNumberDisplayValue(account.successCount, localeTag, 0),
        tone: "success" as const,
        iconName: "check-circle-outline" as const,
      });
    }
    return segments;
  }, [account.failureCount, account.nonSuccessCount, account.successCount, locale, localeTag]);
  const currentFirstByteDisplayValue = buildAccountDurationDisplayValue(
    account.currentFirstResponseByteTotalAvgMs,
    localeTag,
  );
  const currentAvgTotalDisplayValue = buildAccountDurationDisplayValue(
    account.currentAvgTotalMs,
    localeTag,
  );
  const rangeFirstByteValue = formatAccountDurationValue(
    account.firstResponseByteTotalAvgMs ?? account.firstByteAvgMs,
    localeTag,
  );
  const rangeAvgTotalValue = formatAccountDurationValue(account.avgTotalMs, localeTag);
  const totalRequestDisplayValue = buildAccountNumberDisplayValue(
    account.requestCount,
    localeTag,
    0,
  );
  const totalCostDisplayValue = buildAccountCurrencyAmountDisplayValue(
    account.totalCost,
    localeTag,
    2,
  );
  const totalTokenDisplayValue = buildAccountNumberDisplayValue(account.totalTokens, localeTag, 0);
  const currentFirstByteValue = currentFirstByteDisplayValue.fullText;
  const currentAvgTotalValue = currentAvgTotalDisplayValue.fullText;
  const totalRequestValue = totalRequestDisplayValue.fullText;
  const totalCostValue = totalCostDisplayValue.fullText;
  const totalTokenValue = totalTokenDisplayValue.fullText;
  const usageDetailsLabel = t("dashboard.usageBreakdown.title");
  const usageBreakdownLabels =
    locale === "zh"
      ? {
          total: "总计",
          cacheWrite: "缓存写入",
          cacheRead: "缓存读取",
          cacheHitTokens: "缓存读取",
          cacheHitRate: "缓存命中率",
          output: "输出",
          model: "模型",
          input: "输入",
          reasoning: "推理",
          unknown: "未知",
          unavailable: "成本分项未提供",
          tokenUnavailable: "Token 分项未提供",
          unknownModel: "未标识模型",
          reasoningEffort: "思考等级",
          unspecifiedEffort: "未指定",
          effortNone: "无",
          effortMinimal: "最小",
          effortLow: "低",
          effortMedium: "中",
          effortHigh: "高",
          effortXhigh: "极高",
        }
      : {
          total: "Total",
          cacheWrite: "Cache write",
          cacheRead: "Cache read",
          cacheHitTokens: "Cache read",
          cacheHitRate: "Cache hit rate",
          output: "Output",
          model: "Model",
          input: "Input",
          reasoning: "Reasoning",
          unknown: "Unknown",
          unavailable: "Cost breakdown unavailable",
          tokenUnavailable: "Token breakdown unavailable",
          unknownModel: "Unidentified model",
          reasoningEffort: "Reasoning effort",
          unspecifiedEffort: "Unspecified",
          effortNone: "None",
          effortMinimal: "Minimal",
          effortLow: "Low",
          effortMedium: "Medium",
          effortHigh: "High",
          effortXhigh: "XHigh",
        };
  const formatBreakdownNumber = (value: number) => formatAccountNumberValue(value, localeTag, 0);
  const formatBreakdownRatio = (value: number | null) =>
    value == null ? FALLBACK_CELL : formatAccountPercentValue(value, localeTag);
  const formatBreakdownCurrency = (value: number) =>
    formatAccountCurrencyValue(value, localeTag, 4);
  const latencyDetailSections = useMemo<AccountMetricDetailSection[]>(() => {
    const currentFirstByteMs = finiteNumber(account.currentFirstResponseByteTotalAvgMs);
    const firstByteMs = finiteNumber(account.firstResponseByteTotalAvgMs ?? account.firstByteAvgMs);
    const stageFirstByteMs = finiteNumber(account.firstByteAvgMs);
    const currentAvgTotalMs = finiteNumber(account.currentAvgTotalMs);
    const relatedRows: AccountMetricDetailRow[] = [];

    if (
      stageFirstByteMs != null &&
      firstByteMs != null &&
      Math.abs(stageFirstByteMs - firstByteMs) >= 0.5
    ) {
      relatedRows.push({
        label: locale === "zh" ? "阶段首字节" : "Stage first byte",
        value: formatAccountDurationValue(stageFirstByteMs, localeTag),
        tone: "secondary",
      });
    }
    if (
      currentFirstByteMs != null &&
      currentAvgTotalMs != null &&
      currentAvgTotalMs > 0 &&
      currentFirstByteMs <= currentAvgTotalMs
    ) {
      relatedRows.push({
        label: locale === "zh" ? "首字占比" : "First-byte share",
        value: formatAccountPercentValue(currentFirstByteMs / currentAvgTotalMs, localeTag),
        tone: "secondary",
      });
    }

    return [
      {
        title: locale === "zh" ? "当前显示值" : "Current display",
        rows: [
          {
            label: t("dashboard.today.firstResponseTime"),
            value: currentFirstByteValue,
            tone: currentFirstByteValue === FALLBACK_CELL ? "neutral" : "secondary",
          },
          {
            label: t("dashboard.today.responseTime"),
            value: currentAvgTotalValue,
            tone: currentAvgTotalValue === FALLBACK_CELL ? "neutral" : "primary",
          },
        ],
      },
      {
        title: locale === "zh" ? "当前范围统计" : "Current range stats",
        rows: [
          {
            label: t("dashboard.today.firstResponseTime"),
            value: rangeFirstByteValue,
            tone: rangeFirstByteValue === FALLBACK_CELL ? "neutral" : "secondary",
          },
          {
            label: t("dashboard.today.responseTime"),
            value: rangeAvgTotalValue,
            tone: rangeAvgTotalValue === FALLBACK_CELL ? "neutral" : "primary",
          },
        ],
      },
      ...(relatedRows.length > 0
        ? [
            {
              title: locale === "zh" ? "相关数据" : "Related data",
              rows: relatedRows,
            },
          ]
        : []),
    ];
  }, [
    account.avgTotalMs,
    account.currentAvgTotalMs,
    account.currentFirstResponseByteTotalAvgMs,
    account.firstByteAvgMs,
    account.firstResponseByteTotalAvgMs,
    currentAvgTotalValue,
    currentFirstByteValue,
    locale,
    localeTag,
    rangeAvgTotalValue,
    rangeFirstByteValue,
    t,
  ]);
  const requestDetailSections = useMemo<AccountMetricDetailSection[]>(() => {
    const otherCount = Math.max(0, account.nonSuccessCount - account.failureCount);
    const successRate =
      account.requestCount > 0 ? account.successCount / account.requestCount : null;
    const nonSuccessRate =
      account.requestCount > 0 ? account.nonSuccessCount / account.requestCount : null;

    return [
      {
        title: locale === "zh" ? "当前字段" : "Current fields",
        rows: [
          {
            label: locale === "zh" ? "请求数" : "Requests",
            value: totalRequestValue,
            tone: "neutral" as const,
          },
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
            label: locale === "zh" ? "其他" : "Other",
            value: formatAccountNumberValue(otherCount, localeTag, 0),
            tone: "warning" as const,
          },
        ],
      },
      {
        title: locale === "zh" ? "相关数据" : "Related data",
        rows: [
          {
            label: locale === "zh" ? "成功率" : "Success rate",
            value: formatAccountPercentValue(successRate, localeTag),
            tone: "success" as const,
          },
          {
            label: locale === "zh" ? "非成功率" : "Non-success rate",
            value: formatAccountPercentValue(nonSuccessRate, localeTag),
            tone: "warning" as const,
          },
        ],
      },
    ];
  }, [
    account.failureCount,
    account.nonSuccessCount,
    account.requestCount,
    account.successCount,
    locale,
    localeTag,
    totalRequestValue,
  ]);
  const costDetailSections = useMemo<AccountMetricDetailSection[]>(() => {
    const failureCostShare = accountCostShare(account.failureCost, account.totalCost);
    const nonFailureCost = account.totalCost - account.failureCost;
    const averageCost = account.requestCount > 0 ? account.totalCost / account.requestCount : null;

    return [
      {
        title: locale === "zh" ? "当前字段" : "Current fields",
        rows: [
          {
            label: locale === "zh" ? "成本" : "Cost",
            value: totalCostValue,
            tone: "warning" as const,
          },
          {
            label: locale === "zh" ? "失败成本" : "Failure cost",
            value: formatAccountCurrencyValue(account.failureCost, localeTag, 2),
            tone: "error" as const,
          },
          {
            label: locale === "zh" ? "失败成本比率" : "Failure cost ratio",
            value: formatAccountPercentValue(failureCostShare, localeTag),
            tone: "error" as const,
          },
        ],
      },
      {
        title: locale === "zh" ? "相关数据" : "Related data",
        rows: [
          {
            label: locale === "zh" ? "成功/其他成本" : "Success/other cost",
            value: formatAccountCurrencyValue(nonFailureCost, localeTag, 2),
            tone: "warning" as const,
          },
          {
            label: locale === "zh" ? "单次均价" : "Average per request",
            value: formatAccountCurrencyValue(averageCost, localeTag, 4),
            tone: "warning" as const,
          },
        ],
      },
    ];
  }, [
    account.failureCost,
    account.requestCount,
    account.totalCost,
    locale,
    localeTag,
    totalCostValue,
  ]);
  const tokenDetailSections = useMemo<AccountMetricDetailSection[]>(() => {
    const averageTokens =
      account.requestCount > 0 ? account.totalTokens / account.requestCount : null;

    return [
      {
        title: locale === "zh" ? "当前字段" : "Current fields",
        rows: [
          {
            label: "Token",
            value: totalTokenValue,
            tone: "success" as const,
          },
          {
            label: locale === "zh" ? "缓存命中率" : "Cache hit",
            value: formatAccountPercentValue(account.cacheHitRate, localeTag),
            tone: "secondary" as const,
          },
          {
            label: locale === "zh" ? "失败 Token" : "Failure tokens",
            value: formatAccountNumberValue(account.failureTokens, localeTag, 0),
            tone: "error" as const,
          },
        ],
      },
      {
        title: locale === "zh" ? "相关数据" : "Related data",
        rows: [
          {
            label: locale === "zh" ? "成功 Token" : "Success tokens",
            value: formatAccountNumberValue(account.successTokens, localeTag, 0),
            tone: "success" as const,
          },
          {
            label: locale === "zh" ? "非成功 Token" : "Non-success tokens",
            value: formatAccountNumberValue(account.nonSuccessTokens, localeTag, 0),
            tone: "warning" as const,
          },
          {
            label: locale === "zh" ? "单请求 Token" : "Tokens per request",
            value: formatAccountNumberValue(averageTokens, localeTag, 1),
            tone: "success" as const,
          },
        ],
      },
    ];
  }, [
    account.cacheHitRate,
    account.failureTokens,
    account.nonSuccessTokens,
    account.requestCount,
    account.successTokens,
    account.totalTokens,
    locale,
    localeTag,
    totalTokenValue,
  ]);
  const inProgressDisplayValue = buildAccountNumberDisplayValue(
    account.inProgressInvocationCount,
    localeTag,
    0,
  );
  const tokensPerMinuteDisplayValue = buildAccountNumberDisplayValue(
    account.tokensPerMinute,
    localeTag,
    0,
  );
  const spendRateDisplayValue = buildAccountCurrencyAmountDisplayValue(
    account.spendRate,
    localeTag,
    2,
  );
  const modelPerformanceTitle = `${account.displayName} · ${t("dashboard.modelPerformance.title")}`;
  const headerLayout =
    cardWidth > 0 && cardWidth < ACCOUNT_CARD_STACKED_HEADER_BREAKPOINT_PX ? "stacked" : "split";
  const inlineMetricLayout = headerLayout === "stacked" ? "three-columns" : "inline";
  const heroMetricColumnCount =
    cardWidth > 0 && cardWidth < ACCOUNT_CARD_HERO_SINGLE_COLUMN_BREAKPOINT_PX
      ? 1
      : cardWidth > 0 && cardWidth < ACCOUNT_CARD_HERO_TWO_COLUMN_BREAKPOINT_PX
        ? 2
        : 4;
  const recentBreakdownLayout =
    cardWidth > 0 && cardWidth < ACCOUNT_CARD_RECENT_STACK_BREAKPOINT_PX ? "stacked" : "inline";
  return (
    <article
      ref={cardRef}
      data-testid="dashboard-upstream-account-card"
      data-account-key={account.accountKey ?? account.upstreamAccountId ?? "unassigned"}
      data-header-layout={headerLayout}
      data-inline-metric-layout={inlineMetricLayout}
      data-metric-columns={String(heroMetricColumnCount)}
      data-recent-breakdown-layout={recentBreakdownLayout}
      className={ACCOUNT_CARD_CLASS_NAME}
    >
      <div className="flex flex-col gap-2">
        <div
          data-testid="dashboard-upstream-account-header-row"
          className={cn(
            "grid items-start gap-x-4 gap-y-2",
            headerLayout === "stacked" ? "grid-cols-1" : "grid-cols-[minmax(0,1fr)_auto]",
          )}
        >
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <button
              type="button"
              data-motion-surface
              disabled={account.upstreamAccountId == null}
              className={cn(
                "inline-flex min-h-11 min-w-0 max-w-full appearance-none items-center border-0 bg-transparent py-1 text-left text-[1rem] font-semibold text-base-content transition-opacity duration-200 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary sm:min-h-8 sm:py-0",
                account.upstreamAccountId == null
                  ? "cursor-default"
                  : "cursor-pointer hover:opacity-80",
              )}
              onClick={() => {
                if (account.upstreamAccountId == null) return;
                onOpenUpstreamAccount?.(account.upstreamAccountId, account.displayName);
              }}
            >
              <span className="truncate">{account.displayName}</span>
            </button>
            <AccountAttentionBadges
              account={account}
              locale={locale}
              clickable={account.upstreamAccountId != null}
              onClick={handleOpenHealthEventsTab}
            />
            {shouldShowUpstreamPlanBadge(account.planType) ? (
              <Badge
                variant={upstreamPlanBadgeRecipe(account.planType)?.variant ?? "secondary"}
                data-plan={upstreamPlanBadgeRecipe(account.planType)?.dataPlan}
                className={cn(
                  ACCOUNT_HEADER_BADGE_CLASS_NAME,
                  upstreamPlanBadgeRecipe(account.planType)?.className,
                )}
              >
                {compactUpstreamPlanLabel(account.planType)}
              </Badge>
            ) : null}
            <AccountQuickPolicyChips
              draft={policyDraft}
              locale={locale}
              disabled={account.upstreamAccountId == null}
              isSaving={isSavingPolicy || debounceTimerRef.current != null}
              onCyclePriority={handleCyclePriorityPolicy}
              onCycleFastMode={handleCycleFastModePolicy}
              onToggleCutOut={handleToggleCutOut}
              onToggleCutIn={handleToggleCutIn}
            />
          </div>
          {headerLayout === "stacked" ? (
            <div className="flex w-full min-w-0 flex-col gap-y-1.5 text-right self-start">
              <div className="grid w-full min-w-0 grid-cols-3 items-center gap-x-3">
                <div className="min-w-0">
                  <AccountInlineMetric
                    label={t("dashboard.today.inProgressConversations")}
                    value={inProgressDisplayValue}
                    tone="secondary"
                    iconName="send"
                    metricKey="in-progress"
                    alignment="start"
                    fillAvailableWidth
                  />
                </div>
                <div className="min-w-0">
                  <AccountInlineMetric
                    label="TPM"
                    value={tokensPerMinuteDisplayValue}
                    tone="primary"
                    iconName="speedometer"
                    metricKey="tpm"
                    alignment="center"
                    fillAvailableWidth
                    modelPerformance={account.modelPerformance}
                    modelPerformanceTitle={modelPerformanceTitle}
                  />
                </div>
                <div className="min-w-0">
                  <AccountInlineMetric
                    label={t("dashboard.today.spendRate")}
                    value={spendRateDisplayValue}
                    tone="warning"
                    iconName="cash-clock"
                    metricKey="spend-rate"
                    alignment="end"
                    fillAvailableWidth
                    modelPerformance={account.modelPerformance}
                    modelPerformanceTitle={modelPerformanceTitle}
                  />
                </div>
              </div>
            </div>
          ) : (
            <div className="flex min-w-0 flex-wrap items-center justify-end gap-x-5 gap-y-1.5 text-right self-start">
              <AccountInlineMetric
                label={t("dashboard.today.inProgressConversations")}
                value={inProgressDisplayValue}
                tone="secondary"
                iconName="send"
                metricKey="in-progress"
              />
              <AccountInlineMetric
                label="TPM"
                value={tokensPerMinuteDisplayValue}
                tone="primary"
                iconName="speedometer"
                metricKey="tpm"
                valueWidthBudgetCh={ACCOUNT_INLINE_TPM_SPLIT_VALUE_WIDTH_BUDGET_CH}
                modelPerformance={account.modelPerformance}
                modelPerformanceTitle={modelPerformanceTitle}
              />
              <AccountInlineMetric
                label={t("dashboard.today.spendRate")}
                value={spendRateDisplayValue}
                tone="warning"
                iconName="cash-clock"
                metricKey="spend-rate"
                modelPerformance={account.modelPerformance}
                modelPerformanceTitle={modelPerformanceTitle}
              />
            </div>
          )}
        </div>
        {policySaveError ? (
          <div
            role="alert"
            data-testid="dashboard-upstream-account-policy-error"
            className="inline-flex max-w-full rounded-lg border border-error/30 bg-error/10 px-2.5 py-1 text-xs font-medium text-error"
          >
            {locale === "zh" ? "策略保存失败：" : "Policy save failed: "}
            <span className="truncate">{policySaveError}</span>
          </div>
        ) : null}
      </div>

      <div className="mt-4 flex flex-col gap-2.5">
        <div
          className={cn(
            "grid gap-2.5",
            heroMetricColumnCount === 1
              ? "grid-cols-1"
              : heroMetricColumnCount === 2
                ? "grid-cols-2"
                : "grid-cols-4",
          )}
        >
          <AccountHeroMetric
            label={t("dashboard.today.firstResponseTime")}
            value={currentFirstByteDisplayValue}
            tone={currentFirstByteValue === FALLBACK_CELL ? "neutral" : "secondary"}
            iconName="timer-outline"
            metricKey="latency"
            detailSections={latencyDetailSections}
          >
            <AccountSegmentList
              segments={[
                {
                  label: t("dashboard.today.responseTime"),
                  value: currentAvgTotalDisplayValue,
                  tone: currentAvgTotalValue === FALLBACK_CELL ? "neutral" : "primary",
                },
              ]}
              testId="dashboard-upstream-account-latency-breakdown"
              enableTooltips={false}
            />
          </AccountHeroMetric>
          <AccountHeroMetric
            label={locale === "zh" ? "请求数" : "Requests"}
            value={totalRequestDisplayValue}
            tone="neutral"
            iconName="counter"
            metricKey="requests"
            detailSections={requestDetailSections}
          >
            <AccountSegmentList
              segments={requestSummarySegments}
              testId="dashboard-upstream-account-request-breakdown"
              enableTooltips={false}
            />
          </AccountHeroMetric>
          <AccountHeroMetric
            label={locale === "zh" ? "成本" : "Cost"}
            value={totalCostDisplayValue}
            tone="warning"
            iconName="currency-usd"
            metricKey="cost"
            detailSections={costDetailSections}
            tooltipContent={
              <UsageBreakdownTooltip
                title={usageDetailsLabel}
                breakdown={account.usageBreakdown}
                formatNumber={formatBreakdownNumber}
                formatRatio={formatBreakdownRatio}
                formatCurrency={formatBreakdownCurrency}
                labels={usageBreakdownLabels}
              />
            }
          >
            <AccountSegmentList
              segments={costSummarySegments}
              testId="dashboard-upstream-account-cost-breakdown"
              enableTooltips={false}
            />
          </AccountHeroMetric>
          <AccountHeroMetric
            label="Token"
            value={totalTokenDisplayValue}
            tone="success"
            iconName="database-outline"
            metricKey="token"
            detailSections={tokenDetailSections}
            tooltipContent={
              <UsageBreakdownTooltip
                title={usageDetailsLabel}
                breakdown={account.usageBreakdown}
                formatNumber={formatBreakdownNumber}
                formatRatio={formatBreakdownRatio}
                formatCurrency={formatBreakdownCurrency}
                labels={usageBreakdownLabels}
              />
            }
          >
            <AccountSegmentList
              segments={tokenSummarySegments}
              testId="dashboard-upstream-account-token-breakdown"
              enableTooltips={false}
            />
          </AccountHeroMetric>
        </div>
      </div>

      <div
        className={cn(
          "mt-3.5 flex flex-1 flex-col border-t pt-2.5",
          ACCOUNT_CARD_INNER_BORDER_CLASS_NAME,
        )}
      >
        <div
          className={cn(
            "mb-2 flex flex-wrap gap-x-3 gap-y-1.5",
            recentBreakdownLayout === "stacked"
              ? "items-start justify-start"
              : "items-center justify-between",
          )}
        >
          <div className="text-xs font-semibold leading-5 tracking-[0.06em] text-base-content/62">
            {t("dashboard.upstreamAccounts.recentInvocations", {
              count: recentPreviewLimit,
            })}
          </div>
          <div
            className={cn(
              "flex flex-wrap gap-x-3 gap-y-1.5",
              recentBreakdownLayout === "stacked" ? "justify-start" : "items-center justify-end",
            )}
            data-testid="dashboard-upstream-account-recent-breakdown"
          >
            <InvocationPhaseSegments
              counts={account.inProgressPhaseCounts}
              appearance="inline"
              motion="static"
              showLabel={recentBreakdownLayout !== "stacked"}
              className="justify-end"
            />
            {recentBridgeSegments.length > 0 ? (
              <AccountSegmentList
                segments={recentBridgeSegments}
                showLabel={recentBreakdownLayout !== "stacked"}
                showIconWhenLabelHidden
                className="justify-end"
              />
            ) : null}
          </div>
        </div>
        <div className="grid flex-1 auto-rows-fr gap-1.5" aria-live="polite">
          {recentLoading && recentInvocations.length === 0
            ? DASHBOARD_RECENT_SKELETON_IDS.slice(0, recentPreviewLimit).map((skeletonId) => (
                <div
                  key={skeletonId}
                  data-testid="dashboard-upstream-account-recent-skeleton"
                  className="min-h-12 rounded-xl bg-base-200/65 p-2.5"
                >
                  <div className="h-2.5 w-28 animate-pulse rounded bg-base-300/75" />
                  <div className="mt-2 h-2 w-3/4 animate-pulse rounded bg-base-300/55" />
                </div>
              ))
            : null}
          {!recentLoading && recentError ? (
            <div className="flex min-h-24 flex-col items-start justify-center gap-2 rounded-xl bg-error/8 px-3 py-2 text-xs text-base-content/72">
              <span>{t("dashboard.upstreamAccounts.recentError")}</span>
              <button
                type="button"
                className="font-semibold text-error underline decoration-error/45 underline-offset-4 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                onClick={onRetryRecent}
              >
                {t("dashboard.upstreamAccounts.retryRecent")}
              </button>
            </div>
          ) : null}
          {!recentError && (!recentLoading || recentInvocations.length > 0)
            ? recentInvocations.map((invocation: DashboardWorkingConversationInvocationModel) => (
                <AccountRecentInvocationRow
                  key={`${invocation.record.invokeId}:${invocation.record.occurredAt}:${invocation.record.id}`}
                  invocation={invocation}
                  locale={locale}
                  nowMs={nowMs}
                  onOpenUpstreamAccount={onOpenUpstreamAccount}
                  onOpenConversation={onOpenConversation}
                  onOpenInvocation={onOpenInvocation}
                />
              ))
            : null}
        </div>
      </div>
    </article>
  );
}

function DashboardUpstreamAccountGridSkeleton() {
  return (
    <div
      data-testid="dashboard-upstream-account-grid-skeleton"
      className="grid grid-cols-1 gap-4 desktop1660:grid-cols-[repeat(2,minmax(0,1fr))]"
      aria-busy="true"
    >
      {DASHBOARD_ACCOUNT_SKELETON_IDS.map((cardId) => (
        <div key={cardId} className={ACCOUNT_CARD_CLASS_NAME}>
          <div className="flex items-center justify-between gap-4">
            <div className="h-5 w-36 animate-pulse rounded bg-base-300/75" />
            <div className="h-7 w-28 animate-pulse rounded-full bg-base-300/55" />
          </div>
          <div className="mt-5 grid grid-cols-2 gap-2 sm:grid-cols-4">
            {DASHBOARD_ACCOUNT_METRIC_SKELETON_IDS.map((metricId) => (
              <div key={metricId} className="h-20 animate-pulse rounded-xl bg-base-200/72" />
            ))}
          </div>
          <div className="mt-4 border-t border-base-300/45 pt-3">
            <div className="mb-3 h-3 w-28 animate-pulse rounded bg-base-300/65" />
            <div className="grid gap-1.5">
              {DASHBOARD_ACCOUNT_RECENT_SKELETON_IDS.map((rowId) => (
                <div key={rowId} className="h-12 animate-pulse rounded-xl bg-base-200/65" />
              ))}
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

function DashboardUpstreamAccountRefreshStatus({
  label,
  visibleLabel,
  visible,
}: {
  label: string;
  visibleLabel: string;
  visible: boolean;
}) {
  if (!visible) {
    return null;
  }

  return (
    <div
      data-testid="dashboard-upstream-account-refresh-status"
      role="status"
      aria-live="polite"
      aria-label={label}
      title={label}
      className="inline-flex h-7 shrink-0 items-center gap-1 text-info"
    >
      <Spinner
        data-testid="dashboard-upstream-account-refresh-spinner"
        size="sm"
        className="h-3 w-3 border-[1.65px]"
        aria-hidden
      />
      <span
        data-testid="dashboard-upstream-account-refresh-text"
        aria-hidden
        className="hidden whitespace-nowrap text-[11px] font-semibold leading-none desktop:inline"
      >
        {visibleLabel}
      </span>
    </div>
  );
}

interface DashboardWorkingConversationAnchorCardElement extends HTMLElement {
  __dashboardWorkingConversationAnchorKey?: string;
}

type DashboardVisibleAnchorKind = "conversation" | "upstreamAccount";

interface DashboardVisibleAnchorTarget {
  hasHiddenContentAbove: boolean;
  kind: DashboardVisibleAnchorKind;
  selector: string;
  readAnchorKey: (card: HTMLElement) => string;
}

function readDashboardWorkingConversationAnchorKey(card: HTMLElement) {
  return (
    (card as DashboardWorkingConversationAnchorCardElement)
      .__dashboardWorkingConversationAnchorKey ?? ""
  ).trim();
}

function readDashboardUpstreamAccountAnchorKey(card: HTMLElement) {
  return (card.getAttribute("data-account-key") ?? "").trim();
}

function captureVisibleCardAnchor(container: HTMLDivElement, target: DashboardVisibleAnchorTarget) {
  const containerRect = container.getBoundingClientRect();
  const topBoundary = Math.max(0, containerRect.top);
  const viewportBottom =
    typeof window === "undefined" ? Number.POSITIVE_INFINITY : window.innerHeight;
  const cards = Array.from(container.querySelectorAll<HTMLElement>(target.selector));
  let hasHiddenContentAbove = target.hasHiddenContentAbove;
  for (const card of cards) {
    const rect = card.getBoundingClientRect();
    if (rect.top < topBoundary) {
      hasHiddenContentAbove = true;
    }
    if (rect.bottom <= topBoundary) {
      continue;
    }
    if (rect.top >= viewportBottom) continue;
    const anchorKey = target.readAnchorKey(card);
    if (!anchorKey) continue;
    return {
      kind: target.kind,
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
  recentPreviewLimit = 4,
  isLoading,
  isLoadingMore = false,
  error,
  onLoadMore,
  setRefreshTargetCount,
  onOpenUpstreamAccount,
  onOpenConversation,
  onOpenInvocation,
  upstreamAccountActivity: externalUpstreamAccountActivity,
  upstreamAccountActivityLoading: externalUpstreamAccountActivityLoading,
  upstreamAccountActivityRefreshing: externalUpstreamAccountActivityRefreshing,
  upstreamAccountActivityError: externalUpstreamAccountActivityError,
  upstreamAccountRecentLoading: externalUpstreamAccountRecentLoading,
  upstreamAccountRecentError: externalUpstreamAccountRecentError,
  onRetryUpstreamAccountRecent,
  upstreamAccountRecentPreviewLimit: externalUpstreamAccountRecentPreviewLimit,
  onUpstreamAccountActivityEnabledChange,
  onUpstreamAccountPolicyChanged,
  onConversationsChanged,
}: DashboardWorkingConversationsSectionProps) {
  const { t, locale } = useTranslation();
  const [preferredView, setPreferredView] = useState<DashboardWorkspaceView>(() =>
    readPersistedDashboardWorkspaceView(DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY),
  );
  const [conversationSort, setConversationSort] = useState<DashboardWorkspaceSort>(() =>
    readDashboardWorkspaceSort(DASHBOARD_CONVERSATION_SORT_STORAGE_KEY),
  );
  const [upstreamAccountSort, setUpstreamAccountSort] = useState<DashboardWorkspaceSort>(() =>
    readDashboardWorkspaceSort(DASHBOARD_UPSTREAM_ACCOUNT_SORT_STORAGE_KEY),
  );
  const [nowMs, setNowMs] = useState(() => Date.now());
  const [containerWidth, setContainerWidth] = useState(0);
  const [viewportWidth, setViewportWidth] = useState(() =>
    typeof window === "undefined" ? 0 : window.innerWidth,
  );
  const [isBrowserOffline, setIsBrowserOffline] = useState(readBrowserOfflineState);
  const [gridElement, setGridElement] = useState<HTMLDivElement | null>(null);
  const [scrollMargin, setScrollMargin] = useState(0);
  const visibleAnchorRef = useRef<{
    kind: DashboardVisibleAnchorKind;
    anchorKey: string;
    top: number;
  } | null>(null);
  const loadMoreRequestPendingRef = useRef(false);
  const previousLoadingMoreRef = useRef(isLoadingMore);
  const previousRowsLengthRef = useRef(cards.length);
  const upstreamAccountRefreshChipVisibleAtRef = useRef<number | null>(null);
  const upstreamAccountRefreshChipShowTimerRef = useRef<number | null>(null);
  const upstreamAccountRefreshChipHideTimerRef = useRef<number | null>(null);
  const fastModeTriggerRef = useRef<HTMLButtonElement | null>(null);
  const [isUpstreamAccountRefreshChipVisible, setIsUpstreamAccountRefreshChipVisible] =
    useState(false);
  const [selectionModeEnabled, setSelectionModeEnabled] = useState(false);
  const [selectedPromptCacheKeys, setSelectedPromptCacheKeys] = useState<string[]>([]);
  const [routeBindDialogOpen, setRouteBindDialogOpen] = useState(false);
  const [clearAffinityDialogOpen, setClearAffinityDialogOpen] = useState(false);
  const [fastModePopoverOpen, setFastModePopoverOpen] = useState(false);
  const [routeBindTargetKind, setRouteBindTargetKind] =
    useState<DashboardConversationBulkBindTargetKind>("group");
  const [routeBindGroupName, setRouteBindGroupName] = useState("");
  const [routeBindAccountId, setRouteBindAccountId] = useState("");
  const [bulkFastModeRewriteMode, setBulkFastModeRewriteMode] =
    useState<PromptCacheConversationRewriteMode>("keep_original");
  const [bulkActionBusy, setBulkActionBusy] = useState<
    "bind" | "clearAndResetAffinity" | "setFastModeRewriteMode" | null
  >(null);
  const [bulkFeedback, setBulkFeedback] = useState<DashboardConversationBulkFeedback | null>(null);
  const [bindingTargets, setBindingTargets] =
    useState<DashboardConversationBulkBindingTargetsState>({
      accounts: [],
      groups: [],
      loading: false,
      loaded: false,
      error: null,
    });
  const setGridContainerRef = useCallback((node: HTMLDivElement | null) => {
    setGridElement(node);
  }, []);
  const hasInFlightCards = cards.some(
    (card) => card.currentInvocation.isInFlight || card.previousInvocation?.isInFlight === true,
  );
  const localeTag = locale === "zh" ? "zh-CN" : "en-US";
  const networkUploadLabel = t("dashboard.activityOverview.networkUpload");
  const networkDownloadLabel = t("dashboard.activityOverview.networkDownload");
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag), [localeTag]);
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
  const selectedPromptCacheKeySet = useMemo(
    () => new Set(selectedPromptCacheKeys),
    [selectedPromptCacheKeys],
  );
  const currentPromptCacheKeySet = useMemo(
    () => new Set(cards.map((card) => card.promptCacheKey)),
    [cards],
  );
  const selectedConversationCount = selectedPromptCacheKeys.length;
  const closeConversationBulkDialogs = useCallback(() => {
    setRouteBindDialogOpen(false);
    setClearAffinityDialogOpen(false);
    setFastModePopoverOpen(false);
  }, []);
  const resetConversationSelectionState = useCallback(() => {
    setSelectionModeEnabled(false);
    setSelectedPromptCacheKeys([]);
    setBulkFeedback(null);
    closeConversationBulkDialogs();
  }, [closeConversationBulkDialogs]);
  const upstreamAccountsDisabled = activeRange === "usage";
  const activeView: DashboardWorkspaceView =
    upstreamAccountsDisabled && preferredView === "upstreamAccounts"
      ? "conversations"
      : preferredView;
  const upstreamAccountActivityEnabled =
    !upstreamAccountsDisabled && activeView === "upstreamAccounts";
  const hasExternalUpstreamAccountActivity =
    externalUpstreamAccountActivity !== undefined ||
    externalUpstreamAccountActivityLoading !== undefined ||
    externalUpstreamAccountActivityError !== undefined;
  const hookUpstreamAccountActivity = useDashboardUpstreamAccountActivity(
    activeRange,
    !hasExternalUpstreamAccountActivity && upstreamAccountActivityEnabled,
    recentPreviewLimit,
  );
  const upstreamAccountActivity = hasExternalUpstreamAccountActivity
    ? (externalUpstreamAccountActivity ?? null)
    : hookUpstreamAccountActivity.data;
  const upstreamAccountActivityLoading = hasExternalUpstreamAccountActivity
    ? externalUpstreamAccountActivityLoading === true
    : hookUpstreamAccountActivity.isLoading;
  const upstreamAccountActivityRefreshing = hasExternalUpstreamAccountActivity
    ? externalUpstreamAccountActivityRefreshing === true
    : hookUpstreamAccountActivity.isRefreshing;
  const upstreamAccountRecentLoading = hasExternalUpstreamAccountActivity
    ? externalUpstreamAccountRecentLoading === true
    : hookUpstreamAccountActivity.recentLoading;
  const upstreamAccountRecentError = hasExternalUpstreamAccountActivity
    ? (externalUpstreamAccountRecentError ?? null)
    : hookUpstreamAccountActivity.recentError;
  const retryUpstreamAccountRecent = hasExternalUpstreamAccountActivity
    ? onRetryUpstreamAccountRecent
    : hookUpstreamAccountActivity.retryRecent;
  const upstreamAccountActivityError = hasExternalUpstreamAccountActivity
    ? (externalUpstreamAccountActivityError ?? null)
    : hookUpstreamAccountActivity.error;
  const upstreamAccountActivityPending =
    upstreamAccountActivityEnabled &&
    upstreamAccountActivity == null &&
    upstreamAccountActivityError == null;
  const showUpstreamAccountActivityLoading =
    upstreamAccountActivityLoading || upstreamAccountActivityPending;
  const upstreamAccountRecentPreviewLimit = hasExternalUpstreamAccountActivity
    ? (externalUpstreamAccountRecentPreviewLimit ?? recentPreviewLimit)
    : hookUpstreamAccountActivity.recentInvocationLimit;
  const refreshUpstreamAccountActivity = useCallback(() => {
    if (hasExternalUpstreamAccountActivity) {
      onUpstreamAccountPolicyChanged?.();
      return;
    }
    hookUpstreamAccountActivity.reload();
  }, [
    hasExternalUpstreamAccountActivity,
    hookUpstreamAccountActivity,
    onUpstreamAccountPolicyChanged,
  ]);
  useEffect(() => {
    onUpstreamAccountActivityEnabledChange?.(upstreamAccountActivityEnabled);
  }, [onUpstreamAccountActivityEnabledChange, upstreamAccountActivityEnabled]);
  const clearUpstreamAccountRefreshChipTimers = useCallback(() => {
    if (upstreamAccountRefreshChipShowTimerRef.current != null) {
      clearTimeout(upstreamAccountRefreshChipShowTimerRef.current);
      upstreamAccountRefreshChipShowTimerRef.current = null;
    }
    if (upstreamAccountRefreshChipHideTimerRef.current != null) {
      clearTimeout(upstreamAccountRefreshChipHideTimerRef.current);
      upstreamAccountRefreshChipHideTimerRef.current = null;
    }
  }, []);
  const upstreamAccounts = useMemo(
    () =>
      [...(upstreamAccountActivity?.accounts ?? [])].sort((left, right) =>
        compareDashboardUpstreamAccounts(left, right, upstreamAccountSort),
      ),
    [upstreamAccountActivity, upstreamAccountSort],
  );
  const totalNetworkSpeed = useMemo(
    () => ({
      uploadBytesPerSecond: Math.max(
        0,
        upstreamAccountActivity?.networkLiveBucket?.uploadBytesPerSecond ?? 0,
      ),
      downloadBytesPerSecond: Math.max(
        0,
        upstreamAccountActivity?.networkLiveBucket?.downloadBytesPerSecond ?? 0,
      ),
    }),
    [upstreamAccountActivity?.networkLiveBucket],
  );
  useEffect(() => {
    persistDashboardWorkspaceView(DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY, preferredView);
  }, [preferredView]);
  useEffect(() => {
    persistDashboardWorkspaceSort(DASHBOARD_CONVERSATION_SORT_STORAGE_KEY, conversationSort);
  }, [conversationSort]);
  useEffect(() => {
    persistDashboardWorkspaceSort(DASHBOARD_UPSTREAM_ACCOUNT_SORT_STORAGE_KEY, upstreamAccountSort);
  }, [upstreamAccountSort]);
  const countBadgeValue = totalMatched ?? cards.length;
  const accountCountBadgeValue = upstreamAccounts.length;
  const countBadgeLabel =
    activeView === "conversations"
      ? t("dashboard.workingConversations.countBadge", {
          count: countBadgeValue,
        })
      : showUpstreamAccountActivityLoading && upstreamAccounts.length === 0
        ? t("dashboard.upstreamAccounts.countLoading")
        : t("dashboard.upstreamAccounts.countBadge", {
            count: accountCountBadgeValue,
          });
  const shouldReserveUpstreamAccountRefreshChip = activeView === "upstreamAccounts";
  const shouldShowUpstreamAccountRefreshChip =
    shouldReserveUpstreamAccountRefreshChip &&
    upstreamAccounts.length > 0 &&
    isUpstreamAccountRefreshChipVisible;
  const shouldShowTotalNetworkSpeed =
    !upstreamAccountsDisabled &&
    (hasExternalUpstreamAccountActivity ||
      upstreamAccountActivity != null ||
      showUpstreamAccountActivityLoading ||
      upstreamAccountActivityEnabled);
  useEffect(() => {
    if (activeView === "conversations") return;
    resetConversationSelectionState();
  }, [activeView, resetConversationSelectionState]);

  useEffect(() => {
    setSelectedPromptCacheKeys((current) =>
      current.filter((promptCacheKey) => currentPromptCacheKeySet.has(promptCacheKey)),
    );
  }, [currentPromptCacheKeySet]);

  useEffect(() => {
    if (selectedPromptCacheKeys.length > 0) return;
    closeConversationBulkDialogs();
  }, [closeConversationBulkDialogs, selectedPromptCacheKeys.length]);

  const loadConversationBindingTargets = useCallback(async () => {
    setBindingTargets((current) => ({
      ...current,
      loading: true,
      error: null,
    }));
    try {
      const response = await fetchUpstreamAccounts({
        includeAll: true,
        pageSize: 500,
      });
      const accounts = response.items.filter(accountCanBePromptCacheBindingTarget);
      const groups = normalizeConversationBindingGroups(response.groups, accounts, localeTag);
      setBindingTargets({
        accounts,
        groups,
        loading: false,
        loaded: true,
        error: null,
      });
      setRouteBindTargetKind((current) => {
        if (current === "group" && groups.length === 0 && accounts.length > 0) {
          return "upstreamAccount";
        }
        if (current === "upstreamAccount" && accounts.length === 0 && groups.length > 0) {
          return "group";
        }
        return current;
      });
      setRouteBindGroupName((current) => current || groups[0] || "");
      setRouteBindAccountId((current) => current || (accounts[0] ? String(accounts[0].id) : ""));
    } catch (err) {
      setBindingTargets((current) => ({
        ...current,
        loading: false,
        loaded: false,
        error: err instanceof Error ? err.message : String(err),
      }));
    }
  }, [localeTag]);

  useEffect(() => {
    if (
      !routeBindDialogOpen ||
      bindingTargets.loaded ||
      bindingTargets.loading ||
      bindingTargets.error != null
    )
      return;
    void loadConversationBindingTargets();
  }, [
    bindingTargets.error,
    bindingTargets.loaded,
    bindingTargets.loading,
    loadConversationBindingTargets,
    routeBindDialogOpen,
  ]);

  useEffect(() => {
    if (bindingTargets.groups.length > 0 && !bindingTargets.groups.includes(routeBindGroupName)) {
      setRouteBindGroupName(bindingTargets.groups[0] ?? "");
    }
  }, [bindingTargets.groups, routeBindGroupName]);

  useEffect(() => {
    if (
      bindingTargets.accounts.length > 0 &&
      !bindingTargets.accounts.some((account) => String(account.id) === routeBindAccountId)
    ) {
      setRouteBindAccountId(String(bindingTargets.accounts[0]?.id ?? ""));
    }
  }, [bindingTargets.accounts, routeBindAccountId]);

  const applyBulkConversationAction = useCallback(
    async (
      payload:
        | {
            action: "bind";
            bindingKind: "group";
            groupName: string;
          }
        | {
            action: "bind";
            bindingKind: "upstreamAccount";
            upstreamAccountId: number;
          }
        | {
            action: "clearAndResetAffinity";
          }
        | {
            action: "setFastModeRewriteMode";
            fastModeRewriteMode: PromptCacheConversationRewriteMode;
          },
    ) => {
      if (selectedPromptCacheKeys.length === 0) return;
      setBulkActionBusy(payload.action);
      setBulkFeedback(null);
      try {
        const response = await bulkUpdatePromptCacheConversationBindings({
          ...payload,
          promptCacheKeys: selectedPromptCacheKeys,
        });
        const succeededKeys = new Set(
          response.items.filter((item) => item.ok).map((item) => item.promptCacheKey),
        );
        const failedItems = response.items.filter((item) => !item.ok);
        if (succeededKeys.size > 0) {
          setSelectedPromptCacheKeys((current) =>
            current.filter((promptCacheKey) => !succeededKeys.has(promptCacheKey)),
          );
          onConversationsChanged?.();
        }
        const failureMessage = formatDashboardConversationBulkFailureMessage(failedItems, locale);
        if (failedItems.length === 0) {
          setBulkFeedback({
            variant: "success",
            message:
              locale === "zh"
                ? `已更新 ${response.totalSucceeded} 个对话。`
                : `Updated ${response.totalSucceeded} conversations.`,
          });
        } else if (response.totalSucceeded > 0) {
          setBulkFeedback({
            variant: "warning",
            message:
              locale === "zh"
                ? `已更新 ${response.totalSucceeded} 个对话。${failureMessage ?? ""}`
                : `Updated ${response.totalSucceeded} conversations. ${failureMessage ?? ""}`,
          });
        } else {
          setBulkFeedback({
            variant: "error",
            message:
              failureMessage ??
              (locale === "zh"
                ? "批量操作失败，请稍后重试。"
                : "Bulk action failed. Please try again."),
          });
        }
        closeConversationBulkDialogs();
      } catch (err) {
        setBulkFeedback({
          variant: "error",
          message: err instanceof Error ? err.message : String(err),
        });
      } finally {
        setBulkActionBusy(null);
      }
    },
    [closeConversationBulkDialogs, locale, onConversationsChanged, selectedPromptCacheKeys],
  );
  const showWorkingConversationsOfflineState =
    activeView === "conversations" && isBrowserOffline && cards.length === 0;

  useEffect(() => {
    const handleOnline = () => {
      setIsBrowserOffline(false);
    };
    const handleOffline = () => {
      setIsBrowserOffline(true);
    };

    window.addEventListener("online", handleOnline);
    window.addEventListener("offline", handleOffline);
    return () => {
      window.removeEventListener("online", handleOnline);
      window.removeEventListener("offline", handleOffline);
    };
  }, []);
  useEffect(() => {
    if (!shouldReserveUpstreamAccountRefreshChip || upstreamAccounts.length === 0) {
      clearUpstreamAccountRefreshChipTimers();
      upstreamAccountRefreshChipVisibleAtRef.current = null;
      setIsUpstreamAccountRefreshChipVisible(false);
      return;
    }
    if (upstreamAccountActivityRefreshing) {
      if (upstreamAccountRefreshChipHideTimerRef.current != null) {
        clearTimeout(upstreamAccountRefreshChipHideTimerRef.current);
        upstreamAccountRefreshChipHideTimerRef.current = null;
      }
      if (
        isUpstreamAccountRefreshChipVisible ||
        upstreamAccountRefreshChipShowTimerRef.current != null
      ) {
        return;
      }
      upstreamAccountRefreshChipShowTimerRef.current = window.setTimeout(() => {
        upstreamAccountRefreshChipShowTimerRef.current = null;
        upstreamAccountRefreshChipVisibleAtRef.current = Date.now();
        setIsUpstreamAccountRefreshChipVisible(true);
      }, UPSTREAM_ACCOUNT_REFRESH_CHIP_SHOW_DELAY_MS);
      return;
    }
    if (upstreamAccountRefreshChipShowTimerRef.current != null) {
      clearTimeout(upstreamAccountRefreshChipShowTimerRef.current);
      upstreamAccountRefreshChipShowTimerRef.current = null;
    }
    if (!isUpstreamAccountRefreshChipVisible) {
      return;
    }
    const visibleForMs =
      upstreamAccountRefreshChipVisibleAtRef.current == null
        ? UPSTREAM_ACCOUNT_REFRESH_CHIP_MIN_VISIBLE_MS
        : Date.now() - upstreamAccountRefreshChipVisibleAtRef.current;
    const remainingVisibleMs = Math.max(
      0,
      UPSTREAM_ACCOUNT_REFRESH_CHIP_MIN_VISIBLE_MS - visibleForMs,
    );
    if (remainingVisibleMs === 0) {
      upstreamAccountRefreshChipVisibleAtRef.current = null;
      setIsUpstreamAccountRefreshChipVisible(false);
      return;
    }
    if (upstreamAccountRefreshChipHideTimerRef.current != null) {
      return;
    }
    upstreamAccountRefreshChipHideTimerRef.current = window.setTimeout(() => {
      upstreamAccountRefreshChipHideTimerRef.current = null;
      upstreamAccountRefreshChipVisibleAtRef.current = null;
      setIsUpstreamAccountRefreshChipVisible(false);
    }, remainingVisibleMs);
  }, [
    clearUpstreamAccountRefreshChipTimers,
    isUpstreamAccountRefreshChipVisible,
    shouldReserveUpstreamAccountRefreshChip,
    upstreamAccountActivityRefreshing,
    upstreamAccounts.length,
  ]);
  useEffect(
    () => () => {
      clearUpstreamAccountRefreshChipTimers();
    },
    [clearUpstreamAccountRefreshChipTimers],
  );
  const upstreamAccountRows = useMemo(
    () =>
      chunkDashboardUpstreamAccountRows(
        upstreamAccounts,
        resolveDashboardUpstreamAccountColumnCount(Math.max(containerWidth, viewportWidth)),
      ),
    [containerWidth, upstreamAccounts, viewportWidth],
  );
  const cssColumnCount = resolveDashboardWorkingConversationCssColumnCount(gridElement);
  const columnCount =
    cssColumnCount ??
    resolveDashboardWorkingConversationColumnCount(Math.max(containerWidth, viewportWidth));
  const sortedCards = useMemo(
    () =>
      [...cards].sort((left, right) =>
        compareDashboardConversationCards(left, right, conversationSort),
      ),
    [cards, conversationSort],
  );
  const rows = useMemo(
    () => chunkDashboardWorkingConversationRows(sortedCards, columnCount),
    [columnCount, sortedCards],
  );
  const activeSort = activeView === "conversations" ? conversationSort : upstreamAccountSort;
  const activeSortLabel = t(`dashboard.workspaceSort.${activeSort}`);
  const cycleSort = () => {
    if (activeView === "conversations") {
      setConversationSort((current) => nextDashboardWorkspaceSort(current));
    } else {
      setUpstreamAccountSort((current) => nextDashboardWorkspaceSort(current));
    }
  };
  const rowVirtualizer = useWindowVirtualizer({
    count: rows.length,
    estimateSize: () => 360,
    overscan: 3,
    scrollMargin,
  });
  const virtualRows = rowVirtualizer.getVirtualItems();
  const fallbackRowCount = Math.min(
    rows.length,
    Math.max(1, Math.ceil(DASHBOARD_WORKING_CONVERSATIONS_PAGE_SIZE / Math.max(columnCount, 1))),
  );
  const renderedRows =
    virtualRows.length > 0
      ? virtualRows
      : rows.slice(0, fallbackRowCount).map((_, index) => ({
          key: index,
          index,
          start: scrollMargin + index * 360,
        }));
  const hasVirtualizedRowsAbove = renderedRows.length > 0 ? renderedRows[0]?.index > 0 : false;
  const visibleAnchorTarget = useMemo<DashboardVisibleAnchorTarget>(
    () =>
      activeView === "conversations"
        ? {
            hasHiddenContentAbove: hasVirtualizedRowsAbove,
            kind: "conversation",
            selector: '[data-testid="dashboard-working-conversation-card"]',
            readAnchorKey: readDashboardWorkingConversationAnchorKey,
          }
        : {
            hasHiddenContentAbove: false,
            kind: "upstreamAccount",
            selector: '[data-testid="dashboard-upstream-account-card"]',
            readAnchorKey: readDashboardUpstreamAccountAnchorKey,
          },
    [activeView, hasVirtualizedRowsAbove],
  );
  const totalSize = virtualRows.length > 0 ? rowVirtualizer.getTotalSize() : rows.length * 360;
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
      const nextScrollMargin = gridElement.getBoundingClientRect().top + window.scrollY;
      setScrollMargin((current) =>
        Math.abs(current - nextScrollMargin) > 0.5 ? nextScrollMargin : current,
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
    const container = gridElement;
    if (!container) {
      visibleAnchorRef.current = null;
      return;
    }
    const updateAnchor = () => {
      const nextAnchor = captureVisibleCardAnchor(container, visibleAnchorTarget);
      visibleAnchorRef.current = nextAnchor?.hasHiddenContentAbove ? nextAnchor : null;
    };
    updateAnchor();
    window.addEventListener("scroll", updateAnchor, { passive: true });
    return () => {
      window.removeEventListener("scroll", updateAnchor);
    };
  }, [gridElement, visibleAnchorTarget]);

  useEffect(() => {
    if (
      activeView !== "conversations" ||
      !hasMore ||
      previousRowsLengthRef.current !== rows.length ||
      (previousLoadingMoreRef.current && !isLoadingMore)
    ) {
      loadMoreRequestPendingRef.current = false;
    }
    previousRowsLengthRef.current = rows.length;
    previousLoadingMoreRef.current = isLoadingMore;
  }, [activeView, hasMore, isLoadingMore, rows.length]);

  useEffect(() => {
    const container = gridElement;
    if (activeView !== "conversations" || !container || !hasMore || !onLoadMore) return;
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
      maybeLoadMore("mount");
    }, 0);
    const handleScroll = () => {
      maybeLoadMore("scroll");
    };
    window.addEventListener("scroll", handleScroll, { passive: true });
    return () => {
      window.clearTimeout(mountTimer);
      window.removeEventListener("scroll", handleScroll);
    };
  }, [
    activeView,
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
    if (container && pendingAnchor?.anchorKey && pendingAnchor.kind === visibleAnchorTarget.kind) {
      const anchoredCard = Array.from(
        container.querySelectorAll<HTMLElement>(visibleAnchorTarget.selector),
      ).find((card) => visibleAnchorTarget.readAnchorKey(card) === pendingAnchor.anchorKey);
      if (anchoredCard) {
        const containerTopBoundary = Math.max(0, container.getBoundingClientRect().top);
        const nextTop = anchoredCard.getBoundingClientRect().top - containerTopBoundary;
        const delta = nextTop - pendingAnchor.top;
        if (Math.abs(delta) > 0.5 && typeof window !== "undefined") {
          window.scrollBy(0, delta);
        }
      }
    }
    const nextAnchor = container ? captureVisibleCardAnchor(container, visibleAnchorTarget) : null;
    visibleAnchorRef.current = nextAnchor?.hasHiddenContentAbove ? nextAnchor : null;
  }, [cards, columnCount, gridElement, rows, upstreamAccounts, visibleAnchorTarget]);

  const selectionModeButtonLabel =
    locale === "zh"
      ? selectionModeEnabled
        ? "退出选择"
        : "选择模式"
      : selectionModeEnabled
        ? "Exit selection"
        : "Selection mode";
  const selectionSummaryLabel =
    locale === "zh"
      ? `已选 ${selectedConversationCount} 个对话`
      : `${selectedConversationCount} conversations selected`;
  const routeBindDialogTitle = locale === "zh" ? "批量路由绑定" : "Bulk route binding";
  const routeBindDialogDescription =
    locale === "zh"
      ? "支持批量绑定到分组或上游账号；如果要清空绑定，可在此弹窗底部直接进入强操作确认。"
      : "Bind the selected conversations to a group or upstream account. If you need to clear bindings instead, use the destructive action shortcut in this dialog footer.";
  const clearAffinityDialogTitle =
    locale === "zh" ? "清空绑定并重选" : "Clear bindings and reselect";
  const clearAffinityDialogDescription =
    locale === "zh"
      ? "会删除对话级手动绑定，并同时清掉 sticky route 与加密 owner lock，下一次调用重新选择上游账号。"
      : "This removes conversation bindings, sticky routes, and encrypted owner locks so the next request reselects an upstream account.";
  const clearAffinityCalloutTitle =
    locale === "zh"
      ? "会立即清理以下会话级锁定痕迹"
      : "This immediately clears the following conversation-level affinity locks";
  const clearAffinityCalloutDescription =
    locale === "zh"
      ? "下一次调用不再沿用当前 sticky / owner 归属，会重新选择上游账号。"
      : "The next request will reselect an upstream account instead of reusing the current sticky or owner affinity.";
  const clearAffinityCalloutItems =
    locale === "zh"
      ? [
          {
            key: "manual-binding",
            label: "对话级手动绑定",
            detail: "conversation manual binding",
          },
          {
            key: "sticky-route",
            label: "池 sticky route",
            detail: "pool sticky route",
          },
          {
            key: "owner-lock",
            label: "加密 owner lock",
            detail: "encrypted owner lock",
          },
        ]
      : [
          {
            key: "manual-binding",
            label: "Conversation manual binding",
            detail: "Conversation-level account override.",
          },
          {
            key: "sticky-route",
            label: "Pool sticky route",
            detail: "Sticky upstream affinity for this conversation.",
          },
          {
            key: "owner-lock",
            label: "Encrypted owner lock",
            detail: "Encrypted ownership marker used for reuse.",
          },
        ];
  const fastModePopoverTitle = locale === "zh" ? "FAST 模式" : "FAST mode";
  const fastModePopoverDescription =
    locale === "zh"
      ? "从下列策略中点选一项，立即批量写入 conversation 级 FAST 改写策略。"
      : "Pick a policy below to apply a conversation-level FAST rewrite mode immediately.";
  const fastModeOptions = useMemo(
    () => [
      {
        value: "keep_original" as PromptCacheConversationRewriteMode,
        label: t("live.conversations.drawer.policy.rewrite.keepOriginal"),
        description:
          locale === "zh"
            ? "保持原样，不主动补 Fast，也不主动移除 Fast。"
            : "Leave FAST unchanged for selected conversations.",
      },
      {
        value: "fill_missing" as PromptCacheConversationRewriteMode,
        label: t("live.conversations.drawer.policy.rewrite.fillMissing"),
        description:
          locale === "zh"
            ? "只有原请求没带 Fast 时才补上。"
            : "Add FAST only when the original request does not already include it.",
      },
      {
        value: "force_add" as PromptCacheConversationRewriteMode,
        label: t("live.conversations.drawer.policy.rewrite.forceAdd"),
        description:
          locale === "zh"
            ? "强制为所选对话加上 Fast。"
            : "Force FAST onto every selected conversation.",
      },
      {
        value: "force_remove" as PromptCacheConversationRewriteMode,
        label: t("live.conversations.drawer.policy.rewrite.forceRemove"),
        description:
          locale === "zh"
            ? "无论原请求如何，都移除 Fast。"
            : "Remove FAST even when it was originally requested.",
      },
    ],
    [locale, t],
  );
  const conversationBulkActionPanel =
    activeView === "conversations" && selectedConversationCount > 0 ? (
      <div
        className="dashboard-floating-action-shell"
        data-testid="dashboard-working-conversations-bulk-panel-shell"
      >
        <div
          className="dashboard-floating-action-panel"
          data-testid="dashboard-working-conversations-bulk-panel"
        >
          <div className="flex flex-col gap-3 px-4 py-4 sm:px-5">
            <div className="flex min-w-0 flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <div className="min-w-0">
                <div className="flex items-center gap-2 text-[0.72rem] font-semibold uppercase tracking-[0.14em] text-info/78">
                  <AppIcon name="check-circle-outline" className="h-3.5 w-3.5" aria-hidden />
                  <span>{locale === "zh" ? "批量操作" : "Bulk actions"}</span>
                </div>
                <p className="mt-1 text-sm font-semibold text-base-content">
                  {selectionSummaryLabel}
                </p>
                <p className="mt-1 text-xs text-base-content/68">
                  {locale === "zh"
                    ? "成功项会自动出列，失败项会保留选中。"
                    : "Successful items leave the selection automatically; failed items stay selected."}
                </p>
              </div>
              <div className="flex flex-wrap gap-2">
                <Button
                  type="button"
                  size="sm"
                  variant="secondary"
                  disabled={bulkActionBusy != null}
                  data-testid="dashboard-working-conversations-route-bind-button"
                  onClick={() => {
                    if (bindingTargets.error) {
                      setBindingTargets((current) => ({
                        ...current,
                        loaded: false,
                        error: null,
                      }));
                    }
                    setRouteBindDialogOpen(true);
                  }}
                >
                  {locale === "zh" ? "路由绑定" : "Bind route"}
                </Button>
                <Popover
                  open={fastModePopoverOpen}
                  onOpenChange={(nextOpen) => {
                    if (bulkActionBusy == null) setFastModePopoverOpen(nextOpen);
                  }}
                >
                  <PopoverTrigger asChild>
                    <Button
                      ref={fastModeTriggerRef}
                      type="button"
                      size="sm"
                      variant="secondary"
                      disabled={bulkActionBusy != null}
                      data-testid="dashboard-working-conversations-fast-mode-button"
                      className={cn(
                        "gap-2 pr-2.5",
                        fastModePopoverOpen &&
                          "border-info/55 bg-info/16 text-info shadow-[0_0_0_1px_rgba(72,186,255,0.14)]",
                      )}
                    >
                      <span>FAST 模式</span>
                      {bulkActionBusy === "setFastModeRewriteMode" ? (
                        <Spinner size="sm" />
                      ) : (
                        <AppIcon
                          name={fastModePopoverOpen ? "chevron-up" : "chevron-down"}
                          className="h-3.5 w-3.5"
                          aria-hidden
                        />
                      )}
                    </Button>
                  </PopoverTrigger>
                  <BubblePopoverContent
                    anchorElement={fastModeTriggerRef.current}
                    side="top"
                    align="end"
                    sideOffset={10}
                    collisionPadding={12}
                    className="w-[min(24rem,calc(100vw-1rem))] rounded-[1.35rem] px-4 py-4 shadow-[0_24px_70px_rgba(4,12,26,0.44)]"
                    data-testid="dashboard-working-conversations-fast-mode-popover"
                    onOpenAutoFocus={(event) => event.preventDefault()}
                    onCloseAutoFocus={(event) => event.preventDefault()}
                  >
                    <div className="space-y-3">
                      <div className="space-y-1">
                        <div className="flex items-center justify-between gap-3">
                          <p className="text-sm font-semibold text-base-content">
                            {fastModePopoverTitle}
                          </p>
                          <span className="rounded-full border border-base-300/70 bg-base-200/58 px-2.5 py-1 text-[0.68rem] font-semibold uppercase tracking-[0.14em] text-base-content/72">
                            {selectedConversationCount}
                          </span>
                        </div>
                        <p className="text-xs font-medium text-base-content/72">
                          {selectionSummaryLabel}
                        </p>
                        <p className="text-xs leading-5 text-base-content/62">
                          {fastModePopoverDescription}
                        </p>
                      </div>
                      <div
                        role="radiogroup"
                        aria-label={locale === "zh" ? "批量 FAST 模式" : "Bulk FAST mode"}
                        aria-busy={bulkActionBusy === "setFastModeRewriteMode"}
                        className="grid gap-2"
                      >
                        {fastModeOptions.map((option) => {
                          const active = option.value === bulkFastModeRewriteMode;
                          return (
                            <button
                              key={option.value}
                              type="button"
                              role="radio"
                              aria-checked={active}
                              disabled={bulkActionBusy === "setFastModeRewriteMode"}
                              data-testid="dashboard-working-conversations-fast-mode-option"
                              data-value={option.value}
                              className={cn(
                                "rounded-[1.05rem] border px-3.5 py-3 text-left transition-all duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-info/35 disabled:cursor-not-allowed disabled:opacity-70",
                                active
                                  ? "border-info/60 bg-info/14 text-base-content shadow-[inset_0_0_0_1px_rgba(72,186,255,0.16)]"
                                  : "border-base-300/70 bg-base-200/55 text-base-content/82 hover:border-info/28 hover:bg-base-200/78",
                              )}
                              onClick={() => {
                                setBulkFastModeRewriteMode(option.value);
                                void applyBulkConversationAction({
                                  action: "setFastModeRewriteMode",
                                  fastModeRewriteMode: option.value,
                                });
                              }}
                            >
                              <div className="flex items-start justify-between gap-3">
                                <div className="min-w-0">
                                  <div className="font-semibold">{option.label}</div>
                                  <p className="mt-1 text-xs leading-5 text-base-content/62">
                                    {option.description}
                                  </p>
                                </div>
                                <span
                                  className={cn(
                                    "mt-0.5 inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-full border transition-colors duration-150",
                                    active
                                      ? "border-info/55 bg-info/18 text-info"
                                      : "border-base-300/75 bg-base-100/60 text-base-content/30",
                                  )}
                                >
                                  <AppIcon name="check-bold" className="h-3.5 w-3.5" aria-hidden />
                                </span>
                              </div>
                            </button>
                          );
                        })}
                      </div>
                    </div>
                  </BubblePopoverContent>
                </Popover>
                <Button
                  type="button"
                  size="sm"
                  variant="destructive"
                  disabled={bulkActionBusy != null}
                  data-testid="dashboard-working-conversations-clear-affinity-button"
                  onClick={() => setClearAffinityDialogOpen(true)}
                >
                  {locale === "zh" ? "清空绑定并重选" : "Clear and reselect"}
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  disabled={bulkActionBusy != null}
                  data-testid="dashboard-working-conversations-clear-selection-button"
                  onClick={() => setSelectedPromptCacheKeys([])}
                >
                  {locale === "zh" ? "取消选择" : "Clear selection"}
                </Button>
              </div>
            </div>
          </div>
        </div>
      </div>
    ) : null;
  const routeBindSubmitDisabled =
    bulkActionBusy != null ||
    bindingTargets.loading ||
    (routeBindTargetKind === "group" ? !routeBindGroupName : !routeBindAccountId);
  const toggleConversationSelection = useCallback((promptCacheKey: string) => {
    setSelectedPromptCacheKeys((current) =>
      current.includes(promptCacheKey)
        ? current.filter((candidate) => candidate !== promptCacheKey)
        : [...current, promptCacheKey],
    );
  }, []);
  const toggleModifierConversationSelection = useCallback(
    (promptCacheKey: string) => {
      setBulkFeedback(null);
      toggleConversationSelection(promptCacheKey);
    },
    [toggleConversationSelection],
  );
  const handleConversationCardClickCapture = useCallback(
    (event: ReactMouseEvent<HTMLElement>, promptCacheKey: string) => {
      if (!hasMultiSelectModifier(event)) return;
      event.preventDefault();
      event.stopPropagation();
      toggleModifierConversationSelection(promptCacheKey);
    },
    [toggleModifierConversationSelection],
  );
  const handleSelectionCardClick = useCallback(
    (event: ReactMouseEvent<HTMLElement>, promptCacheKey: string) => {
      if (hasMultiSelectModifier(event)) return;
      toggleConversationSelection(promptCacheKey);
    },
    [toggleConversationSelection],
  );
  const handleSelectionCardKeyDown = useCallback(
    (event: ReactKeyboardEvent<HTMLElement>, promptCacheKey: string) => {
      if (event.target !== event.currentTarget) return;
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      toggleConversationSelection(promptCacheKey);
    },
    [toggleConversationSelection],
  );

  if (error && cards.length === 0) {
    return (
      <section className="surface-panel" data-testid="dashboard-working-conversations">
        <div className="surface-panel-body gap-4 desktop:!p-5">
          <div className="section-heading">
            <h2 className="section-title">{t("dashboard.section.workingConversationsTitle")}</h2>
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
      <div className="surface-panel-body gap-5 desktop:!p-5">
        <div
          className="flex min-w-0 flex-col gap-2 desktop:flex-row desktop:items-center desktop:justify-between"
          data-testid="dashboard-working-conversations-controls"
        >
          <SegmentedControl
            size="compact"
            className="w-full desktop:w-auto"
            role="tablist"
            aria-label="Dashboard workspace view"
          >
            <SegmentedControlItem
              active={activeView === "conversations"}
              role="tab"
              aria-selected={activeView === "conversations"}
              className="h-11 flex-1 px-3.5 text-[0.95rem]"
              onClick={() => setPreferredView("conversations")}
            >
              对话
            </SegmentedControlItem>
            <SegmentedControlItem
              active={activeView === "upstreamAccounts"}
              role="tab"
              aria-selected={activeView === "upstreamAccounts"}
              disabled={upstreamAccountsDisabled}
              className="h-11 flex-1 px-3.5 text-[0.95rem]"
              onClick={() => setPreferredView("upstreamAccounts")}
            >
              上游账号
            </SegmentedControlItem>
          </SegmentedControl>
          <div
            className="flex w-full min-w-0 items-center justify-between gap-2 px-4 desktop:w-auto desktop:flex-wrap desktop:justify-end desktop:px-0"
            data-testid="dashboard-working-conversations-actions"
          >
            <div
              className="inline-flex min-w-0 max-w-full flex-wrap items-center gap-2 desktop:flex desktop:min-w-0 desktop:justify-end"
              data-testid="dashboard-working-conversations-badges"
            >
              {shouldReserveUpstreamAccountRefreshChip ? (
                <DashboardUpstreamAccountRefreshStatus
                  label={t("dashboard.upstreamAccounts.refreshing")}
                  visibleLabel={t("dashboard.upstreamAccounts.refreshingShort")}
                  visible={shouldShowUpstreamAccountRefreshChip}
                />
              ) : null}
              {shouldShowTotalNetworkSpeed ? (
                <NetworkSpeedInline
                  uploadBytesPerSecond={totalNetworkSpeed.uploadBytesPerSecond}
                  downloadBytesPerSecond={totalNetworkSpeed.downloadBytesPerSecond}
                  localeTag={localeTag}
                  uploadLabel={networkUploadLabel}
                  downloadLabel={networkDownloadLabel}
                  testId="dashboard-upstream-account-total-network-speed"
                  className="bg-base-100/62"
                />
              ) : null}
              <Badge
                variant="default"
                className="w-fit rounded-full px-3 py-1 font-mono text-xs font-semibold"
              >
                {countBadgeLabel}
              </Badge>
            </div>
            {activeView === "conversations" ? (
              <Button
                type="button"
                variant={selectionModeEnabled ? "secondary" : "ghost"}
                className={cn(
                  "h-11 min-w-0 gap-2 px-3 text-sm desktop:w-auto",
                  selectionModeEnabled
                    ? "border border-info/35 bg-info/10 text-info hover:bg-info/16"
                    : "text-base-content/75 hover:bg-base-200/70 hover:text-base-content",
                )}
                disabled={bulkActionBusy != null}
                onClick={() => {
                  if (selectionModeEnabled) {
                    resetConversationSelectionState();
                    return;
                  }
                  setBulkFeedback(null);
                  setSelectionModeEnabled(true);
                }}
                data-testid="dashboard-working-conversations-selection-mode-button"
              >
                <AppIcon
                  name={selectionModeEnabled ? "close" : "check-circle-outline"}
                  className="h-4 w-4 shrink-0"
                  aria-hidden="true"
                />
                <span className="truncate">{selectionModeButtonLabel}</span>
              </Button>
            ) : null}
            <Button
              type="button"
              variant="ghost"
              className="h-11 min-w-0 gap-2 px-2.5 text-sm text-base-content/75 hover:bg-base-200/70 hover:text-base-content desktop:w-auto"
              onClick={cycleSort}
              title={t("dashboard.workspaceSort.tooltip", {
                current: activeSortLabel,
              })}
              aria-label={t("dashboard.workspaceSort.ariaLabel", {
                current: activeSortLabel,
              })}
              data-testid="dashboard-workspace-sort-button"
            >
              <AppIcon name="sort-variant" className="h-4 w-4 shrink-0" aria-hidden="true" />
              <span className="truncate">{activeSortLabel}</span>
            </Button>
          </div>
        </div>

        {error && cards.length > 0 ? (
          <Alert variant="error">
            <span>{error}</span>
          </Alert>
        ) : null}

        {bulkFeedback ? (
          <Alert variant={bulkFeedback.variant}>
            <span>{bulkFeedback.message}</span>
          </Alert>
        ) : null}

        {activeView === "upstreamAccounts" ? (
          <>
            {upstreamAccountActivityError ? (
              <Alert variant="error">
                <span>{upstreamAccountActivityError}</span>
              </Alert>
            ) : null}
            {showUpstreamAccountActivityLoading && upstreamAccounts.length === 0 ? (
              <DashboardUpstreamAccountGridSkeleton />
            ) : null}
            {!showUpstreamAccountActivityLoading && upstreamAccounts.length === 0 ? (
              <div className="rounded-2xl border border-dashed border-base-300/75 bg-base-100/45 px-5 py-8 text-sm text-base-content/65">
                {t("dashboard.upstreamAccounts.empty")}
              </div>
            ) : null}
            {upstreamAccounts.length > 0 ? (
              <div
                data-testid="dashboard-upstream-account-grid"
                ref={setGridContainerRef}
                className="grid grid-cols-1 gap-4 desktop1660:grid-cols-[repeat(2,minmax(0,1fr))]"
              >
                {upstreamAccountRows.flat().map((account) => (
                  <DashboardUpstreamAccountActivityCard
                    key={account.accountKey ?? account.upstreamAccountId ?? "unassigned"}
                    account={account}
                    locale={locale}
                    localeTag={localeTag}
                    nowMs={nowMs}
                    recentPreviewLimit={upstreamAccountRecentPreviewLimit}
                    onOpenUpstreamAccount={onOpenUpstreamAccount}
                    onOpenConversation={onOpenConversation}
                    onOpenInvocation={onOpenInvocation}
                    onPolicyChanged={refreshUpstreamAccountActivity}
                    recentLoading={upstreamAccountRecentLoading}
                    recentError={upstreamAccountRecentError}
                    onRetryRecent={retryUpstreamAccountRecent}
                  />
                ))}
              </div>
            ) : null}
          </>
        ) : null}

        {showWorkingConversationsOfflineState ? (
          <Alert
            variant="warning"
            className="border-warning/35 bg-warning/10 text-base-content"
            data-testid="dashboard-working-conversations-offline"
          >
            <div className="space-y-1">
              <span className="font-semibold">
                {t("dashboard.workingConversations.offlineTitle")}
              </span>
              <p className="text-sm text-base-content/80">
                {t("dashboard.workingConversations.offlineDescription")}
              </p>
            </div>
          </Alert>
        ) : null}

        {activeView === "conversations" &&
        !showWorkingConversationsOfflineState &&
        isLoading &&
        cards.length === 0 ? (
          <div className="flex min-h-44 items-center justify-center gap-3 rounded-2xl border border-dashed border-base-300/75 bg-base-100/45">
            <Spinner size="sm" aria-label={t("chart.loadingDetailed")} />
            <span className="text-sm text-base-content/70">{t("chart.loadingDetailed")}</span>
          </div>
        ) : null}

        {activeView === "conversations" &&
        !showWorkingConversationsOfflineState &&
        !isLoading &&
        cards.length === 0 ? (
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
                        const isCardSelected = selectedPromptCacheKeySet.has(card.promptCacheKey);
                        const displaySequenceId = formatDashboardWorkingConversationSequenceId(
                          card.conversationSequenceId,
                        );
                        const currentStatusLabel = currentStatusMeta.labelKey
                          ? t(currentStatusMeta.labelKey)
                          : (currentStatusMeta.label ?? t("table.status.unknown"));
                        const sortAnchorLabel =
                          card.sortAnchorEpoch != null
                            ? timestampFormatter.format(new Date(card.sortAnchorEpoch))
                            : FALLBACK_CELL;
                        const sequenceConversationActionLabel = `${t("dashboard.workingConversations.openConversation")} · ${displaySequenceId} · ${card.promptCacheKey}`;
                        const manualBindingBadgeMeta = resolveDashboardManualBindingBadgeMeta(
                          card.manualBinding,
                          t,
                        );
                        const manualBindingActionLabel = manualBindingBadgeMeta
                          ? `${t("dashboard.workingConversations.openConversationSettings")} · ${manualBindingBadgeMeta.accessibleLabel}`
                          : null;

                        return (
                          <article
                            key={card.promptCacheKey}
                            ref={(node) => {
                              if (!node) return;
                              (
                                node as DashboardWorkingConversationAnchorCardElement
                              ).__dashboardWorkingConversationAnchorKey = card.promptCacheKey;
                            }}
                            data-testid="dashboard-working-conversation-card"
                            data-conversation-sequence-id={displaySequenceId}
                            data-selection-mode={selectionModeEnabled ? "true" : "false"}
                            data-selected={isCardSelected ? "true" : "false"}
                            role={selectionModeEnabled ? "button" : undefined}
                            tabIndex={selectionModeEnabled ? 0 : undefined}
                            aria-pressed={selectionModeEnabled ? isCardSelected : undefined}
                            aria-label={
                              selectionModeEnabled
                                ? `${selectionSummaryLabel} · ${displaySequenceId}`
                                : undefined
                            }
                            className={cn(
                              CARD_CLASS_NAME,
                              currentStatusMeta.cardToneClassName,
                              selectionModeEnabled &&
                                "cursor-pointer ring-1 ring-white/8 hover:ring-info/30 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-info",
                              isCardSelected &&
                                "ring-2 ring-info/55 bg-info/10 shadow-[inset_0_1px_0_rgba(255,255,255,0.07),0_22px_34px_rgba(2,6,23,0.24)]",
                            )}
                            onClickCapture={(event) =>
                              handleConversationCardClickCapture(event, card.promptCacheKey)
                            }
                            onClick={
                              selectionModeEnabled
                                ? (event) => handleSelectionCardClick(event, card.promptCacheKey)
                                : undefined
                            }
                            onKeyDown={
                              selectionModeEnabled
                                ? (event) => handleSelectionCardKeyDown(event, card.promptCacheKey)
                                : undefined
                            }
                          >
                            <div className="relative">
                              {selectionModeEnabled || isCardSelected ? (
                                <div className="absolute right-0 top-0 z-[1] rounded-full border border-base-100/8 bg-base-200/78 p-0.5 shadow-[0_10px_24px_rgba(2,6,23,0.26)] backdrop-blur-md">
                                  <span
                                    data-testid="dashboard-working-conversation-selection-indicator"
                                    className={cn(
                                      "inline-flex h-7 w-7 items-center justify-center rounded-full border text-[11px] shadow-[inset_0_1px_0_rgba(255,255,255,0.08)]",
                                      isCardSelected
                                        ? "border-info/42 bg-base-100/94 text-info"
                                        : "border-base-300/70 bg-base-100/90 text-base-content/45",
                                    )}
                                  >
                                    {isCardSelected ? (
                                      <AppIcon
                                        name="check-bold"
                                        className="h-3.5 w-3.5"
                                        aria-hidden
                                      />
                                    ) : (
                                      <span className="h-2.5 w-2.5 rounded-full border border-current/45" />
                                    )}
                                  </span>
                                </div>
                              ) : null}
                              <div className="flex min-w-0 items-center justify-between gap-3">
                                <div className="flex min-w-0 flex-1 items-center gap-2">
                                  {onOpenConversation && !selectionModeEnabled ? (
                                    <button
                                      type="button"
                                      data-testid="dashboard-working-conversation-sequence-button"
                                      className="inline-flex shrink-0 cursor-pointer appearance-none items-center whitespace-nowrap border-0 bg-transparent p-0 text-left font-mono text-[0.95rem] font-semibold tracking-[0.08em] text-base-content transition-opacity duration-200 hover:opacity-80 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                                      aria-label={sequenceConversationActionLabel}
                                      title={sequenceConversationActionLabel}
                                      onClick={() => {
                                        onOpenConversation({
                                          conversationSequenceId: card.conversationSequenceId,
                                          promptCacheKey: card.promptCacheKey,
                                        });
                                      }}
                                    >
                                      <span className="block whitespace-nowrap">
                                        {displaySequenceId}
                                      </span>
                                    </button>
                                  ) : (
                                    <div className="shrink-0 whitespace-nowrap font-mono text-[0.95rem] font-semibold tracking-[0.08em] text-base-content">
                                      {displaySequenceId}
                                    </div>
                                  )}
                                  {manualBindingBadgeMeta ? (
                                    onOpenConversation && !selectionModeEnabled ? (
                                      <button
                                        type="button"
                                        data-testid="dashboard-working-conversation-manual-binding-badge"
                                        className={MANUAL_BINDING_BADGE_BUTTON_CLASS_NAME}
                                        aria-label={manualBindingActionLabel ?? undefined}
                                        title={manualBindingActionLabel ?? undefined}
                                        onClick={(event) => {
                                          event.stopPropagation();
                                          onOpenConversation({
                                            conversationSequenceId: card.conversationSequenceId,
                                            promptCacheKey: card.promptCacheKey,
                                            tab: "settings",
                                          });
                                        }}
                                      >
                                        <span
                                          className={cn(
                                            MANUAL_BINDING_BADGE_CLASS_NAME,
                                            manualBindingBadgeMeta.toneClassName,
                                          )}
                                        >
                                          <span className={MANUAL_BINDING_BADGE_TEXT_CLASS_NAME}>
                                            {manualBindingBadgeMeta.displayValue}
                                          </span>
                                        </span>
                                      </button>
                                    ) : (
                                      <span
                                        data-testid="dashboard-working-conversation-manual-binding-badge"
                                        className={cn(
                                          MANUAL_BINDING_BADGE_CLASS_NAME,
                                          manualBindingBadgeMeta.toneClassName,
                                        )}
                                        title={manualBindingBadgeMeta.accessibleLabel}
                                      >
                                        <span className={MANUAL_BINDING_BADGE_TEXT_CLASS_NAME}>
                                          {manualBindingBadgeMeta.displayValue}
                                        </span>
                                      </span>
                                    )
                                  ) : null}
                                </div>
                                <div className="flex shrink-0 items-center justify-end gap-2 whitespace-nowrap text-[10px] text-base-content/62">
                                  <span className="font-mono">{sortAnchorLabel}</span>
                                  {card.currentInvocation.livePhase ? (
                                    <InvocationPhaseBadge
                                      phase={card.currentInvocation.livePhase}
                                      appearance="inline"
                                      motion="dynamic"
                                      showLabel={false}
                                    />
                                  ) : (
                                    <InlineInvocationStatus
                                      meta={currentStatusMeta}
                                      label={currentStatusLabel}
                                      showLabel={false}
                                    />
                                  )}
                                </div>
                              </div>

                              <div className="mt-2">
                                <div className="grid grid-cols-3 gap-1.5">
                                  <SummaryMetric
                                    label={t("dashboard.workingConversations.requestCountLabel")}
                                    value={numberFormatter.format(card.requestCount)}
                                  />
                                  <SummaryMetric
                                    label={t("dashboard.workingConversations.totalTokensLabel")}
                                    value={numberFormatter.format(card.totalTokens)}
                                  />
                                  <SummaryMetric
                                    label={t("dashboard.workingConversations.totalCostLabel")}
                                    value={currencyFormatter.format(card.totalCost)}
                                  />
                                </div>
                              </div>

                              <div className="mt-2.5 space-y-1.5 sm:mt-3 sm:space-y-2">
                                <InvocationSlot
                                  invocation={card.currentInvocation}
                                  label={t("dashboard.workingConversations.currentInvocation")}
                                  slotKind="current"
                                  conversationSequenceId={card.conversationSequenceId}
                                  promptCacheKey={card.promptCacheKey}
                                  nowMs={nowMs}
                                  locale={locale}
                                  interactionsDisabled={selectionModeEnabled}
                                  onOpenUpstreamAccount={onOpenUpstreamAccount}
                                  onOpenInvocation={onOpenInvocation}
                                />
                                {card.previousInvocation ? (
                                  <InvocationSlot
                                    invocation={card.previousInvocation}
                                    label={t("dashboard.workingConversations.previousInvocation")}
                                    slotKind="previous"
                                    conversationSequenceId={card.conversationSequenceId}
                                    promptCacheKey={card.promptCacheKey}
                                    nowMs={nowMs}
                                    locale={locale}
                                    interactionsDisabled={selectionModeEnabled}
                                    onOpenUpstreamAccount={onOpenUpstreamAccount}
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
      <Dialog
        open={routeBindDialogOpen}
        onOpenChange={(nextOpen) => {
          if (bulkActionBusy == null) setRouteBindDialogOpen(nextOpen);
        }}
      >
        <DialogContent
          className="overflow-hidden p-0"
          data-testid="dashboard-working-conversations-route-bind-dialog"
        >
          <div className="dialog-chrome-surface border-b px-5 py-4 desktop:px-6">
            <DialogHeader>
              <DialogTitle>{routeBindDialogTitle}</DialogTitle>
              <DialogDescription>{routeBindDialogDescription}</DialogDescription>
            </DialogHeader>
          </div>
          <div className="space-y-4 px-5 py-5 desktop:px-6">
            <div className="rounded-xl border border-info/20 bg-info/8 px-3 py-2 text-sm text-base-content/84">
              {selectionSummaryLabel}
            </div>
            <div className="grid gap-3 sm:grid-cols-[4.75rem_minmax(0,0.82fr)_minmax(0,1.18fr)] sm:items-end">
              <span className="field-label flex h-8 items-center sm:pb-1">
                {locale === "zh" ? "绑定到" : "Bind to"}
              </span>
              <SelectField
                value={routeBindTargetKind}
                size="sm"
                aria-label={locale === "zh" ? "批量绑定目标类型" : "Bulk binding target kind"}
                data-testid="dashboard-working-conversations-route-bind-kind-select"
                options={[
                  {
                    value: "group",
                    label: locale === "zh" ? "分组" : "Group",
                    disabled: bindingTargets.groups.length === 0,
                  },
                  {
                    value: "upstreamAccount",
                    label: locale === "zh" ? "上游账号" : "Upstream account",
                    disabled: bindingTargets.accounts.length === 0,
                  },
                ]}
                onValueChange={(nextValue) => {
                  if (nextValue === "group" || nextValue === "upstreamAccount") {
                    setRouteBindTargetKind(nextValue);
                  }
                }}
              />
              {routeBindTargetKind === "group" ? (
                <SelectField
                  value={routeBindGroupName}
                  size="sm"
                  disabled={bindingTargets.loading || bindingTargets.groups.length === 0}
                  aria-label={locale === "zh" ? "批量分组绑定目标" : "Bulk group binding target"}
                  options={bindingTargets.groups.map((groupName) => ({
                    value: groupName,
                    label: groupName,
                  }))}
                  onValueChange={setRouteBindGroupName}
                />
              ) : (
                <SelectField
                  value={routeBindAccountId}
                  size="sm"
                  disabled={bindingTargets.loading || bindingTargets.accounts.length === 0}
                  aria-label={locale === "zh" ? "批量账号绑定目标" : "Bulk account binding target"}
                  options={bindingTargets.accounts.map((account) => ({
                    value: String(account.id),
                    label: conversationBindingAccountLabel(account),
                  }))}
                  onValueChange={setRouteBindAccountId}
                />
              )}
            </div>
            {bindingTargets.loading ? (
              <div className="flex items-center gap-2 rounded-xl border border-base-300/70 bg-base-200/45 px-3 py-3 text-sm text-base-content/72">
                <Spinner size="sm" aria-label={locale === "zh" ? "加载绑定目标" : "Loading"} />
                <span>{locale === "zh" ? "加载绑定目标中…" : "Loading binding targets..."}</span>
              </div>
            ) : null}
            {bindingTargets.error ? (
              <Alert variant="error">
                <div className="flex w-full items-center justify-between gap-3">
                  <span>{bindingTargets.error}</span>
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    onClick={() => void loadConversationBindingTargets()}
                  >
                    {locale === "zh" ? "重试" : "Retry"}
                  </Button>
                </div>
              </Alert>
            ) : null}
            {!bindingTargets.loading &&
            !bindingTargets.error &&
            bindingTargets.groups.length === 0 &&
            bindingTargets.accounts.length === 0 ? (
              <Alert variant="warning">
                <span>
                  {locale === "zh"
                    ? "当前没有可用于 conversation 绑定的分组或上游账号。"
                    : "No eligible groups or upstream accounts are currently available for conversation binding."}
                </span>
              </Alert>
            ) : null}
          </div>
          <DialogFooter className="dialog-chrome-surface border-t px-5 py-4 desktop:px-6">
            <Button
              type="button"
              variant="destructive"
              className="desktop:mr-auto"
              data-testid="dashboard-working-conversations-route-bind-clear-button"
              disabled={bulkActionBusy != null}
              onClick={() => {
                setRouteBindDialogOpen(false);
                setClearAffinityDialogOpen(true);
              }}
            >
              {locale === "zh" ? "清空绑定并重选" : "Clear and reselect"}
            </Button>
            <Button
              type="button"
              variant="ghost"
              disabled={bulkActionBusy != null}
              onClick={() => setRouteBindDialogOpen(false)}
            >
              {locale === "zh" ? "取消" : "Cancel"}
            </Button>
            <Button
              type="button"
              disabled={routeBindSubmitDisabled}
              onClick={() =>
                void applyBulkConversationAction(
                  routeBindTargetKind === "group"
                    ? {
                        action: "bind",
                        bindingKind: "group",
                        groupName: routeBindGroupName,
                      }
                    : {
                        action: "bind",
                        bindingKind: "upstreamAccount",
                        upstreamAccountId: Number(routeBindAccountId),
                      },
                )
              }
            >
              {bulkActionBusy === "bind"
                ? locale === "zh"
                  ? "保存中…"
                  : "Saving..."
                : locale === "zh"
                  ? "应用绑定"
                  : "Apply binding"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog
        open={clearAffinityDialogOpen}
        onOpenChange={(nextOpen) => {
          if (bulkActionBusy == null) setClearAffinityDialogOpen(nextOpen);
        }}
      >
        <DialogContent
          role="alertdialog"
          className="overflow-hidden p-0"
          data-testid="dashboard-working-conversations-clear-affinity-dialog"
        >
          <div className="dialog-chrome-surface border-b px-5 py-4 desktop:px-6">
            <DialogHeader>
              <DialogTitle>{clearAffinityDialogTitle}</DialogTitle>
              <DialogDescription>{clearAffinityDialogDescription}</DialogDescription>
            </DialogHeader>
          </div>
          <div className="space-y-4 px-5 py-5 desktop:px-6">
            <div className="destructive-callout-surface rounded-[1.1rem] px-4 py-4">
              <div className="flex w-full items-start gap-3">
                <div className="destructive-callout-icon mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-full">
                  <AppIcon name="alert-circle-outline" className="h-4.5 w-4.5" aria-hidden />
                </div>
                <div className="min-w-0 flex-1 space-y-3">
                  <div className="space-y-1">
                    <p className="text-sm font-semibold text-base-content">
                      {clearAffinityCalloutTitle}
                    </p>
                    <p className="text-sm leading-6 text-base-content/74">
                      {clearAffinityCalloutDescription}
                    </p>
                  </div>
                  <ul className="destructive-callout-list overflow-hidden rounded-[0.95rem]">
                    {clearAffinityCalloutItems.map((item) => (
                      <li
                        key={item.key}
                        className="destructive-callout-item flex items-start gap-3 px-3 py-2.5"
                      >
                        <span
                          className="destructive-callout-bullet mt-1.5 h-1.5 w-1.5 shrink-0 rounded-full"
                          aria-hidden
                        />
                        <span className="min-w-0">
                          <span className="block text-sm font-medium leading-5 text-base-content/86">
                            {item.label}
                          </span>
                          <span className="block text-xs leading-5 text-base-content/56">
                            {item.detail}
                          </span>
                        </span>
                      </li>
                    ))}
                  </ul>
                </div>
              </div>
            </div>
          </div>
          <div className="dialog-chrome-surface flex w-full items-center justify-between gap-3 border-t px-5 py-4 desktop:px-6">
            <p className="min-w-0 flex-1 text-sm font-medium text-base-content/76">
              {selectionSummaryLabel}
            </p>
            <div className="flex shrink-0 items-center gap-3">
              <Button
                type="button"
                variant="ghost"
                disabled={bulkActionBusy != null}
                onClick={() => setClearAffinityDialogOpen(false)}
              >
                {locale === "zh" ? "取消" : "Cancel"}
              </Button>
              <Button
                type="button"
                variant="destructive"
                disabled={bulkActionBusy != null}
                onClick={() =>
                  void applyBulkConversationAction({
                    action: "clearAndResetAffinity",
                  })
                }
              >
                {bulkActionBusy === "clearAndResetAffinity"
                  ? locale === "zh"
                    ? "处理中…"
                    : "Applying..."
                  : locale === "zh"
                    ? "确认清空"
                    : "Confirm clear"}
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>
      {conversationBulkActionPanel && typeof document !== "undefined" && document.body
        ? createPortal(conversationBulkActionPanel, document.body)
        : conversationBulkActionPanel}
    </section>
  );
}
