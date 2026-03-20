import type { TimeseriesPoint } from '../lib/api'

export const TIMESERIES_BAR_POINT_LIMIT = 7

export type TimeseriesChartMode = 'bucket-bar' | 'cumulative-area'

export interface TimeseriesChartDatum {
  label: string
  totalTokens: number
  totalCost: number
  totalCount: number
}

export function resolveTimeseriesChartMode(pointCount: number): TimeseriesChartMode {
  return pointCount <= TIMESERIES_BAR_POINT_LIMIT ? 'bucket-bar' : 'cumulative-area'
}

export function buildTimeseriesChartData(
  points: TimeseriesPoint[],
  bucketSeconds: number | undefined,
  showDate: boolean,
): TimeseriesChartDatum[] {
  const mode = resolveTimeseriesChartMode(points.length)
  let cumulativeTokens = 0
  let cumulativeCost = 0
  let cumulativeCount = 0

  return points.map((point) => {
    const start = new Date(point.bucketStart)
    const label = formatLocalLabel(start, bucketSeconds, showDate)

    if (mode === 'cumulative-area') {
      cumulativeTokens += point.totalTokens
      cumulativeCost += point.totalCost
      cumulativeCount += point.totalCount
      return {
        label,
        totalTokens: cumulativeTokens,
        totalCost: cumulativeCost,
        totalCount: cumulativeCount,
      }
    }

    return {
      label,
      totalTokens: point.totalTokens,
      totalCost: point.totalCost,
      totalCount: point.totalCount,
    }
  })
}

function pad2(n: number) {
  return n.toString().padStart(2, '0')
}

function formatLocalLabel(date: Date, bucketSeconds: number | undefined, showDate: boolean) {
  const y = date.getFullYear()
  const m = pad2(date.getMonth() + 1)
  const d = pad2(date.getDate())
  const hh = pad2(date.getHours())
  const mm = pad2(date.getMinutes())
  if (!bucketSeconds || bucketSeconds >= 3600) {
    return showDate ? `${y}-${m}-${d} ${hh}:00` : `${hh}:00`
  }
  return showDate ? `${y}-${m}-${d} ${hh}:${mm}` : `${hh}:${mm}`
}
