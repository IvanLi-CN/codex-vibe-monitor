import { describe, expect, it } from 'vitest'
import { formatProxyWeightDelta, formatServiceTier, isPriorityServiceTier } from '../lib/invocation'

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

describe('service tier helpers', () => {
  it('normalizes and formats service tiers', () => {
    expect(formatServiceTier(' Priority ')).toBe('priority')
    expect(formatServiceTier('FLEX')).toBe('flex')
  })

  it('falls back to em dash for empty or missing service tiers', () => {
    expect(formatServiceTier(undefined)).toBe('—')
    expect(formatServiceTier('   ')).toBe('—')
  })

  it('treats only priority as fast mode', () => {
    expect(isPriorityServiceTier('priority')).toBe(true)
    expect(isPriorityServiceTier(' Priority ')).toBe(true)
    expect(isPriorityServiceTier('flex')).toBe(false)
    expect(isPriorityServiceTier(undefined)).toBe(false)
  })
})
