import type { TimeseriesResponse } from '../lib/api'
import {
  parseDateInput,
  resolveClosedNaturalDayEnd,
} from './dashboardNaturalDayWindow'

const MINUTE_MS = 60_000
const DEFAULT_WINDOW_MINUTES = 5

export interface DashboardTodayRateSnapshot {
  tokensPerMinute: number
  spendRate: number
  windowMinutes: number
  available: boolean
}

export function buildDashboardTodayRateSnapshot(
  response: TimeseriesResponse | null,
  options?: { now?: Date; targetWindowMinutes?: number; closedNaturalDay?: boolean },
): DashboardTodayRateSnapshot | null {
  if (!response) {
    return null
  }

  const targetWindowMinutes = Math.max(1, options?.targetWindowMinutes ?? DEFAULT_WINDOW_MINUTES)
  const fallbackNow = options?.now ?? new Date()
  const closedNaturalDayEnd = resolveClosedNaturalDayEnd(
    response,
    options?.closedNaturalDay ?? false,
  )
  const responseEnd = parseDateInput(response.rangeEnd)
  const anchor = closedNaturalDayEnd ?? resolveLiveNaturalDayAnchor(responseEnd, fallbackNow)
  const start = closedNaturalDayEnd
    ? floorToMinute(
        parseDateInput(response.rangeStart) ??
          new Date(closedNaturalDayEnd.getTime() - 24 * 60 * MINUTE_MS),
      )
    : startOfLocalDay(anchor)
  const startMs = start.getTime()
  const anchorMs = anchor.getTime()
  const windowStartMs = Math.max(startMs, anchorMs - targetWindowMinutes * MINUTE_MS)

  if (anchorMs <= startMs) {
    return {
      tokensPerMinute: 0,
      spendRate: 0,
      windowMinutes: 0,
      available: true,
    }
  }

  const pointMap = new Map<number, {
    bucketEndMs: number
    totalTokens: number
    totalCost: number
  }>()

  for (const point of response.points ?? []) {
    const bucketStart = parseDateInput(point.bucketStart)
    const bucketEnd = parseDateInput(point.bucketEnd)
    if (!bucketStart || !bucketEnd) continue

    const bucketStartMs = floorToMinute(bucketStart).getTime()
    const bucketEndMs = bucketEnd.getTime()
    if (bucketStartMs >= anchorMs || bucketEndMs <= windowStartMs) continue

    const current = pointMap.get(bucketStartMs) ?? {
      bucketEndMs,
      totalTokens: 0,
      totalCost: 0,
    }
    current.bucketEndMs = Math.max(current.bucketEndMs, bucketEndMs)
    current.totalTokens += point.totalTokens ?? 0
    current.totalCost += point.totalCost ?? 0
    pointMap.set(bucketStartMs, current)
  }

  const buckets = [...pointMap.entries()]
    .map(([bucketStartMs, bucket]) => ({ bucketStartMs, ...bucket }))
    .sort((a, b) => a.bucketStartMs - b.bucketStartMs)
  const firstActiveBucket = buckets.find((bucket) => (
    bucket.totalTokens > 0 ||
    bucket.totalCost > 0
  ))

  if (!firstActiveBucket) {
    return {
      tokensPerMinute: 0,
      spendRate: 0,
      windowMinutes: Math.max(0, (anchorMs - windowStartMs) / MINUTE_MS),
      available: true,
    }
  }

  const activeStartMs = Math.max(windowStartMs, firstActiveBucket.bucketStartMs)
  const windowMinutes = Math.max((anchorMs - activeStartMs) / MINUTE_MS, 0)
  let totalTokens = 0
  let totalCost = 0
  for (const bucket of buckets) {
    if (bucket.bucketEndMs <= activeStartMs || bucket.bucketStartMs >= anchorMs) continue
    totalTokens += bucket.totalTokens
    totalCost += bucket.totalCost
  }

  return {
    tokensPerMinute: windowMinutes > 0 ? totalTokens / windowMinutes : 0,
    spendRate: windowMinutes > 0 ? totalCost / windowMinutes : 0,
    windowMinutes,
    available: true,
  }
}

function startOfLocalDay(date: Date) {
  const next = new Date(date)
  next.setHours(0, 0, 0, 0)
  return next
}

function resolveLiveNaturalDayAnchor(responseEnd: Date | null, now: Date) {
  if (!responseEnd) return now
  if (isSameLocalDay(responseEnd, now) && now.getTime() > responseEnd.getTime()) {
    return now
  }
  return responseEnd
}

function isSameLocalDay(left: Date, right: Date) {
  return (
    left.getFullYear() === right.getFullYear() &&
    left.getMonth() === right.getMonth() &&
    left.getDate() === right.getDate()
  )
}

function floorToMinute(date: Date) {
  const next = new Date(date)
  next.setSeconds(0, 0)
  return next
}
