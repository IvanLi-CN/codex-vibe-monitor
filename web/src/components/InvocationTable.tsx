import { Fragment, useEffect, useMemo, useState } from 'react'
import { Icon } from '@iconify/react'
import type { ApiInvocation } from '../lib/api'
import { useTranslation } from '../i18n'
import type { TranslationKey } from '../i18n'

interface InvocationTableProps {
  records: ApiInvocation[]
  isLoading: boolean
  error?: string | null
}

const STATUS_META: Record<string, { className: string; key: TranslationKey }> = {
  success: { className: 'badge-success', key: 'table.status.success' },
  failed: { className: 'badge-error', key: 'table.status.failed' },
  running: { className: 'badge-info', key: 'table.status.running' },
  pending: { className: 'badge-warning', key: 'table.status.pending' },
}

const FALLBACK_STATUS_META = { className: 'badge-neutral', key: 'table.status.unknown' as TranslationKey }
const FALLBACK_CELL = '—'

function formatMilliseconds(value: number | null | undefined) {
  if (typeof value !== 'number' || !Number.isFinite(value)) return FALLBACK_CELL
  return `${value.toFixed(1)} ms`
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
    () => new Intl.DateTimeFormat(localeTag, { hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false }),
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
      <div className="alert alert-error">
        <span>{t('table.loadError', { error })}</span>
      </div>
    )
  }

  if (isLoading) {
    return (
      <div className="flex justify-center py-10">
        <span className="loading loading-bars loading-lg" aria-label={t('table.loadingRecordsAria')} />
      </div>
    )
  }

  if (records.length === 0) {
    return <div className="alert">{t('table.noRecords')}</div>
  }

  return (
    <div className="overflow-x-auto">
      <table className="table table-zebra">
        <thead>
          <tr>
            <th>{t('table.column.time')}</th>
            <th>{t('table.column.model')}</th>
            <th>{t('table.column.status')}</th>
            <th>{t('table.column.inputTokens')}</th>
            <th>{t('table.column.outputTokens')}</th>
            <th>{t('table.column.cacheInputTokens')}</th>
            <th>{t('table.column.totalTokens')}</th>
            <th>{t('table.column.costUsd')}</th>
            <th>
              <div className="flex flex-col leading-tight">
                <span>{t('table.column.latency')}</span>
                <span className="text-xs text-base-content/60">{t('table.latency.firstByteTotal')}</span>
              </div>
            </th>
            <th>{t('table.column.error')}</th>
            <th className="w-12 text-right">
              <span className="sr-only">{toggleLabels.header}</span>
            </th>
          </tr>
        </thead>
        <tbody>
          {records.map((record) => {
            const occurred = new Date(record.occurredAt)
            const normalizedStatus = (record.status ?? 'unknown').toLowerCase()
            const meta = STATUS_META[normalizedStatus] ?? FALLBACK_STATUS_META
            const recordId = record.id
            const detailId = `invocation-details-${recordId}`
            const isExpanded = expandedId === recordId
            const errorMessage = record.errorMessage?.trim() ?? ''
            const latencySummary = `${formatMilliseconds(record.tUpstreamTtfbMs)} / ${formatMilliseconds(record.tTotalMs)}`

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
                <tr>
                  <td>
                    {Number.isNaN(occurred.getTime())
                      ? record.occurredAt
                      : dateFormatter.format(occurred)}
                  </td>
                  <td>{record.model ?? FALLBACK_CELL}</td>
                  <td>
                    <span className={`badge whitespace-nowrap ${meta.className}`}>
                      {t(meta.key)}
                    </span>
                  </td>
                  <td>{formatOptionalNumber(record.inputTokens, numberFormatter)}</td>
                  <td>{formatOptionalNumber(record.outputTokens, numberFormatter)}</td>
                  <td>{formatOptionalNumber(record.cacheInputTokens, numberFormatter)}</td>
                  <td>{formatOptionalNumber(record.totalTokens, numberFormatter)}</td>
                  <td>{typeof record.cost === 'number' ? currencyFormatter.format(record.cost) : FALLBACK_CELL}</td>
                  <td className="whitespace-nowrap">{latencySummary}</td>
                  <td className="max-w-xs">
                    {errorMessage ? (
                      <span className="block max-w-xs truncate" title={errorMessage}>
                        {errorMessage}
                      </span>
                    ) : (
                      FALLBACK_CELL
                    )}
                  </td>
                  <td className="text-right">
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
                  <tr className="bg-base-200">
                    <td colSpan={11}>
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
                            <pre className="whitespace-pre-wrap break-words font-mono text-sm">
                              {errorMessage}
                            </pre>
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
