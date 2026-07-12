/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { MemoryRouter } from 'react-router-dom'
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
    t: (key: string, values?: Record<string, string | number>) => {
      const count = values?.count ?? ''
      switch (key) {
        case 'records.summary.notice.newData':
          return `有 ${count} 条新数据`
        case 'records.summary.notice.refreshAction':
          return '加载新数据'
        case 'records.summary.notice.newDataAria':
          return `有 ${count} 条新数据，点击后会并入当前快照。`
        case 'records.summary.notice.refreshAria':
          return `加载这 ${count} 条新数据并刷新当前快照。`
        case 'records.summary.notice.refreshingAria':
          return `正在加载这 ${count} 条新数据并刷新当前快照。`
        default:
          return key
      }
    },
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
  if (typeof globalThis.PointerEvent === 'undefined') {
    Object.defineProperty(window, 'PointerEvent', {
      configurable: true,
      writable: true,
      value: MouseEvent,
    })
    Object.defineProperty(globalThis, 'PointerEvent', {
      configurable: true,
      writable: true,
      value: MouseEvent,
    })
  }
  if (typeof HTMLElement.prototype.hasPointerCapture !== 'function') {
    Object.defineProperty(HTMLElement.prototype, 'hasPointerCapture', {
      configurable: true,
      writable: true,
      value: () => false,
    })
  }
  if (typeof HTMLElement.prototype.setPointerCapture !== 'function') {
    Object.defineProperty(HTMLElement.prototype, 'setPointerCapture', {
      configurable: true,
      writable: true,
      value: () => undefined,
    })
  }
  if (typeof HTMLElement.prototype.releasePointerCapture !== 'function') {
    Object.defineProperty(HTMLElement.prototype, 'releasePointerCapture', {
      configurable: true,
      writable: true,
      value: () => undefined,
    })
  }
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

function render(ui: React.ReactNode, initialEntries?: string[]) {
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(<MemoryRouter initialEntries={initialEntries}>{ui}</MemoryRouter>)
  })
}

function rerender(ui: React.ReactNode) {
  act(() => {
    root?.render(<MemoryRouter>{ui}</MemoryRouter>)
  })
}

function getSelectTrigger(label: string) {
  const trigger = Array.from(document.body.querySelectorAll('button[role="combobox"]')).find(
    (candidate) =>
      candidate instanceof HTMLButtonElement &&
      candidate.getAttribute('aria-label') === label,
  )
  if (!(trigger instanceof HTMLButtonElement)) {
    throw new Error(`missing select trigger: ${label}`)
  }
  return trigger
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
    endpoint: emptyBucket,
    failureKind: emptyBucket,
    promptCacheKey: emptyBucket,
    requesterIp: emptyBucket,
    ...overrides,
  }
}

function mockInvocationRecords(overrides: Partial<ReturnType<typeof hookMocks.useInvocationRecords>> = {}) {
  const draft = { ...createDefaultInvocationRecordsDraft(), ...createDefaultCustomRange(), model: 'alp' }
  hookMocks.useInvocationRecords.mockReturnValue({
    draft,
    appliedDraft: draft,
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
    applyDraft: vi.fn(() => Promise.resolve()),
    setFocus: vi.fn(),
    search: vi.fn(),
    setPage: vi.fn(),
    setPageSize: vi.fn(),
    setSort: vi.fn(),
    ...overrides,
  })
}

function openFilters() {
  const button = host?.querySelector('[data-testid="records-open-filters"]')
  if (!(button instanceof HTMLButtonElement)) {
    throw new Error('missing open filters button')
  }
  act(() => {
    button.click()
  })
  const drawer = document.body.querySelector('[data-testid="records-filters-drawer"]')
  if (!(drawer instanceof HTMLElement)) {
    throw new Error('missing filters drawer')
  }
  return drawer
}

function getNewDataButton() {
  const button = host?.querySelector('[data-testid="records-new-data-button"]')
  if (!(button instanceof HTMLButtonElement)) {
    throw new Error('missing new data button')
  }
  return button
}

function getNewDataLabel(testId: 'records-new-data-label-idle' | 'records-new-data-label-action') {
  const label = host?.querySelector(`[data-testid="${testId}"]`)
  if (!(label instanceof HTMLSpanElement)) {
    throw new Error(`missing new data label: ${testId}`)
  }
  return label
}

describe('RecordsPage suggestions', () => {
  it('does not render the removed proxy filter control', () => {
    mockInvocationRecords()

    render(<RecordsPage />)
    openFilters()

    expect(document.body.querySelector('#records-filter-proxy')).toBeNull()
    expect(document.body.textContent ?? '').not.toContain('records.filters.proxy')
  })

  it('disables browser native autocomplete for filter controls', () => {
    mockInvocationRecords()

    render(<RecordsPage />)
    openFilters()

    const modelInput = document.body.querySelector('#records-filter-model')
    const rangePresetSelect = document.body.querySelector('select[name="rangePreset"]')
    const rangePresetTrigger = getSelectTrigger('records.filters.rangePreset')
    const keywordInput = document.body.querySelector('input[name="keyword"]')
    const minTotalTokensInput = document.body.querySelector('input[name="minTotalTokens"]')

    if (!(modelInput instanceof HTMLInputElement)) {
      throw new Error('missing model input')
    }
    if (!(keywordInput instanceof HTMLInputElement)) {
      throw new Error('missing keyword input')
    }
    if (!(minTotalTokensInput instanceof HTMLInputElement)) {
      throw new Error('missing min total tokens input')
    }

    expect(modelInput.autocomplete).toBe('off')
    expect(modelInput.getAttribute('autocorrect')).toBe('off')
    expect(modelInput.getAttribute('autocapitalize')).toBe('none')
    expect(modelInput.getAttribute('spellcheck')).toBe('false')
    expect(rangePresetSelect).toBeNull()
    expect(rangePresetTrigger.getAttribute('aria-label')).toBe('records.filters.rangePreset')
    expect(keywordInput.autocomplete).toBe('off')
    expect(keywordInput.getAttribute('autocorrect')).toBe('off')
    expect(keywordInput.getAttribute('autocapitalize')).toBe('none')
    expect(keywordInput.getAttribute('spellcheck')).toBe('false')
    expect(minTotalTokensInput.autocomplete).toBe('off')
  })

  it('loads suggestions lazily after a combobox opens', async () => {
    vi.useFakeTimers()
    apiMocks.fetchInvocationSuggestions.mockResolvedValue(createSuggestions())
    mockInvocationRecords()

    render(<RecordsPage />)
    openFilters()

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300)
    })
    await flushAsync()
    expect(apiMocks.fetchInvocationSuggestions).not.toHaveBeenCalled()

    const input = document.body.querySelector('#records-filter-model')
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
    openFilters()

    const input = document.body.querySelector('#records-filter-model')
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

    expect(document.body.textContent).not.toContain('alp-stale')

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300)
    })
    await flushAsync()

    expect(apiMocks.fetchInvocationSuggestions).toHaveBeenCalledTimes(2)
    expect(document.body.textContent).toContain('alp-fresh')
    expect(document.body.textContent).not.toContain('alp-stale')
  })

  it('keeps filter suggestions inside the drawer without changing page surface layering', async () => {
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

    const summaryPanel = host?.querySelector('[data-testid="records-summary-panel"]')
    if (!(summaryPanel instanceof HTMLElement)) {
      throw new Error('missing panel anchors')
    }

    expect(document.body.querySelector('[data-testid="records-filters-drawer"]')).toBeNull()
    const filtersDrawer = openFilters()
    expect(filtersDrawer.dataset.suggestionsOpen).toBe('false')
    expect(filtersDrawer.closest('[role="dialog"]')).not.toBeNull()

    const input = document.body.querySelector('#records-filter-prompt-cache-key')
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

    expect(summaryPanel.className).toBe('surface-panel')

    act(() => {
      input.blur()
    })
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0)
    })
    await flushAsync()

    expect(host?.querySelector('[data-testid="records-filters-panel"]')?.className).toBe('surface-panel')
  })
})

describe('RecordsPage filter drawer', () => {
  it('keeps draft controls out of the page flow and summarizes only applied filters', () => {
    const baseDraft = { ...createDefaultInvocationRecordsDraft(), ...createDefaultCustomRange() }
    const appliedDraft = { ...baseDraft, model: 'gpt-5.3-codex', status: 'failed' }
    const applyDraft = vi.fn(() => Promise.resolve())
    mockInvocationRecords({
      draft: { ...appliedDraft, endpoint: '/v1/responses' },
      appliedDraft,
      applyDraft,
    })

    render(<RecordsPage />)

    expect(host?.querySelector('#records-filter-model')).toBeNull()
    const activeFilters = host?.querySelector('[data-testid="records-active-filters"]')
    expect(activeFilters?.textContent).toContain('records.filters.model: gpt-5.3-codex')
    expect(activeFilters?.textContent).toContain('records.filters.status: records.filters.status.failed')
    expect(activeFilters?.textContent).not.toContain('/v1/responses')

    const modelChip = host?.querySelector('[data-testid="records-active-filter-model"]')
    if (!(modelChip instanceof HTMLButtonElement)) {
      throw new Error('missing applied model filter chip')
    }
    act(() => {
      modelChip.click()
    })

    expect(applyDraft).toHaveBeenCalledWith(expect.objectContaining({ model: '', status: 'failed' }))
  })

  it('opens filters in the shared drawer instead of leaving form controls in the page body', () => {
    mockInvocationRecords()

    render(<RecordsPage />)

    expect(host?.querySelector('[data-testid="records-filters-drawer"]')).toBeNull()
    const drawer = openFilters()
    expect(drawer.closest('[role="dialog"]')).not.toBeNull()
    expect(document.body.querySelector('#records-filter-model')).toBeInstanceOf(HTMLInputElement)
  })
})

describe('RecordsPage new data action', () => {
  it('renders the new data button and switches to the refresh call-to-action on focus', async () => {
    mockInvocationRecords({
      summary: { ...createSummary(), snapshotId: 84, newRecordsCount: 9 },
    })

    render(<RecordsPage />)

    const button = getNewDataButton()
    const idleLabel = getNewDataLabel('records-new-data-label-idle')
    const actionLabel = getNewDataLabel('records-new-data-label-action')

    expect(button.dataset.state).toBe('idle')
    expect(button.dataset.icon).toBe('help')
    expect(idleLabel.textContent).toBe('有 9 条新数据')
    expect(idleLabel.className).toContain('opacity-100')
    expect(actionLabel.className).toContain('opacity-0')
    expect(button.className).toContain('border-warning/35')
    expect(button.getAttribute('aria-label')).toBe('有 9 条新数据，点击后会并入当前快照。')

    act(() => {
      button.focus()
    })
    await flushAsync()

    expect(button.dataset.state).toBe('interactive')
    expect(button.dataset.icon).toBe('help')
    expect(idleLabel.className).toContain('opacity-0')
    expect(actionLabel.textContent).toBe('加载新数据')
    expect(actionLabel.className).toContain('opacity-100')
    expect(button.className).toContain('border-primary/35')
    expect(button.getAttribute('aria-label')).toBe('加载这 9 条新数据并刷新当前快照。')

    act(() => {
      button.blur()
    })
    await flushAsync()

    expect(button.dataset.state).toBe('idle')
    expect(idleLabel.className).toContain('opacity-100')
    expect(actionLabel.className).toContain('opacity-0')
  })

  it('triggers search once and shows a spinning refresh state while the refresh is pending', async () => {
    vi.useFakeTimers()
    let resolveSearch: (() => void) | null = null
    const search = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          resolveSearch = resolve
        }),
    )

    mockInvocationRecords({
      summary: { ...createSummary(), snapshotId: 84, newRecordsCount: 9 },
      search,
    })

    render(<RecordsPage />)

    const button = getNewDataButton()
    const idleLabel = getNewDataLabel('records-new-data-label-idle')
    const actionLabel = getNewDataLabel('records-new-data-label-action')

    act(() => {
      button.click()
    })
    await flushAsync()

    expect(search).toHaveBeenCalledTimes(1)
    expect(search).toHaveBeenCalledWith({ source: 'applied', preserveSummary: true })
    expect(button.disabled).toBe(true)
    expect(button.dataset.state).toBe('loading')
    expect(button.dataset.icon).toBe('refresh')
    expect(button.className).toContain('border-primary/35')
    expect(idleLabel.className).toContain('opacity-0')
    expect(actionLabel.className).toContain('opacity-100')
    expect(actionLabel.textContent).toBe('加载新数据')
    expect(button.getAttribute('aria-label')).toBe('正在加载这 9 条新数据并刷新当前快照。')

    act(() => {
      button.click()
    })
    await flushAsync()

    expect(search).toHaveBeenCalledTimes(1)

    act(() => {
      resolveSearch?.()
    })
    await flushAsync()

    await act(async () => {
      await vi.advanceTimersByTimeAsync(600)
    })
    await flushAsync()

    expect(button.disabled).toBe(false)
    expect(button.dataset.state).toBe('idle')
    expect(button.dataset.icon).toBe('help')
  })

  it('keeps the loading state visible briefly even when refresh resolves immediately', async () => {
    vi.useFakeTimers()
    const search = vi.fn(() => Promise.resolve())

    mockInvocationRecords({
      summary: { ...createSummary(), snapshotId: 84, newRecordsCount: 9 },
      search,
    })

    render(<RecordsPage />)

    const button = getNewDataButton()

    act(() => {
      button.click()
    })
    await flushAsync()

    expect(search).toHaveBeenCalledTimes(1)
    expect(button.dataset.state).toBe('loading')
    expect(button.disabled).toBe(true)

    await act(async () => {
      await vi.advanceTimersByTimeAsync(599)
    })
    await flushAsync()

    expect(button.dataset.state).toBe('loading')
    expect(button.disabled).toBe(true)

    await act(async () => {
      await vi.advanceTimersByTimeAsync(1)
    })
    await flushAsync()

    expect(button.dataset.state).toBe('idle')
    expect(button.disabled).toBe(false)
  })

  it('keeps the new-data button mounted during the minimum loading delay even after the count resets', async () => {
    vi.useFakeTimers()

    const search = vi.fn(() => Promise.resolve())
    let state = {
      draft: { ...createDefaultInvocationRecordsDraft(), ...createDefaultCustomRange(), model: 'alp' },
      focus: 'token',
      page: 1,
      pageSize: 20,
      sortBy: 'occurredAt',
      sortOrder: 'desc',
      records: { snapshotId: 42, total: 0, page: 1, pageSize: 20, records: [] },
      summary: { ...createSummary(), snapshotId: 42, newRecordsCount: 9 },
      recordsError: null,
      summaryError: null,
      isSearching: false,
      isRecordsLoading: false,
      isSummaryLoading: false,
      updateDraft: vi.fn(),
      resetDraft: vi.fn(),
      setFocus: vi.fn(),
      search,
      setPage: vi.fn(),
      setPageSize: vi.fn(),
      setSort: vi.fn(),
    }

    hookMocks.useInvocationRecords.mockImplementation(() => state)

    render(<RecordsPage />)

    act(() => {
      getNewDataButton().click()
    })
    await flushAsync()

    state = {
      ...state,
      records: { ...state.records, snapshotId: 84 },
      summary: { ...createSummary(), snapshotId: 84, newRecordsCount: 0 },
    }
    rerender(<RecordsPage />)
    await flushAsync()

    expect(getNewDataButton().dataset.state).toBe('loading')
    expect(getNewDataButton().textContent).toContain('加载新数据')

    await act(async () => {
      await vi.advanceTimersByTimeAsync(600)
    })
    await flushAsync()

    rerender(<RecordsPage />)
    await flushAsync()

    expect(host?.querySelector('[data-testid="records-new-data-button"]')).toBeNull()
  })

  it('hides stale summary metrics while a refreshed snapshot summary is still loading', () => {
    mockInvocationRecords({
      records: { snapshotId: 84, total: 0, page: 1, pageSize: 20, records: [] },
      summary: {
        ...createSummary(),
        snapshotId: 42,
        token: {
          ...createSummary().token,
          requestCount: 999,
        },
      },
      isSummaryLoading: true,
    })

    render(<RecordsPage />)

    expect(host?.textContent).toContain('…')
    expect(host?.textContent).not.toContain('999')
  })

  it('hides the new-data CTA after a refreshed list lands if the preserved summary is stale', () => {
    mockInvocationRecords({
      records: { snapshotId: 84, total: 0, page: 1, pageSize: 20, records: [] },
      summary: { ...createSummary(), snapshotId: 42, newRecordsCount: 9 },
      summaryError: 'summary failed',
      isSummaryLoading: false,
    })

    render(<RecordsPage />)

    expect(host?.querySelector('[data-testid="records-new-data-button"]')).toBeNull()
  })

  it('hides the new-data CTA during a normal search even if the old summary still reports pending records', () => {
    mockInvocationRecords({
      summary: { ...createSummary(), snapshotId: 42, newRecordsCount: 9 },
      isSearching: true,
    })

    render(<RecordsPage />)

    expect(host?.querySelector('[data-testid="records-new-data-button"]')).toBeNull()
  })

  it('hides the new data button when there is no pending new data', () => {
    mockInvocationRecords({
      summary: { ...createSummary(), snapshotId: 42, newRecordsCount: 0 },
    })

    render(<RecordsPage />)

    expect(host?.querySelector('[data-testid="records-new-data-button"]')).toBeNull()
  })

  it('updates the request ID draft filter from the new input', () => {
    const updateDraft = vi.fn()
    mockInvocationRecords({
      draft: {
        ...createDefaultInvocationRecordsDraft(),
        ...createDefaultCustomRange(),
        requestId: '',
      },
      updateDraft,
    })

    render(<RecordsPage />)
    openFilters()

    const input = document.body.querySelector('input[name="requestId"]')
    if (!(input instanceof HTMLInputElement)) {
      throw new Error('missing request ID input')
    }

    act(() => {
      const valueSetter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
      valueSetter?.call(input, 'invoke-xyz')
      input.dispatchEvent(new Event('input', { bubbles: true }))
      input.dispatchEvent(new Event('change', { bubbles: true }))
    })

    expect(updateDraft).toHaveBeenCalledWith('requestId', 'invoke-xyz')
  })

  it('resets stale filters before searching a request ID deep link', async () => {
    const resetDraft = vi.fn()
    const updateDraft = vi.fn()
    const search = vi.fn()
    mockInvocationRecords({ resetDraft, updateDraft, search })

    vi.useFakeTimers()
    render(<RecordsPage />, ['/records?requestId=invoke-target&rangePreset=7d'])
    await act(async () => {
      await vi.runAllTimersAsync()
    })

    expect(resetDraft).toHaveBeenCalledTimes(1)
    expect(updateDraft).toHaveBeenNthCalledWith(1, 'requestId', 'invoke-target')
    expect(updateDraft).toHaveBeenNthCalledWith(2, 'rangePreset', '7d')
    expect(search).toHaveBeenCalledTimes(1)
  })
})
