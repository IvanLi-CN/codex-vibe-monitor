import { describe, expect, it } from 'vitest'
import { formatProxyWeightDelta } from '../lib/invocation'

describe('formatProxyWeightDelta', () => {
  it('formats positive deltas with plus sign and two decimals', () => {
    expect(formatProxyWeightDelta(0.55)).toBe('↑ +0.55')
  })

  it('formats negative deltas with minus sign and rounds to two decimals', () => {
    expect(formatProxyWeightDelta(-0.678)).toBe('↓ -0.68')
  })

  it('formats zero with explicit sign', () => {
    expect(formatProxyWeightDelta(0)).toBe('↑ +0.00')
    expect(formatProxyWeightDelta(-0)).toBe('↑ +0.00')
    expect(formatProxyWeightDelta(-0.004)).toBe('↑ +0.00')
  })

  it('falls back to em dash for missing or invalid values', () => {
    expect(formatProxyWeightDelta(undefined)).toBe('—')
    expect(formatProxyWeightDelta(null)).toBe('—')
    expect(formatProxyWeightDelta(Number.NaN)).toBe('—')
  })
})
