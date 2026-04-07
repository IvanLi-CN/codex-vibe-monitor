import { useState } from 'react'
import type { ParallelWorkStatsResponse, ParallelWorkWindowResponse } from '../lib/api'
import { Alert } from './ui/alert'
import { useTranslation } from '../i18n'
import { SegmentedControl, SegmentedControlItem } from './ui/segmented-control'

interface ParallelWorkStatsSectionProps {
  stats: ParallelWorkStatsResponse | null
  isLoading: boolean
  error: string | null
  defaultWindowKey?: ParallelWorkWindowKey
}

export type ParallelWorkWindowKey = 'minute7d' | 'hour30d' | 'dayAll'

const WINDOW_KEYS: ParallelWorkWindowKey[] = ['minute7d', 'hour30d', 'dayAll']

const SPARKLINE_WIDTH = 240
const SPARKLINE_HEIGHT = 68
const SPARKLINE_PADDING = 8

function resolveWindowMeta(key: ParallelWorkWindowKey) {
  switch (key) {
    case 'minute7d':
      return {
        titleKey: 'stats.parallelWork.windows.minute7d.title',
        descriptionKey: 'stats.parallelWork.windows.minute7d.description',
        toggleLabelKey: 'stats.parallelWork.windows.minute7d.toggleLabel',
      }
    case 'hour30d':
      return {
        titleKey: 'stats.parallelWork.windows.hour30d.title',
        descriptionKey: 'stats.parallelWork.windows.hour30d.description',
        toggleLabelKey: 'stats.parallelWork.windows.hour30d.toggleLabel',
      }
    case 'dayAll':
      return {
        titleKey: 'stats.parallelWork.windows.dayAll.title',
        descriptionKey: 'stats.parallelWork.windows.dayAll.description',
        toggleLabelKey: 'stats.parallelWork.windows.dayAll.toggleLabel',
      }
  }
}

function buildSparklinePath(points: ParallelWorkWindowResponse['points']) {
  if (points.length === 0) {
    return { linePath: '', areaPath: '', lastPoint: null as null | { x: number; y: number } }
  }

  const usableWidth = SPARKLINE_WIDTH - SPARKLINE_PADDING * 2
  const usableHeight = SPARKLINE_HEIGHT - SPARKLINE_PADDING * 2
  const maxCount = Math.max(...points.map((point) => point.parallelCount), 0)
  const baselineY = SPARKLINE_HEIGHT - SPARKLINE_PADDING
  const coords = points.map((point, index) => {
    const x =
      points.length === 1
        ? SPARKLINE_WIDTH / 2
        : SPARKLINE_PADDING + (usableWidth * index) / (points.length - 1)
    const ratio = maxCount <= 0 ? 0 : point.parallelCount / maxCount
    const y = baselineY - ratio * usableHeight
    return { x, y }
  })
  const linePath = coords
    .map((coord, index) => `${index === 0 ? 'M' : 'L'} ${coord.x.toFixed(2)} ${coord.y.toFixed(2)}`)
    .join(' ')
  const areaPath =
    coords.length === 1
      ? `${linePath} L ${coords[0].x.toFixed(2)} ${baselineY.toFixed(2)} Z`
      : `${linePath} L ${coords[coords.length - 1].x.toFixed(2)} ${baselineY.toFixed(2)} L ${coords[0].x.toFixed(2)} ${baselineY.toFixed(2)} Z`
  return {
    linePath,
    areaPath,
    lastPoint: coords[coords.length - 1] ?? null,
  }
}

function formatAverageCount(value: number | null, locale: string) {
  if (value == null) return '—'
  const formatter = new Intl.NumberFormat(locale, {
    minimumFractionDigits: Number.isInteger(value) ? 0 : 2,
    maximumFractionDigits: 2,
  })
  return formatter.format(value)
}

function formatWholeCount(value: number | null, locale: string) {
  if (value == null) return '—'
  return new Intl.NumberFormat(locale, { maximumFractionDigits: 0 }).format(value)
}

function ParallelWorkSparkline({
  window,
  emptyLabel,
  ariaLabel,
}: {
  window: ParallelWorkWindowResponse
  emptyLabel: string
  ariaLabel: string
}) {
  const { linePath, areaPath, lastPoint } = buildSparklinePath(window.points)

  if (window.points.length === 0) {
    return (
      <div className="flex h-20 items-center justify-center rounded-2xl border border-dashed border-base-300/75 bg-base-200/30 text-sm text-base-content/55">
        {emptyLabel}
      </div>
    )
  }

  return (
    <div className="rounded-2xl border border-base-300/75 bg-base-100/75 p-2.5">
      <svg
        viewBox={`0 0 ${SPARKLINE_WIDTH} ${SPARKLINE_HEIGHT}`}
        className="h-20 w-full"
        role="img"
        aria-label={ariaLabel}
        data-chart-kind="parallel-work-sparkline"
      >
        <line
          x1={SPARKLINE_PADDING}
          y1={SPARKLINE_HEIGHT - SPARKLINE_PADDING}
          x2={SPARKLINE_WIDTH - SPARKLINE_PADDING}
          y2={SPARKLINE_HEIGHT - SPARKLINE_PADDING}
          stroke="oklch(var(--color-base-content) / 0.14)"
          strokeWidth="1"
        />
        <path
          d={areaPath}
          fill="oklch(var(--color-primary) / 0.12)"
          stroke="none"
        />
        <path
          d={linePath}
          fill="none"
          stroke="oklch(var(--color-primary))"
          strokeWidth="2.5"
          strokeLinecap="round"
          strokeLinejoin="round"
        />
        {lastPoint ? (
          <circle
            cx={lastPoint.x}
            cy={lastPoint.y}
            r="3.5"
            fill="oklch(var(--color-primary))"
            stroke="oklch(var(--color-base-100) / 0.95)"
            strokeWidth="1.4"
          />
        ) : null}
      </svg>
    </div>
  )
}

function ParallelWorkWindowCard({
  windowKey,
  window,
}: {
  windowKey: ParallelWorkWindowKey
  window: ParallelWorkWindowResponse
}) {
  const { t, locale } = useTranslation()
  const meta = resolveWindowMeta(windowKey)
  const empty = window.completeBucketCount === 0

  return (
    <article
      className="flex min-h-[20rem] flex-col gap-4 rounded-[1.35rem] border border-base-300/75 bg-base-100/82 p-4 shadow-sm"
      data-testid={`parallel-work-card-${windowKey}`}
    >
      <div className="space-y-1.5">
        <h4 className="text-base font-semibold text-base-content">{t(meta.titleKey)}</h4>
        <p className="text-sm text-base-content/65">{t(meta.descriptionKey)}</p>
        <p className="text-xs text-base-content/52">
          {t('stats.parallelWork.samples', {
            complete: window.completeBucketCount,
            active: window.activeBucketCount,
          })}
        </p>
      </div>

      <div className="grid grid-cols-3 gap-2.5">
        <div className="rounded-2xl border border-base-300/70 bg-base-200/35 px-3 py-2.5">
          <div className="text-[11px] font-medium uppercase tracking-[0.08em] text-base-content/50">
            {t('stats.parallelWork.metrics.min')}
          </div>
          <div className="mt-1 text-xl font-semibold text-base-content">
            {formatWholeCount(window.minCount, locale)}
          </div>
        </div>
        <div className="rounded-2xl border border-base-300/70 bg-base-200/35 px-3 py-2.5">
          <div className="text-[11px] font-medium uppercase tracking-[0.08em] text-base-content/50">
            {t('stats.parallelWork.metrics.max')}
          </div>
          <div className="mt-1 text-xl font-semibold text-base-content">
            {formatWholeCount(window.maxCount, locale)}
          </div>
        </div>
        <div className="rounded-2xl border border-base-300/70 bg-base-200/35 px-3 py-2.5">
          <div className="text-[11px] font-medium uppercase tracking-[0.08em] text-base-content/50">
            {t('stats.parallelWork.metrics.avg')}
          </div>
          <div className="mt-1 text-xl font-semibold text-primary">
            {formatAverageCount(window.avgCount, locale)}
          </div>
        </div>
      </div>

      <ParallelWorkSparkline
        window={window}
        emptyLabel={t('stats.parallelWork.empty')}
        ariaLabel={t('stats.parallelWork.chartAria', {
          title: t(meta.titleKey),
        })}
      />

      {empty ? (
        <p className="rounded-2xl border border-dashed border-base-300/75 bg-base-200/20 px-3 py-2 text-sm text-base-content/58">
          {t('stats.parallelWork.empty')}
        </p>
      ) : (
        <div className="text-xs text-base-content/55">
          {t('stats.parallelWork.rangeSummary', {
            start: window.rangeStart,
            end: window.rangeEnd,
          })}
        </div>
      )}
    </article>
  )
}

function ParallelWorkLoadingCard({ windowKey }: { windowKey: ParallelWorkWindowKey }) {
  const { t } = useTranslation()
  const meta = resolveWindowMeta(windowKey)

  return (
    <article
      className="flex min-h-[20rem] flex-col gap-4 rounded-[1.35rem] border border-base-300/75 bg-base-100/82 p-4 shadow-sm"
      data-testid={`parallel-work-card-${windowKey}`}
    >
      <div className="space-y-1.5">
        <h4 className="text-base font-semibold text-base-content">{t(meta.titleKey)}</h4>
        <p className="text-sm text-base-content/65">{t(meta.descriptionKey)}</p>
        <div className="h-4 w-40 animate-pulse rounded-full bg-base-300/60" />
      </div>
      <div className="grid grid-cols-3 gap-2.5">
        {Array.from({ length: 3 }).map((_, index) => (
          <div
            key={index}
            className="rounded-2xl border border-base-300/70 bg-base-200/35 px-3 py-2.5"
          >
            <div className="h-3 w-10 animate-pulse rounded-full bg-base-300/60" />
            <div className="mt-2 h-7 w-12 animate-pulse rounded-full bg-base-300/60" />
          </div>
        ))}
      </div>
      <div className="flex h-20 items-center justify-center rounded-2xl border border-base-300/75 bg-base-100/75 p-2.5 text-sm text-base-content/55">
        {t('stats.parallelWork.loading')}
      </div>
      <div className="h-4 w-full animate-pulse rounded-full bg-base-300/60" />
    </article>
  )
}

export function ParallelWorkStatsSection({
  stats,
  isLoading,
  error,
  defaultWindowKey = 'minute7d',
}: ParallelWorkStatsSectionProps) {
  const { t } = useTranslation()
  const [activeWindowKey, setActiveWindowKey] = useState<ParallelWorkWindowKey>(defaultWindowKey)
  const activeWindow = stats?.[activeWindowKey] ?? null

  return (
    <section className="surface-panel" data-testid="parallel-work-section">
      <div className="surface-panel-body gap-4">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="section-heading">
            <h3 className="section-title">{t('stats.parallelWork.title')}</h3>
            <p className="section-description">{t('stats.parallelWork.description')}</p>
          </div>
          <div className="w-full overflow-x-auto no-scrollbar sm:w-auto">
            <SegmentedControl
              size="compact"
              className="min-w-max"
              role="tablist"
              aria-label={t('stats.parallelWork.windowToggleAria')}
              data-testid="parallel-work-window-toggle"
            >
              {WINDOW_KEYS.map((windowKey) => {
                const meta = resolveWindowMeta(windowKey)
                const active = windowKey === activeWindowKey
                return (
                  <SegmentedControlItem
                    key={windowKey}
                    active={active}
                    role="tab"
                    aria-selected={active}
                    onClick={() => setActiveWindowKey(windowKey)}
                    data-testid={`parallel-work-window-trigger-${windowKey}`}
                  >
                    {t(meta.toggleLabelKey)}
                  </SegmentedControlItem>
                )
              })}
            </SegmentedControl>
          </div>
        </div>
        {error ? (
          <Alert variant="error">{error}</Alert>
        ) : isLoading || !activeWindow ? (
          <ParallelWorkLoadingCard windowKey={activeWindowKey} />
        ) : (
          <ParallelWorkWindowCard windowKey={activeWindowKey} window={activeWindow} />
        )}
      </div>
    </section>
  )
}
