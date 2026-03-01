import { afterEach, describe, expect, it, vi } from 'vitest'
import { fetchForwardProxyLiveStats, validateForwardProxyCandidate } from './api'

function abortError(): Error {
  const error = new Error('aborted')
  error.name = 'AbortError'
  return error
}

function createAbortAwareFetch() {
  return vi.fn((_input: RequestInfo | URL, init?: RequestInit) => {
    return new Promise<Response>((_resolve, reject) => {
      const signal = init?.signal
      if (!signal) return
      if (signal.aborted) {
        reject(abortError())
        return
      }
      signal.addEventListener(
        'abort',
        () => {
          reject(abortError())
        },
        { once: true },
      )
    })
  })
}

describe('validateForwardProxyCandidate timeout split', () => {
  afterEach(() => {
    vi.useRealTimers()
    vi.unstubAllGlobals()
  })

  it('uses 60s timeout for subscription validation', async () => {
    vi.useFakeTimers()
    const fetchMock = createAbortAwareFetch()
    vi.stubGlobal('fetch', fetchMock as typeof fetch)

    const pending = validateForwardProxyCandidate({
      kind: 'subscriptionUrl',
      value: 'https://example.com/subscription',
    })

    const assertion = expect(pending).rejects.toThrow('validation request timed out after 60s')
    await vi.advanceTimersByTimeAsync(60_000)
    await assertion
    expect(fetchMock).toHaveBeenCalledTimes(1)
  })

  it('keeps 5s timeout for single proxy validation', async () => {
    vi.useFakeTimers()
    const fetchMock = createAbortAwareFetch()
    vi.stubGlobal('fetch', fetchMock as typeof fetch)

    const pending = validateForwardProxyCandidate({
      kind: 'proxyUrl',
      value: 'socks5://127.0.0.1:1080',
    })

    const assertion = expect(pending).rejects.toThrow('validation request timed out after 5s')
    await vi.advanceTimersByTimeAsync(5_000)
    await assertion
    expect(fetchMock).toHaveBeenCalledTimes(1)
  })
})

describe('fetchForwardProxyLiveStats', () => {
  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('normalizes live proxy stats payload', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn(async () => {
        return new Response(
          JSON.stringify({
            rangeStart: '2026-03-01T00:00:00Z',
            rangeEnd: '2026-03-02T00:00:00Z',
            bucketSeconds: 3600,
            nodes: [
              {
                key: '__direct__',
                source: 'direct',
                displayName: 'Direct',
                weight: 1,
                penalized: false,
                stats: {
                  oneMinute: { attempts: 2, successRate: 0.5, avgLatencyMs: 123 },
                  fifteenMinutes: { attempts: 10, successRate: 0.6, avgLatencyMs: 130 },
                  oneHour: { attempts: 40, successRate: 0.7, avgLatencyMs: 140 },
                  oneDay: { attempts: 200, successRate: 0.8, avgLatencyMs: 150 },
                  sevenDays: { attempts: 1200, successRate: 0.9, avgLatencyMs: 160 },
                },
                last24h: [
                  {
                    bucketStart: '2026-03-01T00:00:00Z',
                    bucketEnd: '2026-03-01T01:00:00Z',
                    successCount: 3,
                    failureCount: 1,
                  },
                  {
                    bucketStart: '',
                    bucketEnd: '',
                    successCount: 99,
                    failureCount: 99,
                  },
                ],
              },
            ],
          }),
          { status: 200, headers: { 'Content-Type': 'application/json' } },
        )
      }) as typeof fetch,
    )

    const response = await fetchForwardProxyLiveStats()
    expect(response.bucketSeconds).toBe(3600)
    expect(response.nodes).toHaveLength(1)
    expect(response.nodes[0].displayName).toBe('Direct')
    expect(response.nodes[0].stats.oneMinute.attempts).toBe(2)
    expect(response.nodes[0].last24h).toHaveLength(1)
    expect(response.nodes[0].last24h[0].successCount).toBe(3)
    expect(response.nodes[0].last24h[0].failureCount).toBe(1)
  })
})
