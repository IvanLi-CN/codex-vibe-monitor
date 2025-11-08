import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { fetchVersion } from '../lib/api'
import { subscribeToSseOpen } from '../lib/sse'

const DISMISS_KEY = 'update-dismissed-version'

export function useUpdateAvailable() {
  const [currentVersion, setCurrentVersion] = useState<string | null>(null)
  const [availableVersion, setAvailableVersion] = useState<string | null>(null)
  const [visible, setVisible] = useState(false)
  const initialVersionRef = useRef<string | null>(null)

  const dismissed = useMemo(() => {
    try {
      return localStorage.getItem(DISMISS_KEY)
    } catch {
      return null
    }
  }, [])

  const loadVersion = useCallback(async () => {
    try {
      const v = await fetchVersion()
      return v.backend
    } catch {
      return null
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    ;(async () => {
      const v = await loadVersion()
      if (cancelled) return
      setCurrentVersion(v)
      initialVersionRef.current = v
    })()
    return () => {
      cancelled = true
    }
  }, [loadVersion])

  useEffect(() => {
    const unsubscribe = subscribeToSseOpen(async () => {
      const next = await loadVersion()
      if (!next) return
      const initial = initialVersionRef.current
      if (!initial) {
        initialVersionRef.current = next
        setCurrentVersion(next)
        return
      }
      if (next !== initial && next !== dismissed) {
        setAvailableVersion(next)
        setVisible(true)
      }
    })
    return unsubscribe
  }, [dismissed, loadVersion])

  const dismiss = useCallback(() => {
    if (availableVersion) {
      try {
        localStorage.setItem(DISMISS_KEY, availableVersion)
      } catch (err) {
        // ignore storage errors (Safari private mode, etc.)
        void err
      }
    }
    setVisible(false)
  }, [availableVersion])

  const reload = useCallback(() => {
    window.location.reload()
  }, [])

  // Dev-only helper to force showing the banner
  useEffect(() => {
    if (!import.meta.env.DEV) return
    ;(window as unknown as { __DEV_FORCE_UPDATE_BANNER__?: () => void }).__DEV_FORCE_UPDATE_BANNER__ = () => {
      setAvailableVersion((v) => v ?? (currentVersion ? `${currentVersion}-dev` : 'dev-next'))
      setVisible(true)
    }
  }, [currentVersion])

  return {
    currentVersion,
    availableVersion,
    visible,
    dismiss,
    reload,
  }
}

export default useUpdateAvailable
