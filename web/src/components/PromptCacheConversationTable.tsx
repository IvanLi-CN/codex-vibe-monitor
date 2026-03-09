import { useMemo } from 'react'
import { useTranslation } from '../i18n'
import type {
  PromptCacheConversation,
  PromptCacheConversationRequestPoint,
  PromptCacheConversationsResponse,
} from '../lib/api'
import { Alert } from './ui/alert'
import { InlineChartTooltipSurface, type InlineChartTooltipData } from './ui/inline-chart-tooltip'
import { Spinner } from './ui/spinner'

interface PromptCacheConversationTableProps {
  stats: PromptCacheConversationsResponse | null
  isLoading: boolean
  error?: string | null
}

interface ConversationChartSegment {
  startEpoch: number
  endEpoch: number
  cumulativeTokens: number
  isSuccess: boolean
  point: PromptCacheConversationRequestPoint
}

interface ConversationChartGeometry {
  width: number
  height: number
  segments: Array<ConversationChartSegment & { x1: number; x2: number; y: number }>
  jumps: Array<{ x: number; y1: number; y2: number }>
}

const CHART_WIDTH = 232
const CHART_HEIGHT = 48
const FALLBACK_CELL = '—'

function parseEpoch(raw?: string | null) {
  if (!raw) return null
  const epoch = Date.parse(raw)
  if (Number.isNaN(epoch)) return null
  return Math.floor(epoch / 1000)
}

function formatNumber(value: number, formatter: Intl.NumberFormat) {
  if (!Number.isFinite(value)) return FALLBACK_CELL
  return formatter.format(value)
}

function formatDateLabel(raw: string, formatter: Intl.DateTimeFormat) {
  const value = new Date(raw)
  if (Number.isNaN(value.getTime())) return raw || FALLBACK_CELL
  return formatter.format(value)
}

function buildSegments(
  points: PromptCacheConversationRequestPoint[],
  rangeStartEpoch: number,
  rangeEndEpoch: number,
): ConversationChartSegment[] {
  if (points.length === 0 || rangeEndEpoch <= rangeStartEpoch) return []
  const sorted = [...points].sort((a, b) => {
    const aEpoch = parseEpoch(a.occurredAt) ?? 0
    const bEpoch = parseEpoch(b.occurredAt) ?? 0
    return aEpoch - bEpoch
  })

  const segments: ConversationChartSegment[] = []
  for (let index = 0; index < sorted.length; index += 1) {
    const current = sorted[index]
    const next = sorted[index + 1]
    const currentEpoch = parseEpoch(current.occurredAt)
    if (currentEpoch == null) continue
    const startEpoch = Math.max(rangeStartEpoch, Math.min(rangeEndEpoch, currentEpoch))
    const nextEpoch = next ? (parseEpoch(next.occurredAt) ?? rangeEndEpoch) : rangeEndEpoch
    const endEpoch = Math.max(startEpoch, Math.min(rangeEndEpoch, nextEpoch))
    if (endEpoch <= startEpoch) continue

    segments.push({
      startEpoch,
      endEpoch,
      cumulativeTokens: Math.max(0, current.cumulativeTokens),
      isSuccess: current.isSuccess,
      point: current,
    })
  }

  return segments
}

function resolveRangeEpochs(rangeStart: string, rangeEnd: string) {
  const rangeStartEpoch = parseEpoch(rangeStart)
  const rangeEndEpoch = parseEpoch(rangeEnd)
  if (rangeStartEpoch == null || rangeEndEpoch == null || rangeEndEpoch <= rangeStartEpoch) return null
  return { rangeStartEpoch, rangeEndEpoch }
}

function findVisibleConversationChartMax(conversations: PromptCacheConversation[], rangeStart: string, rangeEnd: string) {
  const range = resolveRangeEpochs(rangeStart, rangeEnd)
  if (!range) return 0
  return Math.max(
    ...conversations.flatMap((conversation) =>
      buildSegments(conversation.last24hRequests, range.rangeStartEpoch, range.rangeEndEpoch).map((segment) => segment.cumulativeTokens),
    ),
    0,
  )
}

function buildGeometry(
  points: PromptCacheConversationRequestPoint[],
  rangeStart: string,
  rangeEnd: string,
  maxCumulativeTokens: number,
): ConversationChartGeometry | null {
  const range = resolveRangeEpochs(rangeStart, rangeEnd)
  if (!range) return null

  const segments = buildSegments(points, range.rangeStartEpoch, range.rangeEndEpoch)
  if (segments.length === 0) return null

  const maxCumulative = Math.max(maxCumulativeTokens, ...segments.map((segment) => segment.cumulativeTokens), 1)
  const span = range.rangeEndEpoch - range.rangeStartEpoch
  const xForEpoch = (epoch: number) => ((epoch - range.rangeStartEpoch) / span) * CHART_WIDTH
  const yForTokens = (tokens: number) => CHART_HEIGHT - (tokens / maxCumulative) * CHART_HEIGHT

  const positioned = segments.map((segment) => ({
    ...segment,
    x1: xForEpoch(segment.startEpoch),
    x2: xForEpoch(segment.endEpoch),
    y: yForTokens(segment.cumulativeTokens),
  }))

  const jumps = positioned
    .slice(1)
    .map((segment, index) => ({
      x: segment.x1,
      y1: positioned[index]?.y ?? segment.y,
      y2: segment.y,
    }))

  return {
    width: CHART_WIDTH,
    height: CHART_HEIGHT,
    segments: positioned,
    jumps,
  }
}

function buildConversationTooltipData(
  point: PromptCacheConversationRequestPoint,
  localeTag: string,
  labels: {
    status: string
    requestTokens: string
    cumulativeTokens: string
  },
  numberFormatter: Intl.NumberFormat,
): InlineChartTooltipData {
  const occurredAt = new Date(point.occurredAt)
  const timeLabel = Number.isNaN(occurredAt.getTime())
    ? point.occurredAt
    : occurredAt.toLocaleString(localeTag, {
        month: '2-digit',
        day: '2-digit',
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
        hour12: false,
      })
  return {
    title: timeLabel,
    rows: [
      { label: labels.status, value: point.status, tone: point.isSuccess ? 'success' : 'error' },
      { label: labels.requestTokens, value: numberFormatter.format(point.requestTokens), tone: 'accent' },
      { label: labels.cumulativeTokens, value: numberFormatter.format(point.cumulativeTokens), tone: point.isSuccess ? 'success' : 'error' },
    ],
  }
}

function ConversationSparkline({
  conversation,
  rangeStart,
  rangeEnd,
  maxCumulativeTokens,
  localeTag,
  tooltipLabels,
  interactionHint,
  ariaLabel,
}: {
  conversation: PromptCacheConversation
  rangeStart: string
  rangeEnd: string
  maxCumulativeTokens: number
  localeTag: string
  tooltipLabels: {
    status: string
    requestTokens: string
    cumulativeTokens: string
  }
  interactionHint: string
  ariaLabel: string
}) {
  const geometry = useMemo(
    () => buildGeometry(conversation.last24hRequests, rangeStart, rangeEnd, maxCumulativeTokens),
    [conversation.last24hRequests, maxCumulativeTokens, rangeEnd, rangeStart],
  )
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag), [localeTag])
  const tooltipData = useMemo(
    () =>
      geometry?.segments.map((segment) => buildConversationTooltipData(segment.point, localeTag, tooltipLabels, numberFormatter)) ?? [],
    [geometry?.segments, localeTag, numberFormatter, tooltipLabels],
  )

  if (!geometry) {
    return <div className="text-[11px] text-base-content/55">{FALLBACK_CELL}</div>
  }

  const defaultIndex = Math.max(0, geometry.segments.length - 1)

  return (
    <InlineChartTooltipSurface
      items={tooltipData}
      defaultIndex={defaultIndex}
      ariaLabel={ariaLabel}
      interactionHint={interactionHint}
      className="h-12 py-0.5"
      chartClassName="h-12"
    >
      {({ activeIndex, getItemProps }) => {
        const activeSegment = activeIndex != null ? geometry.segments[activeIndex] : null
        return (
          <svg
            viewBox={`0 0 ${geometry.width} ${geometry.height}`}
            className="h-11 w-full rounded-md border border-base-300/55 bg-base-100/35"
            data-chart-kind="prompt-cache-sparkline"
          >
            <line
              x1={0}
              y1={geometry.height}
              x2={geometry.width}
              y2={geometry.height}
              stroke="oklch(var(--color-base-content) / 0.16)"
              strokeWidth="1"
            />
            {activeSegment ? (
              <line
                x1={activeSegment.x1}
                y1={0}
                x2={activeSegment.x1}
                y2={geometry.height}
                stroke="oklch(var(--color-primary) / 0.45)"
                strokeWidth="1"
                strokeDasharray="3 2"
              />
            ) : null}
            {geometry.jumps.map((jump, index) => (
              <line
                key={`jump-${index}`}
                x1={jump.x}
                y1={jump.y1}
                x2={jump.x}
                y2={jump.y2}
                stroke="oklch(var(--color-base-content) / 0.28)"
                strokeWidth="1"
              />
            ))}
            {geometry.segments.map((segment, index) => {
              const isActive = activeIndex === index
              return (
                <g key={`${conversation.promptCacheKey}-segment-${index}`}>
                  <line
                    x1={segment.x1}
                    y1={segment.y}
                    x2={segment.x2}
                    y2={segment.y}
                    stroke={segment.isSuccess ? 'oklch(var(--color-success) / 0.95)' : 'oklch(var(--color-error) / 0.92)'}
                    strokeWidth={isActive ? '4' : '2.8'}
                    strokeLinecap="round"
                  />
                  {isActive ? (
                    <circle
                      cx={segment.x1}
                      cy={segment.y}
                      r="3"
                      fill={segment.isSuccess ? 'oklch(var(--color-success) / 0.95)' : 'oklch(var(--color-error) / 0.92)'}
                      stroke="oklch(var(--color-base-100) / 0.95)"
                      strokeWidth="1.2"
                    />
                  ) : null}
                  <rect
                    x={Math.min(segment.x1, segment.x2)}
                    y={0}
                    width={Math.max(Math.abs(segment.x2 - segment.x1), 2)}
                    height={geometry.height}
                    fill="transparent"
                    className="cursor-pointer"
                    {...getItemProps(index)}
                  />
                </g>
              )
            })}
          </svg>
        )
      }}
    </InlineChartTooltipSurface>
  )
}

export function PromptCacheConversationTable({ stats, isLoading, error }: PromptCacheConversationTableProps) {
  const { t, locale } = useTranslation()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag), [localeTag])
  const currencyFormatter = useMemo(
    () => new Intl.NumberFormat(localeTag, { style: 'currency', currency: 'USD', maximumFractionDigits: 4 }),
    [localeTag],
  )
  const dateFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(localeTag, {
        month: '2-digit',
        day: '2-digit',
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
        hour12: false,
      }),
    [localeTag],
  )

  const tooltipLabels = useMemo(
    () => ({
      status: t('live.conversations.chart.tooltip.status'),
      requestTokens: t('live.conversations.chart.tooltip.requestTokens'),
      cumulativeTokens: t('live.conversations.chart.tooltip.cumulativeTokens'),
    }),
    [t],
  )
  const chartInteractionHint = t('live.chart.tooltip.instructions')
  const chartAriaLabel = t('live.conversations.chartAria')

  const rangeStart = stats?.rangeStart ?? ''
  const rangeEnd = stats?.rangeEnd ?? ''
  const conversationChartMax = useMemo(
    () => findVisibleConversationChartMax(stats?.conversations ?? [], rangeStart, rangeEnd),
    [rangeEnd, rangeStart, stats?.conversations],
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

  if (!stats || stats.conversations.length === 0) {
    return <Alert>{t('live.conversations.empty')}</Alert>
  }

  return (
    <div className="overflow-hidden rounded-xl border border-base-300/75 bg-base-100/55">
      <div className="space-y-3 p-3 sm:hidden">
        {stats.conversations.map((conversation) => {
          const createdAtLabel = formatDateLabel(conversation.createdAt, dateFormatter)
          const lastActivityLabel = formatDateLabel(conversation.lastActivityAt, dateFormatter)

          return (
            <article
              key={`${conversation.promptCacheKey}-mobile`}
              className="space-y-3 rounded-lg border border-base-300/70 bg-base-100/70 p-3"
            >
              <div className="space-y-1">
                <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                  {t('live.conversations.table.promptCacheKey')}
                </div>
                <div className="break-all font-mono text-xs">{conversation.promptCacheKey}</div>
              </div>

              <dl className="grid grid-cols-2 gap-x-3 gap-y-2 text-xs">
                <div>
                  <dt className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                    {t('live.conversations.table.requestCount')}
                  </dt>
                  <dd>{formatNumber(conversation.requestCount, numberFormatter)}</dd>
                </div>
                <div>
                  <dt className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                    {t('live.conversations.table.totalTokens')}
                  </dt>
                  <dd>{formatNumber(conversation.totalTokens, numberFormatter)}</dd>
                </div>
                <div>
                  <dt className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                    {t('live.conversations.table.totalCost')}
                  </dt>
                  <dd>
                    {Number.isFinite(conversation.totalCost) ? currencyFormatter.format(conversation.totalCost) : FALLBACK_CELL}
                  </dd>
                </div>
                <div>
                  <dt className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                    {t('live.conversations.table.createdAt')}
                  </dt>
                  <dd>{createdAtLabel}</dd>
                </div>
                <div className="col-span-2">
                  <dt className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                    {t('live.conversations.table.lastActivityAt')}
                  </dt>
                  <dd>{lastActivityLabel}</dd>
                </div>
              </dl>

              <div className="space-y-1">
                <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">
                  {t('live.conversations.table.chart24h')}
                </div>
                <ConversationSparkline
                  conversation={conversation}
                  rangeStart={rangeStart}
                  rangeEnd={rangeEnd}
                  maxCumulativeTokens={conversationChartMax}
                  localeTag={localeTag}
                  tooltipLabels={tooltipLabels}
                  interactionHint={chartInteractionHint}
                  ariaLabel={`${conversation.promptCacheKey} ${chartAriaLabel}`}
                />
              </div>
            </article>
          )
        })}
      </div>

      <table className="hidden w-full table-fixed text-xs sm:table">
        <thead className="bg-base-200/70 uppercase tracking-[0.08em] text-base-content/65">
          <tr>
            <th className="w-[22%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
              {t('live.conversations.table.promptCacheKey')}
            </th>
            <th className="w-[8%] px-2 py-2 text-right font-semibold sm:px-3 sm:py-3">
              {t('live.conversations.table.requestCount')}
            </th>
            <th className="w-[12%] px-2 py-2 text-right font-semibold sm:px-3 sm:py-3">
              {t('live.conversations.table.totalTokens')}
            </th>
            <th className="w-[12%] px-2 py-2 text-right font-semibold sm:px-3 sm:py-3">
              {t('live.conversations.table.totalCost')}
            </th>
            <th className="w-[14%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
              {t('live.conversations.table.createdAt')}
            </th>
            <th className="w-[14%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
              {t('live.conversations.table.lastActivityAt')}
            </th>
            <th className="w-[18%] px-2 py-2 text-left font-semibold sm:px-3 sm:py-3">
              {t('live.conversations.table.chart24h')}
            </th>
          </tr>
        </thead>
        <tbody className="divide-y divide-base-300/65">
          {stats.conversations.map((conversation) => {
            const createdAtLabel = formatDateLabel(conversation.createdAt, dateFormatter)
            const lastActivityLabel = formatDateLabel(conversation.lastActivityAt, dateFormatter)

            return (
              <tr key={conversation.promptCacheKey} className="transition-colors hover:bg-primary/6">
                <td className="max-w-0 px-2 py-2 align-middle sm:px-3 sm:py-3">
                  <div className="truncate font-mono text-xs" title={conversation.promptCacheKey}>
                    {conversation.promptCacheKey}
                  </div>
                </td>
                <td className="px-2 py-2 text-right align-middle sm:px-3 sm:py-3">
                  {formatNumber(conversation.requestCount, numberFormatter)}
                </td>
                <td className="px-2 py-2 text-right align-middle sm:px-3 sm:py-3">
                  {formatNumber(conversation.totalTokens, numberFormatter)}
                </td>
                <td className="px-2 py-2 text-right align-middle sm:px-3 sm:py-3">
                  {Number.isFinite(conversation.totalCost) ? currencyFormatter.format(conversation.totalCost) : FALLBACK_CELL}
                </td>
                <td className="px-2 py-2 align-middle sm:px-3 sm:py-3">{createdAtLabel || FALLBACK_CELL}</td>
                <td className="px-2 py-2 align-middle sm:px-3 sm:py-3">{lastActivityLabel || FALLBACK_CELL}</td>
                <td className="px-2 py-2 align-middle sm:px-3 sm:py-3">
                  <ConversationSparkline
                    conversation={conversation}
                    rangeStart={rangeStart}
                    rangeEnd={rangeEnd}
                    maxCumulativeTokens={conversationChartMax}
                    localeTag={localeTag}
                    tooltipLabels={tooltipLabels}
                    interactionHint={chartInteractionHint}
                    ariaLabel={`${conversation.promptCacheKey} ${chartAriaLabel}`}
                  />
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}
