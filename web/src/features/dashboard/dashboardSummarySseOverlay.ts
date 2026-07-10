import type { StatsResponse } from '../../lib/api'

export function mergeSummarySseOverlay(
  current: StatsResponse,
  incoming: StatsResponse,
): StatsResponse {
  if (incoming.usageBreakdown != null) return incoming
  return {
    ...incoming,
    totalCost: current.totalCost,
    totalTokens: current.totalTokens,
    usageBreakdown: current.usageBreakdown,
  }
}
