import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { DashboardTodayActivityChart, buildTodayMinuteChartData } from './DashboardTodayActivityChart'

vi.mock('recharts', () => ({
  ResponsiveContainer: ({ children }: { children: ReactNode }) => <div data-testid="responsive">{children}</div>,
  CartesianGrid: () => <div data-testid="grid" />,
  XAxis: () => <div data-testid="x-axis" />,
  YAxis: () => <div data-testid="y-axis" />,
  Tooltip: () => <div data-testid="tooltip" />,
  Legend: () => <div data-testid="legend" />,
  ReferenceLine: () => <div data-testid="reference-line" />,
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

const response = {
  rangeStart: '2026-04-08 00:00:00',
  rangeEnd: '2026-04-08 00:03:22',
  bucketSeconds: 60,
  points: [
    {
      bucketStart: '2026-04-08 00:00:00',
      bucketEnd: '2026-04-08 00:00:59',
      totalCount: 3,
      successCount: 2,
      failureCount: 1,
      totalTokens: 120,
      totalCost: 0.5,
    },
    {
      bucketStart: '2026-04-08 00:02:00',
      bucketEnd: '2026-04-08 00:02:59',
      totalCount: 4,
      successCount: 4,
      failureCount: 0,
      totalTokens: 200,
      totalCost: 0.75,
    },
  ],
}

describe('DashboardTodayActivityChart', () => {
  it('builds a continuous minute series and preserves cumulative totals', () => {
    const data = buildTodayMinuteChartData(response, {
      now: new Date(2026, 3, 8, 0, 3, 22),
      localeTag: 'en-US',
    })

    expect(data).toHaveLength(4)
    expect(data[0]).toMatchObject({
      successCount: 2,
      failureCount: 1,
      failureCountNegative: -1,
      totalCount: 3,
      cumulativeCost: 0.5,
      cumulativeTokens: 120,
    })
    expect(data[1]).toMatchObject({
      successCount: 0,
      failureCount: 0,
      totalCount: 0,
      cumulativeCost: 0.5,
      cumulativeTokens: 120,
    })
    expect(data[2]).toMatchObject({
      successCount: 4,
      failureCount: 0,
      totalCount: 4,
      cumulativeCost: 1.25,
      cumulativeTokens: 320,
    })
    expect(data[3]).toMatchObject({
      successCount: 0,
      failureCount: 0,
      totalCount: 0,
      cumulativeCost: 1.25,
      cumulativeTokens: 320,
    })
  })

  it('always expands today charts from the local midnight even when the API rangeStart is UTC', () => {
    const data = buildTodayMinuteChartData(
      {
        rangeStart: '2026-04-08T00:00:00.000Z',
        rangeEnd: '2026-04-08T00:03:00.000Z',
        bucketSeconds: 60,
        points: [
          {
            bucketStart: '2026-04-08T00:00:00.000Z',
            bucketEnd: '2026-04-08T00:00:59.000Z',
            totalCount: 2,
            successCount: 2,
            failureCount: 0,
            totalTokens: 80,
            totalCost: 0.25,
          },
        ],
      },
      {
        now: new Date('2026-04-08T08:03:00+08:00'),
        localeTag: 'en-US',
      },
    )

    expect(data[0]?.label).toBe('00:00')
    expect(data[0]?.epochMs).toBe(new Date('2026-04-08T00:00:00+08:00').getTime())
    expect(data.at(-1)?.label).toBe('08:03')
  })

  it('renders count mode as a composed chart with split success and failure bars', () => {
    const html = renderToStaticMarkup(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalCount"
      />,
    )

    expect(html).toContain('data-testid="dashboard-today-activity-chart"')
    expect(html).toContain('data-chart-mode="count-bars"')
    expect(html).toContain('data-testid="composed-chart"')
    expect(html).not.toContain('data-testid="area-chart"')
    expect(html).toContain('data-testid="bar-series"')
  })

  it('renders cost and token modes as cumulative area charts', () => {
    const costHtml = renderToStaticMarkup(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalCost"
      />,
    )
    const tokenHtml = renderToStaticMarkup(
      <DashboardTodayActivityChart
        response={response}
        loading={false}
        error={null}
        metric="totalTokens"
      />,
    )

    expect(costHtml).toContain('data-chart-mode="cumulative-area"')
    expect(costHtml).toContain('data-testid="area-chart"')
    expect(costHtml).not.toContain('data-testid="composed-chart"')
    expect(tokenHtml).toContain('data-chart-mode="cumulative-area"')
    expect(tokenHtml).toContain('data-testid="area-chart"')
  })
})
