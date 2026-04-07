import { useEffect, useMemo, useState } from 'react'
import { useSummary } from '../hooks/useStats'
import { useTranslation } from '../i18n'
import { metricAccent } from '../lib/chartTheme'
import { useTheme } from '../theme'
import { Last24hTenMinuteHeatmap, type MetricKey } from './Last24hTenMinuteHeatmap'
import { StatsCards } from './StatsCards'
import { SegmentedControl, SegmentedControlItem } from './ui/segmented-control'
import { UsageCalendar } from './UsageCalendar'
import { WeeklyHourlyHeatmap } from './WeeklyHourlyHeatmap'

type RangeKey = '1d' | '7d' | 'usage'

const RANGE_OPTIONS: Array<{ key: RangeKey; labelKey: string }> = [
  { key: '1d', labelKey: 'dashboard.activityOverview.range24h' },
  { key: '7d', labelKey: 'dashboard.activityOverview.range7d' },
  { key: 'usage', labelKey: 'dashboard.activityOverview.rangeUsage' },
]

const METRIC_OPTIONS: Array<{ key: MetricKey; labelKey: string }> = [
  { key: 'totalCount', labelKey: 'metric.totalCount' },
  { key: 'totalCost', labelKey: 'metric.totalCost' },
  { key: 'totalTokens', labelKey: 'metric.totalTokens' },
]

export function DashboardActivityOverview() {
  const { t } = useTranslation()
  const { themeMode } = useTheme()
  const [activeRange, setActiveRange] = useState<RangeKey>('1d')
  const [visitedRanges, setVisitedRanges] = useState<Record<RangeKey, boolean>>({
    '1d': true,
    '7d': false,
    usage: false,
  })
  const [metric24h, setMetric24h] = useState<MetricKey>('totalCount')
  const [metric7d, setMetric7d] = useState<MetricKey>('totalCount')
  const [metricUsage, setMetricUsage] = useState<MetricKey>('totalCount')
  const {
    summary: summary24h,
    isLoading: summary24hLoading,
    error: summary24hError,
  } = useSummary('1d')
  const {
    summary: summary7d,
    isLoading: summary7dLoading,
    error: summary7dError,
  } = useSummary('7d')

  const rangeOptions = useMemo(
    () => RANGE_OPTIONS.map((option) => ({ ...option, label: t(option.labelKey) })),
    [t],
  )
  const metricOptions = useMemo(
    () => METRIC_OPTIONS.map((option) => ({ ...option, label: t(option.labelKey) })),
    [t],
  )

  const activeMetric =
    activeRange === '1d' ? metric24h : activeRange === '7d' ? metric7d : metricUsage
  const activeSummary = activeRange === '1d' ? summary24h : activeRange === '7d' ? summary7d : null
  const activeSummaryLoading =
    activeRange === '1d' ? summary24hLoading : activeRange === '7d' ? summary7dLoading : false
  const activeSummaryError =
    activeRange === '1d' ? summary24hError : activeRange === '7d' ? summary7dError : null

  useEffect(() => {
    setVisitedRanges((current) =>
      current[activeRange]
        ? current
        : {
            ...current,
            [activeRange]: true,
          },
    )
  }, [activeRange])

  const setActiveMetric = (metric: MetricKey) => {
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

        {activeRange === 'usage' ? null : (
          <StatsCards stats={activeSummary} loading={activeSummaryLoading} error={activeSummaryError} />
        )}

        {visitedRanges['1d'] ? (
          <div
            data-testid="dashboard-activity-range-1d"
            data-active={activeRange === '1d'}
            hidden={activeRange !== '1d'}
            aria-hidden={activeRange !== '1d'}
          >
            <Last24hTenMinuteHeatmap
              metric={metric24h}
              onChangeMetric={setMetric24h}
              showHeader={false}
            />
          </div>
        ) : null}

        {visitedRanges['7d'] ? (
          <div
            data-testid="dashboard-activity-range-7d"
            data-active={activeRange === '7d'}
            hidden={activeRange !== '7d'}
            aria-hidden={activeRange !== '7d'}
          >
            <WeeklyHourlyHeatmap
              metric={metric7d}
              onChangeMetric={setMetric7d}
              showHeader={false}
              showSurface={false}
            />
          </div>
        ) : null}

        {visitedRanges.usage ? (
          <div
            data-testid="dashboard-activity-range-usage"
            data-active={activeRange === 'usage'}
            hidden={activeRange !== 'usage'}
            aria-hidden={activeRange !== 'usage'}
          >
            <UsageCalendar
              metric={metricUsage}
              onChangeMetric={setMetricUsage}
              showSurface={false}
              showMetricToggle={false}
              showMeta={false}
            />
          </div>
        ) : null}
      </div>
    </section>
  )
}

export default DashboardActivityOverview
