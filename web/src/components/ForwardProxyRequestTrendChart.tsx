import { type ReactNode, useMemo } from 'react'
import type { ForwardProxyHourlyBucket } from '../lib/api'
import { cn } from '../lib/utils'
import { InlineChartTooltipSurface, type InlineChartTooltipData } from './ui/inline-chart-tooltip'

export interface ForwardProxyRequestTooltipLabels {
  success: string
  failure: string
  total: string
}

interface ForwardProxyRequestTrendChartProps {
  buckets: ForwardProxyHourlyBucket[]
  scaleMax: number
  localeTag: string
  tooltipLabels: ForwardProxyRequestTooltipLabels
  ariaLabel: string
  interactionHint: string
  linkedActiveIndex?: number | null
  onActiveIndexChange?: (index: number | null) => void
  variant?: 'table' | 'dialog'
  className?: string
  emptyState?: ReactNode
  dataChartKind?: string
}

function buildVisibleBarHeights(successCount: number, failureCount: number, scaleMax: number, totalHeightPx: number) {
  if (scaleMax <= 0 || totalHeightPx <= 0) {
    return { empty: totalHeightPx, failure: 0, success: 0 }
  }

  let success = successCount > 0 ? Math.max((successCount / scaleMax) * totalHeightPx, 1) : 0
  let failure = failureCount > 0 ? Math.max((failureCount / scaleMax) * totalHeightPx, 1) : 0
  const maxVisible = Math.max(totalHeightPx, 0)
  let overflow = success + failure - maxVisible

  const shrink = (value: number, minVisible: number, amount: number) => {
    if (amount <= 0 || value <= minVisible) return { nextValue: value, remaining: amount }
    const delta = Math.min(value - minVisible, amount)
    return { nextValue: value - delta, remaining: amount - delta }
  }

  if (overflow > 0) {
    const first = success >= failure ? 'success' : 'failure'
    const second = first == 'success' ? 'failure' : 'success'
    for (const key of [first, second] as const) {
      const minVisible = key == 'success' ? (successCount > 0 ? 1 : 0) : failureCount > 0 ? 1 : 0
      const current = key == 'success' ? success : failure
      const result = shrink(current, minVisible, overflow)
      if (key == 'success') {
        success = result.nextValue
      } else {
        failure = result.nextValue
      }
      overflow = result.remaining
    }
  }

  const used = Math.min(success + failure, maxVisible)
  return {
    empty: Math.max(maxVisible - used, 0),
    failure,
    success,
  }
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

function buildRequestTooltipData(
  bucket: ForwardProxyHourlyBucket,
  localeTag: string,
  labels: ForwardProxyRequestTooltipLabels,
  numberFormatter: Intl.NumberFormat,
): InlineChartTooltipData {
  const total = bucket.successCount + bucket.failureCount
  return {
    title: formatBucketRangeLabel(bucket.bucketStart, bucket.bucketEnd, localeTag),
    rows: [
      { label: labels.success, value: numberFormatter.format(bucket.successCount), tone: 'success' },
      { label: labels.failure, value: numberFormatter.format(bucket.failureCount), tone: 'error' },
      { label: labels.total, value: numberFormatter.format(total), tone: 'accent' },
    ],
  }
}

function resolveRequestDefaultIndex(buckets: ForwardProxyHourlyBucket[]) {
  for (let index = buckets.length - 1; index >= 0; index -= 1) {
    const bucket = buckets[index]
    if ((bucket?.successCount ?? 0) + (bucket?.failureCount ?? 0) > 0) return index
  }
  return Math.max(0, buckets.length - 1)
}

export function ForwardProxyRequestTrendChart({
  buckets,
  scaleMax,
  localeTag,
  tooltipLabels,
  ariaLabel,
  interactionHint,
  linkedActiveIndex = null,
  onActiveIndexChange,
  variant = 'table',
  className,
  emptyState,
  dataChartKind = 'proxy-request-trend',
}: ForwardProxyRequestTrendChartProps) {
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag), [localeTag])
  const tooltipData = useMemo(
    () => buckets.map((bucket) => buildRequestTooltipData(bucket, localeTag, tooltipLabels, numberFormatter)),
    [buckets, localeTag, numberFormatter, tooltipLabels],
  )
  const defaultIndex = useMemo(() => resolveRequestDefaultIndex(buckets), [buckets])

  if (buckets.length === 0) {
    return emptyState ? <>{emptyState}</> : <div className="text-[11px] text-base-content/55">-</div>
  }

  const isDialog = variant === 'dialog'
  const chartHeight = isDialog ? 20 : 40

  return (
    <InlineChartTooltipSurface
      items={tooltipData}
      defaultIndex={defaultIndex}
      ariaLabel={ariaLabel}
      interactionHint={interactionHint}
      linkedActiveIndex={linkedActiveIndex}
      onActiveIndexChange={onActiveIndexChange}
      className={cn(isDialog ? 'py-0' : 'py-0.5', className)}
      chartClassName={isDialog ? 'flex h-8 min-w-0 w-full items-end' : 'flex h-11 items-end'}
    >
      {({ highlightedIndex, getItemProps }) => (
        <div
          className={cn(
            isDialog
              ? 'flex h-8 min-w-0 w-full items-end gap-px rounded-xl border border-base-300/70 bg-base-100/70 px-1.5 py-1'
              : 'flex h-11 items-end gap-px sm:gap-[1.5px] md:gap-[2px]',
          )}
          data-chart-kind={dataChartKind}
        >
          {buckets.map((bucket, index) => {
            const total = bucket.successCount + bucket.failureCount
            const heights = buildVisibleBarHeights(
              bucket.successCount,
              bucket.failureCount,
              scaleMax,
              chartHeight,
            )
            const isActive = highlightedIndex === index
            return (
              <div
                key={`${bucket.bucketStart}-bar`}
                className={cn(
                  isDialog
                    ? 'relative flex h-5 min-w-0 flex-1 cursor-pointer flex-col overflow-hidden rounded-[3px] border border-transparent bg-base-300/35 transition-[transform,background-color,border-color,box-shadow] duration-150 ease-out motion-reduce:transition-none'
                    : 'relative flex h-10 w-[2px] cursor-pointer flex-col overflow-hidden rounded-[3px] border border-transparent bg-base-300/45 transition-[transform,background-color,border-color,box-shadow] duration-150 ease-out motion-reduce:transition-none sm:w-[3px] md:w-[4px] lg:w-[6px]',
                  isActive && 'z-[1] border-primary/60 bg-base-200/70 shadow-[0_0_0_1px_rgba(96,165,250,0.15)]',
                )}
                {...getItemProps(index)}
              >
                {isActive ? <span className="absolute inset-0 bg-primary/8" aria-hidden="true" /> : null}
                <div className="bg-transparent" style={{ height: `${heights.empty}px` }} />
                <div className={cn(total > 0 ? 'bg-error/85' : 'bg-transparent')} style={{ height: `${heights.failure}px` }} />
                <div className={cn(total > 0 ? 'bg-success/85' : 'bg-transparent')} style={{ height: `${heights.success}px` }} />
              </div>
            )
          })}
        </div>
      )}
    </InlineChartTooltipSurface>
  )
}
