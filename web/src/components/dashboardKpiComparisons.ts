import type {
  ParallelWorkStatsResponse,
  ParallelWorkWindowResponse,
  StatsResponse,
  TimeseriesResponse,
} from '../lib/api'

export interface ActiveMinuteAverages {
  activeMinutes: number
  tokensPerMinute: number | null
  spendRate: number | null
}

export interface ParallelWorkKpiSnapshot {
  currentCount: number | null
  dayAverage: number | null
  yesterdayAverage: number | null
}

export function percentDelta(current: number | null | undefined, baseline: number | null | undefined) {
  if (current == null || baseline == null || baseline === 0) return null
  return (current - baseline) / baseline
}

export function failureRate(successCount: number, failureCount: number) {
  const terminalCount = successCount + failureCount
  return terminalCount > 0 ? failureCount / terminalCount : 0
}

export function cacheHitRate(cacheInputTokens: number, totalTokens: number) {
  return totalTokens > 0 ? cacheInputTokens / totalTokens : 0
}

export function sumCacheInputTokens(response: TimeseriesResponse | null | undefined) {
  return (response?.points ?? []).reduce(
    (sum, point) => sum + (point.cacheInputTokens ?? 0),
    0,
  )
}

export function buildActiveMinuteAverages(
  stats: StatsResponse | null | undefined,
  response: TimeseriesResponse | null | undefined,
): ActiveMinuteAverages {
  const activeMinutes = (response?.points ?? []).filter(
    (point) => (point.totalCount ?? 0) > 0,
  ).length
  if (activeMinutes <= 0) {
    return {
      activeMinutes: 0,
      tokensPerMinute: null,
      spendRate: null,
    }
  }
  return {
    activeMinutes,
    tokensPerMinute: (stats?.totalTokens ?? 0) / activeMinutes,
    spendRate: (stats?.totalCost ?? 0) / activeMinutes,
  }
}

function latestParallelCount(window: ParallelWorkWindowResponse | null | undefined) {
  const points = window?.points ?? []
  if (points.length === 0) return null
  return points[points.length - 1]?.parallelCount ?? null
}

export function buildParallelWorkKpiSnapshot(
  current: ParallelWorkStatsResponse | null | undefined,
  yesterday: ParallelWorkStatsResponse | null | undefined,
): ParallelWorkKpiSnapshot {
  return {
    currentCount: latestParallelCount(current?.current),
    dayAverage: current?.current.avgCount ?? null,
    yesterdayAverage: yesterday?.current.avgCount ?? null,
  }
}
