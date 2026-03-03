import { useMemo } from 'react'
import { useTranslation } from '../i18n'
import type {
  ForwardProxyHourlyBucket,
  ForwardProxyLiveNode,
  ForwardProxyLiveStatsResponse,
  ForwardProxyWeightBucket,
  ForwardProxyWindowStats,
} from '../lib/api'
import { cn } from '../lib/utils'
import { Alert } from './ui/alert'
import { Spinner } from './ui/spinner'

interface ForwardProxyLiveTableProps {
  stats: ForwardProxyLiveStatsResponse | null
  isLoading: boolean
  error?: string | null
}

function formatSuccessRate(value?: number) {
  if (value == null || Number.isNaN(value)) return '—'
  return `${(value * 100).toFixed(1)}%`
}

function formatLatency(value?: number) {
  if (value == null || Number.isNaN(value)) return '—'
  return `${value.toFixed(0)} ms`
}

function sumLast24h(node: ForwardProxyLiveNode) {
  return node.last24h.reduce(
    (acc, bucket) => {
      acc.success += bucket.successCount
      acc.failure += bucket.failureCount
      return acc
    },
    { success: 0, failure: 0 },
  )
}

function resolveWeightBuckets(node: ForwardProxyLiveNode): ForwardProxyWeightBucket[] {
  if (node.weight24h.length > 0) return node.weight24h
  if (node.last24h.length === 0) return []
  return node.last24h.map((bucket) => ({
    bucketStart: bucket.bucketStart,
    bucketEnd: bucket.bucketEnd,
    sampleCount: 0,
    minWeight: node.weight,
    maxWeight: node.weight,
    avgWeight: node.weight,
    lastWeight: node.weight,
  }))
}

function bucketTooltipLabel(bucket: ForwardProxyHourlyBucket, localeTag: string, successLabel: string, failureLabel: string) {
  const start = new Date(bucket.bucketStart)
  const end = new Date(bucket.bucketEnd)
  const formatter = new Intl.DateTimeFormat(localeTag, {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  })
  const startLabel = Number.isNaN(start.getTime()) ? bucket.bucketStart : formatter.format(start)
  const endLabel = Number.isNaN(end.getTime()) ? bucket.bucketEnd : formatter.format(end)
  return `${startLabel} - ${endLabel}\n${successLabel}: ${bucket.successCount}\n${failureLabel}: ${bucket.failureCount}`
}

interface WeightTooltipLabels {
  samples: string
  min: string
  max: string
  avg: string
  last: string
}

function formatWeight(value: number) {
  if (!Number.isFinite(value)) return '—'
  return value.toFixed(2)
}

function weightBucketTooltipLabel(bucket: ForwardProxyWeightBucket, localeTag: string, labels: WeightTooltipLabels) {
  const start = new Date(bucket.bucketStart)
  const end = new Date(bucket.bucketEnd)
  const formatter = new Intl.DateTimeFormat(localeTag, {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  })
  const startLabel = Number.isNaN(start.getTime()) ? bucket.bucketStart : formatter.format(start)
  const endLabel = Number.isNaN(end.getTime()) ? bucket.bucketEnd : formatter.format(end)
  return `${startLabel} - ${endLabel}\n${labels.samples}: ${bucket.sampleCount}\n${labels.min}: ${formatWeight(bucket.minWeight)}\n${labels.max}: ${formatWeight(bucket.maxWeight)}\n${labels.avg}: ${formatWeight(bucket.avgWeight)}\n${labels.last}: ${formatWeight(bucket.lastWeight)}`
}

interface WeightTrendGeometry {
  chartWidth: number
  chartHeight: number
  bucketWidth: number
  zeroY: number
  linePath: string
  areaPath: string
  points: Array<{ x: number; y: number }>
}

function buildWeightTrendGeometry(buckets: ForwardProxyWeightBucket[]): WeightTrendGeometry | null {
  if (buckets.length === 0) return null
  const chartWidth = 216
  const chartHeight = 40
  const values = buckets.map((bucket) => bucket.lastWeight)
  const minValue = Math.min(...values, 0)
  const maxValue = Math.max(...values, 0)
  const span = Math.max(maxValue - minValue, Number.EPSILON)
  const bucketWidth = chartWidth / buckets.length
  const points = values.map((value, index) => {
    const ratio = (value - minValue) / span
    const x = bucketWidth * index + bucketWidth / 2
    const y = chartHeight - ratio * chartHeight
    return { x, y }
  })
  const firstPoint = points[0]
  const lastPoint = points[points.length - 1]
  if (!firstPoint || !lastPoint) return null

  const zeroRatio = (0 - minValue) / span
  const zeroY = chartHeight - zeroRatio * chartHeight
  const zeroYClamped = Math.max(0, Math.min(chartHeight, zeroY))
  const linePath = points
    .map((point, index) => `${index === 0 ? 'M' : 'L'} ${point.x.toFixed(2)} ${point.y.toFixed(2)}`)
    .join(' ')
  const areaPath = `${linePath} L ${lastPoint.x.toFixed(2)} ${zeroYClamped.toFixed(2)} L ${firstPoint.x.toFixed(2)} ${zeroYClamped.toFixed(2)} Z`
  return {
    chartWidth,
    chartHeight,
    bucketWidth,
    zeroY: zeroYClamped,
    linePath,
    areaPath,
    points,
  }
}

function WindowCell({ value }: { value: ForwardProxyWindowStats }) {
  return (
    <div className="space-y-0.5 text-[11px] leading-tight">
      <div>{formatSuccessRate(value.successRate)}</div>
      <div className="text-base-content/65">{formatLatency(value.avgLatencyMs)}</div>
    </div>
  )
}

function WeightTrendCell({
  buckets,
  localeTag,
  tooltipLabels,
  ariaLabel,
  clipId,
}: {
  buckets: ForwardProxyWeightBucket[]
  localeTag: string
  tooltipLabels: WeightTooltipLabels
  ariaLabel: string
  clipId: string
}) {
  const geometry = buildWeightTrendGeometry(buckets)
  if (!geometry) {
    return <div className="text-[11px] text-base-content/55">—</div>
  }

  const positiveClipId = `${clipId}-positive`
  const negativeClipId = `${clipId}-negative`
  const positiveHeight = Math.max(geometry.zeroY, 0)
  const negativeHeight = Math.max(geometry.chartHeight - geometry.zeroY, 0)

  return (
    <div className="flex h-11 items-end">
      <svg
        viewBox={`0 0 ${geometry.chartWidth} ${geometry.chartHeight}`}
        className="block h-10 w-full rounded-md border border-base-300/55 bg-base-100/40"
        role="img"
        aria-label={ariaLabel}
      >
        <defs>
          <clipPath id={positiveClipId}>
            <rect x={0} y={0} width={geometry.chartWidth} height={positiveHeight} />
          </clipPath>
          <clipPath id={negativeClipId}>
            <rect x={0} y={geometry.zeroY} width={geometry.chartWidth} height={negativeHeight} />
          </clipPath>
        </defs>
        <line
          x1={0}
          y1={geometry.zeroY}
          x2={geometry.chartWidth}
          y2={geometry.zeroY}
          stroke="oklch(var(--color-base-content) / 0.15)"
          strokeWidth="1"
        />
        <path d={geometry.areaPath} fill="oklch(var(--color-success) / 0.18)" clipPath={`url(#${positiveClipId})`} />
        <path d={geometry.areaPath} fill="oklch(var(--color-error) / 0.16)" clipPath={`url(#${negativeClipId})`} />
        <path
          d={geometry.linePath}
          fill="none"
          stroke="oklch(var(--color-success) / 0.95)"
          clipPath={`url(#${positiveClipId})`}
          strokeWidth="1.6"
          strokeLinejoin="round"
          strokeLinecap="round"
        />
        <path
          d={geometry.linePath}
          fill="none"
          stroke="oklch(var(--color-error) / 0.92)"
          clipPath={`url(#${negativeClipId})`}
          strokeWidth="1.6"
          strokeLinejoin="round"
          strokeLinecap="round"
        />
        {geometry.points.map((point, index) => (
          <circle
            key={`${buckets[index]?.bucketStart ?? index}-dot`}
            cx={point.x}
            cy={point.y}
            r={1.5}
            fill={buckets[index]?.lastWeight >= 0 ? 'oklch(var(--color-success) / 0.95)' : 'oklch(var(--color-error) / 0.9)'}
          />
        ))}
        {buckets.map((bucket, index) => (
          <rect
            key={`${bucket.bucketStart}-hit`}
            x={geometry.bucketWidth * index}
            y={0}
            width={geometry.bucketWidth}
            height={geometry.chartHeight}
            fill="transparent"
          >
            <title>{weightBucketTooltipLabel(bucket, localeTag, tooltipLabels)}</title>
          </rect>
        ))}
      </svg>
    </div>
  )
}

export function ForwardProxyLiveTable({ stats, isLoading, error }: ForwardProxyLiveTableProps) {
  const { t, locale } = useTranslation()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const weightTrendAriaLabel = t('live.proxy.table.weightTrendAria')
  const weightTooltipLabels = useMemo(
    () => ({
      samples: t('live.proxy.table.weightTooltip.samples'),
      min: t('live.proxy.table.weightTooltip.min'),
      max: t('live.proxy.table.weightTooltip.max'),
      avg: t('live.proxy.table.weightTooltip.avg'),
      last: t('live.proxy.table.weightTooltip.last'),
    }),
    [t],
  )

  const rowData = useMemo(
    () =>
      (stats?.nodes ?? []).map((node) => ({
        node,
        windows: [node.stats.oneMinute, node.stats.fifteenMinutes, node.stats.oneHour, node.stats.oneDay, node.stats.sevenDays],
        total24h: sumLast24h(node),
        weightBuckets: resolveWeightBuckets(node),
        maxBucketTotal24h: Math.max(...node.last24h.map((bucket) => bucket.successCount + bucket.failureCount), 0),
      })),
    [stats?.nodes],
  )

  if (error) {
    return (
      <Alert variant="error">
        <span>{error}</span>
      </Alert>
    )
  }

  if (isLoading) {
    return (
      <div className="flex justify-center py-8">
        <Spinner size="lg" aria-label={t('chart.loadingDetailed')} />
      </div>
    )
  }

  if (rowData.length === 0) {
    return <Alert>{t('live.proxy.table.empty')}</Alert>
  }

  return (
    <div className="overflow-hidden rounded-xl border border-base-300/75 bg-base-100/55">
      <table className="w-full table-fixed text-[11px] sm:text-xs">
        <thead className="bg-base-200/70 uppercase tracking-[0.08em] text-base-content/65">
          <tr>
            <th className="w-[38%] px-2 py-2 text-left font-semibold sm:w-[30%] sm:px-3 sm:py-3 md:w-[18%] lg:w-[21%]">
              {t('live.proxy.table.proxy')}
            </th>
            <th className="w-[18%] px-1 py-2 text-center font-semibold sm:w-[13%] sm:px-2 sm:py-3 md:w-[8%] lg:w-[8%]">
              {t('live.proxy.table.oneMinute')}
            </th>
            <th className="hidden px-2 py-3 text-center font-semibold md:table-cell md:w-[8%] lg:w-[8%]">
              {t('live.proxy.table.fifteenMinutes')}
            </th>
            <th className="hidden px-2 py-3 text-center font-semibold md:table-cell md:w-[8%] lg:w-[8%]">
              {t('live.proxy.table.oneHour')}
            </th>
            <th className="hidden px-2 py-3 text-center font-semibold lg:table-cell lg:w-[8%]">
              {t('live.proxy.table.oneDay')}
            </th>
            <th className="hidden px-2 py-3 text-center font-semibold lg:table-cell lg:w-[8%]">
              {t('live.proxy.table.sevenDays')}
            </th>
            <th className="w-[24%] px-2 py-2 text-left font-semibold sm:w-[29%] sm:px-3 sm:py-3 md:w-[31%] lg:w-[21%]">
              {t('live.proxy.table.trend24h')}
            </th>
            <th className="w-[20%] px-2 py-2 text-left font-semibold sm:w-[28%] sm:px-3 sm:py-3 md:w-[27%] lg:w-[18%]">
              {t('live.proxy.table.weightTrend24h')}
            </th>
          </tr>
        </thead>
        <tbody className="divide-y divide-base-300/65">
          {rowData.map(({ node, windows, total24h, weightBuckets, maxBucketTotal24h }) => (
            <tr key={node.key} className={cn('transition-colors hover:bg-primary/6', node.penalized && 'bg-warning/8')}>
              <td className="max-w-0 px-2 py-2 align-middle sm:px-3 sm:py-3">
                <div className="min-w-0">
                  <div className="truncate whitespace-nowrap text-sm font-medium" title={node.displayName}>
                    {node.displayName}
                  </div>
                  <div className="mt-1 text-[11px] text-base-content/65">
                    {t('live.proxy.table.successShort', { count: total24h.success })}
                    {' / '}
                    {t('live.proxy.table.failureShort', { count: total24h.failure })}
                  </div>
                  <div className="mt-0.5 text-[11px] text-base-content/58">
                    {t('live.proxy.table.currentWeight', { value: formatWeight(node.weight) })}
                  </div>
                </div>
              </td>
              {windows.map((window, index) => (
                <td
                  key={`${node.key}-${index}`}
                  className={cn(
                    'px-1 py-2 text-center align-middle sm:px-2 sm:py-3',
                    index === 1 && 'hidden md:table-cell',
                    index === 2 && 'hidden md:table-cell',
                    index === 3 && 'hidden lg:table-cell',
                    index === 4 && 'hidden lg:table-cell',
                  )}
                >
                  <WindowCell value={window} />
                </td>
              ))}
              <td className="px-2 py-2 align-middle sm:px-3 sm:py-3">
                <div className="space-y-1">
                  <div className="flex h-11 items-end gap-px sm:gap-[1.5px] md:gap-[2px]">
                    {node.last24h.map((bucket, index) => {
                      const total = bucket.successCount + bucket.failureCount
                      const successHeight = maxBucketTotal24h > 0 ? (bucket.successCount / maxBucketTotal24h) * 100 : 0
                      const failureHeight = maxBucketTotal24h > 0 ? (bucket.failureCount / maxBucketTotal24h) * 100 : 0
                      const emptyHeight = Math.max(0, 100 - Math.round(successHeight + failureHeight))
                      return (
                        <div
                          key={`${node.key}-${index}`}
                          className="flex h-10 w-[2px] flex-col overflow-hidden rounded-[2px] bg-base-300/45 sm:w-[3px] md:w-[4px] lg:w-[6px]"
                          title={bucketTooltipLabel(
                            bucket,
                            localeTag,
                            t('stats.cards.success'),
                            t('stats.cards.failures'),
                          )}
                        >
                          <div
                            className="bg-transparent"
                            style={{ height: `${emptyHeight}%` }}
                          />
                          <div
                            className={cn(total > 0 ? 'bg-error/85' : 'bg-transparent')}
                            style={{ height: `${Math.round(failureHeight)}%` }}
                          />
                          <div
                            className={cn(total > 0 ? 'bg-success/85' : 'bg-transparent')}
                            style={{ height: `${Math.round(successHeight)}%` }}
                          />
                        </div>
                      )
                    })}
                  </div>
                </div>
              </td>
              <td className="px-2 py-2 align-middle sm:px-3 sm:py-3">
                <WeightTrendCell
                  buckets={weightBuckets}
                  localeTag={localeTag}
                  tooltipLabels={weightTooltipLabels}
                  ariaLabel={weightTrendAriaLabel}
                  clipId={`weight-trend-${node.key.replace(/[^a-zA-Z0-9_-]/g, '-')}`}
                />
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
