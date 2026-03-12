/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type {
  InvocationRecordsNewCountResponse,
  InvocationRecordsQuery,
  InvocationRecordsResponse,
  InvocationRecordsSummaryResponse,
} from '../lib/api'
import { RECORDS_NEW_COUNT_POLL_INTERVAL_MS } from '../lib/invocationRecords'
import { useInvocationRecords } from './useInvocationRecords'

const apiMocks = vi.hoisted(() => ({
  fetchInvocationRecords: vi.fn<(query: InvocationRecordsQuery) => Promise<InvocationRecordsResponse>>(),
  fetchInvocationRecordsSummary: vi.fn<(query: InvocationRecordsQuery) => Promise<InvocationRecordsSummaryResponse>>(),
  fetchInvocationRecordsNewCount: vi.fn<(query: InvocationRecordsQuery) => Promise<InvocationRecordsNewCountResponse>>(),
}))

vi.mock('../lib/api', async () => {
  const actual = await vi.importActual<typeof import('../lib/api')>('../lib/api')
  return {
    ...actual,
    fetchInvocationRecords: apiMocks.fetchInvocationRecords,
    fetchInvocationRecordsSummary: apiMocks.fetchInvocationRecordsSummary,
    fetchInvocationRecordsNewCount: apiMocks.fetchInvocationRecordsNewCount,
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

function createNewCountResponse(overrides: Partial<InvocationRecordsNewCountResponse>): InvocationRecordsNewCountResponse {
  return {
    snapshotId: 42,
    newRecordsCount: 0,
    ...overrides,
  }
}

function Probe() {
  const state = useInvocationRecords()

  return (
    <div>
      <div data-testid="focus">{state.focus}</div>
      <div data-testid="page">{state.page}</div>
      <div data-testid="page-size">{state.pageSize}</div>
      <div data-testid="snapshot">{state.records?.snapshotId ?? 0}</div>
      <div data-testid="model">{state.records?.records[0]?.model ?? ''}</div>
      <div data-testid="new-count">{state.summary?.newRecordsCount ?? 0}</div>
      <div data-testid="summary-snapshot">{state.summary?.snapshotId ?? 0}</div>
      <div data-testid="records-loading">{state.isRecordsLoading ? 'yes' : 'no'}</div>
      <div data-testid="summary-loading">{state.isSummaryLoading ? 'yes' : 'no'}</div>
      <div data-testid="records-error">{state.recordsError ?? ''}</div>
      <div data-testid="summary-error">{state.summaryError ?? ''}</div>
      <button data-testid="focus-network" type="button" onClick={() => state.setFocus('network')}>
        network
      </button>
      <button data-testid="draft-model" type="button" onClick={() => state.updateDraft('model', 'next-model')}>
        draft
      </button>
      <button data-testid="page-2" type="button" onClick={() => void state.setPage(2)}>
        page2
      </button>
      <button data-testid="page-size-50" type="button" onClick={() => void state.setPageSize(50)}>
        pageSize50
      </button>
      <button data-testid="search" type="button" onClick={() => void state.search()}>
        search
      </button>
      <button data-testid="refresh-applied" type="button" onClick={() => void state.search({ source: 'applied', preserveSummary: true })}>
        refreshApplied
      </button>
    </div>
  )
}

describe('useInvocationRecords', () => {
  it('keeps paging on the applied snapshot until search refreshes it', async () => {
    vi.useFakeTimers()

    let newCount42CallCount = 0
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
          newRecordsCount: 9,
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

      return createSummaryResponse({})
    })

    apiMocks.fetchInvocationRecordsNewCount.mockImplementation(async (query) => {
      newCount42CallCount += 1
      return createNewCountResponse({
        snapshotId: query.snapshotId ?? 42,
        newRecordsCount: newCount42CallCount >= 1 ? 3 : 0,
      })
    })

    render(<Probe />)
    await flushAsync()

    expect(text('snapshot')).toBe('42')
    expect(text('page')).toBe('1')
    expect(text('model')).toBe('baseline-model')

    expect(apiMocks.fetchInvocationRecords).toHaveBeenCalledTimes(1)
    expect(apiMocks.fetchInvocationRecordsSummary).toHaveBeenCalledTimes(1)
    expect(apiMocks.fetchInvocationRecordsNewCount).toHaveBeenCalledTimes(0)
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

    const pollQuery = apiMocks.fetchInvocationRecordsNewCount.mock.calls.at(-1)?.[0]
    expect(pollQuery?.snapshotId).toBe(42)
    expect(pollQuery?.model).toBeUndefined()

    const recordsCallsBeforeSearch = apiMocks.fetchInvocationRecords.mock.calls.length
    const summaryCallsBeforeSearch = apiMocks.fetchInvocationRecordsSummary.mock.calls.length
    const newCountCallsBeforeSearch = apiMocks.fetchInvocationRecordsNewCount.mock.calls.length
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

    expect(apiMocks.fetchInvocationRecordsNewCount.mock.calls.length).toBe(newCountCallsBeforeSearch)

    const searchSummaryQuery = apiMocks.fetchInvocationRecordsSummary.mock.calls[summaryCallsBeforeSearch]?.[0]
    expect(searchSummaryQuery?.snapshotId).toBe(84)
    expect(searchSummaryQuery?.model).toBe('next-model')
  })

  it('ignores stale new-count responses after a search even when snapshotId repeats', async () => {
    vi.useFakeTimers()

    let resolvePoll: ((value: InvocationRecordsNewCountResponse) => void) | null = null
    const pollPromise = new Promise<InvocationRecordsNewCountResponse>((resolve) => {
      resolvePoll = resolve
    })

    apiMocks.fetchInvocationRecords.mockImplementation(async (query) => {
      if (query.model === 'next-model') {
        return createListResponse({
          snapshotId: 42,
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
              status: 'success',
            },
          ],
        })
      }

      return createListResponse({ snapshotId: 42 })
    })

    apiMocks.fetchInvocationRecordsSummary.mockResolvedValue(createSummaryResponse({ snapshotId: 42, newRecordsCount: 0 }))
    apiMocks.fetchInvocationRecordsNewCount.mockImplementation(async () => pollPromise)

    render(<Probe />)
    await flushAsync()

    await act(async () => {
      await vi.advanceTimersByTimeAsync(RECORDS_NEW_COUNT_POLL_INTERVAL_MS)
    })

    expect(apiMocks.fetchInvocationRecordsNewCount).toHaveBeenCalledTimes(1)

    click('draft-model')
    click('search')
    await flushAsync()

    expect(text('snapshot')).toBe('42')
    expect(text('model')).toBe('next-model')
    expect(text('new-count')).toBe('0')

    if (!resolvePoll) {
      throw new Error('poll resolver missing')
    }
    const resolvePollFn = resolvePoll as (value: InvocationRecordsNewCountResponse) => void
    resolvePollFn(createNewCountResponse({ snapshotId: 42, newRecordsCount: 7 }))
    await flushAsync()

    expect(text('new-count')).toBe('0')
  })

  it('refreshes using the applied filters and keeps the previous summary visible until the new summary resolves', async () => {
    vi.useFakeTimers()
    let initialListCalls = 0
    let resolveRefreshSummary: ((value: InvocationRecordsSummaryResponse) => void) | null = null

    apiMocks.fetchInvocationRecords.mockImplementation(async (query) => {
      initialListCalls += 1

      if (initialListCalls === 1) {
        return createListResponse({ snapshotId: 42 })
      }

      expect(query.model).toBeUndefined()
      expect(query.snapshotId).toBeUndefined()
      return createListResponse({
        snapshotId: 84,
        records: [
          {
            id: 9,
            invokeId: 'invoke-refresh',
            occurredAt: '2026-03-10T01:00:00Z',
            createdAt: '2026-03-10T01:00:00Z',
            model: 'baseline-refreshed',
            status: 'success',
          },
        ],
      })
    })

    apiMocks.fetchInvocationRecordsSummary
      .mockResolvedValueOnce(createSummaryResponse({ snapshotId: 42, newRecordsCount: 3 }))
      .mockImplementationOnce(
        async () =>
          new Promise<InvocationRecordsSummaryResponse>((resolve) => {
            resolveRefreshSummary = resolve
          }),
      )

    apiMocks.fetchInvocationRecordsNewCount.mockResolvedValue(createNewCountResponse({ snapshotId: 42, newRecordsCount: 3 }))

    render(<Probe />)
    await flushAsync()

    await act(async () => {
      await vi.advanceTimersByTimeAsync(RECORDS_NEW_COUNT_POLL_INTERVAL_MS)
    })
    await flushAsync()

    expect(text('snapshot')).toBe('42')
    expect(text('summary-snapshot')).toBe('42')
    expect(text('new-count')).toBe('3')

    click('draft-model')
    click('refresh-applied')
    await flushAsync()

    expect(text('snapshot')).toBe('84')
    expect(text('model')).toBe('baseline-refreshed')
    expect(text('summary-snapshot')).toBe('42')
    expect(text('new-count')).toBe('3')
    expect(text('summary-loading')).toBe('yes')

    const refreshListQuery = apiMocks.fetchInvocationRecords.mock.calls.at(-1)?.[0]
    expect(refreshListQuery?.model).toBeUndefined()
    expect(refreshListQuery?.snapshotId).toBeUndefined()

    if (!resolveRefreshSummary) {
      throw new Error('refresh summary resolver missing')
    }

    act(() => {
      resolveRefreshSummary?.(createSummaryResponse({ snapshotId: 84, newRecordsCount: 0, totalCount: 1 }))
    })
    await flushAsync()

    const refreshSummaryQuery = apiMocks.fetchInvocationRecordsSummary.mock.calls.at(-1)?.[0]
    expect(refreshSummaryQuery?.model).toBeUndefined()
    expect(refreshSummaryQuery?.snapshotId).toBe(84)
    expect(text('summary-snapshot')).toBe('84')
    expect(text('new-count')).toBe('0')
    expect(text('summary-loading')).toBe('no')
  })

  it('ignores stale overlapping new-count polls for the same snapshot', async () => {
    vi.useFakeTimers()

    const pollResolvers: Array<(value: InvocationRecordsNewCountResponse) => void> = []

    apiMocks.fetchInvocationRecords.mockResolvedValue(createListResponse({ snapshotId: 42 }))
    apiMocks.fetchInvocationRecordsSummary.mockResolvedValue(createSummaryResponse({ snapshotId: 42, newRecordsCount: 0 }))
    apiMocks.fetchInvocationRecordsNewCount.mockImplementation(
      async () =>
        new Promise<InvocationRecordsNewCountResponse>((resolve) => {
          pollResolvers.push(resolve)
        }),
    )

    render(<Probe />)
    await flushAsync()

    await act(async () => {
      await vi.advanceTimersByTimeAsync(RECORDS_NEW_COUNT_POLL_INTERVAL_MS)
    })
    await act(async () => {
      await vi.advanceTimersByTimeAsync(RECORDS_NEW_COUNT_POLL_INTERVAL_MS)
    })

    expect(apiMocks.fetchInvocationRecordsNewCount).toHaveBeenCalledTimes(2)
    expect(pollResolvers).toHaveLength(2)

    act(() => {
      pollResolvers[1](createNewCountResponse({ snapshotId: 42, newRecordsCount: 9 }))
    })
    await flushAsync()
    expect(text('new-count')).toBe('9')

    act(() => {
      pollResolvers[0](createNewCountResponse({ snapshotId: 42, newRecordsCount: 3 }))
    })
    await flushAsync()
    expect(text('new-count')).toBe('9')
  })

  it('shows records as soon as the list resolves even if the summary is still pending', async () => {
    let resolveSummary: ((value: InvocationRecordsSummaryResponse) => void) | null = null
    const summaryPromise = new Promise<InvocationRecordsSummaryResponse>((resolve) => {
      resolveSummary = resolve
    })

    apiMocks.fetchInvocationRecords.mockResolvedValue(createListResponse({ snapshotId: 42 }))
    apiMocks.fetchInvocationRecordsSummary.mockImplementation(async () => summaryPromise)
    apiMocks.fetchInvocationRecordsNewCount.mockResolvedValue(createNewCountResponse({ snapshotId: 42, newRecordsCount: 0 }))

    render(<Probe />)
    await flushAsync()

    expect(text('snapshot')).toBe('42')
    expect(text('model')).toBe('baseline-model')
    expect(text('records-loading')).toBe('no')
    expect(text('summary-loading')).toBe('yes')

    if (!resolveSummary) {
      throw new Error('summary resolver missing')
    }
    const resolveSummaryFn = resolveSummary as (value: InvocationRecordsSummaryResponse) => void
    resolveSummaryFn(createSummaryResponse({ snapshotId: 42, newRecordsCount: 0 }))
    await flushAsync()

    expect(text('summary-loading')).toBe('no')
    expect(text('summary-error')).toBe('')
  })

  it('keeps the last snapshot visible when a new search fails', async () => {
    apiMocks.fetchInvocationRecords.mockImplementation(async (query) => {
      if (query.model === 'next-model') {
        throw new Error('search failed')
      }

      return createListResponse({ snapshotId: 42 })
    })
    apiMocks.fetchInvocationRecordsSummary.mockResolvedValue(createSummaryResponse({ snapshotId: 42, newRecordsCount: 5 }))
    apiMocks.fetchInvocationRecordsNewCount.mockResolvedValue(createNewCountResponse({ snapshotId: 42, newRecordsCount: 0 }))

    render(<Probe />)
    await flushAsync()

    expect(text('snapshot')).toBe('42')
    expect(text('summary-snapshot')).toBe('42')

    click('draft-model')
    click('search')
    await flushAsync()

    expect(text('snapshot')).toBe('42')
    expect(text('model')).toBe('baseline-model')
    expect(text('summary-snapshot')).toBe('42')
    expect(text('new-count')).toBe('0')
    expect(text('records-error')).toContain('search failed')
    expect(text('summary-error')).toBe('')
  })

  it('clears the old summary once a new list snapshot lands before summary resolves', async () => {
    let resolveSummary: ((value: InvocationRecordsSummaryResponse) => void) | null = null
    const nextSummaryPromise = new Promise<InvocationRecordsSummaryResponse>((resolve) => {
      resolveSummary = resolve
    })

    apiMocks.fetchInvocationRecords.mockImplementation(async (query) => {
      if (query.model === 'next-model') {
        return createListResponse({
          snapshotId: 84,
          page: 1,
          pageSize: query.pageSize ?? 20,
          total: 1,
          records: [
            {
              id: 84,
              invokeId: 'invoke-next',
              occurredAt: '2026-03-10T02:00:00Z',
              createdAt: '2026-03-10T02:00:00Z',
              model: 'next-model',
              status: 'success',
            },
          ],
        })
      }

      return createListResponse({ snapshotId: 42 })
    })
    apiMocks.fetchInvocationRecordsSummary.mockImplementation(async (query) => {
      if (query.snapshotId === 84) {
        return nextSummaryPromise
      }
      return createSummaryResponse({ snapshotId: 42, newRecordsCount: 0 })
    })
    apiMocks.fetchInvocationRecordsNewCount.mockResolvedValue(createNewCountResponse({ snapshotId: 42, newRecordsCount: 0 }))

    render(<Probe />)
    await flushAsync()

    expect(text('summary-snapshot')).toBe('42')

    click('draft-model')
    click('search')
    await flushAsync()

    expect(text('snapshot')).toBe('84')
    expect(text('summary-snapshot')).toBe('0')
    expect(text('summary-loading')).toBe('yes')

    if (!resolveSummary) {
      throw new Error('next summary resolver missing')
    }
    const resolveSummaryFn = resolveSummary as (value: InvocationRecordsSummaryResponse) => void
    resolveSummaryFn(createSummaryResponse({ snapshotId: 84, newRecordsCount: 0 }))
    await flushAsync()

    expect(text('summary-snapshot')).toBe('84')
    expect(text('summary-loading')).toBe('no')
  })

  it('keeps the last summary visible when new-count polling fails', async () => {
    vi.useFakeTimers()

    apiMocks.fetchInvocationRecords.mockResolvedValue(createListResponse({ snapshotId: 42 }))
    apiMocks.fetchInvocationRecordsSummary.mockResolvedValue(createSummaryResponse({ snapshotId: 42, newRecordsCount: 0 }))
    apiMocks.fetchInvocationRecordsNewCount.mockRejectedValue(new Error('poll failed'))

    render(<Probe />)
    await flushAsync()

    expect(text('new-count')).toBe('0')
    expect(text('summary-error')).toBe('')

    await act(async () => {
      await vi.advanceTimersByTimeAsync(RECORDS_NEW_COUNT_POLL_INTERVAL_MS)
    })
    await flushAsync()

    expect(text('new-count')).toBe('0')
    expect(text('summary-error')).toBe('')
    expect(apiMocks.fetchInvocationRecordsSummary).toHaveBeenCalledTimes(1)
    expect(apiMocks.fetchInvocationRecordsNewCount).toHaveBeenCalledTimes(1)
  })

  it('keeps the preserved summary alive for lightweight polling when a refreshed summary fails', async () => {
    vi.useFakeTimers()

    apiMocks.fetchInvocationRecords.mockImplementation(async (query) => {
      if (query.snapshotId === undefined) {
        return createListResponse({
          snapshotId: 84,
          total: 1,
          records: [
            {
              id: 84,
              invokeId: 'invoke-refresh',
              occurredAt: '2026-03-10T02:00:00Z',
              createdAt: '2026-03-10T02:00:00Z',
              model: 'baseline-refreshed',
              status: 'success',
            },
          ],
        })
      }

      return createListResponse({ snapshotId: 42 })
    })
    apiMocks.fetchInvocationRecordsSummary
      .mockResolvedValueOnce(createSummaryResponse({ snapshotId: 42, newRecordsCount: 0 }))
      .mockRejectedValueOnce(new Error('summary failed'))
    apiMocks.fetchInvocationRecordsNewCount.mockResolvedValue(createNewCountResponse({ snapshotId: 84, newRecordsCount: 9 }))

    render(<Probe />)
    await flushAsync()

    expect(text('summary-snapshot')).toBe('42')
    expect(text('new-count')).toBe('0')

    click('refresh-applied')
    await flushAsync()

    expect(text('snapshot')).toBe('84')
    expect(text('model')).toBe('baseline-refreshed')
    expect(text('summary-snapshot')).toBe('42')
    expect(text('new-count')).toBe('0')
    expect(text('summary-error')).toContain('summary failed')

    await act(async () => {
      await vi.advanceTimersByTimeAsync(RECORDS_NEW_COUNT_POLL_INTERVAL_MS)
    })
    await flushAsync()

    expect(apiMocks.fetchInvocationRecordsNewCount).toHaveBeenCalledTimes(1)
    expect(text('summary-snapshot')).toBe('42')
    expect(text('new-count')).toBe('9')
  })

  it('keeps the previous page size when a page-size request fails before search', async () => {
    apiMocks.fetchInvocationRecords.mockImplementation(async (query) => {
      if (query.model === 'next-model') {
        return createListResponse({
          snapshotId: 84,
          page: 1,
          pageSize: query.pageSize ?? 20,
          total: 1,
          records: [
            {
              id: 84,
              invokeId: 'invoke-next',
              occurredAt: '2026-03-10T02:00:00Z',
              createdAt: '2026-03-10T02:00:00Z',
              model: 'next-model',
              status: 'success',
            },
          ],
        })
      }

      if (query.snapshotId === 42 && query.pageSize === 50) {
        throw new Error('page size failed')
      }

      return createListResponse({ snapshotId: 42 })
    })

    apiMocks.fetchInvocationRecordsSummary.mockImplementation(async (query) =>
      createSummaryResponse({ snapshotId: query.snapshotId ?? 42, newRecordsCount: 0 }),
    )
    apiMocks.fetchInvocationRecordsNewCount.mockResolvedValue(createNewCountResponse({ snapshotId: 42, newRecordsCount: 0 }))

    render(<Probe />)
    await flushAsync()

    click('page-size-50')
    await flushAsync()

    expect(text('page-size')).toBe('20')

    click('draft-model')
    click('search')
    await flushAsync()

    expect(text('snapshot')).toBe('84')
    expect(text('page-size')).toBe('20')

    const searchQuery = apiMocks.fetchInvocationRecords.mock.calls.at(-1)?.[0]
    expect(searchQuery?.model).toBe('next-model')
    expect(searchQuery?.pageSize).toBe(20)
  })

  it('ignores stale page-size responses that race with a newer search snapshot', async () => {
    let resolveSearch: ((value: InvocationRecordsResponse) => void) | null = null
    let resolveOldPageSize: ((value: InvocationRecordsResponse) => void) | null = null
    const searchPromise = new Promise<InvocationRecordsResponse>((resolve) => {
      resolveSearch = resolve
    })
    const oldPageSizePromise = new Promise<InvocationRecordsResponse>((resolve) => {
      resolveOldPageSize = resolve
    })

    apiMocks.fetchInvocationRecords.mockImplementation(async (query) => {
      if (query.model === 'next-model') {
        return searchPromise
      }

      if (query.snapshotId === 42 && query.pageSize === 50) {
        return oldPageSizePromise
      }

      return createListResponse({ snapshotId: 42 })
    })

    apiMocks.fetchInvocationRecordsSummary.mockImplementation(async (query) =>
      createSummaryResponse({ snapshotId: query.snapshotId ?? 42, newRecordsCount: 0 }),
    )
    apiMocks.fetchInvocationRecordsNewCount.mockResolvedValue(createNewCountResponse({ snapshotId: 42, newRecordsCount: 0 }))

    render(<Probe />)
    await flushAsync()

    expect(text('page-size')).toBe('20')

    click('draft-model')
    click('search')
    await flushAsync()
    click('page-size-50')
    await flushAsync()

    if (!resolveSearch || !resolveOldPageSize) {
      throw new Error('missing async resolver')
    }
    const resolveSearchFn = resolveSearch as (value: InvocationRecordsResponse) => void
    const resolveOldPageSizeFn = resolveOldPageSize as (value: InvocationRecordsResponse) => void

    resolveSearchFn(
      createListResponse({
        snapshotId: 84,
        total: 1,
        page: 1,
        pageSize: 20,
        records: [
          {
            id: 84,
            invokeId: 'invoke-next',
            occurredAt: '2026-03-10T02:00:00Z',
            createdAt: '2026-03-10T02:00:00Z',
            model: 'next-model',
            status: 'success',
          },
        ],
      }),
    )
    await flushAsync()

    expect(text('snapshot')).toBe('84')
    expect(text('model')).toBe('next-model')
    expect(text('page-size')).toBe('20')

    resolveOldPageSizeFn(
      createListResponse({
        snapshotId: 42,
        page: 1,
        pageSize: 50,
        records: [
          {
            id: 2,
            invokeId: 'invoke-old',
            occurredAt: '2026-03-10T00:05:00Z',
            createdAt: '2026-03-10T00:05:00Z',
            model: 'baseline-model',
            status: 'failed',
          },
        ],
      }),
    )
    await flushAsync()

    expect(text('snapshot')).toBe('84')
    expect(text('model')).toBe('next-model')
    expect(text('page-size')).toBe('20')
  })

  it('ignores stale page responses once a new search starts', async () => {
    let resolvePageTwo: ((value: InvocationRecordsResponse) => void) | null = null
    const pageTwoPromise = new Promise<InvocationRecordsResponse>((resolve) => {
      resolvePageTwo = resolve
    })

    apiMocks.fetchInvocationRecords.mockImplementation(async (query) => {
      if (query.snapshotId === 42 && query.page === 2) {
        return pageTwoPromise
      }

      if (query.model === 'next-model') {
        return createListResponse({
          snapshotId: 84,
          total: 1,
          page: 1,
          pageSize: query.pageSize ?? 20,
          records: [
            {
              id: 84,
              invokeId: 'invoke-next',
              occurredAt: '2026-03-10T02:00:00Z',
              createdAt: '2026-03-10T02:00:00Z',
              model: 'next-model',
              status: 'success',
            },
          ],
        })
      }

      return createListResponse({ snapshotId: 42 })
    })

    apiMocks.fetchInvocationRecordsSummary.mockImplementation(async (query) =>
      createSummaryResponse({ snapshotId: query.snapshotId ?? 42, newRecordsCount: 4 }),
    )
    apiMocks.fetchInvocationRecordsNewCount.mockResolvedValue(createNewCountResponse({ snapshotId: 42, newRecordsCount: 0 }))

    render(<Probe />)
    await flushAsync()

    click('page-2')
    click('draft-model')
    click('search')
    await flushAsync()

    expect(text('snapshot')).toBe('84')
    expect(text('page')).toBe('1')
    expect(text('model')).toBe('next-model')
    expect(text('new-count')).toBe('0')

    if (!resolvePageTwo) {
      throw new Error('page two resolver missing')
    }
    const resolvePageTwoFn = resolvePageTwo as (value: InvocationRecordsResponse) => void

    resolvePageTwoFn(
      createListResponse({
        snapshotId: 42,
        page: 2,
        pageSize: 20,
        records: [
          {
            id: 2,
            invokeId: 'invoke-2',
            occurredAt: '2026-03-10T00:05:00Z',
            createdAt: '2026-03-10T00:05:00Z',
            model: 'baseline-model',
            status: 'failed',
          },
        ],
      }),
    )
    await flushAsync()

    expect(text('snapshot')).toBe('84')
    expect(text('page')).toBe('1')
    expect(text('model')).toBe('next-model')
  })
})
