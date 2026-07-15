/* eslint-disable react-refresh/only-export-components */
import {
  type ReactNode,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useNavigate } from "react-router-dom";
import { Alert } from "../../components/ui/alert";
import { Badge } from "../../components/ui/badge";
import { Button } from "../../components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../../components/ui/card";
import {
  Dialog,
  DialogCloseIcon,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "../../components/ui/dialog";
import { FloatingFieldError } from "../../components/ui/floating-field-error";
import { FormFieldFeedback } from "../../components/ui/form-field-feedback";
import { Input } from "../../components/ui/input";
import { Popover, PopoverArrow, PopoverContent, PopoverTrigger } from "../../components/ui/popover";
import { SegmentedControl, SegmentedControlItem } from "../../components/ui/segmented-control";
import { SelectField } from "../../components/ui/select-field";
import { Spinner } from "../../components/ui/spinner";
import { Switch } from "../../components/ui/switch";
import { AccountDetailDrawerShell } from "../../features/account-pool/AccountDetailDrawerShell";
import { EffectiveRoutingRuleCard } from "../../features/account-pool/EffectiveRoutingRuleCard";
import {
  MotherAccountBadge,
  MotherAccountToggle,
} from "../../features/account-pool/MotherAccountToggle";
import { UpstreamAccountAttemptTimeline } from "../../features/account-pool/UpstreamAccountAttemptTimeline";
import { UpstreamAccountGroupCombobox } from "../../features/account-pool/UpstreamAccountGroupCombobox";
import { UpstreamAccountUsageCard } from "../../features/account-pool/UpstreamAccountUsageCard";
import { DashboardActivityOverview } from "../../features/dashboard/DashboardActivityOverview";
import { ACCOUNT_ACTIVITY_RANGE_STORAGE_KEY_PREFIX } from "../../features/dashboard/dashboardActivityRange";
import { ForwardProxyBindingSelector } from "../../features/forward-proxy/ForwardProxyBindingSelector";
import { InvocationTable } from "../../features/invocations/InvocationTable";
import { StickyKeyConversationTable } from "../../features/prompt-cache/StickyKeyConversationTable";
import { AppIcon } from "../../features/shared/AppIcon";
import { useAvailableModelOptions } from "../../hooks/useAvailableModelOptions";
import { useCompactViewport } from "../../hooks/useCompactViewport";
import { useInvocationRecordsRealtime } from "../../hooks/useInvocationRecordsRealtime";
import { useMotherSwitchNotifications } from "../../hooks/useMotherSwitchNotifications";
import {
  type UpstreamAccountDetailRouteTab,
  useUpstreamAccountDetailRoute,
} from "../../hooks/useUpstreamAccountDetailRoute";
import { useUpstreamAccounts } from "../../hooks/useUpstreamAccounts";
import { useUpstreamStickyConversations } from "../../hooks/useUpstreamStickyConversations";
import { useTranslation } from "../../i18n";
import type {
  ApiInvocation,
  ForwardProxyBindingNode,
  StickyKeyConversationSelection,
  UpdateGroupAccountRoutingRulePayload,
  UpstreamAccountDetail,
  UpstreamAccountDuplicateInfo,
  UpstreamAccountSummary,
} from "../../lib/api";
import {
  ApiRequestError,
  fetchInvocationRecordLocation,
  fetchInvocationRecords,
} from "../../lib/api";
import { invocationStableKey } from "../../lib/invocation";
import { upstreamPlanBadgeRecipe } from "../../lib/upstreamAccountBadges";
import {
  type AccountDraft,
  areAccountDraftsEqual,
  mergeDraftAfterAccountSave,
} from "../../lib/upstreamAccountDrafts";
import { isUpstreamAccountNotFoundError } from "../../lib/upstreamAccountErrors";
import {
  buildGroupOptions,
  isExistingGroup,
  normalizeGroupName,
  resolveGroupConcurrencyLimit,
  resolveGroupNote,
} from "../../lib/upstreamAccountGroups";
import type {
  StatusChangeReasonCode,
  StatusChangeReasonFieldKey,
} from "../../lib/upstreamAccountStatusChangeReasons";
import { validateUpstreamBaseUrl } from "../../lib/upstreamBaseUrl";
import { applyMotherUpdateToItems } from "../../lib/upstreamMother";
import { cn } from "../../lib/utils";
import { resolveDisplayNameAfterEmailChange } from "./UpstreamAccountCreate.shared";
import {
  bulkSyncRowStatusVariant,
  resolveBulkSyncCounts,
  shouldAutoHideBulkSyncProgress,
  withBulkSyncSnapshotStatus,
} from "./UpstreamAccounts.bulk-sync";
import {
  DEFAULT_UPSTREAM_ACCOUNT_GROUP_NAME,
  formatGroupFilterValue,
  parseGroupFilterValue,
  persistUpstreamAccountFilters,
  readPersistedUpstreamAccountFilters,
} from "./UpstreamAccounts.filters";
import {
  buildRoutingDraft,
  parseRoutingPositiveInteger,
  parseRoutingTimeoutValue,
  resolveRoutingMaintenance,
} from "./UpstreamAccounts.routing";
import {
  type AccountBusyActionType,
  type ActionErrorState,
  type BusyActionState,
  DEFAULT_ROUTING_TIMEOUTS,
  type GroupFilterState,
  type RoutingDraft,
  UPSTREAM_ACCOUNTS_QUERY_STALE_GRACE_MS,
  type UpstreamAccountsLocationState,
} from "./UpstreamAccounts.shared-types";
import {
  accountHealthStatus,
  accountWorkStatus,
  compactSupportHint,
  compactSupportLabel,
  poolCardMetric,
} from "./UpstreamAccounts.status";
import { useUpstreamAccountGroupSettingsDialog } from "./useUpstreamAccountGroupSettingsDialog";

export type {
  ActionErrorState,
  BusyActionState,
  GroupFilterState,
  RoutingDraft,
  UpstreamAccountsLocationState,
};
export {
  accountHealthStatus,
  accountWorkStatus,
  buildRoutingDraft,
  bulkSyncRowStatusVariant,
  compactSupportHint,
  compactSupportLabel,
  DEFAULT_ROUTING_TIMEOUTS,
  DEFAULT_UPSTREAM_ACCOUNT_GROUP_NAME,
  formatGroupFilterValue,
  parseGroupFilterValue,
  parseRoutingPositiveInteger,
  parseRoutingTimeoutValue,
  persistUpstreamAccountFilters,
  poolCardMetric,
  readPersistedUpstreamAccountFilters,
  resolveBulkSyncCounts,
  resolveRoutingMaintenance,
  shouldAutoHideBulkSyncProgress,
  UPSTREAM_ACCOUNTS_QUERY_STALE_GRACE_MS,
  withBulkSyncSnapshotStatus,
};

const ACCOUNT_RECORD_PAGE_SIZE = 50;
const LEGACY_ACCOUNT_RECORDS_ENABLED = false;
const DEFAULT_STICKY_CONVERSATION_SELECTION_VALUE = "count:50";
const DIRECT_PROXY_KEY = "__direct__";
const STICKY_CONVERSATION_SELECTION_OPTIONS = [
  {
    value: "count:20",
    selection: {
      mode: "count",
      limit: 20,
    } satisfies StickyKeyConversationSelection,
  },
  {
    value: "count:50",
    selection: {
      mode: "count",
      limit: 50,
    } satisfies StickyKeyConversationSelection,
  },
  {
    value: "count:100",
    selection: {
      mode: "count",
      limit: 100,
    } satisfies StickyKeyConversationSelection,
  },
  {
    value: "activityWindow:1",
    selection: {
      mode: "activityWindow",
      activityHours: 1,
    } satisfies StickyKeyConversationSelection,
  },
  {
    value: "activityWindow:3",
    selection: {
      mode: "activityWindow",
      activityHours: 3,
    } satisfies StickyKeyConversationSelection,
  },
  {
    value: "activityWindow:6",
    selection: {
      mode: "activityWindow",
      activityHours: 6,
    } satisfies StickyKeyConversationSelection,
  },
  {
    value: "activityWindow:12",
    selection: {
      mode: "activityWindow",
      activityHours: 12,
    } satisfies StickyKeyConversationSelection,
  },
  {
    value: "activityWindow:24",
    selection: {
      mode: "activityWindow",
      activityHours: 24,
    } satisfies StickyKeyConversationSelection,
  },
] as const;
const STICKY_CONVERSATION_SELECTION_LOOKUP = new Map<string, StickyKeyConversationSelection>(
  STICKY_CONVERSATION_SELECTION_OPTIONS.map((option) => [option.value, option.selection]),
);
type OauthRecoveryHint = {
  titleKey: string;
  bodyKey: string;
};

type AccountDetailTab = UpstreamAccountDetailRouteTab;

type AccountRecordsMode = "latest" | "anchored";
type AccountRecordsLocateError = {
  invokeId: string;
  attemptId?: string | null;
  kind: "notFound" | "request";
};

const ACCOUNT_DETAIL_TABS_REQUIRING_ROSTER_CONTEXT = new Set<AccountDetailTab>(["edit", "routing"]);

type SharedUpstreamAccountDetailDrawerCloseOptions = {
  replace?: boolean;
};

type SharedUpstreamAccountDetailDrawerProps = {
  open: boolean;
  accountId: number | null;
  initialTab?: AccountDetailTab;
  initialDeleteConfirmOpen?: boolean;
  presentation?: "overlay" | "page";
  onInitialDeleteConfirmHandled?: () => void;
  onClose: (options?: SharedUpstreamAccountDetailDrawerCloseOptions) => void;
};

type PendingSaveSession = {
  accountId: number;
  sessionKey: string | null;
  fallbackDraft: AccountDraft;
};

function normalizeProxyKeys(values?: string[]): string[] {
  if (!Array.isArray(values)) return [];
  return Array.from(
    new Set(values.map((value) => value.trim()).filter((value) => value.length > 0)),
  );
}

function proxyNodeLabel(node: ForwardProxyBindingNode | undefined, key: string) {
  if (node) {
    const protocol = node.protocolLabel ? ` · ${node.protocolLabel}` : "";
    return `${node.displayName}${protocol}`;
  }
  return key === DIRECT_PROXY_KEY ? "Direct · DIRECT" : key;
}

function proxyNodeStatusLabel(
  node: ForwardProxyBindingNode | undefined,
  key: string,
  t: (key: string) => string,
) {
  if (node?.selectable || key === DIRECT_PROXY_KEY) {
    return t("accountPool.upstreamAccounts.proxyBindings.statusAvailable");
  }
  if (node) {
    return t("accountPool.upstreamAccounts.proxyBindings.statusUnavailable");
  }
  return t("accountPool.upstreamAccounts.proxyBindings.statusMissing");
}

function proxyNodeTone(
  node: ForwardProxyBindingNode | undefined,
  key: string,
): "direct" | "available" | "unavailable" | "missing" {
  if (key === DIRECT_PROXY_KEY || node?.source === "direct") return "direct";
  if (!node) return "missing";
  return node.selectable ? "available" : "unavailable";
}

function toggleProxyKey(keys: string[], key: string): string[] {
  return keys.includes(key) ? keys.filter((value) => value !== key) : [...keys, key];
}

type RecentSaveResponseGuard = {
  accountId: number;
  sessionKey: string | null;
  startedDraft: AccountDraft;
  draft: AccountDraft;
  fallbackDraft: AccountDraft;
  retainedDraft: AccountDraft;
};

type InlinePolicyField =
  | "allowCutOut"
  | "allowCutIn"
  | "priorityTier"
  | "fastModeRewriteMode"
  | "imageToolRewriteMode"
  | "concurrencyLimit"
  | "upstream429Retry"
  | "availableModels"
  | "timeoutResponsesFirstByte"
  | "timeoutCompactFirstByte"
  | "timeoutResponsesStream"
  | "timeoutCompactStream"
  | "statusChangeReasons"
  | StatusChangeReasonFieldKey;

function createBusyActionKey(type: AccountBusyActionType, accountId: number) {
  return `${type}:${accountId}`;
}

export function isBusyAction(
  busyAction: BusyActionState,
  type: AccountBusyActionType | "routing",
  accountId?: number,
) {
  if (type === "routing") return busyAction.routing;
  if (typeof accountId !== "number") return false;
  return busyAction.accountActions.has(createBusyActionKey(type, accountId));
}

function hasBusyAccountAction(busyAction: BusyActionState, accountId?: number | null) {
  if (typeof accountId !== "number") return false;
  const suffix = `:${accountId}`;
  for (const key of busyAction.accountActions) {
    if (key.endsWith(suffix)) return true;
  }
  return false;
}

type AccountStatusSnapshot = Pick<
  UpstreamAccountSummary,
  | "status"
  | "displayStatus"
  | "enabled"
  | "workStatus"
  | "enableStatus"
  | "healthStatus"
  | "syncState"
>;

function accountEnableStatus(item?: AccountStatusSnapshot | null) {
  if (item?.enableStatus) return item.enableStatus;
  if (item?.enabled === false || item?.displayStatus === "disabled") {
    return "disabled";
  }
  return "enabled";
}

function accountSyncState(item?: AccountStatusSnapshot | null) {
  if (item?.syncState) return item.syncState;
  const legacyStatus = item?.displayStatus ?? item?.status;
  return legacyStatus === "syncing" ? "syncing" : "idle";
}

function formatDateTime(value?: string | null) {
  if (!value) return "—";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(date);
}

function normalizeNumberInput(value: string): number | undefined {
  const trimmed = value.trim();
  if (!trimmed) return undefined;
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : undefined;
}

function normalizeDisplayNameKey(value: string) {
  return value.trim().toLocaleLowerCase();
}

function normalizeGroupUpstream429MaxRetries(value?: number | null) {
  if (!Number.isFinite(value ?? NaN)) return 0;
  return Math.min(5, Math.max(0, Math.trunc(value ?? 0)));
}

function normalizeEnabledGroupUpstream429MaxRetries(value?: number | null) {
  return Math.max(1, normalizeGroupUpstream429MaxRetries(value) || 1);
}

function findDisplayNameConflict(
  items: UpstreamAccountSummary[],
  displayName: string,
  excludeId?: number | null,
) {
  const normalized = normalizeDisplayNameKey(displayName);
  if (!normalized) return null;
  return (
    items.find(
      (item) => item.id !== excludeId && normalizeDisplayNameKey(item.displayName) === normalized,
    ) ?? null
  );
}

function buildDraft(detail: UpstreamAccountDetail | null): AccountDraft {
  return {
    displayName: detail?.displayName ?? "",
    email: detail?.email ?? "",
    groupName: detail?.groupName ?? "",
    isMother: detail?.isMother ?? false,
    note: detail?.note ?? "",
    upstreamBaseUrl: detail?.upstreamBaseUrl ?? "",
    tagIds: detail?.tags?.map((tag) => tag.id) ?? [],
    localPrimaryLimit:
      detail?.localLimits?.primaryLimit == null ? "" : String(detail.localLimits.primaryLimit),
    localSecondaryLimit:
      detail?.localLimits?.secondaryLimit == null ? "" : String(detail.localLimits.secondaryLimit),
    localLimitUnit: detail?.localLimits?.limitUnit ?? "requests",
    apiKey: "",
  };
}

function removeAccountDraftTagIds(draft: AccountDraft, removedTagIds: Set<number>): AccountDraft {
  const nextTagIds = draft.tagIds.filter((tagId) => !removedTagIds.has(tagId));
  return nextTagIds.length === draft.tagIds.length ? draft : { ...draft, tagIds: nextTagIds };
}

function enableStatusVariant(status: string): "success" | "secondary" {
  return status === "enabled" ? "success" : "secondary";
}

function workStatusVariant(status: string): "info" | "warning" | "secondary" {
  if (status === "working") return "info";
  if (status === "degraded") return "warning";
  if (status === "rate_limited") return "warning";
  return "secondary";
}

function healthStatusVariant(status: string): "success" | "warning" | "error" | "secondary" {
  if (status === "normal") return "success";
  if (status === "upstream_unavailable") return "warning";
  if (
    status === "needs_reauth" ||
    status === "upstream_rejected" ||
    status === "error_other" ||
    status === "error"
  ) {
    return "error";
  }
  return "secondary";
}

function syncStateVariant(status: string): "warning" | "secondary" {
  return status === "syncing" ? "warning" : "secondary";
}

function kindVariant(kind: string): "secondary" | "success" {
  return kind === "oauth_codex" ? "success" : "secondary";
}

function isLegacyOauthBridgeExchangeError(lastError?: string | null) {
  const normalized = lastError?.toLocaleLowerCase() ?? "";
  return normalized.includes("oauth bridge token exchange failed");
}

function resolveOauthRecoveryHint(
  kind: string,
  healthStatus: string,
  lastError?: string | null,
): OauthRecoveryHint | null {
  if (kind !== "oauth_codex") return null;
  if (isLegacyOauthBridgeExchangeError(lastError)) {
    return {
      titleKey: "accountPool.upstreamAccounts.hints.bridgeExchangeTitle",
      bodyKey: "accountPool.upstreamAccounts.hints.bridgeExchangeBody",
    };
  }
  if (healthStatus === "upstream_unavailable") {
    return {
      titleKey: "accountPool.upstreamAccounts.hints.dataPlaneUnavailableTitle",
      bodyKey: "accountPool.upstreamAccounts.hints.dataPlaneUnavailableBody",
    };
  }
  if (healthStatus === "upstream_rejected") {
    return {
      titleKey: "accountPool.upstreamAccounts.hints.dataPlaneRejectedTitle",
      bodyKey: "accountPool.upstreamAccounts.hints.dataPlaneRejectedBody",
    };
  }
  if (healthStatus === "needs_reauth") {
    return {
      titleKey: "accountPool.upstreamAccounts.hints.reauthTitle",
      bodyKey: "accountPool.upstreamAccounts.hints.reauthBody",
    };
  }
  return null;
}

function AccountDetailSkeleton() {
  return (
    <div className="grid gap-4">
      {Array.from({ length: 3 }).map((_, index) => (
        <div key={index} className="h-28 animate-pulse rounded-[1.35rem] bg-base-200/75" />
      ))}
    </div>
  );
}

function DetailField({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="metric-cell">
      <p className="metric-label">{label}</p>
      <div className="mt-2 text-sm text-base-content/80">{value ?? "—"}</div>
    </div>
  );
}

function CompactDetailField({
  label,
  value,
  helper,
  title,
}: {
  label: string;
  value: ReactNode;
  helper?: ReactNode;
  title?: string;
}) {
  return (
    <div className="min-w-0 py-0.5">
      <p className="truncate text-[0.66rem] font-semibold uppercase tracking-[0.1em] text-base-content/55">
        {label}
      </p>
      <div
        className="mt-0.5 min-w-0 truncate text-sm font-medium leading-5 text-base-content/85"
        title={title}
      >
        {value || "—"}
      </div>
      {helper ? (
        <div
          className="mt-0.5 truncate text-xs leading-4 text-base-content/55"
          title={typeof helper === "string" ? helper : undefined}
        >
          {helper}
        </div>
      ) : null}
    </div>
  );
}

export function RoutingSettingsDialog({
  open,
  title,
  description,
  closeLabel,
  cancelLabel,
  saveLabel,
  apiKey,
  primarySyncIntervalSecs,
  secondarySyncIntervalSecs,
  priorityAvailableAccountCap,
  timeoutSectionTitle,
  timeoutFields,
  busy,
  apiKeyWritesEnabled,
  timeoutWritesEnabled,
  canSave,
  onApiKeyChange,
  onGenerate,
  onPrimarySyncIntervalChange,
  onSecondarySyncIntervalChange,
  onPriorityAvailableAccountCapChange,
  onClose,
  onSave,
}: {
  open: boolean;
  title: string;
  description: string;
  closeLabel: string;
  cancelLabel: string;
  saveLabel: string;
  apiKey: string;
  primarySyncIntervalSecs: string;
  secondarySyncIntervalSecs: string;
  priorityAvailableAccountCap: string;
  timeoutSectionTitle: string;
  timeoutFields: Array<{
    key: string;
    label: string;
    value: string;
    onChange: (value: string) => void;
  }>;
  busy: boolean;
  apiKeyWritesEnabled: boolean;
  timeoutWritesEnabled: boolean;
  canSave: boolean;
  onApiKeyChange: (value: string) => void;
  onGenerate: () => void;
  onPrimarySyncIntervalChange: (value: string) => void;
  onSecondarySyncIntervalChange: (value: string) => void;
  onPriorityAvailableAccountCapChange: (value: string) => void;
  onClose: () => void;
  onSave: () => void;
}) {
  const { t } = useTranslation();
  const apiKeyInputRef = useRef<HTMLInputElement | null>(null);
  const primaryInputRef = useRef<HTMLInputElement | null>(null);
  const apiKeyInputId = "pool-routing-secret-input";
  const primaryInputId = "pool-routing-primary-sync-interval";
  const secondaryInputId = "pool-routing-secondary-sync-interval";
  const capInputId = "pool-routing-priority-cap";

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => (!busy ? (nextOpen ? undefined : onClose()) : undefined)}
    >
      <DialogContent
        className="flex max-h-[calc(100dvh-0.75rem)] flex-col overflow-hidden p-0 desktop:max-h-[calc(100dvh-2rem)]"
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          if (apiKeyWritesEnabled) {
            apiKeyInputRef.current?.focus();
            return;
          }
          primaryInputRef.current?.focus();
        }}
        onPointerDownOutside={(event) => {
          if (busy) event.preventDefault();
        }}
        onEscapeKeyDown={(event) => {
          if (busy) event.preventDefault();
        }}
      >
        <div className="flex shrink-0 items-start justify-between gap-4 border-b border-base-300/80 px-5 py-4 desktop:px-6 desktop:py-5">
          <DialogHeader className="min-w-0 max-w-[28rem]">
            <DialogTitle>{title}</DialogTitle>
            <DialogDescription>{description}</DialogDescription>
          </DialogHeader>
          <DialogCloseIcon aria-label={closeLabel} disabled={busy} />
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-5 desktop:px-6 desktop:py-6">
          <div className="space-y-4">
            <div className="space-y-3 rounded-2xl border border-base-300/80 bg-base-100/70 p-4">
              <div className="space-y-1">
                <p className="text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82">
                  {t("accountPool.upstreamAccounts.routing.apiKeySectionTitle")}
                </p>
                <p className="text-sm text-base-content/68">
                  {t("accountPool.upstreamAccounts.routing.apiKeySectionDescription")}
                </p>
              </div>
              <div className="field">
                <div className="mb-2 flex flex-wrap items-center justify-between gap-3">
                  <label
                    htmlFor={apiKeyInputId}
                    className="text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82"
                  >
                    {t("accountPool.upstreamAccounts.routing.apiKeyLabel")}
                  </label>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={onGenerate}
                    disabled={busy || !apiKeyWritesEnabled}
                  >
                    <AppIcon name="auto-fix" className="mr-2 h-4 w-4" aria-hidden />
                    {t("accountPool.upstreamAccounts.routing.generate")}
                  </Button>
                </div>
                <Input
                  id={apiKeyInputId}
                  ref={apiKeyInputRef}
                  name="poolRoutingSecret"
                  type="text"
                  value={apiKey}
                  onChange={(event) => onApiKeyChange(event.target.value)}
                  placeholder={t("accountPool.upstreamAccounts.routing.apiKeyPlaceholder")}
                  autoComplete="off"
                  autoCorrect="off"
                  autoCapitalize="none"
                  spellCheck={false}
                  data-1p-ignore="true"
                  data-lpignore="true"
                  disabled={busy || !apiKeyWritesEnabled}
                  className="h-12 rounded-xl border-base-300/90 bg-base-100 px-4 text-[15px] font-mono placeholder:text-base-content/58"
                />
              </div>
            </div>

            <div className="space-y-4 rounded-2xl border border-base-300/80 bg-base-100/70 p-4">
              <div className="space-y-1">
                <p className="text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82">
                  {t("accountPool.upstreamAccounts.routing.maintenanceSectionTitle")}
                </p>
                <p className="text-sm text-base-content/68">
                  {t("accountPool.upstreamAccounts.routing.maintenanceSectionDescription")}
                </p>
              </div>
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="field">
                  <label
                    htmlFor={primaryInputId}
                    className="mb-2 text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82"
                  >
                    {t("accountPool.upstreamAccounts.routing.primarySyncIntervalLabel")}
                  </label>
                  <Input
                    id={primaryInputId}
                    ref={primaryInputRef}
                    name="primarySyncIntervalSecs"
                    type="number"
                    min={60}
                    step={60}
                    inputMode="numeric"
                    value={primarySyncIntervalSecs}
                    onChange={(event) => onPrimarySyncIntervalChange(event.target.value)}
                    placeholder="300"
                    disabled={busy || !timeoutWritesEnabled}
                    className="h-12 rounded-xl border-base-300/90 bg-base-100 px-4"
                  />
                </div>
                <div className="field">
                  <label
                    htmlFor={secondaryInputId}
                    className="mb-2 text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82"
                  >
                    {t("accountPool.upstreamAccounts.routing.secondarySyncIntervalLabel")}
                  </label>
                  <Input
                    id={secondaryInputId}
                    name="secondarySyncIntervalSecs"
                    type="number"
                    min={60}
                    step={60}
                    inputMode="numeric"
                    value={secondarySyncIntervalSecs}
                    onChange={(event) => onSecondarySyncIntervalChange(event.target.value)}
                    placeholder="1800"
                    disabled={busy || !timeoutWritesEnabled}
                    className="h-12 rounded-xl border-base-300/90 bg-base-100 px-4"
                  />
                </div>
              </div>
              <div className="field">
                <label
                  htmlFor={capInputId}
                  className="mb-2 text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82"
                >
                  {t("accountPool.upstreamAccounts.routing.priorityCapLabel")}
                </label>
                <Input
                  id={capInputId}
                  name="priorityAvailableAccountCap"
                  type="number"
                  min={1}
                  step={1}
                  inputMode="numeric"
                  value={priorityAvailableAccountCap}
                  onChange={(event) => onPriorityAvailableAccountCapChange(event.target.value)}
                  placeholder="100"
                  disabled={busy || !timeoutWritesEnabled}
                  className="h-12 rounded-xl border-base-300/90 bg-base-100 px-4"
                />
              </div>
            </div>
            <div className="space-y-3">
              <p className="text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82">
                {timeoutSectionTitle}
              </p>
              <div className="grid gap-3 md:grid-cols-2">
                {timeoutFields.map((field) => (
                  <label key={field.key} className="field">
                    <span className="field-label">{field.label}</span>
                    <Input
                      name={field.key}
                      type="number"
                      min="1"
                      step="1"
                      value={field.value}
                      onChange={(event) => field.onChange(event.target.value)}
                      disabled={busy || !timeoutWritesEnabled}
                      className="h-12 rounded-xl border-base-300/90 bg-base-100 px-4 text-[15px] font-mono"
                    />
                  </label>
                ))}
              </div>
            </div>
          </div>
        </div>
        <DialogFooter className="shrink-0 border-t border-base-300/80 bg-base-100/94 px-5 pb-[max(env(safe-area-inset-bottom),1rem)] pt-4 backdrop-blur desktop:px-6 desktop:py-5">
          <Button type="button" variant="outline" onClick={onClose} disabled={busy}>
            {cancelLabel}
          </Button>
          <Button type="button" onClick={onSave} disabled={busy || !canSave}>
            {busy ? (
              <Spinner size="sm" className="mr-2" />
            ) : (
              <AppIcon name="key-chain-variant" className="mr-2 h-4 w-4" aria-hidden />
            )}
            {saveLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function SharedUpstreamAccountDetailDrawer(props: SharedUpstreamAccountDetailDrawerProps) {
  if (props.accountId == null) {
    return null;
  }
  return <SharedUpstreamAccountDetailDrawerInner {...props} />;
}

function SharedUpstreamAccountDetailDrawerInner({
  open,
  accountId,
  initialTab = "overview",
  initialDeleteConfirmOpen = false,
  presentation = "overlay",
  onInitialDeleteConfirmHandled,
  onClose,
}: SharedUpstreamAccountDetailDrawerProps) {
  const { t } = useTranslation();
  const navigate = useNavigate();
  const isCompactViewport = useCompactViewport();
  const { openUpstreamAccount } = useUpstreamAccountDetailRoute();
  const [detailTab, setDetailTab] = useState<AccountDetailTab>(initialTab);
  const needsRosterContext = open && ACCOUNT_DETAIL_TABS_REQUIRING_ROSTER_CONTEXT.has(detailTab);
  const {
    items,
    groups = [],
    hasUngroupedAccounts = false,
    writesEnabled,
    selectedId,
    selectedSummary,
    detail,
    isDetailLoading,
    detailError = null,
    isDetailRecentActionsHydrated,
    selectAccount,
    loadDetail,
    saveAccount,
    runSync,
    removeAccount,
    saveGroupNote,
    deleteGroupNote,
    forwardProxyNodes = [],
    forwardProxyCatalogState,
    missingDetailAccountId,
  } = useUpstreamAccounts(needsRosterContext ? undefined : null, {
    allowSelectionOutsideList: true,
    fallbackToFirstItem: false,
  });
  const availableModelOptions = useAvailableModelOptions(writesEnabled);
  const notifyMotherSwitches = useMotherSwitchNotifications();
  const [draft, setDraft] = useState<AccountDraft>(buildDraft(null));
  const [actionError, setActionError] = useState<ActionErrorState>(() => ({
    routing: null,
    accountMessages: {},
  }));
  const [busyAction, setBusyAction] = useState<BusyActionState>(() => ({
    routing: false,
    accountActions: new Set(),
  }));
  const [inlinePolicyBusyField, setInlinePolicyBusyField] = useState<InlinePolicyField | null>(
    null,
  );
  const [inlinePolicyErrors, setInlinePolicyErrors] = useState<
    Partial<Record<InlinePolicyField, string | null>>
  >({});
  const [isDeleteConfirmOpen, setIsDeleteConfirmOpen] = useState(false);
  const [stickyConversationSelectionValue, setStickyConversationSelectionValue] = useState(
    DEFAULT_STICKY_CONVERSATION_SELECTION_VALUE,
  );
  const [expandedStickyKeys, setExpandedStickyKeys] = useState<string[]>([]);
  const [focusedAttemptId, setFocusedAttemptId] = useState<string | null>(null);
  const [accountRecords, setAccountRecords] = useState<ApiInvocation[]>([]);
  const [accountRecordsMode, setAccountRecordsMode] = useState<AccountRecordsMode>("latest");
  const [accountRecordsFirstPage, setAccountRecordsFirstPage] = useState(0);
  const [accountRecordsPage, setAccountRecordsPage] = useState(0);
  const [accountRecordsTotal, setAccountRecordsTotal] = useState(0);
  const [accountRecordsHasNewer, setAccountRecordsHasNewer] = useState(false);
  const [accountRecordsHasMore, setAccountRecordsHasMore] = useState(false);
  const [accountRecordsLoading, setAccountRecordsLoading] = useState(false);
  const [accountRecordsError, setAccountRecordsError] = useState<string | null>(null);
  const [accountRecordsLocateError, setAccountRecordsLocateError] =
    useState<AccountRecordsLocateError | null>(null);
  const [accountRecordsScrollTarget, setAccountRecordsScrollTarget] = useState<{
    invokeId: string;
    attemptId?: string | null;
    version: number;
  } | null>(null);
  const [detailDrawerBodyElement, setDetailDrawerBodyElement] = useState<HTMLDivElement | null>(
    null,
  );
  const [groupDraftNotes, setGroupDraftNotes] = useState<Record<string, string>>({});
  const [groupDraftBoundProxyKeys, setGroupDraftBoundProxyKeys] = useState<
    Record<string, string[]>
  >({});
  const [groupDraftConcurrencyLimits, setGroupDraftConcurrencyLimits] = useState<
    Record<string, number>
  >({});
  const [groupDraftNodeShuntEnabled, setGroupDraftNodeShuntEnabled] = useState<
    Record<string, boolean>
  >({});
  const [groupDraftSingleAccountRotationEnabled, setGroupDraftSingleAccountRotationEnabled] =
    useState<Record<string, boolean>>({});
  const [groupDraftUpstream429RetryEnabled, setGroupDraftUpstream429RetryEnabled] = useState<
    Record<string, boolean>
  >({});
  const [groupDraftUpstream429MaxRetries, setGroupDraftUpstream429MaxRetries] = useState<
    Record<string, number>
  >({});
  const [detailDrawerPortalContainer, setDetailDrawerPortalContainer] =
    useState<HTMLElement | null>(null);
  const previousAccountRecordsContextRef = useRef<{
    open: boolean;
    accountId: number | null;
    detailTab: AccountDetailTab;
  } | null>(null);
  const deleteConfirmCancelRef = useRef<HTMLButtonElement | null>(null);
  const detailDrawerTitleId = "upstream-account-detail-title";
  const detailDrawerTabsBaseId = useId();
  const deleteConfirmTitleId = useId();
  const selectedIdRef = useRef<number | null>(selectedId);
  const routeAccountIdRef = useRef<number | null>(accountId);
  const drawerOpenRef = useRef(open);
  const accountRecordsRequestSeqRef = useRef(0);
  const accountRecordsModeRef = useRef<AccountRecordsMode>("latest");
  const accountRecordsSnapshotIdRef = useRef<number | null>(null);
  const accountRecordsAnchorIdRef = useRef<string | null>(null);
  const accountRecordsRef = useRef<ApiInvocation[]>([]);
  const accountRecordsPrependScrollRef = useRef<{
    requestSeq: number;
    scrollHeight: number;
    scrollTop: number;
  } | null>(null);
  const detailDrawerBodyElementRef = useRef<HTMLDivElement | null>(null);
  const accountRecordsLocateAlertRef = useRef<HTMLDivElement | null>(null);
  const accountRecordsAnchorScrollGuardUntilRef = useRef(0);
  useEffect(() => {
    setFocusedAttemptId(null);
  }, []);
  const draftSessionKeyRef = useRef<string | null>(null);
  const activeDraftSessionKeyRef = useRef<string | null>(null);
  const draftBaselineRef = useRef<AccountDraft>(buildDraft(null));
  const latestServerDraftRef = useRef<AccountDraft>(buildDraft(null));
  const knownRemovedTagIdsRef = useRef<Set<number>>(new Set());
  const pendingSaveSessionsRef = useRef<Map<number, PendingSaveSession>>(new Map());
  const recentSaveResponseGuardsRef = useRef<Map<number, RecentSaveResponseGuard>>(new Map());
  const draftSessionSnapshotRef = useRef({
    open,
    accountId,
    version: 0,
  });
  let draftSessionVersion = draftSessionSnapshotRef.current.version;
  if (
    draftSessionSnapshotRef.current.open !== open ||
    draftSessionSnapshotRef.current.accountId !== accountId
  ) {
    const closedActiveSession = draftSessionSnapshotRef.current.open && !open;
    const openedVisibleSession = !draftSessionSnapshotRef.current.open && open;
    const switchedVisibleAccount =
      draftSessionSnapshotRef.current.open &&
      open &&
      draftSessionSnapshotRef.current.accountId !== accountId;
    if (closedActiveSession || openedVisibleSession || switchedVisibleAccount) {
      draftSessionVersion += 1;
    }
    draftSessionSnapshotRef.current = {
      open,
      accountId,
      version: draftSessionVersion,
    };
  }
  const activeDraftSessionKey =
    open && accountId != null ? `${draftSessionVersion}:${accountId}` : null;

  selectedIdRef.current = selectedId;
  routeAccountIdRef.current = accountId;
  drawerOpenRef.current = open;
  activeDraftSessionKeyRef.current = activeDraftSessionKey;

  accountRecordsRef.current = accountRecords;
  accountRecordsModeRef.current = accountRecordsMode;
  detailDrawerBodyElementRef.current = detailDrawerBodyElement;

  useEffect(() => {
    if (open && accountId != null) {
      selectAccount(accountId);
      return;
    }
    selectAccount(null);
  }, [accountId, open, selectAccount]);

  useEffect(
    () => () => {
      recentSaveResponseGuardsRef.current.clear();
    },
    [],
  );

  useEffect(() => {
    if (!open || accountId == null) return;
    if (missingDetailAccountId !== accountId) return;
    onClose({ replace: true });
  }, [accountId, missingDetailAccountId, onClose, open]);

  useEffect(() => {
    if (!open) return;
    setDetailTab(initialTab);
  }, [initialTab, open]);

  useEffect(() => {
    const nextBaseline = removeAccountDraftTagIds(
      buildDraft(detail?.id === accountId ? detail : null),
      knownRemovedTagIdsRef.current,
    );
    if (activeDraftSessionKey == null) {
      draftSessionKeyRef.current = null;
      draftBaselineRef.current = nextBaseline;
      latestServerDraftRef.current = nextBaseline;
      return;
    }

    const previousBaseline = draftBaselineRef.current;
    const previousLatestServerDraft = latestServerDraftRef.current;
    const shouldSeedDraft = draftSessionKeyRef.current !== activeDraftSessionKey;
    draftSessionKeyRef.current = activeDraftSessionKey;

    const recentSaveResponseGuard =
      accountId == null ? null : (recentSaveResponseGuardsRef.current.get(accountId) ?? null);
    const retainedServerDraft = recentSaveResponseGuard?.retainedDraft ?? previousLatestServerDraft;
    const hasAcceptedFresherServerDraft =
      recentSaveResponseGuard != null &&
      !areAccountDraftsEqual(retainedServerDraft, recentSaveResponseGuard.fallbackDraft);
    const shouldIgnoreRecentSaveResponse =
      recentSaveResponseGuard != null &&
      recentSaveResponseGuard.accountId === accountId &&
      recentSaveResponseGuard.sessionKey !== activeDraftSessionKey &&
      hasAcceptedFresherServerDraft &&
      areAccountDraftsEqual(nextBaseline, recentSaveResponseGuard.draft);
    const shouldApplyRecentSaveResponse =
      recentSaveResponseGuard != null &&
      recentSaveResponseGuard.accountId === accountId &&
      recentSaveResponseGuard.sessionKey !== activeDraftSessionKey &&
      areAccountDraftsEqual(nextBaseline, recentSaveResponseGuard.draft);
    if (shouldIgnoreRecentSaveResponse) {
      recentSaveResponseGuardsRef.current.delete(accountId);
      draftBaselineRef.current = retainedServerDraft;
      latestServerDraftRef.current = retainedServerDraft;
      if (shouldSeedDraft) {
        setDraft(retainedServerDraft);
      }
      return;
    }

    if (shouldApplyRecentSaveResponse) {
      latestServerDraftRef.current = nextBaseline;
      setDraft((current) => {
        const matchesSeedBaseline = areAccountDraftsEqual(current, previousBaseline);
        const matchesLatestServerDraft = areAccountDraftsEqual(current, previousLatestServerDraft);
        const matchesStartedDraft = areAccountDraftsEqual(
          current,
          recentSaveResponseGuard.startedDraft,
        );
        const matchesResponseDraft = areAccountDraftsEqual(current, recentSaveResponseGuard.draft);
        recentSaveResponseGuardsRef.current.delete(accountId);
        draftBaselineRef.current = nextBaseline;
        if (shouldSeedDraft || matchesSeedBaseline || matchesLatestServerDraft) {
          return nextBaseline;
        }
        if (matchesStartedDraft || matchesResponseDraft) {
          return mergeDraftAfterAccountSave(
            current,
            recentSaveResponseGuard.startedDraft,
            nextBaseline,
          );
        }
        return current;
      });
      return;
    }

    if (
      recentSaveResponseGuard != null &&
      recentSaveResponseGuard.accountId === accountId &&
      recentSaveResponseGuard.sessionKey !== activeDraftSessionKey &&
      !areAccountDraftsEqual(nextBaseline, recentSaveResponseGuard.draft)
    ) {
      recentSaveResponseGuard.retainedDraft = nextBaseline;
    }

    latestServerDraftRef.current = nextBaseline;
    setDraft((current) => {
      const matchesSeedBaseline = areAccountDraftsEqual(current, previousBaseline);
      const matchesLatestServerDraft = areAccountDraftsEqual(current, previousLatestServerDraft);
      if (shouldSeedDraft || matchesSeedBaseline || matchesLatestServerDraft) {
        draftBaselineRef.current = nextBaseline;
        return nextBaseline;
      }
      return current;
    });
  }, [accountId, activeDraftSessionKey, detail]);

  useEffect(() => {
    if (!open) {
      setDetailTab("overview");
      setIsDeleteConfirmOpen(false);
    }
  }, [open]);

  useEffect(() => {
    if (!open || !initialDeleteConfirmOpen) return;
    setIsDeleteConfirmOpen(true);
    onInitialDeleteConfirmHandled?.();
  }, [initialDeleteConfirmOpen, onInitialDeleteConfirmHandled, open]);

  useEffect(() => {
    setDetailTab(initialTab);
    setExpandedStickyKeys([]);
  }, [initialTab]);

  useEffect(() => {
    setGroupDraftNotes((current) => {
      const nextEntries = Object.entries(current).filter(
        ([groupName]) => !isExistingGroup(groups, groupName),
      );
      if (nextEntries.length === Object.keys(current).length) return current;
      return Object.fromEntries(nextEntries);
    });
    setGroupDraftBoundProxyKeys((current) => {
      const nextEntries = Object.entries(current).filter(
        ([groupName]) => !isExistingGroup(groups, groupName),
      );
      if (nextEntries.length === Object.keys(current).length) return current;
      return Object.fromEntries(nextEntries);
    });
    setGroupDraftNodeShuntEnabled((current) => {
      const nextEntries = Object.entries(current).filter(
        ([groupName]) => !isExistingGroup(groups, groupName),
      );
      if (nextEntries.length === Object.keys(current).length) return current;
      return Object.fromEntries(nextEntries);
    });
    setGroupDraftSingleAccountRotationEnabled((current) => {
      const nextEntries = Object.entries(current).filter(
        ([groupName]) => !isExistingGroup(groups, groupName),
      );
      if (nextEntries.length === Object.keys(current).length) return current;
      return Object.fromEntries(nextEntries);
    });
    setGroupDraftConcurrencyLimits((current) => {
      const nextEntries = Object.entries(current).filter(
        ([groupName]) => !isExistingGroup(groups, groupName),
      );
      if (nextEntries.length === Object.keys(current).length) return current;
      return Object.fromEntries(nextEntries);
    });
    setGroupDraftUpstream429RetryEnabled((current) => {
      const nextEntries = Object.entries(current).filter(
        ([groupName]) => !isExistingGroup(groups, groupName),
      );
      if (nextEntries.length === Object.keys(current).length) return current;
      return Object.fromEntries(nextEntries);
    });
    setGroupDraftUpstream429MaxRetries((current) => {
      const nextEntries = Object.entries(current).filter(
        ([groupName]) => !isExistingGroup(groups, groupName),
      );
      if (nextEntries.length === Object.keys(current).length) return current;
      return Object.fromEntries(nextEntries);
    });
  }, [groups]);

  const availableGroups = useMemo(() => {
    const draftNames = Object.fromEntries([
      ...Object.keys(groupDraftNotes).map((groupName) => [groupName, ""]),
      ...Object.keys(groupDraftBoundProxyKeys).map((groupName) => [groupName, ""]),
      ...Object.keys(groupDraftConcurrencyLimits).map((groupName) => [groupName, ""]),
      ...Object.keys(groupDraftNodeShuntEnabled).map((groupName) => [groupName, ""]),
      ...Object.keys(groupDraftSingleAccountRotationEnabled).map((groupName) => [groupName, ""]),
      ...Object.keys(groupDraftUpstream429RetryEnabled).map((groupName) => [groupName, ""]),
      ...Object.keys(groupDraftUpstream429MaxRetries).map((groupName) => [groupName, ""]),
    ]);
    return {
      options: buildGroupOptions(
        items.map((item) => item.groupName),
        groups,
        draftNames,
      ),
      hasUngrouped: hasUngroupedAccounts,
    };
  }, [
    groupDraftBoundProxyKeys,
    groupDraftConcurrencyLimits,
    groupDraftNodeShuntEnabled,
    groupDraftSingleAccountRotationEnabled,
    groupDraftNotes,
    groupDraftUpstream429MaxRetries,
    groupDraftUpstream429RetryEnabled,
    groups,
    hasUngroupedAccounts,
    items,
  ]);
  const formatGroupAccountCountLabel = useCallback(
    (count: number) => t("accountPool.upstreamAccounts.groupOptionCount", { count }),
    [t],
  );

  const resolveGroupSummaryForName = useCallback(
    (groupName: string) => {
      const normalized = normalizeGroupName(groupName);
      if (!normalized) return null;
      return groups.find((group) => normalizeGroupName(group.groupName) === normalized) ?? null;
    },
    [groups],
  );

  const resolveGroupNoteForName = useCallback(
    (groupName: string) => resolveGroupNote(groups, groupDraftNotes, groupName),
    [groupDraftNotes, groups],
  );

  const resolveGroupConcurrencyLimitForName = useCallback(
    (groupName: string) =>
      resolveGroupConcurrencyLimit(groups, groupDraftConcurrencyLimits, groupName),
    [groupDraftConcurrencyLimits, groups],
  );

  const resolvePendingGroupNoteForName = useCallback(
    (groupName: string) => {
      const normalized = normalizeGroupName(groupName);
      if (!normalized || isExistingGroup(groups, normalized)) return "";
      return groupDraftNotes[normalized]?.trim() ?? "";
    },
    [groupDraftNotes, groups],
  );

  const resolveGroupBoundProxyKeysForName = useCallback(
    (groupName: string) =>
      resolveGroupSummaryForName(groupName)?.boundProxyKeys ??
      groupDraftBoundProxyKeys[normalizeGroupName(groupName)] ??
      [],
    [groupDraftBoundProxyKeys, resolveGroupSummaryForName],
  );

  const resolveGroupNodeShuntEnabledForName = useCallback(
    (groupName: string) => {
      const normalizedGroupName = normalizeGroupName(groupName);
      if (!normalizedGroupName) return false;
      const existingGroup = resolveGroupSummaryForName(normalizedGroupName);
      if (existingGroup) {
        return existingGroup.nodeShuntEnabled === true;
      }
      return groupDraftNodeShuntEnabled[normalizedGroupName] === true;
    },
    [groupDraftNodeShuntEnabled, resolveGroupSummaryForName],
  );

  const resolveGroupSingleAccountRotationEnabledForName = useCallback(
    (groupName: string) => {
      const normalizedGroupName = normalizeGroupName(groupName);
      if (!normalizedGroupName) return false;
      const existingGroup = resolveGroupSummaryForName(normalizedGroupName);
      if (existingGroup) {
        return existingGroup.singleAccountRotationEnabled === true;
      }
      return groupDraftSingleAccountRotationEnabled[normalizedGroupName] === true;
    },
    [groupDraftSingleAccountRotationEnabled, resolveGroupSummaryForName],
  );

  const resolveGroupUpstream429RetryEnabledForName = useCallback(
    (groupName: string) => {
      const normalizedGroupName = normalizeGroupName(groupName);
      if (!normalizedGroupName) return false;
      const existingGroup = resolveGroupSummaryForName(normalizedGroupName);
      if (existingGroup) {
        return existingGroup.upstream429RetryEnabled === true;
      }
      return groupDraftUpstream429RetryEnabled[normalizedGroupName] === true;
    },
    [groupDraftUpstream429RetryEnabled, resolveGroupSummaryForName],
  );

  const resolveGroupUpstream429MaxRetriesForName = useCallback(
    (groupName: string) => {
      const normalizedGroupName = normalizeGroupName(groupName);
      if (!normalizedGroupName) return 0;
      const existingGroup = resolveGroupSummaryForName(normalizedGroupName);
      const retryEnabled = existingGroup
        ? existingGroup.upstream429RetryEnabled === true
        : groupDraftUpstream429RetryEnabled[normalizedGroupName] === true;
      const rawValue = existingGroup
        ? existingGroup.upstream429MaxRetries
        : groupDraftUpstream429MaxRetries[normalizedGroupName];
      return retryEnabled
        ? normalizeEnabledGroupUpstream429MaxRetries(rawValue)
        : normalizeGroupUpstream429MaxRetries(rawValue);
    },
    [
      groupDraftUpstream429MaxRetries,
      groupDraftUpstream429RetryEnabled,
      resolveGroupSummaryForName,
    ],
  );

  const hasGroupSettings = useCallback(
    (groupName: string) =>
      resolveGroupNoteForName(groupName).trim().length > 0 ||
      resolveGroupBoundProxyKeysForName(groupName).length > 0 ||
      resolveGroupConcurrencyLimitForName(groupName) > 0 ||
      resolveGroupNodeShuntEnabledForName(groupName) ||
      resolveGroupSingleAccountRotationEnabledForName(groupName) ||
      resolveGroupUpstream429RetryEnabledForName(groupName) ||
      resolveGroupUpstream429MaxRetriesForName(groupName) > 0,
    [
      resolveGroupBoundProxyKeysForName,
      resolveGroupConcurrencyLimitForName,
      resolveGroupNodeShuntEnabledForName,
      resolveGroupSingleAccountRotationEnabledForName,
      resolveGroupNoteForName,
      resolveGroupUpstream429MaxRetriesForName,
      resolveGroupUpstream429RetryEnabledForName,
    ],
  );

  const clearDraftGroupSettings = useCallback((groupName: string) => {
    const normalizedGroupName = normalizeGroupName(groupName);
    if (!normalizedGroupName) return;
    setGroupDraftNotes((current) => {
      if (!(normalizedGroupName in current)) return current;
      const next = { ...current };
      delete next[normalizedGroupName];
      return next;
    });
    setGroupDraftBoundProxyKeys((current) => {
      if (!(normalizedGroupName in current)) return current;
      const next = { ...current };
      delete next[normalizedGroupName];
      return next;
    });
    setGroupDraftNodeShuntEnabled((current) => {
      if (!(normalizedGroupName in current)) return current;
      const next = { ...current };
      delete next[normalizedGroupName];
      return next;
    });
    setGroupDraftSingleAccountRotationEnabled((current) => {
      if (!(normalizedGroupName in current)) return current;
      const next = { ...current };
      delete next[normalizedGroupName];
      return next;
    });
    setGroupDraftConcurrencyLimits((current) => {
      if (!(normalizedGroupName in current)) return current;
      const next = { ...current };
      delete next[normalizedGroupName];
      return next;
    });
    setGroupDraftUpstream429RetryEnabled((current) => {
      if (!(normalizedGroupName in current)) return current;
      const next = { ...current };
      delete next[normalizedGroupName];
      return next;
    });
    setGroupDraftUpstream429MaxRetries((current) => {
      if (!(normalizedGroupName in current)) return current;
      const next = { ...current };
      delete next[normalizedGroupName];
      return next;
    });
  }, []);

  const { openEditor: openGroupNoteEditor, dialog: groupNoteDialog } =
    useUpstreamAccountGroupSettingsDialog({
      writesEnabled,
      container: detailDrawerPortalContainer,
      resolveGroupState: useCallback(
        (groupName) => {
          const normalized = normalizeGroupName(groupName);
          if (!normalized) return null;
          const existingGroup = resolveGroupSummaryForName(normalized);
          return {
            groupName: normalized,
            note: resolveGroupNoteForName(normalized),
            existing: existingGroup != null,
            accountCount: existingGroup?.accountCount ?? 0,
            concurrencyLimit: resolveGroupConcurrencyLimitForName(normalized),
            boundProxyKeys: resolveGroupBoundProxyKeysForName(normalized),
            nodeShuntEnabled: resolveGroupNodeShuntEnabledForName(normalized),
            singleAccountRotationEnabled:
              resolveGroupSingleAccountRotationEnabledForName(normalized),
            upstream429RetryEnabled: resolveGroupUpstream429RetryEnabledForName(normalized),
            upstream429MaxRetries: resolveGroupUpstream429MaxRetriesForName(normalized),
            routingRule: existingGroup?.routingRule,
            effectiveTimeouts: existingGroup?.effectiveTimeouts ?? null,
            timeoutFieldSources: existingGroup?.timeoutFieldSources ?? null,
          };
        },
        [
          resolveGroupBoundProxyKeysForName,
          resolveGroupConcurrencyLimitForName,
          resolveGroupNodeShuntEnabledForName,
          resolveGroupSingleAccountRotationEnabledForName,
          resolveGroupNoteForName,
          resolveGroupSummaryForName,
          resolveGroupUpstream429MaxRetriesForName,
          resolveGroupUpstream429RetryEnabledForName,
        ],
      ),
      saveGroupSettings: useCallback(
        async (groupName, payload) => {
          const normalizedGroupName = normalizeGroupName(groupName);
          if (!normalizedGroupName) return;

          const normalizedNote = payload.note?.trim() ?? "";
          const normalizedBoundProxyKeys = Array.from(
            new Set(
              (payload.boundProxyKeys ?? [])
                .map((value) => value.trim())
                .filter((value) => value.length > 0),
            ),
          );
          const normalizedConcurrencyLimit = payload.concurrencyLimit ?? 0;
          const normalizedNodeShuntEnabled = payload.nodeShuntEnabled === true;
          const normalizedSingleAccountRotationEnabled =
            payload.singleAccountRotationEnabled === true;
          const normalizedUpstream429RetryEnabled = payload.upstream429RetryEnabled === true;
          const normalizedUpstream429MaxRetries = normalizedUpstream429RetryEnabled
            ? normalizeEnabledGroupUpstream429MaxRetries(payload.upstream429MaxRetries)
            : normalizeGroupUpstream429MaxRetries(payload.upstream429MaxRetries);

          await saveGroupNote(normalizedGroupName, {
            note: normalizedNote || undefined,
            boundProxyKeys: normalizedBoundProxyKeys,
            concurrencyLimit: normalizedConcurrencyLimit,
            nodeShuntEnabled: normalizedNodeShuntEnabled,
            singleAccountRotationEnabled: normalizedSingleAccountRotationEnabled,
            upstream429RetryEnabled: normalizedUpstream429RetryEnabled,
            upstream429MaxRetries: normalizedUpstream429MaxRetries,
            routingRule: payload.routingRule,
          });
          clearDraftGroupSettings(normalizedGroupName);
        },
        [clearDraftGroupSettings, saveGroupNote],
      ),
      deleteGroupSettings: useCallback(
        async (groupName: string) => {
          await deleteGroupNote(groupName);
          clearDraftGroupSettings(groupName);
          setDraft((current) =>
            normalizeGroupName(current.groupName) === normalizeGroupName(groupName)
              ? {
                  ...current,
                  groupName: "",
                }
              : current,
          );
        },
        [clearDraftGroupSettings, deleteGroupNote],
      ),
    });
  const handleDetailGroupCreateRequest = useCallback(
    (groupName: string) => {
      openGroupNoteEditor(groupName, {
        onSaved: (savedGroupName) =>
          setDraft((current) => ({
            ...current,
            groupName: savedGroupName,
          })),
        onDeleted: (deletedGroupName) =>
          setDraft((current) =>
            normalizeGroupName(current.groupName) === deletedGroupName
              ? {
                  ...current,
                  groupName: "",
                }
              : current,
          ),
      });
    },
    [openGroupNoteEditor],
  );
  const stickyConversationSelection = STICKY_CONVERSATION_SELECTION_LOOKUP.get(
    stickyConversationSelectionValue,
  ) ??
    STICKY_CONVERSATION_SELECTION_LOOKUP.get(DEFAULT_STICKY_CONVERSATION_SELECTION_VALUE) ?? {
      mode: "count",
      limit: 50,
    };
  const stickyConversationSelectionOptions = useMemo(
    () =>
      STICKY_CONVERSATION_SELECTION_OPTIONS.map((option) => ({
        value: option.value,
        label:
          option.selection.mode === "count"
            ? t("live.conversations.option.count", {
                count: option.selection.limit,
              })
            : t("live.conversations.option.activityHours", {
                hours: option.selection.activityHours,
              }),
      })),
    [t],
  );
  const {
    stats: stickyConversationStats,
    isLoading: stickyConversationLoading,
    error: stickyConversationError,
  } = useUpstreamStickyConversations(
    selectedId,
    stickyConversationSelection,
    Boolean(open && selectedId && detailTab === "routing"),
  );
  const visibleStickyKeys = useMemo(
    () =>
      stickyConversationStats?.conversations.map((conversation) => conversation.stickyKey) ?? [],
    [stickyConversationStats],
  );
  const hasVisibleStickyConversations = visibleStickyKeys.length > 0;
  const allVisibleStickyKeysExpanded =
    hasVisibleStickyConversations &&
    visibleStickyKeys.every((stickyKey) => expandedStickyKeys.includes(stickyKey));

  useEffect(() => {
    if (!stickyConversationStats) return;
    const visibleStickyKeySet = new Set(
      stickyConversationStats.conversations.map((conversation) => conversation.stickyKey),
    );
    setExpandedStickyKeys((current) => {
      const next = current.filter((stickyKey) => visibleStickyKeySet.has(stickyKey));
      return next.length === current.length ? current : next;
    });
  }, [stickyConversationStats]);

  const loadAccountRecordsPage = useCallback(
    (
      page: number,
      mode: "replace" | "append" | "prepend",
      pageSize = ACCOUNT_RECORD_PAGE_SIZE,
    ): void => {
      const requestSeq = accountRecordsRequestSeqRef.current + 1;
      accountRecordsRequestSeqRef.current = requestSeq;
      if (
        !LEGACY_ACCOUNT_RECORDS_ENABLED ||
        !open ||
        accountId == null ||
        detailTab !== "records"
      ) {
        return;
      }

      const recordsScrollElement = detailDrawerBodyElementRef.current;
      if (mode === "prepend" && recordsScrollElement) {
        accountRecordsPrependScrollRef.current = {
          requestSeq,
          scrollHeight: recordsScrollElement.scrollHeight,
          scrollTop: recordsScrollElement.scrollTop,
        };
      }
      setAccountRecordsLoading(true);
      setAccountRecordsError(null);
      void fetchInvocationRecords({
        upstreamAccountId: accountId,
        page,
        pageSize,
        snapshotId:
          mode === "replace" ? undefined : (accountRecordsSnapshotIdRef.current ?? undefined),
        anchorId: mode === "replace" ? undefined : (accountRecordsAnchorIdRef.current ?? undefined),
        sortBy: "occurredAt",
        sortOrder: "desc",
      })
        .then((response) => {
          if (requestSeq !== accountRecordsRequestSeqRef.current) {
            if (accountRecordsPrependScrollRef.current?.requestSeq === requestSeq) {
              accountRecordsPrependScrollRef.current = null;
            }
            return;
          }
          const responseSnapshotId =
            typeof response.snapshotId === "number" && Number.isFinite(response.snapshotId)
              ? response.snapshotId
              : null;
          const responsePage =
            typeof response.page === "number" && Number.isFinite(response.page)
              ? response.page
              : page;
          const responsePageSize =
            typeof response.pageSize === "number" && Number.isFinite(response.pageSize)
              ? response.pageSize
              : pageSize;
          const responseTotal =
            typeof response.total === "number" && Number.isFinite(response.total)
              ? response.total
              : response.records.length;
          if (mode === "replace" && responseSnapshotId != null) {
            accountRecordsSnapshotIdRef.current = responseSnapshotId;
          }
          setAccountRecords((current) => {
            if (mode === "replace") return response.records;
            const seen = new Set(current.map((record) => invocationStableKey(record)));
            const incoming = response.records.filter((record) => {
              const key = invocationStableKey(record);
              if (seen.has(key)) return false;
              seen.add(key);
              return true;
            });
            if (mode === "prepend") return [...incoming, ...current];
            return [...current, ...incoming];
          });
          if (mode !== "prepend") setAccountRecordsPage(responsePage);
          if (mode !== "append") setAccountRecordsFirstPage(responsePage);
          setAccountRecordsTotal(responseTotal);
          setAccountRecordsHasNewer(responsePage > 1 || mode === "append");
          if (mode !== "prepend") {
            setAccountRecordsHasMore(responsePage * responsePageSize < responseTotal);
          }
        })
        .catch((error) => {
          if (accountRecordsPrependScrollRef.current?.requestSeq === requestSeq) {
            accountRecordsPrependScrollRef.current = null;
          }
          if (requestSeq !== accountRecordsRequestSeqRef.current) return;
          setAccountRecordsError(error instanceof Error ? error.message : String(error));
        })
        .finally(() => {
          if (requestSeq === accountRecordsRequestSeqRef.current) {
            setAccountRecordsLoading(false);
          }
        });
    },
    [accountId, detailTab, open],
  );

  const reloadAccountRecords = useCallback((): void => {
    setAccountRecordsMode("latest");
    setAccountRecordsLocateError(null);
    setAccountRecordsScrollTarget(null);
    accountRecordsAnchorScrollGuardUntilRef.current = 0;
    accountRecordsPrependScrollRef.current = null;
    accountRecordsSnapshotIdRef.current = null;
    accountRecordsAnchorIdRef.current = null;
    loadAccountRecordsPage(1, "replace");
  }, [loadAccountRecordsPage]);

  const resyncAccountRecords = useCallback((): void => {
    const loadedPageWindow =
      Math.ceil(accountRecordsRef.current.length / ACCOUNT_RECORD_PAGE_SIZE) *
      ACCOUNT_RECORD_PAGE_SIZE;
    loadAccountRecordsPage(1, "replace", Math.max(ACCOUNT_RECORD_PAGE_SIZE, loadedPageWindow));
  }, [loadAccountRecordsPage]);

  const loadMoreAccountRecords = useCallback((): void => {
    if (accountRecordsLoading || !accountRecordsHasMore) return;
    loadAccountRecordsPage(accountRecordsPage + 1, "append");
  }, [accountRecordsHasMore, accountRecordsLoading, accountRecordsPage, loadAccountRecordsPage]);

  const loadNewerAccountRecords = useCallback((): void => {
    if (
      accountRecordsMode !== "anchored" ||
      accountRecordsLoading ||
      !accountRecordsHasNewer ||
      accountRecordsFirstPage <= 1
    ) {
      return;
    }
    loadAccountRecordsPage(accountRecordsFirstPage - 1, "prepend");
  }, [
    accountRecordsFirstPage,
    accountRecordsHasNewer,
    accountRecordsLoading,
    accountRecordsMode,
    loadAccountRecordsPage,
  ]);

  const locateAccountRecord = useCallback(
    (invokeId: string): void => {
      const normalizedInvokeId = invokeId.trim();
      if (!normalizedInvokeId || accountId == null) return;
      const requestSeq = accountRecordsRequestSeqRef.current + 1;
      accountRecordsRequestSeqRef.current = requestSeq;
      setDetailTab("records");
      setAccountRecordsMode("anchored");
      setAccountRecordsLoading(true);
      setAccountRecords([]);
      setAccountRecordsFirstPage(0);
      setAccountRecordsPage(0);
      setAccountRecordsTotal(0);
      setAccountRecordsHasNewer(false);
      setAccountRecordsHasMore(false);
      accountRecordsSnapshotIdRef.current = null;
      accountRecordsAnchorIdRef.current = null;
      accountRecordsPrependScrollRef.current = null;
      setAccountRecordsError(null);
      setAccountRecordsLocateError(null);
      setAccountRecordsScrollTarget(null);

      void fetchInvocationRecordLocation({
        requestId: normalizedInvokeId,
        upstreamAccountId: accountId,
        pageSize: ACCOUNT_RECORD_PAGE_SIZE,
      })
        .then((response) => {
          if (requestSeq !== accountRecordsRequestSeqRef.current) return;
          accountRecordsSnapshotIdRef.current = response.snapshotId;
          accountRecordsAnchorIdRef.current = response.anchorId;
          setAccountRecords(response.records);
          setAccountRecordsFirstPage(response.page);
          setAccountRecordsPage(response.page);
          setAccountRecordsTotal(response.total);
          setAccountRecordsHasNewer(response.page > 1);
          setAccountRecordsHasMore(response.page * response.pageSize < response.total);
          setAccountRecordsScrollTarget({
            invokeId: normalizedInvokeId,
            version: requestSeq,
          });
          accountRecordsAnchorScrollGuardUntilRef.current = Date.now() + 1_000;
        })
        .catch((error) => {
          if (requestSeq !== accountRecordsRequestSeqRef.current) return;
          setAccountRecords([]);
          setAccountRecordsFirstPage(0);
          setAccountRecordsPage(0);
          setAccountRecordsTotal(0);
          setAccountRecordsHasNewer(false);
          setAccountRecordsHasMore(false);
          setAccountRecordsLocateError({
            invokeId: normalizedInvokeId,
            kind: error instanceof ApiRequestError && error.status === 404 ? "notFound" : "request",
          });
        })
        .finally(() => {
          if (requestSeq === accountRecordsRequestSeqRef.current) {
            setAccountRecordsLoading(false);
          }
        });
    },
    [accountId],
  );

  useLayoutEffect(() => {
    const pending = accountRecordsPrependScrollRef.current;
    if (!pending || !detailDrawerBodyElement) return;
    accountRecordsPrependScrollRef.current = null;
    detailDrawerBodyElement.scrollTop =
      pending.scrollTop + (detailDrawerBodyElement.scrollHeight - pending.scrollHeight);
  }, [detailDrawerBodyElement]);

  useLayoutEffect(() => {
    if (!accountRecordsLocateError) return;
    accountRecordsLocateAlertRef.current?.focus();
    const timeout = window.setTimeout(() => {
      accountRecordsLocateAlertRef.current?.focus();
    }, 0);
    return () => window.clearTimeout(timeout);
  }, [accountRecordsLocateError]);

  useEffect(() => {
    if (!LEGACY_ACCOUNT_RECORDS_ENABLED) return;
    const previous = previousAccountRecordsContextRef.current;
    const next = {
      open,
      accountId,
      detailTab,
    };
    previousAccountRecordsContextRef.current = next;

    const leftRecordsSurface =
      previous?.open &&
      previous.accountId != null &&
      previous.detailTab === "records" &&
      (!open || accountId == null || detailTab !== "records");
    const switchedAccounts =
      open &&
      detailTab === "records" &&
      previous?.open &&
      previous.detailTab === "records" &&
      previous.accountId != null &&
      accountId != null &&
      previous.accountId !== accountId;
    const enteredRecordsTab =
      open &&
      accountId != null &&
      detailTab === "records" &&
      previous?.open &&
      previous.accountId === accountId &&
      previous.detailTab !== "records";
    if (
      leftRecordsSurface ||
      switchedAccounts ||
      (enteredRecordsTab && accountRecordsModeRef.current === "latest")
    ) {
      accountRecordsRequestSeqRef.current += 1;
    }
    const shouldResetRecords =
      leftRecordsSurface ||
      switchedAccounts ||
      (enteredRecordsTab && accountRecordsModeRef.current === "latest");
    if (shouldResetRecords) {
      setAccountRecords([]);
      setAccountRecordsFirstPage(0);
      setAccountRecordsPage(0);
      setAccountRecordsTotal(0);
      setAccountRecordsHasNewer(false);
      setAccountRecordsHasMore(false);
      accountRecordsSnapshotIdRef.current = null;
      accountRecordsAnchorIdRef.current = null;
      accountRecordsPrependScrollRef.current = null;
      setAccountRecordsError(null);
      setAccountRecordsLocateError(null);
      setAccountRecordsScrollTarget(null);
      if (leftRecordsSurface || switchedAccounts) {
        setAccountRecordsMode("latest");
      }
      setAccountRecordsLoading(!leftRecordsSurface);
    }

    if (!open || accountId == null || detailTab !== "records") {
      return;
    }

    if (accountRecordsModeRef.current === "latest") {
      void reloadAccountRecords();
    }
  }, [accountId, detailTab, open, reloadAccountRecords]);

  useEffect(() => {
    if (!LEGACY_ACCOUNT_RECORDS_ENABLED) return;
    if (!open || accountId == null || detailTab !== "records") return;
    const scrollTarget = detailDrawerBodyElement;
    if (!scrollTarget) return;
    const handleScroll = () => {
      if (scrollTarget.scrollHeight <= scrollTarget.clientHeight) return;
      if (
        scrollTarget.scrollTop < 260 &&
        Date.now() >= accountRecordsAnchorScrollGuardUntilRef.current
      ) {
        loadNewerAccountRecords();
      }
      const remaining =
        scrollTarget.scrollHeight - scrollTarget.scrollTop - scrollTarget.clientHeight;
      if (remaining < 520) {
        loadMoreAccountRecords();
      }
    };
    handleScroll();
    scrollTarget.addEventListener("scroll", handleScroll, { passive: true });
    return () => {
      scrollTarget.removeEventListener("scroll", handleScroll);
    };
  }, [
    accountId,
    detailDrawerBodyElement,
    detailTab,
    loadMoreAccountRecords,
    loadNewerAccountRecords,
    open,
  ]);

  useInvocationRecordsRealtime({
    enabled: Boolean(
      LEGACY_ACCOUNT_RECORDS_ENABLED &&
        open &&
        accountId != null &&
        detailTab === "records" &&
        accountRecordsMode === "latest",
    ),
    isHydrated:
      Boolean(
        LEGACY_ACCOUNT_RECORDS_ENABLED &&
          open &&
          accountId != null &&
          detailTab === "records" &&
          accountRecordsMode === "latest",
      ) && !accountRecordsLoading,
    filters: accountId == null ? undefined : { upstreamAccountId: accountId },
    sortBy: "occurredAt",
    sortOrder: "desc",
    limit: Math.max(ACCOUNT_RECORD_PAGE_SIZE, accountRecords.length + ACCOUNT_RECORD_PAGE_SIZE),
    getRecords: () => accountRecordsRef.current,
    onRecordsChange: (next) => {
      setAccountRecords(next);
      setAccountRecordsError(null);
    },
    onOpenResync: resyncAccountRecords,
  });

  const selectedDetail = detail?.id === selectedId ? detail : null;
  const selected = selectedDetail ?? selectedSummary;
  const selectedAccountProxyKeys = normalizeProxyKeys(selectedDetail?.boundProxyKeys);
  const [accountProxyEditorOpen, setAccountProxyEditorOpen] = useState(false);
  const [accountProxyDraftKeys, setAccountProxyDraftKeys] = useState<string[]>([]);
  const selectedGroupProxyKeys = normalizeProxyKeys(
    selectedDetail?.groupName ? resolveGroupBoundProxyKeysForName(selectedDetail.groupName) : [],
  );
  const selectedEffectiveProxyKeys =
    selectedAccountProxyKeys.length > 0 ? selectedAccountProxyKeys : selectedGroupProxyKeys;
  const selectedProxyNodeByKey = new Map(forwardProxyNodes.map((node) => [node.key, node]));
  const accountProxyEditorBusy = Boolean(
    selectedDetail && hasBusyAccountAction(busyAction, selectedDetail.id),
  );
  const openAccountProxyEditor = useCallback(() => {
    setAccountProxyDraftKeys(selectedAccountProxyKeys);
    setAccountProxyEditorOpen(true);
  }, [selectedAccountProxyKeys]);
  const closeAccountProxyEditor = useCallback(() => {
    if (accountProxyEditorBusy) return;
    setAccountProxyEditorOpen(false);
  }, [accountProxyEditorBusy]);
  useEffect(() => {
    if (!open || !selectedDetail) {
      setAccountProxyEditorOpen(false);
    }
  }, [open, selectedDetail]);
  useEffect(() => {
    if (
      !open ||
      accountId == null ||
      detailTab !== "healthEvents" ||
      isDetailRecentActionsHydrated
    ) {
      return;
    }
    void loadDetail(accountId, { silent: true, includeRecentActions: true });
  }, [accountId, detailTab, isDetailRecentActionsHydrated, loadDetail, open]);
  const handleDetailDrawerClose = useCallback(() => {
    onClose();
  }, [onClose]);
  const handleSelectDetailTab = useCallback(
    (nextTab: AccountDetailTab) => {
      setDetailTab(nextTab);
      if (accountId != null) {
        openUpstreamAccount(accountId, {
          replace: true,
          tab: nextTab,
        });
      }
    },
    [accountId, openUpstreamAccount],
  );
  const locateAccountAttempt = useCallback(
    (attemptId: string | null | undefined) => {
      const normalizedAttemptId = attemptId?.trim() ?? "";
      if (!normalizedAttemptId) return;
      setFocusedAttemptId(normalizedAttemptId);
      handleSelectDetailTab("records");
    },
    [handleSelectDetailTab],
  );
  const selectedPlanBadge = upstreamPlanBadgeRecipe(selected?.planType);
  const selectedRecoveryHint = resolveOauthRecoveryHint(
    selectedDetail?.kind ?? selected?.kind ?? "",
    accountHealthStatus(selectedDetail ?? selected),
    selectedDetail?.lastError ?? selected?.lastError,
  );
  const selectedRecentActions = selectedDetail?.recentActions ?? [];
  const detailTabIds = {
    overview: {
      tab: `${detailDrawerTabsBaseId}-overview-tab`,
      panel: `${detailDrawerTabsBaseId}-overview-panel`,
    },
    records: {
      tab: `${detailDrawerTabsBaseId}-records-tab`,
      panel: `${detailDrawerTabsBaseId}-records-panel`,
    },
    edit: {
      tab: `${detailDrawerTabsBaseId}-edit-tab`,
      panel: `${detailDrawerTabsBaseId}-edit-panel`,
    },
    routing: {
      tab: `${detailDrawerTabsBaseId}-routing-tab`,
      panel: `${detailDrawerTabsBaseId}-routing-panel`,
    },
    healthEvents: {
      tab: `${detailDrawerTabsBaseId}-health-events-tab`,
      panel: `${detailDrawerTabsBaseId}-health-events-panel`,
    },
  } as const;
  const visibleAccountActionError =
    typeof selectedId === "number" ? (actionError.accountMessages[selectedId] ?? null) : null;
  const detailDisplayNameConflict = useMemo(
    () =>
      selectedDetail?.kind === "api_key_codex"
        ? findDisplayNameConflict(items, draft.displayName, selectedDetail?.id ?? null)
        : null,
    [draft.displayName, items, selectedDetail?.id, selectedDetail?.kind],
  );
  const draftUpstreamBaseUrlError = useMemo(() => {
    const code = validateUpstreamBaseUrl(draft.upstreamBaseUrl);
    if (code === "invalid_absolute_url") {
      return t("accountPool.upstreamAccounts.validation.upstreamBaseUrlInvalid");
    }
    if (code === "query_or_fragment_not_allowed") {
      return t("accountPool.upstreamAccounts.validation.upstreamBaseUrlNoQueryOrFragment");
    }
    return null;
  }, [draft.upstreamBaseUrl, t]);

  const accountKindLabel = (kind: string) =>
    kind === "oauth_codex"
      ? t("accountPool.upstreamAccounts.kind.oauth")
      : t("accountPool.upstreamAccounts.kind.apiKey");
  const accountEnableStatusLabel = (status: string) =>
    t(`accountPool.upstreamAccounts.enableStatus.${status}`);
  const accountWorkStatusLabel = (status: string) =>
    t(`accountPool.upstreamAccounts.workStatus.${status}`);
  const accountHealthStatusLabel = (status: string) =>
    t(`accountPool.upstreamAccounts.healthStatus.${status}`);
  const accountSyncStateLabel = (status: string) =>
    t(`accountPool.upstreamAccounts.syncState.${status}`);
  const accountActionLabel = (action?: string | null) => {
    if (!action) return null;
    const key = `accountPool.upstreamAccounts.latestAction.actions.${action}`;
    const translated = t(key);
    return translated === key ? action : translated;
  };
  const accountActionSourceLabel = (source?: string | null) => {
    if (!source) return null;
    const key = `accountPool.upstreamAccounts.latestAction.sources.${source}`;
    const translated = t(key);
    return translated === key ? source : translated;
  };
  const accountActionReasonLabel = (reason?: string | null) => {
    if (!reason) return null;
    const key = `accountPool.upstreamAccounts.latestAction.reasons.${reason}`;
    const translated = t(key);
    return translated === key ? reason : translated;
  };
  const formatDuplicateReasons = (duplicateInfo?: UpstreamAccountDuplicateInfo | null) => {
    const reasons = duplicateInfo?.reasons ?? [];
    return reasons
      .map((reason) => {
        if (reason === "sharedChatgptAccountId") {
          return t("accountPool.upstreamAccounts.duplicate.reasons.sharedChatgptAccountId");
        }
        if (reason === "sharedChatgptUserId") {
          return t("accountPool.upstreamAccounts.duplicate.reasons.sharedChatgptUserId");
        }
        return reason;
      })
      .join(" / ");
  };

  const notifyMotherChange = useCallback(
    (updated: UpstreamAccountSummary) => {
      const nextItems = applyMotherUpdateToItems(items, updated);
      notifyMotherSwitches(items, nextItems);
    },
    [items, notifyMotherSwitches],
  );

  const toggleExpandedStickyKey = useCallback((stickyKey: string) => {
    setExpandedStickyKeys((current) =>
      current.includes(stickyKey)
        ? current.filter((value) => value !== stickyKey)
        : [...current, stickyKey],
    );
  }, []);

  const toggleAllVisibleStickyKeys = useCallback(() => {
    if (!hasVisibleStickyConversations) return;
    setExpandedStickyKeys((current) => {
      const allExpanded = visibleStickyKeys.every((stickyKey) => current.includes(stickyKey));
      if (allExpanded) {
        return current.filter((stickyKey) => !visibleStickyKeys.includes(stickyKey));
      }

      const preserved = current.filter((stickyKey) => !visibleStickyKeys.includes(stickyKey));
      return [...preserved, ...visibleStickyKeys];
    });
  }, [hasVisibleStickyConversations, visibleStickyKeys]);

  const handleOpenRelatedUpstreamAccount = useCallback(
    (nextAccountId: number) => {
      openUpstreamAccount(nextAccountId);
    },
    [openUpstreamAccount],
  );

  const handleOauthLogin = useCallback(
    async (nextAccountId: number) => {
      navigate(`/account-pool/upstream-accounts/new?accountId=${nextAccountId}`);
    },
    [navigate],
  );

  const handleNotFoundClose = useCallback(
    (sourceId: number, error: unknown) => {
      if (selectedIdRef.current !== sourceId) return false;
      if (!isUpstreamAccountNotFoundError(error)) return false;
      onClose({ replace: true });
      return true;
    },
    [onClose],
  );

  const applySavedAccountDraftResponse = useCallback(
    (
      sourceId: number,
      saveDraftSessionKey: string | null,
      saveStartedDraft: AccountDraft,
      fallbackDraft: AccountDraft,
      response: UpstreamAccountDetail,
    ) => {
      const responseFallbackDraft = removeAccountDraftTagIds(
        fallbackDraft,
        knownRemovedTagIdsRef.current,
      );
      const retainedServerDraft = removeAccountDraftTagIds(
        latestServerDraftRef.current,
        knownRemovedTagIdsRef.current,
      );
      const responseDraft = removeAccountDraftTagIds(
        buildDraft(response),
        knownRemovedTagIdsRef.current,
      );
      const startedDraft = removeAccountDraftTagIds(
        saveStartedDraft,
        knownRemovedTagIdsRef.current,
      );
      if (selectedIdRef.current === sourceId && activeDraftSessionKeyRef.current != null) {
        recentSaveResponseGuardsRef.current.set(sourceId, {
          accountId: sourceId,
          sessionKey: saveDraftSessionKey,
          startedDraft,
          draft: responseDraft,
          fallbackDraft: responseFallbackDraft,
          retainedDraft: retainedServerDraft,
        });
      }
      if (
        selectedIdRef.current === sourceId &&
        saveDraftSessionKey != null &&
        activeDraftSessionKeyRef.current === saveDraftSessionKey
      ) {
        draftBaselineRef.current = responseDraft;
        latestServerDraftRef.current = responseDraft;
        setDraft((current) => mergeDraftAfterAccountSave(current, saveStartedDraft, responseDraft));
      }
      return responseDraft;
    },
    [],
  );

  const handleSave = useCallback(
    async (source: UpstreamAccountDetail) => {
      if (source.kind === "api_key_codex" && draftUpstreamBaseUrlError) return;
      if (hasBusyAccountAction(busyAction, source.id)) return;
      const saveDraftSessionKey = activeDraftSessionKey;
      const saveStartedDraft = draft;
      const pendingSaveSession = {
        accountId: source.id,
        sessionKey: saveDraftSessionKey,
        fallbackDraft: latestServerDraftRef.current,
      };
      pendingSaveSessionsRef.current.set(source.id, pendingSaveSession);
      setActionError((current) => {
        const nextMessages = { ...current.accountMessages };
        delete nextMessages[source.id];
        return { ...current, accountMessages: nextMessages };
      });
      setBusyAction((current) => {
        const nextActions = new Set(current.accountActions);
        nextActions.add(createBusyActionKey("save", source.id));
        return { ...current, accountActions: nextActions };
      });
      try {
        const normalizedGroupName = normalizeGroupName(draft.groupName);
        const pendingGroupNote = resolvePendingGroupNoteForName(normalizedGroupName);
        const normalizedEmail = draft.email.trim();
        const response = await saveAccount(source.id, {
          displayName: draft.displayName.trim() || undefined,
          email: normalizedEmail || null,
          groupName: draft.groupName.trim(),
          isMother: draft.isMother,
          note: draft.note.trim() || undefined,
          groupNote: pendingGroupNote || undefined,
          upstreamBaseUrl:
            source.kind === "api_key_codex" ? draft.upstreamBaseUrl.trim() || null : undefined,
          apiKey:
            source.kind === "api_key_codex" && draft.apiKey.trim()
              ? draft.apiKey.trim()
              : undefined,
          localPrimaryLimit:
            source.kind === "api_key_codex"
              ? normalizeNumberInput(draft.localPrimaryLimit)
              : undefined,
          localSecondaryLimit:
            source.kind === "api_key_codex"
              ? normalizeNumberInput(draft.localSecondaryLimit)
              : undefined,
          localLimitUnit:
            source.kind === "api_key_codex" ? draft.localLimitUnit.trim() || undefined : undefined,
        });
        notifyMotherChange(response);
        applySavedAccountDraftResponse(
          source.id,
          saveDraftSessionKey,
          saveStartedDraft,
          pendingSaveSession.fallbackDraft,
          response,
        );
      } catch (err) {
        if (handleNotFoundClose(source.id, err)) return;
        setActionError((current) => ({
          ...current,
          accountMessages: {
            ...current.accountMessages,
            [source.id]: err instanceof Error ? err.message : String(err),
          },
        }));
      } finally {
        if (pendingSaveSessionsRef.current.get(source.id) === pendingSaveSession) {
          pendingSaveSessionsRef.current.delete(source.id);
        }
        setBusyAction((current) => {
          const nextActions = new Set(current.accountActions);
          nextActions.delete(createBusyActionKey("save", source.id));
          return { ...current, accountActions: nextActions };
        });
      }
    },
    [
      activeDraftSessionKey,
      applySavedAccountDraftResponse,
      busyAction,
      draft,
      draftUpstreamBaseUrlError,
      handleNotFoundClose,
      notifyMotherChange,
      resolvePendingGroupNoteForName,
      saveAccount,
    ],
  );
  const handleSaveAccountProxyBindings = useCallback(
    async (source: UpstreamAccountDetail, proxyKeys: string[]) => {
      if (hasBusyAccountAction(busyAction, source.id)) return;
      const normalizedProxyKeys = normalizeProxyKeys(proxyKeys);
      setActionError((current) => {
        const nextMessages = { ...current.accountMessages };
        delete nextMessages[source.id];
        return { ...current, accountMessages: nextMessages };
      });
      setBusyAction((current) => {
        const nextActions = new Set(current.accountActions);
        nextActions.add(createBusyActionKey("save", source.id));
        return { ...current, accountActions: nextActions };
      });
      try {
        const response = await saveAccount(source.id, {
          boundProxyKeys: normalizedProxyKeys.length > 0 ? normalizedProxyKeys : null,
        });
        notifyMotherChange(response);
        return true;
      } catch (err) {
        if (handleNotFoundClose(source.id, err)) return false;
        setActionError((current) => ({
          ...current,
          accountMessages: {
            ...current.accountMessages,
            [source.id]: err instanceof Error ? err.message : String(err),
          },
        }));
        return false;
      } finally {
        setBusyAction((current) => {
          const nextActions = new Set(current.accountActions);
          nextActions.delete(createBusyActionKey("save", source.id));
          return { ...current, accountActions: nextActions };
        });
      }
    },
    [busyAction, handleNotFoundClose, notifyMotherChange, saveAccount],
  );
  const applyAccountProxyEditor = useCallback(async () => {
    if (!selectedDetail) return;
    const saved = await handleSaveAccountProxyBindings(selectedDetail, accountProxyDraftKeys);
    if (saved) {
      setAccountProxyEditorOpen(false);
    }
  }, [accountProxyDraftKeys, handleSaveAccountProxyBindings, selectedDetail]);
  const handleSaveInlineAccountPolicy = useCallback(
    async (
      source: UpstreamAccountDetail,
      field: InlinePolicyField,
      payload: UpdateGroupAccountRoutingRulePayload,
    ) => {
      if (inlinePolicyBusyField != null || hasBusyAccountAction(busyAction, source.id)) {
        return;
      }
      setInlinePolicyErrors((current) => ({ ...current, [field]: null }));
      setInlinePolicyBusyField(field);
      try {
        const response = await saveAccount(source.id, {
          routingRule: payload,
        });
        notifyMotherChange(response);
      } catch (err) {
        if (handleNotFoundClose(source.id, err)) return;
        setInlinePolicyErrors((current) => ({
          ...current,
          [field]: err instanceof Error ? err.message : String(err),
        }));
      } finally {
        setInlinePolicyBusyField((current) => (current === field ? null : current));
      }
    },
    [busyAction, handleNotFoundClose, inlinePolicyBusyField, notifyMotherChange, saveAccount],
  );

  const handleSync = useCallback(
    async (source: UpstreamAccountSummary) => {
      if (hasBusyAccountAction(busyAction, source.id)) return;
      setActionError((current) => {
        const nextMessages = { ...current.accountMessages };
        delete nextMessages[source.id];
        return { ...current, accountMessages: nextMessages };
      });
      setBusyAction((current) => {
        const nextActions = new Set(current.accountActions);
        nextActions.add(createBusyActionKey("sync", source.id));
        return { ...current, accountActions: nextActions };
      });
      try {
        await runSync(source.id);
      } catch (err) {
        if (handleNotFoundClose(source.id, err)) return;
        setActionError((current) => ({
          ...current,
          accountMessages: {
            ...current.accountMessages,
            [source.id]: err instanceof Error ? err.message : String(err),
          },
        }));
      } finally {
        setBusyAction((current) => {
          const nextActions = new Set(current.accountActions);
          nextActions.delete(createBusyActionKey("sync", source.id));
          return { ...current, accountActions: nextActions };
        });
      }
    },
    [busyAction, handleNotFoundClose, runSync],
  );

  const handleToggleEnabled = useCallback(
    async (source: UpstreamAccountSummary, enabled: boolean) => {
      if (hasBusyAccountAction(busyAction, source.id)) return;
      setActionError((current) => {
        const nextMessages = { ...current.accountMessages };
        delete nextMessages[source.id];
        return { ...current, accountMessages: nextMessages };
      });
      setBusyAction((current) => {
        const nextActions = new Set(current.accountActions);
        nextActions.add(createBusyActionKey("toggle", source.id));
        return { ...current, accountActions: nextActions };
      });
      try {
        await saveAccount(source.id, { enabled });
      } catch (err) {
        if (handleNotFoundClose(source.id, err)) return;
        setActionError((current) => ({
          ...current,
          accountMessages: {
            ...current.accountMessages,
            [source.id]: err instanceof Error ? err.message : String(err),
          },
        }));
      } finally {
        setBusyAction((current) => {
          const nextActions = new Set(current.accountActions);
          nextActions.delete(createBusyActionKey("toggle", source.id));
          return { ...current, accountActions: nextActions };
        });
      }
    },
    [busyAction, handleNotFoundClose, saveAccount],
  );

  const handleDelete = useCallback(
    async (source: UpstreamAccountSummary) => {
      if (hasBusyAccountAction(busyAction, source.id)) return;
      setIsDeleteConfirmOpen(false);
      setActionError((current) => {
        const nextMessages = { ...current.accountMessages };
        delete nextMessages[source.id];
        return { ...current, accountMessages: nextMessages };
      });
      setBusyAction((current) => {
        const nextActions = new Set(current.accountActions);
        nextActions.add(createBusyActionKey("delete", source.id));
        return { ...current, accountActions: nextActions };
      });
      try {
        await removeAccount(source.id);
        if (drawerOpenRef.current && routeAccountIdRef.current === source.id) {
          onClose({ replace: true });
        }
      } catch (err) {
        if (handleNotFoundClose(source.id, err)) return;
        setActionError((current) => ({
          ...current,
          accountMessages: {
            ...current.accountMessages,
            [source.id]: err instanceof Error ? err.message : String(err),
          },
        }));
      } finally {
        setBusyAction((current) => {
          const nextActions = new Set(current.accountActions);
          nextActions.delete(createBusyActionKey("delete", source.id));
          return { ...current, accountActions: nextActions };
        });
      }
    },
    [busyAction, handleNotFoundClose, onClose, removeAccount],
  );
  const handleDeleteConfirmOpenChange = useCallback(
    (nextOpen: boolean) => {
      if (!nextOpen && selectedId != null && isBusyAction(busyAction, "delete", selectedId)) return;
      setIsDeleteConfirmOpen(nextOpen);
    },
    [busyAction, selectedId],
  );

  const detailIdentity =
    selected ?? (accountId != null ? { id: accountId, displayName: `#${accountId}` } : null);

  return (
    <>
      {open && accountId != null ? (
        <AccountDetailDrawerShell
          open={open}
          presentation={presentation}
          labelledBy={detailDrawerTitleId}
          closeLabel={t("accountPool.upstreamAccounts.actions.closeDetails")}
          closeDisabled={isBusyAction(busyAction, "delete", accountId)}
          autoFocusCloseButton={!isDeleteConfirmOpen}
          onPortalContainerChange={setDetailDrawerPortalContainer}
          onBodyElementChange={setDetailDrawerBodyElement}
          onClose={handleDetailDrawerClose}
          shellClassName="drawer-shell--detail-wide"
          header={
            <div className="space-y-4">
              <div className="space-y-3">
                {selected ? (
                  <div className="flex flex-wrap items-center gap-2">
                    <Badge variant={enableStatusVariant(accountEnableStatus(selected))}>
                      {accountEnableStatusLabel(accountEnableStatus(selected))}
                    </Badge>
                    <Badge variant={workStatusVariant(accountWorkStatus(selected))}>
                      {accountWorkStatusLabel(accountWorkStatus(selected))}
                    </Badge>
                    <Badge variant={syncStateVariant(accountSyncState(selected))}>
                      {accountSyncStateLabel(accountSyncState(selected))}
                    </Badge>
                    <Badge variant={healthStatusVariant(accountHealthStatus(selected))}>
                      {accountHealthStatusLabel(accountHealthStatus(selected))}
                    </Badge>
                    <Badge variant={kindVariant(selected.kind)}>
                      {accountKindLabel(selected.kind)}
                    </Badge>
                    {selected.planType && selectedPlanBadge ? (
                      <Badge
                        variant={selectedPlanBadge.variant}
                        className={selectedPlanBadge.className}
                        data-plan={selectedPlanBadge.dataPlan}
                      >
                        {selected.planType}
                      </Badge>
                    ) : null}
                    {selected.duplicateInfo ? (
                      <Badge variant="warning">
                        {t("accountPool.upstreamAccounts.duplicate.badge")}
                      </Badge>
                    ) : null}
                    {selected.kind === "api_key_codex" ? (
                      <Badge variant="secondary">
                        {t("accountPool.upstreamAccounts.apiKey.localPlaceholder")}
                      </Badge>
                    ) : null}
                  </div>
                ) : null}
                <div className="section-heading">
                  <p className="text-xs font-semibold uppercase tracking-[0.2em] text-primary/75">
                    {t("accountPool.upstreamAccounts.detailTitle")}
                  </p>
                  <div className="flex flex-wrap items-center gap-2">
                    <h2 id={detailDrawerTitleId} className="section-title">
                      {detailIdentity?.displayName ?? `#${accountId}`}
                    </h2>
                    {selected?.isMother ? (
                      <MotherAccountBadge label={t("accountPool.upstreamAccounts.mother.badge")} />
                    ) : null}
                  </div>
                  <p className="section-description">
                    {selected?.email ??
                      selected?.maskedApiKey ??
                      t("accountPool.upstreamAccounts.identityUnavailable")}
                  </p>
                </div>
              </div>
              {selected ? (
                <div className="flex flex-wrap items-center gap-2">
                  <div className="flex items-center gap-2 rounded-full border border-base-300/80 bg-base-100/70 px-3 py-2 text-sm">
                    <span className="text-base-content/60">
                      {t("accountPool.upstreamAccounts.actions.enable")}
                    </span>
                    <Switch
                      checked={selected.enabled}
                      onCheckedChange={(checked) => void handleToggleEnabled(selected, checked)}
                      disabled={hasBusyAccountAction(busyAction, selected.id) || !writesEnabled}
                      aria-label={t("accountPool.upstreamAccounts.actions.enable")}
                    />
                  </div>
                  <Button
                    type="button"
                    variant="secondary"
                    onClick={() => void handleSync(selected)}
                    disabled={hasBusyAccountAction(busyAction, selected.id)}
                    data-testid="account-sync-button"
                  >
                    {isBusyAction(busyAction, "sync", selected.id) ? (
                      <Spinner size="sm" className="mr-2" />
                    ) : (
                      <AppIcon
                        name="timer-refresh-outline"
                        className="mr-2 h-4 w-4"
                        aria-hidden
                        data-icon-name="timer-refresh-outline"
                      />
                    )}
                    {t("accountPool.upstreamAccounts.actions.syncNow")}
                  </Button>
                  {selected.kind === "oauth_codex" ? (
                    <Button
                      type="button"
                      variant="outline"
                      onClick={() => void handleOauthLogin(selected.id)}
                      disabled={hasBusyAccountAction(busyAction, selected.id) || !writesEnabled}
                    >
                      {isBusyAction(busyAction, "relogin", selected.id) ? (
                        <Spinner size="sm" className="mr-2" />
                      ) : (
                        <AppIcon name="login-variant" className="mr-2 h-4 w-4" aria-hidden />
                      )}
                      {t("accountPool.upstreamAccounts.actions.relogin")}
                    </Button>
                  ) : null}
                  {isCompactViewport ? (
                    <>
                      <Button
                        type="button"
                        variant="destructive"
                        disabled={hasBusyAccountAction(busyAction, selected.id) || !writesEnabled}
                        aria-haspopup="dialog"
                        aria-expanded={isDeleteConfirmOpen}
                        aria-controls={isDeleteConfirmOpen ? deleteConfirmTitleId : undefined}
                        onClick={() => handleDeleteConfirmOpenChange(true)}
                      >
                        {isBusyAction(busyAction, "delete", selected.id) ? (
                          <Spinner size="sm" className="mr-2" />
                        ) : (
                          <AppIcon name="trash-can-outline" className="mr-2 h-4 w-4" aria-hidden />
                        )}
                        {t("accountPool.upstreamAccounts.actions.delete")}
                      </Button>
                      <Dialog
                        open={isDeleteConfirmOpen}
                        onOpenChange={handleDeleteConfirmOpenChange}
                      >
                        <DialogContent
                          container={detailDrawerPortalContainer}
                          role="alertdialog"
                          aria-labelledby={deleteConfirmTitleId}
                          className="flex max-h-[calc(100dvh-0.75rem)] flex-col overflow-hidden p-0 desktop:max-h-[calc(100dvh-2rem)]"
                          onOpenAutoFocus={(event) => {
                            event.preventDefault();
                            deleteConfirmCancelRef.current?.focus();
                          }}
                        >
                          <div className="shrink-0 border-b border-base-300/80 px-5 py-4 desktop:px-6">
                            <DialogHeader className="min-w-0">
                              <div className="flex items-start gap-3">
                                <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-error text-error-content shadow-sm">
                                  <AppIcon
                                    name="trash-can-outline"
                                    className="h-4 w-4"
                                    aria-hidden
                                  />
                                </div>
                                <div className="min-w-0 flex-1 space-y-1">
                                  <DialogTitle id={deleteConfirmTitleId} className="text-lg">
                                    {t("accountPool.upstreamAccounts.deleteConfirmTitle", {
                                      name: selected.displayName,
                                    })}
                                  </DialogTitle>
                                  <DialogDescription>
                                    {t("accountPool.upstreamAccounts.deleteConfirm", {
                                      name: selected.displayName,
                                    })}
                                  </DialogDescription>
                                </div>
                              </div>
                            </DialogHeader>
                          </div>
                          <DialogFooter className="shrink-0 border-t border-base-300/80 bg-base-100/94 px-5 pb-[max(env(safe-area-inset-bottom),1rem)] pt-4 backdrop-blur desktop:px-6 desktop:py-4">
                            <Button
                              ref={deleteConfirmCancelRef}
                              type="button"
                              variant="ghost"
                              onClick={() => setIsDeleteConfirmOpen(false)}
                            >
                              {t("accountPool.upstreamAccounts.actions.cancel")}
                            </Button>
                            <Button
                              type="button"
                              variant="destructive"
                              disabled={
                                hasBusyAccountAction(busyAction, selected.id) || !writesEnabled
                              }
                              onClick={() => void handleDelete(selected)}
                            >
                              {t("accountPool.upstreamAccounts.actions.confirmDelete")}
                            </Button>
                          </DialogFooter>
                        </DialogContent>
                      </Dialog>
                    </>
                  ) : (
                    <Popover
                      open={isDeleteConfirmOpen}
                      onOpenChange={handleDeleteConfirmOpenChange}
                    >
                      <PopoverTrigger asChild>
                        <Button
                          type="button"
                          variant="destructive"
                          disabled={hasBusyAccountAction(busyAction, selected.id) || !writesEnabled}
                          aria-haspopup="dialog"
                          aria-expanded={isDeleteConfirmOpen}
                          aria-controls={isDeleteConfirmOpen ? deleteConfirmTitleId : undefined}
                        >
                          {isBusyAction(busyAction, "delete", selected.id) ? (
                            <Spinner size="sm" className="mr-2" />
                          ) : (
                            <AppIcon
                              name="trash-can-outline"
                              className="mr-2 h-4 w-4"
                              aria-hidden
                            />
                          )}
                          {t("accountPool.upstreamAccounts.actions.delete")}
                        </Button>
                      </PopoverTrigger>
                      {detailDrawerPortalContainer ? (
                        <PopoverContent
                          container={detailDrawerPortalContainer}
                          role="alertdialog"
                          aria-modal="false"
                          aria-labelledby={deleteConfirmTitleId}
                          align="end"
                          side="top"
                          sideOffset={12}
                          className="z-[80] w-[min(22rem,calc(100vw-1.5rem))] rounded-2xl border border-base-300 bg-base-100 p-4 shadow-[0_20px_48px_rgba(15,23,42,0.24)] ring-1 ring-base-100/90"
                          onOpenAutoFocus={(event) => {
                            event.preventDefault();
                            deleteConfirmCancelRef.current?.focus();
                          }}
                          onEscapeKeyDown={(event) => {
                            event.stopPropagation();
                          }}
                        >
                          <div className="space-y-3">
                            <div className="flex items-start gap-2.5">
                              <div className="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-error text-error-content shadow-sm">
                                <AppIcon
                                  name="trash-can-outline"
                                  className="h-3.5 w-3.5"
                                  aria-hidden
                                />
                              </div>
                              <p
                                id={deleteConfirmTitleId}
                                className="min-w-0 break-words pr-2 text-[15px] font-semibold leading-6 text-base-content"
                              >
                                {t("accountPool.upstreamAccounts.deleteConfirmTitle", {
                                  name: selected.displayName,
                                })}
                              </p>
                            </div>
                            <div className="flex justify-end gap-2">
                              <Button
                                ref={deleteConfirmCancelRef}
                                type="button"
                                variant="secondary"
                                size="sm"
                                className="rounded-full px-3.5 font-semibold"
                                onClick={() => setIsDeleteConfirmOpen(false)}
                              >
                                {t("accountPool.upstreamAccounts.actions.cancel")}
                              </Button>
                              <Button
                                type="button"
                                variant="destructive"
                                size="sm"
                                className="rounded-full px-3.5 font-semibold shadow-sm"
                                disabled={
                                  hasBusyAccountAction(busyAction, selected.id) || !writesEnabled
                                }
                                onClick={() => void handleDelete(selected)}
                              >
                                {t("accountPool.upstreamAccounts.actions.confirmDelete")}
                              </Button>
                            </div>
                          </div>
                          <PopoverArrow
                            className="fill-base-100 stroke-base-300 stroke-[1px]"
                            width={18}
                            height={10}
                          />
                        </PopoverContent>
                      ) : null}
                    </Popover>
                  )}
                </div>
              ) : null}
            </div>
          }
        >
          {detailError && selectedDetail ? (
            <Alert variant="error">
              <AppIcon
                name="alert-circle-outline"
                className="mt-0.5 h-4 w-4 shrink-0"
                aria-hidden
              />
              <div>{detailError}</div>
            </Alert>
          ) : null}
          {visibleAccountActionError && !selectedDetail ? (
            <Alert variant="error">
              <AppIcon
                name="alert-circle-outline"
                className="mt-0.5 h-4 w-4 shrink-0"
                aria-hidden
              />
              <div>{visibleAccountActionError}</div>
            </Alert>
          ) : null}
          {isDetailLoading && !selectedDetail ? (
            <AccountDetailSkeleton />
          ) : detailError && !selectedDetail ? (
            <Alert variant="error">
              <AppIcon
                name="alert-circle-outline"
                className="mt-0.5 h-4 w-4 shrink-0"
                aria-hidden
              />
              <div>{detailError}</div>
            </Alert>
          ) : selectedDetail ? (
            <div className="grid min-w-0 gap-5">
              {visibleAccountActionError ? (
                <Alert variant="error">
                  <AppIcon
                    name="alert-circle-outline"
                    className="mt-0.5 h-4 w-4 shrink-0"
                    aria-hidden
                  />
                  <div>{visibleAccountActionError}</div>
                </Alert>
              ) : null}
              <SegmentedControl
                className="w-full flex-wrap justify-start self-stretch desktop:w-auto desktop:self-start"
                role="tablist"
                aria-label={t("accountPool.upstreamAccounts.detailTitle")}
              >
                <SegmentedControlItem
                  id={detailTabIds.overview.tab}
                  active={detailTab === "overview"}
                  role="tab"
                  aria-selected={detailTab === "overview"}
                  aria-controls={detailTabIds.overview.panel}
                  onClick={() => handleSelectDetailTab("overview")}
                >
                  {t("accountPool.upstreamAccounts.detailTabs.overview")}
                </SegmentedControlItem>
                <SegmentedControlItem
                  id={detailTabIds.records.tab}
                  active={detailTab === "records"}
                  role="tab"
                  aria-selected={detailTab === "records"}
                  aria-controls={detailTabIds.records.panel}
                  onClick={() => {
                    setFocusedAttemptId(null);
                    handleSelectDetailTab("records");
                  }}
                >
                  {t("accountPool.upstreamAccounts.detailTabs.records")}
                </SegmentedControlItem>
                <SegmentedControlItem
                  id={detailTabIds.edit.tab}
                  active={detailTab === "edit"}
                  role="tab"
                  aria-selected={detailTab === "edit"}
                  aria-controls={detailTabIds.edit.panel}
                  onClick={() => handleSelectDetailTab("edit")}
                >
                  {t("accountPool.upstreamAccounts.detailTabs.edit")}
                </SegmentedControlItem>
                <SegmentedControlItem
                  id={detailTabIds.routing.tab}
                  active={detailTab === "routing"}
                  role="tab"
                  aria-selected={detailTab === "routing"}
                  aria-controls={detailTabIds.routing.panel}
                  onClick={() => handleSelectDetailTab("routing")}
                >
                  {t("accountPool.upstreamAccounts.detailTabs.routing")}
                </SegmentedControlItem>
                <SegmentedControlItem
                  id={detailTabIds.healthEvents.tab}
                  active={detailTab === "healthEvents"}
                  role="tab"
                  aria-selected={detailTab === "healthEvents"}
                  aria-controls={detailTabIds.healthEvents.panel}
                  onClick={() => handleSelectDetailTab("healthEvents")}
                >
                  {t("accountPool.upstreamAccounts.detailTabs.healthEvents")}
                </SegmentedControlItem>
              </SegmentedControl>

              {detailTab === "overview" ? (
                <div
                  id={detailTabIds.overview.panel}
                  role="tabpanel"
                  aria-labelledby={detailTabIds.overview.tab}
                  className="grid gap-5"
                >
                  {selectedDetail.routingBlockReasonMessage ? (
                    <Alert variant="warning">
                      <AppIcon
                        name="alert-outline"
                        className="mt-0.5 h-4 w-4 shrink-0"
                        aria-hidden
                      />
                      <div>
                        <p className="font-medium">
                          {t("accountPool.upstreamAccounts.routingBlock.title")}
                        </p>
                        <p className="mt-1 text-sm text-warning/90">
                          {selectedDetail.routingBlockReasonMessage}
                        </p>
                      </div>
                    </Alert>
                  ) : null}
                  {selectedDetail.duplicateInfo ? (
                    <Alert variant="warning">
                      <AppIcon
                        name="alert-outline"
                        className="mt-0.5 h-4 w-4 shrink-0"
                        aria-hidden
                      />
                      <div>
                        <p className="font-medium">
                          {t("accountPool.upstreamAccounts.duplicate.badge")}
                        </p>
                        <p className="mt-1 text-sm text-warning/90">
                          {t("accountPool.upstreamAccounts.duplicate.warningBody", {
                            reasons: formatDuplicateReasons(selectedDetail.duplicateInfo),
                            peers: selectedDetail.duplicateInfo.peerAccountIds.join(", "),
                          })}
                        </p>
                      </div>
                    </Alert>
                  ) : null}
                  <div className="rounded-xl border border-base-300/70 bg-base-100/55 px-4 py-2">
                    <div className="grid gap-x-5 gap-y-1 [grid-template-columns:repeat(auto-fit,minmax(8.5rem,1fr))]">
                      <CompactDetailField
                        label={t("accountPool.upstreamAccounts.fields.groupName")}
                        value={selectedDetail.groupName ?? ""}
                        title={selectedDetail.groupName ?? undefined}
                      />
                      {selectedDetail.kind === "oauth_codex" ? (
                        <CompactDetailField
                          label={t("accountPool.upstreamAccounts.fields.imageToolCapability")}
                          value={
                            <Badge
                              variant={
                                (selectedDetail.imageToolCapability ?? "unknown") === "supported"
                                  ? "success"
                                  : (selectedDetail.imageToolCapability ?? "unknown") ===
                                      "unsupported"
                                    ? "warning"
                                    : "secondary"
                              }
                              className="max-w-full truncate"
                            >
                              {t(
                                `accountPool.upstreamAccounts.imageToolCapability.${selectedDetail.imageToolCapability ?? "unknown"}`,
                              )}
                            </Badge>
                          }
                          title={t(
                            `accountPool.upstreamAccounts.imageToolCapabilityHint.${selectedDetail.imageToolCapability ?? "unknown"}`,
                          )}
                        />
                      ) : null}
                      <CompactDetailField
                        label={t("accountPool.upstreamAccounts.mother.fieldLabel")}
                        value={
                          selectedDetail.isMother
                            ? t("accountPool.upstreamAccounts.mother.badge")
                            : t("accountPool.upstreamAccounts.mother.notMother")
                        }
                      />
                      <CompactDetailField
                        label={t("accountPool.upstreamAccounts.fields.email")}
                        value={selectedDetail.email ?? ""}
                        title={selectedDetail.email ?? undefined}
                      />
                      {selectedDetail.kind === "oauth_codex" ? (
                        <CompactDetailField
                          label={t("accountPool.upstreamAccounts.fields.verifiedEmail")}
                          value={selectedDetail.verifiedEmail ?? ""}
                          title={selectedDetail.verifiedEmail ?? undefined}
                        />
                      ) : null}
                      {selectedDetail.kind === "oauth_codex" ? (
                        <>
                          <CompactDetailField
                            label={t("accountPool.upstreamAccounts.fields.accountId")}
                            value={selectedDetail.chatgptAccountId ?? ""}
                            title={selectedDetail.chatgptAccountId ?? undefined}
                          />
                          <CompactDetailField
                            label={t("accountPool.upstreamAccounts.fields.userId")}
                            value={selectedDetail.chatgptUserId ?? ""}
                            title={selectedDetail.chatgptUserId ?? undefined}
                          />
                        </>
                      ) : null}
                      <CompactDetailField
                        label={t("accountPool.upstreamAccounts.fields.lastSuccessSync")}
                        value={formatDateTime(selectedDetail.lastSuccessfulSyncAt)}
                        title={formatDateTime(selectedDetail.lastSuccessfulSyncAt)}
                      />
                    </div>
                  </div>
                  {selectedDetail.kind === "oauth_codex" ? (
                    <div className="grid gap-4 lg:grid-cols-2">
                      <UpstreamAccountUsageCard
                        title={t("accountPool.upstreamAccounts.primaryWindowLabel")}
                        description={t("accountPool.upstreamAccounts.usage.primaryDescription")}
                        window={selectedDetail.primaryWindow}
                        history={selectedDetail.history}
                        historyKey="primaryUsedPercent"
                        emptyLabel={t("accountPool.upstreamAccounts.noHistory")}
                      />
                      <UpstreamAccountUsageCard
                        title={t("accountPool.upstreamAccounts.secondaryWindowLabel")}
                        description={t("accountPool.upstreamAccounts.usage.secondaryDescription")}
                        window={selectedDetail.secondaryWindow}
                        history={selectedDetail.history}
                        historyKey="secondaryUsedPercent"
                        emptyLabel={t("accountPool.upstreamAccounts.noHistory")}
                        accentClassName="text-secondary"
                      />
                    </div>
                  ) : null}
                  <DashboardActivityOverview
                    key={`account-activity-${accountId}`}
                    title={t("accountPool.upstreamAccounts.records.activityOverviewTitle")}
                    storageKey={`${ACCOUNT_ACTIVITY_RANGE_STORAGE_KEY_PREFIX}.${accountId}`}
                    testId="upstream-account-records-activity-overview"
                    upstreamAccountId={accountId}
                  />
                </div>
              ) : null}

              {detailTab === "records" ? (
                <div
                  id={detailTabIds.records.panel}
                  role="tabpanel"
                  aria-labelledby={detailTabIds.records.tab}
                  className="flex min-w-0 flex-col gap-3"
                >
                  {accountId != null ? (
                    <UpstreamAccountAttemptTimeline
                      accountId={accountId}
                      focusedAttemptId={focusedAttemptId}
                    />
                  ) : null}
                  <div className="hidden" aria-hidden="true">
                    {accountRecordsMode === "anchored" ? (
                      <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-info/35 bg-info/8 px-3 py-2 text-sm">
                        <span className="text-base-content/72">
                          {accountRecordsLocateError
                            ? t("accountPool.upstreamAccounts.records.locateUnavailable")
                            : accountRecordsScrollTarget
                              ? t("accountPool.upstreamAccounts.records.located", {
                                  invokeId: accountRecordsScrollTarget.invokeId,
                                })
                              : t("accountPool.upstreamAccounts.records.locating")}
                        </span>
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          onClick={reloadAccountRecords}
                        >
                          <AppIcon name="refresh" className="mr-2 h-4 w-4" aria-hidden />
                          {t("accountPool.upstreamAccounts.records.returnLatest")}
                        </Button>
                      </div>
                    ) : null}
                    {accountRecordsLocateError ? (
                      <Alert
                        ref={accountRecordsLocateAlertRef}
                        role="alert"
                        tabIndex={-1}
                        variant="warning"
                        className="justify-between gap-3 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-warning"
                      >
                        <span className="min-w-0 break-words">
                          {t(
                            accountRecordsLocateError.kind === "notFound"
                              ? "accountPool.upstreamAccounts.records.locateNotFound"
                              : "accountPool.upstreamAccounts.records.locateFailed",
                            { invokeId: accountRecordsLocateError.invokeId },
                          )}
                        </span>
                        {accountRecordsLocateError.kind === "request" ? (
                          <Button
                            type="button"
                            variant="outline"
                            size="sm"
                            onClick={() => locateAccountRecord(accountRecordsLocateError.invokeId)}
                          >
                            {t("accountPool.upstreamAccounts.records.retryLocate")}
                          </Button>
                        ) : null}
                      </Alert>
                    ) : null}
                    <InvocationTable
                      records={accountRecords}
                      isLoading={accountRecordsLoading && accountRecords.length === 0}
                      error={accountRecordsError}
                      emptyLabel={t("accountPool.upstreamAccounts.records.empty")}
                      onOpenUpstreamAccount={handleOpenRelatedUpstreamAccount}
                      scrollElement={detailDrawerBodyElement}
                      showInvokeId
                      scrollTarget={accountRecordsScrollTarget}
                    />
                    {accountRecords.length > 0 ? (
                      <div
                        className="flex justify-center py-2 text-xs text-base-content/62"
                        data-testid="upstream-account-records-infinite-status"
                      >
                        {accountRecordsLoading ? (
                          <span className="inline-flex items-center gap-2">
                            <Spinner size="sm" />
                            {t("accountPool.upstreamAccounts.records.loadingMore")}
                          </span>
                        ) : accountRecordsMode === "anchored" ? (
                          <span>
                            {t("accountPool.upstreamAccounts.records.anchoredLoaded", {
                              loaded: accountRecords.length,
                              total: accountRecordsTotal,
                            })}
                          </span>
                        ) : accountRecordsHasMore ? (
                          <span>
                            {t("accountPool.upstreamAccounts.records.loaded", {
                              loaded: accountRecords.length,
                              total: accountRecordsTotal,
                            })}
                          </span>
                        ) : (
                          <span>
                            {t("accountPool.upstreamAccounts.records.allLoaded", {
                              count: accountRecords.length,
                            })}
                          </span>
                        )}
                      </div>
                    ) : null}
                  </div>
                </div>
              ) : null}

              {detailTab === "edit" ? (
                <div
                  id={detailTabIds.edit.panel}
                  role="tabpanel"
                  aria-labelledby={detailTabIds.edit.tab}
                >
                  <Card className="border-base-300/80 bg-base-100/72">
                    <CardHeader>
                      <CardTitle>{t("accountPool.upstreamAccounts.editTitle")}</CardTitle>
                      <CardDescription>
                        {t("accountPool.upstreamAccounts.editDescription")}
                      </CardDescription>
                    </CardHeader>
                    <CardContent className="grid gap-4 md:grid-cols-2">
                      <label className="field md:col-span-2">
                        <span className="field-label">
                          {t("accountPool.upstreamAccounts.fields.displayName")}
                        </span>
                        <div className="relative">
                          <Input
                            name="detailDisplayName"
                            value={draft.displayName}
                            aria-invalid={detailDisplayNameConflict != null}
                            onChange={(event) =>
                              setDraft((current) => ({
                                ...current,
                                displayName: event.target.value,
                              }))
                            }
                          />
                          {detailDisplayNameConflict ? (
                            <FloatingFieldError
                              message={t(
                                "accountPool.upstreamAccounts.validation.displayNameDuplicate",
                              )}
                            />
                          ) : null}
                        </div>
                      </label>
                      <label className="field md:col-span-2">
                        <span className="field-label">
                          {t("accountPool.upstreamAccounts.fields.email")}
                        </span>
                        <Input
                          name="detailEmail"
                          value={draft.email}
                          onChange={(event) =>
                            setDraft((current) => ({
                              ...current,
                              email: event.target.value,
                              displayName: resolveDisplayNameAfterEmailChange(
                                current.displayName,
                                current.email,
                                event.target.value,
                              ),
                            }))
                          }
                        />
                        {selectedDetail.kind === "oauth_codex" && selectedDetail.verifiedEmail ? (
                          <p className="mt-2 text-xs leading-5 text-base-content/70">
                            {t("accountPool.upstreamAccounts.edit.verifiedEmailHint", {
                              verifiedEmail: selectedDetail.verifiedEmail,
                            })}
                          </p>
                        ) : null}
                      </label>
                      <label className="field md:col-span-2">
                        <span className="field-label">
                          {t("accountPool.upstreamAccounts.fields.groupName")}
                        </span>
                        <div className="flex items-center gap-2">
                          <UpstreamAccountGroupCombobox
                            name="detailGroupName"
                            value={draft.groupName}
                            options={availableGroups.options}
                            placeholder={t(
                              "accountPool.upstreamAccounts.fields.groupNamePlaceholder",
                            )}
                            searchPlaceholder={t(
                              "accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder",
                            )}
                            emptyLabel={t("accountPool.upstreamAccounts.fields.groupNameEmpty")}
                            createLabel={(value) =>
                              t("accountPool.upstreamAccounts.fields.groupNameConfigureValue", {
                                value,
                              })
                            }
                            onCreateRequested={handleDetailGroupCreateRequest}
                            formatAccountCountLabel={formatGroupAccountCountLabel}
                            onValueChange={(value) =>
                              setDraft((current) => ({
                                ...current,
                                groupName: value,
                              }))
                            }
                            className="min-w-0 flex-1"
                          />
                          <Button
                            type="button"
                            size="icon"
                            variant={hasGroupSettings(draft.groupName) ? "secondary" : "outline"}
                            className="shrink-0 rounded-full"
                            aria-label={t("accountPool.upstreamAccounts.groupNotes.actions.edit")}
                            title={t("accountPool.upstreamAccounts.groupNotes.actions.edit")}
                            onClick={() =>
                              openGroupNoteEditor(draft.groupName, {
                                onDeleted: (deletedGroupName) =>
                                  setDraft((current) =>
                                    normalizeGroupName(current.groupName) === deletedGroupName
                                      ? {
                                          ...current,
                                          groupName: "",
                                        }
                                      : current,
                                  ),
                              })
                            }
                            disabled={!writesEnabled || !normalizeGroupName(draft.groupName)}
                          >
                            <AppIcon
                              name="file-document-edit-outline"
                              className="h-4 w-4"
                              aria-hidden
                            />
                          </Button>
                        </div>
                      </label>
                      <div className="md:col-span-2">
                        <MotherAccountToggle
                          checked={draft.isMother}
                          disabled={!writesEnabled}
                          label={t("accountPool.upstreamAccounts.mother.toggleLabel")}
                          description={t("accountPool.upstreamAccounts.mother.toggleDescription")}
                          onToggle={() =>
                            setDraft((current) => ({
                              ...current,
                              isMother: !current.isMother,
                            }))
                          }
                        />
                      </div>
                      <label className="field md:col-span-2">
                        <span className="field-label">
                          {t("accountPool.upstreamAccounts.fields.note")}
                        </span>
                        <textarea
                          className="min-h-24 rounded-xl border border-base-300 bg-base-100 px-3 py-2 text-sm text-base-content shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100"
                          name="detailNote"
                          value={draft.note}
                          onChange={(event) =>
                            setDraft((current) => ({
                              ...current,
                              note: event.target.value,
                            }))
                          }
                        />
                      </label>
                      <div className="field gap-3 md:col-span-2">
                        <span className="field-label">{t("accountPool.tags.field.label")}</span>
                        <div className="flex min-h-12 flex-wrap items-center gap-2 rounded-[1.2rem] border border-base-300/80 bg-base-100/55 px-3 py-2 shadow-sm">
                          {(selectedDetail.tags ?? []).length > 0 ? (
                            selectedDetail.tags.map((tag) => (
                              <Badge
                                key={tag.id}
                                variant="secondary"
                                className="min-w-0 max-w-[10rem] truncate border-base-300/90 bg-base-200/90 px-2 py-px text-[11px] font-medium leading-4 text-base-content/92"
                                title={tag.name}
                              >
                                {tag.name}
                              </Badge>
                            ))
                          ) : (
                            <span className="text-sm text-base-content/60">
                              {t("accountPool.tags.field.empty")}
                            </span>
                          )}
                        </div>
                      </div>
                      {selectedDetail.kind === "api_key_codex" ? (
                        <>
                          <label className="field">
                            <span className="field-label">
                              {t("accountPool.upstreamAccounts.fields.primaryLimit")}
                            </span>
                            <Input
                              name="detailPrimaryLimit"
                              value={draft.localPrimaryLimit}
                              onChange={(event) =>
                                setDraft((current) => ({
                                  ...current,
                                  localPrimaryLimit: event.target.value,
                                }))
                              }
                            />
                          </label>
                          <label className="field">
                            <span className="field-label">
                              {t("accountPool.upstreamAccounts.fields.secondaryLimit")}
                            </span>
                            <Input
                              name="detailSecondaryLimit"
                              value={draft.localSecondaryLimit}
                              onChange={(event) =>
                                setDraft((current) => ({
                                  ...current,
                                  localSecondaryLimit: event.target.value,
                                }))
                              }
                            />
                          </label>
                          <label className="field">
                            <span className="field-label">
                              {t("accountPool.upstreamAccounts.fields.limitUnit")}
                            </span>
                            <Input
                              name="detailLimitUnit"
                              value={draft.localLimitUnit}
                              onChange={(event) =>
                                setDraft((current) => ({
                                  ...current,
                                  localLimitUnit: event.target.value,
                                }))
                              }
                            />
                          </label>
                          <label className="field">
                            <FormFieldFeedback
                              label={t("accountPool.upstreamAccounts.fields.upstreamBaseUrl")}
                              message={draftUpstreamBaseUrlError}
                              messageClassName="md:max-w-[min(20rem,calc(100%-8rem))]"
                            />
                            <div className="relative">
                              <Input
                                name="detailUpstreamBaseUrl"
                                value={draft.upstreamBaseUrl}
                                onChange={(event) =>
                                  setDraft((current) => ({
                                    ...current,
                                    upstreamBaseUrl: event.target.value,
                                  }))
                                }
                                placeholder={t(
                                  "accountPool.upstreamAccounts.fields.upstreamBaseUrlPlaceholder",
                                )}
                                aria-invalid={draftUpstreamBaseUrlError ? "true" : "false"}
                                className={cn(
                                  draftUpstreamBaseUrlError
                                    ? "border-error/70 focus-visible:ring-error"
                                    : "",
                                )}
                              />
                            </div>
                          </label>
                          <label className="field">
                            <span className="field-label">
                              {t("accountPool.upstreamAccounts.fields.rotateApiKey")}
                            </span>
                            <Input
                              name="detailRotateApiKey"
                              value={draft.apiKey}
                              onChange={(event) =>
                                setDraft((current) => ({
                                  ...current,
                                  apiKey: event.target.value,
                                }))
                              }
                              placeholder={t(
                                "accountPool.upstreamAccounts.fields.rotateApiKeyPlaceholder",
                              )}
                            />
                          </label>
                        </>
                      ) : null}
                      <div className="md:col-span-2 flex justify-end">
                        <Button
                          type="button"
                          onClick={() => void handleSave(selectedDetail)}
                          disabled={
                            hasBusyAccountAction(busyAction, selectedDetail.id) ||
                            !writesEnabled ||
                            detailDisplayNameConflict != null ||
                            (selectedDetail.kind === "api_key_codex" &&
                              Boolean(draftUpstreamBaseUrlError))
                          }
                        >
                          {isBusyAction(busyAction, "save", selectedDetail.id) ? (
                            <Spinner size="sm" className="mr-2" />
                          ) : (
                            <AppIcon
                              name="content-save-outline"
                              className="mr-2 h-4 w-4"
                              aria-hidden
                            />
                          )}
                          {t("accountPool.upstreamAccounts.actions.save")}
                        </Button>
                      </div>
                    </CardContent>
                  </Card>
                </div>
              ) : null}

              {detailTab === "routing" ? (
                <div
                  id={detailTabIds.routing.panel}
                  role="tabpanel"
                  aria-labelledby={detailTabIds.routing.tab}
                  className="grid gap-5"
                >
                  <Dialog
                    open={accountProxyEditorOpen}
                    onOpenChange={(nextOpen) => {
                      if (nextOpen) {
                        openAccountProxyEditor();
                      } else {
                        closeAccountProxyEditor();
                      }
                    }}
                  >
                    <DialogContent
                      container={detailDrawerPortalContainer}
                      className="!bottom-0 !top-auto flex max-h-[calc(100dvh-0.75rem)] w-full !translate-y-0 flex-col overflow-hidden rounded-b-none border-base-300 bg-base-100 p-0 desktop:!bottom-auto desktop:!top-1/2 desktop:max-h-[calc(100dvh-2rem)] desktop:w-[min(48rem,calc(100vw-4rem))] desktop:!translate-y-[-50%] desktop:rounded-[1.25rem]"
                    >
                      <div className="flex shrink-0 items-start justify-between gap-4 border-b border-base-300/80 px-5 py-4 desktop:px-6">
                        <DialogHeader className="min-w-0">
                          <DialogTitle className="text-lg">
                            {t("accountPool.upstreamAccounts.proxyBindings.dialogTitle")}
                          </DialogTitle>
                          <DialogDescription>
                            {t("accountPool.upstreamAccounts.proxyBindings.dialogDescription")}
                          </DialogDescription>
                        </DialogHeader>
                        <DialogCloseIcon
                          aria-label={t("accountPool.upstreamAccounts.actions.closeDetails")}
                          disabled={accountProxyEditorBusy}
                        />
                      </div>
                      <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4 desktop:px-6">
                        <ForwardProxyBindingSelector
                          selectedKeys={accountProxyDraftKeys}
                          availableProxyNodes={forwardProxyNodes}
                          disabled={accountProxyEditorBusy || !writesEnabled}
                          catalogKind={forwardProxyCatalogState.kind}
                          catalogFreshness={forwardProxyCatalogState.freshness}
                          onChange={setAccountProxyDraftKeys}
                          labels={{
                            automatic: t(
                              "accountPool.upstreamAccounts.proxyBindings.dialogAutomatic",
                            ),
                            loading: t("accountPool.upstreamAccounts.proxyBindings.loading"),
                            empty: t("accountPool.upstreamAccounts.proxyBindings.dialogEmpty"),
                            missing: t("accountPool.upstreamAccounts.proxyBindings.statusMissing"),
                            unavailable: t(
                              "accountPool.upstreamAccounts.proxyBindings.statusUnavailable",
                            ),
                            chartLabel: t(
                              "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartLabel",
                            ),
                            chartSuccess: t(
                              "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartSuccess",
                            ),
                            chartFailure: t(
                              "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartFailure",
                            ),
                            chartEmpty: t(
                              "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartEmpty",
                            ),
                            chartTotal: t(
                              "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartTotal",
                            ),
                            chartAriaLabel: t(
                              "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartAriaLabel",
                            ),
                            chartInteractionHint: t(
                              "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartInteractionHint",
                            ),
                            chartLocaleTag:
                              typeof navigator === "undefined" ? "en-US" : navigator.language,
                          }}
                          scrollRegionClassName="max-h-[min(29rem,58dvh)]"
                        />
                      </div>
                      <DialogFooter className="border-t border-base-300/80 bg-base-100/94 px-5 pb-[max(env(safe-area-inset-bottom),1rem)] pt-4 backdrop-blur desktop:justify-end desktop:px-6">
                        <Button
                          type="button"
                          variant="ghost"
                          disabled={accountProxyEditorBusy}
                          onClick={closeAccountProxyEditor}
                        >
                          {t("accountPool.upstreamAccounts.actions.cancel")}
                        </Button>
                        <Button
                          type="button"
                          disabled={accountProxyEditorBusy || !writesEnabled || !selectedDetail}
                          onClick={() => void applyAccountProxyEditor()}
                        >
                          {accountProxyEditorBusy ? (
                            <Spinner size="sm" className="mr-2" />
                          ) : (
                            <AppIcon
                              name="content-save-outline"
                              className="mr-2 h-4 w-4"
                              aria-hidden
                            />
                          )}
                          {t("accountPool.upstreamAccounts.proxyBindings.apply")}
                        </Button>
                      </DialogFooter>
                    </DialogContent>
                  </Dialog>
                  <EffectiveRoutingRuleCard
                    rule={selectedDetail.effectiveRoutingRule}
                    identityKey={selectedDetail.id}
                    proxyBindings={{
                      source: selectedAccountProxyKeys.length > 0 ? "account" : "group",
                      items: selectedEffectiveProxyKeys.map((key) => {
                        const node = selectedProxyNodeByKey.get(key);
                        return {
                          key,
                          label: proxyNodeLabel(node, key),
                          status: proxyNodeStatusLabel(node, key, t),
                          tone: proxyNodeTone(node, key),
                          accountOverride: selectedAccountProxyKeys.includes(key),
                        };
                      }),
                      busy: accountProxyEditorBusy,
                      disabled: !writesEnabled,
                      onEdit: openAccountProxyEditor,
                      onClear: () => void handleSaveAccountProxyBindings(selectedDetail, []),
                      onRemove: (key) =>
                        void handleSaveAccountProxyBindings(
                          selectedDetail,
                          toggleProxyKey(selectedAccountProxyKeys, key),
                        ),
                      labels: {
                        field: t("accountPool.upstreamAccounts.proxyBindings.accountTitle"),
                        add: t("accountPool.upstreamAccounts.proxyBindings.addLabel"),
                        clear: t("accountPool.upstreamAccounts.proxyBindings.clear"),
                        empty: t("accountPool.upstreamAccounts.proxyBindings.effectiveEmpty"),
                        hint: t("accountPool.upstreamAccounts.proxyBindings.failoverHint"),
                        remove: t("accountPool.upstreamAccounts.proxyBindings.remove"),
                      },
                    }}
                    editablePolicy={{
                      busyField: inlinePolicyBusyField,
                      errorByField: inlinePolicyErrors,
                      availableModelOptions,
                      onChange: (field, payload) =>
                        handleSaveInlineAccountPolicy(
                          selectedDetail,
                          field as InlinePolicyField,
                          payload,
                        ),
                    }}
                    labels={{
                      title: t("accountPool.upstreamAccounts.effectiveRule.title"),
                      description: t("accountPool.upstreamAccounts.effectiveRule.description"),
                      noTags: t("accountPool.upstreamAccounts.effectiveRule.noTags"),
                      allowCutOut: t("accountPool.upstreamAccounts.effectiveRule.allowCutOut"),
                      denyCutOut: t("accountPool.upstreamAccounts.effectiveRule.denyCutOut"),
                      allowCutIn: t("accountPool.upstreamAccounts.effectiveRule.allowCutIn"),
                      denyCutIn: t("accountPool.upstreamAccounts.effectiveRule.denyCutIn"),
                      sourceTags: t("accountPool.upstreamAccounts.effectiveRule.sourceTags"),
                      sourceBreakdownTitle: t(
                        "accountPool.upstreamAccounts.effectiveRule.sourceBreakdownTitle",
                      ),
                      fieldAllowCutOut: t(
                        "accountPool.upstreamAccounts.effectiveRule.fieldAllowCutOut",
                      ),
                      fieldAllowCutIn: t(
                        "accountPool.upstreamAccounts.effectiveRule.fieldAllowCutIn",
                      ),
                      priorityNoNew: t("accountPool.tags.dialog.priorityNoNew"),
                      fieldPriority: t("accountPool.upstreamAccounts.effectiveRule.fieldPriority"),
                      fieldFastMode: t("accountPool.upstreamAccounts.effectiveRule.fieldFastMode"),
                      fieldConcurrency: t(
                        "accountPool.upstreamAccounts.effectiveRule.fieldConcurrency",
                      ),
                      fieldUpstream429: t(
                        "accountPool.upstreamAccounts.effectiveRule.fieldUpstream429",
                      ),
                      fieldAvailableModels: t(
                        "accountPool.upstreamAccounts.effectiveRule.fieldAvailableModels",
                      ),
                      fieldSystemDeniedModels: t(
                        "accountPool.upstreamAccounts.effectiveRule.fieldSystemDeniedModels",
                      ),
                      fieldProxyBindings: t(
                        "accountPool.upstreamAccounts.effectiveRule.fieldProxyBindings",
                      ),
                      statusChangeReasonSectionTitle: t(
                        "accountPool.upstreamAccounts.statusChangeReasons.sectionTitle",
                      ),
                      statusChangeReasonSectionHint: t(
                        "accountPool.upstreamAccounts.statusChangeReasons.sectionHint",
                      ),
                      statusChangeReasonLabel: (reason: StatusChangeReasonCode) =>
                        t(`accountPool.upstreamAccounts.statusChangeReasons.reasons.${reason}`),
                      statusChangeReasonSummary: (enabled: number, total: number) =>
                        t("accountPool.upstreamAccounts.statusChangeReasons.summary", {
                          enabled,
                          total,
                        }),
                      statusChangeReasonEnabledValue: t(
                        "accountPool.upstreamAccounts.statusChangeReasons.enabledValue",
                      ),
                      statusChangeReasonDisabledValue: t(
                        "accountPool.upstreamAccounts.statusChangeReasons.disabledValue",
                      ),
                      statusChangeReasonToggleEnabled: t(
                        "accountPool.upstreamAccounts.statusChangeReasons.toggleEnabled",
                      ),
                      statusChangeReasonToggleDisabled: t(
                        "accountPool.upstreamAccounts.statusChangeReasons.toggleDisabled",
                      ),
                      availableModelsInherited: t(
                        "accountPool.upstreamAccounts.effectiveRule.availableModelsInherited",
                      ),
                      availableModelsNoneAllowed: t(
                        "accountPool.upstreamAccounts.effectiveRule.availableModelsNoneAllowed",
                      ),
                      systemDeniedModelsEmpty: t(
                        "accountPool.upstreamAccounts.effectiveRule.systemDeniedModelsEmpty",
                      ),
                      sourceRoot: t("accountPool.upstreamAccounts.effectiveRule.sourceRoot"),
                      sourceGroup: t("accountPool.upstreamAccounts.effectiveRule.sourceGroup"),
                      sourceTag: t("accountPool.upstreamAccounts.effectiveRule.sourceTag"),
                      sourceAccount: t("accountPool.upstreamAccounts.effectiveRule.sourceAccount"),
                      sourceConversation: t(
                        "accountPool.upstreamAccounts.effectiveRule.sourceConversation",
                      ),
                      sourceSystem: t("accountPool.upstreamAccounts.effectiveRule.sourceSystem"),
                      overrideEdit: t("accountPool.upstreamAccounts.effectiveRule.overrideEdit"),
                      overrideActive: t(
                        "accountPool.upstreamAccounts.effectiveRule.overrideActive",
                      ),
                      overrideClear: t("accountPool.upstreamAccounts.effectiveRule.overrideClear"),
                      statusChangeReasonResetAction: t(
                        "accountPool.upstreamAccounts.statusChangeReasons.resetAction",
                      ),
                      overrideSaving: t(
                        "accountPool.upstreamAccounts.effectiveRule.overrideSaving",
                      ),
                      inheritValue: t("accountPool.upstreamAccounts.effectiveRule.inheritValue"),
                      cutOutLabel: t("accountPool.upstreamAccounts.effectiveRule.fieldCutOut"),
                      cutInLabel: t("accountPool.upstreamAccounts.effectiveRule.fieldCutIn"),
                      upstream429RetryCountValue: (count) => String(count),
                      availableModelsAddCustom: t(
                        "accountPool.tags.dialog.availableModelsAddCustom",
                      ),
                      availableModelsCustomLabel: (value) =>
                        t("accountPool.tags.dialog.availableModelsCustomLabel", { value }),
                      availableModelsRemove: t("accountPool.tags.dialog.availableModelsRemove"),
                      availableModelsPlaceholder: t(
                        "accountPool.tags.dialog.availableModelsSearchPlaceholder",
                      ),
                      currentValue: t("accountPool.tags.dialog.currentValue"),
                      priorityPrimary: t(
                        "accountPool.upstreamAccounts.effectiveRule.priorityPrimary",
                      ),
                      priorityNormal: t(
                        "accountPool.upstreamAccounts.effectiveRule.priorityNormal",
                      ),
                      priorityFallback: t(
                        "accountPool.upstreamAccounts.effectiveRule.priorityFallback",
                      ),
                      fastModeKeepOriginal: t(
                        "accountPool.upstreamAccounts.effectiveRule.fastModeKeepOriginal",
                      ),
                      fastModeFillMissing: t(
                        "accountPool.upstreamAccounts.effectiveRule.fastModeFillMissing",
                      ),
                      fastModeForceAdd: t(
                        "accountPool.upstreamAccounts.effectiveRule.fastModeForceAdd",
                      ),
                      fastModeForceRemove: t(
                        "accountPool.upstreamAccounts.effectiveRule.fastModeForceRemove",
                      ),
                      imageToolKeepOriginal: t(
                        "accountPool.upstreamAccounts.groupNotes.routingPolicy.imageToolKeepOriginal",
                      ),
                      imageToolFillMissing: t(
                        "accountPool.upstreamAccounts.groupNotes.routingPolicy.imageToolFillMissing",
                      ),
                      imageToolForceAdd: t(
                        "accountPool.upstreamAccounts.groupNotes.routingPolicy.imageToolForceAdd",
                      ),
                      imageToolForceRemove: t(
                        "accountPool.upstreamAccounts.groupNotes.routingPolicy.imageToolForceRemove",
                      ),
                      fieldImageToolRewriteMode: t(
                        "accountPool.upstreamAccounts.effectiveRule.fieldImageToolRewriteMode",
                      ),
                      timeoutSectionTitle: t(
                        "accountPool.upstreamAccounts.routing.timeout.sectionTitle",
                      ),
                      timeoutInheritedValue: t(
                        "accountPool.upstreamAccounts.timeoutEditor.inherited",
                      ),
                      timeoutOverrideValue: t(
                        "accountPool.upstreamAccounts.timeoutEditor.accountOverride",
                      ),
                      timeoutResponsesFirstByte: t(
                        "accountPool.upstreamAccounts.routing.timeout.responsesFirstByte",
                      ),
                      timeoutCompactFirstByte: t(
                        "accountPool.upstreamAccounts.routing.timeout.compactFirstByte",
                      ),
                      timeoutResponsesStream: t(
                        "accountPool.upstreamAccounts.routing.timeout.responsesStream",
                      ),
                      timeoutCompactStream: t(
                        "accountPool.upstreamAccounts.routing.timeout.compactStream",
                      ),
                    }}
                  />

                  <Card className="border-base-300/80 bg-base-100/72">
                    <CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
                      <div>
                        <CardTitle>
                          {t("accountPool.upstreamAccounts.stickyConversations.title")}
                        </CardTitle>
                        <CardDescription>
                          {t("accountPool.upstreamAccounts.stickyConversations.description")}
                        </CardDescription>
                      </div>
                      <div className="flex flex-wrap items-center gap-2">
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          className="gap-2"
                          disabled={stickyConversationLoading || !hasVisibleStickyConversations}
                          onClick={toggleAllVisibleStickyKeys}
                        >
                          <AppIcon
                            name={allVisibleStickyKeysExpanded ? "chevron-up" : "chevron-down"}
                            className="h-4 w-4"
                            aria-hidden
                          />
                          {allVisibleStickyKeysExpanded
                            ? t("live.conversations.actions.collapseAllRecords")
                            : t("live.conversations.actions.expandAllRecords")}
                        </Button>
                        <SelectField
                          label={t("live.conversations.selectionLabel")}
                          className="w-40"
                          name="stickyConversationSelection"
                          size="sm"
                          value={stickyConversationSelectionValue}
                          options={stickyConversationSelectionOptions}
                          onValueChange={(value) => {
                            if (!STICKY_CONVERSATION_SELECTION_LOOKUP.has(value)) return;
                            setStickyConversationSelectionValue(value);
                          }}
                        />
                      </div>
                    </CardHeader>
                    <CardContent>
                      <StickyKeyConversationTable
                        accountId={selectedDetail.id}
                        accountDisplayName={selectedDetail.displayName}
                        stats={stickyConversationStats}
                        isLoading={stickyConversationLoading}
                        error={stickyConversationError}
                        expandedStickyKeys={expandedStickyKeys}
                        onToggleExpandedStickyKey={toggleExpandedStickyKey}
                        onOpenUpstreamAccount={handleOpenRelatedUpstreamAccount}
                      />
                    </CardContent>
                  </Card>
                </div>
              ) : null}

              {detailTab === "healthEvents" ? (
                <div
                  id={detailTabIds.healthEvents.panel}
                  role="tabpanel"
                  aria-labelledby={detailTabIds.healthEvents.tab}
                  className="grid gap-5"
                >
                  <Card className="border-base-300/80 bg-base-100/72">
                    <CardHeader>
                      <CardTitle>{t("accountPool.upstreamAccounts.healthTitle")}</CardTitle>
                      <CardDescription>
                        {t("accountPool.upstreamAccounts.healthDescription")}
                      </CardDescription>
                    </CardHeader>
                    <CardContent className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
                      <DetailField
                        label={t("accountPool.upstreamAccounts.fields.lastSyncedAt")}
                        value={formatDateTime(selectedDetail.lastSyncedAt)}
                      />
                      <DetailField
                        label={t("accountPool.upstreamAccounts.fields.lastRefreshedAt")}
                        value={formatDateTime(selectedDetail.lastRefreshedAt)}
                      />
                      <DetailField
                        label={t("accountPool.upstreamAccounts.fields.tokenExpiresAt")}
                        value={formatDateTime(selectedDetail.tokenExpiresAt)}
                      />
                      <DetailField
                        label={t("accountPool.upstreamAccounts.fields.compactSupport")}
                        value={
                          selectedDetail.compactSupport?.status === "supported"
                            ? t("accountPool.upstreamAccounts.compactSupport.status.supported")
                            : selectedDetail.compactSupport?.status === "unsupported"
                              ? t("accountPool.upstreamAccounts.compactSupport.status.unsupported")
                              : t("accountPool.upstreamAccounts.compactSupport.status.unknown")
                        }
                      />
                      <DetailField
                        label={t("accountPool.upstreamAccounts.fields.credits")}
                        value={
                          selectedDetail.credits?.balance
                            ? `${selectedDetail.credits.balance}`
                            : selectedDetail.credits?.unlimited
                              ? t("accountPool.upstreamAccounts.unlimited")
                              : t("accountPool.upstreamAccounts.unavailable")
                        }
                      />
                      <DetailField
                        label={t("accountPool.upstreamAccounts.fields.compactObservedAt")}
                        value={formatDateTime(selectedDetail.compactSupport?.observedAt)}
                      />
                      <DetailField
                        label={t("accountPool.upstreamAccounts.fields.compactReason")}
                        value={
                          selectedDetail.compactSupport?.reason ??
                          t("accountPool.upstreamAccounts.unavailable")
                        }
                      />
                      <div className="md:col-span-2 xl:col-span-4 rounded-[1.2rem] border border-base-300/80 bg-base-100/75 p-4">
                        {selectedRecoveryHint ? (
                          <Alert variant="warning" className="mb-4">
                            <AppIcon
                              name="alert-outline"
                              className="mt-0.5 h-4 w-4 shrink-0"
                              aria-hidden
                            />
                            <div>
                              <p className="font-semibold text-warning">
                                {t(selectedRecoveryHint.titleKey)}
                              </p>
                              <p className="mt-1 text-sm text-warning/90">
                                {t(selectedRecoveryHint.bodyKey)}
                              </p>
                            </div>
                          </Alert>
                        ) : null}
                        <p className="metric-label">
                          {t("accountPool.upstreamAccounts.latestAction.title")}
                        </p>
                        {selectedDetail.lastAction ? (
                          <div className="mt-3 grid gap-3 md:grid-cols-2 xl:grid-cols-3">
                            <DetailField
                              label={t("accountPool.upstreamAccounts.latestAction.fields.action")}
                              value={
                                accountActionLabel(selectedDetail.lastAction) ??
                                t("accountPool.upstreamAccounts.latestAction.empty")
                              }
                            />
                            <DetailField
                              label={t("accountPool.upstreamAccounts.latestAction.fields.source")}
                              value={
                                accountActionSourceLabel(selectedDetail.lastActionSource) ??
                                t("accountPool.upstreamAccounts.latestAction.unknown")
                              }
                            />
                            <DetailField
                              label={t("accountPool.upstreamAccounts.latestAction.fields.reason")}
                              value={
                                accountActionReasonLabel(selectedDetail.lastActionReasonCode) ??
                                t("accountPool.upstreamAccounts.latestAction.unknown")
                              }
                            />
                            <DetailField
                              label={t(
                                "accountPool.upstreamAccounts.latestAction.fields.httpStatus",
                              )}
                              value={
                                Number.isFinite(selectedDetail.lastActionHttpStatus ?? NaN)
                                  ? `HTTP ${selectedDetail.lastActionHttpStatus}`
                                  : t("accountPool.upstreamAccounts.unavailable")
                              }
                            />
                            <DetailField
                              label={t(
                                "accountPool.upstreamAccounts.latestAction.fields.occurredAt",
                              )}
                              value={formatDateTime(selectedDetail.lastActionAt)}
                            />
                            <DetailField
                              label={t("accountPool.upstreamAccounts.latestAction.fields.invokeId")}
                              value={
                                selectedDetail.lastActionInvokeId ? (
                                  <span className="select-text break-all font-mono">
                                    {selectedDetail.lastActionInvokeId}
                                  </span>
                                ) : (
                                  t("accountPool.upstreamAccounts.unavailable")
                                )
                              }
                            />
                            <div className="metric-cell md:col-span-2 xl:col-span-3">
                              <p className="metric-label">
                                {t("accountPool.upstreamAccounts.latestAction.fields.message")}
                              </p>
                              <p className="mt-2 break-words text-sm leading-6 text-base-content/80">
                                {selectedDetail.lastActionReasonMessage ??
                                  selectedDetail.lastError ??
                                  t("accountPool.upstreamAccounts.noError")}
                              </p>
                            </div>
                          </div>
                        ) : (
                          <p className="mt-2 text-sm leading-6 text-base-content/75">
                            {t("accountPool.upstreamAccounts.latestAction.empty")}
                          </p>
                        )}
                      </div>
                    </CardContent>
                  </Card>

                  <Card className="border-base-300/80 bg-base-100/72">
                    <CardHeader>
                      <CardTitle>{t("accountPool.upstreamAccounts.recentActions.title")}</CardTitle>
                      <CardDescription>
                        {t("accountPool.upstreamAccounts.recentActions.description")}
                      </CardDescription>
                    </CardHeader>
                    <CardContent>
                      {selectedRecentActions.length === 0 ? (
                        <p className="text-sm leading-6 text-base-content/68">
                          {t("accountPool.upstreamAccounts.recentActions.empty")}
                        </p>
                      ) : (
                        <div className="space-y-2">
                          {selectedRecentActions.map((actionEvent) => (
                            <div
                              key={actionEvent.id}
                              className="rounded-[1rem] border border-base-300/70 bg-base-100/70 p-3"
                            >
                              <div className="flex flex-wrap items-center gap-2">
                                <Badge variant="secondary">
                                  {accountActionLabel(actionEvent.action) ??
                                    t("accountPool.upstreamAccounts.latestAction.unknown")}
                                </Badge>
                                <Badge variant="secondary">
                                  {accountActionSourceLabel(actionEvent.source) ??
                                    t("accountPool.upstreamAccounts.latestAction.unknown")}
                                </Badge>
                                {actionEvent.reasonCode ? (
                                  <Badge variant="secondary">
                                    {accountActionReasonLabel(actionEvent.reasonCode)}
                                  </Badge>
                                ) : null}
                                {Number.isFinite(actionEvent.httpStatus ?? NaN) ? (
                                  <Badge variant="secondary">{`HTTP ${actionEvent.httpStatus}`}</Badge>
                                ) : null}
                                <span className="text-xs text-base-content/55">
                                  {formatDateTime(actionEvent.occurredAt)}
                                </span>
                              </div>
                              {actionEvent.reasonMessage ? (
                                <p className="mt-2 text-sm leading-6 text-base-content/75">
                                  {actionEvent.reasonMessage}
                                </p>
                              ) : null}
                              {actionEvent.attemptId != null ? (
                                <button
                                  type="button"
                                  className="mt-2 block select-text break-all font-mono text-xs text-info underline decoration-info/35 underline-offset-4 transition hover:decoration-info focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                                  onClick={() => locateAccountAttempt(actionEvent.attemptId)}
                                >
                                  {t("accountPool.upstreamAccounts.latestAction.fields.attemptId")}:{" "}
                                  {actionEvent.attemptId}
                                </button>
                              ) : actionEvent.invokeId ? (
                                <p className="mt-2 break-all font-mono text-xs text-base-content/55">
                                  {t("accountPool.upstreamAccounts.latestAction.fields.invokeId")}:{" "}
                                  {actionEvent.invokeId}（历史事件未关联尝试）
                                </p>
                              ) : null}
                            </div>
                          ))}
                        </div>
                      )}
                    </CardContent>
                  </Card>
                </div>
              ) : null}
            </div>
          ) : (
            <AccountDetailSkeleton />
          )}
        </AccountDetailDrawerShell>
      ) : null}

      {groupNoteDialog}
    </>
  );
}
