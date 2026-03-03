import { useMemo } from 'react'
import { useTranslation } from '../i18n'
import type {
  PromptCacheConversation,
  PromptCacheConversationRequestPoint,
  PromptCacheConversationsResponse,
} from '../lib/api'
import { Alert } from './ui/alert'
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

function buildGeometry(
  points: PromptCacheConversationRequestPoint[],
  rangeStart: string,
  rangeEnd: string,
): ConversationChartGeometry | null {
  const rangeStartEpoch = parseEpoch(rangeStart)
  const rangeEndEpoch = parseEpoch(rangeEnd)
  if (rangeStartEpoch == null || rangeEndEpoch == null || rangeEndEpoch <= rangeStartEpoch) return null

  const segments = buildSegments(points, rangeStartEpoch, rangeEndEpoch)
  if (segments.length === 0) return null

  const maxCumulative = Math.max(...segments.map((segment) => segment.cumulativeTokens), 1)
  const span = rangeEndEpoch - rangeStartEpoch
  const xForEpoch = (epoch: number) => ((epoch - rangeStartEpoch) / span) * CHART_WIDTH
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

function chartTooltip(
  point: PromptCacheConversationRequestPoint,
  localeTag: string,
  labels: {
    status: string
    requestTokens: string
    cumulativeTokens: string
  },
) {
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
  return `${timeLabel}\n${labels.status}: ${point.status}\n${labels.requestTokens}: ${point.requestTokens}\n${labels.cumulativeTokens}: ${point.cumulativeTokens}`
}

function ConversationSparkline({
  conversation,
  rangeStart,
  rangeEnd,
  localeTag,
  tooltipLabels,
}: {
  conversation: PromptCacheConversation
  rangeStart: string
  rangeEnd: string
  localeTag: string
  tooltipLabels: {
    status: string
    requestTokens: string
    cumulativeTokens: string
  }
}) {
  const geometry = useMemo(
    () => buildGeometry(conversation.last24hRequests, rangeStart, rangeEnd),
    [conversation.last24hRequests, rangeEnd, rangeStart],
  )

  if (!geometry) {
    return <div className="text-[11px] text-base-content/55">{FALLBACK_CELL}</div>
  }

  return (
    <div className="h-12">
      <svg
        viewBox={`0 0 ${geometry.width} ${geometry.height}`}
        className="h-11 w-full rounded-md border border-base-300/55 bg-base-100/35"
        role="img"
        aria-label={conversation.promptCacheKey}
      >
        <line
          x1={0}
          y1={geometry.height}
          x2={geometry.width}
          y2={geometry.height}
          stroke="oklch(var(--color-base-content) / 0.16)"
          strokeWidth="1"
        />
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
        {geometry.segments.map((segment, index) => (
          <g key={`${conversation.promptCacheKey}-segment-${index}`}>
            <line
              x1={segment.x1}
              y1={segment.y}
              x2={segment.x2}
              y2={segment.y}
              stroke={segment.isSuccess ? 'oklch(var(--color-success) / 0.95)' : 'oklch(var(--color-error) / 0.92)'}
              strokeWidth="2.8"
              strokeLinecap="round"
            />
            <rect
              x={Math.min(segment.x1, segment.x2)}
              y={0}
              width={Math.max(Math.abs(segment.x2 - segment.x1), 2)}
              height={geometry.height}
              fill="transparent"
            >
              <title>{chartTooltip(segment.point, localeTag, tooltipLabels)}</title>
            </rect>
          </g>
        ))}
      </svg>
    </div>
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

  const rangeStart = stats?.rangeStart ?? ''
  const rangeEnd = stats?.rangeEnd ?? ''

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
      <table className="w-full table-fixed text-[11px] sm:text-xs">
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
            const createdAt = new Date(conversation.createdAt)
            const lastActivityAt = new Date(conversation.lastActivityAt)
            const createdAtLabel = Number.isNaN(createdAt.getTime()) ? conversation.createdAt : dateFormatter.format(createdAt)
            const lastActivityLabel = Number.isNaN(lastActivityAt.getTime())
              ? conversation.lastActivityAt
              : dateFormatter.format(lastActivityAt)

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
                    localeTag={localeTag}
                    tooltipLabels={tooltipLabels}
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
