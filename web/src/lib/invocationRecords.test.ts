import { describe, expect, it } from 'vitest'
import { buildAppliedInvocationFilters, createDefaultInvocationRecordsDraft } from './invocationRecords'

describe('buildAppliedInvocationFilters', () => {
  it('rejects fractional token filters before sending the request', () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      minTotalTokens: '1.5',
    }

    expect(() => buildAppliedInvocationFilters(draft)).toThrow('minTotalTokens must be a whole number')
  })

  it('treats minute-precision customTo as inclusive-of-minute for exclusive upper bounds', () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      rangePreset: 'custom' as const,
      customFrom: '2026-03-10T10:00',
      customTo: '2026-03-10T10:32',
    }

    const filters = buildAppliedInvocationFilters(draft)
    expect(filters.from).toBeDefined()
    expect(filters.to).toBeDefined()

    const expected = new Date('2026-03-10T10:32').getTime() + 60_000
    expect(new Date(filters.to as string).getTime()).toBe(expected)
  })

  it('keeps second-precision customTo untouched', () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      rangePreset: 'custom' as const,
      customFrom: '2026-03-10T10:00:00',
      customTo: '2026-03-10T10:32:45',
    }

    const filters = buildAppliedInvocationFilters(draft)
    const expected = new Date('2026-03-10T10:32:45').getTime()
    expect(new Date(filters.to as string).getTime()).toBe(expected)
  })
})
