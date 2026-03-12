import { useEffect, useMemo, useRef, useState } from 'react'
import { RecordsNewDataButton } from '../components/RecordsNewDataButton'
import { Button } from '../components/ui/button'
import { FilterableCombobox } from '../components/ui/filterable-combobox'
import { InvocationRecordsSummaryCards } from '../components/InvocationRecordsSummaryCards'
import { InvocationRecordsTable } from '../components/InvocationRecordsTable'
import { useInvocationRecords } from '../hooks/useInvocationRecords'
import { useTranslation } from '../i18n'
import {
  fetchInvocationSuggestions,
  type InvocationFocus,
  type InvocationRangePreset,
  type InvocationSortBy,
  type InvocationSortOrder,
  type InvocationSuggestionField,
  type InvocationSuggestionsResponse,
} from '../lib/api'
import { buildInvocationSuggestionsQuery, createDefaultCustomRange, RECORDS_PAGE_SIZE_OPTIONS } from '../lib/invocationRecords'
import { cn } from '../lib/utils'

const inputClassName =
  'h-9 w-full rounded-md border border-base-300/80 bg-base-100 px-3 text-sm text-base-content shadow-sm outline-none transition focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100 disabled:cursor-not-allowed disabled:opacity-60'

const SUGGESTION_DEBOUNCE_MS = 250
const NEW_DATA_REFRESH_MIN_LOADING_MS = 600

function getVisiblePages(currentPage: number, totalPages: number) {
  if (totalPages <= 1) return [1]
  const start = Math.max(1, currentPage - 2)
  const end = Math.min(totalPages, currentPage + 2)
  const pages: number[] = []
  for (let page = start; page <= end; page += 1) {
    pages.push(page)
  }
  return pages
}

export default function RecordsPage() {
  const { t } = useTranslation()
  const {
    draft,
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
    setFocus,
    search,
    setPage,
    setPageSize,
    setSort,
  } = useInvocationRecords()

  const appliedSnapshotId = records?.snapshotId ?? summary?.snapshotId
  const [suggestions, setSuggestions] = useState<InvocationSuggestionsResponse | null>(null)
  const [isSuggestionsLoading, setIsSuggestionsLoading] = useState(false)
  const [activeSuggestionField, setActiveSuggestionField] = useState<InvocationSuggestionField | null>(null)
  const [isNewDataRefreshPending, setIsNewDataRefreshPending] = useState(false)
  const [cachedNewDataCount, setCachedNewDataCount] = useState(0)
  const suggestionQuery = useMemo(
    () => buildInvocationSuggestionsQuery(draft, appliedSnapshotId, activeSuggestionField ?? undefined),
    [activeSuggestionField, appliedSnapshotId, draft],
  )
  const suggestionsSeqRef = useRef(0)
  const customRangeTouchedRef = useRef(false)

  useEffect(() => {
    suggestionsSeqRef.current += 1
    setSuggestions(null)

    if (!activeSuggestionField) {
      setIsSuggestionsLoading(false)
    }
  }, [activeSuggestionField])

  useEffect(() => {
    if (!activeSuggestionField) {
      setIsSuggestionsLoading(false)
      return
    }

    const requestSeq = suggestionsSeqRef.current + 1
    suggestionsSeqRef.current = requestSeq
    setIsSuggestionsLoading(true)

    const timer = window.setTimeout(() => {
      fetchInvocationSuggestions(suggestionQuery)
        .then((response) => {
          if (requestSeq !== suggestionsSeqRef.current) return
          setSuggestions(response)
          setIsSuggestionsLoading(false)
        })
        .catch(() => {
          if (requestSeq !== suggestionsSeqRef.current) return
          setIsSuggestionsLoading(false)
          // Best-effort: suggestions should never block the page.
        })
    }, SUGGESTION_DEBOUNCE_MS)

    return () => window.clearTimeout(timer)
  }, [activeSuggestionField, suggestionQuery])

  const focusOptions = useMemo(
    () => [
      { value: 'token' as InvocationFocus, label: t('records.focus.token') },
      { value: 'network' as InvocationFocus, label: t('records.focus.network') },
      { value: 'exception' as InvocationFocus, label: t('records.focus.exception') },
    ],
    [t],
  )

  const rangeOptions = useMemo(
    () => [
      { value: 'today' as InvocationRangePreset, label: t('records.filters.rangePreset.today') },
      { value: '1d' as InvocationRangePreset, label: t('records.filters.rangePreset.lastDay') },
      { value: '7d' as InvocationRangePreset, label: t('records.filters.rangePreset.last7Days') },
      { value: '30d' as InvocationRangePreset, label: t('records.filters.rangePreset.last30Days') },
      { value: 'custom' as InvocationRangePreset, label: t('records.filters.rangePreset.custom') },
    ],
    [t],
  )

  const sortOptions = useMemo(
    () => [
      { value: 'occurredAt' as InvocationSortBy, label: t('records.list.sort.occurredAt') },
      { value: 'totalTokens' as InvocationSortBy, label: t('records.list.sort.totalTokens') },
      { value: 'cost' as InvocationSortBy, label: t('records.list.sort.cost') },
      { value: 'tTotalMs' as InvocationSortBy, label: t('records.list.sort.totalMs') },
      { value: 'tUpstreamTtfbMs' as InvocationSortBy, label: t('records.list.sort.ttfb') },
      { value: 'status' as InvocationSortBy, label: t('records.list.sort.status') },
    ],
    [t],
  )

  const total = records?.total ?? 0
  const totalPages = Math.max(1, Math.ceil(total / pageSize) || 1)
  const visiblePages = getVisiblePages(page, totalPages)
  const isCustomRange = draft.rangePreset === 'custom'
  const newRecordsCount = summary?.newRecordsCount ?? 0
  const isNewDataLoading = isNewDataRefreshPending
  const displayNewDataCount = newRecordsCount > 0 ? newRecordsCount : cachedNewDataCount
  const shouldShowNewDataButton =
    (!isSearching || isNewDataRefreshPending) && (newRecordsCount > 0 || (isNewDataLoading && displayNewDataCount > 0))
  const visibleSummary = summary && summary.snapshotId === records?.snapshotId ? summary : null
  const tableLoading = isRecordsLoading
  const listControlsDisabled = isSearching || isRecordsLoading
  const hasOpenSuggestion = activeSuggestionField !== null
  const modelBucket = suggestions?.model
  const proxyBucket = suggestions?.proxy
  const endpointBucket = suggestions?.endpoint
  const failureKindBucket = suggestions?.failureKind
  const promptCacheKeyBucket = suggestions?.promptCacheKey
  const requesterIpBucket = suggestions?.requesterIp

  const handleClearDraft = () => {
    customRangeTouchedRef.current = false
    resetDraft()
  }

  const handleRangePresetChange = (value: InvocationRangePreset) => {
    updateDraft('rangePreset', value)
    if (value === 'custom' && !customRangeTouchedRef.current) {
      const nextRange = createDefaultCustomRange()
      updateDraft('customFrom', nextRange.customFrom)
      updateDraft('customTo', nextRange.customTo)
    }
  }

  useEffect(() => {
    if (newRecordsCount > 0) {
      setCachedNewDataCount(newRecordsCount)
      return
    }

    if (!isNewDataLoading) {
      setCachedNewDataCount(0)
    }
  }, [isNewDataLoading, newRecordsCount])

  const handleSearch = () => {
    void search()
  }

  const handleRefreshNewData = () => {
    if (isNewDataLoading) return
    setIsNewDataRefreshPending(true)
    const minLoadingDelay = new Promise<void>((resolve) => {
      window.setTimeout(resolve, NEW_DATA_REFRESH_MIN_LOADING_MS)
    })

    void Promise.all([
      search({ source: 'applied', preserveSummary: true }),
      minLoadingDelay,
    ]).finally(() => {
      setIsNewDataRefreshPending(false)
    })
  }

  const handleSuggestionOpenChange = (field: InvocationSuggestionField) => (open: boolean) => {
    setActiveSuggestionField((current) => {
      if (open) return field
      return current === field ? null : current
    })
  }

  const handleSortByChange = (value: InvocationSortBy) => {
    void setSort(value, sortOrder)
  }

  const handleSortOrderChange = (value: InvocationSortOrder) => {
    void setSort(sortBy, value)
  }

  return (
    <div className="mx-auto flex w-full max-w-full flex-col gap-6">
      <section
        className={cn('surface-panel', hasOpenSuggestion && 'relative z-10 overflow-visible')}
        data-testid="records-filters-panel"
        data-suggestions-open={hasOpenSuggestion ? 'true' : 'false'}
      >
        <div className="surface-panel-body gap-5">
          <div className="section-heading">
            <h1 className="section-title">{t('records.title')}</h1>
            <p className="section-description">{t('records.subtitle')}</p>
          </div>

          <div className="rounded-2xl border border-base-300/70 bg-base-100/45 p-4">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div className="section-heading">
                <h2 className="section-title text-base">{t('records.filters.title')}</h2>
                <p className="section-description">{t('records.filters.description')}</p>
              </div>
              <div className="flex flex-wrap items-center gap-2">
                <Button type="button" variant="ghost" onClick={handleClearDraft} disabled={isSearching}>
                  {t('records.filters.clearDraft')}
                </Button>
                <Button type="button" onClick={handleSearch} disabled={isSearching}>
                  {isSearching ? t('records.filters.searching') : t('records.filters.search')}
                </Button>
              </div>
            </div>

            <div className="mt-4 grid gap-4 md:grid-cols-2 xl:grid-cols-4">
              <label className="field">
                <span className="field-label">{t('records.filters.rangePreset')}</span>
                <select name="rangePreset" className="field-select" value={draft.rangePreset} onChange={(event) => handleRangePresetChange(event.target.value as InvocationRangePreset)}>
                  {rangeOptions.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field">
                <span className="field-label">{t('records.filters.from')}</span>
                <input
                  className={inputClassName}
                  type="datetime-local"
                  name="customFrom"
                  value={draft.customFrom}
                  disabled={!isCustomRange}
                  onChange={(event) => {
                    customRangeTouchedRef.current = true
                    updateDraft('customFrom', event.target.value)
                  }}
                />
              </label>
              <label className="field">
                <span className="field-label">{t('records.filters.to')}</span>
                <input
                  className={inputClassName}
                  type="datetime-local"
                  name="customTo"
                  value={draft.customTo}
                  disabled={!isCustomRange}
                  onChange={(event) => {
                    customRangeTouchedRef.current = true
                    updateDraft('customTo', event.target.value)
                  }}
                />
              </label>
              <label className="field">
                <span className="field-label">{t('records.filters.status')}</span>
                <select name="status" className="field-select" value={draft.status} onChange={(event) => updateDraft('status', event.target.value)}>
                  <option value="">{t('records.filters.status.all')}</option>
                  <option value="success">{t('records.filters.status.success')}</option>
                  <option value="failed">{t('records.filters.status.failed')}</option>
                  <option value="running">{t('records.filters.status.running')}</option>
                  <option value="pending">{t('records.filters.status.pending')}</option>
                </select>
              </label>

              <label className="field">
                <span className="field-label">{t('records.filters.model')}</span>
                <FilterableCombobox
                  label={t('records.filters.model')}
                  name="model"
                  id="records-filter-model"
                  value={draft.model}
                  onValueChange={(next) => updateDraft('model', next)}
                  options={(modelBucket?.items ?? []).map((item) => item.value)}
                  placeholder={t('records.filters.any')}
                  emptyText={t('records.filters.noMatches')}
                  loading={isSuggestionsLoading && activeSuggestionField === 'model'}
                  loadingText={t('records.filters.searching')}
                  inputClassName={inputClassName}
                  onOpenChange={handleSuggestionOpenChange('model')}
                />
              </label>
              <label className="field">
                <span className="field-label">{t('records.filters.proxy')}</span>
                <FilterableCombobox
                  label={t('records.filters.proxy')}
                  name="proxy"
                  id="records-filter-proxy"
                  value={draft.proxy}
                  onValueChange={(next) => updateDraft('proxy', next)}
                  options={(proxyBucket?.items ?? []).map((item) => item.value)}
                  placeholder={t('records.filters.any')}
                  emptyText={t('records.filters.noMatches')}
                  loading={isSuggestionsLoading && activeSuggestionField === 'proxy'}
                  loadingText={t('records.filters.searching')}
                  inputClassName={inputClassName}
                  onOpenChange={handleSuggestionOpenChange('proxy')}
                />
              </label>
              <label className="field">
                <span className="field-label">{t('records.filters.endpoint')}</span>
                <FilterableCombobox
                  label={t('records.filters.endpoint')}
                  name="endpoint"
                  id="records-filter-endpoint"
                  value={draft.endpoint}
                  onValueChange={(next) => updateDraft('endpoint', next)}
                  options={(endpointBucket?.items ?? []).map((item) => item.value)}
                  placeholder={t('records.filters.any')}
                  emptyText={t('records.filters.noMatches')}
                  loading={isSuggestionsLoading && activeSuggestionField === 'endpoint'}
                  loadingText={t('records.filters.searching')}
                  inputClassName={inputClassName}
                  onOpenChange={handleSuggestionOpenChange('endpoint')}
                />
              </label>
              <label className="field">
                <span className="field-label">{t('records.filters.failureClass')}</span>
                <select name="failureClass" className="field-select" value={draft.failureClass} onChange={(event) => updateDraft('failureClass', event.target.value)}>
                  <option value="">{t('records.filters.failureClass.all')}</option>
                  <option value="service_failure">{t('records.filters.failureClass.service')}</option>
                  <option value="client_failure">{t('records.filters.failureClass.client')}</option>
                  <option value="client_abort">{t('records.filters.failureClass.abort')}</option>
                </select>
              </label>

              <label className="field">
                <span className="field-label">{t('records.filters.failureKind')}</span>
                <FilterableCombobox
                  label={t('records.filters.failureKind')}
                  name="failureKind"
                  id="records-filter-failure-kind"
                  value={draft.failureKind}
                  onValueChange={(next) => updateDraft('failureKind', next)}
                  options={(failureKindBucket?.items ?? []).map((item) => item.value)}
                  placeholder={t('records.filters.any')}
                  emptyText={t('records.filters.noMatches')}
                  loading={isSuggestionsLoading && activeSuggestionField === 'failureKind'}
                  loadingText={t('records.filters.searching')}
                  inputClassName={inputClassName}
                  onOpenChange={handleSuggestionOpenChange('failureKind')}
                />
              </label>
              <label className="field">
                <span className="field-label">{t('records.filters.promptCacheKey')}</span>
                <FilterableCombobox
                  label={t('records.filters.promptCacheKey')}
                  name="promptCacheKey"
                  id="records-filter-prompt-cache-key"
                  value={draft.promptCacheKey}
                  onValueChange={(next) => updateDraft('promptCacheKey', next)}
                  options={(promptCacheKeyBucket?.items ?? []).map((item) => item.value)}
                  placeholder={t('records.filters.any')}
                  emptyText={t('records.filters.noMatches')}
                  loading={isSuggestionsLoading && activeSuggestionField === 'promptCacheKey'}
                  loadingText={t('records.filters.searching')}
                  inputClassName={inputClassName}
                  onOpenChange={handleSuggestionOpenChange('promptCacheKey')}
                />
              </label>
              <label className="field">
                <span className="field-label">{t('records.filters.requesterIp')}</span>
                <FilterableCombobox
                  label={t('records.filters.requesterIp')}
                  name="requesterIp"
                  id="records-filter-requester-ip"
                  value={draft.requesterIp}
                  onValueChange={(next) => updateDraft('requesterIp', next)}
                  options={(requesterIpBucket?.items ?? []).map((item) => item.value)}
                  placeholder={t('records.filters.any')}
                  emptyText={t('records.filters.noMatches')}
                  loading={isSuggestionsLoading && activeSuggestionField === 'requesterIp'}
                  loadingText={t('records.filters.searching')}
                  inputClassName={inputClassName}
                  onOpenChange={handleSuggestionOpenChange('requesterIp')}
                />
              </label>
              <label className="field">
                <span className="field-label">{t('records.filters.keyword')}</span>
                <input name="keyword" className={inputClassName} value={draft.keyword} onChange={(event) => updateDraft('keyword', event.target.value)} />
              </label>

              <label className="field">
                <span className="field-label">{t('records.filters.minTotalTokens')}</span>
                <input name="minTotalTokens" className={inputClassName} type="number" inputMode="numeric" step={1} value={draft.minTotalTokens} onChange={(event) => updateDraft('minTotalTokens', event.target.value)} />
              </label>
              <label className="field">
                <span className="field-label">{t('records.filters.maxTotalTokens')}</span>
                <input name="maxTotalTokens" className={inputClassName} type="number" inputMode="numeric" step={1} value={draft.maxTotalTokens} onChange={(event) => updateDraft('maxTotalTokens', event.target.value)} />
              </label>
              <label className="field">
                <span className="field-label">{t('records.filters.minTotalMs')}</span>
                <input name="minTotalMs" className={inputClassName} type="number" inputMode="decimal" value={draft.minTotalMs} onChange={(event) => updateDraft('minTotalMs', event.target.value)} />
              </label>
              <label className="field">
                <span className="field-label">{t('records.filters.maxTotalMs')}</span>
                <input name="maxTotalMs" className={inputClassName} type="number" inputMode="decimal" value={draft.maxTotalMs} onChange={(event) => updateDraft('maxTotalMs', event.target.value)} />
              </label>
            </div>
          </div>
        </div>
      </section>

      <section className="surface-panel" data-testid="records-summary-panel">
        <div className="surface-panel-body gap-4">
          <div className="flex flex-wrap items-start justify-between gap-3">
            <div className="section-heading">
              <h2 className="section-title">{t('records.summary.title')}</h2>
              <p className="section-description">{t('records.summary.description')}</p>
            </div>
            <div className="flex flex-wrap items-center gap-3">
              {shouldShowNewDataButton ? (
                <RecordsNewDataButton
                  count={displayNewDataCount}
                  isLoading={isNewDataLoading}
                  onRefresh={handleRefreshNewData}
                />
              ) : null}
              <div className="segment-group" role="tablist" aria-label={t('records.focus.label')}>
                {focusOptions.map((option) => (
                  <button
                    key={option.value}
                    type="button"
                    role="tab"
                    aria-selected={focus === option.value}
                    aria-pressed={focus === option.value}
                    onClick={() => setFocus(option.value)}
                    className={cn('segment-button px-3', focus === option.value && 'font-semibold')}
                    data-active={focus === option.value}
                  >
                    {option.label}
                  </button>
                ))}
              </div>
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
              <h2 className="section-title">{t('records.list.title')}</h2>
              <p className="section-description">{t('records.list.description')}</p>
            </div>
            <div className="flex flex-wrap items-end gap-3">
              <div className="rounded-full border border-base-300/70 bg-base-100/55 px-3 py-2 text-sm font-medium text-base-content/80">
                {t('records.list.totalCount', { count: total })}
              </div>
              <label className="field min-w-[7rem]">
                <span className="field-label">{t('records.list.pageSize')}</span>
                <select
                  name="pageSize"
                  className="field-select field-select-sm"
                  value={pageSize}
                  disabled={listControlsDisabled}
                  onChange={(event) => void setPageSize(Number(event.target.value))}
                >
                  {RECORDS_PAGE_SIZE_OPTIONS.map((value) => (
                    <option key={value} value={value}>
                      {value}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field min-w-[10rem]">
                <span className="field-label">{t('records.list.sortBy')}</span>
                <select
                  name="sortBy"
                  className="field-select field-select-sm"
                  value={sortBy}
                  disabled={listControlsDisabled}
                  onChange={(event) => handleSortByChange(event.target.value as InvocationSortBy)}
                >
                  {sortOptions.map((option) => (
                    <option key={option.value} value={option.value}>
                      {option.label}
                    </option>
                  ))}
                </select>
              </label>
              <label className="field min-w-[8rem]">
                <span className="field-label">{t('records.list.sortOrder')}</span>
                <select
                  name="sortOrder"
                  className="field-select field-select-sm"
                  value={sortOrder}
                  disabled={listControlsDisabled}
                  onChange={(event) => handleSortOrderChange(event.target.value as InvocationSortOrder)}
                >
                  <option value="desc">{t('records.list.sort.desc')}</option>
                  <option value="asc">{t('records.list.sort.asc')}</option>
                </select>
              </label>
            </div>
          </div>

          <InvocationRecordsTable
            focus={focus}
            records={records?.records ?? []}
            isLoading={tableLoading}
            error={recordsError}
          />

          <div className="flex flex-wrap items-center justify-between gap-3 rounded-2xl border border-base-300/70 bg-base-100/45 px-4 py-3">
            <div className="text-sm text-base-content/70">{t('records.list.pageLabel', { page, totalPages })}</div>
            <div className="flex flex-wrap items-center gap-2">
              <Button type="button" variant="outline" size="sm" onClick={() => void setPage(page - 1)} disabled={page <= 1 || tableLoading}>
                {t('records.list.prev')}
              </Button>
              {visiblePages.map((pageNumber) => (
                <button
                  key={pageNumber}
                  type="button"
                  className={cn(
                    'inline-flex h-8 min-w-8 items-center justify-center rounded-full border px-3 text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
                    pageNumber === page
                      ? 'border-primary/45 bg-primary/20 text-primary'
                      : 'border-base-300/70 bg-base-100/60 text-base-content/75 hover:bg-base-200/70',
                  )}
                  aria-current={pageNumber === page ? 'page' : undefined}
                  onClick={() => void setPage(pageNumber)}
                  disabled={pageNumber === page || tableLoading}
                >
                  {pageNumber}
                </button>
              ))}
              <Button type="button" variant="outline" size="sm" onClick={() => void setPage(page + 1)} disabled={page >= totalPages || tableLoading}>
                {t('records.list.next')}
              </Button>
            </div>
          </div>
        </div>
      </section>
    </div>
  )
}
