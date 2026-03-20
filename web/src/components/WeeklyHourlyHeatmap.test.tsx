/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { WeeklyHourlyHeatmap } from './WeeklyHourlyHeatmap'

const hookMocks = vi.hoisted(() => ({
  useTimeseries: vi.fn(),
}))

vi.mock('../hooks/useTimeseries', () => ({
  useTimeseries: hookMocks.useTimeseries,
}))

vi.mock('../theme', () => ({
  useTheme: () => ({ themeMode: 'light' }),
}))

vi.mock('../i18n', () => ({
  useTranslation: () => ({
    locale: 'en',
    t: (key: string) => {
      const map: Record<string, string> = {
        'heatmap.title': 'Last 7 days heatmap',
        'heatmap.metricsToggleAria': 'Switch metric',
        'heatmap.noData': 'No data',
        'metric.totalCount': 'Count',
        'metric.totalCost': 'Cost',
        'metric.totalTokens': 'Tokens',
        'unit.calls': 'calls',
      }
      return map[key] ?? key
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

const sampleData = {
  points: [
    {
      bucketStart: '2026-03-20T14:00:00.000Z',
      bucketEnd: '2026-03-20T15:00:00.000Z',
      totalCount: 8,
      successCount: 8,
      failureCount: 0,
      totalTokens: 1500,
      totalCost: 12.5,
    },
  ],
}

describe('WeeklyHourlyHeatmap', () => {
  it('keeps the standalone card shell and header by default', () => {
    hookMocks.useTimeseries.mockReturnValue({
      data: sampleData,
      isLoading: false,
      error: null,
    })

    render(<WeeklyHourlyHeatmap />)

    expect(host?.querySelector('section.surface-panel[data-testid="weekly-hourly-heatmap"]')).not.toBeNull()
    expect(host?.textContent).toContain('Last 7 days heatmap')
    expect(host?.querySelectorAll('button[role="tab"]')).toHaveLength(3)
  })

  it('supports embedded controlled rendering without the outer panel shell', () => {
    hookMocks.useTimeseries.mockReturnValue({
      data: sampleData,
      isLoading: false,
      error: null,
    })

    render(
      <WeeklyHourlyHeatmap metric="totalCost" showHeader={false} showSurface={false} />,
    )

    expect(host?.querySelector('section.surface-panel')).toBeNull()
    expect(host?.querySelectorAll('button[role="tab"]')).toHaveLength(0)
    expect(host?.querySelector('[data-testid="weekly-hourly-heatmap"]')?.tagName).toBe('DIV')
    expect(host?.querySelector('[aria-label*="$12.50"]')).not.toBeNull()
  })
})
