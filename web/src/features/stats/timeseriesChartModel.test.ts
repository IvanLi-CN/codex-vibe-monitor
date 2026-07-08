import { describe, expect, it } from 'vitest'
import {
  buildTimeseriesChartData,
  resolveTimeseriesChartMode,
  TIMESERIES_BAR_POINT_LIMIT,
} from './timeseriesChartModel'

function createPoint(index: number, totalTokens: number, totalCount: number, totalCost: number) {
  const hour = String(index).padStart(2, '0')
  return {
    bucketStart: `2026-03-20T${hour}:00:00Z`,
    bucketEnd: `2026-03-20T${hour}:59:59Z`,
    totalTokens,
    totalCount,
    totalCost,
    successCount: totalCount,
    failureCount: 0,
  }
}

describe('timeseriesChartModel', () => {
  it('keeps bucket bars when point count is at the 7-point threshold', () => {
    const points = Array.from({ length: TIMESERIES_BAR_POINT_LIMIT }, (_, index) =>
      createPoint(index, index + 10, index + 1, index + 0.25),
    )

    expect(resolveTimeseriesChartMode(points.length)).toBe('bucket-bar')
    expect(buildTimeseriesChartData(points, 3600, true)).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ totalTokens: 10, totalCount: 1, totalCost: 0.25 }),
        expect.objectContaining({ totalTokens: 16, totalCount: 7, totalCost: 6.25 }),
      ]),
    )
  })

  it('switches to cumulative values once point count exceeds the threshold', () => {
    const points = [
      createPoint(0, 10, 1, 0.25),
      createPoint(1, 12, 2, 0.5),
      createPoint(2, 14, 3, 0.75),
      createPoint(3, 16, 4, 1),
      createPoint(4, 18, 5, 1.25),
      createPoint(5, 20, 6, 1.5),
      createPoint(6, 22, 7, 1.75),
      createPoint(7, 24, 8, 2),
    ]

    expect(resolveTimeseriesChartMode(points.length)).toBe('cumulative-area')
    expect(buildTimeseriesChartData(points, 3600, true)).toEqual([
      expect.objectContaining({ totalTokens: 10, totalCount: 1, totalCost: 0.25 }),
      expect.objectContaining({ totalTokens: 22, totalCount: 3, totalCost: 0.75 }),
      expect.objectContaining({ totalTokens: 36, totalCount: 6, totalCost: 1.5 }),
      expect.objectContaining({ totalTokens: 52, totalCount: 10, totalCost: 2.5 }),
      expect.objectContaining({ totalTokens: 70, totalCount: 15, totalCost: 3.75 }),
      expect.objectContaining({ totalTokens: 90, totalCount: 21, totalCost: 5.25 }),
      expect.objectContaining({ totalTokens: 112, totalCount: 28, totalCost: 7 }),
      expect.objectContaining({ totalTokens: 136, totalCount: 36, totalCost: 9 }),
    ])
  })
})
