import type { ParallelWorkStatsResponse, StatsResponse, TimeseriesResponse } from '../lib/api'
import { useLayoutEffect, useRef, useState, type KeyboardEvent } from 'react'
import { useTranslation } from '../i18n'
import { cn } from '../lib/utils'
import { getBrowserTimeZone } from '../lib/timeZone'
import {
  AdaptiveDisplayValue,
  AdaptiveMetricValue,
} from './AdaptiveMetricValue'
import {
  buildAdaptiveCurrencyTextSpec,
  buildAdaptiveCurrencyAmountTextSpec,
  buildAdaptiveDurationTextSpec,
  buildAdaptiveNumberTextSpec,
  buildAdaptivePercentTextSpec,
  buildAdaptiveRateCurrencyTextSpec,
  buildAdaptiveTextSpec,
  type AdaptiveCurrencyProfile,
  type AdaptiveDisplayValueSpec,
  type AdaptiveMetricValueKind,
} from './adaptiveMetricValueSpec'
import { AppIcon, type AppIconName } from './AppIcon'
import { Alert } from './ui/alert'
import { Badge } from './ui/badge'
import { Tooltip } from './ui/tooltip'
import type { DashboardTodayRateSnapshot } from './dashboardTodayRateSnapshot'
import { parseDateInput, resolveClosedNaturalDayEnd } from './dashboardNaturalDayWindow'
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
const METRIC_TILE_STACK_META_BREAKPOINT_PX = 176

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
  valueSpec: AdaptiveDisplayValueSpec
  toneClass?: string
  valueTestId?: string
}

interface MetricTileMetaItem {
  label: string
  valueSpec: AdaptiveDisplayValueSpec
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
  currencyProfile?: AdaptiveCurrencyProfile
  toneClass?: string
  valueTestId?: string
  displayText?: string
  displaySpec?: AdaptiveDisplayValueSpec
  subdued?: boolean
  preserveLabelCase?: boolean
  labelTestId?: string
  iconName?: AppIconName
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
  currencyProfile,
  toneClass,
  valueTestId,
  displayText,
  displaySpec,
  subdued = false,
  preserveLabelCase = false,
  labelTestId,
  iconName,
  topRightItem,
  secondaryItems = [],
}: MetricTileProps) {
  const handleLabelKeyDown = (event: KeyboardEvent<HTMLSpanElement>) => {
    if (event.key !== 'Enter' && event.key !== ' ') return
    event.preventDefault()
    event.currentTarget.click()
  }

  const tileRef = useRef<HTMLDivElement | null>(null)
  const [stackMeta, setStackMeta] = useState(false)

  useLayoutEffect(() => {
    const tile = tileRef.current
    if (!tile) return undefined

    const updateStackMeta = () => {
      const nextValue = tile.clientWidth > 0 && tile.clientWidth < METRIC_TILE_STACK_META_BREAKPOINT_PX
      setStackMeta((current) => (current === nextValue ? current : nextValue))
    }

    updateStackMeta()
    const frame = window.requestAnimationFrame(updateStackMeta)
    window.addEventListener('resize', updateStackMeta)

    if (typeof ResizeObserver === 'undefined') {
      return () => {
        window.cancelAnimationFrame(frame)
        window.removeEventListener('resize', updateStackMeta)
      }
    }

    const observer = new ResizeObserver(updateStackMeta)
    observer.observe(tile)

    return () => {
      window.cancelAnimationFrame(frame)
      window.removeEventListener('resize', updateStackMeta)
      observer.disconnect()
    }
  }, [])

  const inlineSecondaryItems = stackMeta ? [] : secondaryItems
  const stackedMetaItems = stackMeta
    ? [
        ...(topRightItem ? [topRightItem] : []),
        ...secondaryItems,
      ]
    : []
  const icon = iconName ? (
    <span
      aria-hidden
      data-testid={valueTestId ? `${valueTestId}-icon` : undefined}
      className={cn(
        'flex h-[1.65rem] w-[1.65rem] shrink-0 items-center justify-center text-[1.55rem] leading-none',
        subdued ? 'text-base-content/45' : toneClass ?? 'text-base-content/65',
      )}
    >
      <AppIcon name={iconName} className={cn(iconName === 'send' && '-rotate-45')} />
    </span>
  ) : null

  return (
    <div
      ref={tileRef}
      data-testid="today-stats-metric-tile"
      data-stack-meta={stackMeta ? 'true' : 'false'}
      className="min-w-0 rounded-xl border border-base-300/75 bg-base-200/60 p-4"
    >
      <div className="flex min-w-0 items-start justify-between gap-3">
        <Tooltip
          className="min-w-0 flex-1"
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
          <span
            data-testid={labelTestId}
            className={cn(
              'block min-w-0 max-w-full cursor-help overflow-hidden text-ellipsis whitespace-nowrap text-left text-xs font-semibold tracking-[0.14em] text-base-content/65 underline decoration-dotted underline-offset-4 transition-colors hover:text-base-content focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
              preserveLabelCase ? 'normal-case' : 'uppercase',
            )}
          >
            {label}
          </span>
        </Tooltip>
        {!stackMeta && topRightItem ? (
          <div className="flex min-w-0 items-baseline justify-end gap-1 text-right text-[11px] leading-5">
            <span className="shrink-0 whitespace-nowrap text-base-content/52">{topRightItem.label}</span>
            <AdaptiveDisplayValue
              spec={topRightItem.valueSpec}
              data-testid={topRightItem.valueTestId}
              className={cn('min-w-0 max-w-full font-semibold text-base-content/82', topRightItem.toneClass)}
            />
          </div>
        ) : null}
      </div>
      {loading ? (
        <div className="mt-2 flex min-w-0 max-w-full items-center gap-2.5">
          {icon}
          <div
            data-testid={valueTestId ? `${valueTestId}-loading` : undefined}
            className="h-8 w-full max-w-[7.5rem] animate-pulse rounded bg-base-300/65"
          />
        </div>
      ) : displaySpec ? (
        <div
          className={cn(
            'mt-2 flex min-w-0 max-w-full items-center gap-2.5 overflow-hidden text-[2.1rem] font-semibold leading-tight lg:text-[2rem]',
            subdued ? 'text-base-content/55' : 'text-base-content',
            toneClass,
          )}
        >
          {icon}
          <AdaptiveDisplayValue
            spec={displaySpec}
            data-testid={valueTestId}
            className={cn('min-w-0 flex-1', subdued && 'text-base-content/55')}
          />
        </div>
      ) : displayText != null ? (
        <div
          className={cn(
            'mt-2 flex min-w-0 max-w-full items-center gap-2.5 overflow-hidden whitespace-nowrap text-[2.1rem] font-semibold leading-tight lg:text-[2rem]',
            subdued ? 'text-base-content/55' : 'text-base-content',
            toneClass,
          )}
        >
          {icon}
          <span data-testid={valueTestId} className="min-w-0 flex-1 overflow-hidden text-ellipsis">
            {displayText}
          </span>
        </div>
      ) : (
        <div
          className={cn(
            'mt-2 flex min-w-0 max-w-full items-center gap-2.5 overflow-hidden text-[2.1rem] font-semibold leading-tight text-base-content lg:text-[2rem]',
            toneClass,
          )}
        >
          {icon}
          <AdaptiveMetricValue
            value={value ?? 0}
            localeTag={localeTag}
            kind={kind}
            currencyProfile={currencyProfile}
            className="min-w-0 flex-1"
            data-testid={valueTestId}
          />
        </div>
      )}
      {stackedMetaItems.length > 0 ? (
        <div
          data-testid={valueTestId ? `${valueTestId}-stacked-meta` : undefined}
          className="mt-3 grid min-h-[4.75rem] grid-cols-1 gap-y-2 text-xs leading-5"
        >
          {stackedMetaItems.map((item, index) => (
            <div key={`${item.label}-${index}`} className="min-w-0">
              <div className="flex min-w-0 items-baseline gap-1">
                <span className="shrink-0 whitespace-nowrap text-base-content/52">{item.label}</span>
                <AdaptiveDisplayValue
                  spec={item.valueSpec}
                  data-testid={item.valueTestId}
                  className={cn('min-w-0 flex-1 text-right font-semibold text-base-content/82', item.toneClass)}
                />
              </div>
            </div>
          ))}
        </div>
      ) : null}
      {inlineSecondaryItems.length > 0 ? (
        <div className="mt-3 grid min-h-[2.75rem] grid-cols-2 gap-x-4 gap-y-2 text-xs leading-5">
          {inlineSecondaryItems.map((item, index) => (
            <div
              key={`${item.label}-${index}`}
              className={cn('min-w-0', index % 2 === 1 ? 'justify-self-end text-right' : undefined)}
            >
              <div className="flex min-w-0 items-baseline gap-1">
                <span className="shrink-0 whitespace-nowrap text-base-content/52">{item.label}</span>
                <AdaptiveDisplayValue
                  spec={item.valueSpec}
                  data-testid={item.valueTestId}
                  className={cn(
                    'min-w-0 flex-1 text-right font-semibold text-base-content/82',
                    item.toneClass,
                  )}
                />
              </div>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  )
}

function buildPercentValueSpec(
  value: number | null,
  localeTag: string,
  options?: {
    maximumFractionDigits?: number
    signDisplay?: Intl.NumberFormatOptions['signDisplay']
  },
) {
  return buildAdaptivePercentTextSpec(value, localeTag, options)
}

function buildRatioValueSpec(value: number | null, localeTag: string) {
  return buildAdaptivePercentTextSpec(value, localeTag, {
    maximumFractionDigits: 1,
  })
}

function buildBaselineRatioValueSpec(value: number | null, localeTag: string) {
  if (value == null || !Number.isFinite(value)) {
    return buildAdaptiveTextSpec('—', [{ key: 'placeholder', value: '—', priority: 0 }])
  }

  const maximumFractionDigits = value >= 10 ? 0 : value >= 1 ? 2 : 3
  return buildAdaptiveNumberTextSpec(value, localeTag, maximumFractionDigits)
}

function comparisonTone(value: number | null) {
  if (value == null || Math.abs(value) < 0.000_001) return 'text-base-content/70'
  return value > 0 ? 'text-success' : 'text-error'
}

function latencyComparisonTone(value: number | null) {
  if (value == null || Math.abs(value) < 0.000_001) return 'text-base-content/70'
  return value > 0 ? 'text-error' : 'text-success'
}

function buildNumberValueSpec(value: number | null, localeTag: string, maximumFractionDigits = 2) {
  return buildAdaptiveNumberTextSpec(value, localeTag, maximumFractionDigits)
}

function buildCurrencyValueSpec(value: number | null, localeTag: string) {
  return buildAdaptiveCurrencyTextSpec(value, localeTag)
}

function buildCurrencyAmountValueSpec(value: number | null, localeTag: string) {
  return buildAdaptiveCurrencyAmountTextSpec(value, localeTag)
}

function buildRateCurrencyValueSpec(value: number | null, localeTag: string) {
  return buildAdaptiveRateCurrencyTextSpec(value, localeTag)
}

function buildLatencyValueSpec(value: number | null, localeTag: string) {
  return buildAdaptiveDurationTextSpec(value, localeTag)
}

function recentWindowAvgTotalMs(
  response: TimeseriesResponse | null | undefined,
  options?: { now?: Date; targetWindowMinutes?: number; closedNaturalDay?: boolean },
) {
  if (!response?.points?.length) return null

  const targetWindowMinutes = Math.max(1, options?.targetWindowMinutes ?? 5)
  const fallbackNow = options?.now ?? new Date()
  const responseEnd = parseDateInput(response.rangeEnd)
  const closedNaturalDayEnd = resolveClosedNaturalDayEnd(
    response,
    options?.closedNaturalDay ?? false,
  )
  const anchor = closedNaturalDayEnd ?? (responseEnd && isSameLocalDay(responseEnd, fallbackNow) && fallbackNow.getTime() > responseEnd.getTime() ? fallbackNow : responseEnd ?? fallbackNow)
  const start = closedNaturalDayEnd
    ? floorToMinute(
        parseDateInput(response.rangeStart) ??
          new Date(closedNaturalDayEnd.getTime() - 24 * 60 * 60_000),
      )
    : startOfLocalDay(anchor)
  const startMs = start.getTime()
  const anchorMs = anchor.getTime()
  const windowStartMs = Math.max(startMs, anchorMs - targetWindowMinutes * 60_000)
  let totalLatencyMs = 0
  let totalLatencySampleWeight = 0

  for (let index = response.points.length - 1; index >= 0; index -= 1) {
    const point = response.points[index]
    const bucketStart = parseDateInput(point?.bucketStart)
    const bucketEnd = parseDateInput(point?.bucketEnd)
    if (!bucketStart || !bucketEnd) continue
    const bucketStartMs = floorToMinute(bucketStart).getTime()
    const bucketEndMs = bucketEnd.getTime()
    if (bucketStartMs >= anchorMs || bucketEndMs <= windowStartMs) continue
    const value = point?.avgTotalMs ?? null
    const sampleCount = point?.totalLatencySampleCount ?? 0
    if (
      value == null ||
      !Number.isFinite(value) ||
      !Number.isFinite(sampleCount) ||
      sampleCount <= 0
    ) {
      continue
    }
    const bucketDurationMs = bucketEndMs - bucketStartMs
    if (bucketDurationMs <= 0) continue
    const overlapStartMs = Math.max(bucketStartMs, windowStartMs)
    const overlapEndMs = Math.min(bucketEndMs, anchorMs)
    const overlapDurationMs = overlapEndMs - overlapStartMs
    if (overlapDurationMs <= 0) continue
    const overlapRatio = overlapDurationMs / bucketDurationMs
    if (!Number.isFinite(overlapRatio) || overlapRatio <= 0) continue
    const weightedSampleCount = sampleCount * overlapRatio
    totalLatencyMs += value * weightedSampleCount
    totalLatencySampleWeight += weightedSampleCount
  }

  if (totalLatencySampleWeight <= 0) {
    return null
  }

  return totalLatencyMs / totalLatencySampleWeight
}

function startOfLocalDay(date: Date) {
  const next = new Date(date)
  next.setHours(0, 0, 0, 0)
  return next
}

function isSameLocalDay(left: Date, right: Date) {
  return (
    left.getFullYear() === right.getFullYear() &&
    left.getMonth() === right.getMonth() &&
    left.getDate() === right.getDate()
  )
}

function floorToMinute(date: Date) {
  const next = new Date(date)
  next.setSeconds(0, 0)
  return next
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
    {
      preferSummaryCurrentCount: isToday,
      allowParallelFallback: dayKind !== 'yesterday',
    },
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
  const inProgressRetryCount = dayKind === 'yesterday'
    ? null
    : (stats?.inProgressRetryConversationCount ?? null)
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
            showInProgressConversations ? 'lg:grid-cols-4 xl:grid-cols-7' : 'lg:grid-cols-3 xl:grid-cols-6',
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
            iconName="speedometer"
            valueTestId="today-stats-value-tpm"
            displayText={rateUnavailable ? RATE_UNAVAILABLE_PLACEHOLDER : undefined}
            subdued={rateUnavailable}
            topRightItem={{
              label: comparisonLabel,
              valueSpec: buildPercentValueSpec(tpmDailyDelta, localeTag, { signDisplay: 'exceptZero' }),
              toneClass: comparisonTone(tpmDailyDelta),
              valueTestId: 'today-stats-secondary-tpm-delta',
            }}
            secondaryItems={[
              {
                label: t('dashboard.today.secondary.dayAverage'),
                valueSpec: buildNumberValueSpec(activeAverages.tokensPerMinute, localeTag, 0),
                valueTestId: 'today-stats-secondary-tpm-day-average',
              },
              {
                label: t('dashboard.today.secondary.perConversation'),
                valueSpec: buildNumberValueSpec(perConversationTpm, localeTag, 0),
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
            toneClass="text-accent"
            iconName="currency-usd"
            valueTestId="today-stats-value-spend-rate"
            displaySpec={
              rateUnavailable
                ? undefined
                : buildCurrencyAmountValueSpec(spendRate, localeTag)
            }
            displayText={rateUnavailable ? RATE_UNAVAILABLE_PLACEHOLDER : undefined}
            subdued={rateUnavailable}
            topRightItem={{
              label: comparisonLabel,
              valueSpec: buildPercentValueSpec(spendRateDailyDelta, localeTag, { signDisplay: 'exceptZero' }),
              toneClass: comparisonTone(spendRateDailyDelta),
              valueTestId: 'today-stats-secondary-spend-rate-delta',
            }}
            secondaryItems={[
              {
                label: t('dashboard.today.secondary.dayAverage'),
                valueSpec: buildRateCurrencyValueSpec(activeAverages.spendRate, localeTag),
                valueTestId: 'today-stats-secondary-spend-rate-day-average',
              },
              {
                label: t('dashboard.today.secondary.perConversation'),
                valueSpec: buildRateCurrencyValueSpec(perConversationSpendRate, localeTag),
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
            iconName="check-circle-outline"
            valueTestId="today-stats-value-success"
            topRightItem={{
              label: comparisonLabel,
              valueSpec: buildBaselineRatioValueSpec(successComparisonRatio, localeTag),
              toneClass: comparisonTone(
                successComparisonRatio == null ? null : successComparisonRatio - 1,
              ),
              valueTestId: 'today-stats-secondary-success-ratio',
            }}
            secondaryItems={[
              {
                label: t('stats.cards.failures'),
                valueSpec: buildNumberValueSpec(failureCount, localeTag, 0),
                toneClass: failureCount > 0 ? 'text-error' : undefined,
                valueTestId: 'today-stats-secondary-failures',
              },
              {
                label: t('dashboard.today.secondary.failureRate'),
                valueSpec: buildRatioValueSpec(terminalFailureRate, localeTag),
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
              iconName="send"
              valueTestId="today-stats-value-in-progress-conversations"
              displayText={
                parallelSnapshot.currentCount == null
                  ? RATE_UNAVAILABLE_PLACEHOLDER
                  : undefined
              }
              subdued={parallelSnapshot.currentCount == null}
              topRightItem={{
                label: comparisonLabel,
                valueSpec: buildPercentValueSpec(parallelDelta, localeTag, { signDisplay: 'exceptZero' }),
                toneClass: comparisonTone(parallelDelta),
                valueTestId: 'today-stats-secondary-in-progress-delta',
              }}
              secondaryItems={[
                {
                  label: t('dashboard.today.secondary.dayAverage'),
                  valueSpec: buildNumberValueSpec(parallelSnapshot.dayAverage, localeTag, 2),
                  valueTestId: 'today-stats-secondary-in-progress-day-average',
                },
                {
                  label: t('dashboard.today.secondary.retry'),
                  valueSpec: buildNumberValueSpec(
                    inProgressRetryCount,
                    localeTag,
                    0,
                  ),
                  valueTestId: 'today-stats-secondary-in-progress-retry',
                },
              ]}
            />
          ) : null}
          <MetricTile
            label={t('dashboard.today.firstResponseTime')}
            description={t('dashboard.today.responseTimeDescription')}
            localeTag={localeTag}
            loading={loading || rateLoading}
            toneClass="text-secondary"
            iconName="timer-outline"
            valueTestId="today-stats-value-response-time"
            displaySpec={buildLatencyValueSpec(
              responseTimeCurrentUnavailable ? null : (responseTimeSnapshot?.responseTimeMs ?? null),
              localeTag,
            )}
            subdued={responseTimeCurrentUnavailable}
            topRightItem={{
              label: comparisonLabel,
              valueSpec: buildPercentValueSpec(rateUnavailable ? null : responseTimeDailyDelta, localeTag, { signDisplay: 'exceptZero' }),
              toneClass: latencyComparisonTone(rateUnavailable ? null : responseTimeDailyDelta),
              valueTestId: 'today-stats-secondary-response-time-delta',
            }}
            secondaryItems={[
              {
                label: t('dashboard.today.secondary.dayAverage'),
                valueSpec: buildLatencyValueSpec(
                  rateUnavailable ? null : (responseTimeSnapshot?.dayAverageMs ?? null),
                  localeTag,
                ),
                valueTestId: 'today-stats-secondary-response-time-day-average',
              },
              {
                label: t('dashboard.today.responseTime'),
                valueSpec: buildLatencyValueSpec(
                  recentWindowAvgTotalMs(timeseries, {
                    closedNaturalDay: dayKind === 'yesterday',
                    now,
                  }),
                  localeTag,
                ),
                valueTestId: 'today-stats-secondary-response-time-avg-total',
              },
            ]}
          />
          <MetricTile
            label={costLabel}
            description={t('dashboard.today.totalCostDescription')}
            value={totalCost}
            localeTag={localeTag}
            loading={loading}
            toneClass="text-accent"
            iconName="currency-usd"
            displaySpec={buildCurrencyAmountValueSpec(totalCost, localeTag)}
            valueTestId="today-stats-value-total-cost"
            topRightItem={{
              label: comparisonLabel,
              valueSpec: buildPercentValueSpec(totalCostDelta, localeTag, { signDisplay: 'exceptZero' }),
              toneClass: comparisonTone(totalCostDelta),
              valueTestId: 'today-stats-secondary-cost-delta',
            }}
            secondaryItems={[
              {
                label: t('dashboard.today.secondary.previous7dAverage'),
                valueSpec: buildCurrencyValueSpec(previous7dDailyCost, localeTag),
                valueTestId: 'today-stats-secondary-cost-previous7d-average',
              },
              {
                label: t('dashboard.today.secondary.failed'),
                valueSpec: buildCurrencyValueSpec(stats?.nonSuccessCost ?? null, localeTag),
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
            preserveLabelCase
            labelTestId="today-stats-label-total-tokens"
            iconName="database-outline"
            toneClass="text-secondary"
            valueTestId="today-stats-value-total-tokens"
            topRightItem={{
              label: comparisonLabel,
              valueSpec: buildPercentValueSpec(totalTokensDelta, localeTag, { signDisplay: 'exceptZero' }),
              toneClass: comparisonTone(totalTokensDelta),
              valueTestId: 'today-stats-secondary-tokens-delta',
            }}
            secondaryItems={[
              {
                label: t('dashboard.today.secondary.cacheHitRate'),
                valueSpec: buildRatioValueSpec(tokenCacheHitRate, localeTag),
                valueTestId: 'today-stats-secondary-cache-hit-rate',
              },
              {
                label: t('dashboard.today.secondary.failed'),
                valueSpec: buildNumberValueSpec(stats?.nonSuccessTokens ?? null, localeTag, 0),
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
