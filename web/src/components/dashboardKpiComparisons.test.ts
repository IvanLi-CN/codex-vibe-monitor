import { describe, expect, it } from 'vitest'
import {
  buildActiveMinuteAverages,
  buildParallelWorkKpiSnapshot,
  buildSameProgressUsageSnapshot,
  cacheHitRate,
  failureRate,
  percentDelta,
  sumCacheInputTokens,
} from './dashboardKpiComparisons'

describe('dashboard KPI comparison helpers', () => {
  it('calculates active-minute day averages only from minute buckets with calls', () => {
    const averages = buildActiveMinuteAverages(
      {
        totalCount: 3,
        successCount: 3,
        failureCount: 0,
        totalCost: 0.3,
        totalTokens: 3000,
      },
      {
        rangeStart: '2026-04-10T00:00:00.000Z',
        rangeEnd: '2026-04-10T00:04:00.000Z',
        bucketSeconds: 60,
        points: [
          {
            bucketStart: '2026-04-10T00:00:00.000Z',
            bucketEnd: '2026-04-10T00:01:00.000Z',
            totalCount: 0,
            successCount: 0,
            failureCount: 0,
            totalTokens: 0,
            cacheInputTokens: 0,
            totalCost: 0,
          },
          {
            bucketStart: '2026-04-10T00:01:00.000Z',
            bucketEnd: '2026-04-10T00:02:00.000Z',
            totalCount: 2,
            successCount: 2,
            failureCount: 0,
            totalTokens: 2000,
            cacheInputTokens: 800,
            totalCost: 0.2,
          },
          {
            bucketStart: '2026-04-10T00:02:00.000Z',
            bucketEnd: '2026-04-10T00:03:00.000Z',
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: 1000,
            cacheInputTokens: 200,
            totalCost: 0.1,
          },
        ],
      },
    )

    expect(averages.activeMinutes).toBe(2)
    expect(averages.tokensPerMinute).toBe(1500)
    expect(averages.spendRate).toBeCloseTo(0.15, 6)
  })

  it('calculates ratios and signed deltas defensively', () => {
    expect(failureRate(90, 10)).toBe(0.1)
    expect(cacheHitRate(250, 1000)).toBe(0.25)
    expect(percentDelta(150, 100)).toBe(0.5)
    expect(percentDelta(50, 100)).toBe(-0.5)
    expect(percentDelta(50, 0)).toBeNull()
  })

  it('builds yesterday usage through the same natural-day progress as today', () => {
    const snapshot = buildSameProgressUsageSnapshot(
      {
        rangeStart: '2026-04-10T00:00:00.000Z',
        rangeEnd: '2026-04-10T00:03:01.000Z',
        bucketSeconds: 60,
        points: [],
      },
      {
        rangeStart: '2026-04-09T00:00:00.000Z',
        rangeEnd: '2026-04-10T00:00:00.000Z',
        bucketSeconds: 60,
        points: [1, 2, 3, 99].map((value, index) => ({
          bucketStart: new Date(Date.parse('2026-04-09T00:00:00.000Z') + index * 60_000).toISOString(),
          bucketEnd: new Date(Date.parse('2026-04-09T00:01:00.000Z') + index * 60_000).toISOString(),
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: value * 100,
          cacheInputTokens: 0,
          totalCost: value,
        })),
      },
    )

    expect(snapshot.totalCost).toBe(6)
    expect(snapshot.totalTokens).toBe(600)
  })

  it('uses local clock progress instead of UTC elapsed time across DST changes', () => {
    const snapshot = buildSameProgressUsageSnapshot(
      {
        rangeStart: '2026-03-08T05:00:00.000Z',
        rangeEnd: '2026-03-08T07:30:00.000Z',
        bucketSeconds: 60,
        points: [],
      },
      {
        rangeStart: '2026-03-07T05:00:00.000Z',
        rangeEnd: '2026-03-08T05:00:00.000Z',
        bucketSeconds: 60,
        points: [
          ['2026-03-07T05:00:00.000Z', 1],
          ['2026-03-07T07:00:00.000Z', 2],
          ['2026-03-07T08:00:00.000Z', 3],
          ['2026-03-07T08:30:00.000Z', 99],
        ].map(([bucketStart, value]) => ({
          bucketStart: String(bucketStart),
          bucketEnd: new Date(Date.parse(String(bucketStart)) + 60_000).toISOString(),
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: Number(value) * 100,
          cacheInputTokens: 0,
          totalCost: Number(value),
        })),
      },
      { timeZone: 'America/New_York' },
    )

    expect(snapshot.totalCost).toBe(6)
    expect(snapshot.totalTokens).toBe(600)
  })

  it('stops at the first matching local progress on DST fall-back days', () => {
    const snapshot = buildSameProgressUsageSnapshot(
      {
        rangeStart: '2026-11-02T05:00:00.000Z',
        rangeEnd: '2026-11-02T06:30:00.000Z',
        bucketSeconds: 60,
        points: [],
      },
      {
        rangeStart: '2026-11-01T04:00:00.000Z',
        rangeEnd: '2026-11-02T05:00:00.000Z',
        bucketSeconds: 60,
        points: [
          ['2026-11-01T04:00:00.000Z', 1],
          ['2026-11-01T05:00:00.000Z', 2],
          ['2026-11-01T05:30:00.000Z', 99],
          ['2026-11-01T06:00:00.000Z', 99],
        ].map(([bucketStart, value]) => ({
          bucketStart: String(bucketStart),
          bucketEnd: new Date(Date.parse(String(bucketStart)) + 60_000).toISOString(),
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: Number(value) * 100,
          cacheInputTokens: 0,
          totalCost: Number(value),
        })),
      },
      { timeZone: 'America/New_York' },
    )

    expect(snapshot.totalCost).toBe(3)
    expect(snapshot.totalTokens).toBe(300)
  })

  it('sums cache tokens and resolves real-time parallel work against yesterday average', () => {
    expect(
      sumCacheInputTokens({
        rangeStart: '',
        rangeEnd: '',
        bucketSeconds: 60,
        points: [
          {
            bucketStart: 'a',
            bucketEnd: 'b',
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: 100,
            cacheInputTokens: 30,
            totalCost: 0.01,
          },
          {
            bucketStart: 'b',
            bucketEnd: 'c',
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: 100,
            cacheInputTokens: 20,
            totalCost: 0.01,
          },
        ],
      }),
    ).toBe(50)

    const snapshot = buildParallelWorkKpiSnapshot(
      {
        current: {
          rangeStart: 'today',
          rangeEnd: 'today',
          bucketSeconds: 60,
          completeBucketCount: 2,
          activeBucketCount: 2,
          minCount: 1,
          maxCount: 4,
          avgCount: 2.5,
          points: [
            { bucketStart: 'a', bucketEnd: 'b', parallelCount: 1 },
            { bucketStart: 'b', bucketEnd: 'c', parallelCount: 4 },
          ],
        },
        minute7d: {} as never,
        hour30d: {} as never,
        dayAll: {} as never,
      },
      {
        current: {
          rangeStart: 'yesterday',
          rangeEnd: 'yesterday',
          bucketSeconds: 60,
          completeBucketCount: 1,
          activeBucketCount: 1,
          minCount: 2,
          maxCount: 2,
          avgCount: 2,
          points: [{ bucketStart: 'a', bucketEnd: 'b', parallelCount: 2 }],
        },
        minute7d: {} as never,
        hour30d: {} as never,
        dayAll: {} as never,
      },
    )

    expect(snapshot.currentCount).toBe(4)
    expect(snapshot.dayAverage).toBe(2.5)
    expect(snapshot.yesterdayAverage).toBe(2)
  })
})
