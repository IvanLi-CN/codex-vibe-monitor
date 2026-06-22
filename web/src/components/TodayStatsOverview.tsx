import type { ParallelWorkStatsResponse, StatsResponse, TimeseriesResponse } from '../lib/api'
import type { KeyboardEvent } from 'react'
import { useTranslation } from '../i18n'
import { cn } from '../lib/utils'
import { getBrowserTimeZone } from '../lib/timeZone'
import { AdaptiveMetricValue, type AdaptiveMetricValueKind } from './AdaptiveMetricValue'
import { Alert } from './ui/alert'
import { Badge } from './ui/badge'
import { Tooltip } from './ui/tooltip'
import type { DashboardTodayRateSnapshot } from './dashboardTodayRateSnapshot'
import { buildDashboardResponseTimeSnapshot } from './dashboardResponseTimeSnapshot'
import {
  buildActiveMinuteAverages,
  buildParallelWorkKpiSnapshot,
  buildSameProgressUsageSnapshot,
  cacheHitRate,
  dividePerConversation,
  failureRate,
  percentDelta,
  ratioOfCurrentToBaseline,
  sumCacheInputTokens,
} from './dashboardKpiComparisons'

const RATE_UNAVAILABLE_PLACEHOLDER = '—'
const PREVIOUS_FULL_DAY_COUNT = 7

export interface TodayStatsOverviewProps {
  stats: StatsResponse | null
  loading: boolean
  error?: string | null
  now?: Date
  rate?: DashboardTodayRateSnapshot | null
  rateLoading?: boolean
  rateError?: string | null
  timeseries?: TimeseriesResponse | null
  comparisonStats?: StatsResponse | null
  comparisonTimeseries?: TimeseriesResponse | null
  previous7dStats?: StatsResponse | null
  parallelWorkStats?: ParallelWorkStatsResponse | null
  comparisonParallelWorkStats?: ParallelWorkStatsResponse | null
  showInProgressConversations?: boolean
  dayKind?: 'today' | 'yesterday'
  showSurface?: boolean
  showHeader?: boolean
  showDayBadge?: boolean
}

interface MetricTileSecondaryItem {
  label: string
  value: string
  toneClass?: string
  valueTestId?: string
}

interface MetricTileMetaItem {
  label: string
  value: string
  toneClass?: string
  valueTestId?: string
}

interface MetricTileProps {
  label: string
  description: string
  value?: number
  localeTag: string
  loading: boolean
  kind?: AdaptiveMetricValueKind
  toneClass?: string
  valueTestId?: string
  displayText?: string
  subdued?: boolean
  topRightItem?: MetricTileMetaItem | null
  secondaryItems?: MetricTileSecondaryItem[]
}

function MetricTile({
  label,
  description,
  value,
  localeTag,
  loading,
  kind = 'number',
  toneClass,
  valueTestId,
  displayText,
  subdued = false,
  topRightItem,
  secondaryItems = [],
}: MetricTileProps) {
  const handleLabelKeyDown = (event: KeyboardEvent<HTMLSpanElement>) => {
    if (event.key !== 'Enter' && event.key !== ' ') return
    event.preventDefault()
    event.currentTarget.click()
  }

  return (
    <div
      data-testid="today-stats-metric-tile"
      className="min-w-0 rounded-xl border border-base-300/75 bg-base-200/60 p-4"
    >
      <div className="flex min-w-0 items-start justify-between gap-3">
        <Tooltip
          content={description}
          clickToOpen
          side="bottom"
          sideOffset={8}
          triggerProps={{
            role: 'button',
            tabIndex: 0,
            onKeyDown: handleLabelKeyDown,
          }}
        >
          <span className="inline-flex cursor-help text-left text-xs font-semibold uppercase tracking-[0.14em] text-base-content/65 underline decoration-dotted underline-offset-4 transition-colors hover:text-base-content focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary">
            {label}
          </span>
        </Tooltip>
        {topRightItem ? (
          <div className="min-w-0 shrink-0 text-right text-[11px] leading-5">
            <span className="text-base-content/52">{topRightItem.label}</span>{' '}
            <span
              data-testid={topRightItem.valueTestId}
              className={cn('font-semibold tabular-nums text-base-content/82', topRightItem.toneClass)}
            >
              {topRightItem.value}
            </span>
          </div>
        ) : null}
      </div>
      {loading ? (
        <div
          data-testid={valueTestId ? `${valueTestId}-loading` : undefined}
          className="mt-2 h-8 w-full max-w-[7.5rem] animate-pulse rounded bg-base-300/65"
        />
      ) : displayText != null ? (
        <div
          data-testid={valueTestId}
          className={cn(
            'mt-2 min-w-0 max-w-full overflow-hidden whitespace-nowrap text-2xl font-semibold leading-tight lg:text-[1.85rem]',
            subdued ? 'text-base-content/55' : 'text-base-content',
            toneClass,
          )}
        >
          {displayText}
        </div>
      ) : (
        <div
          className={cn(
            'mt-2 min-w-0 max-w-full overflow-hidden text-2xl font-semibold leading-tight text-base-content lg:text-[1.85rem]',
            toneClass,
          )}
        >
          <AdaptiveMetricValue
            value={value ?? 0}
            localeTag={localeTag}
            kind={kind}
            data-testid={valueTestId}
          />
        </div>
      )}
      {secondaryItems.length > 0 ? (
        <div className="mt-3 grid min-h-[2.75rem] grid-cols-2 gap-x-4 gap-y-2 text-xs leading-5">
          {secondaryItems.map((item, index) => (
            <div
              key={`${item.label}-${index}`}
              className={cn('min-w-0', index % 2 === 1 ? 'justify-self-end text-right' : undefined)}
            >
              <div
                data-testid={item.valueTestId}
                className="truncate"
              >
                <span className="text-base-content/52">{item.label}</span>{' '}
                <span
                  className={cn(
                    'font-semibold tabular-nums text-base-content/82',
                    item.toneClass,
                  )}
                >
                  {item.value}
                </span>
              </div>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}

function formatPercentValue(value: number | null, localeTag: string) {
  if (value == null || !Number.isFinite(value)) return '—'
  return new Intl.NumberFormat(localeTag, {
    style: 'percent',
    maximumFractionDigits: 1,
    signDisplay: 'exceptZero',
  }).format(value)
}

function formatRatioValue(value: number | null, localeTag: string) {
  if (value == null || !Number.isFinite(value)) return '—'
  return new Intl.NumberFormat(localeTag, {
    style: 'percent',
    maximumFractionDigits: 1,
  }).format(value)
}

function formatBaselineRatioValue(value: number | null, localeTag: string) {
  if (value == null || !Number.isFinite(value)) return '—'
  return new Intl.NumberFormat(localeTag, {
    maximumFractionDigits: value >= 10 ? 0 : value >= 1 ? 2 : 3,
  }).format(value)
}

function comparisonTone(value: number | null) {
  if (value == null || Math.abs(value) < 0.000_001) return 'text-base-content/70'
  return value > 0 ? 'text-success' : 'text-error'
}

function latencyComparisonTone(value: number | null) {
  if (value == null || Math.abs(value) < 0.000_001) return 'text-base-content/70'
  return value > 0 ? 'text-error' : 'text-success'
}

function formatNumberValue(value: number | null, localeTag: string, maximumFractionDigits = 2) {
  if (value == null || !Number.isFinite(value)) return '—'
  return new Intl.NumberFormat(localeTag, {
    maximumFractionDigits,
  }).format(value)
}

function formatCurrencyValue(value: number | null, localeTag: string) {
  if (value == null || !Number.isFinite(value)) return '—'
  return new Intl.NumberFormat(localeTag, {
    style: 'currency',
    currency: 'USD',
    maximumFractionDigits: 2,
  }).format(value)
}

function formatLatencyValue(value: number | null, localeTag: string) {
  if (value == null || !Number.isFinite(value)) return '—'
  if (value < 1000) {
    return `${new Intl.NumberFormat(localeTag, { maximumFractionDigits: 1 }).format(value)} ms`
  }

  const seconds = value / 1000
  const precision = Math.abs(seconds) >= 100 ? 1 : Math.abs(seconds) >= 1 ? 2 : 3
  const rounded = Number(seconds.toFixed(precision))
  return `${rounded.toLocaleString(localeTag, {
    minimumFractionDigits: 0,
    maximumFractionDigits: precision,
  })} s`
}

export function TodayStatsOverview({
  stats,
  loading,
  error,
  now,
  rate,
  rateLoading = false,
  rateError = null,
  timeseries,
  comparisonStats,
  comparisonTimeseries,
  previous7dStats,
  parallelWorkStats,
  comparisonParallelWorkStats,
  showInProgressConversations = true,
  dayKind = 'today',
  showSurface = true,
  showHeader = true,
  showDayBadge = true,
}: TodayStatsOverviewProps) {
  const { t, locale } = useTranslation()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const timeZone = getBrowserTimeZone()

  const successCount = stats?.successCount ?? 0
  const failureCount = stats?.failureCount ?? 0
  const totalCost = stats?.totalCost ?? 0
  const totalTokens = stats?.totalTokens ?? 0
  const isToday = dayKind === 'today'
  const previous7dDailyCost = previous7dStats ? previous7dStats.totalCost / PREVIOUS_FULL_DAY_COUNT : null
  const activeAverages = buildActiveMinuteAverages(stats, timeseries)
  const comparisonActiveAverages = buildActiveMinuteAverages(comparisonStats, comparisonTimeseries)
  const responseTimeSnapshot = buildDashboardResponseTimeSnapshot(timeseries ?? null, {
    closedNaturalDay: dayKind === 'yesterday',
    now,
  })
  const comparisonResponseTimeSnapshot =
    dayKind === 'today'
      ? buildDashboardResponseTimeSnapshot(comparisonTimeseries ?? null, {
          closedNaturalDay: true,
        })
      : null
  const tpmDailyDelta = percentDelta(activeAverages.tokensPerMinute, comparisonActiveAverages.tokensPerMinute)
  const spendRateDailyDelta = percentDelta(activeAverages.spendRate, comparisonActiveAverages.spendRate)
  const responseTimeDailyDelta = percentDelta(
    responseTimeSnapshot?.dayAverageMs,
    comparisonResponseTimeSnapshot?.dayAverageMs,
  )
  const sameProgressUsage = buildSameProgressUsageSnapshot(timeseries, comparisonTimeseries, { timeZone })
  const totalCostDelta = percentDelta(
    totalCost,
    isToday ? (sameProgressUsage.totalCost ?? comparisonStats?.totalCost) : comparisonStats?.totalCost,
  )
  const totalTokensDelta = percentDelta(
    totalTokens,
    isToday ? (sameProgressUsage.totalTokens ?? comparisonStats?.totalTokens) : comparisonStats?.totalTokens,
  )
  const terminalFailureRate = failureRate(successCount, failureCount)
  const tokenCacheHitRate = cacheHitRate(sumCacheInputTokens(timeseries), totalTokens)
  const parallelSnapshot = buildParallelWorkKpiSnapshot(
    stats,
    parallelWorkStats,
    comparisonParallelWorkStats,
    { preferSummaryCurrentCount: isToday },
  )
  const parallelDelta = percentDelta(parallelSnapshot.currentCount, parallelSnapshot.yesterdayAverage)
  const parallelLabel = isToday
    ? t('dashboard.today.inProgressConversations')
    : t('dashboard.today.parallelConversations')
  const parallelDescription = isToday
    ? t('dashboard.today.inProgressConversationsDescription')
    : t('dashboard.today.parallelConversationsDescription')

  const rateUnavailable = !loading && !rateLoading && rateError != null
  const responseTimeCurrentUnavailable = rateUnavailable || responseTimeSnapshot?.responseTimeMs == null
  const tokensPerMinute = rate?.tokensPerMinute ?? 0
  const spendRate = rate?.spendRate ?? 0
  const perConversationTpm = dividePerConversation(tokensPerMinute, stats?.inProgressConversationCount)
  const perConversationSpendRate = dividePerConversation(spendRate, stats?.inProgressConversationCount)
  const costLabel = isToday ? t('dashboard.today.todayCost') : t('dashboard.today.yesterdayCost')
  const tokensLabel = isToday ? t('dashboard.today.todayTokens') : t('dashboard.today.yesterdayTokens')
  const comparisonLabel = isToday
    ? t('dashboard.today.secondary.vsYesterday')
    : t('dashboard.today.secondary.comparison')
  const successComparisonRatio = isToday
    ? ratioOfCurrentToBaseline(successCount, sameProgressUsage.successCount ?? comparisonStats?.successCount)
    : null

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
          className={cn(
            'grid grid-cols-1 gap-3 sm:grid-cols-2',
            showInProgressConversations ? 'lg:grid-cols-7' : 'lg:grid-cols-6',
          )}
        >
          <MetricTile
            label={t('dashboard.today.tokensPerMinute')}
            description={t('dashboard.today.tokensPerMinuteDescription')}
            value={tokensPerMinute}
            localeTag={localeTag}
            loading={loading || rateLoading}
            kind="integer"
            toneClass="text-primary"
            valueTestId="today-stats-value-tpm"
            displayText={rateUnavailable ? RATE_UNAVAILABLE_PLACEHOLDER : undefined}
            subdued={rateUnavailable}
            topRightItem={{
              label: comparisonLabel,
              value: formatPercentValue(tpmDailyDelta, localeTag),
              toneClass: comparisonTone(tpmDailyDelta),
              valueTestId: 'today-stats-secondary-tpm-delta',
            }}
            secondaryItems={[
              {
                label: t('dashboard.today.secondary.dayAverage'),
                value: formatNumberValue(activeAverages.tokensPerMinute, localeTag, 0),
                valueTestId: 'today-stats-secondary-tpm-day-average',
              },
              {
                label: t('dashboard.today.secondary.perConversation'),
                value: formatNumberValue(perConversationTpm, localeTag, 0),
                valueTestId: 'today-stats-secondary-tpm-per-conversation',
              },
            ]}
          />
          <MetricTile
            label={t('dashboard.today.spendRate')}
            description={t('dashboard.today.spendRateDescription')}
            value={spendRate}
            localeTag={localeTag}
            loading={loading || rateLoading}
            kind="currency"
            valueTestId="today-stats-value-spend-rate"
            displayText={rateUnavailable ? RATE_UNAVAILABLE_PLACEHOLDER : undefined}
            subdued={rateUnavailable}
            topRightItem={{
              label: comparisonLabel,
              value: formatPercentValue(spendRateDailyDelta, localeTag),
              toneClass: comparisonTone(spendRateDailyDelta),
              valueTestId: 'today-stats-secondary-spend-rate-delta',
            }}
            secondaryItems={[
              {
                label: t('dashboard.today.secondary.dayAverage'),
                value: formatCurrencyValue(activeAverages.spendRate, localeTag),
                valueTestId: 'today-stats-secondary-spend-rate-day-average',
              },
              {
                label: t('dashboard.today.secondary.perConversation'),
                value: formatCurrencyValue(perConversationSpendRate, localeTag),
                valueTestId: 'today-stats-secondary-spend-rate-per-conversation',
              },
            ]}
          />
          <MetricTile
            label={t('stats.cards.success')}
            description={t('dashboard.today.successDescription')}
            value={successCount}
            localeTag={localeTag}
            loading={loading}
            toneClass="text-success"
            valueTestId="today-stats-value-success"
            topRightItem={{
              label: comparisonLabel,
              value: formatBaselineRatioValue(successComparisonRatio, localeTag),
              toneClass: comparisonTone(
                successComparisonRatio == null ? null : successComparisonRatio - 1,
              ),
              valueTestId: 'today-stats-secondary-success-ratio',
            }}
            secondaryItems={[
              {
                label: t('stats.cards.failures'),
                value: formatNumberValue(failureCount, localeTag, 0),
                toneClass: failureCount > 0 ? 'text-error' : undefined,
                valueTestId: 'today-stats-secondary-failures',
              },
              {
                label: t('dashboard.today.secondary.failureRate'),
                value: formatRatioValue(terminalFailureRate, localeTag),
                toneClass: terminalFailureRate > 0 ? 'text-error' : undefined,
                valueTestId: 'today-stats-secondary-failure-rate',
              },
            ]}
          />
          {showInProgressConversations ? (
            <MetricTile
              label={parallelLabel}
              description={parallelDescription}
              value={parallelSnapshot.currentCount ?? 0}
              localeTag={localeTag}
              loading={loading}
              kind="integer"
              toneClass="text-info"
              valueTestId="today-stats-value-in-progress-conversations"
              topRightItem={{
                label: comparisonLabel,
                value: formatPercentValue(parallelDelta, localeTag),
                toneClass: comparisonTone(parallelDelta),
                valueTestId: 'today-stats-secondary-in-progress-delta',
              }}
              secondaryItems={[
                {
                  label: t('dashboard.today.secondary.dayAverage'),
                  value: formatNumberValue(parallelSnapshot.dayAverage, localeTag, 2),
                  valueTestId: 'today-stats-secondary-in-progress-day-average',
                },
                {
                  label: t('dashboard.today.secondary.retry'),
                  value: formatNumberValue(
                    stats?.inProgressRetryConversationCount ?? null,
                    localeTag,
                    0,
                  ),
                  valueTestId: 'today-stats-secondary-in-progress-retry',
                },
              ]}
            />
          ) : null}
          <MetricTile
            label={t('dashboard.today.responseTime')}
            description={t('dashboard.today.responseTimeDescription')}
            localeTag={localeTag}
            loading={loading || rateLoading}
            valueTestId="today-stats-value-response-time"
            displayText={formatLatencyValue(
              responseTimeCurrentUnavailable ? null : (responseTimeSnapshot?.responseTimeMs ?? null),
              localeTag,
            )}
            subdued={responseTimeCurrentUnavailable}
            topRightItem={{
              label: comparisonLabel,
              value: formatPercentValue(rateUnavailable ? null : responseTimeDailyDelta, localeTag),
              toneClass: latencyComparisonTone(rateUnavailable ? null : responseTimeDailyDelta),
              valueTestId: 'today-stats-secondary-response-time-delta',
            }}
            secondaryItems={[
              {
                label: t('dashboard.today.secondary.dayAverage'),
                value: formatLatencyValue(
                  rateUnavailable ? null : (responseTimeSnapshot?.dayAverageMs ?? null),
                  localeTag,
                ),
                valueTestId: 'today-stats-secondary-response-time-day-average',
              },
              {
                label: t('dashboard.today.secondary.inProgress'),
                value: formatLatencyValue(stats?.inProgressAvgWaitMs ?? null, localeTag),
                valueTestId: 'today-stats-secondary-response-time-in-progress',
              },
            ]}
          />
          <MetricTile
            label={costLabel}
            description={t('dashboard.today.totalCostDescription')}
            value={totalCost}
            localeTag={localeTag}
            loading={loading}
            kind="currency"
            valueTestId="today-stats-value-total-cost"
            topRightItem={{
              label: comparisonLabel,
              value: formatPercentValue(totalCostDelta, localeTag),
              toneClass: comparisonTone(totalCostDelta),
              valueTestId: 'today-stats-secondary-cost-delta',
            }}
            secondaryItems={[
              {
                label: t('dashboard.today.secondary.previous7dAverage'),
                value: formatCurrencyValue(previous7dDailyCost, localeTag),
                valueTestId: 'today-stats-secondary-cost-previous7d-average',
              },
              {
                label: t('dashboard.today.secondary.failed'),
                value: formatCurrencyValue(stats?.nonSuccessCost ?? null, localeTag),
                valueTestId: 'today-stats-secondary-cost-failed',
              },
            ]}
          />
          <MetricTile
            label={tokensLabel}
            description={t('dashboard.today.totalTokensDescription')}
            value={totalTokens}
            localeTag={localeTag}
            loading={loading}
            valueTestId="today-stats-value-total-tokens"
            topRightItem={{
              label: comparisonLabel,
              value: formatPercentValue(totalTokensDelta, localeTag),
              toneClass: comparisonTone(totalTokensDelta),
              valueTestId: 'today-stats-secondary-tokens-delta',
            }}
            secondaryItems={[
              {
                label: t('dashboard.today.secondary.cacheHitRate'),
                value: formatRatioValue(tokenCacheHitRate, localeTag),
                valueTestId: 'today-stats-secondary-cache-hit-rate',
              },
              {
                label: t('dashboard.today.secondary.failed'),
                value: formatNumberValue(stats?.nonSuccessTokens ?? null, localeTag, 0),
                valueTestId: 'today-stats-secondary-tokens-failed',
              },
            ]}
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
