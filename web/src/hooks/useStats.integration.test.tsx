/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type { ApiInvocation, BroadcastPayload, StatsResponse } from '../lib/api'
import { DASHBOARD_SSE_VISIBLE_PATCH_BATCH_MS } from '../lib/dashboardSseLocalPatch'
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

function emitSseRecords(records: ApiInvocation[]) {
  act(() => {
    sseMocks.listeners.forEach((listener) => listener({ type: 'records', records }))
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

function createRecord(overrides: Partial<ApiInvocation> & { id: number; invokeId: string; status: string }): ApiInvocation {
  return {
    id: overrides.id,
    invokeId: overrides.invokeId,
    occurredAt: overrides.occurredAt ?? '2026-04-08T12:00:00.000Z',
    createdAt: overrides.createdAt ?? overrides.occurredAt ?? '2026-04-08T12:00:00.000Z',
    status: overrides.status,
    source: overrides.source ?? 'pool',
    routeMode: overrides.routeMode ?? 'pool',
    model: overrides.model ?? 'gpt-5.5',
    endpoint: overrides.endpoint ?? '/v1/responses',
    totalTokens: overrides.totalTokens ?? 0,
    cost: overrides.cost ?? 0,
    upstreamAccountId: overrides.upstreamAccountId ?? null,
    poolAttemptCount: overrides.poolAttemptCount,
  }
}

function Probe({ window }: { window: string }) {
  const { summary, isLoading, error } = useSummary(window)

  return (
    <div>
      <div data-testid="loading">{isLoading ? 'true' : 'false'}</div>
      <div data-testid="error">{error ?? ''}</div>
      <div data-testid="total">{String(summary?.totalCount ?? 0)}</div>
      <div data-testid="in-progress">{String(summary?.inProgressConversationCount ?? 0)}</div>
    </div>
  )
}

describe('useSummary SSE reconnect behavior', () => {
  it('patches today summary from SSE records after the 1s visible batch without immediate HTTP reconcile', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date(2026, 3, 8, 12, 0, 0))
    apiMocks.fetchSummary.mockResolvedValue(createSummary(10))

    render(<Probe window="today" />)
    await flushAsync()

    expect(apiMocks.fetchSummary).toHaveBeenCalledTimes(1)
    expect(text('total')).toBe('10')

    emitSseRecords([
      createRecord({ id: 11, invokeId: 'success-live', status: 'success', totalTokens: 70, cost: 0.07 }),
      createRecord({ id: 12, invokeId: 'failed-live', status: 'failed', totalTokens: 30, cost: 0.03 }),
    ])
    expect(text('total')).toBe('10')
    expect(apiMocks.fetchSummary).toHaveBeenCalledTimes(1)

    await act(async () => {
      vi.advanceTimersByTime(DASHBOARD_SSE_VISIBLE_PATCH_BATCH_MS)
    })

    expect(text('total')).toBe('12')
    expect(apiMocks.fetchSummary).toHaveBeenCalledTimes(1)

    await act(async () => {
      vi.advanceTimersByTime(4_000)
    })
    emitSseRecords([
      createRecord({ id: 13, invokeId: 'success-live-2', status: 'success', totalTokens: 10, cost: 0.01 }),
    ])
    await flushAsync()

    expect(apiMocks.fetchSummary).toHaveBeenCalledTimes(2)
  })

  it('ignores out-of-day records and replaces running contribution when the terminal record arrives', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date(2026, 3, 8, 12, 0, 0))
    apiMocks.fetchSummary.mockResolvedValue({
      ...createSummary(10),
      inProgressConversationCount: 1,
      inProgressRetryConversationCount: 0,
    })

    render(<Probe window="today" />)
    await flushAsync()

    emitSseRecords([
      createRecord({
        id: 20,
        invokeId: 'older-record',
        status: 'success',
        occurredAt: '2026-04-07T15:59:59.000Z',
        totalTokens: 10,
      }),
    ])
    await act(async () => {
      vi.advanceTimersByTime(DASHBOARD_SSE_VISIBLE_PATCH_BATCH_MS)
    })

    expect(text('total')).toBe('10')
    expect(text('in-progress')).toBe('1')

    emitSseRecords([
      createRecord({
        id: 21,
        invokeId: 'lifecycle-record',
        status: 'running',
        occurredAt: '2026-04-08T12:00:00.000Z',
        poolAttemptCount: 2,
      }),
    ])
    await act(async () => {
      vi.advanceTimersByTime(DASHBOARD_SSE_VISIBLE_PATCH_BATCH_MS)
    })
    expect(text('in-progress')).toBe('2')

    emitSseRecords([
      createRecord({
        id: 21,
        invokeId: 'lifecycle-record',
        status: 'success',
        occurredAt: '2026-04-08T12:00:00.000Z',
        totalTokens: 20,
        cost: 0.02,
      }),
    ])
    await act(async () => {
      vi.advanceTimersByTime(DASHBOARD_SSE_VISIBLE_PATCH_BATCH_MS)
    })

    expect(text('total')).toBe('11')
    expect(text('in-progress')).toBe('1')
  })

  it('consumes hydrated in-progress count when only the terminal record arrives later', async () => {
    vi.useFakeTimers()
    vi.setSystemTime(new Date(2026, 3, 8, 12, 0, 0))
    apiMocks.fetchSummary.mockResolvedValue({
      ...createSummary(10),
      inProgressConversationCount: 1,
      inProgressRetryConversationCount: 1,
    })

    render(<Probe window="today" />)
    await flushAsync()

    emitSseRecords([
      createRecord({
        id: 30,
        invokeId: 'prehydrated-running',
        status: 'success',
        occurredAt: '2026-04-08T12:00:00.000Z',
        createdAt: '2026-04-08T03:59:59.000Z',
        totalTokens: 20,
        cost: 0.02,
        poolAttemptCount: 2,
      }),
    ])
    await act(async () => {
      vi.advanceTimersByTime(DASHBOARD_SSE_VISIBLE_PATCH_BATCH_MS)
    })

    expect(text('total')).toBe('11')
    expect(text('in-progress')).toBe('0')
  })

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
