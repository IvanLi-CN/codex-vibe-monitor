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
})
