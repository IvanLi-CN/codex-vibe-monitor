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
    <div
      className="overflow-x-auto rounded-xl border border-base-300/70 bg-base-100/52 backdrop-blur"
      data-testid="invocation-table-scroll"
    >
      <table className="w-full min-w-full border-separate border-spacing-0 text-sm">
        <thead className="bg-base-200/65 text-[11px] uppercase tracking-[0.08em] text-base-content/70">
          <tr>
            <th className="w-28 px-3 py-2.5 text-left font-semibold whitespace-nowrap">{t('table.column.time')}</th>
            <th className="w-28 px-3 py-2.5 text-center font-semibold whitespace-nowrap">
              <div className="flex flex-col leading-tight">
                <span>{t('table.column.status')}</span>
                <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                  {t('table.column.latency')}
                </span>
              </div>
            </th>
            <th className="w-36 px-3 py-2.5 text-right font-semibold whitespace-nowrap">
              <div className="flex flex-col leading-tight">
                <span>{t('table.column.model')}</span>
                <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                  {t('table.column.costUsd')}
                </span>
              </div>
            </th>
            <th className="w-32 px-3 py-2.5 text-right font-semibold whitespace-nowrap">
              <div className="flex flex-col leading-tight">
                <span>{t('table.column.inputTokens')}</span>
                <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                  {t('table.column.cacheInputTokens')}
                </span>
              </div>
            </th>
            <th className="w-24 px-3 py-2.5 text-right font-semibold whitespace-nowrap">{t('table.column.outputTokens')}</th>
            <th className="w-24 px-3 py-2.5 text-right font-semibold whitespace-nowrap">{t('table.column.totalTokens')}</th>
            <th className="hidden min-w-64 px-3 py-2.5 text-left font-semibold xl:table-cell">
              <div className="flex flex-col leading-tight">
                <span>{t('table.column.error')}</span>
                <span className="text-[10px] font-medium normal-case tracking-normal text-base-content/60">
                  {t('table.details.endpoint')}
                </span>
              </div>
            </th>
            <th className="w-12 px-3 py-2.5 text-right">
              <span className="sr-only">{toggleLabels.header}</span>
            </th>
          </tr>
        </thead>
        <tbody className="divide-y divide-base-300/65">
          {records.map((record, rowIndex) => {
            const occurred = new Date(record.occurredAt)
            const normalizedStatus = (record.status ?? 'unknown').toLowerCase()
            const meta = STATUS_META[normalizedStatus] ?? FALLBACK_STATUS_META
            const recordId = record.id
            const detailId = `invocation-details-${recordId}`
            const isExpanded = expandedId === recordId
            const errorMessage = record.errorMessage?.trim() ?? ''
            const endpointValue = record.endpoint?.trim() || FALLBACK_CELL
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
              { label: 'table.details.codexSessionId', value: record.codexSessionId || FALLBACK_CELL },
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

            const handleToggle = () => {
              setExpandedId((current) => (current === recordId ? null : recordId))
            }

            return (
              <Fragment key={recordId}>
                <tr className={`${rowIndex % 2 === 0 ? 'bg-base-100/38' : 'bg-base-200/22'} hover:bg-primary/6`}>
                  <td className="border-t border-base-300/65 px-3 py-2.5 align-middle">
                    <div className="flex flex-col justify-center gap-1 leading-tight">
                      <span className="block truncate whitespace-nowrap font-medium">{occurredTime}</span>
                      <span className="block truncate whitespace-nowrap text-base-content/70">{occurredDate}</span>
                    </div>
                  </td>
                  <td className="border-t border-base-300/65 px-3 py-2.5 align-middle text-center">
                    <div className="flex flex-col items-center justify-center gap-1 leading-tight">
                      <Badge variant={meta.variant} className="justify-center whitespace-nowrap">
                        {t(meta.key)}
                      </Badge>
                      <span className="block whitespace-nowrap font-mono text-xs text-base-content/70 sm:hidden" title={latencySummary}>
                        {latencyCompactSummary}
                      </span>
                      <span className="hidden whitespace-nowrap font-mono text-xs text-base-content/70 sm:block" title={latencySummary}>
                        {latencySummary}
                      </span>
                    </div>
                  </td>
                  <td className="border-t border-base-300/65 px-3 py-2.5 align-middle">
                    <div className="flex flex-col items-end justify-center gap-1 leading-tight text-right">
                      <span className="block truncate whitespace-nowrap text-base-content/85" title={record.model ?? FALLBACK_CELL}>
                        {record.model ?? FALLBACK_CELL}
                      </span>
                      <span className="block truncate whitespace-nowrap font-mono tabular-nums text-base-content/70">
                        {typeof record.cost === 'number' ? currencyFormatter.format(record.cost) : FALLBACK_CELL}
                      </span>
                    </div>
                  </td>
                  <td className="border-t border-base-300/65 px-3 py-2.5 align-middle">
                    <div className="flex flex-col items-end justify-center gap-1 leading-tight text-right">
                      <span className="block truncate whitespace-nowrap font-mono tabular-nums">
                        {formatOptionalNumber(record.inputTokens, numberFormatter)}
                      </span>
                      <span className="block truncate whitespace-nowrap font-mono tabular-nums text-base-content/70">
                        {formatOptionalNumber(record.cacheInputTokens, numberFormatter)}
                      </span>
                    </div>
                  </td>
                  <td className="border-t border-base-300/65 px-3 py-2.5 align-middle text-right font-mono tabular-nums">
                    <span className="block truncate whitespace-nowrap">
                      {formatOptionalNumber(record.outputTokens, numberFormatter)}
                    </span>
                  </td>
                  <td className="border-t border-base-300/65 px-3 py-2.5 align-middle text-right font-mono tabular-nums">
                    <span className="block truncate whitespace-nowrap">
                      {formatOptionalNumber(record.totalTokens, numberFormatter)}
                    </span>
                  </td>
                  <td className="hidden border-t border-base-300/65 px-3 py-2.5 align-middle xl:table-cell">
                    <div className="flex flex-col justify-center gap-1 leading-tight">
                      <span className="block truncate whitespace-nowrap text-base-content/70" title={endpointValue}>
                        {endpointValue}
                      </span>
                      <span className="block truncate whitespace-nowrap" title={errorMessage || undefined}>
                        {errorMessage || FALLBACK_CELL}
                      </span>
                    </div>
                  </td>
                  <td className="border-t border-base-300/65 px-3 py-2.5 align-middle text-right">
                    <button
                      type="button"
                      className="inline-flex items-center justify-end gap-1 text-lg leading-none text-base-content/70 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                      onClick={handleToggle}
                      aria-expanded={isExpanded}
                      aria-controls={detailId}
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
                    <td colSpan={8} className="border-t border-base-300/65 px-3 py-2.5">
                      <div id={detailId} className="flex flex-col gap-4 p-4">
                        <div className="flex flex-col gap-2">
                          <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">
                            {t('table.detailsTitle')}
                          </span>
                          <div className="grid gap-2 md:grid-cols-2">
                            {detailPairs.map((entry) => (
                              <div key={entry.label} className="flex items-start gap-2">
                                <span className="min-w-36 text-xs uppercase tracking-wide text-base-content/60">{t(entry.label)}</span>
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
                                <span className="min-w-36 text-xs uppercase tracking-wide text-base-content/60">{t(entry.label)}</span>
                                <span className="font-mono text-sm">{entry.value}</span>
                              </div>
                            ))}
                          </div>
                        </div>

                        {errorMessage && (
                          <div className="flex flex-col gap-2">
                            <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">
                              {t('table.errorDetailsTitle')}
                            </span>
                            <pre className="whitespace-pre-wrap break-words font-mono text-sm">{errorMessage}</pre>
                          </div>
                        )}
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
  )
}
