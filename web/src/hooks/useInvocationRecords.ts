import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type {
  InvocationFocus,
  InvocationRecordsNewCountResponse,
  InvocationRecordsQuery,
  InvocationRecordsResponse,
  InvocationRecordsSummaryResponse,
  InvocationSortBy,
  InvocationSortOrder,
} from '../lib/api'
import { fetchInvocationRecords, fetchInvocationRecordsNewCount, fetchInvocationRecordsSummary } from '../lib/api'
import { invocationStableKey } from '../lib/invocation'
import { matchesInvocationLiveFilters, mergeInvocationWindowRecords } from '../lib/invocationRecordsLive'
import { useInvocationRecordsRealtime } from './useInvocationRecordsRealtime'
import {
  buildAppliedInvocationFilters,
  buildInvocationRecordsQuery,
  createDefaultCustomRange,
  createDefaultInvocationRecordsDraft,
  DEFAULT_RECORDS_FOCUS,
  DEFAULT_RECORDS_PAGE_SIZE,
  DEFAULT_RECORDS_SORT_BY,
  DEFAULT_RECORDS_SORT_ORDER,
  RECORDS_NEW_COUNT_POLL_INTERVAL_MS,
  type InvocationRecordsDraftFilters,
} from '../lib/invocationRecords'

export interface UseInvocationRecordsResult {
  draft: InvocationRecordsDraftFilters
  appliedDraft: InvocationRecordsDraftFilters | null
  focus: InvocationFocus
  page: number
  pageSize: number
  sortBy: InvocationSortBy
  sortOrder: InvocationSortOrder
  records: InvocationRecordsResponse | null
  summary: InvocationRecordsSummaryResponse | null
  recordsError: string | null
  summaryError: string | null
  isSearching: boolean
  isRecordsLoading: boolean
  isSummaryLoading: boolean
  updateDraft: <K extends keyof InvocationRecordsDraftFilters>(key: K, value: InvocationRecordsDraftFilters[K]) => void
  resetDraft: () => void
  applyDraft: (draft: InvocationRecordsDraftFilters) => Promise<void>
  setFocus: (focus: InvocationFocus) => void
  search: (options?: InvocationRecordsSearchOptions) => Promise<void>
  setPage: (page: number) => Promise<void>
  setPageSize: (pageSize: number) => Promise<void>
  setSort: (sortBy: InvocationSortBy, sortOrder: InvocationSortOrder) => Promise<void>
}

export interface InvocationRecordsSearchOptions {
  source?: 'draft' | 'applied'
  preserveSummary?: boolean
  draft?: InvocationRecordsDraftFilters
}

interface SearchState {
  filters: Omit<InvocationRecordsQuery, 'page' | 'pageSize' | 'sortBy' | 'sortOrder' | 'snapshotId'>
  snapshotId: number
  generation: number
}

interface LiveMergeState {
  matchingVisibleInsertKeys: string[]
  countedLiveInsertKeys: string[]
  countedFilteredOutBaselineKeys: string[]
}

function isTransientInFlightRecord(record: InvocationRecordsResponse['records'][number]) {
  const status = record.status?.trim().toLowerCase()
  return record.id <= 0 && (status === 'running' || status === 'pending')
}

function mergeTransientInFlightRecords(
  response: InvocationRecordsResponse,
  previous: InvocationRecordsResponse | null,
  options: {
    filters: InvocationRecordsQuery
    sortBy: InvocationSortBy
    sortOrder: InvocationSortOrder
    adjustTotal?: boolean
  },
) {
  const transientRecords =
    previous?.records.filter((record) => isTransientInFlightRecord(record)) ?? []
  if (transientRecords.length === 0 || previous?.snapshotId !== response.snapshotId) {
    return {
      response,
      preservedKeys: [] as string[],
    }
  }

  const responseKeys = new Set(response.records.map((record) => invocationStableKey(record)))
  const recordsToPreserve = transientRecords.filter((record) => {
    const key = invocationStableKey(record)
    return !responseKeys.has(key) && matchesInvocationLiveFilters(record, options.filters)
  })

  if (recordsToPreserve.length === 0) {
    return {
      response,
      preservedKeys: [] as string[],
    }
  }

  const mergedRecords = mergeInvocationWindowRecords(response.records, recordsToPreserve, {
    filters: options.filters,
    sortBy: options.sortBy,
    sortOrder: options.sortOrder,
    limit: response.pageSize,
  })
  const preservedKeySet = new Set(recordsToPreserve.map((record) => invocationStableKey(record)))
  const preservedKeys = mergedRecords
    .map((record) => invocationStableKey(record))
    .filter((key) => preservedKeySet.has(key))

  if (preservedKeys.length === 0) {
    return {
      response,
      preservedKeys,
    }
  }

  return {
    response: {
      ...response,
      total:
        response.total +
        (options.adjustTotal === true && previous
          ? Math.min(preservedKeys.length, Math.max(0, previous.total - response.total))
          : 0),
      records: mergedRecords,
    },
    preservedKeys,
  }
}

export function shouldPollRecordsSummary() {
  return typeof document === 'undefined' || document.visibilityState === 'visible'
}

export function useInvocationRecords(): UseInvocationRecordsResult {
  const [draft, setDraft] = useState<InvocationRecordsDraftFilters>(() => {
    const next = createDefaultInvocationRecordsDraft()
    return { ...next, ...createDefaultCustomRange() }
  })
  const [appliedDraft, setAppliedDraft] = useState<InvocationRecordsDraftFilters | null>(null)
  const [focus, setFocus] = useState<InvocationFocus>(DEFAULT_RECORDS_FOCUS)
  const [page, setPageState] = useState(1)
  const [pageSize, setPageSizeState] = useState<number>(DEFAULT_RECORDS_PAGE_SIZE)
  const [sortBy, setSortByState] = useState<InvocationSortBy>(DEFAULT_RECORDS_SORT_BY)
  const [sortOrder, setSortOrderState] = useState<InvocationSortOrder>(DEFAULT_RECORDS_SORT_ORDER)
  const [records, setRecords] = useState<InvocationRecordsResponse | null>(null)
  const [summary, setSummary] = useState<InvocationRecordsSummaryResponse | null>(null)
  const [recordsError, setRecordsError] = useState<string | null>(null)
  const [summaryError, setSummaryError] = useState<string | null>(null)
  const [isSearching, setIsSearching] = useState(true)
  const [isRecordsLoading, setIsRecordsLoading] = useState(false)
  const [isSummaryLoading, setIsSummaryLoading] = useState(true)
  const [liveMergeState, setLiveMergeState] = useState<LiveMergeState>({
    matchingVisibleInsertKeys: [],
    countedLiveInsertKeys: [],
    countedFilteredOutBaselineKeys: [],
  })
  const appliedRef = useRef<SearchState | null>(null)
  const searchSeqRef = useRef(0)
  const recordsSeqRef = useRef(0)
  const pollSeqRef = useRef(0)
  const draftRef = useRef(draft)
  const pageSizeRef = useRef(pageSize)
  const sortByRef = useRef(sortBy)
  const sortOrderRef = useRef(sortOrder)
  const recordsRef = useRef<InvocationRecordsResponse | null>(records)
  const liveMergeStateRef = useRef(liveMergeState)
  const authoritativeTotalRef = useRef(records?.total ?? 0)

  useEffect(() => {
    draftRef.current = draft
  }, [draft])

  useEffect(() => {
    pageSizeRef.current = pageSize
  }, [pageSize])

  useEffect(() => {
    sortByRef.current = sortBy
  }, [sortBy])

  useEffect(() => {
    sortOrderRef.current = sortOrder
  }, [sortOrder])

  useEffect(() => {
    recordsRef.current = records
  }, [records])

  useEffect(() => {
    liveMergeStateRef.current = liveMergeState
  }, [liveMergeState])

  const loadRecordsPage = useCallback(
    async (nextPage: number, nextPageSize: number, nextSortBy: InvocationSortBy, nextSortOrder: InvocationSortOrder) => {
      const applied = appliedRef.current
      if (!applied) return
      const { filters, generation, snapshotId } = applied
      const requestSeq = recordsSeqRef.current + 1
      recordsSeqRef.current = requestSeq
      setIsRecordsLoading(true)

      const isCurrentRequest = () => {
        if (requestSeq !== recordsSeqRef.current) return false
        const latest = appliedRef.current
        return !!latest && latest.generation === generation && latest.snapshotId === snapshotId
      }

      try {
        const response = await fetchInvocationRecords(
          buildInvocationRecordsQuery(filters, {
            page: nextPage,
            pageSize: nextPageSize,
            sortBy: nextSortBy,
            sortOrder: nextSortOrder,
            snapshotId,
          }),
        )
        if (!isCurrentRequest()) return
        const {
          response: visibleResponse,
          preservedKeys,
        } = mergeTransientInFlightRecords(response, recordsRef.current, {
          filters,
          sortBy: nextSortBy,
          sortOrder: nextSortOrder,
          adjustTotal: false,
        })
        authoritativeTotalRef.current = response.total
        setRecords(visibleResponse)
        setLiveMergeState({
          matchingVisibleInsertKeys: [],
          countedLiveInsertKeys: preservedKeys,
          countedFilteredOutBaselineKeys: [],
        })
        setPageState(visibleResponse.page)
        setPageSizeState(visibleResponse.pageSize)
        pageSizeRef.current = visibleResponse.pageSize
        setSortByState(nextSortBy)
        sortByRef.current = nextSortBy
        setSortOrderState(nextSortOrder)
        sortOrderRef.current = nextSortOrder
        setRecordsError(null)
      } catch (error) {
        if (!isCurrentRequest()) return
        setRecordsError(error instanceof Error ? error.message : String(error))
      } finally {
        if (isCurrentRequest()) {
          setIsRecordsLoading(false)
        }
      }
    },
    [],
  )

  const search = useCallback(async (options?: InvocationRecordsSearchOptions) => {
    const requestSeq = searchSeqRef.current + 1
    searchSeqRef.current = requestSeq
    recordsSeqRef.current += 1
    setIsSearching(true)
    setIsRecordsLoading(true)
    setIsSummaryLoading(true)
    setRecordsError(null)
    setSummaryError(null)

    let listLoaded = false

    try {
      const shouldUseAppliedFilters = options?.source === 'applied' && !options?.draft
      const sourceDraft = options?.draft ?? draftRef.current
      const filters =
        shouldUseAppliedFilters && appliedRef.current
          ? appliedRef.current.filters
          : buildAppliedInvocationFilters(sourceDraft)
      const listResponse = await fetchInvocationRecords(
        buildInvocationRecordsQuery(filters, {
          page: 1,
          pageSize: pageSizeRef.current,
          sortBy: sortByRef.current,
          sortOrder: sortOrderRef.current,
        }),
      )
      if (requestSeq !== searchSeqRef.current) return

      listLoaded = true
      appliedRef.current = { filters, snapshotId: listResponse.snapshotId, generation: requestSeq }
      if (!shouldUseAppliedFilters) {
        setAppliedDraft({ ...sourceDraft })
      }
      const previousRecords = shouldUseAppliedFilters ? recordsRef.current : null
      const { response: visibleListResponse, preservedKeys } = mergeTransientInFlightRecords(
        listResponse,
        previousRecords,
        {
          filters,
          sortBy: sortByRef.current,
          sortOrder: sortOrderRef.current,
          adjustTotal: true,
        },
      )
      if (!options?.preserveSummary) {
        setSummary(null)
      }
      authoritativeTotalRef.current = listResponse.total
      setRecords(visibleListResponse)
      setLiveMergeState({
        matchingVisibleInsertKeys: [],
        countedLiveInsertKeys: preservedKeys,
        countedFilteredOutBaselineKeys: [],
      })
      setPageState(visibleListResponse.page)
      setPageSizeState(visibleListResponse.pageSize)
      setRecordsError(null)
      setIsRecordsLoading(false)
      // Treat search as "list fetch in flight". Once the list is visible, unlock the controls
      // even if the summary request is still pending.
      setIsSearching(false)

      const summaryResponse = await fetchInvocationRecordsSummary({
        ...filters,
        snapshotId: listResponse.snapshotId,
      })
      if (requestSeq !== searchSeqRef.current) return

      setSummary({ ...summaryResponse, newRecordsCount: 0 })
      setSummaryError(null)
    } catch (error) {
      if (requestSeq !== searchSeqRef.current) return
      const message = error instanceof Error ? error.message : String(error)
      if (listLoaded) {
        setSummaryError(message)
      } else {
        setRecordsError(message)
      }
    } finally {
      if (requestSeq === searchSeqRef.current) {
        setIsSearching(false)
        setIsRecordsLoading(false)
        setIsSummaryLoading(false)
      }
    }
  }, [])

  const setPage = useCallback(
    async (nextPage: number) => {
      await loadRecordsPage(nextPage, pageSizeRef.current, sortByRef.current, sortOrderRef.current)
    },
    [loadRecordsPage],
  )

  const setPageSize = useCallback(
    async (nextPageSize: number) => {
      await loadRecordsPage(1, nextPageSize, sortByRef.current, sortOrderRef.current)
    },
    [loadRecordsPage],
  )

  const setSort = useCallback(
    async (nextSortBy: InvocationSortBy, nextSortOrder: InvocationSortOrder) => {
      await loadRecordsPage(1, pageSizeRef.current, nextSortBy, nextSortOrder)
    },
    [loadRecordsPage],
  )

  const resetDraft = useCallback(() => {
    const defaults = createDefaultInvocationRecordsDraft()
    const next = { ...defaults, ...createDefaultCustomRange() }
    draftRef.current = next
    setDraft(next)
  }, [])

  const applyDraft = useCallback(
    async (nextDraft: InvocationRecordsDraftFilters) => {
      draftRef.current = nextDraft
      setDraft(nextDraft)
      await search({ draft: nextDraft })
    },
    [search],
  )

  useEffect(() => {
    void search()
  }, [search])

  const summarySnapshotId = summary?.snapshotId ?? null

  useEffect(() => {
    if (!appliedRef.current || isSearching || !summary) return
    const activeSnapshotId = appliedRef.current.snapshotId
    if (records?.snapshotId !== activeSnapshotId) return
    const timer = window.setInterval(() => {
      if (!shouldPollRecordsSummary()) return
      const applied = appliedRef.current
      if (!applied) return
      const generation = applied.generation
      const pollSeq = pollSeqRef.current + 1
      pollSeqRef.current = pollSeq
      void fetchInvocationRecordsNewCount({
        ...applied.filters,
        snapshotId: applied.snapshotId,
      })
        .then((response: InvocationRecordsNewCountResponse) => {
          if (pollSeq !== pollSeqRef.current) return
          const latest = appliedRef.current
          if (!latest || latest.generation !== generation) return
          if (latest.snapshotId !== response.snapshotId) return
          setSummary((current) => {
            if (!current) return current
            return { ...current, newRecordsCount: response.newRecordsCount }
          })
        })
        .catch(() => {
          // Keep the last successful summary visible when the lightweight poll hiccups.
        })
    }, RECORDS_NEW_COUNT_POLL_INTERVAL_MS)
    return () => window.clearInterval(timer)
  }, [isSearching, records?.snapshotId, summary, summarySnapshotId])

  const reloadCurrentView = useCallback(() => {
    const applied = appliedRef.current
    const currentRecords = recordsRef.current
    if (!applied || !currentRecords) return
    const requestSeq = recordsSeqRef.current + 1
    recordsSeqRef.current = requestSeq
    const { filters, generation, snapshotId } = applied
    let activeSnapshotId = snapshotId

    const isCurrentRequest = () => {
      if (requestSeq !== recordsSeqRef.current) return false
      const latest = appliedRef.current
      return !!latest && latest.generation === generation && latest.snapshotId === activeSnapshotId
    }

    void fetchInvocationRecords(
      buildInvocationRecordsQuery(filters, {
        page: currentRecords.page,
        pageSize: currentRecords.pageSize,
        sortBy: sortByRef.current,
        sortOrder: sortOrderRef.current,
      }),
    )
      .then(async (response) => {
        if (!isCurrentRequest()) return
        activeSnapshotId = response.snapshotId
        appliedRef.current = { filters, snapshotId: response.snapshotId, generation }
        const {
          response: visibleResponse,
          preservedKeys,
        } = mergeTransientInFlightRecords(response, currentRecords, {
          filters,
          sortBy: sortByRef.current,
          sortOrder: sortOrderRef.current,
          adjustTotal: true,
        })
        authoritativeTotalRef.current = response.total
        setRecords(visibleResponse)
        setPageState(visibleResponse.page)
        setPageSizeState(visibleResponse.pageSize)
        pageSizeRef.current = visibleResponse.pageSize
        setRecordsError(null)
        setLiveMergeState({
          matchingVisibleInsertKeys: [],
          countedLiveInsertKeys: preservedKeys,
          countedFilteredOutBaselineKeys: [],
        })
        setSummaryError(null)
        void fetchInvocationRecordsSummary({
          ...filters,
          snapshotId: response.snapshotId,
        })
          .then((summaryResponse) => {
            if (!isCurrentRequest()) return
            setSummary({ ...summaryResponse, newRecordsCount: 0 })
            setSummaryError(null)
          })
          .catch((error) => {
            if (!isCurrentRequest()) return
            setSummaryError(error instanceof Error ? error.message : String(error))
          })
      })
      .catch((error) => {
        if (!isCurrentRequest()) return
        setRecordsError(error instanceof Error ? error.message : String(error))
      })
  }, [])

  useInvocationRecordsRealtime({
    enabled: Boolean(appliedRef.current && records),
    isHydrated: Boolean(appliedRef.current && records && !isSearching && !isRecordsLoading),
    filters: appliedRef.current?.filters,
    sortBy,
    sortOrder,
    limit: pageSize,
    allowVisibleInsertions: page === 1,
    getRecords: () => recordsRef.current?.records ?? [],
    onRecordsChange: (next, meta) => {
      const currentLiveMergeState = liveMergeStateRef.current
      const currentRecordsResponse = recordsRef.current
      const currentVisibleRecords = currentRecordsResponse?.records ?? []
      const currentVisibleKeySet = new Set(
        currentVisibleRecords.map((record) => invocationStableKey(record)),
      )
      const nextVisibleKeySet = new Set(next.map((record) => invocationStableKey(record)))
      const matchingVisibleInsertKeys = currentLiveMergeState.matchingVisibleInsertKeys.filter((key) =>
        nextVisibleKeySet.has(key),
      )
      const countedLiveInsertKeys = [...currentLiveMergeState.countedLiveInsertKeys]
      const countedFilteredOutBaselineKeys = [
        ...currentLiveMergeState.countedFilteredOutBaselineKeys,
      ]
      for (const key of meta.visibleInsertedKeys) {
        if (!matchingVisibleInsertKeys.includes(key)) {
          matchingVisibleInsertKeys.push(key)
        }
        if (!countedLiveInsertKeys.includes(key)) {
          countedLiveInsertKeys.push(key)
        }
      }
      for (const record of meta.payload.records) {
        const key = invocationStableKey(record)
        const stillMatches = matchesInvocationLiveFilters(record, appliedRef.current?.filters)
        if (stillMatches) {
          const removedIndex = countedFilteredOutBaselineKeys.indexOf(key)
          if (removedIndex >= 0) {
            countedFilteredOutBaselineKeys.splice(removedIndex, 1)
          }
          continue
        }

        const liveInsertIndex = countedLiveInsertKeys.indexOf(key)
        if (liveInsertIndex >= 0) {
          countedLiveInsertKeys.splice(liveInsertIndex, 1)
          continue
        }

        if (
          (currentVisibleKeySet.has(key) ||
            countedFilteredOutBaselineKeys.includes(key)) &&
          !countedFilteredOutBaselineKeys.includes(key)
        ) {
          countedFilteredOutBaselineKeys.push(key)
        }
      }
      const total =
        authoritativeTotalRef.current +
        countedLiveInsertKeys.length -
        countedFilteredOutBaselineKeys.length
      setRecords((current) => {
        if (!current) return current
        return {
          ...current,
          total: Math.max(0, total),
          records: next,
        }
      })
      setLiveMergeState({
        matchingVisibleInsertKeys,
        countedLiveInsertKeys,
        countedFilteredOutBaselineKeys,
      })
      setRecordsError(null)
    },
    onOpenResync: reloadCurrentView,
  })

  const api = useMemo(
    () => ({
      draft,
      appliedDraft,
      focus,
      page,
      pageSize,
      sortBy,
      sortOrder,
      records,
      summary: summary
        ? {
            ...summary,
            newRecordsCount: Math.max(
              0,
              summary.newRecordsCount - liveMergeState.matchingVisibleInsertKeys.length,
            ),
          }
        : null,
      recordsError,
      summaryError,
      isSearching,
      isRecordsLoading,
      isSummaryLoading,
      updateDraft: <K extends keyof InvocationRecordsDraftFilters>(key: K, value: InvocationRecordsDraftFilters[K]) => {
        const next = { ...draftRef.current, [key]: value }
        draftRef.current = next
        setDraft(next)
      },
      resetDraft,
      applyDraft,
      setFocus,
      search,
      setPage,
      setPageSize,
      setSort,
    }),
    [appliedDraft, applyDraft, draft, focus, isRecordsLoading, isSearching, isSummaryLoading, liveMergeState.matchingVisibleInsertKeys.length, page, pageSize, records, recordsError, resetDraft, search, setPage, setPageSize, setSort, sortBy, sortOrder, summary, summaryError],
  )

  return api
}
