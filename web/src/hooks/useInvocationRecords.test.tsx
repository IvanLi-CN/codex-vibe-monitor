/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type { InvocationRecordsQuery, InvocationRecordsResponse, InvocationRecordsSummaryResponse } from '../lib/api'
import { RECORDS_NEW_COUNT_POLL_INTERVAL_MS } from '../lib/invocationRecords'
import { useInvocationRecords } from './useInvocationRecords'

const apiMocks = vi.hoisted(() => ({
  fetchInvocationRecords: vi.fn<(query: InvocationRecordsQuery) => Promise<InvocationRecordsResponse>>(),
  fetchInvocationRecordsSummary: vi.fn<(query: InvocationRecordsQuery) => Promise<InvocationRecordsSummaryResponse>>(),
}))

vi.mock('../lib/api', async () => {
  const actual = await vi.importActual<typeof import('../lib/api')>('../lib/api')
  return {
    ...actual,
    fetchInvocationRecords: apiMocks.fetchInvocationRecords,
    fetchInvocationRecordsSummary: apiMocks.fetchInvocationRecordsSummary,
  }
})

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
  vi.clearAllMocks()
  vi.useRealTimers()
})

function render(ui: React.ReactNode) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(ui)
  })
}

function click(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`)
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`)
  }
  act(() => {
    element.dispatchEvent(new MouseEvent('click', { bubbles: true }))
  })
}

function text(testId: string) {
  const element = host?.querySelector(`[data-testid="${testId}"]`)
  if (!(element instanceof HTMLElement)) {
    throw new Error(`Missing element: ${testId}`)
  }
  return element.textContent ?? ''
}


async function flushAsync() {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
    await Promise.resolve()
  })
}

function createListResponse(overrides: Partial<InvocationRecordsResponse>): InvocationRecordsResponse {
  return {
    snapshotId: 42,
    total: 2,
    page: 1,
    pageSize: 20,
    records: [
      {
        id: 1,
        invokeId: 'invoke-1',
        occurredAt: '2026-03-10T00:00:00Z',
        createdAt: '2026-03-10T00:00:00Z',
        model: 'baseline-model',
        status: 'success',
      },
    ],
    ...overrides,
  }
}

function createSummaryResponse(overrides: Partial<InvocationRecordsSummaryResponse>): InvocationRecordsSummaryResponse {
  return {
    snapshotId: 42,
    newRecordsCount: 0,
    totalCount: 2,
    successCount: 2,
    failureCount: 0,
    totalCost: 0.25,
    totalTokens: 1000,
    token: {
      requestCount: 2,
      totalTokens: 1000,
      avgTokensPerRequest: 500,
      cacheInputTokens: 200,
      totalCost: 0.25,
    },
    network: {
      avgTtfbMs: 120,
      p95TtfbMs: 180,
      avgTotalMs: 450,
      p95TotalMs: 600,
    },
    exception: {
      failureCount: 0,
      serviceFailureCount: 0,
      clientFailureCount: 0,
      clientAbortCount: 0,
      actionableFailureCount: 0,
    },
    ...overrides,
  }
}

function Probe() {
  const state = useInvocationRecords()

  return (
    <div>
      <div data-testid="focus">{state.focus}</div>
      <div data-testid="page">{state.page}</div>
      <div data-testid="snapshot">{state.records?.snapshotId ?? 0}</div>
      <div data-testid="model">{state.records?.records[0]?.model ?? ''}</div>
      <div data-testid="new-count">{state.summary?.newRecordsCount ?? 0}</div>
      <button data-testid="focus-network" type="button" onClick={() => state.setFocus('network')}>
        network
      </button>
      <button data-testid="draft-model" type="button" onClick={() => state.updateDraft('model', 'next-model')}>
        draft
      </button>
      <button data-testid="page-2" type="button" onClick={() => void state.setPage(2)}>
        page2
      </button>
      <button data-testid="search" type="button" onClick={() => void state.search()}>
        search
      </button>
    </div>
  )
}

describe('useInvocationRecords', () => {
  it('keeps paging on the applied snapshot until search refreshes it', async () => {
    vi.useFakeTimers()

    let summary42CallCount = 0
    apiMocks.fetchInvocationRecords.mockImplementation(async (query) => {
      if (query.snapshotId === 42) {
        return createListResponse({
          snapshotId: 42,
          page: query.page ?? 1,
          pageSize: query.pageSize ?? 20,
          records: [
            {
              id: query.page === 2 ? 2 : 1,
              invokeId: `invoke-${query.page ?? 1}`,
              occurredAt: '2026-03-10T00:00:00Z',
              createdAt: '2026-03-10T00:00:00Z',
              model: 'baseline-model',
              status: 'success',
            },
          ],
        })
      }

      if (query.model === 'next-model') {
        return createListResponse({
          snapshotId: 84,
          total: 1,
          page: 1,
          pageSize: query.pageSize ?? 20,
          records: [
            {
              id: 9,
              invokeId: 'invoke-next',
              occurredAt: '2026-03-10T01:00:00Z',
              createdAt: '2026-03-10T01:00:00Z',
              model: 'next-model',
              status: 'failed',
            },
          ],
        })
      }

      return createListResponse({ snapshotId: 42 })
    })

    apiMocks.fetchInvocationRecordsSummary.mockImplementation(async (query) => {
      if (query.snapshotId === 84) {
        return createSummaryResponse({
          snapshotId: 84,
          totalCount: 1,
          successCount: 0,
          failureCount: 1,
          token: {
            requestCount: 1,
            totalTokens: 300,
            avgTokensPerRequest: 300,
            cacheInputTokens: 0,
            totalCost: 0.12,
          },
          exception: {
            failureCount: 1,
            serviceFailureCount: 1,
            clientFailureCount: 0,
            clientAbortCount: 0,
            actionableFailureCount: 1,
          },
        })
      }

      summary42CallCount += 1
      return createSummaryResponse({ newRecordsCount: summary42CallCount >= 2 ? 3 : 0 })
    })

    render(<Probe />)
    await flushAsync()

    expect(text('snapshot')).toBe('42')
    expect(text('page')).toBe('1')
    expect(text('model')).toBe('baseline-model')

    expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledTimes(1)
    expect(apiMocks.fetchInvocationRecordsSummary).toHaveBeenCalledTimes(1)
    expect(apiMocks.fetchInvocationRecords.mock.calls[0][0]?.snapshotId).toBeUndefined()
    expect(apiMocks.fetchInvocationRecordsSummary.mock.calls[0][0]?.snapshotId).toBe(42)

    click('focus-network')
    expect(text('focus')).toBe('network')
    expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledTimes(1)
    expect(apiMocks.fetchInvocationRecordsSummary).toHaveBeenCalledTimes(1)

    click('draft-model')
    click('page-2')
    await flushAsync()

    expect(text('page')).toBe('2')
    expect(text('snapshot')).toBe('42')

    const pageQuery = apiMocks.fetchInvocationRecords.mock.calls.at(-1)?.[0]
    expect(pageQuery?.snapshotId).toBe(42)
    expect(pageQuery?.page).toBe(2)
    expect(pageQuery?.model).toBeUndefined()
    expect(text('model')).toBe('baseline-model')

    await act(async () => {
      await vi.advanceTimersByTimeAsync(RECORDS_NEW_COUNT_POLL_INTERVAL_MS)
    })

    await flushAsync()
    expect(text('new-count')).toBe('3')

    const pollQuery = apiMocks.fetchInvocationRecordsSummary.mock.calls.at(-1)?.[0]
    expect(pollQuery?.snapshotId).toBe(42)
    expect(pollQuery?.model).toBeUndefined()

    const recordsCallsBeforeSearch = apiMocks.fetchInvocationRecords.mock.calls.length
    const summaryCallsBeforeSearch = apiMocks.fetchInvocationRecordsSummary.mock.calls.length
    click('search')
    await flushAsync()

    expect(text('snapshot')).toBe('84')
    expect(text('page')).toBe('1')
    expect(text('model')).toBe('next-model')
    expect(text('new-count')).toBe('0')

    const searchQuery = apiMocks.fetchInvocationRecords.mock.calls[recordsCallsBeforeSearch]?.[0]
    expect(searchQuery?.snapshotId).toBeUndefined()
    expect(searchQuery?.page).toBe(1)
    expect(searchQuery?.model).toBe('next-model')

    const searchSummaryQuery = apiMocks.fetchInvocationRecordsSummary.mock.calls[summaryCallsBeforeSearch]?.[0]
    expect(searchSummaryQuery?.snapshotId).toBe(84)
    expect(searchSummaryQuery?.model).toBe('next-model')
  })
})
