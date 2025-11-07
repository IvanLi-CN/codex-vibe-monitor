import { useCallback, useEffect, useMemo, useState } from 'react'
import { fetchTimeseries } from '../lib/api'
import type { ApiInvocation, TimeseriesPoint, TimeseriesResponse } from '../lib/api'
import { subscribeToSse } from '../lib/sse'

export interface UseTimeseriesOptions {
  bucket?: string
  settlementHour?: number
}

export function useTimeseries(range: string, options?: UseTimeseriesOptions) {
  const [data, setData] = useState<TimeseriesResponse | null>(null)
  const [isLoading, setIsLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const bucket = options?.bucket
  const settlementHour = options?.settlementHour

  const normalizedOptions = useMemo<UseTimeseriesOptions>(
    () => ({
      bucket,
      settlementHour,
    }),
    [bucket, settlementHour],
  )

  const load = useCallback(async () => {
    setIsLoading(true)
    try {
      const response = await fetchTimeseries(range, normalizedOptions)
      setData(response)
      setError(null)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      // Bootstrap an empty timeseries so SSE records can hydrate the view
      const bucketSeconds = guessBucketSeconds(bucket) ?? defaultBucketSecondsForRange(range)
      const now = Date.now()
      const rangeSeconds = parseRangeSpec(range) ?? 86_400
      const start = formatEpochToIso(Math.floor((now - rangeSeconds * 1000) / 1000))
      const end = formatEpochToIso(Math.floor(now / 1000))
      setData({ rangeStart: start, rangeEnd: end, bucketSeconds, points: [] })
    } finally {
      setIsLoading(false)
    }
  }, [normalizedOptions, range, bucket])

  useEffect(() => {
    void load()
  }, [load])

  // Auto-retry on transient failures (e.g., backend temporarily unavailable)
  useEffect(() => {
    if (!error) return
    const id = setTimeout(() => {
      void load()
    }, 2000)
    return () => clearTimeout(id)
  }, [error, load])

  useEffect(() => {
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type === 'records') {
        setData((current) => {
          const seeded =
            current ?? {
              rangeStart: formatEpochToIso(Math.floor((Date.now() - (parseRangeSpec(range) ?? 86_400) * 1000) / 1000)),
              rangeEnd: formatEpochToIso(Math.floor(Date.now() / 1000)),
              bucketSeconds: guessBucketSeconds(options?.bucket) ?? defaultBucketSecondsForRange(range),
              points: [],
            }
          return applyRecordsToTimeseries(seeded, payload.records, {
            range,
            bucketSeconds: seeded.bucketSeconds,
            settlementHour: normalizedOptions.settlementHour,
          })
        })
      }
    })
    return unsubscribe
  }, [normalizedOptions.settlementHour, options?.bucket, range])

  return {
    data,
    isLoading,
    error,
    refresh: load,
  }
}

interface UpdateContext {
  range: string
  bucketSeconds?: number
  settlementHour?: number
}

function applyRecordsToTimeseries(
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
