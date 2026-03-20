import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { TimeseriesChart } from './TimeseriesChart'

vi.mock('recharts', () => ({
  ResponsiveContainer: ({ children }: { children: ReactNode }) => <div data-testid="responsive">{children}</div>,
  CartesianGrid: () => <div data-testid="grid" />,
  XAxis: () => <div data-testid="x-axis" />,
  YAxis: () => <div data-testid="y-axis" />,
  Tooltip: () => <div data-testid="tooltip" />,
  Legend: () => <div data-testid="legend" />,
  Area: () => <div data-testid="area-series" />,
  Bar: () => <div data-testid="bar-series" />,
  AreaChart: ({ children }: { children: ReactNode }) => <div data-testid="area-chart">{children}</div>,
  ComposedChart: ({ children }: { children: ReactNode }) => <div data-testid="composed-chart">{children}</div>,
}))

vi.mock('../i18n', () => ({
  useTranslation: () => ({
    locale: 'zh',
    t: (key: string) => key,
  }),
}))

vi.mock('../theme', () => ({
  useTheme: () => ({
    themeMode: 'light',
  }),
}))

function createPoint(index: number) {
  const hour = String(index).padStart(2, '0')
  return {
    bucketStart: `2026-03-20T${hour}:00:00Z`,
    bucketEnd: `2026-03-20T${hour}:59:59Z`,
    totalTokens: index + 10,
    totalCount: index + 1,
    totalCost: index + 0.5,
    successCount: index + 1,
    failureCount: 0,
  }
}

describe('TimeseriesChart', () => {
  it('renders bucket bars at or below the 7-point threshold', () => {
    const html = renderToStaticMarkup(
      <TimeseriesChart
        points={Array.from({ length: 7 }, (_, index) => createPoint(index))}
        isLoading={false}
        bucketSeconds={3600}
      />,
    )

    expect(html).toContain('data-chart-kind="stats-timeseries-trend"')
    expect(html).toContain('data-chart-mode="bucket-bar"')
    expect(html).toContain('data-testid="composed-chart"')
    expect(html).not.toContain('data-testid="area-chart"')
  })

  it('renders cumulative area mode once the dataset has more than 7 points', () => {
    const html = renderToStaticMarkup(
      <TimeseriesChart
        points={Array.from({ length: 8 }, (_, index) => createPoint(index))}
        isLoading={false}
        bucketSeconds={3600}
      />,
    )

    expect(html).toContain('data-chart-kind="stats-timeseries-trend"')
    expect(html).toContain('data-chart-mode="cumulative-area"')
    expect(html).toContain('data-testid="area-chart"')
    expect(html).not.toContain('data-testid="composed-chart"')
  })
})
