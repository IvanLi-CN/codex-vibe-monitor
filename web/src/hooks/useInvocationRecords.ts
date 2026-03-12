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
  setFocus: (focus: InvocationFocus) => void
  search: (options?: InvocationRecordsSearchOptions) => Promise<void>
  setPage: (page: number) => Promise<void>
  setPageSize: (pageSize: number) => Promise<void>
  setSort: (sortBy: InvocationSortBy, sortOrder: InvocationSortOrder) => Promise<void>
}

export interface InvocationRecordsSearchOptions {
  source?: 'draft' | 'applied'
  preserveSummary?: boolean
}

interface SearchState {
  filters: Omit<InvocationRecordsQuery, 'page' | 'pageSize' | 'sortBy' | 'sortOrder' | 'snapshotId'>
  snapshotId: number
  generation: number
}

export function shouldPollRecordsSummary() {
  return typeof document === 'undefined' || document.visibilityState === 'visible'
}

export function useInvocationRecords(): UseInvocationRecordsResult {
  const [draft, setDraft] = useState<InvocationRecordsDraftFilters>(() => {
    const next = createDefaultInvocationRecordsDraft()
    return { ...next, ...createDefaultCustomRange() }
  })
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
  const appliedRef = useRef<SearchState | null>(null)
  const searchSeqRef = useRef(0)
  const recordsSeqRef = useRef(0)
  const pollSeqRef = useRef(0)
  const draftRef = useRef(draft)
  const pageSizeRef = useRef(pageSize)
  const sortByRef = useRef(sortBy)
  const sortOrderRef = useRef(sortOrder)

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
        setRecords(response)
        setPageState(response.page)
        setPageSizeState(response.pageSize)
        pageSizeRef.current = response.pageSize
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
    let nextSnapshotId: number | null = null

    try {
      const filters =
        options?.source === 'applied' && appliedRef.current
          ? appliedRef.current.filters
          : buildAppliedInvocationFilters(draftRef.current)
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
      nextSnapshotId = listResponse.snapshotId
      appliedRef.current = { filters, snapshotId: listResponse.snapshotId, generation: requestSeq }
      if (!options?.preserveSummary) {
        setSummary(null)
      }
      setRecords(listResponse)
      setPageState(listResponse.page)
      setPageSizeState(listResponse.pageSize)
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
        setSummary((current) => {
          if (!current) return null
          return current.snapshotId === nextSnapshotId ? current : null
        })
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
    setDraft({ ...defaults, ...createDefaultCustomRange() })
  }, [])

  useEffect(() => {
    void search()
  }, [search])

  const summarySnapshotId = summary?.snapshotId ?? null

  useEffect(() => {
    if (!appliedRef.current || isSearching || !summary) return
    if (summary.snapshotId !== appliedRef.current.snapshotId) return
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
  }, [isSearching, summary, summarySnapshotId])

  const api = useMemo(
    () => ({
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
      updateDraft: <K extends keyof InvocationRecordsDraftFilters>(key: K, value: InvocationRecordsDraftFilters[K]) => {
        setDraft((current) => ({ ...current, [key]: value }))
      },
      resetDraft,
      setFocus,
      search,
      setPage,
      setPageSize,
      setSort,
    }),
    [draft, focus, isRecordsLoading, isSearching, isSummaryLoading, page, pageSize, records, recordsError, resetDraft, search, setPage, setPageSize, setSort, sortBy, sortOrder, summary, summaryError],
  )

  return api
}
