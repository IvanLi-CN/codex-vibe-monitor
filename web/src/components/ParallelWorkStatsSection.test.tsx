/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import type { ParallelWorkStatsResponse } from '../lib/api'
import { ParallelWorkStatsSection } from './ParallelWorkStatsSection'

vi.mock('./ui/alert', () => ({
  Alert: ({ children }: { children: React.ReactNode }) => <div role="alert">{children}</div>,
}))

vi.mock('../i18n', () => ({
  useTranslation: () => ({
    locale: 'en',
    t: (key: string, values?: Record<string, string | number>) => {
      const map: Record<string, string> = {
        'stats.parallelWork.title': 'Parallel work',
        'stats.parallelWork.description': 'Track active prompt-cache conversations.',
        'stats.parallelWork.loading': 'Loading parallel-work buckets…',
        'stats.parallelWork.empty': 'No complete buckets yet.',
        'stats.parallelWork.windowToggleAria': 'Select parallel-work window',
        'stats.parallelWork.chartAria': `${values?.title ?? ''} trend`,
        'stats.parallelWork.samples': `${values?.complete ?? 0} complete buckets · ${values?.active ?? 0} active buckets`,
        'stats.parallelWork.rangeSummary': `Range: ${values?.start ?? ''} → ${values?.end ?? ''}`,
        'stats.parallelWork.metrics.min': 'Min',
        'stats.parallelWork.metrics.max': 'Max',
        'stats.parallelWork.metrics.avg': 'Avg',
        'stats.parallelWork.windows.minute7d.title': 'Last 7 days · by minute',
        'stats.parallelWork.windows.minute7d.toggleLabel': '7d · minute',
        'stats.parallelWork.windows.minute7d.description': 'Minute buckets',
        'stats.parallelWork.windows.hour30d.title': 'Last 30 days · by hour',
        'stats.parallelWork.windows.hour30d.toggleLabel': '30d · hour',
        'stats.parallelWork.windows.hour30d.description': 'Hour buckets',
        'stats.parallelWork.windows.dayAll.title': 'All history · by day',
        'stats.parallelWork.windows.dayAll.toggleLabel': 'All · day',
        'stats.parallelWork.windows.dayAll.description': 'Day buckets',
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

const populatedStats: ParallelWorkStatsResponse = {
  minute7d: {
    rangeStart: '2026-03-01T00:00:00Z',
    rangeEnd: '2026-03-08T00:00:00Z',
    bucketSeconds: 60,
    completeBucketCount: 10080,
    activeBucketCount: 4132,
    minCount: 0,
    maxCount: 18,
    avgCount: 4.67,
    points: [
      { bucketStart: '2026-03-07T10:00:00Z', bucketEnd: '2026-03-07T10:01:00Z', parallelCount: 1 },
      { bucketStart: '2026-03-07T10:01:00Z', bucketEnd: '2026-03-07T10:02:00Z', parallelCount: 4 },
      { bucketStart: '2026-03-07T10:02:00Z', bucketEnd: '2026-03-07T10:03:00Z', parallelCount: 6 },
    ],
  },
  hour30d: {
    rangeStart: '2026-02-06T00:00:00Z',
    rangeEnd: '2026-03-08T00:00:00Z',
    bucketSeconds: 3600,
    completeBucketCount: 720,
    activeBucketCount: 321,
    minCount: 0,
    maxCount: 9,
    avgCount: 2.13,
    points: [
      { bucketStart: '2026-03-07T00:00:00Z', bucketEnd: '2026-03-07T01:00:00Z', parallelCount: 0 },
      { bucketStart: '2026-03-07T01:00:00Z', bucketEnd: '2026-03-07T02:00:00Z', parallelCount: 2 },
      { bucketStart: '2026-03-07T02:00:00Z', bucketEnd: '2026-03-07T03:00:00Z', parallelCount: 3 },
    ],
  },
  dayAll: {
    rangeStart: '2026-01-01T00:00:00Z',
    rangeEnd: '2026-03-08T00:00:00Z',
    bucketSeconds: 86400,
    completeBucketCount: 67,
    activeBucketCount: 54,
    minCount: 0,
    maxCount: 6,
    avgCount: 2.04,
    points: [
      { bucketStart: '2026-03-05T00:00:00Z', bucketEnd: '2026-03-06T00:00:00Z', parallelCount: 2 },
      { bucketStart: '2026-03-06T00:00:00Z', bucketEnd: '2026-03-07T00:00:00Z', parallelCount: 5 },
      { bucketStart: '2026-03-07T00:00:00Z', bucketEnd: '2026-03-08T00:00:00Z', parallelCount: 4 },
    ],
  },
}

describe('ParallelWorkStatsSection', () => {
  it('renders a single active window card with toggle controls', () => {
    render(
      <ParallelWorkStatsSection stats={populatedStats} isLoading={false} error={null} />,
    )

    expect(host?.querySelector('[data-testid="parallel-work-window-toggle"]')).not.toBeNull()
    expect(host?.querySelectorAll('[data-testid^="parallel-work-card-"]')).toHaveLength(1)
    expect(host?.querySelector('[data-testid="parallel-work-card-minute7d"]')).not.toBeNull()
    expect(host?.querySelector('[data-testid="parallel-work-card-hour30d"]')).toBeNull()
    expect(host?.textContent).toContain('Parallel work')
    expect(host?.textContent).toContain('4.67')
    expect(host?.querySelectorAll('[data-chart-kind="parallel-work-sparkline"]')).toHaveLength(1)
  })

  it('switches to the selected window using the segmented toggle', () => {
    render(
      <ParallelWorkStatsSection stats={populatedStats} isLoading={false} error={null} />,
    )

    const hourTrigger = host?.querySelector(
      '[data-testid="parallel-work-window-trigger-hour30d"]',
    ) as HTMLButtonElement | null
    expect(hourTrigger).not.toBeNull()

    act(() => {
      hourTrigger?.click()
    })

    expect(host?.querySelector('[data-testid="parallel-work-card-minute7d"]')).toBeNull()
    expect(host?.querySelector('[data-testid="parallel-work-card-hour30d"]')).not.toBeNull()
    expect(host?.textContent).toContain('2.13')
  })

  it('renders empty day-all state with null summaries', () => {
    const emptyDayAll: ParallelWorkStatsResponse = {
      ...populatedStats,
      dayAll: {
        rangeStart: '2026-03-08T00:00:00Z',
        rangeEnd: '2026-03-08T00:00:00Z',
        bucketSeconds: 86400,
        completeBucketCount: 0,
        activeBucketCount: 0,
        minCount: null,
        maxCount: null,
        avgCount: null,
        points: [],
      },
    }

    render(
      <ParallelWorkStatsSection
        stats={emptyDayAll}
        isLoading={false}
        error={null}
        defaultWindowKey="dayAll"
      />,
    )

    const dayAllCard = host?.querySelector('[data-testid="parallel-work-card-dayAll"]')
    expect(dayAllCard?.textContent).toContain('No complete buckets yet.')
    expect(dayAllCard?.textContent).toContain('—')
  })

  it('renders a section-level error alert', () => {
    render(<ParallelWorkStatsSection stats={null} isLoading={false} error="boom" />)
    expect(host?.querySelector('[role="alert"]')?.textContent).toContain('boom')
  })
})
