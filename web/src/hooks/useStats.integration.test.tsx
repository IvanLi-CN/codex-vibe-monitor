/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type { BroadcastPayload, StatsResponse } from '../lib/api'
import { clearSummaryRemountCache, useSummary } from './useStats'

const apiMocks = vi.hoisted(() => ({
  fetchSummary: vi.fn<
    (
      window: string,
      options?: { limit?: number; timeZone?: string; signal?: AbortSignal },
    ) => Promise<StatsResponse>
  >(),
}))

const sseMocks = vi.hoisted(() => ({
  listeners: new Set<(payload: BroadcastPayload) => void>(),
  openListeners: new Set<() => void>(),
}))

vi.mock('../lib/api', async () => {
  const actual = await vi.importActual<typeof import('../lib/api')>('../lib/api')
  return {
    ...actual,
    fetchSummary: apiMocks.fetchSummary,
  }
})

vi.mock('../lib/sse', () => ({
  subscribeToSse: (listener: (payload: BroadcastPayload) => void) => {
    sseMocks.listeners.add(listener)
    return () => sseMocks.listeners.delete(listener)
  },
  subscribeToSseOpen: (listener: () => void) => {
    sseMocks.openListeners.add(listener)
    return () => sseMocks.openListeners.delete(listener)
  },
}))

let host: HTMLDivElement | null = null
let root: Root | null = null

beforeAll(() => {
  Object.defineProperty(globalThis, 'IS_REACT_ACT_ENVIRONMENT', {
    configurable: true,
    writable: true,
    value: true,
  })
})

afterEach(() => {
  act(() => {
    root?.unmount()
  })
  host?.remove()
  host = null
  root = null
  clearSummaryRemountCache()
  sseMocks.listeners.clear()
  sseMocks.openListeners.clear()
  vi.useRealTimers()
  vi.clearAllMocks()
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

async function flushAsync(turns = 3) {
  for (let index = 0; index < turns; index += 1) {
    await act(async () => {
      await Promise.resolve()
    })
  }
}

function text(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`)
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`)
  }
  return element.textContent ?? ''
}

function emitSseOpen() {
  act(() => {
    sseMocks.openListeners.forEach((listener) => listener())
  })
}

function createSummary(totalCount: number): StatsResponse {
  return {
    totalCount,
    successCount: Math.max(0, totalCount - 1),
    failureCount: totalCount > 0 ? 1 : 0,
    totalCost: totalCount * 0.1,
    totalTokens: totalCount * 100,
  }
}

function Probe({ window }: { window: string }) {
  const { summary, isLoading, error } = useSummary(window)

  return (
    <div>
      <div data-testid="loading">{isLoading ? 'true' : 'false'}</div>
      <div data-testid="error">{error ?? ''}</div>
      <div data-testid="total">{String(summary?.totalCount ?? 0)}</div>
    </div>
  )
}

describe('useSummary SSE reconnect behavior', () => {
  it('forces a stale yesterday summary refresh on SSE reopen even inside the cooldown window', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date(2026, 3, 8, 23, 59, 59, 500))

    apiMocks.fetchSummary
      .mockResolvedValueOnce(createSummary(7))
      .mockResolvedValueOnce(createSummary(7))
      .mockResolvedValueOnce(createSummary(9))

    render(<Probe window="yesterday" />)
    await flushAsync()

    expect(apiMocks.fetchSummary).toHaveBeenCalledTimes(1)
    expect(text('loading')).toBe('false')
    expect(text('error')).toBe('')
    expect(text('total')).toBe('7')

    emitSseOpen()
    await flushAsync()

    expect(apiMocks.fetchSummary).toHaveBeenCalledTimes(2)
    expect(text('total')).toBe('7')

    vi.setSystemTime(new Date(2026, 3, 9, 0, 0, 1, 500))
    emitSseOpen()
    await flushAsync()

    expect(apiMocks.fetchSummary).toHaveBeenCalledTimes(3)
    expect(text('total')).toBe('9')
  })
})
