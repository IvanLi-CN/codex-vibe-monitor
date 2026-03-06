import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { fetchTimeseries } from '../lib/api'
import type { ApiInvocation, TimeseriesPoint, TimeseriesResponse } from '../lib/api'
import { subscribeToSse, subscribeToSseOpen } from '../lib/sse'

export interface UseTimeseriesOptions {
  bucket?: string
  settlementHour?: number
  preferServerAggregation?: boolean
}

export type TimeseriesSyncMode = 'local' | 'current-day-local' | 'server'

export interface TimeseriesSyncPolicy {
  mode: TimeseriesSyncMode
  recordsRefreshThrottleMs: number
}

interface LoadOptions {
  silent?: boolean
  force?: boolean
}

interface PendingLoad {
  silent: boolean
  waiters: Array<() => void>
}

interface UpdateContext {
  range: string
  bucketSeconds?: number
  settlementHour?: number
}

export const TIMESERIES_RECORDS_RESYNC_THROTTLE_MS = 3_000
export const TIMESERIES_OPEN_RESYNC_COOLDOWN_MS = 3_000

export function resolveTimeseriesSyncPolicy(range: string, options?: UseTimeseriesOptions): TimeseriesSyncPolicy {
  if (options?.preferServerAggregation) {
    return {
      mode: 'server',
      recordsRefreshThrottleMs: TIMESERIES_RECORDS_RESYNC_THROTTLE_MS,
    }
  }

  if (range === '1d' && options?.bucket === '1m') {
    return {
      mode: 'local',
      recordsRefreshThrottleMs: 0,
    }
  }

  if (range === '7d' && options?.bucket === '1h') {
    return {
      mode: 'local',
      recordsRefreshThrottleMs: 0,
    }
  }

  if (range === '90d' && options?.bucket === '1d') {
    return {
      mode: 'current-day-local',
      recordsRefreshThrottleMs: 0,
    }
  }

  const bucketSeconds = guessBucketSeconds(options?.bucket) ?? defaultBucketSecondsForRange(range)
  return {
    mode: bucketSeconds >= 86_400 ? 'server' : 'local',
    recordsRefreshThrottleMs: bucketSeconds >= 86_400 ? TIMESERIES_RECORDS_RESYNC_THROTTLE_MS : 0,
  }
}

export function shouldResyncOnRecordsEvent(range: string, options?: UseTimeseriesOptions) {
  return resolveTimeseriesSyncPolicy(range, options).mode === 'server'
}

export function shouldPatchCurrentDayBucketOnRecordsEvent(range: string, options?: UseTimeseriesOptions) {
  return resolveTimeseriesSyncPolicy(range, options).mode === 'current-day-local'
}

export function getTimeseriesRecordsResyncDelay(lastRefreshAt: number, now: number, throttleMs = TIMESERIES_RECORDS_RESYNC_THROTTLE_MS) {
  return Math.max(0, throttleMs - (now - lastRefreshAt))
}

export function shouldTriggerTimeseriesOpenResync(lastResyncAt: number, now: number, force = false) {
  if (force) return true
  return now - lastResyncAt >= TIMESERIES_OPEN_RESYNC_COOLDOWN_MS
}

export function mergePendingTimeseriesSilentOption(existingSilent: boolean | null, incomingSilent: boolean) {
  return (existingSilent ?? true) && incomingSilent
}

function resolvePendingLoad(pending: PendingLoad | null) {
  if (!pending) return
  pending.waiters.forEach((resolve) => resolve())
}

function createSeededTimeseries(range: string, bucket?: string) {
  const bucketSeconds = guessBucketSeconds(bucket) ?? defaultBucketSecondsForRange(range)
  const now = Date.now()
  const rangeSeconds = parseRangeSpec(range) ?? 86_400
  const start = formatEpochToIso(Math.floor((now - rangeSeconds * 1000) / 1000))
  const end = formatEpochToIso(Math.floor(now / 1000))
  return { rangeStart: start, rangeEnd: end, bucketSeconds, points: [] satisfies TimeseriesPoint[] }
}

export function useTimeseries(range: string, options?: UseTimeseriesOptions) {
  const [data, setData] = useState<TimeseriesResponse | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const bucket = options?.bucket
  const settlementHour = options?.settlementHour
  const preferServerAggregation = options?.preferServerAggregation ?? false
  const hasHydratedRef = useRef(false)
  const activeLoadCountRef = useRef(0)
  const pendingLoadRef = useRef<PendingLoad | null>(null)
  const pendingOpenResyncRef = useRef(false)
  const requestSeqRef = useRef(0)
  const activeRequestControllerRef = useRef<AbortController | null>(null)
  const refreshTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const dayRolloverTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const lastRecordsRefreshAtRef = useRef(0)
  const lastOpenResyncAtRef = useRef(0)
  const localRevisionRef = useRef(0)
  const dataRef = useRef<TimeseriesResponse | null>(null)

  const normalizedOptions = useMemo<UseTimeseriesOptions>(
    () => ({
      bucket,
      settlementHour,
      preferServerAggregation,
    }),
    [bucket, settlementHour, preferServerAggregation],
  )

  const syncPolicy = useMemo(
    () => resolveTimeseriesSyncPolicy(range, normalizedOptions),
    [normalizedOptions, range],
  )

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
    const baselineLocalRevision = localRevisionRef.current
    const controller = new AbortController()
    activeRequestControllerRef.current = controller
    const shouldShowLoading = !(silent && hasHydratedRef.current)
    if (shouldShowLoading) {
      setIsLoading(true)
    }

    try {
      const response = await fetchTimeseries(range, {
        ...normalizedOptions,
        signal: controller.signal,
      })
      if (requestSeq !== requestSeqRef.current) {
        return
      }

      const shouldPreserveLocallyPatchedData =
        syncPolicy.mode !== 'server' && baselineLocalRevision !== localRevisionRef.current

      if (shouldPreserveLocallyPatchedData) {
        if (pendingLoadRef.current) {
          pendingLoadRef.current.silent = mergePendingTimeseriesSilentOption(
            pendingLoadRef.current.silent,
            true,
          )
        } else {
          pendingLoadRef.current = { silent: true, waiters: [] }
        }
      } else {
        dataRef.current = response
        setData(response)
      }

      hasHydratedRef.current = true
      setError(null)

      if (pendingOpenResyncRef.current) {
        pendingOpenResyncRef.current = false
        lastOpenResyncAtRef.current = Date.now()
        if (pendingLoadRef.current) {
          pendingLoadRef.current.silent = mergePendingTimeseriesSilentOption(
            pendingLoadRef.current.silent,
            true,
          )
        } else {
          pendingLoadRef.current = { silent: true, waiters: [] }
        }
      }
    } catch (err) {
      if (requestSeq !== requestSeqRef.current) {
        return
      }
      if (err instanceof Error && err.name === 'AbortError') {
        return
      }
      setError(err instanceof Error ? err.message : String(err))
      const fallback = createSeededTimeseries(range, normalizedOptions.bucket)
      dataRef.current = fallback
      setData(fallback)
      hasHydratedRef.current = true
    } finally {
      if (activeRequestControllerRef.current === controller) {
        activeRequestControllerRef.current = null
      }
      if (requestSeq === requestSeqRef.current && shouldShowLoading) {
        setIsLoading(false)
      }
      activeLoadCountRef.current = Math.max(0, activeLoadCountRef.current - 1)
      if (activeLoadCountRef.current === 0) {
        const pendingLoad = pendingLoadRef.current
        if (pendingLoad) {
          pendingLoadRef.current = null
          void runLoad({ silent: pendingLoad.silent }).finally(() => {
            pendingLoad.waiters.forEach((resolve) => resolve())
          })
        }
      }
    }
  }, [normalizedOptions, range, syncPolicy.mode])

  const load = useCallback(async ({ silent = false, force = false }: LoadOptions = {}) => {
    if (force) {
      activeRequestControllerRef.current?.abort()
      clearPendingLoad()
      clearPendingRefreshTimer()
    }

    if (!force && activeLoadCountRef.current > 0) {
      return new Promise<void>((resolve) => {
        if (pendingLoadRef.current) {
          pendingLoadRef.current.silent = mergePendingTimeseriesSilentOption(
            pendingLoadRef.current.silent,
            silent,
          )
          pendingLoadRef.current.waiters.push(resolve)
          return
        }
        pendingLoadRef.current = { silent, waiters: [resolve] }
      })
    }

    if (syncPolicy.mode === 'server') {
      lastRecordsRefreshAtRef.current = Date.now()
    }

    await runLoad({ silent })
  }, [clearPendingLoad, clearPendingRefreshTimer, runLoad, syncPolicy.mode])

  const triggerRecordsResync = useCallback(() => {
    if (typeof document !== 'undefined' && document.visibilityState !== 'visible') return
    const now = Date.now()
    const delay = getTimeseriesRecordsResyncDelay(
      lastRecordsRefreshAtRef.current,
      now,
      syncPolicy.recordsRefreshThrottleMs,
    )
    const run = () => {
      refreshTimerRef.current = null
      lastRecordsRefreshAtRef.current = Date.now()
      void load({ silent: true })
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
  }, [clearPendingRefreshTimer, load, syncPolicy.recordsRefreshThrottleMs])

  const triggerOpenResync = useCallback((force = false) => {
    if (!hasHydratedRef.current) {
      pendingOpenResyncRef.current = true
      return
    }
    const now = Date.now()
    if (!shouldTriggerTimeseriesOpenResync(lastOpenResyncAtRef.current, now, force)) {
      return
    }
    lastOpenResyncAtRef.current = now
    void load({ silent: true, force: true })
  }, [load])

  useEffect(() => {
    requestSeqRef.current += 1
    activeRequestControllerRef.current?.abort()
    activeRequestControllerRef.current = null
    hasHydratedRef.current = false
    pendingOpenResyncRef.current = false
    lastRecordsRefreshAtRef.current = 0
    lastOpenResyncAtRef.current = 0
    localRevisionRef.current = 0
    dataRef.current = null
    clearPendingLoad()
    clearPendingRefreshTimer()
    clearDayRolloverTimer()
    void load({ force: true })
  }, [clearDayRolloverTimer, clearPendingLoad, clearPendingRefreshTimer, load, options?.bucket, options?.preferServerAggregation, options?.settlementHour, range])

  useEffect(() => {
    if (!error) return
    const id = setTimeout(() => {
      void load()
    }, 2000)
    return () => clearTimeout(id)
  }, [error, load])

  useEffect(() => {
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type !== 'records') return

      if (syncPolicy.mode === 'server') {
        triggerRecordsResync()
        return
      }

      if (syncPolicy.mode === 'current-day-local') {
        const nowEpochSeconds = Math.floor(Date.now() / 1000)
        if (shouldResyncForCurrentDayBucket(dataRef.current, nowEpochSeconds)) {
          triggerOpenResync(true)
          return
        }
        setData((current) => {
          const next = applyRecordsToCurrentDayBucket(current, payload.records, nowEpochSeconds)
          if (next !== current) {
            dataRef.current = next
            localRevisionRef.current += 1
          }
          return next
        })
        return
      }

      setData((current) => {
        const seeded = current ?? createSeededTimeseries(range, normalizedOptions.bucket)
        const next = applyRecordsToTimeseries(seeded, payload.records, {
          range,
          bucketSeconds: seeded.bucketSeconds,
          settlementHour: normalizedOptions.settlementHour,
        })
        if (next !== current) {
          dataRef.current = next
          localRevisionRef.current += 1
        }
        return next
      })
    })
    return unsubscribe
  }, [normalizedOptions.bucket, normalizedOptions.settlementHour, range, syncPolicy.mode, triggerOpenResync, triggerRecordsResync])

  useEffect(() => {
    if (typeof document === 'undefined') return
    const onVisibilityChange = () => {
      if (document.visibilityState !== 'visible') return
      triggerOpenResync(syncPolicy.mode === 'current-day-local')
    }
    document.addEventListener('visibilitychange', onVisibilityChange)
    return () => document.removeEventListener('visibilitychange', onVisibilityChange)
  }, [syncPolicy.mode, triggerOpenResync])

  useEffect(() => {
    const unsubscribe = subscribeToSseOpen(() => {
      triggerOpenResync()
    })
    return unsubscribe
  }, [triggerOpenResync])

  useEffect(() => {
    clearDayRolloverTimer()
    if (syncPolicy.mode !== 'current-day-local') {
      return
    }
    const currentBucketEndEpoch = getCurrentDayBucketEndEpoch(data)
    if (currentBucketEndEpoch == null) {
      return
    }
    const delay = Math.max(0, currentBucketEndEpoch * 1000 - Date.now() + 50)
    dayRolloverTimerRef.current = setTimeout(() => {
      void load({ silent: true, force: true })
    }, delay)
    return clearDayRolloverTimer
  }, [clearDayRolloverTimer, data, load, syncPolicy.mode])

  useEffect(
    () => () => {
      requestSeqRef.current += 1
      activeRequestControllerRef.current?.abort()
      activeRequestControllerRef.current = null
      clearPendingLoad()
      clearPendingRefreshTimer()
      clearDayRolloverTimer()
      pendingOpenResyncRef.current = false
    },
    [clearDayRolloverTimer, clearPendingLoad, clearPendingRefreshTimer],
  )

  return {
    data,
    isLoading,
    error,
    refresh: load,
  }
}

export function getCurrentDayBucketEndEpoch(current: TimeseriesResponse | null, nowEpochSeconds = Math.floor(Date.now() / 1000)) {
  const currentBucket = getCurrentDayBucket(current, nowEpochSeconds)
  return parseIsoEpoch(currentBucket?.bucketEnd)
}

export function shouldResyncForCurrentDayBucket(current: TimeseriesResponse | null, nowEpochSeconds = Math.floor(Date.now() / 1000)) {
  if (!current || current.points.length === 0) {
    return false
  }
  return getCurrentDayBucket(current, nowEpochSeconds) == null
}

function getCurrentDayBucket(current: TimeseriesResponse | null, nowEpochSeconds: number) {
  if (!current) return null
  for (let index = current.points.length - 1; index >= 0; index -= 1) {
    const point = current.points[index]
    const bucketStartEpoch = parseIsoEpoch(point.bucketStart)
    const bucketEndEpoch = parseIsoEpoch(point.bucketEnd)
    if (bucketStartEpoch == null || bucketEndEpoch == null) continue
    if (nowEpochSeconds >= bucketStartEpoch && nowEpochSeconds < bucketEndEpoch) {
      return point
    }
  }
  return null
}

export function applyRecordsToCurrentDayBucket(
  current: TimeseriesResponse | null,
  records: ApiInvocation[],
  nowEpochSeconds = Math.floor(Date.now() / 1000),
) {
  if (!current || records.length === 0) {
    return current
  }

  const currentBucket = getCurrentDayBucket(current, nowEpochSeconds)
  if (!currentBucket) {
    return current
  }

  const bucketStartEpoch = parseIsoEpoch(currentBucket.bucketStart)
  const bucketEndEpoch = parseIsoEpoch(currentBucket.bucketEnd)
  if (bucketStartEpoch == null || bucketEndEpoch == null) {
    return current
  }

  const nextPoints = current.points.map((point) =>
    point.bucketStart === currentBucket.bucketStart ? { ...point } : point,
  )
  const nextBucket = nextPoints.find((point) => point.bucketStart === currentBucket.bucketStart)
  if (!nextBucket) {
    return current
  }

  let mutating = false
  for (const record of records) {
    const occurredEpoch = parseIsoEpoch(record.occurredAt)
    if (occurredEpoch == null || occurredEpoch < bucketStartEpoch || occurredEpoch >= bucketEndEpoch) {
      continue
    }
    nextBucket.totalCount += 1
    if (record.status === 'success') {
      nextBucket.successCount += 1
    } else {
      nextBucket.failureCount += 1
    }
    nextBucket.totalTokens += record.totalTokens ?? 0
    nextBucket.totalCost += record.cost ?? 0
    mutating = true
  }

  if (!mutating) {
    return current
  }

  return {
    ...current,
    points: nextPoints,
  }
}

export function applyRecordsToTimeseries(
  current: TimeseriesResponse | null,
  records: ApiInvocation[],
  context: UpdateContext,
) {
  if (!current || records.length === 0) {
    return current
  }

  const bucketSeconds = context.bucketSeconds
  if (!bucketSeconds || bucketSeconds <= 0) {
    return current
  }

  const offsetSeconds = bucketSeconds >= 86_400 ? (context.settlementHour ?? 0) * 3_600 : 0
  const rangeSeconds = parseRangeSpec(context.range)
  const points = new Map<string, TimeseriesPoint>()
  for (const point of current.points) {
    points.set(point.bucketStart, { ...point })
  }

  let mutating = false
  let latestRangeEndEpoch = parseIsoEpoch(current.rangeEnd)

  for (const record of records) {
    const occurredEpoch = parseIsoEpoch(record.occurredAt)
    if (occurredEpoch == null) continue

    if (latestRangeEndEpoch == null) {
      latestRangeEndEpoch = occurredEpoch + bucketSeconds
    }

    if (rangeSeconds != null && latestRangeEndEpoch != null) {
      const earliestAllowed = latestRangeEndEpoch - rangeSeconds
      if (occurredEpoch < earliestAllowed) {
        continue
      }
    }

    const bucketStartEpoch = alignBucketEpoch(occurredEpoch, bucketSeconds, offsetSeconds)
    const bucketEndEpoch = bucketStartEpoch + bucketSeconds
    const bucketStart = formatEpochToIso(bucketStartEpoch)
    const bucketEnd = formatEpochToIso(bucketEndEpoch)

    let point = points.get(bucketStart)
    if (!point) {
      point = {
        bucketStart,
        bucketEnd,
        totalCount: 0,
        successCount: 0,
        failureCount: 0,
        totalTokens: 0,
        totalCost: 0,
      }
      points.set(bucketStart, point)
    }

    point.bucketEnd = bucketEnd
    point.totalCount += 1
    if (record.status === 'success') {
      point.successCount += 1
    } else {
      point.failureCount += 1
    }
    point.totalTokens += record.totalTokens ?? 0
    point.totalCost += record.cost ?? 0
    mutating = true

    if (latestRangeEndEpoch == null || bucketEndEpoch > latestRangeEndEpoch) {
      latestRangeEndEpoch = bucketEndEpoch
    }
  }

  if (!mutating) {
    return current
  }

  const sortedPoints = Array.from(points.values()).sort((a, b) => {
    const aEpoch = parseIsoEpoch(a.bucketStart) ?? 0
    const bEpoch = parseIsoEpoch(b.bucketStart) ?? 0
    return aEpoch - bEpoch
  })

  if (rangeSeconds != null && latestRangeEndEpoch != null) {
    const earliestAllowed = latestRangeEndEpoch - rangeSeconds
    while (sortedPoints.length > 0) {
      const first = sortedPoints[0]
      const firstEndEpoch = parseIsoEpoch(first.bucketEnd)
      if (firstEndEpoch != null && firstEndEpoch <= earliestAllowed) {
        sortedPoints.shift()
        continue
      }
      break
    }
  }

  const nextRangeEndEpoch = latestRangeEndEpoch ?? parseIsoEpoch(current.rangeEnd)
  const nextRangeEnd = nextRangeEndEpoch != null ? formatEpochToIso(nextRangeEndEpoch) : current.rangeEnd
  const nextRangeStart =
    rangeSeconds != null && nextRangeEndEpoch != null
      ? formatEpochToIso(nextRangeEndEpoch - rangeSeconds)
      : current.rangeStart

  return {
    ...current,
    rangeStart: nextRangeStart,
    rangeEnd: nextRangeEnd,
    points: sortedPoints,
  }
}

function parseRangeSpec(range: string) {
  if (range.endsWith('mo')) {
    const value = Number(range.slice(0, -2))
    return Number.isFinite(value) ? value * 30 * 86_400 : null
  }
  const unit = range.slice(-1)
  const value = Number(range.slice(0, -1))
  if (!Number.isFinite(value)) return null
  switch (unit) {
    case 'd':
      return value * 86_400
    case 'h':
      return value * 3_600
    case 'm':
      return value * 60
    default:
      return null
  }
}

function alignBucketEpoch(epochSeconds: number, bucketSeconds: number, offsetSeconds: number) {
  const adjusted = epochSeconds - offsetSeconds
  const aligned = Math.floor(adjusted / bucketSeconds) * bucketSeconds + offsetSeconds
  return aligned
}

function parseIsoEpoch(value?: string | null) {
  if (!value) return null
  const t = Date.parse(value)
  if (Number.isNaN(t)) return null
  return Math.floor(t / 1000)
}

function formatEpochToIso(epochSeconds: number) {
  return new Date(epochSeconds * 1000).toISOString().replace(/\.\d{3}Z$/, 'Z')
}

function guessBucketSeconds(spec?: string) {
  switch (spec) {
    case '1m':
      return 60
    case '5m':
      return 300
    case '15m':
      return 900
    case '30m':
      return 1800
    case '1h':
      return 3600
    case '6h':
      return 21600
    case '12h':
      return 43200
    case '1d':
      return 86400
    default:
      return undefined
  }
}

function defaultBucketSecondsForRange(range: string) {
  const sec = parseRangeSpec(range) ?? 86_400
  if (sec <= 3_600) return 60
  if (sec <= 172_800) return 1_800
  if (sec <= 2_592_000) return 3_600
  return 86_400
}
