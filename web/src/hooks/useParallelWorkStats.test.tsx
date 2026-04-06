/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type { BroadcastPayload, ParallelWorkStatsResponse } from '../lib/api'
import {
  PARALLEL_WORK_OPEN_RESYNC_COOLDOWN_MS,
  PARALLEL_WORK_REFRESH_THROTTLE_MS,
  getParallelWorkRecordsResyncDelay,
  shouldTriggerParallelWorkOpenResync,
  useParallelWorkStats,
} from './useParallelWorkStats'

const apiMocks = vi.hoisted(() => ({
  fetchParallelWorkStats: vi.fn<
    (options?: { timeZone?: string; signal?: AbortSignal }) => Promise<ParallelWorkStatsResponse>
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
    fetchParallelWorkStats: apiMocks.fetchParallelWorkStats,
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

async function flushAsync() {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

function createStats(): ParallelWorkStatsResponse {
  return {
    minute7d: {
      rangeStart: '2026-03-01T00:00:00Z',
      rangeEnd: '2026-03-08T00:00:00Z',
      bucketSeconds: 60,
      completeBucketCount: 10080,
      activeBucketCount: 3,
      minCount: 0,
      maxCount: 3,
      avgCount: 1.2,
      points: [
        { bucketStart: '2026-03-07T10:00:00Z', bucketEnd: '2026-03-07T10:01:00Z', parallelCount: 1 },
      ],
    },
    hour30d: {
      rangeStart: '2026-02-06T00:00:00Z',
      rangeEnd: '2026-03-08T00:00:00Z',
      bucketSeconds: 3600,
      completeBucketCount: 720,
      activeBucketCount: 2,
      minCount: 0,
      maxCount: 2,
      avgCount: 0.4,
      points: [
        { bucketStart: '2026-03-07T00:00:00Z', bucketEnd: '2026-03-07T01:00:00Z', parallelCount: 2 },
      ],
    },
    dayAll: {
      rangeStart: '2026-01-01T00:00:00Z',
      rangeEnd: '2026-03-08T00:00:00Z',
      bucketSeconds: 86400,
      completeBucketCount: 5,
      activeBucketCount: 5,
      minCount: 1,
      maxCount: 2,
      avgCount: 1.4,
      points: [
        { bucketStart: '2026-03-07T00:00:00Z', bucketEnd: '2026-03-08T00:00:00Z', parallelCount: 2 },
      ],
    },
  }
}

function Probe() {
  const { data, isLoading, error } = useParallelWorkStats()
  return (
    <div>
      <div data-testid="loading">{isLoading ? 'true' : 'false'}</div>
      <div data-testid="error">{error ?? ''}</div>
      <div data-testid="minute-count">{String(data?.minute7d.points[0]?.parallelCount ?? 0)}</div>
    </div>
  )
}

describe('useParallelWorkStats helpers', () => {
  it('computes refresh delay from the last records refresh', () => {
    expect(getParallelWorkRecordsResyncDelay(10_000, 20_000)).toBe(
      PARALLEL_WORK_REFRESH_THROTTLE_MS - 10_000,
    )
    expect(getParallelWorkRecordsResyncDelay(10_000, 80_000)).toBe(0)
  })

  it('enforces the open-resync cooldown unless forced', () => {
    expect(shouldTriggerParallelWorkOpenResync(0, 5_000)).toBe(false)
    expect(
      shouldTriggerParallelWorkOpenResync(0, PARALLEL_WORK_OPEN_RESYNC_COOLDOWN_MS + 1),
    ).toBe(true)
    expect(shouldTriggerParallelWorkOpenResync(0, 1, true)).toBe(true)
  })
})

describe('useParallelWorkStats', () => {
  it('throttles records-triggered silent refreshes to at most once per minute', async () => {
    vi.useFakeTimers()
    apiMocks.fetchParallelWorkStats.mockResolvedValue(createStats())

    render(<Probe />)
    await flushAsync()
    expect(apiMocks.fetchParallelWorkStats).toHaveBeenCalledTimes(1)
    expect(host?.querySelector('[data-testid="minute-count"]')?.textContent).toBe('1')

    act(() => {
      sseMocks.listeners.forEach((listener) => listener({ type: 'records', records: [] }))
    })
    await flushAsync()
    expect(apiMocks.fetchParallelWorkStats).toHaveBeenCalledTimes(2)

    act(() => {
      sseMocks.listeners.forEach((listener) => listener({ type: 'records', records: [] }))
    })
    await vi.advanceTimersByTimeAsync(PARALLEL_WORK_REFRESH_THROTTLE_MS - 1)
    expect(apiMocks.fetchParallelWorkStats).toHaveBeenCalledTimes(2)

    await vi.advanceTimersByTimeAsync(1)
    await flushAsync()
    expect(apiMocks.fetchParallelWorkStats).toHaveBeenCalledTimes(3)
  })

  it('respects the SSE-open cooldown before forcing another refresh', async () => {
    vi.useFakeTimers()
    apiMocks.fetchParallelWorkStats.mockResolvedValue(createStats())

    render(<Probe />)
    await flushAsync()
    expect(apiMocks.fetchParallelWorkStats).toHaveBeenCalledTimes(1)

    act(() => {
      sseMocks.openListeners.forEach((listener) => listener())
    })
    await flushAsync()
    expect(apiMocks.fetchParallelWorkStats).toHaveBeenCalledTimes(2)

    act(() => {
      sseMocks.openListeners.forEach((listener) => listener())
    })
    await flushAsync()
    expect(apiMocks.fetchParallelWorkStats).toHaveBeenCalledTimes(2)

    await vi.advanceTimersByTimeAsync(PARALLEL_WORK_OPEN_RESYNC_COOLDOWN_MS)
    act(() => {
      sseMocks.openListeners.forEach((listener) => listener())
    })
    await flushAsync()
    expect(apiMocks.fetchParallelWorkStats).toHaveBeenCalledTimes(3)
  })
})
