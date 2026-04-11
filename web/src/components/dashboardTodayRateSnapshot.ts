import type { TimeseriesResponse } from '../lib/api'
import {
  parseDateInput,
  resolveClosedNaturalDayEnd,
} from './dashboardNaturalDayWindow'

const MINUTE_MS = 60_000
const DEFAULT_WINDOW_MINUTES = 5

export interface DashboardTodayRateSnapshot {
  tokensPerMinute: number
  costPerMinute: number
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
  const anchor = floorToMinute(
    closedNaturalDayEnd ?? parseDateInput(response.rangeEnd) ?? fallbackNow,
  )
  const start = closedNaturalDayEnd
    ? floorToMinute(
        parseDateInput(response.rangeStart) ??
          new Date(closedNaturalDayEnd.getTime() - 24 * 60 * MINUTE_MS),
      )
    : startOfLocalDay(anchor)
  const startMs = start.getTime()
  const anchorMs = anchor.getTime()

  if (anchorMs <= startMs) {
    return {
      tokensPerMinute: 0,
      costPerMinute: 0,
      windowMinutes: 0,
      available: true,
    }
  }

  const completedMinuteCount = Math.max(0, Math.floor((anchorMs - startMs) / MINUTE_MS))
  const windowMinutes = Math.min(targetWindowMinutes, completedMinuteCount)

  if (windowMinutes <= 0) {
    return {
      tokensPerMinute: 0,
      costPerMinute: 0,
      windowMinutes: 0,
      available: true,
    }
  }

  const pointMap = new Map<number, { totalTokens: number; totalCost: number }>()

  for (const point of response.points ?? []) {
    const bucketStart = parseDateInput(point.bucketStart)
    const bucketEnd = parseDateInput(point.bucketEnd)
    if (!bucketStart || !bucketEnd) continue

    const bucketStartMs = floorToMinute(bucketStart).getTime()
    const bucketEndMs = floorToMinute(bucketEnd).getTime()
    if (bucketStartMs < startMs || bucketEndMs > anchorMs) continue

    const current = pointMap.get(bucketStartMs) ?? { totalTokens: 0, totalCost: 0 }
    current.totalTokens += point.totalTokens ?? 0
    current.totalCost += point.totalCost ?? 0
    pointMap.set(bucketStartMs, current)
  }

  let totalTokens = 0
  let totalCost = 0
  for (let offset = windowMinutes; offset >= 1; offset -= 1) {
    const bucketStartMs = anchorMs - offset * MINUTE_MS
    const bucket = pointMap.get(bucketStartMs)
    totalTokens += bucket?.totalTokens ?? 0
    totalCost += bucket?.totalCost ?? 0
  }

  return {
    tokensPerMinute: totalTokens / windowMinutes,
    costPerMinute: totalCost / windowMinutes,
    windowMinutes,
    available: true,
  }
}

function startOfLocalDay(date: Date) {
  const next = new Date(date)
  next.setHours(0, 0, 0, 0)
  return next
}

function floorToMinute(date: Date) {
  const next = new Date(date)
  next.setSeconds(0, 0)
  return next
}
