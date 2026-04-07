import { useMemo } from 'react'
import type { StatsResponse } from '../lib/api'
import { useTranslation } from '../i18n'
import { cn } from '../lib/utils'
import { getBrowserTimeZone } from '../lib/timeZone'
import { AnimatedDigits } from './AnimatedDigits'
import { Alert } from './ui/alert'
import { Badge } from './ui/badge'

export interface TodayStatsOverviewProps {
  stats: StatsResponse | null
  loading: boolean
  error?: string | null
}

interface MetricTileProps {
  label: string
  value: string
  loading: boolean
  toneClass?: string
}

function MetricTile({ label, value, loading, toneClass }: MetricTileProps) {
  return (
    <div
      data-testid="today-stats-metric-tile"
      className="rounded-xl border border-base-300/75 bg-base-200/60 p-4"
    >
      <div className="text-xs font-semibold uppercase tracking-[0.14em] text-base-content/65">{label}</div>
      {loading ? (
        <div className="mt-2 h-8 w-28 animate-pulse rounded bg-base-300/65" />
      ) : (
        <div
          className={cn(
            'mt-2 text-2xl font-semibold leading-tight text-base-content lg:text-[1.85rem]',
            toneClass,
          )}
        >
          <AnimatedDigits value={value} />
        </div>
      )}
    </div>
  )
}

export function TodayStatsOverview({ stats, loading, error }: TodayStatsOverviewProps) {
  const { t, locale } = useTranslation()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const numberFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag, { maximumFractionDigits: 2 }),
    [localeTag],
  )
  const timeZone = getBrowserTimeZone()

  const totalCount = stats?.totalCount ?? 0
  const successCount = stats?.successCount ?? 0
  const failureCount = stats?.failureCount ?? 0
  const totalCost = stats?.totalCost ?? 0
  const totalTokens = stats?.totalTokens ?? 0

  const countValue = numberFormatter.format(totalCount)
  const successValue = numberFormatter.format(successCount)
  const failureValue = numberFormatter.format(failureCount)
  const costValue = `$${numberFormatter.format(totalCost)}`
  const tokenValue = numberFormatter.format(totalTokens)

  return (
    <section className="surface-panel h-full" data-testid="today-stats-overview-card">
      <div className="surface-panel-body gap-5">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="section-heading">
            <h2 className="section-title">{t('dashboard.today.title')}</h2>
            <p className="section-description">{t('dashboard.today.subtitle', { timezone: timeZone })}</p>
          </div>
          <Badge variant="default" className="px-2 py-[0.18rem] text-[11px]">
            {t('dashboard.today.dayBadge')}
          </Badge>
        </div>

        {error ? (
          <Alert variant="error">{t('stats.cards.loadError', { error })}</Alert>
        ) : (
          <div
            data-testid="today-stats-metrics-grid"
            className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-5"
          >
            <MetricTile
              label={t('stats.cards.totalCalls')}
              value={countValue}
              loading={loading}
              toneClass="text-primary"
            />
            <MetricTile
              label={t('stats.cards.success')}
              value={successValue}
              loading={loading}
              toneClass="text-success"
            />
            <MetricTile
              label={t('stats.cards.failures')}
              value={failureValue}
              loading={loading}
              toneClass="text-error"
            />
            <MetricTile
              label={t('stats.cards.totalCost')}
              value={costValue}
              loading={loading}
            />
            <MetricTile
              label={t('stats.cards.totalTokens')}
              value={tokenValue}
              loading={loading}
            />
          </div>
        )}
      </div>
    </section>
  )
}
