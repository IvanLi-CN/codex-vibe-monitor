import { describe, expect, it } from 'vitest'
import { formatProxyWeightDelta } from '../lib/invocation'

describe('formatProxyWeightDelta', () => {
  it('formats positive deltas as up direction with absolute value', () => {
    expect(formatProxyWeightDelta(0.55)).toEqual({ direction: 'up', value: '0.55' })
  })

  it('formats negative deltas as down direction and rounds to two decimals', () => {
    expect(formatProxyWeightDelta(-0.678)).toEqual({ direction: 'down', value: '0.68' })
  })

  it('formats zero as flat direction', () => {
    expect(formatProxyWeightDelta(0)).toEqual({ direction: 'flat', value: '0.00' })
    expect(formatProxyWeightDelta(-0)).toEqual({ direction: 'flat', value: '0.00' })
    expect(formatProxyWeightDelta(-0.004)).toEqual({ direction: 'flat', value: '0.00' })
  })

  it('falls back to em dash for missing or invalid values', () => {
    expect(formatProxyWeightDelta(undefined)).toEqual({ direction: 'missing', value: '—' })
    expect(formatProxyWeightDelta(null)).toEqual({ direction: 'missing', value: '—' })
    expect(formatProxyWeightDelta(Number.NaN)).toEqual({ direction: 'missing', value: '—' })
  })
})
