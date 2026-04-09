import { afterEach, describe, expect, it } from 'vitest'
import type { TimeseriesResponse } from '../lib/api'
import {
  clearTimeseriesRemountCache,
  TIMESERIES_OPEN_RESYNC_COOLDOWN_MS,
  TIMESERIES_REMOUNT_CACHE_TTL_MS,
  TIMESERIES_RECORDS_RESYNC_THROTTLE_MS,
  applyRecordsToCurrentDayBucket,
  applyRecordsToTimeseries,
  getCurrentDayBucketEndEpoch,
  getLocalDayStartEpoch,
  getNextLocalDayStartEpoch,
  getTimeseriesRemountCacheKey,
  getTimeseriesRecordsResyncDelay,
  mergePendingTimeseriesSilentOption,
  readTimeseriesRemountCache,
  resolveTimeseriesSyncPolicy,
  shouldEnableTimeseriesRemountCache,
  shouldReuseTimeseriesRemountCache,
  shouldPatchCurrentDayBucketOnRecordsEvent,
  shouldResyncForCurrentDayBucket,
  shouldResyncOnRecordsEvent,
  shouldTriggerTimeseriesOpenResync,
  writeTimeseriesRemountCache,
} from './useTimeseries'

afterEach(() => {
  clearTimeseriesRemountCache()
})

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

  it('derives local-day boundaries for the today dashboard range', () => {
    const noonEpoch = Math.floor(new Date(2026, 3, 8, 12, 34, 56).getTime() / 1000)
    expect(getLocalDayStartEpoch(noonEpoch)).toBe(
      Math.floor(new Date(2026, 3, 8, 0, 0, 0).getTime() / 1000),
    )
    expect(getNextLocalDayStartEpoch(noonEpoch)).toBe(
      Math.floor(new Date(2026, 3, 9, 0, 0, 0).getTime() / 1000),
    )
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


describe('useTimeseries natural-day range patching', () => {
  it('keeps today-scoped local patches inside the current local day', () => {
    const now = new Date(2026, 3, 8, 12, 0, 0)
    const currentDayStart = new Date(2026, 3, 8, 0, 0, 0)
    const previousDayStart = new Date(2026, 3, 7, 0, 0, 0)
    const current: TimeseriesResponse = {
      rangeStart: currentDayStart.toISOString().replace(/\.\d{3}Z$/, 'Z'),
      rangeEnd: now.toISOString().replace(/\.\d{3}Z$/, 'Z'),
      bucketSeconds: 60,
      points: [
        {
          bucketStart: new Date(2026, 3, 7, 23, 59, 0).toISOString().replace(/\.\d{3}Z$/, 'Z'),
          bucketEnd: new Date(2026, 3, 8, 0, 0, 0).toISOString().replace(/\.\d{3}Z$/, 'Z'),
          totalCount: 1,
          successCount: 1,
          failureCount: 0,
          totalTokens: 10,
          totalCost: 0.1,
        },
      ],
    }

    const next = applyRecordsToTimeseries(
      current,
      [
        {
          id: 1,
          invokeId: 'yesterday',
          occurredAt: new Date(2026, 3, 7, 23, 59, 30).toISOString().replace(/\.\d{3}Z$/, 'Z'),
          status: 'success',
          totalTokens: 10,
          cost: 0.1,
          createdAt: new Date(2026, 3, 7, 23, 59, 30).toISOString().replace(/\.\d{3}Z$/, 'Z'),
        },
        {
          id: 2,
          invokeId: 'today',
          occurredAt: new Date(2026, 3, 8, 0, 1, 15).toISOString().replace(/\.\d{3}Z$/, 'Z'),
          status: 'failed',
          totalTokens: 25,
          cost: 0.2,
          createdAt: new Date(2026, 3, 8, 0, 1, 15).toISOString().replace(/\.\d{3}Z$/, 'Z'),
        },
      ],
      {
        range: 'today',
        bucketSeconds: 60,
      },
    )

    expect(next?.rangeStart).toBe(currentDayStart.toISOString().replace(/\.\d{3}Z$/, 'Z'))
    expect(next?.points).toHaveLength(1)
    expect(next?.points[0]).toMatchObject({
      bucketStart: new Date(2026, 3, 8, 0, 1, 0).toISOString().replace(/\.\d{3}Z$/, 'Z'),
      totalCount: 1,
      successCount: 0,
      failureCount: 1,
      totalTokens: 25,
      totalCost: 0.2,
    })
    expect(previousDayStart.getTime()).toBeLessThan(currentDayStart.getTime())
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

  it('stores remount cache entries by range and options', () => {
    const response: TimeseriesResponse = {
      rangeStart: '2026-04-08T00:00:00Z',
      rangeEnd: '2026-04-08T00:01:00Z',
      bucketSeconds: 60,
      points: [],
    }

    writeTimeseriesRemountCache('today', { bucket: '1m' }, response, 12_345)

    expect(getTimeseriesRemountCacheKey('today', { bucket: '1m' })).toBe(
      JSON.stringify(['today', '1m', null, false]),
    )
    expect(readTimeseriesRemountCache('today', { bucket: '1m' })).toEqual({
      data: response,
      cachedAt: 12_345,
    })
  })

  it('disables remount caching for current timeseries', () => {
    expect(shouldEnableTimeseriesRemountCache('current')).toBe(false)
    writeTimeseriesRemountCache(
      'current',
      undefined,
      {
        rangeStart: '2026-04-08T00:00:00Z',
        rangeEnd: '2026-04-08T00:01:00Z',
        bucketSeconds: 60,
        points: [],
      },
      1_000,
    )
    expect(readTimeseriesRemountCache('current')).toBeNull()
  })

  it('reuses remount cache only inside the ttl window', () => {
    expect(shouldReuseTimeseriesRemountCache(5_000, 5_000 + TIMESERIES_REMOUNT_CACHE_TTL_MS - 1)).toBe(true)
    expect(shouldReuseTimeseriesRemountCache(5_000, 5_000 + TIMESERIES_REMOUNT_CACHE_TTL_MS)).toBe(false)
  })
})
