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
    } finally {
      setIsLoading(false)
    }
  }, [normalizedOptions, range])

  useEffect(() => {
    void load()
  }, [load])

  useEffect(() => {
    const unsubscribe = subscribeToSse((payload) => {
      if (payload.type === 'records') {
        setData((current) =>
          applyRecordsToTimeseries(current, payload.records, {
            range,
            bucketSeconds: current?.bucketSeconds,
            settlementHour: normalizedOptions.settlementHour,
          }),
        )
      }
    })
    return unsubscribe
  }, [normalizedOptions.settlementHour, range])

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
  let latestRangeEndEpoch = parseNaiveDateTime(current.rangeEnd) ?? null

  for (const record of records) {
    const occurredEpoch = parseNaiveDateTime(record.occurredAt)
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
    const bucketStart = formatNaiveDateTime(bucketStartEpoch)
    const bucketEnd = formatNaiveDateTime(bucketEndEpoch)

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
    const aEpoch = parseNaiveDateTime(a.bucketStart) ?? 0
    const bEpoch = parseNaiveDateTime(b.bucketStart) ?? 0
    return aEpoch - bEpoch
  })

  if (rangeSeconds != null && latestRangeEndEpoch != null) {
    const earliestAllowed = latestRangeEndEpoch - rangeSeconds
    while (sortedPoints.length > 0) {
      const first = sortedPoints[0]
      const firstEndEpoch = parseNaiveDateTime(first.bucketEnd)
      if (firstEndEpoch != null && firstEndEpoch <= earliestAllowed) {
        sortedPoints.shift()
        continue
      }
      break
    }
  }

  const nextRangeEndEpoch = latestRangeEndEpoch ?? parseNaiveDateTime(current.rangeEnd)
  const nextRangeEnd = nextRangeEndEpoch != null ? formatNaiveDateTime(nextRangeEndEpoch) : current.rangeEnd
  const nextRangeStart =
    rangeSeconds != null && nextRangeEndEpoch != null
      ? formatNaiveDateTime(nextRangeEndEpoch - rangeSeconds)
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

function parseNaiveDateTime(value: string) {
  const [datePart, timePart] = value?.split(' ') ?? []
  if (!datePart || !timePart) {
    return null
  }
  const [year, month, day] = datePart.split('-').map(Number)
  const [hour, minute, second] = timePart.split(':').map(Number)
  if ([year, month, day, hour, minute, second].some((part) => !Number.isFinite(part))) {
    return null
  }
  const date = Date.UTC(year, (month ?? 1) - 1, day ?? 1, hour ?? 0, minute ?? 0, second ?? 0)
  return Math.floor(date / 1000)
}

function formatNaiveDateTime(epochSeconds: number) {
  const date = new Date(epochSeconds * 1000)
  const pad = (value: number) => value.toString().padStart(2, '0')
  return `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())} ${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())}:${pad(date.getUTCSeconds())}`
}
