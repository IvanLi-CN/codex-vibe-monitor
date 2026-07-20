import { useEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { Button } from "../components/ui/button";
import { DateTimeRangeField } from "../components/ui/date-time-range-field";
import {
  FilterableCombobox,
  type FilterableComboboxOption,
} from "../components/ui/filterable-combobox";
import { InvocationModelFilterField } from "../components/ui/invocation-model-filter-field";
import { NumericRangeField } from "../components/ui/numeric-range-field";
import { SegmentedControl, SegmentedControlItem } from "../components/ui/segmented-control";
import { SelectField } from "../components/ui/select-field";
import { AccountDetailDrawerShell } from "../features/account-pool/AccountDetailDrawerShell";
import { InvocationRecordsSummaryCards } from "../features/records/InvocationRecordsSummaryCards";
import { InvocationRecordsTable } from "../features/records/InvocationRecordsTable";
import { RecordsNewDataButton } from "../features/records/RecordsNewDataButton";
import { AppIcon } from "../features/shared/AppIcon";
import { useCompactViewport } from "../hooks/useCompactViewport";
import { useInvocationRecords } from "../hooks/useInvocationRecords";
import { useUpstreamAccountDetailRoute } from "../hooks/useUpstreamAccountDetailRoute";
import { useTranslation } from "../i18n";
import {
  fetchInvocationRecordLocation,
  fetchInvocationSuggestions,
  type InvocationFocus,
  type InvocationModelRerouteFilter,
  type InvocationModelTarget,
  type InvocationRangePreset,
  type InvocationSortBy,
  type InvocationSortOrder,
  type InvocationSuggestionField,
  type InvocationSuggestionsResponse,
} from "../lib/api";
import { textInputAutocompleteOffProps } from "../lib/form-autocomplete";
import {
  buildInvocationSuggestionsQuery,
  createDefaultCustomRange,
  createDefaultInvocationRecordsDraft,
  type InvocationRecordsDraftFilters,
  RECORDS_PAGE_SIZE_OPTIONS,
  validateInvocationRecordsDraft,
} from "../lib/invocationRecords";
import { cn } from "../lib/utils";
import { SharedUpstreamAccountDetailDrawer } from "./account-pool/UpstreamAccounts";

const inputClassName =
  "h-9 w-full rounded-md border border-base-300/80 bg-base-100 px-3 text-sm text-base-content shadow-sm outline-none transition focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100 disabled:cursor-not-allowed disabled:opacity-60";

const SUGGESTION_DEBOUNCE_MS = 250;
const NEW_DATA_REFRESH_MIN_LOADING_MS = 600;

type ClearableRecordFilterKey = Exclude<
  keyof InvocationRecordsDraftFilters,
  "rangePreset" | "customFrom" | "customTo"
>;

interface ActiveFilterChip {
  id: string;
  label: string;
  clearKeys?: ClearableRecordFilterKey[];
}

function formatCustomRange(from: string, to: string) {
  const values = [from, to].filter(Boolean).map((value) => value.replace("T", " "));
  return values.join(" - ");
}

function formatNumericRange(min: string, max: string) {
  const normalizedMin = min.trim();
  const normalizedMax = max.trim();
  if (normalizedMin && normalizedMax) return `${normalizedMin} - ${normalizedMax}`;
  if (normalizedMin) return `>= ${normalizedMin}`;
  if (normalizedMax) return `<= ${normalizedMax}`;
  return "";
}

function formatListSummary(values: string[]) {
  const normalized = values.map((value) => value.trim()).filter(Boolean);
  if (normalized.length === 0) return "";
  if (normalized.length <= 2) return normalized.join(", ");
  return `${normalized.slice(0, 2).join(", ")} +${normalized.length - 2}`;
}

function resolveModelSuggestionField(target: InvocationModelTarget): InvocationSuggestionField {
  return target === "response" ? "responseModel" : "requestModel";
}

function parseFiniteDraftNumber(value: string) {
  const normalized = value.trim();
  if (!normalized) return null;
  const parsed = Number(normalized);
  return Number.isFinite(parsed) ? parsed : null;
}

function resolveNumericSliderMax(
  observedMax: number | null | undefined,
  fallbackStep: number,
  ...draftValues: string[]
) {
  const candidates = [
    typeof observedMax === "number" && Number.isFinite(observedMax) ? observedMax : null,
    ...draftValues.map((value) => parseFiniteDraftNumber(value)),
  ].filter((value): value is number => value != null && value >= 0);

  if (candidates.length === 0) return fallbackStep;
  return Math.max(fallbackStep, ...candidates);
}

function mapSuggestionBucketToOptions(
  items: InvocationSuggestionsResponse[keyof InvocationSuggestionsResponse]["items"] | undefined,
): FilterableComboboxOption[] {
  return (items ?? []).map((item) => ({
    value: item.value,
    label: item.label ?? item.value,
    searchText:
      item.label && item.label !== item.value ? `${item.label} ${item.value}` : item.value,
  }));
}

function getVisiblePages(currentPage: number, totalPages: number) {
  if (totalPages <= 1) return [1];
  const start = Math.max(1, currentPage - 2);
  const end = Math.min(totalPages, currentPage + 2);
  const pages: number[] = [];
  for (let page = start; page <= end; page += 1) {
    pages.push(page);
  }
  return pages;
}

export default function RecordsPage() {
  const { t } = useTranslation();
  const [searchParams] = useSearchParams();
  const requestedInvokeId =
    searchParams.get("invokeId")?.trim() || searchParams.get("requestId")?.trim() || null;
  const requestedAttemptId = searchParams.get("attemptId")?.trim() || null;
  const requestedUpstreamAccountIdRaw = searchParams.get("upstreamAccountId")?.trim() || "";
  const requestedUpstreamAccountId =
    requestedUpstreamAccountIdRaw && Number.isFinite(Number(requestedUpstreamAccountIdRaw))
      ? Number(requestedUpstreamAccountIdRaw)
      : null;
  const requestedRangePreset = searchParams.get("rangePreset") === "7d" ? "7d" : null;
  const appliedInvokeIdRef = useRef<string | null>(null);
  const requestedAttemptLocateKeyRef = useRef<string | null>(null);
  const isCompactViewport = useCompactViewport();
  const { upstreamAccountId, openUpstreamAccount, closeUpstreamAccount } =
    useUpstreamAccountDetailRoute();
  const {
    draft,
    appliedDraft,
    focus,
    page,
    pageSize,
    sortBy,
    sortOrder,
    records,
    summary,
    recordsError,
    summaryError,
    isSearching,
    isRecordsLoading,
    isSummaryLoading,
    updateDraft,
    resetDraft,
    applyDraft,
    setFocus,
    search,
    setPage,
    setPageSize,
    setSort,
  } = useInvocationRecords();
  const [autoExpandInvokeId, setAutoExpandInvokeId] = useState<string | null>(null);
  const [focusedAttemptId, setFocusedAttemptId] = useState<string | null>(null);

  const appliedSnapshotId = records?.snapshotId ?? summary?.snapshotId;
  const [suggestions, setSuggestions] = useState<InvocationSuggestionsResponse | null>(null);
  const [isSuggestionsLoading, setIsSuggestionsLoading] = useState(false);
  const [activeSuggestionField, setActiveSuggestionField] =
    useState<InvocationSuggestionField | null>(null);
  const [activeSuggestionQuery, setActiveSuggestionQuery] = useState("");
  const [isNewDataRefreshPending, setIsNewDataRefreshPending] = useState(false);
  const [cachedNewDataCount, setCachedNewDataCount] = useState(0);
  const [isFiltersOpen, setIsFiltersOpen] = useState(false);
  const [modelSearchInput, setModelSearchInput] = useState("");
  const [reasoningSearchInput, setReasoningSearchInput] = useState("");
  const newDataRefreshSeqRef = useRef(0);
  const suggestionQuery = useMemo(
    () =>
      buildInvocationSuggestionsQuery(
        draft,
        appliedSnapshotId,
        activeSuggestionField ?? undefined,
        new Date(),
        activeSuggestionField === "requestModel" ||
          activeSuggestionField === "responseModel" ||
          activeSuggestionField === "reasoningEffort"
          ? activeSuggestionQuery
          : undefined,
      ),
    [activeSuggestionField, activeSuggestionQuery, appliedSnapshotId, draft],
  );
  const suggestionsSeqRef = useRef(0);
  const customRangeTouchedRef = useRef(false);

  useEffect(() => {
    if (activeSuggestionField !== "requestModel" && activeSuggestionField !== "responseModel")
      return;
    const nextField = resolveModelSuggestionField(draft.modelTarget);
    if (nextField !== activeSuggestionField) {
      setActiveSuggestionField(nextField);
    }
  }, [activeSuggestionField, draft.modelTarget]);

  useEffect(() => {
    const requestKey = `${requestedInvokeId}:${requestedRangePreset ?? ""}`;
    if (!requestedInvokeId || appliedInvokeIdRef.current === requestKey) return;
    appliedInvokeIdRef.current = requestKey;
    setAutoExpandInvokeId(requestedInvokeId);
    setFocusedAttemptId(null);
    resetDraft();
    updateDraft("invokeId", requestedInvokeId);
    if (requestedRangePreset) {
      updateDraft("rangePreset", requestedRangePreset);
    }
    const timer = window.setTimeout(() => void search(), 0);
    return () => window.clearTimeout(timer);
  }, [requestedInvokeId, requestedRangePreset, resetDraft, search, updateDraft]);

  useEffect(() => {
    const requestKey = [
      requestedAttemptId ?? "",
      requestedUpstreamAccountId ?? "",
      requestedRangePreset ?? "",
    ].join(":");
    if (!requestedAttemptId || requestedAttemptLocateKeyRef.current === requestKey) return;
    requestedAttemptLocateKeyRef.current = requestKey;
    setAutoExpandInvokeId(null);
    setFocusedAttemptId(requestedAttemptId);
    let cancelled = false;

    void fetchInvocationRecordLocation({
      attemptId: requestedAttemptId,
      upstreamAccountId: requestedUpstreamAccountId ?? undefined,
      pageSize,
    })
      .then(async (response) => {
        if (cancelled) return;
        const resolvedInvokeId =
          response.invokeId?.trim() ||
          response.requestId?.trim() ||
          response.records[response.targetIndex]?.invokeId?.trim() ||
          "";
        if (!resolvedInvokeId) return;
        resetDraft();
        updateDraft("attemptId", requestedAttemptId);
        if (requestedRangePreset) {
          updateDraft("rangePreset", requestedRangePreset);
        }
        setAutoExpandInvokeId(resolvedInvokeId);
        setFocusedAttemptId(response.attemptId?.trim() || requestedAttemptId);
        await search();
        if (!cancelled && response.page > 1) {
          await setPage(response.page);
        }
      })
      .catch(() => {
        if (cancelled) return;
        setAutoExpandInvokeId(null);
      });

    return () => {
      cancelled = true;
    };
  }, [
    pageSize,
    requestedAttemptId,
    requestedRangePreset,
    requestedUpstreamAccountId,
    resetDraft,
    search,
    setPage,
    updateDraft,
  ]);

  useEffect(() => {
    suggestionsSeqRef.current += 1;
    setSuggestions(null);

    if (!activeSuggestionField) {
      setIsSuggestionsLoading(false);
    }
  }, [activeSuggestionField]);

  useEffect(() => {
    if (!activeSuggestionField) {
      setIsSuggestionsLoading(false);
      return;
    }

    const requestSeq = suggestionsSeqRef.current + 1;
    suggestionsSeqRef.current = requestSeq;
    setIsSuggestionsLoading(true);

    const timer = window.setTimeout(() => {
      fetchInvocationSuggestions(suggestionQuery)
        .then((response) => {
          if (requestSeq !== suggestionsSeqRef.current) return;
          setSuggestions(response);
          setIsSuggestionsLoading(false);
        })
        .catch(() => {
          if (requestSeq !== suggestionsSeqRef.current) return;
          setIsSuggestionsLoading(false);
          // Best-effort: suggestions should never block the page.
        });
    }, SUGGESTION_DEBOUNCE_MS);

    return () => window.clearTimeout(timer);
  }, [activeSuggestionField, suggestionQuery]);

  const focusOptions = useMemo(
    () => [
      { value: "token" as InvocationFocus, label: t("records.focus.token") },
      {
        value: "network" as InvocationFocus,
        label: t("records.focus.network"),
      },
      {
        value: "exception" as InvocationFocus,
        label: t("records.focus.exception"),
      },
    ],
    [t],
  );

  const rangeOptions = useMemo(
    () => [
      {
        value: "today" as InvocationRangePreset,
        label: t("records.filters.rangePreset.today"),
      },
      {
        value: "1d" as InvocationRangePreset,
        label: t("records.filters.rangePreset.lastDay"),
      },
      {
        value: "7d" as InvocationRangePreset,
        label: t("records.filters.rangePreset.last7Days"),
      },
      {
        value: "30d" as InvocationRangePreset,
        label: t("records.filters.rangePreset.last30Days"),
      },
      {
        value: "custom" as InvocationRangePreset,
        label: t("records.filters.rangePreset.custom"),
      },
    ],
    [t],
  );

  const sortOptions = useMemo(
    () => [
      {
        value: "occurredAt" as InvocationSortBy,
        label: t("records.list.sort.occurredAt"),
      },
      {
        value: "totalTokens" as InvocationSortBy,
        label: t("records.list.sort.totalTokens"),
      },
      { value: "cost" as InvocationSortBy, label: t("records.list.sort.cost") },
      {
        value: "tTotalMs" as InvocationSortBy,
        label: t("records.list.sort.totalMs"),
      },
      {
        value: "tUpstreamTtfbMs" as InvocationSortBy,
        label: t("records.list.sort.ttfb"),
      },
      {
        value: "status" as InvocationSortBy,
        label: t("records.list.sort.status"),
      },
    ],
    [t],
  );

  const upstreamScopeOptions = useMemo(
    () => [
      { value: "", label: t("records.filters.upstreamScope.all") },
      { value: "internal", label: t("records.filters.upstreamScope.internal") },
      { value: "external", label: t("records.filters.upstreamScope.external") },
    ],
    [t],
  );

  const transportOptions = useMemo(
    () => [
      { value: "", label: t("records.filters.transport.all") },
      { value: "http", label: t("records.filters.transport.http") },
      { value: "websocket", label: t("records.filters.transport.websocket") },
    ],
    [t],
  );

  const total = records?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / pageSize) || 1);
  const visiblePages = getVisiblePages(page, totalPages);
  const visibleSummary = summary && summary.snapshotId === records?.snapshotId ? summary : null;
  const newRecordsCount = visibleSummary?.newRecordsCount ?? 0;
  const isNewDataLoading = isNewDataRefreshPending;
  const displayNewDataCount = newRecordsCount > 0 ? newRecordsCount : cachedNewDataCount;
  const shouldShowNewDataButton =
    (!isSearching || isNewDataRefreshPending) &&
    (newRecordsCount > 0 || (isNewDataLoading && displayNewDataCount > 0));
  const tableLoading = isRecordsLoading;
  const listControlsDisabled = isSearching || isRecordsLoading;
  const hasOpenSuggestion = activeSuggestionField !== null;
  const requestModelBucket = suggestions?.requestModel;
  const responseModelBucket = suggestions?.responseModel;
  const endpointBucket = suggestions?.endpoint;
  const failureKindBucket = suggestions?.failureKind;
  const promptCacheKeyBucket = suggestions?.promptCacheKey;
  const proxyDisplayNameBucket = suggestions?.proxyDisplayName;
  const upstreamAccountBucket = suggestions?.upstreamAccount;
  const requesterIpBucket = suggestions?.requesterIp;
  const serviceTierBucket = suggestions?.serviceTier;
  const reasoningEffortBucket = suggestions?.reasoningEffort;
  const draftValidation = validateInvocationRecordsDraft(draft);
  const hasDraftValidationErrors = Object.values(draftValidation).some((value) => value !== null);
  const timeRangeError =
    draftValidation.timeRange === "invalid"
      ? t("records.filters.validation.timeRange.invalid")
      : draftValidation.timeRange === "order"
        ? t("records.filters.validation.timeRange.order")
        : null;
  const totalTokensRangeError =
    draftValidation.totalTokens === "invalid"
      ? t("records.filters.validation.totalTokens.invalid")
      : draftValidation.totalTokens === "integer"
        ? t("records.filters.validation.totalTokens.integer")
        : draftValidation.totalTokens === "order"
          ? t("records.filters.validation.totalTokens.order")
          : null;
  const totalMsRangeError =
    draftValidation.totalMs === "invalid"
      ? t("records.filters.validation.totalMs.invalid")
      : draftValidation.totalMs === "order"
        ? t("records.filters.validation.totalMs.order")
        : null;
  const modelFilterError =
    draftValidation.modelFilters === "missingModel"
      ? t("records.filters.validation.modelFilters.missingModel")
      : null;
  const totalTokensSliderMax = resolveNumericSliderMax(
    visibleSummary?.token.maxTokensPerRequest,
    1,
    draft.minTotalTokens,
    draft.maxTotalTokens,
  );
  const totalMsSliderMax = resolveNumericSliderMax(
    visibleSummary?.network.maxTotalMs,
    0.1,
    draft.minTotalMs,
    draft.maxTotalMs,
  );

  const modelOptions = useMemo(
    () =>
      mapSuggestionBucketToOptions(
        (draft.modelTarget === "response" ? responseModelBucket : requestModelBucket)?.items,
      ),
    [draft.modelTarget, requestModelBucket?.items, responseModelBucket?.items],
  );
  const endpointOptions = useMemo(
    () => mapSuggestionBucketToOptions(endpointBucket?.items),
    [endpointBucket?.items],
  );
  const failureKindOptions = useMemo(
    () => mapSuggestionBucketToOptions(failureKindBucket?.items),
    [failureKindBucket?.items],
  );
  const promptCacheKeyOptions = useMemo(
    () => mapSuggestionBucketToOptions(promptCacheKeyBucket?.items),
    [promptCacheKeyBucket?.items],
  );
  const proxyDisplayNameOptions = useMemo(
    () => mapSuggestionBucketToOptions(proxyDisplayNameBucket?.items),
    [proxyDisplayNameBucket?.items],
  );
  const upstreamAccountOptions = useMemo(
    () => mapSuggestionBucketToOptions(upstreamAccountBucket?.items),
    [upstreamAccountBucket?.items],
  );
  const requesterIpOptions = useMemo(
    () => mapSuggestionBucketToOptions(requesterIpBucket?.items),
    [requesterIpBucket?.items],
  );
  const serviceTierOptions = useMemo(
    () => mapSuggestionBucketToOptions(serviceTierBucket?.items),
    [serviceTierBucket?.items],
  );
  const reasoningEffortOptions = useMemo(
    () => mapSuggestionBucketToOptions(reasoningEffortBucket?.items),
    [reasoningEffortBucket?.items],
  );

  const activeFilterChips = useMemo<ActiveFilterChip[]>(() => {
    if (!appliedDraft) return [];

    const rangeLabel =
      appliedDraft.rangePreset === "custom"
        ? formatCustomRange(appliedDraft.customFrom, appliedDraft.customTo) ||
          t("records.filters.rangePreset.custom")
        : (rangeOptions.find((option) => option.value === appliedDraft.rangePreset)?.label ??
          t("records.filters.rangePreset"));
    const chips: ActiveFilterChip[] = [
      {
        id: "range",
        label: `${t("records.filters.rangePreset")}: ${rangeLabel}`,
      },
    ];
    const add = (draftKey: ClearableRecordFilterKey, label: string, value: string) => {
      const normalized = value.trim();
      if (!normalized) return;
      chips.push({
        id: draftKey,
        clearKeys: [draftKey],
        label: `${label}: ${normalized}`,
      });
    };
    const addRange = (
      id: string,
      label: string,
      value: string,
      clearKeys: ClearableRecordFilterKey[],
    ) => {
      const normalized = value.trim();
      if (!normalized) return;
      chips.push({ id, clearKeys, label: `${label}: ${normalized}` });
    };

    const statusLabels: Record<string, string> = {
      success: t("records.filters.status.success"),
      warning_success: t("records.filters.status.warningSuccess"),
      failed: t("records.filters.status.failed"),
      interrupted: t("records.filters.status.interrupted"),
      running: t("records.filters.status.running"),
      pending: t("records.filters.status.pending"),
    };
    const failureClassLabels: Record<string, string> = {
      service_failure: t("records.filters.failureClass.service"),
      client_failure: t("records.filters.failureClass.client"),
      client_abort: t("records.filters.failureClass.abort"),
    };
    const upstreamScopeLabels: Record<string, string> = {
      internal: t("records.filters.upstreamScope.internal"),
      external: t("records.filters.upstreamScope.external"),
    };
    const transportLabels: Record<string, string> = {
      http: t("records.filters.transport.http"),
      websocket: t("records.filters.transport.websocket"),
    };
    const modelTargetLabels: Record<InvocationModelTarget, string> = {
      request: t("records.filters.modelTarget.request"),
      response: t("records.filters.modelTarget.response"),
    };
    const modelReroutedLabels: Record<Exclude<InvocationModelRerouteFilter, "all">, string> = {
      rerouted: t("records.filters.modelRerouted.rerouted"),
      notRerouted: t("records.filters.modelRerouted.notRerouted"),
    };
    const appliedModels =
      appliedDraft.models.length > 0
        ? appliedDraft.models
        : appliedDraft.model.trim()
          ? [appliedDraft.model.trim()]
          : [];
    const appliedReasoningEfforts =
      appliedDraft.reasoningEfforts.length > 0
        ? appliedDraft.reasoningEfforts
        : appliedDraft.reasoningEffort.trim()
          ? [appliedDraft.reasoningEffort.trim()]
          : [];
    const modelFilterSummaryParts = [
      appliedModels.length > 0 ? modelTargetLabels[appliedDraft.modelTarget] : null,
      appliedModels.length > 0 ? formatListSummary(appliedModels) : null,
      appliedReasoningEfforts.length > 0
        ? `${t("records.filters.reasoningEffort")}: ${formatListSummary(appliedReasoningEfforts)}`
        : null,
      appliedDraft.modelRerouted !== "all" ? modelReroutedLabels[appliedDraft.modelRerouted] : null,
    ].filter((value): value is string => Boolean(value));

    add(
      "status",
      t("records.filters.status"),
      statusLabels[appliedDraft.status] ?? appliedDraft.status,
    );
    if (modelFilterSummaryParts.length > 0) {
      chips.push({
        id: "modelSelection",
        clearKeys: [
          "model",
          "models",
          "modelTarget",
          "modelRerouted",
          "reasoningEffort",
          "reasoningEfforts",
        ],
        label: `${t("records.filters.model")}: ${modelFilterSummaryParts.join(" · ")}`,
      });
    }
    add("endpoint", t("records.filters.endpoint"), appliedDraft.endpoint);
    add(
      "failureClass",
      t("records.filters.failureClass"),
      failureClassLabels[appliedDraft.failureClass] ?? appliedDraft.failureClass,
    );
    add("invokeId", t("records.filters.invokeId"), appliedDraft.invokeId);
    add("attemptId", t("records.filters.attemptId"), appliedDraft.attemptId);
    add("failureKind", t("records.filters.failureKind"), appliedDraft.failureKind);
    add("promptCacheKey", t("records.filters.promptCacheKey"), appliedDraft.promptCacheKey);
    add(
      "upstreamScope",
      t("records.filters.upstreamScope"),
      upstreamScopeLabels[appliedDraft.upstreamScope] ?? appliedDraft.upstreamScope,
    );
    if (appliedDraft.upstreamAccount.trim()) {
      chips.push({
        id: "upstreamAccount",
        clearKeys: ["upstreamAccount", "upstreamAccountId"],
        label: `${t("records.filters.upstreamAccount")}: ${appliedDraft.upstreamAccount.trim()}`,
      });
    }
    add(
      "transport",
      t("records.filters.transport"),
      transportLabels[appliedDraft.transport] ?? appliedDraft.transport,
    );
    add("proxyDisplayName", t("records.filters.proxyDisplayName"), appliedDraft.proxyDisplayName);
    add("serviceTier", t("records.filters.serviceTier"), appliedDraft.serviceTier);
    add("requesterIp", t("records.filters.requesterIp"), appliedDraft.requesterIp);
    add("keyword", t("records.filters.keyword"), appliedDraft.keyword);
    addRange(
      "totalTokensRange",
      t("records.filters.totalTokensRange"),
      formatNumericRange(appliedDraft.minTotalTokens, appliedDraft.maxTotalTokens),
      ["minTotalTokens", "maxTotalTokens"],
    );
    addRange(
      "totalMsRange",
      t("records.filters.totalMsRange"),
      formatNumericRange(appliedDraft.minTotalMs, appliedDraft.maxTotalMs),
      ["minTotalMs", "maxTotalMs"],
    );

    return chips;
  }, [appliedDraft, rangeOptions, t]);

  const handleClearDraft = () => {
    customRangeTouchedRef.current = false;
    setModelSearchInput("");
    setReasoningSearchInput("");
    setActiveSuggestionQuery("");
    setActiveSuggestionField(null);
    resetDraft();
  };

  const handleTimeRangeChange = (next: {
    preset: InvocationRangePreset;
    from: string;
    to: string;
  }) => {
    let nextFrom = next.from;
    let nextTo = next.to;
    if (
      next.preset === "custom" &&
      draft.rangePreset !== "custom" &&
      !customRangeTouchedRef.current &&
      !next.from &&
      !next.to
    ) {
      const nextRange = createDefaultCustomRange();
      nextFrom = nextRange.customFrom;
      nextTo = nextRange.customTo;
    }
    if (next.preset === "custom" && (nextFrom !== draft.customFrom || nextTo !== draft.customTo)) {
      customRangeTouchedRef.current = true;
    }
    updateDraft("rangePreset", next.preset);
    updateDraft("customFrom", nextFrom);
    updateDraft("customTo", nextTo);
  };

  const handleUpstreamAccountChange = (nextValue: string) => {
    updateDraft("upstreamAccount", nextValue);
    const normalized = nextValue.trim();
    const matched = upstreamAccountOptions.find((option) => {
      const label = option.label?.trim() || option.value;
      return label === normalized || option.value === normalized;
    });
    if (matched) {
      updateDraft("upstreamAccountId", matched.value);
      return;
    }
    updateDraft("upstreamAccountId", /^\d+$/.test(normalized) ? normalized : "");
  };

  const handleUpstreamAccountSelect = (option: FilterableComboboxOption) => {
    updateDraft("upstreamAccount", option.label?.trim() || option.value);
    updateDraft("upstreamAccountId", option.value);
  };

  useEffect(() => {
    if (newRecordsCount > 0) {
      setCachedNewDataCount(newRecordsCount);
      return;
    }

    if (!isNewDataLoading) {
      setCachedNewDataCount(0);
    }
  }, [isNewDataLoading, newRecordsCount]);

  const handleSearch = () => {
    newDataRefreshSeqRef.current += 1;
    setIsNewDataRefreshPending(false);
    void search();
  };

  const closeFilters = () => {
    setIsFiltersOpen(false);
    setActiveSuggestionField(null);
    setActiveSuggestionQuery("");
    setModelSearchInput("");
    setReasoningSearchInput("");
  };

  const handleApplyFilters = () => {
    closeFilters();
    handleSearch();
  };

  const handleRemoveActiveFilter = (clearKeys: ClearableRecordFilterKey[]) => {
    const nextDraft = { ...(appliedDraft ?? draft) };
    const defaults = createDefaultInvocationRecordsDraft();
    const resetDraftField = <K extends ClearableRecordFilterKey>(key: K) => {
      nextDraft[key] = defaults[key];
    };
    for (const key of clearKeys) {
      resetDraftField(key);
    }
    newDataRefreshSeqRef.current += 1;
    setIsNewDataRefreshPending(false);
    void applyDraft(nextDraft);
  };

  const handleModelSuggestionOpenChange = (open: boolean) => {
    const field = resolveModelSuggestionField(draft.modelTarget);
    setActiveSuggestionField((current) => {
      if (open) return field;
      return current === field ? null : current;
    });
    setActiveSuggestionQuery(open ? modelSearchInput : "");
  };

  const handleReasoningSuggestionOpenChange = (open: boolean) => {
    setActiveSuggestionField((current) => {
      if (open) return "reasoningEffort";
      return current === "reasoningEffort" ? null : current;
    });
    setActiveSuggestionQuery(open ? reasoningSearchInput : "");
  };

  const handleModelSearchInputChange = (nextValue: string) => {
    setModelSearchInput(nextValue);
    if (activeSuggestionField === "requestModel" || activeSuggestionField === "responseModel") {
      setActiveSuggestionQuery(nextValue);
    }
  };

  const handleReasoningSearchInputChange = (nextValue: string) => {
    setReasoningSearchInput(nextValue);
    if (activeSuggestionField === "reasoningEffort") {
      setActiveSuggestionQuery(nextValue);
    }
  };

  const handleModelFilterChange = (next: {
    modelTarget: InvocationModelTarget;
    modelRerouted: InvocationModelRerouteFilter;
    models: string[];
    reasoningEfforts: string[];
  }) => {
    updateDraft("modelTarget", next.modelTarget);
    updateDraft("modelRerouted", next.modelRerouted);
    updateDraft("models", next.models);
    updateDraft("reasoningEfforts", next.reasoningEfforts);
  };

  const handleRefreshNewData = () => {
    if (isNewDataLoading) return;
    const refreshSeq = newDataRefreshSeqRef.current + 1;
    newDataRefreshSeqRef.current = refreshSeq;
    setIsNewDataRefreshPending(true);
    const minLoadingDelay = new Promise<void>((resolve) => {
      window.setTimeout(resolve, NEW_DATA_REFRESH_MIN_LOADING_MS);
    });

    void Promise.all([
      search({ source: "applied", preserveSummary: true }),
      minLoadingDelay,
    ]).finally(() => {
      if (newDataRefreshSeqRef.current === refreshSeq) {
        setIsNewDataRefreshPending(false);
      }
    });
  };

  const handleSuggestionOpenChange = (field: InvocationSuggestionField) => (open: boolean) => {
    setActiveSuggestionField((current) => {
      if (open) return field;
      return current === field ? null : current;
    });
    if (!open) {
      setActiveSuggestionQuery("");
    }
  };

  const handleSortByChange = (value: InvocationSortBy) => {
    void setSort(value, sortOrder);
  };

  const handleSortOrderChange = (value: InvocationSortOrder) => {
    void setSort(sortBy, value);
  };

  if (isCompactViewport && upstreamAccountId != null) {
    return (
      <div className="mx-auto flex w-full max-w-full flex-col gap-6">
        <SharedUpstreamAccountDetailDrawer
          open
          presentation="page"
          accountId={upstreamAccountId}
          onClose={closeUpstreamAccount}
        />
      </div>
    );
  }

  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
      <section className="surface-panel" data-testid="records-filters-panel">
        <div className="surface-panel-body gap-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="section-heading">
              <h1 className="section-title">{t("records.title")}</h1>
              <p className="section-description">{t("records.subtitle")}</p>
            </div>
            <Button
              type="button"
              variant="outline"
              onClick={() => setIsFiltersOpen(true)}
              data-testid="records-open-filters"
              aria-label={t("records.filters.openAria")}
            >
              <AppIcon name="tag-outline" className="h-4 w-4" aria-hidden />
              <span>{t("records.filters.open")}</span>
              <span className="min-w-5 rounded-full bg-base-200 px-1.5 py-0.5 text-center text-xs tabular-nums text-base-content/70">
                {activeFilterChips.length}
              </span>
            </Button>
          </div>

          <div
            className="flex flex-wrap gap-2"
            data-testid="records-active-filters"
            aria-label={t("records.filters.active")}
          >
            {activeFilterChips.map((chip) =>
              chip.clearKeys?.length ? (
                <button
                  key={chip.id}
                  type="button"
                  onClick={() => handleRemoveActiveFilter(chip.clearKeys!)}
                  data-testid={`records-active-filter-${chip.id}`}
                  className="inline-flex max-w-full items-center gap-1.5 rounded-full border border-base-300/80 bg-base-100 px-2.5 py-1 text-left text-xs font-medium text-base-content transition hover:border-primary/45 hover:bg-primary/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                  aria-label={t("records.filters.remove", {
                    label: chip.label,
                  })}
                >
                  <span className="truncate">{chip.label}</span>
                  <AppIcon name="close" className="h-3.5 w-3.5 shrink-0" aria-hidden />
                </button>
              ) : (
                <button
                  key={chip.id}
                  type="button"
                  onClick={() => setIsFiltersOpen(true)}
                  data-testid={`records-active-filter-${chip.id}`}
                  className="inline-flex max-w-full items-center rounded-full border border-base-300/80 bg-base-100 px-2.5 py-1 text-left text-xs font-medium text-base-content transition hover:border-primary/45 hover:bg-primary/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                >
                  <span className="truncate">{chip.label}</span>
                </button>
              ),
            )}
          </div>
        </div>
      </section>

      <AccountDetailDrawerShell
        open={isFiltersOpen}
        labelledBy="records-filters-drawer-title"
        closeLabel={t("records.filters.close")}
        onClose={closeFilters}
        shellClassName="desktop:w-[min(34rem,42vw)]"
        header={
          <div className="section-heading">
            <h2 id="records-filters-drawer-title" className="section-title text-base">
              {t("records.filters.title")}
            </h2>
          </div>
        }
      >
        <div
          className="flex min-h-full flex-col gap-5"
          data-testid="records-filters-drawer"
          data-suggestions-open={hasOpenSuggestion ? "true" : "false"}
        >
          <div className="space-y-4">
            <section className="rounded-2xl border border-base-300/70 bg-base-100/35 p-4">
              <div className="mb-3">
                <h3 className="text-sm font-semibold text-base-content">
                  {t("records.filters.groups.range")}
                </h3>
              </div>
              <div className="grid gap-4">
                <DateTimeRangeField
                  label={t("records.filters.rangePreset")}
                  testId="records-filter-time-range"
                  customPresetValue="custom"
                  value={{
                    preset: draft.rangePreset,
                    from: draft.customFrom,
                    to: draft.customTo,
                  }}
                  options={rangeOptions}
                  summary={
                    draft.rangePreset === "custom"
                      ? formatCustomRange(draft.customFrom, draft.customTo) ||
                        t("records.filters.rangePreset.custom")
                      : (rangeOptions.find((option) => option.value === draft.rangePreset)?.label ??
                        t("records.filters.rangePreset"))
                  }
                  fromLabel={t("records.filters.from")}
                  toLabel={t("records.filters.to")}
                  fromName="customFrom"
                  toName="customTo"
                  error={timeRangeError}
                  onChange={handleTimeRangeChange}
                />
                <div className="grid gap-4 min-[769px]:grid-cols-2">
                  <NumericRangeField
                    label={t("records.filters.totalTokensRange")}
                    testId="records-filter-total-tokens-range"
                    surface="embedded"
                    sliderMin={0}
                    sliderMax={totalTokensSliderMax}
                    minAriaLabel={t("records.filters.totalTokensRange.min")}
                    maxAriaLabel={t("records.filters.totalTokensRange.max")}
                    unitLabel="TOKENS"
                    step={1}
                    minValue={draft.minTotalTokens}
                    maxValue={draft.maxTotalTokens}
                    error={totalTokensRangeError}
                    onChange={(next) => {
                      updateDraft("minTotalTokens", next.minValue);
                      updateDraft("maxTotalTokens", next.maxValue);
                    }}
                  />
                  <NumericRangeField
                    label={t("records.filters.totalMsRange")}
                    testId="records-filter-total-ms-range"
                    surface="embedded"
                    sliderMin={0}
                    sliderMax={totalMsSliderMax}
                    minAriaLabel={t("records.filters.totalMsRange.min")}
                    maxAriaLabel={t("records.filters.totalMsRange.max")}
                    unitLabel="MS"
                    step={0.1}
                    minValue={draft.minTotalMs}
                    maxValue={draft.maxTotalMs}
                    error={totalMsRangeError}
                    onChange={(next) => {
                      updateDraft("minTotalMs", next.minValue);
                      updateDraft("maxTotalMs", next.maxValue);
                    }}
                  />
                </div>
              </div>
            </section>

            <section className="rounded-2xl border border-base-300/70 bg-base-100/35 p-4">
              <div className="mb-3">
                <h3 className="text-sm font-semibold text-base-content">
                  {t("records.filters.groups.requestContext")}
                </h3>
              </div>
              <div className="grid gap-4 min-[769px]:grid-cols-2">
                <label className="field">
                  <span className="field-label">{t("records.filters.invokeId")}</span>
                  <input
                    {...textInputAutocompleteOffProps}
                    name="invokeId"
                    className={inputClassName}
                    value={draft.invokeId}
                    onChange={(event) => updateDraft("invokeId", event.target.value)}
                  />
                </label>
                <label className="field">
                  <span className="field-label">{t("records.filters.attemptId")}</span>
                  <input
                    {...textInputAutocompleteOffProps}
                    name="attemptId"
                    className={inputClassName}
                    value={draft.attemptId}
                    onChange={(event) => updateDraft("attemptId", event.target.value)}
                  />
                </label>
                <label className="field">
                  <span className="field-label">{t("records.filters.keyword")}</span>
                  <input
                    {...textInputAutocompleteOffProps}
                    name="keyword"
                    className={inputClassName}
                    value={draft.keyword}
                    onChange={(event) => updateDraft("keyword", event.target.value)}
                  />
                </label>
                <InvocationModelFilterField
                  className="min-[769px]:col-span-2"
                  testId="records-filter-model-selection"
                  label={t("records.filters.model")}
                  hint={t("records.filters.modelHint")}
                  value={{
                    modelTarget: draft.modelTarget,
                    modelRerouted: draft.modelRerouted,
                    models: draft.models,
                    reasoningEfforts: draft.reasoningEfforts,
                  }}
                  onChange={handleModelFilterChange}
                  modelLabel={t("records.filters.model")}
                  reasoningEffortLabel={t("records.filters.reasoningEffort")}
                  modelTargetLabel={t("records.filters.modelTarget")}
                  requestTargetLabel={t("records.filters.modelTarget.request")}
                  responseTargetLabel={t("records.filters.modelTarget.response")}
                  reroutedLabel={t("records.filters.modelRerouted")}
                  reroutedAllLabel={t("records.filters.modelRerouted.all")}
                  reroutedOnlyLabel={t("records.filters.modelRerouted.rerouted")}
                  notReroutedLabel={t("records.filters.modelRerouted.notRerouted")}
                  modelInputValue={modelSearchInput}
                  onModelInputValueChange={handleModelSearchInputChange}
                  modelOptions={modelOptions}
                  modelPlaceholder={t("records.filters.modelPlaceholder")}
                  reasoningEffortInputValue={reasoningSearchInput}
                  onReasoningEffortInputValueChange={handleReasoningSearchInputChange}
                  reasoningEffortOptions={reasoningEffortOptions}
                  reasoningEffortPlaceholder={t("records.filters.reasoningEffortPlaceholder")}
                  emptyText={t("records.filters.noMatches")}
                  loadingText={t("records.filters.searching")}
                  addLabel={t("records.filters.multiValue.add")}
                  modelLoading={
                    isSuggestionsLoading &&
                    (activeSuggestionField === "requestModel" ||
                      activeSuggestionField === "responseModel")
                  }
                  reasoningEffortLoading={
                    isSuggestionsLoading && activeSuggestionField === "reasoningEffort"
                  }
                  error={modelFilterError}
                  modelInputId="records-filter-model-input"
                  reasoningEffortInputId="records-filter-reasoning-effort-input"
                  onModelOpenChange={handleModelSuggestionOpenChange}
                  onReasoningEffortOpenChange={handleReasoningSuggestionOpenChange}
                  inputAutocompleteProps={textInputAutocompleteOffProps}
                />
                <label className="field">
                  <span className="field-label">{t("records.filters.endpoint")}</span>
                  <FilterableCombobox
                    label={t("records.filters.endpoint")}
                    name="endpoint"
                    id="records-filter-endpoint"
                    value={draft.endpoint}
                    onValueChange={(next) => updateDraft("endpoint", next)}
                    options={endpointOptions}
                    placeholder={t("records.filters.any")}
                    emptyText={t("records.filters.noMatches")}
                    loading={isSuggestionsLoading && activeSuggestionField === "endpoint"}
                    loadingText={t("records.filters.searching")}
                    inputClassName={inputClassName}
                    onOpenChange={handleSuggestionOpenChange("endpoint")}
                  />
                </label>
                <label className="field">
                  <span className="field-label">{t("records.filters.promptCacheKey")}</span>
                  <FilterableCombobox
                    label={t("records.filters.promptCacheKey")}
                    name="promptCacheKey"
                    id="records-filter-prompt-cache-key"
                    value={draft.promptCacheKey}
                    onValueChange={(next) => updateDraft("promptCacheKey", next)}
                    options={promptCacheKeyOptions}
                    placeholder={t("records.filters.any")}
                    emptyText={t("records.filters.noMatches")}
                    loading={isSuggestionsLoading && activeSuggestionField === "promptCacheKey"}
                    loadingText={t("records.filters.searching")}
                    inputClassName={inputClassName}
                    onOpenChange={handleSuggestionOpenChange("promptCacheKey")}
                  />
                </label>
              </div>
            </section>

            <section className="rounded-2xl border border-base-300/70 bg-base-100/35 p-4">
              <div className="mb-3">
                <h3 className="text-sm font-semibold text-base-content">
                  {t("records.filters.groups.routing")}
                </h3>
              </div>
              <div className="grid gap-4 min-[769px]:grid-cols-2">
                <SelectField
                  className="field"
                  label={t("records.filters.upstreamScope")}
                  name="upstreamScope"
                  value={draft.upstreamScope}
                  options={upstreamScopeOptions}
                  onValueChange={(value) => updateDraft("upstreamScope", value)}
                />
                <label className="field">
                  <span className="field-label">{t("records.filters.upstreamAccount")}</span>
                  <FilterableCombobox
                    label={t("records.filters.upstreamAccount")}
                    name="upstreamAccount"
                    id="records-filter-upstream-account"
                    value={draft.upstreamAccount}
                    onValueChange={handleUpstreamAccountChange}
                    onOptionSelect={handleUpstreamAccountSelect}
                    options={upstreamAccountOptions}
                    placeholder={t("records.filters.any")}
                    emptyText={t("records.filters.noMatches")}
                    loading={isSuggestionsLoading && activeSuggestionField === "upstreamAccount"}
                    loadingText={t("records.filters.searching")}
                    inputClassName={inputClassName}
                    onOpenChange={handleSuggestionOpenChange("upstreamAccount")}
                  />
                </label>
                <label className="field">
                  <span className="field-label">{t("records.filters.proxyDisplayName")}</span>
                  <FilterableCombobox
                    label={t("records.filters.proxyDisplayName")}
                    name="proxyDisplayName"
                    id="records-filter-proxy-display-name"
                    value={draft.proxyDisplayName}
                    onValueChange={(next) => updateDraft("proxyDisplayName", next)}
                    options={proxyDisplayNameOptions}
                    placeholder={t("records.filters.any")}
                    emptyText={t("records.filters.noMatches")}
                    loading={isSuggestionsLoading && activeSuggestionField === "proxyDisplayName"}
                    loadingText={t("records.filters.searching")}
                    inputClassName={inputClassName}
                    onOpenChange={handleSuggestionOpenChange("proxyDisplayName")}
                  />
                </label>
                <SelectField
                  className="field"
                  label={t("records.filters.transport")}
                  name="transport"
                  value={draft.transport}
                  options={transportOptions}
                  onValueChange={(value) => updateDraft("transport", value)}
                />
                <label className="field">
                  <span className="field-label">{t("records.filters.serviceTier")}</span>
                  <FilterableCombobox
                    label={t("records.filters.serviceTier")}
                    name="serviceTier"
                    id="records-filter-service-tier"
                    value={draft.serviceTier}
                    onValueChange={(next) => updateDraft("serviceTier", next)}
                    options={serviceTierOptions}
                    placeholder={t("records.filters.any")}
                    emptyText={t("records.filters.noMatches")}
                    loading={isSuggestionsLoading && activeSuggestionField === "serviceTier"}
                    loadingText={t("records.filters.searching")}
                    inputClassName={inputClassName}
                    onOpenChange={handleSuggestionOpenChange("serviceTier")}
                  />
                </label>
              </div>
            </section>

            <section className="rounded-2xl border border-base-300/70 bg-base-100/35 p-4">
              <div className="mb-3">
                <h3 className="text-sm font-semibold text-base-content">
                  {t("records.filters.groups.result")}
                </h3>
              </div>
              <div className="grid gap-4 min-[769px]:grid-cols-2">
                <SelectField
                  className="field"
                  label={t("records.filters.status")}
                  name="status"
                  value={draft.status}
                  options={[
                    { value: "", label: t("records.filters.status.all") },
                    {
                      value: "success",
                      label: t("records.filters.status.success"),
                    },
                    {
                      value: "warning_success",
                      label: t("records.filters.status.warningSuccess"),
                    },
                    {
                      value: "failed",
                      label: t("records.filters.status.failed"),
                    },
                    {
                      value: "interrupted",
                      label: t("records.filters.status.interrupted"),
                    },
                    {
                      value: "running",
                      label: t("records.filters.status.running"),
                    },
                    {
                      value: "pending",
                      label: t("records.filters.status.pending"),
                    },
                  ]}
                  onValueChange={(value) => updateDraft("status", value)}
                />
                <SelectField
                  className="field"
                  label={t("records.filters.failureClass")}
                  name="failureClass"
                  value={draft.failureClass}
                  options={[
                    { value: "", label: t("records.filters.failureClass.all") },
                    {
                      value: "service_failure",
                      label: t("records.filters.failureClass.service"),
                    },
                    {
                      value: "client_failure",
                      label: t("records.filters.failureClass.client"),
                    },
                    {
                      value: "client_abort",
                      label: t("records.filters.failureClass.abort"),
                    },
                  ]}
                  onValueChange={(value) => updateDraft("failureClass", value)}
                />
                <label className="field">
                  <span className="field-label">{t("records.filters.failureKind")}</span>
                  <FilterableCombobox
                    label={t("records.filters.failureKind")}
                    name="failureKind"
                    id="records-filter-failure-kind"
                    value={draft.failureKind}
                    onValueChange={(next) => updateDraft("failureKind", next)}
                    options={failureKindOptions}
                    placeholder={t("records.filters.any")}
                    emptyText={t("records.filters.noMatches")}
                    loading={isSuggestionsLoading && activeSuggestionField === "failureKind"}
                    loadingText={t("records.filters.searching")}
                    inputClassName={inputClassName}
                    onOpenChange={handleSuggestionOpenChange("failureKind")}
                  />
                </label>
                <label className="field">
                  <span className="field-label">{t("records.filters.requesterIp")}</span>
                  <FilterableCombobox
                    label={t("records.filters.requesterIp")}
                    name="requesterIp"
                    id="records-filter-requester-ip"
                    value={draft.requesterIp}
                    onValueChange={(next) => updateDraft("requesterIp", next)}
                    options={requesterIpOptions}
                    placeholder={t("records.filters.any")}
                    emptyText={t("records.filters.noMatches")}
                    loading={isSuggestionsLoading && activeSuggestionField === "requesterIp"}
                    loadingText={t("records.filters.searching")}
                    inputClassName={inputClassName}
                    onOpenChange={handleSuggestionOpenChange("requesterIp")}
                  />
                </label>
              </div>
            </section>
          </div>
          <div className="sticky bottom-[-1rem] -mx-4 mt-auto flex flex-col-reverse gap-2 border-t border-base-300/70 bg-base-100 px-4 pb-[max(1rem,env(safe-area-inset-bottom))] pt-3 sm:flex-row sm:justify-end desktop:bottom-[-1.5rem] desktop:-mx-6 desktop:px-6">
            <Button type="button" variant="ghost" onClick={handleClearDraft} disabled={isSearching}>
              {t("records.filters.clearDraft")}
            </Button>
            <Button
              type="button"
              onClick={handleApplyFilters}
              disabled={isSearching || hasDraftValidationErrors}
            >
              {isSearching ? t("records.filters.searching") : t("records.filters.apply")}
            </Button>
          </div>
        </div>
      </AccountDetailDrawerShell>

      <section className="surface-panel" data-testid="records-summary-panel">
        <div className="surface-panel-body gap-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="section-heading">
              <h2 className="section-title">{t("records.summary.title")}</h2>
              <p className="section-description">{t("records.summary.description")}</p>
            </div>
            <div className="flex flex-wrap items-center gap-3">
              {shouldShowNewDataButton ? (
                <RecordsNewDataButton
                  count={displayNewDataCount}
                  isLoading={isNewDataLoading}
                  onRefresh={handleRefreshNewData}
                />
              ) : null}
              <SegmentedControl role="tablist" aria-label={t("records.focus.label")}>
                {focusOptions.map((option) => (
                  <SegmentedControlItem
                    key={option.value}
                    active={focus === option.value}
                    role="tab"
                    aria-selected={focus === option.value}
                    aria-pressed={focus === option.value}
                    onClick={() => setFocus(option.value)}
                  >
                    {option.label}
                  </SegmentedControlItem>
                ))}
              </SegmentedControl>
            </div>
          </div>

          <InvocationRecordsSummaryCards
            focus={focus}
            summary={visibleSummary}
            isLoading={isSummaryLoading}
            error={summaryError}
          />
        </div>
      </section>

      <section className="surface-panel">
        <div className="surface-panel-body gap-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="section-heading">
              <h2 className="section-title">{t("records.list.title")}</h2>
              <p className="section-description">{t("records.list.description")}</p>
            </div>
            <div className="flex flex-wrap items-end gap-3">
              <div className="rounded-full border border-base-300/70 bg-base-100/55 px-3 py-2 text-sm font-medium text-base-content/80">
                {t("records.list.totalCount", { count: total })}
              </div>
              <SelectField
                className="min-w-[7rem]"
                label={t("records.list.pageSize")}
                name="pageSize"
                size="sm"
                value={String(pageSize)}
                disabled={listControlsDisabled}
                options={RECORDS_PAGE_SIZE_OPTIONS.map((value) => ({
                  value: String(value),
                  label: String(value),
                }))}
                onValueChange={(value) => void setPageSize(Number(value))}
              />
              <SelectField
                className="min-w-[10rem]"
                label={t("records.list.sortBy")}
                name="sortBy"
                size="sm"
                value={sortBy}
                disabled={listControlsDisabled}
                options={sortOptions}
                onValueChange={(value) => handleSortByChange(value as InvocationSortBy)}
              />
              <SelectField
                className="min-w-[8rem]"
                label={t("records.list.sortOrder")}
                name="sortOrder"
                size="sm"
                value={sortOrder}
                disabled={listControlsDisabled}
                options={[
                  { value: "desc", label: t("records.list.sort.desc") },
                  { value: "asc", label: t("records.list.sort.asc") },
                ]}
                onValueChange={(value) => handleSortOrderChange(value as InvocationSortOrder)}
              />
            </div>
          </div>

          <InvocationRecordsTable
            focus={focus}
            records={records?.records ?? []}
            isLoading={tableLoading}
            error={recordsError}
            onOpenUpstreamAccount={(accountId) => openUpstreamAccount(accountId)}
            autoExpandInvokeId={autoExpandInvokeId}
            focusedAttemptId={focusedAttemptId}
          />

          <div className="flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-base-300/70 bg-base-100/45 px-4 py-3">
            <div className="text-sm text-base-content/70">
              {t("records.list.pageLabel", { page, totalPages })}
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => void setPage(page - 1)}
                disabled={page <= 1 || tableLoading}
              >
                {t("records.list.prev")}
              </Button>
              {visiblePages.map((pageNumber) => (
                <button
                  key={pageNumber}
                  type="button"
                  className={cn(
                    "inline-flex h-8 min-w-8 items-center justify-center rounded-full border px-3 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary",
                    pageNumber === page
                      ? "border-primary/45 bg-primary/20 text-primary"
                      : "border-base-300/70 bg-base-100/60 text-base-content/75 hover:bg-base-200/70",
                  )}
                  aria-current={pageNumber === page ? "page" : undefined}
                  onClick={() => void setPage(pageNumber)}
                  disabled={pageNumber === page || tableLoading}
                >
                  {pageNumber}
                </button>
              ))}
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => void setPage(page + 1)}
                disabled={page >= totalPages || tableLoading}
              >
                {t("records.list.next")}
              </Button>
            </div>
          </div>
        </div>
      </section>
      {upstreamAccountId != null ? (
        <SharedUpstreamAccountDetailDrawer
          open
          accountId={upstreamAccountId}
          onClose={closeUpstreamAccount}
        />
      ) : null}
    </div>
  );
}
