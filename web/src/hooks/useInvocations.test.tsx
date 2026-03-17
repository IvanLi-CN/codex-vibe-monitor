/** @vitest-environment jsdom */
import { act, useMemo } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type { BroadcastPayload, ListResponse } from '../lib/api'
import { resolveInvocationAccountLabel } from '../lib/invocation'
import { useInvocationStream } from './useInvocations'

const apiMocks = vi.hoisted(() => ({
  fetchInvocations: vi.fn<(limit: number, params?: { model?: string; status?: string }) => Promise<ListResponse>>(),
}))

const sseMocks = vi.hoisted(() => ({
  onMessage: null as null | ((payload: BroadcastPayload) => void),
  onOpen: null as null | (() => void),
}))

vi.mock('../lib/api', async () => {
  const actual = await vi.importActual<typeof import('../lib/api')>('../lib/api')
  return {
    ...actual,
    fetchInvocations: apiMocks.fetchInvocations,
  }
})

vi.mock('../lib/sse', () => ({
  subscribeToSse: (handler: (payload: BroadcastPayload) => void) => {
    sseMocks.onMessage = handler
    return () => {
      sseMocks.onMessage = null
    }
  },
  subscribeToSseOpen: (handler: () => void) => {
    sseMocks.onOpen = handler
    return () => {
      sseMocks.onOpen = null
    }
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
  sseMocks.onMessage = null
  sseMocks.onOpen = null
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

function text(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`)
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`)
  }
  return element.textContent ?? ''
}

function Probe() {
  const filters = useMemo(() => ({ status: 'failed' as const }), [])
  const { records } = useInvocationStream(20, filters, undefined, { enableStream: true })

  return (
    <div>
      <div data-testid="count">{records.length}</div>
      <div data-testid="first-status">{records[0]?.status ?? ''}</div>
    </div>
  )
}

function StreamProbe({ limit = 20 }: { limit?: number }) {
  const { records } = useInvocationStream(limit, undefined, undefined, { enableStream: true })

  return (
    <div>
      <div data-testid="count">{records.length}</div>
      <div data-testid="first-status">{records[0]?.status ?? ''}</div>
      <div data-testid="first-key">{records[0] ? `${records[0].invokeId}@${records[0].occurredAt}` : ''}</div>
      <div data-testid="keys">{records.map((record) => `${record.invokeId}:${record.status}`).join('|')}</div>
    </div>
  )
}

function AccountLabelProbe() {
  const { records } = useInvocationStream(5, undefined, undefined, { enableStream: true })
  const label = records[0]
    ? resolveInvocationAccountLabel(
        records[0].routeMode,
        records[0].status,
        records[0].upstreamAccountName,
        records[0].upstreamAccountId,
        '反向代理',
        '号池路由中',
        '号池账号未知',
      )
    : ''

  return <div data-testid="first-account-label">{label}</div>
}

describe('useInvocationStream', () => {
  it('treats failed filters as resolved failures for incoming SSE records', async () => {
    apiMocks.fetchInvocations.mockResolvedValue({ records: [] })

    render(<Probe />)
    await flushAsync()

    expect(apiMocks.fetchInvocations).toHaveBeenCalledWith(20, { status: 'failed' })
    expect(text('count')).toBe('0')

    if (!sseMocks.onMessage) {
      throw new Error('missing SSE handler')
    }

    act(() => {
      sseMocks.onMessage?.({
        type: 'records',
        records: [
          {
            id: 1,
            invokeId: 'invoke-http-502',
            occurredAt: '2026-03-10T00:00:00Z',
            createdAt: '2026-03-10T00:00:00Z',
            status: 'http_502',
            failureClass: 'service_failure',
          },
          {
            id: 2,
            invokeId: 'invoke-success',
            occurredAt: '2026-03-10T00:01:00Z',
            createdAt: '2026-03-10T00:01:00Z',
            status: 'success',
            failureClass: 'none',
          },
        ],
      })
    })

    expect(text('count')).toBe('1')
    expect(text('first-status')).toBe('http_502')
  })

  it('does not reinsert records that already fell outside the current limit window', async () => {
    apiMocks.fetchInvocations.mockResolvedValue({
      records: [
        {
          id: 10,
          invokeId: 'invocation-a',
          occurredAt: '2026-03-10T00:02:00Z',
          createdAt: '2026-03-10T00:02:00Z',
          status: 'success',
        },
        {
          id: 11,
          invokeId: 'invocation-b',
          occurredAt: '2026-03-10T00:01:00Z',
          createdAt: '2026-03-10T00:01:00Z',
          status: 'success',
        },
      ],
    })

    render(<StreamProbe limit={2} />)
    await flushAsync()

    act(() => {
      sseMocks.onMessage?.({
        type: 'records',
        records: [
          {
            id: -1,
            invokeId: 'invocation-c',
            occurredAt: '2026-03-10T00:03:00Z',
            createdAt: '2026-03-10T00:03:00Z',
            status: 'running',
          },
        ],
      })
    })

    expect(text('keys')).toBe('invocation-c:running|invocation-a:success')

    act(() => {
      sseMocks.onMessage?.({
        type: 'records',
        records: [
          {
            id: -2,
            invokeId: 'invocation-b',
            occurredAt: '2026-03-10T00:01:00Z',
            createdAt: '2026-03-10T00:01:00Z',
            status: 'running',
          },
        ],
      })
    })

    expect(text('keys')).toBe('invocation-c:running|invocation-a:success')
  })

  it('keeps the terminal record when a stale running snapshot arrives later', async () => {
    apiMocks.fetchInvocations.mockResolvedValue({ records: [] })

    render(<StreamProbe limit={5} />)
    await flushAsync()

    act(() => {
      sseMocks.onMessage?.({
        type: 'records',
        records: [
          {
            id: 20,
            invokeId: 'invocation-terminal',
            occurredAt: '2026-03-10T00:04:00Z',
            createdAt: '2026-03-10T00:04:00Z',
            status: 'success',
            totalTokens: 18,
            cost: 0.0025,
            tTotalMs: 2400,
          },
        ],
      })
    })

    expect(text('keys')).toBe('invocation-terminal:success')
    expect(text('first-status')).toBe('success')

    act(() => {
      sseMocks.onMessage?.({
        type: 'records',
        records: [
          {
            id: -20,
            invokeId: 'invocation-terminal',
            occurredAt: '2026-03-10T00:04:00Z',
            createdAt: '2026-03-10T00:04:00Z',
            status: 'running',
          },
        ],
      })
    })

    expect(text('keys')).toBe('invocation-terminal:success')
    expect(text('first-status')).toBe('success')
  })

  it('upgrades the displayed account label when a richer pool snapshot arrives later', async () => {
    apiMocks.fetchInvocations.mockResolvedValue({ records: [] })

    render(<AccountLabelProbe />)
    await flushAsync()

    act(() => {
      sseMocks.onMessage?.({
        type: 'records',
        records: [
          {
            id: -30,
            invokeId: 'invocation-pool-upgrade',
            occurredAt: '2026-03-10T00:05:00Z',
            createdAt: '2026-03-10T00:05:00Z',
            routeMode: 'pool',
            status: 'running',
          },
        ],
      })
    })

    expect(text('first-account-label')).toBe('号池路由中')

    act(() => {
      sseMocks.onMessage?.({
        type: 'records',
        records: [
          {
            id: 30,
            invokeId: 'invocation-pool-upgrade',
            occurredAt: '2026-03-10T00:05:00Z',
            createdAt: '2026-03-10T00:05:00Z',
            routeMode: 'pool',
            upstreamAccountId: 42,
            upstreamAccountName: 'Pool Alpha',
            status: 'success',
            totalTokens: 128,
            tTotalMs: 1400,
          },
        ],
      })
    })

    expect(text('first-account-label')).toBe('Pool Alpha')
  })
})
