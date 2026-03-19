import { describe, expect, it } from 'vitest'
import {
  BUCKET_OPTION_KEYS,
  resolveStatsBucketOptions,
  resolveStatsBucketValue,
} from './stats-options'

describe('resolveStatsBucketOptions', () => {
  it('returns the default range buckets when the backend has no extra limits', () => {
    expect(resolveStatsBucketOptions('1mo').map((option) => option.value)).toEqual([
      '6h',
      '12h',
      '1d',
    ])
  })

  it('keeps only backend-supported buckets for archived ranges', () => {
    expect(
      resolveStatsBucketOptions('1mo', ['1d']).map((option) => option.value),
    ).toEqual(['1d'])
  })

  it('falls back to the daily option when backend support no longer matches the stale UI bucket list', () => {
    expect(
      resolveStatsBucketOptions('thisWeek', ['1d']).map((option) => option.value),
    ).toEqual(['1d'])
  })
})

describe('resolveStatsBucketValue', () => {
  it('keeps the current bucket when it is still supported', () => {
    expect(
      resolveStatsBucketValue('12h', [
        { value: '6h' },
        { value: '12h' },
        { value: '1d' },
      ]),
    ).toBe('12h')
  })

  it('falls back to the first supported bucket when the previous bucket became invalid', () => {
    expect(resolveStatsBucketValue('12h', [{ value: '1d' }])).toBe('1d')
  })
})

describe('BUCKET_OPTION_KEYS', () => {
  it('keeps the 24-hour bucket option for the 7d range', () => {
    expect(BUCKET_OPTION_KEYS['7d']).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          value: '1d',
          labelKey: 'stats.bucket.each24Hours',
        }),
      ]),
    )
  })
})
