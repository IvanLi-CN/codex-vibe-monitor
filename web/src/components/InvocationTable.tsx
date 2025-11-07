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

export function InvocationTable({ records, isLoading, error }: InvocationTableProps) {
  const { t, locale } = useTranslation()
  const localeTag = locale === 'zh' ? 'zh-CN' : 'en-US'
  const [expandedId, setExpandedId] = useState<number | null>(null)

  const toggleLabels = useMemo(() => {
    if (locale === 'zh') {
      return {
        header: '详情',
        show: '展开错误详情',
        hide: '收起错误详情',
        expanded: '已展开',
        collapsed: '未展开',
      }
    }
    return {
      header: 'Details',
      show: 'Show error details',
      hide: 'Hide error details',
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
            <th>{t('table.column.totalTokens')}</th>
            <th>{t('table.column.costUsd')}</th>
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
            const detailId = `invocation-error-${recordId}`
            const isExpanded = expandedId === recordId
            const errorMessage = record.errorMessage?.trim() ?? ''
            const hasErrorDetails = errorMessage.length > 0

            const handleToggle = () => {
              if (!hasErrorDetails) return
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
                  <td>{record.model ?? '—'}</td>
                  <td>
                    <span className={`badge ${meta.className}`}>
                      {t(meta.key)}
                    </span>
                  </td>
                  <td>{numberFormatter.format(record.inputTokens ?? 0)}</td>
                  <td>{numberFormatter.format(record.outputTokens ?? 0)}</td>
                  <td>{numberFormatter.format(record.totalTokens ?? 0)}</td>
                  <td>{currencyFormatter.format(record.cost ?? 0)}</td>
                  <td className="max-w-xs">
                    {hasErrorDetails ? (
                      <button
                        type="button"
                        className="block w-full max-w-xs truncate text-left focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
                        title={record.errorMessage ?? ''}
                        onClick={handleToggle}
                        aria-expanded={isExpanded}
                        aria-controls={detailId}
                        aria-label={isExpanded ? toggleLabels.hide : toggleLabels.show}
                      >
                        {record.errorMessage}
                      </button>
                    ) : (
                      <span title={record.errorMessage ?? ''}>{record.errorMessage || '—'}</span>
                    )}
                  </td>
                  <td className="text-right">
                    {hasErrorDetails ? (
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
                    ) : (
                      <span className="text-base-content/50">—</span>
                    )}
                  </td>
                </tr>
                {isExpanded && (
                  <tr className="bg-base-200">
                    <td colSpan={9}>
                      <div id={detailId} className="flex flex-col gap-2 p-4">
                        <span className="text-xs font-semibold uppercase tracking-wide text-base-content/70">
                          {t('table.errorDetailsTitle')}
                        </span>
                        <pre className="whitespace-pre-wrap break-words font-mono text-sm">
                          {errorMessage || t('table.errorDetailsEmpty')}
                        </pre>
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
