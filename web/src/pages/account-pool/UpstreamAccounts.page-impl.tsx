/* eslint-disable react-hooks/exhaustive-deps */
import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { AppIcon } from "../../components/AppIcon";
import { Link, useLocation, useNavigate } from "react-router-dom";
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
import { formFieldSpanVariants } from "../../components/ui/form-control";
import { SelectField } from "../../components/ui/select-field";
import { SegmentedControl, SegmentedControlItem } from "../../components/ui/segmented-control";
import { Spinner } from "../../components/ui/spinner";
import { AccountTagFilterCombobox } from "../../components/AccountTagFilterCombobox";
import { MultiSelectFilterCombobox } from "../../components/MultiSelectFilterCombobox";
import { UpstreamAccountGroupCombobox } from "../../components/UpstreamAccountGroupCombobox";
import { UpstreamAccountsGroupedRoster, type UpstreamAccountsGroupedRosterGroup } from "../../components/UpstreamAccountsGroupedRoster";
import { UpstreamAccountsTable, type UpstreamAccountsTableLabels } from "../../components/UpstreamAccountsTable";
import { usePoolTags } from "../../hooks/usePoolTags";
import { useUpstreamAccountDetailRoute } from "../../hooks/useUpstreamAccountDetailRoute";
import { useUpstreamAccounts } from "../../hooks/useUpstreamAccounts";
import type {
  BulkUpstreamAccountActionPayload,
  BulkUpstreamAccountSyncCounts,
  BulkUpstreamAccountSyncSnapshot,
  PoolRoutingMaintenanceSettings,
  FetchUpstreamAccountsQuery,
  PoolRoutingTimeoutSettings,
  UpstreamAccountDuplicateInfo,
  UpstreamAccountSummary,
} from "../../lib/api";
import {
  createBulkUpstreamAccountSyncJobEventSource,
  normalizeBulkUpstreamAccountSyncFailedEventPayload,
  normalizeBulkUpstreamAccountSyncRowEventPayload,
  normalizeBulkUpstreamAccountSyncSnapshotEventPayload,
} from "../../lib/api";
import {
  buildGroupNameSuggestions,
} from "../../lib/upstreamAccountGroups";
import { generatePoolRoutingKey } from "../../lib/poolRouting";
import { cn } from "../../lib/utils";
import { useTranslation } from "../../i18n";
import {
  DEFAULT_ROUTING_TIMEOUTS,
  UPSTREAM_ACCOUNTS_QUERY_STALE_GRACE_MS,
  type GroupFilterState,
  type UpstreamAccountsLocationState,
  type ActionErrorState,
  type BusyActionState,
  readPersistedUpstreamAccountFilters,
  persistUpstreamAccountFilters,
  formatGroupFilterValue,
  parseGroupFilterValue,
  isBusyAction,
  resolveRoutingMaintenance,
  buildRoutingDraft,
  accountWorkStatus,
  accountHealthStatus,
  parseRoutingPositiveInteger,
  bulkSyncRowStatusVariant,
  resolveBulkSyncCounts,
  withBulkSyncSnapshotStatus,
  shouldAutoHideBulkSyncProgress,
  compactSupportLabel,
  compactSupportHint,
  parseRoutingTimeoutValue,
  poolCardMetric,
  RoutingSettingsDialog,
  SharedUpstreamAccountDetailDrawer,
} from "./UpstreamAccounts.page-local-shared";
export { SharedUpstreamAccountDetailDrawer } from "./UpstreamAccounts.page-local-shared";

type AccountRosterViewMode = "flat" | "grouped" | "grid";

function normalizeRosterGroupName(value?: string | null) {
  const normalized = value?.trim();
  return normalized ? normalized : null;
}

function parseListQueryIncludeAll(queryKey: string | null): boolean | null {
  if (!queryKey) return null;
  try {
    const parsed = JSON.parse(queryKey) as { includeAll?: boolean };
    return parsed.includeAll === true;
  } catch {
    return null;
  }
}

export default function UpstreamAccountsPage() {
  const { t } = useTranslation();
  const location = useLocation();
  const navigate = useNavigate();
  const { upstreamAccountId, openUpstreamAccount, closeUpstreamAccount } =
    useUpstreamAccountDetailRoute();
  const [initialFilters] = useState(() =>
    readPersistedUpstreamAccountFilters(),
  );
  const [groupFilter, setGroupFilter] = useState<GroupFilterState>(
    () => initialFilters.groupFilter,
  );
  const [selectedTagIds, setSelectedTagIds] = useState<number[]>(
    () => initialFilters.tagIds,
  );
  const [workStatusFilter, setWorkStatusFilter] = useState<string[]>(
    () => initialFilters.workStatus,
  );
  const [enableStatusFilter, setEnableStatusFilter] = useState<string[]>(
    () => initialFilters.enableStatus,
  );
  const [healthStatusFilter, setHealthStatusFilter] = useState<string[]>(
    () => initialFilters.healthStatus,
  );
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(20);
  const [rosterViewMode, setRosterViewMode] = useState<AccountRosterViewMode>("flat");
  const [selectedAccountIds, setSelectedAccountIds] = useState<number[]>([]);
  const [selectedAccountSummaries, setSelectedAccountSummaries] = useState<
    Record<number, UpstreamAccountSummary>
  >({});
  const {
    items: tagItems,
    isLoading: isTagCatalogLoading,
    error: tagCatalogError,
  } = usePoolTags();
  const groupFilterLabels = useMemo(
    () => ({
      all: t("accountPool.upstreamAccounts.groupFilter.all"),
      ungrouped: t("accountPool.upstreamAccounts.groupFilter.ungrouped"),
    }),
    [t],
  );
  const groupFilterQuery = useMemo(
    () => formatGroupFilterValue(groupFilter, groupFilterLabels),
    [groupFilter, groupFilterLabels],
  );
  const validTagIds = useMemo(
    () => new Set(tagItems.map((tag) => tag.id)),
    [tagItems],
  );
  const visibleSelectedTagIds = useMemo(
    () => selectedTagIds.filter((tagId) => validTagIds.has(tagId)),
    [selectedTagIds, validTagIds],
  );
  const canSanitizeSelectedTagIds =
    !isTagCatalogLoading && tagCatalogError == null;
  const canApplySelectedTagIds =
    !isTagCatalogLoading &&
    (tagCatalogError == null ||
      visibleSelectedTagIds.length === selectedTagIds.length);
  const shouldDeferRosterQuery =
    selectedTagIds.length > 0 && isTagCatalogLoading;
  const appliedSelectedTagIds = useMemo(() => {
    if (!canApplySelectedTagIds) {
      return [];
    }
    return visibleSelectedTagIds;
  }, [canApplySelectedTagIds, visibleSelectedTagIds]);
  const persistedSelectedTagIds = useMemo(() => {
    if (!canSanitizeSelectedTagIds) {
      return selectedTagIds;
    }
    return visibleSelectedTagIds;
  }, [canSanitizeSelectedTagIds, selectedTagIds, visibleSelectedTagIds]);
  const accountListQuery = useMemo<FetchUpstreamAccountsQuery | null>(() => {
    if (shouldDeferRosterQuery) {
      return null;
    }
    return {
      groupSearch:
        groupFilter.mode === "search" ? groupFilter.query : undefined,
      groupUngrouped: groupFilter.mode === "ungrouped" ? true : undefined,
      workStatus: workStatusFilter.length > 0 ? workStatusFilter : undefined,
      enableStatus:
        enableStatusFilter.length > 0 ? enableStatusFilter : undefined,
      healthStatus:
        healthStatusFilter.length > 0 ? healthStatusFilter : undefined,
      ...(rosterViewMode === "flat"
        ? { page, pageSize }
        : { includeAll: true }),
      tagIds:
        appliedSelectedTagIds.length > 0 ? appliedSelectedTagIds : undefined,
    };
  }, [
    appliedSelectedTagIds,
    enableStatusFilter,
    groupFilter,
    healthStatusFilter,
    page,
    pageSize,
    rosterViewMode,
    shouldDeferRosterQuery,
    workStatusFilter,
  ]);
  const workStatusFilterOptions = useMemo(
    () => [
      {
        value: "working",
        label: t("accountPool.upstreamAccounts.workStatus.working"),
      },
      {
        value: "degraded",
        label: t("accountPool.upstreamAccounts.workStatus.degraded"),
      },
      {
        value: "idle",
        label: t("accountPool.upstreamAccounts.workStatus.idle"),
      },
      {
        value: "rate_limited",
        label: t("accountPool.upstreamAccounts.workStatus.rate_limited"),
      },
      {
        value: "unavailable",
        label: t("accountPool.upstreamAccounts.workStatus.unavailable"),
      },
    ],
    [t],
  );
  const enableStatusFilterOptions = useMemo(
    () => [
      {
        value: "enabled",
        label: t("accountPool.upstreamAccounts.enableStatus.enabled"),
      },
      {
        value: "disabled",
        label: t("accountPool.upstreamAccounts.enableStatus.disabled"),
      },
    ],
    [t],
  );
  const healthStatusFilterOptions = useMemo(
    () => [
      {
        value: "normal",
        label: t("accountPool.upstreamAccounts.healthStatus.normal"),
      },
      {
        value: "needs_reauth",
        label: t("accountPool.upstreamAccounts.healthStatus.needs_reauth"),
      },
      {
        value: "upstream_unavailable",
        label: t(
          "accountPool.upstreamAccounts.healthStatus.upstream_unavailable",
        ),
      },
      {
        value: "upstream_rejected",
        label: t("accountPool.upstreamAccounts.healthStatus.upstream_rejected"),
      },
      {
        value: "error_other",
        label: t("accountPool.upstreamAccounts.healthStatus.error_other"),
      },
    ],
    [t],
  );
  const pageSizeOptions = useMemo(
    () =>
      [20, 50, 100].map((value) => ({
        value: String(value),
        label: String(value),
      })),
    [],
  );
  const {
    items,
    groups = [],
    hasUngroupedAccounts = false,
    writesEnabled,
    isLoading,
    listError = null,
    listState = {
      queryKey: null,
      dataQueryKey: null,
      freshness: "fresh",
      loadingState: "idle",
      status: "ready",
      hasCurrentQueryData: true,
      isPending: false,
    },
    refresh,
    routing,
    saveRouting,
    runBulkAction,
    startBulkSyncJob,
    getBulkSyncJob,
    stopBulkSyncJob,
    forwardProxyNodes = [],
    total,
    metrics: listMetrics,
  } = useUpstreamAccounts(accountListQuery);
  const [routingDraft, setRoutingDraft] = useState(() =>
    buildRoutingDraft(null),
  );
  const [actionError, setActionError] = useState<ActionErrorState>(() => ({
    routing: null,
    accountMessages: {},
  }));
  const [busyAction, setBusyAction] = useState<BusyActionState>(() => ({
    routing: false,
    accountActions: new Set(),
  }));
  const [isRoutingDialogOpen, setIsRoutingDialogOpen] = useState(false);
  const [isRoutingDialogInspectOnly, setIsRoutingDialogInspectOnly] =
    useState(false);
  const [postCreateWarning, setPostCreateWarning] = useState<string | null>(
    null,
  );
  const [duplicateWarning, setDuplicateWarning] =
    useState<UpstreamAccountsLocationState["duplicateWarning"]>(null);
  const [pendingInitialDeleteConfirm, setPendingInitialDeleteConfirm] =
    useState(false);
  const [bulkActionBusy, setBulkActionBusy] = useState<string | null>(null);
  const [bulkActionMessage, setBulkActionMessage] = useState<string | null>(
    null,
  );
  const [bulkActionError, setBulkActionError] = useState<string | null>(null);
  const [bulkGroupDialogOpen, setBulkGroupDialogOpen] = useState(false);
  const [bulkGroupName, setBulkGroupName] = useState("");
  const [bulkTagsDialogOpen, setBulkTagsDialogOpen] = useState(false);
  const [bulkTagMode, setBulkTagMode] = useState<"add_tags" | "remove_tags">(
    "add_tags",
  );
  const [bulkTagIds, setBulkTagIds] = useState<number[]>([]);
  const [bulkDeleteDialogOpen, setBulkDeleteDialogOpen] = useState(false);
  const [bulkSyncSnapshot, setBulkSyncSnapshot] =
    useState<BulkUpstreamAccountSyncSnapshot | null>(null);
  const [bulkSyncCounts, setBulkSyncCounts] =
    useState<BulkUpstreamAccountSyncCounts | null>(null);
  const [bulkSyncError, setBulkSyncError] = useState<string | null>(null);
  const [isBulkSyncStarting, setIsBulkSyncStarting] = useState(false);
  const [isStaleRosterGraceExpired, setIsStaleRosterGraceExpired] =
    useState(false);
  const bulkSyncEventSourceRef = useRef<EventSource | null>(null);
  const rosterRegionRef = useRef<HTMLDivElement | null>(null);
  const [lastStableRosterRegionHeight, setLastStableRosterRegionHeight] =
    useState<number | null>(null);
  const selectedAccountIdSet = useMemo(
    () => new Set(selectedAccountIds),
    [selectedAccountIds],
  );
  const routingWritesEnabled = routing
    ? (routing.writesEnabled ?? writesEnabled)
    : false;
  const currentRosterViewRequiresIncludeAll = rosterViewMode !== "flat";
  const activeDataQueryIncludeAll = useMemo(
    () => parseListQueryIncludeAll(listState.dataQueryKey),
    [listState.dataQueryKey],
  );
  const shouldBlockSwitchingRoster =
    listState.loadingState === "switching" &&
    activeDataQueryIncludeAll != null &&
    activeDataQueryIncludeAll !== currentRosterViewRequiresIncludeAll;
  const showGraceRoster =
    listState.loadingState === "switching" &&
    !isStaleRosterGraceExpired &&
    !shouldBlockSwitchingRoster;
  const showBlockingRosterLoading =
    listState.loadingState === "initial" ||
    shouldBlockSwitchingRoster ||
    (listState.loadingState === "switching" && isStaleRosterGraceExpired);
  const showBlockingRosterError = listState.status === "error";
  const showBlockingRosterState =
    showBlockingRosterLoading || showBlockingRosterError;
  const visibleRosterItems = showBlockingRosterLoading
    ? shouldBlockSwitchingRoster
      ? []
      : items
    : showBlockingRosterError && !showGraceRoster
      ? []
      : items;
  const visibleListWarning =
    listState.hasCurrentQueryData && listError ? listError : null;
  const hideRosterDerivedUi = showBlockingRosterState && !showGraceRoster;
  const effectiveMetrics = listMetrics ?? {
    total: items.length,
    oauth: items.filter((item) => item.kind === "oauth_codex").length,
    apiKey: items.filter((item) => item.kind === "api_key_codex").length,
    attention: items.filter(
      (item) =>
        accountHealthStatus(item) !== "normal" ||
        accountWorkStatus(item) === "rate_limited",
    ).length,
  };
  const visibleMetrics = hideRosterDerivedUi ? null : effectiveMetrics;
  const effectiveTotal = hideRosterDerivedUi
    ? null
    : (total ?? effectiveMetrics.total);
  const pageCount =
    effectiveTotal == null
      ? null
      : Math.max(1, Math.ceil(effectiveTotal / Math.max(pageSize, 1)));
  const nextPageLimit = pageCount ?? page;
  const showPaginationFooter =
    rosterViewMode === "flat" && (pageCount != null || showBlockingRosterState);
  const paginationStatusText = showBlockingRosterError
    ? t("accountPool.upstreamAccounts.pagination.error")
    : null;
  const rosterRegionMinHeight =
    showBlockingRosterLoading && lastStableRosterRegionHeight != null
      ? `${lastStableRosterRegionHeight}px`
      : undefined;
  const clearBulkSelection = useCallback(() => {
    setSelectedAccountIds([]);
    setBulkGroupDialogOpen(false);
    setBulkTagsDialogOpen(false);
    setBulkDeleteDialogOpen(false);
  }, []);
  const handleGroupFilterChange = useCallback(
    (value: string) => {
      setGroupFilter(parseGroupFilterValue(value, groupFilterLabels));
      setPage(1);
      clearBulkSelection();
    },
    [clearBulkSelection, groupFilterLabels],
  );
  const handleTagFilterChange = useCallback(
    (value: number[]) => {
      setSelectedTagIds(value);
      setPage(1);
      clearBulkSelection();
    },
    [clearBulkSelection],
  );
  const handleWorkStatusFilterChange = useCallback(
    (value: string[]) => {
      setWorkStatusFilter(value);
      setPage(1);
      clearBulkSelection();
    },
    [clearBulkSelection],
  );
  const handleEnableStatusFilterChange = useCallback(
    (value: string[]) => {
      setEnableStatusFilter(value);
      setPage(1);
      clearBulkSelection();
    },
    [clearBulkSelection],
  );
  const handleHealthStatusFilterChange = useCallback(
    (value: string[]) => {
      setHealthStatusFilter(value);
      setPage(1);
      clearBulkSelection();
    },
    [clearBulkSelection],
  );

  const handlePageSizeChange = useCallback(
    (value: number) => {
      setPageSize(value);
      setPage(1);
      clearBulkSelection();
    },
    [clearBulkSelection],
  );

  const handleOpenRoutingDialog = useCallback(() => {
    setRoutingDraft(buildRoutingDraft(routing));
    setIsRoutingDialogInspectOnly(!routingWritesEnabled);
    setIsRoutingDialogOpen(true);
  }, [routing, routingWritesEnabled]);

  const closeBulkSyncEventSource = useCallback(() => {
    bulkSyncEventSourceRef.current?.close();
    bulkSyncEventSourceRef.current = null;
  }, []);

  const clearBulkSyncProgress = useCallback(() => {
    setBulkSyncSnapshot(null);
    setBulkSyncCounts(null);
    setBulkSyncError(null);
  }, []);

  useEffect(() => {
    if (isRoutingDialogOpen && !routing) {
      setRoutingDraft(buildRoutingDraft(null));
      setIsRoutingDialogInspectOnly(false);
      setIsRoutingDialogOpen(false);
      return;
    }
    if (isRoutingDialogOpen) {
      if (!routingWritesEnabled) {
        setRoutingDraft(buildRoutingDraft(routing));
        setIsRoutingDialogInspectOnly(true);
        return;
      }
      if (isRoutingDialogInspectOnly) {
        setRoutingDraft(buildRoutingDraft(routing));
        setIsRoutingDialogInspectOnly(false);
      }
      return;
    }
    setRoutingDraft(buildRoutingDraft(routing));
  }, [
    isRoutingDialogOpen,
    isRoutingDialogInspectOnly,
    routingWritesEnabled,
    routing,
    routing?.maskedApiKey,
    routing?.writesEnabled,
    routing?.maintenance?.primarySyncIntervalSecs,
    routing?.maintenance?.secondarySyncIntervalSecs,
    routing?.maintenance?.priorityAvailableAccountCap,
    routing?.timeouts?.responsesFirstByteTimeoutSecs,
    routing?.timeouts?.compactFirstByteTimeoutSecs,
    routing?.timeouts?.responsesStreamTimeoutSecs,
    routing?.timeouts?.compactStreamTimeoutSecs,
  ]);

  useEffect(() => {
    setSelectedAccountSummaries((current) => {
      const currentPageMap = new Map(items.map((item) => [item.id, item]));
      const next: Record<number, UpstreamAccountSummary> = {};
      for (const accountId of selectedAccountIds) {
        const summary = currentPageMap.get(accountId) ?? current[accountId];
        if (summary) {
          next[accountId] = summary;
        }
      }
      const currentKeys = Object.keys(current);
      const nextKeys = Object.keys(next);
      if (
        currentKeys.length === nextKeys.length &&
        nextKeys.every((key) => current[Number(key)] === next[Number(key)])
      ) {
        return current;
      }
      return next;
    });
  }, [items, selectedAccountIds]);

  useEffect(() => {
    if (!canSanitizeSelectedTagIds) {
      return;
    }
    setSelectedTagIds((current) => {
      const next = current.filter((tagId) => validTagIds.has(tagId));
      return next.length === current.length ? current : next;
    });
  }, [canSanitizeSelectedTagIds, validTagIds]);

  useEffect(() => {
    persistUpstreamAccountFilters({
      workStatus: workStatusFilter,
      enableStatus: enableStatusFilter,
      healthStatus: healthStatusFilter,
      tagIds: persistedSelectedTagIds,
      groupFilter,
    });
  }, [
    enableStatusFilter,
    groupFilter,
    healthStatusFilter,
    persistedSelectedTagIds,
    workStatusFilter,
  ]);

  useEffect(() => {
    return () => {
      closeBulkSyncEventSource();
    };
  }, [closeBulkSyncEventSource]);

  useEffect(() => {
    if (listState.loadingState !== "switching") {
      setIsStaleRosterGraceExpired(false);
      return;
    }

    setIsStaleRosterGraceExpired(false);
    const timer = window.setTimeout(() => {
      setIsStaleRosterGraceExpired(true);
    }, UPSTREAM_ACCOUNTS_QUERY_STALE_GRACE_MS);

    return () => window.clearTimeout(timer);
  }, [listState.dataQueryKey, listState.loadingState, listState.queryKey]);

  useLayoutEffect(() => {
    if (hideRosterDerivedUi) return;
    const region = rosterRegionRef.current;
    if (!region) return;
    const nextHeight = Math.ceil(region.getBoundingClientRect().height);
    if (!(nextHeight > 0)) return;
    setLastStableRosterRegionHeight((current) =>
      current === nextHeight ? current : nextHeight,
    );
  }, [
    bulkActionError,
    bulkActionMessage,
    hideRosterDerivedUi,
    page,
    pageCount,
    pageSize,
    selectedAccountIds.length,
    visibleListWarning,
    visibleRosterItems,
  ]);

  useEffect(() => {
    if (
      rosterViewMode === "flat" &&
      effectiveTotal != null &&
      pageCount != null &&
      effectiveTotal > 0 &&
      page > pageCount
    ) {
      setPage(pageCount);
    }
  }, [effectiveTotal, page, pageCount, rosterViewMode]);

  useEffect(() => {
    const state = location.state as UpstreamAccountsLocationState | null;
    if (!state) return;

    const nextSearchParams = new URLSearchParams(location.search);
    if (typeof state.selectedAccountId === "number" && state.openDetail) {
      nextSearchParams.set(
        "upstreamAccountId",
        String(state.selectedAccountId),
      );
      openUpstreamAccount(state.selectedAccountId, { replace: true });
    }
    setPostCreateWarning(state.postCreateWarning ?? null);
    setDuplicateWarning(state.duplicateWarning ?? null);
    if (state.openDeleteConfirm) {
      setPendingInitialDeleteConfirm(true);
    }
    navigate(
      {
        pathname: location.pathname,
        search: nextSearchParams.toString()
          ? `?${nextSearchParams.toString()}`
          : "",
      },
      { replace: true, state: null },
    );
  }, [
    location.pathname,
    location.search,
    location.state,
    navigate,
    openUpstreamAccount,
  ]);

  useEffect(() => {
    if (!duplicateWarning) return;
    if (
      upstreamAccountId == null ||
      duplicateWarning.accountId === upstreamAccountId
    )
      return;
    setDuplicateWarning(null);
  }, [duplicateWarning, upstreamAccountId]);

  const metrics = useMemo(() => {
    const metricValues = visibleMetrics ?? {
      total: showBlockingRosterError ? "—" : "…",
      oauth: showBlockingRosterError ? "—" : "…",
      apiKey: showBlockingRosterError ? "—" : "…",
      attention: showBlockingRosterError ? "—" : "…",
    };
    return [
      poolCardMetric(
        metricValues.total,
        t("accountPool.upstreamAccounts.metrics.total"),
        "database-outline",
        "text-primary",
      ),
      poolCardMetric(
        metricValues.oauth,
        t("accountPool.upstreamAccounts.metrics.oauth"),
        "badge-account-horizontal-outline",
        "text-success",
      ),
      poolCardMetric(
        metricValues.apiKey,
        t("accountPool.upstreamAccounts.metrics.apiKey"),
        "key-outline",
        "text-info",
      ),
      poolCardMetric(
        metricValues.attention,
        t("accountPool.upstreamAccounts.metrics.attention"),
        "alert-decagram-outline",
        "text-warning",
      ),
    ];
  }, [showBlockingRosterError, t, visibleMetrics]);

  const availableGroups = useMemo(() => {
    return {
      names: buildGroupNameSuggestions(
        visibleRosterItems.map((item) => item.groupName),
        groups,
        {},
      ),
      hasUngrouped: hasUngroupedAccounts,
    };
  }, [groups, hasUngroupedAccounts, visibleRosterItems]);

  const groupFilterSuggestions = useMemo(() => {
    const suggestions = [
      t("accountPool.upstreamAccounts.groupFilter.all"),
      ...availableGroups.names,
    ];
    if (availableGroups.hasUngrouped) {
      suggestions.push(t("accountPool.upstreamAccounts.groupFilter.ungrouped"));
    }
    return suggestions;
  }, [availableGroups, t]);

  const visibleRoutingError = actionError.routing;
  const resolvedRoutingMaintenance = useMemo(
    () => resolveRoutingMaintenance(routing?.maintenance),
    [routing?.maintenance],
  );
  const parsedRoutingMaintenance = useMemo(() => {
    const primarySyncIntervalSecs = parseRoutingPositiveInteger(
      routingDraft.primarySyncIntervalSecs,
    );
    const secondarySyncIntervalSecs = parseRoutingPositiveInteger(
      routingDraft.secondarySyncIntervalSecs,
    );
    const priorityAvailableAccountCap = parseRoutingPositiveInteger(
      routingDraft.priorityAvailableAccountCap,
    );
    if (
      primarySyncIntervalSecs == null ||
      secondarySyncIntervalSecs == null ||
      priorityAvailableAccountCap == null
    ) {
      return null;
    }
    return {
      primarySyncIntervalSecs,
      secondarySyncIntervalSecs,
      priorityAvailableAccountCap,
    };
  }, [
    routingDraft.primarySyncIntervalSecs,
    routingDraft.secondarySyncIntervalSecs,
    routingDraft.priorityAvailableAccountCap,
  ]);
  const routingDraftValidationError = useMemo(() => {
    if (parsedRoutingMaintenance == null) {
      return t(
        "accountPool.upstreamAccounts.routing.validation.integerRequired",
      );
    }
    if (parsedRoutingMaintenance.primarySyncIntervalSecs < 60) {
      return t("accountPool.upstreamAccounts.routing.validation.primaryMin");
    }
    if (parsedRoutingMaintenance.secondarySyncIntervalSecs < 60) {
      return t("accountPool.upstreamAccounts.routing.validation.secondaryMin");
    }
    if (
      parsedRoutingMaintenance.secondarySyncIntervalSecs <
      parsedRoutingMaintenance.primarySyncIntervalSecs
    ) {
      return t(
        "accountPool.upstreamAccounts.routing.validation.secondaryAtLeastPrimary",
      );
    }
    if (parsedRoutingMaintenance.priorityAvailableAccountCap < 1) {
      return t(
        "accountPool.upstreamAccounts.routing.validation.priorityCapMin",
      );
    }
    return null;
  }, [parsedRoutingMaintenance, t]);
  const routingHasApiKeyChange = routingDraft.apiKey.trim().length > 0;
  const routingHasMaintenanceChange =
    parsedRoutingMaintenance != null &&
    (parsedRoutingMaintenance.primarySyncIntervalSecs !==
      resolvedRoutingMaintenance.primarySyncIntervalSecs ||
      parsedRoutingMaintenance.secondarySyncIntervalSecs !==
        resolvedRoutingMaintenance.secondarySyncIntervalSecs ||
      parsedRoutingMaintenance.priorityAvailableAccountCap !==
        resolvedRoutingMaintenance.priorityAvailableAccountCap);
  const resolvedRoutingTimeouts = routing?.timeouts ?? DEFAULT_ROUTING_TIMEOUTS;
  const routingHasTimeoutChange =
    routingDraft.responsesFirstByteTimeoutSecs.trim() !==
      String(resolvedRoutingTimeouts.responsesFirstByteTimeoutSecs) ||
    routingDraft.compactFirstByteTimeoutSecs.trim() !==
      String(resolvedRoutingTimeouts.compactFirstByteTimeoutSecs) ||
    routingDraft.responsesStreamTimeoutSecs.trim() !==
      String(resolvedRoutingTimeouts.responsesStreamTimeoutSecs) ||
    routingDraft.compactStreamTimeoutSecs.trim() !==
      String(resolvedRoutingTimeouts.compactStreamTimeoutSecs);
  const routingDialogCanEdit =
    routingWritesEnabled && !isRoutingDialogInspectOnly;
  const routingCanSave =
    routingDialogCanEdit &&
    !routingDraftValidationError &&
    (routingHasMaintenanceChange ||
      routingHasTimeoutChange ||
      routingHasApiKeyChange);
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
  const accountEnableStatusLabel = (status: string) =>
    t(`accountPool.upstreamAccounts.enableStatus.${status}`);
  const accountWorkStatusLabel = (status: string) =>
    t(`accountPool.upstreamAccounts.workStatus.${status}`);
  const accountWorkingCountLabel = (count: number) =>
    t("accountPool.upstreamAccounts.workStatus.workingWithCount", { count });
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
  const accountRosterLabels = useMemo<UpstreamAccountsTableLabels>(
    () => ({
      selectPage: t("accountPool.upstreamAccounts.bulk.selectPage"),
      selectRow: (name) =>
        t("accountPool.upstreamAccounts.bulk.selectRow", { name }),
      account: t("accountPool.upstreamAccounts.table.account"),
      sync: t("accountPool.upstreamAccounts.table.syncAndCall"),
      lastSuccess: t("accountPool.upstreamAccounts.table.lastSuccessShort"),
      lastCall: t("accountPool.upstreamAccounts.table.lastCallShort"),
      routingBlock: t("accountPool.upstreamAccounts.table.routingBlockShort"),
      latestAction: t("accountPool.upstreamAccounts.table.latestActionShort"),
      windows: t("accountPool.upstreamAccounts.table.windows"),
      never: t("accountPool.upstreamAccounts.never"),
      primary: t("accountPool.upstreamAccounts.primaryWindowLabel"),
      primaryShort: t("accountPool.upstreamAccounts.primaryWindowShortLabel"),
      secondary: t("accountPool.upstreamAccounts.secondaryWindowLabel"),
      secondaryShort: t("accountPool.upstreamAccounts.secondaryWindowShortLabel"),
      nextReset: t("accountPool.upstreamAccounts.table.nextReset"),
      nextResetCompact: t("accountPool.upstreamAccounts.table.nextResetCompact"),
      requestsMetric: t("accountPool.upstreamAccounts.table.requestsMetric"),
      tokensMetric: t("accountPool.upstreamAccounts.table.tokensMetric"),
      costMetric: t("accountPool.upstreamAccounts.table.costMetric"),
      inputTokensMetric: t("accountPool.upstreamAccounts.table.inputTokensMetric"),
      outputTokensMetric: t("accountPool.upstreamAccounts.table.outputTokensMetric"),
      cacheInputTokensMetric: t(
        "accountPool.upstreamAccounts.table.cacheInputTokensMetric",
      ),
      unknown: t("accountPool.upstreamAccounts.latestAction.unknown"),
      unavailable: t("accountPool.upstreamAccounts.unavailable"),
      oauth: t("accountPool.upstreamAccounts.kind.oauth"),
      apiKey: t("accountPool.upstreamAccounts.kind.apiKey"),
      mother: t("accountPool.upstreamAccounts.mother.badge"),
      duplicate: t("accountPool.upstreamAccounts.duplicate.badge"),
      hiddenTagsA11y: (count, names) =>
        t("accountPool.upstreamAccounts.table.hiddenTagsA11y", {
          count,
          names,
        }),
      workStatus: accountWorkStatusLabel,
      workStatusCount: accountWorkingCountLabel,
      enableStatus: accountEnableStatusLabel,
      healthStatus: accountHealthStatusLabel,
      syncState: accountSyncStateLabel,
      action: accountActionLabel,
      compactSupport: (item) => compactSupportLabel(item.compactSupport, t),
      compactSupportHint: (item) => compactSupportHint(item.compactSupport, t),
      actionSource: (
        value: UpstreamAccountSummary | string | null | undefined,
      ) =>
        accountActionSourceLabel(
          typeof value === "string" || value == null
            ? value
            : value.lastActionSource,
        ),
      actionReason: (
        value: UpstreamAccountSummary | string | null | undefined,
      ) =>
        accountActionReasonLabel(
          typeof value === "string" || value == null
            ? value
            : value.lastActionReasonCode,
        ),
      latestActionFieldAction: t(
        "accountPool.upstreamAccounts.latestAction.fields.action",
      ),
      latestActionFieldSource: t(
        "accountPool.upstreamAccounts.latestAction.fields.source",
      ),
      latestActionFieldReason: t(
        "accountPool.upstreamAccounts.latestAction.fields.reason",
      ),
      latestActionFieldHttpStatus: t(
        "accountPool.upstreamAccounts.latestAction.fields.httpStatus",
      ),
      latestActionFieldOccurredAt: t(
        "accountPool.upstreamAccounts.latestAction.fields.occurredAt",
      ),
      latestActionFieldMessage: t(
        "accountPool.upstreamAccounts.latestAction.fields.message",
      ),
      forwardProxyPending: t("accountPool.upstreamAccounts.proxy.pending"),
      forwardProxyUnconfigured: t(
        "accountPool.upstreamAccounts.proxy.unconfigured",
      ),
    }),
    [
      accountActionLabel,
      accountActionReasonLabel,
      accountActionSourceLabel,
      accountEnableStatusLabel,
      accountHealthStatusLabel,
      accountSyncStateLabel,
      accountWorkStatusLabel,
      accountWorkingCountLabel,
      t,
    ],
  );
  const groupedPlanLabel = useCallback(
    (planType?: string | null) => {
      const normalized = planType?.trim().toLowerCase();
      if (!normalized) return null;
      switch (normalized) {
        case "free":
          return t("accountPool.upstreamAccounts.plan.free");
        case "pro":
          return t("accountPool.upstreamAccounts.plan.plus");
        case "team":
          return t("accountPool.upstreamAccounts.plan.team");
        case "enterprise":
          return t("accountPool.upstreamAccounts.plan.enterprise");
        default:
          return normalized;
      }
    },
    [t],
  );
  const groupedRosterGroups = useMemo<UpstreamAccountsGroupedRosterGroup[]>(() => {
    if (hideRosterDerivedUi) {
      return [];
    }
    const forwardProxyNodeLabelMap = new Map(
      forwardProxyNodes.map((node) => [node.key, node.displayName?.trim() || node.key] as const),
    );
    const groupSummaryMap = new Map(
      groups.map((group) => [group.groupName, group] as const),
    );
    const groupOrder = new Map(
      groups.map((group, index) => [group.groupName, index] as const),
    );
    const grouped = new Map<string, UpstreamAccountsGroupedRosterGroup>();
    for (const item of visibleRosterItems) {
      const normalizedGroupName = normalizeRosterGroupName(item.groupName);
      const groupKey = normalizedGroupName ?? "__ungrouped__";
      const groupSummary = normalizedGroupName
        ? groupSummaryMap.get(normalizedGroupName) ?? null
        : null;
      const current = grouped.get(groupKey) ?? {
        id: groupKey,
        groupName: normalizedGroupName,
        displayName:
          normalizedGroupName ??
          t("accountPool.upstreamAccounts.groupFilter.ungrouped"),
        items: [],
        note: groupSummary?.note ?? null,
        boundProxyLabels:
          groupSummary?.boundProxyKeys?.map(
            (proxyKey) => forwardProxyNodeLabelMap.get(proxyKey) ?? proxyKey,
          ) ?? [],
        concurrencyLimit: groupSummary?.concurrencyLimit ?? null,
        nodeShuntEnabled: groupSummary?.nodeShuntEnabled ?? false,
        planCounts: [],
      };
      current.items.push(item);
      grouped.set(groupKey, current);
    }

    const planOrder = ["free", "pro", "team", "enterprise"];
    const result = Array.from(grouped.values()).map((group) => {
      const counts = new Map<string, number>();
      for (const item of group.items) {
        if (item.kind === "api_key_codex") {
          counts.set("api", (counts.get("api") ?? 0) + 1);
        }
        const normalizedPlan = item.planType?.trim().toLowerCase();
        if (!normalizedPlan || normalizedPlan === "local") continue;
        counts.set(normalizedPlan, (counts.get(normalizedPlan) ?? 0) + 1);
      }
      const orderedKeys = [
        ...planOrder.filter((key) => counts.has(key)),
        ...(counts.has("api") ? ["api"] : []),
        ...Array.from(counts.keys())
          .filter((key) => key !== "api" && !planOrder.includes(key))
          .sort(),
      ];
      return {
        ...group,
        planCounts: orderedKeys
          .map((key) => ({
            key,
            label:
              key === "api"
                ? t("accountPool.upstreamAccounts.grouped.apiBadge")
                : (groupedPlanLabel(key) ?? key),
            count: counts.get(key) ?? 0,
          }))
          .filter((plan) => plan.count > 0),
      };
    });

    result.sort((left, right) => {
      const leftOrder =
        left.groupName == null ? Number.MAX_SAFE_INTEGER : (groupOrder.get(left.groupName) ?? Number.MAX_SAFE_INTEGER - 1);
      const rightOrder =
        right.groupName == null ? Number.MAX_SAFE_INTEGER : (groupOrder.get(right.groupName) ?? Number.MAX_SAFE_INTEGER - 1);
      return leftOrder - rightOrder || left.displayName.localeCompare(right.displayName);
    });
    return result;
  }, [forwardProxyNodes, groupedPlanLabel, groups, hideRosterDerivedUi, t, visibleRosterItems]);
  const bulkRemovableTagIds = useMemo(() => {
    const removableIds = new Set<number>();
    for (const summary of Object.values(selectedAccountSummaries)) {
      for (const tag of summary.tags ?? []) {
        removableIds.add(tag.id);
      }
    }
    return Array.from(removableIds);
  }, [selectedAccountSummaries]);
  const bulkRemovableTagIdSet = useMemo(
    () => new Set(bulkRemovableTagIds),
    [bulkRemovableTagIds],
  );
  const bulkUnavailableTagIds = useMemo(
    () =>
      tagItems
        .filter((tag) => !bulkRemovableTagIdSet.has(tag.id))
        .map((tag) => tag.id),
    [bulkRemovableTagIdSet, tagItems],
  );
  const handleSelectAccount = (accountId: number) => {
    openUpstreamAccount(accountId);
  };

  const handleSaveRouting = async () => {
    if (routingDraftValidationError) {
      setActionError((current) => ({
        ...current,
        routing: routingDraftValidationError,
      }));
      return;
    }
    if (!routing) {
      setActionError((current) => ({
        ...current,
        routing: "Pool routing settings are still loading.",
      }));
      return;
    }
    if (!routingWritesEnabled) {
      setActionError((current) => ({
        ...current,
        routing: "Pool routing settings are currently read-only.",
      }));
      return;
    }
    const timeoutEntries: Array<
      [keyof PoolRoutingTimeoutSettings, string, string]
    > = [
      [
        "responsesFirstByteTimeoutSecs",
        t("accountPool.upstreamAccounts.routing.timeout.responsesFirstByte"),
        routingDraft.responsesFirstByteTimeoutSecs,
      ],
      [
        "compactFirstByteTimeoutSecs",
        t("accountPool.upstreamAccounts.routing.timeout.compactFirstByte"),
        routingDraft.compactFirstByteTimeoutSecs,
      ],
      [
        "responsesStreamTimeoutSecs",
        t("accountPool.upstreamAccounts.routing.timeout.responsesStream"),
        routingDraft.responsesStreamTimeoutSecs,
      ],
      [
        "compactStreamTimeoutSecs",
        t("accountPool.upstreamAccounts.routing.timeout.compactStream"),
        routingDraft.compactStreamTimeoutSecs,
      ],
    ];
    const parsedTimeouts = {} as PoolRoutingTimeoutSettings;
    for (const [key, label, raw] of timeoutEntries) {
      const result = parseRoutingTimeoutValue(raw, label);
      if (!result.ok) {
        setActionError((current) => ({ ...current, routing: result.error }));
        return;
      }
      parsedTimeouts[key] = result.value;
    }
    setActionError((current) => ({ ...current, routing: null }));
    const trimmedApiKey = routingDraft.apiKey.trim();
    const payload: {
      apiKey?: string;
      maintenance?: PoolRoutingMaintenanceSettings;
      timeouts?: PoolRoutingTimeoutSettings;
    } = {};
    if (routingWritesEnabled && trimmedApiKey) {
      payload.apiKey = trimmedApiKey;
    }
    if (routingHasMaintenanceChange && parsedRoutingMaintenance) {
      payload.maintenance = parsedRoutingMaintenance;
    }
    if (routingHasTimeoutChange) {
      payload.timeouts = parsedTimeouts;
    }
    if (!payload.apiKey && !payload.maintenance && !payload.timeouts) {
      setIsRoutingDialogInspectOnly(false);
      setIsRoutingDialogOpen(false);
      return;
    }
    setBusyAction((current) => ({ ...current, routing: true }));
    try {
      await saveRouting(payload);
      setRoutingDraft((current) => ({ ...current, apiKey: "" }));
      setIsRoutingDialogInspectOnly(false);
      setIsRoutingDialogOpen(false);
    } catch (err) {
      setActionError((current) => ({
        ...current,
        routing: err instanceof Error ? err.message : String(err),
      }));
    } finally {
      setBusyAction((current) => ({ ...current, routing: false }));
    }
  };

  const isBulkSyncRunning = bulkSyncSnapshot?.status === "running";
  const isBulkSyncBusy = isBulkSyncRunning || isBulkSyncStarting;

  const handleToggleSelectedAccount = useCallback(
    (accountId: number, checked: boolean) => {
      setSelectedAccountIds((current) => {
        if (checked) {
          return current.includes(accountId)
            ? current
            : [...current, accountId];
        }
        return current.filter((value) => value !== accountId);
      });
    },
    [],
  );

  const handleToggleSelectAllCurrentPage = useCallback(
    (checked: boolean) => {
      const currentPageIds = items.map((item) => item.id);
      setSelectedAccountIds((current) => {
        const next = new Set(current);
        if (checked) {
          currentPageIds.forEach((accountId) => next.add(accountId));
        } else {
          currentPageIds.forEach((accountId) => next.delete(accountId));
        }
        return Array.from(next);
      });
    },
    [items],
  );

  const closeBulkOverlays = useCallback(() => {
    setBulkGroupDialogOpen(false);
    setBulkTagsDialogOpen(false);
    setBulkDeleteDialogOpen(false);
  }, []);

  const summarizeBulkAction = useCallback(
    (action: string, succeededCount: number, failedCount: number) => {
      setBulkActionMessage(
        t("accountPool.upstreamAccounts.bulk.resultSummary", {
          action: t(`accountPool.upstreamAccounts.bulk.actionLabel.${action}`),
          succeeded: succeededCount,
          failed: failedCount,
        }),
      );
    },
    [t],
  );

  const handleBulkAction = useCallback(
    async (
      payload: BulkUpstreamAccountActionPayload,
      options?: { clearSelection?: boolean; onSuccess?: () => void },
    ) => {
      if (selectedAccountIds.length === 0) return;
      setBulkActionBusy(payload.action);
      setBulkActionError(null);
      setBulkActionMessage(null);
      try {
        const response = await runBulkAction(payload);
        summarizeBulkAction(
          response.action,
          response.succeededCount,
          response.failedCount,
        );
        options?.onSuccess?.();
        if (options?.clearSelection !== false) {
          clearBulkSelection();
        }
      } catch (err) {
        setBulkActionError(err instanceof Error ? err.message : String(err));
      } finally {
        setBulkActionBusy(null);
      }
    },
    [
      clearBulkSelection,
      runBulkAction,
      selectedAccountIds.length,
      summarizeBulkAction,
    ],
  );

  const handleOpenBulkTagsDialog = useCallback(
    (mode: "add_tags" | "remove_tags") => {
      setBulkTagMode(mode);
      setBulkTagIds([]);
      setBulkTagsDialogOpen(true);
      setBulkActionError(null);
    },
    [],
  );

  const applyBulkSyncTerminalState = useCallback(
    (
      nextSnapshot: BulkUpstreamAccountSyncSnapshot,
      nextCounts: BulkUpstreamAccountSyncCounts | null,
      options?: {
        error?: string | null;
        status?: BulkUpstreamAccountSyncSnapshot["status"];
      },
    ) => {
      const resolvedSnapshot = options?.status
        ? withBulkSyncSnapshotStatus(nextSnapshot, options.status)
        : nextSnapshot;
      const resolvedCounts = resolveBulkSyncCounts(
        resolvedSnapshot,
        nextCounts,
      );
      const shouldHide = shouldAutoHideBulkSyncProgress(
        resolvedSnapshot,
        resolvedCounts,
      );

      closeBulkSyncEventSource();
      if (shouldHide) {
        clearBulkSyncProgress();
      } else {
        setBulkSyncSnapshot(resolvedSnapshot);
        setBulkSyncCounts(resolvedCounts);
        setBulkSyncError(options?.error ?? null);
      }
      void refresh();
    },
    [clearBulkSyncProgress, closeBulkSyncEventSource, refresh],
  );

  const handleStartBulkSync = useCallback(async () => {
    if (selectedAccountIds.length === 0 || isBulkSyncBusy) return;
    setIsBulkSyncStarting(true);
    setBulkActionError(null);
    setBulkActionMessage(null);
    setBulkSyncError(null);
    closeBulkSyncEventSource();
    try {
      const created = await startBulkSyncJob({
        accountIds: selectedAccountIds,
      });
      setBulkSyncSnapshot(created.snapshot);
      setBulkSyncCounts(created.counts);
      const eventSource = createBulkUpstreamAccountSyncJobEventSource(
        created.jobId,
      );
      bulkSyncEventSourceRef.current = eventSource;

      eventSource.addEventListener("snapshot", (event) => {
        const payload = normalizeBulkUpstreamAccountSyncSnapshotEventPayload(
          JSON.parse((event as MessageEvent<string>).data),
        );
        setBulkSyncSnapshot(payload.snapshot);
        setBulkSyncCounts(payload.counts);
      });

      eventSource.addEventListener("row", (event) => {
        const payload = normalizeBulkUpstreamAccountSyncRowEventPayload(
          JSON.parse((event as MessageEvent<string>).data),
        );
        setBulkSyncCounts(payload.counts);
        setBulkSyncSnapshot((current) => {
          if (!current) return current;
          return {
            ...current,
            rows: current.rows.map((row) =>
              row.accountId === payload.row.accountId ? payload.row : row,
            ),
          };
        });
      });

      const handleTerminalEvent = (
        nextSnapshot: BulkUpstreamAccountSyncSnapshot,
        nextCounts: BulkUpstreamAccountSyncCounts,
        error?: string,
        status?: BulkUpstreamAccountSyncSnapshot["status"],
      ) => {
        applyBulkSyncTerminalState(nextSnapshot, nextCounts, { error, status });
      };

      eventSource.addEventListener("completed", (event) => {
        const payload = normalizeBulkUpstreamAccountSyncSnapshotEventPayload(
          JSON.parse((event as MessageEvent<string>).data),
        );
        handleTerminalEvent(
          payload.snapshot,
          payload.counts,
          undefined,
          "completed",
        );
      });

      eventSource.addEventListener("cancelled", (event) => {
        const payload = normalizeBulkUpstreamAccountSyncSnapshotEventPayload(
          JSON.parse((event as MessageEvent<string>).data),
        );
        handleTerminalEvent(
          payload.snapshot,
          payload.counts,
          undefined,
          "cancelled",
        );
      });

      eventSource.addEventListener("failed", (event) => {
        const payload = normalizeBulkUpstreamAccountSyncFailedEventPayload(
          JSON.parse((event as MessageEvent<string>).data),
        );
        handleTerminalEvent(
          payload.snapshot,
          payload.counts,
          payload.error,
          "failed",
        );
      });

      eventSource.onerror = () => {
        void getBulkSyncJob(created.jobId)
          .then((latest) => {
            if (latest.snapshot.status !== "running") {
              applyBulkSyncTerminalState(latest.snapshot, latest.counts);
              return;
            }
            setBulkSyncSnapshot(latest.snapshot);
            setBulkSyncCounts(latest.counts);
          })
          .catch((err) => {
            setBulkSyncError(err instanceof Error ? err.message : String(err));
            closeBulkSyncEventSource();
          });
      };
    } catch (err) {
      setBulkSyncError(err instanceof Error ? err.message : String(err));
    } finally {
      setIsBulkSyncStarting(false);
    }
  }, [
    applyBulkSyncTerminalState,
    closeBulkSyncEventSource,
    getBulkSyncJob,
    isBulkSyncBusy,
    refresh,
    selectedAccountIds,
    startBulkSyncJob,
  ]);

  const handleCancelBulkSync = useCallback(async () => {
    if (!bulkSyncSnapshot?.jobId || bulkSyncSnapshot.status !== "running")
      return;
    try {
      await stopBulkSyncJob(bulkSyncSnapshot.jobId);
    } catch (err) {
      setBulkSyncError(err instanceof Error ? err.message : String(err));
    }
  }, [bulkSyncSnapshot?.jobId, bulkSyncSnapshot?.status, stopBulkSyncJob]);

  const bulkSyncProgressBubble = bulkSyncSnapshot ? (
    <div className="pointer-events-none fixed inset-x-3 bottom-3 z-[65] sm:inset-x-auto sm:right-4 sm:w-[min(30rem,calc(100vw-2rem))]">
      <Card
        className={cn(
          "pointer-events-auto overflow-hidden rounded-[1.75rem] border border-base-300/85 bg-base-100/92 shadow-[0_24px_64px_rgba(15,23,42,0.28)] backdrop-blur-xl",
          bulkSyncSnapshot.status === "running"
            ? "ring-1 ring-primary/20"
            : "ring-1 ring-base-300/60",
        )}
      >
        <CardHeader className="flex flex-col gap-3 border-b border-base-300/70 bg-base-100/78 pb-3 sm:flex-row sm:items-start sm:justify-between">
          <div className="space-y-1">
            <CardTitle className="flex items-center gap-2 text-base">
              <span className="inline-flex h-8 w-8 items-center justify-center rounded-full bg-primary/12 text-primary">
                {bulkSyncSnapshot.status === "running" ? (
                  <Spinner size="sm" />
                ) : (
                  <AppIcon name="refresh" className="h-4 w-4" aria-hidden />
                )}
              </span>
              {t("accountPool.upstreamAccounts.bulk.syncProgressTitle")}
            </CardTitle>
            <CardDescription className="text-xs leading-5 text-base-content/72">
              {t("accountPool.upstreamAccounts.bulk.syncProgressSummary", {
                completed: bulkSyncCounts?.completed ?? 0,
                total: bulkSyncCounts?.total ?? bulkSyncSnapshot.rows.length,
                succeeded: bulkSyncCounts?.succeeded ?? 0,
                failed: bulkSyncCounts?.failed ?? 0,
                skipped: bulkSyncCounts?.skipped ?? 0,
              })}
            </CardDescription>
          </div>
          {bulkSyncSnapshot.status === "running" ? (
            <Button
              type="button"
              variant="outline"
              size="sm"
              onClick={() => void handleCancelBulkSync()}
            >
              {t("accountPool.upstreamAccounts.bulk.cancelSync")}
            </Button>
          ) : (
            <Button
              type="button"
              variant="ghost"
              size="icon"
              className="h-8 w-8 rounded-full text-base-content/62 hover:text-base-content"
              aria-label={t("accountPool.upstreamAccounts.bulk.dismissSync")}
              title={t("accountPool.upstreamAccounts.bulk.dismissSync")}
              onClick={clearBulkSyncProgress}
            >
              <AppIcon name="close" className="h-4 w-4" aria-hidden />
            </Button>
          )}
        </CardHeader>
        <CardContent className="space-y-3 p-4 pt-3">
          {bulkSyncError ? (
            <Alert variant="error">
              <AppIcon
                name="alert-circle-outline"
                className="mt-0.5 h-4 w-4 shrink-0"
                aria-hidden
              />
              <div>{bulkSyncError}</div>
            </Alert>
          ) : null}
          <div className="max-h-[min(52vh,20rem)] space-y-2 overflow-y-auto rounded-2xl border border-base-300/80 bg-base-100/72 p-3">
            {bulkSyncSnapshot.rows.map((row) => (
              <div
                key={row.accountId}
                className="flex flex-col gap-1 rounded-xl border border-base-300/60 px-3 py-2 text-sm"
              >
                <div className="flex items-center justify-between gap-3">
                  <span className="font-medium text-base-content">
                    {row.displayName}
                  </span>
                  <Badge variant={bulkSyncRowStatusVariant(row.status)}>
                    {t(
                      `accountPool.upstreamAccounts.bulk.rowStatus.${row.status}`,
                    )}
                  </Badge>
                </div>
                {row.detail ? (
                  <p className="text-xs text-base-content/68">{row.detail}</p>
                ) : null}
              </div>
            ))}
          </div>
        </CardContent>
      </Card>
    </div>
  ) : null;

  return (
    <div className="grid gap-6">
      <section className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_20rem]">
        <div className="surface-panel overflow-hidden">
          <div className="surface-panel-body gap-5">
            <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
              <div className="section-heading">
                <h2 className="section-title">
                  {t("accountPool.upstreamAccounts.title")}
                </h2>
                <p className="section-description">
                  {t("accountPool.upstreamAccounts.description")}
                </p>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  type="button"
                  variant="secondary"
                  onClick={() => void refresh()}
                  disabled={isBusyAction(busyAction, "routing")}
                >
                  <AppIcon
                    name="refresh"
                    className="mr-2 h-4 w-4"
                    aria-hidden
                  />
                  {t("accountPool.upstreamAccounts.actions.refresh")}
                </Button>
                {writesEnabled ? (
                  <Button asChild>
                    <Link to="/account-pool/upstream-accounts/new">
                      <AppIcon
                        name="plus-circle-outline"
                        className="mr-2 h-4 w-4"
                        aria-hidden
                      />
                      {t("accountPool.upstreamAccounts.actions.addAccount")}
                    </Link>
                  </Button>
                ) : (
                  <Button type="button" disabled>
                    <AppIcon
                      name="plus-circle-outline"
                      className="mr-2 h-4 w-4"
                      aria-hidden
                    />
                    {t("accountPool.upstreamAccounts.actions.addAccount")}
                  </Button>
                )}
              </div>
            </div>

            {!writesEnabled ? (
              <Alert variant="warning">
                <AppIcon
                  name="shield-key-outline"
                  className="mt-0.5 h-4 w-4 shrink-0"
                  aria-hidden
                />
                <div>
                  <p className="font-medium">
                    {t("accountPool.upstreamAccounts.writesDisabledTitle")}
                  </p>
                  <p className="mt-1 text-sm text-warning/90">
                    {t("accountPool.upstreamAccounts.writesDisabledBody")}
                  </p>
                </div>
              </Alert>
            ) : null}

            {visibleRoutingError ? (
              <Alert variant="error">
                <AppIcon
                  name="alert-circle-outline"
                  className="mt-0.5 h-4 w-4 shrink-0"
                  aria-hidden
                />
                <div>{visibleRoutingError}</div>
              </Alert>
            ) : null}

            {duplicateWarning ? (
              <Alert variant="warning">
                <AppIcon
                  name="alert-outline"
                  className="mt-0.5 h-4 w-4 shrink-0"
                  aria-hidden
                />
                <div className="flex min-w-0 flex-1 flex-col gap-2">
                  <p className="font-medium">
                    {t("accountPool.upstreamAccounts.duplicate.warningTitle", {
                      name: duplicateWarning.displayName,
                    })}
                  </p>
                  <p className="text-sm text-warning/90">
                    {t("accountPool.upstreamAccounts.duplicate.warningBody", {
                      reasons: formatDuplicateReasons({
                        peerAccountIds: duplicateWarning.peerAccountIds,
                        reasons: duplicateWarning.reasons,
                      }),
                      peers: duplicateWarning.peerAccountIds.join(", "),
                    })}
                  </p>
                </div>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={() => setDuplicateWarning(null)}
                >
                  {t(
                    "accountPool.upstreamAccounts.actions.dismissDuplicateWarning",
                  )}
                </Button>
              </Alert>
            ) : null}

            {postCreateWarning ? (
              <Alert variant="warning">
                <AppIcon
                  name="alert-outline"
                  className="mt-0.5 h-4 w-4 shrink-0"
                  aria-hidden
                />
                <div className="flex min-w-0 flex-1 flex-col gap-2">
                  <p className="font-medium">
                    {t("accountPool.upstreamAccounts.partialSuccess.title")}
                  </p>
                  <p className="text-sm text-warning/90">{postCreateWarning}</p>
                </div>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  onClick={() => setPostCreateWarning(null)}
                >
                  {t(
                    "accountPool.upstreamAccounts.actions.dismissDuplicateWarning",
                  )}
                </Button>
              </Alert>
            ) : null}

            <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
              {metrics.map((metric) => (
                <Card
                  key={metric.label}
                  className="border-base-300/80 bg-base-100/72"
                >
                  <CardContent className="flex items-center gap-4 p-5">
                    <div
                      className={cn(
                        "flex h-12 w-12 items-center justify-center rounded-2xl bg-base-200/70",
                        metric.accent,
                      )}
                    >
                      <AppIcon
                        name={metric.icon}
                        className="h-6 w-6"
                        aria-hidden
                      />
                    </div>
                    <div>
                      <p className="text-xs font-semibold uppercase tracking-[0.16em] text-base-content/55">
                        {metric.label}
                      </p>
                      <p className="mt-1 text-3xl font-semibold text-base-content">
                        {metric.value}
                      </p>
                    </div>
                  </CardContent>
                </Card>
              ))}
            </div>
          </div>
        </div>

        <div className="grid gap-4">
          <Card className="border-base-300/80 bg-base-100/72">
            <CardHeader>
              <CardTitle>
                {t("accountPool.upstreamAccounts.routing.title")}
              </CardTitle>
              <CardDescription>
                {t("accountPool.upstreamAccounts.routing.description")}
              </CardDescription>
            </CardHeader>
            <CardContent className="space-y-4">
              <div className="rounded-2xl border border-base-300/80 bg-base-100/75 p-3 text-sm text-base-content/75">
                <div className="flex items-start justify-between gap-3">
                  <div className="min-w-0">
                    <p className="metric-label">
                      {t("accountPool.upstreamAccounts.routing.currentKey")}
                    </p>
                    <p className="mt-2 break-all font-mono text-sm text-base-content">
                      {routing?.apiKeyConfigured
                        ? (routing?.maskedApiKey ??
                          t("accountPool.upstreamAccounts.routing.configured"))
                        : t(
                            "accountPool.upstreamAccounts.routing.notConfigured",
                          )}
                    </p>
                  </div>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    onClick={handleOpenRoutingDialog}
                    disabled={!routing}
                  >
                    <AppIcon
                      name="pencil-outline"
                      className="h-4 w-4"
                      aria-hidden
                    />
                    <span className="sr-only">
                      {t("accountPool.upstreamAccounts.routing.edit")}
                    </span>
                  </Button>
                </div>
              </div>
            </CardContent>
          </Card>
        </div>
      </section>

      <section className="grid gap-6">
        <div className="surface-panel overflow-hidden">
          <div className="surface-panel-body gap-4">
            <div className="space-y-4">
              <div className="flex flex-col gap-3 lg:flex-row lg:items-start lg:justify-between">
                <div className="section-heading">
                  <h2 className="section-title">
                    {t("accountPool.upstreamAccounts.listTitle")}
                  </h2>
                  <p className="section-description">
                    {t("accountPool.upstreamAccounts.listDescription")}
                  </p>
                </div>
                <div className="flex items-center justify-start gap-3 lg:justify-end">
                  <SegmentedControl
                    size="compact"
                    role="tablist"
                    aria-label={t("accountPool.upstreamAccounts.viewToggleAria")}
                  >
                    <SegmentedControlItem
                      type="button"
                      role="tab"
                      aria-selected={rosterViewMode === "flat"}
                      aria-pressed={rosterViewMode === "flat"}
                      active={rosterViewMode === "flat"}
                      onClick={() => setRosterViewMode("flat")}
                    >
                      {t("accountPool.upstreamAccounts.viewMode.flat")}
                    </SegmentedControlItem>
                    <SegmentedControlItem
                      type="button"
                      role="tab"
                      aria-selected={rosterViewMode === "grouped"}
                      aria-pressed={rosterViewMode === "grouped"}
                      active={rosterViewMode === "grouped"}
                      onClick={() => setRosterViewMode("grouped")}
                    >
                      {t("accountPool.upstreamAccounts.viewMode.grouped")}
                    </SegmentedControlItem>
                    <SegmentedControlItem
                      type="button"
                      role="tab"
                      aria-selected={rosterViewMode === "grid"}
                      aria-pressed={rosterViewMode === "grid"}
                      active={rosterViewMode === "grid"}
                      onClick={() => setRosterViewMode("grid")}
                    >
                      {t("accountPool.upstreamAccounts.viewMode.grid")}
                    </SegmentedControlItem>
                  </SegmentedControl>
                  {isLoading ? (
                    <div className="flex items-center justify-start lg:justify-end">
                      <Spinner className="text-primary" />
                    </div>
                  ) : null}
                </div>
              </div>

              <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-12">
                <label
                  className={cn(
                    "field min-w-0",
                    formFieldSpanVariants({ size: "compact" }),
                  )}
                >
                  <span className="field-label">
                    {t("accountPool.upstreamAccounts.workStatusFilterLabel")}
                  </span>
                  <MultiSelectFilterCombobox
                    size="filter"
                    options={workStatusFilterOptions}
                    value={workStatusFilter}
                    placeholder={t(
                      "accountPool.upstreamAccounts.workStatusFilter.all",
                    )}
                    searchPlaceholder={t(
                      "accountPool.upstreamAccounts.workStatusFilter.searchPlaceholder",
                    )}
                    emptyLabel={t(
                      "accountPool.upstreamAccounts.workStatusFilter.empty",
                    )}
                    clearLabel={t(
                      "accountPool.upstreamAccounts.workStatusFilter.clear",
                    )}
                    ariaLabel={t(
                      "accountPool.upstreamAccounts.workStatusFilterLabel",
                    )}
                    triggerClassName="border-base-300/90 bg-base-100"
                    onValueChange={handleWorkStatusFilterChange}
                  />
                </label>
                <label
                  className={cn(
                    "field min-w-0",
                    formFieldSpanVariants({ size: "compact" }),
                  )}
                >
                  <span className="field-label">
                    {t("accountPool.upstreamAccounts.enableStatusFilterLabel")}
                  </span>
                  <MultiSelectFilterCombobox
                    size="filter"
                    options={enableStatusFilterOptions}
                    value={enableStatusFilter}
                    placeholder={t(
                      "accountPool.upstreamAccounts.enableStatusFilter.all",
                    )}
                    searchPlaceholder={t(
                      "accountPool.upstreamAccounts.enableStatusFilter.searchPlaceholder",
                    )}
                    emptyLabel={t(
                      "accountPool.upstreamAccounts.enableStatusFilter.empty",
                    )}
                    clearLabel={t(
                      "accountPool.upstreamAccounts.enableStatusFilter.clear",
                    )}
                    ariaLabel={t(
                      "accountPool.upstreamAccounts.enableStatusFilterLabel",
                    )}
                    triggerClassName="border-base-300/90 bg-base-100"
                    onValueChange={handleEnableStatusFilterChange}
                  />
                </label>
                <label
                  className={cn(
                    "field min-w-0",
                    formFieldSpanVariants({ size: "compact" }),
                  )}
                >
                  <span className="field-label">
                    {t("accountPool.upstreamAccounts.healthStatusFilterLabel")}
                  </span>
                  <MultiSelectFilterCombobox
                    size="filter"
                    options={healthStatusFilterOptions}
                    value={healthStatusFilter}
                    placeholder={t(
                      "accountPool.upstreamAccounts.healthStatusFilter.all",
                    )}
                    searchPlaceholder={t(
                      "accountPool.upstreamAccounts.healthStatusFilter.searchPlaceholder",
                    )}
                    emptyLabel={t(
                      "accountPool.upstreamAccounts.healthStatusFilter.empty",
                    )}
                    clearLabel={t(
                      "accountPool.upstreamAccounts.healthStatusFilter.clear",
                    )}
                    ariaLabel={t(
                      "accountPool.upstreamAccounts.healthStatusFilterLabel",
                    )}
                    triggerClassName="border-base-300/90 bg-base-100"
                    onValueChange={handleHealthStatusFilterChange}
                  />
                </label>
                <label
                  className={cn(
                    "field min-w-0",
                    formFieldSpanVariants({ size: "wide" }),
                  )}
                >
                  <span className="field-label">
                    {t("accountPool.upstreamAccounts.groupFilterLabel")}
                  </span>
                  <UpstreamAccountGroupCombobox
                    size="filter"
                    value={groupFilterQuery}
                    suggestions={groupFilterSuggestions}
                    placeholder={t(
                      "accountPool.upstreamAccounts.groupFilterPlaceholder",
                    )}
                    searchPlaceholder={t(
                      "accountPool.upstreamAccounts.groupFilterSearchPlaceholder",
                    )}
                    emptyLabel={t(
                      "accountPool.upstreamAccounts.groupFilterEmpty",
                    )}
                    createLabel={(value) =>
                      t("accountPool.upstreamAccounts.groupFilterUseValue", {
                        value,
                      })
                    }
                    ariaLabel={t(
                      "accountPool.upstreamAccounts.groupFilterLabel",
                    )}
                    triggerClassName="border-base-300/90 bg-base-100"
                    onValueChange={handleGroupFilterChange}
                  />
                </label>
                <label
                  className={cn(
                    "field min-w-0",
                    formFieldSpanVariants({ size: "wide" }),
                  )}
                >
                  <span className="field-label">
                    {t("accountPool.upstreamAccounts.tagFilterLabel")}
                  </span>
                  <AccountTagFilterCombobox
                    size="filter"
                    tags={tagItems}
                    value={appliedSelectedTagIds}
                    placeholder={t(
                      "accountPool.upstreamAccounts.tagFilterPlaceholder",
                    )}
                    searchPlaceholder={t(
                      "accountPool.upstreamAccounts.tagFilterSearchPlaceholder",
                    )}
                    emptyLabel={t(
                      "accountPool.upstreamAccounts.tagFilterEmpty",
                    )}
                    clearLabel={t(
                      "accountPool.upstreamAccounts.tagFilterClear",
                    )}
                    ariaLabel={t(
                      "accountPool.upstreamAccounts.tagFilterAriaLabel",
                    )}
                    triggerClassName="border-base-300/90 bg-base-100"
                    onValueChange={handleTagFilterChange}
                  />
                </label>
              </div>
            </div>

            <div
              ref={rosterRegionRef}
              data-testid="upstream-accounts-roster-region"
              className="flex flex-col gap-4"
              style={
                rosterRegionMinHeight
                  ? { minHeight: rosterRegionMinHeight }
                  : undefined
              }
            >
              {selectedAccountIds.length > 0 &&
              !hideRosterDerivedUi &&
              rosterViewMode !== "grid" ? (
                <div className="rounded-[1.25rem] border border-primary/25 bg-primary/8 px-4 py-3">
                  <div className="flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
                    <div className="text-sm text-base-content/80">
                      {t("accountPool.upstreamAccounts.bulk.selectedCount", {
                        count: selectedAccountIds.length,
                      })}
                    </div>
                    <div className="flex flex-wrap gap-2">
                      <Button
                        type="button"
                        size="sm"
                        variant="secondary"
                        onClick={() =>
                          void handleBulkAction({
                            accountIds: selectedAccountIds,
                            action: "enable",
                          })
                        }
                        disabled={
                          Boolean(bulkActionBusy) ||
                          isBulkSyncBusy ||
                          !writesEnabled
                        }
                      >
                        {t("accountPool.upstreamAccounts.bulk.enable")}
                      </Button>
                      <Button
                        type="button"
                        size="sm"
                        variant="secondary"
                        onClick={() =>
                          void handleBulkAction({
                            accountIds: selectedAccountIds,
                            action: "disable",
                          })
                        }
                        disabled={
                          Boolean(bulkActionBusy) ||
                          isBulkSyncBusy ||
                          !writesEnabled
                        }
                      >
                        {t("accountPool.upstreamAccounts.bulk.disable")}
                      </Button>
                      <Button
                        type="button"
                        size="sm"
                        variant="secondary"
                        onClick={() => {
                          setBulkGroupName("");
                          setBulkGroupDialogOpen(true);
                        }}
                        disabled={
                          Boolean(bulkActionBusy) ||
                          isBulkSyncBusy ||
                          !writesEnabled
                        }
                      >
                        {t("accountPool.upstreamAccounts.bulk.setGroup")}
                      </Button>
                      <Button
                        type="button"
                        size="sm"
                        variant="secondary"
                        onClick={() => handleOpenBulkTagsDialog("add_tags")}
                        disabled={
                          Boolean(bulkActionBusy) ||
                          isBulkSyncBusy ||
                          !writesEnabled
                        }
                      >
                        {t("accountPool.upstreamAccounts.bulk.addTags")}
                      </Button>
                      <Button
                        type="button"
                        size="sm"
                        variant="secondary"
                        onClick={() => handleOpenBulkTagsDialog("remove_tags")}
                        disabled={
                          Boolean(bulkActionBusy) ||
                          isBulkSyncBusy ||
                          !writesEnabled
                        }
                      >
                        {t("accountPool.upstreamAccounts.bulk.removeTags")}
                      </Button>
                      <Button
                        type="button"
                        size="sm"
                        variant="secondary"
                        onClick={() => void handleStartBulkSync()}
                        disabled={Boolean(bulkActionBusy) || isBulkSyncBusy}
                      >
                        {isBulkSyncStarting ? (
                          <Spinner size="sm" className="mr-2" />
                        ) : null}
                        {t("accountPool.upstreamAccounts.bulk.sync")}
                      </Button>
                      <Button
                        type="button"
                        size="sm"
                        variant="destructive"
                        onClick={() => setBulkDeleteDialogOpen(true)}
                        disabled={
                          Boolean(bulkActionBusy) ||
                          isBulkSyncBusy ||
                          !writesEnabled
                        }
                      >
                        {t("accountPool.upstreamAccounts.bulk.delete")}
                      </Button>
                      <Button
                        type="button"
                        size="sm"
                        variant="ghost"
                        onClick={clearBulkSelection}
                        disabled={Boolean(bulkActionBusy)}
                      >
                        {t("accountPool.upstreamAccounts.bulk.clearSelection")}
                      </Button>
                    </div>
                  </div>
                </div>
              ) : null}

              {bulkActionMessage ? (
                <Alert variant="success">
                  <AppIcon
                    name="check-circle-outline"
                    className="mt-0.5 h-4 w-4 shrink-0"
                    aria-hidden
                  />
                  <div>{bulkActionMessage}</div>
                </Alert>
              ) : null}

              {bulkActionError ? (
                <Alert variant="error">
                  <AppIcon
                    name="alert-circle-outline"
                    className="mt-0.5 h-4 w-4 shrink-0"
                    aria-hidden
                  />
                  <div>{bulkActionError}</div>
                </Alert>
              ) : null}

              {rosterViewMode === "flat" ? (
                <UpstreamAccountsTable
                  items={visibleRosterItems}
                  isLoading={showBlockingRosterLoading}
                  error={showBlockingRosterError ? listError : null}
                  loadingTitle={t("accountPool.upstreamAccounts.loadingTitle")}
                  loadingDescription={t(
                    "accountPool.upstreamAccounts.loadingDescription",
                  )}
                  errorTitle={t("accountPool.upstreamAccounts.listErrorTitle")}
                  retryLabel={t("accountPool.upstreamAccounts.listRetry")}
                  onRetry={() => void refresh()}
                  selectedId={upstreamAccountId}
                  selectedAccountIds={selectedAccountIdSet}
                  onSelect={handleSelectAccount}
                  onToggleSelected={handleToggleSelectedAccount}
                  onToggleSelectAllCurrentPage={handleToggleSelectAllCurrentPage}
                  emptyTitle={t("accountPool.upstreamAccounts.emptyTitle")}
                  emptyDescription={t(
                    "accountPool.upstreamAccounts.emptyDescription",
                  )}
                  labels={accountRosterLabels}
                />
              ) : (
                <UpstreamAccountsGroupedRoster
                  groups={groupedRosterGroups}
                  isLoading={showBlockingRosterLoading}
                  error={showBlockingRosterError ? listError : null}
                  loadingTitle={t("accountPool.upstreamAccounts.loadingTitle")}
                  loadingDescription={t(
                    "accountPool.upstreamAccounts.loadingDescription",
                  )}
                  errorTitle={t("accountPool.upstreamAccounts.listErrorTitle")}
                  retryLabel={t("accountPool.upstreamAccounts.listRetry")}
                  onRetry={() => void refresh()}
                  selectedId={upstreamAccountId}
                  selectedAccountIds={selectedAccountIdSet}
                  onSelect={handleSelectAccount}
                  onToggleSelected={
                    rosterViewMode === "grouped"
                      ? handleToggleSelectedAccount
                      : undefined
                  }
                  onToggleSelectAllVisible={
                    rosterViewMode === "grouped"
                      ? handleToggleSelectAllCurrentPage
                      : undefined
                  }
                  emptyTitle={t("accountPool.upstreamAccounts.emptyTitle")}
                  emptyDescription={t(
                    "accountPool.upstreamAccounts.emptyDescription",
                  )}
                  labels={accountRosterLabels}
                  memberLayout={rosterViewMode === "grid" ? "grid" : "list"}
                  selectionMode={rosterViewMode === "grid" ? "none" : "multi"}
                  groupLabels={{
                    count: (count) =>
                      t("accountPool.upstreamAccounts.grouped.accountCount", {
                        count,
                      }),
                    concurrency: (value) =>
                      t("accountPool.upstreamAccounts.grouped.concurrency", {
                        value,
                      }),
                    exclusiveNode: t(
                      "accountPool.upstreamAccounts.grouped.exclusiveNode",
                    ),
                    selectVisible: t("accountPool.upstreamAccounts.bulk.selectPage"),
                    infoTitle: t("accountPool.upstreamAccounts.grouped.infoTitle"),
                    noteLabel: t("accountPool.upstreamAccounts.grouped.noteLabel"),
                    noteEmpty: t("accountPool.upstreamAccounts.grouped.noteEmpty"),
                    proxiesLabel: t("accountPool.upstreamAccounts.grouped.proxiesLabel"),
                    proxiesEmpty: t("accountPool.upstreamAccounts.grouped.proxiesEmpty"),
                  }}
                />
              )}

              {visibleListWarning ? (
                <Alert variant="warning">
                  <AppIcon
                    name="information-outline"
                    className="mt-0.5 h-4 w-4 shrink-0"
                    aria-hidden
                  />
                  <div>{visibleListWarning}</div>
                </Alert>
              ) : null}

              {showPaginationFooter ? (
                <div
                  data-testid="upstream-accounts-pagination-footer"
                  className={cn(
                    "flex flex-col gap-3 border-t border-base-300/70 pt-4 sm:flex-row sm:items-end sm:justify-between",
                    hideRosterDerivedUi ? "mt-auto" : null,
                  )}
                >
                  <div className="space-y-2">
                    {effectiveTotal != null && pageCount != null ? (
                      <div className="text-sm text-base-content/70">
                        {t("accountPool.upstreamAccounts.pagination.summary", {
                          page,
                          pageCount,
                          total: effectiveTotal,
                        })}
                      </div>
                    ) : null}
                    {paginationStatusText ? (
                      <div
                        data-testid="upstream-accounts-pagination-status"
                        className={cn(
                          "inline-flex items-center gap-2 rounded-full border px-3 py-1.5 text-sm",
                          showBlockingRosterLoading
                            ? "border-primary/20 bg-primary/8 text-primary"
                            : "border-error/20 bg-error/8 text-error",
                        )}
                      >
                        {showBlockingRosterLoading ? (
                          <Spinner size="sm" className="h-4 w-4" />
                        ) : (
                          <AppIcon
                            name="alert-circle-outline"
                            className="h-4 w-4"
                            aria-hidden
                          />
                        )}
                        <span>{paginationStatusText}</span>
                      </div>
                    ) : null}
                  </div>
                  <div className="flex flex-wrap items-center gap-3">
                    <div className="flex items-center gap-2 rounded-xl border border-base-300/70 bg-base-100/55 px-3 py-2">
                      <span className="text-sm font-medium text-base-content/65">
                        {t("accountPool.upstreamAccounts.pagination.pageSize")}
                      </span>
                      <SelectField
                        className="min-w-[7rem]"
                        value={String(pageSize)}
                        options={pageSizeOptions}
                        size="sm"
                        disabled={showBlockingRosterLoading}
                        triggerClassName="h-10 rounded-xl border-base-300/90 bg-base-100 px-3 text-sm"
                        aria-label={t(
                          "accountPool.upstreamAccounts.pagination.pageSize",
                        )}
                        onValueChange={(value) =>
                          handlePageSizeChange(Number(value))
                        }
                      />
                    </div>
                    <div className="flex items-center gap-2">
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        className="h-10 rounded-xl px-4"
                        onClick={() =>
                          setPage((current) => Math.max(1, current - 1))
                        }
                        disabled={showBlockingRosterLoading || page <= 1}
                      >
                        {t("accountPool.upstreamAccounts.pagination.previous")}
                      </Button>
                      <Button
                        type="button"
                        variant="outline"
                        size="sm"
                        className="h-10 rounded-xl px-4"
                        onClick={() =>
                          setPage((current) =>
                            Math.min(nextPageLimit, current + 1),
                          )
                        }
                        disabled={
                          showBlockingRosterLoading ||
                          pageCount == null ||
                          page >= pageCount
                        }
                      >
                        {t("accountPool.upstreamAccounts.pagination.next")}
                      </Button>
                    </div>
                  </div>
                </div>
              ) : null}
            </div>
          </div>
        </div>
      </section>

      <Dialog
        open={bulkGroupDialogOpen}
        onOpenChange={(open) =>
          !bulkActionBusy ? setBulkGroupDialogOpen(open) : undefined
        }
      >
        <DialogContent className="p-0">
          <div className="flex items-start justify-between gap-4 border-b border-base-300/80 px-6 py-5">
            <DialogHeader className="min-w-0 max-w-[28rem]">
              <DialogTitle>
                {t("accountPool.upstreamAccounts.bulk.groupDialogTitle")}
              </DialogTitle>
              <DialogDescription>
                {t("accountPool.upstreamAccounts.bulk.groupDialogDescription")}
              </DialogDescription>
            </DialogHeader>
            <DialogCloseIcon
              aria-label={t("accountPool.upstreamAccounts.actions.cancel")}
              disabled={Boolean(bulkActionBusy)}
            />
          </div>
          <div className="space-y-4 px-6 py-6">
            <label className="field">
              <span className="field-label">
                {t("accountPool.upstreamAccounts.bulk.groupField")}
              </span>
              <UpstreamAccountGroupCombobox
                value={bulkGroupName}
                suggestions={groupFilterSuggestions}
                placeholder={t(
                  "accountPool.upstreamAccounts.bulk.groupPlaceholder",
                )}
                searchPlaceholder={t(
                  "accountPool.upstreamAccounts.groupFilterSearchPlaceholder",
                )}
                emptyLabel={t("accountPool.upstreamAccounts.groupFilterEmpty")}
                createLabel={(value) =>
                  t("accountPool.upstreamAccounts.groupFilterUseValue", {
                    value,
                  })
                }
                ariaLabel={t("accountPool.upstreamAccounts.bulk.groupField")}
                onValueChange={setBulkGroupName}
              />
            </label>
          </div>
          <DialogFooter className="border-t border-base-300/80 px-6 py-5">
            <Button
              type="button"
              variant="outline"
              onClick={closeBulkOverlays}
              disabled={Boolean(bulkActionBusy)}
            >
              {t("accountPool.upstreamAccounts.actions.cancel")}
            </Button>
            <Button
              type="button"
              onClick={() =>
                void handleBulkAction(
                  {
                    accountIds: selectedAccountIds,
                    action: "set_group",
                    groupName: bulkGroupName.trim(),
                  },
                  { onSuccess: closeBulkOverlays },
                )
              }
              disabled={Boolean(bulkActionBusy) || !writesEnabled}
            >
              {t("accountPool.upstreamAccounts.bulk.apply")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={bulkTagsDialogOpen}
        onOpenChange={(open) =>
          !bulkActionBusy ? setBulkTagsDialogOpen(open) : undefined
        }
      >
        <DialogContent className="p-0">
          <div className="flex items-start justify-between gap-4 border-b border-base-300/80 px-6 py-5">
            <DialogHeader className="min-w-0 max-w-[28rem]">
              <DialogTitle>
                {t(
                  bulkTagMode === "add_tags"
                    ? "accountPool.upstreamAccounts.bulk.addTagsDialogTitle"
                    : "accountPool.upstreamAccounts.bulk.removeTagsDialogTitle",
                )}
              </DialogTitle>
              <DialogDescription>
                {t("accountPool.upstreamAccounts.bulk.tagsDialogDescription")}
              </DialogDescription>
            </DialogHeader>
            <DialogCloseIcon
              aria-label={t("accountPool.upstreamAccounts.actions.cancel")}
              disabled={Boolean(bulkActionBusy)}
            />
          </div>
          <div className="space-y-4 px-6 py-6">
            <label className="field">
              <span className="field-label">
                {t("accountPool.upstreamAccounts.bulk.tagsField")}
              </span>
              <AccountTagFilterCombobox
                tags={tagItems}
                value={bulkTagIds}
                prioritizedTagIds={
                  bulkTagMode === "remove_tags"
                    ? bulkRemovableTagIds
                    : undefined
                }
                disabledTagIds={
                  bulkTagMode === "remove_tags"
                    ? bulkUnavailableTagIds
                    : undefined
                }
                placeholder={t(
                  "accountPool.upstreamAccounts.bulk.tagsPlaceholder",
                )}
                searchPlaceholder={t(
                  "accountPool.upstreamAccounts.tagFilterSearchPlaceholder",
                )}
                emptyLabel={t("accountPool.upstreamAccounts.tagFilterEmpty")}
                clearLabel={t("accountPool.upstreamAccounts.tagFilterClear")}
                ariaLabel={t("accountPool.upstreamAccounts.bulk.tagsField")}
                onValueChange={setBulkTagIds}
              />
            </label>
          </div>
          <DialogFooter className="border-t border-base-300/80 px-6 py-5">
            <Button
              type="button"
              variant="outline"
              onClick={closeBulkOverlays}
              disabled={Boolean(bulkActionBusy)}
            >
              {t("accountPool.upstreamAccounts.actions.cancel")}
            </Button>
            <Button
              type="button"
              onClick={() =>
                void handleBulkAction(
                  {
                    accountIds: selectedAccountIds,
                    action: bulkTagMode,
                    tagIds: bulkTagIds,
                  },
                  { onSuccess: closeBulkOverlays },
                )
              }
              disabled={
                Boolean(bulkActionBusy) ||
                bulkTagIds.length === 0 ||
                !writesEnabled
              }
            >
              {t("accountPool.upstreamAccounts.bulk.apply")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <Dialog
        open={bulkDeleteDialogOpen}
        onOpenChange={(open) =>
          !bulkActionBusy ? setBulkDeleteDialogOpen(open) : undefined
        }
      >
        <DialogContent className="p-0">
          <div className="flex items-start justify-between gap-4 border-b border-base-300/80 px-6 py-5">
            <DialogHeader className="min-w-0 max-w-[28rem]">
              <DialogTitle>
                {t("accountPool.upstreamAccounts.bulk.deleteDialogTitle")}
              </DialogTitle>
              <DialogDescription>
                {t(
                  "accountPool.upstreamAccounts.bulk.deleteDialogDescription",
                  { count: selectedAccountIds.length },
                )}
              </DialogDescription>
            </DialogHeader>
            <DialogCloseIcon
              aria-label={t("accountPool.upstreamAccounts.actions.cancel")}
              disabled={Boolean(bulkActionBusy)}
            />
          </div>
          <DialogFooter className="px-6 py-5">
            <Button
              type="button"
              variant="outline"
              onClick={closeBulkOverlays}
              disabled={Boolean(bulkActionBusy)}
            >
              {t("accountPool.upstreamAccounts.actions.cancel")}
            </Button>
            <Button
              type="button"
              variant="destructive"
              onClick={() =>
                void handleBulkAction(
                  { accountIds: selectedAccountIds, action: "delete" },
                  { onSuccess: closeBulkOverlays },
                )
              }
              disabled={Boolean(bulkActionBusy) || !writesEnabled}
            >
              {t("accountPool.upstreamAccounts.actions.confirmDelete")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      <RoutingSettingsDialog
        open={isRoutingDialogOpen}
        title={t("accountPool.upstreamAccounts.routing.dialogTitle")}
        description={t(
          "accountPool.upstreamAccounts.routing.dialogDescription",
        )}
        closeLabel={t("accountPool.upstreamAccounts.routing.close")}
        cancelLabel={t("accountPool.upstreamAccounts.actions.cancel")}
        saveLabel={t("accountPool.upstreamAccounts.routing.save")}
        apiKey={routingDraft.apiKey}
        primarySyncIntervalSecs={routingDraft.primarySyncIntervalSecs}
        secondarySyncIntervalSecs={routingDraft.secondarySyncIntervalSecs}
        priorityAvailableAccountCap={routingDraft.priorityAvailableAccountCap}
        timeoutSectionTitle={t(
          "accountPool.upstreamAccounts.routing.timeout.sectionTitle",
        )}
        timeoutFields={[
          {
            key: "responsesFirstByteTimeoutSecs",
            label: t(
              "accountPool.upstreamAccounts.routing.timeout.responsesFirstByte",
            ),
            value: routingDraft.responsesFirstByteTimeoutSecs,
            onChange: (value) =>
              setRoutingDraft((current) => ({
                ...current,
                responsesFirstByteTimeoutSecs: value,
              })),
          },
          {
            key: "compactFirstByteTimeoutSecs",
            label: t(
              "accountPool.upstreamAccounts.routing.timeout.compactFirstByte",
            ),
            value: routingDraft.compactFirstByteTimeoutSecs,
            onChange: (value) =>
              setRoutingDraft((current) => ({
                ...current,
                compactFirstByteTimeoutSecs: value,
              })),
          },
          {
            key: "responsesStreamTimeoutSecs",
            label: t(
              "accountPool.upstreamAccounts.routing.timeout.responsesStream",
            ),
            value: routingDraft.responsesStreamTimeoutSecs,
            onChange: (value) =>
              setRoutingDraft((current) => ({
                ...current,
                responsesStreamTimeoutSecs: value,
              })),
          },
          {
            key: "compactStreamTimeoutSecs",
            label: t(
              "accountPool.upstreamAccounts.routing.timeout.compactStream",
            ),
            value: routingDraft.compactStreamTimeoutSecs,
            onChange: (value) =>
              setRoutingDraft((current) => ({
                ...current,
                compactStreamTimeoutSecs: value,
              })),
          },
        ]}
        busy={isBusyAction(busyAction, "routing")}
        apiKeyWritesEnabled={routingDialogCanEdit}
        timeoutWritesEnabled={routingDialogCanEdit}
        canSave={routingCanSave}
        onApiKeyChange={(value) =>
          setRoutingDraft((current) => ({ ...current, apiKey: value }))
        }
        onGenerate={() =>
          setRoutingDraft((current) => ({
            ...current,
            apiKey: generatePoolRoutingKey(),
          }))
        }
        onPrimarySyncIntervalChange={(value) =>
          setRoutingDraft((current) => ({
            ...current,
            primarySyncIntervalSecs: value,
          }))
        }
        onSecondarySyncIntervalChange={(value) =>
          setRoutingDraft((current) => ({
            ...current,
            secondarySyncIntervalSecs: value,
          }))
        }
        onPriorityAvailableAccountCapChange={(value) =>
          setRoutingDraft((current) => ({
            ...current,
            priorityAvailableAccountCap: value,
          }))
        }
        onClose={() => {
          setRoutingDraft(buildRoutingDraft(routing));
          setIsRoutingDialogInspectOnly(false);
          setIsRoutingDialogOpen(false);
        }}
        onSave={() => void handleSaveRouting()}
      />

      <SharedUpstreamAccountDetailDrawer
        open={upstreamAccountId != null}
        accountId={upstreamAccountId}
        initialDeleteConfirmOpen={pendingInitialDeleteConfirm}
        onInitialDeleteConfirmHandled={() =>
          setPendingInitialDeleteConfirm(false)
        }
        onClose={closeUpstreamAccount}
      />

      {bulkSyncProgressBubble}
    </div>
  );
}
