import { Fragment, type ReactNode, useEffect, useMemo, useState } from 'react'
import { Icon } from '@iconify/react'
import type { ApiInvocation } from '../lib/api'
import {
  formatProxyWeightDelta,
  formatServiceTier,
  getFastIndicatorState,
  type FastIndicatorState,
} from '../lib/invocation'
import { useTranslation } from '../i18n'
import type { TranslationKey } from '../i18n'
import { Alert } from './ui/alert'
import { Badge } from './ui/badge'
import { Spinner } from './ui/spinner'
import { cn } from '../lib/utils'
import { getReasoningEffortTone, REASONING_EFFORT_TONE_CLASSNAMES } from './invocation-table-reasoning'

interface InvocationTableProps {
  records: ApiInvocation[]
  isLoading: boolean
  error?: string | null
}

const STATUS_META: Record<
  string,
  { variant: 'default' | 'secondary' | 'success' | 'warning' | 'error'; key: TranslationKey }
> = {
  success: { variant: 'success', key: 'table.status.success' },
  failed: { variant: 'error', key: 'table.status.failed' },
  running: { variant: 'default', key: 'table.status.running' },
  pending: { variant: 'warning', key: 'table.status.pending' },
}

const FALLBACK_STATUS_META = { variant: 'secondary', key: 'table.status.unknown' as TranslationKey }
const FALLBACK_CELL = '—'
const COMPACT_ENDPOINT = '/v1/responses/compact'

type InvocationDetailLevel = 'full' | 'structured_only'

function formatMilliseconds(value: number | null | undefined) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return FALLBACK_CELL
  return `${value.toFixed(1)} ms`
}

function formatMillisecondsCompact(value: number | null | undefined) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return FALLBACK_CELL
  return Math.round(value).toString()
}

function formatOptionalNumber(value: number | null | undefined, formatter: Intl.NumberFormat) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return FALLBACK_CELL
  return formatter.format(value)
}

function formatOptionalText(value: string | null | undefined) {
  const normalized = value?.trim()
  return normalized ? normalized : FALLBACK_CELL
}

function normalizeDetailLevel(value: ApiInvocation['detailLevel']): InvocationDetailLevel {
  return value === 'structured_only' ? 'structured_only' : 'full'
}

function formatDetailTimestamp(value: string | null | undefined) {
  const normalized = value?.trim()
  if (!normalized) return FALLBACK_CELL

  const parsed = new Date(normalized)
  if (Number.isNaN(parsed.getTime())) return normalized

  return parsed.toISOString().replace('.000Z', 'Z').replace('T', ' ')
}

function renderReasoningEffortBadge(value: string) {
  if (value === FALLBACK_CELL) {
    return <span className="font-mono text-sm text-base-content/70">{FALLBACK_CELL}</span>
  }

  const tone = getReasoningEffortTone(value)

  return (
    <Badge
      variant="secondary"
      className={cn(
        'max-w-full justify-center overflow-hidden px-2 py-0 text-[10px] font-semibold tracking-[0.01em]',
        REASONING_EFFORT_TONE_CLASSNAMES[tone],
      )}
      title={value}
      data-reasoning-effort-tone={tone}
    >
      <span className="block max-w-full truncate whitespace-nowrap">{value}</span>
    </Badge>
  )
}

function resolveProxyDisplayName(record: ApiInvocation) {
  const payloadProxyName = record.proxyDisplayName?.trim()
  if (payloadProxyName) return payloadProxyName
  const sourceValue = record.source?.trim()
  if (sourceValue && sourceValue.toLowerCase() !== 'proxy') return sourceValue
  return FALLBACK_CELL
}

function renderFastIndicator(state: FastIndicatorState, t: (key: TranslationKey) => string) {
  if (state === 'none') return null

  const isEffective = state === 'effective'
  const titleKey: TranslationKey = isEffective
    ? 'table.model.fastPriorityTitle'
    : 'table.model.fastRequestedOnlyTitle'
  const ariaKey: TranslationKey = isEffective
    ? 'table.model.fastPriorityAria'
    : 'table.model.fastRequestedOnlyAria'

  return (
    <span
      className={cn(
        'mt-0.5 inline-flex h-3.5 w-3.5 flex-none',
        isEffective ? 'text-amber-500' : 'text-base-content/50',
      )}
      title={t(titleKey)}
      aria-label={t(ariaKey)}
      data-testid="invocation-fast-icon"
      data-fast-state={state}
      role="img"
    >
      <Icon icon="mdi:lightning-bolt" className="h-3.5 w-3.5" aria-hidden />
    </span>
  )
}

function isCompactEndpoint(endpoint: string | null | undefined) {
  return endpoint?.trim() === COMPACT_ENDPOINT
}

function renderCompactBadge(t: (key: TranslationKey) => string, className?: string) {
  return (
    <Badge
      variant="secondary"
      className={cn(
        'max-w-full gap-1 overflow-hidden border-info/45 bg-info/10 px-2 py-0 text-[10px] font-semibold tracking-[0.06em] text-info',
        className,
      )}
      title={t('table.endpoint.compactHint')}
      data-testid="invocation-compact-badge"
    >
      <Icon icon="mdi:arrow-collapse-horizontal" className="h-3 w-3 flex-none" aria-hidden />
      <span className="block max-w-full truncate whitespace-nowrap">{t('table.endpoint.compactBadge')}</span>
    </Badge>
  )
}

interface InvocationRowViewModel {
  record: ApiInvocation
  recordId: number
  meta: { variant: 'default' | 'secondary' | 'success' | 'warning' | 'error'; key: TranslationKey }
  occurredTime: string
  occurredDate: string
  proxyDisplayName: string
  modelValue: string
  requestedServiceTierValue: string
  serviceTierValue: string
  fastIndicatorState: FastIndicatorState
  costValue: string
  inputTokensValue: string
  cacheInputTokensValue: string
  outputTokensValue: string
  outputReasoningBreakdownValue: string
  reasoningTokensValue: string
  reasoningEffortValue: string
  totalTokensValue: string
  endpointValue: string
  isCompactEndpoint: boolean
  errorMessage: string
  latencySummary: string
  latencyCompactSummary: string
  detailNotice: string | null
  detailPairs: Array<{ label: string; value: ReactNode }>
  timingPairs: Array<{ label: string; value: string }>
}

export function InvocationTable({ records, isLoading, error }: InvocationTableProps) {
  const { t, locale } = useTranslation()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const [expandedId, setExpandedId] = useState<number | null>(null)
  const [isXlUp, setIsXlUp] = useState(() => {
    if (typeof window === 'undefined') return false
    return window.matchMedia('(min-width: 1280px)').matches
  })

  const toggleLabels = useMemo(() => {
    if (locale === 'zh') {
      return {
        header: '详情',
        show: '展开详情',
        hide: '收起详情',
        expanded: '已展开',
        collapsed: '未展开',
      }
    }
    return {
      header: 'Details',
      show: 'Show details',
      hide: 'Hide details',
      expanded: 'Expanded',
      collapsed: 'Collapsed',
    }
  }, [locale])

  const detailLabels = useMemo(() => {
    if (locale === 'zh') {
      return {
        full: 'Full',
        structuredOnly: 'Structured only',
        level: '细节层级',
        prunedAt: '精简时间',
        pruneReason: '精简原因',
        fullHint: '完整调试细节仍在当前在线保留窗口内。',
        structuredHint: '该记录仅保留结构化字段；离线归档保留归档行，超窗 raw file 不保证继续可用。',
        prunedPrefix: '精简于',
      }
    }

    return {
      full: 'Full',
      structuredOnly: 'Structured only',
      level: 'Detail level',
      prunedAt: 'Detail pruned at',
      pruneReason: 'Detail prune reason',
      fullHint: 'Full troubleshooting detail is still available inside the online retention window.',
      structuredHint: 'Only structured fields remain online for this record. Offline archives keep the archived row, but aged raw files may no longer be available.',
      prunedPrefix: 'Pruned at',
    }
  }, [locale])

  useEffect(() => {
    setExpandedId((current) => {
      if (current === null) return current
      return records.some((record) => record.id === current) ? current : null
    })
  }, [records])

  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return
    const mediaQuery = window.matchMedia('(min-width: 1280px)')
    const sync = () => {
      setIsXlUp(mediaQuery.matches)
    }

    sync()
    if (typeof mediaQuery.addEventListener === 'function') {
      mediaQuery.addEventListener('change', sync)
      return () => {
        mediaQuery.removeEventListener('change', sync)
      }
    }

    mediaQuery.addListener(sync)
    return () => {
      mediaQuery.removeListener(sync)
    }
  }, [])

  const dateFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(localeTag, {
        month: '2-digit',
        day: '2-digit',
      }),
    [localeTag],
  )
  const timeFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(localeTag, {
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
        hour12: false,
      }),
    [localeTag],
  )
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag), [localeTag])
  const currencyFormatter = useMemo(
    () =>
      new Intl.NumberFormat(localeTag, {
        style: 'currency',
        currency: 'USD',
        minimumFractionDigits: 4,
        maximumFractionDigits: 4,
      }),
    [localeTag],
  )

  const rows = useMemo<InvocationRowViewModel[]>(
    () =>
      records.map((record) => {
        const occurred = new Date(record.occurredAt)
        const normalizedStatus = (record.status ?? 'unknown').toLowerCase()
        const meta = STATUS_META[normalizedStatus] ?? FALLBACK_STATUS_META
        const recordId = record.id
        const errorMessage = record.errorMessage?.trim() ?? ''
        const endpointValue = record.endpoint?.trim() || FALLBACK_CELL
        const isCompactEndpointValue = isCompactEndpoint(record.endpoint)
        const proxyDisplayName = resolveProxyDisplayName(record)
        const requestedServiceTierValue = formatServiceTier(record.requestedServiceTier)
        const serviceTierValue = formatServiceTier(record.serviceTier)
        const fastIndicatorState = getFastIndicatorState(record.requestedServiceTier, record.serviceTier)
        const reasoningEffortValue = formatOptionalText(record.reasoningEffort)
        const reasoningTokensValue = formatOptionalNumber(record.reasoningTokens, numberFormatter)
        const outputReasoningBreakdownValue = `${t('table.column.reasoningTokensShort')} ${reasoningTokensValue}`
        const latencySummary = `${formatMilliseconds(record.tUpstreamTtfbMs)} / ${formatMilliseconds(record.tTotalMs)}`
        const latencyCompactSummary = `${formatMillisecondsCompact(record.tUpstreamTtfbMs)}/${formatMillisecondsCompact(record.tTotalMs)}`
        const occurredValid = !Number.isNaN(occurred.getTime())
        const occurredTime = occurredValid ? timeFormatter.format(occurred) : record.occurredAt
        const occurredDate = occurredValid ? dateFormatter.format(occurred) : FALLBACK_CELL

        const proxyWeightDeltaView = formatProxyWeightDelta(record.proxyWeightDelta)
        const proxyWeightDeltaValue =
          proxyWeightDeltaView.direction === 'missing' ? (
            FALLBACK_CELL
          ) : (
            <span
              className={`inline-flex items-center gap-1 font-mono ${
                proxyWeightDeltaView.direction === 'up'
                  ? 'text-success'
                  : proxyWeightDeltaView.direction === 'down'
                    ? 'text-error'
                    : 'text-base-content/70'
              }`}
              aria-label={
                proxyWeightDeltaView.direction === 'up'
                  ? t('table.details.proxyWeightDeltaA11yIncrease', { value: proxyWeightDeltaView.value })
                  : proxyWeightDeltaView.direction === 'down'
                    ? t('table.details.proxyWeightDeltaA11yDecrease', { value: proxyWeightDeltaView.value })
                    : t('table.details.proxyWeightDeltaA11yUnchanged', { value: proxyWeightDeltaView.value })
              }
            >
              <Icon
                icon={
                  proxyWeightDeltaView.direction === 'up'
                    ? 'mdi:arrow-up-bold'
                    : proxyWeightDeltaView.direction === 'down'
                      ? 'mdi:arrow-down-bold'
                      : 'mdi:arrow-right-bold'
                }
                className="h-3.5 w-3.5"
                aria-hidden
              />
              <span aria-hidden>{proxyWeightDeltaView.value}</span>
            </span>
          )

        const detailLevel = normalizeDetailLevel(record.detailLevel)
        const detailPrunedAtValue = formatDetailTimestamp(record.detailPrunedAt)
        const detailPruneReasonValue = formatOptionalText(record.detailPruneReason)
        const detailLevelBadgeLabel = detailLevel === 'structured_only' ? detailLabels.structuredOnly : detailLabels.full
        const detailLevelBadgeVariant = detailLevel === 'structured_only' ? 'warning' : 'secondary'
        const detailNotice = detailLevel === 'structured_only' ? detailLabels.structuredHint : null
        const detailPrunedSummary =
          detailLevel === 'structured_only' && detailPrunedAtValue !== FALLBACK_CELL
            ? `${detailLabels.prunedPrefix} ${detailPrunedAtValue}`
            : null
        const detailLevelTooltip =
          detailLevel === 'structured_only'
            ? detailPrunedSummary
              ? `${detailLabels.structuredHint} ${detailPrunedSummary}.`
              : detailLabels.structuredHint
            : detailLabels.fullHint

        const detailPairs: Array<{ label: string; value: ReactNode }> = [
          { label: t('table.details.invokeId'), value: record.invokeId || FALLBACK_CELL },
          { label: t('table.details.source'), value: record.source || FALLBACK_CELL },
          { label: t('table.details.proxy'), value: proxyDisplayName },
          { label: t('table.details.endpoint'), value: record.endpoint || FALLBACK_CELL },
          { label: t('table.details.requesterIp'), value: record.requesterIp || FALLBACK_CELL },
          { label: t('table.details.promptCacheKey'), value: record.promptCacheKey || FALLBACK_CELL },
          { label: t('table.details.requestedServiceTier'), value: requestedServiceTierValue },
          { label: t('table.details.serviceTier'), value: serviceTierValue },
          { label: t('table.details.reasoningEffort'), value: renderReasoningEffortBadge(reasoningEffortValue) },
          { label: t('table.details.reasoningTokens'), value: reasoningTokensValue },
          { label: t('table.details.proxyWeightDelta'), value: proxyWeightDeltaValue },
          { label: t('table.details.failureKind'), value: record.failureKind || FALLBACK_CELL },
          {
            label: detailLabels.level,
            value: (
              <Badge
                variant={detailLevelBadgeVariant}
                className="max-w-full justify-center overflow-hidden px-2 py-0 text-[10px] font-semibold tracking-[0.01em]"
                title={detailLevelTooltip}
                data-testid="invocation-detail-level-badge"
              >
                <span className="block max-w-full truncate whitespace-nowrap">{detailLevelBadgeLabel}</span>
              </Badge>
            ),
          },
          { label: detailLabels.prunedAt, value: detailPrunedAtValue },
          { label: detailLabels.pruneReason, value: detailPruneReasonValue },
        ]
        const timingPairs: Array<{ label: string; value: string }> = [
          { label: t('table.details.stage.requestRead'), value: formatMilliseconds(record.tReqReadMs) },
          { label: t('table.details.stage.requestParse'), value: formatMilliseconds(record.tReqParseMs) },
          { label: t('table.details.stage.upstreamConnect'), value: formatMilliseconds(record.tUpstreamConnectMs) },
          { label: t('table.details.stage.upstreamFirstByte'), value: formatMilliseconds(record.tUpstreamTtfbMs) },
          { label: t('table.details.stage.upstreamStream'), value: formatMilliseconds(record.tUpstreamStreamMs) },
          { label: t('table.details.stage.responseParse'), value: formatMilliseconds(record.tRespParseMs) },
          { label: t('table.details.stage.persistence'), value: formatMilliseconds(record.tPersistMs) },
          { label: t('table.details.stage.total'), value: formatMilliseconds(record.tTotalMs) },
        ]

        return {
          record,
          recordId,
          meta,
          occurredTime,
          occurredDate,
          proxyDisplayName,
          modelValue: record.model ?? FALLBACK_CELL,
          requestedServiceTierValue,
          serviceTierValue,
          fastIndicatorState,
          costValue: typeof record.cost === 'number' ? currencyFormatter.format(record.cost) : FALLBACK_CELL,
          inputTokensValue: formatOptionalNumber(record.inputTokens, numberFormatter),
          cacheInputTokensValue: formatOptionalNumber(record.cacheInputTokens, numberFormatter),
          outputTokensValue: formatOptionalNumber(record.outputTokens, numberFormatter),
          outputReasoningBreakdownValue,
          reasoningTokensValue,
          reasoningEffortValue,
          totalTokensValue: formatOptionalNumber(record.totalTokens, numberFormatter),
          endpointValue,
          isCompactEndpoint: isCompactEndpointValue,
          errorMessage,
          latencySummary,
          latencyCompactSummary,
          detailNotice,
          detailPairs,
          timingPairs,
        }
      }),
    [records, currencyFormatter, dateFormatter, detailLabels, numberFormatter, t, timeFormatter],
  )

  const renderExpandedContent = (
    detailId: string,
    detailPairs: Array<{ label: string; value: ReactNode }>,
    timingPairs: Array<{ label: string; value: string }>,
    errorMessage: string,
    detailNotice: string | null,
    size: 'compact' | 'default',
  ) => (
    <div id={detailId} className={`flex flex-col gap-4 ${size === 'compact' ? 'p-3' : 'p-4'}`}>
      {detailNotice && (
        <div className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs leading-5 text-warning" data-testid="invocation-detail-notice">
          {detailNotice}
        </div>
      )}

      <div className="flex flex-col gap-2">
        <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">{t('table.detailsTitle')}</span>
        <div className="grid gap-2 md:grid-cols-2">
          {detailPairs.map((entry) => (
            <div key={entry.label} className="flex items-start gap-2">
              <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60 md:min-w-36">{entry.label}</span>
              <div className="min-w-0 break-all font-mono text-sm">{entry.value}</div>
            </div>
          ))}
        </div>
      </div>

      <div className="flex flex-col gap-2">
        <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">
          {t('table.details.timingsTitle')}
        </span>
        <div className="grid gap-2 md:grid-cols-2">
          {timingPairs.map((entry) => (
            <div key={entry.label} className="flex items-start gap-2">
              <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60 md:min-w-36">{entry.label}</span>
              <span className="font-mono text-sm">{entry.value}</span>
            </div>
          ))}
        </div>
      </div>

      {errorMessage && (
        <div className="flex flex-col gap-2">
          <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">{t('table.errorDetailsTitle')}</span>
          <pre className="whitespace-pre-wrap break-words font-mono text-sm">{errorMessage}</pre>
        </div>
      )}
    </div>
  )

  if (error) {
    return (
      <Alert variant="error">
        <span>{t('table.loadError', { error })}</span>
      </Alert>
    )
  }

  if (isLoading) {
    return (
      <div className="flex justify-center py-10">
        <Spinner size="lg" aria-label={t('table.loadingRecordsAria')} />
      </div>
    )
  }

  if (records.length === 0) {
    return <Alert>{t('table.noRecords')}</Alert>
  }

  return (
    <div className="space-y-3">
      <div className="space-y-3 md:hidden" data-testid="invocation-list">
        {rows.map((row, rowIndex) => {
          const listDetailId = `invocation-list-details-${row.recordId}`
          const isExpanded = expandedId === row.recordId
          const handleToggle = () => {
            setExpandedId((current) => (current === row.recordId ? null : row.recordId))
          }

          return (
            <article
              key={`mobile-${row.recordId}`}
              data-testid="invocation-list-item"
              className={`rounded-xl border border-base-300/70 px-3 py-3 ${rowIndex % 2 === 0 ? 'bg-base-100/40' : 'bg-base-200/24'}`}
            >
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-semibold">{row.occurredTime}</div>
                  <div className="truncate text-xs text-base-content/65">{row.occurredDate}</div>
                </div>
                <button
                  type="button"
                  className="inline-flex h-8 w-8 items-center justify-center rounded-md text-base-content/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                  onClick={handleToggle}
                  aria-expanded={isExpanded}
                  aria-controls={listDetailId}
                  aria-label={isExpanded ? toggleLabels.hide : toggleLabels.show}
                >
                  <Icon
                    icon={isExpanded ? 'mdi:chevron-down' : 'mdi:chevron-right'}
                    className="h-5 w-5"
                    aria-hidden
                  />
                  <span className="sr-only">{isExpanded ? toggleLabels.expanded : toggleLabels.collapsed}</span>
                </button>
              </div>

              <div className="mt-2 flex min-w-0 flex-wrap items-center gap-2">
                <Badge variant={row.meta.variant}>{t(row.meta.key)}</Badge>
                {row.isCompactEndpoint ? renderCompactBadge(t, 'shrink-0') : null}
                <span className="min-w-0 truncate text-xs text-base-content/75" title={row.proxyDisplayName}>
                  {row.proxyDisplayName}
                </span>
              </div>

              <div className="mt-2 text-xs font-mono text-base-content/70" title={row.latencySummary}>
                {row.latencySummary}
              </div>

              <dl className="mt-3 grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
                <dt className="text-base-content/65">{t('table.column.model')}</dt>
                <dd className="min-w-0">
                  <div className="flex items-start justify-end gap-1 text-right" title={row.modelValue}>
                    <span className="min-w-0 flex-1 truncate">{row.modelValue}</span>
                    {renderFastIndicator(row.fastIndicatorState, t)}
                  </div>
                </dd>
                <dt className="text-base-content/65">{t('table.column.costUsd')}</dt>
                <dd className="truncate text-right font-mono">{row.costValue}</dd>
                <dt className="text-base-content/65">{t('table.column.inputTokens')}</dt>
                <dd className="truncate text-right font-mono">{row.inputTokensValue}</dd>
                <dt className="text-base-content/65">{t('table.column.cacheInputTokens')}</dt>
                <dd className="truncate text-right font-mono">{row.cacheInputTokensValue}</dd>
                <dt className="text-base-content/65">{t('table.column.outputTokens')}</dt>
                <dd className="text-right">
                  <div className="flex flex-col items-end gap-0.5 leading-tight">
                    <span className="truncate font-mono">{row.outputTokensValue}</span>
                    <span
                      className="truncate text-[11px] text-base-content/70"
                      title={`${t('table.details.reasoningTokens')}: ${row.reasoningTokensValue}`}
                    >
                      {row.outputReasoningBreakdownValue}
                    </span>
                  </div>
                </dd>
                <dt className="text-base-content/65">{t('table.column.totalTokens')}</dt>
                <dd className="truncate text-right font-mono">{row.totalTokensValue}</dd>
                <dt className="text-base-content/65">{t('table.column.reasoningEffort')}</dt>
                <dd className="flex justify-end">{renderReasoningEffortBadge(row.reasoningEffortValue)}</dd>
              </dl>

              <div className="mt-3 space-y-1 border-t border-base-300/65 pt-2">
                <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">{t('table.details.endpoint')}</div>
                <div className="truncate text-xs text-base-content/75" title={row.endpointValue}>{row.endpointValue}</div>
                <div className="truncate text-xs" title={row.errorMessage || undefined}>{row.errorMessage || FALLBACK_CELL}</div>
              </div>

              {isExpanded && (
                <div className="mt-3 rounded-lg border border-base-300/70 bg-base-200/58">
                  {renderExpandedContent(
                    listDetailId,
                    row.detailPairs,
                    row.timingPairs,
                    row.errorMessage,
                    row.detailNotice,
                    'compact',
                  )}
                </div>
              )}
            </article>
          )
        })}
      </div>

      <div className="hidden md:block">
        <div
          className="overflow-x-hidden rounded-xl border border-base-300/70 bg-base-100/52 backdrop-blur"
          data-testid="invocation-table-scroll"
        >
          <table className="w-full table-fixed border-separate border-spacing-0 text-sm">
            <thead className="bg-base-200/65 text-[11px] uppercase tracking-[0.08em] text-base-content/70">
              <tr>
                <th className="w-[12%] px-2 py-2.5 text-left font-semibold whitespace-nowrap xl:w-[10%] xl:px-3">{t('table.column.time')}</th>
                <th className="w-[18%] px-2 py-2.5 text-center font-semibold whitespace-nowrap xl:w-[15%] xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.proxy')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.column.latency')}
                    </span>
                  </div>
                </th>
                <th className="w-[19%] px-2 py-2.5 text-right font-semibold whitespace-nowrap xl:w-[16%] xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.model')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.column.costUsd')}
                    </span>
                  </div>
                </th>
                <th className="w-[18%] px-2 py-2.5 text-right font-semibold whitespace-nowrap xl:w-[15%] xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.inputTokens')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.column.cacheInputTokens')}
                    </span>
                  </div>
                </th>
                <th className="w-[12%] px-2 py-2.5 text-right font-semibold whitespace-nowrap xl:w-[11%] xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.outputTokens')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.details.reasoningTokens')}
                    </span>
                  </div>
                </th>
                <th className="w-[14%] px-2 py-2.5 text-right font-semibold whitespace-nowrap xl:w-[12%] xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.totalTokens')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.column.reasoningEffort')}
                    </span>
                  </div>
                </th>
                <th className="hidden w-[18%] px-2 py-2.5 text-left font-semibold xl:table-cell xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.error')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.details.endpoint')}
                    </span>
                  </div>
                </th>
                <th className="w-[10%] px-2 py-2.5 text-right xl:w-[7%] xl:px-3">
                  <span className="sr-only">{toggleLabels.header}</span>
                </th>
              </tr>
            </thead>
            <tbody className="divide-y divide-base-300/65">
              {rows.map((row, rowIndex) => {
                const tableDetailId = `invocation-table-details-${row.recordId}`
                const isExpanded = expandedId === row.recordId
                const handleToggle = () => {
                  setExpandedId((current) => (current === row.recordId ? null : row.recordId))
                }

                return (
                  <Fragment key={row.recordId}>
                    <tr className={`${rowIndex % 2 === 0 ? 'bg-base-100/38' : 'bg-base-200/22'} hover:bg-primary/6`}>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle xl:px-3">
                        <div className="flex min-w-0 flex-col justify-center gap-1 leading-tight">
                          <span className="truncate whitespace-nowrap font-medium">{row.occurredTime}</span>
                          <span className="truncate whitespace-nowrap text-base-content/70">{row.occurredDate}</span>
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle text-center xl:px-3">
                        <div className="flex min-w-0 flex-col items-center justify-center gap-1 leading-tight">
                          <div className="flex min-w-0 flex-wrap items-center justify-center gap-1">
                            <Badge variant={row.meta.variant} className="max-w-full justify-center overflow-hidden">
                              <span className="block max-w-full truncate whitespace-nowrap text-center" title={row.proxyDisplayName}>
                                {row.proxyDisplayName}
                              </span>
                              <span className="sr-only">{t(row.meta.key)}</span>
                            </Badge>
                            {row.isCompactEndpoint ? renderCompactBadge(t, 'shrink-0') : null}
                          </div>
                          <span className="hidden whitespace-nowrap font-mono text-[11px] text-base-content/70 lg:block" title={row.latencySummary}>
                            {row.latencySummary}
                          </span>
                          <span className="whitespace-nowrap font-mono text-[11px] text-base-content/70 lg:hidden" title={row.latencySummary}>
                            {row.latencyCompactSummary}
                          </span>
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle xl:px-3">
                        <div className="flex min-w-0 flex-col items-end justify-center gap-1 leading-tight text-right">
                          <div className="flex w-full items-start justify-end gap-1">
                            <span className="min-w-0 flex-1 truncate whitespace-nowrap text-base-content/85" title={row.modelValue}>
                              {row.modelValue}
                            </span>
                            {renderFastIndicator(row.fastIndicatorState, t)}
                          </div>
                          <span className="w-full truncate whitespace-nowrap font-mono tabular-nums text-base-content/70">
                            {row.costValue}
                          </span>
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle xl:px-3">
                        <div className="flex min-w-0 flex-col items-end justify-center gap-1 leading-tight text-right">
                          <span className="w-full truncate whitespace-nowrap font-mono tabular-nums">
                            {row.inputTokensValue}
                          </span>
                          <span className="w-full truncate whitespace-nowrap font-mono tabular-nums text-base-content/70">
                            {row.cacheInputTokensValue}
                          </span>
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle text-right xl:px-3">
                        <div className="flex min-w-0 flex-col items-end justify-center gap-1 leading-tight text-right">
                          <span className="block w-full truncate whitespace-nowrap font-mono tabular-nums">{row.outputTokensValue}</span>
                          <span
                            className="block w-full truncate whitespace-nowrap text-[11px] text-base-content/70"
                            title={`${t('table.details.reasoningTokens')}: ${row.reasoningTokensValue}`}
                          >
                            {row.outputReasoningBreakdownValue}
                          </span>
                        </div>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle text-right xl:px-3">
                        <div className="flex min-w-0 flex-col items-end justify-center gap-1 leading-tight text-right">
                          <span className="block w-full truncate whitespace-nowrap font-mono tabular-nums">{row.totalTokensValue}</span>
                          <div className="flex w-full justify-end">{renderReasoningEffortBadge(row.reasoningEffortValue)}</div>
                        </div>
                      </td>
                      <td className="hidden min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle xl:table-cell xl:px-3">
                        <div className="flex min-w-0 flex-col justify-center gap-1 leading-tight">
                          <span className="block truncate whitespace-nowrap text-base-content/70" title={row.endpointValue}>
                            {row.endpointValue}
                          </span>
                          <span className="block truncate whitespace-nowrap" title={row.errorMessage || undefined}>
                            {row.errorMessage || FALLBACK_CELL}
                          </span>
                        </div>
                      </td>
                      <td className="border-t border-base-300/65 px-2 py-2.5 align-middle text-right xl:px-3">
                        <button
                          type="button"
                          className="inline-flex items-center justify-end gap-1 text-lg leading-none text-base-content/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                          onClick={handleToggle}
                          aria-expanded={isExpanded}
                          aria-controls={tableDetailId}
                          aria-label={isExpanded ? toggleLabels.hide : toggleLabels.show}
                        >
                          <Icon
                            icon={isExpanded ? 'mdi:chevron-down' : 'mdi:chevron-right'}
                            className="h-4 w-4"
                            aria-hidden
                          />
                          <span className="sr-only">{isExpanded ? toggleLabels.expanded : toggleLabels.collapsed}</span>
                        </button>
                      </td>
                    </tr>
                    {isExpanded && (
                      <tr className="bg-base-200/68">
                        <td colSpan={isXlUp ? 8 : 7} className="border-t border-base-300/65 px-2 py-2.5 xl:px-3">
                          {renderExpandedContent(
                            tableDetailId,
                            row.detailPairs,
                            row.timingPairs,
                            row.errorMessage,
                            row.detailNotice,
                            'default',
                          )}
                        </td>
                      </tr>
                    )}
                  </Fragment>
                )
              })}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  )
}
