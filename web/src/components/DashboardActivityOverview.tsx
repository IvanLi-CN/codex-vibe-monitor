import { memo, useEffect, useMemo, useState } from 'react'
import { useSummary } from '../hooks/useStats'
import { useTimeseries } from '../hooks/useTimeseries'
import { useTranslation } from '../i18n'
import { metricAccent } from '../lib/chartTheme'
import { useTheme } from '../theme'
import { DashboardTodayActivityChart } from './DashboardTodayActivityChart'
import { Last24hTenMinuteHeatmap, type MetricKey } from './Last24hTenMinuteHeatmap'
import { StatsCards } from './StatsCards'
import { TodayStatsOverview } from './TodayStatsOverview'
import { buildDashboardTodayRateSnapshot } from './dashboardTodayRateSnapshot'
import { SegmentedControl, SegmentedControlItem } from './ui/segmented-control'
import { UsageCalendar } from './UsageCalendar'
import { WeeklyHourlyHeatmap } from './WeeklyHourlyHeatmap'

type RangeKey = 'today' | 'yesterday' | '1d' | '7d' | 'usage'

export const DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY = 'dashboard.activityOverview.activeRange.v1'

const DEFAULT_RANGE: RangeKey = 'today'
const RANGE_OPTIONS: Array<{ key: RangeKey; labelKey: string }> = [
  { key: 'today', labelKey: 'dashboard.activityOverview.rangeToday' },
  { key: 'yesterday', labelKey: 'dashboard.activityOverview.rangeYesterday' },
  { key: '1d', labelKey: 'dashboard.activityOverview.range24h' },
  { key: '7d', labelKey: 'dashboard.activityOverview.range7d' },
  { key: 'usage', labelKey: 'dashboard.activityOverview.rangeUsage' },
]

const METRIC_OPTIONS: Array<{ key: MetricKey; labelKey: string }> = [
  { key: 'totalCount', labelKey: 'metric.totalCount' },
  { key: 'totalCost', labelKey: 'metric.totalCost' },
  { key: 'totalTokens', labelKey: 'metric.totalTokens' },
]

function isRangeKey(value: string | null): value is RangeKey {
  return value === 'today' || value === 'yesterday' || value === '1d' || value === '7d' || value === 'usage'
}

function readPersistedRange(): RangeKey {
  if (typeof window === 'undefined') return DEFAULT_RANGE
  try {
    const cached = window.localStorage.getItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY)
    return isRangeKey(cached) ? cached : DEFAULT_RANGE
  } catch {
    return DEFAULT_RANGE
  }
}

function persistRange(range: RangeKey) {
  if (typeof window === 'undefined') return
  try {
    window.localStorage.setItem(DASHBOARD_ACTIVITY_RANGE_STORAGE_KEY, range)
  } catch {
    // Ignore storage write failures and keep the UI responsive.
  }
}

function DashboardNaturalDayRangePanel({
  metric,
  summaryWindow,
  timeseriesRange,
  testId,
}: {
  metric: MetricKey
  summaryWindow: 'today' | 'yesterday'
  timeseriesRange: 'today' | 'yesterday'
  testId: string
}) {
  const { data, isLoading, error } = useTimeseries(timeseriesRange, { bucket: '1m' })

  return (
    <div
      className="flex flex-col gap-5"
      data-testid={testId}
      data-active="true"
    >
      <DashboardNaturalDaySummaryOverview
        summaryWindow={summaryWindow}
        response={data}
        loading={isLoading}
        error={error}
        closedNaturalDay={timeseriesRange === 'yesterday'}
      />
      <DashboardNaturalDayChartSection
        response={data}
        loading={isLoading}
        error={error}
        metric={metric}
        closedNaturalDay={timeseriesRange === 'yesterday'}
      />
    </div>
  )
}

function DashboardNaturalDaySummaryOverview({
  summaryWindow,
  response,
  loading,
  error,
  closedNaturalDay,
}: {
  summaryWindow: 'today' | 'yesterday'
  response: ReturnType<typeof useTimeseries>['data']
  loading: boolean
  error: ReturnType<typeof useTimeseries>['error']
  closedNaturalDay: boolean
}) {
  const {
    summary,
    isLoading: summaryLoading,
    error: summaryError,
  } = useSummary(summaryWindow)
  const rate = useMemo(
    () => buildDashboardTodayRateSnapshot(response, { closedNaturalDay }),
    [closedNaturalDay, response],
  )

  return (
    <TodayStatsOverview
      stats={summary}
      loading={summaryLoading}
      error={summaryError}
      rate={rate}
      rateLoading={loading}
      rateError={error}
      showSurface={false}
      showHeader={false}
      showDayBadge={false}
    />
  )
}

const DashboardNaturalDayChartSection = memo(function DashboardNaturalDayChartSection({
  response,
  loading,
  error,
  metric,
  closedNaturalDay,
}: {
  response: ReturnType<typeof useTimeseries>['data']
  loading: boolean
  error: ReturnType<typeof useTimeseries>['error']
  metric: MetricKey
  closedNaturalDay: boolean
}) {
  return (
    <DashboardTodayActivityChart
      response={response}
      loading={loading}
      error={error}
      metric={metric}
      closedNaturalDay={closedNaturalDay}
    />
  )
})

function DashboardTodayRangePanel({ metric }: { metric: MetricKey }) {
  return (
    <DashboardNaturalDayRangePanel
      metric={metric}
      summaryWindow="today"
      timeseriesRange="today"
      testId="dashboard-activity-range-today"
    />
  )
}

function DashboardYesterdayRangePanel({ metric }: { metric: MetricKey }) {
  return (
    <DashboardNaturalDayRangePanel
      metric={metric}
      summaryWindow="yesterday"
      timeseriesRange="yesterday"
      testId="dashboard-activity-range-yesterday"
    />
  )
}

function Dashboard24HourRangePanel({ metric }: { metric: MetricKey }) {
  const { summary, isLoading, error } = useSummary('1d')

  return (
    <div
      className="flex flex-col gap-5"
      data-testid="dashboard-activity-range-1d"
      data-active="true"
    >
      <StatsCards stats={summary} loading={isLoading} error={error} />
      <Last24hTenMinuteHeatmap
        metric={metric}
        showHeader={false}
      />
    </div>
  )
}

function Dashboard7DayRangePanel({ metric }: { metric: MetricKey }) {
  const { summary, isLoading, error } = useSummary('7d')

  return (
    <div
      className="flex flex-col gap-5"
      data-testid="dashboard-activity-range-7d"
      data-active="true"
    >
      <StatsCards stats={summary} loading={isLoading} error={error} />
      <WeeklyHourlyHeatmap
        metric={metric}
        showHeader={false}
        showSurface={false}
      />
    </div>
  )
}

function DashboardUsageRangePanel({ metric }: { metric: MetricKey }) {
  return (
    <div
      data-testid="dashboard-activity-range-usage"
      data-active="true"
    >
      <UsageCalendar
        metric={metric}
        showSurface={false}
        showMetricToggle={false}
        showMeta={false}
      />
    </div>
  )
}

export function DashboardActivityOverview() {
  const { t } = useTranslation()
  const { themeMode } = useTheme()
  const [activeRange, setActiveRange] = useState<RangeKey>(() => readPersistedRange())
  const [metricToday, setMetricToday] = useState<MetricKey>('totalCount')
  const [metricYesterday, setMetricYesterday] = useState<MetricKey>('totalCount')
  const [metric24h, setMetric24h] = useState<MetricKey>('totalCount')
  const [metric7d, setMetric7d] = useState<MetricKey>('totalCount')
  const [metricUsage, setMetricUsage] = useState<MetricKey>('totalCount')

  const rangeOptions = useMemo(
    () => RANGE_OPTIONS.map((option) => ({ ...option, label: t(option.labelKey) })),
    [t],
  )
  const metricOptions = useMemo(
    () => METRIC_OPTIONS.map((option) => ({ ...option, label: t(option.labelKey) })),
    [t],
  )

  const activeMetric =
    activeRange === 'today'
      ? metricToday
      : activeRange === 'yesterday'
        ? metricYesterday
      : activeRange === '1d'
        ? metric24h
        : activeRange === '7d'
          ? metric7d
          : metricUsage

  useEffect(() => {
    persistRange(activeRange)
  }, [activeRange])

  const setActiveMetric = (metric: MetricKey) => {
    if (activeRange === 'today') {
      setMetricToday(metric)
      return
    }
    if (activeRange === 'yesterday') {
      setMetricYesterday(metric)
      return
    }
    if (activeRange === '1d') {
      setMetric24h(metric)
      return
    }
    if (activeRange === '7d') {
      setMetric7d(metric)
      return
    }
    setMetricUsage(metric)
  }

  return (
    <section className="surface-panel overflow-visible" data-testid="dashboard-activity-overview">
      <div className="surface-panel-body gap-6">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="flex flex-wrap items-center gap-3">
            <div className="section-heading">
              <h2 className="section-title">{t('dashboard.activityOverview.title')}</h2>
            </div>
            <SegmentedControl role="tablist" aria-label={t('dashboard.activityOverview.rangeToggleAria')}>
              {rangeOptions.map((option) => {
                const active = option.key === activeRange
                return (
                  <SegmentedControlItem
                    key={option.key}
                    active={active}
                    role="tab"
                    aria-selected={active}
                    onClick={() => setActiveRange(option.key)}
                  >
                    {option.label}
                  </SegmentedControlItem>
                )
              })}
            </SegmentedControl>
          </div>
          <SegmentedControl size="compact" role="tablist" aria-label={t('heatmap.metricsToggleAria')}>
            {metricOptions.map((option) => {
              const active = option.key === activeMetric
              return (
                <SegmentedControlItem
                  key={option.key}
                  active={active}
                  role="tab"
                  aria-selected={active}
                  style={active ? { color: metricAccent(option.key, themeMode) } : undefined}
                  onClick={() => setActiveMetric(option.key)}
                >
                  {option.label}
                </SegmentedControlItem>
              )
            })}
          </SegmentedControl>
        </div>
        {activeRange === 'today' ? <DashboardTodayRangePanel metric={metricToday} /> : null}
        {activeRange === 'yesterday' ? <DashboardYesterdayRangePanel metric={metricYesterday} /> : null}
        {activeRange === '1d' ? <Dashboard24HourRangePanel metric={metric24h} /> : null}
        {activeRange === '7d' ? <Dashboard7DayRangePanel metric={metric7d} /> : null}
        {activeRange === 'usage' ? <DashboardUsageRangePanel metric={metricUsage} /> : null}
      </div>
    </section>
  )
}

export default DashboardActivityOverview
