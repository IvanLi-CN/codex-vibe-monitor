import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  bulkUpdateUpstreamAccounts,
  cancelBulkUpstreamAccountSyncJob,
  cancelImportedOauthValidationJob,
  createBulkUpstreamAccountSyncJob,
  createApiKeyUpstreamAccount,
  createImportedOauthValidationJob,
  createOauthMailboxSession,
  completeOauthLoginSession,
  createOauthLoginSession,
  fetchBulkUpstreamAccountSyncJob,
  deleteOauthMailboxSession,
  deleteUpstreamAccount,
  fetchOauthMailboxStatuses,
  fetchOauthLoginSession,
  fetchUpstreamAccountDetail,
  fetchUpstreamAccounts,
  importValidatedOauthAccounts,
  reloginUpstreamAccount,
  syncUpstreamAccount,
  updateOauthLoginSession,
  updateUpstreamAccountGroup,
  updatePoolRoutingSettings,
  updateUpstreamAccount,
  validateImportedOauthAccounts,
  type BulkUpstreamAccountActionPayload,
  type BulkUpstreamAccountActionResponse,
  type BulkUpstreamAccountSyncJobPayload,
  type BulkUpstreamAccountSyncJobResponse,
  type CreateApiKeyAccountPayload,
  type CompleteOauthLoginSessionPayload,
  type CreateOauthLoginSessionPayload,
  type FetchUpstreamAccountsQuery,
  type ForwardProxyBindingNode,
  type ImportValidatedOauthAccountsPayload,
  type ImportedOauthImportResponse,
  type ImportedOauthValidationJobResponse,
  type ImportedOauthValidationResponse,
  type LoginSessionStatusResponse,
  type OauthMailboxSession,
  type OauthMailboxStatus,
  type PoolRoutingSettings,
  type UpstreamAccountListMetrics,
  type UpstreamAccountGroupSummary,
  type UpdateOauthLoginSessionPayload,
  type UpdatePoolRoutingSettingsPayload,
  type UpdateUpstreamAccountGroupPayload,
  type UpdateUpstreamAccountPayload,
  type UpstreamAccountDetail,
  type UpstreamAccountSummary,
  type ValidateImportedOauthAccountsPayload,
} from '../lib/api'
import { upsertGroupSummary } from '../lib/upstreamAccountGroups'
import { UPSTREAM_ACCOUNTS_CHANGED_EVENT, emitUpstreamAccountsChanged } from '../lib/upstreamAccountsEvents'
import { isUpstreamAccountNotFoundError } from '../lib/upstreamAccountErrors'
import { subscribeToSse, subscribeToSseOpen } from '../lib/sse'

const LOAD_LIST_FAILED = Symbol('load-list-failed')
const DEFAULT_FETCH_UPSTREAM_ACCOUNTS_QUERY: FetchUpstreamAccountsQuery = {}
export const UPSTREAM_ACCOUNTS_SSE_REFRESH_THROTTLE_MS = 5_000
export const UPSTREAM_ACCOUNTS_OPEN_RESYNC_COOLDOWN_MS = 3_000

export type UpstreamAccountsListFreshness = 'fresh' | 'stale' | 'missing' | 'deferred'
export type UpstreamAccountsListLoadingState = 'idle' | 'deferred' | 'initial' | 'switching' | 'refreshing'
export type UpstreamAccountsListStatus = 'ready' | 'loading' | 'error' | 'deferred'

type UpstreamAccountsListState = {
  queryKey: string | null
  dataQueryKey: string | null
  freshness: UpstreamAccountsListFreshness
  loadingState: UpstreamAccountsListLoadingState
  status: UpstreamAccountsListStatus
  hasCurrentQueryData: boolean
  isPending: boolean
}

type UseUpstreamAccountsOptions = {
  allowSelectionOutsideList?: boolean
  fallbackToFirstItem?: boolean
}

const DEFAULT_OPTIONS: Required<UseUpstreamAccountsOptions> = {
  allowSelectionOutsideList: false,
  fallbackToFirstItem: true,
}

interface LoadOptions {
  silent?: boolean
}

function normalizeQueryStringArray(values?: string[]) {
  if (!values || values.length === 0) return undefined
  const normalized = values
    .map((value) => value.trim())
    .filter((value) => value.length > 0)
    .sort()
  return normalized.length > 0 ? normalized : undefined
}

function normalizeQueryNumberArray(values?: number[]) {
  if (!values || values.length === 0) return undefined
  const normalized = values
    .filter((value) => Number.isFinite(value) && value > 0)
    .sort((left, right) => left - right)
  return normalized.length > 0 ? normalized : undefined
}

export function buildUpstreamAccountsListQueryKey(query?: FetchUpstreamAccountsQuery | null) {
  if (query == null) return null

  return JSON.stringify({
    groupSearch: query.groupSearch?.trim() || undefined,
    groupUngrouped: query.groupUngrouped === true ? true : undefined,
    status: query.status?.trim() || undefined,
    workStatus: normalizeQueryStringArray(query.workStatus),
    enableStatus: normalizeQueryStringArray(query.enableStatus),
    healthStatus: normalizeQueryStringArray(query.healthStatus),
    page: query.page ?? undefined,
    pageSize: query.pageSize ?? undefined,
    tagIds: normalizeQueryNumberArray(query.tagIds),
  })
}

export function getUpstreamAccountsSseRefreshDelay(lastRefreshAt: number, now: number) {
  return Math.max(0, UPSTREAM_ACCOUNTS_SSE_REFRESH_THROTTLE_MS - (now - lastRefreshAt))
}

export function shouldTriggerUpstreamAccountsOpenResync(lastResyncAt: number, now: number, force = false) {
  if (force) return true
  return now - lastResyncAt >= UPSTREAM_ACCOUNTS_OPEN_RESYNC_COOLDOWN_MS
}

export function useUpstreamAccounts(
  query: FetchUpstreamAccountsQuery | null = DEFAULT_FETCH_UPSTREAM_ACCOUNTS_QUERY,
  options?: UseUpstreamAccountsOptions,
) {
  const effectiveQuery = query ?? DEFAULT_FETCH_UPSTREAM_ACCOUNTS_QUERY
  const currentListQueryKey = useMemo(() => buildUpstreamAccountsListQueryKey(query), [query])
  const resolvedOptions = {
    ...DEFAULT_OPTIONS,
    ...options,
  }
  const [items, setItems] = useState<UpstreamAccountSummary[]>([])
  const [groups, setGroups] = useState<UpstreamAccountGroupSummary[]>([])
  const [forwardProxyNodes, setForwardProxyNodes] = useState<ForwardProxyBindingNode[]>([])
  const [hasUngroupedAccounts, setHasUngroupedAccounts] = useState(false)
  const [writesEnabled, setWritesEnabled] = useState(true)
  const [routing, setRouting] = useState<PoolRoutingSettings | null>(null)
  const [total, setTotal] = useState(0)
  const [page, setPage] = useState(effectiveQuery.page ?? 1)
  const [pageSize, setPageSize] = useState(effectiveQuery.pageSize ?? 20)
  const [metrics, setMetrics] = useState<UpstreamAccountListMetrics>({
    total: 0,
    oauth: 0,
    apiKey: 0,
    attention: 0,
  })
  const [selectedId, setSelectedId] = useState<number | null>(null)
  const [detail, setDetail] = useState<UpstreamAccountDetail | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [isListPending, setIsListPending] = useState(false)
  const [isDetailLoading, setIsDetailLoading] = useState(false)
  const [listError, setListError] = useState<string | null>(null)
  const [listDataQueryKey, setListDataQueryKey] = useState<string | null>(null)
  const [detailErrors, setDetailErrors] = useState<Record<number, string>>({})
  const [missingDetailAccountId, setMissingDetailAccountId] = useState<number | null>(null)
  const selectedIdRef = useRef<number | null>(null)
  const currentListQueryKeyRef = useRef<string | null>(currentListQueryKey)
  const listRequestSeqRef = useRef(0)
  const detailRequestSeqRef = useRef(0)
  const detailRequestAccountIdRef = useRef<number | null>(null)
  const detailAbortControllerRef = useRef<AbortController | null>(null)
  const hasHydratedRef = useRef(false)
  const detailHydratedAccountIdsRef = useRef(new Set<number>())
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lastRefreshAtRef = useRef(0)
  const lastOpenResyncAtRef = useRef(0)

  useEffect(() => {
    currentListQueryKeyRef.current = currentListQueryKey
  }, [currentListQueryKey])

  const setSelectedAccount = useCallback((accountId: number | null) => {
    selectedIdRef.current = accountId
    setSelectedId(accountId)
    setMissingDetailAccountId((current) => {
      if (accountId == null) return null
      return current === accountId ? null : current
    })
  }, [])

  const clearDetailError = useCallback((accountId: number) => {
    setDetailErrors((current) => {
      if (!(accountId in current)) return current
      const next = { ...current }
      delete next[accountId]
      return next
    })
  }, [])

  const invalidateDetailRequest = useCallback((accountId?: number | null) => {
    if (accountId != null && detailRequestAccountIdRef.current !== accountId) {
      return
    }
    detailRequestSeqRef.current += 1
    detailRequestAccountIdRef.current = null
    detailAbortControllerRef.current?.abort()
    detailAbortControllerRef.current = null
    setIsDetailLoading(false)
  }, [])

  const invalidateListRequest = useCallback(() => {
    listRequestSeqRef.current += 1
    setIsListPending(false)
    setIsLoading(false)
  }, [])

  const clearPendingRefreshTimer = useCallback(() => {
    if (!refreshTimerRef.current) return
    clearTimeout(refreshTimerRef.current)
    refreshTimerRef.current = null
  }, [])

  const loadList = useCallback(
    async (
      preferredId?: number | null,
      options?: { respectCurrentSelection?: boolean; selectionAnchorId?: number | null; silent?: boolean },
    ): Promise<number | null | typeof LOAD_LIST_FAILED> => {
      const requestQueryKey = currentListQueryKeyRef.current
      listRequestSeqRef.current += 1
      const requestSeq = listRequestSeqRef.current
      const shouldShowLoading = !(options?.silent && hasHydratedRef.current)
      setIsListPending(true)
      if (shouldShowLoading) setIsLoading(true)
      setListError(null)
      try {
        const response = await fetchUpstreamAccounts(effectiveQuery)
        if (requestSeq !== listRequestSeqRef.current) {
          return LOAD_LIST_FAILED
        }
        const currentSelectedId = selectedIdRef.current
        const selectionAnchorId = options?.selectionAnchorId ?? preferredId ?? null
        const shouldPreferRequestedId =
          preferredId != null &&
          (!options?.respectCurrentSelection || currentSelectedId === selectionAnchorId)
        const candidateId = shouldPreferRequestedId ? preferredId : currentSelectedId
        const hasCandidateInList =
          candidateId != null && response.items.some((item) => item.id === candidateId)
        const nextSelectedId =
          hasCandidateInList
            ? candidateId
            : candidateId != null && resolvedOptions.allowSelectionOutsideList
              ? candidateId
              : resolvedOptions.fallbackToFirstItem
                ? response.items[0]?.id ?? null
                : null

        setItems(response.items)
        setGroups(response.groups)
        setForwardProxyNodes(response.forwardProxyNodes ?? [])
        setHasUngroupedAccounts(response.hasUngroupedAccounts)
        setWritesEnabled(response.writesEnabled)
        setRouting(response.routing ?? null)
        setTotal(response.total ?? 0)
        setPage(response.page ?? 1)
        setPageSize(response.pageSize ?? 20)
        setMetrics(response.metrics ?? {
          total: 0,
          oauth: 0,
          apiKey: 0,
          attention: 0,
        })
        setListDataQueryKey(requestQueryKey)
        hasHydratedRef.current = true
        setListError(null)
        setSelectedAccount(nextSelectedId)
        return nextSelectedId
      } catch (err) {
        if (requestSeq !== listRequestSeqRef.current) {
          return LOAD_LIST_FAILED
        }
        setListError(err instanceof Error ? err.message : String(err))
        return LOAD_LIST_FAILED
      } finally {
        if (requestSeq === listRequestSeqRef.current) {
          setIsListPending(false)
          if (shouldShowLoading) {
            setIsLoading(false)
          }
        }
      }
    },
    [effectiveQuery, resolvedOptions.allowSelectionOutsideList, resolvedOptions.fallbackToFirstItem, setSelectedAccount],
  )

  const loadDetail = useCallback(async (accountId: number | null, options: LoadOptions = {}) => {
    detailRequestSeqRef.current += 1
    const requestSeq = detailRequestSeqRef.current
    detailAbortControllerRef.current?.abort()
    detailRequestAccountIdRef.current = accountId

    if (accountId == null) {
      setDetail(null)
      setIsDetailLoading(false)
      setMissingDetailAccountId(null)
      return null
    }

    setDetail((current) => (current?.id === accountId ? current : null))
    const shouldShowLoading =
      !(options.silent && detailHydratedAccountIdsRef.current.has(accountId))
    if (shouldShowLoading) setIsDetailLoading(true)
    const controller = new AbortController()
    detailAbortControllerRef.current = controller
    try {
      const response = await fetchUpstreamAccountDetail(accountId, controller.signal)
      if (requestSeq !== detailRequestSeqRef.current || selectedIdRef.current !== accountId) {
        return null
      }
      setDetail(response)
      detailHydratedAccountIdsRef.current.add(accountId)
      clearDetailError(accountId)
      setMissingDetailAccountId((current) => (current === accountId ? null : current))
      return response
    } catch (err) {
      if (controller.signal.aborted) {
        return null
      }
      if (requestSeq !== detailRequestSeqRef.current || selectedIdRef.current !== accountId) {
        return null
      }
      if (isUpstreamAccountNotFoundError(err)) {
        setDetail((current) => (current?.id === accountId ? null : current))
        clearDetailError(accountId)
        setMissingDetailAccountId(accountId)
        return null
      }
      setDetailErrors((current) => ({
        ...current,
        [accountId]: err instanceof Error ? err.message : String(err),
      }))
      return null
    } finally {
      if (requestSeq === detailRequestSeqRef.current) {
        detailRequestAccountIdRef.current = null
        detailAbortControllerRef.current = null
        if (shouldShowLoading) setIsDetailLoading(false)
      }
    }
  }, [clearDetailError])

  useEffect(() => {
    if (query == null) {
      setIsLoading(true)
      setIsListPending(false)
      setListError(null)
      return
    }
    void loadList()
  }, [loadList, query])

  useEffect(() => {
    void loadDetail(selectedId)
  }, [loadDetail, selectedId])

  const selectedSummary = useMemo(
    () => items.find((item) => item.id === selectedId) ?? null,
    [items, selectedId],
  )

  const refreshCurrentSelectedDetail = useCallback(
    async (skipAccountId?: number | null, options: LoadOptions = {}) => {
      const currentSelectedId = selectedIdRef.current
      if (currentSelectedId == null || currentSelectedId === skipAccountId) {
        return
      }
      await loadDetail(currentSelectedId, options)
    },
    [loadDetail],
  )

  const refresh = useCallback(async (options: LoadOptions = {}) => {
    if (query == null) {
      return
    }
    const currentSelectedId = selectedIdRef.current
    const nextSelectedId = await loadList(currentSelectedId, {
      respectCurrentSelection: true,
      selectionAnchorId: currentSelectedId,
      silent: options.silent,
    })
    if (nextSelectedId === LOAD_LIST_FAILED) {
      return
    }
    if (nextSelectedId != null && nextSelectedId === selectedIdRef.current) {
      await loadDetail(nextSelectedId, options)
    }
  }, [loadDetail, loadList, query])

  useEffect(() => {
    const handleChanged = () => {
      void refresh()
    }
    window.addEventListener(UPSTREAM_ACCOUNTS_CHANGED_EVENT, handleChanged)
    return () => {
      window.removeEventListener(UPSTREAM_ACCOUNTS_CHANGED_EVENT, handleChanged)
    }
  }, [refresh])

  const triggerSseRefresh = useCallback(() => {
    const now = Date.now()
    const delay = getUpstreamAccountsSseRefreshDelay(lastRefreshAtRef.current, now)
    const run = () => {
      refreshTimerRef.current = null
      lastRefreshAtRef.current = Date.now()
      void refresh({ silent: true })
    }
    if (delay === 0) {
      clearPendingRefreshTimer()
      run()
      return
    }
    if (refreshTimerRef.current) return
    refreshTimerRef.current = setTimeout(run, delay)
  }, [clearPendingRefreshTimer, refresh])

  const triggerOpenResync = useCallback(
    (force = false) => {
      if (!hasHydratedRef.current) return
      const now = Date.now()
      if (!shouldTriggerUpstreamAccountsOpenResync(lastOpenResyncAtRef.current, now, force)) return
      lastOpenResyncAtRef.current = now
      void refresh({ silent: true })
    },
    [refresh],
  )

  useEffect(() => {
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type !== 'records') return
      triggerSseRefresh()
    })
    return unsubscribe
  }, [triggerSseRefresh])

  useEffect(() => {
    const unsubscribe = subscribeToSseOpen(() => {
      triggerOpenResync()
    })
    return unsubscribe
  }, [triggerOpenResync])

  const selectAccount = useCallback((accountId: number | null) => {
    setSelectedAccount(accountId)
  }, [setSelectedAccount])

  const beginOauthLogin = useCallback(
    async (payload: CreateOauthLoginSessionPayload) => {
      return createOauthLoginSession(payload)
    },
    [],
  )

  const beginRelogin = useCallback(
    async (accountId: number) => {
      return reloginUpstreamAccount(accountId)
    },
    [],
  )

  const getLoginSession = useCallback(async (loginId: string): Promise<LoginSessionStatusResponse> => {
    return fetchOauthLoginSession(loginId)
  }, [])

  const updateOauthLogin = useCallback(
    async (
      loginId: string,
      payload: UpdateOauthLoginSessionPayload,
      baseUpdatedAt?: string | null,
    ): Promise<LoginSessionStatusResponse> => {
      return updateOauthLoginSession(loginId, payload, baseUpdatedAt)
    },
    [],
  )

  const beginOauthMailboxSession = useCallback(async (): Promise<OauthMailboxSession> => {
    const response = await createOauthMailboxSession()
    setListError(null)
    return response
  }, [])

  const beginOauthMailboxSessionForAddress = useCallback(
    async (emailAddress: string): Promise<OauthMailboxSession> => {
      const response = await createOauthMailboxSession({ emailAddress })
      setListError(null)
      return response
    },
    [],
  )

  const getOauthMailboxStatuses = useCallback(async (sessionIds: string[]): Promise<OauthMailboxStatus[]> => {
    const response = await fetchOauthMailboxStatuses({ sessionIds })
    setListError(null)
    return response
  }, [])

  const removeOauthMailboxSession = useCallback(async (sessionId: string) => {
    await deleteOauthMailboxSession(sessionId)
    setListError(null)
  }, [])

  const completeOauthLogin = useCallback(
    async (loginId: string, payload: CompleteOauthLoginSessionPayload) => {
      const response = await completeOauthLoginSession(loginId, payload)
      invalidateListRequest()
      await loadList(response.id)
      invalidateDetailRequest()
      setDetail(response)
      setSelectedAccount(response.id)
      clearDetailError(response.id)
      emitUpstreamAccountsChanged()
      return response
    },
    [clearDetailError, invalidateDetailRequest, invalidateListRequest, loadList, setSelectedAccount],
  )

  const createApiKeyAccount = useCallback(
    async (payload: CreateApiKeyAccountPayload) => {
      const response = await createApiKeyUpstreamAccount(payload)
      invalidateListRequest()
      await loadList(response.id)
      await loadDetail(response.id)
      setSelectedAccount(response.id)
      clearDetailError(response.id)
      emitUpstreamAccountsChanged()
      return response
    },
    [clearDetailError, invalidateListRequest, loadDetail, loadList, setSelectedAccount],
  )

  const runImportedOauthValidation = useCallback(
    async (payload: ValidateImportedOauthAccountsPayload): Promise<ImportedOauthValidationResponse> => {
      return validateImportedOauthAccounts(payload)
    },
    [],
  )

  const startImportedOauthValidationJob = useCallback(
    async (payload: ValidateImportedOauthAccountsPayload): Promise<ImportedOauthValidationJobResponse> => {
      return createImportedOauthValidationJob(payload)
    },
    [],
  )

  const stopImportedOauthValidationJob = useCallback(async (jobId: string) => {
    await cancelImportedOauthValidationJob(jobId)
  }, [])

  const importOauthAccounts = useCallback(
    async (payload: ImportValidatedOauthAccountsPayload): Promise<ImportedOauthImportResponse> => {
      const response = await importValidatedOauthAccounts(payload)
      invalidateListRequest()
      await loadList(selectedIdRef.current, {
        respectCurrentSelection: true,
        selectionAnchorId: selectedIdRef.current,
      })
      await refreshCurrentSelectedDetail()
      emitUpstreamAccountsChanged()
      return response
    },
    [invalidateListRequest, loadList, refreshCurrentSelectedDetail],
  )

  const saveAccount = useCallback(
    async (accountId: number, payload: UpdateUpstreamAccountPayload) => {
      const response = await updateUpstreamAccount(accountId, payload)
      invalidateListRequest()
      invalidateDetailRequest(accountId)
      await loadList(accountId, { respectCurrentSelection: true, selectionAnchorId: accountId })
      clearDetailError(accountId)
      if (selectedIdRef.current === accountId) {
        setDetail(response)
      } else {
        await refreshCurrentSelectedDetail(accountId)
      }
      emitUpstreamAccountsChanged()
      return response
    },
    [clearDetailError, invalidateDetailRequest, invalidateListRequest, loadList, refreshCurrentSelectedDetail],
  )

  const saveRouting = useCallback(async (payload: UpdatePoolRoutingSettingsPayload) => {
    const response = await updatePoolRoutingSettings(payload)
    setRouting(response)
    return response
  }, [])

  const saveGroupNote = useCallback(
    async (groupName: string, payload: UpdateUpstreamAccountGroupPayload) => {
      const response = await updateUpstreamAccountGroup(groupName, payload)
      setGroups((current) => upsertGroupSummary(current, response))
      invalidateListRequest()
      await loadList(selectedIdRef.current, {
        respectCurrentSelection: true,
        selectionAnchorId: selectedIdRef.current,
      })
      emitUpstreamAccountsChanged()
      return response
    },
    [invalidateListRequest, loadList],
  )

  const runBulkAction = useCallback(
    async (payload: BulkUpstreamAccountActionPayload): Promise<BulkUpstreamAccountActionResponse> => {
      const response = await bulkUpdateUpstreamAccounts(payload)
      invalidateListRequest()
      await loadList(selectedIdRef.current, {
        respectCurrentSelection: true,
        selectionAnchorId: selectedIdRef.current,
      })
      await refreshCurrentSelectedDetail()
      emitUpstreamAccountsChanged()
      return response
    },
    [invalidateListRequest, loadList, refreshCurrentSelectedDetail],
  )

  const startBulkSyncJob = useCallback(
    async (payload: BulkUpstreamAccountSyncJobPayload): Promise<BulkUpstreamAccountSyncJobResponse> => {
      return createBulkUpstreamAccountSyncJob(payload)
    },
    [],
  )

  const getBulkSyncJob = useCallback(
    async (jobId: string): Promise<BulkUpstreamAccountSyncJobResponse> => {
      return fetchBulkUpstreamAccountSyncJob(jobId)
    },
    [],
  )

  const stopBulkSyncJob = useCallback(async (jobId: string) => {
    await cancelBulkUpstreamAccountSyncJob(jobId)
  }, [])

  const runSync = useCallback(
    async (accountId: number) => {
      const response = await syncUpstreamAccount(accountId)
      invalidateListRequest()
      invalidateDetailRequest(accountId)
      await loadList(accountId, { respectCurrentSelection: true, selectionAnchorId: accountId })
      clearDetailError(accountId)
      if (selectedIdRef.current === accountId) {
        setDetail(response)
      } else {
        await refreshCurrentSelectedDetail(accountId)
      }
      emitUpstreamAccountsChanged()
      return response
    },
    [clearDetailError, invalidateDetailRequest, invalidateListRequest, loadList, refreshCurrentSelectedDetail],
  )

  const removeAccount = useCallback(
    async (accountId: number) => {
      await deleteUpstreamAccount(accountId)
      const currentSelectedId = selectedIdRef.current
      const shouldReanchorSelection = currentSelectedId === accountId
      const fallbackSelectedId =
        shouldReanchorSelection && resolvedOptions.fallbackToFirstItem
          ? items.find((item) => item.id !== accountId)?.id ?? null
          : null
      invalidateListRequest()
      if (shouldReanchorSelection) {
        invalidateDetailRequest(accountId)
        setSelectedAccount(fallbackSelectedId)
        setDetail((current) => (current?.id === accountId ? null : current))
      }
      const preferredId = shouldReanchorSelection ? fallbackSelectedId : currentSelectedId
      await loadList(preferredId, {
        respectCurrentSelection: !shouldReanchorSelection,
        selectionAnchorId: preferredId,
      })
      clearDetailError(accountId)
      await refreshCurrentSelectedDetail(accountId)
      emitUpstreamAccountsChanged()
    },
    [
      clearDetailError,
      invalidateDetailRequest,
      invalidateListRequest,
      items,
      loadList,
      refreshCurrentSelectedDetail,
      resolvedOptions.fallbackToFirstItem,
      setSelectedAccount,
    ],
  )

  useEffect(
    () => () => {
      listRequestSeqRef.current += 1
      detailRequestSeqRef.current += 1
      detailRequestAccountIdRef.current = null
      detailAbortControllerRef.current?.abort()
      detailAbortControllerRef.current = null
      clearPendingRefreshTimer()
    },
    [clearPendingRefreshTimer],
  )

  const selectedDetailError = selectedId == null ? null : detailErrors[selectedId] ?? null
  const hasCurrentQueryData =
    currentListQueryKey != null && listDataQueryKey === currentListQueryKey
  const listFreshness: UpstreamAccountsListFreshness =
    query == null
      ? 'deferred'
      : hasCurrentQueryData
        ? 'fresh'
        : listDataQueryKey != null
          ? 'stale'
          : 'missing'
  const listLoadingState: UpstreamAccountsListLoadingState =
    query == null
      ? 'deferred'
      : isListPending
        ? hasCurrentQueryData
          ? 'refreshing'
          : listDataQueryKey != null
            ? 'switching'
            : 'initial'
        : 'idle'
  const listStatus: UpstreamAccountsListStatus =
    query == null
      ? 'deferred'
      : isListPending
        ? 'loading'
        : listError != null && !hasCurrentQueryData
          ? 'error'
          : 'ready'
  const listState: UpstreamAccountsListState = {
    queryKey: currentListQueryKey,
    dataQueryKey: listDataQueryKey,
    freshness: listFreshness,
    loadingState: listLoadingState,
    status: listStatus,
    hasCurrentQueryData,
    isPending: isListPending,
  }

  return {
    items,
    groups,
    forwardProxyNodes,
    hasUngroupedAccounts,
    writesEnabled,
    routing,
    selectedId,
    selectedSummary,
    detail,
    isLoading,
    isDetailLoading,
    listError,
    listState,
    detailError: selectedDetailError,
    error: selectedDetailError ?? listError,
    missingDetailAccountId,
    selectAccount,
    refresh,
    loadDetail,
    beginOauthLogin,
    beginRelogin,
    getLoginSession,
    updateOauthLogin,
    beginOauthMailboxSession,
    beginOauthMailboxSessionForAddress,
    getOauthMailboxStatuses,
    removeOauthMailboxSession,
    completeOauthLogin,
    createApiKeyAccount,
    runImportedOauthValidation,
    startImportedOauthValidationJob,
    stopImportedOauthValidationJob,
    importOauthAccounts,
    saveAccount,
    saveRouting,
    saveGroupNote,
    runBulkAction,
    startBulkSyncJob,
    getBulkSyncJob,
    stopBulkSyncJob,
    runSync,
    removeAccount,
    total,
    page,
    pageSize,
    metrics,
  }
}
