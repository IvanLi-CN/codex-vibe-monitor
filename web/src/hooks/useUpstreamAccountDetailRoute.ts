import { useCallback, useMemo } from 'react'
import { useSearchParams } from 'react-router-dom'

const UPSTREAM_ACCOUNT_ID_PARAM = 'upstreamAccountId'

function parseUpstreamAccountId(raw: string | null) {
  if (!raw) return null
  const parsed = Number(raw)
  if (!Number.isFinite(parsed)) return null
  const accountId = Math.trunc(parsed)
  return accountId > 0 ? accountId : null
}

export function useUpstreamAccountDetailRoute() {
  const [searchParams, setSearchParams] = useSearchParams()
  const upstreamAccountId = useMemo(
    () => parseUpstreamAccountId(searchParams.get(UPSTREAM_ACCOUNT_ID_PARAM)),
    [searchParams],
  )

  const openUpstreamAccount = useCallback(
    (accountId: number, options?: { replace?: boolean }) => {
      const next = new URLSearchParams(searchParams)
      next.set(UPSTREAM_ACCOUNT_ID_PARAM, String(Math.trunc(accountId)))
      setSearchParams(next, { replace: options?.replace ?? false })
    },
    [searchParams, setSearchParams],
  )

  const closeUpstreamAccount = useCallback(
    (options?: { replace?: boolean }) => {
      if (!searchParams.has(UPSTREAM_ACCOUNT_ID_PARAM)) return
      const next = new URLSearchParams(searchParams)
      next.delete(UPSTREAM_ACCOUNT_ID_PARAM)
      setSearchParams(next, { replace: options?.replace ?? false })
    },
    [searchParams, setSearchParams],
  )

  return {
    upstreamAccountId,
    openUpstreamAccount,
    closeUpstreamAccount,
  }
}
