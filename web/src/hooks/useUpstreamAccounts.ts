import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  createApiKeyUpstreamAccount,
  createOauthLoginSession,
  deleteUpstreamAccount,
  fetchOauthLoginSession,
  fetchUpstreamAccountDetail,
  fetchUpstreamAccounts,
  reloginUpstreamAccount,
  syncUpstreamAccount,
  updateUpstreamAccount,
  type CreateApiKeyAccountPayload,
  type CreateOauthLoginSessionPayload,
  type LoginSessionStatusResponse,
  type UpdateUpstreamAccountPayload,
  type UpstreamAccountDetail,
  type UpstreamAccountSummary,
} from '../lib/api'

export function useUpstreamAccounts() {
  const [items, setItems] = useState<UpstreamAccountSummary[]>([])
  const [writesEnabled, setWritesEnabled] = useState(true)
  const [selectedId, setSelectedId] = useState<number | null>(null)
  const [detail, setDetail] = useState<UpstreamAccountDetail | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [isDetailLoading, setIsDetailLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const loadList = useCallback(
    async (preferredId?: number | null) => {
      setIsLoading(true)
      try {
        const response = await fetchUpstreamAccounts()
        setItems(response.items)
        setWritesEnabled(response.writesEnabled)
        setError(null)
        setSelectedId((current) => {
          const nextId = preferredId ?? current
          if (nextId != null && response.items.some((item) => item.id === nextId)) {
            return nextId
          }
          return response.items[0]?.id ?? null
        })
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setIsLoading(false)
      }
    },
    [],
  )

  const loadDetail = useCallback(async (accountId: number | null) => {
    if (accountId == null) {
      setDetail(null)
      return null
    }
    setIsDetailLoading(true)
    try {
      const response = await fetchUpstreamAccountDetail(accountId)
      setDetail(response)
      setError(null)
      return response
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      return null
    } finally {
      setIsDetailLoading(false)
    }
  }, [])

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
    await loadList(selectedId)
    await loadDetail(selectedId)
  }, [loadDetail, loadList, selectedId])

  const selectAccount = useCallback((accountId: number) => {
    setSelectedId(accountId)
  }, [])

  const beginOauthLogin = useCallback(
    async (payload: CreateOauthLoginSessionPayload) => {
      const session = await createOauthLoginSession(payload)
      setError(null)
      return session
    },
    [],
  )

  const beginRelogin = useCallback(
    async (accountId: number) => {
      const session = await reloginUpstreamAccount(accountId)
      setError(null)
      return session
    },
    [],
  )

  const getLoginSession = useCallback(async (loginId: string): Promise<LoginSessionStatusResponse> => {
    const response = await fetchOauthLoginSession(loginId)
    setError(null)
    return response
  }, [])

  const createApiKeyAccount = useCallback(
    async (payload: CreateApiKeyAccountPayload) => {
      const response = await createApiKeyUpstreamAccount(payload)
      await loadList(response.id)
      await loadDetail(response.id)
      setSelectedId(response.id)
      setError(null)
      return response
    },
    [loadDetail, loadList],
  )

  const saveAccount = useCallback(
    async (accountId: number, payload: UpdateUpstreamAccountPayload) => {
      const response = await updateUpstreamAccount(accountId, payload)
      await loadList(accountId)
      setDetail(response)
      setSelectedId(accountId)
      setError(null)
      return response
    },
    [loadList],
  )

  const runSync = useCallback(
    async (accountId: number) => {
      const response = await syncUpstreamAccount(accountId)
      await loadList(accountId)
      setDetail(response)
      setSelectedId(accountId)
      setError(null)
      return response
    },
    [loadList],
  )

  const removeAccount = useCallback(
    async (accountId: number) => {
      await deleteUpstreamAccount(accountId)
      const fallbackId = items.find((item) => item.id !== accountId)?.id ?? null
      setSelectedId(fallbackId)
      await loadList(fallbackId)
      await loadDetail(fallbackId)
      setError(null)
    },
    [items, loadDetail, loadList],
  )

  return {
    items,
    writesEnabled,
    selectedId,
    selectedSummary,
    detail,
    isLoading,
    isDetailLoading,
    error,
    selectAccount,
    refresh,
    loadDetail,
    beginOauthLogin,
    beginRelogin,
    getLoginSession,
    createApiKeyAccount,
    saveAccount,
    runSync,
    removeAccount,
  }
}
