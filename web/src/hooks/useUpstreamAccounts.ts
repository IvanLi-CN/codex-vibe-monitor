import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  createApiKeyUpstreamAccount,
  completeOauthLoginSession,
  createOauthLoginSession,
  deleteUpstreamAccount,
  fetchOauthLoginSession,
  fetchUpstreamAccountDetail,
  fetchUpstreamAccounts,
  reloginUpstreamAccount,
  syncUpstreamAccount,
  updateUpstreamAccountGroup,
  updatePoolRoutingSettings,
  updateUpstreamAccount,
  type CreateApiKeyAccountPayload,
  type CompleteOauthLoginSessionPayload,
  type CreateOauthLoginSessionPayload,
  type LoginSessionStatusResponse,
  type PoolRoutingSettings,
  type UpstreamAccountGroupSummary,
  type UpdatePoolRoutingSettingsPayload,
  type UpdateUpstreamAccountGroupPayload,
  type UpdateUpstreamAccountPayload,
  type UpstreamAccountDetail,
  type UpstreamAccountSummary,
} from '../lib/api'
import { upsertGroupSummary } from '../lib/upstreamAccountGroups'
import { UPSTREAM_ACCOUNTS_CHANGED_EVENT, emitUpstreamAccountsChanged } from '../lib/upstreamAccountsEvents'

const LOAD_LIST_FAILED = Symbol('load-list-failed')

export function useUpstreamAccounts() {
  const [items, setItems] = useState<UpstreamAccountSummary[]>([])
  const [groups, setGroups] = useState<UpstreamAccountGroupSummary[]>([])
  const [writesEnabled, setWritesEnabled] = useState(true)
  const [routing, setRouting] = useState<PoolRoutingSettings | null>(null)
  const [selectedId, setSelectedId] = useState<number | null>(null)
  const [detail, setDetail] = useState<UpstreamAccountDetail | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [isDetailLoading, setIsDetailLoading] = useState(false)
  const [listError, setListError] = useState<string | null>(null)
  const [detailError, setDetailError] = useState<{ accountId: number; message: string } | null>(null)
  const selectedIdRef = useRef<number | null>(null)
  const detailRequestSeqRef = useRef(0)
  const detailAbortControllerRef = useRef<AbortController | null>(null)

  const setSelectedAccount = useCallback((accountId: number | null) => {
    selectedIdRef.current = accountId
    setSelectedId(accountId)
  }, [])

  const clearDetailError = useCallback((accountId: number) => {
    setDetailError((current) => (current?.accountId === accountId ? null : current))
  }, [])

  const invalidateDetailRequest = useCallback(() => {
    detailRequestSeqRef.current += 1
    detailAbortControllerRef.current?.abort()
    detailAbortControllerRef.current = null
    setIsDetailLoading(false)
  }, [])

  const loadList = useCallback(
    async (
      preferredId?: number | null,
      options?: { respectCurrentSelection?: boolean; selectionAnchorId?: number | null },
    ): Promise<number | null | typeof LOAD_LIST_FAILED> => {
      setIsLoading(true)
      try {
        const response = await fetchUpstreamAccounts()
        const currentSelectedId = selectedIdRef.current
        const selectionAnchorId = options?.selectionAnchorId ?? preferredId ?? null
        const shouldPreferRequestedId =
          preferredId != null &&
          (!options?.respectCurrentSelection || currentSelectedId === selectionAnchorId)
        const candidateId = shouldPreferRequestedId ? preferredId : currentSelectedId
        const nextSelectedId =
          candidateId != null && response.items.some((item) => item.id === candidateId)
            ? candidateId
            : response.items[0]?.id ?? null

        setItems(response.items)
        setGroups(response.groups)
        setWritesEnabled(response.writesEnabled)
        setRouting(response.routing ?? null)
        setListError(null)
        setSelectedAccount(nextSelectedId)
        return nextSelectedId
      } catch (err) {
        setListError(err instanceof Error ? err.message : String(err))
        return LOAD_LIST_FAILED
      } finally {
        setIsLoading(false)
      }
    },
    [setSelectedAccount],
  )

  const loadDetail = useCallback(async (accountId: number | null) => {
    detailRequestSeqRef.current += 1
    const requestSeq = detailRequestSeqRef.current
    detailAbortControllerRef.current?.abort()

    if (accountId == null) {
      setDetail(null)
      setIsDetailLoading(false)
      return null
    }

    setDetail((current) => (current?.id === accountId ? current : null))
    setIsDetailLoading(true)
    const controller = new AbortController()
    detailAbortControllerRef.current = controller
    try {
      const response = await fetchUpstreamAccountDetail(accountId, controller.signal)
      if (requestSeq !== detailRequestSeqRef.current || selectedIdRef.current !== accountId) {
        return null
      }
      setDetail(response)
      clearDetailError(accountId)
      return response
    } catch (err) {
      if (controller.signal.aborted) {
        return null
      }
      if (requestSeq !== detailRequestSeqRef.current || selectedIdRef.current !== accountId) {
        return null
      }
      setDetailError({
        accountId,
        message: err instanceof Error ? err.message : String(err),
      })
      return null
    } finally {
      if (requestSeq === detailRequestSeqRef.current) {
        detailAbortControllerRef.current = null
        setIsDetailLoading(false)
      }
    }
  }, [clearDetailError])

  useEffect(() => {
    void loadList()
  }, [loadList])

  useEffect(() => {
    void loadDetail(selectedId)
  }, [loadDetail, selectedId])

  const selectedSummary = useMemo(
    () => items.find((item) => item.id === selectedId) ?? null,
    [items, selectedId],
  )

  const refresh = useCallback(async () => {
    const currentSelectedId = selectedIdRef.current
    const nextSelectedId = await loadList(currentSelectedId, {
      respectCurrentSelection: true,
      selectionAnchorId: currentSelectedId,
    })
    if (nextSelectedId === LOAD_LIST_FAILED) {
      return
    }
    if (nextSelectedId === currentSelectedId) {
      await loadDetail(nextSelectedId)
    }
  }, [loadDetail, loadList])

  useEffect(() => {
    const handleChanged = () => {
      void refresh()
    }
    window.addEventListener(UPSTREAM_ACCOUNTS_CHANGED_EVENT, handleChanged)
    return () => {
      window.removeEventListener(UPSTREAM_ACCOUNTS_CHANGED_EVENT, handleChanged)
    }
  }, [refresh])

  const selectAccount = useCallback((accountId: number) => {
    setSelectedAccount(accountId)
  }, [setSelectedAccount])

  const beginOauthLogin = useCallback(
    async (payload: CreateOauthLoginSessionPayload) => {
      const session = await createOauthLoginSession(payload)
      setListError(null)
      return session
    },
    [],
  )

  const beginRelogin = useCallback(
    async (accountId: number) => {
      const session = await reloginUpstreamAccount(accountId)
      setListError(null)
      return session
    },
    [],
  )

  const getLoginSession = useCallback(async (loginId: string): Promise<LoginSessionStatusResponse> => {
    const response = await fetchOauthLoginSession(loginId)
    setListError(null)
    return response
  }, [])

  const completeOauthLogin = useCallback(
    async (loginId: string, payload: CompleteOauthLoginSessionPayload) => {
      const response = await completeOauthLoginSession(loginId, payload)
      await loadList(response.id)
      invalidateDetailRequest()
      setDetail(response)
      setSelectedAccount(response.id)
      clearDetailError(response.id)
      setListError(null)
      emitUpstreamAccountsChanged()
      return response
    },
    [clearDetailError, invalidateDetailRequest, loadList, setSelectedAccount],
  )

  const createApiKeyAccount = useCallback(
    async (payload: CreateApiKeyAccountPayload) => {
      const response = await createApiKeyUpstreamAccount(payload)
      await loadList(response.id)
      await loadDetail(response.id)
      setSelectedAccount(response.id)
      clearDetailError(response.id)
      setListError(null)
      emitUpstreamAccountsChanged()
      return response
    },
    [clearDetailError, loadDetail, loadList, setSelectedAccount],
  )

  const saveAccount = useCallback(
    async (accountId: number, payload: UpdateUpstreamAccountPayload) => {
      const response = await updateUpstreamAccount(accountId, payload)
      await loadList(accountId, { respectCurrentSelection: true, selectionAnchorId: accountId })
      if (selectedIdRef.current === accountId) {
        invalidateDetailRequest()
        setDetail(response)
        clearDetailError(accountId)
      }
      emitUpstreamAccountsChanged()
      return response
    },
    [clearDetailError, invalidateDetailRequest, loadList],
  )

  const saveRouting = useCallback(async (payload: UpdatePoolRoutingSettingsPayload) => {
    const response = await updatePoolRoutingSettings(payload)
    setRouting(response)
    setListError(null)
    return response
  }, [])

  const saveGroupNote = useCallback(
    async (groupName: string, payload: UpdateUpstreamAccountGroupPayload) => {
      const response = await updateUpstreamAccountGroup(groupName, payload)
      setGroups((current) => upsertGroupSummary(current, response))
      await loadList(selectedIdRef.current, {
        respectCurrentSelection: true,
        selectionAnchorId: selectedIdRef.current,
      })
      emitUpstreamAccountsChanged()
      return response
    },
    [loadList],
  )

  const runSync = useCallback(
    async (accountId: number) => {
      const response = await syncUpstreamAccount(accountId)
      await loadList(accountId, { respectCurrentSelection: true, selectionAnchorId: accountId })
      if (selectedIdRef.current === accountId) {
        invalidateDetailRequest()
        setDetail(response)
        clearDetailError(accountId)
      }
      emitUpstreamAccountsChanged()
      return response
    },
    [clearDetailError, invalidateDetailRequest, loadList],
  )

  const removeAccount = useCallback(
    async (accountId: number) => {
      await deleteUpstreamAccount(accountId)
      const fallbackId = items.find((item) => item.id !== accountId)?.id ?? null
      setSelectedAccount(fallbackId)
      await loadList(fallbackId)
      await loadDetail(fallbackId)
      setListError(null)
      emitUpstreamAccountsChanged()
    },
    [items, loadDetail, loadList, setSelectedAccount],
  )

  useEffect(
    () => () => {
      detailRequestSeqRef.current += 1
      detailAbortControllerRef.current?.abort()
      detailAbortControllerRef.current = null
    },
    [],
  )

  return {
    items,
    groups,
    writesEnabled,
    routing,
    selectedId,
    selectedSummary,
    detail,
    isLoading,
    isDetailLoading,
    error: (detailError?.accountId === selectedId ? detailError.message : null) ?? listError,
    selectAccount,
    refresh,
    loadDetail,
    beginOauthLogin,
    beginRelogin,
    getLoginSession,
    completeOauthLogin,
    createApiKeyAccount,
    saveAccount,
    saveRouting,
    saveGroupNote,
    runSync,
    removeAccount,
  }
}
