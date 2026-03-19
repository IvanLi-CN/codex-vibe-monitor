/** @vitest-environment jsdom */
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest'
import StatsPage from './Stats'
import { BUCKET_OPTION_KEYS } from './stats-options'

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

vi.mock('../i18n', () => ({
  useTranslation: () => ({
    locale: 'zh',
    t: (key: string, values?: Record<string, string | number>) => {
      const map: Record<string, string> = {
        'stats.title': '统计',
        'stats.subtitle': '选择时间范围与聚合粒度',
        'stats.range.today': '今日',
        'stats.range.lastWeek': '最近 7 天',
        'stats.range.lastHour': '最近 1 小时',
        'stats.range.lastDay': '最近 1 天',
        'stats.range.thisWeek': '本周',
        'stats.range.thisMonth': '本月',
        'stats.range.lastMonth': '最近 1 个月',
        'stats.bucket.eachHour': '每小时',
        'stats.bucket.each6Hours': '每 6 小时',
        'stats.bucket.each12Hours': '每 12 小时',
        'stats.bucket.each24Hours': '每 24 小时',
        'stats.bucket.eachMinute': '每分钟',
        'stats.bucket.each5Minutes': '每 5 分钟',
        'stats.bucket.each15Minutes': '每 15 分钟',
        'stats.bucket.each30Minutes': '每 30 分钟',
        'stats.bucket.eachDay': '每天',
        'stats.trendTitle': '趋势',
        'stats.successFailureTitle': '成功/失败次数',
        'stats.errors.title': '错误原因分布',
        'stats.errors.scope.label': '失败范围',
        'stats.errors.scope.service': '服务端故障',
        'stats.errors.scope.client': '调用方错误',
        'stats.errors.scope.abort': '客户端中断',
        'stats.errors.scope.all': '全部失败',
        'stats.errors.actionableRate': `可行动失败率：${values?.rate ?? '0.0%'}`,
        'stats.errors.summary.service': '服务端故障',
        'stats.errors.summary.client': '调用方错误',
        'stats.errors.summary.abort': '客户端中断',
        'stats.errors.summary.actionable': '可行动故障',
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

describe('StatsPage', () => {
  it('uses combobox-based selects instead of native select elements', () => {
    hookMocks.useSummary.mockReturnValue({
      summary: null,
      isLoading: false,
      error: null,
    })
    hookMocks.useTimeseries.mockReturnValue({
      data: { points: [], bucketSeconds: 3600 },
      isLoading: false,
      error: null,
    })
    hookMocks.useErrorDistribution.mockReturnValue({
      data: [],
      isLoading: false,
      error: null,
    })
    hookMocks.useFailureSummary.mockReturnValue({
      data: {
        actionableFailureRate: 0,
        serviceFailureCount: 0,
        clientFailureCount: 0,
        clientAbortCount: 0,
        actionableFailureCount: 0,
      },
      isLoading: false,
      error: null,
    })

    render(<StatsPage />)

    expect(document.querySelectorAll('select')).toHaveLength(0)
    expect(document.querySelectorAll('button[role="combobox"]')).toHaveLength(3)
    expect(host?.querySelector('[data-testid="stats-range-select-trigger"]')?.textContent).toContain('今日')
    expect(host?.querySelector('[data-testid="stats-bucket-select-trigger"]')?.textContent).toContain('每 15 分钟')
  })

  it('offers a 24-hour bucket for the past 7 days range', () => {
    expect(BUCKET_OPTION_KEYS['7d']).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          value: '1d',
          labelKey: 'stats.bucket.each24Hours',
        }),
      ]),
    )
  })
})
