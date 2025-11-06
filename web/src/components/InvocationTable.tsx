import { useMemo } from 'react'
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
          </tr>
        </thead>
        <tbody>
          {records.map((record) => {
            const occurred = new Date(record.occurredAt)
            const normalizedStatus = (record.status ?? 'unknown').toLowerCase()
            const meta = STATUS_META[normalizedStatus] ?? FALLBACK_STATUS_META
            return (
              <tr key={`${record.invokeId}-${record.occurredAt}`}>
                <td>{Number.isNaN(occurred.getTime()) ? record.occurredAt : dateFormatter.format(occurred)}</td>
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
                <td className="max-w-xs truncate" title={record.errorMessage ?? ''}>
                  {record.errorMessage || '—'}
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}
