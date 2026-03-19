/** @vitest-environment jsdom */
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
  ErrorReasonPieChart: () => <div data-testid="error-reason-chart" />,
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

function getSelects() {
  const selects = host?.querySelectorAll('select')
  if (!selects || selects.length < 3) {
    throw new Error('missing selects')
  }
  return Array.from(selects) as HTMLSelectElement[]
}

describe('StatsPage bucket fallback', () => {
  it('falls back to daily when the backend limits an archived range to daily buckets', async () => {
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

    const [rangeSelect] = getSelects()
    act(() => {
      rangeSelect.value = '1mo'
      rangeSelect.dispatchEvent(new Event('change', { bubbles: true }))
    })
    await flushAsync()

    const [, bucketSelect] = getSelects()
    const bucketValues = Array.from(bucketSelect.options).map((option) => option.value)

    expect(bucketValues).toEqual(['1d'])
    expect(bucketSelect.value).toBe('1d')
    expect(calls).toContainEqual({ range: '1mo', bucket: '6h' })
    expect(calls).toContainEqual({ range: '1mo', bucket: '1d' })
  })
})
