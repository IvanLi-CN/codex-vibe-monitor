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
import { BubblePopoverContent } from "../../components/ui/bubble-popover";
import { Button } from "../../components/ui/button";
import { type CategoricalChipTone, Chip } from "../../components/ui/chip";
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
import {
  resolveUpstreamAccountRecentPreviewLimit,
  useDashboardUpstreamAccountActivity,
} from "../../hooks/useDashboardUpstreamAccountActivity";
import {
  DASHBOARD_WORKING_CONVERSATIONS_RECENT_PREVIEW_MAX,
  type DashboardWorkingConversationsBlockedBindingFilter,
} from "../../hooks/useDashboardWorkingConversations";
import type { TranslationKey } from "../../i18n";
import { useTranslation } from "../../i18n";
import type {
  PromptCacheConversationRewriteMode,
  UpstreamAccountActivityAccount,
  UpstreamAccountActivityResponse,
  UpstreamAccountGroupSummary,
  UpstreamAccountSummary,
} from "../../lib/api";
import { bulkUpdatePromptCacheConversationBindings, fetchUpstreamAccounts } from "../../lib/api";
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
} from "../../lib/invocation";
import {
  compactUpstreamPlanLabel,
  shouldShowUpstreamPlanChip,
  upstreamPlanChipRecipe,
} from "../../lib/upstreamAccountChips";
import { cn } from "../../lib/utils";
import {
  InvocationModelContextCluster,
  InvocationReasoningEffortChip,
} from "../invocations/InvocationModelContextCluster";
import { InvocationPhaseChip } from "../invocations/InvocationPhaseChip";
import {
  buildInvocationDetailViewModel,
  FALLBACK_CELL,
  INVOCATION_ACCOUNT_ROUTING_IN_PROGRESS_CLASS_NAME,
  renderEndpointSummary,
  renderFastIndicator,
  renderInvocationModelChip,
} from "../invocations/invocation-details-shared";
import { renderInvocationTransportChip } from "../invocations/invocation-transport-chip";
import { AppIcon } from "../shared/AppIcon";
import { ModelIdentity, resolveModelIdentityIcon } from "../shared/ModelIdentity";
import { DashboardNetworkRecentPopover } from "./DashboardNetworkRecentPopover";
import { DashboardNetworkSpeedCapsule } from "./DashboardNetworkSpeedCapsule";
import { DashboardUpstreamAccountActivityCard } from "./DashboardWorkingAccountActivityCard";
import {
  buildInvocationSummaryFields,
  invocationCacheHitRate,
  renderInvocationSummaryFields,
} from "./DashboardWorkingAccountMetrics";
import { DashboardWorkingConversationGrid as DashboardWorkingConversationGridExternal } from "./DashboardWorkingConversationGrid";
import { CompactLatencyPills } from "./DashboardWorkingConversationLatency";
import {
  DASHBOARD_WORKSPACE_VIEW_STORAGE_KEY,
  type DashboardActivityRangeKey,
  type DashboardWorkspaceView,
  persistDashboardWorkspaceView,
  readPersistedDashboardWorkspaceView,
} from "./dashboardActivityRange";
import {
  DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_MAX,
  type DashboardBulkRouteBindRecentTarget,
  filterAvailableDashboardBulkRouteBindRecentTargets,
  isDashboardBulkRouteBindRecentTargetSelected,
  persistDashboardBulkRouteBindRecentTargets,
  readDashboardBulkRouteBindRecentTargets,
  rememberDashboardBulkRouteBindRecentTarget,
} from "./dashboardBulkRouteBindPreferences";
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

export interface DashboardOpenUpstreamAccountOptions {
  tab?: "overview" | "routing" | "healthEvents";
}

export interface DashboardWorkingConversationsSectionProps {
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
  activeBlockedBindingFilter?: DashboardWorkingConversationsBlockedBindingFilter | null;
  onClearBlockedBindingFilter?: () => void;
}

function readBrowserOfflineState() {
  return typeof navigator === "undefined" ? false : !navigator.onLine;
}

export interface DashboardWorkingConversationSelection {
  conversationSequenceId: string;
  promptCacheKey: string;
  tab?: "overview" | "calls" | "settings";
}

export const ACCOUNT_CARD_CLASS_NAME =
  "flex min-w-0 w-full max-w-full flex-col overflow-hidden rounded-[1rem] border border-[rgba(148,163,184,0.32)] bg-base-100/72 p-4 shadow-[0_6px_12px_rgba(15,23,42,0.07)]";
const ACCOUNT_CARD_SKELETON_CLASS_NAME = `${ACCOUNT_CARD_CLASS_NAME} h-full desktop1660:min-h-[31.5rem]`;

export const ACCOUNT_CARD_INNER_BORDER_CLASS_NAME = "border-[rgba(148,163,184,0.22)]";
export const ACCOUNT_CARD_INNER_RING_CLASS_NAME = "ring-[rgba(148,163,184,0.22)]";
export const DASHBOARD_RECENT_SKELETON_IDS = Array.from(
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
const ROUTE_BIND_RECENT_TARGET_GAP_PX = 8;
const ROUTE_BIND_RECENT_TARGET_MAX_ROWS = 2;

export type DashboardManualBindingChipMeta = {
  displayValue: string;
  accessibleLabel: string;
  tone: "info" | "secondary";
};

type DashboardConversationBulkBindTargetKind = "group" | "upstreamAccount";
type DashboardConversationClearDialogAction = "bind" | "clearAndResetAffinity";

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

function dashboardBulkRouteBindRecentTargetsEqual(
  left: DashboardBulkRouteBindRecentTarget[],
  right: DashboardBulkRouteBindRecentTarget[],
) {
  if (left.length !== right.length) {
    return false;
  }
  return left.every((target, index) => {
    const other = right[index];
    if (!other || target.kind !== other.kind || target.usedAt !== other.usedAt) {
      return false;
    }
    return target.kind === "group"
      ? other.kind === "group" && target.groupName === other.groupName
      : other.kind === "upstreamAccount" && target.upstreamAccountId === other.upstreamAccountId;
  });
}

function hasMultiSelectModifier(
  event: Pick<ReactMouseEvent<HTMLElement>, "metaKey" | "ctrlKey" | "button">,
) {
  return event.button === 0 && (event.metaKey || event.ctrlKey);
}

export function resolveDashboardManualBindingChipMeta(
  binding: DashboardWorkingConversationCardModel["manualBinding"],
  t: (key: TranslationKey, params?: Record<string, string | number>) => string,
): DashboardManualBindingChipMeta | null {
  if (!binding) return null;
  if (binding.bindingKind === "group") {
    const groupName = binding.groupName?.trim();
    if (!groupName) return null;
    return {
      displayValue: groupName,
      accessibleLabel: t("live.conversations.drawer.binding.currentGroup", {
        group: groupName,
      }),
      tone: "info",
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
    tone: "secondary",
  };
}

function blockedBindingConstraintSourceLabel(
  source: string | null | undefined,
  locale: "zh" | "en",
) {
  if (source === "encryptedSessionOwner") {
    return locale === "zh" ? "加密 owner 约束" : "encrypted owner lock";
  }
  if (source === "upstreamAccountBinding") {
    return locale === "zh" ? "显式上游账号绑定" : "explicit upstream-account binding";
  }
  return locale === "zh" ? "单账号约束" : "single-account binding";
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

export type StatusMeta = {
  chipTone: "primary" | "secondary" | "success" | "warning" | "error" | "info";
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

export const CARD_CLASS_NAME =
  "relative min-w-0 overflow-hidden rounded-[1.1rem] p-2 sm:p-3 shadow-[inset_0_1px_0_rgba(255,255,255,0.04),0_16px_28px_rgba(2,6,23,0.18)] transition-shadow duration-200 hover:shadow-[inset_0_1px_0_rgba(255,255,255,0.05),0_20px_34px_rgba(2,6,23,0.22)] focus-within:shadow-[inset_0_1px_0_rgba(255,255,255,0.06),0_0_0_1px_rgba(56,189,248,0.2),0_20px_34px_rgba(2,6,23,0.22)]";

const SLOT_CLASS_NAME =
  "flex min-h-[57px] min-w-0 flex-col overflow-hidden rounded-[0.95rem] px-2.5 py-2 shadow-[inset_0_1px_0_rgba(255,255,255,0.04)]";

const CARD_SURFACE_CLASS_NAME = "working-conversation-card-surface";

const INVOCATION_SURFACE_CLASS_NAME = "working-conversation-slot-surface";
export const DASHBOARD_WORKING_CONVERSATION_ROW_GAP_PX = 16;
export const UPSTREAM_ACCOUNT_RECENT_COMPACT_BADGE_CLASS_NAME =
  "max-w-full px-2 font-semibold shadow-none";

export const ACCOUNT_HEADER_BADGE_CLASS_NAME = "font-semibold";
export const ACCOUNT_CARD_STACKED_HEADER_BREAKPOINT_PX = 620;
export const ACCOUNT_CARD_HERO_SINGLE_COLUMN_BREAKPOINT_PX = 300;
export const ACCOUNT_CARD_HERO_TWO_COLUMN_BREAKPOINT_PX = 760;
export const ACCOUNT_CARD_RECENT_STACK_BREAKPOINT_PX = 520;
export const ACCOUNT_CARD_RECENT_DETAILS_SPLIT_BREAKPOINT_PX = 640;

const UPSTREAM_ACCOUNT_RECENT_IDENTITY_TONES: CategoricalChipTone[] = [
  "sky",
  "cyan",
  "blue",
  "violet",
  "indigo",
  "fuchsia",
  "teal",
  "emerald",
];

export function resolveConversationIdentityTone(seed: string): CategoricalChipTone {
  const hash = hashDashboardWorkingConversationKey(seed);
  const hashValue = Number.parseInt(hash, 16) >>> 0;
  const mixedHash = (hashValue ^ (hashValue >>> 7) ^ (hashValue >>> 13) ^ (hashValue >>> 21)) >>> 0;
  const toneIndex = mixedHash % UPSTREAM_ACCOUNT_RECENT_IDENTITY_TONES.length;
  return UPSTREAM_ACCOUNT_RECENT_IDENTITY_TONES[toneIndex];
}

const STATUS_META: Record<DashboardWorkingConversationTone, StatusMeta> = {
  running: {
    chipTone: "primary",
    icon: "loading",
    labelKey: "table.status.running",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
  },
  pending: {
    chipTone: "warning",
    icon: "timer-refresh-outline",
    labelKey: "table.status.pending",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
  },
  success: {
    chipTone: "success",
    icon: "check-circle-outline",
    labelKey: "table.status.success",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
  },
  warning: {
    chipTone: "warning",
    icon: "alert-outline",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
  },
  error: {
    chipTone: "error",
    icon: "alert-circle-outline",
    labelKey: "table.status.failed",
    cardToneClassName: CARD_SURFACE_CLASS_NAME,
    slotSurfaceClassName: INVOCATION_SURFACE_CLASS_NAME,
  },
  neutral: {
    chipTone: "secondary",
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

export function DashboardImageToolIconChip({
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
    !imageIntentDisplay.showsChip ||
    imageIntentDisplay.chipTone == null ||
    imageIntentDisplay.badgeLabelKey == null
  ) {
    return null;
  }

  const label = t(imageIntentDisplay.badgeLabelKey);

  return (
    <Chip
      tone={imageIntentDisplay.chipTone}
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
    </Chip>
  );
}

export function renderUpstreamAccountRecentModelDisplay(
  hasMismatch: boolean,
  modelValue: string,
  requestModelValue: string,
  responseModelValue: string,
  t: ReturnType<typeof useTranslation>["t"],
) {
  const shouldRenderMismatch =
    hasMismatch && requestModelValue !== FALLBACK_CELL && responseModelValue !== FALLBACK_CELL;

  if (!shouldRenderMismatch) {
    return renderInvocationModelChip(modelValue, {
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
      <ModelIdentity
        model={requestModelValue}
        className="min-w-0 max-w-full"
        textClassName="truncate font-mono leading-none text-base-content/84"
        title={requestModelValue}
      />
      <span
        className="inline-flex h-4 w-4 flex-none items-center justify-center text-base-content/55"
        aria-label={t("table.model.routingMismatchAria")}
        data-testid="dashboard-upstream-account-recent-model-routing-indicator"
        role="img"
      >
        <AppIcon name="compare-horizontal" className="h-3 w-3" aria-hidden />
      </span>
      <ModelIdentity
        model={responseModelValue}
        className="min-w-0 max-w-full"
        textClassName="truncate font-mono leading-none text-base-content/88"
        title={responseModelValue}
      />
    </div>
  );
}

function CompactAccountPlanChip({ planType }: { planType: string | null }) {
  if (!shouldShowUpstreamPlanChip(planType)) return null;
  const label = compactUpstreamPlanLabel(planType);
  if (!label) return null;
  const recipe = upstreamPlanChipRecipe(planType);

  return (
    <Chip
      tone={recipe?.tone ?? "secondary"}
      data-testid="dashboard-working-conversation-account-plan"
      data-plan={recipe?.dataPlan}
      className="h-4 shrink-0 px-1.5 py-0 text-[7.5px] font-semibold leading-none"
      title={planType ?? undefined}
    >
      {label}
    </Chip>
  );
}

export function resolveStatusMeta(
  tone: DashboardWorkingConversationTone,
  status: string,
): StatusMeta {
  const base = STATUS_META[tone];
  const normalized = status.trim().toLowerCase();
  if (normalized === "warning_success") {
    return {
      ...base,
      chipTone: "warning",
      icon: "alert-outline",
      labelKey: "table.status.warningSuccess",
    };
  }
  if (normalized === "interrupted") {
    return {
      ...base,
      chipTone: "error",
      icon: "alert-circle-outline",
      labelKey: "table.status.interrupted",
    };
  }
  if (normalized.startsWith("http_4")) {
    return {
      ...base,
      chipTone: "warning",
      icon: "alert-outline",
      label: formatStatusLabel(status) ?? status,
    };
  }
  if (normalized.startsWith("http_5")) {
    return {
      ...base,
      chipTone: "error",
      icon: "alert-circle-outline",
      label: formatStatusLabel(status) ?? status,
    };
  }
  return base;
}

function statusInlineToneClassName(variant: StatusMeta["chipTone"]) {
  if (variant === "success") return "text-success";
  if (variant === "warning") return "text-warning";
  if (variant === "error") return "text-error";
  if (variant === "info") return "text-info";
  if (variant === "primary") return "text-primary";
  return "text-base-content/62";
}

function buildStatusAssistiveLabel(label: string, detail?: string | null) {
  const resolvedDetail = detail?.trim();
  if (!resolvedDetail) return label;
  return `${label} · ${resolvedDetail}`;
}

export function InlineInvocationStatus({
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
  const toneClassName = statusInlineToneClassName(meta.chipTone);
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

export function SummaryMetric({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="grid min-w-0 grid-cols-[auto_minmax(0,1fr)] items-baseline gap-1 rounded-[0.65rem] bg-base-100/4 px-1 py-0.5 sm:px-2 sm:py-1">
      <span className="truncate text-[8px] font-semibold text-base-content/48 sm:text-[7.5px]">
        {label}
      </span>
      <span className="min-w-0 truncate text-right font-mono text-[10.5px] font-semibold text-base-content sm:text-[10px]">
        {value}
      </span>
    </div>
  );
}

export function PlaceholderSlot({ slotKind }: { slotKind: "previous" | "earlier" }) {
  const { t } = useTranslation();
  const visibleLabel =
    slotKind === "earlier"
      ? t("dashboard.workingConversations.earlierPlaceholder")
      : t("dashboard.workingConversations.previousPlaceholder");
  const accessibleLabel =
    slotKind === "earlier"
      ? t("dashboard.workingConversations.earlierPlaceholderAccessible")
      : t("dashboard.workingConversations.previousPlaceholderAccessible");

  return (
    <fieldset
      data-testid="dashboard-working-conversation-placeholder"
      data-slot-kind={slotKind}
      aria-label={accessibleLabel}
      className={cn(SLOT_CLASS_NAME, INVOCATION_SURFACE_CLASS_NAME, "justify-center")}
    >
      <div className="flex min-w-0 items-center gap-1.5 text-[10px] font-semibold text-base-content/62">
        <AppIcon name="timer-outline" className="h-3.5 w-3.5 shrink-0" aria-hidden />
        <span data-testid="dashboard-working-conversation-placeholder-label" className="truncate">
          {visibleLabel}
        </span>
      </div>
    </fieldset>
  );
}

export function InvocationSlot({
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
  slotKind: "current" | "previous" | "earlier";
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
    [interactionsDisabled, onOpenUpstreamAccount],
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

  const fastIndicator = renderFastIndicator(viewModel.fastIndicatorState, t);
  const shouldGroupModelContext =
    !viewModel.modelHasMismatch && resolveModelIdentityIcon(viewModel.modelValue) != null;
  const modelContextTitle =
    shouldGroupModelContext && viewModel.reasoningEffortValue === FALLBACK_CELL
      ? viewModel.modelValue
      : `${viewModel.modelValue} · ${viewModel.reasoningEffortValue}`;
  const displayConversationSequenceId =
    formatDashboardWorkingConversationSequenceId(conversationSequenceId);
  const usageSummaryFields = useMemo(
    () =>
      buildInvocationSummaryFields({
        cacheHitRate: invocationCacheHitRate(invocation.record),
        totalTokensValue: viewModel.totalTokensValue,
        cost: invocation.record.cost,
        costValue: viewModel.costValue,
        localeTag,
        testIdPrefix: "dashboard-working-conversation-usage",
      }),
    [invocation.record, localeTag, viewModel.costValue, viewModel.totalTokensValue],
  );
  const fastAccessibleLabel =
    viewModel.fastIndicatorState === "effective"
      ? t("table.model.fastPriorityAria")
      : viewModel.fastIndicatorState === "requested_only"
        ? t("table.model.fastRequestedOnlyAria")
        : null;
  const invocationActionLabel = [
    t("dashboard.workingConversations.openInvocation"),
    label,
    displayConversationSequenceId,
    ...(shouldGroupModelContext ? [modelContextTitle, fastAccessibleLabel] : []),
    invocation.record.invokeId,
  ]
    .filter(Boolean)
    .join(" · ");

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

  return (
    <div
      data-testid="dashboard-working-conversation-slot"
      data-slot-kind={slotKind}
      className={cn(
        SLOT_CLASS_NAME,
        statusMeta.slotSurfaceClassName,
        interactionsDisabled
          ? "transition-colors duration-200"
          : "cursor-pointer transition-colors duration-200 hover:bg-base-100/10",
      )}
    >
      <button
        type="button"
        disabled={interactionsDisabled}
        aria-label={interactionsDisabled ? undefined : invocationActionLabel}
        className="block w-full appearance-none border-0 bg-transparent p-0 text-left font-inherit disabled:cursor-default focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary grid min-h-5 min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-x-0 gap-y-1"
        onClick={(event) => {
          event.stopPropagation();
          handleOpenInvocation();
        }}
        data-testid="dashboard-working-conversation-slot-header"
      >
        <div className="flex min-w-0 items-center gap-1 overflow-hidden">
          <div
            data-testid="dashboard-working-conversation-slot-time"
            className="shrink-0 font-mono text-[10px] text-base-content/72"
          >
            {occurredAtShortLabel}
          </div>
          <div
            data-testid="dashboard-working-conversation-slot-model"
            className="min-w-0 max-w-full flex-1 truncate text-[9.5px] font-semibold text-base-content/76"
            title={modelContextTitle}
          >
            {shouldGroupModelContext ? (
              <InvocationModelContextCluster
                modelValue={viewModel.modelValue}
                reasoningEffortValue={viewModel.reasoningEffortValue}
                fastIndicatorState={viewModel.fastIndicatorState}
                grouped
                t={t}
                className="min-w-0 max-w-full"
                testId="dashboard-working-conversation-model-context"
                modelTestId="dashboard-working-conversation-model-name"
              />
            ) : (
              <div className="flex min-w-0 flex-1 items-center gap-0.5">
                <span
                  data-testid="dashboard-working-conversation-model-name"
                  className="min-w-0 max-w-full shrink truncate whitespace-nowrap"
                >
                  {renderInvocationModelChip(viewModel.modelValue, {
                    t,
                    hasMismatch: viewModel.modelHasMismatch,
                    className: "max-w-full",
                    textClassName: "font-mono",
                    iconClassName: "h-3 w-3",
                    testId: "dashboard-working-conversation-model",
                  })}
                </span>
                <span className="shrink-0 text-base-content/28">·</span>
                <InvocationReasoningEffortChip
                  value={viewModel.reasoningEffortValue}
                  testId="dashboard-working-conversation-reasoning-effort"
                  className="h-4 min-h-4 max-w-[4rem] shrink-0 px-1 py-0 text-[8.5px]"
                />
                {fastIndicator ? (
                  <>
                    <span className="shrink-0 text-base-content/28">·</span>
                    {fastIndicator}
                  </>
                ) : null}
              </div>
            )}
          </div>
        </div>
        <div
          data-testid="dashboard-working-conversation-slot-readings"
          className="flex min-w-0 flex-nowrap items-center justify-end gap-1.5"
        >
          <div className="flex min-w-0 shrink items-center justify-end gap-0">
            {invocation.livePhase ? (
              <InvocationPhaseChip
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
            {renderInvocationTransportChip(invocation.record, "h-5 px-1.5 text-[9.5px]")}
            <div className="flex h-5 shrink items-center">
              <div className="flex items-center gap-0">
                {renderEndpointSummary(
                  viewModel.endpointDisplay,
                  t,
                  "h-5 px-1 py-0 text-[9px] font-semibold leading-none shadow-none",
                )}
                <DashboardImageToolIconChip
                  endpointDisplay={viewModel.endpointDisplay}
                  imageIntentDisplay={viewModel.imageIntentDisplay}
                  t={t}
                />
              </div>
            </div>
          </div>
          <CompactLatencyPills
            invocation={invocation}
            nowMs={nowMs}
            localeTag={localeTag}
            t={t}
            className="shrink-0 flex-nowrap text-[10px]"
          />
        </div>
      </button>

      <div className="mt-1.5 space-y-1">
        <div
          data-testid="dashboard-working-conversation-account-line"
          className="grid min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-x-2 text-[9.5px] leading-[1.3] text-base-content"
        >
          <div className="flex min-w-0 items-center gap-1.5 font-mono font-semibold">
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
            <CompactAccountPlanChip planType={viewModel.accountPlanType} />
          </div>
          <div
            className="min-w-0 whitespace-nowrap text-right font-mono text-[9.5px] font-semibold text-base-content/74"
            title={`${t("table.column.inputTokens")}: ${viewModel.inputTokensValue} · Cache write: ${viewModel.cacheWriteTokensValue} · ${t("table.column.cacheInputTokens")}: ${viewModel.cacheInputTokensValue} · ${t("table.column.outputTokens")}: ${viewModel.outputTokensValue} · ${t("table.column.totalTokens")}: ${viewModel.totalTokensValue} · ${t("table.column.costUsd")}: ${viewModel.costValue} · ${t("table.details.reasoningTokens")}: ${viewModel.reasoningTokensValue}`}
          >
            <div data-testid="dashboard-working-conversation-usage-line">
              {renderInvocationSummaryFields(usageSummaryFields)}
            </div>
          </div>
        </div>

        {viewModel.collapsedErrorSummary ? (
          <InvocationErrorSummary
            className="max-w-full"
            textClassName="text-[9.5px] text-error"
            message={viewModel.collapsedErrorSummary}
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

function DashboardUpstreamAccountGridSkeleton() {
  return (
    <div
      data-testid="dashboard-upstream-account-grid-skeleton"
      className="grid grid-cols-1 gap-4 desktop1660:grid-cols-[repeat(2,minmax(0,1fr))]"
      aria-busy="true"
    >
      {DASHBOARD_ACCOUNT_SKELETON_IDS.map((cardId) => (
        <div key={cardId} className={ACCOUNT_CARD_SKELETON_CLASS_NAME}>
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
  activeBlockedBindingFilter,
  onClearBlockedBindingFilter,
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
  const [routeBindSelectionRestorePending, setRouteBindSelectionRestorePending] = useState(false);
  const [clearBindingDialogOpen, setClearBindingDialogOpen] = useState(false);
  const [clearBindingDialogAction, setClearBindingDialogAction] =
    useState<DashboardConversationClearDialogAction>("bind");
  const [fastModePopoverOpen, setFastModePopoverOpen] = useState(false);
  const [routeBindTargetKind, setRouteBindTargetKind] =
    useState<DashboardConversationBulkBindTargetKind>("group");
  const [routeBindGroupName, setRouteBindGroupName] = useState("");
  const [routeBindAccountId, setRouteBindAccountId] = useState("");
  const [routeBindRecentTargets, setRouteBindRecentTargets] = useState<
    DashboardBulkRouteBindRecentTarget[]
  >(() => readDashboardBulkRouteBindRecentTargets());
  const routeBindRecentTargetsMeasureRef = useRef<HTMLDivElement | null>(null);
  const routeBindRecentChipMeasureRefs = useRef(new Map<string, HTMLElement>());
  const [routeBindRecentVisibleCount, setRouteBindRecentVisibleCount] = useState(
    DASHBOARD_BULK_ROUTE_BIND_RECENT_TARGETS_MAX,
  );
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
  const availableRouteBindRecentTargets = useMemo(
    () =>
      bindingTargets.loaded
        ? filterAvailableDashboardBulkRouteBindRecentTargets(routeBindRecentTargets, {
            groups: bindingTargets.groups,
            accounts: bindingTargets.accounts,
          })
        : [],
    [bindingTargets.accounts, bindingTargets.groups, bindingTargets.loaded, routeBindRecentTargets],
  );
  const setGridContainerRef = useCallback((node: HTMLDivElement | null) => {
    setGridElement(node);
  }, []);
  const hasInFlightCards = cards.some(
    (card) =>
      card.currentInvocation.isInFlight ||
      card.previousInvocation?.isInFlight === true ||
      card.earlierInvocation?.isInFlight === true,
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
    setClearBindingDialogOpen(false);
    setClearBindingDialogAction("bind");
    setFastModePopoverOpen(false);
  }, []);
  const resetConversationSelectionState = useCallback(() => {
    setSelectionModeEnabled(false);
    setSelectedPromptCacheKeys([]);
    setBulkFeedback(null);
    closeConversationBulkDialogs();
  }, [closeConversationBulkDialogs]);
  const blockedBindingFilterActive = activeBlockedBindingFilter != null;
  const upstreamAccountsDisabled = activeRange === "usage";
  const activeView: DashboardWorkspaceView = blockedBindingFilterActive
    ? "conversations"
    : upstreamAccountsDisabled && preferredView === "upstreamAccounts"
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
    ? (externalUpstreamAccountRecentPreviewLimit ??
      resolveUpstreamAccountRecentPreviewLimit(externalUpstreamAccountActivity?.accounts ?? []))
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
  const hasInFlightUpstreamAccountRecent = useMemo(
    () =>
      upstreamAccounts.some((account) =>
        account.recentInvocations.some(
          (preview) => buildDashboardWorkingConversationInvocationModel(preview).isInFlight,
        ),
      ),
    [upstreamAccounts],
  );
  const hasLiveTimingRows = hasInFlightCards || hasInFlightUpstreamAccountRecent;
  const totalNetworkSpeed = useMemo(
    () => ({
      uploadBytesPerSecond: Math.max(
        0,
        upstreamAccountActivity?.networkRealtimeRate?.uploadBytesPerSecond ?? 0,
      ),
      downloadBytesPerSecond: Math.max(
        0,
        upstreamAccountActivity?.networkRealtimeRate?.downloadBytesPerSecond ?? 0,
      ),
    }),
    [upstreamAccountActivity?.networkRealtimeRate],
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
  const countChipValue = totalMatched ?? cards.length;
  const accountCountChipValue = upstreamAccounts.length;
  const countChipLabel =
    activeView === "conversations"
      ? t("dashboard.workingConversations.countBadge", {
          count: countChipValue,
        })
      : showUpstreamAccountActivityLoading && upstreamAccounts.length === 0
        ? t("dashboard.upstreamAccounts.countLoading")
        : t("dashboard.upstreamAccounts.countBadge", {
            count: accountCountChipValue,
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
    if (!routeBindDialogOpen) {
      setRouteBindSelectionRestorePending(false);
    }
  }, [routeBindDialogOpen]);

  useEffect(() => {
    if (
      !routeBindDialogOpen ||
      !routeBindSelectionRestorePending ||
      bindingTargets.loading ||
      !bindingTargets.loaded
    ) {
      return;
    }
    if (
      !dashboardBulkRouteBindRecentTargetsEqual(
        routeBindRecentTargets,
        availableRouteBindRecentTargets,
      )
    ) {
      setRouteBindRecentTargets(availableRouteBindRecentTargets);
      persistDashboardBulkRouteBindRecentTargets(availableRouteBindRecentTargets);
    }
    const mostRecentTarget = availableRouteBindRecentTargets[0];
    if (mostRecentTarget?.kind === "group") {
      setRouteBindTargetKind("group");
      setRouteBindGroupName(mostRecentTarget.groupName);
    } else if (mostRecentTarget?.kind === "upstreamAccount") {
      setRouteBindTargetKind("upstreamAccount");
      setRouteBindAccountId(String(mostRecentTarget.upstreamAccountId));
    }
    setRouteBindSelectionRestorePending(false);
  }, [
    availableRouteBindRecentTargets,
    bindingTargets.loaded,
    bindingTargets.loading,
    routeBindDialogOpen,
    routeBindRecentTargets,
    routeBindSelectionRestorePending,
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
            bindingKind: "none";
          }
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
          if (payload.action === "bind" && payload.bindingKind !== "none") {
            setRouteBindRecentTargets((current) => {
              const next =
                payload.bindingKind === "group"
                  ? rememberDashboardBulkRouteBindRecentTarget(current, {
                      kind: "group",
                      groupName: payload.groupName,
                    })
                  : rememberDashboardBulkRouteBindRecentTarget(current, {
                      kind: "upstreamAccount",
                      upstreamAccountId: payload.upstreamAccountId,
                    });
              persistDashboardBulkRouteBindRecentTargets(next);
              return next;
            });
          }
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
    if (!hasLiveTimingRows) return;
    setNowMs(Date.now());
    const timer = window.setInterval(() => {
      setNowMs(Date.now());
    }, 1000);
    return () => window.clearInterval(timer);
  }, [hasLiveTimingRows]);

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
  }, [activeView, gridElement, hasMore, isLoadingMore, onLoadMore]);

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
  }, [gridElement, visibleAnchorTarget]);

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
  const blockedBindingBannerMeta = useMemo(() => {
    if (!activeBlockedBindingFilter) return null;
    const diagnostic =
      cards.find(
        (card) =>
          card.currentInvocation.preview.blockedBinding != null ||
          card.previousInvocation?.preview.blockedBinding != null ||
          card.earlierInvocation?.preview.blockedBinding != null,
      )?.currentInvocation.preview.blockedBinding ??
      cards.find((card) => card.previousInvocation?.preview.blockedBinding != null)
        ?.previousInvocation?.preview.blockedBinding ??
      cards.find((card) => card.earlierInvocation?.preview.blockedBinding != null)
        ?.earlierInvocation?.preview.blockedBinding ??
      null;
    const upstreamAccountId = activeBlockedBindingFilter.upstreamAccountId ?? null;
    const accountLabel =
      diagnostic?.upstreamAccountLabel?.trim() ||
      (upstreamAccountId != null
        ? `#${upstreamAccountId}`
        : locale === "zh"
          ? "目标账号"
          : "target account");
    const constraintSource =
      diagnostic?.constraintSource ?? activeBlockedBindingFilter.constraintSource ?? null;
    const sourceLabel = blockedBindingConstraintSourceLabel(constraintSource, locale);
    return {
      accountLabel,
      sourceLabel,
      title: locale === "zh" ? "单账号会话约束阻塞" : "Single-account conversation binding blocked",
      description:
        locale === "zh"
          ? `这些会话被 ${sourceLabel} 固定在 ${accountLabel}，该账号当前不可选，所以需要先清空绑定并重选。`
          : `These conversations are pinned to ${accountLabel} by ${sourceLabel}, and that account is currently not selectable. Clear the binding before rerouting.`,
    };
  }, [activeBlockedBindingFilter, cards, locale]);
  const routeBindDialogTitle = locale === "zh" ? "批量路由绑定" : "Bulk route binding";
  const routeBindDialogDescription =
    locale === "zh"
      ? "支持批量绑定到分组或上游账号；如果要清空手工绑定，可在此弹窗底部直接进入确认。"
      : "Bind the selected conversations to a group or upstream account. If you need to clear manual bindings instead, use the destructive action shortcut in this dialog footer.";
  const applyRouteBindRecentTargetSelection = useCallback(
    (target: DashboardBulkRouteBindRecentTarget) => {
      if (target.kind === "group") {
        setRouteBindTargetKind("group");
        setRouteBindGroupName(target.groupName);
        return;
      }
      setRouteBindTargetKind("upstreamAccount");
      setRouteBindAccountId(String(target.upstreamAccountId));
    },
    [],
  );
  const routeBindRecentChipItems = useMemo(() => {
    if (!bindingTargets.loaded) {
      return [];
    }
    return availableRouteBindRecentTargets.map((target) => {
      const matchedAccount =
        target.kind === "upstreamAccount"
          ? bindingTargets.accounts.find((account) => account.id === target.upstreamAccountId)
          : null;
      const typeLabel =
        target.kind === "group"
          ? locale === "zh"
            ? "分组"
            : "Group"
          : locale === "zh"
            ? "账号"
            : "Account";
      const displayLabel =
        target.kind === "group"
          ? target.groupName
          : matchedAccount
            ? conversationBindingAccountLabel(matchedAccount)
            : `#${target.upstreamAccountId}`;
      const active = isDashboardBulkRouteBindRecentTargetSelected(target, {
        kind: routeBindTargetKind,
        groupName: routeBindGroupName,
        upstreamAccountId: routeBindAccountId,
      });
      return {
        key:
          target.kind === "group"
            ? `group:${target.groupName}`
            : `upstreamAccount:${target.upstreamAccountId}`,
        target,
        active,
        typeLabel,
        displayLabel,
        title:
          locale === "zh"
            ? `最近使用的${typeLabel}：${displayLabel}`
            : `Recent ${typeLabel.toLowerCase()}: ${displayLabel}`,
      };
    });
  }, [
    availableRouteBindRecentTargets,
    bindingTargets.accounts,
    bindingTargets.loaded,
    locale,
    routeBindAccountId,
    routeBindGroupName,
    routeBindTargetKind,
  ]);
  useLayoutEffect(() => {
    const measureContainer = routeBindRecentTargetsMeasureRef.current;
    if (!measureContainer) {
      setRouteBindRecentVisibleCount(routeBindRecentChipItems.length);
      return undefined;
    }

    const computeVisibleCount = () => {
      const containerWidth = measureContainer.clientWidth;
      if (containerWidth <= 0 || routeBindRecentChipItems.length === 0) {
        setRouteBindRecentVisibleCount(routeBindRecentChipItems.length);
        return;
      }

      const chipWidths = routeBindRecentChipItems.map(
        (item) => routeBindRecentChipMeasureRefs.current.get(item.key)?.offsetWidth ?? 0,
      );

      const countRows = (widths: number[]) => {
        let rows = 1;
        let currentRowWidth = 0;
        for (const width of widths) {
          if (width <= 0) {
            return Number.POSITIVE_INFINITY;
          }
          const nextWidth =
            currentRowWidth === 0
              ? width
              : currentRowWidth + ROUTE_BIND_RECENT_TARGET_GAP_PX + width;
          if (currentRowWidth > 0 && nextWidth > containerWidth + 0.5) {
            rows += 1;
            currentRowWidth = width;
          } else {
            currentRowWidth = nextWidth;
          }
        }
        return rows;
      };

      let nextVisibleCount = routeBindRecentChipItems.length;
      for (
        let visibleCount = routeBindRecentChipItems.length;
        visibleCount >= 0;
        visibleCount -= 1
      ) {
        if (countRows(chipWidths.slice(0, visibleCount)) <= ROUTE_BIND_RECENT_TARGET_MAX_ROWS) {
          nextVisibleCount = visibleCount;
          break;
        }
      }
      setRouteBindRecentVisibleCount((current: number) =>
        current === nextVisibleCount ? current : nextVisibleCount,
      );
    };

    computeVisibleCount();
    window.addEventListener("resize", computeVisibleCount);
    if (typeof ResizeObserver === "undefined") {
      return () => {
        window.removeEventListener("resize", computeVisibleCount);
      };
    }

    const observer = new ResizeObserver(computeVisibleCount);
    observer.observe(measureContainer);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", computeVisibleCount);
    };
  }, [routeBindRecentChipItems]);
  const visibleRouteBindRecentChipItems = routeBindRecentChipItems.slice(
    0,
    Math.max(0, Math.min(routeBindRecentVisibleCount, routeBindRecentChipItems.length)),
  );
  const clearBindingDialogTitle =
    clearBindingDialogAction === "clearAndResetAffinity"
      ? locale === "zh"
        ? "清空并重选"
        : "Clear and reselect"
      : locale === "zh"
        ? "清空绑定"
        : "Clear binding";
  const clearBindingDialogDescription =
    clearBindingDialogAction === "clearAndResetAffinity"
      ? locale === "zh"
        ? "会清空选中对话的单账号亲和性约束，并允许这些会话重新选择健康账号。"
        : "This clears the single-account affinity constraints on the selected conversations and lets them reroute onto a healthy account."
      : locale === "zh"
        ? "会删除选中对话的手工绑定；不会应用当前分组或账号选择，也不会清理 sticky route 或加密 owner lock。"
        : "This removes manual bindings from the selected conversations. It does not apply the current group or account selection, and it does not clear sticky routes or encrypted owner locks.";
  const clearBindingCalloutTitle =
    clearBindingDialogAction === "clearAndResetAffinity"
      ? locale === "zh"
        ? "会立即重置以下会话级亲和性"
        : "This immediately resets the following conversation affinity"
      : locale === "zh"
        ? "会立即清理以下会话级手工绑定"
        : "This immediately clears the following conversation-level manual binding";
  const clearBindingCalloutDescription =
    clearBindingDialogAction === "clearAndResetAffinity"
      ? locale === "zh"
        ? "用于恢复被单账号约束卡住的会话。成功后，这些会话会重新参与路由选择。"
        : "Use this when conversations are blocked by a single-account constraint. Successful items re-enter normal routing."
      : locale === "zh"
        ? "当前弹窗里的分组或账号选择不会被应用；现有 sticky / owner 亲和性保持不变。"
        : "The group or account selection in this dialog will not be applied. Existing sticky and owner affinity stays unchanged.";
  const clearBindingCalloutItems =
    clearBindingDialogAction === "clearAndResetAffinity"
      ? locale === "zh"
        ? [
            {
              key: "manual-binding",
              label: "对话级手动绑定",
              detail: "conversation manual binding",
            },
            {
              key: "sticky-route",
              label: "sticky route",
              detail: "prompt-cache single-account affinity",
            },
            {
              key: "owner-lock",
              label: "加密 owner 约束",
              detail: "encrypted session owner lock",
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
              label: "Sticky route",
              detail: "Prompt-cache single-account affinity.",
            },
            {
              key: "owner-lock",
              label: "Encrypted owner lock",
              detail: "Encrypted-session account affinity.",
            },
          ]
      : locale === "zh"
        ? [
            {
              key: "manual-binding",
              label: "对话级手动绑定",
              detail: "conversation manual binding",
            },
          ]
        : [
            {
              key: "manual-binding",
              label: "Conversation manual binding",
              detail: "Conversation-level account override.",
            },
          ];
  const clearBindingDialogTestId =
    clearBindingDialogAction === "clearAndResetAffinity"
      ? "dashboard-working-conversations-clear-affinity-dialog"
      : "dashboard-working-conversations-clear-binding-dialog";
  const clearBindingConfirmLabel =
    clearBindingDialogAction === "clearAndResetAffinity"
      ? locale === "zh"
        ? "确认清空并重选"
        : "Confirm clear and reselect"
      : locale === "zh"
        ? "确认清空绑定"
        : "Confirm clear binding";
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
                    setRouteBindSelectionRestorePending(true);
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
                          <Chip
                            size="compact"
                            tone="secondary"
                            className="px-2.5 py-1 text-[0.68rem] font-semibold uppercase tracking-[0.14em]"
                          >
                            {selectedConversationCount}
                          </Chip>
                        </div>
                        <p className="text-xs font-medium text-base-content/72">
                          {selectionSummaryLabel}
                        </p>
                        <p className="text-xs leading-5 text-base-content/62">
                          {fastModePopoverDescription}
                        </p>
                      </div>
                      <fieldset
                        aria-label={locale === "zh" ? "批量 FAST 模式" : "Bulk FAST mode"}
                        aria-busy={bulkActionBusy === "setFastModeRewriteMode"}
                        className="m-0 grid gap-2 border-0 p-0"
                      >
                        {fastModeOptions.map((option) => {
                          const active = option.value === bulkFastModeRewriteMode;
                          return (
                            <button
                              key={option.value}
                              type="button"
                              aria-pressed={active}
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
                      </fieldset>
                    </div>
                  </BubblePopoverContent>
                </Popover>
                <Button
                  type="button"
                  size="sm"
                  variant="destructive"
                  disabled={bulkActionBusy != null}
                  data-testid="dashboard-working-conversations-clear-affinity-button"
                  onClick={() => {
                    setClearBindingDialogAction("clearAndResetAffinity");
                    setClearBindingDialogOpen(true);
                  }}
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
  const selectVisibleConversations = useCallback(
    (options?: {
      openClearDialog?: boolean;
      clearDialogAction?: DashboardConversationClearDialogAction;
    }) => {
      if (cards.length === 0) return;
      setBulkFeedback(null);
      setSelectionModeEnabled(true);
      setSelectedPromptCacheKeys(Array.from(new Set(cards.map((card) => card.promptCacheKey))));
      if (options?.openClearDialog) {
        setClearBindingDialogAction(options.clearDialogAction ?? "bind");
        setClearBindingDialogOpen(true);
      }
    },
    [cards],
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
              disabled={upstreamAccountsDisabled || blockedBindingFilterActive}
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
                <DashboardNetworkRecentPopover
                  triggerAriaLabel={t("dashboard.networkRecent.openPanel")}
                  trigger={
                    <DashboardNetworkSpeedCapsule
                      uploadBytesPerSecond={totalNetworkSpeed.uploadBytesPerSecond}
                      downloadBytesPerSecond={totalNetworkSpeed.downloadBytesPerSecond}
                      localeTag={localeTag}
                      uploadLabel={networkUploadLabel}
                      downloadLabel={networkDownloadLabel}
                      testId="dashboard-upstream-account-total-network-speed"
                      className="bg-base-100/62"
                    />
                  }
                />
              ) : null}
              <Chip tone="primary" className="w-fit px-3 py-1 font-mono text-xs font-semibold">
                {countChipLabel}
              </Chip>
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

        {activeView === "conversations" && blockedBindingBannerMeta ? (
          <Alert
            variant={cards.length > 0 ? "info" : "warning"}
            data-testid="dashboard-blocked-binding-banner"
            className="flex-col gap-3 desktop:flex-row desktop:items-center desktop:justify-between"
          >
            <div className="min-w-0 space-y-1">
              <div className="font-semibold">{blockedBindingBannerMeta.title}</div>
              <p className="text-sm leading-6 text-current/85">
                {blockedBindingBannerMeta.description}
              </p>
            </div>
            <div className="flex shrink-0 flex-wrap items-center gap-2">
              <Button
                type="button"
                size="sm"
                variant="ghost"
                disabled={bulkActionBusy != null || cards.length === 0}
                data-testid="dashboard-blocked-binding-select-visible-button"
                onClick={() => selectVisibleConversations()}
              >
                {locale === "zh" ? "选择当前结果" : "Select current results"}
              </Button>
              <Button
                type="button"
                size="sm"
                variant="destructive"
                disabled={bulkActionBusy != null || cards.length === 0}
                data-testid="dashboard-blocked-binding-clear-and-reselect-button"
                onClick={() =>
                  selectVisibleConversations({
                    openClearDialog: true,
                    clearDialogAction: "clearAndResetAffinity",
                  })
                }
              >
                {locale === "zh" ? "清空并重选" : "Clear and reselect"}
              </Button>
              <Button
                type="button"
                size="sm"
                variant="ghost"
                data-testid="dashboard-blocked-binding-clear-filter-button"
                onClick={onClearBlockedBindingFilter}
              >
                {locale === "zh" ? "清除筛选" : "Clear filter"}
              </Button>
            </div>
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
                className="grid items-start grid-cols-1 gap-4 desktop1660:grid-cols-[repeat(2,minmax(0,1fr))]"
              >
                {upstreamAccountRows.flat().map((account) => (
                  <DashboardUpstreamAccountActivityCard
                    key={account.accountKey ?? account.upstreamAccountId ?? "unassigned"}
                    account={account}
                    routingStateVersion={upstreamAccountActivity?.routingStateVersion}
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
            {blockedBindingFilterActive
              ? locale === "zh"
                ? "当前没有匹配这组单账号约束阻塞筛选的工作会话。"
                : "No working conversations currently match this single-account binding block filter."
              : t("dashboard.workingConversations.empty")}
          </div>
        ) : null}

        {activeView === "conversations" && cards.length > 0 ? (
          <div data-testid="dashboard-working-conversations-grid" ref={setGridContainerRef}>
            <DashboardWorkingConversationGridExternal
              renderedRows={renderedRows}
              rows={rows}
              totalSize={totalSize}
              scrollMargin={scrollMargin}
              columnCount={columnCount}
              isLoadingMore={isLoadingMore}
              measureRow={rowVirtualizer.measureElement}
              cardProps={{
                selectionModeEnabled,
                selectedPromptCacheKeySet,
                selectionSummaryLabel,
                nowMs,
                locale,
                numberFormatter,
                currencyFormatter,
                timestampFormatter,
                t,
                onOpenUpstreamAccount,
                onOpenConversation,
                onOpenInvocation,
                handleConversationCardClickCapture,
                handleSelectionCardClick,
                handleSelectionCardKeyDown,
              }}
            />
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
            {visibleRouteBindRecentChipItems.length > 0 ? (
              <div
                className="relative space-y-2"
                data-testid="dashboard-working-conversations-route-bind-recents"
              >
                <p className="text-[0.7rem] font-semibold uppercase tracking-[0.14em] text-base-content/52">
                  {locale === "zh" ? "最近使用" : "Recent targets"}
                </p>
                <div
                  className="flex flex-wrap gap-2"
                  data-testid="dashboard-working-conversations-route-bind-recents-grid"
                >
                  {visibleRouteBindRecentChipItems.map((item) => (
                    <Chip
                      asChild
                      size="compact"
                      tone={
                        item.active
                          ? "primary"
                          : item.target.kind === "group"
                            ? "info"
                            : "secondary"
                      }
                      key={item.key}
                      title={item.title}
                      aria-pressed={item.active}
                      data-testid="dashboard-working-conversations-route-bind-recent-chip"
                      data-kind={item.target.kind}
                    >
                      <button
                        type="button"
                        className="dashboard-route-bind-recent-chip inline-flex min-w-0 max-w-full items-center gap-1 text-left text-[10px] font-medium"
                        onClick={() => applyRouteBindRecentTargetSelection(item.target)}
                      >
                        <span className="shrink-0 text-[8px] font-semibold uppercase tracking-[0.08em] opacity-80">
                          {item.typeLabel}
                        </span>
                        <span className="min-w-0 max-w-[14rem] truncate whitespace-nowrap sm:max-w-[15rem]">
                          {item.displayLabel}
                        </span>
                      </button>
                    </Chip>
                  ))}
                </div>
                <div
                  ref={routeBindRecentTargetsMeasureRef}
                  aria-hidden="true"
                  className="pointer-events-none invisible absolute left-0 top-0 -z-10 flex w-full flex-wrap gap-2"
                >
                  {routeBindRecentChipItems.map((item) => (
                    <Chip
                      asChild
                      size="compact"
                      tone={
                        item.active
                          ? "primary"
                          : item.target.kind === "group"
                            ? "info"
                            : "secondary"
                      }
                      key={`measure:${item.key}`}
                      ref={(node) => {
                        if (node) {
                          routeBindRecentChipMeasureRefs.current.set(item.key, node);
                        } else {
                          routeBindRecentChipMeasureRefs.current.delete(item.key);
                        }
                      }}
                      tabIndex={-1}
                    >
                      <button
                        type="button"
                        className="dashboard-route-bind-recent-chip inline-flex min-w-0 max-w-full items-center gap-1 px-2 text-left text-[10px] font-medium"
                      >
                        <span className="shrink-0 text-[8px] font-semibold uppercase tracking-[0.08em] opacity-80">
                          {item.typeLabel}
                        </span>
                        <span className="min-w-0 max-w-[14rem] truncate whitespace-nowrap sm:max-w-[15rem]">
                          {item.displayLabel}
                        </span>
                      </button>
                    </Chip>
                  ))}
                </div>
              </div>
            ) : null}
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
                setClearBindingDialogAction("bind");
                setClearBindingDialogOpen(true);
              }}
            >
              {locale === "zh" ? "清空绑定" : "Clear binding"}
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
        open={clearBindingDialogOpen}
        onOpenChange={(nextOpen) => {
          if (bulkActionBusy == null) setClearBindingDialogOpen(nextOpen);
        }}
      >
        <DialogContent
          role="alertdialog"
          className="overflow-hidden p-0"
          data-testid={clearBindingDialogTestId}
        >
          <div className="dialog-chrome-surface border-b px-5 py-4 desktop:px-6">
            <DialogHeader>
              <DialogTitle>{clearBindingDialogTitle}</DialogTitle>
              <DialogDescription>{clearBindingDialogDescription}</DialogDescription>
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
                      {clearBindingCalloutTitle}
                    </p>
                    <p className="text-sm leading-6 text-base-content/74">
                      {clearBindingCalloutDescription}
                    </p>
                  </div>
                  <ul className="destructive-callout-list overflow-hidden rounded-[0.95rem]">
                    {clearBindingCalloutItems.map((item) => (
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
                onClick={() => setClearBindingDialogOpen(false)}
              >
                {locale === "zh" ? "取消" : "Cancel"}
              </Button>
              <Button
                type="button"
                variant="destructive"
                disabled={bulkActionBusy != null}
                onClick={() => {
                  void applyBulkConversationAction(
                    clearBindingDialogAction === "clearAndResetAffinity"
                      ? {
                          action: "clearAndResetAffinity",
                        }
                      : {
                          action: "bind",
                          bindingKind: "none",
                        },
                  );
                }}
              >
                {bulkActionBusy === "bind" || bulkActionBusy === "clearAndResetAffinity"
                  ? locale === "zh"
                    ? "处理中…"
                    : "Applying..."
                  : clearBindingConfirmLabel}
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
