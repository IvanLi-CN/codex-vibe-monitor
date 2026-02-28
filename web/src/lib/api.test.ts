import { afterEach, describe, expect, it, vi } from 'vitest'
import { validateForwardProxyCandidate } from './api'

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
