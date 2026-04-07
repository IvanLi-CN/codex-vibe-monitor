/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import { HISTORY_CALENDAR_BUCKET, HISTORY_CALENDAR_RANGE, UsageCalendar } from './UsageCalendar'

vi.mock('react-activity-calendar', () => ({
  default: ({
    data,
  }: {
    data?: Array<{ date: string; count: number }>
  }) => <div data-testid="activity-calendar-mock">{data?.length ?? 0}</div>,
}))

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
        'legend.low': 'Low',
        'legend.high': 'High',
        'metric.totalCount': 'Calls',
        'metric.totalCost': 'Cost',
        'metric.totalTokens': 'Tokens',
        'calendar.title': 'History',
        'calendar.metricsToggleAria': 'Switch metric',
        'calendar.valueSeparator': ': ',
        'calendar.timeZoneLabel': 'Timezone',
        'calendar.weekday.sun': 'Sun',
        'calendar.weekday.mon': 'Mon',
        'calendar.weekday.tue': 'Tue',
        'calendar.weekday.wed': 'Wed',
        'calendar.weekday.thu': 'Thu',
        'calendar.weekday.fri': 'Fri',
        'calendar.weekday.sat': 'Sat',
        'calendar.monthLabel': '{{year}}/{{month}}',
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
  rangeStart: '2026-01-01T00:00:00.000Z',
  rangeEnd: '2026-04-01T00:00:00.000Z',
  bucketSeconds: 86400,
  points: [
    {
      bucketStart: '2026-03-20T00:00:00.000Z',
      bucketEnd: '2026-03-21T00:00:00.000Z',
      totalCount: 8,
      successCount: 8,
      failureCount: 0,
      totalTokens: 1500,
      totalCost: 12.5,
    },
  ],
}

describe('UsageCalendar', () => {
  it('renders the standalone surface and metric toggle by default', () => {
    hookMocks.useTimeseries.mockReturnValue({
      data: sampleData,
      isLoading: false,
      error: null,
    })

    render(<UsageCalendar />)

    expect(host?.querySelector('section.surface-panel[data-testid="usage-calendar-card"]')).not.toBeNull()
    expect(hookMocks.useTimeseries).toHaveBeenCalledWith(HISTORY_CALENDAR_RANGE, { bucket: HISTORY_CALENDAR_BUCKET })
    expect(host?.textContent).toContain('History')
    expect(host?.textContent).toContain('Timezone')
    expect(host?.querySelectorAll('button[role="tab"]')).toHaveLength(3)
  })

  it('supports embedded rendering without its own surface or metric toggle', () => {
    hookMocks.useTimeseries.mockReturnValue({
      data: sampleData,
      isLoading: false,
      error: null,
    })

    render(<UsageCalendar metric="totalTokens" showSurface={false} showMetricToggle={false} showMeta={false} />)

    expect(host?.querySelector('section.surface-panel')).toBeNull()
    expect(host?.querySelector('[data-testid="usage-calendar-card"]')?.tagName).toBe('DIV')
    expect(host?.querySelectorAll('button[role="tab"]')).toHaveLength(0)
    expect(hookMocks.useTimeseries).toHaveBeenCalledWith(HISTORY_CALENDAR_RANGE, { bucket: HISTORY_CALENDAR_BUCKET })
    expect(host?.textContent).not.toContain('History')
    expect(host?.textContent).not.toContain('Timezone')
  })
})
