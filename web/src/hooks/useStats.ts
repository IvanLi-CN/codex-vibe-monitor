import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { fetchSummary } from '../lib/api'
import type { ApiInvocation, StatsResponse } from '../lib/api'
import { subscribeToSse, subscribeToSseOpen } from '../lib/sse'

interface UseSummaryOptions {
  limit?: number
}

const SUPPORTED_SSE_WINDOWS = new Set(['all', '30m', '1h', '1d', '1mo'])
const CALENDAR_SUMMARY_WINDOWS = new Set(['today', 'yesterday', 'thisWeek', 'thisMonth'])
export const UNSUPPORTED_SSE_REFRESH_INTERVAL_MS = 60_000
export const CALENDAR_SUMMARY_RECORDS_REFRESH_THROTTLE_MS = 1_000
export const CURRENT_SUMMARY_RECORDS_REFRESH_THROTTLE_MS = 600
export const CURRENT_SUMMARY_OPEN_RESYNC_COOLDOWN_MS = 3_000
export const CURRENT_SUMMARY_REQUEST_TIMEOUT_MS = 10_000
export const CURRENT_SUMMARY_RETRY_DELAY_MS = 2_000
export const CURRENT_SUMMARY_MAX_RETRY_ATTEMPTS = 3
export const SUMMARY_REMOUNT_CACHE_TTL_MS = 30_000

interface LoadOptions {
  silent?: boolean
  force?: boolean
  trackCurrentThrottle?: boolean
}

interface PendingLoad {
  silent: boolean
  trackCurrentThrottle: boolean
  waiters: Array<() => void>
}

export interface UnsupportedRefreshGate {
  inFlight: boolean
  lastTriggerAt: number
}

interface SummaryRemountCacheEntry {
  stats: StatsResponse
  cachedAt: number
}

const summaryRemountCache = new Map<string, SummaryRemountCacheEntry>()

export function createUnsupportedRefreshGate(): UnsupportedRefreshGate {
  return { inFlight: false, lastTriggerAt: 0 }
}

export function getSummaryRemountCacheKey(window: string, limit?: number) {
  return `${window}::${limit ?? 'default'}`
}

export function shouldEnableSummaryRemountCache(window: string) {
  return window !== 'current' && !isCalendarSummaryWindow(window)
}

export function readSummaryRemountCache(
  window: string,
  limit?: number,
  now = Date.now(),
  ttlMs = SUMMARY_REMOUNT_CACHE_TTL_MS,
) {
  if (!shouldEnableSummaryRemountCache(window)) return null
  const cached = summaryRemountCache.get(getSummaryRemountCacheKey(window, limit))
  if (!cached) return null
  return shouldReuseSummaryRemountCache(cached.cachedAt, now, ttlMs) ? cached : null
}

export function writeSummaryRemountCache(
  window: string,
  limit: number | undefined,
  stats: StatsResponse,
  cachedAt = Date.now(),
) {
  if (!shouldEnableSummaryRemountCache(window)) return
  summaryRemountCache.set(getSummaryRemountCacheKey(window, limit), {
    stats,
    cachedAt,
  })
}

export function clearSummaryRemountCache() {
  summaryRemountCache.clear()
}

export function shouldReuseSummaryRemountCache(
  cachedAt: number,
  now: number,
  ttlMs = SUMMARY_REMOUNT_CACHE_TTL_MS,
) {
  return now - cachedAt < ttlMs
}

export function isCalendarSummaryWindow(window: string) {
  return CALENDAR_SUMMARY_WINDOWS.has(window)
}

export function getCurrentSummarySseRefreshDelay(lastRefreshAt: number, now: number) {
  return Math.max(0, CURRENT_SUMMARY_RECORDS_REFRESH_THROTTLE_MS - (now - lastRefreshAt))
}

function getLocalDayStartEpoch(nowEpochSeconds = Math.floor(Date.now() / 1000)) {
  const value = new Date(nowEpochSeconds * 1000)
  value.setHours(0, 0, 0, 0)
  return Math.floor(value.getTime() / 1000)
}

function getNextLocalDayStartEpoch(nowEpochSeconds = Math.floor(Date.now() / 1000)) {
  const value = new Date(nowEpochSeconds * 1000)
  value.setHours(24, 0, 0, 0)
  return Math.floor(value.getTime() / 1000)
}

export function shouldRefreshCalendarSummaryOnRecords(window: string) {
  return window === 'today' || window === 'thisWeek' || window === 'thisMonth'
}

export function shouldRefreshYesterdaySummaryOnRecords(
  records: Array<Pick<ApiInvocation, 'occurredAt'>>,
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  const rangeEndEpoch = getLocalDayStartEpoch(nowEpochSeconds)
  const rangeStartEpoch = getLocalDayStartEpoch(rangeEndEpoch - 1)
  return records.some((record) => {
    const occurredEpochMs = Date.parse(record.occurredAt ?? '')
    if (!Number.isFinite(occurredEpochMs)) {
      return false
    }
    const occurredEpoch = Math.floor(occurredEpochMs / 1000)
    return occurredEpoch >= rangeStartEpoch && occurredEpoch < rangeEndEpoch
  })
}

export function shouldForceCalendarSummaryOpenResync(
  window: string,
  lastLoadedLocalDayStartEpoch: number | null,
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  if (window !== 'today' && window !== 'yesterday') {
    return false
  }
  return lastLoadedLocalDayStartEpoch !== getLocalDayStartEpoch(nowEpochSeconds)
}

export function getCalendarSummaryDayRolloverRefreshEpoch(
  window: string,
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  if (window !== 'today' && window !== 'yesterday') {
    return null
  }
  return getNextLocalDayStartEpoch(nowEpochSeconds)
}

export function mergePendingSummarySilentOption(existingSilent: boolean | null, incomingSilent: boolean) {
  return (existingSilent ?? true) && incomingSilent
}

export function shouldTriggerCurrentSummaryOpenResync(lastResyncAt: number, now: number, force = false) {
  if (force) return true
  return now - lastResyncAt >= CURRENT_SUMMARY_OPEN_RESYNC_COOLDOWN_MS
}

export function shouldHandleUnsupportedSummaryRefresh(payloadWindow: string, currentWindow: string, supportsSse: boolean): boolean {
  return payloadWindow !== currentWindow && !supportsSse && currentWindow !== 'current' && !isCalendarSummaryWindow(currentWindow)
}

export function shouldRetryCurrentSummaryError(error: string): boolean {
  const normalized = error.toLowerCase()
  return (
    normalized.includes('timed out') ||
    normalized.includes('timeout') ||
    normalized.includes('failed to fetch') ||
    normalized.includes('network error') ||
    normalized.includes('networkerror')
  )
}

function resolvePendingLoad(pending: PendingLoad | null) {
  if (!pending) {
    return
  }
  pending.waiters.forEach((resolve) => resolve())
}

async function runThrottledSummaryRefresh(
  gate: UnsupportedRefreshGate,
  now: number,
  refreshIntervalMs: number,
  refresh: () => Promise<void>,
): Promise<boolean> {
  if (gate.inFlight || now - gate.lastTriggerAt < refreshIntervalMs) {
    return false
  }

  gate.inFlight = true
  gate.lastTriggerAt = now
  try {
    await refresh()
  } catch {
    // Keep fallback refresh best-effort; hook state already records request errors.
  } finally {
    gate.inFlight = false
  }
  return true
}

export async function runUnsupportedSummaryRefresh(
  gate: UnsupportedRefreshGate,
  now: number,
  refresh: () => Promise<void>,
): Promise<boolean> {
  return runThrottledSummaryRefresh(gate, now, UNSUPPORTED_SSE_REFRESH_INTERVAL_MS, refresh)
}

export async function runCalendarSummaryRefresh(
  gate: UnsupportedRefreshGate,
  now: number,
  refresh: () => Promise<void>,
): Promise<boolean> {
  return runThrottledSummaryRefresh(gate, now, CALENDAR_SUMMARY_RECORDS_REFRESH_THROTTLE_MS, refresh)
}

export function useSummary(window: string, options?: UseSummaryOptions) {
  const initialCachedSummary = readSummaryRemountCache(window, options?.limit)
  const [stats, setStats] = useState<StatsResponse | null>(
    () => initialCachedSummary?.stats ?? null,
  )
  const [isLoading, setIsLoading] = useState(() => initialCachedSummary == null)
  const [error, setError] = useState<string | null>(null)
  const unsupportedRefreshRef = useRef<UnsupportedRefreshGate>(createUnsupportedRefreshGate())
  const calendarRefreshRef = useRef<UnsupportedRefreshGate>(createUnsupportedRefreshGate())
  const summaryContextRef = useRef<{ window: string; limit?: number }>({
    window,
    limit: options?.limit,
  })
  const hasHydratedRef = useRef(initialCachedSummary != null)
  const activeLoadCountRef = useRef(0)
  const pendingLoadRef = useRef<PendingLoad | null>(null)
  const pendingOpenResyncRef = useRef(false)
  const requestSeqRef = useRef(0)
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const dayRolloverTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const activeRequestControllerRef = useRef<AbortController | null>(null)
  const lastCurrentRecordsRefreshAtRef = useRef(0)
  const lastOpenResyncAtRef = useRef(0)
  const currentRetryAttemptRef = useRef(0)
  const lastNaturalDayLoadStartEpochRef = useRef<number | null>(
    window === 'today' || window === 'yesterday'
      ? getLocalDayStartEpoch()
      : null,
  )
  summaryContextRef.current.window = window
  summaryContextRef.current.limit = options?.limit

  const clearPendingRefreshTimer = useCallback(() => {
    if (!refreshTimerRef.current) return
    clearTimeout(refreshTimerRef.current)
    refreshTimerRef.current = null
  }, [])

  const clearDayRolloverTimer = useCallback(() => {
    if (!dayRolloverTimerRef.current) return
    clearTimeout(dayRolloverTimerRef.current)
    dayRolloverTimerRef.current = null
  }, [])

  const clearPendingLoad = useCallback(() => {
    resolvePendingLoad(pendingLoadRef.current)
    pendingLoadRef.current = null
  }, [])

  const runLoad = useCallback(async ({ silent = false }: LoadOptions = {}) => {
    activeLoadCountRef.current += 1
    const requestSeq = requestSeqRef.current + 1
    requestSeqRef.current = requestSeq
    const shouldShowLoading = !(silent && hasHydratedRef.current)
    const isCurrentWindow = summaryContextRef.current.window === 'current'
    const controller = new AbortController()
    const timeoutHandle = isCurrentWindow
      ? setTimeout(() => controller.abort(), CURRENT_SUMMARY_REQUEST_TIMEOUT_MS)
      : null
    activeRequestControllerRef.current = controller
    if (shouldShowLoading) {
      setIsLoading(true)
    }
    try {
      const response = await fetchSummary(summaryContextRef.current.window, {
        limit: summaryContextRef.current.limit,
        signal: controller.signal,
      })
      if (requestSeq !== requestSeqRef.current) return
      setStats(response)
      writeSummaryRemountCache(
        summaryContextRef.current.window,
        summaryContextRef.current.limit,
        response,
      )
      lastNaturalDayLoadStartEpochRef.current =
        summaryContextRef.current.window === 'today' || summaryContextRef.current.window === 'yesterday'
          ? getLocalDayStartEpoch()
          : null
      hasHydratedRef.current = true
      currentRetryAttemptRef.current = 0
      setError(null)
      if (pendingOpenResyncRef.current) {
        pendingOpenResyncRef.current = false
        lastOpenResyncAtRef.current = Date.now()
        if (pendingLoadRef.current) {
          pendingLoadRef.current.silent = mergePendingSummarySilentOption(pendingLoadRef.current.silent, true)
        } else {
          pendingLoadRef.current = { silent: true, trackCurrentThrottle: false, waiters: [] }
        }
      }
    } catch (err) {
      if (requestSeq !== requestSeqRef.current) return
      if (timeoutHandle != null && err instanceof Error && err.name === 'AbortError') {
        setError(`summary request timed out after ${Math.floor(CURRENT_SUMMARY_REQUEST_TIMEOUT_MS / 1000)}s`)
      } else {
        setError(err instanceof Error ? err.message : String(err))
      }
    } finally {
      if (timeoutHandle != null) {
        clearTimeout(timeoutHandle)
      }
      if (activeRequestControllerRef.current === controller) {
        activeRequestControllerRef.current = null
      }
      if (requestSeq === requestSeqRef.current && shouldShowLoading) {
        setIsLoading(false)
      }
      activeLoadCountRef.current = Math.max(0, activeLoadCountRef.current - 1)
    }
  }, [])

  const load = useCallback(async (loadOptions: LoadOptions = {}) => {
    const silent = loadOptions.silent ?? false
    const force = loadOptions.force ?? false
    const trackCurrentThrottle = loadOptions.trackCurrentThrottle ?? false
    if (force) {
      // Force refresh keeps the freshest context: cancel current request and drop stale queued refreshes.
      activeRequestControllerRef.current?.abort()
      clearPendingLoad()
      clearPendingRefreshTimer()
      if (window === 'current') {
        lastCurrentRecordsRefreshAtRef.current = Date.now()
      }
    }

    if (!force && activeLoadCountRef.current > 0) {
      return new Promise<void>((resolve) => {
        if (pendingLoadRef.current) {
          pendingLoadRef.current.silent = mergePendingSummarySilentOption(pendingLoadRef.current.silent, silent)
          pendingLoadRef.current.trackCurrentThrottle ||= trackCurrentThrottle
          pendingLoadRef.current.waiters.push(resolve)
          return
        }
        pendingLoadRef.current = { silent, trackCurrentThrottle, waiters: [resolve] }
      })
    }

    if (trackCurrentThrottle) {
      lastCurrentRecordsRefreshAtRef.current = Date.now()
    }
    await runLoad({ silent })

    while (activeLoadCountRef.current === 0 && pendingLoadRef.current) {
      const pending = pendingLoadRef.current
      pendingLoadRef.current = null
      if (pending.trackCurrentThrottle) {
        lastCurrentRecordsRefreshAtRef.current = Date.now()
      }
      await runLoad({ silent: pending.silent })
      pending.waiters.forEach((resolve) => resolve())
    }
  }, [clearPendingLoad, clearPendingRefreshTimer, runLoad, window])

  const triggerCurrentWindowRefresh = useCallback(() => {
    const now = Date.now()
    const delay = getCurrentSummarySseRefreshDelay(lastCurrentRecordsRefreshAtRef.current, now)
    const run = () => {
      refreshTimerRef.current = null
      void load({ silent: true, trackCurrentThrottle: true })
    }

    if (delay === 0) {
      clearPendingRefreshTimer()
      run()
      return
    }

    if (refreshTimerRef.current) {
      return
    }
    refreshTimerRef.current = setTimeout(run, delay)
  }, [clearPendingRefreshTimer, load])

  const triggerOpenResync = useCallback((force = false) => {
    if (!hasHydratedRef.current) {
      pendingOpenResyncRef.current = true
      return
    }
    const now = Date.now()
    if (!shouldTriggerCurrentSummaryOpenResync(lastOpenResyncAtRef.current, now, force)) {
      return
    }
    lastOpenResyncAtRef.current = now
    void load({ silent: true, force: true })
  }, [load])

  useEffect(() => {
    // Invalidate prior async loads when summary query context changes.
    const cachedSummary = readSummaryRemountCache(window, options?.limit)
    requestSeqRef.current += 1
    setStats(cachedSummary?.stats ?? null)
    setError(null)
    setIsLoading(cachedSummary == null)
    hasHydratedRef.current = cachedSummary != null
    pendingOpenResyncRef.current = false
    lastCurrentRecordsRefreshAtRef.current = 0
    lastOpenResyncAtRef.current = 0
    currentRetryAttemptRef.current = 0
    unsupportedRefreshRef.current = createUnsupportedRefreshGate()
    calendarRefreshRef.current = createUnsupportedRefreshGate()
    lastNaturalDayLoadStartEpochRef.current =
      window === 'today' || window === 'yesterday'
        ? getLocalDayStartEpoch()
        : null
    clearPendingLoad()
    clearPendingRefreshTimer()
    clearDayRolloverTimer()
    if (!cachedSummary) {
      void load({ force: true })
      return
    }
    void load({ silent: true, force: true })
  }, [clearDayRolloverTimer, clearPendingLoad, clearPendingRefreshTimer, load, options?.limit, window])

  useEffect(() => {
    if (!error || window !== 'current' || !shouldRetryCurrentSummaryError(error)) {
      return
    }
    if (currentRetryAttemptRef.current >= CURRENT_SUMMARY_MAX_RETRY_ATTEMPTS) {
      return
    }
    currentRetryAttemptRef.current += 1
    const delay = CURRENT_SUMMARY_RETRY_DELAY_MS * currentRetryAttemptRef.current
    const timer = setTimeout(() => {
      void load({ silent: true, force: true })
    }, delay)
    return () => clearTimeout(timer)
  }, [error, load, window])

  useEffect(
    () => () => {
      requestSeqRef.current += 1
      activeRequestControllerRef.current?.abort()
      activeRequestControllerRef.current = null
      clearPendingLoad()
      pendingOpenResyncRef.current = false
      currentRetryAttemptRef.current = 0
      clearPendingRefreshTimer()
      clearDayRolloverTimer()
    },
    [clearDayRolloverTimer, clearPendingLoad, clearPendingRefreshTimer],
  )

  const supportsSse = useMemo(() => SUPPORTED_SSE_WINDOWS.has(window), [window])

  useEffect(() => {
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type === 'summary') {
        if (payload.window === window) {
          setStats(payload.summary)
          writeSummaryRemountCache(window, options?.limit, payload.summary)
          lastNaturalDayLoadStartEpochRef.current =
            window === 'today' || window === 'yesterday'
              ? getLocalDayStartEpoch()
              : null
          hasHydratedRef.current = true
          setError(null)
          setIsLoading(false)
        } else if (shouldHandleUnsupportedSummaryRefresh(payload.window, window, supportsSse)) {
          void runUnsupportedSummaryRefresh(unsupportedRefreshRef.current, Date.now(), () => load({ silent: true }))
        }
      } else if (payload.type === 'records') {
        if (window === 'current') {
          // current 窗口通过节流静默刷新，避免高频事件导致闪烁。
          triggerCurrentWindowRefresh()
        } else if (
          shouldRefreshCalendarSummaryOnRecords(window) ||
          (window === 'yesterday' &&
            shouldRefreshYesterdaySummaryOnRecords(payload.records))
        ) {
          // calendar windows 依旧通过 HTTP 计算，但 records 到达时以 1s 节流静默补拉。
          void runCalendarSummaryRefresh(calendarRefreshRef.current, Date.now(), () => load({ silent: true }))
        }
      }
    })
    return unsubscribe
  }, [load, options?.limit, supportsSse, triggerCurrentWindowRefresh, window])

  useEffect(() => {
    if (typeof document === 'undefined') return
    const onVisibilityChange = () => {
      if (document.visibilityState !== 'visible') return
      if (
        !shouldForceCalendarSummaryOpenResync(
          window,
          lastNaturalDayLoadStartEpochRef.current,
        )
      ) {
        return
      }
      triggerOpenResync(true)
    }
    document.addEventListener('visibilitychange', onVisibilityChange)
    return () => document.removeEventListener('visibilitychange', onVisibilityChange)
  }, [triggerOpenResync, window])

  useEffect(() => {
    const unsubscribe = subscribeToSseOpen(() => {
      triggerOpenResync()
    })
    return unsubscribe
  }, [triggerOpenResync])

  useEffect(() => {
    clearDayRolloverTimer()
    const refreshEpoch = getCalendarSummaryDayRolloverRefreshEpoch(window)
    if (refreshEpoch == null) {
      return
    }
    const delay = Math.max(0, refreshEpoch * 1000 - Date.now() + 50)
    dayRolloverTimerRef.current = setTimeout(() => {
      void load({ silent: true, force: true })
    }, delay)
    return clearDayRolloverTimer
  }, [clearDayRolloverTimer, load, window])

  return {
    summary: stats,
    isLoading,
    error,
    refresh: load,
  }
}
