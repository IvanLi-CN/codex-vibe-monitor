import { describe, expect, it } from 'vitest'
import { buildDashboardTodayRateSnapshot } from './dashboardTodayRateSnapshot'

function minutePoint(offsetMinutes: number, totalTokens: number, totalCost: number) {
  const bucketStart = new Date(2026, 3, 10, 0, offsetMinutes, 0, 0)
  const bucketEnd = new Date(bucketStart.getTime() + 60_000)
  return {
    bucketStart: formatLocal(bucketStart),
    bucketEnd: formatLocal(bucketEnd),
    totalCount: 1,
    successCount: 1,
    failureCount: 0,
    totalTokens,
    totalCost,
  }
}

describe('buildDashboardTodayRateSnapshot', () => {
  it('calculates 5-minute averages from the latest completed minute buckets', () => {
    const snapshot = buildDashboardTodayRateSnapshot(
      {
        rangeStart: '2026-04-10 00:00:00',
        rangeEnd: '2026-04-10 00:06:30',
        bucketSeconds: 60,
        points: [
          minutePoint(1, 600, 0.06),
          minutePoint(2, 800, 0.08),
          minutePoint(3, 1000, 0.1),
          minutePoint(4, 1200, 0.12),
          minutePoint(5, 1400, 0.14),
          minutePoint(6, 5000, 0.5),
        ],
      },
      { now: new Date(2026, 3, 10, 0, 6, 30, 0) },
    )

    expect(snapshot?.tokensPerMinute).toBe(1000)
    expect(snapshot?.costPerMinute).toBeCloseTo(0.1, 6)
    expect(snapshot?.windowMinutes).toBe(5)
    expect(snapshot?.available).toBe(true)
  })

  it('fills missing minutes with zero instead of shrinking the denominator', () => {
    const snapshot = buildDashboardTodayRateSnapshot(
      {
        rangeStart: '2026-04-10 00:00:00',
        rangeEnd: '2026-04-10 00:05:20',
        bucketSeconds: 60,
        points: [minutePoint(1, 1000, 0.1), minutePoint(3, 2000, 0.2)],
      },
      { now: new Date(2026, 3, 10, 0, 5, 20, 0) },
    )

    expect(snapshot?.tokensPerMinute).toBe(600)
    expect(snapshot?.costPerMinute).toBeCloseTo(0.06, 6)
    expect(snapshot?.windowMinutes).toBe(5)
  })

  it('uses the available completed minutes when today has fewer than five full minutes', () => {
    const snapshot = buildDashboardTodayRateSnapshot(
      {
        rangeStart: '2026-04-10 00:00:00',
        rangeEnd: '2026-04-10 00:03:10',
        bucketSeconds: 60,
        points: [minutePoint(0, 600, 0.06), minutePoint(1, 900, 0.09), minutePoint(2, 1500, 0.15)],
      },
      { now: new Date(2026, 3, 10, 0, 3, 10, 0) },
    )

    expect(snapshot?.tokensPerMinute).toBe(1000)
    expect(snapshot?.costPerMinute).toBeCloseTo(0.1, 6)
    expect(snapshot?.windowMinutes).toBe(3)
  })

  it('ignores the current in-progress minute even when a partial bucket exists', () => {
    const snapshot = buildDashboardTodayRateSnapshot(
      {
        rangeStart: '2026-04-10 00:00:00',
        rangeEnd: '2026-04-10 00:05:40',
        bucketSeconds: 60,
        points: [
          minutePoint(0, 500, 0.05),
          minutePoint(1, 500, 0.05),
          minutePoint(2, 500, 0.05),
          minutePoint(3, 500, 0.05),
          minutePoint(4, 500, 0.05),
          minutePoint(5, 9000, 0.9),
        ],
      },
      { now: new Date(2026, 3, 10, 0, 5, 40, 0) },
    )

    expect(snapshot?.tokensPerMinute).toBe(500)
    expect(snapshot?.costPerMinute).toBeCloseTo(0.05, 6)
    expect(snapshot?.windowMinutes).toBe(5)
  })

  it('returns zero values when there are no completed minutes yet', () => {
    const snapshot = buildDashboardTodayRateSnapshot(
      {
        rangeStart: '2026-04-10 00:00:00',
        rangeEnd: '2026-04-10 00:00:20',
        bucketSeconds: 60,
        points: [],
      },
      { now: new Date(2026, 3, 10, 0, 0, 20, 0) },
    )

    expect(snapshot?.tokensPerMinute).toBe(0)
    expect(snapshot?.costPerMinute).toBe(0)
    expect(snapshot?.windowMinutes).toBe(0)
    expect(snapshot?.available).toBe(true)
  })

  it('keeps yesterday closed-day rates on the previous natural day even when rangeEnd is rounded into the next minute', () => {
    const snapshot = buildDashboardTodayRateSnapshot(
      {
        rangeStart: '2026-04-09 00:00:00',
        rangeEnd: '2026-04-10 00:01:00',
        bucketSeconds: 60,
        points: [
          {
            bucketStart: '2026-04-09 23:55:00',
            bucketEnd: '2026-04-09 23:56:00',
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: 500,
            totalCost: 0.05,
          },
          {
            bucketStart: '2026-04-09 23:56:00',
            bucketEnd: '2026-04-09 23:57:00',
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: 600,
            totalCost: 0.06,
          },
          {
            bucketStart: '2026-04-09 23:57:00',
            bucketEnd: '2026-04-09 23:58:00',
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: 700,
            totalCost: 0.07,
          },
          {
            bucketStart: '2026-04-09 23:58:00',
            bucketEnd: '2026-04-09 23:59:00',
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: 800,
            totalCost: 0.08,
          },
          {
            bucketStart: '2026-04-09 23:59:00',
            bucketEnd: '2026-04-10 00:00:00',
            totalCount: 1,
            successCount: 1,
            failureCount: 0,
            totalTokens: 900,
            totalCost: 0.09,
          },
        ],
      },
      {
        now: new Date(2026, 3, 10, 12, 0, 0, 0),
        closedNaturalDay: true,
      },
    )

    expect(snapshot?.tokensPerMinute).toBe(700)
    expect(snapshot?.costPerMinute).toBeCloseTo(0.07, 6)
    expect(snapshot?.windowMinutes).toBe(5)
    expect(snapshot?.available).toBe(true)
  })
})

function formatLocal(date: Date) {
  const year = date.getFullYear()
  const month = String(date.getMonth() + 1).padStart(2, '0')
  const day = String(date.getDate()).padStart(2, '0')
  const hours = String(date.getHours()).padStart(2, '0')
  const minutes = String(date.getMinutes()).padStart(2, '0')
  const seconds = String(date.getSeconds()).padStart(2, '0')
  return `${year}-${month}-${day} ${hours}:${minutes}:${seconds}`
}
