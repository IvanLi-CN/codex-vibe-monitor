import { Fragment, useMemo, useState } from 'react'
import { Icon } from '@iconify/react'
import type { ApiInvocation, InvocationFocus } from '../lib/api'
import { resolveInvocationDisplayStatus } from '../lib/invocationStatus'
import { useTranslation } from '../i18n'
import { Alert } from './ui/alert'
import { Badge } from './ui/badge'
import { Spinner } from './ui/spinner'

interface InvocationRecordsTableProps {
  focus: InvocationFocus
  records: ApiInvocation[]
  isLoading: boolean
  error?: string | null
}

const FALLBACK_CELL = '—'
type StatusMeta = {
  variant: 'default' | 'secondary' | 'success' | 'warning' | 'error'
  labelKey?: string
  label?: string
}

const STATUS_META: Record<string, { variant: StatusMeta['variant']; labelKey: string }> = {
  success: { variant: 'success', labelKey: 'table.status.success' },
  failed: { variant: 'error', labelKey: 'table.status.failed' },
  running: { variant: 'default', labelKey: 'table.status.running' },
  pending: { variant: 'warning', labelKey: 'table.status.pending' },
}

function formatText(value?: string | null) {
  const normalized = value?.trim()
  return normalized ? normalized : FALLBACK_CELL
}

function formatNumber(value: number | null | undefined, formatter: Intl.NumberFormat) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return FALLBACK_CELL
  return formatter.format(value)
}

function formatMilliseconds(value?: number | null) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return FALLBACK_CELL
  return `${Math.round(value)} ms`
}

function formatCost(value: number | null | undefined, formatter: Intl.NumberFormat) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return FALLBACK_CELL
  return formatter.format(value)
}

function formatStatusLabel(status: string) {
  const normalized = status.trim()
  if (!normalized) return null
  const lower = normalized.toLowerCase()
  if (lower.startsWith('http_')) {
    const code = lower.slice('http_'.length)
    if (/^\d{3}$/.test(code)) return `HTTP ${code}`
    return normalized.toUpperCase().replace('_', ' ')
  }
  return normalized
}

function resolveStatusMeta(status?: string | null): StatusMeta {
  const raw = (status ?? '').trim()
  const lower = raw.toLowerCase()
  const known = STATUS_META[lower]
  if (known) return known
  if (!raw) return { variant: 'secondary', labelKey: 'table.status.unknown' }
  if (lower.startsWith('http_4')) return { variant: 'warning', label: formatStatusLabel(raw) ?? raw }
  if (lower.startsWith('http_5')) return { variant: 'error', label: formatStatusLabel(raw) ?? raw }
  if (lower.startsWith('http_')) return { variant: 'secondary', label: formatStatusLabel(raw) ?? raw }
  return { variant: 'secondary', label: raw }
}

function formatOccurredAt(occurredAt: string, formatter: Intl.DateTimeFormat) {
  const value = occurredAt.trim()
  const parsed = new Date(value)
  if (Number.isNaN(parsed.getTime())) return value || FALLBACK_CELL
  return formatter.format(parsed)
}

function resolveProxyName(record: ApiInvocation) {
  const payloadProxyName = record.proxyDisplayName?.trim()
  if (payloadProxyName) return payloadProxyName
  const sourceValue = record.source?.trim()
  if (sourceValue && sourceValue.toLowerCase() !== 'proxy') return sourceValue
  return FALLBACK_CELL
}

function resolveFailureClassMeta(failureClass?: ApiInvocation['failureClass']) {
  switch (failureClass) {
    case 'service_failure':
      return { variant: 'error' as const, labelKey: 'records.filters.failureClass.service' }
    case 'client_failure':
      return { variant: 'warning' as const, labelKey: 'records.filters.failureClass.client' }
    case 'client_abort':
      return { variant: 'secondary' as const, labelKey: 'records.filters.failureClass.abort' }
    default:
      return { variant: 'secondary' as const, labelKey: null }
  }
}

export function InvocationRecordsTable({ focus, records, isLoading, error }: InvocationRecordsTableProps) {
  const { t, locale } = useTranslation()
  const [expandedId, setExpandedId] = useState<number | null>(null)
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const numberFormatter = useMemo(() => new Intl.NumberFormat(localeTag), [localeTag])
  const costFormatter = useMemo(
    () =>
      new Intl.NumberFormat(localeTag, {
        style: 'currency',
        currency: 'USD',
        minimumFractionDigits: 4,
        maximumFractionDigits: 4,
      }),
    [localeTag],
  )
  const dateTimeFormatter = useMemo(
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

  const hasRecords = records.length > 0
  const showBlockingError = Boolean(error) && !hasRecords
  const showInlineError = Boolean(error) && hasRecords

  if (showBlockingError) {
    return <Alert variant="error">{t('records.table.loadError', { error: error ?? '' })}</Alert>
  }

  if (isLoading && !hasRecords) {
    return (
      <div className="flex justify-center py-10">
        <Spinner size="lg" aria-label={t('records.table.loadingAria')} />
      </div>
    )
  }

  if (!hasRecords) {
    return <Alert>{t('records.table.empty')}</Alert>
  }

  const headers = (() => {
    switch (focus) {
      case 'network':
        return [
          t('records.table.network.endpoint'),
          t('records.table.network.requesterIp'),
          t('records.table.network.ttfb'),
          t('records.table.network.totalMs'),
        ]
      case 'exception':
        return [
          t('records.table.exception.failureKind'),
          t('records.table.exception.failureClass'),
          t('records.table.exception.actionable'),
          t('records.table.exception.error'),
        ]
      case 'token':
      default:
        return [
          t('records.table.token.inputCache'),
          t('records.table.token.outputReasoning'),
          t('records.table.token.totalTokens'),
          t('records.table.token.cost'),
        ]
    }
  })()

  const renderFocusCells = (record: ApiInvocation) => {
    switch (focus) {
      case 'network':
        return (
          <>
            <td className="px-3 py-3 align-middle text-left font-mono text-xs">{formatText(record.endpoint)}</td>
            <td className="px-3 py-3 align-middle text-left font-mono text-xs">{formatText(record.requesterIp)}</td>
            <td className="px-3 py-3 align-middle text-right font-mono text-xs">{formatMilliseconds(record.tUpstreamTtfbMs)}</td>
            <td className="px-3 py-3 align-middle text-right font-mono text-xs">{formatMilliseconds(record.tTotalMs)}</td>
          </>
        )
      case 'exception': {
        const failureClass = resolveFailureClassMeta(record.failureClass)
        return (
          <>
            <td className="px-3 py-3 align-middle text-left font-mono text-xs">{formatText(record.failureKind)}</td>
            <td className="px-3 py-3 align-middle text-left text-xs">
              <Badge variant={failureClass.variant}>{failureClass.labelKey ? t(failureClass.labelKey) : FALLBACK_CELL}</Badge>
            </td>
            <td className="px-3 py-3 align-middle text-left text-xs">
              <Badge variant={record.isActionable ? 'warning' : 'secondary'}>
                {record.isActionable ? t('records.table.exception.actionableYes') : t('records.table.exception.actionableNo')}
              </Badge>
            </td>
            <td className="max-w-[18rem] truncate px-3 py-3 align-middle text-left text-xs" title={record.errorMessage ?? undefined}>
              {formatText(record.errorMessage)}
            </td>
          </>
        )
      }
      case 'token':
      default:
        return (
          <>
            <td className="px-3 py-3 align-middle text-right font-mono text-xs">
              <div>{formatNumber(record.inputTokens, numberFormatter)}</div>
              <div className="text-base-content/60">{formatNumber(record.cacheInputTokens, numberFormatter)}</div>
            </td>
            <td className="px-3 py-3 align-middle text-right font-mono text-xs">
              <div>{formatNumber(record.outputTokens, numberFormatter)}</div>
              <div className="text-base-content/60">{formatNumber(record.reasoningTokens, numberFormatter)}</div>
            </td>
            <td className="px-3 py-3 align-middle text-right font-mono text-xs">{formatNumber(record.totalTokens, numberFormatter)}</td>
            <td className="px-3 py-3 align-middle text-right font-mono text-xs">{formatCost(record.cost, costFormatter)}</td>
          </>
        )
    }
  }

  const renderMobileFocus = (record: ApiInvocation) => {
    switch (focus) {
      case 'network':
        return (
          <>
            <div className="flex items-center justify-between gap-3"><dt>{t('records.table.network.endpoint')}</dt><dd className="truncate font-mono">{formatText(record.endpoint)}</dd></div>
            <div className="flex items-center justify-between gap-3"><dt>{t('records.table.network.requesterIp')}</dt><dd className="truncate font-mono">{formatText(record.requesterIp)}</dd></div>
            <div className="flex items-center justify-between gap-3"><dt>{t('records.table.network.ttfb')}</dt><dd className="font-mono">{formatMilliseconds(record.tUpstreamTtfbMs)}</dd></div>
            <div className="flex items-center justify-between gap-3"><dt>{t('records.table.network.totalMs')}</dt><dd className="font-mono">{formatMilliseconds(record.tTotalMs)}</dd></div>
          </>
        )
      case 'exception': {
        const failureClass = resolveFailureClassMeta(record.failureClass)
        return (
          <>
            <div className="flex items-center justify-between gap-3"><dt>{t('records.table.exception.failureKind')}</dt><dd className="truncate font-mono">{formatText(record.failureKind)}</dd></div>
            <div className="flex items-center justify-between gap-3"><dt>{t('records.table.exception.failureClass')}</dt><dd className="truncate">{failureClass.labelKey ? t(failureClass.labelKey) : FALLBACK_CELL}</dd></div>
            <div className="flex items-center justify-between gap-3"><dt>{t('records.table.exception.actionable')}</dt><dd>{record.isActionable ? t('records.table.exception.actionableYes') : t('records.table.exception.actionableNo')}</dd></div>
            <div className="flex items-center justify-between gap-3"><dt>{t('records.table.exception.error')}</dt><dd className="truncate font-mono">{formatText(record.errorMessage)}</dd></div>
          </>
        )
      }
      case 'token':
      default:
        return (
          <>
            <div className="flex items-center justify-between gap-3"><dt>{t('records.table.token.inputCache')}</dt><dd className="font-mono">{formatNumber(record.inputTokens, numberFormatter)} / {formatNumber(record.cacheInputTokens, numberFormatter)}</dd></div>
            <div className="flex items-center justify-between gap-3"><dt>{t('records.table.token.outputReasoning')}</dt><dd className="font-mono">{formatNumber(record.outputTokens, numberFormatter)} / {formatNumber(record.reasoningTokens, numberFormatter)}</dd></div>
            <div className="flex items-center justify-between gap-3"><dt>{t('records.table.token.totalTokens')}</dt><dd className="font-mono">{formatNumber(record.totalTokens, numberFormatter)}</dd></div>
            <div className="flex items-center justify-between gap-3"><dt>{t('records.table.token.cost')}</dt><dd className="font-mono">{formatCost(record.cost, costFormatter)}</dd></div>
          </>
        )
    }
  }

  return (
    <div className="space-y-3">
      {showInlineError ? <Alert variant="error">{t('records.table.loadError', { error: error ?? '' })}</Alert> : null}
      <div className="space-y-3 md:hidden">
        {records.map((record) => {
          const isExpanded = expandedId === record.id
          const statusMeta = resolveStatusMeta(resolveInvocationDisplayStatus(record))
          const statusLabel = statusMeta.labelKey ? t(statusMeta.labelKey) : statusMeta.label ?? t('table.status.unknown')
          return (
            <article key={record.id} className="rounded-xl border border-base-300/70 bg-base-100/45 px-4 py-4">
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <div className="text-sm font-semibold">{formatOccurredAt(record.occurredAt, dateTimeFormatter)}</div>
                  <div className="mt-1 flex flex-wrap items-center gap-2">
                    <Badge variant={statusMeta.variant}>{statusLabel}</Badge>
                    <span className="truncate text-xs text-base-content/70">{resolveProxyName(record)}</span>
                  </div>
                </div>
                <button
                  type="button"
                  className="inline-flex h-8 w-8 items-center justify-center rounded-md text-base-content/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                  onClick={() => setExpandedId((current) => (current === record.id ? null : record.id))}
                  aria-expanded={isExpanded}
                  aria-label={isExpanded ? t('records.table.hideDetails') : t('records.table.showDetails')}
                >
                  <Icon icon={isExpanded ? 'mdi:chevron-down' : 'mdi:chevron-right'} className="h-5 w-5" aria-hidden />
                </button>
              </div>
              <div className="mt-3 text-sm font-medium">{formatText(record.model)}</div>
              <dl className="mt-3 space-y-2 text-xs text-base-content/75">{renderMobileFocus(record)}</dl>
              {isExpanded && (
                <div className="mt-3 rounded-xl border border-base-300/70 bg-base-200/55 p-3 text-xs">
                  <div className="grid gap-2">
                    <div className="flex items-start justify-between gap-3"><span>{t('table.details.invokeId')}</span><span className="break-all font-mono">{record.invokeId}</span></div>
                    <div className="flex items-start justify-between gap-3"><span>{t('table.details.endpoint')}</span><span className="break-all font-mono">{formatText(record.endpoint)}</span></div>
                    <div className="flex items-start justify-between gap-3"><span>{t('table.details.promptCacheKey')}</span><span className="break-all font-mono">{formatText(record.promptCacheKey)}</span></div>
                    <div className="flex items-start justify-between gap-3"><span>{t('table.details.requesterIp')}</span><span className="break-all font-mono">{formatText(record.requesterIp)}</span></div>
                    <div className="flex items-start justify-between gap-3"><span>{t('table.details.stage.upstreamFirstByte')}</span><span className="font-mono">{formatMilliseconds(record.tUpstreamTtfbMs)}</span></div>
                    <div className="flex items-start justify-between gap-3"><span>{t('table.details.stage.total')}</span><span className="font-mono">{formatMilliseconds(record.tTotalMs)}</span></div>
                    <div className="flex items-start justify-between gap-3"><span>{t('table.errorDetailsTitle')}</span><span className="break-all font-mono">{formatText(record.errorMessage)}</span></div>
                  </div>
                </div>
              )}
            </article>
          )
        })}
      </div>

      <div className="hidden md:block overflow-x-auto rounded-xl border border-base-300/70 bg-base-100/50">
        <table className="min-w-full table-fixed border-separate border-spacing-0 text-sm">
          <thead className="bg-base-200/65 text-[11px] uppercase tracking-[0.08em] text-base-content/70">
            <tr>
              <th className="px-3 py-3 text-left font-semibold">{t('table.column.time')}</th>
              <th className="px-3 py-3 text-left font-semibold">{t('table.column.proxy')}</th>
              <th className="px-3 py-3 text-left font-semibold">{t('table.column.model')}</th>
              <th className="px-3 py-3 text-left font-semibold">{t('table.column.status')}</th>
              {headers.map((header) => (
                <th key={header} className="px-3 py-3 text-left font-semibold">{header}</th>
              ))}
              <th className="px-3 py-3 text-right font-semibold">
                <span className="sr-only">{t('records.table.details')}</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {records.map((record, index) => {
              const isExpanded = expandedId === record.id
              const statusMeta = resolveStatusMeta(resolveInvocationDisplayStatus(record))
              const statusLabel = statusMeta.labelKey ? t(statusMeta.labelKey) : statusMeta.label ?? t('table.status.unknown')
              return (
                <Fragment key={record.id}>
                  <tr className={index % 2 === 0 ? 'bg-base-100/30' : 'bg-base-200/18'}>
                    <td className="px-3 py-3 align-middle text-left text-xs font-medium">{formatOccurredAt(record.occurredAt, dateTimeFormatter)}</td>
                    <td className="max-w-[12rem] truncate px-3 py-3 align-middle text-left text-xs" title={resolveProxyName(record)}>{resolveProxyName(record)}</td>
                    <td className="max-w-[14rem] truncate px-3 py-3 align-middle text-left text-xs" title={record.model ?? undefined}>{formatText(record.model)}</td>
                    <td className="px-3 py-3 align-middle text-left text-xs"><Badge variant={statusMeta.variant}>{statusLabel}</Badge></td>
                    {renderFocusCells(record)}
                    <td className="px-3 py-3 align-middle text-right">
                      <button
                        type="button"
                        className="inline-flex items-center justify-center rounded-md text-base-content/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                        onClick={() => setExpandedId((current) => (current === record.id ? null : record.id))}
                        aria-expanded={isExpanded}
                        aria-label={isExpanded ? t('records.table.hideDetails') : t('records.table.showDetails')}
                      >
                        <Icon icon={isExpanded ? 'mdi:chevron-down' : 'mdi:chevron-right'} className="h-4 w-4" aria-hidden />
                      </button>
                    </td>
                  </tr>
                  {isExpanded && (
                    <tr className="bg-base-200/55">
                      <td colSpan={9} className="px-4 py-4">
                        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3 text-xs">
                          <div className="rounded-lg border border-base-300/70 bg-base-100/55 p-3">
                            <div className="mb-2 text-[11px] font-semibold uppercase tracking-[0.08em] text-base-content/60">{t('table.detailsTitle')}</div>
                            <div className="space-y-2">
                              <div className="flex items-start justify-between gap-3"><span>{t('table.details.invokeId')}</span><span className="break-all font-mono">{record.invokeId}</span></div>
                              <div className="flex items-start justify-between gap-3"><span>{t('table.details.source')}</span><span className="break-all font-mono">{formatText(record.source)}</span></div>
                              <div className="flex items-start justify-between gap-3"><span>{t('table.details.endpoint')}</span><span className="break-all font-mono">{formatText(record.endpoint)}</span></div>
                              <div className="flex items-start justify-between gap-3"><span>{t('table.details.promptCacheKey')}</span><span className="break-all font-mono">{formatText(record.promptCacheKey)}</span></div>
                              <div className="flex items-start justify-between gap-3"><span>{t('table.details.requesterIp')}</span><span className="break-all font-mono">{formatText(record.requesterIp)}</span></div>
                            </div>
                          </div>
                          <div className="rounded-lg border border-base-300/70 bg-base-100/55 p-3">
                            <div className="mb-2 text-[11px] font-semibold uppercase tracking-[0.08em] text-base-content/60">{t('records.table.focusTitle')}</div>
                            <div className="space-y-2">
                              <div className="flex items-start justify-between gap-3"><span>{t('records.table.token.totalTokens')}</span><span className="font-mono">{formatNumber(record.totalTokens, numberFormatter)}</span></div>
                              <div className="flex items-start justify-between gap-3"><span>{t('records.table.token.cost')}</span><span className="font-mono">{formatCost(record.cost, costFormatter)}</span></div>
                              <div className="flex items-start justify-between gap-3"><span>{t('records.table.network.ttfb')}</span><span className="font-mono">{formatMilliseconds(record.tUpstreamTtfbMs)}</span></div>
                              <div className="flex items-start justify-between gap-3"><span>{t('records.table.network.totalMs')}</span><span className="font-mono">{formatMilliseconds(record.tTotalMs)}</span></div>
                              <div className="flex items-start justify-between gap-3"><span>{t('records.table.exception.failureKind')}</span><span className="break-all font-mono">{formatText(record.failureKind)}</span></div>
                            </div>
                          </div>
                          <div className="rounded-lg border border-base-300/70 bg-base-100/55 p-3 md:col-span-2 xl:col-span-1">
                            <div className="mb-2 text-[11px] font-semibold uppercase tracking-[0.08em] text-base-content/60">{t('table.errorDetailsTitle')}</div>
                            <pre className="whitespace-pre-wrap break-words font-mono text-xs">{formatText(record.errorMessage)}</pre>
                          </div>
                        </div>
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
  )
}
