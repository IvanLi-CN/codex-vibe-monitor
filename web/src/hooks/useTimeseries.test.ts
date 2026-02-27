import { describe, expect, it } from 'vitest'
import { shouldResyncOnRecordsEvent } from './useTimeseries'

describe('useTimeseries records sync strategy', () => {
  it('forces server resync when backend aggregation is preferred', () => {
    expect(
      shouldResyncOnRecordsEvent('1d', {
        bucket: '15m',
        preferServerAggregation: true,
      }),
    ).toBe(true)
  })

  it('forces server resync for daily buckets to preserve timezone alignment', () => {
    expect(
      shouldResyncOnRecordsEvent('7d', {
        bucket: '1d',
      }),
    ).toBe(true)
  })

  it('keeps local incremental updates for sub-daily buckets by default', () => {
    expect(
      shouldResyncOnRecordsEvent('1d', {
        bucket: '15m',
      }),
    ).toBe(false)
  })
})
