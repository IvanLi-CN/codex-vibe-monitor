import { Fragment, useEffect, useMemo, useState } from 'react'
import { Icon } from '@iconify/react'
import type { ApiInvocation } from '../lib/api'
import { useTranslation } from '../i18n'
import type { TranslationKey } from '../i18n'
import { Alert } from './ui/alert'
import { Badge } from './ui/badge'
import { Spinner } from './ui/spinner'

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

function resolveProxyDisplayName(record: ApiInvocation) {
  const payloadProxyName = record.proxyDisplayName?.trim()
  if (payloadProxyName) return payloadProxyName
  const sourceValue = record.source?.trim()
  if (sourceValue && sourceValue.toLowerCase() !== 'proxy') return sourceValue
  return FALLBACK_CELL
}

interface InvocationRowViewModel {
  record: ApiInvocation
  recordId: number
  meta: { variant: 'default' | 'secondary' | 'success' | 'warning' | 'error'; key: TranslationKey }
  occurredTime: string
  occurredDate: string
  proxyDisplayName: string
  modelValue: string
  costValue: string
  inputTokensValue: string
  cacheInputTokensValue: string
  outputTokensValue: string
  totalTokensValue: string
  endpointValue: string
  errorMessage: string
  latencySummary: string
  latencyCompactSummary: string
  detailPairs: Array<{ label: TranslationKey; value: string }>
  timingPairs: Array<{ label: TranslationKey; value: string }>
}

export function InvocationTable({ records, isLoading, error }: InvocationTableProps) {
  const { t, locale } = useTranslation()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const [expandedId, setExpandedId] = useState<number | null>(null)

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

  useEffect(() => {
    setExpandedId((current) => {
      if (current === null) return current
      return records.some((record) => record.id === current) ? current : null
    })
  }, [records])

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
        const proxyDisplayName = resolveProxyDisplayName(record)
        const latencySummary = `${formatMilliseconds(record.tUpstreamTtfbMs)} / ${formatMilliseconds(record.tTotalMs)}`
        const latencyCompactSummary = `${formatMillisecondsCompact(record.tUpstreamTtfbMs)}/${formatMillisecondsCompact(record.tTotalMs)}`
        const occurredValid = !Number.isNaN(occurred.getTime())
        const occurredTime = occurredValid ? timeFormatter.format(occurred) : record.occurredAt
        const occurredDate = occurredValid ? dateFormatter.format(occurred) : FALLBACK_CELL

        const detailPairs: Array<{ label: TranslationKey; value: string }> = [
          { label: 'table.details.invokeId', value: record.invokeId || FALLBACK_CELL },
          { label: 'table.details.source', value: record.source || FALLBACK_CELL },
          { label: 'table.details.endpoint', value: record.endpoint || FALLBACK_CELL },
          { label: 'table.details.requesterIp', value: record.requesterIp || FALLBACK_CELL },
          { label: 'table.details.promptCacheKey', value: record.promptCacheKey || FALLBACK_CELL },
          { label: 'table.details.failureKind', value: record.failureKind || FALLBACK_CELL },
        ]
        const timingPairs: Array<{ label: TranslationKey; value: string }> = [
          { label: 'table.details.stage.requestRead', value: formatMilliseconds(record.tReqReadMs) },
          { label: 'table.details.stage.requestParse', value: formatMilliseconds(record.tReqParseMs) },
          { label: 'table.details.stage.upstreamConnect', value: formatMilliseconds(record.tUpstreamConnectMs) },
          { label: 'table.details.stage.upstreamFirstByte', value: formatMilliseconds(record.tUpstreamTtfbMs) },
          { label: 'table.details.stage.upstreamStream', value: formatMilliseconds(record.tUpstreamStreamMs) },
          { label: 'table.details.stage.responseParse', value: formatMilliseconds(record.tRespParseMs) },
          { label: 'table.details.stage.persistence', value: formatMilliseconds(record.tPersistMs) },
          { label: 'table.details.stage.total', value: formatMilliseconds(record.tTotalMs) },
        ]

        return {
          record,
          recordId,
          meta,
          occurredTime,
          occurredDate,
          proxyDisplayName,
          modelValue: record.model ?? FALLBACK_CELL,
          costValue: typeof record.cost === 'number' ? currencyFormatter.format(record.cost) : FALLBACK_CELL,
          inputTokensValue: formatOptionalNumber(record.inputTokens, numberFormatter),
          cacheInputTokensValue: formatOptionalNumber(record.cacheInputTokens, numberFormatter),
          outputTokensValue: formatOptionalNumber(record.outputTokens, numberFormatter),
          totalTokensValue: formatOptionalNumber(record.totalTokens, numberFormatter),
          endpointValue,
          errorMessage,
          latencySummary,
          latencyCompactSummary,
          detailPairs,
          timingPairs,
        }
      }),
    [records, currencyFormatter, dateFormatter, numberFormatter, timeFormatter],
  )

  const renderExpandedContent = (
    detailId: string,
    detailPairs: Array<{ label: TranslationKey; value: string }>,
    timingPairs: Array<{ label: TranslationKey; value: string }>,
    errorMessage: string,
    size: 'compact' | 'default',
  ) => (
    <div id={detailId} className={`flex flex-col gap-4 ${size === 'compact' ? 'p-3' : 'p-4'}`}>
      <div className="flex flex-col gap-2">
        <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">{t('table.detailsTitle')}</span>
        <div className="grid gap-2 md:grid-cols-2">
          {detailPairs.map((entry) => (
            <div key={entry.label} className="flex items-start gap-2">
              <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60 md:min-w-36">{t(entry.label)}</span>
              <span className="break-all font-mono text-sm">{entry.value}</span>
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
              <span className="min-w-28 text-xs uppercase tracking-wide text-base-content/60 md:min-w-36">{t(entry.label)}</span>
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

              <div className="mt-2 flex min-w-0 items-center gap-2">
                <Badge variant={row.meta.variant}>{t(row.meta.key)}</Badge>
                <span className="min-w-0 truncate text-xs text-base-content/75" title={row.proxyDisplayName}>
                  {row.proxyDisplayName}
                </span>
              </div>

              <div className="mt-2 text-xs font-mono text-base-content/70" title={row.latencySummary}>
                {row.latencySummary}
              </div>

              <dl className="mt-3 grid grid-cols-2 gap-x-3 gap-y-1 text-xs">
                <dt className="text-base-content/65">{t('table.column.model')}</dt>
                <dd className="truncate text-right" title={row.modelValue}>{row.modelValue}</dd>
                <dt className="text-base-content/65">{t('table.column.costUsd')}</dt>
                <dd className="truncate text-right font-mono">{row.costValue}</dd>
                <dt className="text-base-content/65">{t('table.column.inputTokens')}</dt>
                <dd className="truncate text-right font-mono">{row.inputTokensValue}</dd>
                <dt className="text-base-content/65">{t('table.column.cacheInputTokens')}</dt>
                <dd className="truncate text-right font-mono">{row.cacheInputTokensValue}</dd>
                <dt className="text-base-content/65">{t('table.column.outputTokens')}</dt>
                <dd className="truncate text-right font-mono">{row.outputTokensValue}</dd>
                <dt className="text-base-content/65">{t('table.column.totalTokens')}</dt>
                <dd className="truncate text-right font-mono">{row.totalTokensValue}</dd>
              </dl>

              <div className="mt-3 space-y-1 border-t border-base-300/65 pt-2">
                <div className="text-[10px] uppercase tracking-[0.08em] text-base-content/60">{t('table.details.endpoint')}</div>
                <div className="truncate text-xs text-base-content/75" title={row.endpointValue}>{row.endpointValue}</div>
                <div className="truncate text-xs" title={row.errorMessage || undefined}>{row.errorMessage || FALLBACK_CELL}</div>
              </div>

              {isExpanded && (
                <div className="mt-3 rounded-lg border border-base-300/70 bg-base-200/58">
                  {renderExpandedContent(listDetailId, row.detailPairs, row.timingPairs, row.errorMessage, 'compact')}
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
                <th className="w-[10%] px-2 py-2.5 text-left font-semibold whitespace-nowrap xl:px-3">{t('table.column.time')}</th>
                <th className="w-[15%] px-2 py-2.5 text-center font-semibold whitespace-nowrap xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.proxy')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.column.latency')}
                    </span>
                  </div>
                </th>
                <th className="w-[16%] px-2 py-2.5 text-right font-semibold whitespace-nowrap xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.model')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.column.costUsd')}
                    </span>
                  </div>
                </th>
                <th className="w-[15%] px-2 py-2.5 text-right font-semibold whitespace-nowrap xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.inputTokens')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.column.cacheInputTokens')}
                    </span>
                  </div>
                </th>
                <th className="w-[9%] px-2 py-2.5 text-right font-semibold whitespace-nowrap xl:px-3">{t('table.column.outputTokens')}</th>
                <th className="w-[10%] px-2 py-2.5 text-right font-semibold whitespace-nowrap xl:px-3">{t('table.column.totalTokens')}</th>
                <th className="hidden w-[18%] px-2 py-2.5 text-left font-semibold xl:table-cell xl:px-3">
                  <div className="flex flex-col leading-tight">
                    <span>{t('table.column.error')}</span>
                    <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                      {t('table.details.endpoint')}
                    </span>
                  </div>
                </th>
                <th className="w-[7%] px-2 py-2.5 text-right xl:px-3">
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
                          <Badge variant={row.meta.variant} className="max-w-full justify-center overflow-hidden">
                            <span className="block max-w-full truncate whitespace-nowrap text-center" title={row.proxyDisplayName}>
                              {row.proxyDisplayName}
                            </span>
                            <span className="sr-only">{t(row.meta.key)}</span>
                          </Badge>
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
                          <span className="w-full truncate whitespace-nowrap text-base-content/85" title={row.modelValue}>
                            {row.modelValue}
                          </span>
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
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle text-right font-mono tabular-nums xl:px-3">
                        <span className="block truncate whitespace-nowrap">{row.outputTokensValue}</span>
                      </td>
                      <td className="min-w-0 border-t border-base-300/65 px-2 py-2.5 align-middle text-right font-mono tabular-nums xl:px-3">
                        <span className="block truncate whitespace-nowrap">{row.totalTokensValue}</span>
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
                        <td colSpan={8} className="border-t border-base-300/65 px-2 py-2.5 xl:px-3">
                          {renderExpandedContent(tableDetailId, row.detailPairs, row.timingPairs, row.errorMessage, 'default')}
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
