/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import RecordsPage from './Records'
import { createDefaultCustomRange, createDefaultInvocationRecordsDraft } from '../lib/invocationRecords'
import type { InvocationRecordsSummaryResponse, InvocationSuggestionsResponse } from '../lib/api'

const hookMocks = vi.hoisted(() => ({
  useInvocationRecords: vi.fn(),
}))

const apiMocks = vi.hoisted(() => ({
  fetchInvocationSuggestions: vi.fn<() => Promise<InvocationSuggestionsResponse>>(),
}))

vi.mock('../hooks/useInvocationRecords', () => ({
  useInvocationRecords: hookMocks.useInvocationRecords,
}))

vi.mock('../lib/api', async () => {
  const actual = await vi.importActual<typeof import('../lib/api')>('../lib/api')
  return {
    ...actual,
    fetchInvocationSuggestions: apiMocks.fetchInvocationSuggestions,
  }
})

vi.mock('../i18n', () => ({
  useTranslation: () => ({
    locale: 'zh',
    t: (key: string) => key,
  }),
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

async function flushAsync() {
  await act(async () => {
    await Promise.resolve()
    await Promise.resolve()
  })
}

function createSummary(): InvocationRecordsSummaryResponse {
  return {
    snapshotId: 42,
    newRecordsCount: 0,
    totalCount: 0,
    successCount: 0,
    failureCount: 0,
    totalCost: 0,
    totalTokens: 0,
    token: {
      requestCount: 0,
      totalTokens: 0,
      avgTokensPerRequest: 0,
      cacheInputTokens: 0,
      totalCost: 0,
    },
    network: {
      avgTtfbMs: 0,
      p95TtfbMs: 0,
      avgTotalMs: 0,
      p95TotalMs: 0,
    },
    exception: {
      failureCount: 0,
      serviceFailureCount: 0,
      clientFailureCount: 0,
      clientAbortCount: 0,
      actionableFailureCount: 0,
    },
  }
}

function createSuggestions(overrides: Partial<InvocationSuggestionsResponse> = {}): InvocationSuggestionsResponse {
  const emptyBucket = { items: [], hasMore: false }
  return {
    model: emptyBucket,
    proxy: emptyBucket,
    endpoint: emptyBucket,
    failureKind: emptyBucket,
    promptCacheKey: emptyBucket,
    requesterIp: emptyBucket,
    ...overrides,
  }
}

function mockInvocationRecords(overrides: Partial<ReturnType<typeof hookMocks.useInvocationRecords>> = {}) {
  hookMocks.useInvocationRecords.mockReturnValue({
    draft: { ...createDefaultInvocationRecordsDraft(), ...createDefaultCustomRange(), model: 'alp' },
    focus: 'token',
    page: 1,
    pageSize: 20,
    sortBy: 'occurredAt',
    sortOrder: 'desc',
    records: { snapshotId: 84, total: 0, page: 1, pageSize: 20, records: [] },
    summary: { ...createSummary(), snapshotId: 42 },
    recordsError: null,
    summaryError: null,
    isSearching: false,
    isRecordsLoading: false,
    isSummaryLoading: false,
    updateDraft: vi.fn(),
    resetDraft: vi.fn(),
    setFocus: vi.fn(),
    search: vi.fn(),
    setPage: vi.fn(),
    setPageSize: vi.fn(),
    setSort: vi.fn(),
    ...overrides,
  })
}

describe('RecordsPage suggestions', () => {
  it('loads suggestions lazily after a combobox opens', async () => {
    vi.useFakeTimers()
    apiMocks.fetchInvocationSuggestions.mockResolvedValue(createSuggestions())
    mockInvocationRecords()

    render(<RecordsPage />)

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300)
    })
    await flushAsync()
    expect(apiMocks.fetchInvocationSuggestions).not.toHaveBeenCalled()

    const input = host?.querySelector('#records-filter-model')
    if (!(input instanceof HTMLInputElement)) {
      throw new Error('missing model input')
    }

    act(() => {
      input.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      input.focus()
    })

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300)
    })
    await flushAsync()

    expect(apiMocks.fetchInvocationSuggestions).toHaveBeenCalledTimes(1)
    expect(apiMocks.fetchInvocationSuggestions).toHaveBeenCalledWith(expect.objectContaining({ snapshotId: 84, suggestField: 'model', suggestQuery: 'alp' }))
  })

  it('ignores stale suggestions after the combobox closes', async () => {
    vi.useFakeTimers()

    let resolveFirst: ((value: InvocationSuggestionsResponse) => void) | null = null
    apiMocks.fetchInvocationSuggestions
      .mockImplementationOnce(
        () =>
          new Promise<InvocationSuggestionsResponse>((resolve) => {
            resolveFirst = resolve
          }),
      )
      .mockResolvedValueOnce(
        createSuggestions({
          model: {
            items: [{ value: 'alp-fresh', count: 3 }],
            hasMore: false,
          },
        }),
      )

    mockInvocationRecords()

    render(<RecordsPage />)

    const input = host?.querySelector('#records-filter-model')
    if (!(input instanceof HTMLInputElement)) {
      throw new Error('missing model input')
    }

    act(() => {
      input.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      input.focus()
    })

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300)
    })
    await flushAsync()
    expect(apiMocks.fetchInvocationSuggestions).toHaveBeenCalledTimes(1)

    act(() => {
      input.blur()
    })
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0)
    })
    await flushAsync()

    act(() => {
      resolveFirst?.(
        createSuggestions({
          model: {
            items: [{ value: 'alp-stale', count: 1 }],
            hasMore: false,
          },
        }),
      )
    })
    await flushAsync()

    act(() => {
      input.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      input.focus()
    })
    await flushAsync()

    expect(host?.textContent).not.toContain('alp-stale')

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300)
    })
    await flushAsync()

    expect(apiMocks.fetchInvocationSuggestions).toHaveBeenCalledTimes(2)
    expect(host?.textContent).toContain('alp-fresh')
    expect(host?.textContent).not.toContain('alp-stale')
  })

  it('raises the filters panel while a suggestion dropdown is open', async () => {
    vi.useFakeTimers()
    apiMocks.fetchInvocationSuggestions.mockResolvedValue(
      createSuggestions({
        promptCacheKey: {
          items: [{ value: 'pck-open-1', count: 2 }],
          hasMore: false,
        },
      }),
    )
    mockInvocationRecords({
      draft: { ...createDefaultInvocationRecordsDraft(), ...createDefaultCustomRange(), promptCacheKey: 'pck' },
    })

    render(<RecordsPage />)

    const filtersPanel = host?.querySelector('[data-testid="records-filters-panel"]')
    const summaryPanel = host?.querySelector('[data-testid="records-summary-panel"]')
    if (!(filtersPanel instanceof HTMLElement) || !(summaryPanel instanceof HTMLElement)) {
      throw new Error('missing panel anchors')
    }

    expect(filtersPanel.dataset.suggestionsOpen).toBe('false')
    expect(filtersPanel.className).not.toContain('z-10')

    const input = host?.querySelector('#records-filter-prompt-cache-key')
    if (!(input instanceof HTMLInputElement)) {
      throw new Error('missing prompt cache key input')
    }

    act(() => {
      input.dispatchEvent(new MouseEvent('click', { bubbles: true }))
      input.focus()
    })

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300)
    })
    await flushAsync()

    expect(filtersPanel.dataset.suggestionsOpen).toBe('true')
    expect(filtersPanel.className).toContain('relative')
    expect(filtersPanel.className).toContain('z-10')
    expect(filtersPanel.className).toContain('overflow-visible')
    expect(summaryPanel.className).toBe('surface-panel')

    act(() => {
      input.blur()
    })
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0)
    })
    await flushAsync()

    expect(filtersPanel.dataset.suggestionsOpen).toBe('false')
    expect(filtersPanel.className).not.toContain('z-10')
  })
})
