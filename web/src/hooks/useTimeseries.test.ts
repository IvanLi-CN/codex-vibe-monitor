import { describe, expect, it } from 'vitest'
import type { TimeseriesResponse } from '../lib/api'
import {
  TIMESERIES_OPEN_RESYNC_COOLDOWN_MS,
  TIMESERIES_RECORDS_RESYNC_THROTTLE_MS,
  applyRecordsToCurrentDayBucket,
  getCurrentDayBucketEndEpoch,
  getTimeseriesRecordsResyncDelay,
  mergePendingTimeseriesSilentOption,
  resolveTimeseriesSyncPolicy,
  shouldPatchCurrentDayBucketOnRecordsEvent,
  shouldResyncForCurrentDayBucket,
  shouldResyncOnRecordsEvent,
  shouldTriggerTimeseriesOpenResync,
} from './useTimeseries'

describe('useTimeseries records sync strategy', () => {
  it('uses explicit local incremental policy for dashboard 24h heatmap', () => {
    expect(resolveTimeseriesSyncPolicy('1d', { bucket: '1m' }).mode).toBe('local')
    expect(shouldResyncOnRecordsEvent('1d', { bucket: '1m' })).toBe(false)
  })

  it('uses explicit local incremental policy for dashboard 7d heatmap', () => {
    expect(resolveTimeseriesSyncPolicy('7d', { bucket: '1h' }).mode).toBe('local')
    expect(shouldResyncOnRecordsEvent('7d', { bucket: '1h' })).toBe(false)
  })

  it('uses current-day patch policy for dashboard history calendar', () => {
    expect(resolveTimeseriesSyncPolicy('6mo', { bucket: '1d' }).mode).toBe('current-day-local')
    expect(shouldPatchCurrentDayBucketOnRecordsEvent('6mo', { bucket: '1d' })).toBe(true)
    expect(shouldResyncOnRecordsEvent('6mo', { bucket: '1d' })).toBe(false)
  })

  it('forces server resync when backend aggregation is preferred', () => {
    expect(
      resolveTimeseriesSyncPolicy('1d', {
        bucket: '15m',
        preferServerAggregation: true,
      }).mode,
    ).toBe('server')
    expect(
      shouldResyncOnRecordsEvent('1d', {
        bucket: '15m',
        preferServerAggregation: true,
      }),
    ).toBe(true)
  })

  it('falls back to server resync for other daily buckets', () => {
    expect(resolveTimeseriesSyncPolicy('30d', { bucket: '1d' }).mode).toBe('server')
    expect(shouldResyncOnRecordsEvent('30d', { bucket: '1d' })).toBe(true)
  })
})

describe('useTimeseries current-day bucket patching', () => {
  const base: TimeseriesResponse = {
    rangeStart: '2026-03-05T00:00:00Z',
    rangeEnd: '2026-03-07T00:00:00Z',
    bucketSeconds: 86_400,
    points: [
      {
        bucketStart: '2026-03-05T00:00:00Z',
        bucketEnd: '2026-03-06T00:00:00Z',
        totalCount: 4,
        successCount: 3,
        failureCount: 1,
        totalTokens: 400,
        totalCost: 2,
      },
      {
        bucketStart: '2026-03-06T00:00:00Z',
        bucketEnd: '2026-03-07T00:00:00Z',
        totalCount: 1,
        successCount: 1,
        failureCount: 0,
        totalTokens: 100,
        totalCost: 0.5,
      },
    ],
  }

  it('patches only the active local-day bucket for repeated records events', () => {
    const next = applyRecordsToCurrentDayBucket(
      base,
      [
        {
          id: 1,
          invokeId: 'older',
          occurredAt: '2026-03-05T10:00:00Z',
          status: 'success',
          totalTokens: 10,
          cost: 0.1,
          createdAt: '2026-03-05T10:00:00Z',
        },
        {
          id: 2,
          invokeId: 'today',
          occurredAt: '2026-03-06T08:30:00Z',
          status: 'failed',
          totalTokens: 25,
          cost: 0.2,
          createdAt: '2026-03-06T08:30:00Z',
        },
      ],
      Math.floor(Date.parse('2026-03-06T12:00:00Z') / 1000),
    )

    expect(next?.points[0]).toEqual(base.points[0])
    expect(next?.points[1]).toMatchObject({
      totalCount: 2,
      successCount: 1,
      failureCount: 1,
      totalTokens: 125,
      totalCost: 0.7,
    })
  })

  it('requests a full resync when the current day is no longer covered', () => {
    expect(
      shouldResyncForCurrentDayBucket(base, Math.floor(Date.parse('2026-03-07T01:00:00Z') / 1000)),
    ).toBe(true)
    expect(getCurrentDayBucketEndEpoch(base, Math.floor(Date.parse('2026-03-06T12:00:00Z') / 1000))).toBe(
      Math.floor(Date.parse('2026-03-07T00:00:00Z') / 1000),
    )
  })

  it('requests a full resync when there is no current-day bucket to patch yet', () => {
    expect(shouldResyncForCurrentDayBucket(null, Math.floor(Date.parse('2026-03-06T12:00:00Z') / 1000))).toBe(true)
    expect(
      shouldResyncForCurrentDayBucket(
        {
          ...base,
          points: [],
        },
        Math.floor(Date.parse('2026-03-06T12:00:00Z') / 1000),
      ),
    ).toBe(true)
  })
})

describe('useTimeseries refresh coordination helpers', () => {
  it('reports the remaining resync delay inside the 3s throttle window', () => {
    expect(getTimeseriesRecordsResyncDelay(10_000, 10_250)).toBe(
      TIMESERIES_RECORDS_RESYNC_THROTTLE_MS - 250,
    )
    expect(
      getTimeseriesRecordsResyncDelay(
        20_000,
        20_000 + TIMESERIES_RECORDS_RESYNC_THROTTLE_MS,
      ),
    ).toBe(0)
  })

  it('merges pending silent loads without losing a non-silent refresh', () => {
    expect(mergePendingTimeseriesSilentOption(null, true)).toBe(true)
    expect(mergePendingTimeseriesSilentOption(true, false)).toBe(false)
    expect(mergePendingTimeseriesSilentOption(false, true)).toBe(false)
  })

  it('throttles reconnect resync unless the caller forces it', () => {
    expect(
      shouldTriggerTimeseriesOpenResync(
        30_000,
        30_000 + TIMESERIES_OPEN_RESYNC_COOLDOWN_MS - 1,
      ),
    ).toBe(false)
    expect(shouldTriggerTimeseriesOpenResync(30_000, 30_250, true)).toBe(true)
  })
})
