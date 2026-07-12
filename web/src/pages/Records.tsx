import { useEffect, useMemo, useRef, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { Button } from "../components/ui/button";
import { FilterableCombobox } from "../components/ui/filterable-combobox";
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
  fetchInvocationSuggestions,
  type InvocationFocus,
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
  type InvocationRecordsDraftFilters,
  RECORDS_PAGE_SIZE_OPTIONS,
} from "../lib/invocationRecords";
import { cn } from "../lib/utils";
import { SharedUpstreamAccountDetailDrawer } from "./account-pool/UpstreamAccounts";

const inputClassName =
  "h-9 w-full rounded-md border border-base-300/80 bg-base-100 px-3 text-sm text-base-content shadow-sm outline-none transition focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100 disabled:cursor-not-allowed disabled:opacity-60";

const SUGGESTION_DEBOUNCE_MS = 250;
const NEW_DATA_REFRESH_MIN_LOADING_MS = 600;

type RemovableRecordFilterKey = Exclude<
  keyof InvocationRecordsDraftFilters,
  "rangePreset" | "customFrom" | "customTo"
>;

interface ActiveFilterChip {
  id: string;
  label: string;
  draftKey?: RemovableRecordFilterKey;
}

function formatCustomRange(from: string, to: string) {
  const values = [from, to].filter(Boolean).map((value) => value.replace("T", " "));
  return values.join(" - ");
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
  const requestedInvokeId = searchParams.get("requestId")?.trim() || null;
  const requestedRangePreset = searchParams.get("rangePreset") === "7d" ? "7d" : null;
  const appliedRequestIdRef = useRef<string | null>(null);
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

  const appliedSnapshotId = records?.snapshotId ?? summary?.snapshotId;
  const [suggestions, setSuggestions] = useState<InvocationSuggestionsResponse | null>(null);
  const [isSuggestionsLoading, setIsSuggestionsLoading] = useState(false);
  const [activeSuggestionField, setActiveSuggestionField] =
    useState<InvocationSuggestionField | null>(null);
  const [isNewDataRefreshPending, setIsNewDataRefreshPending] = useState(false);
  const [cachedNewDataCount, setCachedNewDataCount] = useState(0);
  const [isFiltersOpen, setIsFiltersOpen] = useState(false);
  const newDataRefreshSeqRef = useRef(0);
  const suggestionQuery = useMemo(
    () =>
      buildInvocationSuggestionsQuery(draft, appliedSnapshotId, activeSuggestionField ?? undefined),
    [activeSuggestionField, appliedSnapshotId, draft],
  );
  const suggestionsSeqRef = useRef(0);
  const customRangeTouchedRef = useRef(false);

  useEffect(() => {
    const requestKey = `${requestedInvokeId}:${requestedRangePreset ?? ""}`;
    if (!requestedInvokeId || appliedRequestIdRef.current === requestKey) return;
    appliedRequestIdRef.current = requestKey;
    resetDraft();
    updateDraft("requestId", requestedInvokeId);
    if (requestedRangePreset) {
      updateDraft("rangePreset", requestedRangePreset);
    }
    const timer = window.setTimeout(() => void search(), 0);
    return () => window.clearTimeout(timer);
  }, [requestedInvokeId, requestedRangePreset, resetDraft, search, updateDraft]);

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

  const total = records?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / pageSize) || 1);
  const visiblePages = getVisiblePages(page, totalPages);
  const isCustomRange = draft.rangePreset === "custom";
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
  const modelBucket = suggestions?.model;
  const endpointBucket = suggestions?.endpoint;
  const failureKindBucket = suggestions?.failureKind;
  const promptCacheKeyBucket = suggestions?.promptCacheKey;
  const requesterIpBucket = suggestions?.requesterIp;

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
    const add = (draftKey: RemovableRecordFilterKey, label: string, value: string) => {
      const normalized = value.trim();
      if (!normalized) return;
      chips.push({ id: draftKey, draftKey, label: `${label}: ${normalized}` });
    };

    const statusLabels: Record<string, string> = {
      success: t("records.filters.status.success"),
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

    add(
      "status",
      t("records.filters.status"),
      statusLabels[appliedDraft.status] ?? appliedDraft.status,
    );
    add("model", t("records.filters.model"), appliedDraft.model);
    add("endpoint", t("records.filters.endpoint"), appliedDraft.endpoint);
    add(
      "failureClass",
      t("records.filters.failureClass"),
      failureClassLabels[appliedDraft.failureClass] ?? appliedDraft.failureClass,
    );
    add("requestId", t("records.filters.requestId"), appliedDraft.requestId);
    add("failureKind", t("records.filters.failureKind"), appliedDraft.failureKind);
    add("promptCacheKey", t("records.filters.promptCacheKey"), appliedDraft.promptCacheKey);
    add("requesterIp", t("records.filters.requesterIp"), appliedDraft.requesterIp);
    add("keyword", t("records.filters.keyword"), appliedDraft.keyword);
    add("minTotalTokens", t("records.filters.minTotalTokens"), appliedDraft.minTotalTokens);
    add("maxTotalTokens", t("records.filters.maxTotalTokens"), appliedDraft.maxTotalTokens);
    add("minTotalMs", t("records.filters.minTotalMs"), appliedDraft.minTotalMs);
    add("maxTotalMs", t("records.filters.maxTotalMs"), appliedDraft.maxTotalMs);

    return chips;
  }, [appliedDraft, rangeOptions, t]);

  const handleClearDraft = () => {
    customRangeTouchedRef.current = false;
    resetDraft();
  };

  const handleRangePresetChange = (value: InvocationRangePreset) => {
    updateDraft("rangePreset", value);
    if (value === "custom" && !customRangeTouchedRef.current) {
      const nextRange = createDefaultCustomRange();
      updateDraft("customFrom", nextRange.customFrom);
      updateDraft("customTo", nextRange.customTo);
    }
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
  };

  const handleApplyFilters = () => {
    closeFilters();
    handleSearch();
  };

  const handleRemoveActiveFilter = (key: RemovableRecordFilterKey) => {
    const nextDraft = { ...(appliedDraft ?? draft), [key]: "" };
    newDataRefreshSeqRef.current += 1;
    setIsNewDataRefreshPending(false);
    void applyDraft(nextDraft);
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
              chip.draftKey ? (
                <button
                  key={chip.id}
                  type="button"
                  onClick={() => handleRemoveActiveFilter(chip.draftKey!)}
                  data-testid={`records-active-filter-${chip.id}`}
                  className="inline-flex max-w-full items-center gap-1.5 rounded-full border border-base-300/80 bg-base-100 px-2.5 py-1 text-left text-xs font-medium text-base-content transition hover:border-primary/45 hover:bg-primary/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                  aria-label={t("records.filters.remove", { label: chip.label })}
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
        bodyClassName={cn(hasOpenSuggestion && "overflow-visible")}
        header={
          <div className="section-heading">
            <h2 id="records-filters-drawer-title" className="section-title text-base">
              {t("records.filters.title")}
            </h2>
            <p className="section-description">{t("records.filters.description")}</p>
          </div>
        }
      >
        <div
          className="flex min-h-full flex-col gap-5"
          data-testid="records-filters-drawer"
          data-suggestions-open={hasOpenSuggestion ? "true" : "false"}
        >
          <div className="grid gap-4 min-[769px]:grid-cols-2">
            <SelectField
              className="field"
              label={t("records.filters.rangePreset")}
              name="rangePreset"
              value={draft.rangePreset}
              options={rangeOptions}
              onValueChange={(value) => handleRangePresetChange(value as InvocationRangePreset)}
            />
            <label className="field">
              <span className="field-label">{t("records.filters.from")}</span>
              <input
                {...textInputAutocompleteOffProps}
                className={inputClassName}
                type="datetime-local"
                name="customFrom"
                value={draft.customFrom}
                disabled={!isCustomRange}
                onChange={(event) => {
                  customRangeTouchedRef.current = true;
                  updateDraft("customFrom", event.target.value);
                }}
              />
            </label>
            <label className="field">
              <span className="field-label">{t("records.filters.to")}</span>
              <input
                {...textInputAutocompleteOffProps}
                className={inputClassName}
                type="datetime-local"
                name="customTo"
                value={draft.customTo}
                disabled={!isCustomRange}
                onChange={(event) => {
                  customRangeTouchedRef.current = true;
                  updateDraft("customTo", event.target.value);
                }}
              />
            </label>
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

            <label className="field">
              <span className="field-label">{t("records.filters.model")}</span>
              <FilterableCombobox
                label={t("records.filters.model")}
                name="model"
                id="records-filter-model"
                value={draft.model}
                onValueChange={(next) => updateDraft("model", next)}
                options={(modelBucket?.items ?? []).map((item) => item.value)}
                placeholder={t("records.filters.any")}
                emptyText={t("records.filters.noMatches")}
                loading={isSuggestionsLoading && activeSuggestionField === "model"}
                loadingText={t("records.filters.searching")}
                inputClassName={inputClassName}
                onOpenChange={handleSuggestionOpenChange("model")}
              />
            </label>
            <label className="field">
              <span className="field-label">{t("records.filters.endpoint")}</span>
              <FilterableCombobox
                label={t("records.filters.endpoint")}
                name="endpoint"
                id="records-filter-endpoint"
                value={draft.endpoint}
                onValueChange={(next) => updateDraft("endpoint", next)}
                options={(endpointBucket?.items ?? []).map((item) => item.value)}
                placeholder={t("records.filters.any")}
                emptyText={t("records.filters.noMatches")}
                loading={isSuggestionsLoading && activeSuggestionField === "endpoint"}
                loadingText={t("records.filters.searching")}
                inputClassName={inputClassName}
                onOpenChange={handleSuggestionOpenChange("endpoint")}
              />
            </label>
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
              <span className="field-label">{t("records.filters.requestId")}</span>
              <input
                {...textInputAutocompleteOffProps}
                name="requestId"
                className={inputClassName}
                value={draft.requestId}
                onChange={(event) => updateDraft("requestId", event.target.value)}
              />
            </label>

            <label className="field">
              <span className="field-label">{t("records.filters.failureKind")}</span>
              <FilterableCombobox
                label={t("records.filters.failureKind")}
                name="failureKind"
                id="records-filter-failure-kind"
                value={draft.failureKind}
                onValueChange={(next) => updateDraft("failureKind", next)}
                options={(failureKindBucket?.items ?? []).map((item) => item.value)}
                placeholder={t("records.filters.any")}
                emptyText={t("records.filters.noMatches")}
                loading={isSuggestionsLoading && activeSuggestionField === "failureKind"}
                loadingText={t("records.filters.searching")}
                inputClassName={inputClassName}
                onOpenChange={handleSuggestionOpenChange("failureKind")}
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
                options={(promptCacheKeyBucket?.items ?? []).map((item) => item.value)}
                placeholder={t("records.filters.any")}
                emptyText={t("records.filters.noMatches")}
                loading={isSuggestionsLoading && activeSuggestionField === "promptCacheKey"}
                loadingText={t("records.filters.searching")}
                inputClassName={inputClassName}
                onOpenChange={handleSuggestionOpenChange("promptCacheKey")}
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
                options={(requesterIpBucket?.items ?? []).map((item) => item.value)}
                placeholder={t("records.filters.any")}
                emptyText={t("records.filters.noMatches")}
                loading={isSuggestionsLoading && activeSuggestionField === "requesterIp"}
                loadingText={t("records.filters.searching")}
                inputClassName={inputClassName}
                onOpenChange={handleSuggestionOpenChange("requesterIp")}
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

            <label className="field">
              <span className="field-label">{t("records.filters.minTotalTokens")}</span>
              <input
                {...textInputAutocompleteOffProps}
                name="minTotalTokens"
                className={inputClassName}
                type="number"
                inputMode="numeric"
                step={1}
                value={draft.minTotalTokens}
                onChange={(event) => updateDraft("minTotalTokens", event.target.value)}
              />
            </label>
            <label className="field">
              <span className="field-label">{t("records.filters.maxTotalTokens")}</span>
              <input
                {...textInputAutocompleteOffProps}
                name="maxTotalTokens"
                className={inputClassName}
                type="number"
                inputMode="numeric"
                step={1}
                value={draft.maxTotalTokens}
                onChange={(event) => updateDraft("maxTotalTokens", event.target.value)}
              />
            </label>
            <label className="field">
              <span className="field-label">{t("records.filters.minTotalMs")}</span>
              <input
                {...textInputAutocompleteOffProps}
                name="minTotalMs"
                className={inputClassName}
                type="number"
                inputMode="decimal"
                value={draft.minTotalMs}
                onChange={(event) => updateDraft("minTotalMs", event.target.value)}
              />
            </label>
            <label className="field">
              <span className="field-label">{t("records.filters.maxTotalMs")}</span>
              <input
                {...textInputAutocompleteOffProps}
                name="maxTotalMs"
                className={inputClassName}
                type="number"
                inputMode="decimal"
                value={draft.maxTotalMs}
                onChange={(event) => updateDraft("maxTotalMs", event.target.value)}
              />
            </label>
          </div>
          <div className="sticky bottom-[-1rem] -mx-4 mt-auto flex flex-col-reverse gap-2 border-t border-base-300/70 bg-base-100 px-4 pb-[max(1rem,env(safe-area-inset-bottom))] pt-3 sm:flex-row sm:justify-end desktop:bottom-[-1.5rem] desktop:-mx-6 desktop:px-6">
            <Button type="button" variant="ghost" onClick={handleClearDraft} disabled={isSearching}>
              {t("records.filters.clearDraft")}
            </Button>
            <Button type="button" onClick={handleApplyFilters} disabled={isSearching}>
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
            autoExpandInvokeId={requestedInvokeId}
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
