import { describe, expect, it } from 'vitest'
import { buildAppliedInvocationFilters, buildInvocationSuggestionsQuery, createDefaultInvocationRecordsDraft } from './invocationRecords'

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


  it('builds suggestion queries from the full draft filters and current snapshot', () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      rangePreset: 'custom' as const,
      customFrom: '2026-03-10T10:00:00',
      customTo: '2026-03-10T10:32:45',
      status: ' failed ',
      model: ' gpt-5.4 ',
      proxy: ' proxy-a ',
      endpoint: ' /v1/responses ',
      failureClass: ' service_failure ',
      failureKind: ' http_502 ',
      promptCacheKey: ' cache-key ',
      requesterIp: ' 127.0.0.1 ',
      keyword: ' retry ',
      minTotalTokens: '10',
      maxTotalTokens: '20',
      minTotalMs: '1.5',
      maxTotalMs: '2.5',
    }

    const query = buildInvocationSuggestionsQuery(draft, 99)

    expect(query.snapshotId).toBe(99)
    expect(query.status).toBe('failed')
    expect(query.model).toBe('gpt-5.4')
    expect(query.proxy).toBe('proxy-a')
    expect(query.endpoint).toBe('/v1/responses')
    expect(query.failureClass).toBe('service_failure')
    expect(query.failureKind).toBe('http_502')
    expect(query.promptCacheKey).toBe('cache-key')
    expect(query.requesterIp).toBe('127.0.0.1')
    expect(query.keyword).toBe('retry')
    expect(query.minTotalTokens).toBe(10)
    expect(query.maxTotalTokens).toBe(20)
    expect(query.minTotalMs).toBe(1.5)
    expect(query.maxTotalMs).toBe(2.5)
    expect(query.suggestField).toBeUndefined()
    expect(query.suggestQuery).toBeUndefined()
    expect(query.from).toBeDefined()
    expect(query.to).toBeDefined()
  })

  it('includes the active suggestion field and server-side search text when provided', () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      model: ' gpt-5.4-mini ',
    }

    const query = buildInvocationSuggestionsQuery(draft, 42, 'model')

    expect(query.snapshotId).toBe(42)
    expect(query.suggestField).toBe('model')
    expect(query.suggestQuery).toBe('gpt-5.4-mini')
    expect(query.model).toBe('gpt-5.4-mini')
  })

  it('preserves upstream scope across applied filters and suggestion queries', () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      upstreamScope: 'internal' as const,
    }

    expect(buildAppliedInvocationFilters(draft).upstreamScope).toBe('internal')
    expect(buildInvocationSuggestionsQuery(draft, 42).upstreamScope).toBe('internal')
  })

  it('tolerates invalid draft values when building suggestion queries', () => {
    const draft = {
      ...createDefaultInvocationRecordsDraft(),
      rangePreset: 'custom' as const,
      customFrom: '2026-03-10T10:',
      customTo: 'not-a-date',
      minTotalTokens: '1.5',
      maxTotalTokens: 'abc',
    }

    expect(() => buildInvocationSuggestionsQuery(draft, 42)).not.toThrow()

    const query = buildInvocationSuggestionsQuery(draft, 42)
    expect(query.snapshotId).toBe(42)
    expect(query.from).toBeUndefined()
    expect(query.to).toBeUndefined()
    expect(query.minTotalTokens).toBeUndefined()
    expect(query.maxTotalTokens).toBeUndefined()
  })
})
