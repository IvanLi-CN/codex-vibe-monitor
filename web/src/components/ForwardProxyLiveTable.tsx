import { useCallback, useMemo, useState } from 'react'
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
import {
  ForwardProxyRequestTrendChart,
  type ForwardProxyRequestTooltipLabels,
} from './ForwardProxyRequestTrendChart'
import { InlineChartTooltipSurface, type InlineChartTooltipData } from './ui/inline-chart-tooltip'
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

function formatBucketRangeLabel(startRaw: string, endRaw: string, localeTag: string) {
  const start = new Date(startRaw)
  const end = new Date(endRaw)
  const formatter = new Intl.DateTimeFormat(localeTag, {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  })
  const startLabel = Number.isNaN(start.getTime()) ? startRaw : formatter.format(start)
  const endLabel = Number.isNaN(end.getTime()) ? endRaw : formatter.format(end)
  return `${startLabel} - ${endLabel}`
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

function buildWeightTooltipData(bucket: ForwardProxyWeightBucket, localeTag: string, labels: WeightTooltipLabels): InlineChartTooltipData {
  return {
    title: formatBucketRangeLabel(bucket.bucketStart, bucket.bucketEnd, localeTag),
    rows: [
      { label: labels.samples, value: String(bucket.sampleCount), tone: 'accent' },
      { label: labels.min, value: formatWeight(bucket.minWeight), tone: 'error' },
      { label: labels.max, value: formatWeight(bucket.maxWeight), tone: 'success' },
      { label: labels.avg, value: formatWeight(bucket.avgWeight), tone: 'accent' },
      { label: labels.last, value: formatWeight(bucket.lastWeight), tone: bucket.lastWeight >= 0 ? 'success' : 'error' },
    ],
  }
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

interface WeightTrendScale {
  minValue: number
  maxValue: number
}

function buildWeightTrendGeometry(buckets: ForwardProxyWeightBucket[], scale: WeightTrendScale): WeightTrendGeometry | null {
  if (buckets.length === 0) return null
  const chartWidth = 216
  const chartHeight = 40
  const values = buckets.map((bucket) => bucket.lastWeight)
  const minValue = scale.minValue
  const maxValue = scale.maxValue
  const span = Math.max(maxValue - minValue, Number.EPSILON)
  const bucketWidth = chartWidth / buckets.length
  const points = values.map((value, index) => {
    const ratio = Math.max(0, Math.min(1, (value - minValue) / span))
    const x = bucketWidth * index + bucketWidth / 2
    const y = chartHeight - ratio * chartHeight
    return { x, y }
  })
  const firstPoint = points[0]
  const lastPoint = points[points.length - 1]
  if (!firstPoint || !lastPoint) return null

  const zeroRatio = (0 - minValue) / span
  const zeroY = chartHeight - Math.max(0, Math.min(1, zeroRatio)) * chartHeight

  const linePath = points
    .map((point, index) => `${index === 0 ? 'M' : 'L'} ${point.x.toFixed(2)} ${point.y.toFixed(2)}`)
    .join(' ')
  const areaPath = `${linePath} L ${lastPoint.x.toFixed(2)} ${zeroY.toFixed(2)} L ${firstPoint.x.toFixed(2)} ${zeroY.toFixed(2)} Z`

  return {
    chartWidth,
    chartHeight,
    bucketWidth,
    zeroY,
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

function resolveLinkedActiveIndex<T extends { bucketStart: string }>(buckets: T[], activeBucketStart: string | null) {
  if (!activeBucketStart) return null
  const index = buckets.findIndex((bucket) => bucket.bucketStart === activeBucketStart)
  return index >= 0 ? index : null
}

function ProxyTrendCells({
  node,
  weightBuckets,
  requestBucketScaleMax,
  weightTrendScale,
  localeTag,
  requestTooltipLabels,
  weightTooltipLabels,
  requestTrendAriaLabel,
  weightTrendAriaLabel,
  chartInteractionHint,
}: {
  node: ForwardProxyLiveNode
  weightBuckets: ForwardProxyWeightBucket[]
  requestBucketScaleMax: number
  weightTrendScale: WeightTrendScale
  localeTag: string
  requestTooltipLabels: ForwardProxyRequestTooltipLabels
  weightTooltipLabels: WeightTooltipLabels
  requestTrendAriaLabel: string
  weightTrendAriaLabel: string
  chartInteractionHint: string
}) {
  const [activeBucketStart, setActiveBucketStart] = useState<string | null>(null)
  const linkedRequestIndex = useMemo(() => resolveLinkedActiveIndex(node.last24h, activeBucketStart), [activeBucketStart, node.last24h])
  const linkedWeightIndex = useMemo(() => resolveLinkedActiveIndex(weightBuckets, activeBucketStart), [activeBucketStart, weightBuckets])

  const handleRequestActiveIndexChange = useCallback(
    (index: number | null) => {
      setActiveBucketStart(index == null ? null : node.last24h[index]?.bucketStart ?? null)
    },
    [node.last24h],
  )

  const handleWeightActiveIndexChange = useCallback(
    (index: number | null) => {
      setActiveBucketStart(index == null ? null : weightBuckets[index]?.bucketStart ?? null)
    },
    [weightBuckets],
  )

  return (
    <>
      <td className="px-2 py-2 align-middle sm:px-3 sm:py-3">
        <RequestTrendCell
          buckets={node.last24h}
          scaleMax={requestBucketScaleMax}
          localeTag={localeTag}
          tooltipLabels={requestTooltipLabels}
          ariaLabel={`${node.displayName} ${requestTrendAriaLabel}`}
          interactionHint={chartInteractionHint}
          linkedActiveIndex={linkedRequestIndex}
          onActiveIndexChange={handleRequestActiveIndexChange}
        />
      </td>
      <td className="px-2 py-2 align-middle sm:px-3 sm:py-3">
        <WeightTrendCell
          buckets={weightBuckets}
          scale={weightTrendScale}
          localeTag={localeTag}
          tooltipLabels={weightTooltipLabels}
          ariaLabel={`${node.displayName} ${weightTrendAriaLabel}`}
          interactionHint={chartInteractionHint}
          clipId={`weight-trend-${node.key.replace(/[^a-zA-Z0-9_-]/g, '-')}`}
          linkedActiveIndex={linkedWeightIndex}
          onActiveIndexChange={handleWeightActiveIndexChange}
        />
      </td>
    </>
  )
}

function RequestTrendCell({
  buckets,
  scaleMax,
  localeTag,
  tooltipLabels,
  ariaLabel,
  interactionHint,
  linkedActiveIndex,
  onActiveIndexChange,
}: {
  buckets: ForwardProxyHourlyBucket[]
  scaleMax: number
  localeTag: string
  tooltipLabels: ForwardProxyRequestTooltipLabels
  ariaLabel: string
  interactionHint: string
  linkedActiveIndex?: number | null
  onActiveIndexChange?: (index: number | null) => void
}) {
  return (
    <ForwardProxyRequestTrendChart
      buckets={buckets}
      scaleMax={scaleMax}
      localeTag={localeTag}
      tooltipLabels={tooltipLabels}
      ariaLabel={ariaLabel}
      interactionHint={interactionHint}
      linkedActiveIndex={linkedActiveIndex}
      onActiveIndexChange={onActiveIndexChange}
      variant="table"
      dataChartKind="proxy-request-trend"
      emptyState={<div className="text-[11px] text-base-content/55">—</div>}
    />
  )
}

function WeightTrendCell({
  buckets,
  scale,
  localeTag,
  tooltipLabels,
  ariaLabel,
  interactionHint,
  clipId,
  linkedActiveIndex,
  onActiveIndexChange,
}: {
  buckets: ForwardProxyWeightBucket[]
  scale: WeightTrendScale
  localeTag: string
  tooltipLabels: WeightTooltipLabels
  ariaLabel: string
  interactionHint: string
  clipId: string
  linkedActiveIndex?: number | null
  onActiveIndexChange?: (index: number | null) => void
}) {
  const geometry = buildWeightTrendGeometry(buckets, scale)
  const tooltipData = useMemo(() => buckets.map((bucket) => buildWeightTooltipData(bucket, localeTag, tooltipLabels)), [buckets, localeTag, tooltipLabels])
  if (!geometry) {
    return <div className="text-[11px] text-base-content/55">—</div>
  }

  const positiveClipId = `${clipId}-positive`
  const negativeClipId = `${clipId}-negative`
  const positiveHeight = Math.max(geometry.zeroY, 0)
  const negativeHeight = Math.max(geometry.chartHeight - geometry.zeroY, 0)
  const defaultIndex = Math.max(0, buckets.length - 1)

  return (
    <InlineChartTooltipSurface
      items={tooltipData}
      defaultIndex={defaultIndex}
      ariaLabel={ariaLabel}
      interactionHint={interactionHint}
      linkedActiveIndex={linkedActiveIndex}
      onActiveIndexChange={onActiveIndexChange}
      className="py-0.5"
      chartClassName="flex h-11 items-end"
    >
      {({ highlightedIndex, getItemProps }) => {
        const activePoint = highlightedIndex != null ? geometry.points[highlightedIndex] : null
        const activeBucket = highlightedIndex != null ? buckets[highlightedIndex] : null
        return (
          <svg
            viewBox={`0 0 ${geometry.chartWidth} ${geometry.chartHeight}`}
            className="block h-10 w-full rounded-md border border-base-300/55 bg-base-100/40"
            data-chart-kind="proxy-weight-trend"
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
            {activePoint ? (
              <line
                x1={activePoint.x}
                y1={0}
                x2={activePoint.x}
                y2={geometry.chartHeight}
                stroke="oklch(var(--color-primary) / 0.45)"
                strokeWidth="1"
                strokeDasharray="3 2"
              />
            ) : null}
            <path d={geometry.areaPath} fill="oklch(var(--color-success) / 0.18)" clipPath={`url(#${positiveClipId})`} />
            <path d={geometry.areaPath} fill="oklch(var(--color-error) / 0.16)" clipPath={`url(#${negativeClipId})`} />
            <path
              d={geometry.linePath}
              fill="none"
              stroke="oklch(var(--color-success) / 0.95)"
              clipPath={`url(#${positiveClipId})`}
              strokeWidth={activePoint ? '1.9' : '1.6'}
              strokeLinejoin="round"
              strokeLinecap="round"
            />
            <path
              d={geometry.linePath}
              fill="none"
              stroke="oklch(var(--color-error) / 0.92)"
              clipPath={`url(#${negativeClipId})`}
              strokeWidth={activePoint ? '1.9' : '1.6'}
              strokeLinejoin="round"
              strokeLinecap="round"
            />
            {geometry.points.map((point, index) => {
              const isActive = highlightedIndex === index
              const isPositive = (buckets[index]?.lastWeight ?? 0) >= 0
              return (
                <circle
                  key={`${buckets[index]?.bucketStart ?? index}-dot`}
                  cx={point.x}
                  cy={point.y}
                  r={isActive ? 2.6 : 1.5}
                  fill={isPositive ? 'oklch(var(--color-success) / 0.95)' : 'oklch(var(--color-error) / 0.9)'}
                  stroke={isActive ? 'oklch(var(--color-base-100) / 0.95)' : 'none'}
                  strokeWidth={isActive ? '1.2' : '0'}
                />
              )
            })}
            {activePoint && activeBucket ? (
              <circle
                cx={activePoint.x}
                cy={activePoint.y}
                r="4"
                fill="none"
                stroke={activeBucket.lastWeight >= 0 ? 'oklch(var(--color-success) / 0.45)' : 'oklch(var(--color-error) / 0.45)'}
                strokeWidth="1"
              />
            ) : null}
            {buckets.map((bucket, index) => (
              <rect
                key={`${bucket.bucketStart}-hit`}
                x={geometry.bucketWidth * index}
                y={0}
                width={geometry.bucketWidth}
                height={geometry.chartHeight}
                fill="transparent"
                className="cursor-pointer"
                {...getItemProps(index)}
              />
            ))}
          </svg>
        )
      }}
    </InlineChartTooltipSurface>
  )
}

export function ForwardProxyLiveTable({ stats, isLoading, error }: ForwardProxyLiveTableProps) {
  const { t, locale } = useTranslation()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const weightTrendAriaLabel = t('live.proxy.table.weightTrendAria')
  const requestTrendAriaLabel = t('live.proxy.table.requestTrendAria')
  const chartInteractionHint = t('live.chart.tooltip.instructions')
  const requestTooltipLabels = useMemo(
    () => ({
      success: t('stats.cards.success'),
      failure: t('stats.cards.failures'),
      total: t('live.proxy.table.requestTooltip.total'),
    }),
    [t],
  )
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

  const { rowData, requestBucketScaleMax, weightTrendScale } = useMemo(() => {
    const rows = (stats?.nodes ?? []).map((node) => {
      const weightBuckets = resolveWeightBuckets(node)
      return {
        node,
        windows: [node.stats.oneMinute, node.stats.fifteenMinutes, node.stats.oneHour, node.stats.oneDay, node.stats.sevenDays],
        total24h: sumLast24h(node),
        weightBuckets,
      }
    })
    const requestBucketScaleMax = Math.max(
      ...rows.flatMap(({ node }) => node.last24h.map((bucket) => bucket.successCount + bucket.failureCount)),
      0,
    )
    const hasRealWeightHistory = rows.some(({ node }) => node.weight24h.length > 0)
    const allWeightValues = (hasRealWeightHistory ? rows.flatMap(({ node }) => node.weight24h) : rows.flatMap(({ weightBuckets }) => weightBuckets)).flatMap(
      (bucket) => [bucket.minWeight, bucket.maxWeight, bucket.lastWeight],
    )
    const minValue = Math.min(...allWeightValues, 0)
    const maxValue = Math.max(...allWeightValues, 0)
    const padding = Math.max((maxValue - minValue) * 0.08, 0.2)
    return {
      rowData: rows,
      requestBucketScaleMax,
      weightTrendScale: {
        minValue: minValue - padding,
        maxValue: maxValue + padding,
      },
    }
  }, [stats])

  if (isLoading && !stats) {
    return (
      <div className="flex min-h-[240px] items-center justify-center rounded-2xl border border-base-300/75 bg-base-100/55">
        <Spinner size="lg" aria-label={t('chart.loadingDetailed')} />
      </div>
    )
  }

  if (error) {
    return <Alert variant="error">{t('table.loadError', { error })}</Alert>
  }

  if (!stats || stats.nodes.length === 0) {
    return <Alert>{t('live.proxy.table.empty')}</Alert>
  }

  return (
    <div className="overflow-x-auto rounded-2xl border border-base-300/75 bg-base-100/55">
      <table className="w-full min-w-[1180px] table-fixed text-xs sm:min-w-[1260px] lg:min-w-0">
        <thead className="bg-base-200/70 uppercase tracking-[0.08em] text-base-content/65">
          <tr>
            <th className="w-[18%] px-2 py-2 text-left font-semibold sm:w-[30%] sm:px-3 sm:py-3 md:w-[18%] lg:w-[21%]">
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
          {rowData.map(({ node, windows, total24h, weightBuckets }) => (
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
              <ProxyTrendCells
                node={node}
                weightBuckets={weightBuckets}
                requestBucketScaleMax={requestBucketScaleMax}
                weightTrendScale={weightTrendScale}
                localeTag={localeTag}
                requestTooltipLabels={requestTooltipLabels}
                weightTooltipLabels={weightTooltipLabels}
                requestTrendAriaLabel={requestTrendAriaLabel}
                weightTrendAriaLabel={weightTrendAriaLabel}
                chartInteractionHint={chartInteractionHint}
              />
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}
