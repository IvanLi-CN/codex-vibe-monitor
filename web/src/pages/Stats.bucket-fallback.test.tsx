/** @vitest-environment jsdom */
import * as React from 'react'
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type { FailureSummaryResponse, TimeseriesResponse } from '../lib/api'
import StatsPage from './Stats'

const hookMocks = vi.hoisted(() => ({
  useSummary: vi.fn(),
  useTimeseries: vi.fn(),
  useErrorDistribution: vi.fn(),
  useFailureSummary: vi.fn(),
}))

const SelectContext = React.createContext<{
  value?: string
  onValueChange?: (value: string) => void
} | null>(null)

vi.mock('../hooks/useStats', () => ({
  useSummary: hookMocks.useSummary,
}))

vi.mock('../hooks/useTimeseries', () => ({
  useTimeseries: hookMocks.useTimeseries,
}))

vi.mock('../hooks/useErrorDistribution', () => ({
  useErrorDistribution: hookMocks.useErrorDistribution,
}))

vi.mock('../hooks/useFailureSummary', () => ({
  useFailureSummary: hookMocks.useFailureSummary,
}))

vi.mock('../components/ui/select', () => ({
  Select: ({
    value,
    onValueChange,
    children,
  }: {
    value?: string
    onValueChange?: (value: string) => void
    children: React.ReactNode
  }) => (
    <SelectContext.Provider value={{ value, onValueChange }}>
      <div>{children}</div>
    </SelectContext.Provider>
  ),
  SelectTrigger: ({
    children,
    ...props
  }: React.ButtonHTMLAttributes<HTMLButtonElement>) => {
    const ctx = React.useContext(SelectContext)
    return (
      <button type="button" role="combobox" data-value={ctx?.value} {...props}>
        {children}
      </button>
    )
  },
  SelectValue: () => {
    const ctx = React.useContext(SelectContext)
    return <span>{ctx?.value}</span>
  },
  SelectContent: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  SelectItem: ({
    value,
    children,
  }: {
    value: string
    children: React.ReactNode
  }) => {
    const ctx = React.useContext(SelectContext)
    return (
      <button
        type="button"
        data-testid={`select-item-${value}`}
        onClick={() => ctx?.onValueChange?.(value)}
      >
        {children}
      </button>
    )
  },
}))

vi.mock('../i18n', () => ({
  useTranslation: () => ({
    locale: 'zh',
    t: (key: string) => key,
  }),
}))

vi.mock('../components/StatsCards', () => ({
  StatsCards: () => <div data-testid="stats-cards" />,
}))

vi.mock('../components/TimeseriesChart', () => ({
  TimeseriesChart: () => <div data-testid="timeseries-chart" />,
}))

vi.mock('../components/SuccessFailureChart', () => ({
  SuccessFailureChart: () => <div data-testid="success-failure-chart" />,
}))

vi.mock('../components/ErrorReasonPieChart', () => ({
  ErrorReasonPieChart: () => <div data-testid="error-reason-pie-chart" />,
}))

vi.mock('../components/ui/alert', () => ({
  Alert: ({ children }: { children: React.ReactNode }) => <div role="alert">{children}</div>,
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

function createTimeseriesResponse(
  overrides: Partial<TimeseriesResponse> = {},
): TimeseriesResponse {
  return {
    rangeStart: '2026-03-18T00:00:00Z',
    rangeEnd: '2026-03-19T00:00:00Z',
    bucketSeconds: 900,
    effectiveBucket: '15m',
    availableBuckets: ['15m', '30m', '1h', '6h'],
    bucketLimitedToDaily: false,
    points: [],
    ...overrides,
  }
}

function createFailureSummary(): FailureSummaryResponse {
  return {
    rangeStart: '2026-03-18T00:00:00Z',
    rangeEnd: '2026-03-19T00:00:00Z',
    totalFailures: 0,
    serviceFailureCount: 0,
    clientFailureCount: 0,
    clientAbortCount: 0,
    actionableFailureCount: 0,
    actionableFailureRate: 0,
  }
}

describe('StatsPage archived bucket fallback', () => {
  it('re-requests daily buckets after the backend limits an archived range to daily granularity', async () => {
    const calls: Array<{ range: string; bucket?: string }> = []

    hookMocks.useSummary.mockReturnValue({
      summary: null,
      isLoading: false,
      error: null,
    })
    hookMocks.useErrorDistribution.mockReturnValue({
      data: { items: [] },
      isLoading: false,
      error: null,
    })
    hookMocks.useFailureSummary.mockReturnValue({
      data: createFailureSummary(),
      isLoading: false,
      error: null,
    })
    hookMocks.useTimeseries.mockImplementation(
      (range: string, options?: { bucket?: string }) => {
        calls.push({ range, bucket: options?.bucket })
        if (range === '1mo') {
          return {
            data: createTimeseriesResponse({
              bucketSeconds: 86_400,
              effectiveBucket: '1d',
              availableBuckets: ['1d'],
              bucketLimitedToDaily: true,
            }),
            isLoading: false,
            error: null,
          }
        }

        return {
          data: createTimeseriesResponse({
            effectiveBucket: options?.bucket ?? '15m',
          }),
          isLoading: false,
          error: null,
        }
      },
    )

    render(<StatsPage />)

    const rangeItem = host?.querySelector('[data-testid="select-item-1mo"]')
    if (!(rangeItem instanceof HTMLButtonElement)) {
      throw new Error('missing 1mo range item')
    }
    act(() => {
      rangeItem.click()
    })
    await flushAsync()

    const bucketTrigger = host?.querySelector('[data-testid="stats-bucket-select-trigger"]')
    if (!(bucketTrigger instanceof HTMLButtonElement)) {
      throw new Error('missing bucket trigger')
    }

    expect(bucketTrigger.getAttribute('data-value')).toBe('1d')
    expect(calls).toContainEqual({ range: '1mo', bucket: '6h' })
    expect(calls).toContainEqual({ range: '1mo', bucket: '1d' })
  })
})
