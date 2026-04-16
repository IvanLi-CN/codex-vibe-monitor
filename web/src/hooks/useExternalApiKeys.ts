import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  createExternalApiKey,
  disableExternalApiKey,
  fetchExternalApiKeys,
  rotateExternalApiKey,
  type ExternalApiKeySummary,
} from '../lib/api'

type RevealedExternalApiKeySecret = {
  action: 'create' | 'rotate'
  key: ExternalApiKeySummary
  secret: string
}

function sortExternalApiKeys(items: ExternalApiKeySummary[]) {
  return [...items].sort((lhs, rhs) => {
    const left = Date.parse(lhs.createdAt)
    const right = Date.parse(rhs.createdAt)
    if (Number.isFinite(left) && Number.isFinite(right) && left !== right) {
      return left - right
    }
    return lhs.id - rhs.id
  })
}

export function useExternalApiKeys() {
  const [items, setItems] = useState<ExternalApiKeySummary[]>([])
  const [isLoading, setIsLoading] = useState(true)
  const [isMutating, setIsMutating] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [revealedSecret, setRevealedSecret] =
    useState<RevealedExternalApiKeySecret | null>(null)

  const refresh = useCallback(async () => {
    setIsLoading(true)
    try {
      const response = await fetchExternalApiKeys()
      setItems(sortExternalApiKeys(response.items))
      setError(null)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setIsLoading(false)
    }
  }, [])

  useEffect(() => {
    void refresh()
  }, [refresh])

  const createKey = useCallback(async (name: string) => {
    setIsMutating(true)
    try {
      const response = await createExternalApiKey({ name })
      setItems((current) => sortExternalApiKeys([...current, response.key]))
      setRevealedSecret({
        action: 'create',
        key: response.key,
        secret: response.secret,
      })
      setError(null)
      return response
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      setError(message)
      throw err
    } finally {
      setIsMutating(false)
    }
  }, [])

  const rotateKey = useCallback(async (id: number) => {
    setIsMutating(true)
    try {
      const response = await rotateExternalApiKey(id)
      setItems((current) =>
        sortExternalApiKeys([
          ...current.filter((item) => item.id !== id),
          response.key,
        ]),
      )
      setRevealedSecret({
        action: 'rotate',
        key: response.key,
        secret: response.secret,
      })
      setError(null)
      return response
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      setError(message)
      throw err
    } finally {
      setIsMutating(false)
    }
  }, [])

  const disableKey = useCallback(async (id: number) => {
    setIsMutating(true)
    try {
      const response = await disableExternalApiKey(id)
      setItems((current) =>
        current.map((item) => (item.id === id ? response.key : item)),
      )
      setError(null)
      return response
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err)
      setError(message)
      throw err
    } finally {
      setIsMutating(false)
    }
  }, [])

  const activeCount = useMemo(
    () => items.filter((item) => item.status === 'active').length,
    [items],
  )

  return {
    items,
    activeCount,
    isLoading,
    isMutating,
    error,
    revealedSecret,
    refresh,
    createKey,
    rotateKey,
    disableKey,
    clearRevealedSecret: () => setRevealedSecret(null),
  }
}
