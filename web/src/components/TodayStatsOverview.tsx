import type { StatsResponse } from '../lib/api'
import { useTranslation } from '../i18n'
import { cn } from '../lib/utils'
import { getBrowserTimeZone } from '../lib/timeZone'
import { AdaptiveMetricValue, type AdaptiveMetricValueKind } from './AdaptiveMetricValue'
import { Alert } from './ui/alert'
import { Badge } from './ui/badge'

export interface TodayStatsOverviewProps {
  stats: StatsResponse | null
  loading: boolean
  error?: string | null
  showSurface?: boolean
  showHeader?: boolean
  showDayBadge?: boolean
}

interface MetricTileProps {
  label: string
  value: number
  localeTag: string
  loading: boolean
  kind?: AdaptiveMetricValueKind
  toneClass?: string
  valueTestId?: string
}

function MetricTile({
  label,
  value,
  localeTag,
  loading,
  kind = 'number',
  toneClass,
  valueTestId,
}: MetricTileProps) {
  return (
    <div
      data-testid="today-stats-metric-tile"
      className="min-w-0 rounded-xl border border-base-300/75 bg-base-200/60 p-4"
    >
      <div className="text-xs font-semibold uppercase tracking-[0.14em] text-base-content/65">{label}</div>
      {loading ? (
        <div className="mt-2 h-8 w-28 animate-pulse rounded bg-base-300/65" />
      ) : (
        <div
          className={cn(
            'mt-2 min-w-0 overflow-hidden text-2xl font-semibold leading-tight text-base-content lg:text-[1.85rem]',
            toneClass,
          )}
        >
          <AdaptiveMetricValue
            value={value}
            localeTag={localeTag}
            kind={kind}
            data-testid={valueTestId}
          />
        </div>
      )}
    </div>
  )
}

export function TodayStatsOverview({
  stats,
  loading,
  error,
  showSurface = true,
  showHeader = true,
  showDayBadge = true,
}: TodayStatsOverviewProps) {
  const { t, locale } = useTranslation()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const timeZone = getBrowserTimeZone()

  const totalCount = stats?.totalCount ?? 0
  const successCount = stats?.successCount ?? 0
  const failureCount = stats?.failureCount ?? 0
  const totalCost = stats?.totalCost ?? 0
  const totalTokens = stats?.totalTokens ?? 0

  const content = (
    <>
      {showHeader ? (
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="section-heading">
            <h2 className="section-title">{t('dashboard.today.title')}</h2>
            <p className="section-description">{t('dashboard.today.subtitle', { timezone: timeZone })}</p>
          </div>
          {showDayBadge ? (
            <Badge variant="default" className="px-2 py-[0.18rem] text-[11px]">
              {t('dashboard.today.dayBadge')}
            </Badge>
          ) : null}
        </div>
      ) : null}

      {error ? (
        <Alert variant="error">{t('stats.cards.loadError', { error })}</Alert>
      ) : (
        <div
          data-testid="today-stats-metrics-grid"
          className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-5"
        >
          <MetricTile
            label={t('stats.cards.totalCalls')}
            value={totalCount}
            localeTag={localeTag}
            loading={loading}
            toneClass="text-primary"
            valueTestId="today-stats-value-total-calls"
          />
          <MetricTile
            label={t('stats.cards.success')}
            value={successCount}
            localeTag={localeTag}
            loading={loading}
            toneClass="text-success"
            valueTestId="today-stats-value-success"
          />
          <MetricTile
            label={t('stats.cards.failures')}
            value={failureCount}
            localeTag={localeTag}
            loading={loading}
            toneClass="text-error"
            valueTestId="today-stats-value-failures"
          />
          <MetricTile
            label={t('stats.cards.totalCost')}
            value={totalCost}
            localeTag={localeTag}
            loading={loading}
            kind="currency"
            valueTestId="today-stats-value-total-cost"
          />
          <MetricTile
            label={t('stats.cards.totalTokens')}
            value={totalTokens}
            localeTag={localeTag}
            loading={loading}
            valueTestId="today-stats-value-total-tokens"
          />
        </div>
      )}
    </>
  )

  if (!showSurface) {
    return (
      <div className="flex flex-col gap-5" data-testid="today-stats-overview-card">
        {content}
      </div>
    )
  }

  return (
    <section className="surface-panel h-full" data-testid="today-stats-overview-card">
      <div className="surface-panel-body gap-5">{content}</div>
    </section>
  )
}
