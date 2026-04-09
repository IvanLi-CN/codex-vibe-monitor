/* eslint-disable @typescript-eslint/ban-ts-comment, @typescript-eslint/no-unused-vars, react-refresh/only-export-components */
// @ts-nocheck
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { AppIcon, type AppIconName } from "../../components/AppIcon";
import { AccountDetailDrawerShell } from "../../components/AccountDetailDrawerShell";
import { Link, useNavigate } from "react-router-dom";
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
import {
  Popover,
  PopoverArrow,
  PopoverContent,
  PopoverTrigger,
} from "../../components/ui/popover";
import { formFieldSpanVariants } from "../../components/ui/form-control";
import { SelectField } from "../../components/ui/select-field";
import {
  SegmentedControl,
  SegmentedControlItem,
} from "../../components/ui/segmented-control";
import {
  MotherAccountBadge,
  MotherAccountToggle,
} from "../../components/MotherAccountToggle";
import { Spinner } from "../../components/ui/spinner";
import { Switch } from "../../components/ui/switch";
import { AccountTagField } from "../../components/AccountTagField";
import { AccountTagFilterCombobox } from "../../components/AccountTagFilterCombobox";
import { EffectiveRoutingRuleCard } from "../../components/EffectiveRoutingRuleCard";
import { InvocationTable } from "../../components/InvocationTable";
import { MultiSelectFilterCombobox } from "../../components/MultiSelectFilterCombobox";
import { UpstreamAccountGroupCombobox } from "../../components/UpstreamAccountGroupCombobox";
import { UpstreamAccountGroupNoteDialog } from "../../components/UpstreamAccountGroupNoteDialog";
import { UpstreamAccountUsageCard } from "../../components/UpstreamAccountUsageCard";
import { StickyKeyConversationTable } from "../../components/StickyKeyConversationTable";
import { UpstreamAccountsTable } from "../../components/UpstreamAccountsTable";
import { usePoolTags } from "../../hooks/usePoolTags";
import { useMotherSwitchNotifications } from "../../hooks/useMotherSwitchNotifications";
import { useUpstreamAccountDetailRoute } from "../../hooks/useUpstreamAccountDetailRoute";
import { useUpstreamAccounts } from "../../hooks/useUpstreamAccounts";
import { useUpstreamStickyConversations } from "../../hooks/useUpstreamStickyConversations";
import type {
  ApiInvocation,
  BulkUpstreamAccountActionPayload,
  BulkUpstreamAccountSyncCounts,
  BulkUpstreamAccountSyncRow,
  BulkUpstreamAccountSyncSnapshot,
  PoolRoutingMaintenanceSettings,
  CompactSupportState,
  FetchUpstreamAccountsQuery,
  StickyKeyConversationSelection,
  PoolRoutingTimeoutSettings,
  UpstreamAccountDetail,
  UpstreamAccountDuplicateInfo,
  UpstreamAccountSummary,
} from "../../lib/api";
import { DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS } from "../../lib/api";
import {
  createBulkUpstreamAccountSyncJobEventSource,
  fetchInvocationRecords,
  normalizeBulkUpstreamAccountSyncFailedEventPayload,
  normalizeBulkUpstreamAccountSyncRowEventPayload,
  normalizeBulkUpstreamAccountSyncSnapshotEventPayload,
} from "../../lib/api";
import {
  buildGroupNameSuggestions,
  isExistingGroup,
  normalizeGroupName,
  resolveGroupConcurrencyLimit,
  resolveGroupNote,
} from "../../lib/upstreamAccountGroups";
import {
  apiConcurrencyLimitToSliderValue,
  sliderConcurrencyLimitToApiValue,
} from "../../lib/concurrencyLimit";
import {
  areAccountDraftsEqual,
  mergeDraftAfterAccountSave,
  type AccountDraft,
} from "../../lib/upstreamAccountDrafts";
import { resolvePersistedGroupNodeShuntEnabled } from "../../lib/upstreamAccountGroupDrafts";
import { validateUpstreamBaseUrl } from "../../lib/upstreamBaseUrl";
import { generatePoolRoutingKey } from "../../lib/poolRouting";
import { applyMotherUpdateToItems } from "../../lib/upstreamMother";
import { upstreamPlanBadgeRecipe } from "../../lib/upstreamAccountBadges";
import { isUpstreamAccountNotFoundError } from "../../lib/upstreamAccountErrors";
import { cn } from "../../lib/utils";
import { useTranslation, type TranslationValues } from "../../i18n";

type RoutingDraft = {
  apiKey: string;
  maskedApiKey: string | null;
  primarySyncIntervalSecs: string;
  secondarySyncIntervalSecs: string;
  priorityAvailableAccountCap: string;
  responsesFirstByteTimeoutSecs: string;
  compactFirstByteTimeoutSecs: string;
  responsesStreamTimeoutSecs: string;
  compactStreamTimeoutSecs: string;
};

export const DEFAULT_ROUTING_TIMEOUTS: PoolRoutingTimeoutSettings = {
  responsesFirstByteTimeoutSecs: 120,
  compactFirstByteTimeoutSecs: 300,
  responsesStreamTimeoutSecs: 300,
  compactStreamTimeoutSecs: 300,
};
const POSITIVE_INTEGER_PATTERN = /^[1-9]\d*$/;

const ACCOUNT_RECORD_LIMIT_OPTIONS = [20, 50, 100] as const;
const DEFAULT_STICKY_CONVERSATION_SELECTION_VALUE = "count:50";
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
const STICKY_CONVERSATION_SELECTION_LOOKUP = new Map<
  string,
  StickyKeyConversationSelection
>(
  STICKY_CONVERSATION_SELECTION_OPTIONS.map((option) => [
    option.value,
    option.selection,
  ]),
);
const GROUP_UPSTREAM_429_RETRY_OPTIONS = [1, 2, 3, 4, 5] as const;
const UPSTREAM_ACCOUNTS_FILTER_STORAGE_KEY =
  "codex-vibe-monitor.account-pool.upstream-accounts.filters";
export const UPSTREAM_ACCOUNTS_QUERY_STALE_GRACE_MS = 600;
const WORK_STATUS_FILTER_VALUES = [
  "working",
  "degraded",
  "idle",
  "rate_limited",
  "unavailable",
] as const;
const ENABLE_STATUS_FILTER_VALUES = ["enabled", "disabled"] as const;
const HEALTH_STATUS_FILTER_VALUES = [
  "normal",
  "needs_reauth",
  "upstream_unavailable",
  "upstream_rejected",
  "error_other",
] as const;

type GroupFilterMode = "all" | "ungrouped" | "search";

export type GroupFilterState = {
  mode: GroupFilterMode;
  query: string;
};

type PersistedUpstreamAccountsFilters = {
  workStatus: string[];
  enableStatus: string[];
  healthStatus: string[];
  tagIds: number[];
  groupFilter: GroupFilterState;
};

export type UpstreamAccountsLocationState = {
  selectedAccountId?: number;
  openDetail?: boolean;
  openDeleteConfirm?: boolean;
  postCreateWarning?: string | null;
  duplicateWarning?: {
    accountId: number;
    displayName: string;
    peerAccountIds: number[];
    reasons: string[];
  } | null;
};

type GroupSettingsEditorState = {
  open: boolean;
  groupName: string;
  note: string;
  existing: boolean;
  concurrencyLimit: number;
  boundProxyKeys: string[];
  nodeShuntEnabled: boolean;
  upstream429RetryEnabled: boolean;
  upstream429MaxRetries: number;
};

type OauthRecoveryHint = {
  titleKey: string;
  bodyKey: string;
};

export type ActionErrorState = {
  routing: string | null;
  accountMessages: Record<number, string>;
};

type AccountBusyActionType = "save" | "sync" | "toggle" | "relogin" | "delete";

export type BusyActionState = {
  routing: boolean;
  accountActions: Set<string>;
};

type AccountDetailTab =
  | "overview"
  | "records"
  | "edit"
  | "routing"
  | "healthEvents";

type SharedUpstreamAccountDetailDrawerCloseOptions = {
  replace?: boolean;
};

type SharedUpstreamAccountDetailDrawerProps = {
  open: boolean;
  accountId: number | null;
  initialDeleteConfirmOpen?: boolean;
  onInitialDeleteConfirmHandled?: () => void;
  onClose: (options?: SharedUpstreamAccountDetailDrawerCloseOptions) => void;
};

type PendingSaveSession = {
  accountId: number;
  sessionKey: string | null;
  fallbackDraft: AccountDraft;
};

type RecentSaveResponseGuard = {
  accountId: number;
  sessionKey: string | null;
  draft: AccountDraft;
  fallbackDraft: AccountDraft;
  retainedDraft: AccountDraft;
};

const DEFAULT_GROUP_FILTER_STATE: GroupFilterState = {
  mode: "all",
  query: "",
};

const DEFAULT_PERSISTED_UPSTREAM_ACCOUNT_FILTERS: PersistedUpstreamAccountsFilters =
  {
    workStatus: [],
    enableStatus: [],
    healthStatus: [],
    tagIds: [],
    groupFilter: DEFAULT_GROUP_FILTER_STATE,
  };

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function sanitizeFilterValues(
  value: unknown,
  allowedValues: readonly string[],
) {
  if (!Array.isArray(value)) return [];
  const allowed = new Set(allowedValues);
  const next: string[] = [];
  for (const item of value) {
    if (typeof item !== "string" || !allowed.has(item) || next.includes(item)) {
      continue;
    }
    next.push(item);
  }
  return next;
}

function sanitizeTagIds(value: unknown) {
  if (!Array.isArray(value)) return [];
  const next: number[] = [];
  for (const item of value) {
    if (!Number.isInteger(item) || item <= 0 || next.includes(item)) {
      continue;
    }
    next.push(item);
  }
  return next;
}

function sanitizeGroupFilterState(value: unknown): GroupFilterState {
  if (!isPlainObject(value)) return DEFAULT_GROUP_FILTER_STATE;
  const mode = value.mode;
  if (mode === "ungrouped") {
    return {
      mode,
      query: "",
    };
  }
  if (mode === "search") {
    const query = typeof value.query === "string" ? value.query.trim() : "";
    if (query) {
      return {
        mode,
        query,
      };
    }
  }
  return DEFAULT_GROUP_FILTER_STATE;
}

export function readPersistedUpstreamAccountFilters(): PersistedUpstreamAccountsFilters {
  if (typeof window === "undefined") {
    return DEFAULT_PERSISTED_UPSTREAM_ACCOUNT_FILTERS;
  }
  try {
    const raw = window.localStorage.getItem(
      UPSTREAM_ACCOUNTS_FILTER_STORAGE_KEY,
    );
    if (!raw) {
      return DEFAULT_PERSISTED_UPSTREAM_ACCOUNT_FILTERS;
    }
    const parsed = JSON.parse(raw);
    if (!isPlainObject(parsed)) {
      return DEFAULT_PERSISTED_UPSTREAM_ACCOUNT_FILTERS;
    }
    return {
      workStatus: sanitizeFilterValues(
        parsed.workStatus,
        WORK_STATUS_FILTER_VALUES,
      ),
      enableStatus: sanitizeFilterValues(
        parsed.enableStatus,
        ENABLE_STATUS_FILTER_VALUES,
      ),
      healthStatus: sanitizeFilterValues(
        parsed.healthStatus,
        HEALTH_STATUS_FILTER_VALUES,
      ),
      tagIds: sanitizeTagIds(parsed.tagIds),
      groupFilter: sanitizeGroupFilterState(parsed.groupFilter),
    };
  } catch {
    return DEFAULT_PERSISTED_UPSTREAM_ACCOUNT_FILTERS;
  }
}

export function persistUpstreamAccountFilters(
  value: PersistedUpstreamAccountsFilters,
) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(
      UPSTREAM_ACCOUNTS_FILTER_STORAGE_KEY,
      JSON.stringify(value),
    );
  } catch {
    // Ignore storage write failures and keep the current UI state.
  }
}

export function formatGroupFilterValue(
  groupFilter: GroupFilterState,
  labels: { ungrouped: string },
) {
  if (groupFilter.mode === "ungrouped") {
    return labels.ungrouped;
  }
  if (groupFilter.mode === "search") {
    return groupFilter.query;
  }
  return "";
}

export function parseGroupFilterValue(
  value: string,
  labels: { all: string; ungrouped: string },
): GroupFilterState {
  const normalized = value.trim();
  if (!normalized) {
    return DEFAULT_GROUP_FILTER_STATE;
  }
  const normalizedLower = normalized.toLocaleLowerCase();
  if (normalizedLower === labels.all.trim().toLocaleLowerCase()) {
    return DEFAULT_GROUP_FILTER_STATE;
  }
  if (normalizedLower === labels.ungrouped.trim().toLocaleLowerCase()) {
    return {
      mode: "ungrouped",
      query: "",
    };
  }
  return {
    mode: "search",
    query: normalized,
  };
}

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

function hasBusyAccountAction(
  busyAction: BusyActionState,
  accountId?: number | null,
) {
  if (typeof accountId !== "number") return false;
  const suffix = `:${accountId}`;
  for (const key of busyAction.accountActions) {
    if (key.endsWith(suffix)) return true;
  }
  return false;
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
      (item) =>
        item.id !== excludeId &&
        normalizeDisplayNameKey(item.displayName) === normalized,
    ) ?? null
  );
}

function buildDraft(detail: UpstreamAccountDetail | null): AccountDraft {
  return {
    displayName: detail?.displayName ?? "",
    groupName: detail?.groupName ?? "",
    isMother: detail?.isMother ?? false,
    note: detail?.note ?? "",
    upstreamBaseUrl: detail?.upstreamBaseUrl ?? "",
    tagIds: detail?.tags?.map((tag) => tag.id) ?? [],
    localPrimaryLimit:
      detail?.localLimits?.primaryLimit == null
        ? ""
        : String(detail.localLimits.primaryLimit),
    localSecondaryLimit:
      detail?.localLimits?.secondaryLimit == null
        ? ""
        : String(detail.localLimits.secondaryLimit),
    localLimitUnit: detail?.localLimits?.limitUnit ?? "requests",
    apiKey: "",
  };
}

function filterAccountDraftTagIds(
  draft: AccountDraft,
  validTagIds: Set<number>,
): AccountDraft {
  const nextTagIds = draft.tagIds.filter((tagId) => validTagIds.has(tagId));
  return nextTagIds.length === draft.tagIds.length
    ? draft
    : { ...draft, tagIds: nextTagIds };
}

function removeAccountDraftTagId(draft: AccountDraft, tagId: number) {
  return draft.tagIds.includes(tagId)
    ? { ...draft, tagIds: draft.tagIds.filter((value) => value !== tagId) }
    : draft;
}

export function resolveRoutingMaintenance(
  maintenance?: PoolRoutingMaintenanceSettings | null,
): PoolRoutingMaintenanceSettings {
  return {
    primarySyncIntervalSecs:
      maintenance?.primarySyncIntervalSecs ??
      DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.primarySyncIntervalSecs,
    secondarySyncIntervalSecs:
      maintenance?.secondarySyncIntervalSecs ??
      DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.secondarySyncIntervalSecs,
    priorityAvailableAccountCap:
      maintenance?.priorityAvailableAccountCap ??
      DEFAULT_POOL_ROUTING_MAINTENANCE_SETTINGS.priorityAvailableAccountCap,
  };
}

export function buildRoutingDraft(
  routing?: {
    maskedApiKey?: string | null;
    maintenance?: PoolRoutingMaintenanceSettings | null;
    timeouts?: PoolRoutingTimeoutSettings | null;
  } | null,
): RoutingDraft {
  const maintenance = resolveRoutingMaintenance(routing?.maintenance);
  const timeouts = routing?.timeouts ?? DEFAULT_ROUTING_TIMEOUTS;
  return {
    apiKey: "",
    maskedApiKey: routing?.maskedApiKey ?? null,
    primarySyncIntervalSecs: String(maintenance.primarySyncIntervalSecs),
    secondarySyncIntervalSecs: String(maintenance.secondarySyncIntervalSecs),
    priorityAvailableAccountCap: String(
      maintenance.priorityAvailableAccountCap,
    ),
    responsesFirstByteTimeoutSecs: String(
      timeouts.responsesFirstByteTimeoutSecs,
    ),
    compactFirstByteTimeoutSecs: String(timeouts.compactFirstByteTimeoutSecs),
    responsesStreamTimeoutSecs: String(timeouts.responsesStreamTimeoutSecs),
    compactStreamTimeoutSecs: String(timeouts.compactStreamTimeoutSecs),
  };
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
  if (item?.enabled === false || item?.displayStatus === "disabled")
    return "disabled";
  return "enabled";
}

export function accountWorkStatus(item?: AccountStatusSnapshot | null) {
  if (!item) return "idle";
  if (accountEnableStatus(item) !== "enabled") return "idle";
  if (accountSyncState(item) === "syncing") return "idle";
  if (item?.workStatus === "degraded") return "degraded";
  if (item?.workStatus === "rate_limited") return "rate_limited";
  if (accountHealthStatus(item) !== "normal") return "unavailable";
  return item?.workStatus ?? "idle";
}

export function accountHealthStatus(item?: AccountStatusSnapshot | null) {
  if (item?.healthStatus) return item.healthStatus;
  const legacyStatus = item?.displayStatus ?? item?.status ?? "error_other";
  if (
    legacyStatus === "needs_reauth" ||
    legacyStatus === "upstream_unavailable" ||
    legacyStatus === "upstream_rejected" ||
    legacyStatus === "error_other"
  ) {
    return legacyStatus;
  }
  if (legacyStatus === "error") {
    return "error_other";
  }
  return "normal";
}

function accountSyncState(item?: AccountStatusSnapshot | null) {
  if (item?.syncState) return item.syncState;
  const legacyStatus = item?.displayStatus ?? item?.status;
  return legacyStatus === "syncing" ? "syncing" : "idle";
}

export function parseRoutingPositiveInteger(value: string) {
  const trimmed = value.trim();
  if (!trimmed || !/^\d+$/.test(trimmed)) return null;
  const parsed = Number(trimmed);
  return Number.isSafeInteger(parsed) ? parsed : null;
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

function healthStatusVariant(
  status: string,
): "success" | "warning" | "error" | "secondary" {
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

export function bulkSyncRowStatusVariant(
  status: string,
): "success" | "warning" | "error" | "secondary" {
  if (status === "succeeded") return "success";
  if (status === "pending") return "warning";
  if (status === "failed") return "error";
  return "secondary";
}

function computeBulkSyncCounts(
  rows: BulkUpstreamAccountSyncRow[],
): BulkUpstreamAccountSyncCounts {
  return rows.reduce<BulkUpstreamAccountSyncCounts>(
    (counts, row) => {
      counts.total += 1;
      if (row.status === "succeeded") {
        counts.succeeded += 1;
        counts.completed += 1;
      } else if (row.status === "failed") {
        counts.failed += 1;
        counts.completed += 1;
      } else if (row.status === "skipped") {
        counts.skipped += 1;
        counts.completed += 1;
      }
      return counts;
    },
    {
      total: 0,
      completed: 0,
      succeeded: 0,
      failed: 0,
      skipped: 0,
    },
  );
}

export function resolveBulkSyncCounts(
  snapshot: BulkUpstreamAccountSyncSnapshot,
  counts?: BulkUpstreamAccountSyncCounts | null,
) {
  return counts ?? computeBulkSyncCounts(snapshot.rows);
}

export function withBulkSyncSnapshotStatus(
  snapshot: BulkUpstreamAccountSyncSnapshot,
  status: BulkUpstreamAccountSyncSnapshot["status"],
) {
  if (snapshot.status === status) return snapshot;
  return {
    ...snapshot,
    status,
  };
}

export function shouldAutoHideBulkSyncProgress(
  snapshot: BulkUpstreamAccountSyncSnapshot,
  counts: BulkUpstreamAccountSyncCounts,
) {
  return (
    snapshot.status === "completed" &&
    counts.failed === 0 &&
    counts.skipped === 0
  );
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

export function compactSupportLabel(
  support: CompactSupportState | null | undefined,
  t: (key: string) => string,
) {
  if (!support || support.status !== "unsupported") return null;
  return t("accountPool.upstreamAccounts.compactSupport.unsupportedBadge");
}

export function compactSupportHint(
  support: CompactSupportState | null | undefined,
  t: (key: string, values?: TranslationValues) => string,
) {
  if (!support || support.status === "unknown") return null;
  const statusLabel =
    support.status === "unsupported"
      ? t("accountPool.upstreamAccounts.compactSupport.status.unsupported")
      : t("accountPool.upstreamAccounts.compactSupport.status.supported");
  const observedAt = support.observedAt
    ? formatDateTime(support.observedAt)
    : t("accountPool.upstreamAccounts.unavailable");
  if (support.reason) {
    return `${statusLabel} · ${observedAt} · ${support.reason}`;
  }
  return `${statusLabel} · ${observedAt}`;
}

export function parseRoutingTimeoutValue(
  raw: string,
  label: string,
): { ok: true; value: number } | { ok: false; error: string } {
  const trimmed = raw.trim();
  if (!trimmed) {
    return { ok: false, error: `${label} is required.` };
  }
  if (!POSITIVE_INTEGER_PATTERN.test(trimmed)) {
    return { ok: false, error: `${label} must be a positive integer.` };
  }
  const parsed = Number(trimmed);
  if (!Number.isSafeInteger(parsed)) {
    return { ok: false, error: `${label} must be a positive integer.` };
  }
  return { ok: true, value: parsed };
}

export function poolCardMetric(
  value: number | string,
  label: string,
  icon: AppIconName,
  accent: string,
) {
  return { value, label, icon, accent };
}

function AccountDetailSkeleton() {
  return (
    <div className="grid gap-4">
      {Array.from({ length: 3 }).map((_, index) => (
        <div
          key={index}
          className="h-28 animate-pulse rounded-[1.35rem] bg-base-200/75"
        />
      ))}
    </div>
  );
}

function DetailField({ label, value }: { label: string; value: string }) {
  return (
    <div className="metric-cell">
      <p className="metric-label">{label}</p>
      <p className="mt-2 break-all text-sm text-base-content/80">
        {value || "—"}
      </p>
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
      onOpenChange={(nextOpen) =>
        !busy ? (nextOpen ? undefined : onClose()) : undefined
      }
    >
      <DialogContent
        className="flex max-h-[calc(100dvh-2rem)] flex-col overflow-hidden p-0 sm:max-h-[calc(100dvh-4rem)]"
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
        <div className="flex items-start justify-between gap-4 border-b border-base-300/80 px-6 py-5">
          <DialogHeader className="min-w-0 max-w-[28rem]">
            <DialogTitle>{title}</DialogTitle>
            <DialogDescription>{description}</DialogDescription>
          </DialogHeader>
          <DialogCloseIcon aria-label={closeLabel} disabled={busy} />
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-6">
          <div className="space-y-4">
            <div className="space-y-3 rounded-2xl border border-base-300/80 bg-base-100/70 p-4">
              <div className="space-y-1">
                <p className="text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82">
                  {t("accountPool.upstreamAccounts.routing.apiKeySectionTitle")}
                </p>
                <p className="text-sm text-base-content/68">
                  {t(
                    "accountPool.upstreamAccounts.routing.apiKeySectionDescription",
                  )}
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
                    <AppIcon
                      name="auto-fix"
                      className="mr-2 h-4 w-4"
                      aria-hidden
                    />
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
                  placeholder={t(
                    "accountPool.upstreamAccounts.routing.apiKeyPlaceholder",
                  )}
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
                  {t(
                    "accountPool.upstreamAccounts.routing.maintenanceSectionTitle",
                  )}
                </p>
                <p className="text-sm text-base-content/68">
                  {t(
                    "accountPool.upstreamAccounts.routing.maintenanceSectionDescription",
                  )}
                </p>
              </div>
              <div className="grid gap-4 sm:grid-cols-2">
                <div className="field">
                  <label
                    htmlFor={primaryInputId}
                    className="mb-2 text-sm font-semibold uppercase tracking-[0.14em] text-base-content/82"
                  >
                    {t(
                      "accountPool.upstreamAccounts.routing.primarySyncIntervalLabel",
                    )}
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
                    onChange={(event) =>
                      onPrimarySyncIntervalChange(event.target.value)
                    }
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
                    {t(
                      "accountPool.upstreamAccounts.routing.secondarySyncIntervalLabel",
                    )}
                  </label>
                  <Input
                    id={secondaryInputId}
                    name="secondarySyncIntervalSecs"
                    type="number"
                    min={60}
                    step={60}
                    inputMode="numeric"
                    value={secondarySyncIntervalSecs}
                    onChange={(event) =>
                      onSecondarySyncIntervalChange(event.target.value)
                    }
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
                  onChange={(event) =>
                    onPriorityAvailableAccountCapChange(event.target.value)
                  }
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
        <DialogFooter className="border-t border-base-300/80 px-6 py-5">
          <Button
            type="button"
            variant="outline"
            onClick={onClose}
            disabled={busy}
          >
            {cancelLabel}
          </Button>
          <Button type="button" onClick={onSave} disabled={busy || !canSave}>
            {busy ? (
              <Spinner size="sm" className="mr-2" />
            ) : (
              <AppIcon
                name="key-chain-variant"
                className="mr-2 h-4 w-4"
                aria-hidden
              />
            )}
            {saveLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

export function SharedUpstreamAccountDetailDrawer({
  open,
  accountId,
  initialDeleteConfirmOpen = false,
  onInitialDeleteConfirmHandled,
  onClose,
}: SharedUpstreamAccountDetailDrawerProps) {
  const { t, locale } = useTranslation();
  const navigate = useNavigate();
  const { openUpstreamAccount } = useUpstreamAccountDetailRoute();
  const {
    items,
    groups = [],
    forwardProxyNodes = [],
    hasUngroupedAccounts = false,
    writesEnabled,
    selectedId,
    selectedSummary,
    detail,
    isDetailLoading,
    detailError = null,
    selectAccount,
    saveAccount,
    runSync,
    removeAccount,
    saveGroupNote,
    missingDetailAccountId,
  } = useUpstreamAccounts(undefined, {
    allowSelectionOutsideList: true,
    fallbackToFirstItem: false,
  });
  const { items: tagItems, createTag, updateTag, deleteTag } = usePoolTags();
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
  const [isDeleteConfirmOpen, setIsDeleteConfirmOpen] = useState(false);
  const [pageCreatedTagIds, setPageCreatedTagIds] = useState<number[]>([]);
  const [
    stickyConversationSelectionValue,
    setStickyConversationSelectionValue,
  ] = useState(DEFAULT_STICKY_CONVERSATION_SELECTION_VALUE);
  const [expandedStickyKeys, setExpandedStickyKeys] = useState<string[]>([]);
  const [accountRecordLimit, setAccountRecordLimit] = useState<number>(
    ACCOUNT_RECORD_LIMIT_OPTIONS[1],
  );
  const [accountRecords, setAccountRecords] = useState<ApiInvocation[]>([]);
  const [accountRecordsLoading, setAccountRecordsLoading] = useState(false);
  const [accountRecordsError, setAccountRecordsError] = useState<string | null>(
    null,
  );
  const [groupDraftNotes, setGroupDraftNotes] = useState<
    Record<string, string>
  >({});
  const [groupDraftBoundProxyKeys, setGroupDraftBoundProxyKeys] = useState<
    Record<string, string[]>
  >({});
  const [groupDraftConcurrencyLimits, setGroupDraftConcurrencyLimits] =
    useState<Record<string, number>>({});
  const [groupDraftNodeShuntEnabled, setGroupDraftNodeShuntEnabled] = useState<
    Record<string, boolean>
  >({});
  const [
    groupDraftUpstream429RetryEnabled,
    setGroupDraftUpstream429RetryEnabled,
  ] = useState<Record<string, boolean>>({});
  const [groupDraftUpstream429MaxRetries, setGroupDraftUpstream429MaxRetries] =
    useState<Record<string, number>>({});
  const [groupNoteEditor, setGroupNoteEditor] =
    useState<GroupSettingsEditorState>({
      open: false,
      groupName: "",
      note: "",
      existing: false,
      concurrencyLimit: apiConcurrencyLimitToSliderValue(0),
      boundProxyKeys: [],
      nodeShuntEnabled: false,
      upstream429RetryEnabled: false,
      upstream429MaxRetries: 0,
    });
  const [groupNoteBusy, setGroupNoteBusy] = useState(false);
  const [groupNoteError, setGroupNoteError] = useState<string | null>(null);
  const [detailDrawerPortalContainer, setDetailDrawerPortalContainer] =
    useState<HTMLElement | null>(null);
  const [detailTab, setDetailTab] = useState<AccountDetailTab>("overview");
  const validTagIds = useMemo(
    () => new Set(tagItems.map((tag) => tag.id)),
    [tagItems],
  );
  const deleteConfirmCancelRef = useRef<HTMLButtonElement | null>(null);
  const detailDrawerTitleId = "upstream-account-detail-title";
  const detailDrawerTabsBaseId = useId();
  const deleteConfirmTitleId = useId();
  const selectedIdRef = useRef<number | null>(selectedId);
  const routeAccountIdRef = useRef<number | null>(accountId);
  const drawerOpenRef = useRef(open);
  const accountRecordsRequestSeqRef = useRef(0);
  const draftSessionKeyRef = useRef<string | null>(null);
  const activeDraftSessionKeyRef = useRef<string | null>(null);
  const draftBaselineRef = useRef<AccountDraft>(buildDraft(null));
  const latestServerDraftRef = useRef<AccountDraft>(buildDraft(null));
  const validTagIdsRef = useRef(validTagIds);
  const pendingSaveSessionsRef = useRef<Map<number, PendingSaveSession>>(
    new Map(),
  );
  const recentSaveResponseGuardsRef = useRef<
    Map<number, RecentSaveResponseGuard>
  >(new Map());
  validTagIdsRef.current = validTagIds;
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
    const closedActiveSession =
      draftSessionSnapshotRef.current.open && !open;
    const openedVisibleSession =
      !draftSessionSnapshotRef.current.open && open;
    const switchedVisibleAccount =
      draftSessionSnapshotRef.current.open &&
      open &&
      draftSessionSnapshotRef.current.accountId !== accountId;
    if (
      closedActiveSession ||
      openedVisibleSession ||
      switchedVisibleAccount
    ) {
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
    const nextBaseline = filterAccountDraftTagIds(
      buildDraft(detail?.id === accountId ? detail : null),
      validTagIdsRef.current,
    );
    if (activeDraftSessionKey == null) {
      draftSessionKeyRef.current = null;
      draftBaselineRef.current = nextBaseline;
      latestServerDraftRef.current = nextBaseline;
      return;
    }

    const previousBaseline = draftBaselineRef.current;
    const previousLatestServerDraft = latestServerDraftRef.current;
    const shouldSeedDraft =
      draftSessionKeyRef.current !== activeDraftSessionKey;
    draftSessionKeyRef.current = activeDraftSessionKey;

    const pendingSaveSession =
      pendingSaveSessionsRef.current.get(accountId) ?? null;
    const hasPendingLateSaveSessionMismatch =
      pendingSaveSession != null &&
      pendingSaveSession.accountId === accountId &&
      pendingSaveSession.sessionKey != null &&
      pendingSaveSession.sessionKey !== activeDraftSessionKey &&
      areAccountDraftsEqual(nextBaseline, pendingSaveSession.fallbackDraft);

    const recentSaveResponseGuard =
      recentSaveResponseGuardsRef.current.get(accountId) ?? null;
    const retainedServerDraft =
      recentSaveResponseGuard?.retainedDraft ?? previousLatestServerDraft;
    const hasAcceptedFresherServerDraft =
      recentSaveResponseGuard != null &&
      !areAccountDraftsEqual(
        retainedServerDraft,
        recentSaveResponseGuard.fallbackDraft,
      );
    const shouldIgnoreRecentSaveResponse =
      recentSaveResponseGuard != null &&
      recentSaveResponseGuard.accountId === accountId &&
      recentSaveResponseGuard.sessionKey !== activeDraftSessionKey &&
      (hasAcceptedFresherServerDraft || !shouldSeedDraft) &&
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

    if (hasPendingLateSaveSessionMismatch) {
      if (shouldSeedDraft) {
        setDraft(pendingSaveSession.fallbackDraft);
      }
      return;
    }

    latestServerDraftRef.current = nextBaseline;
    setDraft((current) => {
      const matchesSeedBaseline = areAccountDraftsEqual(
        current,
        previousBaseline,
      );
      const matchesLatestServerDraft = areAccountDraftsEqual(
        current,
        previousLatestServerDraft,
      );
      if (
        shouldSeedDraft ||
        matchesSeedBaseline ||
        matchesLatestServerDraft
      ) {
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
    setDetailTab("overview");
    setExpandedStickyKeys([]);
  }, [accountId]);

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

  useEffect(() => {
    for (const pendingSaveSession of pendingSaveSessionsRef.current.values()) {
      pendingSaveSession.fallbackDraft = filterAccountDraftTagIds(
        pendingSaveSession.fallbackDraft,
        validTagIds,
      );
    }
    for (const recentSaveResponseGuard of recentSaveResponseGuardsRef.current.values()) {
      recentSaveResponseGuard.draft = filterAccountDraftTagIds(
        recentSaveResponseGuard.draft,
        validTagIds,
      );
      recentSaveResponseGuard.fallbackDraft = filterAccountDraftTagIds(
        recentSaveResponseGuard.fallbackDraft,
        validTagIds,
      );
      recentSaveResponseGuard.retainedDraft = filterAccountDraftTagIds(
        recentSaveResponseGuard.retainedDraft,
        validTagIds,
      );
    }
    draftBaselineRef.current = filterAccountDraftTagIds(
      draftBaselineRef.current,
      validTagIds,
    );
    latestServerDraftRef.current = filterAccountDraftTagIds(
      latestServerDraftRef.current,
      validTagIds,
    );
    setDraft((current) => filterAccountDraftTagIds(current, validTagIds));
  }, [validTagIds]);

  const availableGroups = useMemo(() => {
    const draftNames = Object.fromEntries([
      ...Object.keys(groupDraftNotes).map((groupName) => [groupName, ""]),
      ...Object.keys(groupDraftBoundProxyKeys).map((groupName) => [
        groupName,
        "",
      ]),
      ...Object.keys(groupDraftConcurrencyLimits).map((groupName) => [
        groupName,
        "",
      ]),
      ...Object.keys(groupDraftNodeShuntEnabled).map((groupName) => [
        groupName,
        "",
      ]),
      ...Object.keys(groupDraftUpstream429RetryEnabled).map((groupName) => [
        groupName,
        "",
      ]),
      ...Object.keys(groupDraftUpstream429MaxRetries).map((groupName) => [
        groupName,
        "",
      ]),
    ]);
    return {
      names: buildGroupNameSuggestions(
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
    groupDraftNotes,
    groupDraftUpstream429MaxRetries,
    groupDraftUpstream429RetryEnabled,
    groups,
    hasUngroupedAccounts,
    items,
  ]);

  const resolveGroupSummaryForName = useCallback(
    (groupName: string) => {
      const normalized = normalizeGroupName(groupName);
      if (!normalized) return null;
      return (
        groups.find(
          (group) => normalizeGroupName(group.groupName) === normalized,
        ) ?? null
      );
    },
    [groups],
  );

  const resolveGroupNoteForName = useCallback(
    (groupName: string) => resolveGroupNote(groups, groupDraftNotes, groupName),
    [groupDraftNotes, groups],
  );

  const resolveGroupConcurrencyLimitForName = useCallback(
    (groupName: string) =>
      resolveGroupConcurrencyLimit(
        groups,
        groupDraftConcurrencyLimits,
        groupName,
      ),
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
      resolveGroupUpstream429RetryEnabledForName(groupName) ||
      resolveGroupUpstream429MaxRetriesForName(groupName) > 0,
    [
      resolveGroupBoundProxyKeysForName,
      resolveGroupConcurrencyLimitForName,
      resolveGroupNodeShuntEnabledForName,
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

  const persistDraftGroupSettings = useCallback(
    async (groupName: string) => {
      const normalizedGroupName = normalizeGroupName(groupName);
      if (!normalizedGroupName) return;
      const hasDraftNote = normalizedGroupName in groupDraftNotes;
      const hasDraftBindings = normalizedGroupName in groupDraftBoundProxyKeys;
      const hasDraftConcurrency =
        normalizedGroupName in groupDraftConcurrencyLimits;
      const hasDraftNodeShuntEnabled =
        normalizedGroupName in groupDraftNodeShuntEnabled;
      const hasDraftUpstream429RetryEnabled =
        normalizedGroupName in groupDraftUpstream429RetryEnabled;
      const hasDraftUpstream429MaxRetries =
        normalizedGroupName in groupDraftUpstream429MaxRetries;
      if (
        !hasDraftNote &&
        !hasDraftBindings &&
        !hasDraftConcurrency &&
        !hasDraftNodeShuntEnabled &&
        !hasDraftUpstream429RetryEnabled &&
        !hasDraftUpstream429MaxRetries
      )
        return;

      const normalizedNote = hasDraftNote
        ? (groupDraftNotes[normalizedGroupName]?.trim() ?? "")
        : "";
      const normalizedBoundProxyKeys = Array.from(
        new Set(
          (groupDraftBoundProxyKeys[normalizedGroupName] ?? [])
            .map((value) => value.trim())
            .filter((value) => value.length > 0),
        ),
      );
      const normalizedConcurrencyLimit = hasDraftConcurrency
        ? (groupDraftConcurrencyLimits[normalizedGroupName] ?? 0)
        : 0;
      const normalizedNodeShuntEnabled = resolvePersistedGroupNodeShuntEnabled(
        hasDraftNodeShuntEnabled,
        groupDraftNodeShuntEnabled[normalizedGroupName],
        resolveGroupNodeShuntEnabledForName(normalizedGroupName),
      );
      const normalizedUpstream429RetryEnabled = hasDraftUpstream429RetryEnabled
        ? groupDraftUpstream429RetryEnabled[normalizedGroupName] === true
        : false;
      const normalizedUpstream429MaxRetries = normalizedUpstream429RetryEnabled
        ? normalizeEnabledGroupUpstream429MaxRetries(
            groupDraftUpstream429MaxRetries[normalizedGroupName],
          )
        : normalizeGroupUpstream429MaxRetries(
            groupDraftUpstream429MaxRetries[normalizedGroupName],
          );

      await saveGroupNote(normalizedGroupName, {
        note: normalizedNote || undefined,
        boundProxyKeys: normalizedBoundProxyKeys,
        concurrencyLimit: normalizedConcurrencyLimit,
        nodeShuntEnabled: normalizedNodeShuntEnabled,
        upstream429RetryEnabled: normalizedUpstream429RetryEnabled,
        upstream429MaxRetries: normalizedUpstream429MaxRetries,
      });
      clearDraftGroupSettings(normalizedGroupName);
    },
    [
      clearDraftGroupSettings,
      groupDraftBoundProxyKeys,
      groupDraftConcurrencyLimits,
      groupDraftNodeShuntEnabled,
      groupDraftNotes,
      groupDraftUpstream429MaxRetries,
      groupDraftUpstream429RetryEnabled,
      resolveGroupNodeShuntEnabledForName,
      saveGroupNote,
    ],
  );

  const openGroupNoteEditor = useCallback(
    (groupName: string) => {
      if (!writesEnabled) return;
      const normalized = normalizeGroupName(groupName);
      if (!normalized) return;
      const existingGroup = resolveGroupSummaryForName(normalized);
      setGroupNoteError(null);
      setGroupNoteEditor({
        open: true,
        groupName: normalized,
        note: resolveGroupNoteForName(normalized),
        existing: existingGroup != null,
        concurrencyLimit: apiConcurrencyLimitToSliderValue(
          resolveGroupConcurrencyLimitForName(normalized),
        ),
        boundProxyKeys: resolveGroupBoundProxyKeysForName(normalized),
        nodeShuntEnabled: resolveGroupNodeShuntEnabledForName(normalized),
        upstream429RetryEnabled:
          resolveGroupUpstream429RetryEnabledForName(normalized),
        upstream429MaxRetries:
          resolveGroupUpstream429MaxRetriesForName(normalized),
      });
    },
    [
      resolveGroupBoundProxyKeysForName,
      resolveGroupConcurrencyLimitForName,
      resolveGroupNodeShuntEnabledForName,
      resolveGroupNoteForName,
      resolveGroupSummaryForName,
      resolveGroupUpstream429MaxRetriesForName,
      resolveGroupUpstream429RetryEnabledForName,
      writesEnabled,
    ],
  );

  const closeGroupNoteEditor = useCallback(() => {
    if (groupNoteBusy) return;
    setGroupNoteEditor((current) => ({ ...current, open: false }));
    setGroupNoteError(null);
  }, [groupNoteBusy]);

  const handleSaveGroupNote = useCallback(async () => {
    if (!writesEnabled) return;
    const normalizedGroupName = normalizeGroupName(groupNoteEditor.groupName);
    if (!normalizedGroupName) return;
    const normalizedNote = groupNoteEditor.note.trim();
    const normalizedConcurrencyLimit = sliderConcurrencyLimitToApiValue(
      groupNoteEditor.concurrencyLimit,
    );
    const normalizedBoundProxyKeys = Array.from(
      new Set(
        groupNoteEditor.boundProxyKeys
          .map((value) => value.trim())
          .filter((value) => value.length > 0),
      ),
    );
    const normalizedNodeShuntEnabled =
      groupNoteEditor.nodeShuntEnabled === true;
    const normalizedUpstream429RetryEnabled =
      groupNoteEditor.upstream429RetryEnabled === true;
    const normalizedUpstream429MaxRetries = normalizedUpstream429RetryEnabled
      ? normalizeEnabledGroupUpstream429MaxRetries(
          groupNoteEditor.upstream429MaxRetries,
        )
      : normalizeGroupUpstream429MaxRetries(
          groupNoteEditor.upstream429MaxRetries,
        );
    setGroupNoteError(null);

    if (!groupNoteEditor.existing) {
      setGroupDraftNotes((current) => {
        const next = { ...current };
        if (normalizedNote) next[normalizedGroupName] = normalizedNote;
        else delete next[normalizedGroupName];
        return next;
      });
      setGroupDraftBoundProxyKeys((current) => {
        const next = { ...current };
        if (normalizedBoundProxyKeys.length > 0)
          next[normalizedGroupName] = normalizedBoundProxyKeys;
        else delete next[normalizedGroupName];
        return next;
      });
      setGroupDraftNodeShuntEnabled((current) => {
        const next = { ...current };
        if (normalizedNodeShuntEnabled) next[normalizedGroupName] = true;
        else delete next[normalizedGroupName];
        return next;
      });
      setGroupDraftConcurrencyLimits((current) => {
        const next = { ...current };
        if (normalizedConcurrencyLimit > 0)
          next[normalizedGroupName] = normalizedConcurrencyLimit;
        else delete next[normalizedGroupName];
        return next;
      });
      setGroupDraftUpstream429RetryEnabled((current) => {
        const next = { ...current };
        if (
          normalizedUpstream429RetryEnabled ||
          normalizedUpstream429MaxRetries > 0
        ) {
          next[normalizedGroupName] = normalizedUpstream429RetryEnabled;
        } else {
          delete next[normalizedGroupName];
        }
        return next;
      });
      setGroupDraftUpstream429MaxRetries((current) => {
        const next = { ...current };
        if (
          normalizedUpstream429RetryEnabled ||
          normalizedUpstream429MaxRetries > 0
        ) {
          next[normalizedGroupName] = normalizedUpstream429MaxRetries;
        } else {
          delete next[normalizedGroupName];
        }
        return next;
      });
      setGroupNoteEditor((current) => ({ ...current, open: false }));
      return;
    }

    setGroupNoteBusy(true);
    try {
      await saveGroupNote(normalizedGroupName, {
        note: normalizedNote || undefined,
        boundProxyKeys: normalizedBoundProxyKeys,
        concurrencyLimit: normalizedConcurrencyLimit,
        nodeShuntEnabled: normalizedNodeShuntEnabled,
        upstream429RetryEnabled: normalizedUpstream429RetryEnabled,
        upstream429MaxRetries: normalizedUpstream429MaxRetries,
      });
      clearDraftGroupSettings(normalizedGroupName);
      setGroupNoteEditor((current) => ({ ...current, open: false }));
    } catch (err) {
      setGroupNoteError(err instanceof Error ? err.message : String(err));
    } finally {
      setGroupNoteBusy(false);
    }
  }, [
    clearDraftGroupSettings,
    groupNoteEditor.boundProxyKeys,
    groupNoteEditor.concurrencyLimit,
    groupNoteEditor.existing,
    groupNoteEditor.groupName,
    groupNoteEditor.nodeShuntEnabled,
    groupNoteEditor.note,
    groupNoteEditor.upstream429MaxRetries,
    groupNoteEditor.upstream429RetryEnabled,
    saveGroupNote,
    writesEnabled,
  ]);

  const handleCreateTag = useCallback(
    async (payload: Parameters<typeof createTag>[0]) => {
      const detail = await createTag(payload);
      setPageCreatedTagIds((current) =>
        current.includes(detail.id) ? current : [...current, detail.id],
      );
      return detail;
    },
    [createTag],
  );

  const handleDeleteTag = useCallback(
    async (tagId: number) => {
      await deleteTag(tagId);
      setPageCreatedTagIds((current) =>
        current.filter((value) => value !== tagId),
      );
      draftBaselineRef.current = removeAccountDraftTagId(
        draftBaselineRef.current,
        tagId,
      );
      latestServerDraftRef.current = removeAccountDraftTagId(
        latestServerDraftRef.current,
        tagId,
      );
      setDraft((current) => removeAccountDraftTagId(current, tagId));
    },
    [deleteTag],
  );

  const stickyConversationSelection = STICKY_CONVERSATION_SELECTION_LOOKUP.get(
    stickyConversationSelectionValue,
  ) ??
    STICKY_CONVERSATION_SELECTION_LOOKUP.get(
      DEFAULT_STICKY_CONVERSATION_SELECTION_VALUE,
    ) ?? { mode: "count", limit: 50 };
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
  const accountRecordLimitOptions = useMemo(
    () =>
      ACCOUNT_RECORD_LIMIT_OPTIONS.map((value) => ({
        value: String(value),
        label: t("accountPool.upstreamAccounts.records.limitOption", {
          count: value,
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
    Boolean(open && selectedId),
  );
  const visibleStickyKeys = useMemo(
    () =>
      stickyConversationStats?.conversations.map(
        (conversation) => conversation.stickyKey,
      ) ?? [],
    [stickyConversationStats],
  );
  const hasVisibleStickyConversations = visibleStickyKeys.length > 0;
  const allVisibleStickyKeysExpanded =
    hasVisibleStickyConversations &&
    visibleStickyKeys.every((stickyKey) =>
      expandedStickyKeys.includes(stickyKey),
    );

  useEffect(() => {
    if (!stickyConversationStats) return;
    const visibleStickyKeySet = new Set(
      stickyConversationStats.conversations.map(
        (conversation) => conversation.stickyKey,
      ),
    );
    setExpandedStickyKeys((current) => {
      const next = current.filter((stickyKey) =>
        visibleStickyKeySet.has(stickyKey),
      );
      return next.length === current.length ? current : next;
    });
  }, [stickyConversationStats]);

  useEffect(() => {
    const requestSeq = accountRecordsRequestSeqRef.current + 1;
    accountRecordsRequestSeqRef.current = requestSeq;
    setAccountRecords([]);
    setAccountRecordsError(null);
    setAccountRecordsLoading(false);

    if (!open || accountId == null || detailTab !== "records") {
      return;
    }

    setAccountRecordsLoading(true);
    void fetchInvocationRecords({
      upstreamAccountId: accountId,
      page: 1,
      pageSize: accountRecordLimit,
      sortBy: "occurredAt",
      sortOrder: "desc",
    })
      .then((response) => {
        if (requestSeq !== accountRecordsRequestSeqRef.current) return;
        setAccountRecords(response.records);
      })
      .catch((error) => {
        if (requestSeq !== accountRecordsRequestSeqRef.current) return;
        setAccountRecordsError(
          error instanceof Error ? error.message : String(error),
        );
      })
      .finally(() => {
        if (requestSeq === accountRecordsRequestSeqRef.current) {
          setAccountRecordsLoading(false);
        }
      });
  }, [accountId, accountRecordLimit, detailTab, open]);

  const selectedDetail = detail?.id === selectedId ? detail : null;
  const selected = selectedDetail ?? selectedSummary;
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
    typeof selectedId === "number"
      ? (actionError.accountMessages[selectedId] ?? null)
      : null;
  const detailDisplayNameConflict = useMemo(
    () =>
      findDisplayNameConflict(
        items,
        draft.displayName,
        selectedDetail?.id ?? null,
      ),
    [draft.displayName, items, selectedDetail?.id],
  );
  const draftUpstreamBaseUrlError = useMemo(() => {
    const code = validateUpstreamBaseUrl(draft.upstreamBaseUrl);
    if (code === "invalid_absolute_url") {
      return t(
        "accountPool.upstreamAccounts.validation.upstreamBaseUrlInvalid",
      );
    }
    if (code === "query_or_fragment_not_allowed") {
      return t(
        "accountPool.upstreamAccounts.validation.upstreamBaseUrlNoQueryOrFragment",
      );
    }
    return null;
  }, [draft.upstreamBaseUrl, t]);
  const tagFieldLabels = {
    label: t("accountPool.tags.field.label"),
    add: t("accountPool.tags.field.add"),
    empty: t("accountPool.tags.field.empty"),
    searchPlaceholder: t("accountPool.tags.field.searchPlaceholder"),
    searchEmpty: t("accountPool.tags.field.searchEmpty"),
    createInline: (value: string) =>
      t("accountPool.tags.field.createInline", {
        value: value || t("accountPool.tags.field.newTag"),
      }),
    selectedFromCurrentPage: t("accountPool.tags.field.currentPage"),
    remove: t("accountPool.tags.field.remove"),
    deleteAndRemove: t("accountPool.tags.field.deleteAndRemove"),
    edit: t("accountPool.tags.field.edit"),
    createTitle: t("accountPool.tags.dialog.createTitle"),
    editTitle: t("accountPool.tags.dialog.editTitle"),
    dialogDescription: t("accountPool.tags.dialog.description"),
    name: t("accountPool.tags.dialog.name"),
    namePlaceholder: t("accountPool.tags.dialog.namePlaceholder"),
    guardEnabled: t("accountPool.tags.dialog.guardEnabled"),
    lookbackHours: t("accountPool.tags.dialog.lookbackHours"),
    maxConversations: t("accountPool.tags.dialog.maxConversations"),
    allowCutOut: t("accountPool.tags.dialog.allowCutOut"),
    allowCutIn: t("accountPool.tags.dialog.allowCutIn"),
    priorityTier: t("accountPool.tags.dialog.priorityTier"),
    priorityPrimary: t("accountPool.tags.dialog.priorityPrimary"),
    priorityNormal: t("accountPool.tags.dialog.priorityNormal"),
    priorityFallback: t("accountPool.tags.dialog.priorityFallback"),
    fastModeRewriteMode: t("accountPool.tags.dialog.fastModeRewriteMode"),
    fastModeKeepOriginal: t("accountPool.tags.dialog.fastModeKeepOriginal"),
    fastModeFillMissing: t("accountPool.tags.dialog.fastModeFillMissing"),
    fastModeForceAdd: t("accountPool.tags.dialog.fastModeForceAdd"),
    fastModeForceRemove: t("accountPool.tags.dialog.fastModeForceRemove"),
    concurrencyLimit: t("accountPool.tags.dialog.concurrencyLimit"),
    concurrencyHint: t("accountPool.tags.dialog.concurrencyHint"),
    currentValue: t("accountPool.tags.dialog.currentValue"),
    unlimited: t("accountPool.tags.dialog.unlimited"),
    cancel: t("accountPool.tags.dialog.cancel"),
    save: t("accountPool.tags.dialog.save"),
    createAction: t("accountPool.tags.dialog.createAction"),
    validation: t("accountPool.tags.dialog.validation"),
  };

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
  const formatDuplicateReasons = (
    duplicateInfo?: UpstreamAccountDuplicateInfo | null,
  ) => {
    const reasons = duplicateInfo?.reasons ?? [];
    return reasons
      .map((reason) => {
        if (reason === "sharedChatgptAccountId") {
          return t(
            "accountPool.upstreamAccounts.duplicate.reasons.sharedChatgptAccountId",
          );
        }
        if (reason === "sharedChatgptUserId") {
          return t(
            "accountPool.upstreamAccounts.duplicate.reasons.sharedChatgptUserId",
          );
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
      const allExpanded = visibleStickyKeys.every((stickyKey) =>
        current.includes(stickyKey),
      );
      if (allExpanded) {
        return current.filter(
          (stickyKey) => !visibleStickyKeys.includes(stickyKey),
        );
      }

      const preserved = current.filter(
        (stickyKey) => !visibleStickyKeys.includes(stickyKey),
      );
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
      navigate(
        `/account-pool/upstream-accounts/new?accountId=${nextAccountId}`,
      );
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

  const handleSave = useCallback(
    async (source: UpstreamAccountDetail) => {
      if (source.kind === "api_key_codex" && draftUpstreamBaseUrlError) return;
      if (hasBusyAccountAction(busyAction, source.id)) return;
      const saveDraftSessionKey = activeDraftSessionKey;
      const saveStartedDraft = draft;
      const pendingSaveSession = {
        accountId: source.id,
        sessionKey: saveDraftSessionKey,
        fallbackDraft: draftBaselineRef.current,
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
        const pendingGroupNote =
          resolvePendingGroupNoteForName(normalizedGroupName);
        const response = await saveAccount(source.id, {
          displayName: draft.displayName.trim() || undefined,
          groupName: draft.groupName.trim(),
          isMother: draft.isMother,
          note: draft.note.trim() || undefined,
          groupNote: pendingGroupNote || undefined,
          tagIds: draft.tagIds,
          upstreamBaseUrl:
            source.kind === "api_key_codex"
              ? draft.upstreamBaseUrl.trim() || null
              : undefined,
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
            source.kind === "api_key_codex"
              ? draft.localLimitUnit.trim() || undefined
              : undefined,
        });
        let partialWarning: string | null = null;
        try {
          await persistDraftGroupSettings(normalizedGroupName);
        } catch (error) {
          partialWarning = t(
            "accountPool.upstreamAccounts.partialSuccess.savedButGroupSettingsFailed",
            {
              error: error instanceof Error ? error.message : String(error),
            },
          );
        }
        notifyMotherChange(response);
        const responseFallbackDraft = filterAccountDraftTagIds(
          pendingSaveSession.fallbackDraft,
          validTagIdsRef.current,
        );
        const retainedServerDraft = filterAccountDraftTagIds(
          latestServerDraftRef.current,
          validTagIdsRef.current,
        );
        const responseDraft = filterAccountDraftTagIds(
          buildDraft(response),
          validTagIdsRef.current,
        );
        if (
          selectedIdRef.current === source.id &&
          activeDraftSessionKeyRef.current != null
        ) {
          const previousRecentSaveResponseGuard =
            recentSaveResponseGuardsRef.current.get(source.id) ?? null;
          const recentSaveResponseGuard = {
            accountId: source.id,
            sessionKey: saveDraftSessionKey,
            draft: responseDraft,
            fallbackDraft: responseFallbackDraft,
            retainedDraft: retainedServerDraft,
          };
          recentSaveResponseGuardsRef.current.set(
            source.id,
            recentSaveResponseGuard,
          );
        }
        if (
          selectedIdRef.current === source.id &&
          saveDraftSessionKey != null &&
          activeDraftSessionKeyRef.current === saveDraftSessionKey
        ) {
          draftBaselineRef.current = responseDraft;
          latestServerDraftRef.current = responseDraft;
          setDraft((current) =>
            mergeDraftAfterAccountSave(
              current,
              saveStartedDraft,
              responseDraft,
            ),
          );
        }
        if (partialWarning) {
          setActionError((current) => ({
            ...current,
            accountMessages: {
              ...current.accountMessages,
              [source.id]: partialWarning,
            },
          }));
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
        if (
          pendingSaveSessionsRef.current.get(source.id) === pendingSaveSession
        ) {
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
      busyAction,
      draft,
      draftUpstreamBaseUrlError,
      handleNotFoundClose,
      notifyMotherChange,
      persistDraftGroupSettings,
      resolvePendingGroupNoteForName,
      saveAccount,
      t,
    ],
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

  const detailIdentity =
    selected ??
    (accountId != null
      ? { id: accountId, displayName: `#${accountId}` }
      : null);

  return (
    <>
      {open && accountId != null ? (
        <AccountDetailDrawerShell
          open={open}
          labelledBy={detailDrawerTitleId}
          closeLabel={t("accountPool.upstreamAccounts.actions.closeDetails")}
          closeDisabled={isBusyAction(busyAction, "delete", accountId)}
          autoFocusCloseButton={!isDeleteConfirmOpen}
          onPortalContainerChange={setDetailDrawerPortalContainer}
          onClose={() => onClose()}
          shellClassName="max-w-[60rem]"
          header={
            <div className="space-y-4">
              <div className="space-y-3">
                {selected ? (
                  <div className="flex flex-wrap items-center gap-2">
                    <Badge
                      variant={enableStatusVariant(
                        accountEnableStatus(selected),
                      )}
                    >
                      {accountEnableStatusLabel(accountEnableStatus(selected))}
                    </Badge>
                    <Badge
                      variant={workStatusVariant(accountWorkStatus(selected))}
                    >
                      {accountWorkStatusLabel(accountWorkStatus(selected))}
                    </Badge>
                    <Badge
                      variant={syncStateVariant(accountSyncState(selected))}
                    >
                      {accountSyncStateLabel(accountSyncState(selected))}
                    </Badge>
                    <Badge
                      variant={healthStatusVariant(
                        accountHealthStatus(selected),
                      )}
                    >
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
                        {t(
                          "accountPool.upstreamAccounts.apiKey.localPlaceholder",
                        )}
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
                      <MotherAccountBadge
                        label={t("accountPool.upstreamAccounts.mother.badge")}
                      />
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
                      onCheckedChange={(checked) =>
                        void handleToggleEnabled(selected, checked)
                      }
                      disabled={
                        hasBusyAccountAction(busyAction, selected.id) ||
                        !writesEnabled
                      }
                      aria-label={t(
                        "accountPool.upstreamAccounts.actions.enable",
                      )}
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
                      disabled={
                        hasBusyAccountAction(busyAction, selected.id) ||
                        !writesEnabled
                      }
                    >
                      {isBusyAction(busyAction, "relogin", selected.id) ? (
                        <Spinner size="sm" className="mr-2" />
                      ) : (
                        <AppIcon
                          name="login-variant"
                          className="mr-2 h-4 w-4"
                          aria-hidden
                        />
                      )}
                      {t("accountPool.upstreamAccounts.actions.relogin")}
                    </Button>
                  ) : null}
                  <Popover
                    open={isDeleteConfirmOpen}
                    onOpenChange={(nextOpen) => {
                      if (
                        isBusyAction(busyAction, "delete", selected.id) &&
                        !nextOpen
                      )
                        return;
                      setIsDeleteConfirmOpen(nextOpen);
                    }}
                  >
                    <PopoverTrigger asChild>
                      <Button
                        type="button"
                        variant="destructive"
                        disabled={
                          hasBusyAccountAction(busyAction, selected.id) ||
                          !writesEnabled
                        }
                        aria-haspopup="dialog"
                        aria-expanded={isDeleteConfirmOpen}
                        aria-controls={
                          isDeleteConfirmOpen ? deleteConfirmTitleId : undefined
                        }
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
                              {t(
                                "accountPool.upstreamAccounts.deleteConfirmTitle",
                                { name: selected.displayName },
                              )}
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
                                hasBusyAccountAction(busyAction, selected.id) ||
                                !writesEnabled
                              }
                              onClick={() => void handleDelete(selected)}
                            >
                              {t(
                                "accountPool.upstreamAccounts.actions.confirmDelete",
                              )}
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
            <div className="grid gap-5">
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
                className="self-start"
                role="tablist"
                aria-label={t("accountPool.upstreamAccounts.detailTitle")}
              >
                <SegmentedControlItem
                  id={detailTabIds.overview.tab}
                  active={detailTab === "overview"}
                  role="tab"
                  aria-selected={detailTab === "overview"}
                  aria-controls={detailTabIds.overview.panel}
                  aria-pressed={detailTab === "overview"}
                  onClick={() => setDetailTab("overview")}
                >
                  {t("accountPool.upstreamAccounts.detailTabs.overview")}
                </SegmentedControlItem>
                <SegmentedControlItem
                  id={detailTabIds.records.tab}
                  active={detailTab === "records"}
                  role="tab"
                  aria-selected={detailTab === "records"}
                  aria-controls={detailTabIds.records.panel}
                  aria-pressed={detailTab === "records"}
                  onClick={() => setDetailTab("records")}
                >
                  {t("accountPool.upstreamAccounts.detailTabs.records")}
                </SegmentedControlItem>
                <SegmentedControlItem
                  id={detailTabIds.edit.tab}
                  active={detailTab === "edit"}
                  role="tab"
                  aria-selected={detailTab === "edit"}
                  aria-controls={detailTabIds.edit.panel}
                  aria-pressed={detailTab === "edit"}
                  onClick={() => setDetailTab("edit")}
                >
                  {t("accountPool.upstreamAccounts.detailTabs.edit")}
                </SegmentedControlItem>
                <SegmentedControlItem
                  id={detailTabIds.routing.tab}
                  active={detailTab === "routing"}
                  role="tab"
                  aria-selected={detailTab === "routing"}
                  aria-controls={detailTabIds.routing.panel}
                  aria-pressed={detailTab === "routing"}
                  onClick={() => setDetailTab("routing")}
                >
                  {t("accountPool.upstreamAccounts.detailTabs.routing")}
                </SegmentedControlItem>
                <SegmentedControlItem
                  id={detailTabIds.healthEvents.tab}
                  active={detailTab === "healthEvents"}
                  role="tab"
                  aria-selected={detailTab === "healthEvents"}
                  aria-controls={detailTabIds.healthEvents.panel}
                  aria-pressed={detailTab === "healthEvents"}
                  onClick={() => setDetailTab("healthEvents")}
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
                          {t(
                            "accountPool.upstreamAccounts.duplicate.warningBody",
                            {
                              reasons: formatDuplicateReasons(
                                selectedDetail.duplicateInfo,
                              ),
                              peers:
                                selectedDetail.duplicateInfo.peerAccountIds.join(
                                  ", ",
                                ),
                            },
                          )}
                        </p>
                      </div>
                    </Alert>
                  ) : null}
                  <div className="metric-grid">
                    <DetailField
                      label={t("accountPool.upstreamAccounts.fields.groupName")}
                      value={selectedDetail.groupName ?? ""}
                    />
                    <DetailField
                      label={t(
                        "accountPool.upstreamAccounts.mother.fieldLabel",
                      )}
                      value={
                        selectedDetail.isMother
                          ? t("accountPool.upstreamAccounts.mother.badge")
                          : t("accountPool.upstreamAccounts.mother.notMother")
                      }
                    />
                    <DetailField
                      label={t("accountPool.upstreamAccounts.fields.email")}
                      value={selectedDetail.email ?? ""}
                    />
                    <DetailField
                      label={t("accountPool.upstreamAccounts.fields.accountId")}
                      value={
                        selectedDetail.chatgptAccountId ??
                        selectedDetail.maskedApiKey ??
                        ""
                      }
                    />
                    <DetailField
                      label={t("accountPool.upstreamAccounts.fields.userId")}
                      value={selectedDetail.chatgptUserId ?? ""}
                    />
                    <DetailField
                      label={t(
                        "accountPool.upstreamAccounts.fields.lastSuccessSync",
                      )}
                      value={formatDateTime(
                        selectedDetail.lastSuccessfulSyncAt,
                      )}
                    />
                  </div>
                  <div className="grid gap-4 xl:grid-cols-2">
                    <UpstreamAccountUsageCard
                      title={t(
                        "accountPool.upstreamAccounts.primaryWindowLabel",
                      )}
                      description={t(
                        "accountPool.upstreamAccounts.usage.primaryDescription",
                      )}
                      window={selectedDetail.primaryWindow}
                      history={selectedDetail.history}
                      historyKey="primaryUsedPercent"
                      emptyLabel={t("accountPool.upstreamAccounts.noHistory")}
                      noteLabel={
                        selectedDetail.kind === "api_key_codex"
                          ? t(
                              "accountPool.upstreamAccounts.apiKey.localPlaceholder",
                            )
                          : undefined
                      }
                    />
                    <UpstreamAccountUsageCard
                      title={t(
                        "accountPool.upstreamAccounts.secondaryWindowLabel",
                      )}
                      description={t(
                        "accountPool.upstreamAccounts.usage.secondaryDescription",
                      )}
                      window={selectedDetail.secondaryWindow}
                      history={selectedDetail.history}
                      historyKey="secondaryUsedPercent"
                      emptyLabel={t("accountPool.upstreamAccounts.noHistory")}
                      noteLabel={
                        selectedDetail.kind === "api_key_codex"
                          ? t(
                              "accountPool.upstreamAccounts.apiKey.localPlaceholder",
                            )
                          : undefined
                      }
                      accentClassName="text-secondary"
                    />
                  </div>
                </div>
              ) : null}

              {detailTab === "records" ? (
                <div
                  id={detailTabIds.records.panel}
                  role="tabpanel"
                  aria-labelledby={detailTabIds.records.tab}
                >
                  <Card className="border-base-300/80 bg-base-100/72">
                    <CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
                      <div>
                        <CardTitle>
                          {t("accountPool.upstreamAccounts.records.title")}
                        </CardTitle>
                        <CardDescription>
                          {t(
                            "accountPool.upstreamAccounts.records.description",
                          )}
                        </CardDescription>
                      </div>
                      <SelectField
                        label={t(
                          "accountPool.upstreamAccounts.records.limitLabel",
                        )}
                        className="w-36"
                        name="upstreamAccountRecordLimit"
                        size="sm"
                        value={String(accountRecordLimit)}
                        options={accountRecordLimitOptions}
                        onValueChange={(value) => {
                          const nextLimit = Number(value);
                          if (
                            !ACCOUNT_RECORD_LIMIT_OPTIONS.includes(
                              nextLimit as (typeof ACCOUNT_RECORD_LIMIT_OPTIONS)[number],
                            )
                          ) {
                            return;
                          }
                          setAccountRecordLimit(nextLimit);
                        }}
                      />
                    </CardHeader>
                    <CardContent>
                      <InvocationTable
                        records={accountRecords}
                        isLoading={accountRecordsLoading}
                        error={accountRecordsError}
                        emptyLabel={t(
                          "accountPool.upstreamAccounts.records.empty",
                        )}
                        onOpenUpstreamAccount={handleOpenRelatedUpstreamAccount}
                      />
                    </CardContent>
                  </Card>
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
                      <CardTitle>
                        {t("accountPool.upstreamAccounts.editTitle")}
                      </CardTitle>
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
                          {t("accountPool.upstreamAccounts.fields.groupName")}
                        </span>
                        <div className="flex items-center gap-2">
                          <UpstreamAccountGroupCombobox
                            name="detailGroupName"
                            value={draft.groupName}
                            suggestions={availableGroups.names}
                            placeholder={t(
                              "accountPool.upstreamAccounts.fields.groupNamePlaceholder",
                            )}
                            searchPlaceholder={t(
                              "accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder",
                            )}
                            emptyLabel={t(
                              "accountPool.upstreamAccounts.fields.groupNameEmpty",
                            )}
                            createLabel={(value) =>
                              t(
                                "accountPool.upstreamAccounts.fields.groupNameUseValue",
                                { value },
                              )
                            }
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
                            variant={
                              hasGroupSettings(draft.groupName)
                                ? "secondary"
                                : "outline"
                            }
                            className="shrink-0 rounded-full"
                            aria-label={t(
                              "accountPool.upstreamAccounts.groupNotes.actions.edit",
                            )}
                            title={t(
                              "accountPool.upstreamAccounts.groupNotes.actions.edit",
                            )}
                            onClick={() => openGroupNoteEditor(draft.groupName)}
                            disabled={
                              !writesEnabled ||
                              !normalizeGroupName(draft.groupName)
                            }
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
                          label={t(
                            "accountPool.upstreamAccounts.mother.toggleLabel",
                          )}
                          description={t(
                            "accountPool.upstreamAccounts.mother.toggleDescription",
                          )}
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
                      <div className="md:col-span-2">
                        <AccountTagField
                          tags={tagItems}
                          selectedTagIds={draft.tagIds}
                          writesEnabled={writesEnabled}
                          pageCreatedTagIds={pageCreatedTagIds}
                          labels={tagFieldLabels}
                          onChange={(tagIds) =>
                            setDraft((current) => ({ ...current, tagIds }))
                          }
                          onCreateTag={handleCreateTag}
                          onUpdateTag={updateTag}
                          onDeleteTag={handleDeleteTag}
                        />
                      </div>
                      {selectedDetail.kind === "api_key_codex" ? (
                        <>
                          <label className="field">
                            <span className="field-label">
                              {t(
                                "accountPool.upstreamAccounts.fields.primaryLimit",
                              )}
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
                              {t(
                                "accountPool.upstreamAccounts.fields.secondaryLimit",
                              )}
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
                              {t(
                                "accountPool.upstreamAccounts.fields.limitUnit",
                              )}
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
                              label={t(
                                "accountPool.upstreamAccounts.fields.upstreamBaseUrl",
                              )}
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
                                aria-invalid={
                                  draftUpstreamBaseUrlError ? "true" : "false"
                                }
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
                              {t(
                                "accountPool.upstreamAccounts.fields.rotateApiKey",
                              )}
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
                            hasBusyAccountAction(
                              busyAction,
                              selectedDetail.id,
                            ) ||
                            !writesEnabled ||
                            detailDisplayNameConflict != null ||
                            (selectedDetail.kind === "api_key_codex" &&
                              Boolean(draftUpstreamBaseUrlError))
                          }
                        >
                          {isBusyAction(
                            busyAction,
                            "save",
                            selectedDetail.id,
                          ) ? (
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
                  <EffectiveRoutingRuleCard
                    rule={selectedDetail.effectiveRoutingRule}
                    labels={{
                      title: t(
                        "accountPool.upstreamAccounts.effectiveRule.title",
                      ),
                      description: t(
                        "accountPool.upstreamAccounts.effectiveRule.description",
                      ),
                      noTags: t(
                        "accountPool.upstreamAccounts.effectiveRule.noTags",
                      ),
                      guardEnabled: t(
                        "accountPool.upstreamAccounts.effectiveRule.guardEnabled",
                      ),
                      guardDisabled: t(
                        "accountPool.upstreamAccounts.effectiveRule.guardDisabled",
                      ),
                      allowCutOut: t(
                        "accountPool.upstreamAccounts.effectiveRule.allowCutOut",
                      ),
                      denyCutOut: t(
                        "accountPool.upstreamAccounts.effectiveRule.denyCutOut",
                      ),
                      allowCutIn: t(
                        "accountPool.upstreamAccounts.effectiveRule.allowCutIn",
                      ),
                      denyCutIn: t(
                        "accountPool.upstreamAccounts.effectiveRule.denyCutIn",
                      ),
                      sourceTags: t(
                        "accountPool.upstreamAccounts.effectiveRule.sourceTags",
                      ),
                      guardRule: (hours, count) =>
                        t(
                          "accountPool.upstreamAccounts.effectiveRule.guardRule",
                          { hours, count },
                        ),
                      allGuardsApply: t(
                        "accountPool.upstreamAccounts.effectiveRule.allGuardsApply",
                      ),
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
                    }}
                  />

                  <Card className="border-base-300/80 bg-base-100/72">
                    <CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-end sm:justify-between">
                      <div>
                        <CardTitle>
                          {t(
                            "accountPool.upstreamAccounts.stickyConversations.title",
                          )}
                        </CardTitle>
                        <CardDescription>
                          {t(
                            "accountPool.upstreamAccounts.stickyConversations.description",
                          )}
                        </CardDescription>
                      </div>
                      <div className="flex flex-wrap items-center gap-2">
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          className="gap-2"
                          disabled={
                            stickyConversationLoading ||
                            !hasVisibleStickyConversations
                          }
                          onClick={toggleAllVisibleStickyKeys}
                        >
                          <AppIcon
                            name={
                              allVisibleStickyKeysExpanded
                                ? "chevron-up"
                                : "chevron-down"
                            }
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
                            if (
                              !STICKY_CONVERSATION_SELECTION_LOOKUP.has(value)
                            )
                              return;
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
                      <CardTitle>
                        {t("accountPool.upstreamAccounts.healthTitle")}
                      </CardTitle>
                      <CardDescription>
                        {t("accountPool.upstreamAccounts.healthDescription")}
                      </CardDescription>
                    </CardHeader>
                    <CardContent className="grid gap-3 md:grid-cols-2 xl:grid-cols-4">
                      <DetailField
                        label={t(
                          "accountPool.upstreamAccounts.fields.lastSyncedAt",
                        )}
                        value={formatDateTime(selectedDetail.lastSyncedAt)}
                      />
                      <DetailField
                        label={t(
                          "accountPool.upstreamAccounts.fields.lastRefreshedAt",
                        )}
                        value={formatDateTime(selectedDetail.lastRefreshedAt)}
                      />
                      <DetailField
                        label={t(
                          "accountPool.upstreamAccounts.fields.tokenExpiresAt",
                        )}
                        value={formatDateTime(selectedDetail.tokenExpiresAt)}
                      />
                      <DetailField
                        label={t(
                          "accountPool.upstreamAccounts.fields.compactSupport",
                        )}
                        value={
                          selectedDetail.compactSupport?.status === "supported"
                            ? t(
                                "accountPool.upstreamAccounts.compactSupport.status.supported",
                              )
                            : selectedDetail.compactSupport?.status ===
                                "unsupported"
                              ? t(
                                  "accountPool.upstreamAccounts.compactSupport.status.unsupported",
                                )
                              : t(
                                  "accountPool.upstreamAccounts.compactSupport.status.unknown",
                                )
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
                        label={t(
                          "accountPool.upstreamAccounts.fields.compactObservedAt",
                        )}
                        value={formatDateTime(
                          selectedDetail.compactSupport?.observedAt,
                        )}
                      />
                      <DetailField
                        label={t(
                          "accountPool.upstreamAccounts.fields.compactReason",
                        )}
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
                              label={t(
                                "accountPool.upstreamAccounts.latestAction.fields.action",
                              )}
                              value={
                                accountActionLabel(selectedDetail.lastAction) ??
                                t(
                                  "accountPool.upstreamAccounts.latestAction.empty",
                                )
                              }
                            />
                            <DetailField
                              label={t(
                                "accountPool.upstreamAccounts.latestAction.fields.source",
                              )}
                              value={
                                accountActionSourceLabel(
                                  selectedDetail.lastActionSource,
                                ) ??
                                t(
                                  "accountPool.upstreamAccounts.latestAction.unknown",
                                )
                              }
                            />
                            <DetailField
                              label={t(
                                "accountPool.upstreamAccounts.latestAction.fields.reason",
                              )}
                              value={
                                accountActionReasonLabel(
                                  selectedDetail.lastActionReasonCode,
                                ) ??
                                t(
                                  "accountPool.upstreamAccounts.latestAction.unknown",
                                )
                              }
                            />
                            <DetailField
                              label={t(
                                "accountPool.upstreamAccounts.latestAction.fields.httpStatus",
                              )}
                              value={
                                Number.isFinite(
                                  selectedDetail.lastActionHttpStatus ?? NaN,
                                )
                                  ? `HTTP ${selectedDetail.lastActionHttpStatus}`
                                  : t(
                                      "accountPool.upstreamAccounts.unavailable",
                                    )
                              }
                            />
                            <DetailField
                              label={t(
                                "accountPool.upstreamAccounts.latestAction.fields.occurredAt",
                              )}
                              value={formatDateTime(
                                selectedDetail.lastActionAt,
                              )}
                            />
                            <DetailField
                              label={t(
                                "accountPool.upstreamAccounts.latestAction.fields.invokeId",
                              )}
                              value={
                                selectedDetail.lastActionInvokeId ??
                                t("accountPool.upstreamAccounts.unavailable")
                              }
                            />
                            <div className="metric-cell md:col-span-2 xl:col-span-3">
                              <p className="metric-label">
                                {t(
                                  "accountPool.upstreamAccounts.latestAction.fields.message",
                                )}
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
                            {t(
                              "accountPool.upstreamAccounts.latestAction.empty",
                            )}
                          </p>
                        )}
                      </div>
                    </CardContent>
                  </Card>

                  <Card className="border-base-300/80 bg-base-100/72">
                    <CardHeader>
                      <CardTitle>
                        {t("accountPool.upstreamAccounts.recentActions.title")}
                      </CardTitle>
                      <CardDescription>
                        {t(
                          "accountPool.upstreamAccounts.recentActions.description",
                        )}
                      </CardDescription>
                    </CardHeader>
                    <CardContent>
                      {selectedRecentActions.length === 0 ? (
                        <p className="text-sm leading-6 text-base-content/68">
                          {t(
                            "accountPool.upstreamAccounts.recentActions.empty",
                          )}
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
                                    t(
                                      "accountPool.upstreamAccounts.latestAction.unknown",
                                    )}
                                </Badge>
                                <Badge variant="secondary">
                                  {accountActionSourceLabel(
                                    actionEvent.source,
                                  ) ??
                                    t(
                                      "accountPool.upstreamAccounts.latestAction.unknown",
                                    )}
                                </Badge>
                                {actionEvent.reasonCode ? (
                                  <Badge variant="secondary">
                                    {accountActionReasonLabel(
                                      actionEvent.reasonCode,
                                    )}
                                  </Badge>
                                ) : null}
                                {Number.isFinite(
                                  actionEvent.httpStatus ?? NaN,
                                ) ? (
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
                              {actionEvent.invokeId ? (
                                <p className="mt-2 text-xs text-base-content/55">
                                  {t(
                                    "accountPool.upstreamAccounts.latestAction.fields.invokeId",
                                  )}
                                  : {actionEvent.invokeId}
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

      <UpstreamAccountGroupNoteDialog
        open={groupNoteEditor.open}
        container={detailDrawerPortalContainer}
        groupName={groupNoteEditor.groupName}
        note={groupNoteEditor.note}
        concurrencyLimit={groupNoteEditor.concurrencyLimit}
        boundProxyKeys={groupNoteEditor.boundProxyKeys}
        nodeShuntEnabled={groupNoteEditor.nodeShuntEnabled}
        availableProxyNodes={forwardProxyNodes}
        busy={groupNoteBusy}
        error={groupNoteError}
        existing={groupNoteEditor.existing}
        onNoteChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current) => ({ ...current, note: value }));
        }}
        onConcurrencyLimitChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current) => ({
            ...current,
            concurrencyLimit: value,
          }));
        }}
        onBoundProxyKeysChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current) => ({
            ...current,
            boundProxyKeys: value,
          }));
        }}
        onNodeShuntEnabledChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current) => ({
            ...current,
            nodeShuntEnabled: value,
          }));
        }}
        upstream429RetryEnabled={groupNoteEditor.upstream429RetryEnabled}
        upstream429MaxRetries={groupNoteEditor.upstream429MaxRetries}
        onUpstream429RetryEnabledChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current) => ({
            ...current,
            upstream429RetryEnabled: value,
            upstream429MaxRetries: value
              ? normalizeEnabledGroupUpstream429MaxRetries(
                  current.upstream429MaxRetries,
                )
              : normalizeGroupUpstream429MaxRetries(
                  current.upstream429MaxRetries,
                ),
          }));
        }}
        onUpstream429MaxRetriesChange={(value) => {
          setGroupNoteError(null);
          setGroupNoteEditor((current) => ({
            ...current,
            upstream429MaxRetries: current.upstream429RetryEnabled
              ? normalizeEnabledGroupUpstream429MaxRetries(value)
              : normalizeGroupUpstream429MaxRetries(value),
          }));
        }}
        onClose={closeGroupNoteEditor}
        onSave={() => void handleSaveGroupNote()}
        title={t("accountPool.upstreamAccounts.groupNotes.dialogTitle")}
        existingDescription={t(
          "accountPool.upstreamAccounts.groupNotes.existingDescription",
        )}
        draftDescription={t(
          "accountPool.upstreamAccounts.groupNotes.draftDescription",
        )}
        noteLabel={t("accountPool.upstreamAccounts.fields.note")}
        notePlaceholder={t(
          "accountPool.upstreamAccounts.groupNotes.notePlaceholder",
        )}
        concurrencyLimitLabel={t(
          "accountPool.upstreamAccounts.groupNotes.concurrency.label",
        )}
        concurrencyLimitHint={t(
          "accountPool.upstreamAccounts.groupNotes.concurrency.hint",
        )}
        concurrencyLimitCurrentLabel={t(
          "accountPool.upstreamAccounts.groupNotes.concurrency.current",
        )}
        concurrencyLimitUnlimitedLabel={t(
          "accountPool.upstreamAccounts.groupNotes.concurrency.unlimited",
        )}
        cancelLabel={t("accountPool.upstreamAccounts.actions.cancel")}
        saveLabel={t("accountPool.upstreamAccounts.actions.save")}
        closeLabel={t("accountPool.upstreamAccounts.actions.closeDetails")}
        existingBadgeLabel={t(
          "accountPool.upstreamAccounts.groupNotes.badges.existing",
        )}
        draftBadgeLabel={t(
          "accountPool.upstreamAccounts.groupNotes.badges.draft",
        )}
        nodeShuntLabel={t(
          "accountPool.upstreamAccounts.groupNotes.nodeShunt.label",
        )}
        nodeShuntHint={t(
          "accountPool.upstreamAccounts.groupNotes.nodeShunt.hint",
        )}
        nodeShuntToggleLabel={t(
          "accountPool.upstreamAccounts.groupNotes.nodeShunt.toggle",
        )}
        nodeShuntWarning={t(
          "accountPool.upstreamAccounts.groupNotes.nodeShunt.warning",
        )}
        upstream429RetryLabel={t(
          "accountPool.upstreamAccounts.groupNotes.upstream429.label",
        )}
        upstream429RetryHint={t(
          "accountPool.upstreamAccounts.groupNotes.upstream429.hint",
        )}
        upstream429RetryToggleLabel={t(
          "accountPool.upstreamAccounts.groupNotes.upstream429.toggle",
        )}
        upstream429RetryCountLabel={t(
          "accountPool.upstreamAccounts.groupNotes.upstream429.countLabel",
        )}
        upstream429RetryCountOptions={GROUP_UPSTREAM_429_RETRY_OPTIONS.map(
          (value) => ({
            value,
            label:
              value === 1
                ? t(
                    "accountPool.upstreamAccounts.groupNotes.upstream429.countOnce",
                  )
                : t(
                    "accountPool.upstreamAccounts.groupNotes.upstream429.countMany",
                    { count: value },
                  ),
          }),
        )}
        proxyBindingsLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.label",
        )}
        proxyBindingsHint={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.hint",
        )}
        proxyBindingsAutomaticLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.automatic",
        )}
        proxyBindingsEmptyLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.empty",
        )}
        proxyBindingsMissingLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.missing",
        )}
        proxyBindingsUnavailableLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.unavailable",
        )}
        proxyBindingsChartLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartLabel",
        )}
        proxyBindingsChartSuccessLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartSuccess",
        )}
        proxyBindingsChartFailureLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartFailure",
        )}
        proxyBindingsChartEmptyLabel={t(
          "accountPool.upstreamAccounts.groupNotes.proxyBindings.chartEmpty",
        )}
        proxyBindingsChartTotalLabel={t(
          "live.proxy.table.requestTooltip.total",
        )}
        proxyBindingsChartAriaLabel={t("live.proxy.table.requestTrendAria")}
        proxyBindingsChartInteractionHint={t("live.chart.tooltip.instructions")}
        proxyBindingsChartLocaleTag={locale === "zh" ? "zh-CN" : "en-US"}
      />
    </>
  );
}
